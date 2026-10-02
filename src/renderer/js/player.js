// Audio engine: two decks (crossfade + gapless preloading) -> 10-band EQ -> leveler -> master.
import { Emitter, mediaUrl, coverUrl, toast } from './util.js';
import { state } from './state.js';

export const EQ_FREQS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

class Deck {
  constructor(player, ctx, dest) {
    this.audio = new Audio();
    this.audio.crossOrigin = 'anonymous';
    this.audio.preload = 'auto';
    this.audio.preservesPitch = true;
    this.node = ctx.createMediaElementSource(this.audio);
    this.gain = ctx.createGain();
    this.node.connect(this.gain).connect(dest);
    this.trackId = null;
    this.audio.addEventListener('ended', () => player._onEnded(this));
    this.audio.addEventListener('error', () => player._onError(this));
  }

  load(track) {
    this.trackId = track.id;
    this.audio.src = mediaUrl(track);
  }

  unload() {
    this.trackId = null;
    this.audio.pause();
    this.audio.removeAttribute('src');
    this.audio.load();
  }
}

class Player extends Emitter {
  constructor() {
    super();
    this.queue = [];
    this.index = -1;
    this.unshuffled = null;
    this.playing = false;
    this.fading = false;
    this.errorStreak = 0;
    this.sleep = null;
  }

  init() {
    const ctx = (this.ctx = new AudioContext({ latencyHint: 'playback' }));
    this.bus = ctx.createGain();
    this.preamp = ctx.createGain();
    this.filters = EQ_FREQS.map((f, i) => {
      const b = ctx.createBiquadFilter();
      b.type = i === 0 ? 'lowshelf' : i === EQ_FREQS.length - 1 ? 'highshelf' : 'peaking';
      b.frequency.value = f;
      b.Q.value = 1.1;
      return b;
    });
    this.leveler = ctx.createDynamicsCompressor();
    this.leveler.threshold.value = -22;
    this.leveler.knee.value = 24;
    this.leveler.ratio.value = 3.5;
    this.leveler.attack.value = 0.005;
    this.leveler.release.value = 0.3;
    this.levelerMakeup = ctx.createGain();
    this.levelerMakeup.gain.value = 1.4;
    this.post = ctx.createGain(); // tap point for analysers: after EQ, before volume
    this.master = ctx.createGain();
    this.analyser = ctx.createAnalyser();
    this.analyser.fftSize = 4096;
    this.analyser.smoothingTimeConstant = 0.72;
    const split = ctx.createChannelSplitter(2);
    this.analyserL = ctx.createAnalyser();
    this.analyserR = ctx.createAnalyser();
    this.analyserL.fftSize = this.analyserR.fftSize = 1024;
    this.post.connect(this.master);
    this.post.connect(this.analyser);
    this.post.connect(split);
    split.connect(this.analyserL, 0);
    split.connect(this.analyserR, 1);
    this.master.connect(ctx.destination);

    this.decks = [new Deck(this, ctx, this.bus), new Deck(this, ctx, this.bus)];
    this.active = 0;

    const p = state.settings.player;
    this.opts = { ...p };
    this.setVolume(p.volume);
    this.applyEq(state.settings.eq);
    this.setSpeed(p.speed || 1);

    setInterval(() => this._tick(), 200);
    this._mediaSession();

    // Restore last session (paused).
    const s = state.settings.session;
    const q = (s?.queue || []).filter((id) => state.track(id));
    if (q.length) {
      this.queue = q;
      this.index = Math.min(Math.max(0, s.index), q.length - 1);
      this._load(this.index, { autoplay: false, startAt: s.position || 0 });
    }
    setInterval(() => this._saveSession(), 5000);
    addEventListener('beforeunload', () => this._saveSession());
  }

  get deck() { return this.decks[this.active]; }
  get other() { return this.decks[1 - this.active]; }
  get current() { return state.track(this.queue[this.index]); }
  get time() { return this.deck.audio.currentTime || 0; }
  get duration() { return this.deck.audio.duration || this.current?.duration || 0; }

  // ---------- routing ----------
  applyEq(eq) {
    this.eq = eq;
    const t = this.ctx.currentTime;
    this.filters.forEach((f, i) => f.gain.setTargetAtTime(eq.enabled ? eq.gains[i] : 0, t, 0.02));
    this.preamp.gain.setTargetAtTime(eq.enabled ? Math.pow(10, eq.preamp / 20) : 1, t, 0.02);
    this._rewire();
  }

  setNormalize(on) {
    this.opts.normalize = on;
    this._savePlayer();
    this._rewire();
  }

  _rewire() {
    const nodes = [this.bus, this.preamp, ...this.filters, this.leveler, this.levelerMakeup];
    nodes.forEach((n) => n.disconnect());
    let chain = [this.bus, this.preamp, ...this.filters];
    if (this.opts?.normalize) chain = [...chain, this.leveler, this.levelerMakeup];
    chain.push(this.post);
    for (let i = 0; i < chain.length - 1; i++) chain[i].connect(chain[i + 1]);
  }

  // ---------- queue ----------
  playList(ids, start = 0, { shuffle } = {}) {
    if (!ids.length) return;
    const doShuffle = shuffle ?? this.opts.shuffle;
    let q = [...ids];
    let idx = start;
    this.unshuffled = null;
    if (doShuffle) {
      this.unshuffled = [...q];
      const first = q.splice(start, 1)[0];
      q = [first, ...shuffleArr(q)];
      idx = 0;
      if (shuffle && !this.opts.shuffle) this.toggleShuffle(true, true);
    }
    this.queue = q;
    this.play(idx);
    this.emit('queue');
  }

  playNext(ids) {
    if (this.index < 0) return this.playList(ids, 0);
    this.queue.splice(this.index + 1, 0, ...ids);
    this.unshuffled?.push(...ids);
    this._resetPreload();
    this.emit('queue');
    toast(`${ids.length > 1 ? ids.length + ' tracks' : 'Track'} will play next`);
  }

  enqueue(ids) {
    if (this.index < 0) return this.playList(ids, 0);
    this.queue.push(...ids);
    this.unshuffled?.push(...ids);
    this._resetPreload();
    this.emit('queue');
    toast(`Added ${ids.length > 1 ? ids.length + ' tracks' : '1 track'} to queue`);
  }

  removeAt(i) {
    if (i === this.index) {
      this.queue.splice(i, 1);
      if (!this.queue.length) {
        this.pause();
        this.deck.unload();
        this.index = -1;
        this.emit('track', null);
        return this.emit('queue');
      }
      return this._load(Math.min(i, this.queue.length - 1), { autoplay: this.playing });
    }
    this.queue.splice(i, 1);
    if (i < this.index) this.index--;
    this._resetPreload();
    this.emit('queue');
  }

  move(from, to) {
    if (from === to) return;
    const [id] = this.queue.splice(from, 1);
    this.queue.splice(to, 0, id);
    if (this.index === from) this.index = to;
    else if (from < this.index && to >= this.index) this.index--;
    else if (from > this.index && to <= this.index) this.index++;
    this._resetPreload();
    this.emit('queue');
  }

  clearUpcoming() {
    this.queue.splice(this.index + 1);
    this.unshuffled = null;
    this._resetPreload();
    this.emit('queue');
  }

  toggleShuffle(force, silent) {
    const on = force ?? !this.opts.shuffle;
    this.opts.shuffle = on;
    if (!silent && this.queue.length) {
      const cur = this.queue[this.index];
      if (on) {
        this.unshuffled = [...this.queue];
        const upcoming = shuffleArr(this.queue.slice(this.index + 1));
        this.queue = [...this.queue.slice(0, this.index + 1), ...upcoming];
      } else if (this.unshuffled) {
        this.queue = this.unshuffled;
        this.index = Math.max(0, this.queue.indexOf(cur));
        this.unshuffled = null;
      }
      this._resetPreload();
      this.emit('queue');
    }
    this._savePlayer();
    this.emit('options');
  }

  cycleRepeat() {
    this.opts.repeat = { off: 'all', all: 'one', one: 'off' }[this.opts.repeat || 'off'];
    this._savePlayer();
    this.emit('options');
  }

  // ---------- transport ----------
  play(index) {
    if (index == null) return this.resume();
    if (index < 0 || index >= this.queue.length) return;
    this._load(index, { autoplay: true });
  }

  async resume() {
    if (this.index < 0) return;
    await this.ctx.resume();
    this.deck.gain.gain.cancelScheduledValues(0);
    this.deck.gain.gain.value = 1;
    await this.deck.audio.play().catch(() => {});
    this._setPlaying(true);
  }

  pause() {
    this.decks.forEach((d) => d.audio.pause());
    if (this.fading) this._finishFade();
    this._setPlaying(false);
  }

  toggle() { this.playing ? this.pause() : this.resume(); }

  stop() {
    this.pause();
    this.deck.audio.currentTime = 0;
    this.emit('time');
  }

  next(manual = false) {
    if (!this.queue.length) return;
    if (!manual && this.opts.repeat === 'one') return this._load(this.index, { autoplay: true });
    let i = this.index + 1;
    if (i >= this.queue.length) {
      if (this.opts.repeat === 'all' || manual) i = 0;
      else {
        this.pause();
        this.deck.audio.currentTime = 0;
        return;
      }
    }
    this._load(i, { autoplay: true });
    return true;
  }

  prev() {
    if (this.time > 3) return this.seek(0);
    const i = this.index - 1 < 0 ? (this.opts.repeat === 'all' ? this.queue.length - 1 : 0) : this.index - 1;
    this._load(i, { autoplay: this.playing || true });
  }

  seek(t) {
    const d = this.duration;
    if (!d) return;
    this.deck.audio.currentTime = Math.max(0, Math.min(d - 0.1, t));
    if (this.fading) this._finishFade();
    this.emit('time');
  }

  setVolume(v) {
    v = Math.max(0, Math.min(1, v));
    this.opts.volume = v;
    this.muted = false;
    this.master.gain.setTargetAtTime(v * v, this.ctx.currentTime, 0.015);
    this._savePlayer();
    this.emit('volume');
  }

  toggleMute() {
    this.muted = !this.muted;
    this.master.gain.setTargetAtTime(this.muted ? 0 : this.opts.volume ** 2, this.ctx.currentTime, 0.015);
    this.emit('volume');
  }

  setSpeed(s) {
    this.opts.speed = s;
    this.decks.forEach((d) => { d.audio.playbackRate = s; d.audio.defaultPlaybackRate = s; });
    this._savePlayer();
    this.emit('options');
  }

  setCrossfade(sec) {
    this.opts.crossfade = sec;
    this._savePlayer();
  }

  setSleep(mode) {
    clearTimeout(this.sleep?.timer);
    this.sleep = null;
    if (mode === 'track') this.sleep = { mode };
    else if (mode > 0) {
      const until = Date.now() + mode * 60000;
      this.sleep = { mode, until, timer: setTimeout(() => this._sleepNow(), mode * 60000) };
    }
    this.emit('sleep');
  }

  _sleepNow() {
    const g = this.deck.gain.gain;
    const t = this.ctx.currentTime;
    g.setValueAtTime(g.value, t);
    g.linearRampToValueAtTime(0, t + 8);
    setTimeout(() => { this.pause(); g.value = 1; }, 8200);
    this.sleep = null;
    this.emit('sleep');
  }

  // ---------- internals ----------
  _load(index, { autoplay, startAt = 0 }) {
    const track = state.track(this.queue[index]);
    if (!track) return;
    if (this.fading) this._finishFade();
    const prevDeck = this.deck;
    // Use the preloaded deck if it already holds this track (gapless).
    if (this.other.trackId === track.id && startAt === 0) {
      this.active = 1 - this.active;
    } else {
      this.deck.load(track);
    }
    if (prevDeck !== this.deck) prevDeck.unload();
    this.index = index;
    this._resetPreload();
    this.counted = false;
    this.listened = 0;
    const a = this.deck.audio;
    a.playbackRate = this.opts.speed || 1;
    this.deck.gain.gain.cancelScheduledValues(0);
    this.deck.gain.gain.value = 1;
    if (startAt) {
      const set = () => { a.currentTime = startAt; };
      a.readyState >= 1 ? set() : a.addEventListener('loadedmetadata', set, { once: true });
    } else if (a.currentTime) a.currentTime = 0;
    if (autoplay) {
      this.ctx.resume();
      a.play().then(() => (this.errorStreak = 0)).catch(() => {});
      this._setPlaying(true);
    } else {
      this._setPlaying(false);
    }
    this.emit('track', track);
    this.emit('queue');
    this._updateMediaSession(track);
  }

  _onEnded(deck) {
    if (deck !== this.deck) return;
    if (this.sleep?.mode === 'track') {
      this.sleep = null;
      this.emit('sleep');
      this.pause();
      return;
    }
    this.next(false);
  }

  _onError(deck) {
    if (deck !== this.deck || !deck.trackId) return;
    const t = state.track(deck.trackId);
    toast(`Can't play "${t?.title || 'file'}" — file missing or unsupported`, { error: true });
    if (++this.errorStreak < 5 && this.playing) setTimeout(() => this.next(true), 600);
    else this._setPlaying(false);
  }

  _setPlaying(on) {
    this.playing = on;
    document.body.classList.toggle('playing', on);
    if ('mediaSession' in navigator) navigator.mediaSession.playbackState = on ? 'playing' : 'paused';
    this.emit('state', on);
  }

  _tick() {
    if (!this.playing) return;
    const a = this.deck.audio;
    const d = a.duration;
    this.emit('time');
    // Play counting: half the track or 30s, whichever comes first.
    this.listened += 0.2 * (a.playbackRate || 1);
    if (!this.counted && d && this.listened >= Math.min(30, d / 2)) {
      this.counted = true;
      const id = this.deck.trackId;
      dk.library.played(id).then((s) => { state.stats[id] = s; state.emit('stats', id); });
    }
    if (!d || !isFinite(d)) return;
    const remaining = (d - a.currentTime) / (a.playbackRate || 1);
    const nextIdx = this._nextIndex();
    const xf = this.opts.crossfade || 0;
    if (xf > 0 && !this.fading && nextIdx != null && remaining <= xf && remaining > 0.3 && this.opts.repeat !== 'one') {
      this._startFade(nextIdx, remaining);
    } else if (!this.fading && nextIdx != null && remaining < 12) {
      // Gapless: get the next file decoded and ready.
      const nid = this.queue[nextIdx];
      if (this.other.trackId !== nid) {
        const t = state.track(nid);
        if (t) this.other.load(t);
      }
    }
    if ('mediaSession' in navigator && navigator.mediaSession.setPositionState) {
      try { navigator.mediaSession.setPositionState({ duration: d, position: Math.min(a.currentTime, d), playbackRate: a.playbackRate || 1 }); } catch {}
    }
  }

  _nextIndex() {
    if (this.index + 1 < this.queue.length) return this.index + 1;
    if (this.opts.repeat === 'all' && this.queue.length) return 0;
    return null;
  }

  _startFade(nextIdx, dur) {
    const track = state.track(this.queue[nextIdx]);
    if (!track) return;
    this.fading = true;
    const out = this.deck;
    const inc = this.other;
    if (inc.trackId !== track.id) inc.load(track);
    const t = this.ctx.currentTime;
    out.gain.gain.cancelScheduledValues(t);
    out.gain.gain.setValueAtTime(1, t);
    out.gain.gain.linearRampToValueAtTime(0, t + dur);
    inc.gain.gain.cancelScheduledValues(t);
    inc.gain.gain.setValueAtTime(0, t);
    inc.gain.gain.linearRampToValueAtTime(1, t + dur);
    inc.audio.currentTime = 0;
    inc.audio.playbackRate = this.opts.speed || 1;
    inc.audio.play().catch(() => {});
    this.active = 1 - this.active;
    this.index = nextIdx;
    this.counted = false;
    this.listened = 0;
    this._fadeOut = out;
    this._fadeTimer = setTimeout(() => this._finishFade(), dur * 1000 + 100);
    this.emit('track', track);
    this.emit('queue');
    this._updateMediaSession(track);
  }

  _finishFade() {
    clearTimeout(this._fadeTimer);
    this.fading = false;
    if (this._fadeOut && this._fadeOut !== this.deck) this._fadeOut.unload();
    this._fadeOut = null;
    this.deck.gain.gain.cancelScheduledValues(0);
    this.deck.gain.gain.value = 1;
  }

  _resetPreload() {
    if (!this.fading && this.other.trackId && this.other.trackId !== this.queue[this.index + 1]) this.other.unload();
  }

  _savePlayer() {
    const { volume, crossfade, speed, visualizer, shuffle, repeat, normalize } = this.opts;
    state.set('player', { ...state.settings.player, volume, crossfade, speed, visualizer, shuffle, repeat, normalize });
  }

  setVisualizer(mode) {
    this.opts.visualizer = mode;
    this._savePlayer();
  }

  _saveSession() {
    state.set('session', { queue: this.queue.slice(0, 5000), index: this.index, position: this.time });
  }

  _mediaSession() {
    if (!('mediaSession' in navigator)) return;
    const ms = navigator.mediaSession;
    ms.setActionHandler('play', () => this.resume());
    ms.setActionHandler('pause', () => this.pause());
    ms.setActionHandler('previoustrack', () => this.prev());
    ms.setActionHandler('nexttrack', () => this.next(true));
    ms.setActionHandler('stop', () => this.stop());
    ms.setActionHandler('seekto', (e) => this.seek(e.seekTime));
    ms.setActionHandler('seekbackward', (e) => this.seek(this.time - (e.seekOffset || 10)));
    ms.setActionHandler('seekforward', (e) => this.seek(this.time + (e.seekOffset || 10)));
  }

  _updateMediaSession(t) {
    if (!('mediaSession' in navigator)) return;
    navigator.mediaSession.metadata = new MediaMetadata({
      title: t.title,
      artist: t.artist,
      album: t.album,
      artwork: t.cover ? [{ src: coverUrl(t), sizes: '512x512' }] : [],
    });
    document.title = `${t.title} — ${t.artist} · DK.FM`;
  }
}

function shuffleArr(a) {
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

export const player = new Player();

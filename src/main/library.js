// Local library: scans folders for audio, reads tags + cover art, tracks stats & playlists.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { app, nativeImage } = require('electron');
const mm = require('music-metadata');
const { Store } = require('./store');

const AUDIO_EXT = new Set(['.mp3', '.m4a', '.aac', '.flac', '.ogg', '.opus', '.wav', '.wma', '.aiff', '.aif', '.webm', '.alac']);

const SCHEMA = 4; // bump to force re-reading tags after metadata changes

// Artist + title fingerprint, so songs you already own (local files with no source IDs) are
// recognised: "Me and My Guitar (feat. X) [Remastered]" by "A Boogie, Y" -> "a boogie|me and my guitar".
const taKey = (artist, title) => {
  const n = (s) => String(s || '').normalize('NFKD').replace(/[\u0300-\u036f]/g, '').toLowerCase()
    // drop bracketed extras (feat., prod., remaster, video credits) unless they mark another version
    .replace(/\s*[([]([^)\]]*)[)\]]/g, (_, inner) => (/remix|live|acoustic|version|edit|mix|slowed|sped|instrumental|cover/.test(inner) ? ' ' + inner : ''))
    .replace(/[^a-z0-9]+/g, ' ').trim();
  return 'ta:' + n(String(artist || '').split(/,|;| feat\.? | & /i)[0]) + '|' + n(title);
};

const idFor = (p) => crypto.createHash('sha1').update(path.resolve(p).toLowerCase()).digest('hex').slice(0, 16);

class Library {
  constructor(emit) {
    this.emit = emit;
    this.store = new Store('library', { tracks: {}, playlists: [], stats: {} });
    this.coverDir = path.join(app.getPath('userData'), 'covers');
    fs.mkdirSync(this.coverDir, { recursive: true });
    this.scanning = false;
    this._emitTimer = null;
  }

  // Coalesce change notifications: a 300-song import would otherwise ship the whole library
  // to the renderer 300 times.
  changed() {
    clearTimeout(this._emitTimer);
    this._emitTimer = setTimeout(() => this.emit('library:changed', this.snapshot()), 250);
  }

  snapshot() {
    const { tracks, playlists, stats } = this.store.data;
    return { tracks: Object.values(tracks), playlists, stats };
  }

  async scan(folders) {
    if (this.scanning) return;
    this.scanning = true;
    try {
      const files = [];
      for (const f of folders) await walk(f, files);
      const tracks = this.store.data.tracks;
      const seen = new Set();
      let done = 0;
      let dirty = false;
      let lastProgress = 0;
      // 8 files in flight: tag parsing is I/O-bound, so this is several times faster on big libraries.
      let next = 0;
      const worker = async () => {
        while (next < files.length) {
          const file = files[next++];
          const id = idFor(file);
          seen.add(id);
          let st;
          try { st = await fs.promises.stat(file); } catch { continue; }
          const existing = tracks[id];
          if (!existing || existing.mtime !== st.mtimeMs || existing.v !== SCHEMA) {
            const t = await this.readTrack(file, st);
            if (t) { tracks[id] = { ...existing, ...t, addedAt: existing?.addedAt ?? Date.now() }; dirty = true; }
          }
          done++;
          if (Date.now() - lastProgress > 150) { lastProgress = Date.now(); this.emit('library:progress', { done, total: files.length }); }
        }
      };
      await Promise.all(Array.from({ length: 8 }, worker));
      // Drop tracks that lived inside a scanned folder but are gone now.
      const roots = folders.map((f) => path.resolve(f).toLowerCase() + path.sep);
      for (const [id, t] of Object.entries(tracks)) {
        const inRoot = roots.some((r) => path.resolve(t.path).toLowerCase().startsWith(r));
        if (inRoot && !seen.has(id)) { delete tracks[id]; dirty = true; }
      }
      this.emit('library:progress', { done: files.length, total: files.length, finished: true });
      if (dirty) { this.store.save(); this.changed(); }
      this.cleanupFiles();
    } finally {
      this.scanning = false;
    }
  }

  async addFiles(paths, extra = {}) {
    const added = [];
    for (const p of paths) {
      if (!AUDIO_EXT.has(path.extname(p).toLowerCase())) continue;
      try {
        const st = await fs.promises.stat(p);
        const t = await this.readTrack(p, st);
        if (!t) continue;
        const prev = this.store.data.tracks[t.id];
        this.store.data.tracks[t.id] = { ...prev, ...t, ...extra, addedAt: prev?.addedAt ?? Date.now() };
        added.push(this.store.data.tracks[t.id]);
      } catch { /* unreadable */ }
    }
    this.store.save();
    this.changed();
    return added;
  }

  async readTrack(file, st) {
    try {
      // duration:false reads length from headers instead of scanning the whole file; only fall
      // back to a full scan for the rare file without usable headers.
      let meta = await mm.parseFile(file, { duration: false, skipPostHeaders: true });
      if (!meta.format.duration) meta = await mm.parseFile(file, { duration: true, skipPostHeaders: true });
      const c = meta.common;
      let cover = null;
      let thumb = null;
      const pic = c.picture?.[0];
      if (pic?.data?.length) {
        // Stored at most 512px (embedded art is often 3000px / several MB; the app never shows
        // it larger than ~256px). Same art across an album is stored once (content hash).
        const hash = crypto.createHash('sha1').update(pic.data).digest('hex').slice(0, 20);
        const name = `${hash}.jpg`;
        const out = path.join(this.coverDir, name);
        if (!fs.existsSync(out)) {
          const img = nativeImage.createFromBuffer(pic.data);
          if (img.isEmpty()) await fs.promises.writeFile(out, pic.data);
          else {
            const { width } = img.getSize();
            await fs.promises.writeFile(out, (width > 512 ? img.resize({ width: 512, quality: 'good' }) : img).toJPEG(85));
          }
        }
        cover = name;
        // 192px thumbnail for lists, queue and grids (full art can be 3000px / several MB).
        const tname = `t_${hash}.jpg`;
        const tout = path.join(this.coverDir, tname);
        if (!fs.existsSync(tout)) {
          const img = nativeImage.createFromBuffer(pic.data);
          if (!img.isEmpty()) await fs.promises.writeFile(tout, img.resize({ width: 192, height: 192, quality: 'good' }).toJPEG(82));
        }
        if (fs.existsSync(tout)) thumb = tname;
      }
      const duration = meta.format.duration || 0;
      let bitrate = meta.format.bitrate ? Math.round(meta.format.bitrate / 1000) : null;
      if ((!bitrate || bitrate < 32 || bitrate > 6000) && duration) bitrate = Math.round((st.size * 8) / duration / 1000);
      const base = path.basename(file, path.extname(file));
      const guess = base.includes(' - ') ? base.split(' - ') : null;
      return {
        id: idFor(file),
        path: file,
        title: c.title || (guess ? guess.slice(1).join(' - ') : base),
        artist: c.artist || (c.artists && c.artists.join(', ')) || (guess ? guess[0] : 'Unknown Artist'),
        album: c.album || '',
        albumArtist: c.albumartist || '',
        year: c.year || null,
        track: c.track?.no || null,
        genre: c.genre?.[0] || '',
        duration,
        bitrate,
        codec: meta.format.codec || path.extname(file).slice(1).toUpperCase(),
        sampleRate: meta.format.sampleRate || null,
        cover,
        thumb,
        mtime: st.mtimeMs,
        size: st.size,
        v: SCHEMA,
      };
    } catch {
      return null;
    }
  }

  // Delete cover art and waveform files no track uses anymore.
  async cleanupFiles() {
    const used = new Set();
    const ids = new Set(Object.keys(this.store.data.tracks));
    for (const t of Object.values(this.store.data.tracks)) { if (t.cover) used.add(t.cover); if (t.thumb) used.add(t.thumb); }
    for (const p of this.store.data.playlists) if (p.cover && !/^https?:/.test(p.cover)) used.add(p.cover);
    const rm = async (dir, keep) => {
      for (const f of await fs.promises.readdir(dir).catch(() => [])) {
        if (keep(f)) continue;
        const p = path.join(dir, f);
        const st = await fs.promises.stat(p).catch(() => null);
        // Leave fresh files alone: a download may have just written one for a track not saved yet.
        if (st && Date.now() - st.mtimeMs > 10 * 60 * 1000) await fs.promises.rm(p, { force: true }).catch(() => {});
      }
    };
    await rm(this.coverDir, (f) => used.has(f));
    await rm(path.join(app.getPath('userData'), 'waves'), (f) => ids.has(f.replace(/.bin$/, '')));
  }

  // sourceKey / Spotify ID / YouTube ID -> track id, so syncs recognise songs already downloaded.
  keyIndex() {
    const m = new Map();
    for (const t of Object.values(this.store.data.tracks)) {
      if (t.sourceKey) m.set(t.sourceKey, t.id);
      if (t.spotifyId) m.set('sp:' + t.spotifyId, t.id);
      if (t.youtubeId) m.set('yt:' + t.youtubeId, t.id);
      const ta = taKey(t.artist, t.title);
      if (!m.has(ta)) m.set(ta, t.id);
    }
    return m;
  }

  removeByPath(p) {
    const id = idFor(p);
    if (this.store.data.tracks[id]) this.removeTrack(id);
  }

  // ---- stats (plays, likes) ----
  bumpPlay(id) {
    const s = (this.store.data.stats[id] ||= { plays: 0, liked: false, lastPlayed: 0 });
    s.plays++;
    s.lastPlayed = Date.now();
    this.store.save();
    return s;
  }

  // Loudness from the renderer's analysis; saved quietly (no library broadcast).
  setGain(id, db) {
    const t = this.store.data.tracks[id];
    if (!t || typeof db !== 'number') return;
    t.gain = db;
    this.store.save();
  }

  bumpSkip(id) {
    const s = (this.store.data.stats[id] ||= { plays: 0, liked: false, lastPlayed: 0 });
    s.skips = (s.skips || 0) + 1;
    this.store.save();
    return s;
  }

  toggleLike(id) {
    const s = (this.store.data.stats[id] ||= { plays: 0, liked: false, lastPlayed: 0 });
    s.liked = !s.liked;
    this.store.save();
    return s;
  }

  // ---- playlists ----
  upsertPlaylist(pl) {
    const list = this.store.data.playlists;
    if (!pl.id) pl.id = crypto.randomUUID();
    const i = list.findIndex((p) => p.id === pl.id);
    if (i >= 0) list[i] = { ...list[i], ...pl };
    else list.push({ name: 'New Playlist', trackIds: [], source: 'user', createdAt: Date.now(), ...pl });
    this.store.save();
    this.changed();
    return list.find((p) => p.id === pl.id);
  }

  deletePlaylist(id) {
    this.store.data.playlists = this.store.data.playlists.filter((p) => p.id !== id);
    this.store.save();
    this.changed();
  }

  removeTrack(id) {
    delete this.store.data.tracks[id];
    for (const p of this.store.data.playlists) p.trackIds = p.trackIds.filter((t) => t !== id);
    this.store.save();
    this.changed();
  }
}

async function walk(dir, out, depth = 0) {
  if (depth > 12) return;
  let entries;
  try { entries = await fs.promises.readdir(dir, { withFileTypes: true }); } catch { return; }
  for (const e of entries) {
    if (e.name.startsWith('.')) continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) await walk(p, out, depth + 1);
    else if (AUDIO_EXT.has(path.extname(e.name).toLowerCase())) out.push(p);
  }
}

module.exports = { Library, idFor, AUDIO_EXT, taKey };

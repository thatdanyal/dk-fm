// Now-playing deck: cover, LCD readout, waveform seek bar, transport, volume, speed, sleep timer.
import { player } from './player.js';
import { state } from './state.js';
import { $, fmtTime, coverUrl, mediaUrl, setRangeFill, contextMenu, cssVar } from './util.js';

const CASSETTE = `<svg viewBox="0 0 32 22" shape-rendering="crispEdges" style="color:var(--faint)"><g fill="currentColor">
<path d="M1 1h30v20H1z M3 3v16h26V3z" fill-rule="evenodd"/><rect x="5" y="5" width="22" height="3"/>
<path d="M7 10h18v6H7z M9 12v2h2v-2z M21 12v2h2v-2z" fill-rule="evenodd" opacity=".7"/><rect x="9" y="18" width="14" height="1"/></g></svg>`;

const peaksCache = new Map();
let peaks = null;
let peaksFor = null;
let hoverX = null;

export function initDeck() {
  $('#np-cover-ph').innerHTML = CASSETTE;
  $('#tp-play').onclick = () => player.toggle();
  $('#tp-prev').onclick = () => player.prev();
  $('#tp-next').onclick = () => player.next(true);
  $('#tp-shuffle').onclick = () => player.toggleShuffle();
  $('#tp-repeat').onclick = () => player.cycleRepeat();
  $('#tp-like').onclick = () => player.current && state.like(player.current.id);
  $('#tp-mute').onclick = () => player.toggleMute();

  const vol = $('#tp-volume');
  vol.value = player.opts.volume;
  vol.oninput = () => player.setVolume(Number(vol.value));
  vol.addEventListener('wheel', (e) => { e.preventDefault(); player.setVolume(player.opts.volume - Math.sign(e.deltaY) * 0.04); }, { passive: false });

  $('#tp-speed').onclick = (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    contextMenu(r.left, r.bottom + 4, [0.75, 0.9, 1, 1.1, 1.25, 1.5, 2].map((s) => ({ label: `${s === player.opts.speed ? '● ' : '  '}${s.toFixed(2)}×`, action: () => player.setSpeed(s) })));
  };
  $('#tp-sleep').onclick = (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    const cur = player.sleep;
    contextMenu(r.left, r.bottom + 4, [
      { label: 'Off', action: () => player.setSleep(0), disabled: !cur },
      { label: 'End of this track', action: () => player.setSleep('track') },
      '-',
      ...[10, 15, 30, 45, 60, 90].map((m) => ({ label: `${m} minutes`, action: () => player.setSleep(m) })),
    ]);
  };

  // seek bar
  const seek = $('#seek');
  const frac = (e) => {
    const r = seek.getBoundingClientRect();
    return Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
  };
  let dragging = false;
  seek.addEventListener('pointerdown', (e) => {
    dragging = true;
    seek.setPointerCapture(e.pointerId);
    player.seek(frac(e) * player.duration);
  });
  seek.addEventListener('pointermove', (e) => {
    const f = frac(e);
    hoverX = f;
    const tip = $('#seek-hover');
    tip.textContent = fmtTime(f * player.duration);
    tip.style.left = f * 100 + '%';
    if (dragging) player.seek(f * player.duration);
    drawWave();
  });
  seek.addEventListener('pointerup', () => (dragging = false));
  seek.addEventListener('pointerleave', () => { hoverX = null; drawWave(); });
  new ResizeObserver(() => drawWave()).observe(seek);

  player.on('track', renderTrack);
  player.on('time', renderTime);
  player.on('state', (on) => { $('#np-state').textContent = on ? '▶ PLAY' : player.current ? '❚❚ PAUSE' : '■ STOP'; });
  player.on('options', renderOptions);
  player.on('volume', renderVolume);
  player.on('sleep', renderSleep);
  state.on('stats', (id) => id === player.current?.id && renderLike());
  renderTrack(player.current);
  renderOptions();
  renderVolume();
}

function renderTrack(t) {
  const img = $('#np-cover');
  if (t?.cover) img.src = coverUrl(t);
  else img.removeAttribute('src');
  $('#np-title').textContent = t ? t.title : 'NO TRACK LOADED';
  $('#np-artist').textContent = t ? t.artist : 'INSERT A TAPE';
  $('#np-album').textContent = t ? [t.album, t.year].filter(Boolean).join(' · ') : '';
  $('#np-tech').textContent = t ? [t.bitrate ? `${t.bitrate} KBPS` : null, t.sampleRate ? `${(t.sampleRate / 1000).toFixed(1)} KHZ` : null, codecLabel(t)].filter(Boolean).join(' · ') : '--- KBPS';
  $('#np-state').textContent = player.playing ? '▶ PLAY' : t ? '❚❚ PAUSE' : '■ STOP';
  requestAnimationFrame(() => {
    const m = $('#np-title').parentElement;
    const over = $('#np-title').scrollWidth > m.clientWidth + 2;
    m.classList.toggle('scroll', over);
    m.style.setProperty('--dur', Math.max(8, (t?.title.length || 10) * 0.35) + 's');
  });
  renderLike();
  renderTime();
  loadPeaks(t);
}

function renderTime() {
  $('#np-elapsed').textContent = fmtTime(player.time);
  $('#np-total').textContent = fmtTime(player.duration);
  drawWave();
}

function renderLike() {
  const t = player.current;
  $('#tp-like').classList.toggle('on', !!(t && state.stat(t.id).liked));
}

function renderOptions() {
  $('#tp-shuffle').classList.toggle('on', !!player.opts.shuffle);
  const r = player.opts.repeat || 'off';
  $('#tp-repeat').classList.toggle('on', r !== 'off');
  $('#tp-repeat').textContent = r === 'one' ? 'RPT 1' : 'RPT';
  $('#tp-speed').textContent = (player.opts.speed || 1).toFixed(1) + '×';
  $('#tp-speed').classList.toggle('on', player.opts.speed !== 1);
}

function renderVolume() {
  const v = $('#tp-volume');
  v.value = player.opts.volume;
  setRangeFill(v);
  $('#tp-vol-readout').textContent = player.muted ? '--' : Math.round(player.opts.volume * 100);
  $('#tp-mute').classList.toggle('muted', !!player.muted || player.opts.volume === 0);
}

function renderSleep() {
  const b = $('#tp-sleep');
  b.classList.toggle('on', !!player.sleep);
  clearInterval(renderSleep.t);
  if (player.sleep?.until) {
    const upd = () => { b.textContent = Math.ceil((player.sleep?.until - Date.now()) / 60000) + 'M'; };
    upd();
    renderSleep.t = setInterval(() => (player.sleep ? upd() : renderSleep()), 15000);
  } else b.textContent = player.sleep?.mode === 'track' ? 'EOT' : 'ZZ';
}

function codecLabel(t) {
  const c = (t.codec || '').toUpperCase();
  if (c.includes('LAYER 3') || c === 'MP3') return 'MP3';
  if (c.includes('/')) return c.split('/').pop();
  return c.split(' ')[0];
}

// ---- waveform ----
async function loadPeaks(t) {
  peaksFor = t?.id || null;
  peaks = t ? peaksCache.get(t.id) || null : null;
  drawWave();
  if (!t || peaks) return;
  try {
    const buf = await (await fetch(mediaUrl(t))).arrayBuffer();
    if (peaksFor !== t.id) return;
    const audio = await new OfflineAudioContext(1, 1, 8000).decodeAudioData(buf);
    const N = 1200;
    const out = new Float32Array(N);
    const chs = Math.min(2, audio.numberOfChannels);
    const per = Math.max(1, Math.floor(audio.length / N));
    for (let c = 0; c < chs; c++) {
      const d = audio.getChannelData(c);
      for (let i = 0; i < N; i++) {
        // RMS per bucket: shows dynamics even on brick-walled masters.
        let sum = 0;
        let n = 0;
        const s = i * per;
        for (let k = 0; k < per; k += 2) {
          const v = d[s + k] || 0;
          sum += v * v;
          n++;
        }
        out[i] = Math.max(out[i], Math.sqrt(sum / Math.max(1, n)));
      }
    }
    let max = 0;
    for (const v of out) max = Math.max(max, v);
    for (let i = 0; i < N; i++) out[i] = max ? out[i] / max : 0;
    peaksCache.set(t.id, out);
    if (peaksCache.size > 60) peaksCache.delete(peaksCache.keys().next().value);
    if (peaksFor === t.id) {
      peaks = out;
      drawWave();
    }
  } catch { /* undecodable: plain bar */ }
}

export function drawWave() {
  const c = $('#wave');
  if (!c || !c.offsetParent) return;
  const W = Math.floor(c.clientWidth / 2);
  const H = Math.floor(c.clientHeight / 2);
  if (c.width !== W || c.height !== H) { c.width = W; c.height = H; }
  const g = c.getContext('2d');
  g.fillStyle = cssVar('--bg');
  g.fillRect(0, 0, W, H);
  const prog = player.duration ? player.time / player.duration : 0;
  const played = cssVar('--accent');
  const rest = cssVar('--line-hi');
  const hov = cssVar('--accent-2');
  const mid = Math.floor(H / 2);
  for (let x = 0; x < W; x += 2) {
    const f = x / W;
    let amp;
    if (peaks) {
      const i0 = Math.floor(f * peaks.length);
      const i1 = Math.max(i0 + 1, Math.floor(((x + 2) / W) * peaks.length));
      amp = 0;
      for (let i = i0; i < i1; i++) amp = Math.max(amp, peaks[i]);
      amp = Math.max(1, Math.round(Math.pow(amp, 1.6) * (mid - 1)));
    } else amp = 1;
    g.fillStyle = f <= prog ? played : hoverX != null && f <= hoverX ? hov : rest;
    if (hoverX != null && f > prog && f <= hoverX) g.globalAlpha = 0.6;
    g.fillRect(x, mid - amp, 1, amp * 2);
    g.globalAlpha = 1;
  }
  g.fillStyle = cssVar('--text');
  g.fillRect(Math.floor(prog * W), 0, 1, H);
}

// Now-playing deck: cover, LCD readout, waveform seek bar, transport, volume, speed, sleep timer.
import { player } from './player.js';
import { state } from './state.js';
import { $, fmtTime, coverUrl, mediaUrl, setRangeFill, contextMenu, cssVar } from './util.js';
import { analyze, cachedPeaks } from './analysis.js';

const CASSETTE = `<svg viewBox="0 0 32 22" shape-rendering="crispEdges" style="color:var(--faint)"><g fill="currentColor">
<path d="M1 1h30v20H1z M3 3v16h26V3z" fill-rule="evenodd"/><rect x="5" y="5" width="22" height="3"/>
<path d="M7 10h18v6H7z M9 12v2h2v-2z M21 12v2h2v-2z" fill-rule="evenodd" opacity=".7"/><rect x="9" y="18" width="14" height="1"/></g></svg>`;

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
  setupMarquee(t ? t.title : 'NO TRACK LOADED');
  renderLike();
  renderTime();
  loadPeaks(t);
}

let shownTime = '';
// LCD-style marquee: long titles step one character at a time, 4x per second, only while playing.
// (A smooth CSS scroll forces 60 full-window redraws per second; this is ~4 tiny ones.)
let marquee = { text: '', pos: 0, on: false };
function setupMarquee(text) {
  const el = $('#np-title');
  el.textContent = text;
  marquee = { text, pos: 0, on: false };
  requestAnimationFrame(() => {
    marquee.on = el.scrollWidth > el.parentElement.clientWidth + 2;
  });
}
setInterval(() => {
  if (!marquee.on || !player.playing || document.hidden) return;
  const loop = marquee.text + '   ·   ';
  marquee.pos = (marquee.pos + 1) % loop.length;
  $('#np-title').textContent = loop.slice(marquee.pos) + loop.slice(0, marquee.pos);
}, 250);

// Blinking logo dot, driven by a slow timer instead of a CSS animation (which would keep the
// compositor producing frames continuously).
setInterval(() => {
  const dot = document.querySelector('.logo-dot');
  if (dot) dot.style.visibility = player.playing && !document.hidden && dot.style.visibility !== 'hidden' ? 'hidden' : '';
}, 600);

function renderTime() {
  const txt = fmtTime(player.time) + '|' + fmtTime(player.duration);
  if (txt !== shownTime) {
    shownTime = txt;
    const [a, b] = txt.split('|');
    $('#np-elapsed').textContent = a;
    $('#np-total').textContent = b;
  }
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
// Peaks are computed once per song (then cached on disk). The bar itself is pre-rendered into two
// offscreen layers (played / unplayed); each frame just stitches them at the playhead, and only
// when the playhead or hover position has actually moved a pixel.
async function loadPeaks(t) {
  peaksFor = t?.id || null;
  peaks = t ? cachedPeaks(t.id) : null;
  invalidateWave();
  if (!t || peaks) return;
  const r = await analyze(t).catch(() => null);
  if (r?.peaks) setPeaks(t.id, r.peaks);
}

function setPeaks(id, out) {
  if (peaksFor === id) {
    peaks = out;
    invalidateWave();
  }
}

let layers = null; // { W, H, played, rest, hover }
let lastDraw = '';

export function invalidateWave() {
  layers = null;
  lastDraw = '';
  drawWave();
}

function buildLayers(W, H) {
  const colors = { played: cssVar('--accent'), rest: cssVar('--line-hi'), hover: cssVar('--accent-2') };
  const mid = Math.floor(H / 2);
  const out = { W, H, bg: cssVar('--bg'), head: cssVar('--text') };
  for (const [k, color] of Object.entries(colors)) {
    const cv = new OffscreenCanvas(W, H);
    const g = cv.getContext('2d');
    g.fillStyle = color;
    for (let x = 0; x < W; x += 2) {
      let amp = 1;
      if (peaks) {
        const i0 = Math.floor((x / W) * peaks.length);
        const i1 = Math.max(i0 + 1, Math.floor(((x + 2) / W) * peaks.length));
        amp = 0;
        for (let i = i0; i < i1; i++) amp = Math.max(amp, peaks[i]);
        amp = Math.max(1, Math.round(Math.pow(amp, 1.6) * (mid - 1)));
      }
      g.fillRect(x, mid - amp, 1, amp * 2);
    }
    out[k] = cv;
  }
  return out;
}

export function drawWave() {
  const c = $('#wave');
  if (!c || !c.offsetParent) return;
  const W = Math.floor(c.clientWidth / 2);
  const H = Math.floor(c.clientHeight / 2);
  if (!W || !H) return;
  if (c.width !== W || c.height !== H) { c.width = W; c.height = H; layers = null; }
  if (!layers) layers = buildLayers(W, H);
  const px = Math.floor((player.duration ? player.time / player.duration : 0) * W);
  const hx = hoverX == null ? -1 : Math.floor(hoverX * W);
  const key = px + '|' + hx;
  if (key === lastDraw) return;
  lastDraw = key;
  const g = c.getContext('2d');
  g.fillStyle = layers.bg;
  g.fillRect(0, 0, W, H);
  g.drawImage(layers.rest, 0, 0);
  if (hx > px) {
    g.globalAlpha = 0.6;
    g.drawImage(layers.hover, px, 0, hx - px, H, px, 0, hx - px, H);
    g.globalAlpha = 1;
  }
  if (px > 0) g.drawImage(layers.played, 0, 0, px, H, 0, 0, px, H);
  g.fillStyle = layers.head;
  g.fillRect(px, 0, 1, H);
}

// Visualizer: LED spectrum bars, phosphor oscilloscope, twin analog VU meters, spectrogram.
// Performance: every mode writes straight into one Uint32 pixel buffer and blits it once per
// frame (instead of thousands of fillRect calls), runs at 30 fps by default, and the loop stops
// entirely once playback is paused and the meters have fallen to rest.
import { player } from './player.js';
import { cssVar, h } from './util.js';

export const MODES = [
  ['bars', 'BARS'],
  ['scope', 'SCOPE'],
  ['vu', 'VU'],
  ['spectro', 'WATERFALL'],
  ['off', 'OFF'],
];

const PX = 2; // render at half resolution for chunky pixels

let canvas, ctx, img, buf, W = 0, H = 0;
let mode;
let col; // packed colors
let freq, wave, waveL, waveR;
let peaks = new Float32Array(0);
let glow = new Float32Array(0); // scope phosphor intensity
let heatLut = new Uint32Array(256);
let bandEdges = null;
const vu = { l: 0, r: 0, hold: [0, 0] };
let running = false;
let lastFrame = 0;
let fps = 30;
let idleFrames = 0;

export function initVisualizer() {
  canvas = document.getElementById('scope-canvas');
  canvas.style.imageRendering = 'pixelated';
  ctx = canvas.getContext('2d', { alpha: false, desynchronized: true });
  mode = player.opts.visualizer || 'bars';
  fps = player.opts.visFps || 30;
  freq = new Uint8Array(player.analyser.frequencyBinCount);
  wave = new Uint8Array(player.analyser.fftSize);
  waveL = new Float32Array(player.analyserL.fftSize);
  waveR = new Float32Array(player.analyserR.fftSize);
  refreshColors();
  renderModeButtons();
  canvas.addEventListener('click', cycleMode);
  player.on('state', wake);
  player.on('track', wake);
  new ResizeObserver(wake).observe(canvas);
  document.addEventListener('visibilitychange', wake);
  wake();
}

export function setFps(n) {
  fps = n;
  player.opts.visFps = n;
}

export function setMode(m) {
  mode = m;
  player.setVisualizer(m);
  renderModeButtons();
  if (buf) { buf.fill(col.bg); glow.fill(0); blit(); }
  wake();
}

export function cycleMode() {
  const i = MODES.findIndex((m) => m[0] === mode);
  setMode(MODES[(i + 1) % MODES.length][0]);
}

function renderModeButtons() {
  const box = document.getElementById('scope-modes');
  box.replaceChildren(...MODES.map(([id, label]) => h('button' + (id === mode ? '.on' : ''), { on: { click: (e) => { e.stopPropagation(); setMode(id); } } }, label)));
}

const pack = (c) => {
  const [r, g, b] = toRgb(c);
  return (255 << 24) | (b << 16) | (g << 8) | r; // little-endian RGBA in a Uint32
};

export function refreshColors() {
  col = {
    bg: pack(cssVar('--lcd-bg') || '#100'),
    a: pack(cssVar('--accent')),
    b: pack(cssVar('--accent-2')),
    text: pack(cssVar('--text')),
    dim: pack(cssVar('--faint')),
  };
  const stops = [cssVar('--lcd-bg'), cssVar('--accent'), cssVar('--accent-2'), cssVar('--text')].map(toRgb);
  heatLut = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    const t = (i / 255) * (stops.length - 1);
    const k = Math.min(stops.length - 2, Math.floor(t));
    const f = t - k;
    const c = stops[k].map((c0, j) => Math.round(c0 + (stops[k + 1][j] - c0) * f));
    heatLut[i] = (255 << 24) | (c[2] << 16) | (c[1] << 8) | c[0];
  }
  if (buf) { buf.fill(col.bg); blit(); }
  wake();
}

// Start the loop if there's anything to draw.
function wake() {
  idleFrames = 0;
  if (!running && canvas) {
    running = true;
    requestAnimationFrame(loop);
  }
}

function loop(now) {
  if (!canvas.offsetParent || document.hidden || mode === 'off') {
    running = false;
    if (mode === 'off' && buf) { buf.fill(col.bg); blit(); }
    return;
  }
  // Stop once paused and everything has decayed (~1.5s of frames).
  if (!player.playing && ++idleFrames > fps * 1.5) {
    running = false;
    return;
  }
  requestAnimationFrame(loop);
  if (now - lastFrame < 1000 / fps - 2) return;
  lastFrame = now;
  resize();
  if (mode === 'bars') bars();
  else if (mode === 'scope') scope();
  else if (mode === 'vu') drawVu();
  else if (mode === 'spectro') spectro();
  blit();
}

function resize() {
  const w = Math.max(1, Math.floor(canvas.clientWidth / PX));
  const hh = Math.max(1, Math.floor(canvas.clientHeight / PX));
  if (w === W && hh === H) return;
  W = canvas.width = w;
  H = canvas.height = hh;
  img = ctx.createImageData(W, H);
  buf = new Uint32Array(img.data.buffer);
  buf.fill(col.bg);
  glow = new Float32Array(W * H);
  bandEdges = null;
}

const blit = () => img && ctx.putImageData(img, 0, 0);

function rect(x, y, w, hh, c) {
  x |= 0; y |= 0;
  const x1 = Math.min(W, x + w), y1 = Math.min(H, y + hh);
  if (x < 0) x = 0;
  if (y < 0) y = 0;
  for (let yy = y; yy < y1; yy++) buf.fill(c, yy * W + x, yy * W + x1);
}

const dot = (x, y, c) => {
  x |= 0; y |= 0;
  if (x >= 0 && y >= 0 && x < W && y < H) buf[y * W + x] = c;
};

// Precomputed log-spaced FFT bin ranges per band.
function bands(n) {
  if (bandEdges?.n === n) return bandEdges;
  const nyq = player.ctx.sampleRate / 2;
  const lo = new Uint16Array(n);
  const hi = new Uint16Array(n);
  for (let i = 0; i < n; i++) {
    const f0 = 30 * Math.pow(16000 / 30, i / n);
    const f1 = 30 * Math.pow(16000 / 30, (i + 1) / n);
    lo[i] = Math.floor((f0 / nyq) * freq.length);
    hi[i] = Math.max(lo[i] + 1, Math.floor((f1 / nyq) * freq.length));
  }
  return (bandEdges = { n, lo, hi });
}

function bandValue(b, i) {
  let m = 0;
  for (let k = b.lo[i], e = Math.min(b.hi[i], freq.length); k < e; k++) if (freq[k] > m) m = freq[k];
  return m / 255;
}

function bars() {
  player.analyser.getByteFrequencyData(freq);
  buf.fill(col.bg);
  const barW = W > 300 ? 5 : 3;
  const n = Math.max(8, Math.floor((W - 2) / (barW + 1)));
  const segs = Math.floor((H - 4) / 3);
  if (peaks.length !== n) peaks = new Float32Array(n);
  const b = bands(n);
  const x0 = (W - n * (barW + 1)) >> 1;
  for (let i = 0; i < n; i++) {
    const lit = Math.round(Math.pow(bandValue(b, i), 1.6) * segs);
    const x = x0 + i * (barW + 1);
    for (let s = 0; s < segs; s++) {
      const y = H - 2 - (s + 1) * 3;
      if (s < lit) {
        const f = s / segs;
        rect(x, y, barW, 2, f > 0.8 ? col.text : f > 0.55 ? col.b : col.a);
      } else if ((s & 1) === 0) rect(x, y, barW, 2, col.dim);
    }
    peaks[i] = Math.max(lit, peaks[i] - 0.25);
    if (peaks[i] > 0.5) rect(x, H - 2 - Math.ceil(peaks[i]) * 3 - 3, barW, 1, col.text);
  }
}

function scope() {
  player.analyser.getByteTimeDomainData(wave);
  // phosphor persistence via an intensity buffer
  for (let i = 0; i < glow.length; i++) glow[i] *= 0.55;
  let start = 0;
  for (let i = 1; i < wave.length / 2; i++) if (wave[i - 1] < 128 && wave[i] >= 128) { start = i; break; }
  const span = Math.min(wave.length - start, 1024);
  let prevY = -1;
  for (let x = 0; x < W; x++) {
    const v = wave[start + Math.floor((x / W) * span)] / 255;
    const y = Math.round((1 - v) * (H - 2)) + 1;
    if (prevY < 0) prevY = y;
    for (let yy = Math.min(prevY, y), e = Math.max(prevY, y); yy <= e; yy++) if (yy >= 0 && yy < H) glow[yy * W + x] = 1;
    prevY = y;
  }
  for (let i = 0; i < glow.length; i++) buf[i] = glow[i] > 0.04 ? heatLut[(glow[i] * 190) | 0] : col.bg;
  // graticule
  for (let x = 0; x < W; x += 16) for (let y = 0; y < H; y += 4) if (glow[y * W + x] < 0.1) buf[y * W + x] = col.dim;
  for (let y = 0; y < H; y += 16) for (let x = 0; x < W; x += 4) if (glow[y * W + x] < 0.1) buf[y * W + x] = col.dim;
}

function rms(a) {
  let s = 0;
  for (let i = 0; i < a.length; i++) s += a[i] * a[i];
  return Math.sqrt(s / a.length);
}

function drawVu() {
  player.analyserL.getFloatTimeDomainData(waveL);
  player.analyserR.getFloatTimeDomainData(waveR);
  // Scale tuned for modern masters: -30 dBFS RMS at rest, red zone from ~-8 dBFS.
  const needle = (r) => Math.max(0, Math.min(1, (20 * Math.log10(Math.max(r, 1e-5)) + 30) / 28));
  const k = 30 / fps; // same ballistics at any frame rate
  vu.l += (needle(rms(waveL)) - vu.l) * 0.18 * k;
  vu.r += (needle(rms(waveR)) - vu.r) * 0.18 * k;
  buf.fill(col.bg);
  const two = W > H * 1.6;
  const mw = two ? W / 2 : W;
  const mh = two ? H : H / 2;
  meter(0, 0, mw, mh, vu.l, 0);
  meter(two ? mw : 0, two ? 0 : mh, mw, mh, vu.r, 1);
}

function meter(x, y, w, hh, v, ch) {
  const cx = x + w / 2;
  const cy = y + hh * 0.92;
  const r = Math.min(w * 0.42, hh * 0.78);
  const a0 = Math.PI * 1.22;
  const a1 = Math.PI * 1.78;
  for (let i = 0; i <= 20; i++) {
    const t = i / 20;
    const a = a0 + (a1 - a0) * t;
    const len = i % 5 === 0 ? 5 : 2;
    for (let k = 0; k < len; k++) dot(cx + Math.cos(a) * (r - k), cy + Math.sin(a) * (r - k), t > 0.78 ? col.a : col.text);
  }
  for (let t = 0.78; t <= 1; t += 0.005) {
    const a = a0 + (a1 - a0) * t;
    dot(cx + Math.cos(a) * (r + 2), cy + Math.sin(a) * (r + 2), col.a);
    dot(cx + Math.cos(a) * (r + 3), cy + Math.sin(a) * (r + 3), col.a);
  }
  const a = a0 + (a1 - a0) * v;
  for (let k = 0; k < r - 2; k++) dot(cx + Math.cos(a) * k, cy + Math.sin(a) * k, col.b);
  rect(cx - 2, cy - 2, 5, 5, col.text);
  vu.hold[ch] = v > 0.85 ? 20 : Math.max(0, vu.hold[ch] - 1);
  rect(x + w - 10, y + 5, 4, 4, vu.hold[ch] ? col.a : col.dim);
  // channel letter in a 3x5 pixel font
  const L = ['100', '100', '100', '100', '111'];
  const R = ['110', '101', '110', '101', '101'];
  (ch ? R : L).forEach((row, j) => [...row].forEach((p, i) => p === '1' && dot(x + 5 + i, y + 5 + j, col.dim)));
}

function spectro() {
  player.analyser.getByteFrequencyData(freq);
  for (let y = 0; y < H; y++) buf.copyWithin(y * W, y * W + 1, y * W + W);
  const b = bands(H);
  for (let y = 0; y < H; y++) buf[y * W + W - 1] = heatLut[(Math.pow(bandValue(b, H - 1 - y), 1.4) * 255) | 0];
}

function toRgb(c) {
  c = String(c || '#000').trim();
  if (c.startsWith('rgb')) return c.match(/\d+/g).slice(0, 3).map(Number);
  if (c.length === 4) c = '#' + [...c.slice(1)].map((x) => x + x).join('');
  const n = parseInt(c.slice(1, 7), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

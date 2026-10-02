// Visualizer: LED spectrum bars, phosphor oscilloscope, twin analog VU meters, spectrogram.
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

let canvas, ctx, colors, mode;
let peaks = [];
let freq, wave, waveL, waveR;
const vu = { l: 0, r: 0, pl: 0, pr: 0, hold: [0, 0] };

export function initVisualizer() {
  canvas = document.getElementById('scope-canvas');
  canvas.style.imageRendering = 'pixelated';
  ctx = canvas.getContext('2d', { alpha: false });
  mode = player.opts.visualizer || 'bars';
  freq = new Uint8Array(player.analyser.frequencyBinCount);
  wave = new Uint8Array(player.analyser.fftSize);
  waveL = new Float32Array(player.analyserL.fftSize);
  waveR = new Float32Array(player.analyserR.fftSize);
  refreshColors();
  renderModeButtons();
  canvas.addEventListener('click', () => {
    const i = MODES.findIndex((m) => m[0] === mode);
    setMode(MODES[(i + 1) % MODES.length][0]);
  });
  requestAnimationFrame(loop);
}

export function setMode(m) {
  mode = m;
  player.setVisualizer(m);
  renderModeButtons();
  ctx.fillStyle = colors.bg;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
}

export function cycleMode() {
  const i = MODES.findIndex((m) => m[0] === mode);
  setMode(MODES[(i + 1) % MODES.length][0]);
}

function renderModeButtons() {
  const box = document.getElementById('scope-modes');
  box.replaceChildren(...MODES.map(([id, label]) => h('button' + (id === mode ? '.on' : ''), { on: { click: (e) => { e.stopPropagation(); setMode(id); } } }, label)));
}

export function refreshColors() {
  colors = {
    bg: cssVar('--lcd-bg') || '#100',
    a: cssVar('--accent'),
    b: cssVar('--accent-2'),
    text: cssVar('--text'),
    dim: cssVar('--faint'),
    line: cssVar('--line-hi'),
  };
  palette = null;
}

function loop() {
  requestAnimationFrame(loop);
  if (!canvas.offsetParent || document.hidden) return;
  const w = Math.max(1, Math.floor(canvas.clientWidth / PX));
  const hgt = Math.max(1, Math.floor(canvas.clientHeight / PX));
  if (canvas.width !== w || canvas.height !== hgt) {
    canvas.width = w;
    canvas.height = hgt;
    ctx.fillStyle = colors.bg;
    ctx.fillRect(0, 0, w, hgt);
  }
  if (mode === 'off') {
    ctx.fillStyle = colors.bg;
    ctx.fillRect(0, 0, w, hgt);
    return;
  }
  ({ bars, scope, vu: drawVu, spectro })[mode]?.(w, hgt);
}

// Log-spaced band edges for the spectrum.
function bandValue(i, n) {
  const nyq = player.ctx.sampleRate / 2;
  const lo = 30 * Math.pow(16000 / 30, i / n);
  const hi = 30 * Math.pow(16000 / 30, (i + 1) / n);
  const a = Math.floor((lo / nyq) * freq.length);
  const b = Math.max(a + 1, Math.floor((hi / nyq) * freq.length));
  let m = 0;
  for (let k = a; k < b && k < freq.length; k++) m = Math.max(m, freq[k]);
  return m / 255;
}

function bars(w, hgt) {
  player.analyser.getByteFrequencyData(freq);
  ctx.fillStyle = colors.bg;
  ctx.fillRect(0, 0, w, hgt);
  const barW = w > 300 ? 5 : 3;
  const gap = 1;
  const n = Math.max(8, Math.floor((w - 2) / (barW + gap)));
  const segH = 2;
  const segGap = 1;
  const segs = Math.floor((hgt - 4) / (segH + segGap));
  if (peaks.length !== n) peaks = new Array(n).fill(0);
  const x0 = Math.floor((w - n * (barW + gap)) / 2);
  for (let i = 0; i < n; i++) {
    const v = Math.pow(bandValue(i, n), 1.6);
    const lit = Math.round(v * segs);
    const x = x0 + i * (barW + gap);
    for (let s = 0; s < segs; s++) {
      const y = hgt - 2 - (s + 1) * (segH + segGap);
      const frac = s / segs;
      if (s < lit) ctx.fillStyle = frac > 0.8 ? colors.text : frac > 0.55 ? colors.b : colors.a;
      else ctx.fillStyle = colors.dim;
      if (s < lit || s % 2 === 0) {
        ctx.globalAlpha = s < lit ? 1 : 0.25;
        ctx.fillRect(x, y, barW, segH);
      }
    }
    ctx.globalAlpha = 1;
    peaks[i] = Math.max(lit, peaks[i] - 0.25);
    const py = hgt - 2 - Math.ceil(peaks[i]) * (segH + segGap) - segH - 1;
    if (peaks[i] > 0.5) {
      ctx.fillStyle = colors.text;
      ctx.fillRect(x, py, barW, 1);
    }
  }
}

function scope(w, hgt) {
  player.analyser.getByteTimeDomainData(wave);
  // phosphor persistence
  ctx.globalAlpha = 0.35;
  ctx.fillStyle = colors.bg;
  ctx.fillRect(0, 0, w, hgt);
  ctx.globalAlpha = 1;
  // graticule
  ctx.fillStyle = colors.dim;
  for (let x = 0; x < w; x += 16) for (let y = 0; y < hgt; y += 4) ctx.fillRect(x, y, 1, 1);
  for (let y = 0; y < hgt; y += 16) for (let x = 0; x < w; x += 4) ctx.fillRect(x, y, 1, 1);
  // find a rising zero crossing for a stable trace
  let start = 0;
  for (let i = 1; i < wave.length / 2; i++) if (wave[i - 1] < 128 && wave[i] >= 128) { start = i; break; }
  const span = Math.min(wave.length - start, 1024);
  ctx.fillStyle = colors.a;
  let prevY = null;
  for (let x = 0; x < w; x++) {
    const v = wave[start + Math.floor((x / w) * span)] / 255;
    const y = Math.round((1 - v) * (hgt - 2)) + 1;
    if (prevY == null) prevY = y;
    const y0 = Math.min(prevY, y);
    const y1 = Math.max(prevY, y);
    ctx.fillRect(x, y0, 1, y1 - y0 + 1);
    prevY = y;
  }
}

function rms(buf) {
  let s = 0;
  for (let i = 0; i < buf.length; i++) s += buf[i] * buf[i];
  return Math.sqrt(s / buf.length);
}

function drawVu(w, hgt) {
  player.analyserL.getFloatTimeDomainData(waveL);
  player.analyserR.getFloatTimeDomainData(waveR);
  const toNeedle = (r) => {
    // Scale tuned for modern masters: -30 dBFS RMS at rest, red zone from ~-8 dBFS.
    const db = 20 * Math.log10(Math.max(r, 1e-5));
    return Math.max(0, Math.min(1, (db + 30) / 28));
  };
  const tl = toNeedle(rms(waveL));
  const tr = toNeedle(rms(waveR));
  // VU ballistics (~300ms)
  vu.l += (tl - vu.l) * 0.18;
  vu.r += (tr - vu.r) * 0.18;
  ctx.fillStyle = colors.bg;
  ctx.fillRect(0, 0, w, hgt);
  const twoUp = w > hgt * 1.6;
  const mw = twoUp ? w / 2 : w;
  const mh = twoUp ? hgt : hgt / 2;
  meter(0, 0, mw, mh, vu.l, 'L', 0);
  meter(twoUp ? mw : 0, twoUp ? 0 : mh, mw, mh, vu.r, 'R', 1);
}

function meter(x, y, w, hgt, v, label, ch) {
  const cx = x + w / 2;
  const cy = y + hgt * 0.92;
  const r = Math.min(w * 0.42, hgt * 0.78);
  const a0 = Math.PI * 1.22;
  const a1 = Math.PI * 1.78;
  // scale
  for (let i = 0; i <= 20; i++) {
    const t = i / 20;
    const a = a0 + (a1 - a0) * t;
    const len = i % 5 === 0 ? 5 : 2;
    ctx.fillStyle = t > 0.78 ? colors.a : colors.text;
    for (let k = 0; k < len; k++) {
      ctx.fillRect(Math.round(cx + Math.cos(a) * (r - k)), Math.round(cy + Math.sin(a) * (r - k)), 1, 1);
    }
  }
  // red zone arc
  ctx.fillStyle = colors.a;
  for (let t = 0.78; t <= 1; t += 0.005) {
    const a = a0 + (a1 - a0) * t;
    ctx.fillRect(Math.round(cx + Math.cos(a) * (r + 2)), Math.round(cy + Math.sin(a) * (r + 2)), 1, 2);
  }
  // needle
  const a = a0 + (a1 - a0) * v;
  ctx.fillStyle = colors.b;
  for (let k = 0; k < r - 2; k++) ctx.fillRect(Math.round(cx + Math.cos(a) * k), Math.round(cy + Math.sin(a) * k), 1, 1);
  ctx.fillStyle = colors.text;
  ctx.fillRect(Math.round(cx) - 2, Math.round(cy) - 2, 5, 5);
  // peak LED
  vu.hold[ch] = v > 0.85 ? 20 : Math.max(0, vu.hold[ch] - 1);
  ctx.fillStyle = vu.hold[ch] ? colors.a : colors.dim;
  ctx.fillRect(Math.round(x + w - 10), Math.round(y + 5), 4, 4);
  ctx.fillStyle = colors.dim;
  ctx.font = '8px PressStart';
  ctx.fillText(label, x + 5, y + 11);
}

let palette;
function heat(v) {
  if (!palette) {
    palette = [];
    const stops = [colors.bg, colors.a, colors.b, colors.text].map(hexToRgb);
    for (let i = 0; i < 256; i++) {
      const t = (i / 255) * (stops.length - 1);
      const k = Math.min(stops.length - 2, Math.floor(t));
      const f = t - k;
      const c = stops[k].map((c0, j) => Math.round(c0 + (stops[k + 1][j] - c0) * f));
      palette.push(`rgb(${c[0]},${c[1]},${c[2]})`);
    }
  }
  return palette[v];
}

function spectro(w, hgt) {
  player.analyser.getByteFrequencyData(freq);
  ctx.drawImage(canvas, 1, 0, w - 1, hgt, 0, 0, w - 1, hgt);
  for (let y = 0; y < hgt; y++) {
    const v = bandValue(hgt - 1 - y, hgt);
    ctx.fillStyle = heat(Math.round(Math.pow(v, 1.4) * 255));
    ctx.fillRect(w - 1, y, 1, 1);
  }
}

function hexToRgb(c) {
  c = c.trim();
  if (c.startsWith('rgb')) return c.match(/\d+/g).slice(0, 3).map(Number);
  if (c.length === 4) c = '#' + [...c.slice(1)].map((x) => x + x).join('');
  const n = parseInt(c.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

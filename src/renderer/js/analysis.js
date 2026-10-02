// One decode per song, ever: produces the waveform peaks (seek bar) and a loudness measurement
// (volume matching). Peaks are cached on disk, loudness on the library track.
import { mediaUrl } from './util.js';
import { state } from './state.js';

const TARGET = -16; // dB, gated block RMS of the 8 kHz analysis signal (relative scale)
const N = 1200; // waveform resolution
const mem = new Map(); // id -> { peaks, gain }
const inflight = new Map(); // id -> Promise

export function analyze(t) {
  if (!t) return Promise.resolve(null);
  const hit = mem.get(t.id);
  if (hit && (hit.gain != null || t.gain != null)) return Promise.resolve({ ...hit, gain: hit.gain ?? t.gain });
  if (inflight.has(t.id)) return inflight.get(t.id);
  const p = run(t).finally(() => inflight.delete(t.id));
  inflight.set(t.id, p);
  return p;
}

export const cachedPeaks = (id) => mem.get(id)?.peaks || null;

async function run(t) {
  // Fast path: peaks on disk and loudness already known -> no decode at all.
  const saved = await dk.wave.get(t.id);
  if (saved?.length && t.gain != null) return remember(t.id, Float32Array.from(new Uint8Array(saved), (v) => v / 255), t.gain);

  const buf = await (await fetch(mediaUrl(t))).arrayBuffer();
  const audio = await new OfflineAudioContext(1, 1, 8000).decodeAudioData(buf);
  const chs = Math.min(2, audio.numberOfChannels);
  const data = [...Array(chs)].map((_, c) => audio.getChannelData(c));
  const len = audio.length;

  // waveform: RMS per bucket (shows dynamics even on brick-walled masters)
  const peaks = new Float32Array(N);
  const per = Math.max(1, Math.floor(len / N));
  for (const d of data) {
    for (let i = 0; i < N; i++) {
      let sum = 0;
      let n = 0;
      for (let k = i * per, e = Math.min(len, k + per); k < e; k += 2) { sum += d[k] * d[k]; n++; }
      peaks[i] = Math.max(peaks[i], Math.sqrt(sum / Math.max(1, n)));
    }
  }
  let max = 0;
  for (const v of peaks) max = Math.max(max, v);
  for (let i = 0; i < N; i++) peaks[i] = max ? peaks[i] / max : 0;

  // loudness: 400 ms blocks, absolute gate (-60 dB) + relative gate (-10 dB), like EBU R128
  const block = Math.floor(audio.sampleRate * 0.4);
  const ms = [];
  let peak = 0;
  for (let s = 0; s + block <= len; s += block) {
    let sum = 0;
    for (const d of data) for (let k = s; k < s + block; k++) { sum += d[k] * d[k]; const a = d[k] < 0 ? -d[k] : d[k]; if (a > peak) peak = a; }
    ms.push(sum / (block * chs));
  }
  const db = (x) => 10 * Math.log10(x || 1e-12);
  const abs = ms.filter((x) => db(x) > -60);
  const mean = (a) => a.reduce((s, x) => s + x, 0) / Math.max(1, a.length);
  const rel = abs.filter((x) => db(x) > db(mean(abs)) - 10);
  const loud = rel.length ? db(mean(rel)) : null;
  let gain = loud == null ? 0 : TARGET - loud;
  // never boost past the song's own peak (and stay within a sane range)
  gain = Math.min(gain, -20 * Math.log10(peak || 1) - 1, 6);
  gain = Math.round(Math.max(-12, gain) * 10) / 10;

  dk.wave.set(t.id, Uint8Array.from(peaks, (v) => Math.round(v * 255)));
  dk.library.setGain(t.id, gain);
  const lt = state.track(t.id);
  if (lt) lt.gain = gain;
  return remember(t.id, peaks, gain);
}

function remember(id, peaks, gain) {
  const r = { peaks, gain };
  mem.set(id, r);
  if (mem.size > 80) mem.delete(mem.keys().next().value);
  return r;
}

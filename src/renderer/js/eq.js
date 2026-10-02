// 10-band graphic EQ with live response curve, presets, crossfade and leveler options.
import { player, EQ_FREQS } from './player.js';
import { state } from './state.js';
import { $, h, setRangeFill, cssVar } from './util.js';

export const PRESETS = {
  Flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  'Bass Boost': [7, 6, 4.5, 2, 0, 0, 0, 0, 0, 0],
  'Sub Rumble': [9, 6, 2, 0, -1, 0, 0, 0, 0, 0],
  Treble: [0, 0, 0, 0, 0, 1, 2.5, 4.5, 6, 7],
  'V-Shape': [6, 4.5, 2, 0, -2, -2, 0, 2, 4.5, 6],
  Vocal: [-2, -2, -1, 1, 3.5, 4, 3, 1.5, 0, -1],
  Rock: [5, 3.5, 2, -1, -2, -1, 1.5, 3, 4, 4.5],
  Electronic: [5.5, 4.5, 1, 0, -2, 1.5, 0.5, 1, 4.5, 5],
  'Hip-Hop': [6, 5, 1.5, 3, -1, -1, 1, -0.5, 1.5, 3],
  Jazz: [3.5, 2.5, 1, 2, -1.5, -1.5, 0, 1, 2.5, 3.5],
  Classical: [4, 3, 2.5, 2, -1, -1, 0, 2, 3, 3.5],
  Acoustic: [4, 4, 3, 1, 2, 1.5, 3, 3.5, 3, 2],
  'Late Night': [-3, -2, 0, 1.5, 2, 2, 1.5, 0, -2, -3.5],
  Loudness: [6, 4, 0, 0, -1.5, 0, -1, -4, 4, 2],
};

const label = (f) => (f >= 1000 ? f / 1000 + 'K' : String(f));

export function initEq() {
  const eq = structuredClone(state.settings.eq);
  const sel = $('#eq-preset');
  const fillPresets = () => {
    const names = Object.keys(PRESETS);
    if (!names.includes(eq.preset)) names.push(eq.preset);
    sel.replaceChildren(...names.map((n) => h('option', { value: n, selected: n === eq.preset }, n)));
  };
  fillPresets();

  const apply = (save = true) => {
    player.applyEq(eq);
    $('.eq').classList.toggle('on', eq.enabled);
    drawCurve();
    if (save) state.set('eq', structuredClone(eq));
  };

  const sliders = [];
  const bands = $('#eq-bands');
  const mk = (lbl, val, onInput, pre) => {
    const input = h('input', { type: 'range', min: -12, max: 12, step: 0.5, value: val });
    const readout = h('b', fmt(val));
    input.oninput = () => { onInput(Number(input.value)); readout.textContent = fmt(input.value); setRangeFill(input); };
    input.ondblclick = () => { input.value = 0; input.oninput(); };
    setRangeFill(input);
    bands.append(h('div.band' + (pre ? '.pre' : ''), readout, input, h('label', lbl)));
    return { input, readout };
  };
  const preSl = mk('PRE', eq.preamp, (v) => { eq.preamp = v; apply(); }, true);
  EQ_FREQS.forEach((f, i) => sliders.push(mk(label(f), eq.gains[i], (v) => {
    eq.gains[i] = v;
    if (eq.preset !== 'Custom') { eq.preset = 'Custom'; fillPresets(); }
    if (!eq.enabled) { eq.enabled = true; $('#eq-on').checked = true; }
    apply();
  })));

  const sync = () => {
    sliders.forEach((s, i) => { s.input.value = eq.gains[i]; s.readout.textContent = fmt(eq.gains[i]); setRangeFill(s.input); });
    preSl.input.value = eq.preamp; preSl.readout.textContent = fmt(eq.preamp); setRangeFill(preSl.input);
  };

  sel.onchange = () => {
    if (!PRESETS[sel.value]) return;
    eq.preset = sel.value;
    eq.gains = [...PRESETS[sel.value]];
    // Auto pre-amp so boosted presets don't clip.
    eq.preamp = -Math.max(0, Math.max(...eq.gains) - 2);
    eq.enabled = true;
    $('#eq-on').checked = true;
    sync();
    apply();
  };
  $('#eq-on').checked = eq.enabled;
  $('#eq-on').onchange = (e) => { eq.enabled = e.target.checked; apply(); };
  $('#eq-reset').onclick = () => { sel.value = 'Flat'; sel.onchange(); };

  const xf = $('#opt-crossfade');
  xf.value = player.opts.crossfade || 0;
  setRangeFill(xf);
  $('#opt-crossfade-v').textContent = xf.value + 's';
  xf.oninput = () => { player.setCrossfade(Number(xf.value)); $('#opt-crossfade-v').textContent = xf.value + 's'; setRangeFill(xf); };
  const norm = $('#opt-normalize');
  norm.checked = !!player.opts.normalize;
  norm.onchange = () => player.setNormalize(norm.checked);

  new ResizeObserver(drawCurve).observe($('#eq-curve'));
  apply(false);
}

const fmt = (v) => (v > 0 ? '+' : '') + Number(v).toFixed(v % 1 ? 1 : 0);

export function drawCurve() {
  const c = $('#eq-curve');
  if (!c?.offsetParent) return;
  const W = Math.floor(c.clientWidth / 2);
  const H = Math.floor(c.clientHeight / 2);
  c.width = W;
  c.height = H;
  const g = c.getContext('2d');
  g.fillStyle = cssVar('--lcd-bg');
  g.fillRect(0, 0, W, H);
  g.fillStyle = cssVar('--faint');
  for (let x = 0; x < W; x += 3) g.fillRect(x, Math.floor(H / 2), 1, 1);
  const fr = new Float32Array(W);
  for (let i = 0; i < W; i++) fr[i] = 20 * Math.pow(20000 / 20, i / W);
  const total = new Float32Array(W);
  const mag = new Float32Array(W);
  const ph = new Float32Array(W);
  for (const f of player.filters) {
    f.getFrequencyResponse(fr, mag, ph);
    for (let i = 0; i < W; i++) total[i] += 20 * Math.log10(mag[i]);
  }
  const pre = player.eq?.enabled ? player.eq.preamp : 0;
  g.fillStyle = cssVar('--accent');
  let prev = null;
  for (let x = 0; x < W; x++) {
    const db = player.eq?.enabled ? total[x] + pre : 0;
    const y = Math.round(H / 2 - (db / 15) * (H / 2 - 2));
    const y0 = prev == null ? y : Math.min(prev, y);
    const y1 = prev == null ? y : Math.max(prev, y);
    g.fillRect(x, y0, 1, y1 - y0 + 1);
    prev = y;
  }
}

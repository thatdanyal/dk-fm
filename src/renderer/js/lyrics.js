// Synced lyrics from LRCLIB (free, no key). Click a line to jump there.
import { player } from './player.js';
import { $, h } from './util.js';

const cache = new Map();
let lines = null;
let activeIdx = -1;
let forId = null;

export function initLyrics() {
  player.on('track', load);
  player.on('time', tick);
  load(player.current);
}

async function load(t) {
  const box = $('#lyrics');
  lines = null;
  activeIdx = -1;
  forId = t?.id || null;
  box.className = 'lyrics';
  if (!t) return box.replaceChildren(h('div.empty', 'NO LYRICS'));
  box.replaceChildren(h('div.empty', 'SEARCHING…'));
  let res = cache.get(t.id);
  if (res === undefined) {
    res = await dk.lyrics({ artist: t.artist.split(/,| feat\.?| & /i)[0].trim(), title: t.title.replace(/\s*[([]\s*(feat|ft|with)[^)\]]*[)\]]/i, ''), album: t.album, duration: t.duration }).catch(() => null);
    cache.set(t.id, res);
  }
  if (forId !== t.id) return;
  if (res?.instrumental) return box.replaceChildren(h('div.empty', '♪ INSTRUMENTAL ♪'));
  if (res?.syncedLyrics) {
    lines = parseLrc(res.syncedLyrics);
    box.replaceChildren(...lines.map((l, i) => h('div.ln', { on: { click: () => player.seek(l.t) }, 'data-i': i }, l.text || '♪')));
    box.scrollTop = 0;
    tick();
  } else if (res?.plainLyrics) {
    box.className = 'lyrics plain';
    box.textContent = res.plainLyrics;
  } else {
    box.replaceChildren(h('div.empty', 'NO LYRICS FOUND'));
  }
}

function parseLrc(src) {
  const out = [];
  for (const raw of src.split('\n')) {
    const stamps = [...raw.matchAll(/\[(\d+):(\d+(?:\.\d+)?)\]/g)];
    const text = raw.replace(/\[[^\]]*\]/g, '').trim();
    for (const s of stamps) out.push({ t: Number(s[1]) * 60 + Number(s[2]), text });
  }
  return out.sort((a, b) => a.t - b.t);
}

function tick() {
  if (!lines) return;
  const now = player.time + 0.25;
  let i = -1;
  for (let k = 0; k < lines.length; k++) if (lines[k].t <= now) i = k; else break;
  if (i === activeIdx) return;
  activeIdx = i;
  const box = $('#lyrics');
  box.querySelectorAll('.ln').forEach((el, k) => {
    el.classList.toggle('active', k === i);
    el.classList.toggle('past', k < i);
  });
  const el = box.querySelector('.ln.active');
  if (el && box.offsetParent) box.scrollTo({ top: el.offsetTop - box.clientHeight / 2 + el.clientHeight / 2 });
}

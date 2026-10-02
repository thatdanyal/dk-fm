// Stats ("Wrapped" whenever you want): built from the local listening history.
// History rows: [trackId, startedAt (unix s), listened (s), skipped (0/1)].
import { player } from './player.js';
import { state } from './state.js';
import { h, thumbUrl, toast } from './util.js';

const PERIODS = [
  ['week', 'THIS WEEK', 7],
  ['month', 'THIS MONTH', 30],
  ['year', 'THIS YEAR', 365],
  ['all', 'ALL TIME', Infinity],
];
let period = 'month';
let cache = null; // { at, rows }

export async function statsView(box) {
  if (!cache || Date.now() - cache.at > 15000) {
    box.replaceChildren(h('div.empty', 'CRUNCHING NUMBERS…'));
    cache = { at: Date.now(), rows: await dk.history.get() };
  }
  render(box, cache.rows);
}

const dayKey = (sec) => {
  const d = new Date(sec * 1000);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
};
const mainArtist = (t) => (t?.artist || 'Unknown').split(/,|;| feat\.? | & /i)[0].trim();

function render(box, all) {
  const days = PERIODS.find((p) => p[0] === period)[2];
  const now = Date.now() / 1000;
  const from = days === Infinity ? 0 : now - days * 86400;
  const rows = all.filter((r) => r[1] >= from && state.track(r[0]));

  const tabs = h('div.view-actions', PERIODS.map(([id, label]) =>
    h('button.btn' + (id === period ? '.primary' : ''), { on: { click: () => { period = id; render(box, all); } } }, label)));
  const head = h('div.view-head', h('div.vh-text', h('div.view-title.glow-text', 'YOUR STATS'), h('div.view-sub', 'UPDATED LIVE · STORED ONLY ON THIS PC')), tabs);

  if (!rows.length) {
    box.replaceChildren(head, h('div.empty', h('span.px', 'NO LISTENING YET'), 'Play some music — your stats build up as you listen.'));
    return;
  }

  const secs = rows.reduce((s, r) => s + r[2], 0);
  const plays = rows.filter((r) => !r[3]);
  const songs = new Map();
  const artists = new Map();
  const skips = new Map();
  for (const r of rows) {
    const t = state.track(r[0]);
    const s = songs.get(r[0]) || { id: r[0], plays: 0, secs: 0 };
    if (!r[3]) s.plays++;
    s.secs += r[2];
    songs.set(r[0], s);
    const a = mainArtist(t);
    const ar = artists.get(a) || { name: a, secs: 0, plays: 0, cover: null };
    ar.secs += r[2];
    if (!r[3]) ar.plays++;
    ar.cover ||= t.cover ? t : null;
    artists.set(a, ar);
    if (r[3]) skips.set(r[0], (skips.get(r[0]) || 0) + 1);
  }
  const topSongs = [...songs.values()].sort((a, b) => b.plays - a.plays || b.secs - a.secs).slice(0, 10);
  const topArtists = [...artists.values()].sort((a, b) => b.secs - a.secs).slice(0, 8);
  const mostSkipped = [...skips.entries()].sort((a, b) => b[1] - a[1]).slice(0, 5);

  // streak: consecutive days (ending today or yesterday) with any listening, over all history
  const dayset = new Set(all.map((r) => dayKey(r[1])));
  let streak = 0;
  const d = new Date();
  if (!dayset.has(dayKey(d.getTime() / 1000))) d.setDate(d.getDate() - 1);
  while (dayset.has(dayKey(d.getTime() / 1000))) { streak++; d.setDate(d.getDate() - 1); }

  const hours = new Array(24).fill(0);
  for (const r of rows) hours[new Date(r[1] * 1000).getHours()] += r[2];
  const peak = hours.indexOf(Math.max(...hours));

  const card = (big, small) => h('div.stat-card', h('div.stat-big', big), h('div.stat-small', small));
  const fmtH = (hr) => `${hr % 12 || 12}${hr < 12 ? 'AM' : 'PM'}`;
  const summary = h('div.stat-cards',
    card(Math.round(secs / 60).toLocaleString(), 'MINUTES LISTENED'),
    card(plays.length.toLocaleString(), 'PLAYS'),
    card(songs.size.toLocaleString(), 'DIFFERENT SONGS'),
    card(artists.size.toLocaleString(), 'ARTISTS'),
    card(`${streak} DAY${streak === 1 ? '' : 'S'}`, 'LISTENING STREAK'),
    card(fmtH(peak), 'YOUR PEAK HOUR'));

  const songList = h('div.stat-list', topSongs.map((s, i) => {
    const t = state.track(s.id);
    return h('div.stat-row', { on: { dblclick: () => player.playList(topSongs.map((x) => x.id), i) }, title: 'Double-click to play' },
      h('span.stat-rank', String(i + 1)),
      t.cover ? h('img', { src: thumbUrl(t), loading: 'lazy' }) : h('span.stat-ph'),
      h('span.stat-name', h('div', t.title), h('div.dim', t.artist)),
      h('span.stat-num', s.plays ? `${s.plays} PLAY${s.plays === 1 ? '' : 'S'}` : `${Math.max(1, Math.round(s.secs / 60))} MIN`));
  }));
  const maxA = topArtists[0]?.secs || 1;
  const artistList = h('div.stat-list', topArtists.map((a, i) =>
    h('div.stat-row',
      h('span.stat-rank', String(i + 1)),
      a.cover ? h('img', { src: thumbUrl(a.cover), loading: 'lazy' }) : h('span.stat-ph'),
      h('span.stat-name', h('div', a.name), h('div.stat-bar', h('i', { style: { width: (a.secs / maxA) * 100 + '%' } }))),
      h('span.stat-num', `${Math.round(a.secs / 60)} MIN`))));

  const maxH = Math.max(...hours, 1);
  const hourChart = h('div.stat-hours', hours.map((v, hr) =>
    h('div.stat-hour', { title: `${fmtH(hr)}: ${Math.round(v / 60)} min` }, h('i', { style: { height: Math.max(2, (v / maxH) * 100) + '%' } }), hr % 6 === 0 ? h('span', fmtH(hr)) : null)));

  // activity: per day (week/month) or per month (year/all)
  const byMonth = days > 31;
  const buckets = new Map();
  for (const r of rows) {
    const dt = new Date(r[1] * 1000);
    const k = byMonth ? `${dt.getFullYear()}-${String(dt.getMonth() + 1).padStart(2, '0')}` : `${dt.getMonth() + 1}/${dt.getDate()}`;
    buckets.set(k, (buckets.get(k) || 0) + r[2]);
  }
  const keys = [];
  const cur = new Date();
  for (let i = (byMonth ? 11 : Math.min(days, 30) - 1); i >= 0; i--) {
    const dt = new Date(cur);
    if (byMonth) dt.setMonth(cur.getMonth() - i, 1);
    else dt.setDate(cur.getDate() - i);
    keys.push(byMonth ? `${dt.getFullYear()}-${String(dt.getMonth() + 1).padStart(2, '0')}` : `${dt.getMonth() + 1}/${dt.getDate()}`);
  }
  const maxB = Math.max(...keys.map((k) => buckets.get(k) || 0), 1);
  const activity = h('div.stat-hours', keys.map((k, i) =>
    h('div.stat-hour', { title: `${k}: ${Math.round((buckets.get(k) || 0) / 60)} min` },
      h('i', { style: { height: Math.max(2, ((buckets.get(k) || 0) / maxB) * 100) + '%' } }),
      i === 0 || i === keys.length - 1 ? h('span', byMonth ? k.slice(2) : k) : null)));

  const skipList = mostSkipped.length ? h('div.stat-list', mostSkipped.map(([id, n]) => {
    const t = state.track(id);
    return h('div.stat-row', h('span.stat-rank', '↷'), h('span.stat-name', h('div', t.title), h('div.dim', t.artist)), h('span.stat-num', `${n} SKIP${n === 1 ? '' : 'S'}`));
  })) : h('div.dim', { style: { padding: '8px' } }, 'Nothing skipped — good taste.');

  const section = (title, ...kids) => h('div.stat-section', h('div.stat-title', title), ...kids);
  box.replaceChildren(head, h('div.stats',
    summary,
    h('div.stat-cols',
      section('TOP SONGS', songList, h('button.mini-btn', { style: { marginTop: '8px' }, on: { click: () => { player.playList(topSongs.map((s) => s.id), 0, { shuffle: false }); toast('Playing your top songs'); } } }, '▶ PLAY TOP SONGS')),
      section('TOP ARTISTS', artistList)),
    h('div.stat-cols',
      section(byMonth ? 'MINUTES PER MONTH' : 'MINUTES PER DAY', activity),
      section('WHEN YOU LISTEN', hourChart)),
    section('MOST SKIPPED (SMART SHUFFLE PLAYS THESE LATER)', skipList)));
}

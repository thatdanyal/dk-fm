// Ctrl+K command palette: one box for songs, albums, artists, playlists, actions and settings.
// Enter = do it · Shift+Enter = add song to queue · Alt+Enter = play song next · paste a link = import.
import { player } from './player.js';
import { state } from './state.js';
import { h, thumbUrl, toast } from './util.js';
import { THEMES, applyTheme } from './theme.js';
import { setView, importLink, radioFor } from './browser.js';
import { showPanel, setEditing, isEditing } from './layout.js';
import { cycleMode } from './visualizer.js';
import { openSettings } from './settings.js';

let open = false;

export function initPalette({ toggleMini }) {
  addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      open ? close() : show(toggleMini);
    }
  });
}

let close = () => {};

function commands(toggleMini) {
  const liked = () => [...state.tracks.values()].filter((t) => state.stat(t.id).liked).map((t) => t.id);
  const all = () => [...state.tracks.keys()];
  const go = (key) => () => { showPanel('browser'); setView({ key }); };
  const cur = player.current;
  return [
    { label: player.playing ? 'Pause' : 'Play', kw: 'play pause resume', icon: player.playing ? '❚❚' : '▶', run: () => player.toggle() },
    { label: 'Next song', kw: 'skip forward', icon: '▶▶', run: () => player.next(true) },
    { label: 'Previous song', kw: 'back', icon: '◀◀', run: () => player.prev() },
    { label: 'Shuffle all songs', kw: 'random everything', icon: '⤮', run: () => player.playList(all(), Math.floor(Math.random() * state.tracks.size), { shuffle: true }) },
    { label: 'Shuffle liked songs', kw: 'favorites hearts random', icon: '♥', run: () => { const l = liked(); l.length ? player.playList(l, 0, { shuffle: true }) : toast('No liked songs yet'); } },
    cur && { label: `More like "${cur.title}"`, kw: 'radio similar discover recommend', icon: '✦', run: () => { showPanel('browser'); radioFor(cur.id); } },
    cur && { label: state.stat(cur.id).liked ? `Unlike "${cur.title}"` : `Like "${cur.title}"`, kw: 'heart favorite', icon: '♥', run: () => state.like(cur.id) },
    { label: `Shuffle: ${player.opts.shuffle ? 'on → off' : 'off → on'}`, kw: 'toggle', icon: '⤮', run: () => player.toggleShuffle() },
    { label: `Repeat: ${player.opts.repeat || 'off'} → next mode`, kw: 'loop toggle', icon: '↻', run: () => player.cycleRepeat() },
    { label: 'Import music from a link', kw: 'spotify youtube soundcloud download add', icon: '⤓', run: go('import') },
    { label: 'Sync all imported playlists now', kw: 'update refresh spotify', icon: '⟳', run: async () => { toast('Syncing playlists…'); const r = await dk.sync.now(); toast(r.busy ? 'A sync is already running' : `${r.added || 0} new songs`); } },
    { label: 'Downloads', kw: 'progress queue', icon: '⇣', run: go('downloads') },
    { label: 'Stats', kw: 'wrapped top songs artists minutes history', icon: '▣', run: go('stats') },
    { label: 'Liked songs', kw: 'favorites', icon: '♥', run: go('liked') },
    { label: 'All songs', kw: 'library tracks', icon: '♫', run: go('all') },
    { label: 'Albums', kw: '', icon: '◉', run: go('albums') },
    { label: 'Artists', kw: '', icon: '☻', run: go('artists') },
    { label: 'Mini player', kw: 'small compact window', icon: '▭', run: toggleMini },
    { label: isEditing() ? 'Finish editing layout' : 'Edit layout', kw: 'move panels arrange', icon: '▦', run: () => setEditing(!isEditing()) },
    { label: 'Cycle visualizer', kw: 'scope bars vu waterfall', icon: '▥', run: cycleMode },
    { label: 'Rescan library', kw: 'refresh folders', icon: '⟳', run: () => { dk.library.scan(); toast('Scanning library…'); } },
    { label: 'Settings', kw: 'preferences options', icon: '⚙', run: () => openSettings() },
    ...Object.entries(THEMES).map(([id, t]) => ({ label: `Theme: ${t.name}`, kw: 'color look appearance', icon: '■', color: t.swatch[0], run: () => { state.set('theme', id); applyTheme(); } })),
  ].filter(Boolean);
}

const norm = (s) => String(s || '').toLowerCase();

// All query words must appear; earlier / word-start matches rank higher.
function score(hay, toks) {
  let sc = 0;
  for (const t of toks) {
    const i = hay.indexOf(t);
    if (i < 0) return -1;
    sc += i === 0 ? 30 : hay[i - 1] === ' ' ? 15 : 5;
    sc -= i * 0.02;
  }
  return sc;
}

function search(q, toggleMini) {
  const query = q.trim();
  const out = [];
  if (/^https?:\/\//i.test(query) && /spotify|youtu|soundcloud/i.test(query)) {
    out.push({ group: 'IMPORT', label: 'Import this link', sub: query, icon: '⤓', run: () => { showPanel('browser'); importLink(query); } });
  }
  const toks = norm(query).split(/\s+/).filter(Boolean);
  const cmds = commands(toggleMini);
  if (!toks.length) return [...out, ...cmds.slice(0, 12).map((c) => ({ ...c, group: 'ACTIONS' }))];

  const songs = [];
  for (const t of state.tracks.values()) {
    const sc = score(norm(`${t.title} ${t.artist} ${t.album}`), toks);
    if (sc >= 0) songs.push([sc + (state.stat(t.id).plays || 0) * 0.5, t]);
  }
  songs.sort((a, b) => b[0] - a[0]);

  const albums = new Map();
  const artists = new Map();
  for (const t of state.tracks.values()) {
    if (t.album) {
      const k = `${norm(t.albumArtist || t.artist)}|${norm(t.album)}`;
      if (!albums.has(k) && score(norm(`${t.album} ${t.albumArtist || t.artist}`), toks) >= 0) albums.set(k, t);
    }
    const a = (t.artist || '').split(/,|;| feat\.? | & /i)[0].trim();
    if (a && !artists.has(norm(a)) && score(norm(a), toks) >= 0) artists.set(norm(a), { name: a, t });
  }
  const pls = state.playlists.filter((p) => score(norm(p.name), toks) >= 0);
  const acts = cmds.map((c) => [score(norm(`${c.label} ${c.kw}`), toks), c]).filter((x) => x[0] >= 0).sort((a, b) => b[0] - a[0]);

  out.push(...acts.slice(0, 4).map(([, c]) => ({ ...c, group: 'ACTIONS' })));
  out.push(...songs.slice(0, 8).map(([, t], i) => ({
    group: 'SONGS', label: t.title, sub: t.artist, img: t.cover ? thumbUrl(t) : null, icon: '♫', track: t.id,
    run: () => player.playList(songs.map((x) => x[1].id), i, { shuffle: false }),
  })));
  out.push(...[...albums.values()].slice(0, 4).map((t) => ({
    group: 'ALBUMS', label: t.album, sub: t.albumArtist || t.artist, img: t.cover ? thumbUrl(t) : null, icon: '◉',
    run: () => { showPanel('browser'); setView({ key: 'album', id: `${norm(t.albumArtist || t.artist)}|${norm(t.album)}` }); },
  })));
  out.push(...[...artists.values()].slice(0, 4).map(({ name, t }) => ({
    group: 'ARTISTS', label: name, img: t.cover ? thumbUrl(t) : null, icon: '☻',
    run: () => { showPanel('browser'); setView({ key: 'artist', id: norm(name) }); },
  })));
  out.push(...pls.slice(0, 4).map((p) => ({
    group: 'PLAYLISTS', label: p.name, sub: `${p.trackIds.length} songs`, icon: '▤',
    run: () => { showPanel('browser'); setView({ key: 'playlist', id: p.id }); },
  })));
  return out;
}

function show(toggleMini) {
  open = true;
  let items = [];
  let active = 0;
  const input = h('input', { placeholder: 'Search songs, albums, artists, playlists, actions — or paste a link', spellcheck: false });
  const list = h('div.palette-list');
  const back = h('div.palette-back', { on: { mousedown: (e) => e.target === back && close() } },
    h('div.palette', input, list,
      h('div.palette-foot', h('span', '↑↓ move'), h('span', 'ENTER do it'), h('span', 'SHIFT+ENTER add to queue'), h('span', 'ALT+ENTER play next'), h('span', 'ESC close'))));
  document.body.append(back);

  const render = () => {
    let group = null;
    const nodes = [];
    items.forEach((it, i) => {
      if (it.group !== group) { group = it.group; nodes.push(h('div.pal-group', group)); }
      nodes.push(h('div.pal-item' + (i === active ? '.active' : ''), { on: { mousemove: () => { if (active !== i) { active = i; render(); } }, click: () => run(i) } },
        h('span.ico', it.img ? h('img', { src: it.img }) : it.color ? h('span', { style: { width: '14px', height: '14px', background: it.color, display: 'block' } }) : it.icon || '·'),
        h('span.txt', it.label, it.sub ? h('span.sub', it.sub) : null),
        it.track ? h('span.kbd', '↵ play') : null));
    });
    if (!items.length) nodes.push(h('div.empty', 'NO MATCHES'));
    list.replaceChildren(...nodes);
    list.querySelector('.active')?.scrollIntoView({ block: 'nearest' });
  };
  const update = () => { items = search(input.value, toggleMini); active = 0; render(); };
  const run = (i, mode) => {
    const it = items[i];
    if (!it) return;
    close();
    if (it.track && mode === 'queue') return player.enqueue([it.track]);
    if (it.track && mode === 'next') return player.playNext([it.track]);
    it.run();
  };
  input.addEventListener('input', update);
  input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); active = Math.min(items.length - 1, active + 1); render(); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); active = Math.max(0, active - 1); render(); }
    else if (e.key === 'Enter') { e.preventDefault(); run(active, e.shiftKey ? 'queue' : e.altKey ? 'next' : null); }
    else if (e.key === 'Escape') { e.preventDefault(); close(); }
  });
  close = () => { open = false; back.remove(); };
  update();
  input.focus();
}

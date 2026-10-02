// Library browser: sources sidebar + views (tracks, albums, artists, playlists, Spotify import, downloads).
import { player } from './player.js';
import { state } from './state.js';
import { $, h, fmtTime, fmtLong, coverUrl, thumbUrl, contextMenu, prompt, toast, debounce } from './util.js';

const ROW = 28;
let view = { key: 'all' };
let search = '';
let sort = null; // { key, dir }
let selection = new Set();
let anchor = null;
let listIds = [];
let renderRows = null;
let spotifyResult = null;
let spotifyBusy = false;
const jobs = new Map();
let openSettings = () => {};

export function initBrowser({ onOpenSettings }) {
  openSettings = onOpenSettings;
  state.on('library', () => { renderSources(); rerenderView(); });
  state.on('stats', () => renderRows?.());
  player.on('track', () => renderRows?.());
  dk.dl.list().then((list) => { list.forEach((j) => jobs.set(j.id, j)); renderSources(); });
  dk.on('dl:job', (j) => { jobs.set(j.id, j); renderSources(); if (view.key === 'downloads') rerenderView(); });
  dk.on('dl:removed', (id) => { jobs.delete(id); renderSources(); if (view.key === 'downloads') rerenderView(); });
  dk.on('dl:track', ({ jobId, track }) => {
    const j = jobs.get(jobId);
    if (!j) return;
    j.tracks[track.idx] = track;
    scheduleJobRender(jobId);
  });
  dk.on('yt:status', () => view.key === 'downloads' && renderEngine());
  renderSources();
  setView({ key: state.tracks.size ? 'all' : 'all' });
}

export function setView(v) {
  view = v;
  search = '';
  sort = null;
  selection.clear();
  anchor = null;
  renderSources();
  rerenderView();
}

export function focusSearch() {
  const s = $('#view .search');
  if (s) s.focus();
}

// ---------------- sidebar ----------------
function renderSources() {
  const box = $('#sources');
  const all = [...state.tracks.values()];
  const liked = all.filter((t) => state.stat(t.id).liked).length;
  const active = [...jobs.values()].reduce((n, j) => n + j.tracks.filter((t) => ['queued', 'searching', 'downloading', 'tagging'].includes(t.status)).length, 0);
  const src = (key, ico, label, cnt, extra = {}) =>
    h('div.src' + (view.key === key && (!extra.id || extra.id === view.id) ? '.active' : ''), {
      on: {
        click: () => setView({ key, ...extra }),
        ...(extra.dropPlaylist ? playlistDrop(extra.id) : {}),
        ...(extra.ctx ? { contextmenu: extra.ctx } : {}),
      },
      title: label,
    }, h('span.ico', ico), h('span.lbl', label), cnt != null ? h('span.cnt', String(cnt)) : null);

  const userPls = state.playlists.filter((p) => p.source !== 'spotify');
  const spPls = state.playlists.filter((p) => p.source === 'spotify');
  box.replaceChildren(
    h('div.src-group', 'LOCAL'),
    src('all', '♫', 'All Tracks', all.length),
    src('liked', '♥', 'Liked', liked),
    src('top', '★', 'Most Played'),
    src('recent', '◷', 'Recently Added'),
    src('albums', '◉', 'Albums'),
    src('artists', '☻', 'Artists'),
    h('div.src.dim', { title: 'Scan another folder for music', on: { click: addFolder } }, h('span.ico', '+'), h('span.lbl', 'Add music folder')),
    h('div.src-group', 'SPOTIFY'),
    src('import', '⤓', 'Import Playlist'),
    src('downloads', '⇣', 'Downloads', active || null),
    ...spPls.map((p) => src('playlist', '▤', p.name, p.trackIds.length, { id: p.id, ctx: (e) => playlistMenu(e, p) })),
    h('div.src-group', 'PLAYLISTS', h('button', { title: 'New playlist', on: { click: newPlaylist } }, '+')),
    ...userPls.map((p) => src('playlist', '▤', p.name, p.trackIds.length, { id: p.id, dropPlaylist: true, ctx: (e) => playlistMenu(e, p) })),
    userPls.length ? null : h('div.src.dim', { on: { click: newPlaylist } }, h('span.ico', '+'), h('span.lbl', 'New playlist')),
  );
}

function playlistDrop(id) {
  return {
    dragover: (e) => { if (e.dataTransfer.types.includes('text/x-dkfm-tracks')) { e.preventDefault(); e.currentTarget.classList.add('drop-target'); } },
    dragleave: (e) => e.currentTarget.classList.remove('drop-target'),
    drop: (e) => {
      e.preventDefault();
      e.currentTarget.classList.remove('drop-target');
      const ids = JSON.parse(e.dataTransfer.getData('text/x-dkfm-tracks') || '[]');
      addToPlaylist(id, ids);
    },
  };
}

async function addFolder() {
  const f = await dk.dialog.folder();
  if (!f) return;
  const folders = state.settings.musicFolders || [];
  if (!folders.includes(f)) state.set('musicFolders', [...folders, f]);
  toast('Scanning ' + f + '…');
  await new Promise((r) => setTimeout(r, 400)); // let the settings save land first
  await dk.library.scan();
  toast(`Library: ${state.tracks.size} tracks`);
}

async function newPlaylist(ids = []) {
  const name = await prompt('NEW PLAYLIST', 'My Playlist');
  if (!name) return null;
  const pl = await dk.library.upsertPlaylist({ name, trackIds: Array.isArray(ids) ? ids : [], source: 'user' });
  return pl;
}

async function addToPlaylist(id, ids) {
  const pl = state.playlists.find((p) => p.id === id);
  if (!pl) return;
  const fresh = ids.filter((x) => !pl.trackIds.includes(x));
  await dk.library.upsertPlaylist({ id, trackIds: [...pl.trackIds, ...fresh] });
  toast(`Added ${fresh.length} track${fresh.length === 1 ? '' : 's'} to "${pl.name}"`);
}

function playlistMenu(e, p) {
  e.preventDefault();
  contextMenu(e.clientX, e.clientY, [
    { label: 'Play', action: () => player.playList(p.trackIds.filter((id) => state.track(id)), 0, { shuffle: false }) },
    { label: 'Shuffle play', action: () => player.playList(p.trackIds.filter((id) => state.track(id)), 0, { shuffle: true }) },
    { label: 'Add to queue', action: () => player.enqueue(p.trackIds.filter((id) => state.track(id))) },
    '-',
    { label: 'Rename…', action: async () => { const n = await prompt('RENAME PLAYLIST', p.name); if (n) dk.library.upsertPlaylist({ id: p.id, name: n }); } },
    p.spotifyUrl ? { label: 'Sync with Spotify', action: () => syncSpotify(p) } : null,
    p.spotifyUrl ? { label: 'Open in Spotify', action: () => dk.shell.openExternal(p.spotifyUrl) } : null,
    { label: 'Delete playlist', action: () => { if (confirm(`Delete playlist "${p.name}"? (Files are kept.)`)) { dk.library.deletePlaylist(p.id); if (view.id === p.id) setView({ key: 'all' }); } } },
  ].filter(Boolean));
}

async function syncSpotify(p) {
  toast(`Syncing "${p.name}"…`);
  try {
    const col = await dk.spotify.fetch(p.spotifyUrl);
    await dk.dl.start(col);
    setView({ key: 'downloads' });
  } catch (err) {
    toast(err.message, { error: true });
  }
}

// ---------------- views ----------------
function rerenderView() {
  const box = $('#view');
  const keepScroll = box.querySelector('.tracks, .grid, .jobs')?.scrollTop || 0;
  renderRows = null;
  const all = [...state.tracks.values()];
  switch (view.key) {
    case 'all':
      return trackView(box, { title: 'ALL TRACKS', ids: all.map((t) => t.id), defaultSort: { key: 'artist', dir: 1 } }, keepScroll);
    case 'liked':
      return trackView(box, { title: 'LIKED', ids: all.filter((t) => state.stat(t.id).liked).map((t) => t.id), defaultSort: { key: 'added', dir: -1 } }, keepScroll);
    case 'top':
      return trackView(box, { title: 'MOST PLAYED', ids: all.filter((t) => state.stat(t.id).plays > 0).map((t) => t.id), defaultSort: { key: 'plays', dir: -1 } }, keepScroll);
    case 'recent':
      return trackView(box, { title: 'RECENTLY ADDED', ids: all.filter((t) => Date.now() - (t.addedAt || 0) < 1000 * 3600 * 24 * 60).map((t) => t.id), defaultSort: { key: 'added', dir: -1 } }, keepScroll);
    case 'albums':
      return albumsView(box, all);
    case 'artists':
      return artistsView(box, all);
    case 'album': {
      const ids = all.filter((t) => albumKey(t) === view.id).map((t) => t.id);
      const first = state.track(ids[0]);
      return trackView(box, { title: first?.album || 'ALBUM', sub: first ? albumArtist(first) : '', cover: first, ids, defaultSort: { key: 'trackno', dir: 1 }, back: { key: 'albums' } }, keepScroll);
    }
    case 'artist': {
      const ids = all.filter((t) => artistKey(t) === view.id).map((t) => t.id);
      return trackView(box, { title: state.track(ids[0])?.artist || view.id, ids, defaultSort: { key: 'album', dir: 1 }, back: { key: 'artists' } }, keepScroll);
    }
    case 'playlist': {
      const p = state.playlists.find((x) => x.id === view.id);
      if (!p) return setView({ key: 'all' });
      const ids = p.trackIds.filter((id) => state.track(id));
      return trackView(box, { title: p.name, sub: p.source === 'spotify' ? 'SPOTIFY IMPORT' : 'PLAYLIST', coverUrl: p.cover, ids, playlist: p }, keepScroll);
    }
    case 'import':
      return importView(box);
    case 'downloads':
      return downloadsView(box);
  }
}

const albumKey = (t) => `${(t.albumArtist || t.artist || '').toLowerCase()}|${(t.album || '').toLowerCase()}`;
const albumArtist = (t) => t.albumArtist || t.artist;
const artistKey = (t) => (t.artist || 'Unknown').split(/,|;| feat\.? | & /i)[0].trim().toLowerCase();

function sortIds(ids, s) {
  if (!s) return ids;
  const get = {
    title: (t) => t.title.toLowerCase(),
    artist: (t) => `${t.artist.toLowerCase()}\u0000${(t.album || '').toLowerCase()}\u0000${String(t.track || 0).padStart(3, '0')}`,
    album: (t) => `${(t.album || '').toLowerCase()}\u0000${String(t.track || 0).padStart(3, '0')}`,
    trackno: (t) => String(t.track || 999).padStart(4, '0') + t.title.toLowerCase(),
    duration: (t) => t.duration || 0,
    plays: (t) => state.stat(t.id).plays,
    added: (t) => t.addedAt || 0,
  }[s.key];
  if (!get) return ids;
  return [...ids].sort((a, b) => {
    const x = get(state.track(a));
    const y = get(state.track(b));
    return (x < y ? -1 : x > y ? 1 : 0) * s.dir;
  });
}

function filterIds(ids) {
  if (!search) return ids;
  const toks = search.toLowerCase().split(/\s+/).filter(Boolean);
  return ids.filter((id) => {
    const t = state.track(id);
    const hay = `${t.title} ${t.artist} ${t.album} ${t.genre}`.toLowerCase();
    return toks.every((k) => hay.includes(k));
  });
}

function trackView(box, cfg, keepScroll = 0) {
  const activeSort = sort || cfg.defaultSort || null;
  listIds = sortIds(filterIds(cfg.ids), activeSort);
  const secs = listIds.reduce((s, id) => s + (state.track(id)?.duration || 0), 0);
  const searchInput = h('input.pixel-input.search', { placeholder: 'SEARCH…', value: search });
  searchInput.addEventListener('input', debounce(() => { search = searchInput.value; selection.clear(); rerenderView(); setTimeout(() => { const s = $('#view .search'); s?.focus(); s?.setSelectionRange(s.value.length, s.value.length); }); }, 160));

  const coverSrc = cfg.cover ? coverUrl(cfg.cover) : cfg.coverUrl || '';
  const head = h('div.view-head',
    cfg.back ? h('button.mini-btn', { on: { click: () => setView(cfg.back) } }, '◂ BACK') : null,
    coverSrc ? h('img.vh-cover', { src: coverSrc }) : null,
    h('div.vh-text', h('div.view-title.glow-text', cfg.title), h('div.view-sub', [cfg.sub, `${listIds.length} TRACKS`, fmtLong(secs)].filter(Boolean).join(' · '))),
    h('div.view-actions',
      h('button.btn.primary', { disabled: !listIds.length, on: { click: () => player.playList(listIds, 0, { shuffle: false }) } }, '▶ PLAY'),
      h('button.btn', { disabled: !listIds.length, on: { click: () => player.playList(listIds, Math.floor(Math.random() * listIds.length), { shuffle: true }) } }, 'SHUFFLE'),
      cfg.playlist ? h('button.btn', { on: { click: (e) => playlistMenu(e, cfg.playlist) } }, '⋯') : null,
      searchInput));

  const cols = [['#', 'trackno', 'r'], ['', null], ['TITLE', 'title'], ['ARTIST', 'artist'], ['ALBUM', 'album'], ['TIME', 'duration', 'r'], ['PLAYS', 'plays', 'r']];
  const thead = h('div.thead', cols.map(([label, key, cls]) => h('span' + (cls ? '.' + cls : '') + (activeSort?.key === key ? '.sorted' : ''), {
    on: { click: () => { if (!key) return; sort = { key, dir: activeSort?.key === key ? -activeSort.dir : 1 }; rerenderView(); } },
  }, label + (activeSort?.key === key ? (activeSort.dir > 0 ? ' ▲' : ' ▼') : ''))));

  if (!cfg.ids.length) {
    box.replaceChildren(head, emptyState());
    return;
  }

  const scroller = h('div.tracks', { tabIndex: 0 });
  const inner = h('div', { style: { height: listIds.length * ROW + 'px', position: 'relative' } });
  scroller.append(inner);
  box.replaceChildren(head, thead, scroller);

  // Virtualized + pooled: a fixed set of row elements is reused while scrolling; only text and
  // classes change. Scroll events are coalesced to one render per frame.
  const pool = [];
  const mkRow = () => {
    const r = h('div.trow', { draggable: true },
      h('span.t-num', h('span')), h('span.t-like', '♥'), h('span'), h('span.t-dim'), h('span.t-dim'), h('span.t-time'), h('span.t-plays'));
    inner.append(r);
    return r;
  };
  const setText = (el, v) => { if (el.textContent !== v) el.textContent = v; };
  let rafQueued = false;
  renderRows = () => {
    rafQueued = false;
    const top = scroller.scrollTop;
    const a = Math.max(0, Math.floor(top / ROW) - 6);
    const b = Math.min(listIds.length, Math.ceil((top + scroller.clientHeight) / ROW) + 6);
    const curId = player.current?.id;
    const numMode = activeSort?.key === 'trackno';
    let used = 0;
    for (let i = a; i < b; i++) {
      const t = state.track(listIds[i]);
      if (!t) continue;
      const st = state.stat(t.id);
      const r = pool[used] || (pool[used] = mkRow());
      used++;
      const sel = selection.has(t.id);
      r.className = 'trow' + (sel ? ' sel' : '') + (t.id === curId ? ' current' : '') + (i % 2 && !sel ? ' odd' : '');
      r.style.transform = `translateY(${i * ROW}px)`;
      r.dataset.i = i;
      r.hidden = false;
      const c = r.children;
      setText(c[0].firstChild, String(numMode && t.track ? t.track : i + 1));
      c[1].className = 't-like' + (st.liked ? ' on' : '');
      c[1].dataset.like = t.id;
      setText(c[2], t.title);
      setText(c[3], t.artist);
      setText(c[4], t.album || '');
      setText(c[5], fmtTime(t.duration));
      setText(c[6], st.plays ? String(st.plays) : '');
    }
    for (let k = used; k < pool.length; k++) pool[k].hidden = true;
  };
  scroller.addEventListener('scroll', () => { if (!rafQueued) { rafQueued = true; requestAnimationFrame(renderRows); } }, { passive: true });
  new ResizeObserver(() => renderRows?.()).observe(scroller);

  const rowIndex = (e) => {
    const r = e.target.closest('.trow');
    return r ? Number(r.dataset.i) : null;
  };
  inner.addEventListener('click', (e) => {
    const like = e.target.closest('[data-like]');
    if (like) return state.like(like.dataset.like);
    const i = rowIndex(e);
    if (i == null) return;
    const id = listIds[i];
    if (e.shiftKey && anchor != null) {
      const [x, y] = [Math.min(anchor, i), Math.max(anchor, i)];
      if (!e.ctrlKey && !e.metaKey) selection.clear();
      for (let k = x; k <= y; k++) selection.add(listIds[k]);
    } else if (e.ctrlKey || e.metaKey) {
      selection.has(id) ? selection.delete(id) : selection.add(id);
      anchor = i;
    } else {
      selection = new Set([id]);
      anchor = i;
    }
    renderRows();
  });
  inner.addEventListener('dblclick', (e) => {
    const i = rowIndex(e);
    if (i != null) player.playList(listIds, i, { shuffle: player.opts.shuffle });
  });
  inner.addEventListener('dragstart', (e) => {
    const i = rowIndex(e);
    if (i == null) return;
    if (!selection.has(listIds[i])) { selection = new Set([listIds[i]]); anchor = i; renderRows(); }
    e.dataTransfer.setData('text/x-dkfm-tracks', JSON.stringify(selectedIds()));
    e.dataTransfer.effectAllowed = 'copy';
  });
  inner.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    const i = rowIndex(e);
    if (i == null) return;
    if (!selection.has(listIds[i])) { selection = new Set([listIds[i]]); anchor = i; renderRows(); }
    trackMenu(e, selectedIds(), cfg.playlist, i);
  });
  scroller.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') { e.preventDefault(); selection = new Set(listIds); renderRows(); }
    if (e.key === 'Enter' && selection.size) { const i = listIds.indexOf([...selection][0]); player.playList(listIds, i); }
    if (e.key === 'Delete' && cfg.playlist && selection.size) removeFromPlaylist(cfg.playlist, selectedIds());
  });
  scroller.scrollTop = keepScroll;
  renderRows();
}

function selectedIds() {
  return listIds.filter((id) => selection.has(id));
}

function removeFromPlaylist(pl, ids) {
  dk.library.upsertPlaylist({ id: pl.id, trackIds: pl.trackIds.filter((x) => !ids.includes(x)) });
  selection.clear();
}

function trackMenu(e, ids, playlist, i) {
  const one = ids.length === 1 ? state.track(ids[0]) : null;
  const liked = one && state.stat(one.id).liked;
  const userPls = state.playlists.filter((p) => p.source !== 'spotify' || p.id === playlist?.id);
  contextMenu(e.clientX, e.clientY, [
    { label: '▶ Play', action: () => player.playList(listIds, i) },
    { label: 'Play next', action: () => player.playNext(ids) },
    { label: 'Add to queue', action: () => player.enqueue(ids) },
    { label: 'Add to playlist', sub: [
      { label: '+ New playlist…', action: () => newPlaylist(ids) },
      ...(userPls.length ? ['-'] : []),
      ...userPls.map((p) => ({ label: p.name, action: () => addToPlaylist(p.id, ids) })),
    ] },
    '-',
    { label: liked ? 'Unlike' : 'Like', action: () => ids.forEach((id) => state.like(id)) },
    { label: 'Go to album', action: () => setView({ key: 'album', id: albumKey(one) }), disabled: !one?.album },
    { label: 'Go to artist', action: () => setView({ key: 'artist', id: artistKey(one) }), disabled: !one },
    { label: 'Show in folder', action: () => dk.library.reveal(ids[0]) },
    '-',
    playlist ? { label: `Remove from "${playlist.name}"`, action: () => removeFromPlaylist(playlist, ids) } : null,
    { label: `Remove from library${ids.length > 1 ? ` (${ids.length})` : ''}`, action: () => { ids.forEach((id) => dk.library.removeTrack(id)); selection.clear(); } },
  ].filter(Boolean));
}

function emptyState() {
  if (view.key === 'playlist') return h('div.empty', h('span.px', 'EMPTY PLAYLIST'), 'Drag tracks onto this playlist in the sidebar, or right-click a track → Add to playlist.');
  if (state.tracks.size) return h('div.empty', h('span.px', 'NOTHING HERE YET'));
  return h('div.empty',
    h('span.px', 'YOUR LIBRARY IS EMPTY'),
    'Add a music folder, drop audio files on the window, or import a Spotify playlist.',
    h('div', { style: { display: 'flex', gap: '8px', justifyContent: 'center' } },
      h('button.btn.primary', { on: { click: () => openSettings('library') } }, 'ADD MUSIC FOLDER'),
      h('button.btn', { on: { click: () => setView({ key: 'import' }) } }, 'IMPORT FROM SPOTIFY')));
}

function albumsView(box, all) {
  const map = new Map();
  for (const t of all) {
    if (!t.album) continue;
    const k = albumKey(t);
    const a = map.get(k) || { key: k, name: t.album, artist: albumArtist(t), cover: null, n: 0, year: t.year };
    a.n++;
    if (!a.cover && t.cover) a.cover = t;
    map.set(k, a);
  }
  const albums = [...map.values()].sort((a, b) => a.artist.localeCompare(b.artist) || a.name.localeCompare(b.name));
  box.replaceChildren(
    h('div.view-head', h('div.vh-text', h('div.view-title.glow-text', 'ALBUMS'), h('div.view-sub', `${albums.length} ALBUMS`))),
    albums.length ? h('div.grid', albums.map((a) => card(a.cover ? thumbUrl(a.cover) : null, a.name, `${a.artist}${a.year ? ' · ' + a.year : ''}`, () => setView({ key: 'album', id: a.key })))) : emptyState());
}

function artistsView(box, all) {
  const map = new Map();
  for (const t of all) {
    const k = artistKey(t);
    const a = map.get(k) || { key: k, name: (t.artist || 'Unknown').split(/,|;| feat\.? | & /i)[0].trim(), cover: null, n: 0 };
    a.n++;
    if (!a.cover && t.cover) a.cover = t;
    map.set(k, a);
  }
  const artists = [...map.values()].sort((a, b) => a.name.localeCompare(b.name));
  box.replaceChildren(
    h('div.view-head', h('div.vh-text', h('div.view-title.glow-text', 'ARTISTS'), h('div.view-sub', `${artists.length} ARTISTS`))),
    artists.length ? h('div.grid', artists.map((a) => card(a.cover ? thumbUrl(a.cover) : null, a.name, `${a.n} TRACK${a.n === 1 ? '' : 'S'}`, () => setView({ key: 'artist', id: a.key })))) : emptyState());
}

function card(img, c1, c2, onClick) {
  return h('div.card', { on: { click: onClick } },
    h('div.art', { style: img ? { backgroundImage: `url("${img}")` } : null }, img ? '' : '♫'),
    h('div.c1', c1), h('div.c2', c2));
}

// ---------------- Spotify import ----------------
function importView(box) {
  const input = h('input.pixel-input', { placeholder: 'Paste a Spotify playlist, album or track link…', value: spotifyResult?.url || '' });
  const go = async () => {
    const url = input.value.trim();
    if (!url || spotifyBusy) return;
    spotifyBusy = true;
    btn.disabled = true;
    btn.textContent = 'LOADING…';
    try {
      spotifyResult = await dk.spotify.fetch(url);
      const owned = new Set([...state.tracks.values()].map((t) => t.spotifyId).filter(Boolean));
      spotifyResult.selected = new Set(spotifyResult.tracks.map((t, i) => (owned.has(t.spotifyId) ? null : i)).filter((i) => i != null));
      if (spotifyResult.warning) toast(spotifyResult.warning, { error: true });
    } catch (err) {
      toast(err.message, { error: true });
    } finally {
      spotifyBusy = false;
      rerenderView();
    }
  };
  input.addEventListener('keydown', (e) => e.key === 'Enter' && go());
  const btn = h('button.btn.primary', { on: { click: go } }, spotifyBusy ? 'LOADING…' : 'FETCH');
  const hasKeys = state.settings.spotifyClientId && state.settings.spotifyClientSecret;
  const top = h('div.import-box',
    h('div.view-title.glow-text', 'IMPORT FROM SPOTIFY'),
    h('div.import-row', input, btn),
    h('div.hint', 'DK.FM finds the best-matching studio version of each song on YouTube Music, downloads it, and tags it with Spotify’s metadata and cover art. ',
      hasKeys ? 'Using your Spotify API keys (full playlists).' : h('span', 'Works without setup for up to 100 tracks. ', h('a', { on: { click: () => openSettings('spotify') } }, 'Add free Spotify API keys'), ' for unlimited playlist size and album info.')));

  if (!spotifyResult) {
    box.replaceChildren(top, h('div.empty', h('span.px', 'PASTE A LINK ABOVE'), 'Playlists, albums and single tracks are supported. The playlist must be public.'));
    setTimeout(() => input.focus());
    return;
  }

  const r = spotifyResult;
  const owned = new Map([...state.tracks.values()].filter((t) => t.spotifyId).map((t) => [t.spotifyId, t]));
  const totalSel = r.selected.size;
  const fmtName = { 'mp3-320': 'MP3 320', 'mp3-v0': 'MP3 V0', m4a: 'M4A/AAC' }[state.settings.downloadFormat] || 'MP3';
  const head = h('div.view-head',
    r.cover ? h('img.vh-cover', { src: r.cover }) : null,
    h('div.vh-text',
      h('div.view-title.glow-text', r.name),
      h('div.view-sub', `${r.type.toUpperCase()} · ${r.owner || ''} · ${r.tracks.length} TRACKS · VIA ${r.via === 'api' ? 'SPOTIFY API' : 'PUBLIC PAGE'}`),
      !r.complete ? h('div.warn', '⚠ Spotify’s public page only lists the first 100 tracks. ', h('a', { style: { cursor: 'pointer', textDecoration: 'underline' }, on: { click: () => openSettings('spotify') } }, 'Add API keys'), ' to get them all.') : null),
    h('div.view-actions',
      h('button.btn', { on: { click: () => { r.selected = new Set(r.tracks.map((_, i) => i)); rerenderView(); } } }, 'ALL'),
      h('button.btn', { on: { click: () => { r.selected = new Set(); rerenderView(); } } }, 'NONE'),
      h('button.btn.primary', { disabled: !totalSel, on: { click: async () => { await dk.dl.start(stripCollection(r), [...r.selected]); setView({ key: 'downloads' }); } } }, `⇣ DOWNLOAD ${totalSel} AS ${fmtName}`)));

  const list = h('div.tracks', { style: { position: 'relative' } },
    r.tracks.map((t, i) => {
      const have = owned.get(t.spotifyId);
      const cb = h('input.chk', { type: 'checkbox', checked: r.selected.has(i), on: { change: (e) => { e.target.checked ? r.selected.add(i) : r.selected.delete(i); updateBtn(); } } });
      return h('label.irow', cb, h('span.t-num', String(i + 1)), t.cover ? h('img', { src: t.cover, loading: 'lazy' }) : h('span'),
        h('span', t.title), h('span.t-dim', t.artists.join(', ')), h('span.t-time', fmtTime((t.durationMs || 0) / 1000)),
        h('span', have ? h('span.badge.ok', 'IN LIBRARY') : t.explicit ? h('span.badge', 'E') : ''));
    }));
  const updateBtn = () => {
    const b = head.querySelector('.btn.primary');
    b.textContent = `⇣ DOWNLOAD ${r.selected.size} AS ${fmtName}`;
    b.disabled = !r.selected.size;
  };
  box.replaceChildren(top, head, list);
}

const stripCollection = (r) => {
  const { selected, ...rest } = r;
  return rest;
};

// ---------------- Downloads ----------------
let jobRenderQueued = new Set();
function scheduleJobRender(jobId) {
  if (view.key !== 'downloads') {
    renderSourcesThrottled();
    return;
  }
  jobRenderQueued.add(jobId);
  if (jobRenderQueued.size > 1) return;
  requestAnimationFrame(() => {
    for (const id of jobRenderQueued) {
      const old = document.querySelector(`.job[data-id="${CSS.escape(id)}"]`);
      const j = jobs.get(id);
      if (old && j) {
        const scroll = old.querySelector('.job-tracks')?.scrollTop || 0;
        const fresh = jobCard(j);
        old.replaceWith(fresh);
        const jt = fresh.querySelector('.job-tracks');
        if (jt) jt.scrollTop = scroll;
      }
    }
    jobRenderQueued = new Set();
    renderSourcesThrottled();
  });
}
const renderSourcesThrottled = debounce(renderSources, 400);

function renderEngine() {
  const el = document.getElementById('engine');
  if (!el) return;
  dk.yt.status().then((s) => {
    el.textContent = {
      idle: 'ENGINE: STANDBY',
      installing: 'ENGINE: DOWNLOADING YT-DLP…',
      updating: 'ENGINE: UPDATING YT-DLP…',
      ready: `ENGINE: YT-DLP ${s.version} ✓`,
      error: `ENGINE ERROR: ${s.error}`,
    }[s.state] || '';
  });
}

const collapsed = new Set();

function downloadsView(box) {
  const list = [...jobs.values()].sort((a, b) => b.createdAt - a.createdAt);
  box.replaceChildren(
    h('div.view-head',
      h('div.vh-text', h('div.view-title.glow-text', 'DOWNLOADS'), h('div.engine#engine', '…')),
      h('div.view-actions',
        h('button.btn', { on: { click: () => setView({ key: 'import' }) } }, '+ IMPORT'),
        h('button.btn', { on: { click: () => dk.shell.openPath(state.settings.downloadDir) } }, 'OPEN FOLDER'))),
    list.length ? h('div.jobs', list.map(jobCard)) : h('div.empty', h('span.px', 'NO DOWNLOADS'), 'Import a Spotify playlist to get started.', h('br'), h('button.btn.primary', { on: { click: () => setView({ key: 'import' }) } }, 'IMPORT PLAYLIST')));
  renderEngine();
}

function jobCard(j) {
  const counts = { done: 0, failed: 0, active: 0, total: 0 };
  for (const t of j.tracks) {
    if (t.status === 'skipped') continue;
    counts.total++;
    if (t.status === 'done') counts.done++;
    else if (t.status === 'failed') counts.failed++;
    else if (t.status !== 'cancelled') counts.active++;
  }
  const pct = counts.total ? ((counts.done + counts.failed) / counts.total) * 100 : 0;
  const isOpen = !collapsed.has(j.id);
  const pl = state.playlists.find((p) => p.id === 'sp-' + j.id);
  return h('div.job', { 'data-id': j.id },
    h('div.job-head',
      j.cover ? h('img', { src: j.cover }) : null,
      h('div.job-info',
        h('div.job-name', j.name),
        h('div.view-sub', `${counts.done}/${counts.total} DONE${counts.failed ? ` · ${counts.failed} FAILED` : ''}${counts.active ? ` · ${counts.active} IN PROGRESS` : ''}`),
        h('div.job-bar', h('i', { style: { width: pct + '%' } }))),
      h('div.job-actions',
        pl && counts.done ? h('button.mini-btn', { on: { click: () => setView({ key: 'playlist', id: pl.id }) } }, '▶ OPEN') : null,
        h('button.mini-btn', { on: { click: () => dk.shell.openPath(j.folder) } }, 'FOLDER'),
        counts.failed ? h('button.mini-btn', { on: { click: () => j.tracks.forEach((t) => t.status === 'failed' && dk.dl.retry(j.id, t.idx)) } }, 'RETRY FAILED') : null,
        counts.active ? h('button.mini-btn', { on: { click: () => dk.dl.cancel(j.id) } }, 'CANCEL') : null,
        !counts.active ? h('button.mini-btn', { on: { click: () => dk.dl.clear(j.id) } }, 'DISMISS') : null,
        h('button.mini-btn', { on: { click: () => { isOpen ? collapsed.add(j.id) : collapsed.delete(j.id); scheduleJobRender(j.id); } } }, isOpen ? '▴' : '▾'))),
    isOpen ? h('div.job-tracks', j.tracks.filter((t) => t.status !== 'skipped').map((t) => jobRow(j, t))) : null);
}

function jobRow(j, t) {
  const st = t.status;
  const label = { queued: 'QUEUED', searching: 'SEARCHING', downloading: `${Math.round(t.progress || 0)}%`, tagging: 'TAGGING', done: t.note ? 'HAD IT' : 'DONE ✓', failed: 'FAILED', cancelled: 'CANCELLED' }[st] || st;
  const match = t.match ? `${t.lowConfidence ? '⚠ ' : ''}${t.match.title}${t.match.channel ? ' — ' + t.match.channel : ''}` : t.error || '';
  return h('div.jrow', {
    title: t.error || (t.match ? `Matched: ${t.match.title} (score ${t.match.score})` : ''),
    on: { contextmenu: (e) => jobTrackMenu(e, j, t), dblclick: () => t.trackId && state.track(t.trackId) && player.playList([t.trackId], 0) },
  },
    h('span.t-num', String(t.idx + 1)),
    h('span', `${t.title} — ${t.artists.join(', ')}`),
    h('span.match' + (t.lowConfidence || st === 'failed' ? '.low' : ''), match),
    h('span.st.st-' + st, st === 'downloading' ? h('span.mini-bar', h('i', { style: { width: (t.progress || 0) + '%' } })) : null, label));
}

function jobTrackMenu(e, j, t) {
  e.preventDefault();
  const busy = ['searching', 'downloading', 'tagging'].includes(t.status);
  contextMenu(e.clientX, e.clientY, [
    { label: 'Play', disabled: !t.trackId, action: () => player.playList([t.trackId], 0) },
    { label: 'Retry', disabled: busy, action: () => dk.dl.retry(j.id, t.idx) },
    { label: 'Use a different match', disabled: busy || !t.candidates?.length, sub: (t.candidates || []).map((c) => ({
      label: `${c.id === t.match?.id ? '● ' : ''}${c.title} (${c.channel || c.source}${c.duration ? ', ' + fmtTime(c.duration) : ''})`,
      action: () => dk.dl.retry(j.id, t.idx, c.id),
    })) },
    { label: 'Paste YouTube link…', disabled: busy, action: async () => {
      const link = await prompt('YOUTUBE LINK FOR THIS TRACK');
      const id = link?.match(/(?:v=|youtu\.be\/|shorts\/)([\w-]{11})/)?.[1] || (link?.length === 11 ? link : null);
      if (id) dk.dl.retry(j.id, t.idx, id);
      else if (link) toast('That is not a YouTube link', { error: true });
    } },
    { label: 'Open match on YouTube', disabled: !t.match, action: () => dk.shell.openExternal(`https://music.youtube.com/watch?v=${t.match.id}`) },
    { label: 'Show in folder', disabled: !t.trackId, action: () => dk.library.reveal(t.trackId) },
  ]);
}

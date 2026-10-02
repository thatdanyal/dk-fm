const { app, BrowserWindow, ipcMain, protocol, dialog, shell, screen } = require('electron');
const fs = require('fs');
const path = require('path');
const { Readable } = require('stream');
const { Store } = require('./store');
const { Library } = require('./library');
const { Downloader } = require('./downloader');
const { fetchSpotify } = require('./spotify');
const yt = require('./ytdlp');
const updater = require('./updater');

protocol.registerSchemesAsPrivileged([
  { scheme: 'dkfm', privileges: { standard: true, secure: true, supportFetchAPI: true, stream: true, corsEnabled: true } },
]);

if (!app.requestSingleInstanceLock()) app.quit();

// Leaner Chromium: run the network service inside the main process (one less process) and skip
// features a music player never uses. DKFM_NO_TUNING=1 disables this for comparison.
if (!process.env.DKFM_NO_TUNING) {
  app.commandLine.appendSwitch('enable-features', 'NetworkServiceInProcess2');
  app.commandLine.appendSwitch('disable-features', 'SpareRendererForSitePerProcess,MediaRouter,DialMediaRouteProvider,AutofillServerCommunication,Translate,OptimizationHints');
  app.commandLine.appendSwitch('disable-renderer-backgrounding');
}

const RENDERER = path.join(__dirname, '..', 'renderer');
let win;
let settings, library, downloader, upd;

const emit = (channel, payload) => {
  if (win && !win.isDestroyed()) win.webContents.send(channel, payload);
};

function defaults() {
  const music = app.getPath('music');
  return {
    theme: 'red-retro',
    accent: null,
    scanlines: true,
    glow: true,
    pixelFont: true,
    musicFolders: [music],
    downloadDir: path.join(music, 'DK.FM'),
    downloadFormat: 'mp3-320',
    downloadConcurrency: 3,
    spotifyClientId: '',
    spotifyClientSecret: '',
    autoUpdate: true,
    layout: null,
    player: { volume: 0.8, crossfade: 0, speed: 1, visualizer: 'bars', shuffle: false, repeat: 'off', normalize: false },
    eq: { enabled: false, preset: 'Flat', gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], preamp: 0 },
    session: { queue: [], index: -1, position: 0 },
    windowBounds: null,
  };
}

// ---------- dkfm:// protocol: app files, local media (with Range for seeking), cover cache ----------
const MIME = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2', '.png': 'image/png', '.jpg': 'image/jpeg', '.svg': 'image/svg+xml', '.mp3': 'audio/mpeg', '.m4a': 'audio/mp4', '.aac': 'audio/aac', '.flac': 'audio/flac', '.ogg': 'audio/ogg', '.opus': 'audio/ogg', '.wav': 'audio/wav', '.webm': 'audio/webm', '.aiff': 'audio/aiff', '.aif': 'audio/aiff' };

function serveFile(file, req) {
  let st;
  try { st = fs.statSync(file); } catch { return new Response('Not found', { status: 404 }); }
  const type = MIME[path.extname(file).toLowerCase()] || 'application/octet-stream';
  const range = req.headers.get('range');
  if (range) {
    const m = range.match(/bytes=(\d*)-(\d*)/);
    let start = m[1] ? parseInt(m[1], 10) : 0;
    let end = m[2] ? parseInt(m[2], 10) : st.size - 1;
    if (!m[1] && m[2]) { start = st.size - parseInt(m[2], 10); end = st.size - 1; }
    end = Math.min(end, st.size - 1);
    if (start >= st.size) return new Response(null, { status: 416, headers: { 'Content-Range': `bytes */${st.size}` } });
    return new Response(Readable.toWeb(fs.createReadStream(file, { start, end })), {
      status: 206,
      headers: { 'Content-Type': type, 'Content-Length': String(end - start + 1), 'Content-Range': `bytes ${start}-${end}/${st.size}`, 'Accept-Ranges': 'bytes', 'Access-Control-Allow-Origin': '*' },
    });
  }
  return new Response(Readable.toWeb(fs.createReadStream(file)), {
    headers: { 'Content-Type': type, 'Content-Length': String(st.size), 'Accept-Ranges': 'bytes', 'Access-Control-Allow-Origin': '*' },
  });
}

function registerProtocol() {
  protocol.handle('dkfm', (req) => {
    const u = new URL(req.url);
    const p = decodeURIComponent(u.pathname);
    if (u.host === 'app') {
      const file = path.normalize(path.join(RENDERER, p === '/' ? 'index.html' : p));
      if (!file.startsWith(RENDERER)) return new Response('Forbidden', { status: 403 });
      const res = serveFile(file, req);
      res.headers.set('Cache-Control', 'no-store');
      return res;
    }
    if (u.host === 'cover') {
      const dir = path.join(app.getPath('userData'), 'covers');
      const file = path.normalize(path.join(dir, path.basename(p)));
      const res = serveFile(file, req);
      res.headers.set('Cache-Control', 'public, max-age=31536000, immutable'); // names are content hashes
      return res;
    }
    if (u.host === 'media') {
      // Only files the library knows about may be streamed.
      const id = p.slice(1);
      const t = library.store.data.tracks[id];
      if (!t) return new Response('Unknown track', { status: 404 });
      return serveFile(t.path, req);
    }
    return new Response('Not found', { status: 404 });
  });
}

// ---------- window ----------
function createWindow() {
  const b = settings.get('windowBounds');
  const visible = b && screen.getAllDisplays().some((d) => {
    const a = d.workArea;
    return b.x < a.x + a.width && b.x + b.width > a.x && b.y < a.y + a.height && b.y + b.height > a.y;
  });
  win = new BrowserWindow({
    width: b?.width || 1360,
    height: b?.height || 860,
    x: visible ? b.x : undefined,
    y: visible ? b.y : undefined,
    minWidth: 380,
    minHeight: 120,
    frame: false,
    titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : 'hidden',
    backgroundColor: '#0d0505',
    show: false,
    icon: path.join(__dirname, '..', '..', 'build', 'icon.png'),
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false, // audio timing (crossfade/gapless) must keep running when hidden
      spellcheck: false,
    },
  });
  if (b?.maximized) win.maximize();
  win.loadURL('dkfm://app/index.html');
  win.once('ready-to-show', () => win.show());
  const saveBounds = () => {
    if (win.isDestroyed() || miniMode) return;
    settings.set('windowBounds', { ...win.getNormalBounds(), maximized: win.isMaximized() });
  };
  win.on('resize', saveBounds);
  win.on('move', saveBounds);
  win.on('maximize', () => emit('win:state', { maximized: true }));
  win.on('unmaximize', () => emit('win:state', { maximized: false }));
  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https:\/\//.test(url)) shell.openExternal(url);
    return { action: 'deny' };
  });
  win.webContents.on('will-navigate', (e) => e.preventDefault());

  // Dev helper: DKFM_SCREENSHOT=out.png captures the window then quits.
  if (process.env.DKFM_SCREENSHOT) {
    win.webContents.once('did-finish-load', () => setTimeout(async () => {
      const img = await win.webContents.capturePage();
      fs.writeFileSync(process.env.DKFM_SCREENSHOT, img.toPNG());
      app.quit();
    }, Number(process.env.DKFM_SCREENSHOT_DELAY || 2500)));
  }
}

let miniMode = false;
let normalBounds = null;

// ---------- IPC ----------
function registerIpc() {
  const h = (ch, fn) => ipcMain.handle(ch, (_e, ...a) => fn(...a));

  h('settings:get', () => ({ ...settings.get(), platform: process.platform, version: app.getVersion() }));
  h('app:gpu', () => app.getGPUFeatureStatus().gpu_compositing);
  h('settings:set', (patch) => {
    settings.patch(patch);
    return settings.get();
  });

  h('library:get', () => library.snapshot());
  h('library:scan', () => library.scan(uniqueFolders()));
  h('library:addFiles', (paths) => library.addFiles(paths));
  h('library:removeTrack', (id) => library.removeTrack(id));
  h('library:like', (id) => library.toggleLike(id));
  h('library:played', (id) => library.bumpPlay(id));
  h('library:upsertPlaylist', (pl) => library.upsertPlaylist(pl));
  h('library:deletePlaylist', (id) => library.deletePlaylist(id));
  h('library:reveal', (id) => {
    const t = library.store.data.tracks[id];
    if (t) shell.showItemInFolder(t.path);
  });

  h('dialog:folder', async () => {
    const r = await dialog.showOpenDialog(win, { properties: ['openDirectory'] });
    return r.canceled ? null : r.filePaths[0];
  });
  h('dialog:files', async () => {
    const r = await dialog.showOpenDialog(win, { properties: ['openFile', 'multiSelections'], filters: [{ name: 'Audio', extensions: ['mp3', 'm4a', 'flac', 'ogg', 'opus', 'wav', 'aac', 'aiff', 'webm'] }] });
    return r.canceled ? [] : library.addFiles(r.filePaths);
  });
  h('shell:openPath', (p) => shell.openPath(p));
  h('shell:openExternal', (u) => /^https:\/\//.test(u) && shell.openExternal(u));

  h('spotify:fetch', (url) => fetchSpotify(url, { clientId: settings.get('spotifyClientId'), clientSecret: settings.get('spotifyClientSecret') }));
  h('dl:start', (collection, selected) => downloader.start(collection, selected));
  h('dl:retry', (jobId, idx, videoId) => downloader.retry(jobId, idx, videoId));
  h('dl:cancel', (jobId) => downloader.cancel(jobId));
  h('dl:clear', (jobId) => downloader.clear(jobId));
  h('dl:list', () => downloader.list());
  h('yt:status', () => yt.getStatus());
  h('yt:ensure', () => yt.ensure().catch((e) => ({ error: e.message })));

  // Waveform peaks cache: each song is decoded once, ever.
  const waveDir = path.join(app.getPath('userData'), 'waves');
  fs.mkdirSync(waveDir, { recursive: true });
  const waveFile = (id) => path.join(waveDir, String(id).replace(/[^a-f0-9]/gi, '') + '.bin');
  h('wave:get', (id) => fs.promises.readFile(waveFile(id)).catch(() => null));
  h('wave:set', (id, data) => fs.promises.writeFile(waveFile(id), Buffer.from(data)).catch(() => {}));

  h('lyrics:get', async (q) => {
    const params = new URLSearchParams({ artist_name: q.artist, track_name: q.title });
    if (q.album) params.set('album_name', q.album);
    if (q.duration) params.set('duration', String(Math.round(q.duration)));
    const headers = { 'User-Agent': `DK.FM/${app.getVersion()} (https://github.com/thatdanyal/dk-fm)` };
    let r = await fetch(`https://lrclib.net/api/get?${params}`, { headers }).catch(() => null);
    if (r?.ok) return r.json();
    r = await fetch(`https://lrclib.net/api/search?${new URLSearchParams({ artist_name: q.artist, track_name: q.title })}`, { headers }).catch(() => null);
    if (!r?.ok) return null;
    const list = await r.json();
    return list.find((x) => x.syncedLyrics) || list[0] || null;
  });

  h('win:minimize', () => win.minimize());
  h('win:maximize', () => (win.isMaximized() ? win.unmaximize() : win.maximize()));
  h('win:close', () => win.close());
  h('win:mini', (on) => {
    miniMode = on;
    if (on) {
      normalBounds = win.getBounds();
      if (win.isMaximized()) win.unmaximize();
      win.setAlwaysOnTop(true, 'floating');
      win.setMinimumSize(380, 120);
      win.setSize(460, 168);
    } else {
      win.setAlwaysOnTop(false);
      if (normalBounds) win.setBounds(normalBounds);
    }
    return on;
  });
  h('win:onTop', (on) => win.setAlwaysOnTop(!!on));

  h('updater:check', () => upd.check());
  h('updater:download', () => upd.download());
  h('updater:install', () => upd.install());
  h('updater:status', () => upd.status());
}

function uniqueFolders() {
  const list = [...settings.get('musicFolders'), settings.get('downloadDir')];
  return [...new Set(list.map((f) => path.resolve(f)))].filter((f) => fs.existsSync(f));
}

app.on('second-instance', () => {
  if (win) {
    if (win.isMinimized()) win.restore();
    win.focus();
  }
});

app.whenReady().then(() => {
  settings = new Store('settings', defaults());
  library = new Library(emit);
  downloader = new Downloader({ emit, library, settings });
  yt.onStatus((s) => emit('yt:status', s));
  registerProtocol();
  registerIpc();
  upd = updater.init(emit, () => settings.get('autoUpdate'));
  createWindow();
  // Warm up: fetch/update yt-dlp in the background so the first import is fast.
  setTimeout(() => yt.ensure().catch(() => {}), 3000);
  app.on('activate', () => BrowserWindow.getAllWindows().length === 0 && createWindow());
});

app.on('before-quit', () => {
  settings?.flush();
  library?.store.flush();
});
app.on('window-all-closed', () => app.quit());

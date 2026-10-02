// Manages the download tools (yt-dlp + ffmpeg): fetched on first use, yt-dlp kept up to date daily.
//  - Windows/macOS use yt-dlp's "onedir" build: ~3x faster startup than the single-file exe, which
//    re-extracts itself to %TEMP% on every launch (we launch it several times per song).
//  - ffmpeg is downloaded on demand instead of bundled, keeping the installer ~30MB smaller.
//  - Electron itself doubles as yt-dlp's JavaScript runtime (ELECTRON_RUN_AS_NODE).
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');
const { spawn, execFile } = require('child_process');
const { pipeline } = require('stream/promises');
const { Readable } = require('stream');
const { app } = require('electron');

const BIN_DIR = () => path.join(app.getPath('userData'), 'bin');
const EXE = process.platform === 'win32' ? '.exe' : '';
const plat = process.platform;
const arch = process.arch;

// onedir zip for win/mac (fast startup); single binary on Linux (no unzip guaranteed there)
const YT_ASSET = {
  win32: arch === 'arm64' ? 'yt-dlp_win_arm64.zip' : 'yt-dlp_win.zip',
  darwin: 'yt-dlp_macos.zip',
  linux: arch === 'arm64' ? 'yt-dlp_linux_aarch64' : 'yt-dlp_linux',
}[plat];
const YT_DIR = () => path.join(BIN_DIR(), 'yt-dlp');
const ytPath = () => (YT_ASSET.endsWith('.zip') ? path.join(YT_DIR(), plat === 'darwin' ? 'yt-dlp_macos' : 'yt-dlp' + EXE) : path.join(YT_DIR(), 'yt-dlp'));
const ffPath = () => path.join(BIN_DIR(), 'ffmpeg' + EXE);
const FF_URL = `https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1/ffmpeg-${plat}-${arch}.gz`;

let ready = null;
let status = { state: 'idle', version: null };
const listeners = new Set();
const setStatus = (s) => { status = { ...status, ...s }; listeners.forEach((fn) => fn(status)); };

async function download(url, dest, { gunzip = false } = {}) {
  const r = await fetch(url, { headers: { 'User-Agent': 'DK.FM' } });
  if (!r.ok) throw new Error(`Download failed (${r.status}): ${url}`);
  const tmp = dest + '.part';
  const src = Readable.fromWeb(r.body);
  await pipeline(src, ...(gunzip ? [zlib.createGunzip()] : []), fs.createWriteStream(tmp));
  fs.renameSync(tmp, dest);
}

function unzip(zip, dir) {
  return new Promise((resolve, reject) => {
    const [cmd, args] = plat === 'win32'
      ? [path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe'), ['-xf', zip, '-C', dir]]
      : ['ditto', ['-x', '-k', zip, dir]];
    execFile(cmd, args, { windowsHide: true }, (err) => (err ? reject(err) : resolve()));
  });
}

async function latestYtVersion() {
  const r = await fetch('https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest', { headers: { 'User-Agent': 'DK.FM' } });
  if (!r.ok) return null;
  return (await r.json()).tag_name;
}

async function installYt(version) {
  setStatus({ state: status.version ? 'updating' : 'installing' });
  fs.mkdirSync(BIN_DIR(), { recursive: true });
  const base = version ? `https://github.com/yt-dlp/yt-dlp/releases/download/${version}` : 'https://github.com/yt-dlp/yt-dlp/releases/latest/download';
  const stage = YT_DIR() + '.new';
  fs.rmSync(stage, { recursive: true, force: true });
  fs.mkdirSync(stage, { recursive: true });
  if (YT_ASSET.endsWith('.zip')) {
    const zip = path.join(BIN_DIR(), YT_ASSET);
    await download(`${base}/${YT_ASSET}`, zip);
    await unzip(zip, stage);
    fs.rmSync(zip, { force: true });
  } else {
    await download(`${base}/${YT_ASSET}`, path.join(stage, 'yt-dlp'));
  }
  fs.rmSync(YT_DIR(), { recursive: true, force: true });
  fs.renameSync(stage, YT_DIR());
  if (plat !== 'win32') fs.chmodSync(ytPath(), 0o755);
}

function run(args, { onLine, signal, bin = ytPath() } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(bin, args, {
      windowsHide: true,
      env: { ...process.env, ELECTRON_RUN_AS_NODE: '1', PYTHONIOENCODING: 'utf-8', PYTHONUTF8: '1' },
    });
    let out = '';
    let err = '';
    let buf = '';
    child.stdout.on('data', (d) => {
      const s = d.toString('utf8');
      out += s;
      if (onLine) {
        buf += s;
        const lines = buf.split(/\r?\n/);
        buf = lines.pop();
        lines.forEach(onLine);
      }
    });
    child.stderr.on('data', (d) => { err += d.toString('utf8'); });
    const abort = () => child.kill();
    signal?.addEventListener('abort', abort, { once: true });
    child.on('error', reject);
    child.on('close', (code) => {
      signal?.removeEventListener('abort', abort);
      if (signal?.aborted) return reject(new Error('Cancelled'));
      if (code === 0) resolve(out);
      else {
        const msg = err.split('\n').filter((l) => l.startsWith('ERROR')).pop() || err.trim().split('\n').pop() || `exited ${code}`;
        reject(new Error(msg.replace(/^ERROR:\s*/, '')));
      }
    });
  });
}

const baseArgs = () => ['--js-runtimes', `node:${process.execPath}`, '--ffmpeg-location', ffPath(), '--no-warnings', '--encoding', 'utf-8', '--no-cache-dir'];

function ensure() {
  if (ready) return ready;
  ready = (async () => {
    fs.mkdirSync(BIN_DIR(), { recursive: true });
    const stamp = path.join(BIN_DIR(), 'yt-dlp.version');
    const have = fs.existsSync(ytPath()) && fs.existsSync(stamp) ? fs.readFileSync(stamp, 'utf8').trim() : null;
    const tasks = [];
    if (!fs.existsSync(ffPath())) {
      tasks.push((async () => {
        await download(FF_URL, ffPath(), { gunzip: true });
        if (plat !== 'win32') fs.chmodSync(ffPath(), 0o755);
      })());
    }
    const checkedFile = path.join(BIN_DIR(), 'last-check');
    const lastCheck = Number(fs.existsSync(checkedFile) ? fs.readFileSync(checkedFile, 'utf8') : 0);
    if (!have || Date.now() - lastCheck > 24 * 3600 * 1000) {
      tasks.push((async () => {
        const latest = await latestYtVersion().catch(() => null);
        if (!have || (latest && latest !== have)) {
          setStatus({ version: have });
          await installYt(latest);
          fs.writeFileSync(stamp, latest || (await run(['--version'])).trim());
        }
        fs.writeFileSync(checkedFile, String(Date.now()));
      })().catch((e) => { if (!have) throw e; })); // offline with a working copy: keep it
    }
    await Promise.all(tasks);
    try { fs.rmSync(path.join(BIN_DIR(), 'yt-dlp.exe'), { force: true }); fs.rmSync(path.join(BIN_DIR(), 'last-update'), { force: true }); } catch { /* still locked; next launch */ }
    const version = fs.readFileSync(stamp, 'utf8').trim();
    setStatus({ state: 'ready', version });
    return version;
  })().catch((e) => {
    ready = null;
    setStatus({ state: 'error', error: e.message });
    throw e;
  });
  return ready;
}

module.exports = {
  ensure,
  run,
  baseArgs,
  ffmpegPath: ffPath,
  getStatus: () => status,
  onStatus: (fn) => listeners.add(fn),
};

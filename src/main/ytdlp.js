// Manages the yt-dlp binary (download on first run, daily self-update) and runs it.
// Electron itself doubles as yt-dlp's JavaScript runtime (ELECTRON_RUN_AS_NODE), so YouTube's
// signature challenges are solved without the user installing Node/Deno.
const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');
const { app } = require('electron');

const BIN_DIR = () => path.join(app.getPath('userData'), 'bin');
const BIN_NAME = process.platform === 'win32' ? 'yt-dlp.exe' : 'yt-dlp';
const ASSET = {
  win32: 'yt-dlp.exe',
  darwin: 'yt-dlp_macos',
  linux: process.arch === 'arm64' ? 'yt-dlp_linux_aarch64' : 'yt-dlp_linux',
}[process.platform];

const binPath = () => path.join(BIN_DIR(), BIN_NAME);

function ffmpegPath() {
  let p = require('ffmpeg-static');
  if (p && p.includes('app.asar')) p = p.replace('app.asar', 'app.asar.unpacked');
  return p;
}

let ready = null;
let status = { state: 'idle', version: null };
const listeners = new Set();
const setStatus = (s) => { status = { ...status, ...s }; listeners.forEach((fn) => fn(status)); };

async function downloadBinary() {
  setStatus({ state: 'installing' });
  fs.mkdirSync(BIN_DIR(), { recursive: true });
  const r = await fetch(`https://github.com/yt-dlp/yt-dlp/releases/latest/download/${ASSET}`);
  if (!r.ok) throw new Error(`Could not download yt-dlp (${r.status})`);
  const buf = Buffer.from(await r.arrayBuffer());
  const tmp = binPath() + '.part';
  fs.writeFileSync(tmp, buf);
  fs.chmodSync(tmp, 0o755);
  fs.renameSync(tmp, binPath());
}

function run(args, { onLine, signal } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(binPath(), args, {
      windowsHide: true,
      env: { ...process.env, ELECTRON_RUN_AS_NODE: '1', PYTHONIOENCODING: 'utf-8' },
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
        const msg = err.split('\n').filter((l) => l.startsWith('ERROR')).pop() || err.trim().split('\n').pop() || `yt-dlp exited ${code}`;
        reject(new Error(msg.replace(/^ERROR:\s*/, '')));
      }
    });
  });
}

// Common flags: Electron as JS runtime, bundled ffmpeg.
const baseArgs = () => ['--js-runtimes', `node:${process.execPath}`, '--ffmpeg-location', ffmpegPath(), '--no-warnings', '--encoding', 'utf-8'];

function ensure() {
  if (ready) return ready;
  ready = (async () => {
    const stampFile = path.join(BIN_DIR(), 'last-update');
    if (!fs.existsSync(binPath())) {
      await downloadBinary();
      fs.writeFileSync(stampFile, String(Date.now()));
    } else {
      const last = Number(fs.existsSync(stampFile) ? fs.readFileSync(stampFile, 'utf8') : 0);
      if (Date.now() - last > 24 * 3600 * 1000) {
        setStatus({ state: 'updating' });
        await run(['-U']).catch(() => {}); // keep going with the old binary if offline
        fs.writeFileSync(stampFile, String(Date.now()));
      }
    }
    const version = (await run(['--version'])).trim();
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
  ffmpegPath,
  getStatus: () => status,
  onStatus: (fn) => listeners.add(fn),
};

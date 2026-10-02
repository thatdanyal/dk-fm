// Auto-update from GitHub Releases. Every push to main publishes a new release (see
// .github/workflows/release.yml). Installed copies check on launch and every 30 minutes; when a
// newer version exists the renderer shows an "Update available" popup with the release notes.
// Nothing is downloaded until the user clicks DOWNLOAD & RESTART.
const { app, shell } = require('electron');
const { autoUpdater } = require('electron-updater');

const REPO = 'https://github.com/thatdanyal/dk-fm';
let status = { state: 'idle', version: app.getVersion() };

function notesToText(notes) {
  if (!notes) return '';
  const list = Array.isArray(notes) ? notes.map((n) => n.note || '').join('\n') : String(notes);
  return list
    .replace(/<li[^>]*>/gi, '\n• ')
    .replace(/<br\s*\/?>|<\/p>|<\/h\d>/gi, '\n')
    .replace(/<[^>]+>/g, '')
    .replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'")
    .replace(/^\s*[-*]\s+/gm, '• ')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

function init(emit, enabled) {
  const set = (s) => { status = { ...status, ...s }; emit('updater:status', status); };
  if (!app.isPackaged) {
    set({ state: 'dev' });
    // DKFM_FAKE_UPDATE=1 previews the update popup when running from source.
    if (process.env.DKFM_FAKE_UPDATE) setTimeout(() => set({ state: 'available', next: '9.9.9', notes: notesToText('<ul><li>Fixed shuffle on playlists</li><li>New Amber CRT tweaks</li></ul>') }), 3000);
    return { check: async () => set({ state: 'dev' }), download: () => {}, install: () => {}, status: () => status };
  }
  autoUpdater.autoDownload = false;
  autoUpdater.autoInstallOnAppQuit = true;
  let installWhenReady = false;

  autoUpdater.on('checking-for-update', () => { if (!['available', 'downloading', 'ready'].includes(status.state)) set({ state: 'checking' }); });
  autoUpdater.on('update-available', (i) => {
    if (['downloading', 'ready'].includes(status.state)) return;
    set({ state: 'available', next: i.version, notes: notesToText(i.releaseNotes), releaseName: i.releaseName || `DK.FM v${i.version}`, date: i.releaseDate });
  });
  autoUpdater.on('update-not-available', () => set({ state: 'latest', checkedAt: Date.now() }));
  autoUpdater.on('download-progress', (p) => set({ state: 'downloading', percent: Math.round(p.percent) }));
  autoUpdater.on('update-downloaded', (i) => {
    set({ state: 'ready', next: i.version });
    if (installWhenReady) setTimeout(() => autoUpdater.quitAndInstall(false, true), 800);
  });
  autoUpdater.on('error', (e) => set({ state: 'error', error: String(e?.message || e).split('\n')[0] }));

  const check = () => autoUpdater.checkForUpdates().catch(() => {});
  if (enabled()) setTimeout(check, 4000);
  setInterval(() => enabled() && check(), 30 * 60 * 1000);

  return {
    check,
    download: () => {
      // Unsigned macOS apps can't self-update (Squirrel.Mac needs a Developer ID), so send
      // Mac users to the release page to grab the new .dmg instead.
      if (process.platform === 'darwin') return shell.openExternal(`${REPO}/releases/latest`);
      installWhenReady = true;
      if (status.state === 'ready') return autoUpdater.quitAndInstall(false, true);
      set({ state: 'downloading', percent: 0 });
      autoUpdater.downloadUpdate().catch((e) => set({ state: 'error', error: String(e?.message || e).split('\n')[0] }));
    },
    install: () => autoUpdater.quitAndInstall(false, true),
    status: () => status,
  };
}

module.exports = { init };

// Auto-update from GitHub Releases. Every push to main publishes a new release (see
// .github/workflows/release.yml); installed copies pick it up here.
const { app } = require('electron');
const { autoUpdater } = require('electron-updater');

let status = { state: 'idle', version: app.getVersion() };

function init(emit, enabled) {
  const set = (s) => { status = { ...status, ...s }; emit('updater:status', status); };
  if (!app.isPackaged) {
    set({ state: 'dev' });
    return { check: async () => set({ state: 'dev' }), install: () => {}, status: () => status };
  }
  autoUpdater.autoDownload = true;
  autoUpdater.autoInstallOnAppQuit = true;
  autoUpdater.on('checking-for-update', () => set({ state: 'checking' }));
  autoUpdater.on('update-available', (i) => set({ state: 'downloading', next: i.version, percent: 0 }));
  autoUpdater.on('update-not-available', () => set({ state: 'latest', checkedAt: Date.now() }));
  autoUpdater.on('download-progress', (p) => set({ state: 'downloading', percent: Math.round(p.percent) }));
  autoUpdater.on('update-downloaded', (i) => set({ state: 'ready', next: i.version }));
  autoUpdater.on('error', (e) => set({ state: 'error', error: String(e?.message || e).split('\n')[0] }));

  const check = () => autoUpdater.checkForUpdates().catch(() => {});
  if (enabled()) setTimeout(check, 4000);
  setInterval(() => enabled() && check(), 30 * 60 * 1000);

  return {
    check,
    install: () => autoUpdater.quitAndInstall(false, true),
    status: () => status,
  };
}

module.exports = { init };

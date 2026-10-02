// System tray: closing the window keeps the music playing; the tray icon has playback controls.
// Optional "start with Windows/macOS" launches DK.FM hidden in the tray (paused).
const path = require('path');
const { app, Tray, Menu, nativeImage } = require('electron');

class TrayController {
  constructor({ getWindow, settings, emit }) {
    Object.assign(this, { getWindow, settings, emit });
    this.tray = null;
    this.now = { title: '', artist: '', playing: false };
    this.quitting = false;
  }

  init() {
    const base = nativeImage.createFromPath(path.join(__dirname, 'icon.png'));
    const size = process.platform === 'darwin' ? 18 : 16;
    const icon = base.resize({ width: size, height: size, quality: 'best' });
    const icon2x = base.resize({ width: size * 2, height: size * 2, quality: 'best' });
    icon.addRepresentation({ scaleFactor: 2, width: size * 2, height: size * 2, buffer: icon2x.toPNG() });
    this.tray = new Tray(icon);
    this.tray.setToolTip('DK.FM');
    this.tray.on('click', () => this.show());
    this.tray.on('double-click', () => this.show());
    this.render();
    app.on('before-quit', () => { this.quitting = true; });
  }

  // Window "close" -> hide to tray (unless the user really quits or turned this off).
  attach(win) {
    win.on('close', (e) => {
      if (this.quitting || this.settings.get('closeToTray') === false) return;
      e.preventDefault();
      win.hide();
      if (process.platform === 'win32' && !this.settings.get('trayHintShown')) {
        this.settings.set('trayHintShown', true);
        this.tray?.displayBalloon({
          iconType: 'info',
          title: 'DK.FM is still running',
          content: 'Music keeps playing here. Right-click the tray icon for controls, or choose Quit to exit.',
        });
      }
    });
  }

  show() {
    const w = this.getWindow();
    if (!w) return;
    if (w.isMinimized()) w.restore();
    w.show();
    w.focus();
  }

  update(now) {
    this.now = { ...this.now, ...now };
    this.render();
  }

  render() {
    if (!this.tray) return;
    const { title, artist, playing } = this.now;
    const label = title ? `${title} — ${artist}`.slice(0, 60) : 'Nothing playing';
    this.tray.setToolTip(title ? `DK.FM · ${label}` : 'DK.FM');
    const cmd = (c) => () => this.emit('tray:cmd', c);
    this.tray.setContextMenu(Menu.buildFromTemplate([
      { label, enabled: false },
      { type: 'separator' },
      { label: playing ? 'Pause' : 'Play', click: cmd('toggle') },
      { label: 'Next', click: cmd('next') },
      { label: 'Previous', click: cmd('prev') },
      { type: 'separator' },
      { label: 'Show DK.FM', click: () => this.show() },
      { label: 'Quit DK.FM', click: () => { this.quitting = true; app.quit(); } },
    ]));
  }
}

// Start at login (hidden in the tray). Not available on Linux through Electron.
function setStartAtLogin(on) {
  if (process.platform === 'linux') return false;
  app.setLoginItemSettings({ openAtLogin: !!on, openAsHidden: true, args: ['--hidden'] });
  return true;
}

const launchedHidden = () => process.argv.includes('--hidden') || app.getLoginItemSettings().wasOpenedAsHidden;

module.exports = { TrayController, setStartAtLogin, launchedHidden };

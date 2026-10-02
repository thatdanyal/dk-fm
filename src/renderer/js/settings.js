// Settings modal.
import { state } from './state.js';
import { player } from './player.js';
import { h, modal, toast } from './util.js';
import { THEMES, applyTheme } from './theme.js';
import { resetLayout } from './layout.js';
import { setFps } from './visualizer.js';

const TABS = [
  ['appearance', 'Appearance'],
  ['library', 'Library'],
  ['downloads', 'Downloads'],
  ['spotify', 'Spotify'],
  ['playback', 'Playback'],
  ['updates', 'Updates'],
  ['keys', 'Shortcuts'],
  ['about', 'About'],
];

export function openSettings(tab = 'appearance') {
  const body = h('div.set-body');
  const tabs = h('div.set-tabs');
  const show = (id) => {
    tab = id;
    tabs.replaceChildren(...TABS.map(([k, label]) => h('button.set-tab' + (k === id ? '.active' : ''), { on: { click: () => show(k) } }, label)));
    body.replaceChildren(...SECTIONS[id]());
    body.scrollTop = 0;
  };
  show(tab);
  modal('SETTINGS', h('div.modal-body', tabs, body));
}

const section = (title, ...kids) => h('div.set-section', h('h3', title), ...kids);
const row = (label, ...kids) => h('div.set-row', h('label', label), ...kids);
const toggle = (key, label, onChange) => h('label.switch', h('input', { type: 'checkbox', checked: !!state.settings[key], on: { change: (e) => { state.set(key, e.target.checked); onChange?.(e.target.checked); } } }), h('span'), label);

const SECTIONS = {
  appearance() {
    const grid = h('div.theme-grid');
    const draw = () => grid.replaceChildren(...Object.entries(THEMES).map(([id, t]) =>
      h('button.theme-card' + (state.settings.theme === id ? '.active' : ''), { on: { click: () => { state.set('theme', id); applyTheme(); draw(); } } },
        h('div.tc-prev', { style: { background: t.bg } }, ...t.swatch.map((c, i) => h('i', { style: { background: c, height: 14 + i * 10 + 'px' } }))),
        h('div.tc-name', t.name))));
    draw();
    const accent = h('input', { type: 'color', value: state.settings.accent || THEMES[state.settings.theme]?.swatch[0] || '#ff3b2f', on: { input: (e) => { state.set('accent', e.target.value); applyTheme(); } } });
    return [
      section('THEME', grid),
      section('ACCENT COLOR', row('Custom accent', accent, h('button.mini-btn', { on: { click: () => { state.set('accent', null); applyTheme(); } } }, 'USE THEME DEFAULT'))),
      section('CRT EFFECTS',
        h('div.set-row', toggle('scanlines', 'SCANLINES + VIGNETTE', applyTheme)),
        h('div.set-row', toggle('glow', 'PHOSPHOR GLOW', applyTheme)),
        h('div.set-row', toggle('pixelFont', 'PIXEL FONT', applyTheme))),
      section('LAYOUT', h('div.set-row', h('button.btn', { on: { click: () => { resetLayout(); toast('Layout reset'); } } }, 'RESET PANEL LAYOUT'))),
    ];
  },

  library() {
    const list = h('div.folder-list');
    const draw = () => list.replaceChildren(...state.settings.musicFolders.map((f) =>
      h('div.folder-item', h('span', f), h('button.mini-btn', { on: { click: () => { state.set('musicFolders', state.settings.musicFolders.filter((x) => x !== f)); draw(); } } }, 'REMOVE'))),
      state.settings.musicFolders.length ? '' : h('div.dim', 'No folders yet.'));
    draw();
    const status = h('span.dim');
    const scan = async () => {
      status.textContent = 'Scanning…';
      await new Promise((r) => setTimeout(r, 350)); // let the debounced settings save land
      await dk.library.scan();
      status.textContent = `Done — ${state.tracks.size} tracks.`;
    };
    return [
      section('MUSIC FOLDERS',
        h('div.dim', { style: { marginBottom: '8px' } }, 'DK.FM scans these folders (and your download folder) for audio files.'),
        list,
        h('div.set-row',
          h('button.btn.primary', { on: { click: async () => { const f = await dk.dialog.folder(); if (f && !state.settings.musicFolders.includes(f)) { state.set('musicFolders', [...state.settings.musicFolders, f]); draw(); scan(); } } } }, '+ ADD FOLDER'),
          h('button.btn', { on: { click: scan } }, 'RESCAN NOW'),
          status)),
      section('FILES', h('div.set-row', h('button.btn', { on: { click: () => dk.dialog.files() } }, 'ADD INDIVIDUAL FILES…')), h('div.dim', 'Tip: you can also drag audio files straight onto the window.')),
    ];
  },

  downloads() {
    const dir = h('input.pixel-input', { value: state.settings.downloadDir, readOnly: true });
    const ver = h('span.dim', '…');
    dk.yt.status().then((s) => (ver.textContent = s.version ? `yt-dlp ${s.version}` : s.state));
    return [
      section('DOWNLOAD FOLDER', row('Save to', dir, h('button.mini-btn', { on: { click: async () => { const f = await dk.dialog.folder(); if (f) { state.set('downloadDir', f); dir.value = f; } } } }, 'CHANGE'))),
      section('FORMAT',
        row('Audio format', h('select.pixel-select', { on: { change: (e) => state.set('downloadFormat', e.target.value) } },
          [['mp3-320', 'MP3 · 320 kbps CBR (max compatibility)'], ['mp3-v0', 'MP3 · V0 VBR (~245 kbps, smaller)'], ['m4a', 'M4A · AAC (no re-encode when possible)']].map(([v, l]) => h('option', { value: v, selected: state.settings.downloadFormat === v }, l)))),
        row('Parallel downloads', h('select.pixel-select', { on: { change: (e) => state.set('downloadConcurrency', Number(e.target.value)) } },
          [1, 2, 3, 4, 5, 6].map((n) => h('option', { value: n, selected: state.settings.downloadConcurrency === n }, String(n)))))),
      section('ENGINE', row('Downloader', ver), h('div.dim', 'yt-dlp is fetched automatically on first use and updated daily so YouTube changes don’t break downloads.')),
    ];
  },

  spotify() {
    const id = h('input.pixel-input', { value: state.settings.spotifyClientId, placeholder: 'Client ID', on: { input: (e) => state.set('spotifyClientId', e.target.value.trim()) } });
    const secret = h('input.pixel-input', { type: 'password', value: state.settings.spotifyClientSecret, placeholder: 'Client Secret', on: { input: (e) => state.set('spotifyClientSecret', e.target.value.trim()) } });
    return [
      section('SPOTIFY API KEYS (OPTIONAL)',
        h('div.dim', { style: { marginBottom: '10px' } }, 'Without keys, DK.FM reads Spotify’s public embed page: no setup, but capped at 100 tracks and no album names. With free developer keys you get full playlists, album names, track numbers and HD cover art.'),
        row('Client ID', id),
        row('Client Secret', secret)),
      section('HOW TO GET KEYS (2 MINUTES)',
        h('ol', { style: { margin: 0, paddingLeft: '22px', lineHeight: 1.4 } },
          h('li', 'Open ', h('a', { style: { color: 'var(--accent-2)', cursor: 'pointer' }, on: { click: () => dk.shell.openExternal('https://developer.spotify.com/dashboard') } }, 'developer.spotify.com/dashboard'), ' and log in.'),
          h('li', 'Click "Create app". Any name/description; Redirect URI: http://127.0.0.1:8888/callback; tick "Web API".'),
          h('li', 'Open the app → Settings → copy the Client ID and Client Secret here.'))),
    ];
  },

  playback() {
    return [
      section('PLAYBACK',
        row('Crossfade', h('select.pixel-select', { on: { change: (e) => player.setCrossfade(Number(e.target.value)) } }, [0, 2, 4, 6, 8, 10, 12].map((n) => h('option', { value: n, selected: (player.opts.crossfade || 0) === n }, n ? `${n} seconds` : 'Off (gapless)')))),
        row('Visualizer FPS', h('select.pixel-select', { on: { change: (e) => { setFps(Number(e.target.value)); player._savePlayer(); } } }, [[30, '30 fps (light, default)'], [60, '60 fps (smooth)'], [15, '15 fps (battery saver)']].map(([v, l]) => h('option', { value: v, selected: (player.opts.visFps || 30) === v }, l)))),
        row('Leveler', h('label.switch', h('input', { type: 'checkbox', checked: !!player.opts.normalize, on: { change: (e) => player.setNormalize(e.target.checked) } }), h('span'), 'EVEN OUT LOUD/QUIET TRACKS'))),
    ];
  },

  updates() {
    const st = h('div.dim', '…');
    const refresh = () => dk.updater.status().then((s) => {
      st.textContent = {
        dev: 'Running from source — auto-update is active in installed builds only.',
        idle: 'Idle.',
        checking: 'Checking GitHub for a new version…',
        latest: 'You’re on the latest version.',
        downloading: `Downloading v${s.next} … ${s.percent || 0}%`,
        ready: `v${s.next} is ready — restart to install.`,
        error: `Update check failed: ${s.error}`,
      }[s.state] || s.state;
    });
    refresh();
    return [
      section('AUTO UPDATE',
        h('div.set-row', toggle('autoUpdate', 'AUTOMATICALLY UPDATE FROM GITHUB')),
        row('Current version', h('span', 'v' + state.settings.version)),
        row('Status', st),
        h('div.set-row',
          h('button.btn', { on: { click: async () => { await dk.updater.check(); setTimeout(refresh, 1500); } } }, 'CHECK NOW'),
          h('button.btn', { on: { click: () => dk.shell.openExternal('https://github.com/thatdanyal/dk-fm/releases') } }, 'RELEASE NOTES'))),
    ];
  },

  keys() {
    const keys = [
      ['Space', 'Play / pause'], ['← / →', 'Seek 5s (Shift: 30s)'], ['Ctrl ← / →', 'Previous / next track'], ['↑ / ↓', 'Volume'],
      ['M', 'Mute'], ['S', 'Shuffle'], ['R', 'Repeat mode'], ['L', 'Like current track'], ['V', 'Cycle visualizer'],
      ['Ctrl F', 'Search library'], ['Ctrl E', 'Edit layout'], ['Ctrl M', 'Mini player'], ['Ctrl ,', 'Settings'], ['Ctrl I', 'Import from Spotify'],
    ];
    return [section('KEYBOARD', h('div.keys', keys.flatMap(([k, d]) => [h('kbd', k), h('span', d)]))), h('div.dim', 'Media keys on your keyboard / headphones work too.')];
  },

  about() {
    return [section('DK.FM', h('div.about',
      h('div', 'v' + state.settings.version + ' · ' + state.settings.platform),
      (() => { const g = h('div', 'Graphics: …'); dk.gpu().then((s) => (g.textContent = 'Graphics: ' + (String(s).startsWith('enabled') ? 'GPU accelerated' : 'Software (no GPU) — lower the visualizer FPS if it feels heavy'))); return g; })(),
      h('p', 'Retro desktop music player. Plays your local library, imports Spotify playlists by finding the best YouTube Music match for every song.'),
      h('p', 'Lyrics from LRCLIB · Downloads powered by yt-dlp + FFmpeg.'),
      h('p', 'Only download music you have the rights to.')))];
  },
};

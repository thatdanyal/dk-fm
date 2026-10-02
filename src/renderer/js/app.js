import { state } from './state.js';
import { player } from './player.js';
import { $, h, toast, modal } from './util.js';
import { applyTheme, THEMES, themeEvents } from './theme.js';
import { initLayout, setEditing, isEditing, PANELS, isHidden, setHidden, showPanel, layoutEvents } from './layout.js';
import { initDeck, drawWave, invalidateWave } from './deck.js';
import { initVisualizer, refreshColors, cycleMode } from './visualizer.js';
import { initQueue } from './queue.js';
import { initEq, drawCurve } from './eq.js';
import { initLyrics } from './lyrics.js';
import { initBrowser, setView, focusSearch } from './browser.js';
import { openSettings } from './settings.js';
import { initPalette } from './palette.js';

async function boot() {
  await state.load();
  document.body.classList.add('platform-' + state.settings.platform);
  applyTheme();
  initLayout();
  player.init();
  initDeck();
  initVisualizer();
  initQueue();
  initEq();
  initLyrics();
  initBrowser({ onOpenSettings: openSettings });
  initTitlebar();
  initShortcuts();
  initDragDrop();
  initUpdater();
  initPalette({ toggleMini });
  initTray();
  themeEvents.on('change', () => { refreshColors(); invalidateWave(); drawCurve(); });
  layoutEvents.on('render', () => requestAnimationFrame(() => { drawWave(); drawCurve(); }));

  dk.on('library:progress', (p) => {
    $('#tb-status').textContent = p.finished ? '' : `SCANNING LIBRARY… ${p.done}/${p.total}`;
  });
  // Rescan on launch to pick up files added while the app was closed.
  setTimeout(() => dk.library.scan(), 800);
}

function initTitlebar() {
  $('#win-ctrls').addEventListener('click', (e) => {
    const b = e.target.closest('[data-act]');
    if (b) dk.win[b.dataset.act]();
  });
  $('#titlebar').addEventListener('dblclick', (e) => { if (!e.target.closest('button')) dk.win.maximize(); });
  $('#btn-layout').onclick = () => setEditing(!isEditing());
  $('#btn-settings').onclick = () => openSettings();
  $('#btn-mini').onclick = toggleMini;

  const dropdown = (btn, menu, build) => {
    btn.onclick = (e) => {
      e.stopPropagation();
      const dd = btn.parentElement;
      const open = !dd.classList.contains('open');
      document.querySelectorAll('.dropdown.open').forEach((d) => d.classList.remove('open'));
      if (open) { build(menu); dd.classList.add('open'); }
    };
  };
  addEventListener('click', () => document.querySelectorAll('.dropdown.open').forEach((d) => d.classList.remove('open')));
  dropdown($('#btn-panels'), $('#panels-menu'), (menu) => {
    menu.replaceChildren(...Object.entries(PANELS).map(([id, name]) =>
      h('button.menu-item', { on: { click: () => setHidden(id, !isHidden(id)) } }, h('span.check', isHidden(id) ? '' : '✓'), name)));
  });
  dropdown($('#btn-theme'), $('#theme-menu'), (menu) => {
    menu.replaceChildren(...Object.entries(THEMES).map(([id, t]) =>
      h('button.menu-item', { on: { click: () => { state.set('theme', id); applyTheme(); } } },
        h('span.check', state.settings.theme === id ? '●' : ''), h('span.swatch', { style: { background: t.swatch[0] } }), t.name)),
      h('div', { style: { height: '2px', background: 'var(--line)', margin: '4px 0' } }),
      h('button.menu-item', { on: { click: () => openSettings('appearance') } }, h('span.check', ''), 'More options…'));
  });
  dk.on('win:state', ({ maximized }) => document.body.classList.toggle('maximized', maximized));
}

let mini = false;
async function toggleMini() {
  mini = !mini;
  if (mini && isEditing()) setEditing(false);
  document.body.classList.toggle('mini', mini);
  $('#btn-mini').classList.toggle('on', mini);
  $('#btn-mini').textContent = mini ? 'FULL' : 'MINI';
  if (mini) showPanel('deck');
  await dk.win.mini(mini);
  requestAnimationFrame(drawWave);
}

function initShortcuts() {
  addEventListener('keydown', (e) => {
    const typing = e.target.closest?.('input, textarea, select') && e.target.type !== 'range' && e.target.type !== 'checkbox';
    const mod = e.ctrlKey || e.metaKey;
    const k = e.key;
    if (mod && k === ',') { e.preventDefault(); return openSettings(); }
    if (mod && k.toLowerCase() === 'f') { e.preventDefault(); showPanel('browser'); return focusSearch(); }
    if (mod && k.toLowerCase() === 'e') { e.preventDefault(); return setEditing(!isEditing()); }
    if (mod && k.toLowerCase() === 'm') { e.preventDefault(); return toggleMini(); }
    if (mod && k.toLowerCase() === 'i') { e.preventDefault(); showPanel('browser'); return setView({ key: 'import' }); }
    if (typing || document.querySelector('.modal-back')) return;
    if (k === ' ') { e.preventDefault(); return player.toggle(); }
    if (mod && k === 'ArrowRight') return player.next(true);
    if (mod && k === 'ArrowLeft') return player.prev();
    if (k === 'ArrowRight') return player.seek(player.time + (e.shiftKey ? 30 : 5));
    if (k === 'ArrowLeft') return player.seek(player.time - (e.shiftKey ? 30 : 5));
    if (e.target.closest?.('.tracks')) return; // let lists use arrows/enter
    if (k === 'ArrowUp') { e.preventDefault(); return player.setVolume(player.opts.volume + 0.05); }
    if (k === 'ArrowDown') { e.preventDefault(); return player.setVolume(player.opts.volume - 0.05); }
    if (mod || e.altKey) return;
    switch (k.toLowerCase()) {
      case 'm': return player.toggleMute();
      case 's': return player.toggleShuffle();
      case 'r': return player.cycleRepeat();
      case 'l': return player.current && state.like(player.current.id);
      case 'v': return cycleMode();
    }
  });
}

// Keep the tray menu/tooltip in step with playback, and obey its buttons.
function initTray() {
  dk.on('win:visible', (v) => {
    window.__dkHidden = !v;
    document.dispatchEvent(new Event('visibilitychange')); // wakes the visualizer when shown again
    if (v) requestAnimationFrame(drawWave);
  });
  const send = () => {
    const t = player.current;
    dk.tray.update({ title: t?.title || '', artist: t?.artist || '', playing: player.playing });
  };
  player.on('track', send);
  player.on('state', send);
  send();
  dk.on('tray:cmd', (c) => ({ toggle: () => player.toggle(), next: () => player.next(true), prev: () => player.prev() })[c]?.());
}

function initDragDrop() {
  let depth = 0;
  const overlay = $('#drop-overlay');
  const isFiles = (e) => e.dataTransfer?.types.includes('Files');
  addEventListener('dragenter', (e) => { if (isFiles(e)) { depth++; overlay.classList.remove('hidden'); } });
  addEventListener('dragleave', (e) => { if (isFiles(e) && --depth <= 0) { depth = 0; overlay.classList.add('hidden'); } });
  addEventListener('dragover', (e) => { if (isFiles(e)) e.preventDefault(); });
  addEventListener('drop', async (e) => {
    if (!isFiles(e)) return;
    e.preventDefault();
    depth = 0;
    overlay.classList.add('hidden');
    const paths = [...e.dataTransfer.files].map((f) => dk.pathForFile(f)).filter(Boolean);
    const added = await dk.library.addFiles(paths);
    if (!added.length) return toast('No playable audio files found', { error: true });
    toast(`Added ${added.length} track${added.length === 1 ? '' : 's'}`);
    const ids = added.map((t) => t.id);
    player.index < 0 ? player.playList(ids, 0) : player.enqueue(ids);
  });
}

// "Ask, then install": a popup announces each new release with its notes. LATER keeps a
// pill in the title bar (and the popup returns next launch); DOWNLOAD & RESTART installs it.
function initUpdater() {
  const pill = $('#btn-update');
  const isMac = state.settings.platform === 'darwin';
  let popup = null; // { close, render }
  const dismissed = new Set();
  let last = {};

  const openPopup = (s) => {
    if (popup) return popup.render(s);
    const body = h('div.modal-pad');
    const foot = h('div.modal-foot');
    const render = (st) => {
      const busy = st.state === 'downloading';
      body.replaceChildren(...[
        h('div', { style: { fontSize: '24px' } }, `DK.FM v${st.next}`, h('span.dim', `  (you have v${state.settings.version})`)),
        st.notes ? h('div', h('div.px', { style: { color: 'var(--accent-2)', margin: '6px 0' } }, "WHAT'S NEW"),
          h('div', { style: { whiteSpace: 'pre-wrap', maxHeight: '220px', overflowY: 'auto', color: 'var(--text)' } }, st.notes)) : null,
        busy ? h('div', h('div.job-bar', h('i', { style: { width: (st.percent || 0) + '%' } })), h('div.dim', `Downloading… ${st.percent || 0}% — DK.FM will restart when done.`)) : null,
        st.state === 'error' ? h('div.warn', 'Update failed: ' + st.error) : null,
        isMac ? h('div.dim', 'macOS: this opens the download page — install the new .dmg over the old app.') : null].filter(Boolean));
      foot.replaceChildren(
        h('button.btn', { disabled: busy, on: { click: later } }, 'LATER'),
        h('button.btn.primary', { disabled: busy, on: { click: () => dk.updater.download() } }, isMac ? 'GET UPDATE' : st.state === 'error' ? 'TRY AGAIN' : 'DOWNLOAD & RESTART'));
    };
    const later = () => { dismissed.add(last.next); popup?.close(); };
    render(s);
    const close = modal('UPDATE AVAILABLE', h('div', body, foot), { small: true, onClose: () => { dismissed.add(last.next); popup = null; } });
    popup = { close, render };
  };

  const render = (s) => {
    last = { ...last, ...s };
    const show = ['available', 'downloading', 'ready'].includes(s.state) || (s.state === 'error' && last.next);
    pill.classList.toggle('hidden', !show);
    if (s.state === 'available') pill.textContent = `UPDATE > v${s.next}`;
    if (s.state === 'downloading') pill.textContent = `UPDATING ${s.percent || 0}%`;
    if (s.state === 'ready') pill.textContent = `RESTART > v${s.next}`;
    if (s.state === 'available' && !dismissed.has(s.next)) openPopup(last);
    else popup?.render(last);
  };
  pill.onclick = () => (last.state === 'ready' ? dk.updater.install() : openPopup(last));
  dk.on('updater:status', render);
  dk.updater.status().then(render);
}

boot().catch((err) => {
  console.error(err);
  document.body.append(h('pre', { style: { color: 'red', padding: '20px', position: 'fixed', inset: '40px 0 0 0', background: '#000', zIndex: 10000, whiteSpace: 'pre-wrap' } }, 'DK.FM failed to start:\n' + (err.stack || err)));
});

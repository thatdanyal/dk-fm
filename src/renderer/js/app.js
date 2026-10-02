import { state } from './state.js';
import { player } from './player.js';
import { $, h, toast } from './util.js';
import { applyTheme, THEMES, themeEvents } from './theme.js';
import { initLayout, setEditing, isEditing, PANELS, isHidden, setHidden, showPanel, layoutEvents } from './layout.js';
import { initDeck, drawWave } from './deck.js';
import { initVisualizer, refreshColors, cycleMode } from './visualizer.js';
import { initQueue } from './queue.js';
import { initEq, drawCurve } from './eq.js';
import { initLyrics } from './lyrics.js';
import { initBrowser, setView, focusSearch } from './browser.js';
import { openSettings } from './settings.js';

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
  themeEvents.on('change', () => { refreshColors(); drawWave(); drawCurve(); });
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
    const typing = e.target.closest('input, textarea, select') && e.target.type !== 'range' && e.target.type !== 'checkbox';
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
    if (e.target.closest('.tracks')) return; // let lists use arrows/enter
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

function initUpdater() {
  const pill = $('#btn-update');
  const render = (s) => {
    pill.classList.toggle('hidden', !['downloading', 'ready'].includes(s.state));
    if (s.state === 'downloading') pill.textContent = `UPDATING ${s.percent || 0}%`;
    if (s.state === 'ready') {
      pill.textContent = `RESTART → v${s.next}`;
      if (!initUpdater.notified) { initUpdater.notified = true; toast(`DK.FM v${s.next} downloaded — click the orange button to restart.`); }
    }
  };
  pill.onclick = () => dk.updater.install();
  dk.on('updater:status', render);
  dk.updater.status().then(render);
}

boot().catch((err) => {
  console.error(err);
  document.body.append(h('pre', { style: { color: 'red', padding: '20px', position: 'fixed', inset: '40px 0 0 0', background: '#000', zIndex: 10000, whiteSpace: 'pre-wrap' } }, 'DK.FM failed to start:\n' + (err.stack || err)));
});

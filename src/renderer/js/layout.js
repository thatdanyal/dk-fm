// Dockable panel layout: columns of stacked panels. Drag headers to move (in LAYOUT mode),
// drag the gaps to resize, collapse or hide any panel. Persisted in settings.
import { state } from './state.js';
import { $, $$, h, Emitter } from './util.js';

export const PANELS = {
  deck: 'Deck',
  scope: 'Scope',
  browser: 'Library',
  queue: 'Queue',
  eq: 'Equalizer',
  lyrics: 'Lyrics',
};

const DEFAULT = () => ({
  columns: [
    { width: 360, panels: [{ id: 'deck', flex: 0 }, { id: 'scope', flex: 1 }, { id: 'eq', flex: 1.4, collapsed: true }] },
    { width: null, panels: [{ id: 'browser', flex: 1 }] },
    { width: 300, panels: [{ id: 'queue', flex: 1.4 }, { id: 'lyrics', flex: 1 }] },
  ],
  hidden: [],
});

export const layoutEvents = new Emitter();
let layout;
let editing = false;
let dragId = null;
const sections = {};

export function initLayout() {
  for (const sec of $$('#panel-store .panel')) {
    const id = sec.dataset.panel;
    const body = h('div.panel-body', ...sec.childNodes);
    const head = h('div.panel-head',
      h('span.panel-title', sec.dataset.title),
      h('div.panel-tools',
        h('button.panel-tool', { title: 'Collapse', on: { click: () => toggleCollapse(id) } }, '▾'),
        h('button.panel-tool', { title: 'Hide panel', on: { click: () => setHidden(id, true) } }, '×')));
    head.addEventListener('dblclick', () => toggleCollapse(id));
    head.addEventListener('dragstart', (e) => {
      if (!editing) return e.preventDefault();
      dragId = id;
      e.dataTransfer.setData('text/x-dkfm-panel', id);
      e.dataTransfer.effectAllowed = 'move';
      setTimeout(() => sec.classList.add('dragging'));
    });
    head.addEventListener('dragend', () => { sec.classList.remove('dragging'); dragId = null; clearMarkers(); });
    sec.replaceChildren(head, body);
    sections[id] = sec;
  }
  layout = sanitize(state.settings.layout) || DEFAULT();
  render();
}

function sanitize(l) {
  if (!l?.columns?.length) return null;
  const seen = new Set();
  for (const c of l.columns) c.panels = (c.panels || []).filter((p) => PANELS[p.id] && !seen.has(p.id) && seen.add(p.id));
  l.hidden = (l.hidden || []).filter((id) => PANELS[id] && !seen.has(id));
  // Panels added in newer versions land in the last column.
  for (const id of Object.keys(PANELS)) if (!seen.has(id) && !l.hidden.includes(id)) l.columns[l.columns.length - 1].panels.push({ id, flex: 1 });
  if (!l.columns.some((c) => c.width == null)) l.columns[Math.floor(l.columns.length / 2)].width = null;
  return l;
}

function save() {
  state.set('layout', structuredClone(layout));
  layoutEvents.emit('change');
}

export function resetLayout() {
  layout = DEFAULT();
  save();
  render();
}

export function isHidden(id) { return layout.hidden.includes(id); }

export function setHidden(id, hide) {
  if (hide) {
    for (const c of layout.columns) c.panels = c.panels.filter((p) => p.id !== id);
    if (!layout.hidden.includes(id)) layout.hidden.push(id);
  } else {
    layout.hidden = layout.hidden.filter((x) => x !== id);
    const col = layout.columns.find((c) => c.width != null) || layout.columns[0];
    col.panels.push({ id, flex: 1 });
  }
  prune();
  save();
  render();
}

export function showPanel(id) {
  if (isHidden(id)) setHidden(id, false);
  const p = findPanel(id);
  if (p?.collapsed) toggleCollapse(id);
}

function findPanel(id) {
  for (const c of layout.columns) for (const p of c.panels) if (p.id === id) return p;
  return null;
}

function toggleCollapse(id) {
  const p = findPanel(id);
  if (!p) return;
  p.collapsed = !p.collapsed;
  save();
  render();
}

export function setEditing(on) {
  editing = on;
  document.body.classList.toggle('layout-edit', on);
  $('#btn-layout').classList.toggle('on', on);
  if (!on) prune();
  render();
  $('.layout-banner')?.remove();
  if (on) {
    document.body.append(h('div.layout-banner', 'DRAG PANEL HEADERS TO MOVE · DRAG GAPS TO RESIZE',
      h('button', { on: { click: () => { resetLayout(); } } }, 'RESET'),
      h('button', { on: { click: () => setEditing(false) } }, 'DONE')));
  } else save();
}
export const isEditing = () => editing;

function prune() {
  layout.columns = layout.columns.filter((c) => c.panels.length);
  if (!layout.columns.length) layout.columns = DEFAULT().columns;
  if (!layout.columns.some((c) => c.width == null)) {
    const withBrowser = layout.columns.find((c) => c.panels.some((p) => p.id === 'browser'));
    (withBrowser || layout.columns[Math.floor(layout.columns.length / 2)]).width = null;
  }
}

function render() {
  const ws = $('#workspace');
  // Park sections so replaceChildren doesn't destroy them.
  const store = $('#panel-store');
  Object.values(sections).forEach((s) => store.append(s));
  const cols = editing ? [{ width: 90, panels: [], phantom: true }, ...layout.columns, { width: 90, panels: [], phantom: true }] : layout.columns;
  const nodes = [];
  cols.forEach((col, ci) => {
    if (ci > 0) nodes.push(colSplitter(cols, ci - 1, ci));
    const el = h('div.column' + (col.width == null ? '.fill' : '') + (col.panels.length ? '' : '.empty') + (editing ? '.editing' : ''));
    if (col.width != null) el.style.width = col.width + 'px';
    if (col.panels.some((p) => p.id === 'deck')) el.classList.add('mini-host');
    col.panels.forEach((p, pi) => {
      const sec = sections[p.id];
      sec.classList.remove('dragging');
      sec.classList.toggle('collapsed', !!p.collapsed);
      sec.style.flex = p.collapsed ? 'none' : p.flex ? `${p.flex} 1 0` : 'none';
      sec.querySelector('.panel-head').draggable = editing;
      sec.querySelector('.panel-tool').textContent = p.collapsed ? '▸' : '▾';
      if (pi > 0) el.append(rowSplitter(col, pi - 1, pi, el));
      el.append(sec);
    });
    if (editing) attachDrop(el, col, cols);
    nodes.push(el);
  });
  ws.replaceChildren(...nodes);
  layoutEvents.emit('render');
}

function colSplitter(cols, a, b) {
  const s = h('div.col-splitter');
  s.addEventListener('pointerdown', (e) => {
    const left = cols[a];
    const right = cols[b];
    const target = left.width != null ? left : right;
    const sign = target === left ? 1 : -1;
    const start = e.clientX;
    const w0 = target.width;
    s.setPointerCapture(e.pointerId);
    s.classList.add('drag');
    const move = (ev) => {
      target.width = Math.max(200, Math.min(900, w0 + (ev.clientX - start) * sign));
      const idx = cols.indexOf(target);
      $$('#workspace > .column')[idx].style.width = target.width + 'px';
    };
    const up = () => { s.classList.remove('drag'); s.removeEventListener('pointermove', move); s.removeEventListener('pointerup', up); save(); };
    s.addEventListener('pointermove', move);
    s.addEventListener('pointerup', up);
  });
  return s;
}

function rowSplitter(col, a, b, colEl) {
  const s = h('div.row-splitter');
  s.addEventListener('pointerdown', (e) => {
    const pa = col.panels[a];
    const pb = col.panels[b];
    if (pa.collapsed || pb.collapsed) return;
    const ea = sections[pa.id];
    const eb = sections[pb.id];
    // Freeze every flexible panel in this column to its pixel height so flex ratios stay stable.
    for (const p of col.panels) if (p.flex && !p.collapsed) p.flex = sections[p.id].getBoundingClientRect().height;
    const ha = ea.getBoundingClientRect().height;
    const hb = eb.getBoundingClientRect().height;
    const start = e.clientY;
    s.setPointerCapture(e.pointerId);
    s.classList.add('drag');
    const move = (ev) => {
      const d = ev.clientY - start;
      const na = Math.max(60, ha + d);
      const nb = Math.max(60, hb - d);
      if (pa.flex) { pa.flex = na; ea.style.flex = `${na} 1 0`; }
      if (pb.flex) { pb.flex = nb; eb.style.flex = `${nb} 1 0`; }
      if (!pa.flex && pb.flex) { pb.flex = nb; eb.style.flex = `${nb} 1 0`; }
    };
    const up = () => { s.classList.remove('drag'); s.removeEventListener('pointermove', move); s.removeEventListener('pointerup', up); save(); };
    s.addEventListener('pointermove', move);
    s.addEventListener('pointerup', up);
  });
  return s;
}

function clearMarkers() { $$('.drop-marker').forEach((m) => m.remove()); }

function attachDrop(el, col, cols) {
  const indexAt = (y) => {
    const secs = [...el.querySelectorAll(':scope > .panel')];
    for (let i = 0; i < secs.length; i++) {
      const r = secs[i].getBoundingClientRect();
      if (y < r.top + r.height / 2) return i;
    }
    return secs.length;
  };
  el.addEventListener('dragover', (e) => {
    if (!dragId) return;
    e.preventDefault();
    const i = indexAt(e.clientY);
    clearMarkers();
    const secs = [...el.querySelectorAll(':scope > .panel')];
    const marker = h('div.drop-marker');
    if (i < secs.length) {
      const prev = secs[i].previousElementSibling;
      el.insertBefore(marker, prev?.classList.contains('row-splitter') ? prev : secs[i]);
    } else el.append(marker);
  });
  el.addEventListener('dragleave', (e) => { if (!el.contains(e.relatedTarget)) clearMarkers(); });
  el.addEventListener('drop', (e) => {
    e.preventDefault();
    if (!dragId) return;
    let i = indexAt(e.clientY);
    let moved;
    for (const c of cols) {
      const k = c.panels.findIndex((p) => p.id === dragId);
      if (k >= 0) {
        if (c === col && k < i) i--;
        moved = c.panels.splice(k, 1)[0];
      }
    }
    moved.flex = moved.id === 'deck' ? 0 : moved.flex || 1;
    col.panels.splice(i, 0, moved);
    if (col.phantom) {
      delete col.phantom;
      col.width = 300;
      layout.columns = cols.filter((c) => !c.phantom || c.panels.length);
    }
    layout.columns = layout.columns.filter((c) => c.panels.length || editing);
    dragId = null;
    clearMarkers();
    prune();
    render();
    save();
  });
}

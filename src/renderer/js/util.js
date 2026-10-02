export const $ = (s, root = document) => root.querySelector(s);
export const $$ = (s, root = document) => [...root.querySelectorAll(s)];

// Tiny hyperscript: h('div.cls#id', {attrs, on:{click}}, ...children)
export function h(tag, props, ...kids) {
  const [, name = 'div', rest = ''] = tag.match(/^([a-z0-9-]*)(.*)$/i);
  const node = document.createElement(name || 'div');
  for (const m of rest.matchAll(/([.#])([\w-]+)/g)) m[1] === '.' ? node.classList.add(m[2]) : (node.id = m[2]);
  if (props && (typeof props !== 'object' || props instanceof Node || Array.isArray(props))) {
    kids.unshift(props);
    props = null;
  }
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k === 'on') for (const [ev, fn] of Object.entries(v)) node.addEventListener(ev, fn);
    else if (k === 'style' && typeof v === 'object') Object.assign(node.style, v);
    else if (k === 'class') node.className += ' ' + v;
    else if (k === 'html') node.innerHTML = v;
    else if (k in node && typeof v !== 'string') node[k] = v;
    else node.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) if (kid != null && kid !== false) node.append(kid instanceof Node ? kid : String(kid));
  return node;
}

export const fmtTime = (s) => {
  if (!isFinite(s) || s < 0) s = 0;
  s = Math.floor(s);
  const hr = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, '0');
  return hr ? `${hr}:${String(m).padStart(2, '0')}:${sec}` : `${String(m).padStart(2, '0')}:${sec}`;
};

export const fmtLong = (s) => {
  const hr = Math.floor(s / 3600);
  const m = Math.round((s % 3600) / 60);
  return hr ? `${hr} HR ${m} MIN` : `${m} MIN`;
};

export const coverUrl = (t) => (t?.cover ? `dkfm://cover/${t.cover}` : '');
// Small (192px) version for lists and grids; falls back to the full cover.
export const thumbUrl = (t) => (t?.thumb ? `dkfm://cover/${t.thumb}` : coverUrl(t));
export const mediaUrl = (t) => `dkfm://media/${t.id}`;

export function debounce(fn, ms) {
  let id;
  return (...a) => {
    clearTimeout(id);
    id = setTimeout(() => fn(...a), ms);
  };
}

export class Emitter {
  constructor() { this._l = {}; }
  on(ev, fn) { (this._l[ev] ||= new Set()).add(fn); return () => this._l[ev].delete(fn); }
  emit(ev, ...a) { this._l[ev]?.forEach((fn) => fn(...a)); }
}

export function toast(msg, opts = {}) {
  const t = h('div.toast' + (opts.error ? '.err' : ''), msg);
  document.getElementById('toasts').append(t);
  setTimeout(() => t.remove(), opts.ms || (opts.error ? 6000 : 3000));
}

// Context menu. items: [{label, action, disabled, sub:[...]}, '-']
export function contextMenu(x, y, items) {
  const root = document.getElementById('ctx-menu');
  const build = (list) => list.map((it) => {
    if (it === '-') return h('div.sep');
    if (it.sub) {
      return h('div.sub', h('button.menu-item', it.label, h('span.spacer'), '▸'), h('div.ctx-menu', build(it.sub)));
    }
    return h('button.menu-item', { disabled: it.disabled, on: { click: () => { hide(); it.action(); } } }, it.label);
  });
  root.replaceChildren(...build(items));
  root.classList.remove('hidden');
  const r = root.getBoundingClientRect();
  root.style.left = Math.min(x, innerWidth - r.width - 8) + 'px';
  root.style.top = Math.min(y, innerHeight - r.height - 8) + 'px';
  const hide = () => {
    root.classList.add('hidden');
    removeEventListener('mousedown', outside, true);
  };
  const outside = (e) => { if (!root.contains(e.target)) hide(); };
  setTimeout(() => addEventListener('mousedown', outside, true));
}

export function modal(title, body, { small = false, onClose } = {}) {
  const close = () => { back.remove(); onClose?.(); removeEventListener('keydown', esc); };
  const esc = (e) => e.key === 'Escape' && close();
  const back = h('div.modal-back', { on: { mousedown: (e) => e.target === back && close() } },
    h('div.modal' + (small ? '.small' : ''),
      h('div.modal-head', h('h2', title), h('button.x', { on: { click: close } }, '×')),
      body));
  document.getElementById('modal-root').append(back);
  addEventListener('keydown', esc);
  return close;
}

export function prompt(title, value = '') {
  return new Promise((resolve) => {
    const input = h('input.pixel-input', { value, style: { width: '100%' } });
    let done = false;
    const finish = (v) => { if (!done) { done = true; close(); resolve(v); } };
    const close = modal(title, h('div',
      h('div.modal-pad', input),
      h('div.modal-foot',
        h('button.btn', { on: { click: () => finish(null) } }, 'CANCEL'),
        h('button.btn.primary', { on: { click: () => finish(input.value.trim() || null) } }, 'OK'))), { small: true, onClose: () => finish(null) });
    input.addEventListener('keydown', (e) => e.key === 'Enter' && finish(input.value.trim() || null));
    setTimeout(() => input.select());
  });
}

export function setRangeFill(input) {
  const min = Number(input.min || 0);
  const max = Number(input.max || 100);
  input.style.setProperty('--fill', ((input.value - min) / (max - min)) * 100 + '%');
}

export const cssVar = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

export const PLACEHOLDER_SVG = `<svg viewBox="0 0 16 16" shape-rendering="crispEdges"><g fill="currentColor"><rect x="3" y="2" width="10" height="12" fill="none" stroke="currentColor"/><rect x="5" y="5" width="2" height="2"/><rect x="9" y="5" width="2" height="2"/><rect x="5" y="9" width="6" height="1"/><rect x="4" y="11" width="8" height="1" opacity=".5"/></g></svg>`;

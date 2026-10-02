// Queue panel: history + upcoming, drag to reorder, save as playlist.
import { player } from './player.js';
import { state } from './state.js';
import { $, h, coverUrl, fmtLong, prompt, toast, contextMenu } from './util.js';

let dragFrom = null;

export function initQueue() {
  player.on('queue', render);
  state.on('library', render);
  $('#queue-clear').onclick = () => player.clearUpcoming();
  $('#queue-save').onclick = async () => {
    if (!player.queue.length) return;
    const name = await prompt('SAVE QUEUE AS PLAYLIST', 'Queue ' + new Date().toLocaleDateString());
    if (!name) return;
    await dk.library.upsertPlaylist({ name, trackIds: [...player.queue], source: 'user' });
    toast(`Saved playlist "${name}"`);
  };
  render();
}

function render() {
  const list = $('#queue-list');
  const q = player.queue;
  const upcoming = q.slice(player.index + 1).map((id) => state.track(id)).filter(Boolean);
  const secs = upcoming.reduce((s, t) => s + (t.duration || 0), 0);
  $('#queue-info').textContent = `${upcoming.length} UP NEXT · ${fmtLong(secs)}`;
  if (!q.length) {
    list.replaceChildren(h('div.empty', h('span.px', 'QUEUE EMPTY'), 'Double-click a track to start'));
    return;
  }
  // Show a little history, then everything upcoming (capped for very long queues).
  const from = Math.max(0, player.index - 3);
  const to = Math.min(q.length, player.index + 400);
  const items = [];
  for (let i = from; i < to; i++) {
    const t = state.track(q[i]);
    if (!t) continue;
    if (i === player.index + 1) items.push(h('li.q-sep', 'UP NEXT'));
    items.push(row(t, i));
  }
  if (to < q.length) items.push(h('li.q-sep', `+ ${q.length - to} MORE`));
  list.replaceChildren(...items);
  list.querySelector('.current')?.scrollIntoView({ block: 'nearest' });
}

function row(t, i) {
  const cls = i < player.index ? '.past' : i === player.index ? '.current' : '';
  const li = h('li.qi' + cls, {
    draggable: true,
    on: {
      dblclick: () => player.play(i),
      contextmenu: (e) => {
        e.preventDefault();
        contextMenu(e.clientX, e.clientY, [
          { label: 'Play now', action: () => player.play(i) },
          { label: 'Move to next', action: () => player.move(i, i > player.index ? player.index + 1 : player.index) , disabled: i <= player.index },
          { label: 'Remove from queue', action: () => player.removeAt(i) },
          '-',
          { label: 'Show in folder', action: () => dk.library.reveal(t.id) },
        ]);
      },
      dragstart: (e) => { dragFrom = i; li.classList.add('dragging'); e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/x-dkfm-queue', String(i)); },
      dragend: () => { li.classList.remove('dragging'); dragFrom = null; },
      dragover: (e) => { if (dragFrom == null) return; e.preventDefault(); li.classList.add('drag-over'); },
      dragleave: () => li.classList.remove('drag-over'),
      drop: (e) => {
        e.preventDefault();
        li.classList.remove('drag-over');
        if (dragFrom == null) return;
        player.move(dragFrom, dragFrom < i ? i - 1 : i);
      },
    },
  },
    t.cover ? h('img', { src: coverUrl(t), loading: 'lazy' }) : h('div.qph'),
    h('div.qtxt', h('div.qt', t.title), h('div.qa', t.artist)),
    h('button.qx', { title: 'Remove', on: { click: (e) => { e.stopPropagation(); player.removeAt(i); } } }, '×'));
  return li;
}

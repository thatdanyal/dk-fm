// Auto-sync: imported playlists (Spotify / YouTube / SoundCloud) are re-checked in the background.
// New songs download automatically; order and removals mirror the source.
const { fetchAny } = require('./sources');
const yt = require('./ytdlp');
const { taKey } = require('./library');

class Sync {
  constructor({ library, downloader, settings, emit }) {
    Object.assign(this, { library, downloader, settings, emit });
    this.running = false;
    this.timer = null;
  }

  start() {
    // First pass a minute after launch (so startup stays fast), then on the chosen interval.
    setTimeout(() => this.runAll(), 60 * 1000);
    this.schedule();
  }

  schedule() {
    clearInterval(this.timer);
    const hours = Number(this.settings.get('syncHours') ?? 6);
    if (hours > 0) this.timer = setInterval(() => this.runAll(), hours * 3600 * 1000);
  }

  creds() {
    return { clientId: this.settings.get('spotifyClientId'), clientSecret: this.settings.get('spotifyClientSecret') };
  }

  syncable(manual) {
    return this.library.store.data.playlists.filter((p) => p.sourceUrl || p.spotifyUrl).filter((p) => manual || p.autoSync !== false);
  }

  async runAll({ manual = false } = {}) {
    if (this.running) return { busy: true };
    if (!manual && Number(this.settings.get('syncHours') ?? 6) <= 0) return { off: true };
    const list = this.syncable(manual);
    if (!list.length) return { added: 0 };
    this.running = true;
    let added = 0;
    try {
      // Tools only matter if the user has imported before; they are already installed then.
      await yt.ensure().catch(() => {});
      for (const p of list) added += await this.syncOne(p).catch(() => 0);
    } finally {
      this.running = false;
    }
    this.emit('sync:done', { added, manual });
    return { added };
  }

  async syncOne(p) {
    const url = p.sourceUrl || p.spotifyUrl;
    const jobId = p.id.replace(/^sp-/, '');
    const job = this.downloader.jobs.get(jobId);
    if (job && job.tracks.some((t) => ['queued', 'searching', 'downloading', 'tagging'].includes(t.status))) return 0;
    const col = await fetchAny(url, this.creds());
    if (col.kind === 'profile') return 0;
    const index = this.library.keyIndex();
    const have = (t) => index.has(t.sourceKey) || (t.spotifyId && index.has('sp:' + t.spotifyId)) || index.has(taKey(t.artists[0], t.title));
    const missing = col.tracks.map((t, i) => (have(t) ? null : i)).filter((i) => i != null);
    if (missing.length) {
      this.downloader.start(col, missing, { silent: true });
      this.emit('sync:new', { playlist: p.name, count: missing.length });
    } else {
      this.downloader.mirror({ ...col, jobId });
    }
    return missing.length;
  }
}

module.exports = { Sync };

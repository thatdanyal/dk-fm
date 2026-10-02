// Watches music folders: files dropped in appear instantly, deleted files disappear.
// One recursive OS watcher per folder (no polling), events debounced per file.
const fs = require('fs');
const path = require('path');
const { AUDIO_EXT } = require('./library');

class Watcher {
  constructor(library) {
    this.library = library;
    this.watchers = [];
    this.pending = new Map();
  }

  watch(folders) {
    this.close();
    for (const dir of folders) {
      try {
        const w = fs.watch(dir, { recursive: true, persistent: false }, (_ev, rel) => {
          if (!rel) return;
          const full = path.join(dir, rel.toString());
          if (!AUDIO_EXT.has(path.extname(full).toLowerCase()) || /\.part\.\w+$/.test(full)) return;
          clearTimeout(this.pending.get(full));
          // Wait for the copy/download to finish writing before reading tags.
          this.pending.set(full, setTimeout(() => this.settle(full), 2500));
        });
        w.on('error', () => {});
        this.watchers.push(w);
      } catch { /* folder missing or unsupported: the startup scan still covers it */ }
    }
  }

  async settle(full) {
    this.pending.delete(full);
    const st = await fs.promises.stat(full).catch(() => null);
    if (st?.isFile()) await this.library.addFiles([full]);
    else this.library.removeByPath(full);
  }

  close() {
    this.watchers.forEach((w) => w.close());
    this.watchers = [];
  }
}

module.exports = { Watcher };

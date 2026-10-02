// Shared renderer state: settings + library snapshot, with change events.
import { Emitter } from './util.js';

class State extends Emitter {
  constructor() {
    super();
    this.settings = {};
    this.tracks = new Map();
    this.playlists = [];
    this.stats = {};
    this._pending = {};
  }

  async load() {
    this.settings = await dk.settings.get();
    this.applyLibrary(await dk.library.get());
    dk.on('library:changed', (snap) => this.applyLibrary(snap));
  }

  applyLibrary(snap) {
    this.tracks = new Map(snap.tracks.map((t) => [t.id, t]));
    this.playlists = snap.playlists;
    this.stats = snap.stats;
    this.emit('library');
  }

  // Persist a top-level settings key (debounced, merged).
  set(key, value) {
    this.settings[key] = value;
    this._pending[key] = value;
    clearTimeout(this._saveTimer);
    this._saveTimer = setTimeout(() => {
      dk.settings.set(this._pending);
      this._pending = {};
    }, 300);
    this.emit('settings', key, value);
  }

  stat(id) {
    return this.stats[id] || { plays: 0, liked: false, lastPlayed: 0 };
  }

  async like(id) {
    this.stats[id] = await dk.library.like(id);
    this.emit('stats', id);
  }

  track(id) {
    return this.tracks.get(id);
  }
}

export const state = new State();

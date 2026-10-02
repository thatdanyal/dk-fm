// Tiny JSON-file persistence in the userData folder. Writes are debounced and atomic.
const fs = require('fs');
const path = require('path');
const { app } = require('electron');

class Store {
  constructor(name, defaults) {
    this.file = path.join(app.getPath('userData'), `${name}.json`);
    this.defaults = defaults;
    this.data = structuredClone(defaults);
    try {
      const raw = JSON.parse(fs.readFileSync(this.file, 'utf8'));
      this.data = deepMerge(structuredClone(defaults), raw);
    } catch { /* first run or corrupt file: keep defaults */ }
    this._timer = null;
  }

  get(key) { return key ? this.data[key] : this.data; }

  set(key, value) {
    this.data[key] = value;
    this.save();
  }

  patch(obj) {
    Object.assign(this.data, obj);
    this.save();
  }

  save() {
    clearTimeout(this._timer);
    this._timer = setTimeout(() => this.flush(), 250);
  }

  flush() {
    clearTimeout(this._timer);
    const tmp = this.file + '.tmp';
    fs.mkdirSync(path.dirname(this.file), { recursive: true });
    fs.writeFileSync(tmp, JSON.stringify(this.data));
    fs.renameSync(tmp, this.file);
  }
}

function deepMerge(base, over) {
  if (!over || typeof over !== 'object' || Array.isArray(over)) return over ?? base;
  for (const [k, v] of Object.entries(over)) {
    if (v && typeof v === 'object' && !Array.isArray(v) && base[k] && typeof base[k] === 'object' && !Array.isArray(base[k])) {
      base[k] = deepMerge(base[k], v);
    } else {
      base[k] = v;
    }
  }
  return base;
}

module.exports = { Store };

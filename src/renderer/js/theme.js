import { state } from './state.js';
import { Emitter } from './util.js';

export const THEMES = {
  'red-retro': { name: 'Red Retro', bg: '#0b0404', swatch: ['#ff3b2f', '#ffae3b', '#ffd9cf'] },
  'amber-crt': { name: 'Amber CRT', bg: '#0a0702', swatch: ['#ffb000', '#ff6a1f', '#ffe2b0'] },
  'green-phosphor': { name: 'Green Phosphor', bg: '#020a04', swatch: ['#33ff66', '#b6ff3b', '#c8ffd4'] },
  synthwave: { name: 'Synthwave', bg: '#0b0414', swatch: ['#ff2bd6', '#22e5ff', '#f6dcff'] },
  ice: { name: 'Ice', bg: '#03080d', swatch: ['#3bc8ff', '#a6f0ff', '#d5f1ff'] },
  mono: { name: 'Mono', bg: '#080808', swatch: ['#f2f2f2', '#9a9a9a', '#555'] },
  paper: { name: 'Paper', bg: '#efe6d6', swatch: ['#c8231b', '#2a6f8f', '#2a0f0a'] },
};

export const themeEvents = new Emitter();

export function applyTheme() {
  const s = state.settings;
  const root = document.documentElement;
  root.dataset.theme = THEMES[s.theme] ? s.theme : 'red-retro';
  if (s.accent) {
    root.style.setProperty('--accent', s.accent);
    root.style.setProperty('--lcd-text', s.accent);
    root.style.setProperty('--glow', s.accent + '88');
  } else {
    ['--accent', '--lcd-text', '--glow'].forEach((v) => root.style.removeProperty(v));
  }
  document.body.classList.toggle('scanlines', !!s.scanlines);
  document.body.classList.toggle('glow-on', !!s.glow);
  document.body.classList.toggle('no-pixel-font', s.pixelFont === false);
  themeEvents.emit('change');
}

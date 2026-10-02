# DK.FM

A retro desktop music player for **Windows, macOS and Linux**. Plays your local library and turns
Spotify playlists into a local, tagged music collection by finding the best-matching studio
version of each song on YouTube Music.

## Features

**Player**
- Gapless playback, or 1–12 s crossfade between tracks
- 10-band EQ with presets, a live response curve, pre-amp and a loudness leveler
- Visualizer: LED spectrum, phosphor oscilloscope, twin analog VU meters and a spectrogram waterfall
- Waveform seek bar, built from each track's real audio
- Synced, clickable lyrics (LRCLIB)
- Queue with drag-to-reorder, play-next, save-as-playlist
- Shuffle / repeat all / repeat one, playback speed, sleep timer
- Mini player (always on top), OS media keys, keyboard shortcuts

**Library**
- Scans your music folders; drag & drop files onto the window
- Albums, artists, liked songs, most played, recently added, playlists
- Fast virtualized lists, multi-select, search, sortable columns

**Spotify import**
- Paste a playlist, album or track link → preview → pick tracks → download
- Matching prefers official YouTube Music audio, checks song length, and avoids live, remix,
  sped-up and cover versions. Each choice is shown so you can swap it from a menu or paste your own link.
- Files are tagged with Spotify's title, artist, album and cover art (MP3 320, MP3 V0 or M4A)
- Imported playlists show up in the library; **Sync** later downloads only new songs
- Works with no setup for up to 100 tracks; add free Spotify API keys in Settings for unlimited playlists

**Look**
- Red Retro by default, plus Amber CRT, Green Phosphor, Synthwave, Ice, Mono and Paper themes, and a custom accent color
- Scanlines, phosphor glow, pixel fonts (each can be turned off)
- **Movable layout**: click LAYOUT, then drag panels between columns, resize them, collapse or hide them

**Auto-update**: every push to `main` builds installers for all three OSes and publishes a GitHub
Release (`v1.0.<build number>`). The app checks on launch and every 30 minutes. When a new
version is out, an **Update available** popup shows the version and "What's new" (your commit
messages since the last release), with **LATER** and **DOWNLOAD & RESTART** buttons. If someone
picks LATER, an UPDATE button stays in the title bar and the popup returns on their next launch.

Write clear commit subjects. They become the release notes your users read.

## Develop

```bash
npm install
npm start
```

Build an installer for your current OS with `npm run dist` (output goes to `dist/`).

## Notes

- `yt-dlp` is downloaded automatically on first run and updated daily. FFmpeg ships with the app.
- macOS builds are unsigned: on first launch, right-click → Open. Auto-update on macOS
  needs a signed app (Apple Developer ID). Windows and Linux (AppImage) auto-update without signing.
- Only download music you have the rights to.

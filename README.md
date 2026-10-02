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

**Import anything**
- Paste a **Spotify** playlist / album / song / profile, **YouTube** or **YouTube Music** playlist / album / video, or **SoundCloud** track / set
- Spotify songs are matched to the best studio version on YouTube Music (length-checked, avoids live / remix / sped-up / covers);
  every pick is shown and can be swapped or replaced with your own link
- **Auto-sync**: imported playlists are re-checked in the background (hourly / 6h / daily). New songs download by themselves
  and the playlist keeps the source's order. Toggle per playlist with a right-click.
- **More like this**: right-click any song for a 50-song radio from YouTube Music; songs you already own are recognised
- **Whole Spotify profile**: import all of someone's public playlists at once (needs free Spotify API keys)
- Saves as M4A (original AAC, no re-encode: smallest files, no quality loss) or MP3, tagged with title/artist/album/cover

**Listening**
- **Stats any time**: minutes, plays, top songs & artists, streak, peak hour, activity, most skipped (week / month / year / all)
- **Smart shuffle**: every song once per lap, same-artist songs spread out, often-skipped songs play later
- **Volume matching**: each song's loudness is measured once and evened out (plus a transparent safety limiter)
- **Ctrl+K command palette**: search and do anything (songs, albums, artists, playlists, actions, themes, paste a link)
- **Auto-add**: files dropped into your music folders appear instantly
- **Tray**: the X button keeps music playing in the system tray (play/pause/next from the tray icon); optional start with Windows/macOS, hidden and paused

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

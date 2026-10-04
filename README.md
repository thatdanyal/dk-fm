# DK.FM

A retro desktop music player for **Windows, macOS and Linux**. Plays your local library and turns
Spotify playlists into a local, tagged music collection by finding the best-matching studio
version of each song on YouTube Music.

Native app written in Rust: **one ~8 MB program**, no bundled browser. On Windows it uses about
25 MB of RAM while playing (around 1.4% CPU with the visualizer on screen) and close to nothing
when it's idle or in the tray.

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
- Command line: `DK.FM --play-pause`, `--next`, `--prev` control the running copy (for hotkey tools and stream decks)

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

**Auto-update**: every push to `main` builds all three OSes and publishes a GitHub Release
(`v2.0.<build number>`). The app checks on launch and every 30 minutes. When a new version is
out, an **Update available** popup shows the version and "What's new" (your commit messages since
the last release), with **LATER** and **DOWNLOAD & RESTART** buttons. Updating swaps the one
program file and restarts (after checking the download is complete). The tests run on all three
OSes first, and nothing is published unless they pass. Pushes to other branches only build and
test, as a dry run.

Write clear commit subjects. They become the release notes your users read.

## Develop

Needs [Rust](https://rustup.rs) (and on Windows the Visual Studio C++ build tools).

```bash
cargo run --release
```

The program ends up in `target/release/`. Useful for testing:

- `DKFM_USER_DATA=<folder>` uses a separate profile instead of your real library and settings.
- `DKFM_INSTALL_DIR=<folder>` makes the Windows installer install into a test folder.
- `DKFM_PROFILE=1` logs frame times and repaint causes to `profile.log` in the data folder.
- `dkfm --test-play [seconds]` plays the library without a window (audio engine check).
- `cargo test` runs the tests (`cargo test -- --ignored` adds the ones that need the internet).

If something goes wrong, DK.FM writes what happened to `crash.log` in the data folder
(`%APPDATA%\DK.FM`, `~/Library/Application Support/DK.FM` or `~/.config/DK.FM`).

## Windows install

The release has two copies of the same program: `…-setup.exe` installs DK.FM (Start menu
shortcut, Apps & features entry, uninstaller) and the plain `.exe` runs from anywhere without
installing. Copies of the old Electron version (1.0.x) update to this one by themselves: the
setup file replaces the Electron files and keeps the library, playlists and settings.

## Notes

- `yt-dlp`, FFmpeg and QuickJS (the small JavaScript runtime yt-dlp needs for YouTube) are
  downloaded the first time you import music, then kept up to date. Only playing local
  music needs none of them.
- macOS builds are unsigned: on first launch, right-click → Open. On macOS the update popup
  opens the release page instead of updating in place. Windows and Linux (AppImage) update
  in place.
- Only download music you have the rights to.

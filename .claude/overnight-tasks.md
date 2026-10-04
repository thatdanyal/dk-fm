# DK.FM — "biggest update yet" task list

Worked on by the overnight routine (a fresh cloud session every 2 hours).
Each run: pick the next unchecked item(s), implement, build, commit, push, tick the box.
Write short notes under **Notes for the next run** so the next session does not re-derive context.
When every box is ticked, set the status line to `STATUS: DONE`.

STATUS: IN PROGRESS

## Tasks (from the user's original request)
- [x] Bug: Stats list keeps reordering/flickering, especially while the cursor moves — make ordering stable and stop re-sorting on hover/redraw
- [x] Layout: user must not be able to make a layout with no deck (pause/unpause, mute, volume must always be reachable); give the deck a minimum size that still fits those controls
- [ ] Layout: per-pixel free layout mode in addition to the existing template/snap layouts; user picks "Template" or "Free (per pixel)" in Settings > Layouts
- [x] Lyrics: text size scales with the size of the lyrics panel as the user resizes/moves it
- [x] Performance: after closing DK.FM nothing should keep using CPU/RAM (make sure every background thread/process — downloads, yt-dlp, audio, tray, watchers — exits on close)
- [x] Queue: drag to reorder songs in the queue
- [x] Queue: hovering a song's album-art area shows a play button; one click plays it
- [x] Album covers on every song row in playlists (like Spotify), with a cached thumbnail
- [ ] Search the web (not the library): user types a song, DK.FM searches YouTube and shows N versions (default 3, adjustable in Settings) with title/channel/duration so the user picks (clean / explicit / instrumental / live etc. — never auto-pick), then downloads the chosen one
- [ ] Discover tab/section for finding new music (build on the existing Home/recommendations)
- [x] Lyrics fallback: if a song has no lyrics, fetch them from YouTube captions (yt-dlp subtitles/auto-captions) and save them with the song
- [ ] "Shazam": identify music currently playing on the PC (capture system audio loopback for a few seconds, fingerprint, look up — e.g. an open song-recognition API / Shazam-style signature), show the result with a GET button
- [x] Media controls outside the app: play/pause, next, previous from the taskbar / OS media overlay like Spotify (Windows SMTC + taskbar thumbnail buttons, MPRIS on Linux, Now Playing on macOS — e.g. the `souvlaki` crate)
- [ ] First-run onboarding: friendly step-by-step instructions shown the first time the app is opened; ASK before creating a desktop/start-menu shortcut (only create it if the user says yes)
- [ ] Update notice: users updating to this version see a "This is the biggest update yet" what's-new screen once
- [ ] Better default settings: simpler, more minimalistic defaults (fewer panels/columns visible out of the box)
- [x] Logo: new DK logo (purple DK + mic) added as assets/icon.png (1024px, rounded, transparent corners) and assets/icon.ico (16–256px) by the user's main session. Done — do not change it.

## Notes for the next run
- 2026-10-04 (manual session, commit 76eb8da "Steadier and smarter..."): the 9 ticked tasks above were shipped on `claude/keen-wright-yk057e`, then the logo/task-list branch `claude/stoic-babbage-2y5job` was merged in. **Work only on `claude/keen-wright-yk057e` from now on.**
  - Builds on Linux, 32 tests pass, cross-compiles for Windows.
  - Not verified (needs the user's Windows PC/network): taskbar thumbnail buttons (`src/taskbar.rs`, ITaskbarList3) and the YouTube-captions lyrics fallback (`src/lyricsrc.rs`). Don't redo them.
  - Lyrics source order: cached -> file tags -> LRCLIB -> YouTube captions; per-song cache + LOOK AGAIN button.
  - Close behaviour: X keeps DK.FM in the tray only while music plays (Settings > System), otherwise quits fully.
  - Album covers in song rows can be turned off in Settings > Look; SMTC/MPRIS were already handled through `souvlaki` in `src/system.rs`.
  - The installer still creates shortcuts without asking; that's part of the first-run onboarding task.
- Remaining, suggested order: better defaults -> update notice ("biggest update yet") -> first-run onboarding + ask before shortcut -> web search with version choice -> Discover -> per-pixel layout mode -> Shazam.

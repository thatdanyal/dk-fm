# DK.FM — "biggest update yet" task list

Worked on by the overnight routine (a fresh cloud session every 2 hours).
Each run: pick the next unchecked item(s), implement, build, commit, push, tick the box.
Write short notes under **Notes for the next run** so the next session does not re-derive context.
When every box is ticked, set the status line to `STATUS: DONE`.

STATUS: DONE

## Tasks (from the user's original request)
- [x] Bug: Stats list keeps reordering/flickering, especially while the cursor moves — make ordering stable and stop re-sorting on hover/redraw
- [x] Layout: user must not be able to make a layout with no deck (pause/unpause, mute, volume must always be reachable); give the deck a minimum size that still fits those controls
- [x] Layout: per-pixel free layout mode in addition to the existing template/snap layouts; user picks "Template" or "Free (per pixel)" in Settings > Layouts
- [x] Lyrics: text size scales with the size of the lyrics panel as the user resizes/moves it
- [x] Performance: after closing DK.FM nothing should keep using CPU/RAM (make sure every background thread/process — downloads, yt-dlp, audio, tray, watchers — exits on close)
- [x] Queue: drag to reorder songs in the queue
- [x] Queue: hovering a song's album-art area shows a play button; one click plays it
- [x] Album covers on every song row in playlists (like Spotify), with a cached thumbnail
- [x] Search the web (not the library): user types a song, DK.FM searches YouTube and shows N versions (default 3, adjustable in Settings) with title/channel/duration so the user picks (clean / explicit / instrumental / live etc. — never auto-pick), then downloads the chosen one
- [x] Discover tab/section for finding new music (build on the existing Home/recommendations)
- [x] Lyrics fallback: if a song has no lyrics, fetch them from YouTube captions (yt-dlp subtitles/auto-captions) and save them with the song
- [x] "Shazam": identify music currently playing on the PC (capture system audio loopback for a few seconds, fingerprint, look up — e.g. an open song-recognition API / Shazam-style signature), show the result with a GET button
- [x] Media controls outside the app: play/pause, next, previous from the taskbar / OS media overlay like Spotify (Windows SMTC + taskbar thumbnail buttons, MPRIS on Linux, Now Playing on macOS — e.g. the `souvlaki` crate)
- [x] First-run onboarding: friendly step-by-step instructions shown the first time the app is opened; ASK before creating a desktop/start-menu shortcut (only create it if the user says yes)
- [x] Update notice: users updating to this version see a "This is the biggest update yet" what's-new screen once
- [x] Better default settings: simpler, more minimalistic defaults (fewer panels/columns visible out of the box)
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
- 2026-10-04 routine run 1 (commits d807318, 3462986): defaults, update notice and onboarding done.
  - Defaults (`src/store.rs`, `default_dock()` in `src/ui/mod.rs`): fresh installs get Library + Deck + Queue/Lyrics (no Scope/EQ), scanlines off, columns num/like/title/artist/time, tabs Recent/Top hidden (`d_hidden_tabs`). Old full layout is the "Everything" preset. Existing users keep their saved values (all fields are saved).
  - Welcome/what's new: `src/ui/welcome.rs` (Modal::Welcome / Modal::WhatsNew, picked by `welcome::first_modal` in App::new). Fresh install = no settings.json at start (`Settings.fresh`, serde-skipped); `onboarded` defaults to true for old settings files, set false only when fresh. `whats_new_seen` vs `welcome::WHATS_NEW`; fresh installs get it pre-set so they never see the notice. **Later runs: add each shipped feature as a line to `NEWS` in welcome.rs.**
  - Installer (`src/install.rs`) no longer creates shortcuts; it only refreshes ones that already exist. The welcome's Windows-only step has two switches (off by default) -> `install::add_shortcuts`. Unverified on Windows (the PowerShell shortcut code itself is unchanged). Screenshots of both screens looked right under Xvfb (`apt install libxkbcommon-x11-0`, run `target/debug/dkfm` with `DKFM_USER_DATA=<tmp>` and `LIBGL_ALWAYS_SOFTWARE=1`).
  - Build deps for the cloud: `apt-get install pkg-config libasound2-dev libgtk-3-dev libdbus-1-dev mingw-w64` and `rustup target add x86_64-pc-windows-gnu`.
- 2026-10-04 routine run 1, continued (commits 627d3ff, a0f77a4, df82283, 7c3d9a6): the last four tasks.
  - Web search: `src/ui/websearch.rs` (View::Web, tab "FIND SONGS"), labels from `sources::version_tags` (unit-tested; uses the new `ITrack.raw_title`), rows via `addsongs::result_rows_tagged`. Settings `web_results` (default 3, Settings > Search) and `web_source` ("songs" = YouTube Music, or "youtube"). Never downloads until + GET. Respects "hide explicit". Real YouTube search not verified here (YouTube is blocked in the cloud); rows checked with fake results in a screenshot.
  - Discover: `src/ui/explore.rs` (View::Discover, tab "DISCOVER"): shelves "Because you play <artist>" (radio from your most-played song of your top 3 artists; SHUFFLE moves to the next artists/songs) + "From a song you liked", minus songs you have. Session cache only. Online part unverified (YouTube blocked).
  - Free layout: `src/ui/freelayout.rs`, Settings > Layouts > LAYOUT STYLE (`layout_mode` "template"|"free", `free_panels` pixel rects relative to the panel area). Open panels = the dock's tabs, so the View menu still works. Edit with Ctrl+E (drag title bar / corner grip). Deck drawn on top every frame, min 230x72, clamped on-screen. First placement copies the template's rects scaled to the window (tabbed panels split vertically). Saved named layouts still only store the template layout. Known nit: a modal's dimming layer may not cover free panels.
  - Shazam: `src/recognize.rs` — capture via cpal (Windows: input stream on the default output device = WASAPI loopback; Linux/mac: an input named "monitor", else default input), 16 kHz resample, Shazam signature (SongRec-style peaks + binary format; my own implementation, header/CRC/base64 unit-tested) and POST to amp.shazam.com. UI: "♫ WHAT'S PLAYING?" on FIND SONGS + Ctrl+K "What's playing?". + GET fills the versions search. **Completely unverified end to end** (no network to Shazam and no audio device here): if Shazam never matches, compare `peaks()`/`encode()` with SongRec's signature_generator.rs.
  - welcome.rs NEWS lists everything shipped on this branch.

## Summary (everything is done)
This branch now has: stable Stats, a deck that can't be lost, drag-to-reorder queue with one-click play, album covers in song lists, lyrics that fit their panel and fall back to YouTube captions, taskbar/media-key controls, full quit on close, the new logo, simpler defaults for new installs, a first-run welcome that asks before adding shortcuts, a one-time "biggest update yet" screen for people who update, FIND SONGS (pick the version: clean/explicit/live/...), DISCOVER, a free per-pixel layout mode, and WHAT'S PLAYING? (Shazam-style).

### Please test on your Windows PC
1. Fresh install (on a PC/user without %APPDATA%\DK.FM): the welcome appears, and NO Start menu/desktop shortcut exists until you turn the switches on in the "Shortcuts" step; then they're created and work.
2. Update over your current install: no welcome, the "THE BIGGEST UPDATE YET" screen shows once (not again after LET'S GO / restart); your existing shortcuts are still there and still open DK.FM.
3. FIND SONGS: search a song with clean & explicit versions (e.g. a rap single) on YOUTUBE MUSIC and YOUTUBE; check the labels make sense, change "Versions per search" in Settings > Search, + GET downloads only what you click.
4. DISCOVER: shelves fill in after a few seconds; + GET and ↻ SHUFFLE work.
5. WHAT'S PLAYING?: play a well-known song in a browser/YouTube, click ♫ WHAT'S PLAYING? on FIND SONGS (also try with DK.FM itself playing). Report whether it names the song, says "silence", or "couldn't name that one".
6. Settings > Layouts > FREE (PER PIXEL): Ctrl+E, move/resize panels, try to hide or shrink the deck (it must stay usable and on top), resize the window, restart (positions kept), switch back to TEMPLATE.
7. Taskbar thumbnail play/pause/next buttons and the YouTube-captions lyrics fallback (from the earlier session, still unverified).
8. Fresh install defaults: just Library + Deck + Queue/Lyrics, no scanlines.

The routine could not disable itself (no tool for that in the cloud session): please turn off trigger trig_01Kz6dKtQC4baHdyrr5gfhbf, or it will start, see STATUS: DONE and stop.

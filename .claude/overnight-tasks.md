# DK.FM — "biggest update yet" task list

Worked on by the overnight routine (a fresh cloud session every 2 hours).
Each run: pick the next unchecked item(s), implement, build, commit, push, tick the box.
Write short notes under **Notes for the next run** so the next session does not re-derive context.
When every box is ticked, set the status line to `STATUS: DONE`.

STATUS: IN PROGRESS

## Tasks (from the user's original request)
- [ ] Bug: Stats list keeps reordering/flickering, especially while the cursor moves — make ordering stable and stop re-sorting on hover/redraw
- [ ] Layout: user must not be able to make a layout with no deck (pause/unpause, mute, volume must always be reachable); give the deck a minimum size that still fits those controls
- [ ] Layout: per-pixel free layout mode in addition to the existing template/snap layouts; user picks "Template" or "Free (per pixel)" in Settings > Layouts
- [ ] Lyrics: text size scales with the size of the lyrics panel as the user resizes/moves it
- [ ] Performance: after closing DK.FM nothing should keep using CPU/RAM (make sure every background thread/process — downloads, yt-dlp, audio, tray, watchers — exits on close)
- [ ] Queue: drag to reorder songs in the queue
- [ ] Queue: hovering a song's album-art area shows a play button; one click plays it
- [ ] Album covers on every song row in playlists (like Spotify), with a cached thumbnail
- [ ] Search the web (not the library): user types a song, DK.FM searches YouTube and shows N versions (default 3, adjustable in Settings) with title/channel/duration so the user picks (clean / explicit / instrumental / live etc. — never auto-pick), then downloads the chosen one
- [ ] Discover tab/section for finding new music (build on the existing Home/recommendations)
- [ ] Lyrics fallback: if a song has no lyrics, fetch them from YouTube captions (yt-dlp subtitles/auto-captions) and save them with the song
- [ ] "Shazam": identify music currently playing on the PC (capture system audio loopback for a few seconds, fingerprint, look up — e.g. an open song-recognition API / Shazam-style signature), show the result with a GET button
- [ ] Media controls outside the app: play/pause, next, previous from the taskbar / OS media overlay like Spotify (Windows SMTC + taskbar thumbnail buttons, MPRIS on Linux, Now Playing on macOS — e.g. the `souvlaki` crate)
- [ ] First-run onboarding: friendly step-by-step instructions shown the first time the app is opened; ASK before creating a desktop/start-menu shortcut (only create it if the user says yes)
- [ ] Update notice: users updating to this version see a "This is the biggest update yet" what's-new screen once
- [ ] Better default settings: simpler, more minimalistic defaults (fewer panels/columns visible out of the box)
- [ ] Logo: the user attached a new logo in another chat — it is NOT available here. Do not invent one; leave assets/icon.* as is and mention it in the final summary

## Notes for the next run
(none yet)

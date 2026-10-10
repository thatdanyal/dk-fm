//! Listen before you download: ▶ on an online song (Songs like this, search results, Discover)
//! fetches a quick temporary copy (a few seconds, only when asked: searching never waits on it)
//! and plays it right away (the whole song), keeping your queue. While it plays the deck offers
//! KEEP (download it for real) or DISCARD (skip it). Previews never join your library, playlists
//! or Stats; once you move on to another song a preview's file is deleted (kept or not: KEEP
//! downloads its own copy), and anything left over is cleared when DK.FM starts.
use super::theme::vt;
use super::widgets::{fill, frame_rect, with_alpha};
use super::App;
use crate::library::PREVIEW;
use crate::sources::ITrack;
use crate::store::Track;
use eframe::egui::{self, Align2, Rect, Sense, Ui, Vec2};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Default)]
pub struct Previews {
    /// source key -> preview id once ready, or why it failed (None = still fetching)
    state: HashMap<String, Option<Result<String, String>>>,
    done: Arc<Mutex<Vec<(String, Result<Track, String>)>>>,
    /// preview id -> the online song it's a preview of (for KEEP)
    items: HashMap<String, ITrack>,
    /// previews whose song is being downloaded (KEEP pressed)
    kept: std::collections::HashSet<String>,
    /// the song playing when previews were last tidied
    last: Option<String>,
    /// online songs fetched for the queue (a mix's), not by ▶: they don't jump in when ready
    auto: std::collections::HashSet<String>,
}

fn dir() -> PathBuf {
    std::env::temp_dir().join("dkfm-previews")
}

/// Previews from an earlier run are just temporary files (and their covers).
pub fn clear_old(covers: PathBuf) {
    let _ = std::fs::remove_dir_all(dir());
    if let Ok(rd) = std::fs::read_dir(&covers) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(PREVIEW) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Previews you've moved on from (another song is playing, or nothing) are deleted, and taken
/// out of the queue so ⏮ doesn't land on them.
fn tidy(app: &mut App) {
    let cur = app.player.current_id();
    if cur == app.previews.last {
        return;
    }
    app.previews.last = cur.clone();
    // (online songs still to come in the queue stay)
    let upcoming: std::collections::HashSet<String> = {
        let st = app.player.st.lock();
        st.queue.iter().skip((st.index + 1).max(0) as usize).cloned().collect()
    };
    let gone: Vec<String> = app.lib.previews.read().keys().filter(|id| Some(*id) != cur.as_ref() && !upcoming.contains(*id)).cloned().collect();
    for id in gone {
        loop {
            let at = app.player.st.lock().queue.iter().position(|q| *q == id);
            let Some(i) = at else { break };
            app.player.remove_at(i);
        }
        if let Some(t) = app.lib.previews.write().remove(&id).filter(|t| !t.path.is_empty()) {
            let mut files = vec![PathBuf::from(&t.path)];
            files.extend(t.cover.as_ref().map(|c| app.lib.cover_path(c)));
            // the player may still have the file open for a moment
            std::thread::spawn(move || {
                for _ in 0..20 {
                    files.retain(|f| f.exists() && std::fs::remove_file(f).is_err());
                    if files.is_empty() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            });
        }
        app.previews.items.remove(&id);
        app.previews.kept.remove(&id);
        app.previews.auto.remove(&id);
        app.previews.state.retain(|_, v| !matches!(v, Some(Ok(x)) if *x == id));
    }
}

pub fn id_of(t: &ITrack) -> String {
    let key: String = t.source_key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("{PREVIEW}{key}")
}

/// Fetch and play (or pause / resume, if it's the preview playing now).
pub fn toggle(app: &mut App, t: &ITrack) {
    let id = id_of(t);
    if app.player.current_id().as_deref() == Some(id.as_str()) {
        app.player.toggle();
        return;
    }
    match app.previews.state.get(&t.source_key) {
        Some(None) => {} // on its way
        Some(Some(Ok(id))) if app.lib.track(id).is_some() => app.player.play_now(id.clone()),
        _ => start(app, t),
    }
}

/// An online song as a song of DK.FM (no file yet: it's fetched when it's played).
pub fn as_track(t: &ITrack) -> Track {
    Track {
        id: id_of(t),
        title: t.title.clone(),
        artist: t.artists.join(", "),
        album: if t.album.is_empty() { "Online (not downloaded)".into() } else { t.album.clone() },
        duration: t.duration_ms.map(|d| d as f64 / 1000.0).unwrap_or(0.0),
        youtube_id: t.youtube_id.clone(),
        ..Default::default()
    }
}

/// A song's cover as a texture: its cover file, or for an online song not fetched yet (a mix's new
/// songs in the queue) its cover from the web.
pub fn cover_tex(app: &mut App, ctx: &egui::Context, t: &Track, size: u32) -> Option<egui::TextureId> {
    if let Some(c) = t.thumb.as_ref().filter(|_| size <= 64).or(t.cover.as_ref()).or(t.thumb.as_ref()) {
        return app.covers.get(ctx, app.lib.cover_path(c), c, size);
    }
    let url = app.previews.items.get(&t.id).and_then(|i| i.cover.clone())?;
    super::home::remote_tex(app, ctx, Some(&url))
}

/// Online songs for the queue (a mix's): each is fetched when its turn comes or while the song
/// before it plays, and deleted once played unless you KEEP it. Returns their queue ids.
pub fn queue_online(app: &mut App, songs: &[ITrack]) -> Vec<String> {
    let mut ids = Vec::new();
    for t in songs {
        let id = id_of(t);
        app.previews.items.insert(id.clone(), t.clone());
        app.lib.previews.write().entry(id.clone()).or_insert_with(|| as_track(t));
        crate::fastlink::warm(t);
        ids.push(id);
    }
    ids
}

fn start(app: &mut App, t: &ITrack) {
    app.previews.state.insert(t.source_key.clone(), None);
    app.previews.items.insert(id_of(t), t.clone());
    let (t, done, ctx, id) = (t.clone(), app.previews.done.clone(), app.covers.ctx.clone(), id_of(t));
    let covers = app.lib.cover_path("");
    std::thread::spawn(move || {
        let r = fetch(&t, &id, &covers);
        done.lock().push((t.source_key.clone(), r));
        if let Some(c) = ctx {
            c.request_repaint();
        }
    });
}

fn fetch(t: &ITrack, id: &str, covers: &std::path::Path) -> Result<Track, String> {
    crate::ytdlp::ensure()?;
    let artist = t.artists.join(", ");
    let url = t
        .direct_url
        .clone()
        .or_else(|| t.youtube_id.as_ref().map(|y| format!("https://music.youtube.com/watch?v={y}")))
        .unwrap_or_else(|| format!("ytsearch1:{artist} {}", t.title));
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let path = d.join(format!("{id}.m4a"));
    // its link was looked up ahead (it was on screen): just the file, in under a second
    if !matches!(crate::fastlink::fetch(t, &path, std::time::Duration::from_secs(20)), Some(Ok(()))) {
        // the small AAC stream: starts fast, never converted
        let out = d.join(format!("{id}.%(ext)s"));
        let args: Vec<String> = ["--no-playlist", "-f", "bestaudio[ext=m4a]/bestaudio", "-x", "--audio-format", "m4a", "-o"].iter().map(|s| s.to_string()).chain([out.display().to_string(), url]).collect();
        crate::ytdlp::run_with(&args, None, None)?;
    }
    if !path.exists() {
        return Err("Couldn't get a preview of that song".into());
    }
    // its cover, so the deck shows it
    let cover = t.cover.as_ref().and_then(|u| crate::net::get_bytes(u).ok()).map(|b| crate::discover::small_square(b, 640)).and_then(|b| {
        let name = format!("{id}.jpg");
        std::fs::write(covers.join(&name), b).ok().map(|_| name)
    });
    Ok(Track {
        id: id.to_string(),
        path: path.display().to_string(),
        title: t.title.clone(),
        artist,
        album: if t.album.is_empty() { "Preview (not downloaded)".into() } else { t.album.clone() },
        duration: t.duration_ms.map(|d| d as f64 / 1000.0).unwrap_or(0.0),
        thumb: cover.clone(),
        cover,
        youtube_id: t.youtube_id.clone(),
        ..Default::default()
    })
}

/// While a preview is playing: ⬇ KEEP downloads the song to your library (in your sound quality)
/// and × DISCARD skips it. Draws nothing (and returns false) otherwise.
pub fn keep_bar(app: &mut App, ui: &mut Ui, w: f32, h: f32) -> bool {
    let Some(id) = app.player.current_id().filter(|id| id.starts_with(PREVIEW)) else { return false };
    let Some(t) = app.previews.items.get(&id).cloned() else { return false };
    let pal = app.pal;
    let kept = app.previews.kept.contains(&id);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let bw = ((w - 6.0) / 2.0).max(40.0);
        let keep = super::deck::tbtn(ui, &pal, if kept { "✔ KEEPING" } else { "⬇ KEEP" }, Vec2::new(bw, h), kept, !kept);
        if !kept && keep.on_hover_text("Like it? Download it: you pick where it goes (library, Liked or a playlist)").clicked() {
            self::keep(app, ui.ctx(), &t);
        }
        if super::deck::tbtn(ui, &pal, "× DISCARD", Vec2::new(bw, h), false, false).on_hover_text("Not for you: skip it and delete it. Nothing is saved").clicked() {
            discard(app, &t);
        }
    });
    true
}

/// KEEP: asks where it should go (library, Liked, a playlist), then downloads it for real.
pub fn keep(app: &mut App, ctx: &egui::Context, t: &ITrack) {
    if !app.previews.kept.contains(&id_of(t)) {
        let pos = ctx.pointer_latest_pos().unwrap_or(ctx.screen_rect().center());
        super::dest::open(app, t, pos, true);
    }
}

/// A place was picked for a kept preview: it's downloading (the button says KEEPING).
pub fn mark_kept(app: &mut App, t: &ITrack) {
    app.previews.kept.insert(id_of(t));
}

/// KEEP was pressed for this song's preview.
pub fn kept(app: &App, t: &ITrack) -> bool {
    app.previews.kept.contains(&id_of(t))
}

/// DISCARD: skip it (its file is deleted once something else plays).
pub fn discard(app: &mut App, t: &ITrack) {
    let id = id_of(t);
    let at = app.player.st.lock().queue.iter().position(|q| *q == id);
    if let Some(i) = at {
        app.player.remove_at(i);
    }
}

/// This song's preview is the song loaded now (playing or paused).
pub fn current(app: &App, t: &ITrack) -> bool {
    app.player.current_id().as_deref() == Some(id_of(t).as_str())
}

/// This song's preview is still being fetched.
pub fn loading(app: &App, t: &ITrack) -> bool {
    matches!(app.previews.state.get(&t.source_key), Some(None))
}

/// This song's preview is loading or is the song playing now (so its button stays visible).
pub fn active(app: &App, t: &ITrack) -> bool {
    matches!(app.previews.state.get(&t.source_key), Some(None)) || app.player.current_id().as_deref() == Some(id_of(t).as_str())
}

/// Previews that finished: ready ones start playing. Old ones are deleted.
pub fn poll(app: &mut App) {
    tidy(app);
    // online songs in the queue the player wants now (or next)
    let wants: Vec<String> = app.player.wants.lock().drain(..).collect();
    for id in wants {
        let Some(t) = app.previews.items.get(&id).cloned() else { continue };
        if !matches!(app.previews.state.get(&t.source_key), Some(None)) && app.lib.track(&id).is_some_and(|x| x.path.is_empty()) {
            app.previews.auto.insert(id);
            start(app, &t);
        }
    }
    let done: Vec<_> = app.previews.done.lock().drain(..).collect();
    for (key, r) in done {
        let id = r.as_ref().map(|t| t.id.clone()).unwrap_or_else(|_| app.previews.items.iter().find(|(_, t)| t.source_key == key).map(|(i, _)| i.clone()).unwrap_or_default());
        let auto = app.previews.auto.contains(&id);
        let now = app.player.current_id().as_deref() == Some(id.as_str());
        match r {
            Ok(track) => {
                app.lib.previews.write().insert(id.clone(), track);
                app.previews.state.insert(key, Some(Ok(id.clone())));
                if !auto {
                    app.player.play_now(id);
                } else if now {
                    let i = app.player.st.lock().index;
                    app.player.play_index(i.max(0) as usize);
                } else {
                    app.player.preload_next();
                }
            }
            Err(e) => {
                app.previews.state.insert(key, Some(Err(e.clone())));
                if !auto {
                    app.toast_err(format!("Preview: {e}"));
                } else if now {
                    app.toast_err(format!("Couldn't play an online song in the mix: {e}"));
                    app.player.next(true);
                }
            }
        }
    }
}

/// The ▶ / ⏸ / … button over a song's cover (or wherever `r` is). Returns its response.
pub fn button(app: &mut App, ui: &mut Ui, r: Rect, t: &ITrack) -> egui::Response {
    let pal = app.pal;
    let resp = ui.interact(r, ui.id().with(("preview", &t.source_key)), Sense::click());
    let id = id_of(t);
    let current = app.player.current_id().as_deref() == Some(id.as_str());
    let playing = current && app.player.status().playing;
    let busy = matches!(app.previews.state.get(&t.source_key), Some(None));
    let over = resp.hovered();
    {
        fill(ui.painter(), r, with_alpha(if over { pal.accent } else { pal.bg }, if over { 235 } else { 170 }));
        frame_rect(ui.painter(), r, 1.0, pal.accent);
        let icon = if busy { "…" } else if playing { "⏸" } else { "▶" };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, vt((r.height() * 0.6).clamp(14.0, 26.0)), if over { pal.ink } else { pal.text });
    }
    if busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
    }
    if over {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let resp = resp.on_hover_text(if busy { "Getting a preview…" } else if playing { "Pause" } else { "Listen before you download (a temporary copy: it isn't added to your library)" });
    if resp.clicked() && !busy {
        toggle(app, t);
    }
    resp
}

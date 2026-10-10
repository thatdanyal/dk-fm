//! Sharing with friends: copy a code (theme / layout / settings / playlist), export `.dkfm` and
//! M3U playlist files, and the preview shown for a pasted code or opened file. Nothing a friend
//! sends is applied until you press APPLY / IMPORT.
use super::browser::View;
use super::theme::{px, vt, Pal};
use super::widgets::{button, caption, dim, fill, fmt_time, frame_rect, switch};
use super::{App, Tab};
use crate::share::{self, Share, SharedSettings};
use crate::sources::Collection;
use crate::store::{Playlist, Track};
use eframe::egui::{self, Align2, Pos2, Rect, Sense, Ui, Vec2};
use egui_dock::{DockState, Node, NodeIndex};
use std::path::Path;

/// A received code or file waiting for APPLY / CANCEL.
pub struct Incoming {
    share: Share,
    /// playlist: your library's copy of each song (None = will be downloaded)
    owned: Vec<Option<String>>,
    /// playlist: the name to save it as
    name: String,
    /// layout: parsed once for the diagram
    dock: Option<DockState<Tab>>,
    /// a whole DK.FM: bring in its playlists, liked songs, other songs, settings & looks
    pick: [bool; 4],
    /// a whole DK.FM: songs already here
    have_all: usize,
    /// THEATER: which of your three versions it replaces
    slot: usize,
}

// ------------------------------------------------------------------------------- sending

/// Copies the code to the clipboard. `what` = e.g. `theme "Neon"`.
pub fn copy(app: &mut App, ctx: &egui::Context, s: &Share, what: &str) {
    let code = share::encode(s);
    let long = code.len() > 2000 && matches!(s, Share::Playlist(_));
    ctx.copy_text(code);
    app.toast(if long {
        format!("Copied the code for {what}. It's long: for chat apps, use \"Export playlist file…\" instead.")
    } else {
        format!("Copied the code for {what}. Paste it to a friend: they press Ctrl+V in DK.FM.")
    });
}

/// A built-in or custom theme by key; the current one with your accent colour.
pub fn theme_share(app: &App, key: &str) -> Share {
    let s = app.settings.lock();
    let name = super::theme::all_themes(&s).into_iter().find(|t| t.0 == key).map(|t| t.1).unwrap_or_else(|| "My theme".into());
    let pal = if key == s.theme { app.pal } else { super::theme::theme_pal(&s, key) };
    Share::Theme(pal.to_custom(&name))
}

/// A saved or built-in layout by name, or the current arrangement (`None`).
pub fn layout_share(app: &App, name: Option<&str>) -> Option<Share> {
    let mut dock = match name {
        None => serde_json::to_value(&app.dock).ok(),
        Some(n) => app.settings.lock().layouts.iter().find(|l| l.name == n).map(|l| l.dock.clone()).or_else(|| App::preset(n).and_then(|d| serde_json::to_value(&d).ok())),
    }?;
    share::fix_rects(&mut dock);
    if let Some(m) = dock.as_object_mut() {
        m.remove("translations"); // the receiver uses its own (keeps codes short)
    }
    Some(Share::Layout(crate::store::NamedLayout { name: name.unwrap_or("My layout").to_string(), dock }))
}

/// One of your THEATER versions.
pub fn theater_share(app: &App, i: usize) -> Share {
    Share::Theater(app.settings.lock().theater[i.min(2)].clone())
}

pub fn settings_share(app: &App) -> Share {
    Share::Settings(Box::new(SharedSettings::from(&*app.settings.lock())))
}

fn tracks_of(app: &App, p: &Playlist) -> Vec<Track> {
    let d = app.lib.data.read();
    p.track_ids.iter().filter_map(|id| d.tracks.get(id).cloned()).collect()
}

pub fn playlist_share(app: &App, p: &Playlist) -> Share {
    Share::Playlist(share::playlist_of(&p.name, &tracks_of(app, p)))
}

fn file_name(name: &str, ext: &str) -> String {
    let n: String = name.chars().filter(|c| !"<>:\"/\\|?*".contains(*c) && !c.is_control()).collect();
    format!("{}.{ext}", if n.trim().is_empty() { "Playlist" } else { n.trim() })
}

/// "Export playlist file…": the playlist as a `.dkfm` file for big playlists.
pub fn export_file(app: &mut App, p: &Playlist) {
    let Some(path) = rfd::FileDialog::new().set_title("Export playlist for a friend").set_file_name(file_name(&p.name, share::EXT)).add_filter("DK.FM playlist", &[share::EXT]).save_file() else { return };
    match std::fs::write(&path, share::file_json(&playlist_share(app, p))) {
        Ok(()) => app.toast(format!("Saved \"{}\". Your friend can drop it onto DK.FM.", path.file_name().unwrap_or_default().to_string_lossy())),
        Err(e) => app.toast_err(format!("Couldn't save the file: {e}")),
    }
}

/// "Export as M3U…": songs inside the folder you save to get relative paths.
pub fn export_m3u(app: &mut App, p: &Playlist) {
    let Some(path) = rfd::FileDialog::new().set_title("Export as M3U playlist").set_file_name(file_name(&p.name, "m3u8")).add_filter("M3U playlist", &["m3u8", "m3u"]).save_file() else { return };
    let tracks = tracks_of(app, p);
    let dir = path.parent();
    let rel = tracks.iter().filter(|t| dir.map(|d| Path::new(&t.path).starts_with(d)).unwrap_or(false)).count();
    match std::fs::write(&path, share::m3u(&tracks, dir)) {
        Ok(()) => app.toast(format!("Saved \"{}\" ({} songs{})", path.file_name().unwrap_or_default().to_string_lossy(), tracks.len(), if rel > 0 { ", paths relative to its folder" } else { "" })),
        Err(e) => app.toast_err(format!("Couldn't save the playlist: {e}")),
    }
}

// ------------------------------------------------------------------------------- receiving

/// Pasted text: opens the preview when it holds a DK.FM code. Returns true if it had one.
pub fn receive(app: &mut App, text: &str) -> bool {
    if !text.contains(share::PREFIX) {
        return false;
    }
    match share::decode(text) {
        Ok(s) => open(app, s),
        Err(e) => app.toast_err(e),
    }
    true
}

pub fn open_file(app: &mut App, path: &Path) {
    match share::read_file(path) {
        Ok(s) => open(app, s),
        Err(e) => app.toast_err(e),
    }
}

pub fn pick_file(app: &mut App) {
    if let Some(p) = rfd::FileDialog::new().set_title("Open a shared playlist").add_filter("DK.FM share", &[share::EXT]).pick_file() {
        open_file(app, &p);
    }
}

fn open(app: &mut App, s: Share) {
    let (owned, name, dock) = match &s {
        Share::Playlist(p) => {
            let idx = app.lib.key_index();
            (p.songs.iter().map(|x| x.keys().iter().find_map(|k| idx.get(k).cloned())).collect(), p.name.clone(), None)
        }
        Share::Layout(l) => (Vec::new(), l.name.clone(), super::load_dock(&l.dock)),
        Share::Theme(t) => (Vec::new(), t.name.clone(), None),
        Share::Theater(t) => (Vec::new(), t.name.clone(), None),
        Share::Settings(_) => (Vec::new(), String::new(), None),
        Share::Everything(a) => (Vec::new(), a.name.clone(), None),
    };
    let have_all = match &s {
        Share::Everything(a) => {
            let idx = app.lib.key_index();
            a.playlists.iter().flat_map(|p| p.songs.iter()).chain(&a.liked).chain(&a.songs).filter(|x| x.keys().iter().any(|k| idx.contains_key(k))).count()
        }
        _ => 0,
    };
    app.palette = None;
    app.pick = None;
    let slot = if app.nowplaying { app.theater.preset } else { 2 };
    app.incoming = Some(Incoming { share: s, owned, name, dock, pick: [true; 4], have_all, slot });
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut inc) = app.incoming.take() else { return };
    let pal = app.pal;
    let (title, size) = match &inc.share {
        Share::Theme(_) => ("SHARED THEME", Vec2::new(480.0, 330.0)),
        Share::Layout(_) => ("SHARED LAYOUT", Vec2::new(480.0, 330.0)),
        Share::Theater(_) => ("SHARED THEATER", Vec2::new(520.0, 400.0)),
        Share::Settings(_) => ("SHARED SETTINGS", Vec2::new(560.0, 420.0)),
        Share::Playlist(_) => ("SHARED PLAYLIST", Vec2::new(600.0, 470.0)),
        Share::Everything(_) => ("A WHOLE DK.FM", Vec2::new(560.0, 420.0)),
    };
    let mut done: Option<bool> = None;
    let open = super::settings::window(pal, ctx, title, size, true, |ui| {
        let ok = match &inc.share {
            Share::Theme(t) => {
                heading(ui, &pal, &format!("Theme \"{}\" from a friend", t.name));
                ui.add_space(6.0);
                theme_preview(ui, &pal, &Pal::from_custom(t));
                ui.add_space(6.0);
                dim(ui, &pal, "APPLY adds it to your themes (Settings → Look) and switches to it.");
                "APPLY"
            }
            Share::Layout(l) => {
                heading(ui, &pal, &format!("Layout \"{}\" from a friend", l.name));
                ui.add_space(6.0);
                if let Some(d) = &inc.dock {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 170.0), Sense::hover());
                    fill(ui.painter(), r, pal.bg);
                    draw_node(ui.painter(), &pal, d.main_surface(), 0, r.shrink(3.0), 0);
                }
                ui.add_space(6.0);
                dim(ui, &pal, "APPLY saves it to your layouts (Settings → Layouts) and switches to it.");
                "APPLY"
            }
            Share::Theater(t) => {
                heading(ui, &pal, &format!("THEATER \"{}\" from a friend", t.name));
                ui.add_space(6.0);
                theater_preview(ui, &pal, t);
                ui.add_space(8.0);
                ui.label(egui::RichText::new("PUT IT IN PLACE OF").font(px(6.0)).color(pal.text));
                let names: Vec<String> = app.settings.lock().theater.iter().map(|x| x.name.clone()).collect();
                ui.horizontal(|ui| {
                    for (i, n) in names.iter().enumerate() {
                        if button(ui, &pal, &format!("{}. {n}", i + 1), inc.slot == i, true).clicked() {
                            inc.slot = i;
                        }
                    }
                });
                dim(ui, &pal, "APPLY replaces that version of THEATER with this one (your other two stay). Hover THEATER to open it.");
                "APPLY"
            }
            Share::Settings(s) => {
                heading(ui, &pal, "Settings from a friend");
                dim(ui, &pal, "Look & behaviour only. Your Spotify login, folders, library, play counts and backups stay as they are.");
                ui.add_space(6.0);
                let cur = SharedSettings::from(&*app.settings.lock());
                let mut shown = (**s).clone();
                shown.font = shown.font.or(cur.font.clone()); // no font in the code: yours stays
                let (new, old) = (summary(&shown), summary(&cur));
                egui::Grid::new("share-set").num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
                    for ((label, v), (_, was)) in new.iter().zip(&old) {
                        ui.label(egui::RichText::new(*label).color(pal.dim));
                        let changed = v != was;
                        ui.label(egui::RichText::new(if changed { format!("{v}  ◀ CHANGES") } else { v.clone() }).color(if changed { pal.accent2 } else { pal.text }));
                        ui.end_row();
                    }
                });
                "APPLY"
            }
            Share::Playlist(p) => {
                let have = inc.owned.iter().filter(|o| o.is_some()).count();
                let get = p.songs.len() - have;
                heading(ui, &pal, &format!("Playlist \"{}\" from a friend", p.name));
                dim(ui, &pal, &format!("{} songs · {have} in your library · {get} to download", p.songs.len()));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Save as").color(pal.dim));
                    ui.add(egui::TextEdit::singleline(&mut inc.name).desired_width(300.0).font(vt(19.0)));
                });
                ui.add_space(4.0);
                let w = ui.available_width();
                egui::Frame::new().fill(pal.bg).stroke(egui::Stroke::new(2.0_f32, pal.line)).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("share-songs").max_height(250.0).auto_shrink([false, false]).show_rows(ui, 22.0, p.songs.len(), |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for i in range {
                            let s = &p.songs[i];
                            let (r, _) = ui.allocate_exact_size(Vec2::new(w - 4.0, 22.0), Sense::hover());
                            let painter = ui.painter();
                            let cy = r.center().y;
                            painter.text(Pos2::new(r.left() + 30.0, cy), Align2::RIGHT_CENTER, (i + 1).to_string(), vt(16.0), pal.dim);
                            let clip = painter.with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.right() - 170.0, r.bottom())));
                            let g = clip.layout_no_wrap(s.title.clone(), vt(18.0), pal.text);
                            let gw = g.size().x;
                            clip.galley(Pos2::new(r.left() + 38.0, cy - g.size().y / 2.0), g, pal.text);
                            clip.text(Pos2::new(r.left() + 46.0 + gw, cy), Align2::LEFT_CENTER, &s.artist, vt(16.0), pal.dim);
                            if s.secs > 0 {
                                painter.text(Pos2::new(r.right() - 118.0, cy), Align2::RIGHT_CENTER, fmt_time(s.secs as f64), vt(16.0), pal.dim);
                            }
                            let (tag, c) = match (&inc.owned[i], &s.youtube) {
                                (Some(_), _) => ("✔ IN LIBRARY", pal.accent2),
                                (None, Some(_)) => ("↓ YOUTUBE", pal.dim),
                                (None, None) => ("↓ FIND", pal.dim),
                            };
                            painter.text(Pos2::new(r.right() - 8.0, cy), Align2::RIGHT_CENTER, tag, px(6.0), c);
                        }
                    });
                });
                ui.add_space(4.0);
                if get > 0 {
                    dim(ui, &pal, "IMPORT makes the playlist from the songs you have and downloads the rest into it: songs with a YouTube link download that video, the others are found on YouTube Music like Spotify imports.");
                }
                if get > 0 { "IMPORT & DOWNLOAD" } else { "IMPORT" }
            }
            Share::Everything(a) => {
                heading(ui, &pal, &format!("Everything from \"{}\"", a.name));
                let total = a.song_count();
                dim(ui, &pal, &format!("{} playlists · {} liked songs · {} other songs · {} songs in all, {} already here, {} to download", a.playlists.len(), a.liked.len(), a.songs.len(), total, inc.have_all, total.saturating_sub(inc.have_all)));
                ui.add_space(8.0);
                caption(ui, &pal, "BRING IN");
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    switch(ui, &pal, &mut inc.pick[0], &format!("THE {} PLAYLISTS, IN THEIR ORDER", a.playlists.len()));
                    ui.add_space(3.0);
                    switch(ui, &pal, &mut inc.pick[1], &format!("THE {} LIKED SONGS", a.liked.len()));
                    ui.add_space(3.0);
                    switch(ui, &pal, &mut inc.pick[2], &format!("THE {} OTHER SONGS", a.songs.len()));
                    ui.add_space(3.0);
                    switch(ui, &pal, &mut inc.pick[3], "SETTINGS, THEMES AND LAYOUTS");
                });
                ui.add_space(8.0);
                dim(ui, &pal, "Songs you already have are used as they are; the rest download in the background (Downloads shows the progress). Your own playlists, likes and songs stay. Settings you have now are backed up first.");
                "BRING IT IN"
            }
        };
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if button(ui, &pal, "CANCEL", false, true).clicked() {
                done = Some(false);
            }
            let can = !matches!(inc.share, Share::Playlist(_)) || !inc.name.trim().is_empty();
            if button(ui, &pal, ok, true, can).clicked() {
                done = Some(true);
            }
        });
    });
    match done {
        Some(true) => apply(app, ctx, inc),
        None if open => app.incoming = Some(inc),
        _ => {}
    }
}

/// A THEATER version drawn small: its two columns with their parts.
fn theater_preview(ui: &mut Ui, pal: &Pal, t: &crate::store::TheaterPreset) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 170.0), Sense::hover());
    fill(ui.painter(), r, pal.bg);
    frame_rect(ui.painter(), r, 2.0, pal.line);
    let cols: Vec<&Vec<String>> = [&t.left, &t.right].into_iter().filter(|c| !c.is_empty()).collect();
    let w = (r.width() - 12.0 - 8.0 * (cols.len().max(1) - 1) as f32) / cols.len().max(1) as f32;
    for (ci, col) in cols.iter().enumerate() {
        let x = r.left() + 6.0 + ci as f32 * (w + 8.0);
        let h = ((r.height() - 12.0) / col.len().max(1) as f32 - 4.0).min(30.0);
        for (i, k) in col.iter().enumerate() {
            let b = Rect::from_min_size(Pos2::new(x, r.top() + 6.0 + i as f32 * (h + 4.0)), Vec2::new(w, h));
            fill(ui.painter(), b, pal.panel_hi);
            frame_rect(ui.painter(), b, 1.0, pal.line_hi);
            let name = crate::ui::nowplaying::PARTS.iter().find(|p| p.0 == k).map(|p| p.1).unwrap_or("?");
            ui.painter().text(b.center(), Align2::CENTER_CENTER, name, px(6.0), pal.text);
        }
    }
    let extras = [Some(format!("BACKGROUND: {}", t.backdrop.to_uppercase())), t.singalong.then(|| "SING-ALONG".to_string()), t.auto_hide.then(|| "HIDES BUTTONS".to_string())];
    dim(ui, pal, &extras.into_iter().flatten().collect::<Vec<_>>().join(" · "));
}

fn heading(ui: &mut Ui, pal: &Pal, text: &str) {
    ui.label(egui::RichText::new(text).font(vt(24.0)).color(pal.text));
}

fn apply(app: &mut App, ctx: &egui::Context, inc: Incoming) {
    match inc.share {
        Share::Theme(t) => {
            let mut key = String::new();
            app.edit_settings(|s| {
                key = share::add_theme(s, t);
                s.theme = key.clone();
                s.accent = None;
            });
            app.set_theme(ctx);
            app.toast(format!("Theme \"{}\" added to your themes", key.trim_start_matches(super::theme::CUSTOM)));
        }
        Share::Layout(l) => {
            let mut name = String::new();
            app.edit_settings(|s| name = share::add_layout(s, l));
            app.apply_layout(&name);
            app.toast(format!("Layout \"{name}\" saved and applied"));
        }
        Share::Settings(s) => {
            let before = app.settings.lock().clone();
            let _ = crate::backup::create(&app.lib, &before, "manual");
            app.edit_settings(|st| s.apply(st));
            let cur = app.settings.lock().clone();
            app.set_theme(ctx);
            app.apply_look(ctx);
            app.apply_player(&cur);
            app.browser.sort = None;
            app.toast("Your friend's settings are applied (your old ones are in Settings → Backup)");
        }
        Share::Everything(a) => bring_in_all(app, ctx, &a, inc.pick),
        Share::Theater(t) => {
            let (slot, name) = (inc.slot.min(2), t.name.clone());
            app.edit_settings(|s| s.theater[slot] = t);
            app.toast(format!("THEATER \"{name}\" is your version {}: hover THEATER to open it", slot + 1));
        }
        Share::Playlist(p) => {
            let name = inc.name.trim().to_string();
            let missing = import_playlist(app, &p, &name, &inc.owned, "a friend");
            app.toast(if missing == 0 { format!("Imported \"{name}\"") } else { format!("Imported \"{name}\": downloading {missing} song{}", if missing == 1 { "" } else { "s" }) });
        }
    }
}

/// A shared playlist into your library: the songs you have now, the rest download into it in
/// order. Returns how many download. The same playlist shared again fills the one made before.
fn import_playlist(app: &mut App, p: &share::SharedPlaylist, name: &str, owned: &[Option<String>], from: &str) -> usize {
    let name = name.trim().to_string();
    {
            let mut seen = std::collections::HashSet::new();
            let have: Vec<String> = owned.iter().flatten().filter(|id| seen.insert((*id).clone())).cloned().collect();
            let missing: Vec<usize> = owned.iter().enumerate().filter(|(_, o)| o.is_none()).map(|(i, _)| i).collect();
            // the same code again: fill the playlist it made before (unless you've changed it)
            let again = app.lib.data.read().playlists.iter().find(|p| p.name == name && p.source.as_deref() == Some("user") && !p.track_ids.is_empty() && p.track_ids.iter().all(|t| have.contains(t))).map(|p| p.id.clone());
            let pid = match again {
                Some(id) => {
                    app.lib.edit_playlist(&id, |p| p.track_ids = have);
                    id
                }
                None => app.lib.new_playlist(&name, have),
            };
            if !missing.is_empty() {
                let col = Collection { kind: "shared".into(), id: pid.clone(), name: name.clone(), owner: from.into(), tracks: p.songs.iter().map(|s| s.to_itrack()).collect(), complete: true, via: "share".into(), source: "share".into(), ..Default::default() };
                app.dl.start_to(col, Some(missing.clone()), Some(pid.clone()));
            }
            app.browser.set_view(View::Playlist(pid));
            missing.len()
    }
}

/// A whole DK.FM: its playlists, liked songs, other songs and settings, as picked.
fn bring_in_all(app: &mut App, ctx: &egui::Context, a: &share::SharedAll, pick: [bool; 4]) {
    let from = format!("{}'s DK.FM", a.name);
    let mut downloading = 0;
    if pick[3] {
        let before = app.settings.lock().clone();
        let _ = crate::backup::create(&app.lib, &before, "manual");
        app.edit_settings(|st| {
            if let Some(s) = &a.settings {
                s.apply(st);
            }
            for t in &a.themes {
                if !st.custom_themes.iter().any(|c| c.name == t.name) {
                    st.custom_themes.push(t.clone());
                }
            }
            for l in &a.layouts {
                share::add_layout(st, l.clone());
            }
        });
        if let Some(d) = a.dock.as_ref().and_then(super::load_dock) {
            app.remember_layout("Layout from the shared DK.FM", false);
            app.set_dock(d);
        }
        let cur = app.settings.lock().clone();
        app.set_theme(ctx);
        app.apply_look(ctx);
        app.apply_player(&cur);
    }
    let idx = app.lib.key_index();
    let owned_of = |songs: &[share::Song]| -> Vec<Option<String>> { songs.iter().map(|x| x.keys().iter().find_map(|k| idx.get(k).cloned())).collect() };
    if pick[0] {
        for p in &a.playlists {
            let owned = owned_of(&p.songs);
            downloading += import_playlist(app, p, &p.name, &owned, &from);
        }
    }
    // liked songs: the ones here are liked now, the others as they arrive
    if pick[1] && !a.liked.is_empty() {
        let owned = owned_of(&a.liked);
        let have: Vec<String> = owned.iter().flatten().cloned().collect();
        app.lib.set_liked(&have, true);
        let missing: Vec<usize> = owned.iter().enumerate().filter(|(_, o)| o.is_none()).map(|(i, _)| i).collect();
        if !missing.is_empty() {
            downloading += missing.len();
            let col = Collection { kind: "shared".into(), id: "all-liked".into(), name: "Liked".into(), owner: from.clone(), tracks: a.liked.iter().map(|s| s.to_itrack()).collect(), complete: true, via: "share".into(), source: "share".into(), ..Default::default() };
            app.dl.start_to(col, Some(missing), Some(crate::downloader::LIKED.into()));
        }
    }
    if pick[2] && !a.songs.is_empty() {
        let owned = owned_of(&a.songs);
        let missing: Vec<usize> = owned.iter().enumerate().filter(|(_, o)| o.is_none()).map(|(i, _)| i).collect();
        if !missing.is_empty() {
            downloading += missing.len();
            let col = Collection { kind: "shared".into(), id: "all-songs".into(), name: from.clone(), owner: from.clone(), tracks: a.songs.iter().map(|s| s.to_itrack()).collect(), complete: true, via: "share".into(), source: "share".into(), ..Default::default() };
            app.dl.start(col, Some(missing));
        }
    }
    if downloading > 0 {
        app.browser.set_view(View::Downloads);
    }
    app.toast(format!("Brought in {from}{}", if downloading > 0 { format!(": downloading {downloading} songs in the background") } else { String::new() }));
}

/// "Save my whole DK.FM…": a .dkfm file to open on another PC (or give a friend).
pub fn export_all(app: &mut App) {
    let name = app.settings.lock().spotify_user.clone();
    let name = if name.is_empty() { "My".to_string() } else { name };
    let all = {
        let s = app.settings.lock().clone();
        let d = app.lib.data.read();
        share::SharedAll::of(&name, &s, &d)
    };
    let n = all.song_count();
    let Some(path) = rfd::FileDialog::new().set_title("Save your whole DK.FM").set_file_name(format!("{name} DK.FM.{}", share::EXT)).add_filter("DK.FM file", &[share::EXT]).save_file() else { return };
    match std::fs::write(&path, share::file_json(&Share::Everything(Box::new(all)))) {
        Ok(()) => app.toast(format!("Saved your DK.FM ({n} songs, playlists and settings). Open the file on the other PC: drop it on DK.FM.")),
        Err(e) => app.toast_err(format!("Couldn't save the file: {e}")),
    }
}

/// What a settings code sets, as (label, value) rows.
fn summary(s: &SharedSettings) -> Vec<(&'static str, String)> {
    let theme = s.theme.strip_prefix(super::theme::CUSTOM).map(|n| format!("{n} (custom)")).or_else(|| super::theme::THEMES.iter().find(|t| t.0 == s.theme).map(|t| t.1.to_string())).unwrap_or_default();
    let on = |b: bool| if b { "on" } else { "off" };
    let col_names: Vec<&str> = s.columns.iter().filter_map(|c| super::browser::COLUMNS.iter().find(|x| x.id == c).map(|x| x.id)).collect();
    let sources: Vec<&str> = s.search_sources.iter().map(|k| match k.as_str() { "library" => "library", "songs" => "YouTube Music", "youtube" => "YouTube", _ => "SoundCloud" }).collect();
    vec![
        ("Theme", format!("{theme}{}", s.accent.as_ref().map(|a| format!(" · accent {a}")).unwrap_or_default())),
        ("Font & size", format!("{} · {:.0}%", match s.font.as_deref() { Some("clean") => "Clean", Some("pixel") => "Pixel", _ => "Your own font" }, s.zoom * 100.0)),
        ("CRT effects", format!("scanlines {} · glow {}", on(s.scanlines), on(s.glow))),
        ("Song lists", format!("{} · {}", s.density, col_names.join(", "))),
        ("Visualizer", format!("{} · {} fps · {}", s.visualizer, s.vis_fps, if s.vis_bars == 0 { "fit".to_string() } else { format!("{} bars", s.vis_bars) })),
        ("Playback", format!("crossfade {} s · smart shuffle {} · volume matching {}", s.crossfade, on(s.smart_shuffle), on(s.match_volume))),
        ("Equalizer", format!("{} · {}", if s.eq.preset.is_empty() { "Flat" } else { &s.eq.preset }, on(s.eq.enabled))),
        ("Shortcuts", if s.keys.is_empty() { "defaults".into() } else { format!("{} changed", s.keys.len()) }),
        ("Tabs & start", format!("{} hidden · opens to {}", s.sidebar_hidden.len(), s.start_view)),
        ("Search", format!("{} · {} results · {} matching", sources.join(", "), s.search_results, s.match_strictness)),
    ]
}

/// A tiny mock of the player in a palette.
fn theme_preview(ui: &mut Ui, pal: &Pal, t: &Pal) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 170.0), Sense::hover());
    let p = ui.painter();
    fill(p, r.translate(Vec2::splat(4.0)), pal.shadow);
    fill(p, r, t.bg);
    let bar = Rect::from_min_size(r.min, Vec2::new(r.width(), 22.0));
    fill(p, bar, t.bg2);
    p.hline(bar.x_range(), bar.bottom(), egui::Stroke::new(2.0_f32, t.line));
    p.text(bar.left_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, "DK.FM", px(8.0), t.accent);
    // sidebar
    let side = Rect::from_min_max(Pos2::new(r.left(), bar.bottom() + 2.0), Pos2::new(r.left() + 110.0, r.bottom() - 26.0));
    fill(p, side, t.bg2);
    for (i, s) in ["All Tracks", "Liked", "Road trip"].iter().enumerate() {
        let row = Rect::from_min_size(side.min + Vec2::new(0.0, 6.0 + i as f32 * 20.0), Vec2::new(side.width(), 18.0));
        if i == 0 {
            fill(p, row, t.sel);
            fill(p, Rect::from_min_size(row.min, Vec2::new(3.0, 18.0)), t.accent);
        }
        p.text(row.left_center() + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, *s, vt(16.0), if i == 0 { t.accent } else { t.text });
    }
    // song list
    let list = Rect::from_min_max(Pos2::new(side.right() + 6.0, side.top()), Pos2::new(r.right() - 120.0, side.bottom()));
    fill(p, list, t.panel);
    frame_rect(p, list, 2.0, t.line);
    for (i, (a, b)) in [("One More Time", "Daft Punk"), ("Midnight City", "M83"), ("Instant Crush", "Daft Punk"), ("Nightcall", "Kavinsky")].iter().enumerate() {
        let row = Rect::from_min_size(list.min + Vec2::new(2.0, 4.0 + i as f32 * 20.0), Vec2::new(list.width() - 4.0, 18.0));
        if i == 1 {
            fill(p, row, t.panel_hi);
        }
        let clip = p.with_clip_rect(row);
        clip.text(row.left_center() + Vec2::new(6.0, 0.0), Align2::LEFT_CENTER, *a, vt(16.0), t.text);
        clip.text(row.left_center() + Vec2::new(row.width() * 0.6, 0.0), Align2::LEFT_CENTER, *b, vt(16.0), t.dim);
    }
    // display + play button
    let lcd = Rect::from_min_size(Pos2::new(r.right() - 112.0, side.top()), Vec2::new(104.0, 44.0));
    fill(p, lcd, t.lcd_bg);
    frame_rect(p, lcd, 2.0, t.line_hi);
    p.text(lcd.center(), Align2::CENTER_CENTER, "03:14", vt(30.0), t.lcd);
    let play = Rect::from_min_size(Pos2::new(lcd.left(), lcd.bottom() + 8.0), Vec2::new(104.0, 24.0));
    fill(p, play, t.accent);
    p.text(play.center(), Align2::CENTER_CENTER, "▶ PLAY", px(7.0), t.ink);
    let sec = Rect::from_min_size(Pos2::new(lcd.left(), play.bottom() + 6.0), Vec2::new(104.0, 20.0));
    frame_rect(p, sec, 2.0, t.accent2);
    p.text(sec.center(), Align2::CENTER_CENTER, "SHUFFLE", px(6.0), t.accent2);
    // every colour
    let n = super::theme::FIELDS.len() as f32;
    let sw = r.width() / n;
    let mut tp = *t;
    for (i, (f, _)) in super::theme::FIELDS.iter().enumerate() {
        let c = *tp.field(f).unwrap();
        fill(p, Rect::from_min_size(Pos2::new(r.left() + i as f32 * sw, r.bottom() - 18.0), Vec2::new(sw, 18.0)), c);
    }
    frame_rect(p, r, 2.0, pal.line_hi);
}

/// Boxes for a dock layout's panels, split like the real thing.
fn draw_node(p: &egui::Painter, pal: &Pal, tree: &egui_dock::Tree<Tab>, i: usize, r: Rect, depth: u32) {
    if i >= tree.len() || depth > 16 {
        return;
    }
    let split = |f: f32| f.clamp(0.05, 0.95);
    match &tree[NodeIndex(i)] {
        Node::Leaf { tabs, .. } => {
            let b = r.shrink(2.0);
            fill(p, b, pal.panel);
            frame_rect(p, b, 2.0, pal.line_hi);
            let names: Vec<&str> = tabs.iter().map(|t| t.name()).collect();
            p.with_clip_rect(b).text(b.center(), Align2::CENTER_CENTER, names.join(" · "), px(6.0), pal.accent2);
        }
        Node::Horizontal { fraction, .. } => {
            let x = r.left() + r.width() * split(*fraction);
            draw_node(p, pal, tree, 2 * i + 1, Rect::from_min_max(r.min, Pos2::new(x, r.bottom())), depth + 1);
            draw_node(p, pal, tree, 2 * i + 2, Rect::from_min_max(Pos2::new(x, r.top()), r.max), depth + 1);
        }
        Node::Vertical { fraction, .. } => {
            let y = r.top() + r.height() * split(*fraction);
            draw_node(p, pal, tree, 2 * i + 1, Rect::from_min_max(r.min, Pos2::new(r.right(), y)), depth + 1);
            draw_node(p, pal, tree, 2 * i + 2, Rect::from_min_max(Pos2::new(r.left(), y), r.max), depth + 1);
        }
        Node::Empty => {}
    }
}

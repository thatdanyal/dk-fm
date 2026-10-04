//! "Add to playlist" checklist, shared by the song right-click menu and Ctrl+K: type to filter,
//! tick to add, untick to remove. A half-filled box means only some of the songs are in it.
//! Up/Down move · Enter adds to the highlighted playlist and closes.
use super::theme::{px, vt};
use super::widgets::{fill, frame_rect};
use super::{App, Modal, PromptAction};
use eframe::egui::{self, Align2, Id, Key, Pos2, Rect, Sense, Ui, Vec2};
use std::time::Instant;

/// Row id of your (local) Liked songs.
const LIKED: &str = "\u{0}liked";

pub struct Picker {
    pub songs: Vec<String>,
    pub filter: String,
    pub active: usize,
    /// net changes this session (playlist id, name, added) for the summary toast
    changes: Vec<(String, String, bool)>,
    toast: Option<Instant>,
}

impl Picker {
    pub fn new(songs: Vec<String>) -> Self {
        Self { songs, filter: String::new(), active: 1, changes: Vec::new(), toast: None }
    }
}

struct Row {
    id: String,
    name: String,
    icon: &'static str,
    have: usize,
}

fn rows(app: &App, songs: &[String], filter: &str, last_first: bool) -> Vec<Row> {
    let last = if last_first { app.settings.lock().last_playlist.clone() } else { String::new() };
    rows_in(&app.lib.data.read(), songs, filter, &last)
}

/// Liked + playlists in sidebar order (the last one used first, if given), filtered by name.
fn rows_in(d: &crate::store::LibraryData, songs: &[String], filter: &str, last: &str) -> Vec<Row> {
    let toks: Vec<String> = filter.to_lowercase().split_whitespace().map(String::from).collect();
    let mut v = vec![Row { id: LIKED.into(), name: "Liked".into(), icon: "♥", have: songs.iter().filter(|s| d.stats.get(*s).map(|x| x.liked).unwrap_or(false)).count() }];
    // the last used one, then pinned ones, then in your sidebar order
    let mut pls = crate::library::sorted_playlists(d, "custom");
    pls.sort_by_key(|p| (p.id != last, !p.pinned));
    v.extend(pls.into_iter().map(|p| Row { id: p.id.clone(), name: p.name.clone(), icon: super::browser::playlist_icon(p), have: songs.iter().filter(|s| p.track_ids.contains(*s)).count() }));
    if !last.is_empty() && v.get(1).map(|r| r.id == last).unwrap_or(false) {
        v.swap(0, 1);
    }
    v.retain(|r| toks.iter().all(|t| r.name.to_lowercase().contains(t)));
    v
}

fn quoted(names: &[&str]) -> String {
    names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")
}

fn toggle(app: &mut App, pk: &mut Picker, r: &Row) {
    let (added, msg) = toggle_in(&app.lib, pk, r);
    if added && r.id != LIKED {
        app.edit_settings(|s| s.last_playlist = r.id.clone());
    }
    pk.toast = app.toast_update(pk.toast, msg);
}

/// Add the songs to (or, if they're all in it already, remove them from) a row's playlist.
/// Returns (added, summary of this session's changes for the toast).
fn toggle_in(lib: &crate::library::Library, pk: &mut Picker, r: &Row) -> (bool, Option<String>) {
    let all = r.have == pk.songs.len();
    if r.id == LIKED {
        lib.set_liked(&pk.songs, !all);
    } else if all {
        lib.playlist_remove(&r.id, &pk.songs);
    } else {
        lib.playlist_add(&r.id, &pk.songs);
    }
    match pk.changes.iter().position(|c| c.0 == r.id) {
        Some(i) if pk.changes[i].2 == all => {
            pk.changes.remove(i); // undone
        }
        Some(_) => {}
        None => pk.changes.push((r.id.clone(), r.name.clone(), !all)),
    }
    let n = pk.songs.len();
    let what = if n == 1 { String::new() } else { format!(" {n} songs") };
    let added: Vec<&str> = pk.changes.iter().filter(|c| c.2).map(|c| c.1.as_str()).collect();
    let removed: Vec<&str> = pk.changes.iter().filter(|c| !c.2).map(|c| c.1.as_str()).collect();
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("Added{what} to {}", quoted(&added)));
    }
    if !removed.is_empty() {
        parts.push(format!("{}emoved{what} from {}", if parts.is_empty() { "R" } else { "r" }, quoted(&removed)));
    }
    (!all, if parts.is_empty() { None } else { Some(parts.join(" · ")) })
}

fn checkbox(ui: &Ui, pal: &super::theme::Pal, c: Pos2, have: usize, n: usize) {
    let p = ui.painter();
    let cb = Rect::from_center_size(c, Vec2::splat(14.0));
    let all = have == n && n > 0;
    fill(p, cb, if all { pal.accent } else { pal.bg });
    frame_rect(p, cb, 2.0, if have > 0 { pal.accent } else { pal.line_hi });
    if all {
        p.text(cb.center(), Align2::CENTER_CENTER, "✔", vt(16.0), pal.ink);
    } else if have > 0 {
        fill(p, Rect::from_center_size(c, Vec2::new(8.0, 4.0)), pal.accent);
    }
}

/// The list (filter box drawn by the caller). Click toggles; with `keys`, Up/Down/Enter work too.
/// Returns true when the caller should close (Enter, or "+ New playlist…").
pub fn list(app: &mut App, ui: &mut Ui, pk: &mut Picker, w: f32, max_h: f32, last_first: bool, keys: bool) -> bool {
    let pal = app.pal;
    let rows = rows(app, &pk.songs, &pk.filter, last_first);
    let n = rows.len() + 1; // row 0 = "+ New playlist…"
    let (down, up, enter) = if keys { ui.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Enter))) } else { (false, false, false) };
    if down {
        pk.active = (pk.active + 1).min(n - 1);
    }
    if up {
        pk.active = pk.active.saturating_sub(1);
    }
    pk.active = pk.active.min(n - 1);
    let mut hit: Option<(usize, bool)> = if enter { Some((pk.active, true)) } else { None };
    egui::ScrollArea::vertical().id_salt("plpick").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in 0..n {
            let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
            if resp.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                pk.active = i;
            }
            let on = i == pk.active;
            if on {
                fill(ui.painter(), r, pal.sel);
                fill(ui.painter(), Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), pal.accent);
                if down || up {
                    resp.scroll_to_me(None);
                }
            }
            let cy = r.center().y;
            let clip = ui.painter().with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.right() - 34.0, r.bottom())));
            if i == 0 {
                ui.painter().text(Pos2::new(r.left() + 16.0, cy), Align2::CENTER_CENTER, "+", vt(20.0), pal.accent2);
                let f = pk.filter.trim();
                clip.text(Pos2::new(r.left() + 32.0, cy), Align2::LEFT_CENTER, if f.is_empty() || rows.iter().any(|r| r.name.eq_ignore_ascii_case(f)) { "New playlist…".to_string() } else { format!("New playlist \"{f}\"…") }, vt(19.0), pal.accent2);
            } else {
                let row = &rows[i - 1];
                checkbox(ui, &pal, Pos2::new(r.left() + 16.0, cy), row.have, pk.songs.len());
                ui.painter().text(Pos2::new(r.left() + 36.0, cy), Align2::CENTER_CENTER, row.icon, vt(17.0), pal.accent2);
                clip.text(Pos2::new(r.left() + 48.0, cy), Align2::LEFT_CENTER, &row.name, vt(19.0), if row.have > 0 { pal.text } else { pal.dim });
            }
            if resp.clicked() {
                hit = Some((i, false));
            }
        }
        if rows.is_empty() {
            ui.label(egui::RichText::new("   NO PLAYLIST MATCHES").font(px(6.0)).color(pal.dim));
        }
    });
    let Some((i, enter)) = hit else { return false };
    if i == 0 {
        let f = pk.filter.trim();
        app.modal = Some(Modal::Prompt { title: "NEW PLAYLIST".into(), text: if f.is_empty() { "My Playlist".into() } else { f.to_string() }, action: PromptAction::NewPlaylist(pk.songs.clone()) });
        return true;
    }
    let row = &rows[i - 1];
    if !enter || row.have < pk.songs.len() {
        toggle(app, pk, row);
    } else {
        pk.toast = app.toast_update(pk.toast, Some(format!("Already in \"{}\"", row.name)));
    }
    enter
}

/// The floating checklist opened from a song's right-click menu. It stays open while you tick;
/// Esc or a click outside closes it.
pub struct Popup {
    pub pk: Picker,
    pos: Pos2,
    frames: u32,
}

impl Popup {
    pub fn new(songs: Vec<String>, pos: Pos2) -> Self {
        Self { pk: Picker::new(songs), pos, frames: 0 }
    }
}

pub fn show_popup(app: &mut App, ctx: &egui::Context) {
    let Some(mut p) = app.pick.take() else { return };
    let pal = app.pal;
    let screen = ctx.screen_rect();
    let (w, h) = (270.0, 400.0f32.min(screen.height() - 40.0));
    let pos = Pos2::new(p.pos.x.min(screen.right() - w - 24.0).max(screen.left() + 4.0), p.pos.y.min(screen.bottom() - h - 24.0).max(screen.top() + 4.0));
    let n = p.pk.songs.len();
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape)) || app.modal.is_some();
    let area = egui::Area::new(Id::new("plpick-popup")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::new().fill(pal.panel).stroke(egui::Stroke::new(2.0_f32, pal.accent)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.set_width(w);
            ui.label(egui::RichText::new(if n == 1 { "ADD TO PLAYLIST".to_string() } else { format!("ADD {n} SONGS TO PLAYLIST") }).font(px(6.0)).color(pal.dim));
            ui.add_space(4.0);
            let te = ui.add(egui::TextEdit::singleline(&mut p.pk.filter).hint_text("Find a playlist…").desired_width(w).font(vt(19.0)));
            te.request_focus();
            if te.changed() {
                p.pk.active = 1;
            }
            ui.add_space(4.0);
            if list(app, ui, &mut p.pk, w, h - 70.0, false, true) {
                close = true;
            }
        });
    });
    p.frames += 1;
    // a click outside closes it (not on the frame it opened: that click chose the menu item)
    if p.frames > 1 && ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().map(|q| !area.response.rect.contains(q)).unwrap_or(false)) {
        close = true;
    }
    if !close {
        app.pick = Some(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Playlist, Track};

    #[test]
    fn checklist_ticks_and_summary() {
        let (_profile, dir) = crate::store::test_profile("picktest");
        let lib = crate::library::Library::load();
        {
            let mut d = lib.data.write();
            for id in ["a", "b"] {
                d.tracks.insert(id.into(), Track { id: id.into(), title: id.into(), ..Default::default() });
            }
            d.playlists.push(Playlist { id: "mine".into(), name: "Mine".into(), track_ids: vec!["a".into()], ..Default::default() });
            d.playlists.push(Playlist { id: "gym".into(), name: "Gym".into(), ..Default::default() });
            d.playlists.push(Playlist { id: "sp".into(), name: "Liked Songs".into(), spotify_url: Some("spotify:liked".into()), ..Default::default() });
        }
        let mut pk = Picker::new(vec!["a".into(), "b".into()]);
        let row = |pk: &Picker, id: &str| rows_in(&lib.data.read(), &pk.songs, "", "").into_iter().find(|r| r.id == id).unwrap();
        // sidebar order: Liked, Spotify Liked Songs, imported, then yours; the last used one goes first
        let order = |last: &str| rows_in(&lib.data.read(), &pk.songs, "", last).into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(order(""), [LIKED, "sp", "mine", "gym"]);
        assert_eq!(order("gym"), ["gym", LIKED, "sp", "mine"]);
        assert_eq!(rows_in(&lib.data.read(), &pk.songs, "gy", "").len(), 1);
        // one of the two songs is in "Mine": half-ticked; ticking adds the other
        assert_eq!(row(&pk, "mine").have, 1);
        let (added, msg) = { let r = row(&pk, "mine"); toggle_in(&lib, &mut pk, &r) };
        assert!(added && row(&pk, "mine").have == 2);
        assert_eq!(msg.as_deref(), Some("Added 2 songs to \"Mine\""));
        let (_, msg) = { let r = row(&pk, "gym"); toggle_in(&lib, &mut pk, &r) };
        assert_eq!(msg.as_deref(), Some("Added 2 songs to \"Mine\", \"Gym\""));
        // unticking removes them all; undoing an add drops it from the summary
        let (added, msg) = { let r = row(&pk, "mine"); toggle_in(&lib, &mut pk, &r) };
        assert!(!added && row(&pk, "mine").have == 0);
        assert_eq!(msg.as_deref(), Some("Added 2 songs to \"Gym\""));
        let (_, msg) = { let r = row(&pk, "gym"); toggle_in(&lib, &mut pk, &r) };
        assert_eq!(msg, None);
        // Liked ticks like the songs
        { let r = row(&pk, LIKED); toggle_in(&lib, &mut pk, &r) };
        assert!(lib.stat("a").liked && lib.stat("b").liked);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

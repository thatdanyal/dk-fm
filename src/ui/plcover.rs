//! Playlist covers: your own picture, else the source's (Spotify) cover, else a 2x2 mosaic of the
//! first 4 album covers. Covers are made in the background once and kept as small jpgs in the
//! covers folder; the choice per playlist is worked out again only when the library changes.
use super::theme::vt;
use super::widgets::{fill, frame_rect};
use super::App;
use crate::library::{first_covers, mosaic_name, remote_cover_name, Library};
use crate::store::{LibraryData, Playlist};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Ui};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Default)]
pub struct PlCovers {
    /// playlist id -> (stamp it was worked out at, cover file)
    cache: HashMap<String, ((u64, u64), Option<String>)>,
    /// generated files: name -> made (true) or failed (false)
    made: Arc<Mutex<HashMap<String, bool>>>,
    started: HashSet<String>,
    /// bumped when a background cover is done
    done: Arc<AtomicU64>,
}

impl PlCovers {
    pub fn stamp(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    /// The cover file to show for a playlist (None = no art, or not made yet).
    pub fn get(&mut self, lib: &Arc<Library>, d: &LibraryData, p: &Playlist) -> Option<String> {
        let key = (lib.gen.load(Ordering::Relaxed), self.stamp());
        if let Some((k, c)) = self.cache.get(&p.id) {
            if *k == key {
                return c.clone();
            }
        }
        let c = self.work_out(lib, d, p);
        self.cache.insert(p.id.clone(), (key, c.clone()));
        c
    }

    fn work_out(&mut self, lib: &Arc<Library>, d: &LibraryData, p: &Playlist) -> Option<String> {
        // (a restored backup has no picture files: then the next choice is used)
        if let Some(c) = p.custom_cover.as_deref().and_then(|c| self.ready(lib, c, |_| None)) {
            return Some(c);
        }
        if let Some(url) = p.cover.as_deref().filter(|u| u.starts_with("http")) {
            let (name, u) = (remote_cover_name(url), url.to_string());
            let n = name.clone();
            if let Some(c) = self.ready(lib, &name, move |lib| crate::net::get_bytes(&u).ok().and_then(|b| lib.save_playlist_cover(&b, Some(n)))) {
                return Some(c);
            }
        }
        let covers = first_covers(d, p);
        if covers.len() < 4 {
            return covers.into_iter().next();
        }
        let first = covers[0].clone();
        self.ready(lib, &mosaic_name(&covers), move |lib| lib.make_mosaic(&covers)).or(Some(first))
    }

    /// `name` when that file is there; else starts making it in the background (None till done).
    fn ready(&mut self, lib: &Arc<Library>, name: &str, make: impl FnOnce(&Library) -> Option<String> + Send + 'static) -> Option<String> {
        match self.made.lock().get(name) {
            Some(true) => return Some(name.into()),
            Some(false) => return None,
            None => {}
        }
        if lib.cover_path(name).exists() {
            self.made.lock().insert(name.into(), true);
            return Some(name.into());
        }
        if self.started.insert(name.to_string()) {
            let (lib, made, done, name) = (lib.clone(), self.made.clone(), self.done.clone(), name.to_string());
            std::thread::spawn(move || {
                let ok = make(&lib).is_some();
                made.lock().insert(name, ok);
                done.fetch_add(1, Ordering::Relaxed);
                if let Some(c) = crate::system::CTX.lock().as_ref() {
                    c.request_repaint();
                }
            });
        }
        None
    }
}

/// Cover file for a playlist (see `PlCovers`).
pub fn cover_of(app: &mut App, p: &Playlist) -> Option<String> {
    let d = app.lib.data.read();
    app.plcovers.get(&app.lib, &d, p)
}

/// Draws a playlist's cover into `r` (its icon when it has no art); `size` = texture px.
pub fn draw(app: &mut App, ui: &Ui, r: Rect, p: &Playlist, size: u32) {
    let pal = app.pal;
    fill(ui.painter(), r, pal.bg);
    let tex = cover_of(app, p).and_then(|c| app.covers.get(ui.ctx(), app.lib.cover_path(&c), &c, size));
    match tex {
        Some(t) => {
            ui.painter().image(t, r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().text(r.center(), Align2::CENTER_CENTER, super::browser::playlist_icon(p), vt(r.height() * 0.5), pal.faint);
        }
    }
    frame_rect(ui.painter(), r, 2.0, pal.line_hi);
}

/// "Change cover…": pick a picture; it's cropped square and kept in the covers folder.
pub fn change(app: &mut App, pid: &str) {
    let Some(f) = rfd::FileDialog::new().set_title("Choose a playlist cover").add_filter("Images", &["jpg", "jpeg", "png"]).pick_file() else { return };
    match std::fs::read(&f).ok().and_then(|b| app.lib.save_playlist_cover(&b, None)) {
        Some(name) => {
            app.lib.edit_playlist(pid, |p| p.custom_cover = Some(name));
            app.toast("Cover changed");
        }
        None => app.toast_err("That picture couldn't be opened (use a JPG or PNG)"),
    }
}

pub fn reset(app: &mut App, pid: &str) {
    app.lib.edit_playlist(pid, |p| p.custom_cover = None);
}

/// Cover menu entries (in the playlist menu and on the header cover).
pub fn menu(app: &mut App, ui: &mut egui::Ui, p: &Playlist) {
    if ui.button("Change cover…").clicked() {
        ui.close_menu();
        change(app, &p.id);
    }
    if ui.add_enabled(p.custom_cover.is_some(), egui::Button::new("Reset cover")).on_hover_text("Back to the playlist's own cover or the album mosaic").clicked() {
        reset(app, &p.id);
        ui.close_menu();
    }
}

//! "SAVE TO": where a new song goes when you + GET it (or KEEP a preview): your library only,
//! Liked, one of your playlists or a new one. The last playlist used is at the top. Also which
//! folder on your PC its file goes in (DK.FM chooses, unless you pick one).
use super::theme::{px, vt};
use super::widgets::fill;
use super::{App, Modal, PromptAction};
use crate::sources::ITrack;
use eframe::egui::{self, Id, Key, Pos2, Sense, Vec2};
use std::path::{Path, PathBuf};

pub struct Dest {
    t: ITrack,
    pos: Pos2,
    filter: String,
    /// a preview being kept (marked KEEPING once a place is picked)
    preview: bool,
    /// the folder you picked for its file (None: DK.FM chooses, or asks if Settings says so)
    pick: Option<PathBuf>,
    frames: u32,
}

/// Ask where `t` should go, in a small menu at `pos`.
pub fn open(app: &mut App, t: &ITrack, pos: Pos2, preview: bool) {
    app.dest = Some(Dest { t: t.clone(), pos, filter: String::new(), preview, pick: None, frames: 0 });
}

/// A folder picker for downloads, opened where DK.FM would put the songs (or the nearest folder
/// of it that exists yet). None: cancelled.
pub fn pick_folder(auto: &Path, title: &str) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title(title);
    if let Some(start) = auto.ancestors().find(|p| p.is_dir()) {
        d = d.set_directory(start);
    }
    d.pick_folder()
}

/// The folder for a download: `picked` if you already chose one; else, with Settings >
/// Downloads > ASK ME EACH TIME, a folder picker (None: you cancelled, so nothing downloads);
/// else Some(None): DK.FM chooses (`auto`).
pub fn folder_for(app: &App, picked: Option<PathBuf>, auto: &Path) -> Option<Option<PathBuf>> {
    if picked.is_some() || !app.settings.lock().ask_folder {
        return Some(picked);
    }
    pick_folder(auto, "Where should the songs go?").map(Some)
}

/// A folder as a short label: its last two parts ("…\Music\Chill").
pub fn short(p: &Path) -> String {
    let parts: Vec<String> = p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).filter(|s| !s.is_empty() && s != "\\" && s != "/").collect();
    if parts.len() <= 2 { p.display().to_string() } else { format!("…{}{}", std::path::MAIN_SEPARATOR, parts[parts.len() - 2..].join(std::path::MAIN_SEPARATOR_STR)) }
}

/// Download `t` into `target` (None: just the library; `downloader::LIKED`: Liked; else a
/// playlist id), its file in `pick` (None: DK.FM chooses, or asks if Settings says so).
pub fn get(app: &mut App, t: &ITrack, target: Option<String>, preview: bool, pick: Option<PathBuf>) {
    let Some(pick) = folder_for(app, pick, &app.dl.auto_folder("track", &t.title)) else { return };
    if preview {
        super::preview::mark_kept(app, t);
    }
    let name = match target.as_deref() {
        None => None,
        Some(crate::downloader::LIKED) => Some("Liked".to_string()),
        Some(id) => app.lib.data.read().playlists.iter().find(|p| p.id == id).map(|p| p.name.clone()),
    };
    if let Some(id) = target.as_ref().filter(|id| *id != crate::downloader::LIKED) {
        app.edit_settings(|s| s.last_playlist = id.clone());
    }
    super::addsongs::get_in(app, t, target, name, pick);
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.dest.take() else { return };
    let pal = app.pal;
    let screen = ctx.screen_rect();
    let (w, h) = (270.0, 380.0f32.min(screen.height() - 40.0));
    let pos = Pos2::new(d.pos.x.min(screen.right() - w - 24.0).max(screen.left() + 4.0), d.pos.y.min(screen.bottom() - h - 24.0).max(screen.top() + 4.0));
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape)) || app.modal.is_some();
    let mut chosen: Option<Option<String>> = None;
    let mut new_pl = false;
    let (mut change, mut reset) = (false, false);
    let auto = app.dl.auto_folder("track", &d.t.title);
    // your playlists, last used first (a synced one keeps songs you add: sync only adds to it)
    let last = app.settings.lock().last_playlist.clone();
    let mut pls: Vec<(String, String)> = {
        let data = app.lib.data.read();
        crate::library::sorted_playlists(&data, "custom").into_iter().map(|p| (p.id.clone(), p.name.clone())).collect()
    };
    pls.sort_by_key(|p| p.0 != last);
    let toks: Vec<String> = d.filter.to_lowercase().split_whitespace().map(String::from).collect();
    pls.retain(|p| toks.iter().all(|t| p.1.to_lowercase().contains(t)));
    let area = egui::Area::new(Id::new("dest-popup")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::new().fill(pal.panel).stroke(egui::Stroke::new(2.0_f32, pal.accent)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.set_width(w);
            ui.label(egui::RichText::new("SAVE TO").font(px(6.0)).color(pal.dim));
            ui.label(egui::RichText::new(&d.t.title).font(vt(19.0)).color(pal.text));
            ui.add_space(4.0);
            let row = |ui: &mut egui::Ui, icon: &str, name: &str, hint: &str| -> bool {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 24.0), Sense::click());
                if resp.hovered() {
                    fill(ui.painter(), r, pal.panel_hi);
                }
                ui.painter().text(Pos2::new(r.left() + 6.0, r.center().y), egui::Align2::LEFT_CENTER, icon, vt(18.0), pal.accent2);
                ui.painter().with_clip_rect(r.shrink2(Vec2::new(30.0, 0.0))).text(Pos2::new(r.left() + 30.0, r.center().y), egui::Align2::LEFT_CENTER, name, vt(19.0), pal.text);
                if !hint.is_empty() {
                    ui.painter().text(Pos2::new(r.right() - 6.0, r.center().y), egui::Align2::RIGHT_CENTER, hint, px(5.0), pal.dim);
                }
                resp.clicked()
            };
            if row(ui, "♫", "Library only", "") {
                chosen = Some(None);
            }
            if row(ui, "♥", "Liked", "") {
                chosen = Some(Some(crate::downloader::LIKED.to_string()));
            }
            if row(ui, "+", "New playlist…", "") {
                new_pl = true;
            }
            ui.separator();
            if pls.len() > 7 || !d.filter.is_empty() {
                ui.add(egui::TextEdit::singleline(&mut d.filter).hint_text("Find a playlist…").desired_width(w).font(vt(18.0)));
            }
            egui::ScrollArea::vertical().max_height(h - 150.0).show(ui, |ui| {
                if pls.is_empty() {
                    ui.label(egui::RichText::new(if d.filter.is_empty() { "No playlists yet." } else { "No playlist with that name." }).color(pal.dim));
                }
                for (i, (id, name)) in pls.iter().enumerate() {
                    if row(ui, "≡", name, if i == 0 && *id == last { "LAST USED" } else { "" }) {
                        chosen = Some(Some(id.clone()));
                    }
                }
            });
            // the folder its file goes in
            ui.separator();
            ui.label(egui::RichText::new("FOLDER ON YOUR PC").font(px(6.0)).color(pal.dim));
            let label = match &d.pick {
                Some(p) => short(p),
                None if app.settings.lock().ask_folder => "Ask when I pick a place".into(),
                None => format!("DK.FM chooses ({})", short(&auto)),
            };
            if row(ui, ">", &label, "CHANGE") {
                change = true;
            }
            if d.pick.is_some() && row(ui, "<", "Let DK.FM choose", "") {
                reset = true;
            }
        });
    });
    d.frames += 1;
    // a click outside closes it (not on the frame it opened: that click was + GET)
    if d.frames > 1 && ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().map(|q| !area.response.rect.contains(q)).unwrap_or(false)) {
        close = true;
    }
    if change {
        if let Some(p) = pick_folder(d.pick.as_deref().unwrap_or(&auto), "Where should this song go?") {
            d.pick = Some(p);
        }
        // (the click that closed the folder picker isn't a click outside)
        d.frames = 0;
        close = false;
    }
    if reset {
        d.pick = None;
    }
    if let Some(target) = chosen {
        get(app, &d.t, target, d.preview, d.pick.clone());
        return;
    }
    if new_pl {
        // (the folder first: cancelling it makes no empty playlist)
        let Some(pick) = folder_for(app, d.pick.clone(), &auto) else { return };
        app.modal = Some(Modal::Prompt { title: "NEW PLAYLIST".into(), text: "My Playlist".into(), action: PromptAction::NewPlaylistGet(Box::new(d.t.clone()), d.preview, pick) });
        return;
    }
    if !close {
        app.dest = Some(d);
    }
}

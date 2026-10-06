//! "SAVE TO": where a new song goes when you + GET it (or KEEP a preview): your library only,
//! Liked, one of your playlists or a new one. The last playlist used is at the top.
use super::theme::{px, vt};
use super::widgets::fill;
use super::{App, Modal, PromptAction};
use crate::sources::ITrack;
use eframe::egui::{self, Id, Key, Pos2, Sense, Vec2};

pub struct Dest {
    t: ITrack,
    pos: Pos2,
    filter: String,
    /// a preview being kept (marked KEEPING once a place is picked)
    preview: bool,
    frames: u32,
}

/// Ask where `t` should go, in a small menu at `pos`.
pub fn open(app: &mut App, t: &ITrack, pos: Pos2, preview: bool) {
    app.dest = Some(Dest { t: t.clone(), pos, filter: String::new(), preview, frames: 0 });
}

/// Download `t` into `target` (None: just the library; `downloader::LIKED`: Liked; else a
/// playlist id).
pub fn get(app: &mut App, t: &ITrack, target: Option<String>, preview: bool) {
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
    super::addsongs::get_to(app, t, target, name);
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
        });
    });
    d.frames += 1;
    // a click outside closes it (not on the frame it opened: that click was + GET)
    if d.frames > 1 && ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().map(|q| !area.response.rect.contains(q)).unwrap_or(false)) {
        close = true;
    }
    if let Some(target) = chosen {
        get(app, &d.t, target, d.preview);
        return;
    }
    if new_pl {
        app.modal = Some(Modal::Prompt { title: "NEW PLAYLIST".into(), text: "My Playlist".into(), action: PromptAction::NewPlaylistGet(Box::new(d.t.clone()), d.preview) });
        return;
    }
    if !close {
        app.dest = Some(d);
    }
}

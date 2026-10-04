//! Lyrics panel: synced lyrics (click a line to jump there) or plain text. Found by lyricsrc.rs
//! (saved lyrics, the file's tags, LRCLIB, then YouTube captions). The text grows and shrinks with
//! the panel; Ctrl+scroll over it makes it bigger or smaller on top of that.
use super::theme::{px, vt};
use super::App;
pub use crate::lyricsrc::Lyr;
use crate::lyricsrc::Source;
use eframe::egui::{self, Ui};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct LyricsState {
    cache: Arc<Mutex<HashMap<String, (Lyr, Source)>>>,
    last_active: Option<usize>,
}

impl LyricsState {
    /// Keep only lookups still running.
    pub fn trim(&mut self) {
        let mut c = self.cache.lock();
        if c.len() > 8 {
            c.retain(|_, l| matches!(l.0, Lyr::Loading));
        }
    }
}

pub fn show(app: &mut App, ui: &mut Ui) {
    show_sized(app, ui, 1.0);
}

/// Lyrics at `scale` times the size that fits the panel (full-screen now playing uses bigger ones).
pub fn show_sized(app: &mut App, ui: &mut Ui, scale: f32) {
    let pal = app.pal;
    let Some(t) = app.current_track() else {
        ui.centered_and_justified(|ui| ui.label(egui::RichText::new("NO LYRICS").color(pal.dim)));
        return;
    };
    // size follows the panel: narrow panels get smaller text, wide ones bigger (and your own
    // Ctrl+scroll zoom on top)
    let area = ui.available_rect_before_wrap();
    if ui.rect_contains_pointer(area) {
        let zoom = ui.input(|i| i.zoom_delta());
        if zoom != 1.0 {
            let z = (app.settings.lock().lyrics_zoom * zoom).clamp(0.6, 2.5);
            app.edit_settings(|s| s.lyrics_zoom = z);
        }
    }
    let fit = (area.width() / 380.0).min(area.height() / 300.0 + 0.35).clamp(0.75, 1.9);
    let scale = scale * fit * app.settings.lock().lyrics_zoom;
    let entry = app.lyrics.cache.lock().get(&t.id).cloned();
    let (lyr, source) = match entry {
        Some(l) => l,
        None => {
            app.lyrics.cache.lock().insert(t.id.clone(), (Lyr::Loading, Source::Lrclib));
            let cache = app.lyrics.cache.clone();
            let ctx = ui.ctx().clone();
            let t = t.clone();
            std::thread::Builder::new()
                .name("lyrics".into())
                .spawn(move || {
                    let r = crate::lyricsrc::find(&t);
                    match &r.0 {
                        Lyr::Synced(v) => super::cjk::ensure(&ctx, &v.iter().map(|l| l.1.as_str()).collect::<String>()),
                        Lyr::Plain(s) => super::cjk::ensure(&ctx, s),
                        _ => {}
                    }
                    cache.lock().insert(t.id.clone(), r);
                    ctx.request_repaint();
                })
                .ok();
            (Lyr::Loading, Source::Lrclib)
        }
    };
    let mut retry = false;
    match lyr {
        Lyr::Loading => center(ui, "SEARCHING…", &pal),
        Lyr::None => {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 2.0 - 30.0).max(4.0));
                ui.label(egui::RichText::new("NO LYRICS FOUND").font(px(8.0)).color(pal.dim));
                ui.add_space(6.0);
                retry = ui.add(egui::Button::new(egui::RichText::new("LOOK AGAIN").font(px(6.0)))).on_hover_text("Search LRCLIB and YouTube captions again").clicked();
            });
        }
        Lyr::Instrumental => center(ui, "♪ INSTRUMENTAL ♪", &pal),
        Lyr::Plain(s) => {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("lyr").show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.label(egui::RichText::new(s).font(vt(19.0 * scale)).color(pal.text));
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(source.label()).font(px(5.0)).color(pal.faint));
                });
            });
        }
        Lyr::Synced(lines) => {
            let now = app.player.status().position + 0.25;
            let active = lines.iter().rposition(|(t, _)| *t <= now);
            let scroll_now = active != app.lyrics.last_active;
            app.lyrics.last_active = active;
            egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("lyr").show(ui, |ui| {
                ui.add_space(ui.available_height() * 0.35);
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0 * scale;
                    for (i, (t, text)) in lines.iter().enumerate() {
                        let (size, color) = if Some(i) == active { (25.0 * scale, pal.accent) } else if active.map(|a| i < a).unwrap_or(false) { (21.0 * scale, pal.faint) } else { (21.0 * scale, pal.dim) };
                        let txt = if text.is_empty() { "♪" } else { text.as_str() };
                        let r = ui.add(egui::Label::new(egui::RichText::new(txt).font(vt(size)).color(color)).wrap().sense(egui::Sense::click()));
                        if r.clicked() {
                            app.player.seek(*t);
                        }
                        if r.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if Some(i) == active && scroll_now {
                            // jump, don't glide: egui animates scrolls at the full refresh rate,
                            // which cost ~10 extra frames a second while lyrics played
                            r.scroll_to_me_animation(Some(egui::Align::Center), egui::style::ScrollAnimation::none());
                        }
                    }
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new(source.label()).font(px(5.0)).color(pal.faint));
                });
                ui.add_space(ui.available_height() * 0.35 + 200.0);
            });
        }
    }
    if retry {
        crate::lyricsrc::forget(&t.id);
        app.lyrics.cache.lock().remove(&t.id);
    }
}

fn center(ui: &mut Ui, s: &str, pal: &super::theme::Pal) {
    ui.centered_and_justified(|ui| ui.label(egui::RichText::new(s).font(px(8.0)).color(pal.dim)));
}

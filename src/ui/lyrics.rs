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
    /// synced lyrics: the line being glided to, where each line sat last frame (centre, from the
    /// top of the text) and the glide (from, to, started)
    target: Option<usize>,
    line_y: Vec<f32>,
    glide: Option<(f32, f32, std::time::Instant)>,
    offset: f32,
    song: String,
    last_pos: f64,
}

/// How long before a line starts the text glides to it, so it's centred right as it's sung.
const LEAD: f64 = 0.45;

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
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
            // a line stays lit until the next one starts, then the highlight moves straight to it
            // (no fading, no greyed-out line left behind); LEAD seconds before that, the text
            // glides (eased) so the next line arrives centred exactly when it begins. Every line
            // is the same size: nothing jumps or re-wraps.
            let pos = app.player.status().position;
            let active = lines.iter().rposition(|(t, _)| *t <= pos);
            let next_at = lines.get(active.map(|a| a + 1).unwrap_or(0)).map(|l| l.0);
            let until = next_at.map(|n| n - pos).unwrap_or(f64::MAX);
            let target = if until <= LEAD { Some(active.map(|a| a + 1).unwrap_or(0)) } else { active };
            let st = &mut app.lyrics;
            // a new song, a seek or a jump of more than one line: snap there, no glide
            if st.song != t.id {
                st.line_y.clear(); // (the last song's line positions)
                st.glide = None;
            }
            let jumped = st.song != t.id || pos < st.last_pos - 0.5 || pos > st.last_pos + 2.0;
            st.song = t.id.clone();
            st.last_pos = pos;
            let view_h = ui.available_height();
            let goal = |st: &LyricsState, i: usize| st.line_y.get(i).map(|y| (y - view_h / 2.0).max(0.0));
            let mut force: Option<f32> = None;
            if target != st.target || jumped {
                let to = target.and_then(|i| goal(st, i));
                match (to, jumped || st.target.is_none()) {
                    (Some(to), true) => force = Some(to),
                    (Some(to), false) => st.glide = Some((st.offset, to, std::time::Instant::now())),
                    _ => {}
                }
                st.target = target;
            }
            let dur = (LEAD as f32).min(until.max(0.15) as f32);
            if let Some((from, to, t0)) = st.glide {
                let k = ease(t0.elapsed().as_secs_f32() / dur.max(0.15));
                force = Some(from + (to - from) * k);
                if k >= 1.0 {
                    st.glide = None;
                } else {
                    ui.ctx().request_repaint();
                }
            }
            // wake up in time for the next glide, and to light the next line the moment it starts
            if until.is_finite() {
                let wait = if until > LEAD { until - LEAD } else { until };
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.01)));
            }
            let size = 22.0 * scale;
            let mut sa = egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("lyr");
            if let Some(o) = force {
                sa = sa.vertical_scroll_offset(o);
            }
            let mut ys = Vec::with_capacity(lines.len());
            let out = sa.show(ui, |ui| {
                let top = ui.min_rect().top();
                ui.add_space(view_h * 0.5);
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0 * scale;
                    for (i, (lt, text)) in lines.iter().enumerate() {
                        let color = if Some(i) == active { pal.accent } else { pal.dim };
                        let txt = if text.is_empty() { "♪" } else { text.as_str() };
                        let r = ui.add(egui::Label::new(egui::RichText::new(txt).font(vt(size)).color(color)).wrap().sense(egui::Sense::click()));
                        ys.push(r.rect.center().y - top);
                        if r.clicked() {
                            app.player.seek(*lt);
                        }
                        if r.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new(source.label()).font(px(5.0)).color(pal.faint));
                });
                ui.add_space(view_h * 0.5);
            });
            let st = &mut app.lyrics;
            st.offset = out.state.offset.y;
            let first = st.line_y.is_empty();
            st.line_y = ys;
            // the first frame of a song had no positions yet: centre the line next frame
            if first || force.is_none() && st.target.is_some() && jumped {
                st.target = None;
                ui.ctx().request_repaint();
            }
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

//! THEATER (full-screen now playing): big cover, title / artist / album, seek bar, controls, and synced
//! lyrics beside it on wide windows. Esc (or F11) leaves. Nothing extra repaints: it redraws on
//! the same playback tick as the deck.
use super::deck::tbtn;
use super::theme::{px, vt};
use super::widgets::{fill, fmt_time, frame_rect, tb_button};
use super::App;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Ui, Vec2};

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) && app.modal.is_none() && app.palette.is_none() && app.incoming.is_none() {
        app.nowplaying = false;
        return;
    }
    let st = app.player.status();
    let t = app.current_track();
    let full = ui.max_rect();
    let wide = full.width() >= 900.0;
    let left = if wide { Rect::from_min_max(full.min, Pos2::new(full.center().x, full.bottom())) } else { full };
    // cover: as big as fits above the text and controls
    let side = (left.width() - 80.0).min(left.height() - 330.0).clamp(120.0, 620.0);
    let art = Rect::from_min_size(Pos2::new(left.center().x - side / 2.0, left.top() + 28.0), Vec2::splat(side));
    fill(ui.painter(), art.translate(Vec2::splat(6.0)), pal.shadow);
    fill(ui.painter(), art, pal.bg2);
    let tex = t.as_ref().and_then(|t| t.cover.clone()).and_then(|c| app.covers.get(ui.ctx(), app.lib.cover_path(&c), &c, 512));
    match tex {
        Some(id) => {
            ui.painter().image(id, art, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().text(art.center(), Align2::CENTER_CENTER, "♫", vt(side * 0.4), pal.faint);
        }
    }
    frame_rect(ui.painter(), art, 2.0, pal.line_hi);
    // title, artist, album, seek bar, controls
    let below = Rect::from_min_max(Pos2::new(left.left() + 40.0, art.bottom() + 18.0), Pos2::new(left.right() - 40.0, left.bottom() - 12.0));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(below), |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.vertical_centered(|ui| {
            let line = |ui: &mut Ui, s: &str, f: egui::FontId, c: Color32| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), f.size + 4.0), Sense::hover());
                ui.painter().with_clip_rect(r).text(r.center(), Align2::CENTER_CENTER, s, f, c);
            };
            match &t {
                Some(t) => {
                    line(ui, &t.title, vt(36.0), pal.text);
                    line(ui, &t.artist, vt(25.0), pal.accent2);
                    let al = [t.album.clone(), t.year.map(|y| y.to_string()).unwrap_or_default()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                    line(ui, &al, vt(19.0), pal.dim);
                }
                None => line(ui, "NOTHING PLAYING", px(12.0), pal.dim),
            }
            ui.add_space(8.0);
            let w = ui.available_width().min(side.max(420.0));
            let (wr, resp) = ui.allocate_exact_size(Vec2::new(w, 44.0), Sense::click_and_drag());
            super::deck::waveform(app, ui, wr, &resp, &st);
            let (tr, _) = ui.allocate_exact_size(Vec2::new(w, 20.0), Sense::hover());
            let dur = if st.duration > 0.0 { st.duration } else { t.as_ref().map(|t| t.duration).unwrap_or(0.0) };
            ui.painter().text(tr.left_center(), Align2::LEFT_CENTER, fmt_time(st.position), vt(19.0), pal.dim);
            ui.painter().text(tr.right_center(), Align2::RIGHT_CENTER, fmt_time(dur), vt(19.0), pal.dim);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let total = 46.0 * 4.0 + 62.0 + 46.0 + 8.0 * 5.0 + 20.0;
                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                let opts = app.player.st.lock().opts.clone();
                if tbtn(ui, &pal, "SHUF", Vec2::new(46.0, 30.0), opts.shuffle, false).clicked() {
                    app.player.toggle_shuffle();
                }
                if tbtn(ui, &pal, "⏮", Vec2::new(46.0, 40.0), false, false).clicked() {
                    app.player.prev();
                }
                if tbtn(ui, &pal, if st.playing { "⏸" } else { "▶" }, Vec2::new(62.0, 50.0), false, true).clicked() {
                    app.player.toggle();
                }
                if tbtn(ui, &pal, "⏭", Vec2::new(46.0, 40.0), false, false).clicked() {
                    app.player.next(true);
                }
                if tbtn(ui, &pal, if opts.repeat == "one" { "RPT1" } else { "RPT" }, Vec2::new(46.0, 30.0), opts.repeat != "off", false).clicked() {
                    app.player.cycle_repeat();
                }
                let liked = t.as_ref().map(|t| app.lib.stat(&t.id).liked).unwrap_or(false);
                if tbtn(ui, &pal, "♥", Vec2::new(46.0, 30.0), liked, false).clicked() {
                    if let Some(t) = &t {
                        app.lib.toggle_like(&t.id);
                    }
                }
            });
        });
    });
    // synced lyrics on the right
    if wide {
        let lr = Rect::from_min_max(Pos2::new(full.center().x + 8.0, full.top() + 56.0), full.max - Vec2::new(28.0, 20.0));
        fill(ui.painter(), lr, pal.bg2);
        frame_rect(ui.painter(), lr, 2.0, pal.line);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(lr.shrink(4.0)), |ui| {
            ui.set_clip_rect(lr.shrink(2.0));
            super::lyrics::show_sized(app, ui, 1.35);
        });
    }
    // leave
    let xr = Rect::from_min_size(Pos2::new(full.right() - 150.0, full.top() + 16.0), Vec2::new(130.0, 26.0));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(xr).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
        if tb_button(ui, &pal, "× CLOSE · ESC", false).on_hover_text("Back to DK.FM (Esc or F11)").clicked() {
            app.nowplaying = false;
        }
    });
}

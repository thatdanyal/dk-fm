//! Now-playing deck: cover, LCD readout, waveform seek bar, transport, volume, like, sleep timer.
use super::theme::{px, vt};
use super::widgets::{fill, fmt_time, frame_rect, with_alpha};
use super::App;
use crate::player::Sleep;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use std::time::{Duration, Instant};

const CASSETTE: &[&str] = &[
    "################################",
    "#..............................#",
    "#..####################.......#.",
    "#..#..................#.......#.",
    "#..####################.......#.",
    "#..............................#",
    "#......##################......#",
    "#......#..##........##..#......#",
    "#......#..##........##..#......#",
    "#......##################......#",
    "#..............................#",
    "#........##############........#",
    "################################",
];

pub fn show(app: &mut App, ui: &mut Ui) {
    let avail = ui.available_size();
    // a wide, short panel (along the bottom, like Spotify's): the player bar
    if avail.x >= 640.0 && avail.y < 200.0 && avail.y >= 44.0 {
        return bar(app, ui);
    }
    // the deck always fits: smaller panels get a compact deck (the controls never disappear)
    if avail.y < 280.0 || avail.x < 300.0 {
        return compact(app, ui);
    }
    let pal = app.pal;
    let st = app.player.status();
    let track = app.current_track();
    // cover + display shrink with the panel; big ones keep the full display
    let top = (avail.y - 224.0).min(avail.x * 0.4).clamp(64.0, 168.0).round();
    egui::Frame::new().inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        // ---- cover + LCD
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(top), Sense::hover());
            cover(app, ui, r, track.as_ref(), 256);
            let w = ui.available_width();
            let (lr, _) = ui.allocate_exact_size(Vec2::new(w, top), Sense::hover());
            lcd(app, ui, lr, &st, track.as_ref(), top < 128.0);
        });
        // ---- waveform seek bar
        let wh = (ui.available_height() - 144.0).clamp(28.0, 54.0);
        let (wr, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), wh), Sense::click_and_drag());
        waveform(app, ui, wr, &resp, &st);
        // ---- transport
        transport(app, ui, &st, 1.0);
        // ---- like / volume / sleep
        ui.horizontal(|ui| {
            let liked = track.as_ref().map(|t| app.lib.stat(&t.id).liked).unwrap_or(false);
            if icon(ui, &pal, "♥", liked).on_hover_text("Like").clicked() {
                if let Some(t) = &track {
                    app.lib.toggle_like(&t.id);
                }
            }
            volume(app, ui, 170.0);
            let private = app.lib.private.load(std::sync::atomic::Ordering::Relaxed);
            if icon(ui, &pal, "PRV", private).on_hover_text(if private { "Private listening is on: plays and history aren't recorded" } else { "Private listening: don't record plays and history" }).clicked() {
                app.set_private(!private);
            }
            if icon(ui, &pal, "🗖", false).on_hover_text("THEATER: the song big, with its lyrics (F11)").clicked() {
                app.open_theater(None);
            }
            sleep_button(app, ui);
        });
        // ---- SHAZAM, its own button along the bottom (KEEP / DISCARD while a preview plays)
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 10.0).min(300.0);
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            if !super::preview::keep_bar(app, ui, w, 26.0) {
                ui.add_space(((w - 150.0) / 2.0).max(0.0));
                shazam_wide(app, ui, 150.0);
            }
        });
    });
}

/// The SHAZAM button as a wide bar (`w` px).
fn shazam_wide(app: &mut App, ui: &mut Ui, w: f32) {
    let pal = app.pal;
    let busy = matches!(*app.browser.web.listen.lock(), super::websearch::Listen::Busy);
    let r = tbtn(ui, &pal, if busy { "LISTENING…" } else { "♫ SHAZAM" }, Vec2::new(w, 26.0), busy, false);
    app.mark("shazam", r.rect);
    if r.on_hover_text("Shazam: name a song playing on this PC or around you (with the microphone)").clicked() && !busy {
        super::websearch::listen(app, ui.ctx());
    }
}

/// SHAZAM: name a song playing on this PC or near the microphone (the answer floats above the
/// window).
fn shazam_button(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let busy = matches!(*app.browser.web.listen.lock(), super::websearch::Listen::Busy);
    let r = icon(ui, &pal, if busy { "…" } else { "SHAZAM" }, busy);
    app.mark("shazam", r.rect);
    if r.on_hover_text("Shazam: name a song playing on this PC or around you (with the microphone)").clicked() && !busy {
        super::websearch::listen(app, ui.ctx());
    }
}

/// Shuffle · previous · play/pause · next · repeat, centred; `k` scales the buttons.
fn transport(app: &mut App, ui: &mut Ui, st: &crate::audio::Status, k: f32) {
    let pal = app.pal;
    // one row as tall as the play button, every button centred on it (so they line up)
    ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 53.0 * k), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let total = (46.0 * 4.0 + 62.0 + 8.0 * 4.0) * k;
        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
        let opts = app.player.st.lock().opts.clone();
        if shuffle_btn(ui, &pal, Vec2::new(46.0, 30.0) * k, opts.shuffle).clicked() {
            app.player.toggle_shuffle();
        }
        if tbtn(ui, &pal, "⏮", Vec2::new(46.0, 40.0) * k, false, false).on_hover_text("Previous").clicked() {
            app.player.prev();
        }
        if tbtn(ui, &pal, if st.playing { "⏸" } else { "▶" }, Vec2::new(62.0, 50.0) * k, false, true).on_hover_text(if st.playing { "Pause" } else { "Play" }).clicked() {
            app.player.toggle();
        }
        if tbtn(ui, &pal, "⏭", Vec2::new(46.0, 40.0) * k, false, false).on_hover_text("Next").clicked() {
            app.player.next(true);
        }
        if repeat_btn(ui, &pal, Vec2::new(46.0, 30.0) * k, &opts.repeat).clicked() {
            app.player.cycle_repeat();
        }
    });
}

/// Shuffle, like Spotify's: two arrows crossing over each other (lit when on).
pub(super) fn shuffle_btn(ui: &mut Ui, pal: &super::theme::Pal, size: Vec2, on: bool) -> egui::Response {
    let resp = tbtn(ui, pal, "", size, on, false);
    let body = Rect::from_min_size(resp.rect.min + if resp.is_pointer_button_down_on() { Vec2::splat(3.0) } else { Vec2::ZERO }, size);
    let fg = if on { pal.ink } else if resp.hovered() { pal.accent } else { pal.text };
    let w = (size.y * 0.8).min(size.x * 0.6).round();
    let h = (w * 0.62).round();
    let ir = Rect::from_center_size(body.center(), Vec2::new(w, h));
    let a = (h * 0.36).max(3.0);
    let p = ui.painter();
    // each arrow: in at one height on the left, crossing the other, out at the other height
    for (y0, y1) in [(ir.bottom(), ir.top()), (ir.top(), ir.bottom())] {
        let end = ir.right() - a * 0.9;
        let pts = [Pos2::new(ir.left(), y0), Pos2::new(ir.left() + w * 0.42, y0), Pos2::new(end - w * 0.38, y1), Pos2::new(end, y1)];
        p.add(egui::epaint::CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, Stroke::new(2.0_f32, fg)));
        let tip = Pos2::new(ir.right(), y1);
        p.add(egui::Shape::convex_polygon(vec![tip, tip + Vec2::new(-a * 1.1, -a * 0.75), tip + Vec2::new(-a * 1.1, a * 0.75)], fg, Stroke::NONE));
    }
    resp.on_hover_text(if on { "Shuffle: on (press to play in order)" } else { "Shuffle" })
}

/// Repeat, like Spotify's: an arrow looping around itself (lit when on, with a 1 when it repeats
/// the song). Each press goes off > all > this song > off.
pub(super) fn repeat_btn(ui: &mut Ui, pal: &super::theme::Pal, size: Vec2, mode: &str) -> egui::Response {
    let on = mode != "off";
    let resp = tbtn(ui, pal, "", size, on, false);
    let body = Rect::from_min_size(resp.rect.min + if resp.is_pointer_button_down_on() { Vec2::splat(3.0) } else { Vec2::ZERO }, size);
    let fg = if on { pal.ink } else if resp.hovered() { pal.accent } else { pal.text };
    let w = (size.y * 0.75).min(size.x * 0.55).round();
    let h = (w * 0.6).round();
    let ir = Rect::from_center_size(body.center(), Vec2::new(w, h));
    let p = ui.painter();
    p.rect_stroke(ir, egui::CornerRadius::same((h / 2.0) as u8), Stroke::new(2.0_f32, fg), egui::StrokeKind::Middle);
    // arrowheads: along the top pointing right, along the bottom pointing left
    let a = (h * 0.42).max(3.0);
    for (tip, dir) in [(Pos2::new(ir.left() + w * 0.72, ir.top()), 1.0_f32), (Pos2::new(ir.left() + w * 0.28, ir.bottom()), -1.0)] {
        let pts = vec![tip + Vec2::new(a * 0.6 * dir, 0.0), tip + Vec2::new(-a * 0.6 * dir, -a * 0.75), tip + Vec2::new(-a * 0.6 * dir, a * 0.75)];
        p.add(egui::Shape::convex_polygon(pts, fg, Stroke::NONE));
    }
    if mode == "one" {
        // a small "1" badge in the corner
        let c = Pos2::new(body.right() - 7.5, body.top() + 7.5);
        p.circle_filled(c, 6.5, fg);
        p.text(c + Vec2::new(0.5, 0.5), Align2::CENTER_CENTER, "1", px(6.0), pal.accent2);
    }
    resp.on_hover_text(match mode { "all" => "Repeat: all (press to repeat this song)", "one" => "Repeat: this song (press to turn off)", _ => "Repeat (press to repeat all)" })
}

/// Mute button + volume slider + percentage, leaving `reserve` px for what follows.
pub(super) fn volume(app: &mut App, ui: &mut Ui, reserve: f32) {
    let pal = app.pal;
    let (muted, mut vol) = { let s = app.player.st.lock(); (s.muted, s.opts.volume) };
    if icon(ui, &pal, if muted || vol == 0.0 { "🔇" } else { "🔊" }, muted).on_hover_text(if muted { "Unmute" } else { "Mute" }).clicked() {
        app.player.toggle_mute();
    }
    let sw = (ui.available_width() - reserve).max(40.0);
    ui.spacing_mut().slider_width = sw;
    if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false)).on_hover_text("Volume").changed() {
        app.player.set_volume(vol);
    }
    if reserve >= 60.0 {
        ui.label(egui::RichText::new(if muted { "--".to_string() } else { format!("{:>3}", (vol * 100.0).round()) }).color(pal.dim));
    }
}

fn sleep_button(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let sleep = app.player.st.lock().sleep;
    let label = match sleep { Sleep::Off => "ZZ".to_string(), Sleep::EndOfTrack => "EOT".into(), Sleep::At(t) => format!("{}M", (t.saturating_duration_since(Instant::now()).as_secs() / 60 + 1)) };
    let r = icon(ui, &pal, &label, sleep != Sleep::Off).on_hover_text("Sleep timer");
    let id = ui.make_persistent_id("sleep-menu");
    if r.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup::popup_below_widget(ui, id, &r, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(170.0);
        if ui.button("Off").clicked() { app.player.set_sleep(Sleep::Off); }
        if ui.button("End of this song").clicked() { app.player.set_sleep(Sleep::EndOfTrack); }
        for m in [10u64, 15, 30, 45, 60, 90] {
            if ui.button(format!("{m} minutes")).clicked() {
                app.player.set_sleep(Sleep::At(Instant::now() + Duration::from_secs(m * 60)));
            }
        }
    });
}

/// The deck along the bottom of the window, like Spotify's player bar: the song (cover, title,
/// artist, ♥ right after the title) on the left, the controls over the seek bar in the middle of
/// the window, and SHAZAM, the sleep timer, THEATER and the volume packed on the right.
fn bar(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let st = app.player.status();
    let track = app.current_track();
    let full = ui.available_rect_before_wrap().shrink2(Vec2::new(14.0, 4.0));
    ui.allocate_rect(full, Sense::hover());
    let w = full.width();
    // the controls sit in the middle of the bar, whatever is either side of them
    let mid_w = (w * 0.40).clamp(360.0, 620.0).min(w - 2.0 * 230.0).max(300.0);
    let mid = Rect::from_center_size(full.center(), Vec2::new(mid_w, full.height()));
    let left = Rect::from_min_max(full.min, Pos2::new(mid.left() - 16.0, full.bottom()));
    let right = Rect::from_min_max(Pos2::new(mid.right() + 16.0, full.top()), full.max);
    // ---- the song: cover, title (♥ right after it) and artist
    let side = (full.height() - 6.0).clamp(34.0, 64.0);
    let cr = Rect::from_min_size(Pos2::new(left.left(), left.center().y - side / 2.0), Vec2::splat(side));
    cover(app, ui, cr, track.as_ref(), 128);
    let tx = cr.right() + 12.0;
    let heart_w = 30.0;
    let text_max = (left.right() - tx - heart_w).max(20.0);
    let title = track.as_ref().map(|t| t.title.clone()).unwrap_or_else(|| "NO TRACK LOADED".into());
    let artist = track.as_ref().map(|t| t.artist.clone()).unwrap_or_default();
    let cy = cr.center().y;
    let tg = ui.painter().layout_no_wrap(title, vt(21.0), pal.text);
    let title_w = tg.size().x.min(text_max);
    {
        let p = ui.painter_at(Rect::from_min_max(Pos2::new(tx, left.top()), Pos2::new(tx + text_max, left.bottom())));
        p.galley(Pos2::new(tx, cy - 1.0 - tg.size().y), tg, pal.text);
        p.text(Pos2::new(tx, cy + 2.0), Align2::LEFT_TOP, artist, vt(17.0), pal.dim);
    }
    if track.is_some() {
        let hr = Rect::from_min_size(Pos2::new(tx + title_w + 4.0, cy - 1.0 - 26.0), Vec2::new(heart_w, 26.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(hr).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            let liked = track.as_ref().map(|t| app.lib.stat(&t.id).liked).unwrap_or(false);
            if icon(ui, &pal, "♥", liked).on_hover_text(if liked { "Liked: click to unlike" } else { "Like" }).clicked() {
                if let Some(t) = &track {
                    app.lib.toggle_like(&t.id);
                }
            }
        });
    }
    // ---- controls over the seek bar (the times either side of it)
    let seek_h = if full.height() < 56.0 { 12.0 } else { 18.0 };
    let k = ((full.height() - seek_h - 6.0) / 53.0).clamp(0.42, 0.72);
    let gap = 4.0;
    let block = 53.0 * k + gap + seek_h;
    let top = mid.center().y - block / 2.0;
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(Rect::from_min_size(Pos2::new(mid.left(), top), Vec2::new(mid_w, 53.0 * k))), |ui| {
        transport(app, ui, &st, k);
    });
    let sy = top + 53.0 * k + gap;
    let time_w = 48.0;
    let pp = ui.painter();
    pp.text(Pos2::new(mid.left() + time_w - 6.0, sy + seek_h / 2.0), Align2::RIGHT_CENTER, fmt_time(st.position), vt(17.0), pal.dim);
    pp.text(Pos2::new(mid.right() - time_w + 6.0, sy + seek_h / 2.0), Align2::LEFT_CENTER, fmt_time(st.duration.max(0.0)), vt(17.0), pal.dim);
    let wr = Rect::from_min_max(Pos2::new(mid.left() + time_w, sy), Pos2::new(mid.right() - time_w, sy + seek_h));
    let resp = ui.interact(wr, ui.id().with("bar-seek"), Sense::click_and_drag());
    waveform(app, ui, wr, &resp, &st);
    // ---- on the right, packed against the edge: the volume last (like Spotify), then THEATER,
    // the sleep timer and SHAZAM (KEEP / DISCARD while a preview plays)
    let vol_w = (right.width() * 0.45).clamp(110.0, 170.0);
    let vr = Rect::from_min_max(Pos2::new(right.right() - vol_w, right.top()), right.max);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(vr).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        volume(app, ui, 4.0);
    });
    let rest = Rect::from_min_max(right.min, Pos2::new(vr.left() - 8.0, right.bottom()));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rest).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
        if ui.available_width() > 40.0 && icon(ui, &pal, "🗖", false).on_hover_text("THEATER: the song big, with its lyrics (F11)").clicked() {
            app.open_theater(None);
        }
        if ui.available_width() > 50.0 {
            sleep_button(app, ui);
        }
        let preview = app.player.current_id().is_some_and(|id| id.starts_with(crate::library::PREVIEW));
        if preview && ui.available_width() > 130.0 {
            let kw = ui.available_width().min(200.0);
            super::preview::keep_bar(app, ui, kw, 28.0);
        } else if !preview && ui.available_width() > 90.0 {
            shazam_button(app, ui);
        }
    });
}

/// The deck in a small panel: song + seek bar when there's room, and always play/pause, previous,
/// next and the volume.
fn compact(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let st = app.player.status();
    let track = app.current_track();
    egui::Frame::new().inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        let h = ui.available_height();
        let w = ui.available_width();
        // ---- song (cover + title / artist)
        if h >= 84.0 {
            let side = (h - 50.0 - if h >= 130.0 { 34.0 } else { 0.0 }).clamp(30.0, 64.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
                cover(app, ui, r, track.as_ref(), 128);
                let (tr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), side), Sense::hover());
                let p = ui.painter_at(tr);
                let title = track.as_ref().map(|t| t.title.clone()).unwrap_or_else(|| "NO TRACK LOADED".into());
                let artist = track.as_ref().map(|t| t.artist.clone()).unwrap_or_default();
                let cy = tr.center().y;
                let two = side >= 40.0;
                p.text(Pos2::new(tr.left(), if two { cy - 2.0 } else { cy }), if two { Align2::LEFT_BOTTOM } else { Align2::LEFT_CENTER }, title, vt(21.0), pal.text);
                if two {
                    p.text(Pos2::new(tr.left(), cy + 2.0), Align2::LEFT_TOP, artist, vt(17.0), pal.dim);
                }
            });
        }
        // ---- seek bar
        if h >= 130.0 {
            let (wr, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click_and_drag());
            waveform(app, ui, wr, &resp, &st);
        }
        // ---- controls (always)
        let k = if h < 44.0 { ((h - 4.0) / 50.0).clamp(0.5, 0.8) } else { 0.8 };
        ui.horizontal(|ui| {
            if tbtn(ui, &pal, "⏮", Vec2::new(40.0, 34.0) * k, false, false).on_hover_text("Previous").clicked() {
                app.player.prev();
            }
            if tbtn(ui, &pal, if st.playing { "⏸" } else { "▶" }, Vec2::new(48.0, 40.0) * k, false, true).on_hover_text(if st.playing { "Pause" } else { "Play" }).clicked() {
                app.player.toggle();
            }
            if tbtn(ui, &pal, "⏭", Vec2::new(40.0, 34.0) * k, false, false).on_hover_text("Next").clicked() {
                app.player.next(true);
            }
            let row = h >= 160.0; // room for a row of its own below
            if !row && ui.available_width() > 170.0 {
                let preview = app.player.current_id().is_some_and(|id| id.starts_with(crate::library::PREVIEW));
                volume(app, ui, if preview { 200.0 } else if ui.available_width() > 280.0 { 140.0 } else { 80.0 });
                if !(preview && super::preview::keep_bar(app, ui, 190.0, 34.0 * k)) {
                    shazam_button(app, ui);
                }
            } else if ui.available_width() > 90.0 {
                volume(app, ui, if ui.available_width() > 200.0 { 60.0 } else { 8.0 });
            } else {
                let muted = app.player.st.lock().muted;
                if icon(ui, &pal, if muted { "🔇" } else { "🔊" }, muted).on_hover_text(if muted { "Unmute" } else { "Mute" }).clicked() {
                    app.player.toggle_mute();
                }
            }
        });
        if h >= 160.0 {
            ui.horizontal(|ui| {
                let liked = track.as_ref().map(|t| app.lib.stat(&t.id).liked).unwrap_or(false);
                if icon(ui, &pal, "♥", liked).on_hover_text("Like").clicked() {
                    if let Some(t) = &track {
                        app.lib.toggle_like(&t.id);
                    }
                }
                let w = (ui.available_width() - 4.0).min(150.0);
                if !super::preview::keep_bar(app, ui, (ui.available_width() - 4.0).min(300.0), 26.0) {
                    shazam_wide(app, ui, w);
                }
            });
        }
    });
}

pub fn show_mini(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let st = app.player.status();
    let track = app.current_track();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
        cover(app, ui, r, track.as_ref(), 128);
        let (lr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
        lcd(app, ui, lr, &st, track.as_ref(), true);
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let w = ui.available_width() - 3.0 * 38.0 - 12.0;
        let (wr, resp) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::click_and_drag());
        waveform(app, ui, wr, &resp, &st);
        if tbtn(ui, &pal, "⏮", Vec2::new(34.0, 32.0), false, false).clicked() { app.player.prev(); }
        if tbtn(ui, &pal, if st.playing { "⏸" } else { "▶" }, Vec2::new(38.0, 34.0), false, true).clicked() { app.player.toggle(); }
        if tbtn(ui, &pal, "⏭", Vec2::new(34.0, 32.0), false, false).clicked() { app.player.next(true); }
    });
}

fn cover(app: &mut App, ui: &mut Ui, r: Rect, t: Option<&crate::store::Track>, size: u32) {
    let pal = app.pal;
    fill(ui.painter(), r, pal.bg);
    let tex = t.and_then(|t| super::preview::cover_tex(app, ui.ctx(), t, size));
    match tex {
        Some(id) => {
            ui.painter().image(id, r.shrink(2.0), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            // pixel-art cassette placeholder
            let cell = (r.width() * 0.7 / 32.0).floor().max(1.0);
            let origin = r.center() - Vec2::new(16.0 * cell, 6.5 * cell);
            for (y, row) in CASSETTE.iter().enumerate() {
                for (x, ch) in row.chars().enumerate() {
                    if ch == '#' {
                        fill(ui.painter(), Rect::from_min_size(origin + Vec2::new(x as f32 * cell, y as f32 * cell), Vec2::splat(cell)), pal.faint);
                    }
                }
            }
        }
    }
    frame_rect(ui.painter(), r, 2.0, pal.line_hi);
    // right-click the cover: replace it (or the lyrics) for a song of yours
    if let Some(t) = t.filter(|t| app.lib.track(&t.id).is_some()) {
        let resp = ui.interact(r, ui.id().with(("deck-cover", r.min.x as i32, r.min.y as i32)), Sense::click());
        resp.context_menu(|ui| super::browser::replace_items(app, ui, Some(t)));
    }
}

fn codec_label(t: &crate::store::Track) -> String {
    let c = t.codec.to_uppercase();
    if c.contains("LAYER 3") || c == "MPEG" { "MP3".into() } else { c.rsplit('/').next().unwrap_or("").split(' ').next().unwrap_or("").to_string() }
}

fn lcd(app: &App, ui: &mut Ui, r: Rect, st: &crate::audio::Status, t: Option<&crate::store::Track>, mini: bool) {
    let pal = app.pal;
    let p = ui.painter_at(r);
    fill(&p, r, pal.lcd_bg);
    frame_rect(&p, r, 2.0, pal.line_hi);
    let x = r.left() + 8.0;
    let glow = app.settings.lock().glow;
    let text = |pos: Pos2, s: &str, f: egui::FontId, c: Color32| {
        if glow {
            p.text(pos + Vec2::new(0.0, 0.0), Align2::LEFT_TOP, s, f.clone(), with_alpha(c, 50));
        }
        p.text(pos, Align2::LEFT_TOP, s, f, c);
    };
    if !mini {
        let state = if st.playing { "> PLAY" } else if t.is_some() { "|| PAUSE" } else { "STOP" };
        text(Pos2::new(x, r.top() + 6.0), state, px(6.0), pal.lcd);
        if let Some(t) = t {
            let sw = p.layout_no_wrap(state.to_string(), px(6.0), pal.lcd).size().x;
            let full = [t.bitrate.map(|b| format!("{b} KBPS")), t.sample_rate.map(|s| format!("{:.1} KHZ", s as f32 / 1000.0)), Some(codec_label(t))].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            let short = [t.bitrate.map(|b| format!("{b}K")), Some(codec_label(t))].into_iter().flatten().collect::<Vec<_>>().join(" ");
            let fits = |s: &str| p.layout_no_wrap(s.to_string(), px(6.0), pal.lcd).size().x + sw + 24.0 < r.width();
            let tech = if fits(&full) { full } else if fits(&short) { short } else { String::new() };
            p.text(Pos2::new(r.right() - 8.0, r.top() + 6.0), Align2::RIGHT_TOP, tech, px(6.0), with_alpha(pal.lcd, 200));
        }
    }
    let ty = if mini { r.top() + 4.0 } else { r.top() + 20.0 };
    let big = if mini { 26.0 } else { 46.0 };
    let el = fmt_time(st.position);
    let g = p.layout_no_wrap(el.clone(), vt(big), pal.lcd);
    let w = g.size().x;
    text(Pos2::new(x, ty), &el, vt(big), pal.lcd);
    p.text(Pos2::new(x + w + 6.0, ty + big * 0.42), Align2::LEFT_TOP, format!("/ {}", fmt_time(if st.duration > 0.0 { st.duration } else { t.map(|t| t.duration).unwrap_or(0.0) })), vt(big * 0.55), with_alpha(pal.lcd, 150));
    // title: LCD-style marquee stepping one character at a time
    let title = t.map(|t| t.title.clone()).unwrap_or_else(|| "NO TRACK LOADED".into());
    let tf = vt(if mini { 20.0 } else { 25.0 });
    let tw = p.layout_no_wrap(title.clone(), tf.clone(), pal.text).size().x;
    let avail = r.width() - 16.0;
    let shown = if tw > avail && st.playing {
        let lp = format!("{title}   ·   ");
        let chars: Vec<char> = lp.chars().collect();
        let step = ((ui.ctx().input(|i| i.time) * 4.0) as usize) % chars.len();
        chars[step..].iter().chain(chars[..step].iter()).collect::<String>()
    } else {
        title
    };
    let ty2 = if mini { r.top() + 30.0 } else { r.top() + 68.0 };
    p.text(Pos2::new(x, ty2), Align2::LEFT_TOP, shown, tf, pal.text);
    if mini && r.height() >= 76.0 {
        p.text(Pos2::new(x, r.top() + 54.0), Align2::LEFT_TOP, t.map(|t| t.artist.clone()).unwrap_or_default(), vt(18.0), pal.dim);
    }
    if !mini {
        p.text(Pos2::new(x, r.top() + 94.0), Align2::LEFT_TOP, t.map(|t| t.artist.clone()).unwrap_or_else(|| "INSERT A TAPE".into()), vt(19.0), pal.text);
        if let Some(t) = t {
            let al = [t.album.clone(), t.year.map(|y| y.to_string()).unwrap_or_default()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            p.text(Pos2::new(x, r.top() + 110.0), Align2::LEFT_TOP, al, vt(16.0), with_alpha(pal.text, 140));
        }
    }
}

pub(super) fn waveform(app: &mut App, ui: &mut Ui, r: Rect, resp: &egui::Response, st: &crate::audio::Status) {
    let pal = app.pal;
    let p = ui.painter_at(r);
    fill(&p, r, pal.bg);
    frame_rect(&p, r, 2.0, pal.line_hi);
    let inner = r.shrink(3.0);
    let dur = if st.duration > 0.0 { st.duration } else { 0.0 };
    let prog = if dur > 0.0 { (st.position / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let hover = resp.hover_pos().map(|h| ((h.x - inner.left()) / inner.width()).clamp(0.0, 1.0));
    let peaks = st.current.as_ref().and_then(|id| app.player.analyzer.get(id));
    let mid = inner.center().y;
    let bars = (inner.width() / 3.0) as usize;
    for i in 0..bars {
        let f = i as f32 / bars as f32;
        let amp = match &peaks {
            Some(pk) => {
                let a = (f * pk.len() as f32) as usize;
                let b = (((i + 1) as f32 / bars as f32) * pk.len() as f32) as usize;
                let m = pk[a.min(pk.len() - 1)..b.max(a + 1).min(pk.len())].iter().copied().max().unwrap_or(0) as f32 / 255.0;
                (m.powf(1.6) * (inner.height() / 2.0 - 1.0)).max(1.0)
            }
            None => 1.0,
        };
        let c = if f <= prog { pal.accent } else if hover.map(|h| f <= h).unwrap_or(false) { with_alpha(pal.accent2, 150) } else { pal.line_hi };
        let x = inner.left() + i as f32 * 3.0;
        fill(&p, Rect::from_min_max(Pos2::new(x, mid - amp), Pos2::new(x + 2.0, mid + amp)), c);
    }
    let px_ = inner.left() + prog * inner.width();
    p.vline(px_, inner.y_range(), Stroke::new(1.0_f32, pal.text));
    if let Some(h) = hover {
        let tip = fmt_time(h as f64 * dur);
        let pos = Pos2::new(inner.left() + h * inner.width(), r.top() - 2.0);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("seek-tip")));
        let g = painter.layout_no_wrap(tip, vt(18.0), pal.ink);
        let br = Rect::from_center_size(pos - Vec2::new(0.0, g.size().y / 2.0 + 2.0), g.size() + Vec2::new(8.0, 2.0));
        fill(&painter, br, pal.accent);
        painter.galley(br.min + Vec2::new(4.0, 1.0), g, pal.ink);
    }
    if (resp.clicked() || resp.dragged()) && dur > 0.0 {
        if let Some(h) = resp.interact_pointer_pos() {
            let f = ((h.x - inner.left()) / inner.width()).clamp(0.0, 1.0);
            app.player.seek(f as f64 * dur);
        }
    }
    if peaks.is_none() && st.current.is_some() {
        if let Some(id) = &st.current {
            app.player.analyzer.request(id);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(400));
    }
}

/// Transport button (big play button = filled accent).
pub fn tbtn(ui: &mut Ui, pal: &super::theme::Pal, label: &str, size: Vec2, on: bool, primary: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(size + Vec2::splat(3.0), Sense::click());
    let pressed = resp.is_pointer_button_down_on();
    let body = Rect::from_min_size(r.min + if pressed { Vec2::splat(3.0) } else { Vec2::ZERO }, size);
    let p = ui.painter();
    if !pressed {
        fill(p, body.translate(Vec2::splat(3.0)), pal.shadow);
    }
    let (bg, fg, br) = if primary {
        (pal.accent, pal.ink, pal.accent)
    } else if on {
        (pal.accent2, pal.ink, pal.accent2)
    } else {
        (pal.panel_hi, if resp.hovered() { pal.accent } else { pal.text }, if resp.hovered() { pal.accent } else { pal.line_hi })
    };
    fill(p, body, bg);
    frame_rect(p, body, 2.0, br);
    let font = if label.chars().all(|c| c.is_ascii_alphanumeric()) { px(if size.y < 24.0 { 6.0 } else { 7.0 }) } else { vt((if primary { 30.0_f32 } else { 24.0 }).min(size.y * 0.75)) };
    p.text(body.center(), Align2::CENTER_CENTER, label, font, fg);
    resp
}

fn icon(ui: &mut Ui, pal: &super::theme::Pal, label: &str, on: bool) -> egui::Response {
    let font = if label.chars().all(|c| c.is_ascii_alphanumeric()) { px(7.0) } else { vt(20.0) };
    let g = ui.painter().layout_no_wrap(label.to_string(), font.clone(), pal.text);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(g.size().x.max(16.0) + 10.0, 26.0), Sense::click());
    if resp.hovered() {
        frame_rect(ui.painter(), r, 2.0, pal.line_hi);
    }
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, font, if on { pal.accent } else if resp.hovered() { pal.text } else { pal.dim });
    resp
}

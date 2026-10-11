//! Lyrics panel: synced lyrics (click a line to jump there) or plain text. Found by lyricsrc.rs
//! (saved lyrics, the file's tags, LRCLIB, then YouTube captions). The text grows and shrinks with
//! the panel; Ctrl+scroll over it makes it bigger or smaller on top of that.
use super::theme::{px, vt};
use super::App;
pub use crate::lyricsrc::Lyr;
use crate::lyricsrc::Source;
use eframe::egui::{self, Pos2, Ui};
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
    /// translations: (song, language) -> None while it's being made
    tr: Arc<Mutex<HashMap<(String, String), Option<Result<crate::lang::Translation, String>>>>>,
    /// sing-along word times per song
    words: Arc<Mutex<HashMap<String, Words>>>,
    /// your timing nudges (seconds per song, more = later), loaded when first needed
    offsets: Option<HashMap<String, f64>>,
    /// when the timing bar was last used (it stays up a moment after)
    nudged: Option<std::time::Instant>,
}

#[derive(Clone)]
enum Words {
    Loading,
    Estimated,
    Exact(Arc<crate::wordsync::Times>),
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
        let mut t = self.tr.lock();
        if t.len() > 8 {
            t.retain(|_, v| v.is_none());
        }
        let mut w = self.words.lock();
        if w.len() > 8 {
            w.retain(|_, v| matches!(v, Words::Loading));
        }
    }

    /// Drop what's known about a song's lyrics (it got new audio): looked up again when shown.
    pub fn forget(&mut self, id: &str) {
        self.cache.lock().remove(id);
        self.words.lock().remove(id);
        let o = self.offsets.get_or_insert_with(crate::wordsync::offsets);
        if o.remove(id).is_some() {
            crate::wordsync::save_offsets(o);
        }
    }

    /// Your timing nudge for a song (seconds; more = the lyrics come later).
    pub fn offset(&mut self, id: &str) -> f64 {
        self.offsets.get_or_insert_with(crate::wordsync::offsets).get(id).copied().unwrap_or(0.0)
    }

    /// Nudge a song's lyrics by `by` seconds (saved).
    pub fn nudge(&mut self, id: &str, by: f64) {
        let o = self.offsets.get_or_insert_with(crate::wordsync::offsets);
        let v = ((o.get(id).copied().unwrap_or(0.0) + by) * 10.0).round() / 10.0;
        if v.abs() < 0.05 {
            o.remove(id);
        } else {
            o.insert(id.to_string(), v.clamp(-30.0, 30.0));
        }
        crate::wordsync::save_offsets(o);
        self.nudged = Some(std::time::Instant::now());
    }

    /// Word times for sing-along: exact ones once they're found (looked up in the background).
    fn words_for(&self, t: &crate::store::Track, lines: &[(f64, String)], ctx: &egui::Context) -> Option<Arc<crate::wordsync::Times>> {
        let have = self.words.lock().get(&t.id).cloned();
        match have {
            Some(Words::Exact(w)) => Some(w),
            Some(_) => None,
            None => {
                self.words.lock().insert(t.id.clone(), Words::Loading);
                let (slot, ctx, t, lines) = (self.words.clone(), ctx.clone(), t.clone(), lines.to_vec());
                std::thread::Builder::new()
                    .name("word-times".into())
                    .spawn(move || {
                        let w = crate::wordsync::exact(&t, &lines).map(|w| Words::Exact(Arc::new(w))).unwrap_or(Words::Estimated);
                        slot.lock().insert(t.id.clone(), w);
                        ctx.request_repaint();
                    })
                    .ok();
                None
            }
        }
    }
}

pub fn show(app: &mut App, ui: &mut Ui) {
    show_sized(app, ui, 1.0, false);
}

/// Sing-along: the colours of words already sung and words still to come. An accent lights the
/// sung ones if it differs from the text and stands out from the background more than the other
/// lines do (a black-and-white theme's grey accent doesn't); otherwise sung words are the bright
/// text and the ones to come are half as bright.
fn sung_colors(pal: &super::theme::Pal) -> (egui::Color32, egui::Color32) {
    let diff = |a: egui::Color32, b: egui::Color32| (a.r() as i32 - b.r() as i32).abs() + (a.g() as i32 - b.g() as i32).abs() + (a.b() as i32 - b.b() as i32).abs();
    let lum = |c: egui::Color32| 0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32;
    let pops = |c: egui::Color32| (lum(c) - lum(pal.bg)).abs() > (lum(pal.dim) - lum(pal.bg)).abs() * 1.15;
    match [pal.accent, pal.accent2].into_iter().find(|c| diff(*c, pal.text) >= 120 && pops(*c)) {
        Some(c) => (c, pal.text),
        None => {
            // (half way to the bright text: dimmer than what's sung, brighter than the other lines)
            let mid = |a: u8, b: u8| ((a as u16 + b as u16) / 2) as u8;
            (pal.text, egui::Color32::from_rgb(mid(pal.dim.r(), pal.text.r()), mid(pal.dim.g(), pal.text.g()), mid(pal.dim.b(), pal.text.b())))
        }
    }
}

/// Lyrics at `scale` times the size that fits the panel (THEATER uses bigger ones). `sing`:
/// sing-along (each word lights up as it's sung, dots count in the singing after a long break).
pub fn show_sized(app: &mut App, ui: &mut Ui, scale: f32, sing: bool) {
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
    // in steps of a tenth: every text size gets its letters drawn into egui's font texture once,
    // so a size that slid with the panel's width kept adding to it while you resized
    let scale = (scale * fit * app.settings.lock().lyrics_zoom * 10.0).round() / 10.0;
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
    // lyrics in another language: translated into yours (once per song, in the background)
    let (mode, to) = { let s = app.settings.lock(); (s.translate.clone(), s.language()) };
    let tr: Option<crate::lang::Translation> = match (&lyr, mode.as_str()) {
        (_, "off") => None,
        (Lyr::Synced(_) | Lyr::Plain(_), _) => {
            let key = (t.id.clone(), to.clone());
            let have = app.lyrics.tr.lock().get(&key).cloned();
            match have {
                Some(Some(Ok(tr))) if !tr.is_empty() => Some(tr),
                Some(_) => None,
                None => {
                    app.lyrics.tr.lock().insert(key.clone(), None);
                    let lines: Vec<String> = match &lyr {
                        Lyr::Synced(v) => v.iter().map(|l| l.1.clone()).collect(),
                        Lyr::Plain(s) => s.lines().map(String::from).collect(),
                        _ => Vec::new(),
                    };
                    let (slot, ctx) = (app.lyrics.tr.clone(), ui.ctx().clone());
                    std::thread::Builder::new()
                        .name("translate".into())
                        .spawn(move || {
                            let r = crate::lang::translate(&key.0, &lines, &key.1);
                            if let Ok(t) = &r {
                                super::cjk::ensure(&ctx, &t.lines.iter().flatten().cloned().collect::<String>());
                            }
                            slot.lock().insert(key, Some(r));
                            ctx.request_repaint();
                        })
                        .ok();
                    None
                }
            }
        }
        _ => None,
    };
    let only = mode == "only";
    let mut cycle = false;
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
                    match &tr {
                        Some(tr) => {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (i, line) in s.lines().enumerate() {
                                let under = tr.lines.get(i).cloned().flatten();
                                match (&under, only) {
                                    (Some(u), true) => {
                                        ui.label(egui::RichText::new(u).font(vt(19.0 * scale)).color(pal.text));
                                    }
                                    _ => {
                                        ui.label(egui::RichText::new(line).font(vt(19.0 * scale)).color(pal.text));
                                        if let Some(u) = &under {
                                            ui.label(egui::RichText::new(u).font(vt(16.0 * scale)).color(pal.accent2));
                                        }
                                    }
                                }
                            }
                        }
                        None => {
                            ui.label(egui::RichText::new(s).font(vt(19.0 * scale)).color(pal.text));
                        }
                    }
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(source.label()).font(px(5.0)).color(pal.faint));
                    cycle = tr_footer(ui, &pal, tr.as_ref(), only);
                });
            });
        }
        Lyr::Synced(lines) => {
            // a line stays lit until the next one starts, then the highlight moves straight to it
            // (no fading, no greyed-out line left behind); LEAD seconds before that, the text
            // glides (eased) so the next line arrives centred exactly when it begins. Every line
            // is the same size: nothing jumps or re-wraps.
            let status = app.player.status();
            // (your nudge: lyrics that come later are earlier in the song's time)
            let offset = app.lyrics.offset(&t.id);
            let pos = status.position - offset;
            let exact = if sing { app.lyrics.words_for(&t, &lines, ui.ctx()) } else { None };
            let times_of = |i: usize| -> Vec<f64> {
                let (lt, text) = &lines[i];
                let gap = lines.get(i + 1).map(|l| l.0 - lt).unwrap_or(8.0);
                crate::wordsync::line_times(exact.as_ref().and_then(|e| e.get(i)), text, *lt, gap)
            };
            let active = lines.iter().rposition(|(t, _)| *t <= pos);
            let next_at = lines.get(active.map(|a| a + 1).unwrap_or(0)).map(|l| l.0);
            let until = next_at.map(|n| n - pos).unwrap_or(f64::MAX);
            let target = if until <= LEAD { Some(active.map(|a| a + 1).unwrap_or(0)) } else { active };
            // sing-along: the next line with words, and whether it comes after a long break
            // (dots count it in); a line's singing ends when its last word does
            let lead_in = if sing {
                let from = active.map(|a| a + 1).unwrap_or(0);
                lines.iter().enumerate().skip(from).find(|(_, l)| !l.1.trim().is_empty()).and_then(|(n, (at, _))| {
                    let sung_before = (0..n).rev().find(|&k| !lines[k].1.trim().is_empty());
                    let quiet_from = sung_before.map(|k| *times_of(k).last().unwrap()).unwrap_or(0.0);
                    (at - quiet_from >= 6.0 || sung_before.is_none() && *at >= 3.0).then_some((n, at - pos))
                })
            } else {
                None
            };
            let mut lead_rect: Option<egui::Rect> = None;
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
                    // 30 frames a second is smooth for this half-second glide (the screen's
                    // full rate, 60-144, only cost CPU)
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
                }
            }
            // wake up in time for the next glide, and to light the next line the moment it starts
            // (none after the last line: `until` is f64::MAX there, far too long for a timer; none
            // while paused: the song isn't moving, so it would only wake up again and again)
            if next_at.is_some() && status.playing {
                let wait = if until > LEAD { until - LEAD } else { until };
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait.clamp(0.01, 3600.0)));
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
                        let under = tr.as_ref().and_then(|t| t.lines.get(i).cloned().flatten());
                        let txt = match (&under, only) {
                            (Some(u), true) => u.as_str(),
                            _ if text.is_empty() => "♪",
                            _ => text.as_str(),
                        };
                        let label = if sing && Some(i) == active && !text.is_empty() {
                            // sing-along: each word lights up the moment it's sung (a translation
                            // shown alone shares the line's time out among its own words)
                            let mine = times_of(i);
                            let times = if txt == text.as_str() { mine } else { crate::wordsync::spread(txt, mine[0], *mine.last().unwrap()) };
                            let cut = crate::wordsync::sung_upto(txt, &times, pos);
                            // wake up for the next word, not before
                            if let (Some(next), true) = (times.iter().find(|w| **w > pos), status.playing) {
                                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((next - pos).clamp(0.01, 30.0)));
                            }
                            let mut job = egui::text::LayoutJob::default();
                            let f = |c: egui::Color32| egui::TextFormat { font_id: vt(size), color: c, ..Default::default() };
                            let (sung, to_come) = sung_colors(&pal);
                            job.append(&txt[..cut], 0.0, f(sung));
                            job.append(&txt[cut..], 0.0, f(to_come));
                            job.halign = egui::Align::Center;
                            egui::WidgetText::LayoutJob(job)
                        } else {
                            egui::RichText::new(txt).font(vt(size)).color(color).into()
                        };
                        let mut r = ui.add(egui::Label::new(label).wrap().sense(egui::Sense::click()));
                        // its translation right under it, smaller (lit with it)
                        if let (Some(u), false) = (&under, only) {
                            let gap = ui.spacing().item_spacing.y;
                            ui.add_space(-gap + 1.0);
                            let c = if Some(i) == active { pal.accent2 } else { pal.faint };
                            r = r.union(ui.add(egui::Label::new(egui::RichText::new(u).font(vt(size * 0.78)).color(c)).wrap().sense(egui::Sense::click())));
                        }
                        ys.push(r.rect.center().y - top);
                        if lead_in.map(|l| l.0) == Some(i) {
                            lead_rect = Some(r.rect);
                        }
                        if r.clicked() {
                            app.player.seek((*lt + offset).max(0.0));
                        }
                        if r.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new(source.label()).font(px(5.0)).color(pal.faint));
                    cycle = tr_footer(ui, &pal, tr.as_ref(), only);
                });
                ui.add_space(view_h * 0.5);
            });
            // sing-along score: grade the lines that ended, and show how it's going
            if sing {
                super::nowplaying::score_lines(app, &t.id, &lines, pos, &|i| *times_of(i).last().unwrap());
                super::nowplaying::score_overlay(app, ui, out.inner_rect, true);
            }
            // sing-along: after a long break, three dots over the next line go out one a second
            if let (Some((_, left)), true) = (lead_in, status.playing) {
                if left > 0.0 && left <= 3.0 {
                    if let Some(r) = lead_rect.filter(|r| out.inner_rect.intersects(*r)) {
                        let p = ui.painter().with_clip_rect(out.inner_rect);
                        let (rad, step) = (size * 0.16, size * 0.7);
                        let y = r.top() - size * 0.45;
                        for k in 0..3 {
                            let lit = (left.ceil() as i32) > k;
                            let c = Pos2::new(r.center().x + (k as f32 - 1.0) * step, y);
                            if lit {
                                p.circle_filled(c, rad, sung_colors(&pal).0);
                            } else {
                                p.circle_stroke(c, rad, egui::Stroke::new(1.5_f32, pal.faint));
                            }
                        }
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((left - left.floor()).max(0.02)));
                } else if left > 3.0 && left < 60.0 {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left - 3.0));
                }
            }
            if sing {
                timing_bar(app, ui, out.inner_rect, &t.id, offset, exact.is_some());
            }
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
    if cycle {
        app.edit_settings(|s| s.translate = if s.translate == "only" { "under".into() } else { "only".into() });
    }
    if retry {
        crate::lyricsrc::forget(&t.id);
        crate::wordsync::forget(&t.id);
        app.lyrics.cache.lock().remove(&t.id);
        app.lyrics.words.lock().remove(&t.id);
    }
}

/// Sing-along's bar (bottom of the lyrics, while the mouse is over them or just after a nudge):
/// VOCALS on / off (karaoke: the singer taken down), lyrics EARLIER / LATER for this song ([ and ]
/// too) and where the word times came from.
fn timing_bar(app: &mut App, ui: &mut Ui, area: egui::Rect, id: &str, offset: f64, exact: bool) {
    let pal = app.pal;
    let mut by = 0.0;
    if ui.input(|i| i.key_pressed(egui::Key::OpenBracket) && i.modifiers.is_none()) {
        by = -0.1;
    }
    if ui.input(|i| i.key_pressed(egui::Key::CloseBracket) && i.modifiers.is_none()) {
        by = 0.1;
    }
    let recent = app.lyrics.nudged.is_some_and(|t| t.elapsed().as_secs_f32() < 2.5);
    if !ui.rect_contains_pointer(area) && !recent && by == 0.0 {
        return;
    }
    let bar = egui::Rect::from_center_size(egui::pos2(area.center().x, area.bottom() - 22.0), egui::vec2(500.0_f32.min(area.width() - 8.0), 30.0));
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("lyric-timing"))).with_clip_rect(area);
    p.rect_filled(bar, 0.0, super::widgets::with_alpha(pal.panel, 235));
    p.rect_stroke(bar, 0.0, egui::Stroke::new(1.5_f32, pal.line), egui::StrokeKind::Inside);
    let btn = |ui: &mut Ui, r: egui::Rect, label: &str, salt: &str, tip: &str| -> bool {
        let resp = ui.interact(r, ui.id().with(("lyric-timing", salt)), egui::Sense::click());
        let c = if resp.hovered() { pal.accent } else { pal.text };
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        p.text(r.center(), egui::Align2::CENTER_CENTER, label, px(6.0), c);
        resp.on_hover_text(tip).clicked()
    };
    let w = 92.0;
    let cut = app.player.vocals_cut();
    let vr = egui::Rect::from_min_size(bar.min, egui::vec2(110.0, bar.height()));
    if cut {
        p.rect_filled(vr.shrink(3.0), 0.0, super::widgets::with_alpha(pal.accent, 60));
    }
    if btn(ui, vr, if cut { "VOCALS: OFF" } else { "VOCALS: ON" }, "v", "Karaoke: take the singer's voice down (works on most studio songs: it takes out what's in the middle of the mix). The voice is back when THEATER closes") {
        app.player.set_vocals_cut(!cut);
        app.lyrics.nudged = Some(std::time::Instant::now());
    }
    p.vline(vr.right(), bar.y_range().shrink(5.0), egui::Stroke::new(1.0_f32, pal.line));
    let bar = egui::Rect::from_min_max(egui::pos2(vr.right(), bar.top()), bar.max);
    if btn(ui, egui::Rect::from_min_size(bar.min, egui::vec2(w, bar.height())), "◀ EARLIER", "e", "The words come 0.1 s sooner ([)") {
        by = -0.1;
    }
    if btn(ui, egui::Rect::from_min_size(egui::pos2(bar.right() - w, bar.top()), egui::vec2(w, bar.height())), "LATER ▶", "l", "The words come 0.1 s later (])") {
        by = 0.1;
    }
    let mid = if offset.abs() < 0.05 { "IN TIME".to_string() } else { format!("{:+.1} S", offset) };
    p.text(bar.center() - egui::vec2(0.0, 6.0), egui::Align2::CENTER_CENTER, mid, px(6.0), pal.accent2);
    p.text(bar.center() + egui::vec2(0.0, 7.0), egui::Align2::CENTER_CENTER, if exact { "exact word timing" } else { "estimated word timing" }, vt(14.0), pal.dim);
    if by != 0.0 {
        app.lyrics.nudge(id, by);
    }
    if recent {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
}

/// "TRANSLATED FROM SPANISH · TRANSLATION ONLY / SHOW THE ORIGINAL TOO" under translated lyrics.
/// True when clicked (switches between the two).
fn tr_footer(ui: &mut Ui, pal: &super::theme::Pal, tr: Option<&crate::lang::Translation>, only: bool) -> bool {
    let Some(tr) = tr else { return false };
    ui.add_space(4.0);
    let label = format!("TRANSLATED FROM {} · {}", crate::lang::short_name(&tr.from), if only { "SHOW THE ORIGINAL TOO" } else { "TRANSLATION ONLY" });
    let r = ui.add(egui::Label::new(egui::RichText::new(label).font(px(5.0)).color(pal.accent2)).sense(egui::Sense::click()));
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r.on_hover_text("Lyrics in another language are translated into yours (Settings > Language)").clicked()
}

fn center(ui: &mut Ui, s: &str, pal: &super::theme::Pal) {
    ui.centered_and_justified(|ui| ui.label(egui::RichText::new(s).font(px(8.0)).color(pal.dim)));
}

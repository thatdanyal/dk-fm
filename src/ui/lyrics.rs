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
    }
}

pub fn show(app: &mut App, ui: &mut Ui) {
    show_sized(app, ui, 1.0, false);
}

/// About how long a line is sung for: a little per letter, never past the next line.
pub(super) fn sung_for(text: &str, gap: f64) -> f64 {
    let letters = text.chars().filter(|c| c.is_alphanumeric()).count() as f64;
    (letters * 0.085 + 0.45).min(gap - 0.2).max(0.5)
}

/// Sing-along: the colour of words already sung, the accent that stands out most from the text
/// (in a black-and-white theme the main accent can be close to the text colour).
fn sung_color(pal: &super::theme::Pal) -> egui::Color32 {
    let d = |a: egui::Color32| {
        let b = pal.text;
        (a.r() as i32 - b.r() as i32).abs() + (a.g() as i32 - b.g() as i32).abs() + (a.b() as i32 - b.b() as i32).abs()
    };
    if d(pal.accent) >= 120 || d(pal.accent) >= d(pal.accent2) { pal.accent } else { pal.accent2 }
}

/// Sing-along: how far into `text` the singer is (`t` seconds into a line sung for `dur`), as a
/// byte position. Each word gets time by its length; the word being sung fills letter by letter.
fn sung_upto(text: &str, t: f64, dur: f64) -> usize {
    if t <= 0.0 {
        return 0;
    }
    if t >= dur {
        return text.len();
    }
    let words: Vec<(usize, usize)> = {
        let mut v = Vec::new();
        let mut start = None;
        for (i, c) in text.char_indices() {
            match (c.is_whitespace(), start) {
                (false, None) => start = Some(i),
                (true, Some(s)) => {
                    v.push((s, i));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            v.push((s, text.len()));
        }
        v
    };
    let weight = |w: &(usize, usize)| text[w.0..w.1].chars().count() as f64 + 1.5;
    let total: f64 = words.iter().map(weight).sum();
    let mut at = t / dur * total;
    for w in &words {
        let k = weight(w);
        if at < k {
            let chars: Vec<usize> = text[w.0..w.1].char_indices().map(|(i, _)| w.0 + i).chain([w.1]).collect();
            let n = ((at / k) * (chars.len() - 1) as f64).round() as usize;
            return chars[n.min(chars.len() - 1)];
        }
        at -= k;
    }
    text.len()
}

/// Lyrics at `scale` times the size that fits the panel (THEATER uses bigger ones). `sing`:
/// sing-along (the line lights up word by word, a 3-2-1 countdown before the singing starts).
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
            let pos = status.position;
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
                            // sing-along: what's been sung is lit, the rest waits in plain text
                            let gap = lines.get(i + 1).map(|l| l.0 - lt).unwrap_or(8.0);
                            let dur = sung_for(txt, gap);
                            let cut = sung_upto(txt, pos - lt, dur);
                            if cut < txt.len() && status.playing {
                                ui.ctx().request_repaint_after(std::time::Duration::from_millis(45));
                            }
                            let mut job = egui::text::LayoutJob::default();
                            let f = |c: egui::Color32| egui::TextFormat { font_id: vt(size), color: c, ..Default::default() };
                            job.append(&txt[..cut], 0.0, f(sung_color(&pal)));
                            job.append(&txt[cut..], 0.0, f(pal.text));
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
                        if r.clicked() {
                            app.player.seek(*lt);
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
                super::nowplaying::score_lines(app, &t.id, &lines, pos);
                super::nowplaying::score_overlay(app, ui, out.inner_rect, true);
            }
            // sing-along: 3, 2, 1 before the singing starts (and after a long break)
            if sing && status.playing {
                let next = lines.iter().enumerate().skip(active.map(|a| a + 1).unwrap_or(0)).find(|(_, l)| !l.1.trim().is_empty());
                if let Some((n, (at, _))) = next {
                    let prev = active.map(|a| lines[a].0);
                    let quiet = match active {
                        None => true,
                        Some(a) => lines[a].1.trim().is_empty() || at - prev.unwrap_or(0.0) >= 8.0 && pos - prev.unwrap_or(0.0) > sung_for(&lines[a].1, at - prev.unwrap_or(0.0)) + 1.0,
                    };
                    let left = at - pos;
                    if quiet && n < lines.len() && left > 0.0 && left <= 3.0 {
                        let area = out.inner_rect;
                        let bh = (size * 3.4).min(area.height() * 0.45);
                        let k = bh / (size * 3.4);
                        let c = Pos2::new(area.center().x, area.top() + 10.0 + bh / 2.0);
                        let p = ui.painter();
                        let r = egui::Rect::from_center_size(c, egui::vec2(size * 4.2 * k, bh));
                        let size = size * k;
                        p.rect_filled(r, 0.0, super::widgets::with_alpha(pal.bg2, 235));
                        p.rect_stroke(r, 0.0, egui::Stroke::new(2.0_f32, pal.accent), egui::StrokeKind::Inside);
                        p.text(c - egui::vec2(0.0, size * 0.35), egui::Align2::CENTER_CENTER, format!("{}", left.ceil() as u32), vt(size * 2.6), pal.accent);
                        p.text(c + egui::vec2(0.0, size * 1.15), egui::Align2::CENTER_CENTER, "GET READY", px((size * 0.3).max(6.0)), pal.text);
                        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((left - left.floor()).max(0.02)));
                    } else if quiet && left > 3.0 && left < 30.0 {
                        // wake up for the countdown
                        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left - 3.0));
                    }
                }
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
        app.lyrics.cache.lock().remove(&t.id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sing_along_fills_word_by_word() {
        let line = "Never gonna give you up";
        let dur = sung_for(line, 6.0);
        assert!(dur > 1.0 && dur < 3.0, "{dur}");
        // not started, done, and always on a character boundary in between, moving forward
        assert_eq!(sung_upto(line, -0.1, dur), 0);
        assert_eq!(sung_upto(line, dur + 0.1, dur), line.len());
        let mut last = 0;
        for k in 0..=40 {
            let at = sung_upto(line, dur * k as f64 / 40.0, dur);
            assert!(at >= last && line.is_char_boundary(at));
            last = at;
        }
        // half way through the time: about half way through the words
        let mid = sung_upto(line, dur / 2.0, dur);
        assert!((8..=16).contains(&mid), "{mid}");
        // never runs past the next line
        assert!(sung_for(line, 1.0) <= 0.8 + 1e-9);
        // other alphabets
        let jp = "君の名は 希望";
        let at = sung_upto(jp, 0.5, 1.0);
        assert!(jp.is_char_boundary(at) && at > 0 && at < jp.len());
    }
}

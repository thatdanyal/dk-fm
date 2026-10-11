//! REPLACE LYRICS… (a song's right-click menu, or under its lyrics): for a song with the wrong
//! lyrics, or none. FIND searches LRCLIB (any words: title, artist, a line you remember); click
//! one to read it, then USE THESE. TYPE / PASTE takes your own: lines with times like
//! [01:23.45] light up in time, without times they're plain text. Either way they're the song's
//! lyrics from then on (never looked up again by themselves); word timing and translations
//! are worked out again for the new words.
use super::settings::window;
use super::theme::{px, vt};
use super::widgets::{button, fill, fmt_time, frame_rect, outline_button};
use super::App;
use crate::lyricsrc::{Found, Lyr, Source};
use eframe::egui::{self, Align2, Sense, Ui, Vec2};
use parking_lot::Mutex;
use std::sync::Arc;

type Results = Arc<Mutex<Option<Result<Vec<Found>, String>>>>;

pub struct Fix {
    id: String,
    query: String,
    typing: bool,
    /// TYPE / PASTE's text (starts as the song's lyrics now, to fix a line or two)
    text: String,
    results: Results,
    searched: String,
    picked: Option<usize>,
}

/// Open REPLACE LYRICS for a song (searching right away).
pub fn open(app: &mut App, id: &str, ctx: &egui::Context) {
    let Some(t) = app.lib.track(id) else { return };
    let query = format!("{} {}", crate::library::main_artist(&t.artist), crate::lyricsrc::bare_title(&t.title));
    let text = crate::lyricsrc::saved(id).map(|l| crate::lyricsrc::to_text(&l)).unwrap_or_default();
    let mut f = Fix { id: id.to_string(), query, typing: false, text, results: Arc::new(Mutex::new(None)), searched: String::new(), picked: None };
    search(&mut f, ctx);
    app.new_lyrics = Some(f);
}

fn search(f: &mut Fix, ctx: &egui::Context) {
    let q = f.query.trim().to_string();
    if q.is_empty() || q == f.searched {
        return;
    }
    f.searched = q.clone();
    f.picked = None;
    let slot: Results = Arc::new(Mutex::new(None));
    f.results = slot.clone();
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("lyrics-search".into())
        .spawn(move || {
            let r = crate::lyricsrc::search(&q);
            if let Ok(v) = &r {
                let all: String = v.iter().map(|f| format!("{}{}{}", f.title, f.artist, crate::lyricsrc::to_text(&f.lyr))).collect();
                super::cjk::ensure(&ctx, &all);
            }
            *slot.lock() = Some(r);
            ctx.request_repaint();
        })
        .ok();
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut f) = app.new_lyrics.take() else { return };
    let Some(t) = app.lib.track(&f.id) else { return };
    let pal = app.pal;
    let mut chosen: Option<(Lyr, Source)> = None;
    let mut look_again = false;
    let mut cancel = false;
    let open = window(pal, ctx, "REPLACE LYRICS", Vec2::new(760.0, 500.0), !f.typing, |ui| {
        ui.label(egui::RichText::new(format!("{} - {}", t.artist, t.title)).font(vt(22.0)).color(pal.text));
        ui.horizontal(|ui| {
            if outline_button(ui, &pal, "FIND", !f.typing).on_hover_text("Search LRCLIB's lyrics").clicked() {
                f.typing = false;
            }
            if outline_button(ui, &pal, "TYPE / PASTE", f.typing).on_hover_text("Write or paste the lyrics yourself (or fix a line of the ones it has)").clicked() {
                f.typing = true;
            }
            ui.label(egui::RichText::new(if t.duration > 0.0 { format!("  song length {}", fmt_time(t.duration)) } else { String::new() }).font(vt(16.0)).color(pal.dim));
        });
        ui.add_space(6.0);
        let body_h = ui.available_height() - 40.0;
        if f.typing {
            egui::ScrollArea::vertical().max_height(body_h - 22.0).auto_shrink([false, false]).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut f.text).desired_width(f32::INFINITY).desired_rows(14).font(vt(18.0)).hint_text("Paste or type the lyrics here, one line per line.\nLines that start with a time like [01:23.45] light up in time."));
            });
            ui.horizontal(|ui| {
                let l = crate::lyricsrc::from_text(&f.text);
                let what = match &l {
                    Lyr::Synced(v) => format!("{} lines that light up in time", v.len()),
                    Lyr::Plain(s) => format!("{} lines (no times: shown as plain text)", s.lines().count()),
                    _ => "Nothing typed yet".to_string(),
                };
                ui.label(egui::RichText::new(what).font(vt(15.0)).color(pal.dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ok = !matches!(l, Lyr::None);
                    if button(ui, &pal, "SAVE THESE LYRICS", true, ok).clicked() && ok {
                        chosen = Some((l, Source::Yours));
                    }
                });
            });
        } else {
            let mut go = false;
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut f.query).desired_width(420.0).font(vt(19.0)).hint_text("Song, artist, or a line from it"));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = true;
                }
                if button(ui, &pal, "SEARCH", true, true).clicked() {
                    go = true;
                }
            });
            if go {
                search(&mut f, ui.ctx());
            }
            ui.add_space(6.0);
            let res = f.results.lock().clone();
            let h = ui.available_height() - 40.0;
            match res {
                None => message(ui, h, "SEARCHING…", &pal),
                Some(Err(e)) => message(ui, h, &format!("Couldn't search: {e}"), &pal),
                Some(Ok(list)) if list.is_empty() => message(ui, h, "NOTHING FOUND: TRY OTHER WORDS, OR TYPE / PASTE THEM", &pal),
                Some(Ok(list)) => {
                    // (dev hook: DKFM_VIEW=newlyrics shows the first one for a screenshot)
                    if f.picked.is_none() && std::env::var("DKFM_VIEW").is_ok_and(|v| v == "newlyrics") {
                        f.picked = Some(0);
                    }
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(Vec2::new(390.0, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                            egui::ScrollArea::vertical().id_salt("lyr-found").max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                                for (i, c) in list.iter().enumerate() {
                                    if row(app, ui, &t, c, f.picked == Some(i)) {
                                        f.picked = Some(i);
                                    }
                                }
                            });
                        });
                        ui.add_space(8.0);
                        let w = ui.available_width();
                        ui.allocate_ui_with_layout(Vec2::new(w, h), egui::Layout::top_down(egui::Align::Min), |ui| match f.picked.and_then(|i| list.get(i)) {
                            None => message(ui, h, "CLICK ONE TO READ IT", &pal),
                            Some(c) => {
                                egui::ScrollArea::vertical().id_salt("lyr-read").max_height(h - 40.0).auto_shrink([false, false]).show(ui, |ui| {
                                    ui.set_max_width(w - 14.0);
                                    match &c.lyr {
                                        Lyr::Synced(v) => {
                                            for (at, line) in v {
                                                ui.horizontal(|ui| {
                                                    ui.label(egui::RichText::new(fmt_time(*at)).font(vt(14.0)).color(pal.faint));
                                                    ui.add(egui::Label::new(egui::RichText::new(line).font(vt(17.0)).color(pal.text)).wrap());
                                                });
                                            }
                                        }
                                        Lyr::Plain(s) => {
                                            ui.add(egui::Label::new(egui::RichText::new(s).font(vt(17.0)).color(pal.text)).wrap());
                                        }
                                        _ => {
                                            ui.label(egui::RichText::new("♪ INSTRUMENTAL ♪").font(px(7.0)).color(pal.dim));
                                        }
                                    }
                                });
                                ui.add_space(6.0);
                                if button(ui, &pal, "USE THESE LYRICS", true, true).clicked() {
                                    chosen = Some((c.lyr.clone(), Source::Lrclib));
                                }
                            }
                        });
                    });
                }
            }
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if button(ui, &pal, "NO LYRICS", false, true).on_hover_text("This song has no words (or you don't want them shown)").clicked() {
                chosen = Some((Lyr::Instrumental, Source::Yours));
            }
            if button(ui, &pal, "FIND THEM BY ITSELF AGAIN", false, true).on_hover_text("Forget these lyrics and look them up again the usual way").clicked() {
                look_again = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, &pal, "CANCEL", false, true).clicked() {
                    cancel = true;
                }
            });
        });
    });
    if let Some((l, src)) = chosen {
        crate::lyricsrc::choose(&t.id, &l, src);
        app.lyrics.replaced(&t.id);
        app.toast(format!("New lyrics for {}", t.title));
        return;
    }
    if look_again {
        crate::lyricsrc::forget_all(&t.id);
        app.lyrics.replaced(&t.id);
        app.toast(format!("Looking for {}'s lyrics again", t.title));
        return;
    }
    if open && !cancel {
        app.new_lyrics = Some(f);
    }
}

fn message(ui: &mut Ui, h: f32, s: &str, pal: &super::theme::Pal) {
    ui.allocate_ui(Vec2::new(ui.available_width(), h), |ui| {
        ui.centered_and_justified(|ui| ui.label(egui::RichText::new(s).font(px(6.0)).color(pal.dim)));
    });
}

/// One of LRCLIB's: title, artist · album, length against the song's, SYNCED / PLAIN.
/// True when clicked.
fn row(app: &App, ui: &mut Ui, song: &crate::store::Track, c: &Found, picked: bool) -> bool {
    let pal = app.pal;
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click());
    if picked {
        fill(ui.painter(), r, pal.panel_hi);
        frame_rect(ui.painter(), r.shrink(1.0), 2.0, pal.accent);
    } else if resp.hovered() {
        fill(ui.painter(), r, pal.panel_hi);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let (kind, kc) = match c.lyr {
        Lyr::Synced(_) => ("SYNCED", pal.accent2),
        Lyr::Plain(_) => ("PLAIN", pal.dim),
        _ => ("NO WORDS", pal.dim),
    };
    let right = r.right() - 8.0;
    ui.painter().text(egui::pos2(right, r.top() + 12.0), Align2::RIGHT_CENTER, kind, px(5.0), kc);
    // length: green when it's about the song's (the same recording: synced times will fit)
    if c.duration > 0.0 {
        let off = c.duration - song.duration;
        let col = if song.duration <= 0.0 { pal.dim } else if off.abs() < 2.5 { pal.accent2 } else if off.abs() > 10.0 { pal.accent } else { pal.text };
        ui.painter().text(egui::pos2(right, r.top() + 30.0), Align2::RIGHT_CENTER, fmt_time(c.duration), vt(15.0), col);
    }
    let p = ui.painter().with_clip_rect(egui::Rect::from_min_max(r.min, egui::pos2(right - 70.0, r.bottom())));
    p.text(egui::pos2(r.left() + 8.0, r.top() + 12.0), Align2::LEFT_CENTER, &c.title, vt(18.0), pal.text);
    let by = [c.artist.as_str(), c.album.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    p.text(egui::pos2(r.left() + 8.0, r.top() + 30.0), Align2::LEFT_CENTER, by, vt(15.0), pal.dim);
    resp.clicked()
}

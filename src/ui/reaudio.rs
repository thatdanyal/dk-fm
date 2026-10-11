//! REPLACE AUDIO… (a song's right-click menu): for a song whose title, album and lyrics are right
//! but whose sound is another recording (a different song, a live or sped-up version, a music
//! video with a long intro). It searches YouTube Music for the song; listen to any result first
//! (▶), then USE THIS: the song is downloaded again from that one and its file is replaced.
//! Its title, album, cover, likes, plays and playlists stay; its lyrics are looked up again for
//! the new recording. A pasted YouTube link works too.
use super::settings::window;
use super::theme::{px, vt};
use super::widgets::{button, fill, fmt_time, frame_rect};
use super::App;
use crate::sources::ITrack;
use eframe::egui::{self, Align2, Rect, Sense, Ui, Vec2};
use parking_lot::Mutex;
use std::sync::Arc;

type Results = Arc<Mutex<Option<Result<Vec<ITrack>, String>>>>;

pub struct Fix {
    id: String,
    query: String,
    /// "songs" (YouTube Music) or "youtube" (all videos)
    source: &'static str,
    results: Results,
    /// what `results` was searched for (source, query)
    searched: (String, String),
}

/// A song whose audio is being replaced: (song, job). When it's done its lyrics and loudness are
/// looked up again, and if it's playing it starts over with the new sound.
pub type Pending = (String, String);

/// Open REPLACE AUDIO for a song (searching right away).
pub fn open(app: &mut App, id: &str, ctx: &egui::Context) {
    let Some(t) = app.lib.track(id) else { return };
    let query = format!("{} {}", crate::library::main_artist(&t.artist), crate::lyricsrc::bare_title(&t.title));
    let mut f = Fix { id: id.to_string(), query, source: "songs", results: Arc::new(Mutex::new(None)), searched: Default::default() };
    search(&mut f, ctx);
    app.fix = Some(f);
}

/// The YouTube video in a pasted link (or a bare 11-character id).
fn video_in(text: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?:v=|youtu\.be/|shorts/|/embed/)([\w-]{11})").ok()?;
    re.captures(text).map(|c| c[1].to_string()).or_else(|| {
        let t = text.trim();
        (t.len() == 11 && !t.contains(' ') && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && t.chars().any(|c| c.is_ascii_digit() || c.is_ascii_uppercase())).then(|| t.to_string())
    })
}

fn search(f: &mut Fix, ctx: &egui::Context) {
    let key = (f.source.to_string(), f.query.trim().to_string());
    if key.1.is_empty() || key == f.searched {
        return;
    }
    f.searched = key.clone();
    let slot: Results = Arc::new(Mutex::new(None));
    f.results = slot.clone();
    // a link: just that video
    if let Some(v) = video_in(&key.1) {
        let t = ITrack { title: "The video in your link".into(), youtube_id: Some(v.clone()), source_key: format!("yt:{v}"), raw_title: format!("youtube.com/watch?v={v}"), ..Default::default() };
        *slot.lock() = Some(Ok(vec![t]));
        return;
    }
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("reaudio-search".into())
        .spawn(move || {
            let r = crate::sources::search(&key.1, &key.0, 12).map(|v| v.into_iter().filter(|t| t.youtube_id.is_some()).collect());
            *slot.lock() = Some(r);
            ctx.request_repaint();
        })
        .ok();
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut f) = app.fix.take() else { return };
    let Some(t) = app.lib.track(&f.id) else { return };
    let pal = app.pal;
    let mut use_video: Option<String> = None;
    let mut cancel = false;
    let open = window(pal, ctx, "REPLACE AUDIO", Vec2::new(640.0, 470.0), true, |ui| {
        ui.label(egui::RichText::new(format!("{} - {}", t.artist, t.title)).font(vt(22.0)).color(pal.text));
        let now = t.youtube_id.as_deref().map(|y| format!(" · now from youtube.com/watch?v={y}")).unwrap_or_default();
        ui.label(egui::RichText::new(format!("{}{now}", if t.duration > 0.0 { fmt_time(t.duration) } else { "?:??".into() })).font(vt(16.0)).color(pal.dim));
        ui.add_space(8.0);
        let mut go = false;
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut f.query).desired_width(330.0).font(vt(19.0)).hint_text("Search, or paste a YouTube link"));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            for (s, label, tip) in [("songs", "SONGS", "YouTube Music's songs (the album recordings)"), ("youtube", "VIDEOS", "All of YouTube: music videos, live versions, uploads")] {
                if super::widgets::outline_button(ui, &pal, label, f.source == s).on_hover_text(tip).clicked() {
                    f.source = s;
                    go = true;
                }
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
        let list_h = ui.available_height() - 40.0;
        match res {
            None => {
                ui.allocate_ui(Vec2::new(ui.available_width(), list_h), |ui| {
                    ui.centered_and_justified(|ui| ui.label(egui::RichText::new("SEARCHING…").font(px(7.0)).color(pal.dim)));
                });
            }
            Some(Err(e)) => {
                ui.allocate_ui(Vec2::new(ui.available_width(), list_h), |ui| {
                    ui.centered_and_justified(|ui| ui.label(egui::RichText::new(format!("Couldn't search: {e}")).font(vt(18.0)).color(pal.dim)));
                });
            }
            Some(Ok(list)) if list.is_empty() => {
                ui.allocate_ui(Vec2::new(ui.available_width(), list_h), |ui| {
                    ui.centered_and_justified(|ui| ui.label(egui::RichText::new("NOTHING FOUND: TRY OTHER WORDS, OR VIDEOS").font(px(6.0)).color(pal.dim)));
                });
            }
            Some(Ok(list)) => {
                egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
                    for c in &list {
                        if let Some(v) = row(app, ui, &t, c) {
                            use_video = Some(v);
                        }
                    }
                });
            }
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("The new sound replaces the file. Title, cover, likes, plays and playlists stay.").font(vt(15.0)).color(pal.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, &pal, "CANCEL", false, true).clicked() {
                    cancel = true;
                }
            });
        });
    });
    if let Some(v) = use_video {
        match app.dl.replace_audio(&t.id, &v) {
            Some(job) => {
                app.fixing.push((t.id.clone(), job));
                app.toast(format!("Getting the new audio for {}: see DOWNLOADS for progress", t.title));
            }
            None => app.toast_err("Only songs DK.FM downloaded can get new audio (your own files stay as they are)"),
        }
        return;
    }
    if open && !cancel {
        app.fix = Some(f);
    }
}

/// One result: ▶ listen, its title and channel, its length against the song's, USE THIS.
/// Returns the video to use when USE THIS is clicked.
fn row(app: &mut App, ui: &mut Ui, song: &crate::store::Track, c: &ITrack) -> Option<String> {
    let pal = app.pal;
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 44.0), Sense::hover());
    if resp.hovered() {
        fill(ui.painter(), r, pal.panel_hi);
    }
    let play = Rect::from_min_size(r.min + Vec2::new(4.0, 7.0), Vec2::splat(30.0));
    super::preview::button(app, ui, play, c);
    let current = c.youtube_id.is_some() && c.youtube_id == song.youtube_id;
    let use_r = Rect::from_min_size(egui::pos2(r.right() - 96.0, r.top() + 8.0), Vec2::new(90.0, 28.0));
    let len_x = use_r.left() - 12.0;
    let p = ui.painter().with_clip_rect(Rect::from_min_max(egui::pos2(play.right() + 8.0, r.top()), egui::pos2(len_x - 70.0, r.bottom())));
    let title = if c.raw_title.is_empty() { c.title.clone() } else { c.raw_title.clone() };
    p.text(egui::pos2(play.right() + 10.0, r.top() + 13.0), Align2::LEFT_CENTER, &title, vt(19.0), pal.text);
    let by = [c.artists.join(", "), c.album.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    p.text(egui::pos2(play.right() + 10.0, r.top() + 31.0), Align2::LEFT_CENTER, by, vt(15.0), pal.dim);
    // length: green when it's about the song's (the same recording), red when far off
    if let Some(ms) = c.duration_ms {
        let secs = ms as f64 / 1000.0;
        let off = secs - song.duration;
        let (txt, col) = if song.duration <= 0.0 {
            (fmt_time(secs), pal.dim)
        } else if off.abs() < 1.5 {
            (fmt_time(secs), pal.accent2)
        } else {
            (format!("{} ({}{}s)", fmt_time(secs), if off > 0.0 { "+" } else { "-" }, off.abs().round()), if off.abs() > 10.0 { pal.accent } else { pal.text })
        };
        ui.painter().text(egui::pos2(len_x, r.center().y), Align2::RIGHT_CENTER, txt, vt(17.0), col);
    }
    if current {
        ui.painter().text(use_r.center(), Align2::CENTER_CENTER, "CURRENT AUDIO", px(5.0), pal.dim);
        return None;
    }
    let u = ui.interact(use_r, ui.id().with(("reaudio-use", &c.source_key)), Sense::click());
    fill(ui.painter(), use_r, if u.hovered() { pal.accent } else { pal.panel });
    frame_rect(ui.painter(), use_r, 1.5, pal.accent);
    ui.painter().text(use_r.center(), Align2::CENTER_CENTER, "USE THIS", px(6.0), if u.hovered() { pal.ink } else { pal.text });
    if u.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    u.on_hover_text("Replace this song's audio with this one").clicked().then(|| c.youtube_id.clone()).flatten()
}

/// Songs whose new audio finished: lyrics, loudness and waveform start over for the new sound,
/// and the one playing starts again with it.
pub fn poll(app: &mut App) {
    if app.fixing.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut app.fixing);
    for (id, job) in pending {
        match app.dl.job_result(&job) {
            None => app.fixing.push((id, job)),
            Some(Ok(())) => {
                crate::lyricsrc::forget(&id);
                crate::wordsync::forget(&id);
                app.lyrics.forget(&id);
                app.lib.forget_analysis(&id);
                app.player.analyzer.peaks.lock().remove(&id);
                let name = app.lib.track(&id).map(|t| t.title).unwrap_or_default();
                if app.player.current_id().as_deref() == Some(id.as_str()) {
                    let i = app.player.st.lock().index;
                    app.player.play_index(i.max(0) as usize);
                }
                app.toast(format!("New audio for {name}"));
            }
            Some(Err(e)) => app.toast_err(format!("Couldn't get the new audio: {e}")),
        }
    }
}

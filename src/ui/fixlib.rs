//! FIX MY LIBRARY (the ⋯ menu): songs with something wrong, each with the button that fixes it.
//!   NO COVER: REPLACE COVER
//!   NO LYRICS (looked up, none found): REPLACE LYRICS
//!   WRONG LENGTH: a song DK.FM downloaded whose length is far from every release of it on both
//!     Deezer and Apple Music (their free catalogues, no account; one alone often misses the album
//!     version): usually another recording (live, sped up, a video with an intro): REPLACE AUDIO
//! Lengths are checked in the background while the screen is open, each song once (saved in
//! fixcheck.json, with what you chose to IGNORE).
use super::theme::{px, vt};
use super::widgets::{fill, fmt_time, frame_rect, outline_button};
use super::App;
use crate::store::{data_dir, Track};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Ui, Vec2};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// How far off (seconds) a song's length must be from every release of it to be listed.
const OFF: f64 = 15.0;

#[derive(Serialize, Deserialize, Default)]
struct Checks {
    /// song -> lengths (s) of its releases on Deezer (empty: not found there)
    #[serde(default)]
    lengths: HashMap<String, Vec<u32>>,
    /// the same from Apple Music, for songs Deezer's lengths don't fit
    #[serde(default)]
    apple: HashMap<String, Vec<u32>>,
    /// "song|kind" you chose to ignore
    #[serde(default)]
    ignored: HashSet<String>,
}

fn path() -> std::path::PathBuf {
    data_dir().join("fixcheck.json")
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Cover,
    Lyrics,
    Length,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Kind::Cover => "cover",
            Kind::Lyrics => "lyrics",
            Kind::Length => "length",
        }
    }
}

#[derive(Default)]
pub struct FixState {
    checks: Arc<Mutex<Option<Checks>>>,
    running: Arc<AtomicBool>,
    /// "" = all, else a Kind key
    filter: &'static str,
    /// (library generation, lengths known, when) -> rows
    rows: Vec<(String, Kind, String)>,
    made: Option<(u64, usize, std::time::Instant)>,
    /// songs left to check / checked this time
    progress: Arc<Mutex<(usize, usize)>>,
}

fn load(c: &Mutex<Option<Checks>>) {
    let mut g = c.lock();
    if g.is_none() {
        *g = Some(std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
    }
}

fn save(c: &Checks) {
    crate::store::save_json(&path(), c);
}

/// Letters and digits only, lower case, without (…) / […] parts: for matching names.
fn norm(s: &str) -> String {
    static PARENS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\s*[\(\[][^\)\]]*[\)\]]").unwrap());
    PARENS.replace_all(s, "").chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Whether a song's length is far from all of these (and there are some).
fn off(lengths: Option<&Vec<u32>>, secs: f64) -> Option<f64> {
    let best = lengths?.iter().filter(|d| **d >= 60).map(|d| *d as f64).min_by(|a, b| (a - secs).abs().total_cmp(&(b - secs).abs()))?;
    ((best - secs).abs() > OFF).then_some(best)
}

/// The lengths of this song's releases on Apple Music (the same title by the same artist).
fn apple_lengths(t: &Track) -> Result<Vec<u32>, String> {
    let artist = crate::library::main_artist(&t.artist);
    let title = crate::lyricsrc::bare_title(&t.title);
    let v = crate::net::get_json(&format!("https://itunes.apple.com/search?entity=song&limit=50&term={}", crate::net::enc(&format!("{artist} {title}"))), &[])?;
    let (na, nt) = (norm(&artist), norm(&title));
    Ok(v["results"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter(|x| {
            let a = norm(x["artistName"].as_str().unwrap_or(""));
            !nt.is_empty() && norm(x["trackName"].as_str().unwrap_or("")) == nt && !a.is_empty() && (a.contains(&na) || na.contains(&a))
        })
        .filter_map(|x| x["trackTimeMillis"].as_u64().map(|d| (d / 1000) as u32))
        .filter(|d| *d >= 60)
        .collect())
}

/// The lengths of this song's releases on Deezer (the same title by the same artist).
fn deezer_lengths(t: &Track) -> Result<Vec<u32>, String> {
    let artist = crate::library::main_artist(&t.artist);
    let title = crate::lyricsrc::bare_title(&t.title);
    let v = crate::net::get_json(&format!("https://api.deezer.com/search?q={}&limit=25", crate::net::enc(&format!("{artist} {title}"))), &[])?;
    if v.get("error").is_some() {
        return Err(v["error"]["message"].as_str().unwrap_or("Deezer error").to_string());
    }
    let (na, nt) = (norm(&artist), norm(&title));
    if na.is_empty() || nt.is_empty() {
        return Ok(Vec::new());
    }
    Ok(v["data"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter(|x| {
            let a = norm(x["artist"]["name"].as_str().unwrap_or(""));
            norm(x["title"].as_str().unwrap_or("")) == nt && !a.is_empty() && (a.contains(&na) || na.contains(&a))
        })
        .filter_map(|x| x["duration"].as_u64().map(|d| d as u32))
        // (30-second clips and intros aren't the song)
        .filter(|d| *d >= 60)
        .collect())
}

/// Check the lengths of these songs in the background (one at a time, gently).
fn start_checks(app: &App, songs: Vec<Track>) {
    let st = &app.fixlib;
    if songs.is_empty() || st.running.swap(true, Ordering::Relaxed) {
        return;
    }
    let (checks, running, progress, ctx) = (st.checks.clone(), st.running.clone(), st.progress.clone(), app.covers.ctx.clone());
    *progress.lock() = (songs.len(), 0);
    std::thread::Builder::new()
        .name("fix-lengths".into())
        .spawn(move || {
            for (n, t) in songs.iter().enumerate() {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                // (a network problem: stop, and try the rest next time)
                let known = checks.lock().as_ref().and_then(|c| c.lengths.get(&t.id).cloned());
                let found = match known {
                    Some(k) => k,
                    None => {
                        let Ok(found) = deezer_lengths(t) else { break };
                        found
                    }
                };
                // Deezer's don't fit: ask Apple Music too (Deezer's search often misses the album version)
                let apple = if off(Some(&found), t.duration).is_some() {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    let Ok(a) = apple_lengths(t) else { break };
                    Some(a)
                } else {
                    None
                };
                if let Some(c) = checks.lock().as_mut() {
                    c.lengths.insert(t.id.clone(), found);
                    if let Some(a) = apple {
                        c.apple.insert(t.id.clone(), a);
                    }
                    if n % 10 == 9 {
                        save(c);
                    }
                }
                progress.lock().1 = n + 1;
                if let Some(c) = &ctx {
                    c.request_repaint();
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            if let Some(c) = checks.lock().as_ref() {
                save(c);
            }
            running.store(false, Ordering::Relaxed);
            if let Some(c) = &ctx {
                c.request_repaint();
            }
        })
        .ok();
}

/// Stop checking (the screen was left).
pub fn stop(app: &App) {
    app.fixlib.running.store(false, Ordering::Relaxed);
}

/// What's wrong, worked out again when the library changes, lengths come in, or every few s.
fn rows(app: &mut App) {
    load(&app.fixlib.checks);
    let gen = app.lib.gen.load(Ordering::Relaxed);
    let known = app.fixlib.checks.lock().as_ref().map(|c| c.lengths.len()).unwrap_or(0);
    if let Some((g, k, at)) = app.fixlib.made {
        if g == gen && k == known && at.elapsed().as_secs() < 3 {
            return;
        }
    }
    let tracks: Vec<Track> = { let d = app.lib.data.read(); d.tracks.values().cloned().collect() };
    let mut out = Vec::new();
    let mut unchecked = Vec::new();
    {
        let g = app.fixlib.checks.lock();
        let c = g.as_ref().unwrap();
        let ignored = |id: &str, k: Kind| c.ignored.contains(&format!("{id}|{}", k.key()));
        for t in &tracks {
            if t.cover.is_none() && t.thumb.is_none() && !ignored(&t.id, Kind::Cover) {
                out.push((t.id.clone(), Kind::Cover, "No cover".to_string()));
            }
            if crate::lyricsrc::found_none(&t.id) && !ignored(&t.id, Kind::Lyrics) {
                out.push((t.id.clone(), Kind::Lyrics, "No lyrics found".to_string()));
            }
            if app.dl.requalifiable(t) {
                let deezer = off(c.lengths.get(&t.id), t.duration);
                match (c.lengths.contains_key(&t.id), deezer, c.apple.get(&t.id)) {
                    (false, _, _) => unchecked.push(t.clone()),
                    // (off on Deezer and not asked Apple Music yet)
                    (true, Some(_), None) if t.duration > 0.0 => unchecked.push(t.clone()),
                    (true, Some(_), Some(a)) if t.duration > 0.0 && !ignored(&t.id, Kind::Length) => {
                        if let Some(best) = off(Some(a), t.duration) {
                            let by = t.duration - best;
                            out.push((t.id.clone(), Kind::Length, format!("{} long, the song is {} ({}{:.0} s): maybe another recording", fmt_time(t.duration), fmt_time(best), if by > 0.0 { "+" } else { "-" }, by.abs())));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let name = |id: &str| tracks.iter().find(|t| t.id == id).map(|t| (t.artist.to_lowercase(), t.title.to_lowercase())).unwrap_or_default();
    out.sort_by_key(|r| (r.1 as u8, name(&r.0)));
    app.fixlib.rows = out;
    app.fixlib.made = Some((gen, known, std::time::Instant::now()));
    if !unchecked.is_empty() {
        start_checks(app, unchecked);
    }
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    rows(app);
    let all = app.fixlib.rows.clone();
    let count = |k: Kind| all.iter().filter(|r| r.1 == k).count();
    let (left, done) = *app.fixlib.progress.lock();
    let checking = app.fixlib.running.load(Ordering::Relaxed);
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.label(egui::RichText::new("FIX MY LIBRARY").font(px(12.0)).color(pal.text));
        ui.label(egui::RichText::new(if all.is_empty() && !checking { "NOTHING TO FIX ✔".to_string() } else { format!("{} THING{} TO FIX", all.len(), if all.len() == 1 { "" } else { "S" }) }).color(pal.dim));
        ui.label(egui::RichText::new("Songs with no cover, no lyrics, or a length far from the real song's (usually another recording: live, sped up, or a video with an intro). Each has the button that fixes it.").color(pal.dim));
        if checking {
            ui.label(egui::RichText::new(format!("CHECKING LENGTHS ON DEEZER… {done} / {left}")).font(px(5.0)).color(pal.accent2));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (k, label) in [("", format!("ALL ({})", all.len())), ("cover", format!("NO COVER ({})", count(Kind::Cover))), ("lyrics", format!("NO LYRICS ({})", count(Kind::Lyrics))), ("length", format!("WRONG LENGTH ({})", count(Kind::Length)))] {
                if outline_button(ui, &pal, &label, app.fixlib.filter == k).clicked() {
                    app.fixlib.filter = k;
                }
            }
        });
    });
    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), egui::Stroke::new(2.0_f32, pal.line));
    let filter = app.fixlib.filter;
    let shown: Vec<_> = all.into_iter().filter(|r| filter.is_empty() || r.1.key() == filter).collect();
    let mut act: Option<(String, Kind, bool)> = None;
    let row_h = 52.0;
    egui::ScrollArea::vertical().id_salt("fixlib").auto_shrink([false; 2]).show_rows(ui, row_h, shown.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (id, kind, what) in &shown[range] {
            let Some(t) = app.lib.track(id) else { continue };
            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::click());
            if resp.hovered() {
                fill(ui.painter(), r, pal.panel_hi);
            }
            let cover = Rect::from_min_size(r.min + Vec2::new(12.0, 6.0), Vec2::splat(40.0));
            fill(ui.painter(), cover, pal.bg);
            if let Some(tex) = super::preview::cover_tex(app, ui.ctx(), &t, 64) {
                ui.painter().image(tex, cover, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            frame_rect(ui.painter(), cover, 1.0, pal.line);
            let fix_r = Rect::from_min_size(Pos2::new(r.right() - 250.0, r.top() + 12.0), Vec2::new(150.0, 28.0));
            let ign_r = Rect::from_min_size(Pos2::new(r.right() - 92.0, r.top() + 12.0), Vec2::new(80.0, 28.0));
            let p = ui.painter().with_clip_rect(Rect::from_min_max(r.min, Pos2::new(fix_r.left() - 8.0, r.bottom())));
            p.text(Pos2::new(cover.right() + 10.0, r.top() + 16.0), Align2::LEFT_CENTER, format!("{} - {}", t.artist, t.title), vt(19.0), pal.text);
            p.text(Pos2::new(cover.right() + 10.0, r.top() + 36.0), Align2::LEFT_CENTER, what, vt(16.0), if *kind == Kind::Length { pal.accent } else { pal.dim });
            let label = match kind {
                Kind::Cover => "REPLACE COVER",
                Kind::Lyrics => "REPLACE LYRICS",
                Kind::Length => "REPLACE AUDIO",
            };
            for (br, text, ignore) in [(fix_r, label, false), (ign_r, "IGNORE", true)] {
                let b = ui.interact(br, ui.id().with(("fix", id, kind.key(), ignore)), Sense::click());
                fill(ui.painter(), br, if b.hovered() { pal.accent } else { pal.panel });
                frame_rect(ui.painter(), br, 1.5, if ignore { pal.line_hi } else { pal.accent });
                ui.painter().text(br.center(), Align2::CENTER_CENTER, text, px(5.0), if b.hovered() { pal.ink } else { pal.text });
                if b.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let tip = if ignore { "Leave it as it is (it won't be listed again)" } else { "Fix it" };
                if b.on_hover_text(tip).clicked() {
                    act = Some((id.clone(), *kind, ignore));
                }
            }
            if resp.on_hover_text("Click to play it").clicked() {
                app.player.play_pick(vec![id.clone()], 0, None);
            }
        }
    });
    if shown.is_empty() && !checking {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(egui::RichText::new("NOTHING HERE ✔").font(px(7.0)).color(pal.dim)));
    }
    match act {
        Some((id, kind, true)) => {
            if let Some(c) = app.fixlib.checks.lock().as_mut() {
                c.ignored.insert(format!("{id}|{}", kind.key()));
                save(c);
            }
            app.fixlib.made = None;
        }
        Some((id, Kind::Cover, false)) => super::newcover::open(app, &id, ui.ctx()),
        Some((id, Kind::Lyrics, false)) => super::newlyrics::open(app, &id, ui.ctx()),
        Some((id, Kind::Length, false)) => super::reaudio::open(app, &id, ui.ctx()),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_match_without_extras() {
        assert_eq!(super::norm("Immortal (Explicit) [Remastered]"), "immortal");
        assert_eq!(super::norm("AC/DC"), "acdc");
    }
}

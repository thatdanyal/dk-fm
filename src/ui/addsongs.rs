//! "ADD SONGS" box inside a playlist: search your library (instant), or find new songs on
//! YouTube Music (SONGS) / YouTube and download them straight into the playlist. Online searches
//! run on a background thread and are cached per query; songs you already have are added instantly.
use super::theme::{px, vt, Pal};
use super::widgets::{fill, fmt_time, frame_rect, tb_button};
use super::App;
use crate::downloader::TStatus;
use crate::library::ta_key;
use crate::sources::{self, Collection, ITrack};
use crate::store::Playlist;
use eframe::egui::{self, Align2, Pos2, Rect, Sense, Ui, Vec2};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// (source, query)
type Key = (String, String);
type Found = Result<Vec<ITrack>, String>;

#[derive(Default)]
pub struct AddBox {
    pub open: bool,
    pub query: String,
    pub focus: bool,
    edited: Option<Instant>,
    /// library matches for (query, lib.gen)
    local: Option<(String, u64, Vec<String>)>,
    /// recent online searches (newest last, ≤ 12)
    online: Vec<(Key, Found)>,
    busy: HashSet<Key>,
    slot: Arc<Mutex<Vec<(Key, Found)>>>,
    /// online results already in the library: (lib.gen, key) -> track id per result
    owned: Option<(u64, Key, Vec<Option<String>>)>,
}

pub const SOURCES: [(&str, &str); 3] = [("songs", "SONGS"), ("youtube", "YOUTUBE"), ("soundcloud", "SOUNDCLOUD")];

enum Btn {
    Add,
    Added,
    Have,
    Get,
    Busy(String),
    Retry,
}

/// One result row; returns true when its button was clicked. `tip` = what + GET does.
fn row(ui: &mut Ui, pal: &Pal, w: f32, title: &str, sub: &str, secs: Option<f64>, btn: Btn, tip: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
    let br = Rect::from_min_size(Pos2::new(r.right() - 92.0, r.top() + 3.0), Vec2::new(86.0, 20.0));
    let over = resp.hovered() && ui.input(|i| i.pointer.hover_pos()).map(|q| br.contains(q)).unwrap_or(false);
    if resp.hovered() {
        fill(ui.painter(), r, pal.panel_hi);
    }
    let p = ui.painter();
    let cy = r.center().y;
    let clip = p.with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.right() - 160.0, r.bottom())));
    let g = clip.layout_no_wrap(title.to_string(), vt(19.0), pal.text);
    let gw = g.size().x;
    clip.galley(Pos2::new(r.left() + 8.0, cy - g.size().y / 2.0), g, pal.text);
    clip.text(Pos2::new(r.left() + 16.0 + gw, cy), Align2::LEFT_CENTER, sub, vt(17.0), pal.dim);
    if let Some(s) = secs {
        p.text(Pos2::new(r.right() - 104.0, cy), Align2::RIGHT_CENTER, fmt_time(s), vt(17.0), pal.dim);
    }
    let (label, clickable) = match &btn {
        Btn::Add => ("+ ADD".to_string(), true),
        Btn::Get => ("+ GET".to_string(), true),
        Btn::Retry => ("FAILED ↻".to_string(), true),
        Btn::Added => ("✔ ADDED".to_string(), false),
        Btn::Have => ("✔ HAVE IT".to_string(), false),
        Btn::Busy(s) => (s.clone(), false),
    };
    if clickable {
        fill(p, br, if over { pal.accent } else { pal.bg });
        frame_rect(p, br, 2.0, pal.accent);
    }
    p.text(br.center(), Align2::CENTER_CENTER, label, px(6.0), if clickable && over { pal.ink } else if clickable { pal.accent2 } else { pal.dim });
    if clickable && over {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if matches!(btn, Btn::Get | Btn::Retry) {
            resp.clone().on_hover_text(tip);
        }
    }
    clickable && resp.clicked() && over
}

pub fn show(app: &mut App, ui: &mut Ui, pl: &Playlist) {
    let pal = app.pal;
    let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
    // where to look (Settings > Search): library and online sources, in order
    let (order, picked, n) = { let s = app.settings.lock(); (s.search_sources.clone(), s.search_source.clone(), s.search_results.clamp(1, 50) as usize) };
    let online: Vec<(&str, &str)> = order.iter().filter_map(|k| SOURCES.iter().find(|s| s.0 == k).copied()).collect();
    let use_lib = order.iter().any(|k| k == "library");
    let lib_first = use_lib && order.iter().position(|k| k == "library") < order.iter().position(|k| SOURCES.iter().any(|s| s.0 == k));
    let source = online.iter().find(|s| s.0 == picked).or(online.first()).map(|s| s.0.to_string());
    // finished online searches
    let done: Vec<(Key, Found)> = app.browser.add.slot.lock().drain(..).collect();
    for (k, r) in done {
        let b = &mut app.browser.add;
        b.busy.remove(&k);
        b.online.retain(|x| x.0 != k);
        b.online.push((k, r));
        if b.online.len() > 12 {
            b.online.drain(..1);
        }
    }
    let max_h = (ui.available_height() * 0.5).clamp(120.0, 380.0);
    let mut enter = false;
    let mut close = false;
    egui::Frame::new().fill(pal.bg2).stroke(egui::Stroke::new(2.0_f32, pal.line)).inner_margin(egui::Margin::symmetric(10, 8)).outer_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("ADD SONGS").font(px(7.0)).color(pal.accent2));
            let te = ui.add(egui::TextEdit::singleline(&mut app.browser.add.query).hint_text("Song, artist or album…").desired_width((ui.available_width() - 40.0).max(80.0)).font(vt(19.0)));
            if app.browser.add.focus {
                te.request_focus();
                app.browser.add.focus = false;
            }
            if te.changed() {
                app.browser.add.edited = Some(Instant::now());
            }
            enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if tb_button(ui, &pal, "×", false).on_hover_text("Close").clicked() {
                close = true;
            }
        });
        let q = app.browser.add.query.trim().to_string();
        if q.is_empty() {
            ui.label(egui::RichText::new("Search your library, or find new songs on YouTube Music / YouTube and download them straight into this playlist.").color(pal.dim));
            return;
        }
        let w = ui.available_width();
        egui::ScrollArea::vertical().id_salt("addsongs").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if lib_first {
                library_rows(app, ui, pl, &q, gen, w);
                ui.add_space(8.0);
            }
            if let Some(source) = &source {
                online_section(app, ui, pl, (&q, gen, w), &online, source, n, enter);
            }
            if use_lib && !lib_first {
                ui.add_space(8.0);
                library_rows(app, ui, pl, &q, gen, w);
            }
        });
    });
    if close {
        app.browser.add.open = false;
    }
    ui.add_space(6.0);
}

fn library_rows(app: &mut App, ui: &mut Ui, pl: &Playlist, q: &str, gen: u64, w: f32) {
    let pal = app.pal;
    if app.browser.add.local.as_ref().map(|l| l.0 != q || l.1 != gen).unwrap_or(true) {
        let toks: Vec<String> = q.to_lowercase().split_whitespace().map(String::from).collect();
        let d = app.lib.data.read();
        let mut hits: Vec<(f32, &String)> = d.tracks.values().filter_map(|t| super::palette::score(&format!("{} {} {}", t.title, t.artist, t.album).to_lowercase(), &toks).map(|s| (s + d.stats.get(&t.id).map(|x| x.plays as f32 * 0.5).unwrap_or(0.0), &t.id))).collect();
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        let ids = hits.into_iter().take(6).map(|h| h.1.clone()).collect();
        drop(d);
        app.browser.add.local = Some((q.to_string(), gen, ids));
    }
    ui.label(egui::RichText::new("IN YOUR LIBRARY").font(px(6.0)).color(pal.dim));
    ui.add_space(2.0);
    let local = app.browser.add.local.as_ref().map(|l| l.2.clone()).unwrap_or_default();
    if local.is_empty() {
        ui.label(egui::RichText::new("  No matches in your library").color(pal.dim));
    }
    for id in &local {
        let Some(t) = app.lib.track(id) else { continue };
        let inp = pl.track_ids.contains(id);
        if row(ui, &pal, w, &t.title, format!("{} · {}", t.artist, t.album).trim_end_matches(" · "), Some(t.duration), if inp { Btn::Added } else { Btn::Add }, "") {
            app.lib.playlist_add(&pl.id, std::slice::from_ref(id));
            app.edit_settings(|s| s.last_playlist = pl.id.clone());
        }
    }
}

/// `at` = (query, lib.gen, width)
fn online_section(app: &mut App, ui: &mut Ui, pl: &Playlist, at: (&str, u64, f32), online: &[(&str, &str)], source: &str, n: usize, enter: bool) {
    let (q, gen, w) = at;
    let pal = app.pal;
    let mut switch = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("FIND NEW SONGS").font(px(6.0)).color(pal.dim));
        ui.add_space(6.0);
        for (k, label) in online {
            let tip = match *k { "songs" => "Songs on YouTube Music (studio versions)", "youtube" => "Any video on YouTube", _ => "Tracks on SoundCloud" };
            if tb_button(ui, &pal, label, source == *k).on_hover_text(tip).clicked() && source != *k {
                switch = Some(*k);
            }
        }
    });
    let source = match switch {
        Some(k) => {
            app.edit_settings(|s| s.search_source = k.to_string());
            k.to_string()
        }
        None => source.to_string(),
    };
    let key: Key = (source.clone(), q.to_string());
    let b = &mut app.browser.add;
    let cached = b.online.iter().find(|x| x.0 == key).map(|x| x.1.clone());
    let idle = b.edited.map(|t| t.elapsed() >= Duration::from_millis(700)).unwrap_or(true);
    if cached.is_none() && !b.busy.contains(&key) && q.chars().count() >= 2 {
        if idle || enter || switch.is_some() {
            b.busy.insert(key.clone());
            let (slot, ctx, k) = (b.slot.clone(), ui.ctx().clone(), key.clone());
            std::thread::spawn(move || {
                let r = sources::search(&k.1, &k.0, n);
                if let Ok(v) = &r {
                    super::cjk::ensure(&ctx, &v.iter().map(|t| format!("{} {}", t.title, t.artists.join(" "))).collect::<String>());
                }
                slot.lock().push((k, r));
                ctx.request_repaint();
            });
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }
    match cached {
        None => {
            let tools = crate::ytdlp::STATUS.lock().state.clone();
            ui.label(egui::RichText::new(if tools == "installing" { "SEARCHING… (getting the download tools first)" } else { "SEARCHING…" }).font(px(6.0)).color(pal.accent));
        }
        Some(Err(e)) => {
            ui.label(egui::RichText::new(format!("  {e}")).color(pal.accent));
        }
        Some(Ok(found)) => {
            // "hide explicit songs": only where the source marks them (YouTube Music)
            let found: Vec<ITrack> = if app.settings.lock().hide_explicit { found.into_iter().filter(|t| !t.explicit).collect() } else { found };
            online_rows(app, ui, pl, &key, &found, gen, w)
        }
    }
}

fn online_rows(app: &mut App, ui: &mut Ui, pl: &Playlist, key: &Key, found: &[ITrack], gen: u64, w: f32) {
    // which results you already have (recomputed only when the library or the results change)
    if app.browser.add.owned.as_ref().map(|o| o.0 != gen || o.1 != *key).unwrap_or(true) {
        let owned = owned_ids(app, found);
        app.browser.add.owned = Some((gen, key.clone(), owned));
    }
    let owned = app.browser.add.owned.as_ref().map(|o| o.2.clone()).unwrap_or_default();
    if result_rows(app, ui, Some(pl), found, &owned, w) {
        app.edit_settings(|s| s.last_playlist = pl.id.clone());
    }
}

/// The library songs these online results are, if you have them.
pub fn owned_ids(app: &App, found: &[ITrack]) -> Vec<Option<String>> {
    let idx = app.lib.key_index();
    found.iter().map(|t| idx.get(&t.source_key).or_else(|| idx.get(&ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title))).cloned()).collect()
}

/// Online results, each with + ADD (you have it) / + GET (download it, into `pl` if given) or its
/// download progress. Returns true when a button was clicked.
pub fn result_rows(app: &mut App, ui: &mut Ui, pl: Option<&Playlist>, found: &[ITrack], owned: &[Option<String>], w: f32) -> bool {
    let pal = app.pal;
    // download progress of results being fetched
    let jobs: HashMap<String, (TStatus, f32)> = {
        let want: HashSet<String> = found.iter().map(|t| format!("track-{}", col_id(t))).collect();
        app.dl.jobs.lock().iter().filter(|j| want.contains(&j.id)).filter_map(|j| j.tracks.first().map(|t| (j.id.clone(), (t.status.clone(), t.progress)))).collect()
    };
    if jobs.values().any(|j| j.0.active()) {
        ui.ctx().request_repaint_after(Duration::from_millis(400));
    }
    let tip = if pl.is_some() { "Download it into this playlist" } else { "Download it to your library" };
    let mut clicked = false;
    for (i, t) in found.iter().enumerate() {
        let cid = col_id(t);
        let have = owned.get(i).cloned().flatten();
        let btn = match (&have, jobs.get(&format!("track-{cid}"))) {
            (Some(id), _) if pl.map(|p| p.track_ids.contains(id)).unwrap_or(true) => if pl.is_some() { Btn::Added } else { Btn::Have },
            (Some(_), _) => Btn::Add,
            (None, Some((s, p))) => match s {
                TStatus::Queued => Btn::Busy("QUEUED".into()),
                TStatus::Searching => Btn::Busy("STARTING".into()),
                TStatus::Downloading => Btn::Busy(format!("{p:.0}%")),
                TStatus::Tagging | TStatus::Done => Btn::Busy("SAVING".into()),
                _ => Btn::Retry,
            },
            (None, None) => Btn::Get,
        };
        let sub = [t.artists.join(", "), t.album.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        if row(ui, &pal, w, &t.title, &sub, t.duration_ms.map(|d| d as f64 / 1000.0), btn, tip) {
            clicked = true;
            match (have, pl) {
                (Some(id), Some(pl)) => {
                    app.lib.playlist_add(&pl.id, &[id]);
                }
                (Some(_), None) => {}
                (None, _) => {
                    let src = if t.source_key.starts_with("sc:") { "soundcloud" } else { "youtube" };
                    let col = Collection { kind: "track".into(), id: cid.clone(), name: t.title.clone(), owner: t.artists.join(", "), cover: t.cover.clone(), tracks: vec![t.clone()], complete: true, via: src.into(), source: src.into(), url: t.direct_url.clone().unwrap_or_default(), warning: None };
                    app.dl.start_to(col, None, pl.map(|p| p.id.clone()));
                    app.toast(match pl { Some(p) => format!("Downloading \"{}\" into \"{}\"", t.title, p.name), None => format!("Downloading \"{}\"", t.title) });
                }
            }
        }
    }
    clicked
}

/// Download job id for a result ("yt" + video id, or "sc" + SoundCloud id).
fn col_id(t: &ITrack) -> String {
    t.source_key.replace(':', "")
}

//! "Recommended" under a playlist (like Spotify's): YouTube Music radio seeded from a few random
//! songs of the playlist, minus songs you already have (hidden ones included). Fetched in the
//! background only once the section is on screen, and kept per playlist for the session.
use super::theme::{px, vt};
use super::widgets::button;
use super::App;
use crate::library::{main_artist, ta_key, track_keys};
use crate::sources::{self, ITrack};
use crate::store::Playlist;
use eframe::egui::{self, Ui};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const SHOWN: usize = 10;
const ROW: f32 = 26.0;
const HEAD: f32 = 70.0;

type Found = Result<Vec<ITrack>, String>;

#[derive(Default)]
pub struct Recs {
    lists: HashMap<String, Entry>,
}

#[derive(Default)]
struct Entry {
    /// songs found, and where the 10 shown start
    pool: Vec<ITrack>,
    at: usize,
    /// everything shown so far (the next fetch leaves it out)
    seen: HashSet<String>,
    busy: bool,
    err: Option<String>,
    slot: Arc<Mutex<Option<Found>>>,
    /// (lib.gen, at) -> which shown songs you have now (downloaded since)
    owned: Option<(u64, usize, Vec<Option<String>>)>,
}

fn keys_of(t: &ITrack) -> Vec<String> {
    track_keys(Some(&t.source_key), t.spotify_id.as_deref(), t.youtube_id.as_deref(), t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title)
}

/// Height of the section, so the song list makes room for it.
pub fn height(app: &App, pid: &str) -> f32 {
    let n = app.recs.lists.get(pid).map(|e| e.pool.len().saturating_sub(e.at).min(SHOWN)).unwrap_or(0);
    HEAD + n.max(1) as f32 * ROW + 24.0
}

/// Radio of up to 3 random songs of the playlist, merged in turns.
fn fetch(app: &mut App, pl: &Playlist, ctx: &egui::Context) {
    let seeds: Vec<(String, String, Option<String>)> = {
        let d = app.lib.data.read();
        let mut cands: Vec<&crate::store::Track> = pl.track_ids.iter().filter_map(|i| d.tracks.get(i)).filter(|t| !d.stats.get(&t.id).map(|s| s.hidden).unwrap_or(false)).collect();
        fastrand::shuffle(&mut cands);
        cands.into_iter().take(3).map(|t| (t.title.clone(), main_artist(&t.artist), t.youtube_id.clone())).collect()
    };
    let mut skip: HashSet<String> = app.lib.key_index().into_keys().collect();
    let e = app.recs.lists.entry(pl.id.clone()).or_default();
    if seeds.is_empty() {
        e.err = Some("Add a few songs first".into());
        return;
    }
    e.busy = true;
    e.err = None;
    skip.extend(e.seen.iter().cloned());
    let (slot, ctx) = (e.slot.clone(), ctx.clone());
    std::thread::spawn(move || {
        let handles: Vec<_> = seeds.into_iter().map(|(title, artist, yid)| std::thread::spawn(move || sources::radio(&title, &artist, yid.as_deref()))).collect();
        let (mut lists, mut err) = (Vec::new(), None);
        for h in handles {
            match h.join().unwrap_or_else(|_| Err("Radio failed".into())) {
                Ok(c) => lists.push(c.tracks),
                Err(e) => err = Some(e),
            }
        }
        let r = if lists.is_empty() {
            Err(err.unwrap_or_else(|| "Nothing found".into()))
        } else {
            let mut out: Vec<ITrack> = Vec::new();
            let mut taken = HashSet::new();
            for i in 0..lists.iter().map(|l| l.len()).max().unwrap_or(0) {
                for t in lists.iter().filter_map(|l| l.get(i)) {
                    // (the same song from two radios, or in two versions of its title, once)
                    if !keys_of(t).iter().any(|k| skip.contains(k)) && taken.insert(t.source_key.clone()) && taken.insert(ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title)) {
                        out.push(t.clone());
                    }
                }
            }
            super::cjk::ensure(&ctx, &out.iter().map(|t| format!("{} {}", t.title, t.artists.join(" "))).collect::<String>());
            Ok(out)
        };
        *slot.lock() = Some(r);
        ctx.request_repaint();
    });
}

pub fn show(app: &mut App, ui: &mut Ui, pl: &Playlist) {
    let pal = app.pal;
    if let Some(e) = app.recs.lists.get_mut(&pl.id) {
        let done = e.slot.lock().take();
        if let Some(r) = done {
            e.busy = false;
            match r {
                Ok(v) => {
                    e.pool = v;
                    e.at = 0;
                    e.owned = None;
                    if e.pool.is_empty() {
                        e.err = Some("Nothing new found this time: try REFRESH".into());
                    }
                }
                Err(x) => e.err = Some(x),
            }
        }
    } else {
        fetch(app, pl, ui.ctx()); // first time on screen
    }
    let mut refresh = false;
    egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 18, bottom: 8 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.painter().hline(ui.max_rect().x_range(), ui.max_rect().top() - 8.0, egui::Stroke::new(2.0_f32, pal.line));
        let busy = app.recs.lists.get(&pl.id).map(|e| e.busy).unwrap_or(false);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("RECOMMENDED").font(px(9.0)).color(pal.text));
                ui.label(egui::RichText::new("Songs like this playlist's · + GET downloads one into it").font(vt(17.0)).color(pal.dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, &pal, if busy { "FINDING…" } else { "↻ REFRESH" }, false, !busy).on_hover_text("Show 10 other songs").clicked() {
                    refresh = true;
                }
            });
        });
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
        let Some(e) = app.recs.lists.get(&pl.id) else { return };
        let shown: Vec<ITrack> = e.pool.iter().skip(e.at).take(SHOWN).cloned().collect();
        if shown.is_empty() {
            let msg = match &e.err {
                Some(x) => format!("  {x}"),
                None if crate::ytdlp::STATUS.lock().state == "installing" => "  FINDING SONGS… (getting the download tools first)".into(),
                None => "  FINDING SONGS…".into(),
            };
            ui.label(egui::RichText::new(msg).font(if e.err.is_some() { vt(18.0) } else { px(6.0) }).color(pal.accent));
            return;
        }
        if e.owned.as_ref().map(|o| o.0 != gen || o.1 != e.at).unwrap_or(true) {
            let owned = super::addsongs::owned_ids(app, &shown);
            let at = e.at;
            if let Some(e) = app.recs.lists.get_mut(&pl.id) {
                e.owned = Some((gen, at, owned));
            }
        }
        let owned = app.recs.lists.get(&pl.id).and_then(|e| e.owned.as_ref()).map(|o| o.2.clone()).unwrap_or_default();
        let w = ui.available_width();
        super::addsongs::result_rows(app, ui, Some(pl), &shown, &owned, w);
    });
    if refresh {
        let more = match app.recs.lists.get_mut(&pl.id) {
            Some(e) => {
                let shown: Vec<String> = e.pool.iter().skip(e.at).take(SHOWN).flat_map(keys_of).collect();
                e.seen.extend(shown);
                if e.at + 2 * SHOWN <= e.pool.len() {
                    e.at += SHOWN;
                    false
                } else {
                    true
                }
            }
            None => true,
        };
        if more {
            fetch(app, pl, ui.ctx());
        }
    }
}

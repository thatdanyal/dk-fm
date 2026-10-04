//! DISCOVER: new music picked from what you listen to. A shelf per top artist ("Because you play
//! …", YouTube Music radio from your most played song of theirs) and one from a song you liked,
//! minus songs you already have. Fetched in the background the first time the tab opens, kept for
//! the session; SHUFFLE picks other seeds.
use super::theme::{px, vt};
use super::widgets::button;
use super::App;
use crate::library::{main_artist, ta_key, track_keys};
use crate::sources::{self, ITrack};
use eframe::egui::{self, Ui};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;

/// songs per shelf
const PER: usize = 6;

struct Shelf {
    title: String,
    sub: String,
    songs: Vec<ITrack>,
}

#[derive(Default)]
pub struct Explore {
    shelves: Vec<Shelf>,
    busy: bool,
    err: Option<String>,
    started: bool,
    /// how many times SHUFFLE was pressed (picks other artists / songs)
    round: usize,
    slot: Arc<Mutex<Option<Result<Vec<Shelf>, String>>>>,
    /// everything shown so far (SHUFFLE shows other songs)
    seen: HashSet<String>,
    owned: Option<(u64, Vec<Vec<Option<String>>>)>,
}

fn keys_of(t: &ITrack) -> Vec<String> {
    track_keys(Some(&t.source_key), t.spotify_id.as_deref(), t.youtube_id.as_deref(), t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title)
}

/// (shelf title, subtitle, seed title, seed artist, seed YouTube id)
type Seed = (String, String, String, String, Option<String>);

fn seeds(app: &App, round: usize) -> Vec<Seed> {
    let d = app.lib.data.read();
    let ok = |t: &&crate::store::Track| !d.stats.get(&t.id).map(|s| s.hidden).unwrap_or(false);
    let plays = |id: &str| d.stats.get(id).map(|s| s.plays).unwrap_or(0);
    let mut out = Vec::new();
    // 3 top artists per round (round 0: your top 3, round 1: the next 3, ...), wrapping around
    let top = crate::discover::top_artists(&d, 12);
    if !top.is_empty() {
        for i in 0..3.min(top.len()) {
            let (k, name) = &top[(round * 3 + i) % top.len()];
            let mut theirs: Vec<&crate::store::Track> = d.tracks.values().filter(ok).filter(|t| main_artist(&t.artist).to_lowercase() == *k).collect();
            theirs.sort_by(|a, b| plays(&b.id).cmp(&plays(&a.id)).then(a.id.cmp(&b.id)));
            if let Some(t) = theirs.get(round / top.len().div_ceil(3) % theirs.len().max(1)) {
                out.push((format!("BECAUSE YOU PLAY {}", name.to_uppercase()), format!("Songs like \"{}\"", t.title), t.title.clone(), main_artist(&t.artist), t.youtube_id.clone()));
            }
        }
    }
    // a song you liked (a different one each round)
    let mut liked: Vec<&crate::store::Track> = d.tracks.values().filter(ok).filter(|t| d.stats.get(&t.id).map(|s| s.liked).unwrap_or(false)).collect();
    liked.sort_by(|a, b| a.id.cmp(&b.id));
    if !liked.is_empty() {
        let t = liked[fastrand::usize(..liked.len())];
        if !out.iter().any(|s| s.2 == t.title) {
            out.push(("FROM A SONG YOU LIKED".into(), format!("Songs like \"{}\" by {}", t.title, main_artist(&t.artist)), t.title.clone(), main_artist(&t.artist), t.youtube_id.clone()));
        }
    }
    out
}

fn fetch(app: &mut App, ctx: &egui::Context) {
    let seeds = seeds(app, app.explore.round);
    let e = &mut app.explore;
    e.started = true;
    if seeds.is_empty() {
        e.err = Some("Play or add a few songs first: Discover finds music like yours.".into());
        return;
    }
    e.busy = true;
    e.err = None;
    let mut skip: HashSet<String> = app.lib.key_index().into_keys().collect();
    skip.extend(e.seen.iter().cloned());
    let (slot, ctx) = (e.slot.clone(), ctx.clone());
    std::thread::spawn(move || {
        let handles: Vec<_> = seeds.into_iter().map(|s| std::thread::spawn(move || (sources::radio(&s.2, &s.3, s.4.as_deref()), s.0, s.1))).collect();
        let (mut shelves, mut err, mut taken) = (Vec::new(), None, HashSet::new());
        for h in handles {
            let Ok((r, title, sub)) = h.join() else { continue };
            match r {
                Ok(c) => {
                    // (a song on two shelves, or in two versions of its title, once)
                    let songs: Vec<ITrack> = c.tracks.into_iter().filter(|t| !keys_of(t).iter().any(|k| skip.contains(k)) && taken.insert(t.source_key.clone()) && taken.insert(ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title))).take(PER).collect();
                    if !songs.is_empty() {
                        shelves.push(Shelf { title, sub, songs });
                    }
                }
                Err(e) => err = Some(e),
            }
        }
        let all: String = shelves.iter().flat_map(|s| s.songs.iter()).map(|t| format!("{} {}", t.title, t.artists.join(" "))).collect();
        super::cjk::ensure(&ctx, &all);
        *slot.lock() = Some(if shelves.is_empty() { Err(err.unwrap_or_else(|| "Nothing new found this time: try SHUFFLE".into())) } else { Ok(shelves) });
        ctx.request_repaint();
    });
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let done = app.explore.slot.lock().take();
    if let Some(r) = done {
        let e = &mut app.explore;
        e.busy = false;
        e.owned = None;
        match r {
            Ok(v) => {
                e.seen.extend(v.iter().flat_map(|s| s.songs.iter()).flat_map(keys_of));
                e.shelves = v;
            }
            Err(x) => e.err = Some(x),
        }
    }
    if !app.explore.started {
        fetch(app, ui.ctx());
    }
    let mut shuffle = false;
    egui::ScrollArea::vertical().id_salt("discover").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::same(16)).show(ui, |ui| {
            let busy = app.explore.busy;
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("DISCOVER").font(px(12.0)).color(pal.text));
                    ui.label(egui::RichText::new("New music picked from what you play · + GET downloads it").font(vt(17.0)).color(pal.dim));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if button(ui, &pal, if busy { "FINDING…" } else { "↻ SHUFFLE" }, false, !busy).on_hover_text("Other artists, other songs").clicked() {
                        shuffle = true;
                    }
                });
            });
            ui.add_space(10.0);
            if app.explore.shelves.is_empty() {
                let msg = match &app.explore.err {
                    Some(x) => x.clone(),
                    None if crate::ytdlp::STATUS.lock().state == "installing" => "FINDING MUSIC… (getting the download tools first)".into(),
                    None => "FINDING MUSIC…".into(),
                };
                ui.label(egui::RichText::new(msg).font(if app.explore.err.is_some() { vt(19.0) } else { px(7.0) }).color(pal.accent));
                return;
            }
            if let Some(x) = &app.explore.err {
                ui.label(egui::RichText::new(x).color(pal.dim));
            }
            let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
            if app.explore.owned.as_ref().map(|o| o.0 != gen).unwrap_or(true) {
                let owned: Vec<Vec<Option<String>>> = app.explore.shelves.iter().map(|s| super::addsongs::owned_ids(app, &s.songs)).collect();
                app.explore.owned = Some((gen, owned));
            }
            let owned = app.explore.owned.as_ref().map(|o| o.1.clone()).unwrap_or_default();
            let shelves: Vec<(String, String, Vec<ITrack>)> = app.explore.shelves.iter().map(|s| (s.title.clone(), s.sub.clone(), s.songs.clone())).collect();
            let w = ui.available_width();
            for (i, (title, sub, songs)) in shelves.iter().enumerate() {
                ui.label(egui::RichText::new(title).font(px(9.0)).color(pal.accent2));
                ui.label(egui::RichText::new(sub).font(vt(17.0)).color(pal.dim));
                ui.add_space(4.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                super::addsongs::result_rows(app, ui, None, songs, owned.get(i).map(|v| v.as_slice()).unwrap_or(&[]), w);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add_space(14.0);
            }
        });
    });
    if shuffle {
        app.explore.round += 1;
        fetch(app, ui.ctx());
    }
}

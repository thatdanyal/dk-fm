//! DISCOVER: music that's new to you, picked from what you listen to, across your genres. Each
//! shelf is YouTube Music radio from one of your songs: two of your top artists ("Because you play
//! …"), your biggest genres (a favourite of each, by a different artist), a song you liked, and
//! the artists none of those are by that you've never had ("New artists for you"). Songs you have
//! are left out, and so is anything Discover showed you in the last 45 days (discover-seen.json).
//! Fetched in the background the first time the tab opens; SHUFFLE picks other seeds.
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

/// Songs Discover showed you (song key -> when, unix s): not shown again for 45 days.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Seen {
    #[serde(default)]
    keys: std::collections::HashMap<String, i64>,
}

const SEEN_DAYS: i64 = 45;

fn seen_path() -> std::path::PathBuf {
    crate::store::data_dir().join("discover-seen.json")
}

/// Remember these songs as shown (forgetting ones shown long ago).
fn remember(songs: &[&ITrack]) {
    let mut s: Seen = crate::store::load_json(&seen_path());
    let now = crate::store::now_secs();
    s.keys.retain(|_, at| now - *at < SEEN_DAYS * crate::discover::DAY);
    for t in songs {
        s.keys.insert(t.source_key.clone(), now);
    }
    crate::store::save_json(&seen_path(), &s);
}

fn seeds(app: &App, round: usize) -> Vec<Seed> {
    let d = app.lib.data.read();
    let genres = app.home.genres.lock();
    let ok = |t: &&crate::store::Track| !d.stats.get(&t.id).map(|s| s.hidden).unwrap_or(false);
    let plays = |id: &str| d.stats.get(id).map(|s| s.plays).unwrap_or(0);
    let mut out: Vec<Seed> = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    // 2 top artists per round (round 0: your top 2, round 1: the next 2, ...), wrapping around
    let top = crate::discover::top_artists(&d, 12);
    if !top.is_empty() {
        for i in 0..2.min(top.len()) {
            let (k, name) = &top[(round * 2 + i) % top.len()];
            used.insert(k.clone());
            let mut theirs: Vec<&crate::store::Track> = d.tracks.values().filter(ok).filter(|t| main_artist(&t.artist).to_lowercase() == *k).collect();
            theirs.sort_by(|a, b| plays(&b.id).cmp(&plays(&a.id)).then(a.id.cmp(&b.id)));
            if let Some(t) = theirs.get(round / top.len().div_ceil(2) % theirs.len().max(1)) {
                out.push((format!("BECAUSE YOU PLAY {}", name.to_uppercase()), format!("Songs like \"{}\"", t.title), t.title.clone(), main_artist(&t.artist), t.youtube_id.clone()));
            }
        }
    }
    // your biggest genres (by plays), 3 per round: a favourite of each by an artist not used yet
    let genre_of = |t: &crate::store::Track| {
        let g = t.genre.split([';', ',']).next().unwrap_or("").trim().to_string();
        if g.is_empty() { genres.of(&main_artist(&t.artist).to_lowercase()).unwrap_or("").to_string() } else { g }
    };
    let mut by_genre: std::collections::HashMap<String, (u64, Vec<&crate::store::Track>)> = Default::default();
    for t in d.tracks.values().filter(ok) {
        let g = genre_of(t);
        if !g.is_empty() {
            let e = by_genre.entry(g).or_default();
            e.0 += plays(&t.id) as u64 + 1;
            e.1.push(t);
        }
    }
    let mut gs: Vec<(String, (u64, Vec<&crate::store::Track>))> = by_genre.into_iter().filter(|g| g.1 .1.len() >= 3).collect();
    gs.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(&b.0)));
    for i in 0..3.min(gs.len()) {
        let (g, (_, songs)) = &gs[(round * 3 + i) % gs.len()];
        let mut songs: Vec<&&crate::store::Track> = songs.iter().filter(|t| !used.contains(&main_artist(&t.artist).to_lowercase())).collect();
        songs.sort_by(|a, b| plays(&b.id).cmp(&plays(&a.id)).then(a.id.cmp(&b.id)));
        if let Some(t) = songs.get(round / gs.len().div_ceil(3) % songs.len().max(1)) {
            used.insert(main_artist(&t.artist).to_lowercase());
            out.push((format!("MORE {}", g.to_uppercase()), format!("Songs like \"{}\" by {}", t.title, main_artist(&t.artist)), t.title.clone(), main_artist(&t.artist), t.youtube_id.clone()));
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
    super::home::start_genres(app);
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
    let shown_before: Seen = crate::store::load_json(&seen_path());
    let now = crate::store::now_secs();
    skip.extend(shown_before.keys.into_iter().filter(|(_, at)| now - at < SEEN_DAYS * crate::discover::DAY).map(|(k, _)| k));
    // artists you have songs by: anyone else is new to you
    let known: HashSet<String> = app.lib.data.read().tracks.values().map(|t| main_artist(&t.artist).to_lowercase()).collect();
    let (slot, ctx) = (e.slot.clone(), ctx.clone());
    std::thread::spawn(move || {
        let handles: Vec<_> = seeds.into_iter().map(|s| std::thread::spawn(move || (sources::radio(&s.2, &s.3, s.4.as_deref()), s.0, s.1))).collect();
        let (mut shelves, mut err, mut taken) = (Vec::new(), None, HashSet::new());
        let mut new_artists: Vec<ITrack> = Vec::new();
        for h in handles {
            let Ok((r, title, sub)) = h.join() else { continue };
            match r {
                Ok(c) => {
                    // (a song on two shelves, or in two versions of its title, once)
                    let mut fresh = c.tracks.into_iter().filter(|t| !keys_of(t).iter().any(|k| skip.contains(k)));
                    let mut songs: Vec<ITrack> = Vec::new();
                    // (at most 2 by one artist: a shelf is for finding more than one)
                    let mut per: std::collections::HashMap<String, usize> = Default::default();
                    let mut later: Vec<ITrack> = Vec::new();
                    for t in fresh.by_ref() {
                        if songs.len() >= PER {
                            break;
                        }
                        let a = t.artists.first().map(|a| main_artist(a).to_lowercase()).unwrap_or_default();
                        if per.get(&a).is_some_and(|n| *n >= 2) {
                            later.push(t);
                            continue;
                        }
                        if taken.insert(t.source_key.clone()) && taken.insert(ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title)) {
                            *per.entry(a).or_default() += 1;
                            songs.push(t);
                        }
                    }
                    let fresh = later.into_iter().chain(fresh);
                    // the rest: songs by artists you've never had
                    new_artists.extend(fresh.filter(|t| t.artists.first().is_some_and(|a| !known.contains(&main_artist(a).to_lowercase()))));
                    if !songs.is_empty() {
                        shelves.push(Shelf { title, sub, songs });
                    }
                }
                Err(e) => err = Some(e),
            }
        }
        let mut artists = HashSet::new();
        let fresh: Vec<ITrack> = new_artists
            .into_iter()
            .filter(|t| artists.insert(t.artists.first().map(|a| main_artist(a).to_lowercase()).unwrap_or_default()) && taken.insert(t.source_key.clone()) && taken.insert(ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title)))
            .take(PER)
            .collect();
        if !fresh.is_empty() {
            shelves.push(Shelf { title: "NEW ARTISTS FOR YOU".into(), sub: "One song each by artists you don't have yet, from all of the above".into(), songs: fresh });
        }
        remember(&shelves.iter().flat_map(|s| s.songs.iter()).collect::<Vec<_>>());
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
                    ui.label(egui::RichText::new("Music that's new to you, across your genres · ▶ listens first · + GET downloads it").font(vt(17.0)).color(pal.dim));
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

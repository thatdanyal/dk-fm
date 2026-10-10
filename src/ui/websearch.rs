//! FIND MUSIC: search YouTube Music like Spotify's search — ALL, SONGS, ARTISTS, ALBUMS,
//! PLAYLISTS, PROFILES, PODCASTS & SHOWS, AUDIOBOOKS — with LOAD MORE, and pages for an artist
//! (songs, albums, singles), an album / playlist / podcast (its songs) and a profile (its
//! playlists). Songs are labelled CLEAN / EXPLICIT / LIVE / INSTRUMENTAL... so you pick the
//! version; nothing downloads until you click + GET, and ▶ plays a preview first.
//! SHAZAM (a button on the deck) names a song playing on the PC or near the microphone; see
//! recognize.rs.
use super::theme::{px, vt};
use super::widgets::{button, fill, frame_rect, tb_button};
use super::App;
use crate::sources::{self, ArtistPage, Hit, HitPage, ITrack, Release};
use eframe::egui::{self, Align2, Pos2, Rect, Sense, Ui, Vec2};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use crate::recognize::Ear;

#[derive(Default, Clone)]
pub enum Listen {
    #[default]
    Idle,
    /// asking what to listen to: this PC or the microphone
    Choose,
    Busy,
    Got(crate::recognize::Heard),
    NoMatch,
    Failed(String),
}

/// Results loaded so far for one search / page.
#[derive(Default, Clone)]
struct Res {
    hits: Vec<Hit>,
    more: Option<String>,
    busy: bool,
    err: Option<String>,
}

/// A page opened from the results (◀ BACK returns).
#[derive(Clone)]
pub enum Open {
    Artist(Hit),
    /// album, playlist or podcast: its songs
    List(Hit),
    Profile(Hit),
}

type Done = (String, Result<HitPage, String>, bool);

#[derive(Default)]
pub struct WebState {
    pub listen: Arc<Mutex<Listen>>,
    /// voice search (the search bar's microphone)
    pub voice: Arc<Mutex<Voice>>,
    /// what SHAZAM listened to last (AGAIN uses it, the chooser suggests it)
    pub ear: Ear,
    pub query: String,
    pub focus: bool,
    edited: Option<Instant>,
    /// the category tab ("" = ALL)
    pub cat: String,
    /// by key: "s|<cat>|<query>" searches, "pl|<playlist>" songs, "dc|<browse>|<params>" albums,
    /// "pf|<browse>" a profile's playlists
    res: HashMap<String, Res>,
    slot: Arc<Mutex<Vec<Done>>>,
    artists: HashMap<String, Option<Result<ArtistPage, String>>>,
    artist_slot: Arc<Mutex<Vec<(String, Result<ArtistPage, String>)>>>,
    pub stack: Vec<Open>,
    /// which results you already have: key -> (lib.gen, per song)
    owned: HashMap<String, (u64, Vec<Option<String>>)>,
    /// LYRICS: your library's saved lyrics (read in the background, when it was read)
    lyrics: Arc<Mutex<Option<(Instant, Arc<Vec<(String, Vec<(f64, String)>)>>)>>>,
    lyrics_busy: Arc<std::sync::atomic::AtomicBool>,
    /// looking up lyrics for the whole library: (done, of)
    pub fill: Arc<Mutex<Option<(usize, usize)>>>,
}

/// SHAZAM was pressed: ask what to listen to (pressed again: never mind).
pub fn listen(app: &mut App, ctx: &egui::Context) {
    let mut l = app.browser.web.listen.lock();
    *l = match *l {
        Listen::Busy => Listen::Busy,
        Listen::Choose => Listen::Idle,
        _ => Listen::Choose,
    };
    // the card was already drawn this frame
    ctx.request_repaint();
}

/// Start listening to `ear` for a song (in the background).
pub fn listen_to(app: &mut App, ctx: &egui::Context, ear: Ear) {
    let slot = app.browser.web.listen.clone();
    if matches!(*slot.lock(), Listen::Busy) {
        return;
    }
    app.browser.web.ear = ear;
    *slot.lock() = Listen::Busy;
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        *slot.lock() = match crate::recognize::identify(ear) {
            Ok(Some(h)) => Listen::Got(h),
            Ok(None) => Listen::NoMatch,
            Err(e) => Listen::Failed(e),
        };
        ctx.request_repaint();
    });
}

/// Voice search, from pressing the microphone to what was heard.
#[derive(Default, Clone)]
pub enum Voice {
    #[default]
    Idle,
    Listening,
    Heard(String),
    Failed(String),
}

/// The search bar's microphone was pressed: listen in the background (Windows does the hearing;
/// DK.FM's thread only waits for its answer).
pub fn voice(app: &mut App, ctx: &egui::Context) {
    let slot = app.browser.web.voice.clone();
    if matches!(*slot.lock(), Voice::Listening) {
        return;
    }
    *slot.lock() = Voice::Listening;
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("voice".into())
        .spawn(move || {
            let r = crate::voice::listen();
            *slot.lock() = match r {
                Ok(t) => Voice::Heard(t),
                Err(e) => Voice::Failed(e),
            };
            ctx.request_repaint();
        })
        .ok();
}

/// The search bar was typed in (`enter` = search now, else after a short pause).
pub fn typed(app: &mut App, enter: bool) {
    if app.browser.view_of(true) != &super::browser::View::Web {
        let q = std::mem::take(&mut app.browser.web.query);
        app.browser.set_view(super::browser::View::Web);
        app.browser.web.query = q;
    }
    let w = &mut app.browser.web;
    w.edited = if enter { None } else { Some(Instant::now()) };
    w.stack.clear();
}

/// Search for `q` in FIND MUSIC (e.g. what SHAZAM heard).
pub fn search_for(app: &mut App, q: String, cat: &str) {
    app.show_panel(super::Tab::Home);
    app.browser.set_view(super::browser::View::Web);
    let w = &mut app.browser.web;
    w.query = q;
    w.edited = None;
    w.cat = cat.into();
    w.stack.clear();
}

/// Load (or load more of) `key`.
fn fetch(app: &mut App, ctx: &egui::Context, key: String, append: bool, job: impl FnOnce(Option<String>) -> Result<HitPage, String> + Send + 'static) {
    let w = &mut app.browser.web;
    let r = w.res.entry(key.clone()).or_default();
    if r.busy {
        return;
    }
    r.busy = true;
    r.err = None;
    let more = if append { r.more.clone() } else { None };
    let (slot, ctx) = (w.slot.clone(), ctx.clone());
    std::thread::spawn(move || {
        let res = job(more);
        if let Ok(p) = &res {
            super::cjk::ensure(&ctx, &p.hits.iter().map(|h| format!("{} {}", h.title, h.sub)).collect::<String>());
        }
        slot.lock().push((key, res, append));
        ctx.request_repaint();
    });
}

fn take_results(app: &mut App) {
    let done: Vec<Done> = app.browser.web.slot.lock().drain(..).collect();
    for (key, res, append) in done {
        let r = app.browser.web.res.entry(key).or_default();
        r.busy = false;
        match res {
            Ok(p) => {
                if !append {
                    r.hits.clear();
                }
                r.hits.extend(p.hits);
                r.more = p.more;
            }
            Err(e) => r.err = Some(e),
        }
    }
    let got: Vec<_> = app.browser.web.artist_slot.lock().drain(..).collect();
    for (k, r) in got {
        app.browser.web.artists.insert(k, Some(r));
    }
    // keep memory small: the last searches only
    if app.browser.web.res.len() > 40 {
        let keep: Vec<String> = app.browser.web.res.iter().filter(|(_, r)| r.busy).map(|(k, _)| k.clone()).collect();
        app.browser.web.res.retain(|k, _| keep.contains(k));
    }
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    take_results(app);
    let cat = if app.browser.web.cat.is_empty() { "all".to_string() } else { app.browser.web.cat.clone() };
    egui::Frame::new().inner_margin(egui::Margin::same(14)).show(ui, |ui| {
        // (the search box is the search bar in HOME's tab strip)
        ui.horizontal_wrapped(|ui| {
            for (k, label) in sources::CATEGORIES {
                if tb_button(ui, &pal, label, cat == k).clicked() && cat != k {
                    app.browser.web.cat = k.to_string();
                    app.browser.web.stack.clear();
                }
            }
        });
        ui.add_space(6.0);
        if let Some(open) = app.browser.web.stack.last().cloned() {
            if tb_button(ui, &pal, "◀ BACK", false).clicked() {
                app.browser.web.stack.pop();
                return;
            }
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("web-page").auto_shrink([false; 2]).show(ui, |ui| match open {
                Open::Artist(h) => artist_page(app, ui, &h),
                Open::List(h) => list_page(app, ui, &h),
                Open::Profile(h) => profile_page(app, ui, &h),
            });
            return;
        }
        let q = app.browser.web.query.trim().to_string();
        if q.chars().count() < 2 {
            intro(app, ui);
            return;
        }
        // a pasted link: what it points to, to listen to and + GET
        if sources::is_link(&q) {
            link_results(app, ui, &q);
            return;
        }
        // search after a short pause in typing, or right away on Enter
        let key = format!("s|{cat}|{q}");
        let idle = app.browser.web.edited.map(|t| t.elapsed() >= Duration::from_millis(700)).unwrap_or(true);
        if !app.browser.web.res.contains_key(&key) {
            if idle {
                let (q2, c2) = (q.clone(), cat.clone());
                fetch(app, ui.ctx(), key.clone(), false, move |more| sources::catalog(&q2, &c2, more.as_deref()));
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            }
        }
        let res = app.browser.web.res.get(&key).cloned().unwrap_or(Res { busy: true, ..Default::default() });
        egui::ScrollArea::vertical().id_salt(("web-results", &key)).auto_shrink([false; 2]).show(ui, |ui| {
            if cat == "lyrics" {
                lyrics_mine(app, ui, &q);
                ui.label(egui::RichText::new("ONLINE").font(px(8.0)).color(app.pal.accent2));
                ui.label(egui::RichText::new("Songs with that line, from YouTube Music: ▶ to listen, + GET to keep").color(app.pal.dim));
            }
            if cat == "all" {
                all_results(app, ui, &res, &key);
            } else {
                results(app, ui, &res, &key);
                let (q2, c2) = (q.clone(), cat.clone());
                load_more(app, ui, &res, &key, move |more| sources::catalog(&q2, &c2, more.as_deref()));
            }
        });
    });
}

/// Words of a lyric line, for matching: lower case, letters and digits only, one space apart.
fn words(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or(c) } else if c == '\'' || c == '’' { '\0' } else { ' ' }).filter(|c| *c != '\0').collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Your songs whose lyrics have `q`: (song id, the line, its time; -1 for plain lyrics). A line
/// you remember can run over two lines of the lyrics.
pub fn lyric_matches(saved: &[(String, Vec<(f64, String)>)], q: &str, max: usize) -> Vec<(String, String, f64)> {
    let q = words(q);
    if q.len() < 4 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (id, lines) in saved {
        let w: Vec<String> = lines.iter().map(|l| words(&l.1)).collect();
        // the line itself first, else one that runs on into the next line
        let hit = (0..lines.len()).find(|&i| w[i].contains(&q)).or_else(|| (0..lines.len().saturating_sub(1)).find(|&i| !w[i].is_empty() && format!("{} {}", w[i], w[i + 1]).contains(&q)));
        if let Some(i) = hit {
            out.push((id.clone(), lines[i].1.clone(), lines[i].0));
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

/// LYRICS: songs in your library whose lyrics have the line (click plays it from there), and
/// looking up lyrics for the songs that have none saved yet.
fn lyrics_mine(app: &mut App, ui: &mut Ui, q: &str) {
    let pal = app.pal;
    // your saved lyrics, re-read now and then (songs you play get theirs saved)
    let fresh = app.browser.web.lyrics.lock().as_ref().map(|(t, _)| t.elapsed() < Duration::from_secs(60)).unwrap_or(false);
    if !fresh && !app.browser.web.lyrics_busy.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let (slot, busy, ctx) = (app.browser.web.lyrics.clone(), app.browser.web.lyrics_busy.clone(), ui.ctx().clone());
        std::thread::spawn(move || {
            let all = crate::lyricsrc::all_saved();
            *slot.lock() = Some((Instant::now(), Arc::new(all)));
            busy.store(false, std::sync::atomic::Ordering::Relaxed);
            ctx.request_repaint();
        });
    }
    let saved = app.browser.web.lyrics.lock().as_ref().map(|(_, v)| v.clone());
    ui.label(egui::RichText::new("IN YOUR LIBRARY").font(px(8.0)).color(pal.accent2));
    let Some(saved) = saved else {
        ui.label(egui::RichText::new("READING YOUR LYRICS…").font(px(6.0)).color(pal.accent));
        return;
    };
    let hits = lyric_matches(&saved, q, 25);
    if hits.is_empty() {
        ui.label(egui::RichText::new("None of your songs' saved lyrics have that line.").color(pal.dim));
    }
    let mut play = None;
    for (id, line, at) in &hits {
        let Some(t) = app.lib.track(id) else { continue };
        let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click());
        if resp.hovered() {
            fill(ui.painter(), r, pal.panel_hi);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let art = Rect::from_min_size(r.min + Vec2::new(4.0, 4.0), Vec2::splat(36.0));
        fill(ui.painter(), art, pal.bg);
        if let Some(tex) = super::preview::cover_tex(app, ui.ctx(), &t, 64) {
            ui.painter().image(tex, art, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), egui::Color32::WHITE);
        }
        let p = ui.painter().with_clip_rect(r);
        p.text(Pos2::new(art.right() + 10.0, r.top() + 13.0), Align2::LEFT_CENTER, format!("{} · {}", t.title, t.artist), vt(19.0), pal.text);
        p.text(Pos2::new(art.right() + 10.0, r.top() + 32.0), Align2::LEFT_CENTER, format!("“{line}”"), vt(18.0), pal.accent2);
        if resp.on_hover_text(if *at >= 0.0 { "Play it from this line" } else { "Play it" }).clicked() {
            play = Some((id.clone(), *at));
        }
    }
    if let Some((id, at)) = play {
        // a little before the line, so it's sung after you hear it come in
        app.player.play_now_at(id, (at - 1.5).max(0.0));
    }
    // songs without saved lyrics yet: look them all up (LRCLIB, one at a time)
    let missing = {
        let have: std::collections::HashSet<&str> = saved.iter().map(|s| s.0.as_str()).collect();
        app.lib.data.read().tracks.keys().filter(|id| !have.contains(id.as_str())).count()
    };
    let fill = *app.browser.web.fill.lock();
    match fill {
        Some((done, of)) => {
            ui.label(egui::RichText::new(format!("LOOKING UP LYRICS… {done} / {of}")).font(px(6.0)).color(pal.accent));
            ui.ctx().request_repaint_after(Duration::from_millis(700));
        }
        None if missing > 0 => {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(format!("{missing} of your songs have no lyrics saved yet (lyrics are saved when a song plays).")).color(pal.dim));
                if button(ui, &pal, "LOOK THEM ALL UP", false, true).on_hover_text("Finds lyrics for every song in the background (a few minutes for a big library), so a line finds any of your songs").clicked() {
                    fill_lyrics(app, ui.ctx());
                }
            });
        }
        None => {}
    }
    ui.add_space(12.0);
}

/// Looks up lyrics for every song without saved ones, in the background, gently (LRCLIB is a
/// free service): about two songs a second.
pub fn fill_lyrics(app: &mut App, ctx: &egui::Context) {
    let tracks: Vec<crate::store::Track> = {
        let have: std::collections::HashSet<String> = crate::lyricsrc::all_saved().into_iter().map(|s| s.0).collect();
        app.lib.data.read().tracks.values().filter(|t| !have.contains(&t.id)).cloned().collect()
    };
    let (fill, slot, ctx) = (app.browser.web.fill.clone(), app.browser.web.lyrics.clone(), ctx.clone());
    *fill.lock() = Some((0, tracks.len()));
    std::thread::Builder::new()
        .name("lyrics-fill".into())
        .spawn(move || {
            let n = tracks.len();
            for (i, t) in tracks.iter().enumerate() {
                crate::lyricsrc::find_light(t);
                *fill.lock() = Some((i + 1, n));
                std::thread::sleep(Duration::from_millis(400));
            }
            *fill.lock() = None;
            *slot.lock() = None; // read them again
            ctx.request_repaint();
        })
        .ok();
}

/// The songs a pasted link points to (a video, a song, a playlist…).
fn link_results(app: &mut App, ui: &mut Ui, url: &str) {
    let key = format!("url|{url}");
    if !app.browser.web.res.contains_key(&key) {
        let (u, creds) = (url.to_string(), app.dl.creds());
        fetch(app, ui.ctx(), key.clone(), false, move |_| sources::link_songs(&u, &creds));
    }
    let res = app.browser.web.res.get(&key).cloned().unwrap_or(Res { busy: true, ..Default::default() });
    ui.label(egui::RichText::new("FROM THE LINK YOU PASTED").font(px(8.0)).color(app.pal.accent2));
    if res.hits.len() > 1 {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} songs. Pick some below, or bring them all in as a playlist (kept in sync):", res.hits.len())).color(app.pal.dim));
            if button(ui, &app.pal, "IMPORT ALL", true, true).clicked() {
                app.browser.set_view(super::browser::View::Import);
                super::import::fetch_link(app, ui.ctx(), url.to_string());
            }
        });
    }
    ui.add_space(4.0);
    egui::ScrollArea::vertical().id_salt(("web-link", url)).auto_shrink([false; 2]).show(ui, |ui| {
        if !status(app, ui, &res) {
            let songs: Vec<Hit> = res.hits.iter().filter(|h| h.track.is_some()).cloned().collect();
            song_rows(app, ui, &songs, &key);
        }
    });
}

/// Before searching: what this tab does, and how DK.FM gets the best sound.
fn intro(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let quality = crate::downloader::quality_label(&app.settings.lock().download_format);
    ui.label(egui::RichText::new("Search all of YouTube Music: pick a category above, open an artist to see their songs and albums, and click ▶ to listen before you + GET anything.").color(pal.dim));
    ui.add_space(6.0);
    ui.label(egui::RichText::new(format!("Sound: every download takes the best audio YouTube has for that song (sound quality: {quality}, change it in Settings > Downloads). Results from YouTube Music's SONGS are the studio versions, the best source; videos can be quieter or have extra sounds.")).color(pal.dim));
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Can't find a cover, remix, instrumental or a small artist? YOUTUBE and SOUNDCLOUD above search everything uploaded there. You can also paste a link to a song or playlist into the search bar.").color(pal.dim));
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Hear a song on your PC or around you? Press SHAZAM on the deck.").color(pal.dim));
}

/// ALL: a few artists, songs (the versions to pick from), albums and playlists.
fn all_results(app: &mut App, ui: &mut Ui, res: &Res, key: &str) {
    if status(app, ui, res) {
        return;
    }
    let n = app.settings.lock().web_results.clamp(1, 20) as usize;
    let sections = [("artist", "ARTISTS", "artists"), ("song", "SONGS", "songs"), ("album", "ALBUMS", "albums"), ("playlist", "PLAYLISTS", "playlists")];
    for (kind, title, cat) in sections {
        let hits: Vec<Hit> = res.hits.iter().filter(|h| h.kind == kind).cloned().collect();
        if hits.is_empty() {
            continue;
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(title).font(px(8.0)).color(app.pal.accent2));
            if tb_button(ui, &app.pal, "SEE ALL ›", false).clicked() {
                app.browser.web.cat = cat.into();
            }
        });
        if kind == "song" {
            ui.label(egui::RichText::new("Pick the version you want: clean, explicit, live, instrumental… (SEE ALL for more)").color(app.pal.dim));
            song_rows(app, ui, &hits.into_iter().take(n).collect::<Vec<_>>(), &format!("{key}|songs"));
        } else {
            hit_rows(app, ui, &hits);
        }
        ui.add_space(10.0);
    }
    // covers, remixes and small artists aren't in YouTube Music's catalogue
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Not here? Covers, remixes, instrumentals and small artists:").color(app.pal.dim));
        if tb_button(ui, &app.pal, "SEARCH YOUTUBE ›", false).clicked() {
            app.browser.web.cat = "youtube".into();
        }
        if tb_button(ui, &app.pal, "SEARCH SOUNDCLOUD ›", false).clicked() {
            app.browser.web.cat = "soundcloud".into();
        }
    });
}

/// One category's results.
fn results(app: &mut App, ui: &mut Ui, res: &Res, key: &str) {
    if status(app, ui, res) {
        return;
    }
    let songs: Vec<Hit> = res.hits.iter().filter(|h| h.track.is_some()).cloned().collect();
    if songs.len() == res.hits.len() {
        song_rows(app, ui, &songs, key);
    } else {
        hit_rows(app, ui, &res.hits);
    }
}

/// SEARCHING… / an error / nothing found; true = nothing to list.
fn status(app: &App, ui: &mut Ui, res: &Res) -> bool {
    let pal = app.pal;
    if res.hits.is_empty() {
        if res.busy {
            let tools = crate::ytdlp::STATUS.lock().state.clone();
            ui.label(egui::RichText::new(if tools == "installing" { "SEARCHING… (getting the download tools first)" } else { "SEARCHING…" }).font(px(7.0)).color(pal.accent));
        } else if let Some(e) = &res.err {
            ui.label(egui::RichText::new(e).color(pal.accent));
        } else {
            ui.label(egui::RichText::new("Nothing found.").color(pal.dim));
        }
        return true;
    }
    false
}

fn load_more(app: &mut App, ui: &mut Ui, res: &Res, key: &str, job: impl FnOnce(Option<String>) -> Result<HitPage, String> + Send + 'static) {
    if res.hits.is_empty() || res.more.is_none() {
        return;
    }
    ui.add_space(6.0);
    if res.busy {
        ui.label(egui::RichText::new("LOADING…").font(px(6.0)).color(app.pal.accent));
    } else if button(ui, &app.pal, "LOAD MORE", false, true).clicked() {
        fetch(app, ui.ctx(), key.to_string(), true, job);
    }
}

/// Songs: cover, ▶ preview, version labels, + GET.
fn song_rows(app: &mut App, ui: &mut Ui, hits: &[Hit], key: &str) {
    let hide_explicit = app.settings.lock().hide_explicit;
    let found: Vec<ITrack> = hits.iter().filter_map(|h| h.track.clone()).collect();
    // labels are worked out on everything found, before any hiding
    let tags: Vec<Vec<&str>> = found.iter().map(|t| sources::version_tags(t, &found)).collect();
    let (found, tags): (Vec<ITrack>, Vec<Vec<&str>>) = found.iter().cloned().zip(tags).filter(|(t, _)| !(hide_explicit && t.explicit)).unzip();
    if found.is_empty() {
        ui.label(egui::RichText::new("Only explicit versions found (Settings > Search hides them).").color(app.pal.dim));
        return;
    }
    let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
    let stale = app.browser.web.owned.get(key).map(|o| o.0 != gen || o.1.len() != found.len()).unwrap_or(true);
    if stale {
        let owned = super::addsongs::owned_ids(app, &found);
        app.browser.web.owned.insert(key.to_string(), (gen, owned));
    }
    let owned = app.browser.web.owned.get(key).map(|o| o.1.clone()).unwrap_or_default();
    let w = ui.available_width();
    ui.spacing_mut().item_spacing.y = 2.0;
    super::addsongs::result_rows_tagged(app, ui, None, &found, &owned, &tags, w);
}

/// Artists, albums, playlists, profiles, podcasts: a row each; click opens it.
fn hit_rows(app: &mut App, ui: &mut Ui, hits: &[Hit]) {
    let pal = app.pal;
    let have = super::home::have_keys(app);
    for h in hits {
        if h.track.is_some() {
            song_rows(app, ui, std::slice::from_ref(h), &format!("one|{}", h.track.as_ref().map(|t| t.source_key.as_str()).unwrap_or("")));
            continue;
        }
        let w = ui.available_width();
        let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 52.0), Sense::click());
        let hovered = ui.rect_contains_pointer(r);
        if hovered {
            fill(ui.painter(), r, pal.panel_hi);
        }
        let tr = Rect::from_min_size(r.min + Vec2::new(4.0, 4.0), Vec2::splat(44.0));
        fill(ui.painter(), tr, pal.bg);
        if let Some(tex) = super::home::remote_tex(app, ui.ctx(), h.thumb.as_ref()) {
            ui.painter().image(tex, tr, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), egui::Color32::WHITE);
        }
        let label = match h.kind.as_str() { "artist" => "ARTIST", "album" => "ALBUM", "playlist" => "PLAYLIST", "profile" => "PROFILE", "podcast" => "PODCAST", _ => "" };
        let clip = ui.painter().with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.right() - 130.0, r.bottom())));
        clip.text(Pos2::new(tr.right() + 10.0, r.top() + 8.0), Align2::LEFT_TOP, &h.title, vt(20.0), pal.text);
        clip.text(Pos2::new(tr.right() + 10.0, r.top() + 29.0), Align2::LEFT_TOP, if h.sub.is_empty() { label.to_string() } else { h.sub.clone() }, vt(17.0), pal.dim);
        // album: + GET downloads it; everything opens on click
        let mut acted = false;
        if h.kind == "album" {
            if let Some(rel) = h.release() {
                let (txt, on) = super::home::get_button(app, &rel, crate::discover::have(&have, &rel));
                let br = Rect::from_min_size(Pos2::new(r.right() - 120.0, r.center().y - 12.0), Vec2::new(112.0, 24.0));
                let bresp = ui.interact(br, ui.id().with(("hit-get", &rel.playlist)), Sense::click());
                frame_rect(ui.painter(), br, 2.0, if on { pal.accent } else { pal.line_hi });
                if bresp.hovered() && on {
                    fill(ui.painter(), br.shrink(2.0), pal.accent);
                }
                ui.painter().text(br.center(), Align2::CENTER_CENTER, &txt, px(6.0), if bresp.hovered() && on { pal.ink } else if on { pal.accent2 } else { pal.dim });
                if on && bresp.on_hover_text("Download the whole album (songs you have are skipped)").clicked() {
                    super::home::get_release(app, &rel);
                    acted = true;
                }
            }
        } else {
            ui.painter().text(Pos2::new(r.right() - 12.0, r.center().y), Align2::RIGHT_CENTER, "OPEN ›", px(6.0), if hovered { pal.accent } else { pal.dim });
        }
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && !acted {
            let open = match h.kind.as_str() {
                "artist" => Some(Open::Artist(h.clone())),
                "profile" => Some(Open::Profile(h.clone())),
                "album" | "playlist" | "podcast" if h.playlist.is_some() => Some(Open::List(h.clone())),
                _ => None,
            };
            if let Some(o) = open {
                app.browser.web.stack.push(o);
            }
        }
    }
}

fn header(app: &mut App, ui: &mut Ui, h: &Hit, kind: &str) {
    let pal = app.pal;
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(96.0), Sense::hover());
        fill(ui.painter(), r, pal.bg);
        if let Some(tex) = super::home::remote_tex(app, ui.ctx(), h.thumb.as_ref()) {
            ui.painter().image(tex, r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), egui::Color32::WHITE);
        }
        frame_rect(ui.painter(), r, 2.0, pal.line_hi);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(kind).font(px(6.0)).color(pal.accent2));
            ui.label(egui::RichText::new(&h.title).font(px(12.0)).color(pal.text));
            ui.label(egui::RichText::new(&h.sub).color(pal.dim));
        });
    });
    ui.add_space(8.0);
}

/// An album, playlist or podcast: its songs (LOAD MORE), + GET / IMPORT for all of them.
fn list_page(app: &mut App, ui: &mut Ui, h: &Hit) {
    let pal = app.pal;
    let Some(pl) = h.playlist.clone() else { return };
    header(app, ui, h, match h.kind.as_str() { "album" => "ALBUM", "podcast" => "PODCAST", _ => "PLAYLIST" });
    ui.horizontal(|ui| match h.kind.as_str() {
        "album" => {
            if let Some(rel) = h.release() {
                let have = super::home::have_keys(app);
                let (txt, on) = super::home::get_button(app, &rel, crate::discover::have(&have, &rel));
                if button(ui, &pal, &txt, true, on).on_hover_text("Download the whole album into your library (songs you have are skipped)").clicked() {
                    super::home::get_release(app, &rel);
                }
            }
        }
        _ => {
            if button(ui, &pal, "IMPORT AS PLAYLIST", true, true).on_hover_text("Pick its songs and download them as a playlist in your library").clicked() {
                app.browser.set_view(super::browser::View::Import);
                super::import::fetch_link(app, ui.ctx(), format!("https://music.youtube.com/playlist?list={pl}"));
            }
        }
    });
    ui.add_space(6.0);
    let key = format!("pl|{pl}");
    if !app.browser.web.res.contains_key(&key) {
        let p2 = pl.clone();
        fetch(app, ui.ctx(), key.clone(), false, move |more| sources::playlist_songs(&p2, more.as_deref()));
    }
    let res = app.browser.web.res.get(&key).cloned().unwrap_or_default();
    if !status(app, ui, &res) {
        song_rows(app, ui, &res.hits, &key);
    }
    let p2 = pl.clone();
    load_more(app, ui, &res, &key, move |more| sources::playlist_songs(&p2, more.as_deref()));
}

fn release_hit(r: &Release) -> Hit {
    Hit { kind: "album".into(), title: r.title.clone(), sub: [Some(r.kind.clone()), Some(r.artist.clone()).filter(|a| !a.is_empty()), r.year.map(|y| y.to_string())].into_iter().flatten().collect::<Vec<_>>().join(" • "), thumb: r.cover.clone(), browse: None, playlist: Some(r.playlist.clone()), track: None }
}

/// An artist: top songs (MORE SONGS lists all of them), albums, singles & EPs.
fn artist_page(app: &mut App, ui: &mut Ui, h: &Hit) {
    let pal = app.pal;
    let Some(browse) = h.browse.clone() else { return };
    header(app, ui, h, "ARTIST");
    if !app.browser.web.artists.contains_key(&browse) {
        app.browser.web.artists.insert(browse.clone(), None);
        let (slot, ctx, b) = (app.browser.web.artist_slot.clone(), ui.ctx().clone(), browse.clone());
        std::thread::spawn(move || {
            let r = sources::artist_page(&b);
            slot.lock().push((b, r));
            ctx.request_repaint();
        });
    }
    let page = match app.browser.web.artists.get(&browse) {
        Some(Some(Ok(p))) => p.clone(),
        Some(Some(Err(e))) => {
            ui.label(egui::RichText::new(e).color(pal.accent));
            return;
        }
        _ => {
            ui.label(egui::RichText::new("LOADING…").font(px(7.0)).color(pal.accent));
            return;
        }
    };
    // songs: the top 5, or all of them once asked
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("SONGS").font(px(8.0)).color(pal.accent2));
    });
    let all_key = page.songs_all.as_ref().map(|p| format!("pl|{p}"));
    match all_key.as_ref().and_then(|k| app.browser.web.res.get(k).cloned()) {
        Some(res) => {
            let k = all_key.clone().unwrap();
            if !status(app, ui, &res) {
                song_rows(app, ui, &res.hits, &k);
            }
            let p2 = page.songs_all.clone().unwrap();
            load_more(app, ui, &res, &k, move |more| sources::playlist_songs(&p2, more.as_deref()));
        }
        None => {
            let top: Vec<Hit> = page.songs.iter().cloned().map(|t| Hit { kind: "song".into(), title: t.title.clone(), sub: t.artists.join(", "), thumb: t.cover.clone(), track: Some(t), ..Default::default() }).collect();
            song_rows(app, ui, &top, &format!("top|{browse}"));
            if let Some(p) = page.songs_all.clone() {
                if button(ui, &pal, "MORE SONGS", false, true).clicked() {
                    let k = format!("pl|{p}");
                    fetch(app, ui.ctx(), k, false, move |more| sources::playlist_songs(&p, more.as_deref()));
                }
            }
        }
    }
    ui.add_space(10.0);
    // albums, then singles & EPs (ALL lists the rest)
    for (label, kinds) in [("Albums", &["Album"][..]), ("Singles & EPs", &["Single", "EP"][..])] {
        let more = page.more.iter().find(|m| m.0 == label).cloned();
        let dkey = more.as_ref().map(|(_, b, p)| format!("dc|{b}|{p}"));
        let full = dkey.as_ref().and_then(|k| app.browser.web.res.get(k).cloned());
        let rels: Vec<Hit> = match &full {
            Some(r) if !r.hits.is_empty() => r.hits.clone(),
            _ => page.releases.iter().filter(|r| kinds.contains(&r.kind.as_str())).map(release_hit).collect(),
        };
        if rels.is_empty() {
            continue;
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(label.to_uppercase()).font(px(8.0)).color(pal.accent2));
            if let (Some((_, b, p)), None) = (&more, &full) {
                if tb_button(ui, &pal, "SEE ALL ›", false).clicked() {
                    let (artist, b, p) = (h.title.clone(), b.clone(), p.clone());
                    fetch(app, ui.ctx(), dkey.clone().unwrap(), false, move |_| sources::discography(&artist, &b, &p).map(|v| HitPage { hits: v.iter().map(release_hit).collect(), more: None }));
                }
            }
            if full.as_ref().is_some_and(|r| r.busy) {
                ui.label(egui::RichText::new("LOADING…").font(px(6.0)).color(pal.accent));
            }
        });
        hit_rows(app, ui, &rels);
        ui.add_space(10.0);
    }
}

/// A profile: its public playlists.
fn profile_page(app: &mut App, ui: &mut Ui, h: &Hit) {
    let Some(browse) = h.browse.clone() else { return };
    header(app, ui, h, "PROFILE");
    let key = format!("pf|{browse}");
    if !app.browser.web.res.contains_key(&key) {
        fetch(app, ui.ctx(), key.clone(), false, move |_| sources::profile_playlists(&browse).map(|hits| HitPage { hits, more: None }));
    }
    let res = app.browser.web.res.get(&key).cloned().unwrap_or_default();
    if !status(app, ui, &res) {
        hit_rows(app, ui, &res.hits);
    }
}

/// SHAZAM's answer (or that it's listening), floating above everything near the bottom.
pub fn shazam_card(app: &mut App, ctx: &egui::Context) {
    let pal = app.pal;
    let heard = app.browser.web.listen.lock().clone();
    if matches!(heard, Listen::Idle) {
        return;
    }
    let mut close = false;
    let ear = app.browser.web.ear;
    let mut go: Option<Ear> = None;
    let other = |e: Ear| if e == Ear::Pc { (Ear::Mic, "USE THE MIC") } else { (Ear::Pc, "USE THIS PC") };
    egui::Area::new(egui::Id::new("shazam-card")).anchor(Align2::CENTER_BOTTOM, [0.0, -70.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::new().fill(pal.panel).stroke(egui::Stroke::new(2.0_f32, pal.accent)).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_max_width(520.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("SHAZAM").font(px(8.0)).color(pal.accent2));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if tb_button(ui, &pal, "×", false).on_hover_text("Close").clicked() {
                        close = true;
                    }
                });
            });
            match &heard {
                Listen::Choose => {
                    ui.label(egui::RichText::new("What should SHAZAM listen to?").color(pal.text));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if button(ui, &pal, "THIS PC", ear == Ear::Pc, true).on_hover_text("A song playing on this PC: in a browser, a game, a video…").clicked() {
                            go = Some(Ear::Pc);
                        }
                        if button(ui, &pal, "MICROPHONE", ear == Ear::Mic, true).on_hover_text("A song playing around you: a radio, a shop, a party, another phone…").clicked() {
                            go = Some(Ear::Mic);
                        }
                    });
                }
                Listen::Busy => {
                    let msg = match ear {
                        Ear::Pc => "Listening to this PC for about 10 seconds: keep the song playing…",
                        Ear::Mic => "Listening through the microphone for about 10 seconds: hold it near the music…",
                    };
                    ui.label(egui::RichText::new(msg).color(pal.text));
                    ctx.request_repaint_after(Duration::from_millis(400));
                }
                Listen::Got(h) => {
                    ui.label(egui::RichText::new(&h.title).font(vt(24.0)).color(pal.text));
                    ui.label(egui::RichText::new(if h.album.is_empty() { h.artist.clone() } else { format!("{} · {}", h.artist, h.album) }).color(pal.dim));
                    ui.add_space(4.0);
                    let t = ITrack { title: h.title.clone(), artists: vec![h.artist.clone()], source_key: format!("shazam:{}:{}", h.artist, h.title), ..Default::default() };
                    let (current, loading) = (super::preview::current(app, &t), super::preview::loading(app, &t));
                    let playing = current && app.player.status().playing;
                    if loading {
                        ctx.request_repaint_after(Duration::from_millis(300));
                    }
                    ui.horizontal(|ui| {
                        let label = if loading { "GETTING IT…" } else if playing { "⏸ PAUSE" } else if current { "▶ RESUME" } else { "▶ PLAY FULL SONG" };
                        if button(ui, &pal, label, !current, !loading).on_hover_text("Listen to the whole song now, without saving it. Like it? KEEP it. If you don't, it's deleted when you play something else").clicked() {
                            super::preview::toggle(app, &t);
                        }
                        if button(ui, &pal, "FIND IT", false, true).on_hover_text("Its versions in FIND MUSIC: listen, then + GET the one you want").clicked() {
                            search_for(app, format!("{} {}", h.title, h.artist), "songs");
                            close = true;
                        }
                        if button(ui, &pal, "AGAIN", false, true).clicked() {
                            go = Some(ear);
                        }
                    });
                    if current {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let kept = super::preview::kept(app, &t);
                            if button(ui, &pal, if kept { "✔ KEEPING" } else { "⬇ KEEP" }, !kept, !kept).on_hover_text("Download it: you pick where it goes (library, Liked or a playlist)").clicked() {
                                super::preview::keep(app, ctx, &t);
                            }
                            if button(ui, &pal, "× DISCARD", false, true).on_hover_text("Not for you: stop it and delete it").clicked() {
                                super::preview::discard(app, &t);
                            }
                            ui.label(egui::RichText::new(if kept { "Downloading it to your library" } else { "Not saved yet" }).color(pal.dim));
                        });
                    }
                }
                Listen::NoMatch => {
                    ui.label(egui::RichText::new("Couldn't name that one. Try again during a clear part of the song.").color(pal.text));
                    ui.horizontal(|ui| {
                        if button(ui, &pal, "TRY AGAIN", true, true).clicked() {
                            go = Some(ear);
                        }
                        let (e, t) = other(ear);
                        if button(ui, &pal, t, false, true).clicked() {
                            go = Some(e);
                        }
                    });
                }
                Listen::Failed(e) => {
                    ui.label(egui::RichText::new(e).color(pal.accent));
                    ui.horizontal(|ui| {
                        if button(ui, &pal, "TRY AGAIN", false, true).clicked() {
                            go = Some(ear);
                        }
                        let (e, t) = other(ear);
                        if button(ui, &pal, t, false, true).clicked() {
                            go = Some(e);
                        }
                    });
                }
                Listen::Idle => {}
            }
        });
    });
    if close {
        *app.browser.web.listen.lock() = Listen::Idle;
    }
    if let Some(e) = go {
        *app.browser.web.listen.lock() = Listen::Idle;
        listen_to(app, ctx, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_remembered_line() {
        let saved = vec![
            ("a".to_string(), vec![(1.0, "We're no strangers to love".to_string()), (4.0, "You know the rules, and so do I".to_string())]),
            ("b".to_string(), vec![(-1.0, "Is this the real life?".to_string()), (-1.0, "Is this just fantasy?".to_string())]),
        ];
        // punctuation, case and apostrophes don't matter
        assert_eq!(lyric_matches(&saved, "were no strangers", 5), vec![("a".to_string(), "We're no strangers to love".to_string(), 1.0)]);
        // a line you remember can run over two lines of the lyrics
        assert_eq!(lyric_matches(&saved, "real life is this just", 5)[0].0, "b");
        assert_eq!(lyric_matches(&saved, "to love you know the rules", 5)[0].2, 1.0);
        // a phrase that's all on one line finds that line, not the one before it
        assert_eq!(lyric_matches(&saved, "you know the rules", 5)[0].2, 4.0);
        assert!(lyric_matches(&saved, "nothing like this here", 5).is_empty());
        assert!(lyric_matches(&saved, "is", 5).is_empty()); // too short to mean anything
    }
}

//! Home: jump back in, mixes made from your library, new releases from your artists, recently
//! played and this month's top songs. The local sections are rebuilt only when the library or the
//! listening history changes; new releases come from a daily background check (discover.rs).
//! Also artist pages: your top songs by them, their albums you have, and more of theirs online.
use super::browser::{album_key, artist_key, View};
use super::theme::{px, vt, Pal};
use super::widgets::{button, fill, fmt_time, frame_rect};
use super::App;
use crate::discover::{self, Mix, Releases, DAY};
use crate::downloader::TStatus;
use crate::library::{main_artist, ta_key};
use crate::sources::{self, ArtistPage, ITrack, Release};
use crate::store::{now_ms, now_secs, Playlist};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, TextureId, Ui, Vec2};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

enum Jump {
    Playlist(Playlist),
    Album { key: String, title: String, artist: String, cover: Option<String> },
}

type Slot<T> = Arc<Mutex<Option<T>>>;

#[derive(Default)]
pub struct HomeState {
    /// (lib.gen, history length, genres looked up) the local sections were built from
    key: (u64, usize, u64),
    /// artists' genres (discover::Genres), filled in the background
    pub genres: Arc<Mutex<discover::Genres>>,
    genres_gen: Arc<std::sync::atomic::AtomicU64>,
    genres_started: bool,
    /// a mix's online songs (mix id -> songs), fetched once per run
    mix_online: Arc<Mutex<HashMap<String, Result<Vec<ITrack>, String>>>>,
    mix_started: HashSet<String>,
    jump: Vec<Jump>,
    recent: Vec<String>,
    top: Vec<(String, u32)>,
    pub mixes: Vec<Mix>,
    /// your top artists, for new releases: (key, name)
    artists: Vec<(String, String)>,
    rel: Arc<Mutex<RelState>>,
    /// (lib.gen, keys of every song and album you have)
    have: Option<(u64, Arc<HashSet<String>>)>,
    /// releases you asked for: playlist -> download job (None while its song list is fetched)
    gets: Arc<Mutex<HashMap<String, Result<Option<String>, String>>>>,
    /// remote covers: downloading, and ready on disk
    fetching: HashSet<String>,
    ready: Arc<Mutex<HashSet<String>>>,
    page: Option<ArtistCache>,
    online: HashMap<String, Online>,
    /// scroll Home / an artist page to here once (screenshots: DKFM_SCROLL)
    pub scroll: Option<f32>,
}

#[derive(Default)]
struct RelState {
    cache: Option<Releases>,
    busy: bool,
    err: Option<String>,
    /// after a failed check (offline), wait before the next one
    next_try: Option<Instant>,
}

// ------------------------------------------------------------------------------- mixes, online

/// Starts fetching the mixes' online songs (each once per run, one after another).
fn mixes_online(app: &mut App) {
    let todo: Vec<(String, discover::Seed)> = app.home.mixes.iter().filter(|m| m.seed != discover::Seed::None && !app.home.mix_started.contains(&m.id)).map(|m| (m.id.clone(), m.seed.clone())).collect();
    if todo.is_empty() {
        return;
    }
    app.home.mix_started.extend(todo.iter().map(|t| t.0.clone()));
    let (slot, ctx) = (app.home.mix_online.clone(), app.covers.ctx.clone());
    std::thread::spawn(move || {
        for (id, seed) in todo {
            let r = discover::mix_online(&seed);
            slot.lock().insert(id, r);
            if let Some(c) = &ctx {
                c.request_repaint();
            }
        }
    });
}

/// How many online songs a mix adds (on top of yours).
const MIX_ONLINE: usize = 12;

/// A mix's online songs you don't have yet (None: still being fetched).
pub fn mix_online(app: &App, id: &str) -> Option<Result<Vec<ITrack>, String>> {
    let r = app.home.mix_online.lock().get(id).cloned()?;
    Some(r.map(|v| {
        let owned = super::addsongs::owned_ids(app, &v);
        let mut seen = HashSet::new();
        v.into_iter().zip(owned).filter(|(t, o)| o.is_none() && seen.insert(ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title))).map(|(t, _)| t).take(MIX_ONLINE).collect()
    }))
}

/// Plays a mix: your songs, with its online songs woven in (one after every two of yours). The
/// online ones are fetched as their turn comes and aren't saved unless you KEEP them.
pub fn play_mix(app: &mut App, id: &str, shuffle: bool) {
    let Some(m) = app.home.mixes.iter().find(|m| m.id == id).cloned() else { return };
    let mut mine: Vec<String> = m.ids.clone();
    let mut online = mix_online(app, id).and_then(|r| r.ok()).unwrap_or_default();
    if shuffle {
        fastrand::shuffle(&mut mine);
        fastrand::shuffle(&mut online);
    }
    let theirs = super::preview::queue_online(app, &online);
    let (mine, theirs): (Vec<&String>, Vec<&String>) = (mine.iter().collect(), theirs.iter().collect());
    let q = discover::weave(&mine, &theirs);
    if !q.is_empty() {
        app.player.play_list(q, 0, Some(false));
    }
}

const SEC_ROW: f32 = 26.0;
const SEC_HEAD: f32 = 70.0;

/// Height of a mix's online section under its songs.
pub fn mix_section_height(app: &App, id: &str) -> f32 {
    if app.home.mixes.iter().any(|m| m.id == id && m.seed == discover::Seed::None) {
        return 0.0;
    }
    let n = mix_online(app, id).and_then(|r| r.ok()).map(|v| v.len()).unwrap_or(0);
    SEC_HEAD + n.max(1) as f32 * SEC_ROW + 24.0
}

/// Under a mix's songs: the online songs it adds, with ▶ (listen) and + GET.
pub fn mix_section(app: &mut App, ui: &mut Ui, id: &str) {
    let pal = app.pal;
    mixes_online(app);
    let Some(m) = app.home.mixes.iter().find(|m| m.id == id).cloned() else { return };
    if m.seed == discover::Seed::None {
        return;
    }
    egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 18, bottom: 8 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.painter().hline(ui.max_rect().x_range(), ui.max_rect().top() - 8.0, egui::Stroke::new(2.0_f32, pal.line));
        let sub = match &m.seed {
            discover::Seed::Artist(a) => format!("{a}'s top songs you don't have yet"),
            _ => "Songs like your favourites here, that you don't have yet".to_string(),
        };
        ui.label(egui::RichText::new("ALSO IN THIS MIX").font(px(9.0)).color(pal.text));
        ui.label(egui::RichText::new(format!("{sub} · they play in the mix without being saved · + GET keeps one")).font(vt(17.0)).color(pal.dim));
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        match mix_online(app, id) {
            None => {
                ui.label(egui::RichText::new("  FINDING SONGS…").font(px(6.0)).color(pal.accent));
                ui.ctx().request_repaint_after(Duration::from_millis(500));
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(format!("  {e}")).font(vt(18.0)).color(pal.accent));
            }
            Some(Ok(v)) if v.is_empty() => {
                ui.label(egui::RichText::new("  You have them all already").font(vt(18.0)).color(pal.dim));
            }
            Some(Ok(v)) => {
                let w = ui.available_width();
                let owned = vec![None; v.len()];
                super::addsongs::result_rows(app, ui, None, &v, &owned, w);
            }
        }
    });
}

/// An artist page's local part (rebuilt when the library changes).
struct ArtistCache {
    key: String,
    gen: u64,
    name: String,
    /// their songs, most played first
    ids: Vec<String>,
    /// (album key, title, cover, year, songs)
    albums: Vec<(String, String, Option<String>, Option<u32>, usize)>,
}

/// "More from this artist" (online, fetched once per session).
#[derive(Default)]
struct Online {
    slot: Slot<Result<ArtistPage, String>>,
    /// songs and releases you didn't have when it arrived
    shown: Option<Result<(Vec<ITrack>, Vec<Release>), String>>,
    owned: Option<(u64, Vec<Option<String>>)>,
}

enum Act {
    Open(View),
    Play(Vec<String>, usize, bool),
    PlayMix(String),
    PlayPlaylist(String),
    PlayAlbum(String),
    Get(Release),
    Web(String),
}

// ------------------------------------------------------------------------------- widgets

/// Cards per row and their width for a shelf `w` wide.
fn columns(w: f32) -> (usize, f32) {
    let n = ((w + 14.0) / 164.0).floor().max(1.0) as usize;
    (n, (w - 14.0 * (n as f32 - 1.0)) / n as f32)
}

/// A cover card: art, two lines of text, ▶ on hover (when `play`), and an optional button under it
/// (label, clickable). Returns (clicked, ▶ clicked, button clicked, response).
fn card(ui: &mut Ui, pal: &Pal, cw: f32, title: &str, sub: &str, tex: Option<TextureId>, icon: &str, play: bool, btn: Option<(&str, bool)>) -> (bool, bool, bool, egui::Response) {
    let h = cw + 46.0 + if btn.is_some() { 26.0 } else { 0.0 };
    let (r, resp) = ui.allocate_exact_size(Vec2::new(cw, h), Sense::click());
    let art = Rect::from_min_size(r.min, Vec2::splat(cw));
    let pos = ui.input(|i| i.pointer.hover_pos());
    let over = |x: Rect| resp.hovered() && pos.map(|p| x.contains(p)).unwrap_or(false);
    let pb = Rect::from_min_size(art.right_bottom() - Vec2::new(44.0, 44.0), Vec2::splat(36.0));
    let bb = Rect::from_min_size(Pos2::new(r.left(), art.bottom() + 46.0), Vec2::new(cw.min(120.0), 22.0));
    if ui.is_rect_visible(r) {
        let p = ui.painter();
        fill(p, art.translate(Vec2::splat(4.0)), pal.shadow);
        fill(p, art, pal.bg);
        match tex {
            Some(t) => {
                p.image(t, art, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.text(art.center(), Align2::CENTER_CENTER, icon, vt(cw * 0.32), pal.faint);
            }
        }
        frame_rect(p, art, 2.0, if resp.hovered() { pal.accent } else { pal.line_hi });
        if play && resp.hovered() {
            fill(p, pb.translate(Vec2::splat(3.0)), pal.shadow);
            fill(p, pb, if over(pb) { pal.accent2 } else { pal.accent });
            p.text(pb.center(), Align2::CENTER_CENTER, "▶", vt(26.0), pal.ink);
        }
        let clip = p.with_clip_rect(r);
        clip.text(Pos2::new(r.left(), art.bottom() + 8.0), Align2::LEFT_TOP, title, vt(19.0), pal.text);
        clip.text(Pos2::new(r.left(), art.bottom() + 26.0), Align2::LEFT_TOP, sub, vt(16.0), pal.dim);
        if let Some((label, on)) = btn {
            if on {
                fill(p, bb, if over(bb) { pal.accent } else { pal.bg });
                frame_rect(p, bb, 2.0, pal.accent);
            }
            p.text(bb.center(), Align2::CENTER_CENTER, label, px(6.0), if on && over(bb) { pal.ink } else if on { pal.accent2 } else { pal.dim });
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let clicked = resp.clicked();
    let (on_play, on_btn) = (clicked && play && over(pb), clicked && btn.map(|b| b.1).unwrap_or(false) && over(bb));
    (clicked && !on_play && !on_btn, on_play, on_btn, resp)
}

fn shelf(ui: &mut Ui, pal: &Pal, title: &str, sub: &str) {
    ui.add_space(14.0);
    ui.label(egui::RichText::new(title).font(px(9.0)).color(pal.text));
    if !sub.is_empty() {
        ui.label(egui::RichText::new(sub).font(vt(17.0)).color(pal.dim));
    }
    ui.add_space(8.0);
}

/// Lays out `n` cards in rows of as many as fit (at most `rows` rows).
fn cards(ui: &mut Ui, n: usize, rows: usize, mut each: impl FnMut(&mut Ui, usize, f32)) {
    let (per, cw) = columns(ui.available_width());
    for row in 0..rows.min(n.div_ceil(per)) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            for i in row * per..((row + 1) * per).min(n) {
                each(ui, i, cw);
            }
        });
        ui.add_space(12.0);
    }
}

/// A cover file of the library (`cover`) as a texture.
fn tex(app: &mut App, ctx: &egui::Context, cover: Option<&String>, size: u32) -> Option<TextureId> {
    let c = cover?;
    app.covers.get(ctx, app.lib.cover_path(c), c, size)
}

/// A cover from the web (kept in the discover folder).
pub(super) fn remote_tex(app: &mut App, ctx: &egui::Context, url: Option<&String>) -> Option<TextureId> {
    let url = url?;
    let name = crate::library::remote_cover_name(url);
    let path = discover::covers_dir().join(&name);
    if app.home.ready.lock().contains(&name) {
        return app.covers.get(ctx, path, &name, 192);
    }
    if app.home.fetching.insert(url.clone()) {
        if path.exists() {
            app.home.ready.lock().insert(name);
        } else {
            let (ready, url, ctx) = (app.home.ready.clone(), url.clone(), ctx.clone());
            std::thread::spawn(move || {
                let _ = std::fs::create_dir_all(discover::covers_dir());
                let tmp = path.with_extension("part");
                if crate::net::get_bytes(&url).ok().filter(|b| !b.is_empty()).map(discover::square).map(|b| std::fs::write(&tmp, b).is_ok() && std::fs::rename(&tmp, &path).is_ok()).unwrap_or(false) {
                    ready.lock().insert(name);
                    ctx.request_repaint();
                }
            });
        }
    }
    None
}

fn ago(secs: i64) -> String {
    match secs / DAY {
        0 => "today".into(),
        1 => "yesterday".into(),
        d if d < 14 => format!("{d} days ago"),
        d => format!("{} weeks ago", d / 7),
    }
}

// ------------------------------------------------------------------------------- Home

fn rebuild(app: &mut App) {
    start_genres(app);
    let key = (app.lib.gen.load(std::sync::atomic::Ordering::Relaxed), app.lib.history.read().events.len(), app.home.genres_gen.load(std::sync::atomic::Ordering::Relaxed));
    if app.home.key == key {
        return;
    }
    app.home.key = key;
    rebuild_now(app);
}

/// Looks up your artists' genres in the background (once per run, only those not known yet).
pub fn start_genres(app: &mut App) {
    if !app.home.genres_started {
        app.home.genres_started = true;
        *app.home.genres.lock() = discover::Genres::load();
        let artists = discover::top_artists(&app.lib.data.read(), 80);
        if !app.home.genres.lock().missing(&artists, now_secs()).is_empty() {
            let (g, gen, ctx) = (app.home.genres.clone(), app.home.genres_gen.clone(), app.covers.ctx.clone());
            std::thread::spawn(move || {
                discover::fill_genres(&artists, |got| {
                    *g.lock() = got.clone();
                    gen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if let Some(c) = &ctx {
                        c.request_repaint();
                    }
                })
            });
        }
    }
}

fn rebuild_now(app: &mut App) {
    let n_artists = app.settings.lock().release_artists.clamp(1, 30) as usize;
    let d = app.lib.data.read();
    let h = app.lib.history.read();
    // jump back in: playlists you played, and albums you listened to (2+ of their songs in a row)
    let mut jump: Vec<(f64, Jump)> = d.playlists.iter().filter(|p| p.last_played > 0.0).map(|p| (p.last_played / 1000.0, Jump::Playlist(p.clone()))).collect();
    let mut seen = HashSet::new();
    let mut prev: Option<String> = None;
    for e in h.events.iter().rev().take(5000) {
        let k = d.tracks.get(&e.0).filter(|t| !t.album.is_empty()).map(|t| (album_key(t), t));
        if let (Some((k, t)), Some(p)) = (&k, &prev) {
            if k == p && seen.insert(k.clone()) {
                let artist = if t.album_artist.is_empty() { t.artist.clone() } else { t.album_artist.clone() };
                jump.push((e.1 as f64, Jump::Album { key: k.clone(), title: t.album.clone(), artist, cover: t.thumb.clone().or(t.cover.clone()) }));
            }
        }
        prev = k.map(|x| x.0);
    }
    jump.sort_by(|a, b| b.0.total_cmp(&a.0));
    // recently played songs, and this month's most played
    let mut seen = HashSet::new();
    let recent: Vec<String> = h.events.iter().rev().filter(|e| d.tracks.contains_key(&e.0) && seen.insert(e.0.as_str())).take(16).map(|e| e.0.clone()).collect();
    let from = now_secs() - 30 * DAY;
    let mut count: HashMap<&str, u32> = HashMap::new();
    for e in h.events.iter().rev().take_while(|e| e.1 >= from).filter(|e| e.3 == 0 && d.tracks.contains_key(&e.0)) {
        *count.entry(e.0.as_str()).or_default() += 1;
    }
    let mut top: Vec<(String, u32)> = count.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(10);
    let mixes = discover::mixes(&d, &app.home.genres.lock(), now_ms());
    let artists = discover::top_artists(&d, n_artists);
    drop((d, h));
    let s = &mut app.home;
    (s.jump, s.recent, s.top, s.mixes, s.artists) = (jump.into_iter().take(12).map(|j| j.1).collect(), recent, top, mixes, artists);
}

/// Keys of everything you have (songs and albums), for "you don't have it yet".
pub(super) fn have_keys(app: &mut App) -> Arc<HashSet<String>> {
    let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
    if app.home.have.as_ref().map(|h| h.0 != gen).unwrap_or(true) {
        let mut keys: HashSet<String> = app.lib.key_index().into_keys().collect();
        keys.extend(app.lib.data.read().tracks.values().filter(|t| !t.album.is_empty()).map(|t| ta_key(if t.album_artist.is_empty() { &t.artist } else { &t.album_artist }, &t.album)));
        app.home.have = Some((gen, Arc::new(keys)));
    }
    app.home.have.as_ref().unwrap().1.clone()
}

/// New releases of your top artists (starts the daily check when it's due).
fn releases(app: &mut App, ctx: &egui::Context) -> (Vec<(Release, i64)>, bool, Option<String>) {
    let artists = app.home.artists.clone();
    let now = now_secs();
    {
        let mut st = app.home.rel.lock();
        let due = st.cache.as_ref().map(|c| c.due(&artists, now)).unwrap_or(true);
        if due && !st.busy && !artists.is_empty() && st.next_try.map(|t| Instant::now() >= t).unwrap_or(true) {
            st.busy = true;
            let (rel, ctx, artists) = (app.home.rel.clone(), ctx.clone(), artists.clone());
            let first = st.cache.is_none();
            std::thread::Builder::new().name("releases".into()).spawn(move || {
                if first {
                    rel.lock().cache = Some(Releases::load()); // what's saved shows at once
                    ctx.request_repaint();
                }
                let r2 = rel.clone();
                let c2 = ctx.clone();
                let (c, err) = discover::check_online(&artists, move |c| {
                    r2.lock().cache = Some(c.clone()); // show each artist's as it comes in
                    c2.request_repaint();
                });
                let mut s = rel.lock();
                s.cache = Some(c);
                s.busy = false;
                s.next_try = err.as_ref().map(|_| Instant::now() + Duration::from_secs(30 * 60));
                s.err = err;
                ctx.request_repaint();
            }).ok();
        }
    }
    let gets: HashSet<String> = app.home.gets.lock().keys().cloned().collect();
    let have = have_keys(app);
    let st = app.home.rel.lock();
    // (ones you're getting stay, even once you have them)
    let list = st.cache.as_ref().map(|c| c.fresh(&artists, now, |r| discover::have(&have, r) && !gets.contains(&r.playlist))).unwrap_or_default();
    (list, st.busy, st.err.clone())
}

/// Download a release like an import: the songs you don't have (an album or EP also becomes a
/// playlist; a single's songs just join your library).
pub(super) fn get_release(app: &mut App, r: &Release) {
    let kind = if r.kind == "Single" { "track" } else { "album" };
    let Some(pick) = super::dest::folder_for(app, None, &app.dl.auto_folder(kind, &r.title)) else { return };
    let (dl, lib, gets) = (app.dl.clone(), app.lib.clone(), app.home.gets.clone());
    gets.lock().insert(r.playlist.clone(), Ok(None));
    let (url, title, pl) = (r.url(), r.title.clone(), r.playlist.clone());
    app.toast(format!("Getting \"{title}\"…"));
    std::thread::spawn(move || {
        let r = sources::generic(&url, Some(title), Some(kind), 300).map(|col| {
            let keys: HashSet<String> = lib.key_index().into_keys().collect();
            let sel: Vec<usize> = col.tracks.iter().enumerate().filter(|(_, t)| !super::import::owned(&keys, t)).map(|(i, _)| i).collect();
            Some(dl.start_in(col, Some(sel), None, pick))
        });
        if let Err(e) = &r {
            dl.notices.lock().push(e.clone());
        }
        gets.lock().insert(pl, r);
    });
}

/// The + GET button of a release: (label, clickable).
pub(super) fn get_button(app: &App, r: &Release, have: bool) -> (String, bool) {
    let st = app.home.gets.lock().get(&r.playlist).cloned();
    match st {
        None if have => ("✔ HAVE IT".into(), false),
        None => ("+ GET".into(), true),
        Some(Err(_)) => ("FAILED ↻".into(), true),
        Some(Ok(None)) => ("FETCHING…".into(), false),
        Some(Ok(Some(job))) => {
            let jobs = app.dl.jobs.lock();
            // dismissed from Downloads: what you have now decides (a cancelled one can be got again)
            let Some(j) = jobs.iter().find(|j| j.id == job) else { return if have { ("✔ HAVE IT".into(), false) } else { ("+ GET".into(), true) } };
            let total = j.tracks.iter().filter(|t| t.status != TStatus::Skipped).count();
            let done = j.tracks.iter().filter(|t| t.status == TStatus::Done).count();
            if j.tracks.iter().any(|t| t.status.active()) {
                if let Some(c) = &*crate::system::CTX.lock() {
                    c.request_repaint_after(Duration::from_millis(500));
                }
                (format!("{done}/{total} ⬇"), false)
            } else if done < total {
                (format!("{done}/{total} · RETRY"), true)
            } else {
                ("✔ GOT IT".into(), false)
            }
        }
    }
}

/// Release cards with + GET.
fn release_cards(app: &mut App, ui: &mut Ui, list: &[(Release, i64)], rows: usize, link_artist: bool, acts: &mut Vec<Act>) {
    let pal = app.pal;
    let now = now_secs();
    let have = have_keys(app);
    cards(ui, list.len(), rows, |ui, i, cw| {
        let (r, day) = &list[i];
        let t = remote_tex(app, ui.ctx(), r.cover.as_ref());
        let (label, on) = get_button(app, r, discover::have(&have, r));
        let when = if *day > 0 { ago(now - day) } else { r.year.map(|y| y.to_string()).unwrap_or_default() };
        let sub = if link_artist { format!("{} · {} · {when}", r.artist, r.kind) } else { format!("{} · {when}", r.kind) };
        let (click, _, get, resp) = card(ui, &pal, cw, &r.title, &sub, t, "💿", false, Some((&label, on)));
        if get {
            acts.push(Act::Get(r.clone()));
        } else if click && link_artist {
            acts.push(Act::Open(View::Artist(main_artist(&r.artist).to_lowercase())));
        }
        resp.context_menu(|ui| {
            if ui.add_enabled(on, egui::Button::new("Get it (download)")).clicked() {
                acts.push(Act::Get(r.clone()));
                ui.close_menu();
            }
            if ui.button("Open on YouTube Music").clicked() {
                acts.push(Act::Web(r.url()));
                ui.close_menu();
            }
        });
    });
}

fn run(app: &mut App, acts: Vec<Act>) {
    for a in acts {
        match a {
            Act::Open(v) => app.browser.set_view(v),
            Act::Play(ids, i, shuffle) => app.player.play_pick(ids, i, Some(shuffle)),
            Act::PlayMix(id) => play_mix(app, &id, true),
            Act::PlayPlaylist(id) => {
                let ids: Vec<String> = { let d = app.lib.data.read(); d.playlists.iter().find(|p| p.id == id).map(|p| p.track_ids.iter().filter(|t| d.tracks.contains_key(*t)).cloned().collect()).unwrap_or_default() };
                if !ids.is_empty() {
                    app.player.play_list(ids, 0, Some(false));
                    app.lib.touch_playlist(&id);
                }
            }
            Act::PlayAlbum(k) => {
                let mut v: Vec<(u32, String, String)> = app.lib.data.read().tracks.values().filter(|t| album_key(t) == k).map(|t| (t.track.unwrap_or(999), t.title.to_lowercase(), t.id.clone())).collect();
                v.sort();
                app.player.play_list(v.into_iter().map(|x| x.2).collect(), 0, Some(false));
            }
            Act::Get(r) => get_release(app, &r),
            Act::Web(u) => {
                let _ = open::that(u);
            }
        }
    }
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    rebuild(app);
    mixes_online(app);
    if app.lib.data.read().tracks.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("WELCOME TO DK.FM").font(px(10.0)).color(pal.accent));
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Add a music folder, drop audio files on the window, or import a playlist: Home fills up as you listen.").color(pal.dim));
            ui.add_space(12.0);
            if button(ui, &pal, "IMPORT MUSIC", true, true).clicked() {
                app.browser.set_view(View::Import);
            }
        });
        return;
    }
    let mut acts: Vec<Act> = Vec::new();
    let mut sa = egui::ScrollArea::vertical().id_salt("home").auto_shrink([false; 2]);
    if let Some(y) = app.home.scroll.take() {
        sa = sa.vertical_scroll_offset(y);
    }
    sa.show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 10, bottom: 24 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            let hour = { use chrono::Timelike; chrono::Local::now().hour() };
            ui.label(egui::RichText::new(match hour { 5..=11 => "GOOD MORNING", 12..=17 => "GOOD AFTERNOON", _ => "GOOD EVENING" }).font(px(12.0)).color(pal.text));
            // ---- jump back in
            if !app.home.jump.is_empty() {
                shelf(ui, &pal, "JUMP BACK IN", "Playlists and albums you played lately");
                let n = app.home.jump.len();
                cards(ui, n, 1, |ui, i, cw| {
                    let (title, sub, t, icon, open, play) = match &app.home.jump[i] {
                        Jump::Playlist(p) => {
                            let (p, n) = (p.clone(), p.track_ids.len());
                            let c = super::plcover::cover_of(app, &p);
                            (p.name.clone(), format!("PLAYLIST · {n} SONGS"), tex(app, ui.ctx(), c.as_ref(), 192), super::browser::playlist_icon(&p), View::Playlist(p.id.clone()), Act::PlayPlaylist(p.id.clone()))
                        }
                        Jump::Album { key, title, artist, cover } => {
                            let (key, title, sub, cover) = (key.clone(), title.clone(), format!("ALBUM · {artist}"), cover.clone());
                            (title, sub, tex(app, ui.ctx(), cover.as_ref(), 192), "💿", View::Album(key.clone()), Act::PlayAlbum(key))
                        }
                    };
                    let (click, p, _, _) = card(ui, &pal, cw, &title, &sub, t, icon, true, None);
                    if p {
                        acts.push(play);
                    } else if click {
                        acts.push(Act::Open(open));
                    }
                });
            }
            // ---- mixes
            if !app.home.mixes.is_empty() {
                shelf(ui, &pal, "MADE FROM YOUR LIBRARY", "Mixes of your most played artists and genres · PLAY shuffles them");
                let mixes = app.home.mixes.clone();
                cards(ui, mixes.len(), 2, |ui, i, cw| {
                    let m = &mixes[i];
                    let c = super::plcover::cover_of(app, &Playlist { id: m.id.clone(), track_ids: m.ids.clone(), ..Default::default() });
                    let t = tex(app, ui.ctx(), c.as_ref(), 192);
                    let (click, p, _, _) = card(ui, &pal, cw, &m.name, &m.sub, t, "✨", true, None);
                    if p {
                        acts.push(Act::PlayMix(m.id.clone()));
                    } else if click {
                        acts.push(Act::Open(View::Mix(m.id.clone())));
                    }
                });
            }
            // ---- new releases
            if app.settings.lock().new_releases {
                shelf(ui, &pal, "NEW RELEASES FROM YOUR ARTISTS", &format!("Out in the last {} days on YouTube Music, not in your library yet · + GET downloads it", discover::WINDOW));
                let (list, busy, err) = releases(app, ui.ctx());
                if list.is_empty() {
                    let msg = match (busy, err) {
                        (true, _) => "Checking YouTube Music…".to_string(),
                        (_, Some(_)) => "Couldn't check for new releases (offline?): DK.FM tries again later.".into(),
                        _ => "Nothing new from your top artists lately.".into(),
                    };
                    ui.label(egui::RichText::new(msg).color(pal.dim));
                } else {
                    release_cards(app, ui, &list, 1, true, &mut acts);
                }
            }
            // ---- recently played
            if !app.home.recent.is_empty() {
                shelf(ui, &pal, "RECENTLY PLAYED", "");
                let ids = app.home.recent.clone();
                cards(ui, ids.len(), 1, |ui, i, cw| {
                    let Some(t) = app.lib.track(&ids[i]) else { return };
                    let tx = tex(app, ui.ctx(), t.thumb.as_ref().or(t.cover.as_ref()), 192);
                    let (click, p, _, resp) = card(ui, &pal, cw, &t.title, &t.artist, tx, "♫", true, None);
                    if click || p {
                        acts.push(Act::Play(ids.clone(), i, false));
                    }
                    resp.context_menu(|ui| {
                        ui.set_min_width(210.0);
                        super::browser::track_menu(app, ui, &ids, i, None);
                    });
                });
            }
            // ---- top songs this month
            if !app.home.top.is_empty() {
                shelf(ui, &pal, "YOUR TOP SONGS THIS MONTH", "");
                let ids: Vec<String> = app.home.top.iter().map(|t| t.0.clone()).collect();
                let w = ui.available_width();
                let half = if w > 700.0 { (w - 20.0) / 2.0 } else { w };
                let per_col = if w > 700.0 { ids.len().div_ceil(2) } else { ids.len() };
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 20.0;
                    for col in 0..ids.len().div_ceil(per_col.max(1)) {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for i in col * per_col..((col + 1) * per_col).min(ids.len()) {
                                let len = app.lib.track(&ids[i]).map(|t| fmt_time(t.duration)).unwrap_or_default();
                                if song_row(app, ui, half, &ids, i, &len) {
                                    acts.push(Act::Play(ids.clone(), i, false));
                                }
                            }
                        });
                    }
                });
                ui.add_space(8.0);
                if button(ui, &pal, "▶ PLAY THEM", false, true).clicked() {
                    acts.push(Act::Play(ids, 0, false));
                }
            }
        });
    });
    run(app, acts);
}

/// A song in a short list: number, cover, title — artist, `right` text; double-click plays
/// (returns true), right-click = the song menu.
fn song_row(app: &mut App, ui: &mut Ui, w: f32, ids: &[String], i: usize, right: &str) -> bool {
    let pal = app.pal;
    let Some(t) = app.lib.track(&ids[i]) else { return false };
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::click());
    let tx = tex(app, ui.ctx(), t.thumb.as_ref().or(t.cover.as_ref()), 64);
    let p = ui.painter();
    if resp.hovered() {
        fill(p, r, pal.panel_hi);
    }
    let cur = app.player.current_id().as_deref() == Some(t.id.as_str());
    let cy = r.center().y;
    p.text(Pos2::new(r.left() + 22.0, cy), Align2::RIGHT_CENTER, if cur { "▶".to_string() } else { (i + 1).to_string() }, vt(19.0), pal.accent);
    let ic = Rect::from_center_size(Pos2::new(r.left() + 44.0, cy), Vec2::splat(28.0));
    fill(p, ic, pal.bg);
    if let Some(tx) = tx {
        p.image(tx, ic, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }
    frame_rect(p, ic, 1.0, pal.line_hi);
    let rw = p.layout_no_wrap(right.to_string(), vt(17.0), pal.dim).size().x;
    let clip = p.with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.right() - rw - 16.0, r.bottom())));
    let g = clip.layout_no_wrap(t.title.clone(), vt(19.0), if cur { pal.accent } else { pal.text });
    let gw = g.size().x;
    clip.galley(Pos2::new(r.left() + 66.0, cy - g.size().y / 2.0), g, pal.text);
    clip.text(Pos2::new(r.left() + 74.0 + gw, cy), Align2::LEFT_CENTER, &t.artist, vt(17.0), pal.dim);
    p.text(Pos2::new(r.right() - 8.0, cy), Align2::RIGHT_CENTER, right, vt(17.0), pal.dim);
    if resp.secondary_clicked() && !app.browser.selection.contains(&t.id) {
        app.browser.selection = HashSet::from([t.id.clone()]);
    }
    resp.context_menu(|ui| {
        ui.set_min_width(210.0);
        super::browser::track_menu(app, ui, ids, i, None);
    });
    resp.double_clicked()
}

// ------------------------------------------------------------------------------- artist pages

fn artist_cache(app: &mut App, k: &str) {
    let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
    if app.home.page.as_ref().map(|p| p.key == k && p.gen == gen).unwrap_or(false) {
        return;
    }
    let d = app.lib.data.read();
    let plays = |id: &str| d.stats.get(id).map(|s| s.plays).unwrap_or(0);
    let mut ids: Vec<&crate::store::Track> = d.tracks.values().filter(|t| artist_key(t) == k).collect();
    ids.sort_by(|a, b| plays(&b.id).cmp(&plays(&a.id)).then(b.added_at.total_cmp(&a.added_at)).then(a.id.cmp(&b.id)));
    let mut albums: Vec<(String, String, Option<String>, Option<u32>, usize)> = Vec::new();
    for t in ids.iter().filter(|t| !t.album.is_empty()) {
        let ak = album_key(t);
        match albums.iter_mut().find(|a| a.0 == ak) {
            Some(a) => {
                a.4 += 1;
                a.2 = a.2.take().or(t.thumb.clone().or(t.cover.clone()));
            }
            None => albums.push((ak, t.album.clone(), t.thumb.clone().or(t.cover.clone()), t.year, 1)),
        }
    }
    albums.sort_by(|a, b| b.3.cmp(&a.3).then(a.1.cmp(&b.1)));
    let page = ArtistCache { key: k.to_string(), gen, name: ids.first().map(|t| main_artist(&t.artist)).unwrap_or_default(), ids: ids.iter().map(|t| t.id.clone()).collect(), albums };
    drop(d);
    app.home.page = Some(page);
}

/// Starts "More from this artist" (once per session), and takes its result when it's in.
fn online(app: &mut App, ctx: &egui::Context, k: &str, name: &str, start: bool) {
    let o = app.home.online.entry(k.to_string()).or_default();
    if o.shown.is_none() {
        let got = o.slot.lock().take();
        match got {
            Some(r) => {
                let have = have_keys(app);
                let idx = app.lib.key_index();
                let shown = r.map(|p| {
                    let songs: Vec<ITrack> = p.songs.into_iter().filter(|t| !idx.contains_key(&t.source_key) && !idx.contains_key(&ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title))).take(10).collect();
                    let mut seen = HashSet::new();
                    let rels: Vec<Release> = p.releases.into_iter().filter(|r| !discover::have(&have, r) && seen.insert(ta_key(&r.artist, &r.title))).collect();
                    (songs, rels)
                });
                app.home.online.get_mut(k).unwrap().shown = Some(shown);
            }
            None if start && Arc::strong_count(&o.slot) == 1 => {
                let (slot, name, ctx) = (o.slot.clone(), name.to_string(), ctx.clone());
                std::thread::spawn(move || {
                    let r = (|| {
                        let id = sources::artist_id(&name)?.ok_or("Not found on YouTube Music")?;
                        let mut page = sources::artist_page(&id)?;
                        // more of their songs than the page's top 5
                        let want = name.to_lowercase();
                        for t in sources::search(&name, "songs", 20).unwrap_or_default() {
                            if t.artists.iter().any(|a| a.to_lowercase() == want) && !page.songs.iter().any(|s| s.youtube_id == t.youtube_id) {
                                page.songs.push(t);
                            }
                        }
                        super::cjk::ensure(&ctx, &page.songs.iter().map(|t| t.title.as_str()).chain(page.releases.iter().map(|r| r.title.as_str())).collect::<String>());
                        Ok::<ArtistPage, String>(page)
                    })();
                    *slot.lock() = Some(r);
                    ctx.request_repaint();
                });
            }
            None => {}
        }
    }
}

pub fn artist_page(app: &mut App, ui: &mut Ui, k: &str) {
    let pal = app.pal;
    artist_cache(app, k);
    let Some(page) = app.home.page.as_ref() else { return };
    if page.ids.is_empty() {
        app.browser.set_view(View::Artists);
        return;
    }
    let (name, ids, albums) = (page.name.clone(), page.ids.clone(), page.albums.clone());
    let mut acts: Vec<Act> = Vec::new();
    let mut sa = egui::ScrollArea::vertical().id_salt(("artist", k)).auto_shrink([false; 2]);
    if let Some(y) = app.home.scroll.take() {
        sa = sa.vertical_scroll_offset(y);
    }
    sa.show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 16, top: 10, bottom: 24 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            // ---- header
            if button(ui, &pal, "◀ ARTISTS", false, true).clicked() {
                acts.push(Act::Open(View::Artists));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let cover = ids.iter().filter_map(|id| app.lib.track(id)).find_map(|t| t.cover.clone());
                let (r, _) = ui.allocate_exact_size(Vec2::splat(128.0), Sense::hover());
                fill(ui.painter(), r.translate(Vec2::splat(4.0)), pal.shadow);
                fill(ui.painter(), r, pal.bg);
                match tex(app, ui.ctx(), cover.as_ref(), 256) {
                    Some(t) => {
                        ui.painter().image(t, r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, "👤", vt(56.0), pal.faint);
                    }
                }
                frame_rect(ui.painter(), r, 2.0, pal.line_hi);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("ARTIST").font(px(6.0)).color(pal.dim));
                    ui.label(egui::RichText::new(&name).font(px(16.0)).color(pal.text));
                    let n = |x: usize, w: &str| format!("{x} {w}{}", if x == 1 { "" } else { "S" });
                    ui.label(egui::RichText::new(format!("{} · {} IN YOUR LIBRARY", n(ids.len(), "SONG"), n(albums.len(), "ALBUM"))).color(pal.dim));
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if button(ui, &pal, "▶ PLAY", true, true).clicked() {
                            acts.push(Act::Play(ids.clone(), 0, false));
                        }
                        if button(ui, &pal, "SHUFFLE", false, true).clicked() {
                            acts.push(Act::Play(ids.clone(), fastrand::usize(..ids.len()), true));
                        }
                        if button(ui, &pal, &format!("ALL {} SONGS", ids.len()), false, true).on_hover_text("As a list you can sort and search").clicked() {
                            acts.push(Act::Open(View::ArtistSongs(k.to_string())));
                        }
                        if button(ui, &pal, "✨ RADIO", false, true).on_hover_text("Songs like theirs (YouTube Music)").clicked() {
                            if let Some(t) = app.lib.track(&ids[0]) {
                                super::import::radio_for(app, &t);
                            }
                        }
                    });
                });
            });
            // ---- top songs
            shelf(ui, &pal, "POPULAR IN YOUR LIBRARY", "Double-click to play");
            let w = ui.available_width();
            ui.spacing_mut().item_spacing.y = 0.0;
            let top: Vec<String> = ids.iter().take(5).cloned().collect();
            for i in 0..top.len() {
                let len = app.lib.track(&top[i]).map(|t| fmt_time(t.duration)).unwrap_or_default();
                if song_row(app, ui, w, &ids, i, &len) {
                    acts.push(Act::Play(ids.clone(), i, false));
                }
            }
            ui.spacing_mut().item_spacing.y = 2.0;
            // ---- albums
            if !albums.is_empty() {
                let more = if albums.len() > 2 * columns(ui.available_width()).0 { "Newest first · see them all in ALL SONGS or Albums" } else { "" };
                shelf(ui, &pal, "ALBUMS IN YOUR LIBRARY", more);
                cards(ui, albums.len(), 2, |ui, i, cw| {
                    let (ak, title, cover, year, n) = &albums[i];
                    let t = tex(app, ui.ctx(), cover.as_ref(), 192);
                    let sub = format!("{}{n} SONG{}", year.map(|y| format!("{y} · ")).unwrap_or_default(), if *n == 1 { "" } else { "S" });
                    let (click, p, _, _) = card(ui, &pal, cw, title, &sub, t, "💿", true, None);
                    if p {
                        acts.push(Act::PlayAlbum(ak.clone()));
                    } else if click {
                        acts.push(Act::Open(View::Album(ak.clone())));
                    }
                });
            }
            // ---- more from them online (fetched when this part scrolls into view)
            shelf(ui, &pal, &format!("MORE FROM {} ON YOUTUBE MUSIC", name.to_uppercase()), "Songs and releases you don't have yet · + GET downloads them");
            let (spot, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
            online(app, ui.ctx(), k, &name, ui.is_rect_visible(spot));
            let shown = app.home.online.get(k).and_then(|o| o.shown.clone());
            match shown {
                None => {
                    ui.label(egui::RichText::new("LOOKING…").font(px(6.0)).color(pal.accent));
                }
                Some(Err(e)) => {
                    ui.label(egui::RichText::new(e).color(pal.dim));
                }
                Some(Ok((songs, rels))) => {
                    if songs.is_empty() && rels.is_empty() {
                        ui.label(egui::RichText::new("You have everything of theirs that YouTube Music lists here.").color(pal.dim));
                    }
                    if !songs.is_empty() {
                        let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
                        if app.home.online.get(k).and_then(|o| o.owned.as_ref()).map(|o| o.0 != gen).unwrap_or(true) {
                            let owned = super::addsongs::owned_ids(app, &songs);
                            if let Some(o) = app.home.online.get_mut(k) {
                                o.owned = Some((gen, owned));
                            }
                        }
                        let owned = app.home.online.get(k).and_then(|o| o.owned.as_ref()).map(|o| o.1.clone()).unwrap_or_default();
                        ui.spacing_mut().item_spacing.y = 0.0;
                        super::addsongs::result_rows(app, ui, None, &songs, &owned, w);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add_space(12.0);
                    }
                    if !rels.is_empty() {
                        let list: Vec<(Release, i64)> = rels.into_iter().map(|r| (r, 0)).collect();
                        release_cards(app, ui, &list, 99, false, &mut acts);
                    }
                }
            }
        });
    });
    run(app, acts);
}

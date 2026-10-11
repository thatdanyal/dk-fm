//! PODCASTS: the shows you follow and their episodes. Search Apple Podcasts or paste a show's
//! RSS link, FOLLOW, then ▶ an episode: it downloads, plays at the podcast speed and picks up
//! where you stopped. CONTINUE LISTENING lists the ones you're in the middle of. Back 15 s /
//! ahead 30 s and the speed are at the top while an episode plays. See podcasts.rs.
use super::theme::{px, vt};
use super::widgets::{button, fill, fmt_time, frame_rect, outline_button};
use super::App;
use crate::podcasts::{Episode, Found, Podcasts, Show};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Ui, Vec2};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

type Slot<T> = Arc<Mutex<Option<T>>>;

#[derive(Default)]
pub struct PodState {
    data: Option<Podcasts>,
    /// show id, or "" = CONTINUE LISTENING
    picked: String,
    query: String,
    results: Option<Slot<Result<Vec<Found>, String>>>,
    /// feeds read in the background, to merge in
    fetched: Arc<Mutex<Vec<Result<crate::podcasts::Show, String>>>>,
    refreshing: bool,
    /// episodes downloading: id -> 0-1
    loading: Arc<Mutex<HashMap<String, f32>>>,
    /// downloads done: (episode, file or error)
    done: Arc<Mutex<Vec<(String, Result<std::path::PathBuf, String>)>>>,
    /// the episode playing last frame, its last position, when progress was last saved
    playing: Option<String>,
    last_pos: f64,
    saved: Option<Instant>,
    dirty: bool,
}

const SPEEDS: [f32; 6] = [0.8, 1.0, 1.25, 1.5, 1.75, 2.0];

fn data(app: &mut App) -> &mut Podcasts {
    app.podcasts.data.get_or_insert_with(Podcasts::load)
}

/// An episode's show notes (shown in the lyrics panel while it plays).
pub fn about(app: &mut App, id: &str) -> String {
    data(app).episode(id).map(|(_, e)| e.about.clone()).unwrap_or_default()
}

fn speed_label(s: f32) -> String {
    let t = format!("{s:.2}");
    format!("{}×", t.trim_end_matches('0').trim_end_matches('.'))
}

/// The show's cover as a file in the covers folder (fetched once).
fn cover_name(show: &Show) -> String {
    format!("pc_{}.jpg", show.id)
}

/// Every frame: downloads that finished start playing, feeds read are merged, and the episode
/// playing has its place saved (finished ones are marked played and their file deleted).
pub fn tick(app: &mut App) {
    let fetched: Vec<_> = std::mem::take(&mut *app.podcasts.fetched.lock());
    if !fetched.is_empty() {
        let mut errs = Vec::new();
        let lib = app.lib.clone();
        for r in fetched {
            match r {
                Ok(show) => {
                    let d = data(app);
                    match d.shows.iter_mut().find(|s| s.id == show.id) {
                        Some(s) => *s = show.clone(),
                        None => d.shows.push(show.clone()),
                    }
                    d.save();
                    // its cover, once
                    if let Some(url) = show.cover.clone().filter(|_| !lib.cover_path(&cover_name(&show)).exists()) {
                        let (lib, name) = (lib.clone(), cover_name(&show));
                        std::thread::spawn(move || {
                            if let Ok(b) = crate::net::get_bytes(&url) {
                                lib.save_playlist_cover(&b, Some(name));
                            }
                        });
                    }
                }
                Err(e) => errs.push(e),
            }
        }
        app.podcasts.refreshing = false;
        if let Some(e) = errs.first() {
            app.toast_err(e.clone());
        }
    }
    let done: Vec<_> = std::mem::take(&mut *app.podcasts.done.lock());
    for (id, r) in done {
        app.podcasts.loading.lock().remove(&id);
        match r {
            Ok(file) => play_file(app, &id, file),
            Err(e) => app.toast_err(e),
        }
    }
    // the episode playing: where you are
    let cur = app.player.current_id().filter(|id| id.starts_with(crate::audio::EPISODE));
    if cur != app.podcasts.playing {
        if let Some(prev) = app.podcasts.playing.take() {
            finish(app, &prev);
        }
        app.podcasts.playing = cur.clone();
        app.podcasts.last_pos = -1.0;
        if cur.is_some() {
            let s = data(app).speed;
            app.player.set_podcast_speed(s);
        }
    }
    let Some(id) = cur else { return };
    let st = app.player.status();
    let now = crate::store::now_secs();
    let pos = st.position;
    let moved = pos - app.podcasts.last_pos;
    if (moved - 0.0).abs() < 1e-6 {
        return;
    }
    let listened = if app.podcasts.last_pos >= 0.0 && st.playing && (0.0..5.0).contains(&moved) { moved } else { 0.0 };
    app.podcasts.last_pos = pos;
    let year = chrono::Local::now().format("%Y").to_string().parse().unwrap_or(0);
    let d = data(app);
    let dur = d.episode(&id).map(|(_, e)| e.duration).filter(|x| *x > 0.0).unwrap_or(st.duration);
    let p = d.progress.entry(id.clone()).or_default();
    p.pos = pos;
    p.at = now;
    if dur > 60.0 && pos >= dur - 20.0 {
        p.done = true;
    }
    *d.listened.entry(year).or_default() += listened;
    app.podcasts.dirty = true;
    if app.podcasts.saved.is_none_or(|t| t.elapsed().as_secs() >= 15) {
        save(app);
    }
}

fn save(app: &mut App) {
    if app.podcasts.dirty {
        data(app).save();
        app.podcasts.dirty = false;
    }
    app.podcasts.saved = Some(Instant::now());
}

/// An episode stopped being the one playing: saved, and if it's finished its file goes.
fn finish(app: &mut App, id: &str) {
    save(app);
    let done = data(app).progress.get(id).is_some_and(|p| p.done);
    if done {
        loop {
            let at = app.player.st.lock().queue.iter().position(|q| q == id);
            let Some(i) = at else { break };
            app.player.remove_at(i);
        }
        if let Some(t) = app.lib.episodes.write().remove(id) {
            let f = std::path::PathBuf::from(t.path);
            std::thread::spawn(move || {
                for _ in 0..20 {
                    if !f.exists() || std::fs::remove_file(&f).is_ok() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            });
        }
    }
}

/// Play an episode whose file is here (from where you stopped).
fn play_file(app: &mut App, id: &str, file: std::path::PathBuf) {
    let d = data(app);
    let Some((show, e)) = d.episode(id).map(|(s, e)| (s.clone(), e.clone())) else { return };
    let resume = d.progress.get(id).filter(|p| !p.done).map(|p| (p.pos - 3.0).max(0.0)).unwrap_or(0.0);
    let s = d.speed;
    let cover = Some(cover_name(&show)).filter(|c| app.lib.cover_path(c).exists());
    let t = crate::store::Track { id: e.id.clone(), path: file.display().to_string(), title: e.title.clone(), artist: show.title.clone(), album: show.title.clone(), duration: e.duration, cover: cover.clone(), thumb: cover, codec: "PODCAST".into(), ..Default::default() };
    app.lib.episodes.write().insert(e.id.clone(), t);
    app.player.set_podcast_speed(s);
    app.player.play_now_at(e.id.clone(), resume);
}

/// ▶ on an episode: play it (downloading it first if needed).
fn play(app: &mut App, e: &Episode) {
    if app.player.current_id().as_deref() == Some(e.id.as_str()) {
        app.player.toggle();
        return;
    }
    let f = crate::podcasts::file_of(e);
    if f.exists() {
        return play_file(app, &e.id, f);
    }
    if app.podcasts.loading.lock().contains_key(&e.id) {
        return;
    }
    app.podcasts.loading.lock().insert(e.id.clone(), 0.0);
    let (e, loading, done, ctx) = (e.clone(), app.podcasts.loading.clone(), app.podcasts.done.clone(), app.covers.ctx.clone());
    std::thread::Builder::new()
        .name("episode".into())
        .spawn(move || {
            let r = crate::podcasts::download(&e, &|p| {
                loading.lock().insert(e.id.clone(), p);
                if let Some(c) = &ctx {
                    c.request_repaint_after(std::time::Duration::from_millis(250));
                }
            });
            done.lock().push((e.id.clone(), r));
            if let Some(c) = &ctx {
                c.request_repaint();
            }
        })
        .ok();
}

fn fetch(app: &mut App, feeds: Vec<String>) {
    if feeds.is_empty() {
        return;
    }
    app.podcasts.refreshing = true;
    let (out, ctx) = (app.podcasts.fetched.clone(), app.covers.ctx.clone());
    std::thread::spawn(move || {
        for f in feeds {
            let r = crate::podcasts::fetch(&f);
            out.lock().push(r);
        }
        if let Some(c) = &ctx {
            c.request_repaint();
        }
    });
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    // feeds not read for 3 hours: read again (once per visit)
    if !app.podcasts.refreshing {
        let now = crate::store::now_secs();
        let old: Vec<String> = data(app).shows.iter().filter(|s| now - s.refreshed > 3 * 3600).map(|s| s.feed.clone()).collect();
        fetch(app, old);
    }
    let mut search = false;
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("PODCASTS").font(px(12.0)).color(pal.text));
            ui.add_space(12.0);
            let r = ui.add(egui::TextEdit::singleline(&mut app.podcasts.query).desired_width(420.0).font(vt(19.0)).hint_text("Find a podcast or audiobook, or paste its RSS link"));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                search = true;
            }
            if button(ui, &pal, "SEARCH", true, true).clicked() {
                search = true;
            }
            if app.podcasts.results.is_some() && button(ui, &pal, "✕ CLOSE", false, true).clicked() {
                app.podcasts.results = None;
            }
        });
        ui.add_space(4.0);
        // speed, and back / ahead while an episode plays
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("SPEED").font(px(5.0)).color(pal.dim));
            let now = data(app).speed;
            for s in SPEEDS {
                if outline_button(ui, &pal, &speed_label(s), (now - s).abs() < 0.01).on_hover_text("Podcast speed (voices keep their pitch; music always plays at 1×)").clicked() {
                    data(app).speed = s;
                    data(app).save();
                    app.player.set_podcast_speed(s);
                }
            }
            if let Some(t) = app.current_track().filter(|t| t.id.starts_with(crate::audio::EPISODE)) {
                ui.add_space(14.0);
                let pos = app.player.status().position;
                if button(ui, &pal, "◀ 15 S", false, true).on_hover_text("Back 15 seconds").clicked() {
                    app.player.seek((pos - 15.0).max(0.0));
                }
                if button(ui, &pal, "30 S ▶", false, true).on_hover_text("Ahead 30 seconds").clicked() {
                    app.player.seek(pos + 30.0);
                }
                ui.add(egui::Label::new(egui::RichText::new(format!("{} · {}", t.artist, t.title)).font(vt(16.0)).color(pal.accent2)).truncate());
            }
        });
    });
    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), egui::Stroke::new(2.0_f32, pal.line));
    if search {
        let q = app.podcasts.query.trim().to_string();
        if q.starts_with("http://") || q.starts_with("https://") {
            app.toast("Reading the feed…");
            fetch(app, vec![q]);
        } else if !q.is_empty() {
            let slot: Slot<Result<Vec<Found>, String>> = Arc::new(Mutex::new(None));
            app.podcasts.results = Some(slot.clone());
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                *slot.lock() = Some(crate::podcasts::search(&q));
                ctx.request_repaint();
            });
        }
    }
    if let Some(slot) = app.podcasts.results.clone() {
        results(app, ui, &slot);
        return;
    }
    // (dev hook: DKFM_EPISODE=1 plays the first show's newest episode, from DKFM_EPISODE_AT s)
    static DEV: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if std::env::var("DKFM_EPISODE").is_ok() && !DEV.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(e) = data(app).shows.first().and_then(|s| s.episodes.first()).cloned() {
            DEV.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(at) = std::env::var("DKFM_EPISODE_AT").ok().and_then(|v| v.parse::<f64>().ok()) {
                data(app).progress.entry(e.id.clone()).or_default().pos = at;
            }
            play(app, &e);
        }
    }
    if data(app).shows.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("NO PODCASTS YET").font(px(8.0)).color(pal.dim));
            ui.label(egui::RichText::new("Search for a show above and press FOLLOW. Audiobooks with an RSS feed work too.").font(vt(18.0)).color(pal.dim));
        });
        return;
    }
    let body = ui.available_rect_before_wrap();
    let side = Rect::from_min_size(body.min, Vec2::new(240.0, body.height()));
    let main = Rect::from_min_max(Pos2::new(side.right() + 6.0, body.top()), body.max);
    ui.painter().vline(side.right() + 2.0, body.y_range(), egui::Stroke::new(1.0_f32, pal.line));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(side), |ui| shows_list(app, ui));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(main), |ui| episodes(app, ui));
    ui.allocate_rect(body, Sense::hover());
}

fn results(app: &mut App, ui: &mut Ui, slot: &Slot<Result<Vec<Found>, String>>) {
    let pal = app.pal;
    let r = slot.lock().clone();
    let list = match r {
        None => return center(ui, &pal, "SEARCHING…"),
        Some(Err(e)) => return center(ui, &pal, &format!("Couldn't search: {e}")),
        Some(Ok(l)) if l.is_empty() => return center(ui, &pal, "NOTHING FOUND"),
        Some(Ok(l)) => l,
    };
    let following: Vec<String> = data(app).shows.iter().map(|s| s.feed.clone()).collect();
    let mut follow = None;
    egui::ScrollArea::vertical().id_salt("pod-results").auto_shrink([false; 2]).show(ui, |ui| {
        for f in &list {
            let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::hover());
            let cov = Rect::from_min_size(r.min + Vec2::new(12.0, 6.0), Vec2::splat(52.0));
            fill(ui.painter(), cov, pal.bg);
            if let Some(t) = super::home::remote_tex(app, ui.ctx(), f.cover.as_ref()) {
                ui.painter().image(t, cov, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            let br = Rect::from_min_size(Pos2::new(r.right() - 130.0, r.top() + 18.0), Vec2::new(118.0, 28.0));
            let p = ui.painter().with_clip_rect(Rect::from_min_max(r.min, Pos2::new(br.left() - 8.0, r.bottom())));
            p.text(Pos2::new(cov.right() + 12.0, r.top() + 22.0), Align2::LEFT_CENTER, &f.title, vt(20.0), pal.text);
            p.text(Pos2::new(cov.right() + 12.0, r.top() + 42.0), Align2::LEFT_CENTER, &f.author, vt(16.0), pal.dim);
            let on = following.contains(&f.feed);
            let mut ui2 = ui.new_child(egui::UiBuilder::new().max_rect(br));
            if button(&mut ui2, &pal, if on { "✔ FOLLOWING" } else { "+ FOLLOW" }, !on, !on).clicked() {
                follow = Some(f.feed.clone());
            }
        }
    });
    if let Some(feed) = follow {
        app.toast("Following: its episodes are on their way");
        fetch(app, vec![feed]);
    }
}

fn center(ui: &mut Ui, pal: &super::theme::Pal, s: &str) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| ui.label(egui::RichText::new(s).font(px(6.0)).color(pal.dim)));
}

fn shows_list(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let shows: Vec<(String, String, String)> = data(app).shows.iter().map(|s| (s.id.clone(), s.title.clone(), cover_name(s))).collect();
    let continuing = !data(app).in_progress().is_empty();
    if app.podcasts.picked.is_empty() && !continuing {
        app.podcasts.picked = shows.first().map(|s| s.0.clone()).unwrap_or_default();
    }
    egui::ScrollArea::vertical().id_salt("pod-shows").auto_shrink([false; 2]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut rows: Vec<(String, String, Option<String>)> = Vec::new();
        if continuing {
            rows.push((String::new(), "CONTINUE LISTENING".into(), None));
        }
        rows.extend(shows.into_iter().map(|(id, t, c)| (id, t, Some(c))));
        for (id, title, cover) in rows {
            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 50.0), Sense::click());
            if app.podcasts.picked == id {
                fill(ui.painter(), r, pal.panel_hi);
                fill(ui.painter(), Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), pal.accent);
            } else if resp.hovered() {
                fill(ui.painter(), r, pal.panel_hi);
            }
            let cov = Rect::from_min_size(r.min + Vec2::new(8.0, 5.0), Vec2::splat(40.0));
            match cover.filter(|c| app.lib.cover_path(c).exists()) {
                Some(c) => {
                    if let Some(t) = app.covers.get(ui.ctx(), app.lib.cover_path(&c), &c, 64) {
                        ui.painter().image(t, cov, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                }
                None => {
                    fill(ui.painter(), cov, pal.bg);
                    ui.painter().text(cov.center(), Align2::CENTER_CENTER, if id.is_empty() { "▶" } else { "♪" }, vt(20.0), pal.accent2);
                }
            }
            let p = ui.painter().with_clip_rect(r.shrink(2.0));
            p.text(Pos2::new(cov.right() + 8.0, r.center().y), Align2::LEFT_CENTER, &title, if id.is_empty() { px(5.0) } else { vt(18.0) }, pal.text);
            if resp.clicked() {
                app.podcasts.picked = id;
            }
        }
    });
}

fn episodes(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let picked = app.podcasts.picked.clone();
    let d = data(app);
    let (title, list): (String, Vec<(Show, Episode)>) = if picked.is_empty() {
        ("CONTINUE LISTENING".into(), d.in_progress().into_iter().map(|(s, e)| (s.clone(), e.clone())).collect())
    } else {
        match d.shows.iter().find(|s| s.id == picked) {
            Some(s) => (s.title.clone(), s.episodes.iter().map(|e| (s.clone(), e.clone())).collect()),
            None => (String::new(), Vec::new()),
        }
    };
    let author = d.shows.iter().find(|s| s.id == picked).map(|s| s.author.clone()).unwrap_or_default();
    let progress = d.progress.clone();
    let mut unfollow = false;
    let mut refresh = false;
    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&title).font(vt(24.0)).color(pal.text));
                if !author.is_empty() {
                    ui.label(egui::RichText::new(&author).font(vt(16.0)).color(pal.dim));
                }
            });
            if !picked.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if button(ui, &pal, "UNFOLLOW", false, true).clicked() {
                        unfollow = true;
                    }
                    if button(ui, &pal, if app.podcasts.refreshing { "CHECKING…" } else { "CHECK FOR NEW" }, false, !app.podcasts.refreshing).clicked() {
                        refresh = true;
                    }
                });
            }
        });
    });
    if unfollow {
        let d = data(app);
        d.shows.retain(|s| s.id != picked);
        d.save();
        app.podcasts.picked.clear();
        return;
    }
    if refresh {
        let feed = data(app).shows.iter().find(|s| s.id == picked).map(|s| s.feed.clone());
        fetch(app, feed.into_iter().collect());
    }
    let loading = app.podcasts.loading.lock().clone();
    let cur = app.player.current_id();
    let playing = app.player.status().playing;
    let mut act: Option<(Episode, &'static str)> = None;
    let row_h = 58.0;
    egui::ScrollArea::vertical().id_salt("pod-eps").auto_shrink([false; 2]).show_rows(ui, row_h, list.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (show, e) in &list[range] {
            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::hover());
            let here = cur.as_deref() == Some(e.id.as_str());
            if here {
                fill(ui.painter(), r, pal.panel_hi);
            } else if resp.hovered() {
                fill(ui.painter(), r, super::widgets::with_alpha(pal.panel_hi, 160));
            }
            let pr = progress.get(&e.id);
            let done = pr.is_some_and(|p| p.done);
            let play_r = Rect::from_min_size(Pos2::new(r.left() + 10.0, r.top() + 12.0), Vec2::splat(34.0));
            let pb = ui.interact(play_r, ui.id().with(("ep-play", &e.id)), Sense::click());
            fill(ui.painter(), play_r, if pb.hovered() { pal.accent } else { pal.panel });
            frame_rect(ui.painter(), play_r, 1.5, pal.accent);
            let fg = if pb.hovered() { pal.ink } else { pal.text };
            match loading.get(&e.id) {
                Some(p) => {
                    ui.painter().text(play_r.center(), Align2::CENTER_CENTER, format!("{:.0}%", p * 100.0), px(4.0), pal.text);
                }
                None if here && playing => {
                    // pause: two bars
                    for dx in [-5.0, 5.0] {
                        fill(ui.painter(), Rect::from_center_size(play_r.center() + Vec2::new(dx, 0.0), Vec2::new(5.0, 16.0)), fg);
                    }
                }
                None => {
                    ui.painter().text(play_r.center(), Align2::CENTER_CENTER, "▶", vt(18.0), fg);
                }
            }
            if pb.on_hover_text(if crate::podcasts::file_of(e).exists() { "Play" } else { "Download and play" }).clicked() {
                act = Some((e.clone(), "play"));
            }
            let mr = Rect::from_min_size(Pos2::new(r.right() - 120.0, r.top() + 15.0), Vec2::new(110.0, 28.0));
            let mb = ui.interact(mr, ui.id().with(("ep-mark", &e.id)), Sense::click());
            ui.painter().text(mr.center(), Align2::CENTER_CENTER, if done { "MARK UNPLAYED" } else { "MARK PLAYED" }, px(4.0), if mb.hovered() { pal.accent } else { pal.dim });
            if mb.clicked() {
                act = Some((e.clone(), if done { "unplayed" } else { "played" }));
            }
            let x = play_r.right() + 12.0;
            let p = ui.painter().with_clip_rect(Rect::from_min_max(r.min, Pos2::new(mr.left() - 8.0, r.bottom())));
            p.text(Pos2::new(x, r.top() + 16.0), Align2::LEFT_CENTER, &e.title, vt(19.0), if done { pal.dim } else { pal.text });
            let date = if e.date > 0 { crate::store::local_stamp(e.date, false) } else { String::new() };
            let len = if e.duration > 0.0 { fmt_time(e.duration) } else { String::new() };
            let state = match pr {
                Some(p) if p.done => "PLAYED ✔".to_string(),
                Some(p) if p.pos > 5.0 && e.duration > 0.0 => format!("{} LEFT", fmt_time((e.duration - p.pos).max(0.0))),
                _ => String::new(),
            };
            let line = [if picked.is_empty() { show.title.clone() } else { String::new() }, date, len, state].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            p.text(Pos2::new(x, r.top() + 36.0), Align2::LEFT_CENTER, line, vt(15.0), pal.dim);
            // how far you are
            if let (Some(p), true) = (pr.filter(|p| !p.done && p.pos > 5.0), e.duration > 0.0) {
                let bar = Rect::from_min_size(Pos2::new(x, r.bottom() - 7.0), Vec2::new((mr.left() - 20.0 - x).max(10.0), 3.0));
                fill(ui.painter(), bar, pal.line);
                fill(ui.painter(), Rect::from_min_size(bar.min, Vec2::new(bar.width() * (p.pos / e.duration).clamp(0.0, 1.0) as f32, 3.0)), pal.accent2);
            }
            if !e.about.is_empty() {
                resp.on_hover_text(&e.about);
            }
        }
    });
    if list.is_empty() {
        center(ui, &pal, if picked.is_empty() { "NOTHING STARTED" } else { "NO EPISODES YET" });
    }
    match act {
        Some((e, "play")) => play(app, &e),
        Some((e, mark)) => {
            let d = data(app);
            let p = d.progress.entry(e.id.clone()).or_default();
            p.done = mark == "played";
            if !p.done {
                p.pos = 0.0;
            }
            d.save();
        }
        None => {}
    }
}

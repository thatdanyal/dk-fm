//! Ctrl+K command palette: songs, albums, artists, playlists, actions, themes — or paste a link.
//! Enter = do it · Shift+Enter = add song to queue · Alt+Enter = play song next ·
//! Tab (or the + on a song) = add it to playlists (Esc goes back).
use super::browser::{album_key, artist_key, View};
use super::plpick::{self, Picker};
use super::theme::{self, px, vt};
use super::widgets::{fill, frame_rect};
use super::{App, Modal, Tab};
use crate::library::main_artist;
use eframe::egui::{self, Align2, Color32, Id, Key, Pos2, Rect, Sense, Vec2};

#[derive(Default)]
pub struct PaletteState {
    pub query: String,
    pub active: usize,
    pub focused: bool,
    /// playlist picker for a song (Tab on a song, or "Add current song to playlist…")
    pub pick: Option<Picker>,
}

#[derive(Clone)]
enum Act {
    Toggle,
    Next,
    Prev,
    ShuffleAll,
    ShuffleLiked,
    Radio,
    LikeCurrent,
    PickCurrent,
    ToggleShuffle,
    CycleRepeat,
    View(View),
    /// SHAZAM (also on the deck)
    Listen,
    Import(String),
    SyncAll,
    OpenShare,
    Mini,
    Layout,
    Visualizer,
    Rescan,
    Settings,
    Theme(String),
    ApplyLayout(String),
    PlaySong(Vec<String>, usize),
    Private,
    Undo,
    PlayFolder(String),
    NowPlaying,
    ResetLayout,
    Panel(Tab),
    Tour,
}

struct Item {
    group: &'static str,
    label: String,
    sub: String,
    icon: String,
    cover: Option<String>,
    song: Option<String>,
    act: Act,
}

pub fn score(hay: &str, toks: &[String]) -> Option<f32> {
    let mut s = 0.0;
    for t in toks {
        let i = hay.find(t.as_str())?;
        s += if i == 0 { 30.0 } else if hay.as_bytes()[i - 1] == b' ' { 15.0 } else { 5.0 };
        s -= i as f32 * 0.02;
    }
    Some(s)
}

fn commands(app: &App) -> Vec<(String, &'static str, &'static str, Act)> {
    let st = app.player.status();
    let o = app.player.st.lock().opts.clone();
    let cur = app.current_track();
    let mut v: Vec<(String, &'static str, &'static str, Act)> = vec![
        (if st.playing { "Pause".into() } else { "Play".into() }, "play pause resume", "▶", Act::Toggle),
        ("Next song".into(), "skip forward", "⏭", Act::Next),
        ("Previous song".into(), "back", "⏮", Act::Prev),
        ("Shuffle all songs".into(), "random everything", "🔀", Act::ShuffleAll),
        ("Shuffle liked songs".into(), "favorites hearts random", "♥", Act::ShuffleLiked),
    ];
    if let Some(t) = &cur {
        v.push((format!("More like \"{}\"", t.title), "radio similar discover recommend", "✨", Act::Radio));
        v.push((format!("{} \"{}\"", if app.lib.stat(&t.id).liked { "Unlike" } else { "Like" }, t.title), "heart favorite", "♥", Act::LikeCurrent));
        v.push(("Add current song to playlist…".into(), "save playing track playlists", "+", Act::PickCurrent));
    }
    v.extend([
        (format!("Shuffle: {}", if o.shuffle { "on > off" } else { "off > on" }), "toggle", "🔀", Act::ToggleShuffle),
        (format!("Repeat: {} > next mode", o.repeat), "loop toggle", "🔁", Act::CycleRepeat),
        (format!("Private listening: {}", if app.lib.private.load(std::sync::atomic::Ordering::Relaxed) { "on > off" } else { "off > on" }), "incognito stats history record secret", "🔒", Act::Private),
        ("Home".into(), "start discover mixes new releases recently played", "🏠", Act::View(View::Home)),
        ("THEATER: the song big, with its lyrics".into(), "now playing full screen fullscreen big cover lyrics f11 karaoke", "🗖", Act::NowPlaying),
        ("Discover new music".into(), "discover new recommendations similar radio explore", "🔍", Act::View(View::Discover)),
        ("Shazam: name the song playing on this PC".into(), "shazam identify recognize listen what song is this whats playing", "♫", Act::Listen),
        ("Find music online (songs, artists, albums, podcasts…)".into(), "search youtube web download get new song artist album playlist podcast audiobook profile clean explicit live instrumental version", "🌐", Act::View(View::Web)),
        ("Take the tour".into(), "tutorial help guide how to features walkthrough instructions", "💡", Act::Tour),
        ("Import music from a link".into(), "spotify youtube soundcloud download add", "📥", Act::View(View::Import)),
        ("Sync all imported playlists now".into(), "update refresh spotify", "🔄", Act::SyncAll),
        ("Open a playlist file from a friend (.dkfm)…".into(), "share shared import friend code", "📂", Act::OpenShare),
        ("Downloads".into(), "progress", "⬇", Act::View(View::Downloads)),
        ("Stats".into(), "wrapped top songs artists minutes history", "📊", Act::View(View::Stats)),
        ("Liked songs".into(), "favorites", "♥", Act::View(View::Liked)),
        ("All songs".into(), "library tracks", "♫", Act::View(View::All)),
        ("Albums".into(), "", "💿", Act::View(View::Albums)),
        ("Artists".into(), "", "👤", Act::View(View::Artists)),
        ("Mini player".into(), "small compact window", "🗖", Act::Mini),
        (if app.layout_edit { "Finish editing layout".into() } else { "Edit layout".into() }, "move panels arrange", "🔧", Act::Layout),
        ("Cycle visualizer".into(), "scope bars vu waterfall", "📈", Act::Visualizer),
        ("Rescan library".into(), "refresh folders", "🔄", Act::Rescan),
        ("Settings".into(), "preferences options", "⚙", Act::Settings),
        ("Undo last change".into(), "restore removed deleted songs playlist merge ctrl+z", "↩", Act::Undo),
    ]);
    let s = app.settings.lock().clone();
    for (k, name) in theme::all_themes(&s) {
        v.push((format!("Theme: {name}"), "color look appearance", "■", Act::Theme(k)));
    }
    v.push(("Reset layout".into(), "default panels restore missing broken", "■", Act::ResetLayout));
    for t in Tab::ALL.into_iter().filter(|t| app.dock.find_tab(t).is_none()) {
        v.push((format!("Show the {} panel", t.name().to_lowercase()), "panels layout open missing", "■", Act::Panel(t)));
    }
    for name in app.layout_names() {
        v.push((format!("Layout: {name}"), "panels arrange switch saved", "■", Act::ApplyLayout(name)));
    }
    v
}

fn search(app: &App, q: &str) -> Vec<Item> {
    let q = q.trim();
    let mut out = Vec::new();
    if (q.starts_with("http://") || q.starts_with("https://")) && crate::sources::detect(q).is_some() {
        out.push(Item { group: "IMPORT", label: "Import this link".into(), sub: q.into(), icon: "📥".into(), cover: None, song: None, act: Act::Import(q.into()) });
    }
    let toks: Vec<String> = q.to_lowercase().split_whitespace().map(String::from).collect();
    let cmds = commands(app);
    if toks.is_empty() {
        out.extend(cmds.into_iter().take(12).map(|(l, _, i, a)| Item { group: "ACTIONS", label: l, sub: String::new(), icon: i.into(), cover: None, song: None, act: a }));
        return out;
    }
    let mut acts: Vec<(f32, (String, &str, &str, Act))> = cmds.into_iter().filter_map(|c| score(&format!("{} {}", c.0.to_lowercase(), c.1), &toks).map(|s| (s, c))).collect();
    acts.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    out.extend(acts.into_iter().take(4).map(|(_, (l, _, i, a))| Item { group: "ACTIONS", label: l, sub: String::new(), icon: i.into(), cover: None, song: None, act: a }));

    let d = app.lib.data.read();
    let mut songs: Vec<(f32, &crate::store::Track)> = d.tracks.values().filter_map(|t| score(&format!("{} {} {}", t.title, t.artist, t.album).to_lowercase(), &toks).map(|s| (s + d.stats.get(&t.id).map(|x| x.plays as f32 * 0.5).unwrap_or(0.0), t))).collect();
    songs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let ids: Vec<String> = songs.iter().map(|s| s.1.id.clone()).collect();
    for (i, (_, t)) in songs.iter().take(8).enumerate() {
        out.push(Item { group: "SONGS", label: t.title.clone(), sub: t.artist.clone(), icon: "♫".into(), cover: t.thumb.clone().or(t.cover.clone()), song: Some(t.id.clone()), act: Act::PlaySong(ids.clone(), i) });
    }
    let mut albums: Vec<(String, &crate::store::Track)> = Vec::new();
    let mut artists: Vec<(String, String, &crate::store::Track)> = Vec::new();
    for t in d.tracks.values() {
        if !t.album.is_empty() && albums.len() < 4 && !albums.iter().any(|a| a.0 == album_key(t)) && score(&format!("{} {}", t.album, t.artist).to_lowercase(), &toks).is_some() {
            albums.push((album_key(t), t));
        }
        let a = main_artist(&t.artist);
        if artists.len() < 4 && !artists.iter().any(|x| x.0 == artist_key(t)) && score(&a.to_lowercase(), &toks).is_some() {
            artists.push((artist_key(t), a, t));
        }
    }
    for (k, t) in albums {
        out.push(Item { group: "ALBUMS", label: t.album.clone(), sub: t.artist.clone(), icon: "💿".into(), cover: t.thumb.clone().or(t.cover.clone()), song: None, act: Act::View(View::Album(k)) });
    }
    for (k, name, t) in artists {
        out.push(Item { group: "ARTISTS", label: name, sub: String::new(), icon: "👤".into(), cover: t.thumb.clone().or(t.cover.clone()), song: None, act: Act::View(View::Artist(k)) });
    }
    for p in d.playlists.iter().filter(|p| score(&p.name.to_lowercase(), &toks).is_some()).take(4) {
        out.push(Item { group: "PLAYLISTS", label: p.name.clone(), sub: format!("{} songs", p.track_ids.len()), icon: "☰".into(), cover: None, song: None, act: Act::View(View::Playlist(p.id.clone())) });
    }
    for f in d.folders.iter().filter(|f| score(&f.name.to_lowercase(), &toks).is_some()).take(2) {
        out.push(Item { group: "PLAYLISTS", label: format!("Play folder \"{}\"", f.name), sub: "every playlist in it".into(), icon: "📁".into(), cover: None, song: None, act: Act::PlayFolder(f.id.clone()) });
    }
    out
}

fn run(app: &mut App, ctx: &egui::Context, act: Act) {
    match act {
        Act::Toggle => app.player.toggle(),
        Act::OpenShare => super::sharing::pick_file(app),
        Act::Next => app.player.next(true),
        Act::Prev => app.player.prev(),
        Act::ShuffleAll => {
            let ids: Vec<String> = app.lib.data.read().tracks.keys().cloned().collect();
            app.player.play_list(ids, 0, Some(true));
        }
        Act::ShuffleLiked => {
            let ids: Vec<String> = app.lib.data.read().stats.iter().filter(|(_, s)| s.liked).map(|(k, _)| k.clone()).collect();
            if ids.is_empty() { app.toast("No liked songs yet") } else { app.player.play_list(ids, 0, Some(true)) }
        }
        Act::Radio => {
            if let Some(t) = app.current_track() {
                super::import::radio_for(app, &t);
            }
        }
        Act::PickCurrent => {} // handled in show(): switches the palette to the picker
        Act::LikeCurrent => {
            if let Some(id) = app.player.current_id() {
                app.lib.toggle_like(&id);
            }
        }
        Act::ToggleShuffle => app.player.toggle_shuffle(),
        Act::CycleRepeat => app.player.cycle_repeat(),
        Act::View(v) => app.browser.set_view(v),
        Act::Listen => super::websearch::listen(app, ctx),
        Act::Tour => super::tour::start(app),
        Act::Import(u) => {
            app.browser.set_view(View::Import);
            super::import::fetch_link(app, ctx, u);
        }
        Act::SyncAll => {
            app.toast("Syncing playlists…");
            let dl = app.dl.clone();
            std::thread::spawn(move || {
                let n = dl.sync_all(true);
                dl.notices.lock().push(format!("Sync done: {n} new song{}", if n == 1 { "" } else { "s" }));
            });
        }
        Act::Mini => app.toggle_mini(ctx),
        Act::Layout => app.toggle_layout_edit(),
        Act::Visualizer => app.scope.cycle(&app.player),
        Act::Rescan => {
            app.rescan();
            app.toast("Scanning library…");
        }
        Act::Settings => app.modal = Some(Modal::Settings(super::settings::SetTab::Look)),
        Act::Theme(k) => {
            app.edit_settings(|s| s.theme = k);
            app.set_theme(ctx);
        }
        Act::PlaySong(ids, i) => app.player.play_pick(ids, i, Some(false)),
        Act::Private => app.run_action(ctx, "private"),
        Act::Undo => app.undo(),
        Act::PlayFolder(id) => super::browser::play_folder(app, &id, false),
        Act::NowPlaying => app.nowplaying = !app.mini,
        Act::ResetLayout => app.reset_layout(),
        Act::Panel(t) => app.show_panel(t),
        Act::ApplyLayout(name) => {
            if app.apply_layout(&name) {
                app.toast(format!("Layout: {name}"));
            }
        }
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut st) = app.palette.take() else { return };
    if st.query.contains(crate::share::PREFIX) {
        super::sharing::receive(app, &st.query); // a friend's code: its preview replaces the palette
        return;
    }
    if st.pick.is_some() {
        if !show_pick(app, ctx, &mut st) {
            app.palette = Some(st);
        }
        return;
    }
    let pal = app.pal;
    let items = search(app, &st.query);
    st.active = st.active.min(items.len().saturating_sub(1));
    let (down, up, enter, esc, shift, alt, tab) = ctx.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Enter), i.key_pressed(Key::Escape), i.modifiers.shift, i.modifiers.alt, i.key_pressed(Key::Tab)));
    if down {
        st.active = (st.active + 1).min(items.len().saturating_sub(1));
    }
    if up {
        st.active = st.active.saturating_sub(1);
    }
    dim_screen(ctx);
    let mut chosen: Option<(usize, u8)> = if enter && !items.is_empty() { Some((st.active, if shift { 1 } else if alt { 2 } else { 0 })) } else { None };
    let mut pick: Option<String> = if tab { items.get(st.active).and_then(|it| it.song.clone()) } else { None };
    let mut close = esc;
    let w = width(ctx);
    let screen = ctx.screen_rect();
    egui::Area::new(Id::new("palette")).order(egui::Order::Foreground).fixed_pos(top_left(ctx, w)).show(ctx, |ui| {
        egui::Frame::new().fill(pal.panel).stroke(egui::Stroke::new(2.0_f32, pal.accent)).show(ui, |ui| {
            ui.set_width(w);
            let te = ui.add(egui::TextEdit::singleline(&mut st.query).hint_text("Search songs, albums, artists, playlists, actions — or paste a link").font(vt(24.0)).desired_width(w).margin(egui::Margin::symmetric(12, 10)).frame(false).lock_focus(true));
            if !st.focused {
                te.request_focus();
                st.focused = true;
            }
            if te.changed() {
                st.active = 0;
            }
            ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), egui::Stroke::new(2.0_f32, pal.line));
            egui::ScrollArea::vertical().max_height(screen.height() * 0.55).show(ui, |ui| {
                let mut group = "";
                for (i, it) in items.iter().enumerate() {
                    if it.group != group {
                        group = it.group;
                        ui.label(egui::RichText::new(format!("  {group}")).font(px(6.0)).color(pal.dim));
                    }
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::click());
                    if resp.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                        st.active = i;
                    }
                    let on = i == st.active;
                    if on {
                        fill(ui.painter(), r, pal.accent);
                        if down || up {
                            resp.scroll_to_me(None);
                        }
                    }
                    let ic = Rect::from_min_size(r.min + Vec2::new(8.0, 3.0), Vec2::splat(28.0));
                    fill(ui.painter(), ic, pal.bg);
                    frame_rect(ui.painter(), ic, 1.0, pal.line_hi);
                    let tex = it.cover.as_ref().and_then(|c| app.covers.get(ctx, app.lib.cover_path(c), c, 64));
                    match tex {
                        Some(t) => {
                            ui.painter().image(t, ic, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                        }
                        None => {
                            let color = if let Act::Theme(k) = &it.act { theme::theme_pal(&app.settings.lock(), k).accent } else { pal.accent2 }; // the icon box stays dark on the highlighted row too
                            ui.painter().text(ic.center(), Align2::CENTER_CENTER, &it.icon, vt(18.0), color);
                        }
                    }
                    let right = if it.song.is_some() && on { 190.0 } else { 70.0 };
                    let clip = ui.painter().with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.right() - right, r.bottom())));
                    let g = clip.layout_no_wrap(it.label.clone(), vt(20.0), if on { pal.ink } else { pal.text });
                    let gw = g.size().x;
                    clip.galley(Pos2::new(r.left() + 44.0, r.center().y - g.size().y / 2.0), g, pal.text);
                    if !it.sub.is_empty() {
                        clip.text(Pos2::new(r.left() + 52.0 + gw, r.center().y), Align2::LEFT_CENTER, &it.sub, vt(17.0), if on { pal.ink } else { pal.dim });
                    }
                    let mut plus_hit = false;
                    if it.song.is_some() {
                        // "+" = add to playlists (same as Tab)
                        let pr = Rect::from_center_size(Pos2::new(r.right() - 22.0, r.center().y), Vec2::new(26.0, 24.0));
                        let ph = ui.input(|i| i.pointer.hover_pos()).map(|q| pr.contains(q)).unwrap_or(false);
                        frame_rect(ui.painter(), pr, 2.0, if on { pal.ink } else if ph { pal.accent } else { pal.line_hi });
                        ui.painter().text(pr.center(), Align2::CENTER_CENTER, "+", vt(22.0), if on { pal.ink } else { pal.accent2 });
                        if on {
                            ui.painter().text(Pos2::new(r.right() - 44.0, r.center().y), Align2::RIGHT_CENTER, "ENTER play · TAB playlist", vt(16.0), pal.ink);
                        }
                        plus_hit = resp.clicked() && resp.interact_pointer_pos().map(|q| pr.contains(q)).unwrap_or(false);
                        if ph {
                            resp.clone().on_hover_text("Add to playlist (Tab)");
                        }
                    }
                    if plus_hit {
                        pick = it.song.clone();
                    } else if resp.clicked() {
                        chosen = Some((i, 0));
                    }
                }
                if items.is_empty() {
                    ui.label(egui::RichText::new("   NO MATCHES").color(pal.dim));
                }
            });
            ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), egui::Stroke::new(2.0_f32, pal.line));
            ui.label(egui::RichText::new("  UP/DOWN move · ENTER do it · SHIFT+ENTER queue · ALT+ENTER next · TAB playlist · ESC close").color(pal.dim));
        });
    });
    if ctx.input(|i| i.pointer.any_click()) && !ctx.is_pointer_over_area() {
        close = true;
    }
    if let Some((i, mode)) = chosen {
        let it = &items[i];
        match (&it.song, mode, &it.act) {
            (None, _, Act::PickCurrent) => pick = app.player.current_id(),
            (Some(id), 1, _) => app.player.enqueue(vec![id.clone()]),
            (Some(id), 2, _) => app.player.play_next(vec![id.clone()]),
            _ => run(app, ctx, it.act.clone()),
        }
        close = true;
    }
    if let Some(id) = pick {
        st.pick = Some(Picker::new(vec![id]));
        close = false;
    }
    if !close {
        app.palette = Some(st);
    }
}

fn width(ctx: &egui::Context) -> f32 {
    640.0f32.min(ctx.screen_rect().width() - 32.0)
}

fn top_left(ctx: &egui::Context, w: f32) -> Pos2 {
    let s = ctx.screen_rect();
    Pos2::new(s.center().x - w / 2.0, s.top() + s.height() * 0.12)
}

fn dim_screen(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, Id::new("pal-dim")));
    fill(&painter, ctx.screen_rect(), Color32::from_black_alpha(140));
}

/// Playlist-picker mode: type to filter, click to tick/untick, Enter adds & closes, Esc goes back.
/// Returns true when the palette should close.
fn show_pick(app: &mut App, ctx: &egui::Context, st: &mut PaletteState) -> bool {
    let pal = app.pal;
    let Some(mut pk) = st.pick.take() else { return false };
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        st.focused = false; // back to the search, focused again
        return false;
    }
    let title = pk.songs.first().and_then(|id| app.lib.track(id)).map(|t| format!("{} — {}", t.title, t.artist)).unwrap_or_default();
    dim_screen(ctx);
    let w = width(ctx);
    let max_h = ctx.screen_rect().height() * 0.5;
    let mut close = false;
    egui::Area::new(Id::new("palette")).order(egui::Order::Foreground).fixed_pos(top_left(ctx, w)).show(ctx, |ui| {
        egui::Frame::new().fill(pal.panel).stroke(egui::Stroke::new(2.0_f32, pal.accent)).show(ui, |ui| {
            ui.set_width(w);
            let te = ui.add(egui::TextEdit::singleline(&mut pk.filter).hint_text("Find a playlist…").font(vt(24.0)).desired_width(w).margin(egui::Margin::symmetric(12, 10)).frame(false).lock_focus(true));
            te.request_focus();
            if te.changed() {
                pk.active = 1;
            }
            ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), egui::Stroke::new(2.0_f32, pal.line));
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("ADD TO PLAYLIST").font(px(6.0)).color(pal.dim));
                ui.label(egui::RichText::new(&title).font(vt(18.0)).color(pal.accent2));
            });
            close = plpick::list(app, ui, &mut pk, w, max_h, true, true);
            ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), egui::Stroke::new(2.0_f32, pal.line));
            ui.label(egui::RichText::new("  UP/DOWN move · CLICK tick / untick · ENTER add & close · ESC back").color(pal.dim));
        });
    });
    if ctx.input(|i| i.pointer.any_click()) && !ctx.is_pointer_over_area() {
        close = true;
    }
    st.pick = Some(pk);
    close || app.modal.is_some()
}

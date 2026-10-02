//! Library browser: sources sidebar + views (songs, albums, artists, playlists, stats, import,
//! downloads). Song lists are virtualised (only visible rows are laid out).
use super::theme::{px, vt};
use super::widgets::{button, fill, fmt_long, fmt_time, frame_rect, with_alpha};
use super::{App, Modal, PromptAction};
use crate::library::main_artist;
use crate::store::Track;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Ui, Vec2};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum View {
    All,
    Liked,
    Top,
    Recent,
    Albums,
    Artists,
    Album(String),
    Artist(String),
    Playlist(String),
    Stats,
    Import,
    Downloads,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum SortKey {
    TrackNo,
    Title,
    Artist,
    Album,
    Duration,
    Plays,
    Added,
}

pub struct BrowserState {
    pub view: View,
    pub search: String,
    pub sort: Option<(SortKey, bool)>,
    pub selection: HashSet<String>,
    anchor: Option<usize>,
    list: Vec<String>,
    list_key: Option<(View, String, Option<(SortKey, bool)>, u64)>,
    pub focus_search: bool,
    groups_key: u64,
    albums: Vec<(String, String, String, Option<String>, Option<u32>, usize)>, // key, name, artist, cover, year, n
    artists: Vec<(String, String, Option<String>, usize)>,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self { view: View::All, search: String::new(), sort: None, selection: HashSet::new(), anchor: None, list: Vec::new(), list_key: None, focus_search: false, groups_key: 0, albums: Vec::new(), artists: Vec::new() }
    }
}

impl BrowserState {
    pub fn set_view(&mut self, v: View) {
        self.view = v;
        self.search.clear();
        self.sort = None;
        self.selection.clear();
        self.anchor = None;
    }
}

pub fn album_key(t: &Track) -> String {
    format!("{}|{}", if t.album_artist.is_empty() { &t.artist } else { &t.album_artist }.to_lowercase(), t.album.to_lowercase())
}
pub fn artist_key(t: &Track) -> String {
    main_artist(&t.artist).to_lowercase()
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let full = ui.available_rect_before_wrap();
    let side_w = 182.0;
    let side = Rect::from_min_size(full.min, Vec2::new(side_w, full.height()));
    let main = Rect::from_min_max(Pos2::new(full.left() + side_w, full.top()), full.max);
    fill(ui.painter(), side, pal.bg2);
    ui.painter().vline(side.right(), side.y_range(), egui::Stroke::new(2.0_f32, pal.line));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(side), |ui| sidebar(app, ui));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(main.shrink2(Vec2::new(2.0, 0.0))), |ui| {
        ui.set_clip_rect(main);
        match app.browser.view.clone() {
            View::Albums => albums_view(app, ui),
            View::Artists => artists_view(app, ui),
            View::Stats => super::stats::show(app, ui),
            View::Import => super::import::show(app, ui),
            View::Downloads => super::import::downloads(app, ui),
            _ => tracks_view(app, ui),
        }
    });
    let _ = pal;
}

// ------------------------------------------------------------------------------- sidebar

fn sidebar(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let (n_all, n_liked, playlists) = {
        let d = app.lib.data.read();
        (d.tracks.len(), d.stats.values().filter(|s| s.liked).count(), d.playlists.clone())
    };
    let active_dl = app.dl.active_count();
    egui::ScrollArea::vertical().id_salt("side").auto_shrink([false; 2]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let group = |ui: &mut Ui, s: &str| {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(format!("  {s}")).font(px(6.0)).color(pal.dim));
            ui.add_space(3.0);
        };
        let mut go: Option<View> = None;
        let mut item = |ui: &mut Ui, app: &App, v: Option<View>, ico: &str, label: &str, cnt: Option<usize>| -> egui::Response {
            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
            let active = v.as_ref() == Some(&app.browser.view);
            let p = ui.painter();
            if active {
                fill(p, r, pal.sel);
                fill(p, Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), pal.accent);
            } else if resp.hovered() {
                fill(p, r, pal.panel_hi);
            }
            p.text(r.min + Vec2::new(14.0, 12.0), Align2::CENTER_CENTER, ico, vt(18.0), pal.accent2);
            let clip = p.with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.right() - 30.0, r.bottom())));
            clip.text(r.min + Vec2::new(28.0, 12.0), Align2::LEFT_CENTER, label, vt(19.0), if active { pal.accent } else { pal.text });
            if let Some(c) = cnt {
                p.text(Pos2::new(r.right() - 8.0, r.center().y), Align2::RIGHT_CENTER, c.to_string(), vt(16.0), pal.dim);
            }
            if resp.clicked() {
                if let Some(v) = v {
                    go = Some(v);
                }
            }
            resp
        };
        group(ui, "LOCAL");
        item(ui, app, Some(View::All), "♫", "All Tracks", Some(n_all));
        item(ui, app, Some(View::Liked), "♥", "Liked", Some(n_liked));
        item(ui, app, Some(View::Top), "★", "Most Played", None);
        item(ui, app, Some(View::Recent), "🕘", "Recently Added", None);
        item(ui, app, Some(View::Albums), "💿", "Albums", None);
        item(ui, app, Some(View::Artists), "👤", "Artists", None);
        item(ui, app, Some(View::Stats), "📊", "Stats", None);
        if item(ui, app, None, "+", "Add music folder", None).clicked() {
            add_folder(app);
        }
        group(ui, "IMPORTED");
        item(ui, app, Some(View::Import), "📥", "Import Music", None);
        item(ui, app, Some(View::Downloads), "⬇", "Downloads", if active_dl > 0 { Some(active_dl) } else { None });
        for p in playlists.iter().filter(|p| p.is_imported()) {
            let ico = match p.source.as_deref() { Some("youtube") => "▶", Some("soundcloud") => "☁", _ => "♪" };
            let r = item(ui, app, Some(View::Playlist(p.id.clone())), ico, &p.name, Some(p.track_ids.len()));
            let pl = p.clone();
            r.context_menu(|ui| playlist_menu_ui(app, ui, &pl));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("  PLAYLISTS").font(px(6.0)).color(pal.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Label::new(egui::RichText::new("+ ").font(vt(20.0)).color(pal.dim)).sense(Sense::click())).clicked() {
                    app.modal = Some(Modal::Prompt { title: "NEW PLAYLIST".into(), text: "My Playlist".into(), action: PromptAction::NewPlaylist(vec![]) });
                }
            });
        });
        ui.add_space(3.0);
        let user: Vec<_> = playlists.iter().filter(|p| !p.is_imported()).cloned().collect();
        for p in &user {
            let r = item(ui, app, Some(View::Playlist(p.id.clone())), "☰", &p.name, Some(p.track_ids.len()));
            let pl = p.clone();
            r.context_menu(|ui| playlist_menu_ui(app, ui, &pl));
        }
        if user.is_empty() && item(ui, app, None, "+", "New playlist", None).clicked() {
            app.modal = Some(Modal::Prompt { title: "NEW PLAYLIST".into(), text: "My Playlist".into(), action: PromptAction::NewPlaylist(vec![]) });
        }
        if let Some(v) = go {
            app.browser.set_view(v);
        }
    });
}


pub fn add_folder(app: &mut App) {
    if let Some(f) = rfd::FileDialog::new().set_title("Add a music folder").pick_folder() {
        let s = f.to_string_lossy().into_owned();
        app.edit_settings(|st| {
            if !st.music_folders.contains(&s) {
                st.music_folders.push(s.clone());
            }
        });
        app.toast(format!("Scanning {s}…"));
        app.rescan();
    }
}

pub fn playlist_menu_ui(app: &mut App, ui: &mut Ui, p: &crate::store::Playlist) {
    let ids: Vec<String> = p.track_ids.iter().filter(|id| app.lib.track(id).is_some()).cloned().collect();
    if ui.button("Play").clicked() {
        app.player.play_list(ids.clone(), 0, Some(false));
        ui.close_menu();
    }
    if ui.button("Shuffle play").clicked() {
        app.player.play_list(ids.clone(), 0, Some(true));
        ui.close_menu();
    }
    if ui.button("Add to queue").clicked() {
        app.player.enqueue(ids.clone());
        ui.close_menu();
    }
    ui.separator();
    if ui.button("Rename…").clicked() {
        app.modal = Some(Modal::Prompt { title: "RENAME PLAYLIST".into(), text: p.name.clone(), action: PromptAction::RenamePlaylist(p.id.clone()) });
        ui.close_menu();
    }
    if p.is_imported() {
        if ui.button("Sync now").clicked() {
            let (dl, pl) = (app.dl.clone(), p.clone());
            app.toast(format!("Syncing \"{}\"…", p.name));
            std::thread::spawn(move || {
                let r = dl.sync_one(&pl);
                dl.notices.lock().push(match r {
                    Ok(0) => format!("\"{}\" is up to date", pl.name),
                    Ok(n) => format!("{n} new song{} downloading", if n == 1 { "" } else { "s" }),
                    Err(e) => e,
                });
            });
            ui.close_menu();
        }
        let on = p.auto_sync != Some(false);
        if ui.button(format!("Auto-sync: {}", if on { "ON ✔" } else { "OFF" })).clicked() {
            app.lib.edit_playlist(&p.id, |x| x.auto_sync = Some(!on));
            ui.close_menu();
        }
        if ui.button("Open original").clicked() {
            if let Some(u) = p.url() {
                let _ = open::that(u);
            }
            ui.close_menu();
        }
    }
    if ui.button("Delete playlist").clicked() {
        app.lib.delete_playlist(&p.id);
        if app.browser.view == View::Playlist(p.id.clone()) {
            app.browser.set_view(View::All);
        }
        ui.close_menu();
    }
}

// ------------------------------------------------------------------------------- song lists

fn sort_ids(app: &App, ids: &mut Vec<String>, s: Option<(SortKey, bool)>) {
    let Some((k, asc)) = s else { return };
    let d = app.lib.data.read();
    let key = |id: &String| -> (String, f64) {
        let t = &d.tracks[id];
        let st = d.stats.get(id).cloned().unwrap_or_default();
        match k {
            SortKey::Title => (t.title.to_lowercase(), 0.0),
            SortKey::Artist => (format!("{}\0{}\0{:03}", t.artist.to_lowercase(), t.album.to_lowercase(), t.track.unwrap_or(0)), 0.0),
            SortKey::Album => (format!("{}\0{:03}", t.album.to_lowercase(), t.track.unwrap_or(0)), 0.0),
            SortKey::TrackNo => (format!("{:04}{}", t.track.unwrap_or(999), t.title.to_lowercase()), 0.0),
            SortKey::Duration => (String::new(), t.duration),
            SortKey::Plays => (String::new(), st.plays as f64),
            SortKey::Added => (String::new(), t.added_at),
        }
    };
    let mut keyed: Vec<((String, f64), String)> = ids.drain(..).map(|id| (key(&id), id)).collect();
    keyed.sort_by(|a, b| a.0 .0.cmp(&b.0 .0).then(a.0 .1.partial_cmp(&b.0 .1).unwrap_or(std::cmp::Ordering::Equal)));
    if !asc {
        keyed.reverse();
    }
    *ids = keyed.into_iter().map(|x| x.1).collect();
}

struct ListCfg {
    title: String,
    sub: String,
    cover: Option<String>,
    remote_cover: Option<String>,
    playlist: Option<crate::store::Playlist>,
    default_sort: Option<(SortKey, bool)>,
    back: Option<View>,
}

fn tracks_view(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
    let view = app.browser.view.clone();
    let cfg = {
        let d = app.lib.data.read();
        let all = || d.tracks.values();
        let now = crate::store::now_ms();
        let (ids, cfg): (Vec<String>, ListCfg) = match &view {
            View::All => (all().map(|t| t.id.clone()).collect(), ListCfg { title: "ALL TRACKS".into(), sub: String::new(), cover: None, remote_cover: None, playlist: None, default_sort: Some((SortKey::Artist, true)), back: None }),
            View::Liked => (all().filter(|t| d.stats.get(&t.id).map(|s| s.liked).unwrap_or(false)).map(|t| t.id.clone()).collect(), ListCfg { title: "LIKED".into(), sub: String::new(), cover: None, remote_cover: None, playlist: None, default_sort: Some((SortKey::Added, false)), back: None }),
            View::Top => (all().filter(|t| d.stats.get(&t.id).map(|s| s.plays > 0).unwrap_or(false)).map(|t| t.id.clone()).collect(), ListCfg { title: "MOST PLAYED".into(), sub: String::new(), cover: None, remote_cover: None, playlist: None, default_sort: Some((SortKey::Plays, false)), back: None }),
            View::Recent => (all().filter(|t| now - t.added_at < 60.0 * 86400.0 * 1000.0).map(|t| t.id.clone()).collect(), ListCfg { title: "RECENTLY ADDED".into(), sub: String::new(), cover: None, remote_cover: None, playlist: None, default_sort: Some((SortKey::Added, false)), back: None }),
            View::Album(k) => {
                let ids: Vec<String> = all().filter(|t| album_key(t) == *k).map(|t| t.id.clone()).collect();
                let first = ids.first().and_then(|i| d.tracks.get(i));
                (ids.clone(), ListCfg { title: first.map(|t| t.album.clone()).unwrap_or_default(), sub: first.map(|t| if t.album_artist.is_empty() { t.artist.clone() } else { t.album_artist.clone() }).unwrap_or_default(), cover: first.and_then(|t| t.cover.clone()), remote_cover: None, playlist: None, default_sort: Some((SortKey::TrackNo, true)), back: Some(View::Albums) })
            }
            View::Artist(k) => {
                let ids: Vec<String> = all().filter(|t| artist_key(t) == *k).map(|t| t.id.clone()).collect();
                let name = ids.first().and_then(|i| d.tracks.get(i)).map(|t| main_artist(&t.artist)).unwrap_or_default();
                (ids, ListCfg { title: name, sub: String::new(), cover: None, remote_cover: None, playlist: None, default_sort: Some((SortKey::Album, true)), back: Some(View::Artists) })
            }
            View::Playlist(pid) => {
                let Some(p) = d.playlists.iter().find(|p| p.id == *pid).cloned() else {
                    drop(d);
                    app.browser.set_view(View::All);
                    return;
                };
                let ids: Vec<String> = p.track_ids.iter().filter(|id| d.tracks.contains_key(*id)).cloned().collect();
                let sub = if p.is_imported() { format!("{} · {}", p.source.clone().unwrap_or_else(|| "spotify".into()).to_uppercase(), if p.auto_sync == Some(false) { "SYNC OFF" } else { "AUTO-SYNC" }) } else { "PLAYLIST".into() };
                (ids, ListCfg { title: p.name.clone(), sub, cover: None, remote_cover: p.cover.clone(), playlist: Some(p), default_sort: None, back: None })
            }
            _ => (Vec::new(), ListCfg { title: String::new(), sub: String::new(), cover: None, remote_cover: None, playlist: None, default_sort: None, back: None }),
        };
        let key = (view.clone(), app.browser.search.clone(), app.browser.sort.or(cfg.default_sort), gen);
        if app.browser.list_key.as_ref() != Some(&key) {
            let toks: Vec<String> = app.browser.search.to_lowercase().split_whitespace().map(String::from).collect();
            let mut ids: Vec<String> = ids
                .into_iter()
                .filter(|id| {
                    if toks.is_empty() {
                        return true;
                    }
                    let t = &d.tracks[id];
                    let hay = format!("{} {} {} {}", t.title, t.artist, t.album, t.genre).to_lowercase();
                    toks.iter().all(|k| hay.contains(k))
                })
                .collect();
            drop(d);
            sort_ids(app, &mut ids, key.2);
            app.browser.list = ids;
            app.browser.list_key = Some(key);
        }
        cfg
    };
    let ids = app.browser.list.clone();
    let active_sort = app.browser.sort.or(cfg.default_sort);
    let secs: f64 = ids.iter().filter_map(|id| app.lib.data.read().tracks.get(id).map(|t| t.duration)).sum();

    // ---- header
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if let Some(b) = cfg.back.clone() {
                if button(ui, &pal, "◀ BACK", false, true).clicked() {
                    app.browser.set_view(b);
                }
            }
            if let Some(c) = &cfg.cover {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                if let Some(tex) = app.covers.get(ui.ctx(), app.lib.cover_path(c), c, 128) {
                    ui.painter().image(tex, r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
                frame_rect(ui.painter(), r, 2.0, pal.line_hi);
            }
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&cfg.title).font(px(12.0)).color(pal.text));
                let sub = [cfg.sub.clone(), format!("{} TRACKS", ids.len()), fmt_long(secs)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                ui.label(egui::RichText::new(sub).color(pal.dim));
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if button(ui, &pal, "▶ PLAY", true, !ids.is_empty()).clicked() {
                app.player.play_list(ids.clone(), 0, Some(false));
            }
            if button(ui, &pal, "SHUFFLE", false, !ids.is_empty()).clicked() {
                app.player.play_list(ids.clone(), fastrand::usize(..ids.len().max(1)), Some(true));
            }
            if let Some(p) = cfg.playlist.clone() {
                let r = button(ui, &pal, "…", false, true);
                let id = ui.make_persistent_id("pl-menu");
                if r.clicked() {
                    ui.memory_mut(|m| m.toggle_popup(id));
                }
                egui::popup::popup_below_widget(ui, id, &r, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    ui.set_min_width(180.0);
                    playlist_menu_ui(app, ui, &p);
                });
            }
            let te = ui.add(egui::TextEdit::singleline(&mut app.browser.search).hint_text("SEARCH…").desired_width(200.0).font(vt(19.0)));
            if app.browser.focus_search {
                te.request_focus();
                app.browser.focus_search = false;
            }
        });
    });
    if app.lib.data.read().tracks.is_empty() || ids.is_empty() && app.browser.search.is_empty() {
        empty_state(app, ui);
        return;
    }

    // ---- columns
    let w = ui.available_width();
    let narrow = w < 640.0;
    let fixed = 44.0 + 24.0 + 56.0 + if narrow { 0.0 } else { 44.0 } + 16.0;
    let flex = (w - fixed - 8.0 * 6.0).max(120.0);
    let (tw, aw, alw) = if narrow { (flex * 0.6, flex * 0.4, 0.0) } else { (flex * 3.0 / 7.0, flex * 2.0 / 7.0, flex * 2.0 / 7.0) };
    let cols: Vec<(&str, Option<SortKey>, f32, bool)> = {
        let mut c = vec![("#", Some(SortKey::TrackNo), 44.0, true), ("", None, 24.0, false), ("TITLE", Some(SortKey::Title), tw, false), ("ARTIST", Some(SortKey::Artist), aw, false)];
        if !narrow {
            c.push(("ALBUM", Some(SortKey::Album), alw, false));
        }
        c.push(("TIME", Some(SortKey::Duration), 56.0, true));
        if !narrow {
            c.push(("PLAYS", Some(SortKey::Plays), 44.0, true));
        }
        c
    };
    let (hr, _) = ui.allocate_exact_size(Vec2::new(w, 24.0), Sense::hover());
    ui.painter().hline(hr.x_range(), hr.bottom(), egui::Stroke::new(2.0_f32, pal.line));
    let mut x = hr.left() + 8.0;
    for (label, key, cw, right) in &cols {
        let cr = Rect::from_min_size(Pos2::new(x, hr.top()), Vec2::new(*cw, hr.height()));
        let sorted = key.is_some() && active_sort.map(|s| Some(s.0) == *key).unwrap_or(false);
        let txt = format!("{label}{}", if sorted { if active_sort.unwrap().1 { " ▲" } else { " ▼" } } else { "" });
        let resp = ui.interact(cr, ui.id().with(("th", *label)), Sense::click());
        ui.painter().text(if *right { Pos2::new(cr.right(), cr.center().y) } else { Pos2::new(cr.left(), cr.center().y) }, if *right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }, txt, px(6.0), if sorted { pal.accent } else if resp.hovered() { pal.text } else { pal.dim });
        if resp.clicked() {
            if let Some(k) = key {
                let asc = !matches!(active_sort, Some((kk, true)) if kk == *k);
                app.browser.sort = Some((*k, asc));
            }
        }
        x += cw + 8.0;
    }

    // ---- rows (virtualised)
    let cur = app.player.current_id();
    let mut act: Option<RowAct> = None;
    let num_mode = matches!(active_sort, Some((SortKey::TrackNo, _)));
    egui::ScrollArea::vertical().id_salt(("tracks", format!("{view:?}"))).auto_shrink([false; 2]).show_rows(ui, 28.0, ids.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in range {
            let id = &ids[i];
            let Some(t) = app.lib.track(id) else { continue };
            let st = app.lib.stat(id);
            let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
            let sel = app.browser.selection.contains(id);
            let p = ui.painter();
            if sel {
                fill(p, r, pal.sel);
            } else if resp.hovered() {
                fill(p, r, pal.panel_hi);
            } else if i % 2 == 1 {
                fill(p, r, with_alpha(pal.text, 2));
            }
            let is_cur = cur.as_deref() == Some(id.as_str());
            let mut x = r.left() + 8.0;
            let cy = r.center().y;
            for (label, _, cw, right) in &cols {
                let cr = Rect::from_min_max(Pos2::new(x, r.top()), Pos2::new(x + cw, r.bottom()));
                let clip = p.with_clip_rect(cr);
                let (txt, color): (String, Color32) = match *label {
                    "#" => (if is_cur { "▶".into() } else if num_mode && t.track.is_some() { t.track.unwrap().to_string() } else { (i + 1).to_string() }, if is_cur { pal.accent } else { pal.dim }),
                    "" => ("♥".into(), if st.liked { pal.accent } else { pal.faint }),
                    "TITLE" => (t.title.clone(), if is_cur { pal.accent } else { pal.text }),
                    "ARTIST" => (t.artist.clone(), pal.dim),
                    "ALBUM" => (t.album.clone(), pal.dim),
                    "TIME" => (fmt_time(t.duration), pal.dim),
                    _ => (if st.plays > 0 { st.plays.to_string() } else { String::new() }, pal.dim),
                };
                clip.text(if *right { Pos2::new(cr.right(), cy) } else if label.is_empty() { Pos2::new(cr.center().x, cy) } else { Pos2::new(cr.left(), cy) }, if *right { Align2::RIGHT_CENTER } else if label.is_empty() { Align2::CENTER_CENTER } else { Align2::LEFT_CENTER }, txt, vt(19.0), color);
                if label.is_empty() && resp.clicked() && resp.interact_pointer_pos().map(|pp| cr.contains(pp)).unwrap_or(false) {
                    act = Some(RowAct::Like(id.clone()));
                }
                x += cw + 8.0;
            }
            if resp.clicked() && act.is_none() {
                let m = ui.input(|i| i.modifiers);
                act = Some(RowAct::Select(i, m.shift, m.command));
            }
            if resp.double_clicked() {
                act = Some(RowAct::Play(i));
            }
            if resp.secondary_clicked() && !app.browser.selection.contains(id) {
                app.browser.selection = HashSet::from([id.clone()]);
                app.browser.anchor = Some(i);
            }
            resp.context_menu(|ui| {
                ui.set_min_width(210.0);
                track_menu(app, ui, &ids, i, cfg.playlist.as_ref());
            });
        }
    });
    match act {
        Some(RowAct::Like(id)) => app.lib.toggle_like(&id),
        Some(RowAct::Play(i)) => app.player.play_list(ids.clone(), i, None),
        Some(RowAct::Select(i, shift, cmd)) => {
            let id = ids[i].clone();
            if shift && app.browser.anchor.is_some() {
                let a = app.browser.anchor.unwrap();
                if !cmd {
                    app.browser.selection.clear();
                }
                for k in a.min(i)..=a.max(i) {
                    app.browser.selection.insert(ids[k].clone());
                }
            } else if cmd {
                if !app.browser.selection.remove(&id) {
                    app.browser.selection.insert(id);
                }
                app.browser.anchor = Some(i);
            } else {
                app.browser.selection = HashSet::from([id]);
                app.browser.anchor = Some(i);
            }
        }
        None => {}
    }
    // keyboard: Ctrl+A / Enter / Delete
    if !ui.ctx().wants_keyboard_input() && app.palette.is_none() && app.modal.is_none() {
        let (all, enter, del) = ui.input(|i| (i.modifiers.command && i.key_pressed(egui::Key::A), i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Delete)));
        if all {
            app.browser.selection = ids.iter().cloned().collect();
        }
        if enter {
            if let Some(i) = ids.iter().position(|x| app.browser.selection.contains(x)) {
                app.player.play_list(ids.clone(), i, None);
            }
        }
        if del {
            if let Some(p) = &cfg.playlist {
                let sel = app.browser.selection.clone();
                app.lib.edit_playlist(&p.id, |pl| pl.track_ids.retain(|x| !sel.contains(x)));
                app.browser.selection.clear();
            }
        }
    }
}

enum RowAct {
    Select(usize, bool, bool),
    Play(usize),
    Like(String),
}

fn track_menu(app: &mut App, ui: &mut Ui, ids: &[String], i: usize, playlist: Option<&crate::store::Playlist>) {
    let sel: Vec<String> = ids.iter().filter(|x| app.browser.selection.contains(*x)).cloned().collect();
    let sel = if sel.is_empty() { vec![ids[i].clone()] } else { sel };
    let one = if sel.len() == 1 { app.lib.track(&sel[0]) } else { None };
    if ui.button("▶ Play").clicked() {
        app.player.play_list(ids.to_vec(), i, None);
        ui.close_menu();
    }
    if ui.button("Play next").clicked() {
        app.player.play_next(sel.clone());
        ui.close_menu();
    }
    if ui.button("Add to queue").clicked() {
        app.player.enqueue(sel.clone());
        ui.close_menu();
    }
    if ui.add_enabled(one.is_some(), egui::Button::new("✨ More like this")).clicked() {
        if let Some(t) = &one {
            super::import::radio_for(app, t);
        }
        ui.close_menu();
    }
    ui.menu_button("Add to playlist", |ui| {
        if ui.button("+ New playlist…").clicked() {
            app.modal = Some(Modal::Prompt { title: "NEW PLAYLIST".into(), text: "My Playlist".into(), action: PromptAction::NewPlaylist(sel.clone()) });
            ui.close_menu();
        }
        let pls: Vec<_> = app.lib.data.read().playlists.iter().filter(|p| !p.is_imported()).cloned().collect();
        if !pls.is_empty() {
            ui.separator();
        }
        for p in pls {
            if ui.button(&p.name).clicked() {
                let fresh: Vec<String> = sel.iter().filter(|x| !p.track_ids.contains(x)).cloned().collect();
                let n = fresh.len();
                app.lib.edit_playlist(&p.id, |pl| pl.track_ids.extend(fresh));
                app.toast(format!("Added {n} song{} to \"{}\"", if n == 1 { "" } else { "s" }, p.name));
                ui.close_menu();
            }
        }
    });
    ui.separator();
    let liked = one.as_ref().map(|t| app.lib.stat(&t.id).liked).unwrap_or(false);
    if ui.button(if liked { "Unlike" } else { "Like" }).clicked() {
        for id in &sel {
            app.lib.toggle_like(id);
        }
        ui.close_menu();
    }
    if ui.add_enabled(one.as_ref().map(|t| !t.album.is_empty()).unwrap_or(false), egui::Button::new("Go to album")).clicked() {
        if let Some(t) = &one {
            app.browser.set_view(View::Album(album_key(t)));
        }
        ui.close_menu();
    }
    if ui.add_enabled(one.is_some(), egui::Button::new("Go to artist")).clicked() {
        if let Some(t) = &one {
            app.browser.set_view(View::Artist(artist_key(t)));
        }
        ui.close_menu();
    }
    if ui.button("Show in folder").clicked() {
        if let Some(t) = app.lib.track(&sel[0]) {
            super::queue::reveal(&t.path);
        }
        ui.close_menu();
    }
    ui.separator();
    if let Some(p) = playlist {
        if ui.button(format!("Remove from \"{}\"", p.name)).clicked() {
            let s = sel.clone();
            app.lib.edit_playlist(&p.id, |pl| pl.track_ids.retain(|x| !s.contains(x)));
            app.browser.selection.clear();
            ui.close_menu();
        }
    }
    if ui.button(format!("Remove from library{}", if sel.len() > 1 { format!(" ({})", sel.len()) } else { String::new() })).clicked() {
        for id in &sel {
            app.lib.remove_track(id);
        }
        app.browser.selection.clear();
        ui.close_menu();
    }
}

fn empty_state(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        let lib_empty = app.lib.data.read().tracks.is_empty();
        let (title, msg) = match (&app.browser.view, lib_empty) {
            (View::Playlist(_), false) => ("EMPTY PLAYLIST", "Right-click songs > Add to playlist."),
            (_, false) => ("NOTHING HERE YET", ""),
            _ => ("YOUR LIBRARY IS EMPTY", "Add a music folder, drop audio files on the window, or import a playlist."),
        };
        ui.label(egui::RichText::new(title).font(px(10.0)).color(pal.accent));
        ui.add_space(8.0);
        ui.label(egui::RichText::new(msg).color(pal.dim));
        if lib_empty {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 360.0).max(0.0) / 2.0);
                if button(ui, &pal, "ADD MUSIC FOLDER", true, true).clicked() {
                    add_folder(app);
                }
                if button(ui, &pal, "IMPORT MUSIC", false, true).clicked() {
                    app.browser.set_view(View::Import);
                }
            });
        }
    });
}

// ------------------------------------------------------------------------------- grids

fn rebuild_groups(app: &mut App) {
    let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
    if app.browser.groups_key == gen {
        return;
    }
    app.browser.groups_key = gen;
    let d = app.lib.data.read();
    let mut albums: std::collections::HashMap<String, (String, String, Option<String>, Option<u32>, usize)> = Default::default();
    let mut artists: std::collections::HashMap<String, (String, Option<String>, usize)> = Default::default();
    for t in d.tracks.values() {
        if !t.album.is_empty() {
            let e = albums.entry(album_key(t)).or_insert_with(|| (t.album.clone(), if t.album_artist.is_empty() { t.artist.clone() } else { t.album_artist.clone() }, None, t.year, 0));
            e.4 += 1;
            if e.2.is_none() {
                e.2 = t.thumb.clone().or(t.cover.clone());
            }
        }
        let e = artists.entry(artist_key(t)).or_insert_with(|| (main_artist(&t.artist), None, 0));
        e.2 += 1;
        if e.1.is_none() {
            e.1 = t.thumb.clone().or(t.cover.clone());
        }
    }
    let mut al: Vec<_> = albums.into_iter().map(|(k, v)| (k, v.0, v.1, v.2, v.3, v.4)).collect();
    al.sort_by(|a, b| a.2.to_lowercase().cmp(&b.2.to_lowercase()).then(a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let mut ar: Vec<_> = artists.into_iter().map(|(k, v)| (k, v.0, v.1, v.2)).collect();
    ar.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    drop(d);
    app.browser.albums = al;
    app.browser.artists = ar;
}

fn grid(app: &mut App, ui: &mut Ui, title: &str, items: Vec<(String, String, String, Option<String>)>, mk: impl Fn(String) -> View) {
    let pal = app.pal;
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.label(egui::RichText::new(title).font(px(12.0)).color(pal.text));
        ui.label(egui::RichText::new(format!("{} {}", items.len(), title)).color(pal.dim));
    });
    let mut go = None;
    egui::ScrollArea::vertical().id_salt(title).auto_shrink([false; 2]).show(ui, |ui| {
        let w = ui.available_width() - 24.0;
        let n = ((w + 14.0) / 164.0).floor().max(1.0) as usize;
        let cw = (w - 14.0 * (n as f32 - 1.0)) / n as f32;
        egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
            for row in items.chunks(n) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    for (key, c1, c2, cover) in row {
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(cw, cw + 46.0), Sense::click());
                        let art = Rect::from_min_size(r.min, Vec2::splat(cw));
                        if ui.is_rect_visible(r) {
                            let p = ui.painter();
                            fill(p, art.translate(Vec2::splat(4.0)), pal.shadow);
                            fill(p, art, pal.bg);
                            let tex = cover.as_ref().and_then(|c| app.covers.get(ui.ctx(), app.lib.cover_path(c), c, 192));
                            match tex {
                                Some(t) => {
                                    ui.painter().image(t, art, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                                }
                                None => {
                                    ui.painter().text(art.center(), Align2::CENTER_CENTER, "♫", vt(48.0), pal.faint);
                                }
                            }
                            frame_rect(ui.painter(), art, 2.0, if resp.hovered() { pal.accent } else { pal.line_hi });
                            let clip = ui.painter().with_clip_rect(r);
                            clip.text(Pos2::new(r.left(), art.bottom() + 8.0), Align2::LEFT_TOP, c1, vt(19.0), pal.text);
                            clip.text(Pos2::new(r.left(), art.bottom() + 26.0), Align2::LEFT_TOP, c2, vt(16.0), pal.dim);
                        }
                        if resp.clicked() {
                            go = Some(mk(key.clone()));
                        }
                    }
                });
                ui.add_space(14.0);
            }
        });
    });
    if let Some(v) = go {
        app.browser.set_view(v);
    }
}

fn albums_view(app: &mut App, ui: &mut Ui) {
    rebuild_groups(app);
    let items = app.browser.albums.iter().map(|a| (a.0.clone(), a.1.clone(), format!("{}{}", a.2, a.4.map(|y| format!(" · {y}")).unwrap_or_default()), a.3.clone())).collect();
    grid(app, ui, "ALBUMS", items, View::Album);
}

fn artists_view(app: &mut App, ui: &mut Ui) {
    rebuild_groups(app);
    let items = app.browser.artists.iter().map(|a| (a.0.clone(), a.1.clone(), format!("{} TRACK{}", a.3, if a.3 == 1 { "" } else { "S" }), a.2.clone())).collect();
    grid(app, ui, "ARTISTS", items, View::Artist);
}

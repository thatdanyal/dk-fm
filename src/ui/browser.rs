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
    Home,
    All,
    Liked,
    Top,
    Recent,
    Albums,
    Artists,
    Album(String),
    Artist(String),
    /// an artist's songs as a list (from their page)
    ArtistSongs(String),
    /// a "Made from your library" mix on Home
    Mix(String),
    Playlist(String),
    Stats,
    Import,
    Downloads,
    Duplicates,
    /// new music picked from what you play
    Discover,
    /// FIND SONGS: search YouTube for versions of a song to download
    Web,
}

/// Songs being dragged (to reorder a playlist, or onto a sidebar playlist).
pub struct DragSongs(pub Vec<String>);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum SortKey {
    TrackNo,
    Title,
    Artist,
    Album,
    Duration,
    Plays,
    Added,
    Year,
    Genre,
    Bitrate,
}

pub const SORTS: [(SortKey, &str, &str); 10] = [
    (SortKey::TrackNo, "num", "Track number"), (SortKey::Title, "title", "Title"), (SortKey::Artist, "artist", "Artist"), (SortKey::Album, "album", "Album"),
    (SortKey::Duration, "time", "Length"), (SortKey::Plays, "plays", "Plays"), (SortKey::Added, "added", "Date added"), (SortKey::Year, "year", "Year"),
    (SortKey::Genre, "genre", "Genre"), (SortKey::Bitrate, "bitrate", "Bitrate"),
];

/// "artist" = A-Z, "-plays" = most first, "" = the list's own order.
pub fn parse_sort(s: &str) -> Option<(SortKey, bool)> {
    let (name, asc) = match s.strip_prefix('-') { Some(n) => (n, false), None => (s, true) };
    SORTS.iter().find(|x| x.1 == name).map(|x| (x.0, asc))
}
pub fn sort_text(s: Option<(SortKey, bool)>) -> String {
    s.and_then(|(k, asc)| SORTS.iter().find(|x| x.0 == k).map(|x| format!("{}{}", if asc { "" } else { "-" }, x.1))).unwrap_or_default()
}

/// Screens with their own default sort: (key, name, built-in default).
pub const SCREENS: [(&str, &str, &str); 7] = [("all", "All Tracks", "artist"), ("liked", "Liked", "-added"), ("top", "Most Played", "-plays"), ("recent", "Recently Added", "-added"), ("album", "An album", "num"), ("artist", "An artist", "album"), ("playlist", "A playlist", "")];

fn default_sort(app: &App, screen: &str) -> Option<(SortKey, bool)> {
    let s = app.settings.lock();
    parse_sort(s.sorts.get(screen).map(|x| x.as_str()).unwrap_or_else(|| SCREENS.iter().find(|x| x.0 == screen).map(|x| x.2).unwrap_or("")))
}

/// Track list columns: (id, header, fixed width or 0 = shares the free space by `share`,
/// right-aligned, sort, name in Settings).
pub struct Col {
    pub id: &'static str,
    head: &'static str,
    width: f32,
    share: f32,
    right: bool,
    sort: Option<SortKey>,
    pub name: &'static str,
    /// still shown when the list is narrow
    narrow: bool,
}
const fn col(id: &'static str, head: &'static str, width: f32, share: f32, right: bool, sort: Option<SortKey>, name: &'static str, narrow: bool) -> Col {
    Col { id, head, width, share, right, sort, name, narrow }
}
pub const COLUMNS: [Col; 11] = [
    col("num", "#", 44.0, 0.0, true, Some(SortKey::TrackNo), "# (position / track number)", true),
    col("like", "", 24.0, 0.0, false, None, "♥ Like", true),
    col("title", "TITLE", 0.0, 3.0, false, Some(SortKey::Title), "Title", true),
    col("artist", "ARTIST", 0.0, 2.0, false, Some(SortKey::Artist), "Artist", true),
    col("album", "ALBUM", 0.0, 2.0, false, Some(SortKey::Album), "Album", false),
    col("genre", "GENRE", 0.0, 1.5, false, Some(SortKey::Genre), "Genre", false),
    col("year", "YEAR", 44.0, 0.0, true, Some(SortKey::Year), "Year", false),
    col("time", "TIME", 56.0, 0.0, true, Some(SortKey::Duration), "Length", true),
    col("plays", "PLAYS", 44.0, 0.0, true, Some(SortKey::Plays), "Plays", false),
    col("bitrate", "KBPS", 48.0, 0.0, true, Some(SortKey::Bitrate), "Bitrate", false),
    col("added", "ADDED", 84.0, 0.0, true, Some(SortKey::Added), "Date added", false),
];

/// Screens in the tab strip above the library (the sidebar lists only playlists):
/// (id, place, icon, tab label, name in Settings). "tab" = the tabs on the left, "more" = the
/// buttons at the right end (Import, Downloads, and the ⋯ menu for the rest).
pub const NAV: [(&str, &str, &str, &str, &str); 13] = [
    ("home", "tab", "🏠", "HOME", "Home"), ("discover", "tab", "🧭", "DISCOVER", "Discover new music"), ("web", "tab", "🌐", "FIND SONGS", "Find songs online"), ("all", "tab", "♫", "SONGS", "All songs"), ("albums", "tab", "💿", "ALBUMS", "Albums"), ("artists", "tab", "👤", "ARTISTS", "Artists"),
    ("recent", "tab", "🕘", "RECENT", "Recently added"), ("top", "tab", "★", "TOP", "Most played"), ("stats", "tab", "📊", "STATS", "Stats"),
    ("import", "more", "📥", "+ IMPORT", "Import music"), ("downloads", "more", "⬇", "⬇", "Downloads"), ("dupes", "more", "📋", "Duplicates", "Duplicates"), ("folder", "more", "+", "Add music folder…", "Add music folder"),
];

/// Tabs in your order (unknown ids dropped, new ones put in at their default place).
pub fn nav_order(order: &[String]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = order.iter().filter_map(|o| NAV.iter().find(|s| s.0 == o).map(|s| s.0)).collect();
    for (i, s) in NAV.iter().enumerate() {
        if !v.contains(&s.0) {
            v.insert(i.min(v.len()), s.0);
        }
    }
    v
}

/// The tab a screen belongs to (a playlist or Liked has none).
fn nav_of(v: &View) -> Option<&'static str> {
    Some(match v {
        View::Home | View::Mix(_) => "home",
        View::All => "all",
        View::Albums | View::Album(_) => "albums",
        View::Artists | View::Artist(_) | View::ArtistSongs(_) => "artists",
        View::Recent => "recent",
        View::Top => "top",
        View::Stats => "stats",
        View::Import => "import",
        View::Downloads => "downloads",
        View::Duplicates => "dupes",
        View::Web => "web",
        View::Discover => "discover",
        View::Liked | View::Playlist(_) => return None,
    })
}

/// Screens DK.FM can open to: (key, name).
pub const START: [(&str, &str); 9] = [("home", "Home"), ("all", "All Tracks"), ("liked", "Liked"), ("top", "Most Played"), ("recent", "Recently Added"), ("albums", "Albums"), ("artists", "Artists"), ("stats", "Stats"), ("import", "Import Music")];

pub fn view_for(key: &str) -> Option<View> {
    Some(match key {
        "home" => View::Home,
        "all" => View::All,
        "liked" => View::Liked,
        "top" => View::Top,
        "recent" => View::Recent,
        "albums" => View::Albums,
        "artists" => View::Artists,
        "stats" => View::Stats,
        "dupes" => View::Duplicates,
        "import" => View::Import,
        "downloads" => View::Downloads,
        "web" => View::Web,
        "discover" => View::Discover,
        _ => return None,
    })
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
    pub dupes: super::dupes::DupeState,
    pub add: super::addsongs::AddBox,
    pub web: super::websearch::WebState,
    side_key: Option<(u64, u64, String)>,
    side_rows: Vec<SideRow>,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self { view: View::All, search: String::new(), sort: None, selection: HashSet::new(), anchor: None, list: Vec::new(), list_key: None, focus_search: false, groups_key: 0, albums: Vec::new(), artists: Vec::new(), dupes: Default::default(), add: Default::default(), web: Default::default(), side_key: None, side_rows: Vec::new() }
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
    let side_w = (full.width() * 0.3).clamp(132.0, 184.0).round();
    let side = Rect::from_min_size(full.min, Vec2::new(side_w, full.height()));
    let main = Rect::from_min_max(Pos2::new(full.left() + side_w, full.top()), full.max);
    fill(ui.painter(), side, pal.bg2);
    ui.painter().vline(side.right(), side.y_range(), egui::Stroke::new(2.0_f32, pal.line));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(side), |ui| sidebar(app, ui));
    let strip_h = nav_strip(app, ui, Rect::from_min_max(Pos2::new(main.left() + 2.0, main.top()), main.max));
    let main = Rect::from_min_max(Pos2::new(main.left(), main.top() + strip_h), main.max);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(main.shrink2(Vec2::new(2.0, 0.0))), |ui| {
        ui.set_clip_rect(main);
        match app.browser.view.clone() {
            View::Home => super::home::show(app, ui),
            View::Artist(k) => super::home::artist_page(app, ui, &k),
            View::Albums => albums_view(app, ui),
            View::Artists => artists_view(app, ui),
            View::Stats => super::stats::show(app, ui),
            View::Import => super::import::show(app, ui),
            View::Downloads => super::import::downloads(app, ui),
            View::Duplicates => super::dupes::show(app, ui),
            View::Web => super::websearch::show(app, ui),
            View::Discover => super::explore::show(app, ui),
            _ => tracks_view(app, ui),
        }
    });
    let _ = pal;
}

// ------------------------------------------------------------------------------- tab strip

/// The tabs along the top of the library (HOME · SONGS · ...), with Import, Downloads and a ⋯
/// menu at the right end. Tabs wrap onto more rows when the panel is narrow. Returns its height.
fn nav_strip(app: &mut App, ui: &mut Ui, area: Rect) -> f32 {
    let pal = app.pal;
    let (order, hidden) = { let s = app.settings.lock(); (nav_order(&s.sidebar_order), s.sidebar_hidden.clone()) };
    let shown: Vec<_> = order.iter().filter_map(|id| NAV.iter().find(|n| n.0 == *id)).filter(|n| !hidden.iter().any(|h| h == n.0)).collect();
    let cur = nav_of(&app.browser.view);
    let active_dl = app.dl.active_count();
    let (font, row_h, pad) = (px(7.0), 30.0, 6.0);
    let text_w = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), pal.text).size().x;
    // right end: [+ IMPORT] [⬇ n] [⋯]
    let mut right: Vec<(&str, f32)> = Vec::new();
    for n in shown.iter().filter(|n| n.1 == "more") {
        match n.0 {
            "import" => right.push(("import", text_w(n.3) + 16.0)),
            "downloads" => right.push(("downloads", 30.0 + if active_dl > 0 { text_w(&active_dl.to_string()) + 10.0 } else { 0.0 })),
            _ => {}
        }
    }
    right.push(("more", 30.0));
    let right_w: f32 = right.iter().map(|r| r.1).sum::<f32>() + 4.0 * (right.len() - 1) as f32;
    // tabs flow left to right; the first row leaves room for the right end
    let mut tabs: Vec<(&str, &str, Rect)> = Vec::new();
    let (mut x, mut y) = (area.left() + pad, area.top() + 4.0);
    let mut limit = area.right() - pad - right_w - 6.0;
    for n in shown.iter().filter(|n| n.1 == "tab") {
        let w = text_w(n.3) + 14.0;
        if x + w > limit && x > area.left() + pad {
            (x, y, limit) = (area.left() + pad, y + row_h, area.right() - pad);
        }
        tabs.push((n.0, n.3, Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, row_h))));
        x += w;
    }
    let h = (y - area.top()) + row_h + 6.0;
    let strip = Rect::from_min_size(area.min, Vec2::new(area.width(), h));
    ui.painter().hline(strip.x_range(), strip.bottom() - 1.0, egui::Stroke::new(2.0_f32, pal.line));
    let mut go: Option<&str> = None;
    for (id, label, r) in &tabs {
        let resp = ui.interact(*r, ui.id().with(("nav", *id)), Sense::click());
        let on = cur == Some(*id);
        if on {
            fill(ui.painter(), r.shrink2(Vec2::new(0.0, 2.0)), pal.sel);
            fill(ui.painter(), Rect::from_min_max(Pos2::new(r.left(), r.bottom() - 3.0), r.max), pal.accent);
        } else if resp.hovered() {
            fill(ui.painter(), r.shrink2(Vec2::new(0.0, 2.0)), pal.panel_hi);
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *label, font.clone(), if on { pal.accent } else if resp.hovered() { pal.text } else { pal.dim });
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            go = Some(id);
        }
    }
    let mut rx = area.right() - pad - right_w;
    let ry = area.top() + 7.0;
    for (id, w) in right {
        let r = Rect::from_min_size(Pos2::new(rx, ry), Vec2::new(w, row_h - 6.0));
        rx += w + 4.0;
        let resp = ui.interact(r, ui.id().with(("nav", id)), Sense::click());
        let on = cur == Some(id) || id == "more" && cur == Some("dupes");
        let p = ui.painter();
        if on {
            fill(p, r, pal.accent);
        } else {
            fill(p, r, pal.panel_hi);
            frame_rect(p, r, 2.0, if resp.hovered() { pal.accent } else { pal.line_hi });
        }
        let fg = if on { pal.ink } else if resp.hovered() { pal.accent } else { pal.text };
        match id {
            "import" => {
                p.text(r.center(), Align2::CENTER_CENTER, "+ IMPORT", font.clone(), fg);
            }
            "downloads" => {
                p.text(Pos2::new(r.left() + 15.0, r.center().y), Align2::CENTER_CENTER, "⬇", vt(18.0), fg);
                if active_dl > 0 {
                    let b = Rect::from_min_max(Pos2::new(r.left() + 26.0, r.top() + 5.0), Pos2::new(r.right() - 5.0, r.bottom() - 5.0));
                    fill(p, b, if on { pal.ink } else { pal.accent });
                    p.text(b.center(), Align2::CENTER_CENTER, active_dl.to_string(), px(6.0), if on { pal.accent } else { pal.ink });
                }
            }
            _ => {
                for i in -1..=1 {
                    fill(p, Rect::from_center_size(r.center() + Vec2::new(i as f32 * 6.0, 0.0), Vec2::splat(3.0)), fg);
                }
            }
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let resp = match id {
            "import" => resp.on_hover_text("Import music: Spotify, YouTube, SoundCloud links or your own files"),
            "downloads" => resp.on_hover_text(if active_dl > 0 { format!("Downloads ({active_dl} active)") } else { "Downloads".into() }),
            _ => resp.on_hover_text("More"),
        };
        if id != "more" {
            if resp.clicked() {
                go = Some(id);
            }
            continue;
        }
        let pid = ui.make_persistent_id("nav-more");
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(pid));
        }
        egui::popup::popup_below_widget(ui, pid, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(190.0);
            for n in shown.iter().filter(|n| n.1 == "more" && !matches!(n.0, "import" | "downloads")) {
                if ui.button(format!("{}  {}", n.2, n.3)).clicked() {
                    go = Some(n.0);
                }
            }
            // a tab or button you hid is still one click away here
            let hid: Vec<_> = NAV.iter().filter(|n| hidden.iter().any(|h| h == n.0) && !matches!(n.0, "dupes" | "folder")).collect();
            if !hid.is_empty() {
                ui.separator();
                for n in hid {
                    if ui.button(format!("{}  {}", n.2, n.4)).clicked() {
                        go = Some(n.0);
                    }
                }
            }
            ui.separator();
            if ui.button("Customize tabs…").clicked() {
                app.modal = Some(Modal::Settings(super::settings::SetTab::Sidebar));
            }
        });
    }
    match go {
        Some("folder") => add_folder(app),
        Some(id) => {
            if let Some(v) = view_for(id) {
                app.browser.set_view(v);
            }
        }
        None => {}
    }
    h
}

// ------------------------------------------------------------------------------- sidebar

/// A playlist or folder being dragged in the sidebar (to reorder, pin, or file it).
#[derive(Clone)]
enum SideDrag {
    Playlist(String),
    Folder(String),
}

/// A row of the sidebar's playlist list (rebuilt only when the library changes).
#[derive(Clone)]
pub struct SideRow {
    is_folder: bool,
    id: String,
    name: String,
    icon: &'static str,
    n: usize,
    /// the folder a playlist row is listed in
    folder: Option<String>,
    pinned: bool,
    cover: Option<String>,
}

/// Sidebar sort options: (key, name).
pub const PL_SORTS: [(&str, &str); 4] = [("custom", "Custom order (drag)"), ("name", "Name"), ("added", "Recently added"), ("played", "Recently played")];

/// Pinned playlists first, then folders (with their playlists unless collapsed), then the rest.
fn side_rows(app: &mut App, sort: &str) -> Vec<SideRow> {
    let d = app.lib.data.read();
    let pls = crate::library::sorted_playlists(&d, sort);
    let folder_of = |p: &crate::store::Playlist| p.folder.clone().filter(|f| d.folders.iter().any(|x| x.id == *f));
    let covers = &mut app.plcovers;
    let mut row = |p: &crate::store::Playlist, folder: Option<String>| SideRow { is_folder: false, id: p.id.clone(), name: p.name.clone(), icon: playlist_icon(p), n: p.track_ids.len(), folder, pinned: p.pinned, cover: covers.get(&app.lib, &d, p) };
    let mut rows: Vec<SideRow> = pls.iter().filter(|p| p.pinned).map(|p| row(p, folder_of(p))).collect();
    for f in crate::library::sorted_folders(&d, sort) {
        let inside: Vec<&&crate::store::Playlist> = pls.iter().filter(|p| folder_of(p).as_deref() == Some(f.id.as_str())).collect();
        rows.push(SideRow { is_folder: true, id: f.id.clone(), name: f.name.clone(), icon: if f.collapsed { "📁" } else { "📂" }, n: inside.len(), folder: None, pinned: false, cover: None });
        if !f.collapsed {
            rows.extend(inside.into_iter().filter(|p| !p.pinned).map(|p| row(p, Some(f.id.clone()))));
        }
    }
    rows.extend(pls.iter().filter(|p| !p.pinned && folder_of(p).is_none()).map(|p| row(p, None)));
    rows
}

/// Everything a sidebar drop can do.
enum SideAct {
    Place { pid: String, folder: Option<Option<String>>, pinned: bool, near: Option<(String, bool)> },
    MoveFolder(String, Option<String>),
}

fn sidebar(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let n_liked = app.lib.data.read().stats.values().filter(|s| s.liked).count();
    let (sort, thumbs) = { let s = app.settings.lock(); (s.playlist_sort.clone(), s.sidebar_covers) };
    let custom = sort == "custom";
    let key = (app.lib.gen.load(std::sync::atomic::Ordering::Relaxed), app.plcovers.stamp(), sort.clone());
    if app.browser.side_key.as_ref() != Some(&key) {
        app.browser.side_rows = side_rows(app, &sort);
        app.browser.side_key = Some(key);
    }
    let rows = app.browser.side_rows.clone();
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut go: Option<View> = None;
    let mut drops: Vec<(View, Vec<String>)> = Vec::new();
    // one sidebar row: `indent` px, a cover thumbnail instead of the icon, draggable, pinned mark
    let mut item = |ui: &mut Ui, app: &mut App, v: Option<View>, ico: &str, label: &str, cnt: Option<usize>, indent: f32, cover: Option<&str>, drag: Option<SideDrag>, pin: bool| -> egui::Response {
        let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), if drag.is_some() { Sense::click_and_drag() } else { Sense::click() });
        let active = v.as_ref() == Some(&app.browser.view);
        let droppable = matches!(v, Some(View::Playlist(_)) | Some(View::Liked));
        if droppable && resp.dnd_hover_payload::<DragSongs>().is_some() {
            fill(ui.painter(), r, pal.sel);
            frame_rect(ui.painter(), r, 2.0, pal.accent);
        } else if active {
            fill(ui.painter(), r, pal.sel);
            fill(ui.painter(), Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), pal.accent);
        } else if resp.hovered() {
            fill(ui.painter(), r, pal.panel_hi);
        }
        let x = r.left() + indent;
        let tex = cover.and_then(|c| app.covers.get(ui.ctx(), app.lib.cover_path(c), c, 48));
        let p = ui.painter();
        match tex {
            Some(t) => {
                p.image(t, Rect::from_center_size(Pos2::new(x + 14.0, r.center().y), Vec2::splat(18.0)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.text(Pos2::new(x + 14.0, r.top() + 12.0), Align2::CENTER_CENTER, ico, vt(18.0), pal.accent2);
            }
        }
        let right = r.right() - if pin { 44.0 } else { 30.0 };
        let clip = p.with_clip_rect(Rect::from_min_max(r.min, Pos2::new(right, r.bottom())));
        clip.text(Pos2::new(x + 28.0, r.top() + 12.0), Align2::LEFT_CENTER, label, vt(19.0), if active { pal.accent } else { pal.text });
        if pin {
            p.text(Pos2::new(r.right() - 34.0, r.center().y), Align2::CENTER_CENTER, "📌", vt(14.0), pal.accent2);
        }
        if let Some(c) = cnt {
            p.text(Pos2::new(r.right() - 8.0, r.center().y), Align2::RIGHT_CENTER, c.to_string(), vt(16.0), pal.dim);
        }
        // (a release check takes the payload even when it's another type, so check the type first)
        if droppable && egui::DragAndDrop::has_payload_of_type::<DragSongs>(ui.ctx()) {
            if let Some(d) = resp.dnd_release_payload::<DragSongs>() {
                drops.push((v.clone().unwrap(), d.0.clone()));
            }
        }
        if let Some(d) = drag {
            if resp.drag_started() {
                resp.dnd_set_drag_payload(d);
            }
        }
        if resp.clicked() {
            if let Some(v) = v {
                go = Some(v);
            }
        }
        resp
    };
    // header (as tall as one row of tabs, so the two lines meet): PLAYLISTS · ↕ sort/folders · + new
    let (hr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
    ui.painter().hline(hr.x_range(), hr.bottom() - 1.0, egui::Stroke::new(2.0_f32, pal.line));
    ui.painter().text(Pos2::new(hr.left() + 10.0, hr.center().y), Align2::LEFT_CENTER, "PLAYLISTS", px(7.0), pal.accent2);
    let hbtn = |ui: &mut Ui, x: f32, txt: &str, size: f32, on: bool, tip: &str, id: &str| {
        let r = Rect::from_center_size(Pos2::new(x, hr.center().y), Vec2::splat(24.0));
        let resp = ui.interact(r, ui.id().with(id), Sense::click()).on_hover_text(tip);
        if resp.hovered() {
            frame_rect(ui.painter(), r, 2.0, pal.line_hi);
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, txt, vt(size), if on { pal.accent } else if resp.hovered() { pal.text } else { pal.dim });
        resp
    };
    if hbtn(ui, hr.right() - 18.0, "+", 24.0, false, "New playlist", "pl-new").clicked() {
        app.modal = Some(Modal::Prompt { title: "NEW PLAYLIST".into(), text: "My Playlist".into(), action: PromptAction::NewPlaylist(vec![]) });
    }
    let r = hbtn(ui, hr.right() - 44.0, "↕", 15.0, !custom, "Sort playlists · new folder", "pl-sortb");
    let id = ui.make_persistent_id("pl-sort");
    if r.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup::popup_below_widget(ui, id, &r, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(190.0);
        ui.label(egui::RichText::new("SORT BY").font(px(6.0)).color(pal.dim));
        for (k, name) in PL_SORTS {
            if ui.button(format!("{} {name}", if sort == k { "•" } else { "  " })).clicked() {
                app.edit_settings(|s| s.playlist_sort = k.to_string());
            }
        }
        ui.separator();
        if ui.button("New folder…").clicked() {
            app.modal = Some(Modal::Prompt { title: "NEW FOLDER".into(), text: "New Folder".into(), action: PromptAction::NewFolder(None) });
        }
    });
    ui.add_space(3.0);
    let mut acts: Vec<SideAct> = Vec::new();
    let mut toggle: Option<String> = None;
    let dragging = egui::DragAndDrop::payload::<SideDrag>(ui.ctx());
    let take = |r: &egui::Response| if dragging.is_some() { r.dnd_release_payload::<SideDrag>() } else { None };
    egui::ScrollArea::vertical().id_salt("side").auto_shrink([false; 2]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        item(ui, app, Some(View::Liked), "♥", "Liked", Some(n_liked), 0.0, None, None, false);
        for row in &rows {
            if row.is_folder {
                let resp = item(ui, app, None, row.icon, &row.name, Some(row.n), 0.0, None, Some(SideDrag::Folder(row.id.clone())), false);
                let r = resp.rect;
                if resp.clicked() {
                    toggle = Some(row.id.clone());
                }
                match resp.dnd_hover_payload::<SideDrag>().as_deref() {
                    Some(SideDrag::Playlist(_)) => frame_rect(ui.painter(), r, 2.0, pal.accent),
                    Some(SideDrag::Folder(f)) if custom && *f != row.id => {
                        ui.painter().hline(r.x_range(), r.top(), egui::Stroke::new(3.0_f32, pal.accent));
                    }
                    _ => {}
                }
                match take(&resp).as_deref() {
                    Some(SideDrag::Playlist(pid)) => acts.push(SideAct::Place { pid: pid.clone(), folder: Some(Some(row.id.clone())), pinned: false, near: None }),
                    Some(SideDrag::Folder(f)) if custom => acts.push(SideAct::MoveFolder(f.clone(), Some(row.id.clone()))),
                    _ => {}
                }
                let (fid, name) = (row.id.clone(), row.name.clone());
                resp.context_menu(|ui| folder_menu_ui(app, ui, &fid, &name));
                continue;
            }
            let indent = if row.folder.is_some() && !row.pinned { 12.0 } else { 0.0 };
            let resp = item(ui, app, Some(View::Playlist(row.id.clone())), row.icon, &row.name, Some(row.n), indent, if thumbs { row.cover.as_deref() } else { None }, Some(SideDrag::Playlist(row.id.clone())), row.pinned);
            let r = resp.rect;
            // dropping a playlist here: into this row's place (pinned / folder), and in custom
            // order also just above or below it
            let below = ui.ctx().pointer_hover_pos().map(|p| p.y > r.center().y).unwrap_or(false);
            if let Some(SideDrag::Playlist(pid)) = resp.dnd_hover_payload::<SideDrag>().as_deref() {
                if *pid != row.id {
                    if custom {
                        ui.painter().with_clip_rect(r.expand(2.0)).hline(r.x_range(), if below { r.bottom() } else { r.top() }, egui::Stroke::new(3.0_f32, pal.accent));
                    } else {
                        frame_rect(ui.painter(), r, 2.0, pal.accent);
                    }
                }
            }
            if let Some(SideDrag::Playlist(pid)) = take(&resp).as_deref() {
                // a pinned row keeps the dragged playlist's own folder
                let folder = if row.pinned { None } else { Some(row.folder.clone()) };
                acts.push(SideAct::Place { pid: pid.clone(), folder, pinned: row.pinned, near: custom.then(|| (row.id.clone(), below)) });
            }
            let pid = row.id.clone();
            resp.context_menu(|ui| {
                let pl = app.lib.data.read().playlists.iter().find(|p| p.id == pid).cloned();
                if let Some(pl) = pl {
                    playlist_menu_ui(app, ui, &pl);
                }
            });
        }
        let none_yours = app.lib.data.read().playlists.iter().all(|p| p.is_imported());
        if none_yours && item(ui, app, None, "+", "New playlist", None, 0.0, None, None, false).clicked() {
            app.modal = Some(Modal::Prompt { title: "NEW PLAYLIST".into(), text: "My Playlist".into(), action: PromptAction::NewPlaylist(vec![]) });
        }
        // the empty space below: drop here to take a playlist out of its folder / unpin it
        let h = ui.available_height().max(28.0);
        let (zr, zresp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
        if dragging.is_some() && zresp.dnd_hover_payload::<SideDrag>().is_some() {
            ui.painter().hline(zr.x_range(), zr.top() + 1.0, egui::Stroke::new(3.0_f32, pal.accent));
            ui.painter().text(Pos2::new(zr.left() + 28.0, zr.top() + 12.0), Align2::LEFT_CENTER, "move here (no folder)", vt(16.0), pal.dim);
        }
        match take(&zresp).as_deref() {
            Some(SideDrag::Playlist(pid)) => acts.push(SideAct::Place { pid: pid.clone(), folder: Some(None), pinned: false, near: custom.then(|| (String::new(), true)) }),
            Some(SideDrag::Folder(f)) if custom => acts.push(SideAct::MoveFolder(f.clone(), None)),
            _ => {}
        }
    });
    // what's being dragged follows the pointer
    if let (Some(d), Some(pos)) = (dragging, ui.ctx().pointer_hover_pos()) {
        let name = match &*d {
            SideDrag::Playlist(id) | SideDrag::Folder(id) => rows.iter().find(|r| r.id == *id).map(|r| format!("{} {}", r.icon, r.name)).unwrap_or_default(),
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag-side")));
        let g = p.layout_no_wrap(name, vt(19.0), pal.ink);
        let r = Rect::from_min_size(pos + Vec2::new(14.0, 6.0), g.size() + Vec2::new(14.0, 6.0));
        fill(&p, r, pal.accent);
        p.galley(r.min + Vec2::new(7.0, 3.0), g, pal.ink);
    }
    if let Some(f) = toggle {
        app.lib.edit_folder(&f, |f| f.collapsed = !f.collapsed);
    }
    for a in acts {
        match a {
            SideAct::Place { pid, folder, pinned, near } => {
                let folder = folder.unwrap_or_else(|| app.lib.data.read().playlists.iter().find(|p| p.id == pid).and_then(|p| p.folder.clone()));
                app.lib.place_playlist(&pid, folder, pinned, near.as_ref().map(|(o, after)| (o.as_str(), *after)));
            }
            SideAct::MoveFolder(f, before) => app.lib.move_folder(&f, before.as_deref()),
        }
    }
    if let Some(v) = go {
        app.browser.set_view(v);
    }
    for (v, ids) in drops {
        drop_songs(app, &v, &ids);
    }
}

/// Every song in a folder's playlists (in sidebar order, each song once, hidden ones left out).
pub fn play_folder(app: &mut App, fid: &str, shuffle: bool) {
    let sort = app.settings.lock().playlist_sort.clone();
    let mut ids: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let pls = app.lib.folder_playlists(fid, &sort);
    {
        let d = app.lib.data.read();
        for p in pls {
            ids.extend(p.track_ids.iter().filter(|t| d.tracks.contains_key(*t) && seen.insert((*t).clone())).cloned());
        }
    }
    if ids.is_empty() {
        app.toast("That folder has no songs yet");
        return;
    }
    let start = if shuffle { fastrand::usize(..ids.len()) } else { 0 };
    app.player.play_list(ids, start, Some(shuffle));
}

fn folder_menu_ui(app: &mut App, ui: &mut Ui, fid: &str, name: &str) {
    if ui.button("Play folder").clicked() {
        play_folder(app, fid, false);
        ui.close_menu();
    }
    if ui.button("Shuffle play").clicked() {
        play_folder(app, fid, true);
        ui.close_menu();
    }
    ui.separator();
    if ui.button("Rename folder…").clicked() {
        app.modal = Some(Modal::Prompt { title: "RENAME FOLDER".into(), text: name.into(), action: PromptAction::RenameFolder(fid.into()) });
        ui.close_menu();
    }
    if ui.button("New folder…").clicked() {
        app.modal = Some(Modal::Prompt { title: "NEW FOLDER".into(), text: "New Folder".into(), action: PromptAction::NewFolder(None) });
        ui.close_menu();
    }
    if ui.button("Delete folder").on_hover_text("Its playlists stay").clicked() {
        app.lib.delete_folder(fid);
        app.toast(format!("Deleted folder \"{name}\" (its playlists are kept)"));
        ui.close_menu();
    }
}

pub fn playlist_icon(p: &crate::store::Playlist) -> &'static str {
    match (p.url(), p.source.as_deref()) {
        (Some("spotify:liked"), _) => "♥",
        (_, Some("youtube")) => "▶",
        (_, Some("soundcloud")) => "☁",
        _ if p.is_imported() => "♪",
        _ => "☰",
    }
}

/// Songs dropped on a sidebar entry: add to that playlist, or like them.
fn drop_songs(app: &mut App, v: &View, ids: &[String]) {
    let s = |n: usize| if n == 1 { "" } else { "s" };
    match v {
        View::Liked => {
            app.lib.set_liked(ids, true);
            app.toast(format!("Liked {} song{}", ids.len(), s(ids.len())));
        }
        View::Playlist(pid) => {
            let name = app.lib.data.read().playlists.iter().find(|p| p.id == *pid).map(|p| p.name.clone()).unwrap_or_default();
            let n = app.lib.playlist_add(pid, ids);
            app.toast(if n == 0 { format!("Already in \"{name}\"") } else { format!("Added {n} song{} to \"{name}\"", s(n)) });
        }
        _ => {}
    }
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
        app.lib.touch_playlist(&p.id);
        ui.close_menu();
    }
    if ui.button("Shuffle play").clicked() {
        app.player.play_list(ids.clone(), fastrand::usize(..ids.len().max(1)), Some(true));
        app.lib.touch_playlist(&p.id);
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
    if ui.button(if p.description.is_empty() { "Add description…" } else { "Edit description…" }).clicked() {
        app.modal = Some(Modal::Prompt { title: "DESCRIPTION".into(), text: p.description.clone(), action: PromptAction::Describe(p.id.clone()) });
        ui.close_menu();
    }
    super::plcover::menu(app, ui, p);
    if ui.button(if p.pinned { "Unpin" } else { "Pin to top" }).clicked() {
        app.lib.place_playlist(&p.id, p.folder.clone(), !p.pinned, None);
        ui.close_menu();
    }
    let folders = app.lib.data.read().folders.clone();
    ui.menu_button("Move to folder", |ui| {
        for f in &folders {
            let here = p.folder.as_deref() == Some(f.id.as_str());
            if ui.button(format!("{} {}", if here { "•" } else { "  " }, f.name)).clicked() {
                app.lib.place_playlist(&p.id, Some(f.id.clone()), p.pinned, None);
                ui.close_menu();
            }
        }
        if p.folder.is_some() && ui.button("  No folder").clicked() {
            app.lib.place_playlist(&p.id, None, p.pinned, None);
            ui.close_menu();
        }
        if !folders.is_empty() {
            ui.separator();
        }
        if ui.button("New folder…").clicked() {
            app.modal = Some(Modal::Prompt { title: "NEW FOLDER".into(), text: "New Folder".into(), action: PromptAction::NewFolder(Some(p.id.clone())) });
            ui.close_menu();
        }
    });
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
    ui.separator();
    if ui.button("Share (copy code)").on_hover_text("A code to paste to a friend: they press Ctrl+V in DK.FM").clicked() {
        let sh = super::sharing::playlist_share(app, p);
        super::sharing::copy(app, ui.ctx(), &sh, &format!("playlist \"{}\"", p.name));
        ui.close_menu();
    }
    if ui.button("Export playlist file…").on_hover_text("A .dkfm file for big playlists: your friend drops it onto DK.FM").clicked() {
        ui.close_menu();
        super::sharing::export_file(app, p);
    }
    if ui.button("Export as M3U…").on_hover_text("For other players and devices").clicked() {
        ui.close_menu();
        super::sharing::export_m3u(app, p);
    }
    ui.separator();
    if ui.button("Delete playlist").clicked() {
        let u = app.lib.snapshot(format!("Deleted playlist \"{}\"", p.name), std::slice::from_ref(&p.id), &[]);
        app.lib.delete_playlist(&p.id);
        app.undoable(u);
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
            SortKey::Year => (String::new(), t.year.unwrap_or(0) as f64),
            SortKey::Genre => (t.genre.to_lowercase(), 0.0),
            SortKey::Bitrate => (String::new(), t.bitrate.unwrap_or(0) as f64),
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
            View::All => (all().map(|t| t.id.clone()).collect(), ListCfg { title: "ALL TRACKS".into(), sub: String::new(), cover: None, playlist: None, default_sort: default_sort(app, "all"), back: None }),
            View::Liked => (all().filter(|t| d.stats.get(&t.id).map(|s| s.liked).unwrap_or(false)).map(|t| t.id.clone()).collect(), ListCfg { title: "LIKED".into(), sub: String::new(), cover: None, playlist: None, default_sort: default_sort(app, "liked"), back: None }),
            View::Top => (all().filter(|t| d.stats.get(&t.id).map(|s| s.plays > 0).unwrap_or(false)).map(|t| t.id.clone()).collect(), ListCfg { title: "MOST PLAYED".into(), sub: String::new(), cover: None, playlist: None, default_sort: default_sort(app, "top"), back: None }),
            View::Recent => (all().filter(|t| now - t.added_at < 60.0 * 86400.0 * 1000.0).map(|t| t.id.clone()).collect(), ListCfg { title: "RECENTLY ADDED".into(), sub: String::new(), cover: None, playlist: None, default_sort: default_sort(app, "recent"), back: None }),
            View::Album(k) => {
                let ids: Vec<String> = all().filter(|t| album_key(t) == *k).map(|t| t.id.clone()).collect();
                let first = ids.first().and_then(|i| d.tracks.get(i));
                (ids.clone(), ListCfg { title: first.map(|t| t.album.clone()).unwrap_or_default(), sub: first.map(|t| if t.album_artist.is_empty() { t.artist.clone() } else { t.album_artist.clone() }).unwrap_or_default(), cover: first.and_then(|t| t.cover.clone()), playlist: None, default_sort: default_sort(app, "album"), back: Some(View::Albums) })
            }
            View::ArtistSongs(k) => {
                let ids: Vec<String> = all().filter(|t| artist_key(t) == *k).map(|t| t.id.clone()).collect();
                let name = ids.first().and_then(|i| d.tracks.get(i)).map(|t| main_artist(&t.artist)).unwrap_or_default();
                (ids, ListCfg { title: name, sub: "ALL SONGS".into(), cover: None, playlist: None, default_sort: default_sort(app, "artist"), back: Some(View::Artist(k.clone())) })
            }
            View::Mix(k) => {
                let Some(m) = app.home.mixes.iter().find(|m| m.id == *k).cloned() else {
                    drop(d);
                    app.browser.set_view(View::Home);
                    return;
                };
                let ids: Vec<String> = m.ids.iter().filter(|id| d.tracks.contains_key(*id)).cloned().collect();
                (ids, ListCfg { title: m.name.to_uppercase(), sub: format!("MADE FROM YOUR LIBRARY · {}", m.sub), cover: None, playlist: None, default_sort: None, back: Some(View::Home) })
            }
            View::Playlist(pid) => {
                let Some(p) = d.playlists.iter().find(|p| p.id == *pid).cloned() else {
                    drop(d);
                    app.browser.set_view(View::All);
                    return;
                };
                let ids: Vec<String> = p.track_ids.iter().filter(|id| d.tracks.contains_key(*id)).cloned().collect();
                let sub = if p.is_imported() { format!("{} · {}", p.source.clone().unwrap_or_else(|| "spotify".into()).to_uppercase(), if p.auto_sync == Some(false) { "SYNC OFF" } else { "AUTO-SYNC" }) } else { "PLAYLIST".into() };
                (ids, ListCfg { title: p.name.clone(), sub, cover: None, playlist: Some(p), default_sort: default_sort(app, "playlist"), back: None })
            }
            _ => (Vec::new(), ListCfg { title: String::new(), sub: String::new(), cover: None, playlist: None, default_sort: None, back: None }),
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
            if let Some(p) = &cfg.playlist {
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(96.0), Sense::click());
                super::plcover::draw(app, ui, r, p, 192);
                if resp.hovered() {
                    fill(ui.painter(), r.shrink(2.0), with_alpha(pal.bg, 170));
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, "CHANGE", px(7.0), pal.text);
                }
                let resp = resp.on_hover_text("Change cover (right-click to reset)");
                if resp.clicked() {
                    super::plcover::change(app, &p.id);
                }
                resp.context_menu(|ui| super::plcover::menu(app, ui, p));
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
                if let Some(d) = cfg.playlist.as_ref().map(|p| p.description.as_str()).filter(|d| !d.is_empty()) {
                    ui.label(egui::RichText::new(d).font(vt(18.0)).color(pal.text));
                }
                let sub = [cfg.sub.clone(), format!("{} TRACKS", ids.len()), fmt_long(secs)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                ui.label(egui::RichText::new(sub).color(pal.dim));
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if button(ui, &pal, "▶ PLAY", true, !ids.is_empty()).clicked() {
                app.player.play_list(ids.clone(), 0, Some(false));
                if let Some(p) = &cfg.playlist { app.lib.touch_playlist(&p.id); }
            }
            if button(ui, &pal, "SHUFFLE", false, !ids.is_empty()).clicked() {
                app.player.play_list(ids.clone(), fastrand::usize(..ids.len().max(1)), Some(true));
                if let Some(p) = &cfg.playlist { app.lib.touch_playlist(&p.id); }
            }
            if let View::Mix(k) = &view {
                if button(ui, &pal, "+ SAVE AS PLAYLIST", false, !ids.is_empty()).on_hover_text("Keep this mix as one of your playlists").clicked() {
                    let name = app.home.mixes.iter().find(|m| m.id == *k).map(|m| m.name.clone()).unwrap_or_default();
                    app.modal = Some(Modal::Prompt { title: "SAVE MIX AS PLAYLIST".into(), text: name, action: PromptAction::NewPlaylist(ids.clone()) });
                }
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
                let open = app.browser.add.open;
                if button(ui, &pal, if open { "ADD SONGS ▲" } else { "+ ADD SONGS" }, false, true).on_hover_text("Find songs in your library or online and add them here").clicked() {
                    app.browser.add.open = !open;
                    app.browser.add.focus = !open;
                }
            }
            let te = ui.add(egui::TextEdit::singleline(&mut app.browser.search).hint_text("SEARCH…").desired_width(200.0).font(vt(19.0)));
            if app.browser.focus_search {
                te.request_focus();
                app.browser.focus_search = false;
            }
        });
    });
    // add-songs box: open on request, and always in an empty playlist
    if let Some(p) = &cfg.playlist {
        if app.browser.add.open || ids.is_empty() && app.browser.search.is_empty() {
            super::addsongs::show(app, ui, p);
        }
    }
    if app.lib.data.read().tracks.is_empty() || ids.is_empty() && app.browser.search.is_empty() {
        empty_state(app, ui);
        return;
    }

    // ---- columns (Settings > Lists: which, in what order; flexible ones share the free space)
    let w = ui.available_width();
    let narrow = w < 640.0;
    let (chosen, row_h, covers) = {
        let s = app.settings.lock();
        let compact = s.density == "compact";
        (s.columns.clone(), match (compact, s.list_covers) { (true, false) => 22.0, (false, false) => 28.0, (true, true) => 28.0, (false, true) => 40.0 }, s.list_covers)
    };
    let shown: Vec<&Col> = chosen.iter().filter_map(|c| COLUMNS.iter().find(|x| x.id == c)).filter(|c| !narrow || c.narrow).collect();
    let shown: Vec<&Col> = if shown.iter().any(|c| c.share > 0.0) { shown } else { COLUMNS.iter().filter(|c| c.id == "title").chain(shown).collect() };
    let fixed: f32 = shown.iter().map(|c| c.width).sum::<f32>() + 16.0;
    let flex = (w - fixed - 8.0 * (shown.len().max(7) - 1) as f32).max(120.0);
    let shares: f32 = shown.iter().map(|c| c.share).sum();
    let cols: Vec<(&Col, f32)> = shown.into_iter().map(|c| (c, if c.share > 0.0 { flex * c.share / shares } else { c.width })).collect();
    let (hr, _) = ui.allocate_exact_size(Vec2::new(w, 24.0), Sense::hover());
    ui.painter().hline(hr.x_range(), hr.bottom(), egui::Stroke::new(2.0_f32, pal.line));
    let mut x = hr.left() + 8.0;
    for (c, cw) in &cols {
        let (label, key, right) = (c.head, c.sort, c.right);
        let cr = Rect::from_min_size(Pos2::new(x, hr.top()), Vec2::new(*cw, hr.height()));
        let sorted = key.is_some() && active_sort.map(|s| Some(s.0) == key).unwrap_or(false);
        let txt = format!("{label}{}", if sorted { if active_sort.unwrap().1 { " ▲" } else { " ▼" } } else { "" });
        let resp = ui.interact(cr, ui.id().with(("th", c.id)), Sense::click());
        ui.painter().text(if right { Pos2::new(cr.right(), cr.center().y) } else { Pos2::new(cr.left(), cr.center().y) }, if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }, txt, px(6.0), if sorted { pal.accent } else if resp.hovered() { pal.text } else { pal.dim });
        if resp.clicked() {
            if let Some(k) = key {
                let asc = !matches!(active_sort, Some((kk, true)) if kk == k);
                app.browser.sort = Some((k, asc));
            }
        }
        x += cw + 8.0;
    }

    // ---- rows (virtualised)
    let cur = app.player.current_id();
    let mut act: Option<RowAct> = None;
    let num_mode = matches!(active_sort, Some((SortKey::TrackNo, _)));
    // drag rows to reorder: only in a playlist shown in its own order
    let reorder = cfg.playlist.is_some() && active_sort.is_none() && app.browser.search.is_empty();
    // "Recommended" below a playlist's songs (fetched once it scrolls into view)
    let recs = cfg.playlist.as_ref().filter(|_| app.browser.search.is_empty() && app.settings.lock().recommend).cloned();
    let extra = recs.as_ref().map(|p| super::recs::height(app, &p.id)).unwrap_or(0.0);
    egui::ScrollArea::vertical().id_salt(("tracks", format!("{view:?}"))).auto_shrink([false; 2]).show_viewport(ui, |ui, vp| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let total = row_h * ids.len() as f32;
        ui.set_height(total + extra);
        let top = ui.max_rect().top();
        let first = ((vp.min.y / row_h).floor().max(0.0) as usize).min(ids.len());
        let last = ((vp.max.y / row_h).ceil().max(0.0) as usize + 1).min(ids.len());
        let rows = Rect::from_x_y_ranges(ui.max_rect().x_range(), top + first as f32 * row_h..=top + last as f32 * row_h);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rows), |ui| {
            ui.skip_ahead_auto_ids(first);
            for i in first..last {
                let id = &ids[i];
                let Some(t) = app.lib.track(id) else { continue };
                let st = app.lib.stat(id);
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, row_h), Sense::click_and_drag());
                let sel = app.browser.selection.contains(id);
                if resp.drag_started() {
                    let songs: Vec<String> = if sel { ids.iter().filter(|x| app.browser.selection.contains(*x)).cloned().collect() } else { vec![id.clone()] };
                    resp.dnd_set_drag_payload(DragSongs(songs));
                }
                if reorder {
                    let below = ui.ctx().pointer_hover_pos().map(|p| p.y > r.center().y).unwrap_or(false);
                    if resp.dnd_hover_payload::<DragSongs>().is_some() {
                        let y = if below { r.bottom() } else { r.top() };
                        ui.painter().with_clip_rect(r.expand(2.0)).hline(r.x_range(), y, egui::Stroke::new(3.0_f32, pal.accent));
                    }
                    if let Some(d) = resp.dnd_release_payload::<DragSongs>() {
                        let before = if below { ids.get(i + 1).cloned() } else { Some(id.clone()) };
                        act = Some(RowAct::Move(d.0.clone(), before));
                    }
                }
                let p = ui.painter();
                if sel {
                    fill(p, r, pal.sel);
                } else if resp.hovered() {
                    fill(p, r, pal.panel_hi);
                } else if i % 2 == 1 {
                    fill(p, r, with_alpha(pal.text, 2));
                }
                let is_cur = cur.as_deref() == Some(id.as_str());
                let dim = |c: Color32| if st.hidden { pal.faint } else { c };
                let mut x = r.left() + 8.0;
                let cy = r.center().y;
                for (c, cw) in &cols {
                    let right = c.right;
                    let cr = Rect::from_min_max(Pos2::new(x, r.top()), Pos2::new(x + cw, r.bottom()));
                    let clip = p.with_clip_rect(cr);
                    let (txt, color): (String, Color32) = match c.id {
                        // hovering a row: ▶ here plays it with one click
                        "num" if resp.hovered() => ("▶".into(), pal.accent),
                        "num" => (if is_cur { "▶".into() } else if num_mode && t.track.is_some() { t.track.unwrap().to_string() } else { (i + 1).to_string() }, if is_cur { pal.accent } else { pal.dim }),
                        "like" => ("♥".into(), if st.liked { pal.accent } else { pal.faint }),
                        "title" => (if st.hidden { format!("🚫 {}", t.title) } else { t.title.clone() }, if is_cur { pal.accent } else { dim(pal.text) }),
                        "artist" => (t.artist.clone(), pal.dim),
                        "album" => (t.album.clone(), pal.dim),
                        "genre" => (t.genre.clone(), pal.dim),
                        "year" => (t.year.map(|y| y.to_string()).unwrap_or_default(), pal.dim),
                        "time" => (fmt_time(t.duration), pal.dim),
                        "bitrate" => (t.bitrate.map(|b| b.to_string()).unwrap_or_default(), pal.dim),
                        "added" => (if t.added_at > 0.0 { crate::store::local_stamp((t.added_at / 1000.0) as i64, false) } else { String::new() }, pal.dim),
                        _ => (if st.plays > 0 { st.plays.to_string() } else { String::new() }, pal.dim),
                    };
                    let like = c.id == "like";
                    let color = if c.id == "title" || like { color } else { dim(color) };
                    // album cover before the title, like Spotify
                    let cr = if c.id == "title" && covers {
                        let side = row_h - 8.0;
                        let tr = Rect::from_min_size(Pos2::new(cr.left(), cy - side / 2.0), Vec2::splat(side));
                        fill(&clip, tr, pal.bg);
                        if let Some(cv) = t.thumb.as_ref().or(t.cover.as_ref()) {
                            if let Some(tex) = app.covers.get(ui.ctx(), app.lib.cover_path(cv), cv, 64) {
                                clip.image(tex, tr, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), if st.hidden { with_alpha(Color32::WHITE, 90) } else { Color32::WHITE });
                            }
                        } else {
                            clip.text(tr.center(), Align2::CENTER_CENTER, "♫", vt(side * 0.6), pal.faint);
                        }
                        Rect::from_min_max(Pos2::new(cr.left() + side + 8.0, cr.top()), cr.max)
                    } else {
                        cr
                    };
                    let clip = p.with_clip_rect(cr);
                    clip.text(if right { Pos2::new(cr.right(), cy) } else if like { Pos2::new(cr.center().x, cy) } else { Pos2::new(cr.left(), cy) }, if right { Align2::RIGHT_CENTER } else if like { Align2::CENTER_CENTER } else { Align2::LEFT_CENTER }, txt, vt(19.0), color);
                    if like && resp.clicked() && resp.interact_pointer_pos().map(|pp| cr.contains(pp)).unwrap_or(false) {
                        act = Some(RowAct::Like(id.clone()));
                    }
                    if c.id == "num" && resp.clicked() && resp.interact_pointer_pos().map(|pp| cr.contains(pp)).unwrap_or(false) {
                        act = Some(RowAct::Play(i));
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
        if let Some(p) = &recs {
            if vp.max.y > total {
                let r = Rect::from_min_size(Pos2::new(ui.max_rect().left(), top + total), Vec2::new(w, extra));
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(r), |ui| super::recs::show(app, ui, p));
            }
        }
    });
    // what's being dragged follows the pointer
    if let (Some(d), Some(pos)) = (egui::DragAndDrop::payload::<DragSongs>(ui.ctx()), ui.ctx().pointer_hover_pos()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let label = if d.0.len() == 1 { app.lib.track(&d.0[0]).map(|t| format!("♪ {}", t.title)).unwrap_or_default() } else { format!("♪ {} songs", d.0.len()) };
        let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag-songs")));
        let g = p.layout_no_wrap(label, vt(19.0), pal.ink);
        let r = Rect::from_min_size(pos + Vec2::new(14.0, 6.0), g.size() + Vec2::new(14.0, 6.0));
        fill(&p, r, pal.accent);
        p.galley(r.min + Vec2::new(7.0, 3.0), g, pal.ink);
    }
    match act {
        Some(RowAct::Move(songs, before)) => {
            if let Some(p) = &cfg.playlist {
                let u = app.lib.snapshot(format!("Moved {} in \"{}\"", plural(songs.len()), p.name), std::slice::from_ref(&p.id), &[]);
                app.lib.playlist_move(&p.id, &songs, before.as_deref());
                app.undoable(u);
            }
        }
        Some(RowAct::Like(id)) => app.lib.toggle_like(&id),
        Some(RowAct::Play(i)) => {
            app.player.play_pick(ids.clone(), i, None);
            if let Some(p) = &cfg.playlist {
                app.lib.touch_playlist(&p.id);
            }
        }
        Some(RowAct::Select(i, shift, cmd)) => {
            let id = ids[i].clone();
            if let Some(a) = app.browser.anchor.filter(|_| shift) {
                if !cmd {
                    app.browser.selection.clear();
                }
                // (the list may have shrunk since the anchor was set, e.g. by a search)
                for id in ids.iter().take(a.max(i) + 1).skip(a.min(i)) {
                    app.browser.selection.insert(id.clone());
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
    if !ui.ctx().wants_keyboard_input() && app.palette.is_none() && app.modal.is_none() && app.pick.is_none() {
        let (all, enter, del) = ui.input(|i| (i.modifiers.command && i.key_pressed(egui::Key::A), i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Delete)));
        if all {
            app.browser.selection = ids.iter().cloned().collect();
        }
        if enter {
            if let Some(i) = ids.iter().position(|x| app.browser.selection.contains(x)) {
                app.player.play_pick(ids.clone(), i, None);
            }
        }
        if del {
            if let Some(p) = &cfg.playlist {
                let sel: Vec<String> = ids.iter().filter(|x| app.browser.selection.contains(*x)).cloned().collect();
                if !sel.is_empty() {
                    remove_from_playlist(app, p, &sel);
                }
            }
        }
    }
}

enum RowAct {
    Move(Vec<String>, Option<String>),
    Select(usize, bool, bool),
    Play(usize),
    Like(String),
}

pub(super) fn track_menu(app: &mut App, ui: &mut Ui, ids: &[String], i: usize, playlist: Option<&crate::store::Playlist>) {
    let sel: Vec<String> = ids.iter().filter(|x| app.browser.selection.contains(*x)).cloned().collect();
    let sel = if sel.is_empty() { vec![ids[i].clone()] } else { sel };
    let one = if sel.len() == 1 { app.lib.track(&sel[0]) } else { None };
    if ui.button("▶ Play").clicked() {
        app.player.play_pick(ids.to_vec(), i, None);
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
    if ui.button("Add to playlist…").clicked() {
        let pos = ui.ctx().pointer_latest_pos().unwrap_or(ui.min_rect().right_top());
        app.pick = Some(super::plpick::Popup::new(sel.clone(), pos));
        ui.close_menu();
    }
    ui.separator();
    let liked = one.as_ref().map(|t| app.lib.stat(&t.id).liked).unwrap_or(false);
    if ui.button(if liked { "Unlike" } else { "Like" }).clicked() {
        for id in &sel {
            app.lib.toggle_like(id);
        }
        ui.close_menu();
    }
    let hidden = sel.iter().all(|id| app.lib.is_hidden(id));
    if ui.button(if hidden { "Unhide song" } else { "Hide song (don't play this)" }).on_hover_text("Hidden songs stay listed (dimmed) but shuffle, playlists and radio skip them").clicked() {
        app.lib.set_hidden(&sel, !hidden);
        app.toast(if hidden { format!("Unhid {}", plural(sel.len())) } else { format!("Hid {}: they won't play unless you pick them", plural(sel.len())) });
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
            remove_from_playlist(app, p, &sel);
            ui.close_menu();
        }
    }
    if ui.button(format!("Remove from library{}", if sel.len() > 1 { format!(" ({})", sel.len()) } else { String::new() })).clicked() {
        let u = app.lib.snapshot(format!("Removed {} from your library", plural(sel.len())), &[], &sel);
        for id in &sel {
            app.lib.remove_track(id);
        }
        app.undoable(u);
        app.browser.selection.clear();
        ui.close_menu();
    }
}

fn plural(n: usize) -> String {
    if n == 1 { "1 song".into() } else { format!("{n} songs") }
}

fn remove_from_playlist(app: &mut App, p: &crate::store::Playlist, sel: &[String]) {
    let u = app.lib.snapshot(format!("Removed {} from \"{}\"", plural(sel.len()), p.name), std::slice::from_ref(&p.id), &[]);
    app.lib.playlist_remove(&p.id, sel);
    app.undoable(u);
    app.browser.selection.clear();
}

fn empty_state(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        let lib_empty = app.lib.data.read().tracks.is_empty();
        let (title, msg) = match (&app.browser.view, lib_empty) {
            (View::Playlist(_), false) => ("EMPTY PLAYLIST", "Search above, or right-click songs anywhere > Add to playlist."),
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
    ar.sort_by_key(|a| a.1.to_lowercase());
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

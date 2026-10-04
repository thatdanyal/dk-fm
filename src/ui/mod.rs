//! The DK.FM window: custom title bar, dockable panels (egui_dock), overlays and shortcuts.
//! The GUI only repaints when something changes (or ~4 Hz while playing / at the visualizer
//! frame rate when the scope is visible), which is what keeps CPU and RAM low.
pub mod addsongs;
pub mod browser;
pub mod cjk;
pub mod deck;
pub mod dupes;
pub mod eqpanel;
pub mod fonts;
pub mod home;
pub mod import;
pub mod keys;
pub mod lyrics;
pub mod nowplaying;
pub mod palette;
pub mod plcover;
pub mod plpick;
pub mod queue;
pub mod recs;
pub mod scope;
pub mod settings;
pub mod sharing;
pub mod stats;
pub mod theme;
pub mod widgets;

use crate::downloader::Downloader;
use crate::library::Library;
use crate::player::Player;
use crate::store::Settings;
use crate::system::{self, Media, Tray, UpdState};
use crate::watcher::FolderWatcher;
use eframe::egui::{self, Align2, Color32, Id, LayerId, Order, Rect, Sense, Vec2, ViewportCommand};
use egui_dock::{DockArea, DockState, NodeIndex};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use theme::{px, vt, Pal};
use widgets::{fill, tb_button};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum Tab {
    Deck,
    Scope,
    Library,
    Queue,
    Eq,
    Lyrics,
}

impl Tab {
    pub const ALL: [Tab; 6] = [Tab::Deck, Tab::Scope, Tab::Library, Tab::Queue, Tab::Eq, Tab::Lyrics];
    pub fn name(&self) -> &'static str {
        match self {
            Tab::Deck => "DECK",
            Tab::Scope => "SCOPE",
            Tab::Library => "LIBRARY",
            Tab::Queue => "QUEUE",
            Tab::Eq => "EQUALIZER",
            Tab::Lyrics => "LYRICS",
        }
    }
}

pub enum Modal {
    Settings(settings::SetTab),
    Prompt { title: String, text: String, action: PromptAction },
    Update,
}

#[derive(Clone)]
pub enum PromptAction {
    NewPlaylist(Vec<String>),
    RenamePlaylist(String),
    /// playlist description (may be empty)
    Describe(String),
    /// new folder, optionally moving a playlist into it
    NewFolder(Option<String>),
    RenameFolder(String),
    PasteYoutube(String, usize),
}

pub struct App {
    pub lib: Arc<Library>,
    pub player: Arc<Player>,
    pub dl: Arc<Downloader>,
    pub watcher: Arc<FolderWatcher>,
    pub settings: Arc<Mutex<Settings>>,
    pub settings_dirty: Arc<AtomicBool>,
    pub pal: Pal,
    theme_key: String,
    dock: DockState<Tab>,
    pub layout_edit: bool,
    pub mini: bool,
    normal_size: Option<Vec2>,
    pub covers: widgets::Covers,
    pub browser: browser::BrowserState,
    pub import: import::ImportState,
    pub scope: scope::ScopeState,
    pub lyrics: lyrics::LyricsState,
    pub stats: stats::StatsState,
    pub toasts: Vec<(String, Instant, bool)>,
    pub modal: Option<Modal>,
    pub palette: Option<palette::PaletteState>,
    /// "Add to playlist" checklist opened from a song's right-click menu
    pub pick: Option<plpick::Popup>,
    pub update: Arc<Mutex<UpdState>>,
    update_dismissed: Option<String>,
    tray: Option<Tray>,
    media: Option<Media>,
    last_flush: Instant,
    started_hidden: bool,
    frames: u64,
    cjk_gen: u64,
    pub drop_hover: bool,
    /// settings window state (theme editor draft, key capture, confirmations)
    pub setui: settings::SetUi,
    /// a friend's code / file waiting in its preview
    pub incoming: Option<sharing::Incoming>,
    pub plcovers: plcover::PlCovers,
    /// "Removed 3 songs from Chill · UNDO" (the last undoable change, for a few seconds)
    pub undo_toast: Option<(String, Instant)>,
    pub home: home::HomeState,
    pub recs: recs::Recs,
    /// full-screen now playing
    pub nowplaying: bool,
}

pub fn default_dock() -> DockState<Tab> {
    let mut d = DockState::new(vec![Tab::Library]);
    let s = d.main_surface_mut();
    let [lib, left] = s.split_left(NodeIndex::root(), 0.27, vec![Tab::Deck]);
    let [_deck, scope] = s.split_below(left, 0.46, vec![Tab::Scope, Tab::Eq]);
    let _ = scope;
    let [_lib, right] = s.split_right(lib, 0.72, vec![Tab::Queue]);
    let _ = s.split_below(right, 0.58, vec![Tab::Lyrics]);
    d
}

impl App {
    pub fn new(cc: &eframe::CreationContext, lib: Arc<Library>, player: Arc<Player>, dl: Arc<Downloader>, watcher: Arc<FolderWatcher>, settings: Arc<Mutex<Settings>>, settings_dirty: Arc<AtomicBool>, hidden: bool) -> Self {
        let (tkey, pal, dock_json, start) = {
            let s = settings.lock();
            lib.private.store(s.private_listening && s.keep_private, Ordering::Relaxed);
            (s.theme.clone(), theme::resolve(&s), s.dock.clone(), s.start_view.clone())
        };
        theme::apply(&cc.egui_ctx, &pal);
        *system::CTX.lock() = Some(cc.egui_ctx.clone());

        // window handle for tray / media keys / reliable show-hide on Windows
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = cc.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    system::HWND.store(w.hwnd.get(), Ordering::Relaxed);
                }
            }
        }
        let ctx = cc.egui_ctx.clone();
        *player.repaint.lock() = Some(Box::new(move || ctx.request_repaint()));
        let ctx = cc.egui_ctx.clone();
        *dl.repaint.lock() = Some(Box::new(move || ctx.request_repaint()));
        *crate::single::WAKE.lock() = Some(Box::new(system::show_window));
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(15)); // startup (scan, first frames) is done
            system::trim_memory();
        });
        let (p, l) = (player.clone(), lib.clone());
        *crate::single::COMMAND.lock() = Some(Box::new(move |cmd| match cmd {
            "play-pause" if p.status().current.is_none() && p.current_id().is_none() => {
                // nothing loaded yet: shuffle the whole library
                let ids: Vec<String> = l.data.read().tracks.keys().cloned().collect();
                p.play_list(ids, 0, Some(true));
            }
            "play-pause" => p.toggle(),
            "next" => p.next(true),
            "prev" => p.prev(),
            _ => {}
        }));

        let dock = dock_json.and_then(|v| serde_json::from_value::<DockState<Tab>>(v).ok()).filter(|d| d.iter_all_tabs().count() > 0).unwrap_or_else(default_dock);
        let tray = Tray::new(player.clone(), lib.clone());
        let media = Some(Media::new(player.clone()));
        let update = Arc::new(Mutex::new(UpdState::Idle));
        let auto = settings.lock().auto_update;
        if auto {
            let u = update.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(4));
                loop {
                    *u.lock() = match system::check_update() {
                        Ok(Some(up)) => UpdState::Available(up),
                        Ok(None) => UpdState::Latest,
                        Err(e) => UpdState::Error(e),
                    };
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_secs(30 * 60));
                }
            });
        }
        let mut covers = widgets::Covers::new();
        covers.ctx = Some(cc.egui_ctx.clone());
        crate::backup::auto_loop(lib.clone(), settings.clone());
        let mut browser = browser::BrowserState::default();
        if let Some(v) = browser::view_for(&start) {
            browser.set_view(v);
        }
        let mut app = Self {
            lib,
            player,
            dl,
            watcher,
            settings,
            settings_dirty,
            pal,
            theme_key: tkey,
            dock,
            layout_edit: false,
            mini: false,
            normal_size: None,
            covers,
            browser,
            import: import::ImportState::default(),
            scope: scope::ScopeState::new(),
            lyrics: lyrics::LyricsState::default(),
            stats: stats::StatsState::default(),
            toasts: Vec::new(),
            modal: None,
            palette: None,
            pick: None,
            update,
            update_dismissed: None,
            tray,
            media,
            last_flush: Instant::now(),
            started_hidden: hidden,
            frames: 0,
            cjk_gen: 0,
            drop_hover: false,
            setui: Default::default(),
            incoming: None,
            plcovers: Default::default(),
            undo_toast: None,
            home: Default::default(),
            recs: Default::default(),
            nowplaying: false,
        };
        app.apply_look(&cc.egui_ctx);
        app
    }

    /// Loads a system font when Japanese/Korean/Chinese text shows up (see cjk.rs).
    fn check_alphabets(&mut self, ctx: &egui::Context) {
        let gen = self.lib.gen.load(Ordering::Relaxed);
        if gen != self.cjk_gen {
            self.cjk_gen = gen;
            let mut text = String::new();
            {
                let d = self.lib.data.read();
                for t in d.tracks.values() {
                    for s in [&t.title, &t.artist, &t.album] {
                        text.push_str(s);
                    }
                }
                for p in &d.playlists {
                    text.push_str(&p.name);
                }
            }
            cjk::ensure(ctx, &text);
        }
        let typed: String = ctx.input(|i| i.events.iter().filter_map(|e| match e { egui::Event::Text(t) | egui::Event::Paste(t) | egui::Event::Ime(egui::ImeEvent::Preedit(t) | egui::ImeEvent::Commit(t)) => Some(t.clone()), _ => None }).collect());
        if !typed.is_empty() {
            cjk::ensure(ctx, &typed);
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toasts.push((msg.into(), Instant::now(), false));
    }
    /// Replace an earlier toast (by its timestamp) with a new message, or just remove it.
    pub fn toast_update(&mut self, prev: Option<Instant>, msg: Option<String>) -> Option<Instant> {
        self.toasts.retain(|t| Some(t.1) != prev);
        let now = Instant::now();
        self.toasts.push((msg?, now, false));
        Some(now)
    }
    pub fn toast_err(&mut self, msg: impl Into<String>) {
        self.toasts.push((msg.into(), Instant::now(), true));
    }

    /// Keep a change undoable (Ctrl+Z or the toast's UNDO button).
    pub fn undoable(&mut self, u: crate::library::Undo) {
        self.undo_toast = Some((u.label.clone(), Instant::now()));
        self.lib.push_undo(u);
    }

    pub fn undo(&mut self) {
        self.undo_toast = None;
        match self.lib.undo() {
            Some(u) if u.trashed > 0 => self.toast(format!("Undone: {}. The {} removed file{} are in the Recycle Bin: restore them from there to play them again", u.label, u.trashed, if u.trashed == 1 { "" } else { "s" })),
            Some(u) => self.toast(format!("Undone: {}", u.label)),
            None => self.toast("Nothing to undo"),
        }
    }

    /// Private listening: plays, skips and listening history aren't recorded while it's on.
    pub fn set_private(&mut self, on: bool) {
        self.lib.private.store(on, Ordering::Relaxed);
        self.edit_settings(|s| s.private_listening = on);
        self.toast(if on { "Private listening on: plays and history aren't recorded" } else { "Private listening off" });
    }

    pub fn set_theme(&mut self, ctx: &egui::Context) {
        let (k, pal) = { let s = self.settings.lock(); (s.theme.clone(), theme::resolve(&s)) };
        self.theme_key = k;
        self.pal = pal;
        theme::apply(ctx, &self.pal);
        self.scope.invalidate();
        self.settings_dirty.store(true, Ordering::Relaxed);
    }

    /// Font, zoom and visualizer settings (each only does work when it actually changed).
    pub fn apply_look(&mut self, ctx: &egui::Context) {
        let (font, heads, zoom, vis, bars) = { let s = self.settings.lock(); (s.font.clone(), s.pixel_headings, s.zoom.clamp(0.9, 1.5), s.vis_colors.clone(), s.vis_bars) };
        theme::set_font(ctx, &font, heads);
        if (ctx.zoom_factor() - zoom).abs() > 0.001 {
            ctx.set_zoom_factor(zoom);
        }
        let colors = vis.and_then(|c| Some([theme::parse_hex(&c[0])?, theme::parse_hex(&c[1])?, theme::parse_hex(&c[2])?]));
        if colors != self.scope.colors || bars != self.scope.bars {
            self.scope.colors = colors;
            self.scope.bars = bars;
            self.scope.invalidate();
        }
    }

    /// After restoring a backup: reapply every setting and rescan the music folders.
    pub fn reload_all(&mut self, ctx: &egui::Context) {
        let s = self.settings.lock().clone();
        self.set_theme(ctx);
        self.apply_look(ctx);
        self.dock = s.dock.clone().and_then(|v| serde_json::from_value::<DockState<Tab>>(v).ok()).filter(|d| d.iter_all_tabs().count() > 0).unwrap_or_else(default_dock);
        self.player.set_volume(s.player.volume);
        self.apply_player(&s);
        self.browser.set_view(browser::View::All);
        self.settings_dirty.store(true, Ordering::Relaxed);
        self.rescan();
    }

    /// Playback, visualizer and EQ settings from `s` into the player.
    pub fn apply_player(&mut self, s: &Settings) {
        let p = &self.player;
        p.set_crossfade(s.player.crossfade);
        p.set_normalize(s.player.normalize);
        p.set_match_volume(s.player.match_volume);
        p.set_smart_shuffle(s.player.smart_shuffle);
        p.set_visualizer(&s.player.visualizer, s.player.vis_fps);
        p.set_eq(s.eq.clone());
        self.scope.mode = s.player.visualizer.clone();
        self.scope.invalidate();
    }

    // ------------------------------------------------------------ saved layouts
    pub const PRESETS: [&'static str; 2] = ["Default", "Minimal"];

    pub fn preset(name: &str) -> Option<DockState<Tab>> {
        match name {
            "Default" => Some(default_dock()),
            "Minimal" => {
                let mut d = DockState::new(vec![Tab::Library]);
                let _ = d.main_surface_mut().split_left(NodeIndex::root(), 0.26, vec![Tab::Deck, Tab::Queue]);
                Some(d)
            }
            _ => None,
        }
    }

    /// Switch to a built-in or saved layout by name.
    pub fn apply_layout(&mut self, name: &str) -> bool {
        let saved = self.settings.lock().layouts.iter().find(|l| l.name == name).and_then(|l| serde_json::from_value::<DockState<Tab>>(l.dock.clone()).ok());
        match saved.filter(|d| d.iter_all_tabs().count() > 0).or_else(|| Self::preset(name)) {
            Some(d) => {
                self.dock = d;
                self.save_dock();
                true
            }
            None => false,
        }
    }

    /// Save the current layout under a name (replacing one with the same name).
    pub fn save_layout(&mut self, name: &str) {
        let Ok(v) = serde_json::to_value(&self.dock) else { return };
        self.edit_settings(|s| {
            s.layouts.retain(|l| l.name != name);
            s.layouts.push(crate::store::NamedLayout { name: name.to_string(), dock: v });
        });
    }

    /// Built-in and saved layout names.
    pub fn layout_names(&self) -> Vec<String> {
        let saved: Vec<String> = self.settings.lock().layouts.iter().map(|l| l.name.clone()).collect();
        let mut v: Vec<String> = Self::PRESETS.iter().map(|s| s.to_string()).filter(|p| !saved.contains(p)).collect();
        v.extend(saved);
        v
    }

    pub fn edit_settings(&self, f: impl FnOnce(&mut Settings)) {
        f(&mut self.settings.lock());
        self.settings_dirty.store(true, Ordering::Relaxed);
    }

    pub fn music_folders(&self) -> Vec<PathBuf> {
        let s = self.settings.lock();
        let mut v: Vec<PathBuf> = s.music_folders.iter().map(PathBuf::from).collect();
        v.push(PathBuf::from(&s.download_dir));
        v.sort();
        v.dedup();
        v.into_iter().filter(|p| p.exists()).collect()
    }

    pub fn rescan(&self) {
        let _ = std::fs::create_dir_all(&self.settings.lock().download_dir);
        let f = self.music_folders();
        self.lib.scan(f.clone());
        self.watcher.watch(&f);
    }

    pub fn show_panel(&mut self, tab: Tab) {
        if let Some(loc) = self.dock.find_tab(&tab) {
            self.dock.set_active_tab(loc);
        } else {
            self.dock.main_surface_mut().push_to_first_leaf(tab);
        }
        self.save_dock();
    }

    pub fn toggle_panel(&mut self, tab: Tab) {
        if let Some(loc) = self.dock.find_tab(&tab) {
            if self.dock.iter_all_tabs().count() > 1 {
                self.dock.remove_tab(loc);
            }
        } else {
            self.dock.main_surface_mut().push_to_first_leaf(tab);
        }
        self.save_dock();
    }

    pub fn reset_layout(&mut self) {
        self.dock = default_dock();
        self.save_dock();
    }

    fn save_dock(&self) {
        let v = serde_json::to_value(&self.dock).ok();
        self.edit_settings(|s| s.dock = v);
    }

    pub fn toggle_mini(&mut self, ctx: &egui::Context) {
        self.mini = !self.mini;
        if self.mini {
            self.normal_size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(460.0, 172.0)));
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
        } else {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(self.normal_size.unwrap_or(Vec2::new(1360.0, 860.0))));
        }
    }

    pub fn current_track(&self) -> Option<crate::store::Track> {
        self.player.current_id().and_then(|id| self.lib.track(&id))
    }

    // ------------------------------------------------------------ title bar
    fn titlebar(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        egui::TopBottomPanel::top("titlebar").exact_height(if self.mini { 28.0 } else { 38.0 }).frame(egui::Frame::new().fill(pal.bg2).inner_margin(egui::Margin { left: 10, right: 0, top: 0, bottom: 0 })).show(ctx, |ui| {
            let full = ui.max_rect();
            let drag = ui.interact(full, Id::new("tb-drag"), Sense::click_and_drag());
            if drag.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            if drag.double_clicked() && !self.mini {
                let m = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!m));
            }
            ui.painter().hline(full.x_range(), full.bottom() - 1.0, egui::Stroke::new(2.0_f32, pal.line));
            ui.horizontal_centered(|ui| {
                // logo
                let blink = self.player.status().playing && (ctx.input(|i| i.time) * 1.6) as i64 % 2 == 0;
                let logo = egui::text::LayoutJob::default();
                let mut job = logo;
                let f = px(if self.mini { 10.0 } else { 13.0 });
                job.append("DK", 0.0, egui::TextFormat::simple(f.clone(), pal.accent));
                job.append(".", 0.0, egui::TextFormat::simple(f.clone(), if blink { Color32::TRANSPARENT } else { pal.accent2 }));
                job.append("FM", 0.0, egui::TextFormat::simple(f, pal.text));
                ui.label(job);
                ui.add_space(10.0);
                if !self.mini {
                    if tb_button(ui, &pal, "LAYOUT", self.layout_edit).clicked() {
                        self.layout_edit = !self.layout_edit;
                    }
                    let r = tb_button(ui, &pal, "PANELS", false);
                    let pid = ui.make_persistent_id("panels-menu");
                    if r.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(pid));
                    }
                    egui::popup::popup_below_widget(ui, pid, &r, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                        ui.set_min_width(160.0);
                        for t in Tab::ALL {
                            let on = self.dock.find_tab(&t).is_some();
                            if ui.button(format!("{} {}", if on { "✔" } else { "  " }, t.name())).clicked() {
                                self.toggle_panel(t);
                            }
                        }
                    });
                    let r = tb_button(ui, &pal, "THEME", false);
                    let mut chosen = None;
                    let tid = ui.make_persistent_id("theme-menu");
                    if r.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(tid));
                    }
                    egui::popup::popup_below_widget(ui, tid, &r, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        ui.set_min_width(180.0);
                        let s = self.settings.lock().clone();
                        for (k, name) in theme::all_themes(&s) {
                            let sw = theme::theme_pal(&s, &k).accent;
                            let resp = ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                                fill(ui.painter(), r, sw);
                                ui.button(format!("{} {name}", if self.theme_key == k { "•" } else { " " }))
                            });
                            if resp.inner.clicked() {
                                chosen = Some(k);
                            }
                        }
                    });
                    if let Some(k) = chosen {
                        self.edit_settings(|s| s.theme = k);
                        self.set_theme(ctx);
                    }
                    if tb_button(ui, &pal, "SETTINGS", false).clicked() {
                        self.modal = Some(Modal::Settings(settings::SetTab::Look));
                    }
                    if self.lib.private.load(Ordering::Relaxed) {
                        ui.add_space(10.0);
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(86.0, 22.0), Sense::click());
                        fill(ui.painter(), r, pal.accent2);
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, "PRIVATE", px(7.0), pal.ink);
                        if resp.on_hover_text("Private listening: plays and history aren't recorded. Click to turn off.").clicked() {
                            self.set_private(false);
                        }
                    }
                    // centre status
                    if self.lib.scanning.load(Ordering::Relaxed) {
                        let (d, t) = (self.lib.scan_done.load(Ordering::Relaxed), self.lib.scan_total.load(Ordering::Relaxed));
                        ui.add_space(20.0);
                        ui.label(egui::RichText::new(format!("SCANNING LIBRARY… {d}/{t}")).color(pal.dim));
                        ctx.request_repaint_after(Duration::from_millis(300));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    // window buttons drawn as pixel shapes (no font glyph needed)
                    let wb = |ui: &mut egui::Ui, glyph: &str, close: bool| {
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(44.0, ui.available_height()), Sense::click());
                        if resp.hovered() {
                            fill(ui.painter(), r, if close { pal.accent } else { pal.panel_hi });
                        }
                        let c = if resp.hovered() && close { pal.ink } else if resp.hovered() { pal.text } else { pal.dim };
                        let m = r.center();
                        let px2 = |dx: f32, dy: f32| fill(ui.painter(), Rect::from_min_size(m + Vec2::new(dx, dy), Vec2::splat(2.0)), c);
                        match glyph {
                            "close" => (0..5).for_each(|i| { let d = i as f32 * 2.0 - 5.0; px2(d, d); px2(d, -d - 2.0); }),
                            "min" => (0..5).for_each(|i| px2(i as f32 * 2.0 - 5.0, 3.0)),
                            "max" => widgets::frame_rect(ui.painter(), Rect::from_center_size(m, Vec2::splat(10.0)), 2.0, c),
                            _ => {
                                widgets::frame_rect(ui.painter(), Rect::from_center_size(m + Vec2::new(-1.0, 1.0), Vec2::splat(8.0)), 2.0, c);
                                widgets::frame_rect(ui.painter(), Rect::from_center_size(m + Vec2::new(2.0, -2.0), Vec2::splat(8.0)), 1.0, c);
                            }
                        }
                        resp
                    };
                    if wb(ui, "close", true).clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                    if !self.mini {
                        let m = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        if wb(ui, if m { "restore" } else { "max" }, false).clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Maximized(!m));
                        }
                    }
                    if wb(ui, "min", false).clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                    }
                    ui.add_space(6.0);
                    if tb_button(ui, &pal, if self.mini { "FULL" } else { "MINI" }, self.mini).clicked() {
                        self.toggle_mini(ctx);
                    }
                    let up = self.update.lock().clone();
                    let label = match &up {
                        UpdState::Available(u) => Some(format!("UPDATE > v{}", u.version)),
                        UpdState::Downloading(p) => Some(format!("UPDATING {p}%")),
                        _ => None,
                    };
                    if let (Some(l), false) = (label, self.mini) {
                        ui.add_space(6.0);
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(l.len() as f32 * 8.0 + 16.0, 24.0), Sense::click());
                        fill(ui.painter(), r, pal.accent2);
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, l, px(7.0), pal.ink);
                        if resp.clicked() {
                            self.modal = Some(Modal::Update);
                        }
                    }
                });
            });
        });
    }

    // ------------------------------------------------------------ shortcuts
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.setui.capturing() {
            return; // Settings > Shortcuts is recording a new key
        }
        let pressed: Vec<keys::Combo> = ctx.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Key { key, pressed: true, modifiers, .. } = e { Some(keys::Combo::from_event(*key, *modifiers)) } else { None }).collect());
        if pressed.is_empty() {
            return;
        }
        let typing = ctx.wants_keyboard_input();
        let map = self.settings.lock().keys.clone();
        for c in pressed {
            for a in keys::matching(&map, c) {
                // Ctrl shortcuts that don't edit text work everywhere; the rest not while typing,
                // and plain keys not over the palette or a window either
                let global = keys::GLOBAL.contains(&a) && (c.ctrl || c.alt);
                if !global && (typing || (!c.ctrl && !c.alt && (self.palette.is_some() || self.modal.is_some() || self.incoming.is_some()))) {
                    continue;
                }
                self.run_action(ctx, a);
            }
        }
    }

    pub fn run_action(&mut self, ctx: &egui::Context, action: &str) {
        let pos = || self.player.status().position;
        match action {
            "palette" => self.palette = if self.palette.is_some() { None } else { Some(palette::PaletteState::default()) },
            "settings" => self.modal = Some(Modal::Settings(settings::SetTab::Look)),
            "layout" => self.layout_edit = !self.layout_edit,
            "mini" => self.toggle_mini(ctx),
            "search" => {
                self.show_panel(Tab::Library);
                self.browser.focus_search = true;
            }
            "import" => {
                self.show_panel(Tab::Library);
                self.browser.set_view(browser::View::Import);
            }
            "next" => self.player.next(true),
            "prev" => self.player.prev(),
            "play" => self.player.toggle(),
            "fwd" => self.player.seek(pos() + 5.0),
            "back" => self.player.seek(pos() - 5.0),
            "fwd_long" => self.player.seek(pos() + 30.0),
            "back_long" => self.player.seek(pos() - 30.0),
            "vol_up" | "vol_down" => {
                let v = self.player.st.lock().opts.volume;
                self.player.set_volume(v + if action == "vol_up" { 0.05 } else { -0.05 })
            }
            "mute" => self.player.toggle_mute(),
            "shuffle" => self.player.toggle_shuffle(),
            "repeat" => self.player.cycle_repeat(),
            "like" => {
                if let Some(id) = self.player.current_id() {
                    self.lib.toggle_like(&id);
                }
            }
            "vis" => self.scope.cycle(&self.player),
            "rescan" => {
                self.rescan();
                self.toast("Scanning library…");
            }
            "undo" => self.undo(),
            "private" => {
                let on = !self.lib.private.load(Ordering::Relaxed);
                self.set_private(on);
            }
            "nowplaying" => self.nowplaying = !self.nowplaying && !self.mini,
            _ => {}
        }
    }

    fn overlays(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        // toasts (bottom-right)
        self.toasts.retain(|t| t.1.elapsed() < Duration::from_secs(if t.2 { 6 } else { 3 }));
        if !self.toasts.is_empty() {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("toasts")));
            let mut y = screen.bottom() - 16.0;
            for (msg, _, err) in self.toasts.iter().rev() {
                let galley = painter.layout(msg.clone(), vt(19.0), if *err { pal.accent } else { pal.text }, 380.0);
                let size = galley.size() + Vec2::new(24.0, 16.0);
                let r = Rect::from_min_size(egui::pos2(screen.right() - 16.0 - size.x, y - size.y), size);
                fill(&painter, r.translate(Vec2::splat(4.0)), pal.shadow);
                fill(&painter, r, pal.panel);
                widgets::frame_rect(&painter, r, 2.0, pal.accent);
                painter.galley(r.min + Vec2::new(12.0, 8.0), galley, pal.text);
                y -= size.y + 8.0;
            }
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        // undo: the last change with an UNDO button
        if let Some((label, t)) = self.undo_toast.clone() {
            if t.elapsed() > Duration::from_secs(8) {
                self.undo_toast = None;
            } else {
                let mut undo = false;
                let lift = if self.layout_edit { 60.0 } else { 16.0 };
                egui::Area::new(Id::new("undo-toast")).anchor(Align2::CENTER_BOTTOM, [0.0, -lift]).order(Order::Tooltip).show(ctx, |ui| {
                    egui::Frame::new().fill(pal.panel).stroke(egui::Stroke::new(2.0_f32, pal.accent)).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&label).font(vt(19.0)).color(pal.text));
                            ui.add_space(8.0);
                            undo = widgets::button(ui, &pal, "UNDO", true, true).on_hover_text("Ctrl+Z").clicked();
                        });
                    });
                });
                if undo {
                    self.undo();
                }
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
        // CRT scanlines + vignette
        if self.settings.lock().scanlines {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("scan")));
            let c = if pal.dark { Color32::from_black_alpha(46) } else { Color32::from_black_alpha(10) };
            let mut y = screen.top() + 2.0;
            while y < screen.bottom() {
                painter.hline(screen.x_range(), y, egui::Stroke::new(1.0_f32, c));
                y += 3.0;
            }
        }
        if self.drop_hover {
            let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("drop")));
            let s = ctx.screen_rect();
            fill(&painter, s, Color32::from_black_alpha(170));
            painter.text(s.center(), Align2::CENTER_CENTER, "DROP AUDIO FILES TO ADD · OR A .DKFM SHARE", px(14.0), pal.accent);
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        self.drop_hover = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if dropped.is_empty() {
            return;
        }
        let mut files = Vec::new();
        for p in dropped {
            if p.extension().map(|e| e.eq_ignore_ascii_case(crate::share::EXT)).unwrap_or(false) {
                sharing::open_file(self, &p);
            } else if p.is_dir() {
                for e in walkdir::WalkDir::new(&p).into_iter().flatten() {
                    if crate::library::is_audio(e.path()) {
                        files.push(e.into_path());
                    }
                }
            } else {
                files.push(p);
            }
        }
        if files.is_empty() {
            return; // only share files
        }
        let ids = self.lib.add_files(&files, |_| {});
        if ids.is_empty() {
            self.toast_err("No playable audio files found");
            return;
        }
        self.toast(format!("Added {} song{}", ids.len(), if ids.len() == 1 { "" } else { "s" }));
        if self.player.current_id().is_none() { self.player.play_list(ids, 0, Some(false)) } else { self.player.enqueue(ids) }
    }

    /// Dev/testing only: DKFM_TEST_PLAY=1 starts playback; DKFM_SCREENSHOT=<png> saves a frame
    /// after DKFM_SCREENSHOT_DELAY ms (default 3000) and quits.
    fn dev_hooks(&mut self, ctx: &egui::Context) {
        if self.frames == 3 && std::env::var("DKFM_TEST_PLAY").is_ok() {
            let ids: Vec<String> = { let d = self.lib.data.read(); let mut v: Vec<_> = d.tracks.values().map(|t| (t.artist.clone(), t.title.clone(), t.id.clone())).collect(); v.sort(); v.into_iter().map(|x| x.2).collect() };
            self.player.set_volume(0.03);
            self.player.play_list(ids, 0, Some(false));
        }
        if self.frames == 3 {
            match std::env::var("DKFM_VIEW").unwrap_or_default().as_str() {
                "stats" => self.browser.set_view(browser::View::Stats),
                "home" => self.browser.set_view(browser::View::Home),
                // the first "Made from your library" mix
                "mix" => {
                    self.home.mixes = crate::discover::mixes(&self.lib.data.read(), crate::store::now_ms());
                    if let Some(m) = self.home.mixes.first() { self.browser.set_view(browser::View::Mix(m.id.clone())); }
                }
                "nowplaying" => self.nowplaying = true,
                // artist=<name>: that artist's page; artist: your top artist's
                v if v.starts_with("artist") => {
                    let k = v.strip_prefix("artist=").map(|n| n.to_lowercase()).or_else(|| crate::discover::top_artists(&self.lib.data.read(), 1).first().map(|a| a.0.clone()));
                    if let Some(k) = k { self.browser.set_view(browser::View::Artist(k)); }
                }
                // a short playlist, so its Recommended section is on screen
                "recs" => {
                    let p = self.lib.data.read().playlists.iter().filter(|p| (3..=8).contains(&p.track_ids.len())).map(|p| p.id.clone()).next();
                    if let Some(p) = p { self.browser.set_view(browser::View::Playlist(p)); }
                }
                "import" => self.browser.set_view(browser::View::Import),
                "downloads" => self.browser.set_view(browser::View::Downloads),
                "albums" => self.browser.set_view(browser::View::Albums),
                "dupes" => self.browser.set_view(browser::View::Duplicates),
                "playlist" => {
                    let p = self.lib.data.read().playlists.iter().find(|p| p.track_ids.len() > 3).map(|p| p.id.clone());
                    if let Some(p) = p { self.browser.set_view(browser::View::Playlist(p)); }
                }
                // playlist=<name>: that playlist
                v if v.starts_with("playlist=") => {
                    let p = self.lib.data.read().playlists.iter().find(|p| p.name == v[9..]).map(|p| p.id.clone());
                    if let Some(p) = p { self.browser.set_view(browser::View::Playlist(p)); }
                }
                // private listening on + an undo toast (3 songs removed from the first big playlist, then put back)
                "private-undo" => {
                    self.lib.private.store(true, Ordering::Relaxed);
                    let p = self.lib.data.read().playlists.iter().find(|p| p.track_ids.len() > 3).cloned();
                    if let Some(p) = p {
                        let sel: Vec<String> = p.track_ids.iter().take(3).cloned().collect();
                        let u = self.lib.snapshot(format!("Removed 3 songs from \"{}\"", p.name), std::slice::from_ref(&p.id), &[]);
                        self.lib.playlist_remove(&p.id, &sel);
                        self.undoable(u);
                        self.browser.set_view(browser::View::Playlist(p.id.clone()));
                    }
                }
                "settings" => self.modal = Some(Modal::Settings(settings::SetTab::Look)),
                // settings-look, settings-search, settings-backup, ... (a tab by name)
                v if v.starts_with("settings-") => self.modal = Some(Modal::Settings(settings::tab_named(&v[9..]))),
                v if v.starts_with("layout-") => {
                    self.apply_layout(&v[7..]);
                }
                "theme-editor" => {
                    self.modal = Some(Modal::Settings(settings::SetTab::Look));
                    self.setui.edit_theme(self.pal, "My Red Retro");
                }
                "palette" => self.palette = Some(palette::PaletteState { query: std::env::var("DKFM_QUERY").unwrap_or_default(), ..Default::default() }),
                // Ctrl+K playlist picker for the first song matching DKFM_QUERY
                "palette-pick" => {
                    let q = std::env::var("DKFM_QUERY").unwrap_or_default().to_lowercase();
                    let id = self.lib.data.read().tracks.values().find(|t| t.title.to_lowercase().contains(&q)).map(|t| t.id.clone());
                    self.palette = Some(palette::PaletteState { query: q, pick: id.map(|i| plpick::Picker::new(vec![i])), ..Default::default() });
                }
                // right-click > Add to playlist… for 3 songs of the first playlist (one is in it, so its box is half-ticked)
                "pick" => {
                    let ids: Vec<String> = { let d = self.lib.data.read(); let p = d.playlists.iter().find(|p| p.track_ids.len() > 3); let mut v: Vec<String> = p.map(|p| p.track_ids.iter().take(1).cloned().collect()).unwrap_or_default(); let more: Vec<String> = d.tracks.keys().filter(|k| !v.contains(k)).take(2).cloned().collect(); v.extend(more); v };
                    self.browser.set_view(browser::View::All);
                    self.pick = Some(plpick::Popup::new(ids, egui::pos2(700.0, 200.0)));
                }
                // playlist view with the add-songs box open, searching DKFM_QUERY (online results too)
                "playlist-add" => {
                    let p = self.lib.data.read().playlists.iter().find(|p| p.track_ids.len() > 3).map(|p| p.id.clone());
                    if let Some(p) = p { self.browser.set_view(browser::View::Playlist(p)); }
                    self.browser.add.open = true;
                    self.browser.add.query = std::env::var("DKFM_QUERY").unwrap_or_default();
                }
                // share previews: a code made from this profile's own data, as if pasted
                v if v.starts_with("share-") => {
                    let s = match &v[6..] {
                        "theme" => Some(sharing::theme_share(self, "synthwave")),
                        "layout" => sharing::layout_share(self, Some("Default")),
                        "settings" => Some(sharing::settings_share(self)),
                        _ => {
                            let p = self.lib.data.read().playlists.iter().find(|p| p.track_ids.len() > 3).cloned();
                            p.map(|p| sharing::playlist_share(self, &p))
                        }
                    };
                    if let Some(s) = s {
                        sharing::receive(self, &format!("check this out {}", crate::share::encode(&s)));
                    }
                }
                "mini" => self.toggle_mini(ctx),
                "layout" => self.layout_edit = true,
                "eq" => self.show_panel(Tab::Eq),
                "radio" => {
                    let t = self.lib.data.read().tracks.values().find(|t| t.title.contains("Guitar")).cloned();
                    if let Some(t) = t { import::radio_for(self, &t); }
                }
                _ => {}
            }
            // DKFM_SHARE=<code or .dkfm file>: open its preview
            if let Ok(v) = std::env::var("DKFM_SHARE") {
                if !sharing::receive(self, &v) {
                    sharing::open_file(self, std::path::Path::new(&v));
                }
            }
        }
        let Ok(path) = std::env::var("DKFM_SCREENSHOT") else { return };
        let delay: f64 = std::env::var("DKFM_SCREENSHOT_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(3000.0);
        let t = ctx.input(|i| i.time) * 1000.0;
        static ASKED: AtomicBool = AtomicBool::new(false);
        // DKFM_SCROLL=<px>: Home / an artist page kept scrolled down (it grows as things load)
        if t > 500.0 {
            self.home.scroll = std::env::var("DKFM_SCROLL").ok().and_then(|v| v.parse().ok());
        }
        if t > delay && !ASKED.swap(true, Ordering::Relaxed) {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        let shot = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
        if let Some(img) = shot {
            let [w, h] = img.size;
            let px: Vec<u8> = img.pixels.iter().flat_map(|c| [c.r(), c.g(), c.b(), 255]).collect();
            if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, px) {
                let _ = buf.save(&path);
            }
            system::QUIT.store(true, Ordering::Relaxed);
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn persist(&mut self) {
        if self.last_flush.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_flush = Instant::now();
        let lib = self.lib.clone();
        let save_settings = self.settings_dirty.swap(false, Ordering::Relaxed);
        let s = if save_settings { Some(self.settings.lock().clone()) } else { None };
        std::thread::spawn(move || {
            lib.flush();
            if let Some(s) = s {
                s.save();
            }
        });
    }
}

struct Viewer<'a> {
    app: &'a mut App,
}

impl egui_dock::TabViewer for Viewer<'_> {
    type Tab = Tab;
    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        egui::RichText::new(format!("■ {}", tab.name())).font(px(7.0)).into()
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Deck => deck::show(self.app, ui),
            Tab::Scope => scope::show(self.app, ui),
            Tab::Library => browser::show(self.app, ui),
            Tab::Queue => queue::show(self.app, ui),
            Tab::Eq => eqpanel::show(self.app, ui),
            Tab::Lyrics => lyrics::show(self.app, ui),
        }
    }
    fn closeable(&mut self, _tab: &mut Tab) -> bool {
        self.app.layout_edit
    }
    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }
}

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(self.pal.bg).to_array()
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let t_update = Instant::now();
        self.frames += 1;
        self.scope.drawn = false;
        self.check_alphabets(ctx);
        if self.frames == 2 && self.started_hidden {
            system::hide_window();
        }
        // notices from background workers
        let n: Vec<String> = self.player.notices.lock().drain(..).chain(self.dl.notices.lock().drain(..)).collect();
        for m in n {
            self.toast(m);
        }
        // close button -> tray (unless quitting for real)
        if ctx.input(|i| i.viewport().close_requested()) {
            let to_tray = self.settings.lock().close_to_tray && cfg!(not(target_os = "linux")) && self.tray.is_some();
            if to_tray && !system::QUIT.load(Ordering::Relaxed) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                if self.mini {
                    self.toggle_mini(ctx);
                }
                system::hide_window();
                if !self.settings.lock().tray_hint_shown {
                    self.edit_settings(|s| s.tray_hint_shown = true);
                }
            } else {
                self.player.shutdown();
                self.lib.flush();
                self.settings.lock().save();
            }
        }
        self.shortcuts(ctx);
        // Ctrl+V of a friend's code anywhere outside a text box (text boxes check their own)
        if !ctx.wants_keyboard_input() {
            let pasted = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Paste(t) = e { Some(t.clone()) } else { None }));
            if let Some(t) = pasted {
                sharing::receive(self, &t);
            }
        }
        self.handle_drops(ctx);
        self.titlebar(ctx);

        if self.mini {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(self.pal.panel).inner_margin(egui::Margin::same(6))).show(ctx, |ui| deck::show_mini(self, ui));
        } else if self.nowplaying {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(self.pal.bg)).show(ctx, |ui| nowplaying::show(self, ui));
        } else {
            let pal = self.pal;
            egui::CentralPanel::default().frame(egui::Frame::new().fill(pal.bg).inner_margin(egui::Margin::same(4))).show(ctx, |ui| {
                let mut style = egui_dock::Style::from_egui(ui.style().as_ref());
                style.tab_bar.bg_fill = pal.bg2;
                style.tab_bar.height = 24.0;
                style.tab_bar.hline_color = pal.line;
                style.tab.active.bg_fill = pal.panel;
                style.tab.active.text_color = pal.accent;
                style.tab.inactive.bg_fill = pal.bg2;
                style.tab.inactive.text_color = pal.dim;
                style.tab.focused.bg_fill = pal.panel;
                style.tab.focused.text_color = pal.accent;
                style.tab.hovered.text_color = pal.text;
                style.tab.tab_body.bg_fill = pal.panel;
                style.tab.tab_body.stroke = egui::Stroke::new(2.0_f32, pal.line);
                style.tab.tab_body.inner_margin = egui::Margin::same(0);
                style.separator.color_idle = pal.bg;
                style.separator.color_hovered = pal.accent;
                style.separator.color_dragged = pal.accent2;
                style.separator.width = 5.0;
                style.main_surface_border_stroke = egui::Stroke::NONE;
                let mut dock = std::mem::replace(&mut self.dock, DockState::new(vec![]));
                let before = serde_json::to_string(&dock).unwrap_or_default();
                DockArea::new(&mut dock)
                    .style(style)
                    .draggable_tabs(self.layout_edit)
                    .show_close_buttons(self.layout_edit)
                    .show_add_buttons(false)
                    .tab_context_menus(false)
                    .show_leaf_close_all_buttons(false)
                    .show_leaf_collapse_buttons(false)
                    .show_inside(ui, &mut Viewer { app: self });
                if dock.iter_all_tabs().count() == 0 {
                    dock = default_dock();
                }
                let changed = serde_json::to_string(&dock).unwrap_or_default() != before;
                self.dock = dock;
                if changed {
                    self.save_dock();
                }
            });
            if self.layout_edit {
                egui::Area::new(Id::new("layout-banner")).anchor(Align2::CENTER_BOTTOM, [0.0, -14.0]).order(Order::Foreground).show(ctx, |ui| {
                    egui::Frame::new().fill(self.pal.accent2).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("DRAG PANEL TABS TO MOVE · DRAG GAPS TO RESIZE · × HIDES").font(px(8.0)).color(self.pal.ink));
                            if ui.button("RESET").clicked() {
                                self.reset_layout();
                            }
                            if ui.button("DONE").clicked() {
                                self.layout_edit = false;
                            }
                        });
                    });
                });
            }
        }

        // overlays: palette, modal, update popup
        plpick::show_popup(self, ctx);
        if self.palette.is_some() {
            palette::show(self, ctx);
        }
        settings::show_modal(self, ctx);
        sharing::show(self, ctx);
        if let UpdState::Available(u) = self.update.lock().clone() {
            if self.update_dismissed.as_deref() != Some(&u.version) && self.modal.is_none() {
                self.modal = Some(Modal::Update);
                self.update_dismissed = Some(u.version.clone());
            }
        }
        self.overlays(ctx);

        // tray + OS media overlay
        let st = self.player.status();
        let cur = self.current_track();
        let private = self.lib.private.load(Ordering::Relaxed);
        if let Some(t) = &self.tray {
            t.update(&cur.as_ref().map(|t| format!("{} — {}", t.title, t.artist)).unwrap_or_default(), st.playing, private);
        }
        if self.settings.lock().private_listening != private {
            self.edit_settings(|s| s.private_listening = private); // turned on/off from the tray
        }
        if let (Some(m), Some(t)) = (self.media.as_mut(), cur.as_ref()) {
            let cover = t.cover.as_ref().map(|c| self.lib.cover_path(c).to_string_lossy().into_owned());
            m.update(&t.title, &t.artist, &t.album, cover.as_deref(), st.duration, st.playing, st.position);
        }

        // repaint cadence: idle = only on input; playing = 4 Hz (time readout), or the
        // visualizer's fps while it's actually on screen; nothing while minimized or in the tray
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let tick = if st.playing && !minimized && !system::HIDDEN.load(Ordering::Relaxed) {
            if self.scope.drawn && self.scope.mode != "off" {
                1000 / self.player.st.lock().opts.vis_fps.clamp(10, 60)
            } else {
                250
            }
        } else {
            0
        };
        ticker::set(ctx, tick);
        self.persist();
        self.dev_hooks(ctx);
        profile(t_update.elapsed().as_secs_f32(), frame.info().cpu_usage.unwrap_or(0.0), ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.player.shutdown();
        self.lib.flush();
        self.settings.lock().save();
    }
}

/// `DKFM_PROFILE=1`: every 5 s, log average time per frame in our UI code (`update`) and for the
/// whole frame including egui's tessellation and painting, to profile.log in the data folder.
fn profile(update_s: f32, frame_s: f32, ctx: &egui::Context) {
    use std::io::Write;
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("DKFM_PROFILE").is_some()) {
        return;
    }
    static ACC: Mutex<(u32, f32, f32, Option<Instant>)> = Mutex::new((0, 0.0, 0.0, None));
    static CAUSES: Mutex<Option<std::collections::HashMap<String, u32>>> = Mutex::new(None);
    for c in ctx.repaint_causes() {
        *CAUSES.lock().get_or_insert_with(Default::default).entry(c.to_string()).or_default() += 1;
    }
    let mut a = ACC.lock();
    a.0 += 1;
    a.1 += update_s;
    a.2 += frame_s;
    let start = *a.3.get_or_insert_with(Instant::now);
    if start.elapsed() >= Duration::from_secs(5) {
        let causes = CAUSES.lock().take().unwrap_or_default();
        let line = format!("{} frames/5s  update {:.2} ms  whole frame {:.2} ms  causes {:?}
", a.0, a.1 / a.0 as f32 * 1000.0, a.2 / a.0 as f32 * 1000.0, causes);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(crate::store::data_dir().join("profile.log")) {
            let _ = f.write_all(line.as_bytes());
        }
        *a = (0, 0.0, 0.0, Some(Instant::now()));
    }
}

/// Paces animation frames from a timer thread. (egui's own `request_repaint_after` subtracts a
/// predicted frame time from every delay, which roughly doubled the visualizer's frame rate.)
mod ticker {
    use super::egui;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    static PERIOD_MS: AtomicU32 = AtomicU32::new(0);
    static THREAD: OnceLock<std::thread::Thread> = OnceLock::new();

    /// `ms` = 0 stops ticking (the thread sleeps until needed again).
    pub fn set(ctx: &egui::Context, ms: u32) {
        let old = PERIOD_MS.swap(ms, Ordering::Relaxed);
        let t = THREAD.get_or_init(|| {
            let ctx = ctx.clone();
            std::thread::Builder::new().name("ticker".into()).spawn(move || run(ctx)).expect("ticker thread").thread().clone()
        });
        if old == 0 && ms != 0 {
            t.unpark();
        }
    }

    fn run(ctx: egui::Context) {
        let mut next = Instant::now();
        loop {
            let ms = PERIOD_MS.load(Ordering::Relaxed);
            if ms == 0 {
                std::thread::park();
                next = Instant::now();
                continue;
            }
            next += Duration::from_millis(ms as u64);
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                next = now; // fell behind (e.g. the PC was asleep): don't try to catch up
            }
            if PERIOD_MS.load(Ordering::Relaxed) != 0 {
                // a non-zero delay asks for exactly one frame (zero would ask for two)
                ctx.request_repaint_after(Duration::from_nanos(1));
            }
        }
    }
}

//! Settings window, text prompts, and the "Update available" popup.
use super::browser::{self, COLUMNS, NAV, SCREENS, SORTS, START};
use super::keys::{self, Combo};
use super::theme::{self, px, vt, Pal};
use super::widgets::{button, caption, dim, fill, frame_rect, switch, tb_button};
use super::{App, Modal, PromptAction};
use crate::backup;
use crate::system::{self, UpdState};
use eframe::egui::{self, Align2, Rect, Sense, Ui, Vec2};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq)]
pub enum SetTab {
    Look,
    Layouts,
    Lists,
    Sidebar,
    Search,
    Discover,
    Library,
    Downloads,
    Spotify,
    Playback,
    Keys,
    Backup,
    System,
    Updates,
    About,
}

const TABS: [(SetTab, &str); 15] = [
    (SetTab::Look, "Look"),
    (SetTab::Layouts, "Layouts"),
    (SetTab::Lists, "Lists"),
    (SetTab::Sidebar, "Tabs & sidebar"),
    (SetTab::Search, "Search"),
    (SetTab::Discover, "Discover"),
    (SetTab::Library, "Library"),
    (SetTab::Downloads, "Downloads"),
    (SetTab::Spotify, "Spotify"),
    (SetTab::Playback, "Playback"),
    (SetTab::Keys, "Shortcuts"),
    (SetTab::Backup, "Backup"),
    (SetTab::System, "System"),
    (SetTab::Updates, "Updates"),
    (SetTab::About, "About"),
];

/// A tab by its (lowercase) name or first word, e.g. "look", "shortcuts", "tabs" (or "sidebar").
pub fn tab_named(name: &str) -> SetTab {
    if name.eq_ignore_ascii_case("sidebar") {
        return SetTab::Sidebar;
    }
    TABS.iter().find(|t| t.1.eq_ignore_ascii_case(name) || t.1.split(' ').next().is_some_and(|w| w.eq_ignore_ascii_case(name))).map(|t| t.0).unwrap_or(SetTab::Look)
}

/// State of the settings window between frames.
#[derive(Default)]
pub struct SetUi {
    /// shortcut being recorded
    capture: Option<&'static str>,
    /// theme editor: working copy (previewed live) and its name
    draft: Option<Pal>,
    draft_name: String,
    /// inline rename: (what: "theme" | "layout", old name, new text)
    rename: Option<(&'static str, String, String)>,
    layout_name: String,
    /// asking before restoring this backup / deleting all backups
    confirm: Option<Confirm>,
    /// backup list, re-read after changes or every few seconds
    backups: Option<(std::time::Instant, Vec<backup::Entry>)>,
    /// the search box above the tabs
    query: String,
}

enum Confirm {
    Restore(PathBuf),
    DeleteAll,
}

impl SetUi {
    /// Ask whether to restore this backup (Settings > Backup).
    pub fn ask_restore(&mut self, p: PathBuf) {
        self.confirm = Some(Confirm::Restore(p));
    }

    /// Open the theme editor on a palette (dev hook for screenshots).
    pub fn edit_theme(&mut self, p: Pal, name: &str) {
        self.draft = Some(p);
        self.draft_name = name.into();
    }
    pub fn capturing(&self) -> bool {
        self.capture.is_some()
    }
}

pub(super) fn window(pal: theme::Pal, ctx: &egui::Context, title: &str, size: Vec2, esc_closes: bool, body: impl FnOnce(&mut Ui)) -> bool {
    // dim the app behind
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("modal-dim")));
    fill(&painter, ctx.screen_rect(), egui::Color32::from_black_alpha(150));
    let mut open = true;
    egui::Window::new(egui::RichText::new(title).font(px(10.0)).color(pal.accent))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size(size)
        .order(egui::Order::Foreground)
        .open(&mut open)
        .frame(egui::Frame::window(&ctx.style()).fill(pal.panel).stroke(egui::Stroke::new(2.0_f32, pal.accent)).inner_margin(egui::Margin::same(14)))
        .show(ctx, body);
    let esc = esc_closes && ctx.input(|i| i.key_pressed(egui::Key::Escape));
    open && !esc
}

pub fn show_modal(app: &mut App, ctx: &egui::Context) {
    if app.incoming.is_some() {
        return; // a friend's share preview is on top; this comes back after
    }
    let Some(m) = app.modal.take() else { return };
    let keep = match m {
        Modal::Settings(tab) => {
            let mut t = tab;
            let esc = !app.setui.capturing();
            let open = window(app.pal, ctx, "SETTINGS", Vec2::new(780.0, 520.0), esc, |ui| settings_body(app, ui, &mut t));
            if open && !CLOSE.swap(false, std::sync::atomic::Ordering::Relaxed) {
                app.modal.get_or_insert(Modal::Settings(t));
                if let Some(Modal::Settings(x)) = app.modal.as_mut() {
                    *x = t;
                }
            } else {
                // closed: drop an unsaved theme preview and any half-finished edits
                if app.setui.draft.take().is_some() {
                    app.set_theme(ctx);
                }
                app.setui = Default::default();
            }
            false
        }
        Modal::Prompt { title, mut text, action } => {
            let mut done: Option<bool> = None;
            let open = window(app.pal, ctx, &title, Vec2::new(420.0, 90.0), true, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY).font(vt(20.0)));
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if button(ui, &app.pal, "CANCEL", false, true).clicked() {
                        done = Some(false);
                    }
                    if button(ui, &app.pal, "OK", true, true).clicked() {
                        done = Some(true);
                    }
                });
            });
            match done {
                Some(true) if !text.trim().is_empty() || matches!(action, PromptAction::Describe(_)) => {
                    run_prompt(app, action, text.trim().to_string());
                    false
                }
                Some(_) => false,
                None if open => {
                    app.modal = Some(Modal::Prompt { title, text, action });
                    false
                }
                None => false,
            }
        }
        Modal::Welcome { step, start_menu, desktop } => {
            app.modal = super::welcome::show_welcome(app, ctx, step, start_menu, desktop);
            false
        }
        Modal::WhatsNew => {
            if super::welcome::show_whats_new(app, ctx) {
                app.modal = Some(Modal::WhatsNew);
            }
            false
        }
        Modal::Update => {
            let st = app.update.lock().clone();
            let mut close = false;
            let open = window(app.pal, ctx, "UPDATE AVAILABLE", Vec2::new(440.0, 160.0), true, |ui| match &st {
                UpdState::Available(u) => {
                    ui.label(egui::RichText::new(format!("DK.FM v{}", u.version)).font(vt(24.0)).color(app.pal.text));
                    dim(ui, &app.pal, &format!("you have v{}", env!("CARGO_PKG_VERSION")));
                    if !u.notes.is_empty() {
                        ui.add_space(6.0);
                        caption(ui, &app.pal, "WHAT'S NEW");
                        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| ui.label(egui::RichText::new(&u.notes).color(app.pal.text)));
                    }
                    if u.asset.is_none() {
                        dim(ui, &app.pal, "This opens the download page — install the new version over the old one.");
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if button(ui, &app.pal, "LATER", false, true).clicked() {
                            close = true;
                        }
                        let label = if u.asset.is_some() { "DOWNLOAD & RESTART" } else { "GET UPDATE" };
                        if button(ui, &app.pal, label, true, true).clicked() {
                            let (u2, slot, ctx2) = (u.clone(), app.update.clone(), ctx.clone());
                            *slot.lock() = UpdState::Downloading(0);
                            std::thread::spawn(move || match system::install_update(&u2) {
                                Ok(()) if u2.asset.is_some() => {
                                    system::QUIT.store(true, std::sync::atomic::Ordering::Relaxed);
                                    ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                                Ok(()) => *slot.lock() = UpdState::Available(u2),
                                Err(e) => {
                                    *slot.lock() = UpdState::Error(e);
                                    ctx2.request_repaint();
                                }
                            });
                        }
                    });
                }
                UpdState::Downloading(_) => {
                    ui.label(egui::RichText::new("Downloading… DK.FM restarts when done.").color(app.pal.text));
                    ui.spinner();
                }
                UpdState::Error(e) => {
                    ui.label(egui::RichText::new(format!("Update failed: {e}")).color(app.pal.accent2));
                }
                _ => close = true,
            });
            if open && !close {
                app.modal = Some(Modal::Update);
            }
            false
        }
    };
    let _ = keep;
}

fn run_prompt(app: &mut App, action: PromptAction, text: String) {
    match action {
        PromptAction::NewPlaylist(ids) => {
            let n = ids.len();
            let id = app.lib.new_playlist(&text, ids);
            if n > 0 {
                app.edit_settings(|s| s.last_playlist = id);
            }
            app.toast(if n > 0 { format!("Saved \"{text}\" ({n} songs)") } else { format!("Created \"{text}\"") });
        }
        PromptAction::RenamePlaylist(id) => app.lib.edit_playlist(&id, |p| p.name = text),
        PromptAction::Describe(id) => app.lib.edit_playlist(&id, |p| p.description = text),
        PromptAction::NewFolder(pid) => {
            let f = app.lib.new_folder(&text);
            if let Some(pid) = pid {
                let pinned = app.lib.data.read().playlists.iter().any(|p| p.id == pid && p.pinned);
                app.lib.place_playlist(&pid, Some(f), pinned, None);
            }
            app.toast(format!("Created folder \"{text}\": drag playlists onto it"));
        }
        PromptAction::RenameFolder(id) => app.lib.edit_folder(&id, |f| f.name = text),
        PromptAction::PasteYoutube(job, idx) => {
            let re = regex::Regex::new(r"(?:v=|youtu\.be/|shorts/)([\w-]{11})").unwrap();
            let id = re.captures(&text).map(|c| c[1].to_string()).or(if text.len() == 11 { Some(text.clone()) } else { None });
            match id {
                Some(v) => app.dl.retry(&job, idx, Some(v)),
                None => app.toast_err("That is not a YouTube link"),
            }
        }
    }
}

fn row(ui: &mut Ui, pal: &theme::Pal, label: &str, f: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized(Vec2::new(170.0, 24.0), egui::Label::new(egui::RichText::new(label).color(pal.dim)));
        f(ui);
    });
}

/// Everything Settings has, for its search box: (tab, setting, other words people might type).
const FIND: &[(SetTab, &str, &str)] = &[
    (SetTab::Look, "Theme", "colors colours dark light red retro skin"),
    (SetTab::Look, "Album Cover theme: colours from the song playing", "adaptive artwork cover match dynamic influence"),
    (SetTab::Look, "Accent color", "colour highlight"),
    (SetTab::Look, "Font, text size and zoom", "pixel letters bigger smaller scale"),
    (SetTab::Look, "Song lists: compact or comfortable", "density rows spacing"),
    (SetTab::Look, "Album covers next to song titles", "art pictures"),
    (SetTab::Look, "CRT effects: scanlines and phosphor glow", "retro tv"),
    (SetTab::Look, "Visualizer style, bars and colours", "scope spectrum oscilloscope"),
    (SetTab::Look, "Theme editor and your own themes", "custom make"),
    (SetTab::Layouts, "Layout style: template or free (per pixel)", "panels move arrange"),
    (SetTab::Layouts, "Panel layouts: save, switch, reset", "default"),
    (SetTab::Playback, "THEATER: the song big, with its lyrics", "full screen fullscreen now playing study focus lyrics big cover f11"),
    (SetTab::Lists, "Song list columns", "year genre bitrate added album"),
    (SetTab::Lists, "Default sort", "order"),
    (SetTab::Lists, "When DK.FM starts: open to", "start screen"),
    (SetTab::Sidebar, "Tabs along the top of HOME", "order hide nav"),
    (SetTab::Sidebar, "Sort playlists by", "recently played name added custom order"),
    (SetTab::Sidebar, "Playlist covers in the sidebar", "pictures"),
    (SetTab::Search, "FIND MUSIC: versions per search", "results youtube music"),
    (SetTab::Search, "Songs like this: how many", "similar recommendations count"),
    (SetTab::Search, "Add songs search: results and sources", "soundcloud youtube"),
    (SetTab::Search, "Import matching strictness", "accuracy wrong songs"),
    (SetTab::Search, "Hide explicit songs", "clean swearing"),
    (SetTab::Discover, "Home: new releases from your artists", "albums singles"),
    (SetTab::Discover, "Recommended songs under each playlist", "suggestions"),
    (SetTab::Library, "Music folders", "scan local files add folder"),
    (SetTab::Downloads, "Download folder", "where saved location path"),
    (SetTab::Downloads, "Sound quality", "lossless flac high standard bitrate audio better quality mp3 upgrade smaller shrink space convert"),
    (SetTab::Downloads, "Parallel downloads", "speed faster at once"),
    (SetTab::Downloads, "Auto-sync imported playlists", "spotify update check hours"),
    (SetTab::Downloads, "File names", "pattern rename"),
    (SetTab::Downloads, "Download engine (yt-dlp)", "update tools"),
    (SetTab::Spotify, "Connect Spotify", "account login import whole library liked songs"),
    (SetTab::Spotify, "Spotify app: Client ID", "developer setup"),
    (SetTab::Playback, "Crossfade", "gapless fade between songs"),
    (SetTab::Playback, "Visualizer FPS", "frames smooth cpu"),
    (SetTab::Playback, "Smart shuffle", "random artists skipped"),
    (SetTab::Playback, "Volume matching", "loudness normalize replaygain"),
    (SetTab::Playback, "Leveler", "compressor quiet loud"),
    (SetTab::Playback, "Private listening", "history stats incognito"),
    (SetTab::Keys, "Keyboard shortcuts", "hotkeys keys bindings"),
    (SetTab::Backup, "Automatic backups", "restore save"),
    (SetTab::Backup, "Your backups: restore, export, import", "undo recover"),
    (SetTab::Backup, "Move to a new PC / give a friend your DK.FM", "transfer copy whole"),
    (SetTab::Backup, "Share your setup with a friend", "code theme layout"),
    (SetTab::System, "The X button: quit or keep playing in the tray", "close minimize background"),
    (SetTab::System, "Start with your computer", "startup login boot"),
    (SetTab::Updates, "Automatic updates", "new version check"),
    (SetTab::About, "About DK.FM", "version made by credits"),
];

/// Settings matching `q` (every word somewhere in the name, its tab or its keywords).
fn find(q: &str) -> Vec<(SetTab, &'static str)> {
    let words: Vec<String> = q.to_lowercase().split_whitespace().map(String::from).collect();
    FIND.iter()
        .filter(|(t, name, kw)| {
            let tab = TABS.iter().find(|x| x.0 == *t).map(|x| x.1).unwrap_or("");
            let hay = format!("{name} {tab} {kw}").to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|(t, name, _)| (*t, *name))
        .collect()
}

fn settings_body(app: &mut App, ui: &mut Ui, tab: &mut SetTab) {
    let pal = app.pal;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(150.0);
            ui.add(egui::TextEdit::singleline(&mut app.setui.query).hint_text("🔍 Search settings").desired_width(146.0).font(vt(18.0)));
            ui.add_space(4.0);
            for (t, name) in TABS {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(150.0, 28.0), Sense::click());
                if *tab == t {
                    fill(ui.painter(), r, pal.sel);
                    fill(ui.painter(), Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), pal.accent);
                }
                ui.painter().text(r.min + Vec2::new(12.0, 14.0), Align2::LEFT_CENTER, name, vt(20.0), if *tab == t { pal.accent } else if resp.hovered() { pal.text } else { pal.dim });
                if resp.clicked() {
                    *tab = t;
                    app.setui.capture = None;
                    app.setui.rename = None;
                    app.setui.confirm = None;
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("set-body").max_width(590.0).show(ui, |ui| {
            ui.vertical(|ui| {
            ui.set_width(580.0);
            let q = app.setui.query.trim().to_string();
            if !q.is_empty() {
                let hits = find(&q);
                caption(ui, &pal, &format!("SETTINGS MATCHING \"{}\"", q.to_uppercase()));
                if hits.is_empty() {
                    dim(ui, &pal, "Nothing matches. Try another word, like \"theme\", \"quality\" or \"spotify\".");
                }
                for (t, name) in hits {
                    let tab_name = TABS.iter().find(|x| x.0 == t).map(|x| x.1).unwrap_or("");
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
                    if resp.hovered() {
                        fill(ui.painter(), r, pal.panel_hi);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    ui.painter().text(r.left_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, name, vt(20.0), if resp.hovered() { pal.accent } else { pal.text });
                    ui.painter().text(r.right_center() - Vec2::new(8.0, 0.0), Align2::RIGHT_CENTER, format!("{} ›", tab_name.to_uppercase()), px(6.0), pal.dim);
                    if resp.clicked() {
                        *tab = t;
                        app.setui.query.clear();
                    }
                }
                return;
            }
            match tab {
                SetTab::Look => look(app, ui),
                SetTab::Layouts => layouts(app, ui),
                SetTab::Lists => lists(app, ui),
                SetTab::Sidebar => sidebar(app, ui),
                SetTab::Search => search(app, ui),
                SetTab::Discover => discover(app, ui),
                SetTab::Library => library(app, ui),
                SetTab::Downloads => downloads(app, ui),
                SetTab::Spotify => spotify(app, ui),
                SetTab::Playback => playback(app, ui),
                SetTab::Keys => keys(app, ui),
                SetTab::Backup => backups(app, ui),
                SetTab::System => system_tab(app, ui),
                SetTab::Updates => updates(app, ui),
                SetTab::About => about(app, ui),
            }
            });
        });
    });
}

/// A row of options, the current one highlighted; returns the clicked one.
fn choice<T: PartialEq + Clone>(ui: &mut Ui, pal: &Pal, cur: &T, opts: &[(T, &str)]) -> Option<T> {
    let mut out = None;
    ui.horizontal(|ui| {
        for (v, label) in opts {
            if tb_button(ui, pal, label, v == cur).clicked() && v != cur {
                out = Some(v.clone());
            }
        }
    });
    out
}

/// ▲ ▼ buttons; returns -1 / +1 when clicked.
fn updown(ui: &mut Ui, pal: &Pal, first: bool, last: bool) -> i32 {
    let mut d = 0;
    ui.add_enabled_ui(!first, |ui| {
        if tb_button(ui, pal, "▲", false).on_hover_text("Move up").clicked() {
            d = -1;
        }
    });
    ui.add_enabled_ui(!last, |ui| {
        if tb_button(ui, pal, "▼", false).on_hover_text("Move down").clicked() {
            d = 1;
        }
    });
    d
}

/// A small checkbox square; returns true when clicked.
fn tick(ui: &mut Ui, pal: &Pal, on: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
    let b = Rect::from_center_size(r.center(), Vec2::splat(14.0));
    frame_rect(ui.painter(), b, 2.0, if on || resp.hovered() { pal.accent } else { pal.line_hi });
    if on {
        fill(ui.painter(), b.shrink(4.0), pal.accent);
    }
    resp.clicked()
}

fn spacer(ui: &mut Ui) {
    ui.add_space(14.0);
}

// ------------------------------------------------------------------------------- look

fn look(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let s = app.settings.lock().clone();
    caption(ui, &pal, "THEME");
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for (k, name) in theme::all_themes(&s) {
            let tp = theme::theme_pal(&s, &k);
            let (r, resp) = ui.allocate_exact_size(Vec2::new(128.0, 80.0), Sense::click());
            fill(ui.painter(), r.translate(Vec2::splat(3.0)), pal.shadow);
            fill(ui.painter(), r, tp.bg);
            for (i, c) in [tp.accent, tp.accent2, tp.text].iter().enumerate() {
                let h = 14.0 + i as f32 * 10.0;
                fill(ui.painter(), Rect::from_min_size(egui::pos2(r.left() + 8.0 + i as f32 * 38.0, r.top() + 50.0 - h), Vec2::new(34.0, h)), *c);
            }
            fill(ui.painter(), Rect::from_min_max(egui::pos2(r.left(), r.bottom() - 26.0), r.max), pal.panel);
            ui.painter().with_clip_rect(r).text(egui::pos2(r.left() + 8.0, r.bottom() - 13.0), Align2::LEFT_CENTER, &name, vt(17.0), pal.text);
            frame_rect(ui.painter(), r, 2.0, if s.theme == k { pal.accent } else if resp.hovered() { pal.text } else { pal.line_hi });
            if resp.clicked() {
                chosen = Some(k);
            }
        }
    });
    if let Some(k) = chosen {
        app.setui.draft = None;
        app.edit_settings(|s| s.theme = k);
        app.set_theme(ui.ctx());
    }
    if s.theme == super::covertheme::KEY {
        ui.add_space(6.0);
        row(ui, &pal, "Cover influence", |ui| {
            let opts: Vec<(String, &str)> = super::covertheme::STRENGTHS.iter().map(|x| (x.0.to_string(), x.1)).collect();
            if let Some(v) = choice(ui, &pal, &s.cover_strength, &opts) {
                app.edit_settings(|s| s.cover_strength = v);
            }
        });
        dim(ui, &pal, "DK.FM takes its colours from the cover of the song playing, toned down so they always look good: accents follow the cover, backgrounds and text only take a tint. Grey and black-and-white covers keep Red Retro.");
    }
    ui.add_space(4.0);
    if button(ui, &pal, "SHARE CURRENT THEME…", false, true).on_hover_text("Copy a code for this theme (with your accent colour) to send to a friend").clicked() {
        let name = theme::all_themes(&s).into_iter().find(|t| t.0 == s.theme).map(|t| t.1).unwrap_or_default();
        let sh = super::sharing::theme_share(app, &s.theme);
        super::sharing::copy(app, ui.ctx(), &sh, &format!("theme \"{name}\""));
    }
    spacer(ui);
    caption(ui, &pal, "ACCENT COLOR");
    let mut col = s.accent.as_deref().and_then(theme::parse_hex).unwrap_or(pal.accent);
    ui.horizontal(|ui| {
        if ui.color_edit_button_srgba(&mut col).changed() {
            let h = theme::to_hex(col);
            app.edit_settings(|s| s.accent = Some(h));
            app.set_theme(ui.ctx());
        }
        if button(ui, &pal, "USE THEME DEFAULT", false, true).clicked() {
            app.edit_settings(|s| s.accent = None);
            app.set_theme(ui.ctx());
        }
    });
    spacer(ui);
    theme_editor(app, ui, &s);
    spacer(ui);
    caption(ui, &pal, "FONT");
    let _ = super::fonts::list(ui.ctx()); // start reading font names (once, in the background)
    let mut font = s.font.clone();
    let font_name = |f: &str| match f { "pixel" => "Pixel (VT323 + Press Start 2P)".to_string(), "clean" => "Clean (sans-serif)".to_string(), f => super::fonts::name_of(f.trim_start_matches("sys:")) };
    row(ui, &pal, "Text font", |ui| {
        egui::ComboBox::from_id_salt("font").selected_text(font_name(&font)).width(300.0).height(300.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut font, "pixel".to_string(), font_name("pixel"));
            ui.selectable_value(&mut font, "clean".to_string(), font_name("clean"));
            ui.separator();
            match super::fonts::list(ui.ctx()) {
                None => {
                    ui.label("Finding fonts on this computer…");
                }
                Some(list) => {
                    for f in list {
                        ui.selectable_value(&mut font, format!("sys:{}", f.path), &f.name);
                    }
                }
            }
        });
    });
    if font != s.font {
        app.edit_settings(|s| s.font = font.clone());
        app.apply_look(ui.ctx());
    }
    if font != "pixel" {
        let mut ph = s.pixel_headings;
        if switch(ui, &pal, &mut ph, "KEEP THE PIXEL FONT FOR HEADINGS & BUTTONS") {
            app.edit_settings(|s| s.pixel_headings = ph);
            app.apply_look(ui.ctx());
        }
        ui.add_space(4.0);
    }
    let zoom = (s.zoom * 100.0).round() as u32;
    row(ui, &pal, "Text size / zoom", |ui| {
        if let Some(z) = choice(ui, &pal, &zoom, &[(90, "90%"), (100, "100%"), (110, "110%"), (125, "125%"), (150, "150%")]) {
            app.edit_settings(|s| s.zoom = z as f32 / 100.0);
            app.apply_look(ui.ctx());
        }
    });
    row(ui, &pal, "Song lists", |ui| {
        if let Some(d) = choice(ui, &pal, &s.density, &[("compact".to_string(), "COMPACT"), ("comfortable".to_string(), "COMFORTABLE")]) {
            app.edit_settings(|s| s.density = d);
        }
    });
    let mut lc = s.list_covers;
    if switch(ui, &pal, &mut lc, "ALBUM COVERS NEXT TO SONG TITLES") {
        app.edit_settings(|s| s.list_covers = lc);
    }
    spacer(ui);
    caption(ui, &pal, "CRT EFFECTS");
    let (mut sc, mut gl) = (s.scanlines, s.glow);
    if switch(ui, &pal, &mut sc, "SCANLINES") {
        app.edit_settings(|s| s.scanlines = sc);
    }
    if switch(ui, &pal, &mut gl, "PHOSPHOR GLOW") {
        app.edit_settings(|s| s.glow = gl);
    }
    spacer(ui);
    caption(ui, &pal, "VISUALIZER");
    let mode = app.player.st.lock().opts.visualizer.clone();
    row(ui, &pal, "Style", |ui| {
        let opts: Vec<(String, &str)> = super::scope::MODES.iter().map(|m| (m.0.to_string(), m.1)).collect();
        if let Some(m) = choice(ui, &pal, &mode, &opts) {
            let fps = app.player.st.lock().opts.vis_fps;
            app.player.set_visualizer(&m, fps);
            app.scope.mode = m;
            app.scope.invalidate();
        }
    });
    row(ui, &pal, "Bars", |ui| {
        if let Some(b) = choice(ui, &pal, &s.vis_bars, &[(0, "FIT"), (16, "16"), (32, "32"), (48, "48"), (64, "64")]) {
            app.edit_settings(|s| s.vis_bars = b);
            app.apply_look(ui.ctx());
        }
    });
    let mut follow = s.vis_colors.is_none();
    if switch(ui, &pal, &mut follow, "COLOURS FOLLOW THE THEME") {
        let v = if follow { None } else { Some([theme::to_hex(pal.accent), theme::to_hex(pal.accent2), theme::to_hex(pal.text)]) };
        app.edit_settings(|s| s.vis_colors = v);
        app.apply_look(ui.ctx());
    }
    if let Some(cols) = &s.vis_colors {
        row(ui, &pal, "Low · mid · peak", |ui| {
            let mut c = cols.clone();
            let mut changed = false;
            for h in c.iter_mut() {
                let mut col = theme::parse_hex(h).unwrap_or(pal.accent);
                if ui.color_edit_button_srgba(&mut col).changed() {
                    *h = theme::to_hex(col);
                    changed = true;
                }
            }
            if changed {
                app.edit_settings(|s| s.vis_colors = Some(c));
                app.apply_look(ui.ctx());
            }
        });
    }
}

fn theme_editor(app: &mut App, ui: &mut Ui, s: &crate::store::Settings) {
    let pal = app.pal;
    caption(ui, &pal, "THEME EDITOR");
    dim(ui, &pal, "Start from any theme, change any colour (you see it right away), then save it as your own theme.");
    let themes = theme::all_themes(s);
    let mut start = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Start from").color(pal.dim));
        egui::ComboBox::from_id_salt("edit-from").selected_text(if app.setui.draft.is_some() { "(editing)" } else { "Pick a theme…" }).width(200.0).show_ui(ui, |ui| {
            for (k, name) in &themes {
                if ui.selectable_label(false, name).clicked() {
                    start = Some((k.clone(), name.clone()));
                }
            }
        });
        if app.setui.draft.is_none() && button(ui, &pal, "EDIT CURRENT THEME", false, true).clicked() {
            let name = themes.iter().find(|t| t.0 == s.theme).map(|t| t.1.clone()).unwrap_or_default();
            start = Some((s.theme.clone(), name));
        }
    });
    if let Some((k, name)) = start {
        app.setui.draft = Some(if k == s.theme { app.pal } else { theme::theme_pal(s, &k) });
        app.setui.draft_name = if k.starts_with(theme::CUSTOM) { name } else { format!("My {name}") };
    }
    if let Some(mut d) = app.setui.draft {
        let mut changed = false;
        egui::Grid::new("theme-grid").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
            for (i, (f, label)) in theme::FIELDS.iter().enumerate() {
                let c = d.field(f).unwrap();
                changed |= ui.color_edit_button_srgba(c).changed();
                ui.label(egui::RichText::new(*label).color(pal.text));
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        let mut dark = d.dark;
        if switch(ui, &pal, &mut dark, "DARK THEME (WIDGET SHADING)") {
            d.dark = dark;
            changed = true;
        }
        if changed {
            app.setui.draft = Some(d);
            app.pal = d;
            theme::apply(ui.ctx(), &d);
            app.scope.invalidate();
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Name").color(pal.dim));
            ui.add(egui::TextEdit::singleline(&mut app.setui.draft_name).desired_width(200.0));
            let name = app.setui.draft_name.trim().to_string();
            if button(ui, &pal, "SAVE THEME", true, !name.is_empty()).clicked() {
                let ct = d.to_custom(&name);
                app.edit_settings(|s| {
                    s.custom_themes.retain(|c| c.name != name);
                    s.custom_themes.push(ct);
                    s.theme = format!("{}{name}", theme::CUSTOM);
                    s.accent = None;
                });
                app.setui.draft = None;
                app.set_theme(ui.ctx());
                app.toast(format!("Saved theme \"{name}\""));
            }
            if button(ui, &pal, "DISCARD", false, true).clicked() {
                app.setui.draft = None;
                app.set_theme(ui.ctx());
            }
        });
    }
    // your themes
    if !s.custom_themes.is_empty() {
        ui.add_space(8.0);
        caption(ui, &pal, "YOUR THEMES");
        let mut action: Option<(&str, String, String)> = None;
        for c in &s.custom_themes {
            ui.horizontal(|ui| {
                if let Some((_, old, text)) = app.setui.rename.as_mut().filter(|r| r.0 == "theme" && r.1 == c.name) {
                    ui.add(egui::TextEdit::singleline(text).desired_width(200.0));
                    if button(ui, &pal, "OK", true, !text.trim().is_empty()).clicked() {
                        action = Some(("rename", old.clone(), text.trim().to_string()));
                    }
                    if button(ui, &pal, "CANCEL", false, true).clicked() {
                        action = Some(("cancel", String::new(), String::new()));
                    }
                } else {
                    cell(ui, 200.0, egui::RichText::new(&c.name).color(pal.text));
                    if button(ui, &pal, "USE", false, true).clicked() {
                        action = Some(("use", c.name.clone(), String::new()));
                    }
                    if button(ui, &pal, "RENAME", false, true).clicked() {
                        action = Some(("start", c.name.clone(), String::new()));
                    }
                    if button(ui, &pal, "SHARE…", false, true).on_hover_text("Copy a code to send to a friend").clicked() {
                        action = Some(("share", c.name.clone(), String::new()));
                    }
                    if button(ui, &pal, "DELETE", false, true).clicked() {
                        action = Some(("delete", c.name.clone(), String::new()));
                    }
                }
            });
        }
        let key = |n: &str| format!("{}{n}", theme::CUSTOM);
        if let Some(("share", n, _)) = &action {
            if let Some(c) = s.custom_themes.iter().find(|c| c.name == *n) {
                super::sharing::copy(app, ui.ctx(), &crate::share::Share::Theme(c.clone()), &format!("theme \"{n}\""));
            }
        }
        match action {
            Some(("use", n, _)) => {
                app.edit_settings(|s| s.theme = key(&n));
                app.set_theme(ui.ctx());
            }
            Some(("start", n, _)) => app.setui.rename = Some(("theme", n.clone(), n)),
            Some(("cancel", _, _)) => app.setui.rename = None,
            Some(("rename", old, new)) => {
                app.edit_settings(|s| {
                    if !s.custom_themes.iter().any(|c| c.name == new) {
                        if let Some(c) = s.custom_themes.iter_mut().find(|c| c.name == old) {
                            c.name = new.clone();
                        }
                        if s.theme == key(&old) {
                            s.theme = key(&new);
                        }
                    }
                });
                app.setui.rename = None;
                app.set_theme(ui.ctx());
            }
            Some(("delete", n, _)) => {
                app.edit_settings(|s| {
                    s.custom_themes.retain(|c| c.name != n);
                    if s.theme == key(&n) {
                        s.theme = "red-retro".into();
                    }
                });
                app.set_theme(ui.ctx());
                app.toast(format!("Deleted theme \"{n}\""));
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------------------- layouts

fn layouts(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    caption(ui, &pal, "LAYOUT STYLE");
    let mode = app.settings.lock().layout_mode.clone();
    row(ui, &pal, "Panels", |ui| {
        if let Some(m) = choice(ui, &pal, &mode, &[("template".to_string(), "TEMPLATE"), ("free".to_string(), "FREE (PER PIXEL)")]) {
            app.edit_settings(|s| s.layout_mode = m);
        }
    });
    dim(ui, &pal, if mode == "free" { "Free: put every panel exactly where you want it, any size, even overlapping. Ctrl+E, then drag a title bar to move and the corner to resize." } else { "Template: panels snap together side by side and fill the window. Ctrl+E, then drag tabs to move and the gaps to resize." });
    spacer(ui);
    caption(ui, &pal, "PANEL LAYOUT");
    dim(ui, &pal, "Arrange the panels (LAYOUT in the title bar, or Ctrl+E), then save the arrangement here. Switch any time — also from Ctrl+K: type \"layout\".");
    ui.horizontal(|ui| {
        if button(ui, &pal, "EDIT LAYOUT", false, true).clicked() {
            if !app.layout_edit {
                app.toggle_layout_edit();
            }
            CLOSE.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if button(ui, &pal, "RESET PANEL LAYOUT", false, true).on_hover_text("Back to the default layout (UNDO / Ctrl+Z puts yours back)").clicked() {
            app.reset_layout();
        }
    });
    dim(ui, &pal, "In a small window (not maximized), panels that don't fit fold into tabs for the time being; the layout you set comes back at full size.");
    spacer(ui);
    caption(ui, &pal, "SAVE THE CURRENT LAYOUT");
    ui.horizontal(|ui| {
        let te = ui.add(egui::TextEdit::singleline(&mut app.setui.layout_name).hint_text("Name, e.g. Focus").desired_width(220.0));
        let name = app.setui.layout_name.trim().to_string();
        let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (button(ui, &pal, "SAVE", true, !name.is_empty()).clicked() || enter && !name.is_empty()) && !App::PRESETS.contains(&name.as_str()) {
            app.save_layout(&name);
            app.setui.layout_name.clear();
            app.toast(format!("Saved layout \"{name}\""));
        }
        if button(ui, &pal, "SHARE CURRENT…", false, true).on_hover_text("Copy a code for the current arrangement to send to a friend").clicked() {
            if let Some(sh) = super::sharing::layout_share(app, None) {
                super::sharing::copy(app, ui.ctx(), &sh, "your layout");
            }
        }
    });
    spacer(ui);
    caption(ui, &pal, "LAYOUTS");
    let saved: Vec<String> = app.settings.lock().layouts.iter().map(|l| l.name.clone()).collect();
    let mut action: Option<(&str, String, String)> = None;
    for name in app.layout_names() {
        let builtin = App::PRESETS.contains(&name.as_str()) && !saved.contains(&name);
        ui.horizontal(|ui| {
            if let Some((_, old, text)) = app.setui.rename.as_mut().filter(|r| r.0 == "layout" && r.1 == name) {
                ui.add(egui::TextEdit::singleline(text).desired_width(200.0));
                if button(ui, &pal, "OK", true, !text.trim().is_empty()).clicked() {
                    action = Some(("rename", old.clone(), text.trim().to_string()));
                }
                if button(ui, &pal, "CANCEL", false, true).clicked() {
                    action = Some(("cancel", String::new(), String::new()));
                }
                return;
            }
            cell(ui, 200.0, egui::RichText::new(if builtin { format!("{name} (built-in)") } else { name.clone() }).color(pal.text));
            if button(ui, &pal, "USE", true, true).clicked() {
                action = Some(("use", name.clone(), String::new()));
            }
            if button(ui, &pal, "SHARE…", false, true).on_hover_text("Copy a code to send to a friend").clicked() {
                action = Some(("share", name.clone(), String::new()));
            }
            if !builtin {
                if button(ui, &pal, "RENAME", false, true).clicked() {
                    action = Some(("start", name.clone(), String::new()));
                }
                if button(ui, &pal, "DELETE", false, true).clicked() {
                    action = Some(("delete", name.clone(), String::new()));
                }
            }
        });
    }
    match action {
        Some(("use", n, _)) => {
            app.apply_layout(&n);
            app.toast(format!("Layout: {n}"));
        }
        Some(("share", n, _)) => {
            if let Some(sh) = super::sharing::layout_share(app, Some(&n)) {
                super::sharing::copy(app, ui.ctx(), &sh, &format!("layout \"{n}\""));
            }
        }
        Some(("start", n, _)) => app.setui.rename = Some(("layout", n.clone(), n)),
        Some(("cancel", _, _)) => app.setui.rename = None,
        Some(("rename", old, new)) => {
            app.edit_settings(|s| {
                if !s.layouts.iter().any(|l| l.name == new) {
                    if let Some(l) = s.layouts.iter_mut().find(|l| l.name == old) {
                        l.name = new;
                    }
                }
            });
            app.setui.rename = None;
        }
        Some(("delete", n, _)) => app.edit_settings(|s| s.layouts.retain(|l| l.name != n)),
        _ => {}
    }
}

// ------------------------------------------------------------------------------- lists

fn lists(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let s = app.settings.lock().clone();
    caption(ui, &pal, "SONG LIST COLUMNS");
    dim(ui, &pal, "Tick the columns to show and move them into the order you like. Narrow lists keep only #, title, artist and length.");
    let mut cols = s.columns.clone();
    let all: Vec<&str> = cols.iter().filter_map(|c| COLUMNS.iter().find(|x| x.id == c).map(|x| x.id)).chain(COLUMNS.iter().map(|c| c.id).filter(|c| !s.columns.iter().any(|x| x == c))).collect();
    let mut changed = false;
    let n_on = cols.len();
    for id in &all {
        let c = COLUMNS.iter().find(|x| x.id == *id).unwrap();
        let pos = cols.iter().position(|x| x == id);
        ui.horizontal(|ui| {
            if tick(ui, &pal, pos.is_some()) && (pos.is_none() || *id != "title") {
                match pos {
                    Some(i) => {
                        cols.remove(i);
                    }
                    None => cols.push(id.to_string()),
                }
                changed = true;
            }
            cell(ui, 240.0, egui::RichText::new(c.name).color(if pos.is_some() { pal.text } else { pal.dim }));
            if let Some(i) = pos {
                let d = updown(ui, &pal, i == 0, i + 1 == n_on);
                if d != 0 {
                    cols.swap(i, (i as i32 + d) as usize);
                    changed = true;
                }
            }
        });
    }
    if button(ui, &pal, "RESET COLUMNS", false, cols != crate::store::d_columns()).clicked() {
        cols = crate::store::d_columns();
        changed = true;
    }
    if changed {
        app.edit_settings(|s| s.columns = cols);
    }
    spacer(ui);
    caption(ui, &pal, "DEFAULT SORT");
    dim(ui, &pal, "How each screen is sorted when you open it (click a column header to sort differently for now).");
    for (screen, name, def) in SCREENS {
        let cur = browser::parse_sort(s.sorts.get(screen).map(|x| x.as_str()).unwrap_or(def));
        let mut next = cur;
        row(ui, &pal, name, |ui| {
            let label = |k: Option<(browser::SortKey, bool)>| k.and_then(|(k, _)| SORTS.iter().find(|x| x.0 == k)).map(|x| x.2).unwrap_or(if screen == "playlist" { "Playlist order" } else { "Unsorted" });
            egui::ComboBox::from_id_salt(("sort", screen)).selected_text(label(cur)).width(160.0).show_ui(ui, |ui| {
                if screen == "playlist" && ui.selectable_label(cur.is_none(), "Playlist order").clicked() {
                    next = None;
                }
                for (k, _, n) in SORTS {
                    if ui.selectable_label(cur.map(|c| c.0) == Some(k), n).clicked() {
                        next = Some((k, cur.map(|c| c.1).unwrap_or(!matches!(k, browser::SortKey::Plays | browser::SortKey::Added))));
                    }
                }
            });
            if let Some((k, asc)) = cur {
                if tb_button(ui, &pal, if asc { "▲ ASCENDING" } else { "▼ DESCENDING" }, false).clicked() {
                    next = Some((k, !asc));
                }
            }
        });
        if next != cur {
            let t = browser::sort_text(next);
            app.edit_settings(|s| {
                if t == def {
                    s.sorts.remove(screen);
                } else {
                    s.sorts.insert(screen.to_string(), t);
                }
            });
            app.browser.sort = None;
        }
    }
    spacer(ui);
    caption(ui, &pal, "WHEN DK.FM STARTS");
    row(ui, &pal, "Open to", |ui| {
        let mut v = s.start_view.clone();
        egui::ComboBox::from_id_salt("start").selected_text(START.iter().find(|x| x.0 == v).map(|x| x.1).unwrap_or("All Tracks")).width(200.0).show_ui(ui, |ui| {
            for (k, n) in START {
                ui.selectable_value(&mut v, k.to_string(), n);
            }
        });
        if v != s.start_view {
            app.edit_settings(|s| s.start_view = v);
        }
    });
    spacer(ui);
    hidden_songs(app, ui);
}

/// Songs you hid ("Don't play this"), to unhide.
fn hidden_songs(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let mut hidden: Vec<crate::store::Track> = { let d = app.lib.data.read(); d.stats.iter().filter(|(_, s)| s.hidden).filter_map(|(id, _)| d.tracks.get(id).cloned()).collect() };
    hidden.sort_by_cached_key(|t| (t.artist.to_lowercase(), t.title.to_lowercase()));
    caption(ui, &pal, &format!("HIDDEN SONGS ({})", hidden.len()));
    dim(ui, &pal, "Right-click a song > Hide song: it stays in your lists (dimmed, 🚫) but shuffle, playlists and radio skip it. It still plays when you pick it yourself.");
    let mut unhide: Vec<String> = Vec::new();
    for t in &hidden {
        ui.horizontal(|ui| {
            cell(ui, 440.0, egui::RichText::new(format!("{} — {}", t.title, t.artist)).color(pal.text));
            if tb_button(ui, &pal, "UNHIDE", false).clicked() {
                unhide.push(t.id.clone());
            }
        });
    }
    if hidden.len() > 1 && button(ui, &pal, "UNHIDE ALL", false, true).clicked() {
        unhide = hidden.iter().map(|t| t.id.clone()).collect();
    }
    if !unhide.is_empty() {
        app.lib.set_hidden(&unhide, false);
    }
}

fn sidebar(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let (mut order, mut hidden) = { let s = app.settings.lock(); (browser::nav_order(&s.sidebar_order), s.sidebar_hidden.clone()) };
    caption(ui, &pal, "TABS");
    dim(ui, &pal, "Choose which screens the tabs above your library show, and their order. The left sidebar lists only your playlists. Hidden ones stay in the ••• menu at the right end.");
    let mut changed = false;
    for (g, title) in [("tab", "TABS"), ("more", "RIGHT END: IMPORT · DOWNLOADS · ••• MENU")] {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(title).font(px(6.0)).color(pal.dim));
        let ids: Vec<&str> = order.iter().copied().filter(|id| NAV.iter().any(|s| s.0 == *id && s.1 == g)).collect();
        for (i, id) in ids.iter().enumerate() {
            let (_, _, ico, _, label) = NAV.iter().find(|s| s.0 == *id).unwrap();
            let shown = !hidden.iter().any(|h| h == id);
            ui.horizontal(|ui| {
                if tick(ui, &pal, shown) {
                    if shown { hidden.push(id.to_string()) } else { hidden.retain(|h| h != id) }
                    changed = true;
                }
                cell(ui, 240.0, egui::RichText::new(format!("{ico}  {label}")).color(if shown { pal.text } else { pal.dim }));
                let d = updown(ui, &pal, i == 0, i + 1 == ids.len());
                if d != 0 {
                    let other = ids[(i as i32 + d) as usize];
                    let (a, b) = (order.iter().position(|x| x == id).unwrap(), order.iter().position(|x| *x == other).unwrap());
                    order.swap(a, b);
                    changed = true;
                }
            });
        }
    }
    spacer(ui);
    caption(ui, &pal, "PLAYLISTS");
    dim(ui, &pal, "Drag playlists to reorder them (custom order), onto a folder to file them, or right-click > Pin to top. Make folders with the ↕ button above the list.");
    let (sort, mut covers) = { let s = app.settings.lock(); (s.playlist_sort.clone(), s.sidebar_covers) };
    row(ui, &pal, "Sort playlists by", |ui| {
        let opts: Vec<(String, &str)> = browser::PL_SORTS.iter().map(|(k, n)| (k.to_string(), *n)).collect();
        if let Some(v) = choice(ui, &pal, &sort, &opts) {
            app.edit_settings(|s| s.playlist_sort = v);
        }
    });
    if switch(ui, &pal, &mut covers, "SHOW PLAYLIST COVERS IN THE SIDEBAR") {
        app.edit_settings(|s| s.sidebar_covers = covers);
    }
    ui.add_space(8.0);
    if button(ui, &pal, "RESET TABS", false, true).clicked() {
        order = browser::nav_order(&[]);
        hidden.clear();
        changed = true;
    }
    if changed {
        app.edit_settings(|s| {
            s.sidebar_order = order.iter().map(|x| x.to_string()).collect();
            s.sidebar_hidden = hidden;
        });
    }
}

// ------------------------------------------------------------------------------- search

const SOURCE_NAMES: [(&str, &str); 4] = [("library", "Your library"), ("songs", "YouTube Music (SONGS)"), ("youtube", "YouTube"), ("soundcloud", "SoundCloud")];

fn search(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let s = app.settings.lock().clone();
    caption(ui, &pal, "FIND MUSIC");
    dim(ui, &pal, "FIND MUSIC searches YouTube Music. Under ALL it lists a few versions of the song to pick from (clean, explicit, live...); SEE ALL / LOAD MORE list more.");
    row(ui, &pal, "Versions per search", |ui| {
        if let Some(n) = choice(ui, &pal, &s.web_results, &[(1, "1"), (3, "3"), (5, "5"), (10, "10")]) {
            app.edit_settings(|s| s.web_results = n);
        }
    });
    spacer(ui);
    caption(ui, &pal, "SONGS LIKE THIS");
    dim(ui, &pal, "How many songs \"Songs like this\" lists at first (SHOW MORE lists more).");
    row(ui, &pal, "Songs to list", |ui| {
        if let Some(n) = choice(ui, &pal, &s.like_count, &[(5, "5"), (10, "10"), (20, "20"), (50, "50")]) {
            app.edit_settings(|s| s.like_count = n);
        }
    });
    spacer(ui);
    caption(ui, &pal, "ADD SONGS SEARCH");
    dim(ui, &pal, "The ADD SONGS box in a playlist searches your library and finds new songs online.");
    row(ui, &pal, "Results per search", |ui| {
        if let Some(n) = choice(ui, &pal, &s.search_results, &[(5, "5"), (10, "10"), (25, "25"), (50, "50")]) {
            app.edit_settings(|s| s.search_results = n);
        }
    });
    row(ui, &pal, "Start on tab", |ui| {
        let opts: Vec<(String, &str)> = super::addsongs::SOURCES.iter().filter(|x| s.search_sources.iter().any(|k| k == x.0)).map(|x| (x.0.to_string(), x.1)).collect();
        if let Some(k) = choice(ui, &pal, &s.search_source, &opts) {
            app.edit_settings(|s| s.search_source = k);
        }
    });
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Where to look, in order").color(pal.dim));
    let mut order: Vec<String> = s.search_sources.iter().filter(|k| SOURCE_NAMES.iter().any(|x| x.0 == k.as_str())).cloned().collect();
    let all: Vec<&str> = order.iter().map(|s| s.as_str()).chain(SOURCE_NAMES.iter().map(|x| x.0).filter(|k| !s.search_sources.iter().any(|x| x == k))).map(|k| SOURCE_NAMES.iter().find(|x| x.0 == k).unwrap().0).collect();
    let mut changed = false;
    let n_on = order.len();
    for k in all {
        let pos = order.iter().position(|x| x == k);
        ui.horizontal(|ui| {
            if tick(ui, &pal, pos.is_some()) {
                match pos {
                    Some(i) => {
                        order.remove(i);
                    }
                    None => order.push(k.to_string()),
                }
                changed = true;
            }
            cell(ui, 240.0, egui::RichText::new(SOURCE_NAMES.iter().find(|x| x.0 == k).unwrap().1).color(if pos.is_some() { pal.text } else { pal.dim }));
            if let Some(i) = pos {
                let d = updown(ui, &pal, i == 0, i + 1 == n_on);
                if d != 0 {
                    order.swap(i, (i as i32 + d) as usize);
                    changed = true;
                }
            }
        });
    }
    if changed {
        app.edit_settings(|s| s.search_sources = order);
    }
    ui.add_space(6.0);
    let mut hx = s.hide_explicit;
    if switch(ui, &pal, &mut hx, "HIDE EXPLICIT SONGS") {
        app.edit_settings(|s| s.hide_explicit = hx);
    }
    dim(ui, &pal, "Where the source marks them: YouTube Music search results, and Spotify imports (explicit songs start unticked).");
    spacer(ui);
    caption(ui, &pal, "IMPORT MATCHING");
    dim(ui, &pal, "When importing from Spotify, DK.FM finds each song on YouTube Music / YouTube. How sure must it be?");
    row(ui, &pal, "Match strictness", |ui| {
        if let Some(m) = choice(ui, &pal, &s.match_strictness, &[("relaxed".to_string(), "RELAXED"), ("normal".to_string(), "NORMAL"), ("strict".to_string(), "STRICT")]) {
            app.edit_settings(|s| s.match_strictness = m);
        }
    });
    dim(ui, &pal, match s.match_strictness.as_str() {
        "relaxed" => "Relaxed: finds more songs, but sometimes a live or cover version.",
        "strict" => "Strict: only clear matches; more songs may be skipped (pick them by hand in Downloads).",
        _ => "Normal: a good balance (recommended).",
    });
}

fn discover(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let s = app.settings.lock().clone();
    caption(ui, &pal, "HOME");
    dim(ui, &pal, "Home (top of the sidebar) shows what you played lately, mixes made from your library right on this PC, your top songs this month and new releases. Make it your start screen in Lists > When DK.FM starts.");
    let mut nr = s.new_releases;
    if switch(ui, &pal, &mut nr, "NEW RELEASES FROM YOUR ARTISTS") {
        app.edit_settings(|s| s.new_releases = nr);
    }
    dim(ui, &pal, "Looks up your most played artists on YouTube Music at most once a day, and only while Home is open. Nothing downloads until you press + GET.");
    row(ui, &pal, "Artists to check", |ui| {
        if let Some(n) = choice(ui, &pal, &s.release_artists, &[(5, "5"), (10, "10"), (20, "20"), (30, "30")]) {
            app.edit_settings(|s| s.release_artists = n);
        }
    });
    spacer(ui);
    caption(ui, &pal, "PLAYLISTS");
    let mut rec = s.recommend;
    if switch(ui, &pal, &mut rec, "RECOMMENDED SONGS UNDER EACH PLAYLIST") {
        app.edit_settings(|s| s.recommend = rec);
    }
    dim(ui, &pal, "At the end of a playlist DK.FM suggests 10 songs like it (YouTube Music radio of a few of its songs), leaving out songs you have or hid. + downloads one straight into the playlist; REFRESH shows others. Only looked up when you scroll down to it.");
}

fn library(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    caption(ui, &pal, "MUSIC FOLDERS");
    dim(ui, &pal, "DK.FM scans these folders (and your download folder) and watches them for new files.");
    let folders = app.settings.lock().music_folders.clone();
    for f in folders {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&f).color(pal.text));
            if button(ui, &pal, "REMOVE", false, true).clicked() {
                app.edit_settings(|s| s.music_folders.retain(|x| *x != f));
                app.rescan();
            }
        });
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if button(ui, &pal, "+ ADD FOLDER", true, true).clicked() {
            super::browser::add_folder(app);
        }
        if button(ui, &pal, "RESCAN NOW", false, true).clicked() {
            app.rescan();
            app.toast("Scanning library…");
        }
    });
    ui.add_space(14.0);
    caption(ui, &pal, "FILES");
    if button(ui, &pal, "ADD INDIVIDUAL FILES…", false, true).clicked() {
        if let Some(files) = rfd::FileDialog::new().add_filter("Audio", crate::library::AUDIO_EXT).pick_files() {
            let n = app.lib.add_files(&files, |_| {}).len();
            app.toast(format!("Added {n} songs"));
        }
    }
    dim(ui, &pal, "Tip: you can also drag audio files or folders onto the window.");
}

fn downloads(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    caption(ui, &pal, "DOWNLOAD FOLDER");
    let dir = app.settings.lock().download_dir.clone();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&dir).color(pal.text));
        if button(ui, &pal, "CHANGE", false, true).clicked() {
            if let Some(f) = rfd::FileDialog::new().pick_folder() {
                app.edit_settings(|s| s.download_dir = f.to_string_lossy().into_owned());
                app.rescan();
            }
        }
    });
    ui.add_space(14.0);
    caption(ui, &pal, "SOUND QUALITY");
    let fmt = app.settings.lock().download_format.clone();
    // older settings saved "m4a", which is HIGH now
    let cur = if matches!(fmt.as_str(), "standard" | "lossless" | "mp3-320" | "mp3-v0") { fmt.clone() } else { "high".to_string() };
    row(ui, &pal, "Downloads", |ui| {
        if let Some(f) = choice(ui, &pal, &cur, &[("standard".to_string(), "STANDARD"), ("high".to_string(), "HIGH"), ("lossless".to_string(), "LOSSLESS")]) {
            app.edit_settings(|s| s.download_format = f);
        }
    });
    dim(ui, &pal, match cur.as_str() {
        "standard" => "YouTube's AAC stream as it is (about 128 kbps). Smallest files, about 4 MB a song.",
        "lossless" => "The best stream YouTube has, saved as FLAC so nothing more is lost on the way. YouTube itself has no lossless audio, so it sounds like HIGH but files are about 5x bigger (25-40 MB a song).",
        "mp3-320" | "mp3-v0" => "MP3, for old devices and car stereos. Slightly lower quality than HIGH.",
        _ => "The best stream YouTube has (Opus, about 160 kbps), kept as 256 kbps AAC. Recommended: the best sound for its size, about 8 MB a song.",
    });
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let mut mp3 = cur.starts_with("mp3");
        if switch(ui, &pal, &mut mp3, "SAVE AS MP3 INSTEAD (OLD DEVICES)") {
            app.edit_settings(|s| s.download_format = if mp3 { "mp3-320".into() } else { "high".into() });
        }
    });
    // songs you already have: download them again in the new quality, like Spotify does
    let fmt_now = app.settings.lock().download_format.clone();
    // (not the song playing now: its file is in use)
    let cur = app.player.current_id();
    let up: Vec<String> = app.dl.upgradable(&fmt_now).into_iter().filter(|id| Some(id) != cur.as_ref()).collect();
    let busy = app.dl.jobs.lock().iter().any(|j| j.id == "upgrade-quality" && j.tracks.iter().any(|t| t.status.active()));
    if busy {
        dim(ui, &pal, "Upgrading the songs you already have: see DOWNLOADS for progress.");
    } else if up.is_empty() {
        dim(ui, &pal, "Every song you downloaded is already in this quality (or better).");
    } else {
        let per = if fmt_now == "lossless" { 30.0 } else { 8.0 };
        let size = gb_text(up.len() as f64 * per / 1024.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(format!("{} song{} you downloaded {} in a lower quality.", up.len(), if up.len() == 1 { "" } else { "s" }, if up.len() == 1 { "is" } else { "are" })).color(pal.text));
            if button(ui, &pal, &format!("UPGRADE TO {}", crate::downloader::quality_label(&fmt_now)), true, true).on_hover_text(format!("Downloads them again in this quality, in the background (about {size}). Each keeps its playlists, likes and plays; old files go to the Recycle Bin.")).clicked() {
                app.dl.upgrade(&up);
                app.toast(format!("Upgrading {} songs: see DOWNLOADS for progress", up.len()));
            }
        });
        dim(ui, &pal, &format!("About {size}, in the background. Songs that came from your own files (not downloaded by DK.FM) stay as they are."));
    }
    // FLACs from LOSSLESS, now that a smaller quality is set: made smaller here, same sound
    let small: Vec<String> = app.dl.shrinkable(&fmt_now).into_iter().filter(|id| Some(id) != cur.as_ref()).collect();
    let shrinking = app.dl.jobs.lock().iter().any(|j| j.id == "shrink-quality" && j.tracks.iter().any(|t| t.status.active()));
    if shrinking {
        dim(ui, &pal, "Making your FLAC songs smaller: see DOWNLOADS for progress.");
    } else if !small.is_empty() {
        let kbps = match fmt_now.as_str() { "standard" => 128.0, "mp3-320" => 320.0, "mp3-v0" => 245.0, _ => 256.0 };
        let freed: f64 = { let d = app.lib.data.read(); small.iter().filter_map(|id| d.tracks.get(id)).map(|t| t.duration * (t.bitrate.unwrap_or(900) as f64 - kbps).max(0.0) * 1000.0 / 8.0).sum::<f64>() / 1e9 };
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(format!("{} song{} from LOSSLESS {} FLAC.", small.len(), if small.len() == 1 { "" } else { "s" }, if small.len() == 1 { "is" } else { "are" })).color(pal.text));
            if button(ui, &pal, &format!("MAKE THEM {} (FREES ABOUT {})", crate::downloader::quality_label(&fmt_now), gb_text(freed).to_uppercase()), true, true).on_hover_text("Converted on this PC in a few minutes, nothing downloaded. They sound the same: YouTube's audio isn't lossless, so the FLAC holds exactly what this quality keeps. Playlists, likes and plays stay; the FLACs go to the Recycle Bin.").clicked() {
                app.dl.shrink(&small);
                app.toast(format!("Making {} songs smaller: see DOWNLOADS for progress", small.len()));
            }
        });
        dim(ui, &pal, "Converted on this PC, nothing downloaded, and they sound the same (YouTube's audio isn't lossless).");
    }
    let mut conc = app.settings.lock().download_concurrency;
    row(ui, &pal, "Parallel downloads", |ui| {
        if ui.add(egui::Slider::new(&mut conc, 1..=6)).changed() {
            app.edit_settings(|s| s.download_concurrency = conc);
        }
    });
    ui.add_space(14.0);
    naming(app, ui);
    ui.add_space(14.0);
    caption(ui, &pal, "AUTO-SYNC");
    let mut h = app.settings.lock().sync_hours;
    let hb = h;
    row(ui, &pal, "Check imported playlists", |ui| {
        egui::ComboBox::from_id_salt("sync").selected_text(match h { 0 => "Never", 1 => "Every hour", 24 => "Once a day", _ => "Every 6 hours" }).show_ui(ui, |ui| {
            ui.selectable_value(&mut h, 1, "Every hour");
            ui.selectable_value(&mut h, 6, "Every 6 hours");
            ui.selectable_value(&mut h, 24, "Once a day");
            ui.selectable_value(&mut h, 0, "Never (sync manually)");
        });
    });
    if h != hb {
        app.edit_settings(|s| s.sync_hours = h);
    }
    dim(ui, &pal, "New songs added to an imported playlist download automatically, in the same order. Turn it off per playlist by right-clicking it.");
    ui.add_space(14.0);
    caption(ui, &pal, "ENGINE");
    let st = crate::ytdlp::STATUS.lock().clone();
    dim(ui, &pal, &if st.version.is_empty() { "Download tools are fetched the first time you import (yt-dlp, ffmpeg, QuickJS) and yt-dlp is updated daily.".into() } else { format!("yt-dlp {} · updated daily", st.version) });
}

/// Set from inside the settings body to close the window after this frame.
static CLOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Result of a background "Connect Spotify" login, picked up by the settings tab.
static LOGIN: parking_lot::Mutex<Option<Result<crate::spotify_auth::Login, String>>> = parking_lot::Mutex::new(None);
static LOGGING_IN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn spotify(app: &mut App, ui: &mut Ui) {
    use std::sync::atomic::Ordering;
    let pal = app.pal;
    if let Some(r) = LOGIN.lock().take() {
        match r {
            Ok(l) => {
                crate::sources::seed_user_token(&l.refresh, &l.access, l.expires);
                app.edit_settings(|s| {
                    s.spotify_refresh_token = l.refresh.clone();
                    s.spotify_user = l.name.clone();
                });
                app.toast(format!("Connected to Spotify as {}", l.name));
            }
            Err(e) => app.toast_err(e),
        }
    }
    let (mut id, refresh, user) = { let s = app.settings.lock(); (s.spotify_client_id.clone(), s.spotify_refresh_token.clone(), s.spotify_user.clone()) };
    caption(ui, &pal, "YOUR SPOTIFY ACCOUNT");
    dim(ui, &pal, "Connect once to bring over everything at once: Liked Songs, every playlist (private ones too) and saved albums. Imported lists keep syncing.");
    ui.add_space(6.0);
    let busy = LOGGING_IN.load(Ordering::Relaxed);
    ui.horizontal(|ui| {
        if !refresh.is_empty() {
            ui.label(egui::RichText::new(format!("CONNECTED AS {}", user.to_uppercase())).font(px(8.0)).color(pal.accent2));
            if button(ui, &pal, "IMPORT MY WHOLE LIBRARY", true, true).clicked() {
                CLOSE.store(true, Ordering::Relaxed);
                super::import::fetch_library(app);
            }
            if button(ui, &pal, "DISCONNECT", false, true).clicked() {
                app.edit_settings(|s| {
                    s.spotify_refresh_token.clear();
                    s.spotify_user.clear();
                });
            }
        } else if button(ui, &pal, if busy { "WAITING FOR BROWSER…" } else { "CONNECT SPOTIFY" }, true, !busy && !id.trim().is_empty()).clicked() {
            LOGGING_IN.store(true, Ordering::Relaxed);
            let (cid, ctx) = (id.trim().to_string(), ui.ctx().clone());
            std::thread::spawn(move || {
                let r = crate::spotify_auth::login(&cid);
                *LOGIN.lock() = Some(r);
                LOGGING_IN.store(false, Ordering::Relaxed);
                ctx.request_repaint();
            });
        }
    });
    if busy {
        dim(ui, &pal, "Finish logging in on the Spotify page that opened in your browser.");
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    } else if refresh.is_empty() && id.trim().is_empty() {
        dim(ui, &pal, "Paste your Client ID below first (see HOW TO SET UP).");
    }
    ui.add_space(14.0);
    caption(ui, &pal, "SPOTIFY APP");
    row(ui, &pal, "Client ID", |ui| {
        if ui.add(egui::TextEdit::singleline(&mut id).desired_width(300.0)).changed() {
            app.edit_settings(|s| s.spotify_client_id = id.trim().to_string());
        }
    });
    // (no Client Secret: Connect Spotify doesn't need one)
    dim(ui, &pal, "Only the Client ID is needed. Without connecting, DK.FM still reads public links (the first 100 songs of a playlist).");
    ui.add_space(14.0);
    caption(ui, &pal, "HOW TO SET UP (2 MINUTES, FREE)");
    if ui.link("1. Open developer.spotify.com/dashboard and log in.").clicked() {
        let _ = open::that("https://developer.spotify.com/dashboard");
    }
    ui.label(format!("2. Create app: any name; Redirect URI {}; tick \"Web API\".", crate::spotify_auth::REDIRECT));
    ui.label("3. Copy the Client ID into the box above, then click CONNECT SPOTIFY.");
    dim(ui, &pal, "The API costs nothing, but Spotify requires the app's owner to have Premium, and allows up to 5 accounts per app (add others under User Management).");
}

fn playback(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    caption(ui, &pal, "PLAYBACK");
    let mut o = app.player.st.lock().opts.clone();
    row(ui, &pal, "Crossfade", |ui| {
        if ui.add(egui::Slider::new(&mut o.crossfade, 0..=12).suffix(" s")).changed() {
            app.player.set_crossfade(o.crossfade);
        }
    });
    if switch(ui, &pal, &mut o.smart_shuffle, "SMART SHUFFLE: SPREAD OUT ARTISTS, OFTEN-SKIPPED SONGS LATER") {
        app.player.set_smart_shuffle(o.smart_shuffle);
    }
    ui.add_space(4.0);
    if switch(ui, &pal, &mut o.match_volume, "VOLUME MATCHING: EVERY SONG AT THE SAME LOUDNESS") {
        app.player.set_match_volume(o.match_volume);
    }
    ui.add_space(4.0);
    if switch(ui, &pal, &mut o.normalize, "LEVELER: COMPRESS LOUD/QUIET PARTS") {
        app.player.set_normalize(o.normalize);
    }
    spacer(ui);
    caption(ui, &pal, "PRIVATE LISTENING");
    dim(ui, &pal, "While it's on, plays, skips and listening history aren't recorded (Stats, Most Played and Smart Shuffle ignore what you play). Also in the deck (PRV), the tray menu and Ctrl+K.");
    let mut on = app.lib.private.load(std::sync::atomic::Ordering::Relaxed);
    if switch(ui, &pal, &mut on, "PRIVATE LISTENING") {
        app.set_private(on);
    }
    ui.add_space(4.0);
    let mut keep = app.settings.lock().keep_private;
    if switch(ui, &pal, &mut keep, "KEEP IT ON AFTER RESTARTING DK.FM") {
        app.edit_settings(|s| s.keep_private = keep);
    }
    ui.add_space(8.0);
    row(ui, &pal, "Visualizer FPS", |ui| {
        for f in [15u32, 30, 60] {
            if super::widgets::outline_button(ui, &pal, &format!("{f}"), o.vis_fps == f).clicked() {
                let m = o.visualizer.clone();
                app.player.set_visualizer(&m, f);
            }
        }
    });
}

fn system_tab(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    caption(ui, &pal, "WINDOW & TRAY");
    if cfg!(target_os = "linux") {
        dim(ui, &pal, "The tray icon isn't available on Linux; closing the window quits DK.FM.");
    } else {
        let mode = app.settings.lock().close_mode().to_string();
        row(ui, &pal, "The X button", |ui| {
            if let Some(m) = choice(ui, &pal, &mode, &[("playing".to_string(), "TRAY WHILE PLAYING"), ("tray".to_string(), "ALWAYS TRAY"), ("quit".to_string(), "QUITS")]) {
                app.edit_settings(|s| s.close_mode = m);
            }
        });
        dim(ui, &pal, match mode.as_str() {
            "tray" => "Closing the window keeps DK.FM running in the tray, even when paused.",
            "quit" => "Closing the window quits DK.FM (music stops).",
            _ => "Closing the window while music plays keeps it playing in the tray; with nothing playing, DK.FM quits completely.",
        });
        dim(ui, &pal, "Right-click the tray icon for play/pause, next and Quit.");
    }
    ui.add_space(10.0);
    let mut sl = app.settings.lock().start_at_login;
    if switch(ui, &pal, &mut sl, "START WITH YOUR COMPUTER (HIDDEN IN THE TRAY, PAUSED)") {
        match system::set_start_at_login(sl) {
            Ok(()) => app.edit_settings(|s| s.start_at_login = sl),
            Err(e) => app.toast_err(format!("Couldn't change startup setting: {e}")),
        }
    }
}

fn updates(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    caption(ui, &pal, "AUTO UPDATE");
    let mut a = app.settings.lock().auto_update;
    if switch(ui, &pal, &mut a, "CHECK GITHUB FOR UPDATES") {
        app.edit_settings(|s| s.auto_update = a);
    }
    dim(ui, &pal, &format!("Current version: v{}", env!("CARGO_PKG_VERSION")));
    let st = app.update.lock().clone();
    dim(ui, &pal, &match st {
        UpdState::Idle => "Not checked yet.".to_string(),
        UpdState::Checking => "Checking…".into(),
        UpdState::Latest => "You're on the latest version.".into(),
        UpdState::Available(u) => format!("v{} is available.", u.version),
        UpdState::Downloading(_) => "Downloading…".into(),
        UpdState::Error(e) => format!("Update check failed: {e}"),
    });
    ui.horizontal(|ui| {
        if button(ui, &pal, "CHECK NOW", false, true).clicked() {
            let (slot, ctx) = (app.update.clone(), ui.ctx().clone());
            *slot.lock() = UpdState::Checking;
            std::thread::spawn(move || {
                *slot.lock() = match system::check_update() { Ok(Some(u)) => UpdState::Available(u), Ok(None) => UpdState::Latest, Err(e) => UpdState::Error(e) };
                ctx.request_repaint();
            });
        }
        if button(ui, &pal, "RELEASE NOTES", false, true).clicked() {
            let _ = open::that("https://github.com/thatdanyal/dk-fm/releases");
        }
    });
}


fn naming(app: &mut App, ui: &mut Ui) {
    use crate::downloader::{render_name, NameParts, NAME_TOKENS};
    let pal = app.pal;
    caption(ui, &pal, "FILE NAMES");
    dim(ui, &pal, "How downloaded songs are named inside the download folder. \"/\" makes a folder. {folder} is the playlist or album (songs you get one at a time: Downloads).");
    let mut pat = app.settings.lock().name_pattern.clone();
    let before = pat.clone();
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut pat).desired_width(380.0).font(vt(19.0)));
        if button(ui, &pal, "RESET", false, pat != crate::store::DEFAULT_PATTERN).clicked() {
            pat = crate::store::DEFAULT_PATTERN.into();
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Insert").color(pal.dim));
        for t in NAME_TOKENS {
            if tb_button(ui, &pal, t, false).clicked() {
                if !pat.is_empty() && !pat.ends_with(['/', ' ']) {
                    pat.push(' ');
                }
                pat.push_str(t);
            }
        }
        if tb_button(ui, &pal, "/", false).on_hover_text("New folder level").clicked() {
            pat.push('/');
        }
    });
    if pat != before {
        app.edit_settings(|s| s.name_pattern = pat.clone());
    }
    let ext = crate::downloader::fmt_args(&app.settings.lock().download_format).0;
    let ex = |folder: &str, album: &str, track: Option<u32>| render_name(&pat, &NameParts { artist: "Daft Punk", album, title: "One More Time", track, year: Some(2001), folder }).display().to_string();
    ui.label(egui::RichText::new(format!("Example: {}.{ext}", ex("Party Mix", "Discovery", Some(1)))).color(pal.accent2));
    dim(ui, &pal, &format!("A single song: {}.{ext}", ex("Downloads", "", None)));
}

// ------------------------------------------------------------------------------- shortcuts

fn keys(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    // recording a new key: the first key pressed (with Ctrl / Alt / Shift) becomes the shortcut
    if let Some(action) = app.setui.capture {
        let pressed = ui.input(|i| i.events.iter().find_map(|e| if let egui::Event::Key { key, pressed: true, modifiers, .. } = e { Some(Combo::from_event(*key, *modifiers)) } else { None }));
        if let Some(c) = pressed {
            let plain = !c.ctrl && !c.alt && !c.shift;
            let new = match c.key {
                egui::Key::Escape if plain => None,
                egui::Key::Backspace | egui::Key::Delete if plain => Some(String::new()),
                _ => Some(c.text()),
            };
            if let Some(b) = new {
                app.edit_settings(|s| {
                    if b == keys::default_of(action) {
                        s.keys.remove(action);
                    } else {
                        s.keys.insert(action.to_string(), b);
                    }
                });
            }
            app.setui.capture = None;
        }
        ui.ctx().request_repaint();
    }
    let map = app.settings.lock().keys.clone();
    let clashes = keys::conflicts(&map);
    caption(ui, &pal, "KEYBOARD SHORTCUTS");
    dim(ui, &pal, "Click a shortcut, then press the new keys (Esc cancels, Backspace turns it off).");
    if !clashes.is_empty() {
        ui.label(egui::RichText::new(format!("⚠ {} shortcut{} used twice — change one of each pair.", clashes.len(), if clashes.len() == 1 { " is" } else { "s are" })).color(pal.accent));
    }
    ui.add_space(4.0);
    let name = |a: &str| keys::ACTIONS.iter().find(|x| x.0 == a).map(|x| x.1).unwrap_or("");
    for (action, label, def) in keys::ACTIONS {
        let cur = map.get(action).cloned().unwrap_or_else(|| def.to_string());
        let clash: Vec<&str> = clashes.iter().filter_map(|(a, b)| if *a == action { Some(*b) } else if *b == action { Some(*a) } else { None }).collect();
        ui.horizontal(|ui| {
            cell(ui, 250.0, egui::RichText::new(label).color(pal.text));
            let rec = app.setui.capture == Some(action);
            let text = if rec { "PRESS KEYS…".to_string() } else if cur.is_empty() { "—".into() } else { cur.to_uppercase() };
            let (r, resp) = ui.allocate_exact_size(Vec2::new(150.0, 24.0), Sense::click());
            fill(ui.painter(), r, if rec { pal.accent } else { pal.bg });
            frame_rect(ui.painter(), r, 2.0, if !clash.is_empty() { pal.accent } else if resp.hovered() { pal.text } else { pal.line_hi });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, px(7.0), if rec { pal.ink } else if cur.is_empty() { pal.dim } else { pal.accent2 });
            if resp.clicked() {
                app.setui.capture = if rec { None } else { Some(action) };
                resp.surrender_focus(); // so Space / Enter are recorded, not "clicked"
            }
            if map.contains_key(action) && tb_button(ui, &pal, "RESET", false).on_hover_text(format!("Back to {}", if def.is_empty() { "none" } else { def })).clicked() {
                app.edit_settings(|s| {
                    s.keys.remove(action);
                });
            }
            if !clash.is_empty() {
                ui.label(egui::RichText::new(format!("also: {}", clash.iter().map(|a| name(a)).collect::<Vec<_>>().join(", "))).font(vt(16.0)).color(pal.accent));
            }
        });
    }
    ui.add_space(8.0);
    if button(ui, &pal, "RESET ALL SHORTCUTS", false, !map.is_empty()).clicked() {
        app.edit_settings(|s| s.keys.clear());
        app.setui.capture = None;
    }
    dim(ui, &pal, "Keyboard media keys and the system media overlay work too. In Ctrl+K: Enter plays, Tab adds to a playlist.");
}

// ------------------------------------------------------------------------------- backups

fn fmt_size(b: u64) -> String {
    if b >= 1 << 20 { format!("{:.1} MB", b as f64 / (1u64 << 20) as f64) } else { format!("{} KB", b.div_ceil(1024)) }
}

fn fmt_date(secs: i64) -> String {
    // "2026-10-03_142501" -> "2026-10-03 · 14:25"
    let s = crate::store::local_stamp(secs, true);
    format!("{} · {}:{}", &s[..10], &s[11..13], &s[13..15])
}

fn backups(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let s = app.settings.lock().clone();
    let stale = app.setui.backups.as_ref().map(|b| b.0.elapsed().as_secs() > 5).unwrap_or(true);
    if stale {
        app.setui.backups = Some((std::time::Instant::now(), backup::list()));
    }
    let list: Vec<(PathBuf, i64, u64, String)> = app.setui.backups.as_ref().map(|b| b.1.iter().map(|e| (e.path.clone(), e.created, e.size, e.kind.clone())).collect()).unwrap_or_default();
    let refresh = |app: &mut App| app.setui.backups = None;
    caption(ui, &pal, "AUTOMATIC BACKUPS");
    dim(ui, &pal, "A small compressed copy of your library, playlists, play counts, listening history and settings. Song files and cover art aren't included (covers come back from the songs); your Spotify login isn't either.");
    let mut on = s.auto_backup;
    if switch(ui, &pal, &mut on, "BACK UP AUTOMATICALLY") {
        app.edit_settings(|s| s.auto_backup = on);
    }
    ui.add_space(4.0);
    row(ui, &pal, "How often", |ui| {
        if let Some(v) = choice(ui, &pal, &s.backup_every, &[("daily".to_string(), "DAILY"), ("weekly".to_string(), "WEEKLY")]) {
            app.edit_settings(|s| s.backup_every = v);
        }
    });
    row(ui, &pal, "Keep the newest", |ui| {
        if let Some(v) = choice(ui, &pal, &s.backup_keep, &[(1, "1"), (3, "3"), (5, "5"), (10, "10")]) {
            app.edit_settings(|s| s.backup_keep = v);
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if button(ui, &pal, "BACK UP NOW", true, true).clicked() {
            match backup::create(&app.lib, &s, "manual") {
                Ok(_) => app.toast("Backup saved"),
                Err(e) => app.toast_err(e),
            }
            refresh(app);
        }
        if button(ui, &pal, "EXPORT BACKUP…", false, true).clicked() {
            let name = format!("DK.FM backup {}.{}", crate::store::local_stamp(crate::store::now_secs(), false), backup::EXT);
            if let Some(p) = rfd::FileDialog::new().set_title("Export a DK.FM backup").set_file_name(&name).add_filter("DK.FM backup", &[backup::EXT]).save_file() {
                let r = { let (d, h) = (app.lib.data.read(), app.lib.history.read()); backup::write(&p, &d, &h, &s) };
                match r {
                    Ok(n) => app.toast(format!("Exported backup ({})", fmt_size(n))),
                    Err(e) => app.toast_err(e),
                }
            }
        }
        if button(ui, &pal, "IMPORT BACKUP…", false, true).clicked() {
            if let Some(p) = rfd::FileDialog::new().set_title("Restore a DK.FM backup").add_filter("DK.FM backup", &[backup::EXT]).pick_file() {
                match backup::read(&p) {
                    Ok(_) => app.setui.confirm = Some(Confirm::Restore(p)),
                    Err(e) => app.toast_err(e),
                }
            }
        }
    });
    // asking first
    match app.setui.confirm.take() {
        Some(Confirm::Restore(p)) => {
            let mut keep = true;
            egui::Frame::new().fill(pal.bg2).stroke(egui::Stroke::new(2.0_f32, pal.accent)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                let when = p.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| fmt_date(d.as_secs() as i64)).unwrap_or_default();
                ui.label(egui::RichText::new(format!("Restore your playlists and library from the backup of {when}?")).color(pal.text));
                dim(ui, &pal, "Your library, playlists, play counts, history and settings go back to that point. What you have now is backed up first, so you can undo this.");
                ui.horizontal(|ui| {
                    if button(ui, &pal, "CANCEL", false, true).clicked() {
                        keep = false;
                    }
                    if button(ui, &pal, "RESTORE", true, true).clicked() {
                        keep = false;
                        match backup::restore(&app.lib, &app.settings, &p) {
                            Ok(()) => {
                                app.reload_all(ui.ctx());
                                app.toast("Backup restored — rescanning your music folders");
                            }
                            Err(e) => app.toast_err(format!("Couldn't restore: {e}")),
                        }
                        app.setui.backups = None;
                    }
                });
            });
            if keep {
                app.setui.confirm = Some(Confirm::Restore(p));
            }
        }
        Some(Confirm::DeleteAll) => {
            let mut keep = true;
            egui::Frame::new().fill(pal.bg2).stroke(egui::Stroke::new(2.0_f32, pal.accent)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.label(egui::RichText::new(format!("Move all {} backups to the Recycle Bin?", list.len())).color(pal.text));
                ui.horizontal(|ui| {
                    if button(ui, &pal, "CANCEL", false, true).clicked() {
                        keep = false;
                    }
                    if button(ui, &pal, "DELETE ALL", true, true).clicked() {
                        keep = false;
                        match system::trash(&backup::dir()) {
                            Ok(()) => app.toast("Backups moved to the Recycle Bin"),
                            Err(e) => app.toast_err(e),
                        }
                        app.setui.backups = None;
                    }
                });
            });
            if keep {
                app.setui.confirm = Some(Confirm::DeleteAll);
            }
        }
        None => {}
    }
    spacer(ui);
    caption(ui, &pal, "YOUR BACKUPS");
    if list.is_empty() {
        dim(ui, &pal, "No backups yet.");
    }
    for (path, created, size, kind) in &list {
        ui.horizontal(|ui| {
            cell(ui, 200.0, egui::RichText::new(fmt_date(*created)).color(pal.text));
            let k = match kind.as_str() { "auto" => "automatic", "before-restore" => "before a restore", _ => "manual" };
            cell(ui, 160.0, egui::RichText::new(k).color(pal.dim));
            cell(ui, 70.0, egui::RichText::new(fmt_size(*size)).color(pal.dim));
            if button(ui, &pal, "RESTORE", false, true).clicked() {
                app.setui.confirm = Some(Confirm::Restore(path.clone()));
            }
        });
    }
    ui.add_space(6.0);
    let total: u64 = list.iter().map(|b| b.2).sum();
    dim(ui, &pal, &format!("{} on disk · {}", fmt_size(total), backup::dir().display()));
    ui.horizontal(|ui| {
        if button(ui, &pal, "OPEN FOLDER", false, !list.is_empty()).clicked() {
            let _ = open::that(backup::dir());
        }
        if button(ui, &pal, "DELETE ALL BACKUPS", false, !list.is_empty()).clicked() {
            app.setui.confirm = Some(Confirm::DeleteAll);
        }
    });
    spacer(ui);
    caption(ui, &pal, "MOVE TO A NEW PC · GIVE A FRIEND A HEAD START");
    dim(ui, &pal, "One file with everything you've made and got: your playlists (in order), liked songs, every other song, and your settings, themes and layouts. Drop it on DK.FM on the other PC: songs already there are used, the rest download by themselves. It doesn't sync afterwards, and never holds your Spotify login, folders, play counts or history.");
    if button(ui, &pal, "SAVE MY WHOLE DK.FM…", true, true).clicked() {
        super::sharing::export_all(app);
    }
    spacer(ui);
    caption(ui, &pal, "SHARE YOUR SETUP WITH A FRIEND");
    dim(ui, &pal, "A short code with your look & behaviour: theme, font, text size, lists, sidebar, shortcuts, visualizer, EQ and search. Never your Spotify login, folders, library, play counts or backups.");
    if button(ui, &pal, "COPY SETTINGS CODE", true, true).clicked() {
        let sh = super::sharing::settings_share(app);
        super::sharing::copy(app, ui.ctx(), &sh, "your settings");
    }
    dim(ui, &pal, "Got a code (theme, layout, settings or playlist)? Press Ctrl+V anywhere in DK.FM, or paste it into Ctrl+K. You see what it does before anything changes.");
}

fn about(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    caption(ui, &pal, "DK.FM");
    ui.label(egui::RichText::new(format!("v{} · native · {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS)).color(pal.text));
    ui.label(egui::RichText::new("Made by Danyal Khan").font(px(8.0)).color(pal.accent));
    ui.add_space(4.0);
    dim(ui, &pal, "Retro desktop music player. Plays your local library and imports Spotify / YouTube / SoundCloud playlists.");
    dim(ui, &pal, "Lyrics from LRCLIB · downloads powered by yt-dlp, FFmpeg and QuickJS · fonts VT323 & Press Start 2P (OFL).");
    dim(ui, &pal, "Only download music you have the rights to.");
}

/// Left-aligned text in a fixed-width cell (cut off with … when too long).
/// "7.4 GB", or "120 MB" under a gigabyte.
fn gb_text(gb: f64) -> String {
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", (gb * 1024.0).max(1.0)) }
}

fn cell(ui: &mut Ui, w: f32, text: egui::RichText) {
    ui.allocate_ui_with_layout(Vec2::new(w, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(w);
        ui.add(egui::Label::new(text).truncate());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_search_finds_by_everyday_words() {
        let tabs = |q: &str| find(q).into_iter().map(|(t, _)| TABS.iter().find(|x| x.0 == t).unwrap().1).collect::<Vec<_>>();
        assert_eq!(tabs("lossless"), ["Downloads"]);
        assert!(tabs("dark").contains(&"Look"));
        assert!(tabs("spotify").contains(&"Spotify"));
        assert_eq!(tabs("sound QUALITY"), ["Downloads"]);
        assert!(tabs("zzzz").is_empty());
    }
}

//! Settings window, text prompts, and the "Update available" popup.
use super::theme::{self, px, vt};
use super::widgets::{button, caption, dim, fill, frame_rect, switch};
use super::{App, Modal, PromptAction};
use crate::system::{self, UpdState};
use eframe::egui::{self, Align2, Rect, Sense, Ui, Vec2};

#[derive(Clone, Copy, PartialEq)]
pub enum SetTab {
    Appearance,
    Library,
    Downloads,
    Spotify,
    Playback,
    System,
    Updates,
    Keys,
    About,
}

const TABS: [(SetTab, &str); 9] = [
    (SetTab::Appearance, "Appearance"),
    (SetTab::Library, "Library"),
    (SetTab::Downloads, "Downloads"),
    (SetTab::Spotify, "Spotify"),
    (SetTab::Playback, "Playback"),
    (SetTab::System, "System"),
    (SetTab::Updates, "Updates"),
    (SetTab::Keys, "Shortcuts"),
    (SetTab::About, "About"),
];

fn window(pal: theme::Pal, ctx: &egui::Context, title: &str, size: Vec2, body: impl FnOnce(&mut Ui)) -> bool {
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
    let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    open && !esc
}

pub fn show_modal(app: &mut App, ctx: &egui::Context) {
    let Some(m) = app.modal.take() else { return };
    let keep = match m {
        Modal::Settings(tab) => {
            let mut t = tab;
            let open = window(app.pal, ctx, "SETTINGS", Vec2::new(780.0, 520.0), |ui| settings_body(app, ui, &mut t));
            if open && !CLOSE.swap(false, std::sync::atomic::Ordering::Relaxed) {
                app.modal.get_or_insert(Modal::Settings(t));
                if let Some(Modal::Settings(x)) = app.modal.as_mut() {
                    *x = t;
                }
            }
            false
        }
        Modal::Prompt { title, mut text, action } => {
            let mut done: Option<bool> = None;
            let open = window(app.pal, ctx, &title, Vec2::new(420.0, 90.0), |ui| {
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
                Some(true) if !text.trim().is_empty() => {
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
        Modal::Update => {
            let st = app.update.lock().clone();
            let mut close = false;
            let open = window(app.pal, ctx, "UPDATE AVAILABLE", Vec2::new(440.0, 160.0), |ui| match &st {
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
            app.lib.new_playlist(&text, ids);
            app.toast(if n > 0 { format!("Saved \"{text}\" ({n} songs)") } else { format!("Created \"{text}\"") });
        }
        PromptAction::RenamePlaylist(id) => app.lib.edit_playlist(&id, |p| p.name = text),
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

fn settings_body(app: &mut App, ui: &mut Ui, tab: &mut SetTab) {
    let pal = app.pal;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(150.0);
            for (t, name) in TABS {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(150.0, 28.0), Sense::click());
                if *tab == t {
                    fill(ui.painter(), r, pal.sel);
                    fill(ui.painter(), Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), pal.accent);
                }
                ui.painter().text(r.min + Vec2::new(12.0, 14.0), Align2::LEFT_CENTER, name, vt(20.0), if *tab == t { pal.accent } else if resp.hovered() { pal.text } else { pal.dim });
                if resp.clicked() {
                    *tab = t;
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("set-body").max_width(590.0).show(ui, |ui| {
            ui.vertical(|ui| {
            ui.set_width(580.0);
            match tab {
                SetTab::Appearance => appearance(app, ui),
                SetTab::Library => library(app, ui),
                SetTab::Downloads => downloads(app, ui),
                SetTab::Spotify => spotify(app, ui),
                SetTab::Playback => playback(app, ui),
                SetTab::System => system_tab(app, ui),
                SetTab::Updates => updates(app, ui),
                SetTab::Keys => keys(app, ui),
                SetTab::About => about(app, ui),
            }
            });
        });
    });
}

fn appearance(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    caption(ui, &pal, "THEME");
    let cur = app.settings.lock().theme.clone();
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for (k, name) in theme::THEMES {
            let tp = theme::palette(k, None);
            let (r, resp) = ui.allocate_exact_size(Vec2::new(128.0, 80.0), Sense::click());
            fill(ui.painter(), r.translate(Vec2::splat(3.0)), pal.shadow);
            fill(ui.painter(), r, tp.bg);
            for (i, c) in [tp.accent, tp.accent2, tp.text].iter().enumerate() {
                let h = 14.0 + i as f32 * 10.0;
                fill(ui.painter(), Rect::from_min_size(egui::pos2(r.left() + 8.0 + i as f32 * 38.0, r.top() + 50.0 - h), Vec2::new(34.0, h)), *c);
            }
            fill(ui.painter(), Rect::from_min_max(egui::pos2(r.left(), r.bottom() - 26.0), r.max), pal.panel);
            ui.painter().text(egui::pos2(r.left() + 8.0, r.bottom() - 13.0), Align2::LEFT_CENTER, *name, vt(17.0), pal.text);
            frame_rect(ui.painter(), r, 2.0, if cur == *k { pal.accent } else if resp.hovered() { pal.text } else { pal.line_hi });
            if resp.clicked() {
                chosen = Some(k.to_string());
            }
        }
    });
    if let Some(k) = chosen {
        app.edit_settings(|s| s.theme = k);
        app.set_theme(ui.ctx());
    }
    ui.add_space(14.0);
    caption(ui, &pal, "ACCENT COLOR");
    let mut col = app.settings.lock().accent.as_deref().and_then(theme::parse_hex).unwrap_or(pal.accent);
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
    ui.add_space(14.0);
    caption(ui, &pal, "CRT EFFECTS");
    let (mut sc, mut gl) = { let s = app.settings.lock(); (s.scanlines, s.glow) };
    if switch(ui, &pal, &mut sc, "SCANLINES") {
        app.edit_settings(|s| s.scanlines = sc);
    }
    if switch(ui, &pal, &mut gl, "PHOSPHOR GLOW") {
        app.edit_settings(|s| s.glow = gl);
    }
    ui.add_space(14.0);
    caption(ui, &pal, "LAYOUT");
    if button(ui, &pal, "RESET PANEL LAYOUT", false, true).clicked() {
        app.reset_layout();
        app.toast("Layout reset");
    }
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
    caption(ui, &pal, "FORMAT");
    let mut fmt = app.settings.lock().download_format.clone();
    let before = fmt.clone();
    row(ui, &pal, "Audio format", |ui| {
        egui::ComboBox::from_id_salt("fmt").selected_text(match fmt.as_str() { "mp3-320" => "MP3 · 320 kbps", "mp3-v0" => "MP3 · V0", _ => "M4A · AAC original" }).width(330.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut fmt, "m4a".to_string(), "M4A · AAC original (recommended: smallest, no quality loss)");
            ui.selectable_value(&mut fmt, "mp3-v0".to_string(), "MP3 · V0 VBR (~245 kbps)");
            ui.selectable_value(&mut fmt, "mp3-320".to_string(), "MP3 · 320 kbps (largest, for old devices)");
        });
    });
    if fmt != before {
        app.edit_settings(|s| s.download_format = fmt);
    }
    let mut conc = app.settings.lock().download_concurrency;
    row(ui, &pal, "Parallel downloads", |ui| {
        if ui.add(egui::Slider::new(&mut conc, 1..=6)).changed() {
            app.edit_settings(|s| s.download_concurrency = conc);
        }
    });
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
    let (mut id, mut sec, refresh, user) = { let s = app.settings.lock(); (s.spotify_client_id.clone(), s.spotify_client_secret.clone(), s.spotify_refresh_token.clone(), s.spotify_user.clone()) };
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
    row(ui, &pal, "Client Secret (optional)", |ui| {
        if ui.add(egui::TextEdit::singleline(&mut sec).password(true).desired_width(300.0)).changed() {
            app.edit_settings(|s| s.spotify_client_secret = sec.trim().to_string());
        }
    });
    dim(ui, &pal, "The secret is only used for album and song links when you're not connected. Without any of this, DK.FM still reads public links (first 100 songs).");
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
    ui.add_space(8.0);
    row(ui, &pal, "Visualizer FPS", |ui| {
        for f in [15u32, 30, 60] {
            if ui.selectable_label(o.vis_fps == f, format!("{f}")).clicked() {
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
        let mut t = app.settings.lock().close_to_tray;
        if switch(ui, &pal, &mut t, "CLOSE BUTTON KEEPS MUSIC PLAYING IN THE TRAY") {
            app.edit_settings(|s| s.close_to_tray = t);
        }
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

fn keys(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    caption(ui, &pal, "KEYBOARD");
    for (k, d) in [
        ("Space", "Play / pause"), ("Left / Right", "Seek 5 s (Shift: 30 s)"), ("Ctrl Left / Right", "Previous / next song"), ("Up / Down", "Volume"),
        ("M", "Mute"), ("S", "Shuffle"), ("R", "Repeat mode"), ("L", "Like current song"), ("V", "Cycle visualizer"),
        ("Ctrl K", "Command palette: search & do anything"), ("Ctrl F", "Search library"), ("Ctrl E", "Edit layout"), ("Ctrl M", "Mini player"), ("Ctrl ,", "Settings"), ("Ctrl I", "Import music"),
    ] {
        row(ui, &pal, k, |ui| {
            ui.label(egui::RichText::new(d).color(pal.text));
        });
    }
    dim(ui, &pal, "Keyboard media keys and the system media overlay work too.");
}

fn about(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    caption(ui, &pal, "DK.FM");
    ui.label(egui::RichText::new(format!("v{} · native · {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS)).color(pal.text));
    dim(ui, &pal, "Retro desktop music player. Plays your local library and imports Spotify / YouTube / SoundCloud playlists.");
    dim(ui, &pal, "Lyrics from LRCLIB · downloads powered by yt-dlp, FFmpeg and QuickJS · fonts VT323 & Press Start 2P (OFL).");
    dim(ui, &pal, "Only download music you have the rights to.");
}

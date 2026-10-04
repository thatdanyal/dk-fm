//! The first-run welcome (a few short steps; asks before adding any shortcut) and the one-time
//! "what's new" notice for people who updated.
use super::theme::{px, vt, Pal};
use super::widgets::{button, caption, dim, switch};
use super::{settings::window, App, Modal};
use eframe::egui::{self, Vec2};

/// Bump this (and rewrite NEWS) to show the notice again after a future update.
pub const WHATS_NEW: &str = "biggest-update-2026-10";

/// What's new, in plain words, one line each.
const NEWS: &[(&str, &str)] = &[
    ("🧭", "DISCOVER: new music picked from the artists you play and the songs you like."),
    ("🌐", "FIND SONGS: search YouTube and pick the version you want (clean, explicit, live, instrumental...), then + GET it."),
    ("🎤", "A new DK.FM logo, everywhere: the app, the taskbar and the tray."),
    ("✨", "Simpler for newcomers: fewer panels and columns open at first. Try the \"Default\" layout, or \"Everything\" for all panels."),
    ("📊", "Stats stay put: no more lists jumping around while you move the mouse."),
    ("📐", "Free layout: put every panel exactly where you want it, to the pixel (Settings > Layouts > FREE)."),
    ("🔊", "The deck can't be closed or squeezed too small, so play, pause and volume are always there."),
    ("☰", "Drag songs in the queue to reorder them; hover a cover and click ▶ to play it."),
    ("💿", "Album covers next to every song in your lists."),
    ("📝", "Lyrics grow and shrink with their panel, and come from YouTube captions when nothing else has them."),
    ("⏯", "Play, pause and skip from the taskbar thumbnail and your keyboard's media keys."),
    ("🚪", "Closing DK.FM with nothing playing really quits: nothing left running in the background."),
];

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Hello,
    Music,
    Songs,
    Shortcuts,
    Ready,
}

fn steps() -> Vec<Step> {
    // shortcuts are a Windows thing (the installer used to make them without asking)
    let mut v = vec![Step::Hello, Step::Music, Step::Songs];
    if cfg!(windows) {
        v.push(Step::Shortcuts);
    }
    v.push(Step::Ready);
    v
}

/// What to show when DK.FM opens: the welcome for a fresh install, the notice after an update.
pub fn first_modal(app: &App) -> Option<Modal> {
    let mut s = app.settings.lock();
    if s.fresh && s.whats_new_seen != WHATS_NEW {
        // a fresh install gets the welcome, never the update notice
        s.whats_new_seen = WHATS_NEW.into();
        app.settings_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if !s.onboarded {
        Some(Modal::Welcome { step: 0, start_menu: false, desktop: false })
    } else if s.whats_new_seen != WHATS_NEW {
        Some(Modal::WhatsNew)
    } else {
        None
    }
}

pub fn logo(ui: &mut egui::Ui, size: f32) {
    let id = egui::Id::new("dkfm-logo");
    let tex = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id)).or_else(|| {
        let img = image::load_from_memory(include_bytes!("../../assets/icon.png")).ok()?.resize(192, 192, image::imageops::FilterType::Lanczos3).to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let t = ui.ctx().load_texture("dkfm-logo", ci, egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
        Some(t)
    });
    if let Some(t) = tex {
        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(size)));
    }
}

fn title(ui: &mut egui::Ui, pal: &Pal, text: &str) {
    ui.label(egui::RichText::new(text).font(px(12.0)).color(pal.accent));
    ui.add_space(8.0);
}

fn line(ui: &mut egui::Ui, pal: &Pal, icon: &str, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(icon).font(vt(20.0)).color(pal.accent2));
        ui.label(egui::RichText::new(text).font(vt(20.0)).color(pal.text));
    });
}

/// The welcome steps. Returns the modal to keep showing (None = finished or skipped).
pub fn show_welcome(app: &mut App, ctx: &egui::Context, step: usize, mut start_menu: bool, mut desktop: bool) -> Option<Modal> {
    let pal = app.pal;
    let all = steps();
    let step = step.min(all.len() - 1);
    let folders: Vec<String> = app.music_folders().iter().map(|p| p.display().to_string()).collect();
    let (mut next, mut back, mut skip, mut add_folder) = (false, false, false, false);
    let open = window(pal, ctx, "WELCOME", Vec2::new(520.0, 330.0), false, |ui| {
        ui.vertical_centered(|ui| match all[step] {
            Step::Hello => {
                logo(ui, 110.0);
                ui.add_space(6.0);
                title(ui, &pal, "WELCOME TO DK.FM");
                ui.label(egui::RichText::new("A music player for the songs on your PC, plus anything you bring in from Spotify, YouTube or SoundCloud.").font(vt(21.0)).color(pal.text));
                ui.add_space(4.0);
                dim(ui, &pal, "A few quick pointers: under a minute.");
            }
            Step::Music => {
                title(ui, &pal, "YOUR MUSIC");
                ui.label(egui::RichText::new("DK.FM plays the songs in these folders and notices new ones by itself:").font(vt(20.0)).color(pal.text));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                    for f in &folders {
                        ui.label(egui::RichText::new(format!("📁 {f}")).font(vt(18.0)).color(pal.accent2));
                    }
                });
                ui.add_space(6.0);
                if button(ui, &pal, "+ ADD A FOLDER", false, true).clicked() {
                    add_folder = true;
                }
                dim(ui, &pal, "You can change these any time in Settings.");
            }
            Step::Songs => {
                title(ui, &pal, "GETTING SONGS");
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    line(ui, &pal, "📥", "+ IMPORT (Ctrl+I): paste a Spotify, YouTube or SoundCloud link and DK.FM downloads the songs.");
                    line(ui, &pal, "🔎", "Ctrl+K: find any song in your library, or do anything else.");
                    line(ui, &pal, "🏠", "HOME: mixes from your library and new releases from your artists.");
                    line(ui, &pal, "🖱", "Right-click a song for everything you can do with it.");
                });
            }
            Step::Shortcuts => {
                title(ui, &pal, "SHORTCUTS");
                ui.label(egui::RichText::new("Want a shortcut to open DK.FM?").font(vt(21.0)).color(pal.text));
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    switch(ui, &pal, &mut start_menu, "IN THE START MENU");
                    ui.add_space(4.0);
                    switch(ui, &pal, &mut desktop, "ON THE DESKTOP");
                });
                ui.add_space(10.0);
                dim(ui, &pal, "Nothing is added unless you turn it on.");
            }
            Step::Ready => {
                logo(ui, 72.0);
                ui.add_space(4.0);
                title(ui, &pal, "YOU'RE ALL SET");
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    line(ui, &pal, "⏯", "Space plays and pauses; Ctrl+← / Ctrl+→ skip.");
                    line(ui, &pal, "🖥", "F11: full-screen now playing with lyrics.");
                    line(ui, &pal, "🧩", "Ctrl+E: move panels around. Ctrl+, : Settings.");
                });
            }
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{}/{}", step + 1, all.len())).font(px(7.0)).color(pal.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let last = step + 1 == all.len();
                if button(ui, &pal, if last { "START LISTENING" } else { "NEXT" }, true, true).clicked() {
                    next = true;
                }
                if step > 0 && button(ui, &pal, "BACK", false, true).clicked() {
                    back = true;
                }
                if !last && button(ui, &pal, "SKIP", false, true).clicked() {
                    skip = true;
                }
            });
        });
    });
    if add_folder {
        super::browser::add_folder(app);
    }
    let finished = (next && step + 1 == all.len()) || skip || !open;
    if finished {
        // only what was switched on (both start off)
        #[cfg(windows)]
        if start_menu || desktop {
            std::thread::spawn(move || crate::install::add_shortcuts(start_menu, desktop));
        }
        app.edit_settings(|s| s.onboarded = true);
        return None;
    }
    let step = if next { step + 1 } else if back { step.saturating_sub(1) } else { step };
    Some(Modal::Welcome { step, start_menu, desktop })
}

/// The once-only notice after updating. Returns whether it stays open.
pub fn show_whats_new(app: &mut App, ctx: &egui::Context) -> bool {
    let pal = app.pal;
    let mut done = false;
    let open = window(pal, ctx, "WHAT'S NEW", Vec2::new(560.0, 420.0), true, |ui| {
        ui.horizontal(|ui| {
            logo(ui, 72.0);
            ui.vertical(|ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("THE BIGGEST UPDATE YET").font(px(12.0)).color(pal.accent));
                ui.add_space(4.0);
                dim(ui, &pal, &format!("DK.FM v{}", env!("CARGO_PKG_VERSION")));
            });
        });
        ui.add_space(8.0);
        caption(ui, &pal, "HERE'S WHAT YOU GET");
        egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
            for (icon, text) in NEWS {
                line(ui, &pal, icon, text);
            }
        });
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(ui, &pal, "LET'S GO", true, true).clicked() {
                done = true;
            }
        });
    });
    if done || !open {
        app.edit_settings(|s| s.whats_new_seen = WHATS_NEW.into());
        return false;
    }
    true
}

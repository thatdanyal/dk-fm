//! The first-run welcome (a few short steps; asks before adding any shortcut) and the one-time
//! "what's new" notice for people who updated.
use super::theme::{px, vt, Pal};
use super::widgets::{button, caption, dim, switch};
use super::{settings::window, App, Modal};
use eframe::egui::{self, Vec2};

/// Bump this (and rewrite NEWS) to show the notice again after a future update.
pub const WHATS_NEW: &str = "biggest-update-2026-10b";


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
    let version = env!("CARGO_PKG_VERSION");
    if s.fresh && s.last_version != version {
        // a fresh install gets the welcome (with the whole tour), never the update notice
        s.whats_new_seen = WHATS_NEW.into();
        s.last_version = version.into();
        s.tour_seen = super::tour::TOUR_VERSION;
        app.settings_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if !s.onboarded {
        Some(Modal::Welcome { step: 0, start_menu: false, desktop: false })
    } else if s.last_version != version {
        // just updated: say what changed (and offer a tour of anything new)
        Some(Modal::WhatsNew)
    } else {
        None
    }
}

/// The DK.FM logo, `size` points square (sharp at any size: one texture per pixel size).
pub fn logo(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let res = ((size * ui.ctx().pixels_per_point()).round() as u32).clamp(16, 256);
    let id = egui::Id::new(("dkfm-logo", res));
    let tex = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id)).or_else(|| {
        let img = image::load_from_memory(include_bytes!("../../assets/icon.png")).ok()?.resize(res, res, image::imageops::FilterType::Lanczos3).to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let t = ui.ctx().load_texture("dkfm-logo", ci, egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
        Some(t)
    });
    match tex {
        Some(t) => ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(size))),
        None => ui.allocate_response(Vec2::splat(size), egui::Sense::hover()),
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
    let (mut next, mut back, mut skip, mut add_folder, mut tour) = (false, false, false, false, false);
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
                    line(ui, &pal, "🌐", "HOME > Search: find any song, artist or album, listen first, then + GET it.");
                    line(ui, &pal, "♫", "SHAZAM on the deck names a song playing on your PC or, with the microphone, around you.");
                    line(ui, &pal, "🔎", "Ctrl+K: find any song in your library, or do anything else.");
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
                    line(ui, &pal, "▶", "Space plays and pauses; Ctrl+← / Ctrl+→ skip.");
                    line(ui, &pal, "📺", "F11: THEATER, the song big with its lyrics.");
                    line(ui, &pal, "🔲", "Ctrl+E: move panels around. Ctrl+, : Settings.");
                });
                ui.add_space(6.0);
                dim(ui, &pal, "New here? SHOW ME AROUND points out the main features one by one (about a minute).");
            }
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{}/{}", step + 1, all.len())).font(px(7.0)).color(pal.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let last = step + 1 == all.len();
                if last && button(ui, &pal, "SHOW ME AROUND", true, true).clicked() {
                    next = true;
                    tour = true;
                }
                if button(ui, &pal, if last { "I'LL EXPLORE MYSELF" } else { "NEXT" }, !last, true).clicked() {
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
        app.edit_settings(|s| {
            s.onboarded = true;
            s.tour_seen = super::tour::TOUR_VERSION;
        });
        if tour {
            super::tour::start(app);
        }
        return None;
    }
    let step = if next { step + 1 } else if back { step.saturating_sub(1) } else { step };
    Some(Modal::Welcome { step, start_menu, desktop })
}

/// The tour steps this person was last offered (from before this was remembered: the first
/// tour, which the big update before offered everyone).
fn tour_seen(s: &crate::store::Settings) -> u32 {
    s.tour_seen.max(1)
}

/// What's new in this version: the release notes built into it, one line per change.
fn release_notes() -> Vec<String> {
    let v: Vec<String> = env!("DKFM_NOTES")
        .split("; ")
        .map(|x| x.trim().trim_end_matches('.'))
        .filter(|x| !x.is_empty())
        .map(|x| {
            let mut c = x.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .take(16)
        .collect();
    if v.is_empty() {
        vec!["Fixes and improvements".into()]
    } else {
        v
    }
}

/// The notice once after each update: what changed, and when the update added tour steps, a
/// tour of just those (SHOW ME WHAT'S NEW). Returns whether it stays open.
pub fn show_whats_new(app: &mut App, ctx: &egui::Context) -> bool {
    let pal = app.pal;
    let seen = tour_seen(&app.settings.lock());
    let new = super::tour::has_new(seen);
    let notes = release_notes();
    let (mut done, mut tour) = (false, false);
    // tall enough for the notes (a big update's list scrolls past about a dozen lines)
    let list_h = (notes.len() as f32 * 34.0).clamp(60.0, 340.0);
    let size = Vec2::new(600.0, list_h + if new { 210.0 } else { 180.0 });
    let open = window(pal, ctx, "WHAT'S NEW", size, true, |ui| {
        ui.horizontal(|ui| {
            logo(ui, if new { 72.0 } else { 48.0 });
            ui.vertical(|ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(if new { "NEW FEATURES" } else { "DK.FM UPDATED" }).font(px(12.0)).color(pal.accent));
                ui.add_space(4.0);
                dim(ui, &pal, &format!("DK.FM v{}", env!("CARGO_PKG_VERSION")));
            });
        });
        ui.add_space(8.0);
        caption(ui, &pal, "WHAT CHANGED");
        egui::ScrollArea::vertical().max_height(list_h).show(ui, |ui| {
            for n in &notes {
                line(ui, &pal, "•", n);
            }
        });
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if new {
                if button(ui, &pal, "SHOW ME WHAT'S NEW", true, true).on_hover_text("A quick tour of just the new features").clicked() {
                    done = true;
                    tour = true;
                }
                if button(ui, &pal, "LATER", false, true).on_hover_text("Ctrl+K > Take the tour shows everything any time").clicked() {
                    done = true;
                }
            } else if button(ui, &pal, "OK", true, true).clicked() {
                done = true;
            }
        });
    });
    if done || !open {
        app.edit_settings(|s| {
            s.whats_new_seen = WHATS_NEW.into();
            s.last_version = env!("CARGO_PKG_VERSION").into();
            s.tour_seen = super::tour::TOUR_VERSION;
        });
        if tour {
            super::tour::start_new(app, seen);
        }
        return false;
    }
    true
}

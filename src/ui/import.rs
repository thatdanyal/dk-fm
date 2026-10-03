//! Import (Spotify / YouTube / SoundCloud / profiles / "More like this" radio) and Downloads.
use super::browser::View;
use super::theme::{px, vt};
use super::widgets::{button, fill, fmt_time, frame_rect, tb_button};
use super::{App, Modal, PromptAction};
use crate::downloader::TStatus;
use crate::library::ta_key;
use crate::sources::{self, Collection, Fetched, ITrack};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, TextureHandle, Ui, Vec2};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub struct ImportState {
    pub input: String,
    pub busy: bool,
    pending: Arc<Mutex<Option<(Result<Fetched, String>, Option<egui::ColorImage>)>>>,
    pub result: Option<Fetched>,
    cover: Option<TextureHandle>,
    pub selected: HashSet<usize>,
    collapsed: HashSet<String>,
}

fn library_keys(app: &App) -> HashSet<String> {
    app.lib.key_index().into_keys().collect()
}

fn owned(keys: &HashSet<String>, t: &ITrack) -> bool {
    keys.contains(&t.source_key) || t.spotify_id.as_ref().map(|s| keys.contains(&format!("sp:{s}"))).unwrap_or(false) || t.youtube_id.as_ref().map(|y| keys.contains(&format!("yt:{y}"))).unwrap_or(false) || keys.contains(&ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title))
}

fn spawn_fetch(app: &mut App, ctx: &egui::Context, job: impl FnOnce() -> Result<Fetched, String> + Send + 'static) {
    app.import.busy = true;
    app.import.result = None;
    app.import.cover = None;
    let slot = app.import.pending.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let r = job();
        if let Ok(f) = &r {
            super::cjk::ensure(&ctx, &format!("{f:?}"));
        }
        let cover_url = match &r {
            Ok(Fetched::Collection(c)) => c.cover.clone(),
            Ok(Fetched::Profile { cover, .. }) => cover.clone(),
            _ => None,
        };
        let img = cover_url.and_then(|u| crate::net::get_bytes(&u).ok()).and_then(|b| image::load_from_memory(&b).ok()).map(|i| {
            let i = i.thumbnail(128, 128).to_rgba8();
            egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], &i)
        });
        *slot.lock() = Some((r, img));
        ctx.request_repaint();
    });
}

pub fn fetch_link(app: &mut App, ctx: &egui::Context, url: String) {
    app.import.input = url.clone();
    let creds = app.dl.creds();
    spawn_fetch(app, ctx, move || sources::fetch_any(&url, &creds));
}

/// Connected account: list Liked Songs, every playlist and saved albums for a one-click import.
pub fn fetch_library(app: &mut App) {
    app.browser.set_view(View::Import);
    app.import.input = String::new();
    let creds = app.dl.creds();
    let ctx = app.covers.ctx.clone().unwrap();
    spawn_fetch(app, &ctx, move || sources::library(&creds));
}

/// "More like this": YouTube Music radio for a song, shown in the import view for picking.
pub fn radio_for(app: &mut App, t: &crate::store::Track) {
    app.browser.set_view(View::Import);
    let (title, artist, yid) = (t.title.clone(), t.artist.clone(), t.youtube_id.clone());
    let ctx = app.covers.ctx.clone().unwrap();
    app.import.input = String::new();
    spawn_fetch(app, &ctx, move || sources::radio(&title, &artist, yid.as_deref()).map(Fetched::Collection));
}

fn take_result(app: &mut App, ctx: &egui::Context) {
    let got = app.import.pending.lock().take();
    if let Some((r, img)) = got {
        app.import.busy = false;
        match r {
            Ok(mut f) => {
                // radio leaves out songs you hid
                if let Fetched::Collection(c) = &mut f {
                    if c.kind == "radio" {
                        let hidden: HashSet<String> = {
                            let d = app.lib.data.read();
                            d.stats.iter().filter(|(_, s)| s.hidden).filter_map(|(id, _)| d.tracks.get(id)).flat_map(|t| crate::library::track_keys(t.source_key.as_deref(), t.spotify_id.as_deref(), t.youtube_id.as_deref(), &t.artist, &t.title)).collect()
                        };
                        if !hidden.is_empty() {
                            c.tracks.retain(|t| !owned(&hidden, t));
                        }
                    }
                }
                let keys = library_keys(app);
                let hide_explicit = app.settings.lock().hide_explicit;
                app.import.selected = match &f {
                    Fetched::Collection(c) => c.tracks.iter().enumerate().filter(|(_, t)| !owned(&keys, t) && !(hide_explicit && t.explicit)).map(|(i, _)| i).collect(),
                    Fetched::Profile { playlists, .. } => (0..playlists.len()).collect(),
                };
                if let Fetched::Collection(c) = &f {
                    if let Some(w) = &c.warning {
                        app.toast_err(w.clone());
                    }
                }
                app.import.result = Some(f);
                app.import.cover = img.map(|i| ctx.load_texture("import-cover", i, egui::TextureOptions::LINEAR));
            }
            Err(e) => app.toast_err(e),
        }
    }
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    take_result(app, ui.ctx());
    // first visit: fetch the download tools in the background (nothing is downloaded before that)
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        std::thread::spawn(|| {
            let _ = crate::ytdlp::ensure();
        });
    }
    egui::Frame::new().inner_margin(egui::Margin::same(16)).show(ui, |ui| {
        ui.label(egui::RichText::new("IMPORT MUSIC").font(px(12.0)).color(pal.text));
        ui.add_space(6.0);
        let mut go = false;
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut app.import.input).hint_text("Paste a Spotify, YouTube, YouTube Music or SoundCloud link, or a friend's DK.FM code…").desired_width(ui.available_width() - 110.0).font(vt(19.0)));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            if button(ui, &pal, if app.import.busy { "LOADING…" } else { "FETCH" }, true, !app.import.busy).clicked() {
                go = true;
            }
        });
        if app.import.input.contains(crate::share::PREFIX) {
            let t = std::mem::take(&mut app.import.input);
            super::sharing::receive(app, &t);
        }
        if go && !app.import.input.trim().is_empty() && !app.import.busy {
            let u = app.import.input.trim().to_string();
            fetch_link(app, ui.ctx(), u);
        }
        let connected = !app.settings.lock().spotify_refresh_token.is_empty();
        ui.label(egui::RichText::new("Spotify playlists, albums & songs · YouTube / YouTube Music playlists, albums & videos · SoundCloud. Spotify songs are matched to the best studio version on YouTube Music. Imported playlists stay in sync automatically.").color(pal.dim));
        ui.add_space(8.0);
        if connected {
            if button(ui, &pal, "IMPORT MY WHOLE SPOTIFY LIBRARY", true, !app.import.busy).clicked() {
                fetch_library(app);
            }
        } else if ui.add(egui::Label::new(egui::RichText::new("Bring over your whole Spotify library at once (Liked Songs, all playlists, albums): connect Spotify >").color(pal.accent2)).sense(Sense::click())).clicked() {
            app.modal = Some(Modal::Settings(super::settings::SetTab::Spotify));
        }
        ui.add_space(4.0);
        if ui.add(egui::Label::new(egui::RichText::new("Got a playlist file from a friend? Open .dkfm file >").color(pal.accent2)).sense(Sense::click())).clicked() {
            super::sharing::pick_file(app);
        }
    });
    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), egui::Stroke::new(2.0_f32, pal.line));
    let Some(res) = app.import.result.clone() else {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            if app.import.busy {
                ui.label(egui::RichText::new("TUNING IN…").font(px(9.0)).color(pal.accent));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
            } else {
                ui.label(egui::RichText::new("PASTE A LINK ABOVE").font(px(9.0)).color(pal.accent));
                ui.label(egui::RichText::new("Tip: right-click any song > \"More like this\" to discover similar music.").color(pal.dim));
            }
        });
        return;
    };
    match res {
        Fetched::Profile { name, playlists, .. } => profile(app, ui, &name, &playlists),
        Fetched::Collection(c) => collection(app, ui, &c),
    }
}

fn header_cover(app: &App, ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
    fill(ui.painter(), r, app.pal.bg);
    if let Some(t) = &app.import.cover {
        ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }
    frame_rect(ui.painter(), r, 2.0, app.pal.line_hi);
}

fn collection(app: &mut App, ui: &mut Ui, c: &Collection) {
    let pal = app.pal;
    let keys = library_keys(app);
    let fmt = match app.settings.lock().download_format.as_str() { "mp3-320" => "MP3 320", "mp3-v0" => "MP3 V0", _ => "M4A" };
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            header_cover(app, ui);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&c.name).font(px(11.0)).color(pal.text));
                let via = match c.via.as_str() { "api" => "SPOTIFY API", "embed" => "SPOTIFY PUBLIC PAGE", "youtube" => "YOUTUBE", _ => "SOUNDCLOUD" };
                let kind = if c.kind == "radio" { "RADIO".to_string() } else { c.kind.to_uppercase() };
                ui.label(egui::RichText::new([kind, c.owner.clone(), format!("{} TRACKS", c.tracks.len()), via.into()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")).color(pal.dim));
                if !c.complete {
                    ui.label(egui::RichText::new("⚠ Spotify's public page only lists the first 100 tracks. Connect your Spotify account in Settings to get all of your own playlists.").color(pal.accent2));
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if button(ui, &pal, "ALL", false, true).clicked() {
                app.import.selected = (0..c.tracks.len()).collect();
            }
            if button(ui, &pal, "NONE", false, true).clicked() {
                app.import.selected.clear();
            }
            let n = app.import.selected.len();
            if button(ui, &pal, &format!("DOWNLOAD {n} AS {fmt}"), true, n > 0).clicked() {
                let mut sel: Vec<usize> = app.import.selected.iter().copied().collect();
                sel.sort();
                app.dl.start(c.clone(), Some(sel), false);
                app.browser.set_view(View::Downloads);
            }
        });
    });
    egui::ScrollArea::vertical().id_salt("import-list").auto_shrink([false; 2]).show_rows(ui, 28.0, c.tracks.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in range {
            let t = &c.tracks[i];
            let w = ui.available_width();
            let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
            if resp.hovered() {
                fill(ui.painter(), r, pal.panel_hi);
            }
            let on = app.import.selected.contains(&i);
            let cb = Rect::from_center_size(Pos2::new(r.left() + 20.0, r.center().y), Vec2::splat(14.0));
            fill(ui.painter(), cb, if on { pal.accent } else { pal.bg });
            frame_rect(ui.painter(), cb, 2.0, if on { pal.accent } else { pal.line_hi });
            if on {
                ui.painter().text(cb.center(), Align2::CENTER_CENTER, "✔", vt(16.0), pal.ink);
            }
            if resp.clicked() {
                if !app.import.selected.remove(&i) {
                    app.import.selected.insert(i);
                }
            }
            let p = ui.painter();
            let cy = r.center().y;
            p.text(Pos2::new(r.left() + 60.0, cy), Align2::RIGHT_CENTER, (i + 1).to_string(), vt(18.0), pal.dim);
            let narrow = w < 600.0;
            let tw = if narrow { w - 260.0 } else { (w - 290.0) * 0.6 };
            p.with_clip_rect(Rect::from_min_size(Pos2::new(r.left() + 72.0, r.top()), Vec2::new(tw, 28.0))).text(Pos2::new(r.left() + 72.0, cy), Align2::LEFT_CENTER, &t.title, vt(19.0), pal.text);
            if !narrow {
                let ax = r.left() + 80.0 + tw;
                p.with_clip_rect(Rect::from_min_size(Pos2::new(ax, r.top()), Vec2::new((w - 290.0) * 0.4, 28.0))).text(Pos2::new(ax, cy), Align2::LEFT_CENTER, t.artists.join(", "), vt(19.0), pal.dim);
            }
            p.text(Pos2::new(r.right() - 130.0, cy), Align2::RIGHT_CENTER, t.duration_ms.map(|d| fmt_time(d as f64 / 1000.0)).unwrap_or_default(), vt(18.0), pal.dim);
            if owned(&keys, t) {
                p.text(Pos2::new(r.right() - 12.0, cy), Align2::RIGHT_CENTER, "IN LIBRARY", px(6.0), pal.accent2);
            } else if t.explicit {
                p.text(Pos2::new(r.right() - 12.0, cy), Align2::RIGHT_CENTER, "E", px(6.0), pal.dim);
            }
        }
    });
}

fn profile(app: &mut App, ui: &mut Ui, name: &str, playlists: &[sources::ProfilePlaylist]) {
    let pal = app.pal;
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            header_cover(app, ui);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(name).font(px(11.0)).color(pal.text));
                let count = |k: &str| playlists.iter().filter(|p| p.kind == k).count();
                let songs: u64 = playlists.iter().map(|p| p.total).sum();
                ui.label(egui::RichText::new(format!("{} PLAYLISTS · {} ALBUMS{} · {songs} SONGS", count("PLAYLIST"), count("ALBUM"), if count("LIKED") > 0 { " · LIKED SONGS" } else { "" })).color(pal.dim));
                ui.label(egui::RichText::new("Songs you already have are skipped. Everything keeps syncing: new likes and playlist additions download by themselves.").color(pal.dim));
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if button(ui, &pal, "ALL", false, true).clicked() {
                app.import.selected = (0..playlists.len()).collect();
            }
            if button(ui, &pal, "NONE", false, true).clicked() {
                app.import.selected.clear();
            }
            let n = app.import.selected.len();
            if button(ui, &pal, &format!("IMPORT {n} SELECTED"), true, n > 0).clicked() {
                let urls: Vec<(String, String)> = app.import.selected.iter().map(|&i| (playlists[i].name.clone(), playlists[i].url.clone())).collect();
                let (dl, lib) = (app.dl.clone(), app.lib.clone());
                app.toast(format!("Importing {} lists — they'll stay in sync automatically", urls.len()));
                app.browser.set_view(View::Downloads);
                std::thread::spawn(move || {
                    for (name, url) in urls {
                        match sources::fetch_any(&url, &dl.creds()) {
                            Ok(Fetched::Collection(c)) => {
                                let keys: HashSet<String> = lib.key_index().into_keys().collect();
                                let sel: Vec<usize> = c.tracks.iter().enumerate().filter(|(_, t)| !owned(&keys, t)).map(|(i, _)| i).collect();
                                dl.start(c, Some(sel), false);
                            }
                            Ok(_) => {}
                            Err(e) => dl.notices.lock().push(format!("{name}: {e}")),
                        }
                    }
                });
            }
        });
    });
    egui::ScrollArea::vertical().id_salt("profile").auto_shrink([false; 2]).show(ui, |ui| {
        for (i, p) in playlists.iter().enumerate() {
            let mut on = app.import.selected.contains(&i);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                if ui.checkbox(&mut on, "").changed() {
                    if on { app.import.selected.insert(i); } else { app.import.selected.remove(&i); }
                }
                ui.label(egui::RichText::new(&p.kind).font(px(6.0)).color(if p.kind == "LIKED" { pal.accent } else { pal.dim }));
                ui.label(egui::RichText::new(&p.name).color(pal.text));
                ui.label(egui::RichText::new(format!("{} ♪ · {}", p.total, p.owner)).color(pal.dim));
                if !p.note.is_empty() {
                    ui.label(egui::RichText::new(format!("⚠ {}", p.note)).color(pal.accent2));
                }
            });
        }
    });
}

// ---------------------------------------------------------------------------- downloads

pub fn downloads(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let tools = crate::ytdlp::STATUS.lock().clone();
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("DOWNLOADS").font(px(12.0)).color(pal.text));
                let eng = match tools.state.as_str() {
                    "installing" => "ENGINE: DOWNLOADING TOOLS…".to_string(),
                    "updating" => "ENGINE: UPDATING YT-DLP…".into(),
                    "ready" => format!("ENGINE: YT-DLP {} ✔", tools.version),
                    "error" => format!("ENGINE ERROR: {}", tools.error),
                    _ => "ENGINE: STANDBY".into(),
                };
                ui.label(egui::RichText::new(eng).color(pal.dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, &pal, "OPEN FOLDER", false, true).clicked() {
                    let _ = open::that(&app.settings.lock().download_dir);
                }
                if button(ui, &pal, "+ IMPORT", false, true).clicked() {
                    app.browser.set_view(View::Import);
                }
            });
        });
    });
    // snapshot the jobs so the UI never holds the lock while drawing
    struct JView {
        id: String,
        name: String,
        folder: std::path::PathBuf,
        kind: String,
        col_id: String,
        tracks: Vec<crate::downloader::JTrack>,
    }
    let jobs: Vec<JView> = {
        let mut j: Vec<JView> = app.dl.jobs.lock().iter().map(|j| JView { id: j.id.clone(), name: j.col.name.clone(), folder: j.folder.clone(), kind: j.col.kind.clone(), col_id: j.col.id.clone(), tracks: j.tracks.clone() }).collect();
        j.reverse();
        j
    };
    if jobs.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("NO DOWNLOADS").font(px(9.0)).color(pal.accent));
            ui.label(egui::RichText::new("Import a playlist to get started.").color(pal.dim));
        });
        return;
    }
    let any_active = jobs.iter().any(|j| j.tracks.iter().any(|t| t.status.active()));
    if any_active {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
    }
    let mut actions: Vec<Box<dyn FnOnce(&mut App)>> = Vec::new();
    egui::ScrollArea::vertical().id_salt("jobs").auto_shrink([false; 2]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
            for j in &jobs {
                let total = j.tracks.iter().filter(|t| t.status != TStatus::Skipped).count();
                let done = j.tracks.iter().filter(|t| t.status == TStatus::Done).count();
                let failed = j.tracks.iter().filter(|t| t.status == TStatus::Failed).count();
                let active = j.tracks.iter().filter(|t| t.status.active()).count();
                egui::Frame::new().fill(pal.bg2).stroke(egui::Stroke::new(2.0_f32, pal.line_hi)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&j.name).font(px(9.0)).color(pal.text));
                            ui.label(egui::RichText::new(format!("{done}/{total} DONE{}{}", if failed > 0 { format!(" · {failed} FAILED") } else { String::new() }, if active > 0 { format!(" · {active} IN PROGRESS") } else { String::new() })).color(pal.dim));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let open = !app.import.collapsed.contains(&j.id);
                            if tb_button(ui, &pal, if open { "▲" } else { "▼" }, false).clicked() {
                                let id = j.id.clone();
                                actions.push(Box::new(move |a: &mut App| { if !a.import.collapsed.remove(&id) { a.import.collapsed.insert(id); } }));
                            }
                            if active == 0 && tb_button(ui, &pal, "DISMISS", false).clicked() {
                                let id = j.id.clone();
                                actions.push(Box::new(move |a: &mut App| a.dl.clear(&id)));
                            }
                            if active > 0 && tb_button(ui, &pal, "CANCEL", false).clicked() {
                                let id = j.id.clone();
                                actions.push(Box::new(move |a: &mut App| a.dl.cancel(&id)));
                            }
                            if failed > 0 && tb_button(ui, &pal, "RETRY FAILED", false).clicked() {
                                let id = j.id.clone();
                                let idxs: Vec<usize> = j.tracks.iter().enumerate().filter(|(_, t)| t.status == TStatus::Failed).map(|(i, _)| i).collect();
                                actions.push(Box::new(move |a: &mut App| { for i in idxs { a.dl.retry(&id, i, None); } }));
                            }
                            if tb_button(ui, &pal, "FOLDER", false).clicked() {
                                let _ = open::that(&j.folder);
                            }
                            if (j.kind == "playlist" || j.kind == "album" || j.kind == "shared") && done > 0 && tb_button(ui, &pal, "▶ OPEN", false).clicked() {
                                let pid = if j.kind == "shared" { j.col_id.clone() } else { format!("sp-{}-{}", j.kind, j.col_id) };
                                actions.push(Box::new(move |a: &mut App| a.browser.set_view(View::Playlist(pid))));
                            }
                        });
                    });
                    let (br, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 10.0), Sense::hover());
                    fill(ui.painter(), br, pal.bg);
                    frame_rect(ui.painter(), br, 2.0, pal.line_hi);
                    let f = if total > 0 { (done + failed) as f32 / total as f32 } else { 0.0 };
                    fill(ui.painter(), Rect::from_min_size(br.min + Vec2::splat(2.0), Vec2::new((br.width() - 4.0) * f, 6.0)), pal.accent);
                    if !app.import.collapsed.contains(&j.id) {
                        ui.add_space(6.0);
                        for (i, t) in j.tracks.iter().enumerate().filter(|(_, t)| t.status != TStatus::Skipped) {
                            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
                            if resp.hovered() {
                                fill(ui.painter(), r, pal.panel_hi);
                            }
                            let p = ui.painter();
                            let cy = r.center().y;
                            let w = r.width();
                            p.text(Pos2::new(r.left() + 26.0, cy), Align2::RIGHT_CENTER, (i + 1).to_string(), vt(17.0), pal.dim);
                            let title = format!("{} — {}", t.t.title, t.t.artists.join(", "));
                            p.with_clip_rect(Rect::from_min_size(Pos2::new(r.left() + 36.0, r.top()), Vec2::new(w * 0.42, 24.0))).text(Pos2::new(r.left() + 36.0, cy), Align2::LEFT_CENTER, title, vt(18.0), pal.text);
                            if w > 560.0 {
                                let m = t.matched.as_ref().map(|m| format!("{}{}", if t.low_confidence { "⚠ " } else { "" }, if m.source == "direct" { "direct link".into() } else { format!("{} — {}", m.title, m.channel) })).or(t.error.clone()).unwrap_or_default();
                                p.with_clip_rect(Rect::from_min_size(Pos2::new(r.left() + 46.0 + w * 0.42, r.top()), Vec2::new(w * 0.58 - 200.0, 24.0))).text(Pos2::new(r.left() + 46.0 + w * 0.42, cy), Align2::LEFT_CENTER, m, vt(16.0), if t.low_confidence || t.status == TStatus::Failed { pal.accent2 } else { pal.dim });
                            }
                            let (label, col) = match t.status {
                                TStatus::Queued => ("QUEUED".to_string(), pal.dim),
                                TStatus::Searching => ("SEARCHING".into(), pal.text),
                                TStatus::Downloading => (format!("{:.0}%", t.progress), pal.text),
                                TStatus::Tagging => ("TAGGING".into(), pal.text),
                                TStatus::Done => (if t.note.is_some() { "HAD IT".into() } else { "DONE ✔".into() }, pal.accent2),
                                TStatus::Failed => ("FAILED".into(), pal.accent),
                                _ => ("CANCELLED".into(), pal.dim),
                            };
                            if t.status == TStatus::Downloading {
                                let b = Rect::from_min_size(Pos2::new(r.right() - 130.0, cy - 4.0), Vec2::new(60.0, 8.0));
                                frame_rect(p, b, 1.0, pal.line_hi);
                                fill(p, Rect::from_min_size(b.min, Vec2::new(60.0 * t.progress / 100.0, 8.0)), pal.accent);
                            }
                            p.text(Pos2::new(r.right() - 6.0, cy), Align2::RIGHT_CENTER, label, px(6.0), col);
                            if let Some(e) = &t.error {
                                resp.clone().on_hover_text(e);
                            }
                            let busy = matches!(t.status, TStatus::Searching | TStatus::Downloading | TStatus::Tagging);
                            let (jid, tt) = (j.id.clone(), t.clone());
                            resp.context_menu(|ui| {
                                ui.set_min_width(220.0);
                                if ui.add_enabled(tt.track_id.is_some(), egui::Button::new("Play")).clicked() {
                                    let id = tt.track_id.clone().unwrap();
                                    actions.push(Box::new(move |a: &mut App| a.player.play_pick(vec![id], 0, Some(false))));
                                    ui.close_menu();
                                }
                                if ui.add_enabled(!busy, egui::Button::new("Retry")).clicked() {
                                    let id = jid.clone();
                                    actions.push(Box::new(move |a: &mut App| a.dl.retry(&id, i, None)));
                                    ui.close_menu();
                                }
                                ui.add_enabled_ui(!busy && !tt.candidates.is_empty(), |ui| {
                                    ui.menu_button("Use a different match", |ui| {
                                        for c in &tt.candidates {
                                            let lbl = format!("{}{} ({}{})", if Some(&c.id) == tt.matched.as_ref().map(|m| &m.id) { "• " } else { "" }, c.title, if c.channel.is_empty() { &c.source } else { &c.channel }, c.duration.map(|d| format!(", {}", fmt_time(d))).unwrap_or_default());
                                            if ui.button(lbl).clicked() {
                                                let (id, vid) = (jid.clone(), c.id.clone());
                                                actions.push(Box::new(move |a: &mut App| a.dl.retry(&id, i, Some(vid))));
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                });
                                if ui.add_enabled(!busy, egui::Button::new("Paste YouTube link…")).clicked() {
                                    let id = jid.clone();
                                    actions.push(Box::new(move |a: &mut App| a.modal = Some(Modal::Prompt { title: "YOUTUBE LINK FOR THIS SONG".into(), text: String::new(), action: PromptAction::PasteYoutube(id, i) })));
                                    ui.close_menu();
                                }
                                if ui.add_enabled(tt.matched.as_ref().map(|m| !m.id.is_empty()).unwrap_or(false), egui::Button::new("Open match on YouTube")).clicked() {
                                    let _ = open::that(format!("https://music.youtube.com/watch?v={}", tt.matched.as_ref().unwrap().id));
                                    ui.close_menu();
                                }
                                if ui.add_enabled(tt.track_id.is_some(), egui::Button::new("Show in folder")).clicked() {
                                    let id = tt.track_id.clone().unwrap();
                                    actions.push(Box::new(move |a: &mut App| { if let Some(t) = a.lib.track(&id) { super::queue::reveal(&t.path); } }));
                                    ui.close_menu();
                                }
                            });
                        }
                    }
                    let _ = &j.folder;
                });
                ui.add_space(12.0);
            }
        });
    });
    for a in actions {
        a(app);
    }
    let _: HashMap<(), ()> = HashMap::new();
}

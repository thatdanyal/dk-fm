//! Duplicates: copies of the same song (same artist + title, about the same length). Merging
//! keeps one copy, moves plays / likes / playlist spots onto it and sends the extra files to the
//! Recycle Bin.
use super::theme::{px, vt};
use super::widgets::{button, fill, fmt_time, frame_rect};
use super::App;
use eframe::egui::{self, Align2, Pos2, Rect, Sense, Ui, Vec2};
use std::collections::HashMap;

#[derive(Default)]
pub struct DupeState {
    groups: Vec<Vec<String>>,
    gen: u64,
    /// group's first id -> the copy you picked to keep (default: the best one)
    keep: HashMap<String, String>,
}

pub(super) fn merge(app: &mut App, groups: Vec<(String, Vec<String>)>) {
    let playing = app.player.current_id();
    let all: Vec<String> = groups.iter().flat_map(|g| g.1.iter().cloned()).collect();
    let mut u = app.lib.snapshot(format!("Merged {} duplicate{}", groups.len(), if groups.len() == 1 { "" } else { "s" }), &[], &all);
    let mut files = Vec::new();
    let mut skipped = 0;
    for (keep, all) in groups {
        if playing.as_ref().map(|p| *p != keep && all.contains(p)).unwrap_or(false) {
            skipped += 1; // that file is open in the player
            continue;
        }
        files.extend(app.lib.merge_duplicates(&keep, &all));
    }
    let n = files.len();
    let dl = app.dl.clone();
    std::thread::spawn(move || {
        let failed: Vec<String> = files.iter().filter_map(|f| crate::system::trash(f).err()).collect();
        if let Some(e) = failed.first() {
            dl.notices.lock().push(format!("{} file{} could not be moved to the Recycle Bin: {e}", failed.len(), if failed.len() == 1 { "" } else { "s" }));
        }
    });
    u.trashed = n;
    if n > 0 {
        app.undoable(u);
    }
    app.toast(format!("Merged: {n} extra cop{} moved to the Recycle Bin{}", if n == 1 { "y" } else { "ies" }, if skipped > 0 { format!(" ({skipped} skipped: playing now)") } else { String::new() }));
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
    if app.browser.dupes.gen != gen {
        app.browser.dupes.groups = app.lib.duplicate_groups();
        app.browser.dupes.gen = gen;
    }
    let groups = app.browser.dupes.groups.clone();
    let keep_of = |app: &App, g: &[String]| app.browser.dupes.keep.get(&g[0]).filter(|k| g.contains(k)).cloned().unwrap_or_else(|| g[0].clone());
    let extra: usize = groups.iter().map(|g| g.len() - 1).sum();
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.label(egui::RichText::new("DUPLICATES").font(px(12.0)).color(pal.text));
        ui.label(egui::RichText::new(if groups.is_empty() { "NO DUPLICATES · YOUR LIBRARY IS CLEAN".to_string() } else { format!("{} SONGS HAVE EXTRA COPIES · {extra} FILES CAN GO", groups.len()) }).color(pal.dim));
        ui.label(egui::RichText::new("DK.FM keeps the best copy (lossless, then higher bitrate, then has cover art); click a row to keep that one instead. Plays, likes and playlist spots move to it, and the extra files go to the Recycle Bin.").color(pal.dim));
        ui.add_space(6.0);
        if button(ui, &pal, &format!("MERGE ALL ({extra})"), true, extra > 0).clicked() {
            let all: Vec<(String, Vec<String>)> = groups.iter().map(|g| (keep_of(app, g), g.clone())).collect();
            merge(app, all);
        }
    });
    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), egui::Stroke::new(2.0_f32, pal.line));
    let mut act: Option<Box<dyn FnOnce(&mut App)>> = None;
    egui::ScrollArea::vertical().id_salt("dupes").auto_shrink([false; 2]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for g in &groups {
            let keep = keep_of(app, g);
            let tracks: Vec<_> = g.iter().filter_map(|id| app.lib.track(id)).collect();
            let Some(first) = tracks.first() else { continue };
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(egui::RichText::new(format!("{} — {}", first.title, first.artist)).font(vt(20.0)).color(pal.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(12.0);
                    if button(ui, &pal, "MERGE", false, true).clicked() {
                        let (k, all) = (keep.clone(), g.clone());
                        act = Some(Box::new(move |a: &mut App| merge(a, vec![(k, all)])));
                    }
                });
            });
            for t in &tracks {
                let w = ui.available_width();
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
                let on = t.id == keep;
                if resp.hovered() {
                    fill(ui.painter(), r, pal.panel_hi);
                }
                let p = ui.painter();
                let cy = r.center().y;
                let cb = Rect::from_center_size(Pos2::new(r.left() + 26.0, cy), Vec2::splat(14.0));
                fill(p, cb, if on { pal.accent } else { pal.bg });
                frame_rect(p, cb, 2.0, if on { pal.accent } else { pal.line_hi });
                p.text(Pos2::new(r.left() + 42.0, cy), Align2::LEFT_CENTER, if on { "KEEP" } else { "REMOVE" }, px(6.0), if on { pal.accent } else { pal.dim });
                let plays = app.lib.stat(&t.id).plays;
                let info = format!("{} · {} kbps · {}{}{}", if t.codec.is_empty() { "?" } else { &t.codec }, t.bitrate.unwrap_or(0), fmt_time(t.duration), if t.cover.is_some() { " · cover" } else { "" }, if plays > 0 { format!(" · {plays} plays") } else { String::new() });
                p.text(Pos2::new(r.left() + 110.0, cy), Align2::LEFT_CENTER, info, vt(17.0), pal.dim);
                let path_x = r.left() + 380.0;
                if w > 520.0 {
                    p.with_clip_rect(Rect::from_min_max(Pos2::new(path_x, r.top()), Pos2::new(r.right() - 8.0, r.bottom()))).text(Pos2::new(path_x, cy), Align2::LEFT_CENTER, &t.path, vt(16.0), pal.faint);
                }
                let resp = resp.on_hover_text(&t.path);
                if resp.clicked() {
                    let (g0, id) = (g[0].clone(), t.id.clone());
                    act = Some(Box::new(move |a: &mut App| {
                        a.browser.dupes.keep.insert(g0, id);
                    }));
                }
                let path = t.path.clone();
                resp.context_menu(|ui| {
                    if ui.button("Show in folder").clicked() {
                        super::queue::reveal(&path);
                        ui.close_menu();
                    }
                });
            }
        }
    });
    if let Some(a) = act {
        a(app);
    }
}

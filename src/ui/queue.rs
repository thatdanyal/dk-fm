//! Queue: history + upcoming, drag to reorder, save as playlist.
use super::theme::{px, vt};
use super::widgets::{fill, fmt_long, frame_rect, tb_button};
use super::{App, Modal, PromptAction};
use eframe::egui::{self, Align2, Pos2, Rect, Sense, Ui, Vec2};

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let (queue, index) = { let st = app.player.st.lock(); (st.queue.clone(), st.index) };
    let upcoming: Vec<_> = queue.iter().skip((index + 1).max(0) as usize).filter_map(|id| app.lib.track(id)).collect();
    let secs: f64 = upcoming.iter().map(|t| t.duration).sum();
    egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} UP NEXT · {}", upcoming.len(), fmt_long(secs))).font(px(6.0)).color(pal.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if tb_button(ui, &pal, "CLEAR", false).clicked() {
                    app.player.clear_upcoming();
                }
                if tb_button(ui, &pal, "SAVE", false).clicked() && !queue.is_empty() {
                    app.modal = Some(Modal::Prompt { title: "SAVE QUEUE AS PLAYLIST".into(), text: "My Queue".into(), action: PromptAction::NewPlaylist(queue.clone()) });
                }
            });
        });
    });
    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), egui::Stroke::new(2.0_f32, pal.line));
    if queue.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("QUEUE EMPTY").font(px(9.0)).color(pal.accent));
            ui.label(egui::RichText::new("Double-click a song to start").color(pal.dim));
        });
        return;
    }
    let from = (index - 3).max(0) as usize;
    let to = (index as usize + 150).min(queue.len());
    let mut action: Option<Box<dyn FnOnce(&App)>> = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("queue").show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in from..to {
            let Some(t) = app.lib.track(&queue[i]) else { continue };
            if i as isize == index + 1 {
                ui.label(egui::RichText::new("  UP NEXT").font(px(5.0)).color(pal.dim));
            }
            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click_and_drag());
            let p = ui.painter();
            let current = i as isize == index;
            if current {
                fill(p, r, pal.sel);
                fill(p, Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), pal.accent);
            } else if resp.hovered() {
                fill(p, r, pal.panel_hi);
            }
            let alpha = if (i as isize) < index { 100 } else { 255 };
            let thumb = Rect::from_min_size(r.min + Vec2::new(8.0, 6.0), Vec2::splat(30.0));
            fill(p, thumb, pal.bg);
            if let Some(c) = t.thumb.clone().or(t.cover.clone()) {
                if let Some(tex) = app.covers.get(ui.ctx(), app.lib.cover_path(&c), &c, 64) {
                    ui.painter().image(tex, thumb, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), egui::Color32::from_white_alpha(alpha));
                }
            }
            frame_rect(ui.painter(), thumb, 1.0, pal.line_hi);
            let clip = ui.painter().with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.right() - 26.0, r.bottom())));
            clip.text(r.min + Vec2::new(46.0, 5.0), Align2::LEFT_TOP, &t.title, vt(18.0), super::widgets::with_alpha(if current { pal.accent } else { pal.text }, alpha));
            clip.text(r.min + Vec2::new(46.0, 22.0), Align2::LEFT_TOP, &t.artist, vt(16.0), super::widgets::with_alpha(pal.dim, alpha));
            if resp.hovered() {
                let xr = Rect::from_min_size(Pos2::new(r.right() - 24.0, r.top() + 11.0), Vec2::splat(20.0));
                let xresp = ui.interact(xr, ui.id().with(("qx", i)), Sense::click());
                ui.painter().text(xr.center(), Align2::CENTER_CENTER, "×", vt(20.0), if xresp.hovered() { pal.accent } else { pal.dim });
                if xresp.clicked() {
                    action = Some(Box::new(move |a: &App| a.player.remove_at(i)));
                }
            }
            if resp.double_clicked() {
                action = Some(Box::new(move |a: &App| a.player.play_index(i)));
            }
            // drag to reorder
            if resp.drag_started() {
                ui.memory_mut(|m| m.data.insert_temp(egui::Id::new("qdrag"), i));
            }
            let dragging: Option<usize> = ui.memory(|m| m.data.get_temp(egui::Id::new("qdrag")));
            if let (Some(from_i), Some(pos)) = (dragging, ui.ctx().pointer_hover_pos()) {
                if r.contains(pos) && from_i != i {
                    ui.painter().hline(r.x_range(), r.top() + 1.0, egui::Stroke::new(3.0_f32, pal.accent2));
                    if ui.input(|inp| inp.pointer.any_released()) {
                        let to_i = if from_i < i { i - 1 } else { i };
                        action = Some(Box::new(move |a: &App| a.player.move_item(from_i, to_i)));
                        ui.memory_mut(|m| m.data.remove::<usize>(egui::Id::new("qdrag")));
                    }
                }
            }
            resp.context_menu(|ui| {
                if ui.button("Play now").clicked() {
                    action = Some(Box::new(move |a: &App| a.player.play_index(i)));
                    ui.close_menu();
                }
                if ui.add_enabled((i as isize) > index + 1, egui::Button::new("Move to next")).clicked() {
                    let to_i = (index + 1) as usize;
                    action = Some(Box::new(move |a: &App| a.player.move_item(i, to_i)));
                    ui.close_menu();
                }
                if ui.button("Remove from queue").clicked() {
                    action = Some(Box::new(move |a: &App| a.player.remove_at(i)));
                    ui.close_menu();
                }
                if ui.button("Show in folder").clicked() {
                    reveal(&t.path);
                    ui.close_menu();
                }
            });
        }
        if to < queue.len() {
            ui.label(egui::RichText::new(format!("  + {} MORE", queue.len() - to)).font(px(5.0)).color(pal.dim));
        }
    });
    if ui.input(|i| i.pointer.any_released()) {
        ui.memory_mut(|m| m.data.remove::<usize>(egui::Id::new("qdrag")));
    }
    if let Some(a) = action {
        a(app);
    }
}

/// Open the file's folder with the file selected.
pub fn reveal(path: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{path}\"")).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").args(["-R", path]).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(dir) = std::path::Path::new(path).parent() {
            let _ = open::that(dir);
        }
    }
}

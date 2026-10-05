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
    let dragging = egui::DragAndDrop::payload::<QueueDrag>(ui.ctx()).map(|d| d.0);
    // a drop moves the dragged song to just before `slot`
    let mut drop_at: Option<(usize, usize)> = None;
    let area = ui.available_rect_before_wrap();
    egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("queue").show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in from..to {
            let Some(t) = app.lib.track(&queue[i]) else { continue };
            if i as isize == index + 1 {
                ui.label(egui::RichText::new("  UP NEXT").font(px(5.0)).color(pal.dim));
            }
            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click_and_drag());
            // (not `resp.hovered()`: the play and × buttons take the hover from the row, which made
            // them vanish under the mouse every other frame so clicks never landed)
            let hovered = ui.rect_contains_pointer(r) && ui.ctx().dragged_id().is_none();
            let p = ui.painter();
            let current = i as isize == index;
            let moving = dragging == Some(i);
            if moving {
                fill(p, r, pal.bg2);
            } else if current {
                fill(p, r, pal.sel);
                fill(p, Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), pal.accent);
            } else if hovered && dragging.is_none() {
                fill(p, r, pal.panel_hi);
            }
            let alpha = if moving { 70 } else if (i as isize) < index { 100 } else { 255 };
            // cover: hovering shows a play button, one click plays the song
            let thumb = Rect::from_min_size(r.min + Vec2::new(8.0, 6.0), Vec2::splat(30.0));
            fill(p, thumb, pal.bg);
            if let Some(c) = t.thumb.clone().or(t.cover.clone()) {
                if let Some(tex) = app.covers.get(ui.ctx(), app.lib.cover_path(&c), &c, 64) {
                    ui.painter().image(tex, thumb, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), egui::Color32::from_white_alpha(alpha));
                }
            }
            frame_rect(ui.painter(), thumb, 1.0, pal.line_hi);
            if hovered && dragging.is_none() {
                let presp = ui.interact(thumb, ui.id().with(("qplay", i)), Sense::click());
                let over = presp.hovered();
                fill(ui.painter(), thumb, super::widgets::with_alpha(if over { pal.accent } else { pal.bg }, if over { 235 } else { 170 }));
                ui.painter().text(thumb.center(), Align2::CENTER_CENTER, if current && app.player.status().playing { "⏸" } else { "▶" }, vt(22.0), if over { pal.ink } else { pal.text });
                if over {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if presp.on_hover_text(if current { "Play / pause" } else { "Play" }).clicked() {
                    action = Some(if current { Box::new(|a: &App| a.player.toggle()) } else { Box::new(move |a: &App| a.player.play_index(i)) });
                }
            }
            let clip = ui.painter().with_clip_rect(Rect::from_min_max(r.min, Pos2::new(r.right() - 26.0, r.bottom())));
            clip.text(r.min + Vec2::new(46.0, 5.0), Align2::LEFT_TOP, &t.title, vt(18.0), super::widgets::with_alpha(if current { pal.accent } else { pal.text }, alpha));
            clip.text(r.min + Vec2::new(46.0, 22.0), Align2::LEFT_TOP, &t.artist, vt(16.0), super::widgets::with_alpha(pal.dim, alpha));
            if hovered && dragging.is_none() {
                // grip: drag to move
                let gx = r.right() - 34.0;
                for k in 0..3 {
                    fill(ui.painter(), Rect::from_min_size(Pos2::new(gx - 8.0, r.center().y - 5.0 + k as f32 * 4.0), Vec2::new(8.0, 2.0)), pal.faint);
                }
                let xr = Rect::from_min_size(Pos2::new(r.right() - 24.0, r.top() + 11.0), Vec2::splat(20.0));
                let xresp = ui.interact(xr, ui.id().with(("qx", i)), Sense::click());
                ui.painter().text(xr.center(), Align2::CENTER_CENTER, "×", vt(20.0), if xresp.hovered() { pal.accent } else { pal.dim });
                if xresp.on_hover_text("Remove from queue").clicked() {
                    action = Some(Box::new(move |a: &App| a.player.remove_at(i)));
                }
                if !thumb.contains(ui.ctx().pointer_hover_pos().unwrap_or_default()) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
            if resp.double_clicked() {
                action = Some(Box::new(move |a: &App| a.player.play_index(i)));
            }
            // drag to reorder
            if resp.drag_started() {
                resp.dnd_set_drag_payload(QueueDrag(i));
            }
            if let (Some(from_i), Some(pos)) = (dragging, ui.ctx().pointer_hover_pos()) {
                if r.contains(pos) {
                    let below = pos.y > r.center().y;
                    let slot = if below { i + 1 } else { i };
                    if slot != from_i && slot != from_i + 1 {
                        let y = if below { r.bottom() - 1.0 } else { r.top() + 1.0 };
                        ui.painter().hline(r.x_range(), y, egui::Stroke::new(3.0_f32, pal.accent));
                    }
                    if resp.dnd_release_payload::<QueueDrag>().is_some() {
                        drop_at = Some((from_i, slot));
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
                if ui.add_enabled(i > 0 && (i as isize - 1) > index, egui::Button::new("Move up")).clicked() {
                    action = Some(Box::new(move |a: &App| a.player.move_item(i, i - 1)));
                    ui.close_menu();
                }
                if ui.add_enabled(i + 1 < queue.len() && (i as isize) > index, egui::Button::new("Move down")).clicked() {
                    action = Some(Box::new(move |a: &App| a.player.move_item(i, i + 1)));
                    ui.close_menu();
                }
                if ui.add_enabled(i + 1 < queue.len(), egui::Button::new("Move to the end")).clicked() {
                    let last = queue.len() - 1;
                    action = Some(Box::new(move |a: &App| a.player.move_item(i, last)));
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
        // below the last song: drop here to move it to the end of what's shown
        let h = ui.available_height().max(30.0);
        let (zr, zresp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
        if let Some(from_i) = dragging {
            if zresp.dnd_hover_payload::<QueueDrag>().is_some() && from_i + 1 != to {
                ui.painter().hline(zr.x_range(), zr.top() + 1.0, egui::Stroke::new(3.0_f32, pal.accent));
            }
            if zresp.dnd_release_payload::<QueueDrag>().is_some() {
                drop_at = Some((from_i, to));
            }
        }
        // near the top / bottom edge while dragging: scroll
        if let (Some(_), Some(pos)) = (dragging, ui.ctx().pointer_hover_pos()) {
            let edge = 28.0;
            let dy = if pos.y < area.top() + edge && pos.y > area.top() - 10.0 { 6.0 } else if pos.y > area.bottom() - edge && pos.y < area.bottom() + 10.0 { -6.0 } else { 0.0 };
            if dy != 0.0 && area.x_range().contains(pos.x) {
                ui.scroll_with_delta(Vec2::new(0.0, dy));
                ui.ctx().request_repaint();
            }
        }
    });
    // the song being dragged follows the pointer
    if let (Some(i), Some(pos)) = (dragging, ui.ctx().pointer_hover_pos()) {
        if let Some(t) = queue.get(i).and_then(|id| app.lib.track(id)) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag-queue")));
            let g = p.layout_no_wrap(format!("♪ {}", t.title), vt(19.0), pal.ink);
            let gr = Rect::from_min_size(pos + Vec2::new(14.0, 6.0), g.size() + Vec2::new(14.0, 6.0));
            fill(&p, gr, pal.accent);
            p.galley(gr.min + Vec2::new(7.0, 3.0), g, pal.ink);
        }
    }
    if let Some((from_i, slot)) = drop_at {
        if slot != from_i && slot != from_i + 1 {
            let to_i = if from_i < slot { slot - 1 } else { slot };
            action = Some(Box::new(move |a: &App| a.player.move_item(from_i, to_i)));
        }
    }
    if let Some(a) = action {
        a(app);
    }
}

/// A queue row being dragged (its index).
struct QueueDrag(usize);

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

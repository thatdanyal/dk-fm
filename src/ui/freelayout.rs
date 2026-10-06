//! "Free (per pixel)" layout, the alternative to the template (docked) layout: every open panel
//! is a box at an exact pixel position and size. In layout edit mode (Ctrl+E) drag a panel's title
//! bar to move it and its corner to resize it. The open panels are the same as in the template
//! layout (View menu / Ctrl+K), so switching back and forth keeps them.
//!
//! The deck (play / pause, volume) is always drawn on top, can't go below its compact size and is
//! kept inside the window, so it can never be lost.
use super::theme::{px, Pal};
use super::widgets::{fill, frame_rect};
use super::{App, Tab};
use eframe::egui::{self, Align2, Id, Order, Pos2, Rect, Sense, Vec2};
use std::collections::BTreeMap;

const BAR: f32 = 24.0;
const DECK_MIN: Vec2 = Vec2::new(230.0, 48.0 + BAR);
const MIN: Vec2 = Vec2::new(140.0, 60.0 + BAR);

fn key(t: &Tab) -> String {
    t.name().to_lowercase()
}

fn min_of(t: &Tab) -> Vec2 {
    if *t == Tab::Deck { DECK_MIN } else { MIN }
}

/// Where a panel goes the first time it shows up in free mode: where the template layout last
/// had it, else a default spot.
fn initial(app: &App, tab: &Tab, area: Rect, i: usize) -> Rect {
    let tree = app.dock.main_surface();
    // the template's rects, scaled from the size it was last drawn at to this area
    let root = (!tree.is_empty()).then(|| tree[egui_dock::NodeIndex::root()].rect()).flatten().filter(|r| r.is_positive() && r.is_finite());
    for node in tree.iter() {
        if let (Some(r), Some(tabs), Some(root)) = (node.rect(), node.tabs(), root) {
            let Some(n) = tabs.iter().position(|t| t == tab) else { continue };
            if !(r.is_positive() && r.is_finite()) {
                continue;
            }
            let scale = area.size() / root.size();
            let r = Rect::from_min_size(area.min + (r.min - root.min) * scale, r.size() * scale);
            // panels tabbed together share their pane: one strip each
            let h = r.height() / tabs.len() as f32;
            return Rect::from_min_size(Pos2::new(r.left(), r.top() + h * n as f32), Vec2::new(r.width(), h));
        }
    }
    match tab {
        Tab::Library | Tab::Home => Rect::from_min_max(Pos2::new(area.left() + area.width() * 0.27, area.top()), area.max),
        Tab::Deck => Rect::from_min_size(area.min, Vec2::new(area.width() * 0.27, 300.0)),
        _ => Rect::from_min_size(area.min + Vec2::new(20.0 + 24.0 * i as f32, 320.0 + 24.0 * i as f32), Vec2::new(area.width() * 0.27, 260.0)),
    }
}

/// Keeps a panel inside the area and at least its minimum size.
fn fit(r: Rect, min: Vec2, area: Rect) -> Rect {
    let size = r.size().max(min).min(area.size().max(min));
    let mut pos = r.min.round();
    pos.x = pos.x.clamp(area.left(), (area.right() - size.x).max(area.left()));
    pos.y = pos.y.clamp(area.top(), (area.bottom() - size.y).max(area.top()));
    Rect::from_min_size(pos, size.round())
}

pub fn show(app: &mut App, ctx: &egui::Context, area: Rect) {
    let pal = app.pal;
    let edit = app.layout_edit;
    let mut tabs: Vec<Tab> = app.dock.iter_all_tabs().map(|(_, t)| *t).collect();
    if !tabs.contains(&Tab::Deck) {
        tabs.push(Tab::Deck);
    }
    // the deck last: on top of everything
    tabs.sort_by_key(|t| *t == Tab::Deck);
    let saved: BTreeMap<String, [f32; 4]> = app.settings.lock().free_panels.clone();
    let mut out = saved.clone();
    // positions are kept relative to the area's corner
    let at = |v: [f32; 4]| Rect::from_min_size(area.min + Vec2::new(v[0], v[1]), Vec2::new(v[2], v[3]));
    for (i, tab) in tabs.iter().enumerate() {
        let k = key(tab);
        let mut r = fit(saved.get(&k).map(|v| at(*v)).unwrap_or_else(|| initial(app, tab, area, i)), min_of(tab), area);
        let id = Id::new(("free-panel", k.as_str()));
        let resp = egui::Area::new(id).order(Order::Middle).fixed_pos(r.min).constrain(false).movable(false).show(ctx, |ui| {
            let (frame, _) = ui.allocate_exact_size(r.size(), Sense::hover());
            let bar = Rect::from_min_size(frame.min, Vec2::new(frame.width(), BAR));
            let body = Rect::from_min_max(Pos2::new(frame.left() + 2.0, bar.bottom()), frame.max - Vec2::splat(2.0));
            let p = ui.painter();
            fill(p, frame, pal.panel);
            fill(p, bar, pal.bg2);
            p.text(Pos2::new(bar.left() + 8.0, bar.center().y), Align2::LEFT_CENTER, format!("■ {}", tab.name()), px(7.0), if edit { pal.accent } else { pal.dim });
            frame_rect(p, frame, 2.0, if edit { pal.accent2 } else { pal.line });
            let mut moved = Vec2::ZERO;
            let mut grown = Vec2::ZERO;
            if edit {
                let drag = ui.interact(bar, id.with("move"), Sense::drag());
                if drag.hovered() || drag.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
                moved = drag.drag_delta();
                let corner = Rect::from_min_size(frame.max - Vec2::splat(16.0), Vec2::splat(16.0));
                let grip = ui.interact(corner, id.with("size"), Sense::drag());
                if grip.hovered() || grip.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                }
                grown = grip.drag_delta();
                handle(ui.painter(), &pal, corner);
                // size readout while editing: it's per pixel
                ui.painter().text(Pos2::new(bar.right() - 8.0, bar.center().y), Align2::RIGHT_CENTER, format!("{}×{} @ {},{}", r.width() as i32, r.height() as i32, (r.left() - area.left()) as i32, (r.top() - area.top()) as i32), px(6.0), pal.dim);
            }
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body), |ui| {
                ui.set_clip_rect(body);
                match tab {
                    Tab::Deck => super::deck::show(app, ui),
                    Tab::Scope => super::scope::show(app, ui),
                    Tab::Library => super::browser::show(app, ui),
                    Tab::Home => super::browser::show_home(app, ui),
                    Tab::Queue => super::queue::show(app, ui),
                    Tab::Eq => super::eqpanel::show(app, ui),
                    Tab::Lyrics => super::lyrics::show(app, ui),
                }
            });
            (moved, grown)
        });
        let (moved, grown) = resp.inner;
        if *tab == Tab::Deck {
            ctx.move_to_top(resp.response.layer_id);
        }
        // saved when placed the first time or moved: squeezing into a smaller window doesn't
        // overwrite where you put it
        if moved != Vec2::ZERO || grown != Vec2::ZERO || !saved.contains_key(&k) {
            r = fit(Rect::from_min_size(r.min + moved, r.size() + grown), min_of(tab), area);
            out.insert(k, [r.left() - area.left(), r.top() - area.top(), r.width(), r.height()]);
        }
    }
    if out != saved {
        app.edit_settings(|s| s.free_panels = out);
    }
}

fn handle(p: &egui::Painter, pal: &Pal, r: Rect) {
    for i in 0..3 {
        let o = 4.0 + i as f32 * 4.0;
        p.line_segment([Pos2::new(r.right() - o, r.bottom() - 2.0), Pos2::new(r.right() - 2.0, r.bottom() - o)], egui::Stroke::new(1.5_f32, pal.accent2));
    }
}

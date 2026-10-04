//! Retro widgets + helpers shared by all panels.
use super::theme::{px, vt, Pal};
use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

pub fn fill(p: &egui::Painter, r: Rect, c: Color32) {
    p.rect_filled(r, CornerRadius::ZERO, c);
}
pub fn frame_rect(p: &egui::Painter, r: Rect, w: f32, c: Color32) {
    p.rect_stroke(r, CornerRadius::ZERO, Stroke::new(w, c), StrokeKind::Inside);
}

pub fn fmt_time(s: f64) -> String {
    let s = if s.is_finite() && s > 0.0 { s as u64 } else { 0 };
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m:02}:{sec:02}") }
}

pub fn fmt_long(s: f64) -> String {
    let h = (s / 3600.0) as u64;
    let m = ((s % 3600.0) / 60.0).round() as u64;
    if h > 0 { format!("{h} HR {m} MIN") } else { format!("{m} MIN") }
}

/// Chunky pixel button with a hard drop shadow. `primary` = filled accent.
pub fn button(ui: &mut Ui, pal: &Pal, text: &str, primary: bool, enabled: bool) -> Response {
    let font = px(8.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE);
    let size = Vec2::new(galley.size().x + 20.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    let pressed = resp.is_pointer_button_down_on();
    let r = if pressed { rect.translate(Vec2::splat(2.0)) } else { rect.shrink2(Vec2::new(0.0, 0.0)) };
    let r = Rect::from_min_size(r.min, size - Vec2::splat(2.0));
    if !pressed {
        fill(p, r.translate(Vec2::splat(2.0)), pal.shadow);
    }
    let hovered = resp.hovered() && enabled;
    let (bg, fg, border) = if primary { (pal.accent, pal.ink, pal.accent) } else { (pal.panel_hi, if hovered { pal.accent } else { pal.text }, if hovered { pal.accent } else { pal.line_hi }) };
    fill(p, r, if primary && hovered { lighten(bg) } else { bg });
    frame_rect(p, r, 2.0, border);
    let fg = if enabled { fg } else { pal.dim };
    p.text(r.center(), Align2::CENTER_CENTER, text, font, fg);
    if enabled && hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Small flat text button used in title bar / panel tools.
pub fn tb_button(ui: &mut Ui, pal: &Pal, text: &str, on: bool) -> Response {
    let font = px(7.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 16.0, 24.0), Sense::click());
    let p = ui.painter();
    if on {
        fill(p, rect, pal.accent);
    } else if resp.hovered() {
        frame_rect(p, rect, 2.0, pal.line_hi);
    }
    p.text(rect.center(), Align2::CENTER_CENTER, text, font, if on { pal.ink } else if resp.hovered() { pal.text } else { pal.dim });
    resp
}

/// On/off switch with a pixel label.
pub fn switch(ui: &mut Ui, pal: &Pal, on: &mut bool, label: &str) -> bool {
    let font = px(7.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), pal.text);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(34.0 + galley.size().x, 18.0), Sense::click());
    let p = ui.painter();
    let track = Rect::from_min_size(rect.min + Vec2::new(0.0, 2.0), Vec2::new(28.0, 14.0));
    fill(p, track, pal.bg);
    frame_rect(p, track, 2.0, if *on { pal.accent } else { pal.line_hi });
    let knob = Rect::from_min_size(track.min + Vec2::new(if *on { 16.0 } else { 3.0 }, 3.0), Vec2::splat(8.0));
    fill(p, knob, if *on { pal.accent } else { pal.dim });
    p.text(rect.min + Vec2::new(34.0, 9.0), Align2::LEFT_CENTER, label, font, pal.text);
    if resp.clicked() {
        *on = !*on;
        return true;
    }
    false
}

pub fn lighten(c: Color32) -> Color32 {
    let f = |v: u8| (v as f32 * 1.15).min(255.0) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

pub fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// Pixel-font caption (section titles).
pub fn caption(ui: &mut Ui, pal: &Pal, text: &str) {
    ui.label(egui::RichText::new(text).font(px(7.0)).color(pal.accent2));
}

pub fn dim(ui: &mut Ui, pal: &Pal, text: &str) -> Response {
    ui.label(egui::RichText::new(text).font(vt(17.0)).color(pal.dim))
}

// ------------------------------------------------------------------------------- cover textures

/// Loads cover art on a background thread at the size it's shown, with a small LRU so memory
/// stays bounded no matter how big the library is.
pub struct Covers {
    map: HashMap<(String, u32), TextureHandle>,
    order: VecDeque<(String, u32)>,
    pending: HashSet<(String, u32)>,
    tx: crossbeam_channel::Sender<((String, u32), Option<egui::ColorImage>)>,
    rx: crossbeam_channel::Receiver<((String, u32), Option<egui::ColorImage>)>,
    jobs: Arc<Mutex<VecDeque<((String, u32), PathBuf)>>>,
    wake: Arc<(Mutex<bool>, parking_lot::Condvar)>,
    pub ctx: Option<egui::Context>,
}

impl Covers {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let jobs: Arc<Mutex<VecDeque<((String, u32), PathBuf)>>> = Arc::new(Mutex::new(VecDeque::new()));
        let wake = Arc::new((Mutex::new(false), parking_lot::Condvar::new()));
        let (j, w, t) = (jobs.clone(), wake.clone(), tx.clone());
        std::thread::Builder::new()
            .name("covers".into())
            .spawn(move || loop {
                let next = j.lock().pop_back(); // newest first: what's on screen now
                match next {
                    Some((key, path)) => {
                        let img = image::open(&path).ok().map(|i| {
                            let i = i.thumbnail(key.1, key.1).to_rgba8();
                            egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], &i)
                        });
                        let _ = t.send((key, img));
                    }
                    None => {
                        let mut g = w.0.lock();
                        if !*g {
                            w.1.wait_for(&mut g, std::time::Duration::from_millis(500));
                        }
                        *g = false;
                    }
                }
            })
            .ok();
        Self { map: HashMap::new(), order: VecDeque::new(), pending: HashSet::new(), tx, rx, jobs, wake, ctx: None }
    }

    /// Forget every texture (they load again when shown).
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    /// Texture for a cover file at `size` px (None while loading).
    pub fn get(&mut self, ctx: &egui::Context, path: PathBuf, name: &str, size: u32) -> Option<egui::TextureId> {
        while let Ok((key, img)) = self.rx.try_recv() {
            self.pending.remove(&key);
            if let Some(img) = img {
                let tex = ctx.load_texture(format!("{}@{}", key.0, key.1), img, egui::TextureOptions::LINEAR);
                self.map.insert(key.clone(), tex);
                self.order.push_back(key);
                let cap = 260;
                while self.order.len() > cap {
                    if let Some(old) = self.order.pop_front() {
                        self.map.remove(&old);
                    }
                }
            }
        }
        let key = (name.to_string(), size);
        if let Some(t) = self.map.get(&key) {
            return Some(t.id());
        }
        if self.pending.insert(key.clone()) {
            let _ = &self.tx;
            self.jobs.lock().push_back((key, path));
            *self.wake.0.lock() = true;
            self.wake.1.notify_one();
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }
        None
    }
}

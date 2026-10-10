//! Colour picker for the theme editor: a colour wheel (colour around the edge, paler towards the
//! middle) and a brightness bar for the mouse, red / green / blue from 0 to 255 to type, and a box
//! that takes a pasted colour (#ff3b2f, ff3b2f, f32, "255, 59, 47" or rgb(255, 59, 47)).
use super::theme::{px, Pal};
use super::widgets::{fill, frame_rect};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};

/// Hue, saturation, value (0..1 each) of an sRGB colour.
fn hsv(c: Color32) -> (f32, f32, f32) {
    let (r, g, b) = (c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

fn from_hsv(h: f32, s: f32, v: f32) -> Color32 {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let u = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(u(r), u(g), u(b))
}

/// A typed or pasted colour: hex (6 or 3 digits, # optional) or three numbers 0-255.
pub fn parse(s: &str) -> Option<Color32> {
    let t = s.trim().trim_start_matches("rgb").trim_start_matches("RGB").trim_matches(|c| c == '(' || c == ')').trim();
    let nums: Vec<&str> = t.split(|c: char| c == ',' || c.is_whitespace()).filter(|x| !x.is_empty()).collect();
    if nums.len() == 3 && nums.iter().all(|n| n.chars().all(|c| c.is_ascii_digit())) {
        let v: Vec<u8> = nums.iter().filter_map(|n| n.parse::<u32>().ok().map(|x| x.min(255) as u8)).collect();
        return (v.len() == 3).then(|| Color32::from_rgb(v[0], v[1], v[2]));
    }
    let h = t.trim_start_matches('#');
    match h.len() {
        3 if h.chars().all(|c| c.is_ascii_hexdigit()) => {
            let d: Vec<u8> = h.chars().map(|c| c.to_digit(16).unwrap_or(0) as u8 * 17).collect();
            Some(Color32::from_rgb(d[0], d[1], d[2]))
        }
        6 => super::theme::parse_hex(h),
        _ => None,
    }
}

/// Wheel + brightness + R G B + paste box for `c`. `text` is the paste box's contents (kept
/// between frames; it follows the colour unless you're typing in it). True when `c` changed.
pub fn picker(ui: &mut Ui, pal: &Pal, id: &str, c: &mut Color32, text: &mut String) -> bool {
    let before = *c;
    let (mut h, mut s, mut v) = hsv(*c);
    // remember the colour around the wheel for greys and black (they have none of their own)
    let mem = egui::Id::new(("colorpick-hue", id));
    if s < 0.01 || v < 0.01 {
        h = ui.ctx().data(|d| d.get_temp(mem)).unwrap_or(h);
    }
    ui.horizontal(|ui| {
        // the wheel
        let side = 150.0;
        let (r, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click_and_drag());
        let ctr = r.center();
        let rad = side / 2.0 - 2.0;
        let mut mesh = egui::Mesh::default();
        // (drawn at full brightness so dark colours still show their wheel; the bar sets brightness)
        mesh.colored_vertex(ctr, Color32::WHITE);
        const N: u32 = 72;
        for i in 0..=N {
            let a = i as f32 / N as f32;
            let ang = a * std::f32::consts::TAU;
            mesh.colored_vertex(ctr + Vec2::new(ang.cos(), -ang.sin()) * rad, from_hsv(a, 1.0, 1.0));
            if i > 0 {
                mesh.add_triangle(0, i, i + 1);
            }
        }
        ui.painter().add(mesh);
        ui.painter().circle_stroke(ctr, rad, Stroke::new(2.0_f32, pal.line_hi));
        let mut moved = false;
        if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.is_pointer_button_down_on() || resp.clicked()) {
            moved = true;
            let d = p - ctr;
            h = (-d.y).atan2(d.x).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
            s = (d.length() / rad).clamp(0.0, 1.0);
            if v < 0.05 {
                v = 1.0; // picking a colour for black: light it up
            }
        }
        let ang = h * std::f32::consts::TAU;
        let at = ctr + Vec2::new(ang.cos(), -ang.sin()) * rad * s;
        ui.painter().circle_stroke(at, 6.0, Stroke::new(2.0_f32, Color32::BLACK));
        ui.painter().circle_stroke(at, 4.0, Stroke::new(2.0_f32, Color32::WHITE));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        // brightness: a bar from black to the full colour
        let (br, bresp) = ui.allocate_exact_size(Vec2::new(22.0, side), Sense::click_and_drag());
        let mut m = egui::Mesh::default();
        let top = from_hsv(h, s, 1.0);
        m.colored_vertex(br.left_top(), top);
        m.colored_vertex(br.right_top(), top);
        m.colored_vertex(br.right_bottom(), Color32::BLACK);
        m.colored_vertex(br.left_bottom(), Color32::BLACK);
        m.add_triangle(0, 1, 2);
        m.add_triangle(0, 2, 3);
        ui.painter().add(m);
        frame_rect(ui.painter(), br, 2.0, pal.line_hi);
        if let Some(p) = bresp.interact_pointer_pos().filter(|_| bresp.is_pointer_button_down_on() || bresp.clicked()) {
            moved = true;
            v = (1.0 - (p.y - br.top()) / br.height()).clamp(0.0, 1.0);
        }
        let y = br.bottom() - v * br.height();
        fill(ui.painter(), Rect::from_min_max(Pos2::new(br.left() - 3.0, y - 2.0), Pos2::new(br.right() + 3.0, y + 2.0)), pal.text);
        if bresp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        ui.add_space(8.0);
        if moved {
            let picked = from_hsv(h, s, v);
            *c = Color32::from_rgba_unmultiplied(picked.r(), picked.g(), picked.b(), c.a());
        }
        // numbers and the paste box
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let big = Rect::from_min_size(ui.cursor().min, Vec2::new(120.0, 34.0));
            ui.allocate_rect(big, Sense::hover());
            fill(ui.painter(), big, *c);
            frame_rect(ui.painter(), big, 2.0, pal.line_hi);
            let [mut r8, mut g8, mut b8, a] = c.to_srgba_unmultiplied();
            let mut typed = false;
            for (label, val, col) in [("R", &mut r8, Color32::from_rgb(230, 70, 70)), ("G", &mut g8, Color32::from_rgb(70, 200, 90)), ("B", &mut b8, Color32::from_rgb(80, 130, 240))] {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label).font(px(7.0)).color(col));
                    typed |= ui.add(egui::DragValue::new(val).range(0..=255).speed(1.0)).changed();
                    let mut f = *val as f32;
                    ui.spacing_mut().slider_width = 110.0;
                    if ui.add(egui::Slider::new(&mut f, 0.0..=255.0).show_value(false)).changed() {
                        *val = f.round() as u8;
                        typed = true;
                    }
                });
            }
            if typed {
                *c = Color32::from_rgba_unmultiplied(r8, g8, b8, a);
            }
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("PASTE").font(px(6.0)).color(pal.dim));
                let te = ui.add(egui::TextEdit::singleline(text).desired_width(150.0).hint_text("#ff3b2f or 255, 59, 47")).on_hover_text("Type or paste a colour: #ff3b2f, ff3b2f, or red, green, blue from 0 to 255 like 255, 59, 47");
                if te.changed() {
                    if let Some(p) = parse(text) {
                        *c = Color32::from_rgba_unmultiplied(p.r(), p.g(), p.b(), c.a());
                    }
                } else if !te.has_focus() {
                    let [r, g, b, _] = c.to_srgba_unmultiplied();
                    *text = format!("#{r:02x}{g:02x}{b:02x}");
                }
            });
        });
    });
    ui.ctx().data_mut(|d| d.insert_temp(mem, h));
    *c != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_colours() {
        assert_eq!(parse("#ff3b2f"), Some(Color32::from_rgb(255, 59, 47)));
        assert_eq!(parse("ff3b2f"), Some(Color32::from_rgb(255, 59, 47)));
        assert_eq!(parse("f00"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(parse("255, 59, 47"), Some(Color32::from_rgb(255, 59, 47)));
        assert_eq!(parse("rgb(255,59,47)"), Some(Color32::from_rgb(255, 59, 47)));
        assert_eq!(parse("255 59 47"), Some(Color32::from_rgb(255, 59, 47)));
        assert_eq!(parse("300, 0, 0"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(parse("nope"), None);
    }

    #[test]
    fn hsv_round_trip() {
        for c in [Color32::from_rgb(255, 59, 47), Color32::from_rgb(12, 200, 99), Color32::from_rgb(0, 0, 0), Color32::from_rgb(128, 128, 128)] {
            let (h, s, v) = hsv(c);
            assert_eq!(from_hsv(h, s, v), c);
        }
    }
}

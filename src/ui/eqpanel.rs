//! 10-band EQ with live response curve and presets, plus crossfade and leveler.
use super::theme::{px, vt};
use super::widgets::{fill, frame_rect, switch, tb_button};
use super::App;
use crate::audio::{eq_curve, EQ_FREQS};
use eframe::egui::{self, Align2, Pos2, Rect, Sense, Ui, Vec2};

pub const PRESETS: &[(&str, [f32; 10])] = &[
    ("Flat", [0.0; 10]),
    ("Bass Boost", [7.0, 6.0, 4.5, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Sub Rumble", [9.0, 6.0, 2.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.5, 4.5, 6.0, 7.0]),
    ("V-Shape", [6.0, 4.5, 2.0, 0.0, -2.0, -2.0, 0.0, 2.0, 4.5, 6.0]),
    ("Vocal", [-2.0, -2.0, -1.0, 1.0, 3.5, 4.0, 3.0, 1.5, 0.0, -1.0]),
    ("Rock", [5.0, 3.5, 2.0, -1.0, -2.0, -1.0, 1.5, 3.0, 4.0, 4.5]),
    ("Electronic", [5.5, 4.5, 1.0, 0.0, -2.0, 1.5, 0.5, 1.0, 4.5, 5.0]),
    ("Hip-Hop", [6.0, 5.0, 1.5, 3.0, -1.0, -1.0, 1.0, -0.5, 1.5, 3.0]),
    ("Jazz", [3.5, 2.5, 1.0, 2.0, -1.5, -1.5, 0.0, 1.0, 2.5, 3.5]),
    ("Classical", [4.0, 3.0, 2.5, 2.0, -1.0, -1.0, 0.0, 2.0, 3.0, 3.5]),
    ("Acoustic", [4.0, 4.0, 3.0, 1.0, 2.0, 1.5, 3.0, 3.5, 3.0, 2.0]),
    ("Late Night", [-3.0, -2.0, 0.0, 1.5, 2.0, 2.0, 1.5, 0.0, -2.0, -3.5]),
    ("Loudness", [6.0, 4.0, 0.0, 0.0, -1.5, 0.0, -1.0, -4.0, 4.0, 2.0]),
];

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let mut eq = app.player.st.lock().eq.clone();
    let mut changed = false;
    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            changed |= switch(ui, &pal, &mut eq.enabled, "EQ");
            egui::ComboBox::from_id_salt("eq-preset").selected_text(eq.preset.clone()).width(150.0).show_ui(ui, |ui| {
                for (name, g) in PRESETS {
                    if ui.selectable_label(eq.preset == *name, *name).clicked() {
                        eq.preset = name.to_string();
                        eq.gains = g.to_vec();
                        eq.preamp = -(g.iter().cloned().fold(0.0, f32::max) - 2.0).max(0.0); // keep boosted presets from clipping
                        eq.enabled = true;
                        changed = true;
                    }
                }
            });
            if tb_button(ui, &pal, "RESET", false).clicked() {
                eq.preset = "Flat".into();
                eq.gains = vec![0.0; 10];
                eq.preamp = 0.0;
                changed = true;
            }
        });
        // response curve
        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 60.0), Sense::hover());
        fill(ui.painter(), r, pal.lcd_bg);
        frame_rect(ui.painter(), r, 2.0, pal.line_hi);
        let n = (r.width() / 2.0) as usize;
        let freqs: Vec<f32> = (0..n).map(|i| 20.0 * (20000.0f32 / 20.0).powf(i as f32 / n as f32)).collect();
        let mut g = [0f32; 10];
        for (i, v) in eq.gains.iter().take(10).enumerate() {
            g[i] = *v;
        }
        let curve = if eq.enabled { eq_curve(&g, eq.preamp, &freqs) } else { vec![0.0; n] };
        let mid = r.center().y;
        let mut prev: Option<f32> = None;
        for (i, db) in curve.iter().enumerate() {
            let y = (mid - (db / 15.0) * (r.height() / 2.0 - 3.0)).clamp(r.top() + 2.0, r.bottom() - 2.0);
            let p0 = prev.unwrap_or(y);
            let x = r.left() + i as f32 * 2.0;
            fill(ui.painter(), Rect::from_min_max(Pos2::new(x, p0.min(y)), Pos2::new(x + 2.0, p0.max(y) + 2.0)), pal.accent);
            prev = Some(y);
        }
        // sliders
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = (ui.available_height() - 50.0).clamp(70.0, 160.0);
            let col = |ui: &mut Ui, label: &str, v: &mut f32| -> bool {
                let mut c = false;
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(format!("{}{:.0}", if *v > 0.0 { "+" } else { "" }, v)).font(vt(15.0)).color(pal.accent2));
                    let r = ui.add(egui::Slider::new(v, -12.0..=12.0).vertical().show_value(false).step_by(0.5));
                    if r.double_clicked() {
                        *v = 0.0;
                        c = true;
                    }
                    c |= r.changed();
                    ui.label(egui::RichText::new(label).font(vt(15.0)).color(pal.dim));
                });
                c
            };
            changed |= col(ui, "PRE", &mut eq.preamp);
            ui.separator();
            for (i, f) in EQ_FREQS.iter().enumerate() {
                let lbl = if *f >= 1000.0 { format!("{}K", f / 1000.0) } else { format!("{f}") };
                if col(ui, &lbl, &mut eq.gains[i]) {
                    changed = true;
                    eq.preset = "Custom".into();
                    eq.enabled = true;
                }
            }
        });
        ui.add_space(6.0);
        let mut opts = app.player.st.lock().opts.clone();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("X-FADE").font(px(6.0)).color(pal.text));
            ui.spacing_mut().slider_width = 90.0;
            if ui.add(egui::Slider::new(&mut opts.crossfade, 0..=12).show_value(false)).changed() {
                app.player.set_crossfade(opts.crossfade);
            }
            ui.label(egui::RichText::new(format!("{}s", opts.crossfade)).color(pal.accent2));
            ui.add_space(10.0);
            if switch(ui, &pal, &mut opts.normalize, "LEVELER") {
                app.player.set_normalize(opts.normalize);
            }
        });
        let _ = Align2::CENTER_CENTER;
    });
    if changed {
        app.player.set_eq(eq);
    }
}

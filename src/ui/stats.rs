//! Stats ("Wrapped" whenever you want) from the local listening history.
use super::theme::{px, vt};
use super::widgets::{button, fill, frame_rect};
use super::App;
use crate::library::main_artist;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Ui, Vec2};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct StatsState {
    period: usize,
}

const PERIODS: [(&str, i64); 4] = [("THIS WEEK", 7), ("THIS MONTH", 30), ("THIS YEAR", 365), ("ALL TIME", i64::MAX)];

fn local(t: i64) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(t, 0).single().unwrap_or_else(chrono::Local::now)
}

/// Local calendar day number (for streaks).
fn day_key(t: i64) -> i64 {
    use chrono::Datelike;
    local(t).num_days_from_ce() as i64
}

fn local_hour(t: i64) -> usize {
    use chrono::Timelike;
    local(t).hour() as usize
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let now = crate::store::now_secs();
    let days = PERIODS[app.stats.period].1;
    let from = if days == i64::MAX { 0 } else { now - days * 86400 };
    let all: Vec<(String, i64, i64, u8)> = app.lib.history.read().events.clone();
    let rows: Vec<&(String, i64, i64, u8)> = all.iter().filter(|r| r.1 >= from && app.lib.data.read().tracks.contains_key(&r.0)).collect();
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("YOUR STATS").font(px(12.0)).color(pal.text));
                ui.label(egui::RichText::new("UPDATED LIVE · STORED ONLY ON THIS PC").color(pal.dim));
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (i, (label, _)) in PERIODS.iter().enumerate() {
                if button(ui, &pal, label, i == app.stats.period, true).clicked() {
                    app.stats.period = i;
                }
            }
        });
    });
    if rows.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("NO LISTENING YET").font(px(9.0)).color(pal.accent));
            ui.label(egui::RichText::new("Play some music — your stats build up as you listen.").color(pal.dim));
        });
        return;
    }
    let d = app.lib.data.read();
    let secs: i64 = rows.iter().map(|r| r.2).sum();
    let plays = rows.iter().filter(|r| r.3 == 0).count();
    let mut songs: HashMap<&str, (u32, i64)> = HashMap::new();
    let mut artists: HashMap<String, (i64, Option<String>)> = HashMap::new();
    let mut skips: HashMap<&str, u32> = HashMap::new();
    let mut hours = [0i64; 24];
    for r in &rows {
        let t = &d.tracks[&r.0];
        let e = songs.entry(r.0.as_str()).or_default();
        if r.3 == 0 { e.0 += 1; }
        e.1 += r.2;
        let a = artists.entry(main_artist(&t.artist)).or_default();
        a.0 += r.2;
        if a.1.is_none() { a.1 = t.thumb.clone().or(t.cover.clone()); }
        if r.3 == 1 { *skips.entry(r.0.as_str()).or_default() += 1; }
        hours[local_hour(r.1)] += r.2;
    }
    let mut top: Vec<(&str, u32, i64)> = songs.iter().map(|(k, v)| (*k, v.0, v.1)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    top.truncate(10);
    let mut tart: Vec<(String, i64, Option<String>)> = artists.into_iter().map(|(k, v)| (k, v.0, v.1)).collect();
    tart.sort_by(|a, b| b.1.cmp(&a.1));
    tart.truncate(8);
    let mut sk: Vec<(&str, u32)> = skips.into_iter().collect();
    sk.sort_by(|a, b| b.1.cmp(&a.1));
    sk.truncate(5);
    let dayset: HashSet<i64> = all.iter().map(|r| day_key(r.1)).collect();
    let mut streak = 0;
    let mut day = day_key(now);
    if !dayset.contains(&day) { day -= 1; }
    while dayset.contains(&day) { streak += 1; day -= 1; }
    let peak = hours.iter().enumerate().max_by_key(|x| x.1).map(|x| x.0).unwrap_or(0);
    let fmt_h = |h: usize| format!("{}{}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" });
    let top_ids: Vec<String> = top.iter().map(|t| t.0.to_string()).collect();
    let tracks: HashMap<String, crate::store::Track> = top.iter().filter_map(|t| d.tracks.get(t.0).map(|x| (t.0.to_string(), x.clone()))).chain(sk.iter().filter_map(|t| d.tracks.get(t.0).map(|x| (t.0.to_string(), x.clone())))).collect();
    drop(d);
    let mut play: Option<usize> = None;
    egui::ScrollArea::vertical().id_salt("stats").auto_shrink([false; 2]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
            // summary cards
            let cards = [
                (format!("{}", secs / 60), "MINUTES LISTENED"),
                (plays.to_string(), "PLAYS"),
                (songs.len().to_string(), "DIFFERENT SONGS"),
                (tart.len().max(0).to_string(), "TOP ARTISTS"),
                (format!("{streak} DAY{}", if streak == 1 { "" } else { "S" }), "LISTENING STREAK"),
                (fmt_h(peak), "YOUR PEAK HOUR"),
            ];
            let w = ui.available_width();
            let n = ((w + 10.0) / 160.0).floor().max(1.0) as usize;
            let cw = (w - 10.0 * (n as f32 - 1.0)) / n as f32;
            for row in cards.chunks(n) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    for (big, small) in row {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(cw, 66.0), Sense::hover());
                        fill(ui.painter(), r.translate(Vec2::splat(3.0)), pal.shadow);
                        fill(ui.painter(), r, pal.lcd_bg);
                        frame_rect(ui.painter(), r, 2.0, pal.line_hi);
                        ui.painter().text(r.min + Vec2::new(12.0, 8.0), Align2::LEFT_TOP, big, vt(34.0), pal.lcd);
                        ui.painter().text(r.min + Vec2::new(12.0, 48.0), Align2::LEFT_TOP, *small, px(6.0), pal.dim);
                    }
                });
                ui.add_space(10.0);
            }
            let section = |ui: &mut Ui, title: &str, f: &mut dyn FnMut(&mut Ui)| {
                egui::Frame::new().fill(pal.bg2).stroke(egui::Stroke::new(2.0_f32, pal.line)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new(title).font(px(7.0)).color(pal.accent2));
                    ui.add_space(6.0);
                    f(ui);
                });
                ui.add_space(12.0);
            };
            section(ui, "TOP SONGS", &mut |ui| {
                for (i, (id, p, s)) in top.iter().enumerate() {
                    let Some(t) = tracks.get(*id) else { continue };
                    let r = ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{:>2}", i + 1)).color(pal.accent));
                        ui.label(egui::RichText::new(&t.title).color(pal.text));
                        ui.label(egui::RichText::new(&t.artist).color(pal.dim));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(if *p > 0 { format!("{p} PLAY{}", if *p == 1 { "" } else { "S" }) } else { format!("{} MIN", (s / 60).max(1)) }).color(pal.dim));
                        });
                    });
                    if r.response.interact(Sense::click()).double_clicked() {
                        play = Some(i);
                    }
                }
                ui.add_space(6.0);
                if button(ui, &pal, "▶ PLAY TOP SONGS", false, true).clicked() {
                    play = Some(0);
                }
            });
            section(ui, "TOP ARTISTS", &mut |ui| {
                let max = tart.first().map(|a| a.1).unwrap_or(1).max(1);
                for (i, (name, s, _)) in tart.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{:>2}", i + 1)).color(pal.accent));
                        ui.label(egui::RichText::new(name).color(pal.text));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(format!("{} MIN", s / 60)).color(pal.dim));
                            let (r, _) = ui.allocate_exact_size(Vec2::new(120.0, 8.0), Sense::hover());
                            frame_rect(ui.painter(), r, 1.0, pal.line_hi);
                            fill(ui.painter(), Rect::from_min_size(r.min, Vec2::new(120.0 * *s as f32 / max as f32, 8.0)), pal.accent);
                        });
                    });
                }
            });
            section(ui, "WHEN YOU LISTEN", &mut |ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 110.0), Sense::hover());
                let maxh = *hours.iter().max().unwrap_or(&1).max(&1) as f32;
                let bw = r.width() / 24.0;
                for (h, v) in hours.iter().enumerate() {
                    let hh = ((*v as f32 / maxh) * (r.height() - 18.0)).max(2.0);
                    let x = r.left() + h as f32 * bw;
                    let mut y = r.bottom() - 16.0;
                    while y > r.bottom() - 16.0 - hh {
                        fill(ui.painter(), Rect::from_min_size(Pos2::new(x + 1.0, y - 3.0), Vec2::new(bw - 2.0, 3.0)), pal.accent);
                        y -= 4.0;
                    }
                    if h % 6 == 0 {
                        ui.painter().text(Pos2::new(x, r.bottom()), Align2::LEFT_BOTTOM, fmt_h(h), vt(14.0), pal.dim);
                    }
                }
            });
            section(ui, "MOST SKIPPED (SMART SHUFFLE PLAYS THESE LATER)", &mut |ui| {
                if sk.is_empty() {
                    ui.label(egui::RichText::new("Nothing skipped — good taste.").color(pal.dim));
                }
                for (id, n) in &sk {
                    if let Some(t) = tracks.get(*id) {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("↻").color(pal.accent));
                            ui.label(egui::RichText::new(&t.title).color(pal.text));
                            ui.label(egui::RichText::new(&t.artist).color(pal.dim));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(format!("{n} SKIP{}", if *n == 1 { "" } else { "S" })).color(pal.dim));
                            });
                        });
                    }
                }
            });
        });
    });
    if let Some(i) = play {
        app.player.play_list(top_ids, i, Some(false));
    }
    let _ = Color32::WHITE;
}

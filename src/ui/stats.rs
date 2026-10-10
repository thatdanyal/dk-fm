//! Stats ("Wrapped" whenever you want) from the local listening history.
use super::theme::{px, vt};
use super::widgets::{button, fill, frame_rect};
use super::App;
use crate::library::main_artist;
use eframe::egui::{self, Align2, Pos2, Rect, Sense, Ui, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub struct StatsState {
    period: usize,
    /// what's on screen, worked out once per (period, history length, library change). Building
    /// it every frame re-shuffled songs with equal counts (HashMap order), so the lists jumped
    /// around whenever the mouse moved.
    cache: Option<((usize, usize, u64, u64), Arc<Computed>)>,
    /// karaoke.json's last change, looked at every few seconds (not every frame)
    karaoke_seen: Option<(std::time::Instant, u64)>,
}

#[derive(Default)]
struct Computed {
    secs: i64,
    plays: usize,
    songs: usize,
    /// (id, plays, seconds)
    top: Vec<(String, u32, i64)>,
    /// (artist, seconds)
    artists: Vec<(String, i64)>,
    skipped: Vec<(String, u32)>,
    hours: [i64; 24],
    streak: usize,
    /// sing-along scores: your best per song (id, score), highest first
    karaoke: Vec<(String, u32)>,
    tracks: HashMap<String, crate::store::Track>,
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

fn compute(app: &App, period: usize) -> Computed {
    let now = crate::store::now_secs();
    let days = PERIODS[period].1;
    let from = if days == i64::MAX { 0 } else { now - days * 86400 };
    let d = app.lib.data.read();
    let h = app.lib.history.read();
    let rows: Vec<&(String, i64, i64, u8)> = h.events.iter().filter(|r| r.1 >= from && d.tracks.contains_key(&r.0)).collect();
    let mut c = Computed { secs: rows.iter().map(|r| r.2).sum(), plays: rows.iter().filter(|r| r.3 == 0).count(), ..Default::default() };
    let mut songs: HashMap<&str, (u32, i64)> = HashMap::new();
    let mut artists: HashMap<String, i64> = HashMap::new();
    let mut skips: HashMap<&str, u32> = HashMap::new();
    for r in &rows {
        let t = &d.tracks[&r.0];
        let e = songs.entry(r.0.as_str()).or_default();
        if r.3 == 0 {
            e.0 += 1;
        }
        e.1 += r.2;
        *artists.entry(main_artist(&t.artist)).or_default() += r.2;
        if r.3 == 1 {
            *skips.entry(r.0.as_str()).or_default() += 1;
        }
        c.hours[local_hour(r.1)] += r.2;
    }
    c.songs = songs.len();
    // ties are broken by name, so the order never changes between frames
    let title = |id: &str| d.tracks.get(id).map(|t| t.title.to_lowercase()).unwrap_or_default();
    let mut top: Vec<(String, u32, i64)> = songs.iter().map(|(k, v)| (k.to_string(), v.0, v.1)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then_with(|| title(&a.0).cmp(&title(&b.0))).then(a.0.cmp(&b.0)));
    top.truncate(10);
    let mut tart: Vec<(String, i64)> = artists.into_iter().collect();
    tart.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    tart.truncate(8);
    let mut sk: Vec<(String, u32)> = skips.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    sk.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| title(&a.0).cmp(&title(&b.0))).then(a.0.cmp(&b.0)));
    sk.truncate(5);
    let dayset: HashSet<i64> = h.events.iter().map(|r| day_key(r.1)).collect();
    let mut day = day_key(now);
    if !dayset.contains(&day) {
        day -= 1;
    }
    while dayset.contains(&day) {
        c.streak += 1;
        day -= 1;
    }
    // karaoke bests are all-time (a score isn't tied to a week)
    c.karaoke = crate::karaoke::bests().into_iter().filter(|k| d.tracks.contains_key(&k.0)).take(10).collect();
    c.tracks = top.iter().map(|t| &t.0).chain(sk.iter().map(|t| &t.0)).chain(c.karaoke.iter().map(|k| &k.0)).filter_map(|id| d.tracks.get(id).map(|x| (id.clone(), x.clone()))).collect();
    (c.top, c.artists, c.skipped) = (top, tart, sk);
    c
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let kar = match app.stats.karaoke_seen {
        Some((at, v)) if at.elapsed().as_secs() < 5 => v,
        _ => {
            let v = std::fs::metadata(crate::store::data_dir().join("karaoke.json")).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            app.stats.karaoke_seen = Some((std::time::Instant::now(), v));
            v
        }
    };
    let key = (app.stats.period, app.lib.history.read().events.len(), app.lib.gen.load(std::sync::atomic::Ordering::Relaxed), kar);
    if app.stats.cache.as_ref().map(|c| c.0 != key).unwrap_or(true) {
        let c = compute(app, app.stats.period);
        app.stats.cache = Some((key, Arc::new(c)));
    }
    let c = app.stats.cache.as_ref().unwrap().1.clone();
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
    if c.top.is_empty() && c.secs == 0 {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("NO LISTENING YET").font(px(9.0)).color(pal.accent));
            ui.label(egui::RichText::new("Play some music — your stats build up as you listen.").color(pal.dim));
        });
        return;
    }
    let (secs, plays, streak, hours) = (c.secs, c.plays, c.streak, c.hours);
    let (top, tart, sk, tracks) = (&c.top, &c.artists, &c.skipped, &c.tracks);
    let peak = hours.iter().enumerate().max_by_key(|x| x.1).map(|x| x.0).unwrap_or(0);
    let fmt_h = |h: usize| format!("{}{}", if h.is_multiple_of(12) { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" });
    let top_ids: Vec<String> = top.iter().map(|t| t.0.clone()).collect();
    let mut play: Option<usize> = None;
    egui::ScrollArea::vertical().id_salt("stats").auto_shrink([false; 2]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
            // summary cards
            let cards = [
                (format!("{}", secs / 60), "MINUTES LISTENED"),
                (plays.to_string(), "PLAYS"),
                (c.songs.to_string(), "DIFFERENT SONGS"),
                (tart.len().to_string(), "TOP ARTISTS"),
                (format!("{streak} DAY{}", if streak == 1 { "" } else { "S" }), "LISTENING STREAK"),
                (fmt_h(peak), "YOUR PEAK HOUR"),
                (c.karaoke.first().map(|k| k.1.to_string()).unwrap_or_else(|| "--".into()), "KARAOKE HIGH SCORE"),
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
                    let Some(t) = tracks.get(id) else { continue };
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
                for (i, (name, s)) in tart.iter().enumerate() {
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
            section(ui, "KARAOKE HIGH SCORES (ALL TIME)", &mut |ui| {
                if c.karaoke.is_empty() {
                    ui.label(egui::RichText::new("No scores yet: open THEATER on a song with synced lyrics and press ♪ SCORE ME.").color(pal.dim));
                }
                for (i, (id, score)) in c.karaoke.iter().enumerate() {
                    let Some(t) = tracks.get(id) else { continue };
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(if i == 0 { "★".to_string() } else { format!("{:>2}", i + 1) }).color(pal.accent));
                        ui.label(egui::RichText::new(&t.title).color(pal.text));
                        ui.label(egui::RichText::new(&t.artist).color(pal.dim));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(format!("{score} · {}", crate::karaoke::grade(*score))).color(if i == 0 { pal.accent2 } else { pal.dim }));
                        });
                    });
                }
            });
            section(ui, "MOST SKIPPED (SMART SHUFFLE PLAYS THESE LATER)", &mut |ui| {
                if sk.is_empty() {
                    ui.label(egui::RichText::new("Nothing skipped — good taste.").color(pal.dim));
                }
                for (id, n) in sk.iter() {
                    if let Some(t) = tracks.get(id) {
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
        app.player.play_pick(top_ids, i, Some(false));
    }
}

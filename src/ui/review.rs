//! YEAR IN REVIEW (Stats > ★ YEAR IN REVIEW): your year on one card, "Wrapped" style, from the
//! listening history on this PC: minutes, your #1 song with its cover, top songs and artists,
//! your busiest month, longest streak, what time of day you listen, new songs found, podcast
//! hours and your best karaoke score. SAVE AS PICTURE writes the card to a PNG to share.
use super::theme::{px, vt};
use super::widgets::{button, fill, frame_rect, outline_button};
use super::App;
use crate::library::main_artist;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Ui, Vec2};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct Review {
    pub year: i32,
    card: Option<(i32, u64, Year)>,
    /// SAVE AS PICTURE: frames the card has been shown over the whole window (the picture is
    /// taken of that, so a card taller than its panel still comes out whole)
    saving: Option<u32>,
}

#[derive(Default, Clone)]
struct Year {
    secs: i64,
    plays: usize,
    songs: usize,
    /// (id, plays)
    top: Vec<(String, u32)>,
    /// (artist, seconds)
    artists: Vec<(String, i64)>,
    months: [i64; 12],
    streak: usize,
    hours: [i64; 24],
    added: usize,
    podcast_secs: f64,
    karaoke: Option<(String, u32)>,
    tracks: HashMap<String, crate::store::Track>,
}

fn local(t: i64) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(t, 0).single().unwrap_or_else(chrono::Local::now)
}

/// The years there's listening history for, newest first (this year always).
pub fn years(app: &App) -> Vec<i32> {
    use chrono::Datelike;
    let mut y: Vec<i32> = app.lib.history.read().events.iter().map(|r| local(r.1).year()).collect::<HashSet<_>>().into_iter().collect();
    let now = chrono::Local::now().year();
    if !y.contains(&now) {
        y.push(now);
    }
    y.sort_by(|a, b| b.cmp(a));
    y
}

fn compute(app: &App, year: i32) -> Year {
    use chrono::{Datelike, Timelike};
    let d = app.lib.data.read();
    let h = app.lib.history.read();
    let rows: Vec<&(String, i64, i64, u8)> = h.events.iter().filter(|r| local(r.1).year() == year && d.tracks.contains_key(&r.0)).collect();
    let mut y = Year { secs: rows.iter().map(|r| r.2).sum(), plays: rows.iter().filter(|r| r.3 == 0).count(), ..Default::default() };
    let mut songs: HashMap<&str, (u32, i64)> = HashMap::new();
    let mut artists: HashMap<String, i64> = HashMap::new();
    let mut days: HashSet<i64> = HashSet::new();
    for r in &rows {
        let t = local(r.1);
        let e = songs.entry(r.0.as_str()).or_default();
        if r.3 == 0 {
            e.0 += 1;
        }
        e.1 += r.2;
        *artists.entry(main_artist(&d.tracks[&r.0].artist)).or_default() += r.2;
        y.months[t.month0() as usize] += r.2;
        y.hours[t.hour() as usize] += r.2;
        days.insert(t.num_days_from_ce() as i64);
    }
    y.songs = songs.len();
    let title = |id: &str| d.tracks.get(id).map(|t| t.title.to_lowercase()).unwrap_or_default();
    let mut top: Vec<(String, u32, i64)> = songs.iter().map(|(k, v)| (k.to_string(), v.0, v.1)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then_with(|| title(&a.0).cmp(&title(&b.0))));
    y.top = top.into_iter().take(5).map(|t| (t.0, t.1)).collect();
    let mut a: Vec<(String, i64)> = artists.into_iter().collect();
    a.sort_by(|x, z| z.1.cmp(&x.1).then_with(|| x.0.to_lowercase().cmp(&z.0.to_lowercase())));
    a.truncate(5);
    y.artists = a;
    // the longest run of days in a row with music
    let mut sorted: Vec<i64> = days.into_iter().collect();
    sorted.sort();
    let mut run = 0;
    for (i, day) in sorted.iter().enumerate() {
        run = if i > 0 && sorted[i - 1] == day - 1 { run + 1 } else { 1 };
        y.streak = y.streak.max(run);
    }
    y.added = d.tracks.values().filter(|t| t.added_at > 0.0 && local((t.added_at / 1000.0) as i64).year() == year).count();
    y.podcast_secs = crate::podcasts::Podcasts::load().listened.get(&year).copied().unwrap_or(0.0);
    y.karaoke = crate::karaoke::bests().into_iter().find(|k| d.tracks.contains_key(&k.0));
    y.tracks = y.top.iter().map(|t| &t.0).chain(y.karaoke.iter().map(|k| &k.0)).filter_map(|id| d.tracks.get(id).map(|t| (id.clone(), t.clone()))).collect();
    y
}

/// What time of day you listen most, as a name.
fn listener(hours: &[i64; 24]) -> (&'static str, &'static str) {
    let part = |r: std::ops::Range<usize>| r.map(|h| hours[h]).sum::<i64>();
    let parts = [(part(5..12), "EARLY BIRD", "mornings are yours"), (part(12..18), "DAYTIME LISTENER", "music all afternoon"), (part(18..23), "EVENING REGULAR", "the night starts with music"), (part(23..24) + part(0..5), "NIGHT OWL", "you listen while others sleep")];
    let best = parts.iter().max_by_key(|p| p.0).unwrap();
    (best.1, best.2)
}

const MONTHS: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

/// The YEAR IN REVIEW screen (in place of Stats while it's open). False when it's closed.
pub fn show(app: &mut App, ui: &mut Ui) -> bool {
    let pal = app.pal;
    let years = years(app);
    if !years.contains(&app.review.year) {
        app.review.year = years[0];
    }
    let year = app.review.year;
    let gen = app.lib.history.read().events.len() as u64 ^ (app.lib.gen.load(std::sync::atomic::Ordering::Relaxed) << 20);
    if app.review.card.as_ref().is_none_or(|c| c.0 != year || c.1 != gen) {
        let y = compute(app, year);
        app.review.card = Some((year, gen, y));
    }
    let y = app.review.card.as_ref().unwrap().2.clone();
    save_shot(app, ui);
    let mut open = true;
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if button(ui, &pal, "◀ STATS", false, true).clicked() {
                open = false;
            }
            for yr in years.iter().take(6) {
                if outline_button(ui, &pal, &yr.to_string(), *yr == year).clicked() {
                    app.review.year = *yr;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, &pal, "SAVE AS PICTURE", true, y.secs > 0 && app.review.saving.is_none()).on_hover_text("Save the card as a PNG to share").clicked() {
                    app.review.saving = Some(0);
                }
            });
        });
    });
    egui::ScrollArea::vertical().id_salt("review").auto_shrink([false; 2]).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            let w = ui.available_width().min(560.0);
            card(app, ui, &y, year, w);
        });
        ui.add_space(20.0);
    });
    // saving: the card over everything for a moment, then the picture
    if let Some(n) = app.review.saving {
        let ctx = ui.ctx().clone();
        let screen = ctx.screen_rect();
        let dim = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("review-dim")));
        fill(&dim, screen, pal.bg);
        let w = 560.0f32.min(screen.width() - 20.0);
        let inner = egui::Area::new(egui::Id::new("review-save")).fade_in(false).order(egui::Order::Tooltip).fixed_pos(Pos2::new(screen.center().x - w / 2.0, screen.top() + 8.0)).show(&ctx, |ui| card(app, ui, &y, year, w)).inner;
        if n == 4 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(ReviewShot(inner))));
        }
        // (no picture came back: give up)
        app.review.saving = if n > 90 { None } else { Some(n + 1) };
        ctx.request_repaint();
    }
    // (dev hook: DKFM_REVIEW_SAVE=<png> saves the card there once, without asking)
    static DEV: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if std::env::var("DKFM_REVIEW_SAVE").is_ok() && !DEV.swap(true, std::sync::atomic::Ordering::Relaxed) {
        app.review.saving = Some(0);
    }
    open
}

struct ReviewShot(Rect);

/// SAVE AS PICTURE: the screenshot came back; the card is cut out of it and saved.
fn save_shot(app: &mut App, ui: &mut Ui) {
    let shot = ui.ctx().input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { user_data, image, .. } => user_data.data.as_ref().and_then(|d| d.downcast_ref::<ReviewShot>()).map(|r| (r.0, image.clone())),
            _ => None,
        })
    });
    let Some((r, img)) = shot else { return };
    app.review.saving = None;
    let ppp = ui.ctx().pixels_per_point();
    let cut = img.region(&r, Some(ppp));
    let name = format!("DK.FM {} in review.png", app.review.year);
    let dev = std::env::var("DKFM_REVIEW_SAVE").ok().map(std::path::PathBuf::from);
    let Some(path) = dev.or_else(|| rfd::FileDialog::new().set_title("Save your year in review").set_file_name(&name).add_filter("PNG picture", &["png"]).save_file()) else { return };
    let px: Vec<u8> = cut.pixels.iter().flat_map(|c| [c.r(), c.g(), c.b(), 255]).collect();
    match image::RgbaImage::from_raw(cut.size[0] as u32, cut.size[1] as u32, px).map(|b| b.save(&path)) {
        Some(Ok(())) => app.toast("Saved: share it anywhere"),
        _ => app.toast_err("Couldn't save the picture"),
    }
}

/// The card itself. Returns where it is on screen.
fn card(app: &mut App, ui: &mut Ui, y: &Year, year: i32, w: f32) -> Rect {
    let pal = app.pal;
    let res = egui::Frame::new().fill(pal.bg2).stroke(egui::Stroke::new(2.0_f32, pal.accent2)).inner_margin(egui::Margin::same(22)).show(ui, |ui| {
        ui.set_width(w - 44.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("DK.FM").font(px(8.0)).color(pal.accent2));
            ui.label(egui::RichText::new(format!("{year} IN REVIEW")).font(px(14.0)).color(pal.text));
        });
        ui.add_space(14.0);
        if y.secs == 0 {
            ui.vertical_centered(|ui| ui.label(egui::RichText::new("No listening this year yet: play some music and come back.").font(vt(20.0)).color(pal.dim)));
            return;
        }
        // minutes
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(format!("{}", y.secs / 60)).font(vt(78.0)).color(pal.accent));
            ui.label(egui::RichText::new("MINUTES OF MUSIC").font(px(7.0)).color(pal.dim));
            ui.label(egui::RichText::new(format!("{} plays · {} different songs · {} new songs found", y.plays, y.songs, y.added)).font(vt(18.0)).color(pal.text));
        });
        ui.add_space(16.0);
        // your #1 song, big
        if let Some((id, n)) = y.top.first() {
            if let Some(t) = y.tracks.get(id) {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(120.0), Sense::hover());
                    fill(ui.painter(), r, pal.lcd_bg);
                    if let Some(tex) = super::preview::cover_tex(app, ui.ctx(), t, 256) {
                        ui.painter().image(tex, r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                    frame_rect(ui.painter(), r, 2.0, pal.accent2);
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.add_space(14.0);
                        ui.label(egui::RichText::new("YOUR SONG OF THE YEAR").font(px(6.0)).color(pal.accent2));
                        ui.add(egui::Label::new(egui::RichText::new(&t.title).font(vt(30.0)).color(pal.text)).wrap());
                        ui.label(egui::RichText::new(&t.artist).font(vt(20.0)).color(pal.dim));
                        ui.label(egui::RichText::new(format!("played {n} time{}", if *n == 1 { "" } else { "s" })).font(vt(17.0)).color(pal.accent));
                    });
                });
            }
        }
        ui.add_space(14.0);
        // top songs | top artists
        ui.columns(2, |cols| {
            cols[0].label(egui::RichText::new("TOP SONGS").font(px(6.0)).color(pal.accent2));
            for (i, (id, _)) in y.top.iter().enumerate() {
                if let Some(t) = y.tracks.get(id) {
                    cols[0].add(egui::Label::new(egui::RichText::new(format!("{}  {}", i + 1, t.title)).font(vt(18.0)).color(pal.text)).truncate());
                }
            }
            cols[1].label(egui::RichText::new("TOP ARTISTS").font(px(6.0)).color(pal.accent2));
            for (i, (a, s)) in y.artists.iter().enumerate() {
                cols[1].add(egui::Label::new(egui::RichText::new(format!("{}  {a} · {} min", i + 1, s / 60)).font(vt(18.0)).color(pal.text)).truncate());
            }
        });
        ui.add_space(14.0);
        // the year month by month
        ui.label(egui::RichText::new("YOUR YEAR").font(px(6.0)).color(pal.accent2));
        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 90.0), Sense::hover());
        let maxm = *y.months.iter().max().unwrap_or(&1).max(&1) as f32;
        let best = y.months.iter().enumerate().max_by_key(|m| m.1).map(|m| m.0).unwrap_or(0);
        let bw = r.width() / 12.0;
        for (m, v) in y.months.iter().enumerate() {
            let hh = (*v as f32 / maxm * (r.height() - 18.0)).max(2.0);
            let b = Rect::from_min_max(Pos2::new(r.left() + m as f32 * bw + 3.0, r.bottom() - 16.0 - hh), Pos2::new(r.left() + (m + 1) as f32 * bw - 3.0, r.bottom() - 16.0));
            fill(ui.painter(), b, if m == best { pal.accent } else { pal.accent2 });
            ui.painter().text(Pos2::new(b.center().x, r.bottom()), Align2::CENTER_BOTTOM, MONTHS[m], vt(13.0), if m == best { pal.text } else { pal.dim });
        }
        ui.add_space(12.0);
        // little facts
        let (who, why) = listener(&y.hours);
        let facts = [
            (format!("{} DAYS", y.streak), "LONGEST STREAK".to_string()),
            (MONTHS[best].to_string(), "BUSIEST MONTH".to_string()),
            (who.to_string(), why.to_string()),
            (match y.podcast_secs {
                s if s >= 3600.0 => format!("{:.0} H", s / 3600.0),
                s if s >= 60.0 => format!("{:.0} MIN", s / 60.0),
                _ => "--".into(),
            }, "OF PODCASTS".to_string()),
            (y.karaoke.as_ref().map(|k| k.1.to_string()).unwrap_or_else(|| "--".into()), "BEST KARAOKE SCORE".to_string()),
        ];
        let fw = (ui.available_width() - 8.0) / 2.0;
        for row in facts.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (big, small) in row {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(fw, 58.0), Sense::hover());
                    fill(ui.painter(), r, pal.lcd_bg);
                    frame_rect(ui.painter(), r, 1.5, pal.line_hi);
                    ui.painter().text(r.min + Vec2::new(10.0, 6.0), Align2::LEFT_TOP, big, vt(28.0), pal.lcd);
                    ui.painter().text(r.min + Vec2::new(10.0, 40.0), Align2::LEFT_TOP, small.to_uppercase(), px(5.0), pal.dim);
                }
            });
            ui.add_space(8.0);
        }
    });
    res.response.rect
}

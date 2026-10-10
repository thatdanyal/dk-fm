//! THEATER (full-screen now playing), your way: three versions of it (presets) you name and
//! arrange yourself. Each is two columns of parts (cover, song, seek bar, controls, lyrics, up
//! next, clock, visualizer, volume, plays) top to bottom; an empty right column makes one wide
//! column. LAYOUT (inside THEATER) adds, removes and moves parts, renames a version and makes one
//! the default. In the title bar, clicking THEATER opens the default; hovering it lists all three.
//! 1 / 2 / 3 switch while it's open; Esc (or F11) leaves. Nothing extra repaints: it redraws on
//! the same playback tick as the deck (the clock asks for one repaint a minute).
use super::deck::tbtn;
use super::theme::{px, vt};
use super::widgets::{button, fill, fmt_time, frame_rect, tb_button, with_alpha};
use super::App;
use crate::store::TheaterPreset;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Ui, Vec2};

/// Parts a THEATER can show: (key, name, what it is).
pub const PARTS: [(&str, &str, &str); 10] = [
    ("cover", "COVER", "The album cover, big"),
    ("info", "SONG", "Title, artist and album"),
    ("seek", "SEEK BAR", "The waveform with the time: click or drag to jump"),
    ("controls", "CONTROLS", "Shuffle, back, play, next, repeat and like"),
    ("lyrics", "LYRICS", "Synced lyrics (translated when they're in another language)"),
    ("next", "UP NEXT", "The next songs in the queue: click one to play it"),
    ("clock", "CLOCK", "The time and the date"),
    ("visualizer", "VISUALIZER", "Bars or scope (click it to change the style)"),
    ("volume", "VOLUME", "Mute and a volume slider"),
    ("stats", "PLAYS", "How often you've played it, and when you got it"),
];

fn part_name(k: &str) -> &'static str {
    PARTS.iter().find(|p| p.0 == k).map(|p| p.1).unwrap_or("?")
}

/// Sing-along scoring while it's on (the microphone is listening).
pub struct Score {
    listener: crate::karaoke::Listener,
    /// the song being scored, and its first line not graded yet
    song: String,
    next: usize,
    last_pos: f64,
    grades: Vec<u32>,
    /// your best for this song before now
    best: Option<u32>,
    /// the last line's grade (score, when)
    last: Option<(u32, std::time::Instant)>,
    /// the song's final score: (score, your best before it, when)
    card: Option<(u32, Option<u32>, std::time::Instant)>,
}

impl Score {
    pub fn start(app: &App) -> Self {
        Score { listener: crate::karaoke::listen(app.player.engine.shared.clone()), song: String::new(), next: 0, last_pos: 0.0, grades: Vec::new(), best: None, last: None, card: None }
    }
    fn total(&self) -> Option<u32> {
        (!self.grades.is_empty()).then(|| (self.grades.iter().sum::<u32>() as f32 / self.grades.len() as f32).round() as u32)
    }
}

/// Sing-along scoring: grades the lines that just ended (called with the synced lyrics drawn).
pub fn score_lines(app: &mut App, id: &str, lines: &[(f64, String)], pos: f64) {
    let Some(s) = app.theater.score.as_mut() else { return };
    let err = s.listener.error.lock().take();
    if let Some(e) = err {
        app.theater.score = None;
        app.toast_err(e);
        return;
    }
    if s.song != id {
        s.song = id.to_string();
        s.best = crate::karaoke::best(id);
        s.next = 0;
        s.grades.clear();
        s.last = None;
        s.listener.ticks.lock().clear();
    }
    // a seek: grade from where the song is now (lines skipped don't count against you)
    if pos < s.last_pos - 2.0 || pos > s.last_pos + 4.0 {
        s.next = lines.iter().position(|l| l.0 >= pos).unwrap_or(lines.len());
    }
    s.last_pos = pos;
    let last_sung = lines.iter().rposition(|l| !l.1.trim().is_empty());
    while s.next < lines.len() {
        let (at, text) = (&lines[s.next].0, &lines[s.next].1);
        if text.trim().is_empty() {
            s.next += 1;
            continue;
        }
        let gap = lines.get(s.next + 1).map(|l| l.0 - at).unwrap_or(8.0);
        let end = at + super::lyrics::sung_for(text, gap);
        if pos < end {
            break;
        }
        let heard: Vec<crate::karaoke::Tick> = s.listener.ticks.lock().iter().filter(|t| t.0 >= *at && t.0 <= end).copied().collect();
        let g = crate::karaoke::line_score(&heard);
        s.grades.push(g);
        s.last = Some((g, std::time::Instant::now()));
        if Some(s.next) == last_sung {
            if let Some(total) = s.total() {
                let before = crate::karaoke::save_best(id, total);
                s.card = Some((total, before, std::time::Instant::now()));
            }
        }
        s.next += 1;
    }
}

/// Over the lyrics while scoring: your score so far, the last line's grade, and the final card.
pub fn score_overlay(app: &mut App, ui: &mut Ui, area: Rect, synced: bool) {
    let pal = app.pal;
    let Some(s) = app.theater.score.as_ref() else { return };
    // (on its own layer, above the lyrics scrolling under it)
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("karaoke-score"))).with_clip_rect(area);
    let p = &p;
    // top right: score so far
    let badge = Rect::from_min_size(Pos2::new(area.right() - 150.0, area.top() + 8.0), Vec2::new(140.0, 46.0));
    fill(p, badge.translate(Vec2::splat(3.0)), pal.shadow);
    fill(p, badge, pal.panel);
    frame_rect(p, badge, 2.0, pal.accent2);
    match (synced, s.total()) {
        (false, _) => {
            p.text(badge.center(), Align2::CENTER_CENTER, "NEEDS SYNCED\nLYRICS", px(6.0), pal.dim);
        }
        (true, None) => {
            p.text(badge.center() - Vec2::new(0.0, 8.0), Align2::CENTER_CENTER, "♪ SCORING", px(7.0), pal.accent2);
            let sub = s.best.map(|b| format!("your best: {b}")).unwrap_or_else(|| "sing along!".into());
            p.text(badge.center() + Vec2::new(0.0, 11.0), Align2::CENTER_CENTER, sub, vt(17.0), pal.dim);
        }
        (true, Some(t)) => {
            p.text(badge.center() - Vec2::new(0.0, 9.0), Align2::CENTER_CENTER, format!("{t}"), vt(30.0), pal.accent2);
            p.text(badge.center() + Vec2::new(0.0, 13.0), Align2::CENTER_CENTER, "SCORE", px(6.0), pal.dim);
        }
    }
    // the line that just ended
    if let Some((g, at)) = s.last {
        let age = at.elapsed().as_secs_f32();
        if age < 1.6 {
            let a = (255.0 * (1.0 - (age / 1.6).powi(2))) as u8;
            let c = if g >= 65 { pal.accent } else if g >= 40 { pal.accent2 } else { pal.dim };
            p.text(Pos2::new(badge.center().x, badge.bottom() + 20.0), Align2::CENTER_CENTER, crate::karaoke::grade(g), px(8.0), with_alpha(c, a));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
        }
    }
    // the song's final score
    if let Some((total, before, at)) = s.card {
        if at.elapsed().as_secs() < 10 {
            let r = Rect::from_center_size(area.center(), Vec2::new(380.0_f32.min(area.width() - 20.0), 170.0));
            fill(p, r.translate(Vec2::splat(5.0)), pal.shadow);
            fill(p, r, pal.panel);
            frame_rect(p, r, 3.0, pal.accent);
            p.text(Pos2::new(r.center().x, r.top() + 24.0), Align2::CENTER_CENTER, "YOUR SCORE", px(8.0), pal.dim);
            p.text(Pos2::new(r.center().x, r.top() + 70.0), Align2::CENTER_CENTER, format!("{total}"), vt(64.0), pal.accent);
            p.text(Pos2::new(r.center().x, r.top() + 112.0), Align2::CENTER_CENTER, crate::karaoke::grade(total), px(9.0), pal.accent2);
            let best = match before {
                Some(b) if total > b => format!("NEW BEST! (was {b})"),
                Some(b) => format!("YOUR BEST: {b}"),
                None => "FIRST TIME SINGING THIS ONE".into(),
            };
            p.text(Pos2::new(r.center().x, r.top() + 146.0), Align2::CENTER_CENTER, best, vt(19.0), pal.text);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

/// THEATER's state between frames.
#[derive(Default)]
pub struct TheaterUi {
    /// the version showing (0..3)
    pub preset: usize,
    /// LAYOUT (the editor) is open
    pub edit: bool,
    /// the mouse last moved: (where, when) (for hiding the buttons when it rests)
    idle: (Pos2, f64),
    /// sing-along scoring (the microphone is on while this is Some)
    pub score: Option<Score>,
}

impl App {
    /// Opens THEATER: version `preset`, or your default.
    pub fn open_theater(&mut self, preset: Option<usize>) {
        if self.mini {
            return;
        }
        self.theater.preset = preset.unwrap_or_else(|| self.settings.lock().theater_default).min(2);
        self.theater.edit = false;
        self.nowplaying = true;
    }
}

/// The THEATER title-bar button's menu: hovering it lists your three versions (★ = default).
/// Returns the one clicked, if any (Some(None): edit layouts).
pub fn hover_menu(app: &mut App, ui: &mut Ui, button: &egui::Response) -> Option<Option<usize>> {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    let id = egui::Id::new("theater-hover-menu");
    // open while the button or the menu itself is under the mouse
    let last: Option<Rect> = ctx.data(|d| d.get_temp(id));
    let pos = ctx.pointer_hover_pos();
    let over_menu = matches!((last, pos), (Some(r), Some(p)) if r.expand(6.0).contains(p));
    if !button.hovered() && !over_menu {
        ctx.data_mut(|d| d.remove::<Rect>(id));
        return None;
    }
    let (presets, def) = { let s = app.settings.lock(); (s.theater.clone(), s.theater_default) };
    let mut picked = None;
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(button.rect.left_bottom() + Vec2::new(0.0, 2.0)).show(&ctx, |ui| {
        egui::Frame::new().fill(pal.panel).stroke(egui::Stroke::new(2.0_f32, pal.accent)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.set_min_width(220.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, p) in presets.iter().enumerate() {
                let open = app.nowplaying && app.theater.preset == i;
                let label = format!("{} {}{}", if open { "▶" } else { " " }, p.name, if i == def { "  ★" } else { "" });
                if ui.add_sized(Vec2::new(220.0, 24.0), egui::Button::new(egui::RichText::new(label).font(vt(20.0))).selected(open)).on_hover_text(if i == def { "Your default: clicking THEATER opens this one" } else { "Open this version of THEATER" }).clicked() {
                    picked = Some(Some(i));
                }
            }
            ui.add_space(2.0);
            if ui.add_sized(Vec2::new(220.0, 22.0), egui::Button::new(egui::RichText::new("  LAYOUT… (ADD, MOVE, RENAME)").font(px(6.0)).color(pal.dim))).clicked() {
                picked = Some(None);
            }
        });
    });
    ctx.data_mut(|d| d.insert_temp(id, area.response.rect.union(button.rect)));
    if picked.is_some() {
        ctx.data_mut(|d| d.remove::<Rect>(id));
    }
    picked
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    let busy = app.modal.is_some() || app.palette.is_some() || app.incoming.is_some();
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !busy {
        if app.theater.edit {
            app.theater.edit = false;
        } else {
            app.nowplaying = false;
            return;
        }
    }
    // 1 / 2 / 3: another version
    if !busy && !ctx.wants_keyboard_input() {
        for (k, i) in [(egui::Key::Num1, 0), (egui::Key::Num2, 1), (egui::Key::Num3, 2)] {
            if ui.input(|inp| inp.key_pressed(k) && inp.modifiers.is_none()) {
                app.theater.preset = i;
            }
        }
    }
    let preset = { let s = app.settings.lock(); s.theater.get(app.theater.preset).cloned().unwrap_or_else(|| crate::store::d_theater()[0].clone()) };
    if !preset.singalong {
        app.theater.score = None;
    }
    let full = ui.max_rect();
    backdrop(app, ui, full, &preset);
    // the parts, in one or two columns (room on the right for the editor while it's open)
    let room = if app.theater.edit { Rect::from_min_max(full.min, Pos2::new(full.right() - 340.0, full.bottom())) } else { full };
    let body = Rect::from_min_max(room.min + Vec2::new(24.0, 52.0), room.max - Vec2::new(24.0, 16.0));
    let two = !preset.right.is_empty() && !preset.left.is_empty() && body.width() >= 760.0;
    if two {
        let mid = body.center().x;
        column(app, ui, Rect::from_min_max(body.min, Pos2::new(mid - 12.0, body.bottom())), &preset.left, &preset);
        column(app, ui, Rect::from_min_max(Pos2::new(mid + 12.0, body.top()), body.max), &preset.right, &preset);
    } else {
        // narrow window (or one column): everything in one, left's parts first
        let all: Vec<String> = preset.left.iter().chain(preset.right.iter()).cloned().collect();
        column(app, ui, body, &all, &preset);
    }
    // top: which version, LAYOUT and CLOSE (they rest out of sight when "hide when idle" is on)
    let now = ctx.input(|i| i.time);
    let pointer = ctx.pointer_hover_pos().unwrap_or_default();
    if pointer != app.theater.idle.0 {
        app.theater.idle = (pointer, now);
    }
    let resting = preset.auto_hide && !app.theater.edit && now - app.theater.idle.1 > 3.0;
    if resting {
        ctx.set_cursor_icon(egui::CursorIcon::None);
    } else {
        if preset.auto_hide && !app.theater.edit {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((3.05 - (now - app.theater.idle.1)).max(0.05)));
        }
        let bar = Rect::from_min_max(Pos2::new(full.left() + 16.0, full.top() + 14.0), Pos2::new(room.right() - 16.0, full.top() + 42.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.label(egui::RichText::new(&preset.name).font(px(7.0)).color(pal.dim)).on_hover_text("Press 1, 2 or 3 for your other versions of THEATER");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if tb_button(ui, &pal, "× CLOSE · ESC", false).on_hover_text("Back to DK.FM (Esc or F11)").clicked() {
                    app.nowplaying = false;
                }
                if tb_button(ui, &pal, "LAYOUT", app.theater.edit).on_hover_text("Add, remove and move what THEATER shows; name your three versions and pick the default").clicked() {
                    app.theater.edit = !app.theater.edit;
                }
                if preset.singalong && preset.left.iter().chain(&preset.right).any(|k| k == "lyrics") {
                    let on = app.theater.score.is_some();
                    if tb_button(ui, &pal, if on { "♪ SCORING · STOP" } else { "♪ SCORE ME" }, on).on_hover_text("Sing along and get a score: the microphone listens, and each line is graded on timing and on singing notes that fit the song. Headphones give a fair score (with speakers the microphone hears the song too). Nothing is recorded or saved but your best score.").clicked() {
                        app.theater.score = if on { None } else { Some(Score::start(app)) };
                    }
                }
            });
        });
    }
    if app.theater.edit {
        editor(app, ui, Rect::from_min_max(Pos2::new(full.right() - 330.0, full.top() + 48.0), full.max - Vec2::new(10.0, 10.0)));
    }
}

/// Behind everything: the theme's background, a glow of the cover's colours, or the cover itself.
fn backdrop(app: &mut App, ui: &mut Ui, r: Rect, preset: &TheaterPreset) {
    let pal = app.pal;
    let p = ui.painter();
    fill(p, r, pal.bg);
    match preset.backdrop.as_str() {
        "glow" => {
            // the accents (the cover's colours with the Album Cover theme) fading down the screen
            let mut mesh = egui::Mesh::default();
            let (a, b) = (with_alpha(pal.accent, 70), with_alpha(pal.accent2, 40));
            let clear = with_alpha(pal.bg, 0);
            mesh.colored_vertex(r.left_top(), a);
            mesh.colored_vertex(r.right_top(), b);
            mesh.colored_vertex(Pos2::new(r.right(), r.center().y + r.height() * 0.2), clear);
            mesh.colored_vertex(Pos2::new(r.left(), r.center().y + r.height() * 0.2), clear);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            p.add(mesh);
        }
        "cover" => {
            let t = app.current_track();
            if let Some(tex) = t.as_ref().and_then(|t| super::preview::cover_tex(app, ui.ctx(), t, 512)) {
                // the cover filling the screen (cropped, not stretched), dimmed well behind the text
                let side = r.width().max(r.height());
                let uv_w = r.width() / side;
                let uv_h = r.height() / side;
                let uv = Rect::from_min_max(Pos2::new(0.5 - uv_w / 2.0, 0.5 - uv_h / 2.0), Pos2::new(0.5 + uv_w / 2.0, 0.5 + uv_h / 2.0));
                ui.painter().image(tex, r, uv, Color32::WHITE);
                fill(ui.painter(), r, with_alpha(pal.bg, 214));
            }
        }
        _ => {}
    }
}

/// How tall a part wants to be in a column `w` wide (None: it takes a share of what's left).
fn fixed_height(k: &str) -> Option<f32> {
    Some(match k {
        "info" => 100.0,
        "seek" => 74.0,
        "controls" => 58.0,
        "next" => 214.0,
        "clock" => 104.0,
        "volume" => 40.0,
        "stats" => 30.0,
        _ => return None,
    })
}

/// One column: fixed parts get their height, the cover the biggest square that fits, lyrics and
/// the visualizer share the rest. A column with nothing to stretch sits in the middle.
fn column(app: &mut App, ui: &mut Ui, r: Rect, parts: &[String], preset: &TheaterPreset) {
    if parts.is_empty() {
        return;
    }
    const GAP: f32 = 12.0;
    let fixed: f32 = parts.iter().filter_map(|k| fixed_height(k)).sum::<f32>() + GAP * (parts.len() as f32 - 1.0);
    let flex: Vec<&String> = parts.iter().filter(|k| fixed_height(k).is_none() && *k != "cover").collect();
    let mut left = (r.height() - fixed).max(0.0);
    let cover = if parts.iter().any(|k| k == "cover") {
        let side = (r.width() - 40.0).min(left - flex.len() as f32 * 140.0).clamp(100.0, 640.0);
        left = (left - side).max(0.0);
        side
    } else {
        0.0
    };
    let weight = |k: &str| if k == "lyrics" { 2.0 } else { 1.0 };
    let wsum: f32 = flex.iter().map(|k| weight(k)).sum();
    let used = fixed + cover + if flex.is_empty() { 0.0 } else { left };
    let mut y = r.top() + if flex.is_empty() { ((r.height() - used) / 2.0).max(0.0) } else { 0.0 };
    for k in parts {
        let h = match k.as_str() {
            "cover" => cover,
            k => fixed_height(k).unwrap_or_else(|| left * weight(k) / wsum.max(1.0)),
        };
        let pr = Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(r.width(), h));
        part(app, ui, pr, k, preset);
        y += h + GAP;
    }
}

fn part(app: &mut App, ui: &mut Ui, r: Rect, k: &str, preset: &TheaterPreset) {
    let pal = app.pal;
    let t = app.current_track();
    let st = app.player.status();
    match k {
        "cover" => {
            let side = r.width().min(r.height());
            let art = Rect::from_center_size(r.center(), Vec2::splat(side));
            fill(ui.painter(), art.translate(Vec2::splat(6.0)), pal.shadow);
            fill(ui.painter(), art, pal.bg2);
            match t.as_ref().and_then(|t| super::preview::cover_tex(app, ui.ctx(), t, 512)) {
                Some(id) => {
                    ui.painter().image(id, art, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    ui.painter().text(art.center(), Align2::CENTER_CENTER, "♫", vt(side * 0.4), pal.faint);
                }
            }
            frame_rect(ui.painter(), art, 2.0, pal.line_hi);
        }
        "info" => {
            let p = ui.painter().with_clip_rect(r);
            match &t {
                Some(t) => {
                    p.text(Pos2::new(r.center().x, r.top() + 22.0), Align2::CENTER_CENTER, &t.title, vt(36.0), pal.text);
                    p.text(Pos2::new(r.center().x, r.top() + 56.0), Align2::CENTER_CENTER, &t.artist, vt(25.0), pal.accent2);
                    let al = [t.album.clone(), t.year.map(|y| y.to_string()).unwrap_or_default()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                    p.text(Pos2::new(r.center().x, r.top() + 82.0), Align2::CENTER_CENTER, al, vt(19.0), pal.dim);
                }
                None => {
                    p.text(r.center(), Align2::CENTER_CENTER, "NOTHING PLAYING", px(12.0), pal.dim);
                }
            }
        }
        "seek" => {
            let w = r.width().min(620.0);
            let wr = Rect::from_min_size(Pos2::new(r.center().x - w / 2.0, r.top() + 4.0), Vec2::new(w, 44.0));
            let resp = ui.interact(wr, ui.id().with("theater-seek"), Sense::click_and_drag());
            super::deck::waveform(app, ui, wr, &resp, &st);
            let dur = if st.duration > 0.0 { st.duration } else { t.as_ref().map(|t| t.duration).unwrap_or(0.0) };
            let ty = wr.bottom() + 14.0;
            ui.painter().text(Pos2::new(wr.left(), ty), Align2::LEFT_CENTER, fmt_time(st.position), vt(19.0), pal.dim);
            ui.painter().text(Pos2::new(wr.right(), ty), Align2::RIGHT_CENTER, fmt_time(dur), vt(19.0), pal.dim);
        }
        "controls" => {
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                let total = 46.0 * 4.0 + 62.0 + 46.0 + 8.0 * 5.0 + 20.0;
                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                let opts = app.player.st.lock().opts.clone();
                if tbtn(ui, &pal, "SHUF", Vec2::new(46.0, 30.0), opts.shuffle, false).clicked() {
                    app.player.toggle_shuffle();
                }
                if tbtn(ui, &pal, "⏮", Vec2::new(46.0, 40.0), false, false).clicked() {
                    app.player.prev();
                }
                if tbtn(ui, &pal, if st.playing { "⏸" } else { "▶" }, Vec2::new(62.0, 50.0), false, true).clicked() {
                    app.player.toggle();
                }
                if tbtn(ui, &pal, "⏭", Vec2::new(46.0, 40.0), false, false).clicked() {
                    app.player.next(true);
                }
                if super::deck::repeat_btn(ui, &pal, Vec2::new(46.0, 30.0), &opts.repeat).clicked() {
                    app.player.cycle_repeat();
                }
                let liked = t.as_ref().map(|t| app.lib.stat(&t.id).liked).unwrap_or(false);
                if tbtn(ui, &pal, "♥", Vec2::new(46.0, 30.0), liked, false).clicked() {
                    if let Some(t) = &t {
                        app.lib.toggle_like(&t.id);
                    }
                }
            });
        }
        "lyrics" => {
            fill(ui.painter(), r, with_alpha(pal.bg2, 200));
            frame_rect(ui.painter(), r, 2.0, pal.line);
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(r.shrink(4.0)), |ui| {
                ui.set_clip_rect(r.shrink(2.0));
                super::lyrics::show_sized(app, ui, preset.lyrics_size, preset.singalong);
            });
        }
        "visualizer" => {
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(r), |ui| super::scope::show(app, ui));
        }
        "next" => up_next(app, ui, r),
        "clock" => {
            let now = chrono::Local::now();
            let p = ui.painter();
            p.text(Pos2::new(r.center().x, r.top() + 40.0), Align2::CENTER_CENTER, now.format("%-I:%M").to_string(), vt(76.0), pal.text);
            p.text(Pos2::new(r.center().x, r.top() + 88.0), Align2::CENTER_CENTER, now.format("%A, %B %-d").to_string().to_uppercase(), px(8.0), pal.dim);
            // the next minute
            let secs = 60 - now.timestamp() % 60;
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(secs.max(1) as u64));
        }
        "volume" => {
            let w = r.width().min(420.0);
            let vr = Rect::from_center_size(r.center(), Vec2::new(w, 30.0));
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(vr).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| super::deck::volume(app, ui, 60.0));
        }
        "stats" => {
            let txt = t.as_ref().map(|t| {
                let s = app.lib.stat(&t.id);
                let plays = match s.plays { 0 => "NEVER PLAYED BEFORE".to_string(), 1 => "PLAYED ONCE".to_string(), n => format!("PLAYED {n} TIMES") };
                let added = if t.added_at > 0.0 { format!(" · YOURS SINCE {}", crate::store::local_stamp((t.added_at / 1000.0) as i64, false).to_uppercase()) } else { String::new() };
                format!("{plays}{added}{}", if s.liked { " · ♥" } else { "" })
            });
            if let Some(txt) = txt {
                ui.painter().with_clip_rect(r).text(r.center(), Align2::CENTER_CENTER, txt, px(7.0), pal.dim);
            }
        }
        _ => {}
    }
}

/// The next few songs in the queue; click one to play it.
fn up_next(app: &mut App, ui: &mut Ui, r: Rect) {
    let pal = app.pal;
    let (queue, index) = { let s = app.player.st.lock(); (s.queue.clone(), s.index) };
    ui.painter().text(Pos2::new(r.left() + 4.0, r.top() + 10.0), Align2::LEFT_CENTER, "UP NEXT", px(7.0), pal.dim);
    let start = (index + 1).max(0) as usize;
    let rows = ((r.height() - 24.0) / 38.0).floor().max(0.0) as usize;
    let mut play = None;
    for (n, i) in (start..queue.len()).take(rows).enumerate() {
        let Some(t) = app.lib.track(&queue[i]) else { continue };
        let row = Rect::from_min_size(Pos2::new(r.left(), r.top() + 24.0 + n as f32 * 38.0), Vec2::new(r.width(), 34.0));
        let resp = ui.interact(row, ui.id().with(("theater-next", i)), Sense::click());
        if resp.hovered() {
            fill(ui.painter(), row, pal.panel_hi);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let art = Rect::from_min_size(row.min + Vec2::new(4.0, 2.0), Vec2::splat(30.0));
        fill(ui.painter(), art, pal.bg2);
        if let Some(tex) = super::preview::cover_tex(app, ui.ctx(), &t, 64) {
            ui.painter().image(tex, art, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        let p = ui.painter().with_clip_rect(row);
        p.text(Pos2::new(art.right() + 10.0, row.top() + 9.0), Align2::LEFT_CENTER, &t.title, vt(20.0), pal.text);
        p.text(Pos2::new(art.right() + 10.0, row.top() + 25.0), Align2::LEFT_CENTER, &t.artist, vt(16.0), pal.dim);
        if resp.clicked() {
            play = Some(i);
        }
    }
    if start >= queue.len() {
        ui.painter().text(Pos2::new(r.left() + 4.0, r.top() + 40.0), Align2::LEFT_CENTER, "Nothing after this song", vt(18.0), pal.faint);
    }
    if let Some(i) = play {
        app.player.play_index(i);
    }
}

/// LAYOUT: pick a version, rename it, make it the default, add / remove / move its parts, the
/// background, the lyrics size and hiding the buttons. Changes show right away.
fn editor(app: &mut App, ui: &mut Ui, r: Rect) {
    let pal = app.pal;
    fill(ui.painter(), r.translate(Vec2::splat(4.0)), pal.shadow);
    fill(ui.painter(), r, pal.panel);
    frame_rect(ui.painter(), r, 2.0, pal.accent);
    let i = app.theater.preset;
    let (mut p, def) = { let s = app.settings.lock(); (s.theater[i].clone(), s.theater_default) };
    let before = p.clone();
    let mut make_default = false;
    let mut reset = false;
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(r.shrink(10.0)), |ui| {
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(egui::RichText::new("THEATER LAYOUT").font(px(9.0)).color(pal.text));
            ui.label(egui::RichText::new("Your three versions (1, 2, 3 switch while THEATER is open):").font(vt(17.0)).color(pal.dim));
            ui.horizontal(|ui| {
                let names: Vec<String> = app.settings.lock().theater.iter().map(|x| x.name.clone()).collect();
                for (n, name) in names.iter().enumerate() {
                    let label = format!("{}{}", name, if n == def { " ★" } else { "" });
                    if button(ui, &pal, &label, n == i, true).clicked() {
                        app.theater.preset = n;
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("NAME").font(px(6.0)).color(pal.dim));
                let mut name = p.name.clone();
                if ui.add(egui::TextEdit::singleline(&mut name).desired_width(170.0).char_limit(20).font(vt(19.0))).changed() {
                    p.name = name.to_uppercase();
                }
            });
            if i == def {
                ui.label(egui::RichText::new("★ YOUR DEFAULT: clicking THEATER (or F11) opens this one").font(vt(17.0)).color(pal.accent2));
            } else if button(ui, &pal, "★ MAKE THIS THE DEFAULT", false, true).on_hover_text("Clicking THEATER (or F11) opens this one").clicked() {
                make_default = true;
            }
            ui.add_space(6.0);
            for side in [0usize, 1] {
                ui.label(egui::RichText::new(if side == 0 { "LEFT COLUMN (TOP TO BOTTOM)" } else { "RIGHT COLUMN (EMPTY = ONE WIDE COLUMN)" }).font(px(6.0)).color(pal.text));
                let list = if side == 0 { p.left.clone() } else { p.right.clone() };
                if list.is_empty() {
                    ui.label(egui::RichText::new("  (nothing)").font(vt(17.0)).color(pal.faint));
                }
                for (j, k) in list.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        let small = |ui: &mut Ui, s: &str, on: bool, tip: &str| ui.add_enabled(on, egui::Button::new(egui::RichText::new(s).font(vt(18.0))).min_size(Vec2::new(26.0, 22.0))).on_hover_text(tip).clicked();
                        let col = if side == 0 { &mut p.left } else { &mut p.right };
                        if small(ui, "▲", j > 0, "Move up") {
                            col.swap(j, j - 1);
                        }
                        if small(ui, "▼", j + 1 < list.len(), "Move down") {
                            col.swap(j, j + 1);
                        }
                        let (arrow, tip) = if side == 0 { ("▶", "Move to the right column") } else { ("◀", "Move to the left column") };
                        if small(ui, arrow, true, tip) {
                            let k = col.remove(j);
                            if side == 0 { p.right.push(k) } else { p.left.push(k) }
                            return;
                        }
                        if small(ui, "×", true, "Take it out") {
                            col.remove(j);
                            return;
                        }
                        ui.label(egui::RichText::new(part_name(k)).font(vt(19.0)).color(pal.text)).on_hover_text(PARTS.iter().find(|x| x.0 == k).map(|x| x.2).unwrap_or(""));
                    });
                }
                ui.add_space(4.0);
            }
            let missing: Vec<&(&str, &str, &str)> = PARTS.iter().filter(|x| !p.left.iter().chain(p.right.iter()).any(|k| k == x.0)).collect();
            if !missing.is_empty() {
                ui.label(egui::RichText::new("ADD").font(px(6.0)).color(pal.text));
                ui.horizontal_wrapped(|ui| {
                    for (k, name, tip) in missing {
                        if button(ui, &pal, &format!("+ {name}"), false, true).on_hover_text(*tip).clicked() {
                            // into the shorter column
                            if !p.right.is_empty() && p.right.len() < p.left.len() { p.right.push(k.to_string()) } else { p.left.push(k.to_string()) }
                        }
                    }
                });
            }
            ui.add_space(6.0);
            ui.label(egui::RichText::new("BACKGROUND").font(px(6.0)).color(pal.text));
            ui.horizontal(|ui| {
                for (b, label, tip) in [("plain", "PLAIN", "The theme's background"), ("glow", "GLOW", "A glow of the theme's colours (the cover's, with the Album Cover theme)"), ("cover", "COVER", "The album cover behind everything, dimmed")] {
                    if button(ui, &pal, label, p.backdrop == b || b == "plain" && p.backdrop.is_empty(), true).on_hover_text(tip).clicked() {
                        p.backdrop = b.into();
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("LYRICS SIZE").font(px(6.0)).color(pal.text));
                ui.add(egui::Slider::new(&mut p.lyrics_size, 0.7..=2.6).show_value(false));
            });
            ui.checkbox(&mut p.singalong, egui::RichText::new("Sing-along: words light up as they're sung, with a 3-2-1 countdown before the singing starts").font(vt(18.0)));
            ui.checkbox(&mut p.auto_hide, egui::RichText::new("Hide the buttons and the mouse when it rests (3 s)").font(vt(18.0)));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if button(ui, &pal, "DONE", true, true).clicked() {
                    app.theater.edit = false;
                }
                if button(ui, &pal, "RESET THIS ONE", false, true).on_hover_text("Back to how this version first was").clicked() {
                    reset = true;
                }
            });
            if button(ui, &pal, "COPY SHARE CODE", false, true).on_hover_text("Copy a code for this version to send to a friend: they paste it into DK.FM (Ctrl+V, or Settings > Share codes)").clicked() {
                let sh = super::sharing::theater_share(app, i);
                super::sharing::copy(app, ui.ctx(), &sh, &format!("THEATER \"{}\"", p.name));
            }
        });
    });
    if reset {
        p = crate::store::d_theater()[i].clone();
    }
    if p != before || make_default {
        app.edit_settings(|s| {
            s.theater[i] = p;
            if make_default {
                s.theater_default = i;
            }
        });
    }
}

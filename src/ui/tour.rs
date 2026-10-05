//! The guided tour (offered at the end of the welcome, and from Ctrl+K / the what's-new screen):
//! one feature at a time, lit up on screen with a short explanation. Panels and buttons say
//! where they are with `App::mark` while they're drawn; a step whose part isn't on screen is
//! explained in the middle instead.
use super::theme::{px, vt};
use super::widgets::{button, fill, frame_rect, with_alpha};
use super::App;
use eframe::egui::{self, Align2, Id, LayerId, Order, Pos2, Rect};

/// (what it points at, title, explanation)
pub const STEPS: [(&str, &str, &str); 10] = [
    ("import", "IMPORT YOUR MUSIC", "Paste a link to a Spotify, SoundCloud or YouTube Music playlist, album, song or profile, and DK.FM downloads every song. Connect Spotify in Settings to import your whole library (and keep it in sync)."),
    ("web", "FIND MUSIC", "Search songs, artists, albums, playlists, profiles, podcasts and audiobooks, like on Spotify. ▶ plays a preview first; + GET downloads the version you want."),
    ("shazam", "SHAZAM", "Hear a song in a video, a game or a browser tab? Press SHAZAM and DK.FM names it, then finds it for you."),
    ("discover", "DISCOVER", "New music picked from the artists you play most and the songs you like, minus what you already have."),
    ("library", "SONGS LIKE THIS", "Right-click any song (or click … at the end of its row) > Songs like this: similar songs to listen to first and download if you like them."),
    ("stats", "STATS", "Your listening: top songs, artists and albums, how long you listened and when. Play counts live here, out of the way while you listen."),
    ("scope", "VISUALIZER", "The scope: click it (or press V) to switch between bars, oscilloscope, VU meters and a waterfall. It starts off; pick a style to turn it on."),
    ("eq", "EQUALIZER", "Shape the sound: pick a preset or drag the bands. Turn it on with the EQ switch (it starts flat and off)."),
    ("layout", "LAYOUT", "Move and resize the panels: LAYOUT, then drag a panel's tab or the gaps between panels. RESET puts the default back, and UNDO (Ctrl+Z) takes it back again. STUDY shows just the lyrics and the deck."),
    ("theme", "THEMES", "Change the colours here, or make your own in Settings > Look."),
];

pub fn start(app: &mut App) {
    app.modal = None;
    app.tour = Some(0);
}

/// Draws the current step over everything.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(step) = app.tour else { return };
    let Some((id, title, text)) = STEPS.get(step).copied() else {
        app.tour = None;
        return;
    };
    let pal = app.pal;
    let screen = ctx.screen_rect();
    let target = app.marks.get(id).copied().filter(|r| r.is_positive() && screen.intersects(*r)).map(|r| r.expand(4.0).intersect(screen));
    // dim everything but the part being explained
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("tour-dim")));
    let shade = with_alpha(egui::Color32::BLACK, 150);
    match target {
        Some(t) => {
            fill(&painter, Rect::from_min_max(screen.min, Pos2::new(screen.right(), t.top())), shade);
            fill(&painter, Rect::from_min_max(Pos2::new(screen.left(), t.bottom()), screen.max), shade);
            fill(&painter, Rect::from_min_max(Pos2::new(screen.left(), t.top()), Pos2::new(t.left(), t.bottom())), shade);
            fill(&painter, Rect::from_min_max(Pos2::new(t.right(), t.top()), Pos2::new(screen.right(), t.bottom())), shade);
            frame_rect(&painter, t, 3.0, pal.accent2);
        }
        None => fill(&painter, screen, shade),
    }
    // the explanation: below the part (or above it, or in the middle)
    let w = 360.0;
    let (anchor, pos) = match target {
        Some(t) if t.bottom() + 190.0 < screen.bottom() => (Align2::LEFT_TOP, Pos2::new(t.center().x - w / 2.0, t.bottom() + 12.0)),
        Some(t) if t.top() - 190.0 > screen.top() => (Align2::LEFT_BOTTOM, Pos2::new(t.center().x - w / 2.0, t.top() - 12.0)),
        Some(t) if t.width() > screen.width() * 0.4 => (Align2::CENTER_CENTER, t.center()),
        _ => (Align2::CENTER_CENTER, screen.center()),
    };
    let pos = if anchor == Align2::CENTER_CENTER { pos } else { Pos2::new(pos.x.clamp(screen.left() + 10.0, screen.right() - w - 10.0), pos.y) };
    let (mut next, mut back, mut end) = (false, false, false);
    egui::Area::new(Id::new("tour-card")).order(Order::Tooltip).pivot(anchor).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::new().fill(pal.panel).stroke(egui::Stroke::new(2.0_f32, pal.accent2)).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
            ui.set_width(w - 28.0);
            ui.label(egui::RichText::new(format!("{} / {}", step + 1, STEPS.len())).font(px(6.0)).color(pal.dim));
            ui.label(egui::RichText::new(title).font(px(10.0)).color(pal.accent));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(text).font(vt(20.0)).color(pal.text));
            if target.is_none() && matches!(id, "scope" | "eq") {
                ui.label(egui::RichText::new("(Open it from PANELS in the title bar.)").font(vt(17.0)).color(pal.dim));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if button(ui, &pal, "END TOUR", false, true).clicked() {
                    end = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let last = step + 1 == STEPS.len();
                    if button(ui, &pal, if last { "DONE" } else { "NEXT" }, true, true).clicked() {
                        next = true;
                    }
                    if step > 0 && button(ui, &pal, "BACK", false, true).clicked() {
                        back = true;
                    }
                });
            });
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        end = true;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        next = true;
    }
    app.tour = if end || (next && step + 1 == STEPS.len()) {
        None
    } else if next {
        Some(step + 1)
    } else if back {
        Some(step.saturating_sub(1))
    } else {
        Some(step)
    };
    if app.tour.is_none() {
        app.toast("Tour done. Ctrl+K > \"Tour\" shows it again.");
    }
    // the parts are where they were drawn last frame: keep repainting while it's open
    ctx.request_repaint_after(std::time::Duration::from_millis(200));
}

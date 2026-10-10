//! The guided tour (offered at the end of the welcome, and from Ctrl+K / the what's-new screen):
//! one feature at a time, lit up on screen with a short explanation. Panels and buttons say
//! where they are with `App::mark` while they're drawn (a panel behind another tab is brought to
//! the front for its step); a step whose part isn't on screen is explained in the middle instead.
use super::theme::{px, vt};
use super::widgets::{button, fill, frame_rect, with_alpha};
use super::App;
use eframe::egui::{self, Align2, Id, LayerId, Order, Pos2, Rect};

/// Bump with every update that adds steps: people who update are offered a tour of just the
/// steps newer than the last tour they were offered.
pub const TOUR_VERSION: u32 = 6;

/// (what it points at, title, explanation, the TOUR_VERSION that added it)
pub const STEPS: [(&str, &str, &str, u32); 23] = [
    ("homepanel", "HOME", "Home, Discover, Songs, Albums, Artists, Recent, Top and Stats have their own HOME tab, next to LIBRARY (your playlists).", 2),
    ("web", "SEARCH", "Search all of YouTube Music right here: songs, artists, albums, playlists, profiles, podcasts and audiobooks. ▶ plays a preview first; + GET downloads the version you want.", 2),
    ("import", "IMPORT YOUR MUSIC", "Paste a link to a Spotify, SoundCloud or YouTube Music playlist, album, song or profile, and DK.FM downloads every song. Connect Spotify in Settings to import your whole library (and keep it in sync).", 1),
    ("downloads", "DOWNLOADS", "What's downloading right now. Cancel any time; CLEAR FINISHED tidies the list.", 2),
    ("shazam", "SHAZAM", "Hear a song in a video, a game or a browser tab, or on a radio in the room? Press SHAZAM, pick THIS PC or MICROPHONE, and DK.FM names it, then finds it for you.", 1),
    ("discover", "DISCOVER", "New music picked from the artists you play most and the songs you like, minus what you already have.", 1),
    ("library", "LIBRARY", "Your playlists: the one you played last moves to the top. Drag the line beside them to make the column wider. ⬇ Downloads lists every song you downloaded; select some and press DELETE to remove them.", 2),
    ("library", "SONGS LIKE THIS", "Right-click any song (or click … at the end of its row) > Songs like this: similar songs to listen to first and download if you like them.", 1),
    ("stats", "STATS", "Your listening: top songs, artists and albums, how long you listened and when. Play counts live here, out of the way while you listen.", 1),
    ("scope", "VISUALIZER", "The scope: click it (or press V) to switch between bars, oscilloscope, VU meters and a waterfall. It starts off; pick a style to turn it on.", 1),
    ("eq", "EQUALIZER", "Shape the sound: pick a preset or drag the bands. Turn it on with the EQ switch (it starts flat and off).", 1),
    ("layout", "LAYOUT", "Move and resize the panels: LAYOUT, then drag a panel's tab or the gaps between panels. RESET puts the default back, and UNDO (Ctrl+Z) takes it back again. THEATER shows the cover and the lyrics big.", 1),
    ("theme", "THEMES", "Change the colours here, or make your own in Settings > Look.", 1),
    ("theme", "ALBUM COVER THEME", "THEME > Album Cover: DK.FM's colours follow the cover of the song playing, toned down so they always look good. Settings > Look sets how much.", 3),
    ("homepanel", "MADE FOR YOU", "Home's MADE FOR YOU mixes have your songs and new ones you don't have yet, side by side: double-click one to play it, + GET keeps it.", 4),
    ("theater", "THEATER, YOUR WAY", "Hover THEATER to pick one of your three versions (CLASSIC, KARAOKE, PARTY); a click opens your default. Inside, LAYOUT adds, removes and moves what's shown, renames a version and makes it the default. 1, 2, 3 switch; Esc leaves.", 4),
    ("settings", "LYRICS IN YOUR LANGUAGE", "Lyrics in another language are translated into yours, under each line. Pick your language in Settings > Language.", 4),
    ("theater", "SING ALONG", "THEATER's KARAOKE version sings along: words light up as they're sung, with a 3-2-1 countdown before the singing starts. Turn it on for any version in THEATER > LAYOUT.", 5),
    ("theater", "SCORE YOUR SINGING", "In a sing-along THEATER (KARAOKE), ♪ SCORE ME listens through your microphone and grades every line: PERFECT, GREAT, GOOD. Headphones give a fair score. Your best per song is kept.", 6),
    ("web", "FIND A SONG BY ITS LYRICS", "Remember a line but not the song? Type it in the search bar and pick LYRICS: your own songs that have it (click to play from that line) and the songs online.", 6),
    ("theme", "KEEP THESE COLOURS", "Like how the Album Cover theme looks with this song? THEME > + SAVE THESE COLOURS keeps it as a theme of yours.", 5),
    ("refresh", "REFRESH", "↻ checks for a DK.FM update and syncs your Spotify playlists right now.", 2),
    ("settings", "SETTINGS", "Settings has a search box: type what you're looking for, like \"quality\" or \"spotify\".", 2),
];

/// The steps of the tour that's on: all of them, or only those newer than `app.tour_from`.
fn steps(app: &App) -> Vec<(&'static str, &'static str, &'static str, u32)> {
    STEPS.iter().copied().filter(|s| s.3 > app.tour_from).collect()
}

/// Is there anything new to show someone who was last offered tour `seen`?
pub fn has_new(seen: u32) -> bool {
    STEPS.iter().any(|s| s.3 > seen)
}

/// A tour of just what's new since tour `seen`.
pub fn start_new(app: &mut App, seen: u32) {
    app.modal = None;
    app.tour_from = seen;
    app.tour = Some(0);
}

pub fn start(app: &mut App) {
    app.modal = None;
    app.tour_from = 0;
    app.tour = Some(0);
}

/// Draws the current step over everything.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(step) = app.tour else { return };
    let list = steps(app);
    let Some((id, title, text, _)) = list.get(step).copied() else {
        app.tour = None;
        return;
    };
    // a panel that shares its spot with others (the scope sits behind the EQ by default) comes to
    // the front for its step, so it can be lit up where it is
    let tab = match id {
        "scope" => Some(super::Tab::Scope),
        "eq" => Some(super::Tab::Eq),
        "library" => Some(super::Tab::Library),
        "homepanel" | "web" | "import" | "downloads" | "discover" | "stats" => Some(super::Tab::Home),
        _ => None,
    };
    if let Some(tab) = tab {
        if app.dock.find_tab(&tab).is_some() && super::reveal(&mut app.dock, tab) {
            app.marks.remove(id); // where it was last frame: behind another tab
            ctx.request_repaint();
        }
    }
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
            ui.label(egui::RichText::new(format!("{}{} / {}", if app.tour_from > 0 { "NEW · " } else { "" }, step + 1, list.len())).font(px(6.0)).color(pal.dim));
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
                    let last = step + 1 == list.len();
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
    app.tour = if end || (next && step + 1 == list.len()) {
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

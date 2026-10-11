//! REPLACE COVER… (a song's right-click menu, or right-click the cover on the deck): for a song
//! with the wrong cover art, or none. Search album covers (Apple Music's, big and clean) or
//! YouTube Music's, pick a picture from your PC, or paste a picture's link. It's used for the
//! song or its whole album, and put into the music files too (a song that's playing gets it in
//! its file once another song is on), so other players show it as well.
use super::settings::window;
use super::theme::{px, vt};
use super::widgets::{button, fill, frame_rect, outline_button};
use super::App;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Ui, Vec2};
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Clone)]
struct Art {
    /// shown in the grid / used
    small: String,
    big: String,
    title: String,
    by: String,
}

type Results = Arc<Mutex<Option<Result<Vec<Art>, String>>>>;

pub struct Fix {
    id: String,
    /// the songs on its album in your library (this one included; just it when it has no album)
    album: Vec<String>,
    whole: bool,
    query: String,
    /// "albums" (Apple Music) or "songs" (YouTube Music)
    source: &'static str,
    results: Results,
    searched: (String, String),
    link: String,
}

/// Where a new cover comes from.
enum From {
    Web(String),
    File(std::path::PathBuf),
}

/// Open REPLACE COVER for a song (searching right away).
pub fn open(app: &mut App, id: &str, ctx: &egui::Context) {
    let Some(t) = app.lib.track(id) else { return };
    let album: Vec<String> = if t.album.trim().is_empty() {
        vec![t.id.clone()]
    } else {
        let key = super::browser::album_key(&t);
        let d = app.lib.data.read();
        let mut v: Vec<String> = d.tracks.values().filter(|x| super::browser::album_key(x) == key).map(|x| x.id.clone()).collect();
        v.sort();
        v
    };
    let what = if t.album.trim().is_empty() { crate::lyricsrc::bare_title(&t.title) } else { t.album.clone() };
    let query = format!("{} {}", crate::library::main_artist(if t.album_artist.is_empty() { &t.artist } else { &t.album_artist }), what);
    let whole = album.len() > 1;
    let mut f = Fix { id: id.to_string(), album, whole, query, source: "albums", results: Arc::new(Mutex::new(None)), searched: Default::default(), link: String::new() };
    search(&mut f, ctx);
    app.new_cover = Some(f);
}

fn search(f: &mut Fix, ctx: &egui::Context) {
    let key = (f.source.to_string(), f.query.trim().to_string());
    if key.1.is_empty() || key == f.searched {
        return;
    }
    f.searched = key.clone();
    let slot: Results = Arc::new(Mutex::new(None));
    f.results = slot.clone();
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("cover-search".into())
        .spawn(move || {
            let r = if key.0 == "albums" { apple(&key.1) } else { youtube_music(&key.1) };
            *slot.lock() = Some(r);
            ctx.request_repaint();
        })
        .ok();
}

/// Apple Music's album covers (the iTunes search: no account needed, any size).
fn apple(q: &str) -> Result<Vec<Art>, String> {
    let v = crate::net::get_json(&format!("https://itunes.apple.com/search?term={}&entity=album&limit=24", crate::net::enc(q)), &[])?;
    let mut out: Vec<Art> = Vec::new();
    for x in v["results"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let Some(url) = x["artworkUrl100"].as_str().filter(|u| u.contains("100x100")) else { continue };
        let art = Art { small: url.replace("100x100", "300x300"), big: url.replace("100x100", "1000x1000"), title: x["collectionName"].as_str().unwrap_or("").into(), by: x["artistName"].as_str().unwrap_or("").into() };
        if !out.iter().any(|a| a.big == art.big) {
            out.push(art);
        }
    }
    Ok(out)
}

/// The covers of YouTube Music's songs (their albums' or singles' art).
fn youtube_music(q: &str) -> Result<Vec<Art>, String> {
    let mut out: Vec<Art> = Vec::new();
    for t in crate::sources::search(q, "songs", 20)? {
        let Some(c) = t.cover.clone() else { continue };
        if out.iter().any(|a| a.big == c) {
            continue;
        }
        let title = if t.album.is_empty() { t.title.clone() } else { t.album.clone() };
        out.push(Art { small: c.clone(), big: c, title, by: t.artists.join(", ") });
    }
    Ok(out)
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut f) = app.new_cover.take() else { return };
    let Some(t) = app.lib.track(&f.id) else { return };
    let pal = app.pal;
    let mut chosen: Option<From> = None;
    let mut cancel = false;
    let open = window(pal, ctx, "REPLACE COVER", Vec2::new(760.0, 520.0), true, |ui| {
        ui.label(egui::RichText::new(format!("{} - {}", t.artist, t.title)).font(vt(22.0)).color(pal.text));
        ui.add_space(6.0);
        let body_h = ui.available_height() - 40.0;
        ui.horizontal_top(|ui| {
            // left: the cover now, which songs get the new one, your own picture
            ui.allocate_ui(Vec2::new(180.0, body_h), |ui| {
                ui.vertical(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(170.0), Sense::hover());
                    fill(ui.painter(), r, pal.bg);
                    match super::preview::cover_tex(app, ui.ctx(), &t, 256) {
                        Some(tex) => {
                            ui.painter().image(tex, r.shrink(2.0), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                        }
                        None => {
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, "NO COVER", px(6.0), pal.faint);
                        }
                    }
                    frame_rect(ui.painter(), r, 2.0, pal.line_hi);
                    ui.label(egui::RichText::new("NOW").font(px(5.0)).color(pal.dim));
                    ui.add_space(8.0);
                    if f.album.len() > 1 {
                        ui.label(egui::RichText::new("USE IT FOR").font(px(5.0)).color(pal.dim));
                        if outline_button(ui, &pal, &format!("THE ALBUM ({} SONGS)", f.album.len()), f.whole).on_hover_text(format!("Every song of \"{}\" in your library", t.album)).clicked() {
                            f.whole = true;
                        }
                        if outline_button(ui, &pal, "JUST THIS SONG", !f.whole).clicked() {
                            f.whole = false;
                        }
                        ui.add_space(8.0);
                    }
                    if button(ui, &pal, "PICK A PICTURE…", false, true).on_hover_text("A JPG or PNG from your PC (cut square from the middle)").clicked() {
                        if let Some(p) = rfd::FileDialog::new().set_title("Choose a cover").add_filter("Images", &["jpg", "jpeg", "png"]).pick_file() {
                            chosen = Some(From::File(p));
                        }
                    }
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(&mut f.link).desired_width(170.0).font(vt(16.0)).hint_text("or paste a picture's link"));
                    let link = f.link.trim();
                    let ok = link.starts_with("http://") || link.starts_with("https://");
                    if button(ui, &pal, "USE THE LINK", false, ok).clicked() && ok {
                        chosen = Some(From::Web(link.to_string()));
                    }
                });
            });
            ui.add_space(10.0);
            // right: search
            ui.vertical(|ui| {
                let mut go = false;
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut f.query).desired_width(250.0).font(vt(19.0)).hint_text("Album or song, and artist"));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        go = true;
                    }
                    for (s, label, tip) in [("albums", "ALBUMS", "Apple Music's album covers (big and clean)"), ("songs", "YT MUSIC", "YouTube Music's covers (singles and less known songs too)")] {
                        if outline_button(ui, &pal, label, f.source == s).on_hover_text(tip).clicked() {
                            f.source = s;
                            go = true;
                        }
                    }
                    if button(ui, &pal, "SEARCH", true, true).clicked() {
                        go = true;
                    }
                });
                if go {
                    search(&mut f, ui.ctx());
                }
                ui.add_space(6.0);
                let h = body_h - 40.0;
                let res = f.results.lock().clone();
                match res {
                    None => message(ui, h, "SEARCHING…", &pal),
                    Some(Err(e)) => message(ui, h, &format!("Couldn't search: {e}"), &pal),
                    Some(Ok(list)) if list.is_empty() => message(ui, h, "NOTHING FOUND: TRY OTHER WORDS, OR YT MUSIC", &pal),
                    Some(Ok(list)) => {
                        egui::ScrollArea::vertical().id_salt("cover-found").max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                            let side = 128.0;
                            let cols = ((ui.available_width() + 8.0) / (side + 8.0)).floor().max(1.0) as usize;
                            for chunk in list.chunks(cols) {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    for a in chunk {
                                        if tile(app, ui, a, side) {
                                            chosen = Some(From::Web(a.big.clone()));
                                        }
                                    }
                                });
                                ui.add_space(6.0);
                            }
                        });
                    }
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Click a cover to use it. It goes into the music files too, so other players show it.").font(vt(15.0)).color(pal.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, &pal, "CANCEL", false, true).clicked() {
                    cancel = true;
                }
            });
        });
    });
    if let Some(from) = chosen {
        let ids = if f.whole { f.album.clone() } else { vec![f.id.clone()] };
        let name = if ids.len() > 1 { format!("\"{}\"", t.album) } else { format!("\"{}\"", t.title) };
        apply(app, ctx, ids, from, name);
        return;
    }
    if open && !cancel {
        app.new_cover = Some(f);
    }
}

/// One cover in the grid (its album and artist under it). True when clicked.
fn tile(app: &mut App, ui: &mut Ui, a: &Art, side: f32) -> bool {
    let pal = app.pal;
    let (r, resp) = ui.allocate_exact_size(Vec2::new(side, side + 34.0), Sense::click());
    let img = Rect::from_min_size(r.min, Vec2::splat(side));
    fill(ui.painter(), img, pal.bg);
    match super::home::remote_tex(app, ui.ctx(), Some(&a.small)) {
        Some(tex) => {
            ui.painter().image(tex, img, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().text(img.center(), Align2::CENTER_CENTER, "…", px(6.0), pal.faint);
        }
    }
    frame_rect(ui.painter(), img, if resp.hovered() { 3.0 } else { 1.5 }, if resp.hovered() { pal.accent } else { pal.line });
    let p = ui.painter().with_clip_rect(r);
    p.text(egui::pos2(r.left() + 2.0, img.bottom() + 9.0), Align2::LEFT_CENTER, &a.title, vt(15.0), pal.text);
    p.text(egui::pos2(r.left() + 2.0, img.bottom() + 25.0), Align2::LEFT_CENTER, &a.by, vt(14.0), pal.dim);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text(format!("{}\n{}\nClick to use this cover", a.title, a.by)).clicked()
}

fn message(ui: &mut Ui, h: f32, s: &str, pal: &super::theme::Pal) {
    ui.allocate_ui(Vec2::new(ui.available_width(), h), |ui| {
        ui.centered_and_justified(|ui| ui.label(egui::RichText::new(s).font(px(6.0)).color(pal.dim)));
    });
}

/// Get the picture (in the background), show it for the songs right away, then put it in their
/// files: the song playing waits until it isn't (its file is being read).
fn apply(app: &mut App, ctx: &egui::Context, ids: Vec<String>, from: From, name: String) {
    let (lib, player, ctx) = (app.lib.clone(), app.player.clone(), ctx.clone());
    if matches!(from, From::Web(_)) {
        app.toast(format!("Getting the new cover for {name}…"));
    }
    std::thread::Builder::new()
        .name("new-cover".into())
        .spawn(move || {
            let note = |m: String| {
                player.notices.lock().push(m);
                ctx.request_repaint();
            };
            let bytes = match &from {
                From::Web(u) => crate::net::get_bytes(u),
                From::File(p) => std::fs::read(p).map_err(|e| e.to_string()),
            };
            let jpeg = match bytes.map(|b| crate::library::cover_jpeg(&b)) {
                Ok(Some(j)) => j,
                Ok(None) => return note("That picture couldn't be opened (use a JPG or PNG)".into()),
                Err(e) => return note(format!("Couldn't get that picture: {e}")),
            };
            if !lib.set_cover(&ids, &jpeg) {
                return note("Couldn't save that picture".into());
            }
            note(format!("New cover for {name}"));
            let mut failed = 0;
            for id in &ids {
                while player.current_id().as_deref() == Some(id.as_str()) {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
                let Some(t) = lib.track(id) else { continue };
                if crate::library::embed_cover(std::path::Path::new(&t.path), &jpeg).is_err() {
                    failed += 1;
                }
            }
            if failed > 0 {
                note(format!("DK.FM shows the new cover, but couldn't put it in {failed} of the music files (read-only?)"));
            }
        })
        .ok();
}

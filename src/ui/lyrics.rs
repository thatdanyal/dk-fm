//! Synced lyrics from LRCLIB; click a line to jump there.
use super::theme::{px, vt};
use super::App;
use eframe::egui::{self, Ui};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub enum Lyr {
    Loading,
    None,
    Instrumental,
    Plain(String),
    Synced(Vec<(f64, String)>),
}

#[derive(Default)]
pub struct LyricsState {
    cache: Arc<Mutex<HashMap<String, Lyr>>>,
    last_active: Option<usize>,
}

fn parse_lrc(src: &str) -> Vec<(f64, String)> {
    let re = regex::Regex::new(r"\[(\d+):(\d+(?:\.\d+)?)\]").unwrap();
    let strip = regex::Regex::new(r"\[[^\]]*\]").unwrap();
    let mut out = Vec::new();
    for line in src.lines() {
        let text = strip.replace_all(line, "").trim().to_string();
        for c in re.captures_iter(line) {
            let t = c[1].parse::<f64>().unwrap_or(0.0) * 60.0 + c[2].parse::<f64>().unwrap_or(0.0);
            out.push((t, text.clone()));
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    out
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let Some(t) = app.current_track() else {
        ui.centered_and_justified(|ui| ui.label(egui::RichText::new("NO LYRICS").color(pal.dim)));
        return;
    };
    let entry = app.lyrics.cache.lock().get(&t.id).cloned();
    let lyr = match entry {
        Some(l) => l,
        None => {
            app.lyrics.cache.lock().insert(t.id.clone(), Lyr::Loading);
            let cache = app.lyrics.cache.clone();
            let ctx = ui.ctx().clone();
            let (id, artist, title, album, dur) = (t.id.clone(), crate::library::main_artist(&t.artist), t.title.clone(), t.album.clone(), t.duration);
            std::thread::spawn(move || {
                let clean = regex::Regex::new(r"(?i)\s*[\(\[]\s*(feat|ft|with)[^\)\]]*[\)\]]").unwrap().replace_all(&title, "").to_string();
                let r = crate::net::lyrics(&artist, &clean, &album, dur);
                let l = match r {
                    Some(v) if v["instrumental"].as_bool() == Some(true) => Lyr::Instrumental,
                    Some(v) if v["syncedLyrics"].is_string() => Lyr::Synced(parse_lrc(v["syncedLyrics"].as_str().unwrap())),
                    Some(v) if v["plainLyrics"].is_string() => Lyr::Plain(v["plainLyrics"].as_str().unwrap().to_string()),
                    _ => Lyr::None,
                };
                cache.lock().insert(id, l);
                ctx.request_repaint();
            });
            Lyr::Loading
        }
    };
    match lyr {
        Lyr::Loading => center(ui, "SEARCHING…", &pal),
        Lyr::None => center(ui, "NO LYRICS FOUND", &pal),
        Lyr::Instrumental => center(ui, "♪ INSTRUMENTAL ♪", &pal),
        Lyr::Plain(s) => {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("lyr").show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin::same(12)).show(ui, |ui| ui.label(egui::RichText::new(s).font(vt(19.0)).color(pal.text)));
            });
        }
        Lyr::Synced(lines) => {
            let now = app.player.status().position + 0.25;
            let active = lines.iter().rposition(|(t, _)| *t <= now);
            let scroll_now = active != app.lyrics.last_active;
            app.lyrics.last_active = active;
            egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("lyr").show(ui, |ui| {
                ui.add_space(ui.available_height() * 0.35);
                ui.vertical_centered(|ui| {
                    for (i, (t, text)) in lines.iter().enumerate() {
                        let (size, color) = if Some(i) == active { (25.0, pal.accent) } else if active.map(|a| i < a).unwrap_or(false) { (21.0, pal.faint) } else { (21.0, pal.dim) };
                        let txt = if text.is_empty() { "♪" } else { text.as_str() };
                        let r = ui.add(egui::Label::new(egui::RichText::new(txt).font(vt(size)).color(color)).wrap().sense(egui::Sense::click()));
                        if r.clicked() {
                            app.player.seek(*t);
                        }
                        if r.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if Some(i) == active && scroll_now {
                            // jump, don't glide: egui animates scrolls at the full refresh rate,
                            // which cost ~10 extra frames a second while lyrics played
                            r.scroll_to_me_animation(Some(egui::Align::Center), egui::style::ScrollAnimation::none());
                        }
                    }
                });
                ui.add_space(ui.available_height() * 0.35 + 200.0);
            });
        }
    }
}

fn center(ui: &mut Ui, s: &str, pal: &super::theme::Pal) {
    ui.centered_and_justified(|ui| ui.label(egui::RichText::new(s).font(px(8.0)).color(pal.dim)));
}

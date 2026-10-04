//! FIND SONGS: search YouTube Music / YouTube (not your library) and pick which version of a
//! song to download. Shows a few results (Settings > Search, default 3), each labelled CLEAN /
//! EXPLICIT / INSTRUMENTAL / LIVE ... where that can be told; nothing is ever picked for you.
use super::theme::{px, vt};
use super::widgets::{button, tb_button};
use super::App;
use crate::sources::{self, ITrack};
use eframe::egui::{self, Ui};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// (source, query, how many)
type Key = (String, String, usize);
type Found = Result<Vec<ITrack>, String>;

pub const SOURCES: [(&str, &str, &str); 2] = [("songs", "YOUTUBE MUSIC", "Songs on YouTube Music: studio versions, marked explicit where they are"), ("youtube", "YOUTUBE", "Any video on YouTube: live shows, covers, remixes...")];

#[derive(Default)]
pub struct WebState {
    pub query: String,
    pub focus: bool,
    edited: Option<Instant>,
    /// recent searches (newest last, ≤ 12)
    done: Vec<(Key, Found)>,
    busy: HashSet<Key>,
    slot: Arc<Mutex<Vec<(Key, Found)>>>,
    /// (lib.gen, key) -> which results you already have
    owned: Option<(u64, Key, Vec<Option<String>>)>,
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let finished: Vec<(Key, Found)> = app.browser.web.slot.lock().drain(..).collect();
    for (k, r) in finished {
        let w = &mut app.browser.web;
        w.busy.remove(&k);
        w.done.retain(|x| x.0 != k);
        w.done.push((k, r));
        if w.done.len() > 12 {
            w.done.drain(..1);
        }
    }
    let (n, mut source, hide_explicit) = { let s = app.settings.lock(); (s.web_results.clamp(1, 20) as usize, s.web_source.clone(), s.hide_explicit) };
    if !SOURCES.iter().any(|s| s.0 == source) {
        source = SOURCES[0].0.into();
    }
    egui::Frame::new().inner_margin(egui::Margin::same(16)).show(ui, |ui| {
        ui.label(egui::RichText::new("FIND SONGS ONLINE").font(px(12.0)).color(pal.text));
        ui.add_space(6.0);
        let mut go = false;
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut app.browser.web.query).hint_text("Song and artist, e.g. \"Blinding Lights The Weeknd\"").desired_width(ui.available_width() - 110.0).font(vt(19.0)));
            if app.browser.web.focus {
                r.request_focus();
                app.browser.web.focus = false;
            }
            if r.changed() {
                app.browser.web.edited = Some(Instant::now());
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            if button(ui, &pal, "SEARCH", true, true).clicked() {
                go = true;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (k, label, tip) in SOURCES {
                if tb_button(ui, &pal, label, source == k).on_hover_text(tip).clicked() && source != k {
                    source = k.to_string();
                    app.edit_settings(|s| s.web_source = k.to_string());
                }
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new(format!("{n} version{} per search (Settings > Search)", if n == 1 { "" } else { "s" })).font(vt(17.0)).color(pal.dim));
        });
        ui.label(egui::RichText::new("Pick the version you want: clean, explicit, live, instrumental... Nothing downloads until you click + GET.").color(pal.dim));
        ui.add_space(8.0);

        let q = app.browser.web.query.trim().to_string();
        if q.chars().count() < 2 {
            return;
        }
        let key: Key = (source.clone(), q.clone(), n);
        let w = &mut app.browser.web;
        let cached = w.done.iter().find(|x| x.0 == key).map(|x| x.1.clone());
        let idle = w.edited.map(|t| t.elapsed() >= Duration::from_millis(800)).unwrap_or(true);
        if cached.is_none() && !w.busy.contains(&key) {
            if idle || go {
                w.busy.insert(key.clone());
                let (slot, ctx, k) = (w.slot.clone(), ui.ctx().clone(), key.clone());
                std::thread::spawn(move || {
                    let r = sources::search(&k.1, &k.0, k.2);
                    if let Ok(v) = &r {
                        super::cjk::ensure(&ctx, &v.iter().map(|t| format!("{} {}", t.title, t.artists.join(" "))).collect::<String>());
                    }
                    slot.lock().push((k, r));
                    ctx.request_repaint();
                });
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
        }
        match cached {
            None => {
                let tools = crate::ytdlp::STATUS.lock().state.clone();
                ui.label(egui::RichText::new(if tools == "installing" { "SEARCHING… (getting the download tools first)" } else { "SEARCHING…" }).font(px(7.0)).color(pal.accent));
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(e).color(pal.accent));
            }
            Some(Ok(found)) => {
                // labels are worked out on everything found, before any hiding
                let tags: Vec<Vec<&str>> = found.iter().map(|t| sources::version_tags(t, &found)).collect();
                let (found, tags): (Vec<ITrack>, Vec<Vec<&str>>) = found.iter().cloned().zip(tags).filter(|(t, _)| !(hide_explicit && t.explicit)).unzip();
                if found.is_empty() {
                    ui.label(egui::RichText::new("Only explicit versions found (Settings > Search hides them).").color(pal.dim));
                    return;
                }
                let gen = app.lib.gen.load(std::sync::atomic::Ordering::Relaxed);
                if app.browser.web.owned.as_ref().map(|o| o.0 != gen || o.1 != key).unwrap_or(true) {
                    let owned = super::addsongs::owned_ids(app, &found);
                    app.browser.web.owned = Some((gen, key.clone(), owned));
                }
                let owned = app.browser.web.owned.as_ref().map(|o| o.2.clone()).unwrap_or_default();
                let w = ui.available_width();
                egui::ScrollArea::vertical().id_salt("websearch").auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    super::addsongs::result_rows_tagged(app, ui, None, &found, &owned, &tags, w);
                });
            }
        }
    });
}

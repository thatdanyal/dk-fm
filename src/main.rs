#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! DK.FM — native retro music player.
mod analysis;
mod audio;
mod backup;
mod discover;
mod downloader;
#[cfg(windows)]
mod install;
mod library;
mod lyricsrc;
mod net;
mod player;
mod recognize;
mod share;
mod single;
mod sources;
#[cfg(windows)]
mod taskbar;
mod spotify_auth;
mod store;
mod system;
mod ui;
mod watcher;
mod ytdlp;

use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn main() -> eframe::Result {
    crash_log();
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("--test-play") {
        test_play(&args);
        return Ok(());
    }
    if args.get(1).map(|s| s.as_str()) == Some("--test-download") {
        test_download(&args);
        return Ok(());
    }
    #[cfg(windows)]
    if install::handle(&args) {
        return Ok(());
    }
    system::cleanup_old();
    // already running? ask it to show itself and quit
    let command = ["--play-pause", "--next", "--prev"].into_iter().find(|c| args.iter().any(|a| a == c)).map(|c| &c[2..]);
    let Some(_instance) = single::acquire(command) else { return Ok(()) };
    let hidden = args.iter().any(|a| a == "--hidden");

    let lib = library::Library::load();
    let settings = Arc::new(Mutex::new(store::Settings::load()));
    let dirty = Arc::new(AtomicBool::new(false));
    let player = player::Player::new(lib.clone(), settings.clone(), dirty.clone());
    let dl = downloader::Downloader::new(lib.clone(), settings.clone());
    let watcher = watcher::FolderWatcher::new(lib.clone());

    // scan + watch music folders (new/changed files only; unchanged ones are skipped)
    let folders: Vec<PathBuf> = {
        let s = settings.lock();
        let _ = std::fs::create_dir_all(&s.download_dir);
        let mut v: Vec<PathBuf> = s.music_folders.iter().map(PathBuf::from).chain(std::iter::once(PathBuf::from(&s.download_dir))).filter(|p| p.exists()).collect();
        v.sort();
        v.dedup();
        v
    };
    lib.scan(folders.clone());
    watcher.watch(&folders);

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).unwrap_or_default();
    let opts = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("DK.FM")
            .with_app_id("dkfm")
            // DKFM_WIN=<w>,<h>: dev/testing window size
            .with_inner_size(std::env::var("DKFM_WIN").ok().and_then(|v| v.split_once(',').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))).unwrap_or([1360.0, 860.0]))
            .with_min_inner_size([380.0, 120.0])
            .with_decorations(false)
            .with_icon(Arc::new(icon)),
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };
    eframe::run_native("DK.FM", opts, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, lib, player, dl, watcher, settings, dirty, hidden)))))
}

/// Headless engine check: `dkfm --test-play [seconds]`.
fn test_play(args: &[String]) {
    let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);
    let lib = library::Library::load();
    let settings = Arc::new(Mutex::new(store::Settings::load()));
    let p = player::Player::new(lib.clone(), settings, Arc::new(AtomicBool::new(false)));
    let ids: Vec<String> = {
        let d = lib.data.read();
        let mut v: Vec<_> = d.tracks.values().map(|t| (t.artist.clone(), t.title.clone(), t.id.clone())).collect();
        v.sort();
        v.into_iter().map(|x| x.2).collect()
    };
    println!("library: {} tracks", ids.len());
    p.set_volume(0.15);
    p.play_list(ids, 0, Some(false));
    for i in 0..secs {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let s = p.status();
        println!("t={:>2}s playing={} pos={:.2}/{:.1} track={:?}", i + 1, s.playing, s.position, s.duration, s.current.as_ref().and_then(|id| lib.track(id)).map(|t| t.title));
        for n in std::mem::take(&mut *p.notices.lock()) {
            println!("      {n}");
        }
    }
}

/// Headless import check: `dkfm --test-download <link> [max-songs]`.
fn test_download(args: &[String]) {
    let url = args.get(2).cloned().unwrap_or_default();
    let max: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);
    let lib = library::Library::load();
    let settings = Arc::new(Mutex::new(store::Settings::load()));
    let dl = downloader::Downloader::new(lib.clone(), settings.clone());
    let t0 = std::time::Instant::now();
    let col = match sources::fetch_any(&url, &dl.creds()) {
        Ok(sources::Fetched::Collection(c)) => c,
        Ok(_) => return println!("profile link"),
        Err(e) => return println!("fetch failed: {e}"),
    };
    println!("fetched \"{}\" ({} tracks, via {}) in {:.1}s", col.name, col.tracks.len(), col.via, t0.elapsed().as_secs_f32());
    let n = col.tracks.len().min(max);
    let job = dl.start(col, Some((0..n).collect()));
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let jobs = dl.jobs.lock();
        let j = jobs.iter().find(|j| j.id == job).unwrap();
        if j.tracks.iter().all(|t| !t.status.active()) {
            for t in &j.tracks {
                if t.status == downloader::TStatus::Skipped { continue; }
                let lt = t.track_id.as_ref().and_then(|id| lib.track(id));
                println!("{:?} {} | {} — {} | match: {:?} | file: {:?} | tags: {:?}", t.status, t.note.as_deref().unwrap_or(""), t.t.title, t.t.artists.join(", "), t.matched.as_ref().map(|m| &m.title), lt.as_ref().map(|l| &l.path), lt.as_ref().map(|l| (&l.title, &l.artist, &l.album, l.cover.is_some(), l.bitrate)));
                if let Some(e) = &t.error { println!("   error: {e}"); }
            }
            break;
        }
    }
    println!("total {:.1}s", t0.elapsed().as_secs_f32());
    lib.flush();
}

/// Errors (panics) leave no message on screen (no console on Windows): write what happened to
/// crash.log in the data folder, so it can be reported and fixed. A panic in a background
/// thread only stops that thread; one while drawing the window closes that screen (App::recover).
fn crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        // (`info` has the file and line; release builds have no symbols for a backtrace)
        let line = format!("[{}] DK.FM {} error in thread '{thread}': {info}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), env!("CARGO_PKG_VERSION"));
        let path = store::data_dir().join("crash.log");
        // keep the file small: just the latest few crashes
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        let mut cut = old.len().saturating_sub(16 * 1024);
        while !old.is_char_boundary(cut) {
            cut += 1;
        }
        let keep = &old[cut..];
        let _ = std::fs::create_dir_all(store::data_dir());
        let _ = std::fs::write(&path, format!("{keep}{line}"));
        default(info);
    }));
}

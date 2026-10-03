//! OS integration: tray icon, media keys / OS media overlay, start-at-login, window show/hide,
//! and self-update from GitHub Releases.
use crate::player::Player;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Arc;

pub static HWND: AtomicIsize = AtomicIsize::new(0);
pub static HIDDEN: AtomicBool = AtomicBool::new(false);
pub static CTX: Mutex<Option<eframe::egui::Context>> = Mutex::new(None);

/// Send a file to the Recycle Bin / Trash (never a permanent delete).
pub fn trash(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE, SHFILEOPSTRUCTW};
        let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
        from.extend([0, 0]); // double-NUL terminated list
        let mut op: SHFILEOPSTRUCTW = std::mem::zeroed();
        op.wFunc = FO_DELETE as _;
        op.pFrom = from.as_ptr();
        op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as _;
        let r = SHFileOperationW(&mut op);
        if r != 0 || op.fAnyOperationsAborted != 0 {
            return Err(format!("Could not move {} to the Recycle Bin (error {r})", path.display()));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let home = dirs::home_dir().ok_or("no home folder")?;
        let dir = if cfg!(target_os = "macos") { home.join(".Trash") } else { dirs::data_dir().unwrap_or_else(|| home.join(".local/share")).join("Trash/files") };
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let name = path.file_name().ok_or("bad path")?.to_string_lossy().into_owned();
        let mut dest = dir.join(&name);
        let mut n = 1;
        while dest.exists() {
            dest = dir.join(format!("{n} {name}"));
            n += 1;
        }
        std::fs::rename(path, &dest).or_else(|_| std::fs::copy(path, &dest).and_then(|_| std::fs::remove_file(path))).map_err(|e| e.to_string())
    }
}

/// Bring the window back (works even when the GUI loop is idle because the window is hidden).
pub fn show_window() {
    HIDDEN.store(false, Ordering::Relaxed);
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW};
        let h = HWND.load(Ordering::Relaxed) as windows_sys::Win32::Foundation::HWND;
        if !h.is_null() {
            ShowWindow(h, SW_SHOW);
            ShowWindow(h, SW_RESTORE);
            SetForegroundWindow(h);
        }
    }
    if let Some(ctx) = CTX.lock().as_ref() {
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }
}

/// Hand pages we aren't using back to Windows: mostly one-off startup work (the graphics
/// driver's shader compiler, the library scan). Anything still in use is paged straight back in,
/// so this frees RAM for other programs without slowing DK.FM down.
pub fn trim_memory() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

pub fn hide_window() {
    HIDDEN.store(true, Ordering::Relaxed);
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(2)); // after the last frame is drawn
        trim_memory();
    });
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        let h = HWND.load(Ordering::Relaxed) as windows_sys::Win32::Foundation::HWND;
        if !h.is_null() {
            ShowWindow(h, SW_HIDE);
            return;
        }
    }
    if let Some(ctx) = CTX.lock().as_ref() {
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
    }
}

pub static QUIT: AtomicBool = AtomicBool::new(false);

// ------------------------------------------------------------------------------------- tray

#[cfg(not(target_os = "linux"))]
pub struct Tray {
    tray: tray_icon::TrayIcon,
    now: tray_icon::menu::MenuItem,
    play: tray_icon::menu::MenuItem,
}

#[cfg(target_os = "linux")]
pub struct Tray;

impl Tray {
    #[cfg(not(target_os = "linux"))]
    pub fn new(player: Arc<Player>) -> Option<Self> {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
        use tray_icon::{TrayIconBuilder, TrayIconEvent};
        let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?.resize(32, 32, image::imageops::FilterType::Lanczos3).to_rgba8();
        let icon = tray_icon::Icon::from_rgba(img.to_vec(), img.width(), img.height()).ok()?;
        let now = MenuItem::new("Nothing playing", false, None);
        let play = MenuItem::new("Play", true, None);
        let next = MenuItem::new("Next", true, None);
        let prev = MenuItem::new("Previous", true, None);
        let show = MenuItem::new("Show DK.FM", true, None);
        let quit = MenuItem::new("Quit DK.FM", true, None);
        let menu = Menu::new();
        menu.append_items(&[&now, &PredefinedMenuItem::separator(), &play, &next, &prev, &PredefinedMenuItem::separator(), &show, &quit]).ok()?;
        let tray = TrayIconBuilder::new().with_icon(icon).with_tooltip("DK.FM").with_menu(Box::new(menu)).build().ok()?;
        let (pid, nid, vid, sid, qid) = (play.id().clone(), next.id().clone(), prev.id().clone(), show.id().clone(), quit.id().clone());
        let p = player.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if e.id == pid {
                p.toggle();
            } else if e.id == nid {
                p.next(true);
            } else if e.id == vid {
                p.prev();
            } else if e.id == sid {
                show_window();
            } else if e.id == qid {
                QUIT.store(true, Ordering::Relaxed);
                show_window();
                if let Some(ctx) = CTX.lock().as_ref() {
                    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
                }
            }
        }));
        TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: tray_icon::MouseButton::Left, button_state: tray_icon::MouseButtonState::Up, .. } = e {
                show_window();
            }
        }));
        Some(Self { tray, now, play })
    }

    #[cfg(target_os = "linux")]
    pub fn new(_player: Arc<Player>) -> Option<Self> {
        None
    }

    #[allow(unused_variables)]
    pub fn update(&self, label: &str, playing: bool) {
        #[cfg(not(target_os = "linux"))]
        {
            let l: String = label.chars().take(60).collect();
            self.now.set_text(if l.is_empty() { "Nothing playing".to_string() } else { l.clone() });
            self.play.set_text(if playing { "Pause" } else { "Play" });
            let _ = self.tray.set_tooltip(Some(if l.is_empty() { "DK.FM".to_string() } else { format!("DK.FM · {l}") }));
        }
    }
}

// ------------------------------------------------------------------------------------- media keys

pub struct Media {
    controls: Option<souvlaki::MediaControls>,
    last: String,
}

impl Media {
    pub fn new(player: Arc<Player>) -> Self {
        use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig, SeekDirection};
        let hwnd = HWND.load(Ordering::Relaxed);
        let cfg = PlatformConfig { dbus_name: "dkfm", display_name: "DK.FM", hwnd: if hwnd != 0 { Some(hwnd as *mut std::ffi::c_void) } else { None } };
        let controls = MediaControls::new(cfg).ok().and_then(|mut c| {
            let p = player.clone();
            c.attach(move |e| match e {
                MediaControlEvent::Play => p.resume(),
                MediaControlEvent::Pause => p.pause(),
                MediaControlEvent::Toggle => p.toggle(),
                MediaControlEvent::Next => p.next(true),
                MediaControlEvent::Previous => p.prev(),
                MediaControlEvent::Stop => p.pause(),
                MediaControlEvent::SetPosition(pos) => p.seek(pos.0.as_secs_f64()),
                MediaControlEvent::Seek(d) => {
                    let s = p.status().position;
                    p.seek(if matches!(d, SeekDirection::Forward) { s + 10.0 } else { s - 10.0 });
                }
                MediaControlEvent::Raise => show_window(),
                _ => {}
            })
            .ok()?;
            Some(c)
        });
        Self { controls, last: String::new() }
    }

    pub fn update(&mut self, title: &str, artist: &str, album: &str, cover: Option<&str>, duration: f64, playing: bool, position: f64) {
        let Some(c) = self.controls.as_mut() else { return };
        let key = format!("{title}|{artist}|{playing}");
        if key != self.last {
            self.last = key;
            let url = cover.map(|p| format!("file:///{}", p.replace('\\', "/")));
            let _ = c.set_metadata(souvlaki::MediaMetadata { title: Some(title), artist: Some(artist), album: Some(album), cover_url: url.as_deref(), duration: Some(std::time::Duration::from_secs_f64(duration.max(0.0))) });
        }
        let prog = Some(souvlaki::MediaPosition(std::time::Duration::from_secs_f64(position.max(0.0))));
        let _ = c.set_playback(if playing { souvlaki::MediaPlayback::Playing { progress: prog } } else { souvlaki::MediaPlayback::Paused { progress: prog } });
    }
}

// ------------------------------------------------------------------------------------- start at login

pub fn set_start_at_login(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    set_start_at_login_for(&exe, on)
}

pub fn set_start_at_login_for(exe: &std::path::Path, on: bool) -> Result<(), String> {
    let al = auto_launch::AutoLaunchBuilder::new()
        .set_app_name("DK.FM")
        .set_app_path(&exe.to_string_lossy())
        .set_args(&["--hidden"])
        .build()
        .map_err(|e| e.to_string())?;
    if on { al.enable() } else { al.disable() }.map_err(|e| e.to_string())
}

// ------------------------------------------------------------------------------------- updater

#[derive(Clone, Debug)]
pub struct Update {
    pub version: String,
    pub notes: String,
    pub asset: Option<String>,
    pub page: String,
}

#[derive(Clone, Debug, Default)]
pub enum UpdState {
    #[default]
    Idle,
    Checking,
    Latest,
    Available(Update),
    Downloading(u8),
    Error(String),
}

fn ver(v: &str) -> (u32, u32, u32) {
    let mut p = v.trim_start_matches('v').split('.').map(|x| x.parse().unwrap_or(0));
    (p.next().unwrap_or(0), p.next().unwrap_or(0), p.next().unwrap_or(0))
}

pub fn check_update() -> Result<Option<Update>, String> {
    let j = crate::net::get_json("https://api.github.com/repos/thatdanyal/dk-fm/releases/latest", &[("Accept", "application/vnd.github+json")])?;
    let tag = j["tag_name"].as_str().unwrap_or("");
    if ver(tag) <= ver(env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let want = if cfg!(windows) { "windows-x64.exe" } else if cfg!(target_os = "linux") { ".AppImage" } else { "" };
    let asset = if want.is_empty() {
        None
    } else {
        j["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str().map(|n| n.ends_with(want) && !n.contains("setup")).unwrap_or(false))).and_then(|x| x["browser_download_url"].as_str()).map(String::from)
    };
    Ok(Some(Update {
        version: tag.trim_start_matches('v').into(),
        notes: j["body"].as_str().unwrap_or("").replace("\r", "").trim().to_string(),
        asset,
        page: j["html_url"].as_str().unwrap_or("https://github.com/thatdanyal/dk-fm/releases/latest").into(),
    }))
}

/// Replace the running program with the new one and restart. (Windows allows renaming a running
/// .exe; on Linux the AppImage file is swapped. macOS builds aren't signed, so they open the page.)
pub fn install_update(u: &Update) -> Result<(), String> {
    let Some(url) = &u.asset else {
        let _ = open::that(&u.page);
        return Ok(());
    };
    let target = if cfg!(target_os = "linux") {
        std::env::var("APPIMAGE").map(std::path::PathBuf::from).map_err(|_| "Not running from an AppImage — download the new version from the release page.".to_string())?
    } else {
        std::env::current_exe().map_err(|e| e.to_string())?
    };
    let new = target.with_extension("new");
    crate::net::download(url, &new, false)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755));
    }
    let old = target.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&target, &old).map_err(|e| format!("could not replace the app: {e}"))?;
    if let Err(e) = std::fs::rename(&new, &target) {
        let _ = std::fs::rename(&old, &target);
        return Err(e.to_string());
    }
    std::process::Command::new(&target).arg("--updated").spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove the previous version left behind by an update.
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
    if let Ok(p) = std::env::var("APPIMAGE") {
        let _ = std::fs::remove_file(std::path::PathBuf::from(p).with_extension("old"));
    }
    // The Electron version kept its built-in browser's caches in the data folder we share
    // (~10 MB). Remove exactly those names; everything else in the folder is ours.
    let data = crate::store::data_dir();
    for d in [
        "blob_storage", "Cache", "Code Cache", "Crashpad", "DawnCache", "DawnGraphiteCache", "DawnWebGPUCache", "GPUCache",
        "GPUPersistentCache", "GrShaderCache", "IndexedDB", "Local Storage", "Network", "Service Worker", "Session Storage",
        "ShaderCache", "Shared Dictionary", "VideoDecodeStats", "WebStorage",
    ] {
        let _ = std::fs::remove_dir_all(data.join(d));
    }
    for f in [
        ".running", ".updaterId", "declarative_performance_observer.db", "declarative_performance_observer.db-journal",
        "DevToolsActivePort", "DIPS", "DIPS-wal", "Local State", "lockfile", "Network Persistent State", "Preferences",
        "SharedStorage", "SharedStorage-wal", "TransportSecurity", "Trust Tokens", "Trust Tokens-journal",
    ] {
        let _ = std::fs::remove_file(data.join(f));
    }
    // the old Electron version's update cache (it downloaded the installer that moved us over)
    #[cfg(windows)]
    if let (Some(local), Ok(exe)) = (dirs::data_local_dir(), std::env::current_exe()) {
        let cache = local.join("dk-fm-updater");
        if cache.exists() && !exe.starts_with(&cache) {
            let _ = std::fs::remove_dir_all(cache);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn trash_moves_file_away() {
        let f = std::env::temp_dir().join("dkfm-trash-test.txt");
        std::fs::write(&f, b"DK.FM recycle bin test - safe to delete").unwrap();
        super::trash(&f).unwrap();
        assert!(!f.exists());
    }
}

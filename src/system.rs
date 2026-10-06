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
        // Linux file managers only show (and can restore) a trashed file that has a matching
        // info/<name>.trashinfo (freedesktop.org trash spec)
        let info = (!cfg!(target_os = "macos")).then(|| dir.with_file_name("info").join(format!("{}.trashinfo", dest.file_name().unwrap_or_default().to_string_lossy())));
        if let Some(info) = &info {
            let full = std::path::absolute(path).unwrap_or(path.to_path_buf());
            let url = full.to_string_lossy().split('/').map(|p| urlencoding::encode(p).into_owned()).collect::<Vec<_>>().join("/");
            let _ = std::fs::create_dir_all(info.parent().unwrap_or(&dir));
            let _ = std::fs::write(info, format!("[Trash Info]\nPath={url}\nDeletionDate={}\n", chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")));
        }
        let moved = std::fs::rename(path, &dest).or_else(|_| std::fs::copy(path, &dest).and_then(|_| std::fs::remove_file(path))).map_err(|e| e.to_string());
        if moved.is_err() {
            if let Some(info) = &info {
                let _ = std::fs::remove_file(info);
            }
        }
        moved
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
    private: tray_icon::menu::CheckMenuItem,
}

#[cfg(target_os = "linux")]
pub struct Tray;

impl Tray {
    #[cfg(not(target_os = "linux"))]
    pub fn new(player: Arc<Player>, lib: Arc<crate::library::Library>) -> Option<Self> {
        use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
        use tray_icon::{TrayIconBuilder, TrayIconEvent};
        let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?.resize(32, 32, image::imageops::FilterType::Lanczos3).to_rgba8();
        let icon = tray_icon::Icon::from_rgba(img.to_vec(), img.width(), img.height()).ok()?;
        let now = MenuItem::new("Nothing playing", false, None);
        let play = MenuItem::new("Play", true, None);
        let next = MenuItem::new("Next", true, None);
        let prev = MenuItem::new("Previous", true, None);
        let private = CheckMenuItem::new("Private listening", true, lib.private.load(Ordering::Relaxed), None);
        let show = MenuItem::new("Show DK.FM", true, None);
        let quit = MenuItem::new("Quit DK.FM", true, None);
        let menu = Menu::new();
        menu.append_items(&[&now, &PredefinedMenuItem::separator(), &play, &next, &prev, &PredefinedMenuItem::separator(), &private, &show, &quit]).ok()?;
        let tray = TrayIconBuilder::new().with_icon(icon).with_tooltip("DK.FM").with_menu(Box::new(menu)).build().ok()?;
        let (pid, nid, vid, sid, qid, rid) = (play.id().clone(), next.id().clone(), prev.id().clone(), show.id().clone(), quit.id().clone(), private.id().clone());
        let p = player.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if e.id == pid {
                p.toggle();
            } else if e.id == nid {
                p.next(true);
            } else if e.id == vid {
                p.prev();
            } else if e.id == rid {
                lib.private.fetch_xor(true, Ordering::Relaxed);
                if let Some(ctx) = CTX.lock().as_ref() {
                    ctx.request_repaint();
                }
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
        Some(Self { tray, now, play, private })
    }

    #[cfg(target_os = "linux")]
    pub fn new(_player: Arc<Player>, _lib: Arc<crate::library::Library>) -> Option<Self> {
        None
    }

    #[allow(unused_variables)]
    pub fn update(&self, label: &str, playing: bool, private: bool) {
        #[cfg(not(target_os = "linux"))]
        {
            let l: String = label.chars().take(60).collect();
            self.now.set_text(if l.is_empty() { "Nothing playing".to_string() } else { l.clone() });
            self.play.set_text(if playing { "Pause" } else { "Play" });
            if self.private.is_checked() != private {
                self.private.set_checked(private);
            }
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
        // Linux: media keys go over D-Bus; without a session bus its thread would only fail
        let controls = MediaControls::new(cfg).ok().filter(|_| session_bus()).and_then(|mut c| {
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
            let _ = c.set_metadata(souvlaki::MediaMetadata { title: Some(title), artist: Some(artist), album: Some(album), cover_url: url.as_deref(), duration: Some(std::time::Duration::from_secs_f64(duration.max(0.0).min(1e7))) });
        }
        let prog = Some(souvlaki::MediaPosition(std::time::Duration::from_secs_f64(position.max(0.0).min(1e7))));
        let _ = c.set_playback(if playing { souvlaki::MediaPlayback::Playing { progress: prog } } else { souvlaki::MediaPlayback::Paused { progress: prog } });
    }
}

/// A D-Bus session bus to connect to (always true off Linux).
fn session_bus() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    match std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        // unix:path=/run/user/1000/bus,guid=… (other kinds of address: let zbus try)
        Ok(a) => a.split(';').any(|a| a.strip_prefix("unix:path=").map(|p| std::path::Path::new(p.split(',').next().unwrap_or(p)).exists()).unwrap_or(true)),
        Err(_) => std::env::var_os("XDG_RUNTIME_DIR").map(|d| std::path::Path::new(&d).join("bus").exists()).unwrap_or(false),
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
    /// the asset's size in bytes (from GitHub), to check the download is complete
    pub size: Option<u64>,
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
    let file = if want.is_empty() { None } else { j["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str().map(|n| n.ends_with(want) && !n.contains("setup")).unwrap_or(false))) };
    Ok(Some(Update {
        version: tag.trim_start_matches('v').into(),
        notes: j["body"].as_str().unwrap_or("").replace("\r", "").trim().to_string(),
        asset: file.and_then(|x| x["browser_download_url"].as_str()).map(String::from),
        size: file.and_then(|x| x["size"].as_u64()),
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
    // never swap in a cut-off or wrong file: the app couldn't start (or update) any more
    if let Err(e) = check_program(&new, u.size) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
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

/// A downloaded update is complete (GitHub's size) and is a program for this OS.
fn check_program(path: &std::path::Path, size: Option<u64>) -> Result<(), String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size.is_some_and(|s| s != len) || len < 1024 * 1024 {
        return Err("The download was incomplete. Please try again.".into());
    }
    let mut head = [0u8; 4];
    std::io::Read::read_exact(&mut std::fs::File::open(path).map_err(|e| e.to_string())?, &mut head).map_err(|e| e.to_string())?;
    let ok = if cfg!(windows) { head[..2] == *b"MZ" } else { head == *b"\x7fELF" };
    if ok {
        Ok(())
    } else {
        Err("The downloaded file isn't a DK.FM program. Please try again later.".into())
    }
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
        #[cfg(target_os = "linux")]
        {
            // with its .trashinfo, so file managers show it and can put it back
            let trash = dirs::data_dir().unwrap().join("Trash");
            let info = std::fs::read_dir(trash.join("info")).unwrap().flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().contains("dkfm-trash-test.txt")).max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok()).expect("trashinfo");
            let text = std::fs::read_to_string(&info).unwrap();
            assert!(text.starts_with("[Trash Info]\nPath=/") && text.contains("dkfm-trash-test.txt\nDeletionDate="), "{text}");
            let name = info.file_stem().unwrap();
            assert!(trash.join("files").join(name).exists());
            let _ = std::fs::remove_file(trash.join("files").join(name));
            let _ = std::fs::remove_file(&info);
        }
    }

    #[test]
    fn update_must_be_a_whole_program() {
        let f = std::env::temp_dir().join(format!("dkfm-update-test-{}", std::process::id()));
        let magic: &[u8] = if cfg!(windows) { b"MZ\x90\0" } else { b"\x7fELF" };
        let mut prog = magic.to_vec();
        prog.resize(2 * 1024 * 1024, 0);
        std::fs::write(&f, &prog).unwrap();
        assert!(super::check_program(&f, Some(prog.len() as u64)).is_ok());
        assert!(super::check_program(&f, None).is_ok());
        // cut off
        assert!(super::check_program(&f, Some(prog.len() as u64 + 1)).is_err());
        // an error page instead of the program
        std::fs::write(&f, b"<html>rate limited</html>").unwrap();
        assert!(super::check_program(&f, None).is_err());
        prog[..4].copy_from_slice(b"<htm");
        std::fs::write(&f, &prog).unwrap();
        assert!(super::check_program(&f, None).is_err());
        let _ = std::fs::remove_file(&f);
    }
}

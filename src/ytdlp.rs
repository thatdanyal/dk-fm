//! Download tools, fetched only the first time someone imports: yt-dlp (fast-starting folder
//! build on Windows/macOS), ffmpeg (DK.FM's own small audio-only build), and QuickJS (2 MB JavaScript runtime yt-dlp needs for YouTube).
//! yt-dlp is checked for updates daily so YouTube changes don't break downloads.
use crate::net;
use crate::store::data_dir;
use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct ToolStatus {
    pub state: String, // idle | installing | updating | ready | error
    pub version: String,
    pub error: String,
}

pub static STATUS: Mutex<ToolStatus> = Mutex::new(ToolStatus { state: String::new(), version: String::new(), error: String::new() });
static READY: AtomicBool = AtomicBool::new(false);
static ENSURING: Mutex<()> = Mutex::new(());

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

fn bin_dir() -> PathBuf {
    data_dir().join("bin")
}
fn yt_dir() -> PathBuf {
    bin_dir().join("yt-dlp")
}
pub fn yt_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        yt_dir().join("yt-dlp_macos")
    } else {
        yt_dir().join(format!("yt-dlp{EXE}"))
    }
}
pub fn ffmpeg_path() -> PathBuf {
    bin_dir().join(format!("ffmpeg{EXE}"))
}
fn qjs_path() -> PathBuf {
    bin_dir().join(format!("qjs{EXE}"))
}

fn yt_asset() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => "yt-dlp_win_arm64.zip",
        ("windows", _) => "yt-dlp_win.zip",
        ("macos", _) => "yt-dlp_macos.zip",
        ("linux", "aarch64") => "yt-dlp_linux_aarch64",
        _ => "yt-dlp_linux",
    }
}
/// A full ffmpeg (about 80 MB): for computers the small one isn't built for.
fn ffmpeg_url() -> String {
    let os = match std::env::consts::OS { "windows" => "win32", "macos" => "darwin", o => o };
    let arch = match std::env::consts::ARCH { "x86_64" => "x64", "aarch64" => "arm64", a => a };
    format!("https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1/ffmpeg-{os}-{arch}.gz")
}

/// DK.FM's own audio-only ffmpeg (about 4 MB instead of 80; scripts/build-ffmpeg.sh, published
/// by .github/workflows/ffmpeg.yml), if there's one for this computer.
const SMALL_FFMPEG: &str = "ffmpeg-audio-1";
fn small_ffmpeg_url() -> Option<String> {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "aarch64") => "darwin-arm64",
        _ => return None,
    };
    Some(format!("https://github.com/thatdanyal/dk-fm/releases/download/{SMALL_FFMPEG}/ffmpeg-audio-{target}.gz"))
}

/// Says which ffmpeg is installed (the small one's name), so it's swapped in once.
fn ffmpeg_stamp() -> PathBuf {
    bin_dir().join("ffmpeg.version")
}

/// Downloads the small ffmpeg next to the old one and checks it runs; returns its path.
fn fetch_small_ffmpeg() -> Result<PathBuf, String> {
    let url = small_ffmpeg_url().ok_or("no small ffmpeg for this computer")?;
    let new = bin_dir().join(format!("ffmpeg.new{EXE}"));
    net::download(&url, &new, true)?;
    make_exec(&new);
    let ok = command(&new).arg("-version").output().map(|o| o.status.success()).unwrap_or(false);
    if !ok {
        let _ = std::fs::remove_file(&new);
        return Err("the small ffmpeg didn't start".into());
    }
    Ok(new)
}

/// ffmpeg for a new install: the small one, else the full one.
fn install_ffmpeg() -> Result<(), String> {
    match fetch_small_ffmpeg() {
        Ok(new) => {
            std::fs::rename(&new, ffmpeg_path()).map_err(|e| e.to_string())?;
            let _ = std::fs::write(ffmpeg_stamp(), SMALL_FFMPEG);
        }
        Err(e) => {
            eprintln!("small ffmpeg: {e}; getting the full one");
            net::download(&ffmpeg_url(), &ffmpeg_path(), true)?;
            make_exec(&ffmpeg_path());
        }
    }
    Ok(())
}

/// An install with the old 80 MB ffmpeg gets the small one instead (once; if the old one is
/// busy converting a song right now, next time).
fn shrink_ffmpeg() {
    if small_ffmpeg_url().is_none() || std::fs::read_to_string(ffmpeg_stamp()).map(|s| s.trim() == SMALL_FFMPEG).unwrap_or(false) {
        return;
    }
    let Ok(new) = fetch_small_ffmpeg() else { return };
    let old = bin_dir().join(format!("ffmpeg.old{EXE}"));
    let _ = std::fs::remove_file(&old);
    if std::fs::rename(ffmpeg_path(), &old).is_ok() {
        if std::fs::rename(&new, ffmpeg_path()).is_ok() {
            let _ = std::fs::write(ffmpeg_stamp(), SMALL_FFMPEG);
            let _ = std::fs::remove_file(&old);
        } else {
            let _ = std::fs::rename(&old, ffmpeg_path());
        }
    }
    let _ = std::fs::remove_file(&new);
}
fn qjs_url() -> String {
    let name = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => "qjs-windows-x86_64.exe",
        ("macos", "aarch64") => "qjs-darwin-arm64",
        ("macos", _) => "qjs-darwin-x86_64",
        ("linux", "aarch64") => "qjs-linux-aarch64",
        _ => "qjs-linux-x86_64",
    };
    format!("https://github.com/quickjs-ng/quickjs/releases/download/v0.17.0/{name}")
}

fn set_status(state: &str, version: &str, error: &str) {
    let mut s = STATUS.lock();
    s.state = state.into();
    if !version.is_empty() {
        s.version = version.into();
    }
    s.error = error.into();
}

#[cfg(unix)]
fn make_exec(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
}
#[cfg(not(unix))]
fn make_exec(_: &Path) {}

fn install_yt(version: Option<&str>) -> Result<(), String> {
    let base = match version {
        Some(v) => format!("https://github.com/yt-dlp/yt-dlp/releases/download/{v}"),
        None => "https://github.com/yt-dlp/yt-dlp/releases/latest/download".into(),
    };
    let asset = yt_asset();
    let stage = bin_dir().join("yt-dlp.new");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    if asset.ends_with(".zip") {
        let zip_path = bin_dir().join(asset);
        net::download(&format!("{base}/{asset}"), &zip_path, false)?;
        let f = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
        let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
        z.extract(&stage).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&zip_path);
    } else {
        net::download(&format!("{base}/{asset}"), &stage.join("yt-dlp"), false)?;
    }
    prune_yt(&stage);
    let _ = std::fs::remove_dir_all(yt_dir());
    std::fs::rename(&stage, yt_dir()).map_err(|e| e.to_string())?;
    make_exec(&yt_path());
    Ok(())
}

/// Parts of yt-dlp's folder build DK.FM never uses (about 5 MB): browser imitation (curl_cffi,
/// for sites other than YouTube and SoundCloud), reading browser cookies (sqlite) and live-stream
/// chat (websockets). yt-dlp loads each only if it's there, so without them it works as before.
fn prune_yt(dir: &Path) {
    const UNUSED: [&str; 6] = ["curl_cffi", "_cffi_backend", "websockets", "sqlite3.dll", "_sqlite3", "libsqlite3"];
    let Ok(rd) = std::fs::read_dir(dir.join("_internal")) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if UNUSED.iter().any(|u| n.starts_with(u)) {
            let p = e.path();
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        }
    }
}

fn latest_version() -> Option<String> {
    net::get_json("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest", &[]).ok()?.get("tag_name")?.as_str().map(String::from)
}

/// Install / update the tools if needed. Blocking; safe to call from many threads.
pub fn ensure() -> Result<String, String> {
    if READY.load(Ordering::Relaxed) {
        return Ok(STATUS.lock().version.clone());
    }
    let _g = ENSURING.lock();
    if READY.load(Ordering::Relaxed) {
        return Ok(STATUS.lock().version.clone());
    }
    let _ = std::fs::create_dir_all(bin_dir());
    let stamp = bin_dir().join("yt-dlp.version");
    let have = if yt_path().exists() { std::fs::read_to_string(&stamp).ok().map(|s| s.trim().to_string()) } else { None };
    let check_file = bin_dir().join("last-check");
    let last: u64 = std::fs::read_to_string(&check_file).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let now = crate::store::now_secs() as u64;
    let last = if last > now * 10 { last / 1000 } else { last }; // the Electron build stored milliseconds
    let res = (|| {
        if !ffmpeg_path().exists() {
            set_status("installing", "", "");
            install_ffmpeg()?;
        } else {
            shrink_ffmpeg();
        }
        // (a yt-dlp installed before it was trimmed)
        prune_yt(&yt_dir());
        if !qjs_path().exists() {
            net::download(&qjs_url(), &qjs_path(), false)?;
            make_exec(&qjs_path());
        }
        if have.is_none() || now.saturating_sub(last) > 24 * 3600 {
            let latest = latest_version();
            if have.is_none() || (latest.is_some() && latest != have) {
                set_status(if have.is_some() { "updating" } else { "installing" }, "", "");
                match install_yt(latest.as_deref()) {
                    Ok(()) => {
                        let v = latest.clone().unwrap_or_else(|| run(&["--version"], None, None).unwrap_or_default().trim().to_string());
                        let _ = std::fs::write(&stamp, &v);
                    }
                    Err(e) if have.is_some() => eprintln!("yt-dlp update failed (keeping old copy): {e}"),
                    Err(e) => return Err(e),
                }
            }
            let _ = std::fs::write(&check_file, now.to_string());
        }
        Ok(())
    })();
    // old single-file / folder layouts from the Electron build are replaced above; nothing else to migrate
    match res {
        Ok(()) => {
            let v = std::fs::read_to_string(&stamp).unwrap_or_default().trim().to_string();
            set_status("ready", &v, "");
            READY.store(true, Ordering::Relaxed);
            Ok(v)
        }
        Err(e) => {
            set_status("error", "", &e);
            Err(e)
        }
    }
}

pub fn base_args() -> Vec<String> {
    vec![
        "--no-js-runtimes".into(),
        "--js-runtimes".into(),
        format!("quickjs:{}", qjs_path().display()),
        "--ffmpeg-location".into(),
        ffmpeg_path().display().to_string(),
        "--no-warnings".into(),
        "--encoding".into(),
        "utf-8".into(),
        "--no-cache-dir".into(),
    ]
}

pub fn command(bin: &Path) -> Command {
    let mut c = Command::new(bin);
    c.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1").stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Run yt-dlp; `on_line` gets stdout lines as they arrive; `cancel` kills it.
pub fn run(args: &[&str], mut on_line: Option<&mut dyn FnMut(&str)>, cancel: Option<&Arc<AtomicBool>>) -> Result<String, String> {
    let mut cmd = command(&yt_path());
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("could not start yt-dlp: {e}"))?;
    // cancelling stops it right away, even while it prints nothing (converting with ffmpeg)
    let running = Arc::new(AtomicBool::new(true));
    if let Some(c) = cancel.cloned() {
        let (pid, running) = (child.id(), running.clone());
        std::thread::spawn(move || {
            while running.load(Ordering::Relaxed) {
                if c.load(Ordering::Relaxed) {
                    kill_tree(pid);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });
    }
    let _done = Done(running);
    let stderr = child.stderr.take();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = stderr {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let mut out = String::new();
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
                let _ = child.kill();
                return Err("Cancelled".into());
            }
            if let Some(f) = on_line.as_mut() {
                f(&line);
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let err = err_thread.join().unwrap_or_default();
    if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
        return Err("Cancelled".into());
    }
    // --ignore-errors: some entries failed (DRM, removed…) but the listing came out
    if status.success() || (args.contains(&"--ignore-errors") && out.trim_start().starts_with('{')) {
        Ok(out)
    } else {
        let msg = err.lines().rfind(|l| l.starts_with("ERROR")).or_else(|| err.lines().last()).unwrap_or("yt-dlp failed").trim_start_matches("ERROR: ").to_string();
        Err(msg)
    }
}

/// Clears `running` when `run` returns, however it returns.
struct Done(Arc<AtomicBool>);
impl Drop for Done {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Stops a yt-dlp we started and what it started (ffmpeg), by process id.
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(0x0800_0000).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("pkill").args(["-TERM", "-P", &pid.to_string()]).status();
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    }
}

/// Convenience: yt-dlp with base args + extra args.
pub fn run_with(extra: &[String], on_line: Option<&mut dyn FnMut(&str)>, cancel: Option<&Arc<AtomicBool>>) -> Result<String, String> {
    let mut all = base_args();
    all.extend(extra.iter().cloned());
    let refs: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
    run(&refs, on_line, cancel)
}

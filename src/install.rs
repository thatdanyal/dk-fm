//! Windows setup, built into the app itself (no separate installer program).
//!
//! Started as `DK.FM-…-setup.exe` (a fresh download, or the old Electron DK.FM's updater, which
//! runs it with `--updated --force-run`), the exe copies itself to the install folder as
//! DK.FM.exe, removes the old Electron files, takes over the Apps & features entry and the
//! shortcuts, then starts DK.FM. `--uninstall` (from Apps & features) reverses that and keeps
//! the user's music, playlists and settings.
//!
//! `DKFM_INSTALL_DIR=<dir>` installs into a test folder instead: shortcuts go inside it, the
//! Apps & features entry gets a separate test id, and start-at-login is left alone.
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const EXE: &str = "DK.FM.exe";
/// The id the Electron installer registered under (derived from the app id, so the same on every
/// PC); reusing it means DK.FM shows up once in Apps & features, not twice.
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\ab07819b-c8e9-5f03-866c-b8ff431cec79";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const ELECTRON_RUN_VALUE: &str = "electron.app.DK.FM";
const NO_WINDOW: u32 = 0x0800_0000;

/// Returns true if this launch was a setup/uninstall run (and the app should not start).
pub fn handle(args: &[String]) -> bool {
    let has = |f: &str| args.iter().any(|a| a.eq_ignore_ascii_case(f));
    let Ok(me) = std::env::current_exe() else { return false };
    if has("--uninstall") {
        uninstall(&me, has("/S"));
        return true;
    }
    let name = me.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !(name.contains("setup") || has("--install")) {
        if has("--updated") {
            refresh_version(&me);
        }
        return false;
    }
    let silent = has("/S");
    match install(&me) {
        Ok(exe) => {
            if !silent || has("--force-run") {
                let _ = Command::new(&exe).spawn();
            }
        }
        Err(e) => {
            if !silent {
                message(&format!("DK.FM couldn't be installed.\n\n{e}"), false);
            }
        }
    }
    true
}

struct Target {
    dir: PathBuf,
    key: String,
    start_menu: PathBuf,
    desktop: PathBuf,
    test: bool,
}

impl Target {
    fn detect() -> Target {
        if let Ok(d) = std::env::var("DKFM_INSTALL_DIR") {
            let dir = PathBuf::from(d);
            return Target { start_menu: dir.join("_start_menu"), desktop: dir.join("_desktop"), key: format!("{UNINSTALL_KEY}-test"), dir, test: true };
        }
        let default = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("Programs").join("DK.FM");
        // reuse the folder the previous version was installed to (the Electron installer let
        // people pick one)
        let dir = reg_get(UNINSTALL_KEY, "DisplayIcon")
            .map(|v| v.trim_end_matches(",0").trim_matches('"').to_string())
            .and_then(|p| Path::new(&p).parent().map(Path::to_path_buf))
            .filter(|d| d.is_dir())
            .unwrap_or(default);
        Target {
            dir,
            key: UNINSTALL_KEY.into(),
            start_menu: dirs::data_dir().unwrap_or_default().join(r"Microsoft\Windows\Start Menu\Programs"),
            desktop: dirs::desktop_dir().unwrap_or_default(),
            test: false,
        }
    }
}

fn install(me: &Path) -> Result<PathBuf, String> {
    let mut t = Target::detect();
    if !writable(&t.dir) {
        // e.g. an old install in Program Files: install per-user instead
        t.dir = dirs::data_local_dir().ok_or("no AppData folder")?.join("Programs").join("DK.FM");
        if !writable(&t.dir) {
            return Err(format!("Can't write to {}", t.dir.display()));
        }
    }
    let exe = t.dir.join(EXE);
    if same_path(me, &exe) {
        return Ok(exe);
    }

    // the old version is quitting so it can be replaced; give it a moment, then make sure
    close_running(&t.dir, Duration::from_secs(20));
    remove_electron_files(&t.dir);

    // Windows lets a locked .exe be renamed but not overwritten
    let old = exe.with_extension("old");
    let _ = std::fs::remove_file(&old);
    if exe.exists() && std::fs::remove_file(&exe).is_err() {
        std::fs::rename(&exe, &old).map_err(|e| format!("Couldn't replace {}: {e}", exe.display()))?;
    }
    std::fs::copy(me, &exe).map_err(|e| format!("Couldn't copy DK.FM to {}: {e}", t.dir.display()))?;

    register(&t, &exe);
    // shortcuts are only refreshed where they already are: a fresh install asks first (in the
    // welcome screens, see `add_shortcuts`)
    for lnk in [t.start_menu.join("DK.FM.lnk"), t.desktop.join("DK.FM.lnk")] {
        if lnk.exists() {
            shortcut(&lnk, &exe);
        }
    }
    // the Electron version's start-at-login entry points at a program that no longer exists
    if !t.test && reg_get(RUN_KEY, ELECTRON_RUN_VALUE).is_some() {
        reg(&["delete", RUN_KEY, "/v", ELECTRON_RUN_VALUE, "/f"]);
        reg(&["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run", "/v", ELECTRON_RUN_VALUE, "/f"]);
        let _ = crate::system::set_start_at_login_for(&exe, true);
    }
    Ok(exe)
}

/// Start menu and/or desktop shortcuts to the running DK.FM, once the user said yes.
pub fn add_shortcuts(start_menu: bool, desktop: bool) {
    let Ok(exe) = std::env::current_exe() else { return };
    let t = Target::detect();
    if start_menu {
        shortcut(&t.start_menu.join("DK.FM.lnk"), &exe);
    }
    if desktop {
        shortcut(&t.desktop.join("DK.FM.lnk"), &exe);
    }
}

fn uninstall(me: &Path, silent: bool) {
    let t = Target::detect();
    if !same_path(me, &t.dir.join(EXE)) {
        if !silent {
            message("This copy of DK.FM isn't installed, so there's nothing to uninstall.\nJust delete the file.", false);
        }
        return;
    }
    if !silent && !message("Remove DK.FM from this PC?\n\nYour music files, playlists and settings are kept.", true) {
        return;
    }
    close_running(&t.dir, Duration::ZERO);
    let _ = std::fs::remove_file(t.start_menu.join("DK.FM.lnk"));
    let _ = std::fs::remove_file(t.desktop.join("DK.FM.lnk"));
    reg(&["delete", &t.key, "/f"]);
    if !t.test {
        let _ = crate::system::set_start_at_login_for(me, false);
    }
    // a running .exe can't delete itself: let cmd do it once we've exited
    let script = format!(
        "ping -n 3 127.0.0.1 >nul & del /f /q \"{exe}\" \"{old}\" & rmdir \"{dir}\"",
        exe = me.display(),
        old = me.with_extension("old").display(),
        dir = t.dir.display()
    );
    let _ = Command::new("cmd").arg("/c").raw_arg(&script).creation_flags(NO_WINDOW).spawn();
}

/// After a self-update, show the new version number in Apps & features.
fn refresh_version(me: &Path) {
    let t = Target::detect();
    if same_path(me, &t.dir.join(EXE)) {
        reg(&["add", &t.key, "/v", "DisplayVersion", "/d", env!("CARGO_PKG_VERSION"), "/f"]);
    }
}

fn register(t: &Target, exe: &Path) {
    let exe_s = exe.display().to_string();
    let size_kb = std::fs::metadata(exe).map(|m| m.len() / 1024).unwrap_or(0).to_string();
    let uninstall = format!("\"{exe_s}\" --uninstall");
    let quiet = format!("{uninstall} /S");
    let icon = format!("{exe_s},0");
    let dir = t.dir.display().to_string();
    let values: [(&str, &str, &str); 12] = [
        ("DisplayName", "REG_SZ", "DK.FM"),
        ("DisplayVersion", "REG_SZ", env!("CARGO_PKG_VERSION")),
        ("DisplayIcon", "REG_SZ", &icon),
        ("Publisher", "REG_SZ", "thatdanyal"),
        ("Comments", "REG_SZ", "Retro desktop music player"),
        ("InstallLocation", "REG_SZ", &dir),
        ("UninstallString", "REG_SZ", &uninstall),
        ("QuietUninstallString", "REG_SZ", &quiet),
        ("URLInfoAbout", "REG_SZ", "https://github.com/thatdanyal/dk-fm#readme"),
        ("EstimatedSize", "REG_DWORD", &size_kb),
        ("NoModify", "REG_DWORD", "1"),
        ("NoRepair", "REG_DWORD", "1"),
    ];
    for (name, kind, data) in values {
        reg(&["add", &t.key, "/v", name, "/t", kind, "/d", data, "/f"]);
    }
}

/// Files the Electron version installed next to DK.FM.exe. Only these are removed, never the
/// whole folder.
fn remove_electron_files(dir: &Path) {
    for d in ["locales", "resources"] {
        let _ = std::fs::remove_dir_all(dir.join(d));
    }
    let named = ["Uninstall DK.FM.exe", "LICENSE.electron.txt", "LICENSES.chromium.html", "vk_swiftshader_icd.json"];
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let ext = Path::new(&name).extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        if named.contains(&name.as_str()) || ["pak", "dll", "dat", "bin"].contains(&ext.as_str()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

// ------------------------------------------------------------------------------------- helpers

fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".dkfm-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    norm(a) == norm(b)
}

fn reg(args: &[&str]) -> bool {
    Command::new("reg").args(args).creation_flags(NO_WINDOW).output().map(|o| o.status.success()).unwrap_or(false)
}

fn reg_get(key: &str, value: &str) -> Option<String> {
    let out = Command::new("reg").args(["query", key, "/v", value]).creation_flags(NO_WINDOW).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix(value)?.trim_start();
        let (_, data) = rest.split_once(char::is_whitespace)?; // skip the REG_xx type
        Some(data.trim().to_string())
    })
}

fn shortcut(lnk: &Path, exe: &Path) {
    if let Some(p) = lnk.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let q = |p: &Path| p.display().to_string().replace('\'', "''");
    let ps = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{lnk}');$s.TargetPath='{exe}';$s.WorkingDirectory='{dir}';$s.IconLocation='{exe},0';$s.Description='DK.FM';$s.Save()",
        lnk = q(lnk),
        exe = q(exe),
        dir = q(exe.parent().unwrap_or(exe))
    );
    let _ = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &ps]).creation_flags(NO_WINDOW).output();
}

/// Waits up to `grace` for programs running from `dir` (other than this one) to exit, then
/// closes whatever is left. Matching is by full path, so nothing else on the PC is touched.
fn close_running(dir: &Path, grace: Duration) {
    let start = Instant::now();
    loop {
        let pids = running_in(dir);
        if pids.is_empty() {
            return;
        }
        if start.elapsed() >= grace {
            for pid in pids {
                terminate(pid);
            }
            std::thread::sleep(Duration::from_millis(800));
            return;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn running_in(dir: &Path) -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};

    let prefix = format!("{}\\", std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()).display()).to_lowercase().replace(r"\\?\", "");
    let me = std::process::id();
    let mut found = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return found;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            if e.th32ProcessID != me && e.th32ProcessID != 0 {
                let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, e.th32ProcessID);
                if !h.is_null() {
                    let mut buf = [0u16; 1024];
                    let mut len = buf.len() as u32;
                    if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0 {
                        let path = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
                        if path.starts_with(&prefix) {
                            found.push(e.th32ProcessID);
                        }
                    }
                    CloseHandle(h);
                }
            }
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    found
}

fn terminate(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 0);
            CloseHandle(h);
        }
    }
}

/// A plain Windows message box. With `ask`, shows Yes/No and returns true for Yes.
fn message(text: &str, ask: bool) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO};
    let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (t, c) = (w(text), w("DK.FM"));
    let flags = if ask { MB_YESNO | MB_ICONQUESTION } else { MB_OK | MB_ICONINFORMATION };
    unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), flags) == IDYES }
}

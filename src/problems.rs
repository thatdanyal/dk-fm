//! Problems: what went wrong, kept so it can be looked at and reported (Settings > Problems).
//! problems.log in the data folder has errors shown on screen and failed downloads; crash.log
//! (main.rs) has crashes. Both stay small (the latest 32 / 16 KB). COPY DEBUG INFO puts the
//! version, system, tool versions, library size and the latest of both logs on the clipboard.
use crate::store::data_dir;

const KEEP: usize = 32 * 1024;

fn path() -> std::path::PathBuf {
    data_dir().join("problems.log")
}

/// Write down a problem: `kind` is what was being done ("download", "error", …).
pub fn note(kind: &str, msg: &str) {
    let line = format!("[{}] {kind}: {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg.replace('\n', " ").trim());
    let old = std::fs::read_to_string(path()).unwrap_or_default();
    let mut cut = old.len().saturating_sub(KEEP);
    while !old.is_char_boundary(cut) {
        cut += 1;
    }
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(path(), format!("{}{line}", &old[cut..]));
}

/// The latest problems and crashes, newest first: (when, what, text).
pub fn recent(n: usize) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for (file, kind) in [(path(), ""), (data_dir().join("crash.log"), "crash")] {
        for l in std::fs::read_to_string(file).unwrap_or_default().lines() {
            let Some((when, rest)) = l.strip_prefix('[').and_then(|l| l.split_once("] ")) else { continue };
            let (what, text) = if kind.is_empty() { rest.split_once(": ").map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or(("?".into(), rest.into())) } else { (kind.to_string(), rest.to_string()) };
            out.push((when.to_string(), what, text));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(n);
    out
}

pub fn clear() {
    let _ = std::fs::remove_file(path());
    let _ = std::fs::remove_file(data_dir().join("crash.log"));
}

/// Everything useful for finding a bug, as text to paste.
pub fn debug_info(songs: usize, playlists: usize) -> String {
    let tool = |f: &str| std::fs::read_to_string(crate::ytdlp::bin_dir().join(f)).map(|s| s.trim().to_string()).unwrap_or_else(|_| "not installed".into());
    let mut s = format!(
        "DK.FM {} · {} {} · yt-dlp {} · ffmpeg {} · {songs} songs, {playlists} playlists\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        tool("yt-dlp.version"),
        tool("ffmpeg.version"),
    );
    let list = recent(60);
    if list.is_empty() {
        s += "No problems written down.\n";
    }
    for (when, what, text) in list {
        s += &format!("[{when}] {what}: {text}\n");
    }
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn notes_newest_first_with_crashes() {
        let (_g, dir) = crate::store::test_profile("problemtest");
        super::note("download", "Daft Punk - One More Time: no match\nsecond line");
        std::fs::write(dir.join("crash.log"), "[2000-01-01 10:00:00] DK.FM 2.0.0 error in thread 'x': boom\n").unwrap();
        let r = super::recent(10);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].1.as_str(), r[0].2.as_str()), ("download", "Daft Punk - One More Time: no match second line"));
        assert_eq!(r[1].1, "crash");
        assert!(super::debug_info(3, 1).contains("3 songs, 1 playlists"));
        super::clear();
        assert!(super::recent(10).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

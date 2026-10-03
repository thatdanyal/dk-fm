//! Backups of your library, playlists, play counts, history and settings: one gzip-compressed
//! JSON file each, kept in the data folder's `backups` (never the music folder). Song files and
//! cover art aren't included — covers are rebuilt from the songs. The Spotify login isn't either.
use crate::library::Library;
use crate::store::{data_dir, History, LibraryData, Settings};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const EXT: &str = "dkfmbackup";

#[derive(Serialize, Deserialize, Default)]
pub struct Backup {
    pub version: u32,
    pub created: i64,
    pub library: LibraryData,
    pub history: History,
    pub settings: Settings,
}

pub struct Entry {
    pub path: PathBuf,
    /// unix seconds
    pub created: i64,
    pub size: u64,
    /// "auto" | "manual" | "before-restore"
    pub kind: String,
}

pub fn dir() -> PathBuf {
    data_dir().join("backups")
}

/// Writes a backup file (atomically). The Spotify refresh token is left out.
pub fn write(path: &Path, lib: &LibraryData, hist: &History, s: &Settings) -> Result<u64, String> {
    let mut settings = s.clone();
    settings.spotify_refresh_token.clear();
    #[derive(Serialize)]
    struct Out<'a> {
        version: u32,
        created: i64,
        library: &'a LibraryData,
        history: &'a History,
        settings: &'a Settings,
    }
    let json = serde_json::to_vec(&Out { version: 1, created: crate::store::now_secs(), library: lib, history: hist, settings: &settings }).map_err(|e| e.to_string())?;
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&json).map_err(|e| e.to_string())?;
    let bytes = gz.finish().map_err(|e| e.to_string())?;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| format!("Couldn't save the backup: {e}"))?;
    Ok(bytes.len() as u64)
}

pub fn read(path: &Path) -> Result<Backup, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut json = Vec::new();
    GzDecoder::new(f).read_to_end(&mut json).map_err(|_| "That isn't a DK.FM backup file".to_string())?;
    let mut b: Backup = serde_json::from_slice(&json).map_err(|_| "That isn't a DK.FM backup file".to_string())?;
    if b.version == 0 {
        return Err("That isn't a DK.FM backup file".into());
    }
    if b.settings.eq.gains.len() != 10 {
        b.settings.eq.gains = vec![0.0; 10];
    }
    Ok(b)
}

/// A new backup in `dir`, named by date and kind.
pub fn create_in(dir: &Path, lib: &LibraryData, hist: &History, s: &Settings, kind: &str) -> Result<PathBuf, String> {
    let name = format!("dkfm-{}-{kind}.{EXT}", crate::store::local_stamp(crate::store::now_secs(), true));
    let path = dir.join(name);
    write(&path, lib, hist, s)?;
    Ok(path)
}

pub fn create(lib: &Library, s: &Settings, kind: &str) -> Result<PathBuf, String> {
    let (d, h) = (lib.data.read(), lib.history.read());
    create_in(&dir(), &d, &h, s, kind)
}

/// Backups in `dir`, newest first.
pub fn list_in(dir: &Path) -> Vec<Entry> {
    let mut v: Vec<Entry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == EXT).unwrap_or(false))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let created = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
            let stem = e.path().file_stem()?.to_string_lossy().into_owned();
            let kind = ["before-restore", "manual", "auto"].into_iter().find(|k| stem.ends_with(k)).unwrap_or("manual").to_string();
            Some(Entry { path: e.path(), created, size: m.len(), kind })
        })
        .collect();
    v.sort_by(|a, b| b.created.cmp(&a.created).then(b.path.cmp(&a.path)));
    v
}

pub fn list() -> Vec<Entry> {
    list_in(&dir())
}

/// Keeps the newest `keep` backups (all kinds), removing older ones.
pub fn prune_in(dir: &Path, keep: usize) {
    for e in list_in(dir).into_iter().skip(keep.max(1)) {
        let _ = std::fs::remove_file(&e.path);
    }
}

/// An automatic backup is due (newest automatic one older than a day / week).
pub fn due(dir: &Path, every: &str) -> bool {
    let period = if every == "weekly" { 7 * 86400 } else { 86400 };
    let last = list_in(dir).into_iter().filter(|e| e.kind == "auto").map(|e| e.created).max().unwrap_or(0);
    crate::store::now_secs() - last >= period - 600
}

/// Reads `from`, saves the current state as a "before-restore" backup, and returns what to load.
/// Your Spotify login stays as it is now.
pub fn restore_in(dir: &Path, from: &Path, lib: &LibraryData, hist: &History, cur: &Settings) -> Result<Backup, String> {
    let mut b = read(from)?;
    create_in(dir, lib, hist, cur, "before-restore")?;
    b.settings.spotify_refresh_token = cur.spotify_refresh_token.clone();
    b.settings.spotify_user = cur.spotify_user.clone();
    Ok(b)
}

/// Restores a backup into the running app's library and settings (the UI reapplies the rest).
pub fn restore(lib: &Library, settings: &Mutex<Settings>, from: &Path) -> Result<(), String> {
    let cur = settings.lock().clone();
    let b = {
        let (d, h) = (lib.data.read(), lib.history.read());
        restore_in(&dir(), from, &d, &h, &cur)?
    };
    lib.replace(b.library, b.history);
    *settings.lock() = b.settings;
    Ok(())
}


/// Background thread: makes the automatic backup when it's due (checked hourly).
pub fn auto_loop(lib: std::sync::Arc<Library>, settings: std::sync::Arc<Mutex<Settings>>) {
    std::thread::Builder::new()
        .name("backup".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(90)); // after the startup scan
            loop {
                let s = settings.lock().clone();
                let empty = { let d = lib.data.read(); d.tracks.is_empty() && d.playlists.is_empty() };
                if s.auto_backup && !empty && due(&dir(), &s.backup_every) && create(&lib, &s, "auto").is_ok() {
                    prune_in(&dir(), s.backup_keep as usize);
                }
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Playlist, Track};

    #[test]
    fn backup_round_trip_and_restore() {
        let dir = std::env::temp_dir().join(format!("dkfm-backup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut lib = LibraryData::default();
        lib.tracks.insert("t1".into(), Track { id: "t1".into(), path: "C:/x/a.m4a".into(), title: "Song".into(), artist: "Artist".into(), ..Default::default() });
        lib.playlists.push(Playlist { id: "p1".into(), name: "Mine".into(), track_ids: vec!["t1".into()], ..Default::default() });
        lib.stats.insert("t1".into(), crate::store::Stat { plays: 7, liked: true, ..Default::default() });
        let hist = History { events: vec![("t1".into(), 100, 180, 0)] };
        let s = Settings { theme: "amber-crt".into(), spotify_refresh_token: "SECRET".into(), spotify_user: "me".into(), ..Default::default() };

        let path = create_in(&dir, &lib, &hist, &s, "manual").unwrap();
        assert!(path.starts_with(&dir));
        // compressed, and the Spotify login isn't in it
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(&raw[..2], &[0x1f, 0x8b]);
        let b = read(&path).unwrap();
        assert_eq!(b.library.tracks["t1"].title, "Song");
        assert_eq!(b.library.playlists[0].track_ids, vec!["t1".to_string()]);
        assert_eq!(b.library.stats["t1"].plays, 7);
        assert_eq!(b.history.events.len(), 1);
        assert_eq!(b.settings.theme, "amber-crt");
        assert!(b.settings.spotify_refresh_token.is_empty());

        // restoring keeps a copy of the current state first, and today's login
        let now = Settings { spotify_refresh_token: "NEW".into(), spotify_user: "you".into(), ..Default::default() };
        let empty = LibraryData::default();
        let r = restore_in(&dir, &path, &empty, &History::default(), &now).unwrap();
        assert_eq!(r.library.tracks.len(), 1);
        assert_eq!((r.settings.spotify_refresh_token.as_str(), r.settings.spotify_user.as_str(), r.settings.theme.as_str()), ("NEW", "you", "amber-crt"));
        let l = list_in(&dir);
        assert_eq!(l.len(), 2);
        assert!(l.iter().any(|e| e.kind == "before-restore") && l.iter().any(|e| e.kind == "manual"));
        assert!(read(&l.iter().find(|e| e.kind == "before-restore").unwrap().path).unwrap().library.tracks.is_empty());

        // not a backup
        std::fs::write(dir.join("junk.dkfmbackup"), b"hello").unwrap();
        assert!(read(&dir.join("junk.dkfmbackup")).is_err());
        std::fs::remove_file(dir.join("junk.dkfmbackup")).unwrap();

        // keep N
        assert!(due(&dir, "daily"));
        std::thread::sleep(std::time::Duration::from_millis(1100));
        create_in(&dir, &lib, &hist, &s, "auto").unwrap();
        assert!(!due(&dir, "daily"));
        prune_in(&dir, 1);
        let l = list_in(&dir);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].kind, "auto");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

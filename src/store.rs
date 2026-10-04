//! Persistent data in the same folder and JSON formats the Electron version used, so upgrading
//! keeps the library, playlists, play counts, settings and listening history.
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};


/// %APPDATA%\DK.FM on Windows, ~/Library/Application Support/DK.FM on macOS, ~/.config/DK.FM on Linux.
pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("DKFM_USER_DATA") {
        return PathBuf::from(p);
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("DK.FM")
}

/// Tests: point the data folder at a fresh temp folder. The folder is one setting for the whole
/// process, so tests that use it run one at a time while they hold the returned guard.
#[cfg(test)]
pub fn test_profile(name: &str) -> (parking_lot::MutexGuard<'static, ()>, PathBuf) {
    static LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let guard = LOCK.lock();
    let dir = std::env::temp_dir().join(format!("dkfm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("DKFM_USER_DATA", &dir);
    (guard, dir)
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    let Ok(bytes) = std::fs::read(path) else { return T::default() };
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            // unreadable: set it aside instead of letting the next save overwrite it, so it can
            // still be recovered (or restored from a backup)
            if !bytes.is_empty() {
                let _ = std::fs::rename(path, path.with_extension(format!("broken-{}.json", now_secs())));
            }
            T::default()
        }
    }
}

/// Atomic write (temp file + rename) so a crash never leaves a half-written file.
pub fn save_json<T: Serialize>(path: &Path, v: &T) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let Ok(bytes) = serde_json::to_vec(v) else { return };
    // on disk before the rename, so a power cut can't leave an empty file in its place
    let written = std::fs::File::create(&tmp).and_then(|mut f| {
        std::io::Write::write_all(&mut f, &bytes)?;
        f.sync_all()
    });
    if written.is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

// ---------------------------------------------------------------- settings

fn d_true() -> bool { true }
fn d_theme() -> String { "red-retro".into() }
fn d_format() -> String { "m4a".into() }
fn d_conc() -> u32 { 3 }
fn d_sync() -> u32 { 6 }
fn d_vol() -> f32 { 0.8 }
fn d_vis() -> String { "bars".into() }
fn d_fps() -> u32 { 30 }
fn d_repeat() -> String { "off".into() }
fn d_one() -> f32 { 1.0 }
fn d_songs() -> String { "songs".into() }
fn d_ten() -> u32 { 10 }
fn d_five() -> u32 { 5 }
fn d_sources() -> Vec<String> { ["library", "songs", "youtube"].map(String::from).to_vec() }
fn d_normal() -> String { "normal".into() }
fn d_pixel() -> String { "pixel".into() }
fn d_comfy() -> String { "comfortable".into() }
fn d_all() -> String { "all".into() }
fn d_daily() -> String { "daily".into() }
fn d_custom() -> String { "custom".into() }
/// tabs a fresh install starts without (Settings > Tabs & sidebar brings them back)
fn d_hidden_tabs() -> Vec<String> { ["recent", "top"].map(String::from).to_vec() }
pub fn d_columns() -> Vec<String> { ["num", "like", "title", "artist", "time"].map(String::from).to_vec() }
pub const DEFAULT_PATTERN: &str = "{folder}/{artist} - {title}";
fn d_pattern() -> String { DEFAULT_PATTERN.into() }

/// A theme made in the theme editor: every palette colour as hex, by field name.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CustomTheme {
    pub name: String,
    #[serde(default)] pub dark: bool,
    #[serde(default)] pub colors: std::collections::BTreeMap<String, String>,
}

/// A saved panel layout (egui_dock state).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NamedLayout {
    pub name: String,
    pub dock: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerOpts {
    #[serde(default = "d_vol")] pub volume: f32,
    #[serde(default)] pub crossfade: u32,
    #[serde(default = "d_one")] pub speed: f32,
    #[serde(default = "d_vis")] pub visualizer: String,
    #[serde(default = "d_fps")] pub vis_fps: u32,
    #[serde(default)] pub shuffle: bool,
    #[serde(default = "d_repeat")] pub repeat: String,
    #[serde(default)] pub normalize: bool,
    #[serde(default = "d_true")] pub match_volume: bool,
    #[serde(default = "d_true")] pub smart_shuffle: bool,
}
impl Default for PlayerOpts {
    fn default() -> Self { serde_json::from_value(Value::Object(Map::new())).unwrap() }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Eq {
    #[serde(default)] pub enabled: bool,
    #[serde(default)] pub preset: String,
    #[serde(default)] pub gains: Vec<f32>,
    #[serde(default)] pub preamp: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Session {
    #[serde(default)] pub queue: Vec<String>,
    #[serde(default)] pub index: i64,
    #[serde(default)] pub position: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "d_theme")] pub theme: String,
    #[serde(default)] pub accent: Option<String>,
    #[serde(default)] pub scanlines: bool,
    #[serde(default = "d_true")] pub glow: bool,
    #[serde(default)] pub music_folders: Vec<String>,
    #[serde(default)] pub download_dir: String,
    #[serde(default = "d_format")] pub download_format: String,
    #[serde(default = "d_conc")] pub download_concurrency: u32,
    #[serde(default = "d_sync")] pub sync_hours: u32,
    #[serde(default)] pub spotify_client_id: String,
    #[serde(default)] pub spotify_client_secret: String,
    /// "Connect Spotify" (PKCE) refresh token and the connected account's name
    #[serde(default)] pub spotify_refresh_token: String,
    #[serde(default)] pub spotify_user: String,
    #[serde(default = "d_true")] pub auto_update: bool,
    #[serde(default = "d_true")] pub close_to_tray: bool,
    /// the window's X button: "playing" (keep playing in the tray while music plays, else quit),
    /// "tray" (always to the tray) or "quit". Empty = from the older close_to_tray switch.
    #[serde(default)] pub close_mode: String,
    #[serde(default)] pub start_at_login: bool,
    #[serde(default)] pub tray_hint_shown: bool,
    #[serde(default)] pub player: PlayerOpts,
    #[serde(default)] pub eq: Eq,
    #[serde(default)] pub session: Session,
    /// egui_dock layout (the Electron layout format isn't compatible, so it lives under a new key)
    #[serde(default)] pub dock: Option<Value>,
    /// "find new songs" in a playlist: "songs" (YouTube Music) or "youtube", and how many results
    #[serde(default = "d_songs")] pub search_source: String,
    #[serde(default = "d_ten")] pub search_results: u32,
    /// playlist songs were last added to (listed first in Ctrl+K's playlist picker)
    #[serde(default)] pub last_playlist: String,
    /// where "add songs" looks, in order: library, songs (YouTube Music), youtube, soundcloud
    #[serde(default = "d_sources")] pub search_sources: Vec<String>,
    #[serde(default)] pub hide_explicit: bool,
    /// import matching: "relaxed" | "normal" | "strict"
    #[serde(default = "d_normal")] pub match_strictness: String,
    // ---- look
    /// "pixel" (VT323 + Press Start 2P), "clean" (bundled sans) or "sys:<font file>"
    #[serde(default = "d_pixel")] pub font: String,
    /// headings and buttons keep the pixel font when another font is chosen
    #[serde(default = "d_true")] pub pixel_headings: bool,
    #[serde(default = "d_one")] pub zoom: f32,
    /// track list rows: "compact" | "comfortable"
    #[serde(default = "d_comfy")] pub density: String,
    /// lyrics text size on top of fitting the panel (Ctrl+scroll over the lyrics)
    #[serde(default = "d_one")] pub lyrics_zoom: f32,
    /// album covers next to song titles in lists
    #[serde(default = "d_true")] pub list_covers: bool,
    #[serde(default)] pub custom_themes: Vec<CustomTheme>,
    /// visualizer colours: none = follow the theme, else [low, mid, peak] hex
    #[serde(default)] pub vis_colors: Option<[String; 3]>,
    /// visualizer bar count (0 = fit the panel)
    #[serde(default)] pub vis_bars: u32,
    // ---- layouts, lists, sidebar
    #[serde(default)] pub layouts: Vec<NamedLayout>,
    #[serde(default = "d_columns")] pub columns: Vec<String>,
    /// default sort per screen ("all", "liked", ...) -> "artist" / "-plays" ("-" = descending, "" = list order)
    #[serde(default)] pub sorts: std::collections::BTreeMap<String, String>,
    #[serde(default = "d_all")] pub start_view: String,
    #[serde(default)] pub sidebar_order: Vec<String>,
    #[serde(default = "d_hidden_tabs")] pub sidebar_hidden: Vec<String>,
    // ---- downloads
    /// file name pattern under the download folder ("/" makes folders)
    #[serde(default = "d_pattern")] pub name_pattern: String,
    // ---- shortcuts: action -> "Ctrl+K" ("" = off); only changed ones are stored
    #[serde(default)] pub keys: std::collections::BTreeMap<String, String>,
    // ---- backups
    #[serde(default = "d_true")] pub auto_backup: bool,
    #[serde(default = "d_five")] pub backup_keep: u32,
    /// "daily" | "weekly"
    #[serde(default = "d_daily")] pub backup_every: String,
    // ---- playlists in the sidebar: "custom" (drag to reorder) | "name" | "added" | "played"
    #[serde(default = "d_custom")] pub playlist_sort: String,
    #[serde(default = "d_true")] pub sidebar_covers: bool,
    // ---- private listening (plays, skips and history aren't recorded while on)
    #[serde(default)] pub private_listening: bool,
    /// keep it on after a restart (else it turns off)
    #[serde(default)] pub keep_private: bool,
    // ---- discovery
    /// "Recommended" songs under each playlist (YouTube Music radio of a few of its songs)
    #[serde(default = "d_true")] pub recommend: bool,
    /// Home: new releases from your top artists (checked online at most once a day)
    #[serde(default = "d_true")] pub new_releases: bool,
    #[serde(default = "d_ten")] pub release_artists: u32,
    // ---- welcome screens
    /// the first-run welcome was finished (settings from before it existed count as done)
    #[serde(default = "d_true")] pub onboarded: bool,
    /// the "what's new" notice last shown (see ui::welcome::WHATS_NEW)
    #[serde(default)] pub whats_new_seen: String,
    /// no settings file yet when DK.FM started: a fresh install
    #[serde(skip)] pub fresh: bool,
    /// keep any keys this version doesn't know about
    #[serde(flatten)] pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self { serde_json::from_value(Value::Object(Map::new())).unwrap() }
}

impl Settings {
    pub fn close_mode(&self) -> &str {
        match self.close_mode.as_str() {
            "" if self.close_to_tray => "playing",
            "" => "quit",
            m => m,
        }
    }
    pub fn load() -> Self {
        let path = data_dir().join("settings.json");
        let fresh = !path.exists();
        let mut s: Settings = load_json(&path);
        s.fresh = fresh;
        if fresh {
            s.onboarded = false;
        }
        let music = dirs::audio_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Music"));
        if s.music_folders.is_empty() && !s.extra.contains_key("musicFoldersSet") {
            s.music_folders = vec![music.to_string_lossy().into_owned()];
        }
        if s.download_dir.is_empty() {
            s.download_dir = music.join("DK.FM").to_string_lossy().into_owned();
        }
        if s.eq.gains.len() != 10 {
            s.eq.gains = vec![0.0; 10];
        }
        if s.eq.preset.is_empty() {
            s.eq.preset = "Flat".into();
        }
        s
    }
    pub fn save(&self) {
        save_json(&data_dir().join("settings.json"), self);
    }
}

// ---------------------------------------------------------------- library data

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub path: String,
    #[serde(default)] pub title: String,
    #[serde(default)] pub artist: String,
    #[serde(default)] pub album: String,
    #[serde(default)] pub album_artist: String,
    #[serde(default)] pub year: Option<u32>,
    #[serde(default)] pub track: Option<u32>,
    #[serde(default)] pub genre: String,
    #[serde(default)] pub duration: f64,
    #[serde(default)] pub bitrate: Option<u32>,
    #[serde(default)] pub codec: String,
    #[serde(default)] pub sample_rate: Option<u32>,
    #[serde(default)] pub cover: Option<String>,
    #[serde(default)] pub thumb: Option<String>,
    #[serde(default)] pub mtime: f64,
    #[serde(default)] pub size: u64,
    #[serde(default)] pub added_at: f64,
    #[serde(default)] pub v: u32,
    #[serde(default)] pub gain: Option<f32>,
    /// loudness trim measured by the native analyser (full-band; not comparable with `gain`)
    #[serde(default, skip_serializing_if = "Option::is_none")] pub gain_v2: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub spotify_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub youtube_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub source_key: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    #[serde(default)] pub track_ids: Vec<String>,
    #[serde(default)] pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub spotify_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub cover: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub auto_sync: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub last_sync: Option<f64>,
    #[serde(default)] pub created_at: f64,
    /// you changed this synced playlist in DK.FM: sync then only adds new songs from the source
    #[serde(default, skip_serializing_if = "std::ops::Not::not")] pub edited: bool,
    /// songs you removed from it (song keys, so sync never brings them back)
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub removed: Vec<String>,
    /// your own cover (a file in the covers folder): shown over the source's cover and the mosaic
    #[serde(default, skip_serializing_if = "Option::is_none")] pub custom_cover: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")] pub description: String,
    /// the sidebar folder it's in
    #[serde(default, skip_serializing_if = "Option::is_none")] pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")] pub pinned: bool,
    /// when you last played it (unix ms)
    #[serde(default, skip_serializing_if = "is_zero")] pub last_played: f64,
}

fn is_zero(v: &f64) -> bool { *v == 0.0 }

/// A folder of playlists in the sidebar.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Folder {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")] pub collapsed: bool,
}

impl Playlist {
    pub fn is_imported(&self) -> bool {
        self.source_url.is_some() || self.spotify_url.is_some()
    }
    pub fn url(&self) -> Option<&str> {
        self.source_url.as_deref().or(self.spotify_url.as_deref())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stat {
    #[serde(default)] pub plays: u32,
    #[serde(default)] pub liked: bool,
    #[serde(default)] pub last_played: f64,
    #[serde(default)] pub skips: u32,
    /// "Don't play this": skipped by shuffle, playlists and radio
    #[serde(default, skip_serializing_if = "std::ops::Not::not")] pub hidden: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LibraryData {
    #[serde(default)] pub tracks: HashMap<String, Track>,
    #[serde(default)] pub playlists: Vec<Playlist>,
    #[serde(default)] pub stats: HashMap<String, Stat>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub folders: Vec<Folder>,
    /// playlists were dragged into your own order (before that: Liked Songs, imported, then yours)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")] pub custom_order: bool,
}

/// History rows: [trackId, startedAt (unix s), listened (s), skipped (0/1)].
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct History {
    #[serde(default)] pub events: Vec<(String, i64, i64, u8)>,
}


/// Local "2026-10-03" for a unix time, plus "_142501" when `time` (no strftime: smaller binary).
pub fn local_stamp(secs: i64, time: bool) -> String {
    use chrono::{Datelike, TimeZone, Timelike};
    let d = chrono::Local.timestamp_opt(secs, 0).single().unwrap_or_else(chrono::Local::now);
    let date = format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day());
    if time { format!("{date}_{:02}{:02}{:02}", d.hour(), d.minute(), d.second()) } else { date }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_file_is_kept_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("dkfm-jsontest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("library.json");
        let mut lib = LibraryData::default();
        lib.playlists.push(Playlist { id: "p".into(), name: "Mine".into(), ..Default::default() });
        save_json(&path, &lib);
        assert_eq!(load_json::<LibraryData>(&path).playlists.len(), 1);
        assert!(!dir.join("library.json.tmp").exists());
        // cut off in the middle: loads empty, and the broken file is set aside
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(load_json::<LibraryData>(&path).playlists.is_empty());
        assert!(!path.exists());
        let kept: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(kept.iter().any(|n| n.starts_with("library.broken-") && n.ends_with(".json")), "{kept:?}");
        // a missing file is just empty
        assert!(load_json::<LibraryData>(&dir.join("nope.json")).tracks.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

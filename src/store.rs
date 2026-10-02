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

pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Atomic write (temp file + rename) so a crash never leaves a half-written file.
pub fn save_json<T: Serialize>(path: &Path, v: &T) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(bytes) = serde_json::to_vec(v) {
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
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
    #[serde(default = "d_true")] pub scanlines: bool,
    #[serde(default = "d_true")] pub glow: bool,
    #[serde(default)] pub music_folders: Vec<String>,
    #[serde(default)] pub download_dir: String,
    #[serde(default = "d_format")] pub download_format: String,
    #[serde(default = "d_conc")] pub download_concurrency: u32,
    #[serde(default = "d_sync")] pub sync_hours: u32,
    #[serde(default)] pub spotify_client_id: String,
    #[serde(default)] pub spotify_client_secret: String,
    #[serde(default = "d_true")] pub auto_update: bool,
    #[serde(default = "d_true")] pub close_to_tray: bool,
    #[serde(default)] pub start_at_login: bool,
    #[serde(default)] pub tray_hint_shown: bool,
    #[serde(default)] pub player: PlayerOpts,
    #[serde(default)] pub eq: Eq,
    #[serde(default)] pub session: Session,
    /// egui_dock layout (the Electron layout format isn't compatible, so it lives under a new key)
    #[serde(default)] pub dock: Option<Value>,
    /// keep any keys this version doesn't know about
    #[serde(flatten)] pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self { serde_json::from_value(Value::Object(Map::new())).unwrap() }
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Settings = load_json(&data_dir().join("settings.json"));
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
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LibraryData {
    #[serde(default)] pub tracks: HashMap<String, Track>,
    #[serde(default)] pub playlists: Vec<Playlist>,
    #[serde(default)] pub stats: HashMap<String, Stat>,
}

/// History rows: [trackId, startedAt (unix s), listened (s), skipped (0/1)].
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct History {
    #[serde(default)] pub events: Vec<(String, i64, i64, u8)>,
}


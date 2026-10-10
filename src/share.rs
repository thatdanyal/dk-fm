//! Sharing with friends, no server or accounts: a theme, layout, settings or playlist becomes a
//! text code (`DKFM1:<type>:<base64url of deflated JSON>`) to paste into a chat, or a `.dkfm`
//! file (the same JSON) for big playlists; plus M3U export. Everything received is validated and
//! size-limited here, and only shown in a preview until you confirm. Settings codes are built
//! from a whitelist of look & behaviour options: never the Spotify login, folders, stats or library.
use crate::sources::ITrack;
use crate::store::{CustomTheme, Eq, NamedLayout, Settings, Track};
use crate::ui::{browser, keys, scope, theme};
use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

pub const PREFIX: &str = "DKFM1:";
pub const EXT: &str = "dkfm";
/// longest text accepted as a code (bigger playlists go in a file)
pub const MAX_CODE: usize = 1 << 20;
pub const MAX_FILE: u64 = 8 << 20;
/// unpacked JSON limit, so a small code can't expand into gigabytes
const MAX_JSON: u64 = 8 << 20;
pub const MAX_SONGS: usize = 10_000;
const BAD: &str = "That DK.FM code is damaged or incomplete. Ask your friend to copy it again.";

/// A song in a shared playlist (short keys keep codes small).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Song {
    #[serde(rename = "t")]
    pub title: String,
    #[serde(rename = "a", default)]
    pub artist: String,
    #[serde(rename = "d", default)]
    pub secs: u32,
    #[serde(rename = "sp", default, skip_serializing_if = "Option::is_none")]
    pub spotify: Option<String>,
    #[serde(rename = "yt", default, skip_serializing_if = "Option::is_none")]
    pub youtube: Option<String>,
    /// source key ("sc:123", ...) when it isn't just the Spotify / YouTube id
    #[serde(rename = "k", default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SharedPlaylist {
    pub name: String,
    pub songs: Vec<Song>,
}

/// Look & behaviour settings that make sense on a friend's computer.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct SharedSettings {
    pub theme: String,
    pub accent: Option<String>,
    /// the custom theme in use, if any
    pub custom_themes: Vec<CustomTheme>,
    pub scanlines: bool,
    pub glow: bool,
    /// "pixel" | "clean" (a font file on your computer isn't shared)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    pub pixel_headings: bool,
    pub zoom: f32,
    pub density: String,
    pub vis_colors: Option<[String; 3]>,
    pub vis_bars: u32,
    pub visualizer: String,
    pub vis_fps: u32,
    pub crossfade: u32,
    pub normalize: bool,
    pub match_volume: bool,
    pub smart_shuffle: bool,
    pub eq: Eq,
    pub columns: Vec<String>,
    pub sorts: BTreeMap<String, String>,
    pub start_view: String,
    pub sidebar_order: Vec<String>,
    pub sidebar_hidden: Vec<String>,
    pub keys: BTreeMap<String, String>,
    pub search_source: String,
    pub search_results: u32,
    pub search_sources: Vec<String>,
    pub hide_explicit: bool,
    pub match_strictness: String,
}

impl Default for SharedSettings {
    fn default() -> Self {
        Self::from(&Settings::default())
    }
}

impl From<&Settings> for SharedSettings {
    fn from(s: &Settings) -> Self {
        let custom = s.theme.strip_prefix(theme::CUSTOM).and_then(|n| s.custom_themes.iter().find(|c| c.name == n)).cloned();
        Self {
            theme: s.theme.clone(),
            accent: s.accent.clone(),
            custom_themes: custom.into_iter().collect(),
            scanlines: s.scanlines,
            glow: s.glow,
            font: Some(s.font.clone()).filter(|f| f == "pixel" || f == "clean"),
            pixel_headings: s.pixel_headings,
            zoom: s.zoom,
            density: s.density.clone(),
            vis_colors: s.vis_colors.clone(),
            vis_bars: s.vis_bars,
            visualizer: s.player.visualizer.clone(),
            vis_fps: s.player.vis_fps,
            crossfade: s.player.crossfade,
            normalize: s.player.normalize,
            match_volume: s.player.match_volume,
            smart_shuffle: s.player.smart_shuffle,
            eq: s.eq.clone(),
            columns: s.columns.clone(),
            sorts: s.sorts.clone(),
            start_view: s.start_view.clone(),
            sidebar_order: s.sidebar_order.clone(),
            sidebar_hidden: s.sidebar_hidden.clone(),
            keys: s.keys.clone(),
            search_source: s.search_source.clone(),
            search_results: s.search_results,
            search_sources: s.search_sources.clone(),
            hide_explicit: s.hide_explicit,
            match_strictness: s.match_strictness.clone(),
        }
    }
}

impl SharedSettings {
    /// Drops or fixes anything a current DK.FM wouldn't accept.
    fn sanitize(mut self) -> Self {
        let d = Settings::default();
        let one_of = |v: &mut String, ok: &[&str], def: &str| {
            if !ok.contains(&v.as_str()) {
                *v = def.to_string();
            }
        };
        self.custom_themes = self.custom_themes.into_iter().take(1).filter_map(clean_theme).collect();
        let custom_ok = self.theme.strip_prefix(theme::CUSTOM).map(|n| self.custom_themes.iter().any(|c| c.name == n));
        if !(custom_ok == Some(true) || custom_ok.is_none() && theme::THEMES.iter().any(|t| t.0 == self.theme)) {
            self.theme = d.theme.clone();
        }
        self.accent = self.accent.filter(|a| theme::parse_hex(a).is_some());
        self.font = self.font.filter(|f| f == "pixel" || f == "clean");
        self.zoom = if self.zoom.is_finite() { self.zoom.clamp(0.9, 1.5) } else { 1.0 };
        one_of(&mut self.density, &["compact", "comfortable"], &d.density);
        self.vis_colors = self.vis_colors.filter(|c| c.iter().all(|h| theme::parse_hex(h).is_some()));
        self.vis_bars = self.vis_bars.min(128);
        if !scope::MODES.iter().any(|m| m.0 == self.visualizer) {
            self.visualizer = d.player.visualizer.clone();
        }
        self.vis_fps = self.vis_fps.clamp(10, 60);
        self.crossfade = self.crossfade.min(12);
        self.eq.gains.resize(10, 0.0);
        for g in self.eq.gains.iter_mut().chain([&mut self.eq.preamp]) {
            *g = if g.is_finite() { g.clamp(-12.0, 12.0) } else { 0.0 };
        }
        self.eq.preset = short(&self.eq.preset, 40);
        let mut cols: Vec<String> = Vec::new();
        for c in self.columns {
            if browser::COLUMNS.iter().any(|x| x.id == c) && !cols.contains(&c) {
                cols.push(c);
            }
        }
        self.columns = if cols.iter().any(|c| c == "title") { cols } else { d.columns.clone() };
        self.sorts.retain(|k, v| browser::SCREENS.iter().any(|s| s.0 == k) && (v.is_empty() || browser::parse_sort(v).map(|p| browser::sort_text(Some(p)) == *v).unwrap_or(false)));
        if browser::view_for(&self.start_view).is_none() {
            self.start_view = d.start_view.clone();
        }
        let side = |v: &mut Vec<String>| {
            v.retain(|x| browser::NAV.iter().any(|s| s.0 == x));
            v.dedup();
        };
        side(&mut self.sidebar_order);
        side(&mut self.sidebar_hidden);
        self.keys.retain(|a, k| keys::ACTIONS.iter().any(|x| x.0 == a) && (k.is_empty() || keys::Combo::parse(k).is_some()));
        self.search_sources.retain(|k| ["library", "songs", "youtube", "soundcloud"].contains(&k.as_str()));
        self.search_sources.dedup();
        if self.search_sources.is_empty() {
            self.search_sources = d.search_sources.clone();
        }
        one_of(&mut self.search_source, &["songs", "youtube", "soundcloud"], &d.search_source);
        self.search_results = self.search_results.clamp(1, 50);
        one_of(&mut self.match_strictness, &["relaxed", "normal", "strict"], &d.match_strictness);
        self
    }

    /// Puts these into your settings (the shared custom theme is added to your themes). Your
    /// login, folders, library, backups and window stay as they are.
    pub fn apply(&self, s: &mut Settings) {
        s.theme = match self.theme.strip_prefix(theme::CUSTOM) {
            Some(_) => match self.custom_themes.first() {
                Some(c) => add_theme(s, c.clone()),
                None => s.theme.clone(),
            },
            None => self.theme.clone(),
        };
        s.accent = self.accent.clone();
        s.scanlines = self.scanlines;
        s.glow = self.glow;
        if let Some(f) = &self.font {
            s.font = f.clone();
        }
        s.pixel_headings = self.pixel_headings;
        s.zoom = self.zoom;
        s.density = self.density.clone();
        s.vis_colors = self.vis_colors.clone();
        s.vis_bars = self.vis_bars;
        s.player.visualizer = self.visualizer.clone();
        s.player.vis_fps = self.vis_fps;
        s.player.crossfade = self.crossfade;
        s.player.normalize = self.normalize;
        s.player.match_volume = self.match_volume;
        s.player.smart_shuffle = self.smart_shuffle;
        s.eq = self.eq.clone();
        s.columns = self.columns.clone();
        s.sorts = self.sorts.clone();
        s.start_view = self.start_view.clone();
        s.sidebar_order = self.sidebar_order.clone();
        s.sidebar_hidden = self.sidebar_hidden.clone();
        s.keys = self.keys.clone();
        s.search_source = self.search_source.clone();
        s.search_results = self.search_results;
        s.search_sources = self.search_sources.clone();
        s.hide_explicit = self.hide_explicit;
        s.match_strictness = self.match_strictness.clone();
    }
}

/// Your whole DK.FM as a file: playlists (in order), liked songs, every other song, settings,
/// themes and layouts — to move to a new PC, or give a friend a head start. No music files: the
/// songs download again on the other PC, by their exact YouTube / Spotify ids. Never the Spotify
/// login, folders, play counts, history or backups.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct SharedAll {
    pub name: String,
    pub settings: Option<SharedSettings>,
    pub themes: Vec<CustomTheme>,
    pub layouts: Vec<NamedLayout>,
    /// the layout in use
    pub dock: Option<Value>,
    pub playlists: Vec<SharedPlaylist>,
    pub liked: Vec<Song>,
    /// the rest of the library: songs in no playlist and not liked
    pub songs: Vec<Song>,
}

/// Most songs a whole-DK.FM file brings in.
pub const MAX_ALL_SONGS: usize = 60_000;

impl SharedAll {
    pub fn of(name: &str, s: &Settings, d: &crate::store::LibraryData) -> SharedAll {
        let mut used = std::collections::HashSet::new();
        let tracks = |ids: &mut dyn Iterator<Item = &String>| -> Vec<Track> { ids.filter_map(|id| d.tracks.get(id)).cloned().collect() };
        let playlists: Vec<SharedPlaylist> = d
            .playlists
            .iter()
            .map(|p| {
                used.extend(p.track_ids.iter().cloned());
                playlist_of(&p.name, &tracks(&mut p.track_ids.iter()))
            })
            .filter(|p| !p.songs.is_empty())
            .collect();
        let liked_ids: Vec<&String> = d.stats.iter().filter(|(id, st)| st.liked && d.tracks.contains_key(*id)).map(|(id, _)| id).collect();
        used.extend(liked_ids.iter().map(|i| (*i).clone()));
        let liked = tracks(&mut liked_ids.into_iter()).iter().map(Song::of).filter(|s| !s.title.is_empty()).collect();
        let mut rest: Vec<&Track> = d.tracks.values().filter(|t| !used.contains(&t.id)).collect();
        rest.sort_by_cached_key(|t| (t.artist.to_lowercase(), t.title.to_lowercase()));
        SharedAll {
            name: short(name, 100),
            settings: Some(SharedSettings::from(s)),
            themes: s.custom_themes.clone(),
            layouts: s.layouts.clone(),
            dock: s.dock.clone(),
            playlists,
            liked,
            songs: rest.into_iter().map(Song::of).filter(|s| !s.title.is_empty()).collect(),
        }
    }

    pub fn song_count(&self) -> usize {
        self.playlists.iter().map(|p| p.songs.len()).sum::<usize>() + self.liked.len() + self.songs.len()
    }
}

#[derive(Debug)]
pub enum Share {
    Theme(CustomTheme),
    Layout(NamedLayout),
    Settings(Box<SharedSettings>),
    Playlist(SharedPlaylist),
    /// one of your THEATER versions
    Theater(crate::store::TheaterPreset),
    /// your whole DK.FM (files only: too big for a code)
    Everything(Box<SharedAll>),
}

impl Share {
    pub fn kind(&self) -> &'static str {
        match self {
            Share::Theme(_) => "theme",
            Share::Layout(_) => "layout",
            Share::Settings(_) => "settings",
            Share::Playlist(_) => "playlist",
            Share::Theater(_) => "theater",
            Share::Everything(_) => "all",
        }
    }
    fn json(&self) -> Value {
        match self {
            Share::Theme(t) => serde_json::to_value(t),
            Share::Layout(l) => serde_json::to_value(l),
            Share::Settings(s) => serde_json::to_value(s),
            Share::Playlist(p) => serde_json::to_value(p),
            Share::Theater(t) => serde_json::to_value(t),
            Share::Everything(a) => serde_json::to_value(a),
        }
        .unwrap_or(Value::Null)
    }
}

/// The text code for something to share.
pub fn encode(s: &Share) -> String {
    let json = serde_json::to_vec(&s.json()).unwrap_or_default();
    let mut z = DeflateEncoder::new(Vec::new(), Compression::best());
    let _ = z.write_all(&json);
    format!("{PREFIX}{}:{}", s.kind(), crate::spotify_auth::b64(&z.finish().unwrap_or_default(), true))
}

/// The code inside a message ("here: DKFM1:theme:abc… enjoy" -> "DKFM1:theme:abc…").
pub fn find(text: &str) -> Option<&str> {
    let rest = &text[text.find(PREFIX)?..];
    let end = rest.char_indices().skip(PREFIX.len()).find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))).map(|(i, _)| i).unwrap_or(rest.len());
    Some(&rest[..end])
}

pub fn decode(text: &str) -> Result<Share, String> {
    if text.len() > MAX_CODE {
        return Err("That code is too big to paste. Ask your friend for the .dkfm file instead.".into());
    }
    let code = find(text).ok_or("There's no DK.FM code in that text.")?;
    let (kind, data) = code[PREFIX.len()..].split_once(':').ok_or(BAD)?;
    let bytes = unb64(data).filter(|b| !b.is_empty()).ok_or(BAD)?;
    let mut json = Vec::new();
    DeflateDecoder::new(&bytes[..]).take(MAX_JSON + 1).read_to_end(&mut json).map_err(|_| BAD)?;
    if json.len() as u64 > MAX_JSON {
        return Err("That code unpacks to something far too big, so it was ignored.".into());
    }
    parse(kind, serde_json::from_slice(&json).map_err(|_| BAD)?)
}

/// `.dkfm` file contents: `{"dkfm":1,"type":"playlist","data":{…}}`.
pub fn file_json(s: &Share) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "dkfm": 1, "type": s.kind(), "data": s.json() })).unwrap_or_default()
}

/// A `.dkfm` file (or a text file holding a code).
pub fn read_file(path: &Path) -> Result<Share, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_FILE {
        return Err("That file is too big to be a DK.FM share.".into());
    }
    from_file_bytes(&std::fs::read(path).map_err(|e| e.to_string())?)
}

pub fn from_file_bytes(b: &[u8]) -> Result<Share, String> {
    const NOT: &str = "That isn't a DK.FM share file.";
    if b.len() as u64 > MAX_FILE {
        return Err(NOT.into());
    }
    let text = std::str::from_utf8(b).map_err(|_| NOT)?;
    if text.trim_start().starts_with(PREFIX) {
        return decode(text.trim());
    }
    let v: Value = serde_json::from_str(text).map_err(|_| NOT)?;
    if v["dkfm"].as_u64().unwrap_or(0) == 0 {
        return Err(NOT.into());
    }
    parse(v["type"].as_str().unwrap_or(""), v["data"].clone())
}

fn parse(kind: &str, v: Value) -> Result<Share, String> {
    match kind {
        "theme" => serde_json::from_value(v).ok().and_then(clean_theme).map(Share::Theme).ok_or_else(|| BAD.into()),
        "layout" => {
            let mut l: NamedLayout = serde_json::from_value(v).map_err(|_| BAD)?;
            fix_rects(&mut l.dock);
            if let Some(m) = l.dock.as_object_mut() {
                // the panel menus' texts are always our own, never a friend's
                m.insert("translations".into(), serde_json::to_value(egui_dock::Translations::english()).unwrap_or_default());
            }
            crate::ui::load_dock(&l.dock).ok_or(BAD)?;
            Ok(Share::Layout(NamedLayout { name: short(&l.name, 60).or_if_empty("Shared layout"), dock: l.dock }))
        }
        "theater" => {
            let t: crate::store::TheaterPreset = serde_json::from_value(v).map_err(|_| BAD)?;
            Ok(Share::Theater(clean_theater(t)))
        }
        "settings" => serde_json::from_value::<SharedSettings>(v).map(|s| Share::Settings(Box::new(s.sanitize()))).map_err(|_| BAD.into()),
        "playlist" => {
            let p: SharedPlaylist = serde_json::from_value(v).map_err(|_| BAD)?;
            if p.songs.len() > MAX_SONGS {
                return Err(format!("That playlist has more than {MAX_SONGS} songs, which is more than DK.FM imports at once."));
            }
            let songs: Vec<Song> = p.songs.into_iter().filter_map(clean_song).collect();
            if songs.is_empty() {
                return Err("That shared playlist has no songs.".into());
            }
            Ok(Share::Playlist(SharedPlaylist { name: short(&p.name, 200).or_if_empty("Shared playlist"), songs }))
        }
        "all" => {
            let a: SharedAll = serde_json::from_value(v).map_err(|_| BAD)?;
            if a.song_count() > MAX_ALL_SONGS || a.playlists.len() > 2000 {
                return Err(format!("That DK.FM has more than {MAX_ALL_SONGS} songs, which is more than DK.FM brings in at once."));
            }
            let layouts: Vec<NamedLayout> = a
                .layouts
                .into_iter()
                .take(100)
                .filter_map(|mut l| {
                    fix_rects(&mut l.dock);
                    crate::ui::load_dock(&l.dock)?;
                    Some(NamedLayout { name: short(&l.name, 60).or_if_empty("Shared layout"), dock: l.dock })
                })
                .collect();
            let dock = a.dock.and_then(|mut d| {
                fix_rects(&mut d);
                crate::ui::load_dock(&d).map(|_| d)
            });
            let songs = |v: Vec<Song>| v.into_iter().filter_map(clean_song).take(MAX_SONGS * 3).collect::<Vec<_>>();
            Ok(Share::Everything(Box::new(SharedAll {
                name: short(&a.name, 100).or_if_empty("a DK.FM"),
                settings: a.settings.map(|s| s.sanitize()),
                themes: a.themes.into_iter().take(100).filter_map(clean_theme).collect(),
                layouts,
                dock,
                playlists: a.playlists.into_iter().map(|p| SharedPlaylist { name: short(&p.name, 200).or_if_empty("Playlist"), songs: p.songs.into_iter().filter_map(clean_song).take(MAX_SONGS).collect() }).filter(|p| !p.songs.is_empty()).collect(),
                liked: songs(a.liked),
                songs: songs(a.songs),
            })))
        }
        _ => Err("That code needs a newer DK.FM. Update DK.FM (Settings → Updates) and paste it again.".into()),
    }
}

// ------------------------------------------------------------------------------- validation

/// Printable text, at most `max` characters.
/// A friend's THEATER version, keeping only parts and settings DK.FM knows.
fn clean_theater(t: crate::store::TheaterPreset) -> crate::store::TheaterPreset {
    let mut seen = std::collections::HashSet::new();
    let mut keep = |v: Vec<String>| -> Vec<String> { v.into_iter().filter(|k| crate::ui::nowplaying::PARTS.iter().any(|p| p.0 == k) && seen.insert(k.clone())).collect() };
    let left = keep(t.left);
    let right = keep(t.right);
    crate::store::TheaterPreset {
        name: short(&t.name, 20).to_uppercase().or_if_empty("SHARED"),
        left,
        right,
        backdrop: if ["plain", "glow", "cover"].contains(&t.backdrop.as_str()) { t.backdrop } else { "plain".into() },
        lyrics_size: if t.lyrics_size.is_finite() { t.lyrics_size.clamp(0.7, 2.6) } else { 1.35 },
        auto_hide: t.auto_hide,
        singalong: t.singalong,
    }
}

fn short(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

trait OrIfEmpty {
    fn or_if_empty(self, d: &str) -> String;
}
impl OrIfEmpty for String {
    fn or_if_empty(self, d: &str) -> String {
        if self.is_empty() { d.into() } else { self }
    }
}

fn id_ok(s: &str, len: std::ops::RangeInclusive<usize>) -> bool {
    len.contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn yt_ok(s: &String) -> bool {
    id_ok(s, 11..=11)
}
fn sp_ok(s: &String) -> bool {
    s.len() == 22 && s.bytes().all(|c| c.is_ascii_alphanumeric())
}
fn key_ok(s: &String) -> bool {
    ["sp:", "yt:", "sc:"].iter().any(|p| s.strip_prefix(p).map(|r| id_ok(r, 1..=64)).unwrap_or(false))
}

fn clean_theme(t: CustomTheme) -> Option<CustomTheme> {
    let colors: BTreeMap<String, String> = t.colors.into_iter().filter(|(k, v)| theme::FIELDS.iter().any(|f| f.0 == k) && theme::parse_hex(v).is_some()).collect();
    (!colors.is_empty()).then(|| CustomTheme { name: short(&t.name, 60).or_if_empty("Shared theme"), dark: t.dark, colors })
}

fn clean_song(s: Song) -> Option<Song> {
    let title = short(&s.title, 300);
    (!title.is_empty()).then(|| Song {
        title,
        artist: short(&s.artist, 300),
        secs: s.secs.min(86_400),
        spotify: s.spotify.filter(sp_ok),
        youtube: s.youtube.filter(yt_ok),
        key: s.key.filter(key_ok),
    })
}

/// A dock layout that was never drawn has infinite rectangles, which JSON writes as null: make
/// them 0 so it loads again (egui_dock recomputes them anyway).
pub fn fix_rects(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if x.is_null() && (k == "x" || k == "y") {
                    *x = Value::from(0.0);
                } else {
                    fix_rects(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(fix_rects),
        _ => {}
    }
}

/// Adds a received theme to your themes and returns its key; a different theme with the same
/// name is never overwritten (the new one becomes "Name (2)").
pub fn add_theme(s: &mut Settings, mut t: CustomTheme) -> String {
    let base = t.name.clone();
    let mut n = 1;
    loop {
        match s.custom_themes.iter().find(|c| c.name == t.name) {
            Some(c) if c.colors == t.colors && c.dark == t.dark => break,
            Some(_) => {
                n += 1;
                t.name = format!("{base} ({n})");
            }
            None => {
                s.custom_themes.push(t.clone());
                break;
            }
        }
    }
    format!("{}{}", theme::CUSTOM, t.name)
}

/// Same for a layout: returns the name it's saved under.
pub fn add_layout(s: &mut Settings, mut l: NamedLayout) -> String {
    let base = l.name.clone();
    let mut n = 1;
    loop {
        let taken = crate::ui::App::PRESETS.contains(&l.name.as_str());
        match s.layouts.iter().find(|x| x.name == l.name) {
            Some(x) if x.dock == l.dock => break,
            None if !taken => {
                s.layouts.push(l.clone());
                break;
            }
            _ => {
                n += 1;
                l.name = format!("{base} ({n})");
            }
        }
    }
    l.name
}

// ------------------------------------------------------------------------------- playlists

impl Song {
    pub fn of(t: &Track) -> Song {
        let spotify = t.spotify_id.clone().filter(sp_ok);
        let youtube = t.youtube_id.clone().filter(yt_ok);
        // the source key only when it adds something
        let key = t.source_key.clone().filter(key_ok).filter(|k| Some(k.as_str()) != spotify.as_ref().map(|s| format!("sp:{s}")).as_deref() && Some(k.as_str()) != youtube.as_ref().map(|y| format!("yt:{y}")).as_deref());
        Song { title: short(&t.title, 300), artist: short(&t.artist, 300), secs: t.duration.max(0.0).round().min(86_400.0) as u32, spotify, youtube, key }
    }

    /// Every key the library might know this song by.
    pub fn keys(&self) -> Vec<String> {
        crate::library::track_keys(self.key.as_deref(), self.spotify.as_deref(), self.youtube.as_deref(), &self.artist, &self.title)
    }

    /// For the downloader: songs with a YouTube id download that video; the rest are matched
    /// on YouTube Music like Spotify imports.
    pub fn to_itrack(&self) -> ITrack {
        ITrack {
            title: self.title.clone(),
            artists: self.artist.split(", ").filter(|a| !a.trim().is_empty()).map(String::from).collect(),
            duration_ms: (self.secs > 0).then(|| self.secs as u64 * 1000),
            spotify_id: self.spotify.clone(),
            youtube_id: self.youtube.clone(),
            direct_url: self.youtube.as_ref().map(|y| format!("https://www.youtube.com/watch?v={y}")),
            source_key: self.key.clone().or_else(|| self.youtube.as_ref().map(|y| format!("yt:{y}"))).or_else(|| self.spotify.as_ref().map(|s| format!("sp:{s}"))).unwrap_or_default(),
            ..Default::default()
        }
    }
}

pub fn playlist_of(name: &str, tracks: &[Track]) -> SharedPlaylist {
    SharedPlaylist { name: short(name, 200), songs: tracks.iter().map(Song::of).filter(|s| !s.title.is_empty()).take(MAX_SONGS).collect() }
}

/// Extended M3U: `#EXTINF:<seconds>,<artist> - <title>` then the file. Songs inside `dir` (the
/// folder the playlist is saved in) get paths relative to it, so the folder can be moved or
/// copied to another device; the rest keep absolute paths.
pub fn m3u(tracks: &[Track], dir: Option<&Path>) -> String {
    let mut out = String::from("#EXTM3U\n");
    for t in tracks {
        let secs = if t.duration > 0.0 { t.duration.round() as i64 } else { -1 };
        let label = if t.artist.is_empty() { t.title.clone() } else { format!("{} - {}", t.artist, t.title) };
        let label: String = label.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let p = Path::new(&t.path);
        let shown = dir.and_then(|d| p.strip_prefix(d).ok()).unwrap_or(p);
        out.push_str(&format!("#EXTINF:{secs},{label}\n{}\n", shown.display()));
    }
    out
}

/// base64 (url-safe or standard, padding optional) -> bytes.
fn unb64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, title: &str, artist: &str, path: &str) -> Track {
        Track { id: id.into(), path: path.into(), title: title.into(), artist: artist.into(), duration: 201.4, ..Default::default() }
    }

    #[test]
    fn b64_decodes_what_spotify_auth_encodes() {
        for n in 0..40u8 {
            let v: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(unb64(&crate::spotify_auth::b64(&v, true)).unwrap(), v);
            assert_eq!(unb64(&crate::spotify_auth::b64(&v, false)).unwrap(), v);
        }
        assert!(unb64("ab$c").is_none());
    }

    #[test]
    fn theme_and_layout_round_trip() {
        let t = theme::palette("synthwave", None).to_custom("Neon");
        let code = encode(&Share::Theme(t.clone()));
        assert!(code.starts_with("DKFM1:theme:"));
        assert!(code.len() < 600, "{}", code.len());
        // found inside a chat message, with backticks around it
        let Share::Theme(back) = decode(&format!("try this `{code}` :)")).unwrap() else { panic!() };
        assert_eq!((back.name.as_str(), back.dark, &back.colors), ("Neon", true, &t.colors));

        // a never-drawn layout (infinite rects -> null), sent without the menu texts
        let mut dock = serde_json::to_value(crate::ui::default_dock()).unwrap();
        dock.as_object_mut().unwrap().remove("translations");
        let code = encode(&Share::Layout(NamedLayout { name: "Focus".into(), dock: dock.clone() }));
        assert!(code.len() < 1000, "{}", code.len());
        let Share::Layout(l) = decode(&code).unwrap() else { panic!() };
        let back: egui_dock::DockState<crate::ui::Tab> = serde_json::from_value(l.dock.clone()).unwrap();
        assert_eq!((l.name.as_str(), back.iter_all_tabs().count()), ("Focus", 7));
        // not a dock layout
        assert!(decode(&encode(&Share::Layout(NamedLayout { name: "x".into(), dock: serde_json::json!({"a": 1}) }))).is_err());

        // received themes never overwrite yours
        let mut s = Settings::default();
        assert_eq!(add_theme(&mut s, t.clone()), "custom:Neon");
        assert_eq!(add_theme(&mut s, t.clone()), "custom:Neon");
        let mut other = t.clone();
        other.colors.insert("bg".into(), "#000000".into());
        assert_eq!(add_theme(&mut s, other), "custom:Neon (2)");
        assert_eq!(s.custom_themes.len(), 2);
        assert_eq!(add_layout(&mut s, NamedLayout { name: "Default".into(), dock: dock.clone() }), "Default (2)");
    }

    #[test]
    fn playlist_round_trip_and_file() {
        let mut a = track("1", "One More Time", "Daft Punk", "C:/m/a.m4a");
        a.youtube_id = Some("FGBhQbmPwH8".into());
        a.source_key = Some("yt:FGBhQbmPwH8".into());
        let mut b = track("2", "Harder", "Daft Punk, Kanye West", "C:/m/b.m4a");
        b.spotify_id = Some("4cOdK2wGLETKBW3PvgPWqT".into());
        b.source_key = Some("url:https://example.com/x".into()); // not a shareable key
        let c = track("3", "Local Demo", "", "C:/m/c.mp3");
        let p = playlist_of("Road trip", &[a, b, c]);
        assert_eq!(p.songs[0].key, None, "redundant source key dropped");
        assert_eq!(p.songs[1].key, None);
        assert_eq!(p.songs[0].secs, 201);
        let code = encode(&Share::Playlist(p.clone()));
        let Share::Playlist(back) = decode(&code).unwrap() else { panic!() };
        assert_eq!(back, p);
        let file = file_json(&Share::Playlist(p.clone()));
        let Share::Playlist(back) = from_file_bytes(&file).unwrap() else { panic!() };
        assert_eq!(back, p);
        // a text file holding a code works too
        assert!(matches!(from_file_bytes(code.as_bytes()), Ok(Share::Playlist(_))));
        // downloader input: YouTube songs download that video, Spotify ones get matched
        let it = back.songs[0].to_itrack();
        assert_eq!((it.direct_url.as_deref(), it.source_key.as_str()), (Some("https://www.youtube.com/watch?v=FGBhQbmPwH8"), "yt:FGBhQbmPwH8"));
        let it = back.songs[1].to_itrack();
        assert_eq!((it.direct_url, it.source_key.as_str(), it.artists.len(), it.duration_ms), (None, "sp:4cOdK2wGLETKBW3PvgPWqT", 2, Some(201_000)));
        assert!(back.songs[2].keys().iter().any(|k| k.starts_with("ta:")));
    }

    #[test]
    fn rejects_garbage_malformed_and_oversized() {
        for bad in ["hello", "DKFM1:", "DKFM1:theme", "DKFM1:theme:", "DKFM1:theme:!!!!", "DKFM1:theme:AAAA", "DKFM1:playlist:Zm9vYmFy"] {
            assert!(decode(bad).is_err(), "{bad}");
        }
        // valid deflate, not JSON / wrong shape / unknown type
        let pack = |kind: &str, json: &[u8]| {
            let mut z = DeflateEncoder::new(Vec::new(), Compression::default());
            z.write_all(json).unwrap();
            format!("{PREFIX}{kind}:{}", crate::spotify_auth::b64(&z.finish().unwrap(), true))
        };
        assert!(decode(&pack("theme", b"not json")).is_err());
        assert!(decode(&pack("theme", br##"{"name":"x","colors":{"bg":"red","evil":"#ffffff"}}"##)).is_err());
        assert!(decode(&pack("playlist", br#"{"name":"x","songs":[]}"#)).is_err());
        assert!(decode(&pack("playlist", br#"{"name":"x","songs":[{"t":""}]}"#)).is_err());
        assert!(decode(&pack("virus", br#"{}"#)).unwrap_err().contains("newer"));
        // a tiny code that unpacks to more than the limit (zip bomb)
        let bomb = vec![b' '; MAX_JSON as usize + 10];
        let code = pack("theme", &bomb);
        assert!(code.len() < 20_000);
        assert!(decode(&code).unwrap_err().contains("too big"));
        // too long to paste
        assert!(decode(&format!("{PREFIX}theme:{}", "A".repeat(MAX_CODE))).unwrap_err().contains("too big"));
        // too many songs
        let many: Vec<Song> = (0..MAX_SONGS + 1).map(|i| Song { title: format!("s{i}"), ..Default::default() }).collect();
        assert!(decode(&encode(&Share::Playlist(SharedPlaylist { name: "big".into(), songs: many }))).unwrap_err().contains("more than"));
        // bad fields are cleaned: ids, keys, control characters, length
        let j = serde_json::json!({"name": "a\u{7}b", "songs": [{"t": "ok", "a": "x".repeat(1000), "yt": "../../etc", "sp": "short", "k": "url:http://x", "d": 999999}]});
        let Share::Playlist(p) = decode(&pack("playlist", &serde_json::to_vec(&j).unwrap())).unwrap() else { panic!() };
        assert_eq!(p.name, "ab");
        assert_eq!((p.songs[0].youtube.clone(), p.songs[0].spotify.clone(), p.songs[0].key.clone(), p.songs[0].secs, p.songs[0].artist.len()), (None, None, None, 86_400, 300));
        // files
        assert!(from_file_bytes(b"{\"type\":\"theme\"}").is_err());
        assert!(from_file_bytes(&[0xff, 0xfe, 0x00]).is_err());
    }

    #[test]
    fn settings_code_has_no_secrets_or_paths() {
        let mut s = Settings {
            spotify_client_id: "CLIENTID123".into(),
            spotify_client_secret: "SECRET456".into(),
            spotify_refresh_token: "REFRESH789".into(),
            spotify_user: "danyal".into(),
            music_folders: vec!["C:\\Users\\A\\Music".into()],
            download_dir: "D:\\Downloads\\DKFM".into(),
            font: "sys:C:\\Windows\\Fonts\\comic.ttf".into(),
            last_playlist: "PLAYLISTID".into(),
            theme: "custom:Mine".into(),
            zoom: 1.25,
            ..Default::default()
        };
        s.custom_themes = vec![theme::palette("ice", None).to_custom("Mine"), theme::palette("mono", None).to_custom("Other")];
        s.layouts.push(NamedLayout { name: "L".into(), dock: serde_json::json!({"x": 1}) });
        s.session.queue = vec!["TRACKID".into()];
        s.keys.insert("play".into(), "Ctrl+P".into());
        s.extra.insert("windowPos".into(), serde_json::json!([10, 20]));
        let code = encode(&Share::Settings(Box::new(SharedSettings::from(&s))));
        let Share::Settings(back) = decode(&code).unwrap() else { panic!() };
        let json = serde_json::to_string(&*back).unwrap();
        for secret in ["CLIENTID123", "SECRET456", "REFRESH789", "danyal", "Music", "Downloads", "comic", "Fonts", "PLAYLISTID", "TRACKID", "windowPos", "Other", "layouts", "dock"] {
            assert!(!json.contains(secret), "settings code leaks {secret}: {json}");
        }
        assert_eq!(back.font, None);
        assert_eq!(back.custom_themes.len(), 1);
        // applying keeps your own login, folders and font file, and adds the theme
        let mut mine = Settings { spotify_refresh_token: "MINE".into(), download_dir: "E:\\Mine".into(), font: "clean".into(), ..Default::default() };
        back.apply(&mut mine);
        assert_eq!((mine.spotify_refresh_token.as_str(), mine.download_dir.as_str(), mine.font.as_str()), ("MINE", "E:\\Mine", "clean"));
        assert_eq!((mine.theme.as_str(), mine.zoom, mine.keys.get("play").map(|s| s.as_str())), ("custom:Mine", 1.25, Some("Ctrl+P")));
        assert_eq!(mine.custom_themes.len(), 1);
        // nonsense values are fixed, not applied
        let j = serde_json::json!({"theme": "../../x", "zoom": 40.0, "columns": ["evil"], "keys": {"play": "Ctrl+Nope", "rm -rf": "X"}, "visualizer": "hack", "eq": {"gains": [1e9]}, "searchSources": ["ftp"]});
        let mut z = DeflateEncoder::new(Vec::new(), Compression::default());
        z.write_all(&serde_json::to_vec(&j).unwrap()).unwrap();
        let Share::Settings(x) = decode(&format!("{PREFIX}settings:{}", crate::spotify_auth::b64(&z.finish().unwrap(), true))).unwrap() else { panic!() };
        assert_eq!((x.theme.as_str(), x.zoom, x.columns.contains(&"title".to_string()), x.keys.len(), x.visualizer.as_str()), ("red-retro", 1.5, true, 0, "off"));
        assert_eq!((x.eq.gains.len(), x.eq.gains[0]), (10, 12.0));
        assert!(!x.search_sources.is_empty());
    }

    #[test]
    fn m3u_output() {
        let tracks = vec![track("1", "One More Time", "Daft Punk", "/music/DK.FM/Party/Daft Punk - One More Time.m4a"), Track { duration: 0.0, ..track("2", "Intro\nline", "", "/other/intro.mp3") }];
        let abs = m3u(&tracks, None);
        assert_eq!(abs, "#EXTM3U\n#EXTINF:201,Daft Punk - One More Time\n/music/DK.FM/Party/Daft Punk - One More Time.m4a\n#EXTINF:-1,Intro line\n/other/intro.mp3\n");
        let rel = m3u(&tracks, Some(Path::new("/music/DK.FM")));
        assert!(rel.contains("\nParty/Daft Punk - One More Time.m4a\n"), "{rel}");
        assert!(rel.contains("\n/other/intro.mp3\n"), "songs elsewhere stay absolute");
    }

    #[test]
    fn whole_dkfm_file_round_trip() {
        let mut d = crate::store::LibraryData::default();
        for (id, title) in [("1", "One More Time"), ("2", "Harder"), ("3", "Around the World"), ("4", "Digital Love")] {
            let mut t = track(id, title, "Daft Punk", &format!("C:/m/{id}.m4a"));
            t.youtube_id = Some(format!("yt{id}abcdefgh"));
            d.tracks.insert(id.into(), t);
        }
        d.playlists.push(crate::store::Playlist { id: "p".into(), name: "Party".into(), track_ids: vec!["2".into(), "1".into()], ..Default::default() });
        d.stats.entry("3".into()).or_default().liked = true;
        let mut s = Settings::default();
        s.spotify_refresh_token = "SECRET".into();
        s.music_folders = vec!["C:/Users/me/Music".into()];
        let all = SharedAll::of("Dany", &s, &d);
        assert_eq!((all.playlists.len(), all.playlists[0].songs.len(), all.liked.len(), all.songs.len()), (1, 2, 1, 1));
        assert_eq!(all.playlists[0].songs[0].title, "Harder"); // in the playlist's order
        let bytes = file_json(&Share::Everything(Box::new(all)));
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("SECRET") && !text.contains("C:/Users/me") && !text.contains("C:/m/"), "{text}");
        let Share::Everything(back) = from_file_bytes(&bytes).unwrap() else { panic!() };
        assert_eq!((back.name.as_str(), back.song_count(), back.songs[0].title.as_str()), ("Dany", 4, "Digital Love"));
        assert!(back.songs[0].youtube.is_some());
    }
}
#[cfg(test)]
mod theater_tests {
    use super::*;

    #[test]
    fn theater_codes_round_trip_and_stay_safe() {
        let mut t = crate::store::d_theater()[2].clone();
        t.name = "my party".into();
        let back = decode(&encode(&Share::Theater(t.clone()))).unwrap();
        let Share::Theater(b) = back else { panic!() };
        assert_eq!(b.name, "MY PARTY");
        assert_eq!((b.left.clone(), b.right.clone()), (t.left, t.right));
        // unknown parts, repeats and silly values are dropped or fixed
        let bad = serde_json::json!({ "name": "", "left": ["cover", "hack", "cover"], "right": ["lyrics"], "backdrop": "x", "lyricsSize": 99.0 });
        let Share::Theater(c) = parse("theater", bad).unwrap() else { panic!() };
        assert_eq!(c.left, vec!["cover"]);
        assert_eq!(c.right, vec!["lyrics"]);
        assert_eq!((c.backdrop.as_str(), c.lyrics_size, c.name.as_str()), ("plain", 2.6, "SHARED"));
    }
}

//! Local library: folder scanning (parallel), tag/cover reading, playlists, play stats, history.
use crate::store::{self, data_dir, Folder, History, LibraryData, Playlist, Stat, Track};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::*;
use lofty::tag::ItemKey;
use parking_lot::RwLock;
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

pub const AUDIO_EXT: &[&str] = &["mp3", "m4a", "aac", "flac", "ogg", "opus", "wav", "aiff", "aif", "alac", "mp4"];
const SCHEMA: u32 = 5;

pub fn is_audio(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| AUDIO_EXT.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

/// Same id as the Electron build: sha1(lowercased absolute path), first 16 hex chars.
pub fn id_for(p: &Path) -> String {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut h = Sha1::new();
    h.update(abs.to_string_lossy().to_lowercase().as_bytes());
    let out = h.finalize();
    out.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Artist + title fingerprint so songs you own are recognised even without source IDs.
use std::sync::LazyLock;
static RE_BRACKET: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\s*[\(\[]([^\)\]]*)[\)\]]").unwrap());
static RE_VERSION: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"remix|live|acoustic|version|edit|mix|slowed|sped|instrumental|cover").unwrap());
static RE_ARTIST_SPLIT: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i),|;| feat\.? | & ").unwrap());

pub fn ta_key(artist: &str, title: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    fn n(s: &str) -> String {
        let lower: String = s.nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect::<String>().to_lowercase();
        // drop bracketed extras unless they mark another version
        let stripped = RE_BRACKET.replace_all(&lower, |c: &regex::Captures| if RE_VERSION.is_match(&c[1]) { format!(" {}", &c[1]) } else { String::new() });
        stripped.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
    }
    format!("ta:{}|{}", n(&main_artist(artist)), n(title))
}

/// Every key a song is known by (source, Spotify, YouTube, artist+title).
pub fn track_keys(source_key: Option<&str>, spotify_id: Option<&str>, youtube_id: Option<&str>, artist: &str, title: &str) -> Vec<String> {
    let mut k: Vec<String> = Vec::with_capacity(4);
    k.extend(source_key.filter(|s| !s.is_empty()).map(String::from));
    k.extend(spotify_id.map(|s| format!("sp:{s}")));
    k.extend(youtube_id.map(|s| format!("yt:{s}")));
    k.push(ta_key(artist, title));
    k
}

fn keys_of(t: &Track) -> Vec<String> {
    track_keys(t.source_key.as_deref(), t.spotify_id.as_deref(), t.youtube_id.as_deref(), &t.artist, &t.title)
}

pub fn main_artist(a: &str) -> String {
    RE_ARTIST_SPLIT.split(a).next().unwrap_or("Unknown").trim().to_string()
}

pub struct Library {
    pub data: RwLock<LibraryData>,
    pub history: RwLock<History>,
    pub gen: AtomicU64,
    pub scanning: AtomicBool,
    pub scan_done: AtomicUsize,
    pub scan_total: AtomicUsize,
    /// private listening: plays, skips and history aren't recorded
    pub private: AtomicBool,
    dirty: AtomicBool,
    history_dirty: AtomicBool,
    cover_dir: PathBuf,
    /// recent undoable changes (memory only)
    undo: parking_lot::Mutex<Vec<Undo>>,
}

/// One undoable change: the playlists and songs it touched, as they were before it.
pub struct Undo {
    pub label: String,
    playlists: Vec<(usize, Playlist)>,
    tracks: Vec<Track>,
    stats: Vec<(String, Option<Stat>)>,
    /// files it sent to the Recycle Bin (only their library entries come back)
    pub trashed: usize,
}
const UNDO_MAX: usize = 20;

/// Playlists in sidebar order. `sort`: "custom" (your order; until you drag one, Liked Songs,
/// then imported, then your own), "name", "added" (newest first) or "played" (most recent first).
/// Outside your own order Spotify Liked Songs stays first.
pub fn sorted_playlists<'a>(d: &'a LibraryData, sort: &str) -> Vec<&'a Playlist> {
    let mut v: Vec<&Playlist> = d.playlists.iter().collect();
    let liked = |p: &Playlist| p.url() != Some("spotify:liked");
    match sort {
        "name" => v.sort_by_cached_key(|p| (liked(p), p.name.to_lowercase())),
        "added" => v.sort_by(|a, b| liked(a).cmp(&liked(b)).then(b.created_at.total_cmp(&a.created_at))),
        "played" => v.sort_by(|a, b| liked(a).cmp(&liked(b)).then(b.last_played.total_cmp(&a.last_played))),
        _ if !d.custom_order => v.sort_by_key(|p| (liked(p), !p.is_imported())),
        _ => {}
    }
    v
}

/// Folders in sidebar order (by name unless you sort playlists your own way).
pub fn sorted_folders<'a>(d: &'a LibraryData, sort: &str) -> Vec<&'a Folder> {
    let mut v: Vec<&Folder> = d.folders.iter().collect();
    if sort != "custom" {
        v.sort_by_cached_key(|f| f.name.to_lowercase());
    }
    v
}

/// The first 4 different album covers in a playlist (small cover files).
pub fn first_covers(d: &LibraryData, p: &Playlist) -> Vec<String> {
    let mut v: Vec<String> = Vec::with_capacity(4);
    for t in p.track_ids.iter().filter_map(|id| d.tracks.get(id)) {
        if let Some(c) = t.thumb.as_ref().or(t.cover.as_ref()) {
            if !v.contains(c) {
                v.push(c.clone());
                if v.len() == 4 {
                    break;
                }
            }
        }
    }
    v
}

fn short_hash(s: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(s);
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// File names of a playlist's generated covers: a 2×2 mosaic of 4 album covers, a downloaded
/// source (Spotify) cover.
pub fn mosaic_name(covers: &[String]) -> String {
    format!("mz_{}.jpg", short_hash(covers.join("|").as_bytes()))
}
pub fn remote_cover_name(url: &str) -> String {
    format!("pc_{}.jpg", short_hash(url.as_bytes()))
}

impl Library {
    pub fn load() -> Arc<Self> {
        let dir = data_dir();
        let cover_dir = dir.join("covers");
        let _ = std::fs::create_dir_all(&cover_dir);
        let _ = std::fs::create_dir_all(dir.join("waves"));
        let lib = Arc::new(Self {
            data: RwLock::new(store::load_json(&dir.join("library.json"))),
            history: RwLock::new(store::load_json(&dir.join("history.json"))),
            gen: AtomicU64::new(1),
            scanning: AtomicBool::new(false),
            scan_done: AtomicUsize::new(0),
            scan_total: AtomicUsize::new(0),
            private: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            history_dirty: AtomicBool::new(false),
            cover_dir,
            undo: Default::default(),
        });
        // save changes every few seconds even while the window is hidden or minimized (no frames
        // are drawn then), so downloads and auto-sync survive a crash, shutdown or forced close
        let weak = Arc::downgrade(&lib);
        std::thread::Builder::new()
            .name("lib-save".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                match weak.upgrade() {
                    Some(l) => l.flush(),
                    None => break,
                }
            })
            .ok();
        lib
    }

    pub fn cover_path(&self, name: &str) -> PathBuf {
        self.cover_dir.join(name)
    }

    /// Something changed: UI views rebuild, file is saved by the flusher.
    pub fn changed(&self) {
        self.gen.fetch_add(1, Ordering::Relaxed);
        self.dirty.store(true, Ordering::Relaxed);
    }
    /// Saved but no UI rebuild needed (stats, gains).
    pub fn quiet_changed(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn flush(&self) {
        // one writer at a time (background saver, UI, exit)
        static SAVING: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
        let _g = SAVING.lock();
        if self.dirty.swap(false, Ordering::Relaxed) {
            store::save_json(&data_dir().join("library.json"), &*self.data.read());
        }
        if self.history_dirty.swap(false, Ordering::Relaxed) {
            store::save_json(&data_dir().join("history.json"), &*self.history.read());
        }
    }

    /// Swap in restored data (from a backup). Songs whose cover art file is gone are re-read by
    /// the next scan, which rebuilds it from the song file.
    pub fn replace(&self, mut data: LibraryData, history: History) {
        for t in data.tracks.values_mut() {
            if t.cover.iter().chain(t.thumb.iter()).any(|c| !self.cover_dir.join(c).exists()) {
                t.v = 0;
            }
        }
        *self.data.write() = data;
        *self.history.write() = history;
        self.history_dirty.store(true, Ordering::Relaxed);
        self.changed();
    }

    // ------------------------------------------------------------ scanning
    pub fn scan(self: &Arc<Self>, folders: Vec<PathBuf>) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        let lib = self.clone();
        std::thread::Builder::new().name("scan".into()).spawn(move || {
            let mut files = Vec::new();
            for f in &folders {
                for e in walkdir::WalkDir::new(f).max_depth(12).into_iter().filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.')).flatten() {
                    if e.file_type().is_file() && is_audio(e.path()) {
                        files.push(e.into_path());
                    }
                }
            }
            lib.scan_total.store(files.len(), Ordering::Relaxed);
            lib.scan_done.store(0, Ordering::Relaxed);
            let existing: HashMap<String, (f64, u32)> = lib.data.read().tracks.iter().map(|(k, t)| (k.clone(), (t.mtime, t.v))).collect();
            let seen: HashSet<String> = files.iter().map(|f| id_for(f)).collect();
            // parse new/changed files on a few threads (tag parsing is I/O bound)
            let work: Vec<PathBuf> = files
                .into_iter()
                .filter(|f| {
                    let id = id_for(f);
                    let mtime = mtime_ms(f);
                    !matches!(existing.get(&id), Some((m, v)) if (*m - mtime).abs() < 1.0 && *v == SCHEMA)
                })
                .collect();
            let skipped = lib.scan_total.load(Ordering::Relaxed) - work.len();
            lib.scan_done.store(skipped, Ordering::Relaxed);
            let queue = Arc::new(parking_lot::Mutex::new(work));
            let results = Arc::new(parking_lot::Mutex::new(Vec::new()));
            let threads: Vec<_> = (0..6)
                .map(|_| {
                    let (q, r, lib) = (queue.clone(), results.clone(), lib.clone());
                    std::thread::spawn(move || loop {
                        let Some(f) = q.lock().pop() else { break };
                        if let Some(t) = lib.read_track(&f) {
                            r.lock().push(t);
                        }
                        lib.scan_done.fetch_add(1, Ordering::Relaxed);
                    })
                })
                .collect();
            for t in threads {
                let _ = t.join();
            }
            let roots: Vec<String> = folders.iter().map(|f| format!("{}{}", std::path::absolute(f).unwrap_or(f.clone()).to_string_lossy().to_lowercase(), std::path::MAIN_SEPARATOR)).collect();
            let mut dirty = false;
            {
                let mut d = lib.data.write();
                for t in results.lock().drain(..) {
                    merge_track(&mut d.tracks, t);
                    dirty = true;
                }
                let gone: Vec<String> = d
                    .tracks
                    .iter()
                    .filter(|(id, t)| !seen.contains(*id) && roots.iter().any(|r| t.path.to_lowercase().starts_with(r)))
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in gone {
                    d.tracks.remove(&id);
                    dirty = true;
                }
            }
            if dirty {
                lib.changed();
            }
            lib.cleanup_files();
            lib.scanning.store(false, Ordering::SeqCst);
        }).ok();
    }

    pub fn add_files(&self, paths: &[PathBuf], extra: impl Fn(&mut Track)) -> Vec<String> {
        let mut ids = Vec::new();
        for p in paths {
            if !is_audio(p) {
                continue;
            }
            if let Some(mut t) = self.read_track(p) {
                extra(&mut t);
                ids.push(t.id.clone());
                merge_track(&mut self.data.write().tracks, t);
            }
        }
        if !ids.is_empty() {
            self.changed();
        }
        ids
    }

    pub fn remove_path(&self, p: &Path) {
        let id = id_for(p);
        if self.data.read().tracks.contains_key(&id) {
            self.remove_track(&id);
        }
    }

    pub fn read_track(&self, file: &Path) -> Option<Track> {
        let meta = std::fs::metadata(file).ok()?;
        let tagged = lofty::read_from_path(file).ok();
        let props = tagged.as_ref().map(|t| t.properties().clone());
        let tag = tagged.as_ref().and_then(|t| t.primary_tag().or_else(|| t.first_tag()));
        let base = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let guess: Option<(String, String)> = base.split_once(" - ").map(|(a, b)| (a.trim().to_string(), b.trim().to_string()));
        let get = |k: &ItemKey| tag.and_then(|t| t.get_string(k).map(|s| s.to_string()));
        let title = tag.and_then(|t| t.title().map(|s| s.to_string())).filter(|s| !s.is_empty()).or_else(|| guess.as_ref().map(|g| g.1.clone())).unwrap_or(base.clone());
        let artist = tag.and_then(|t| t.artist().map(|s| s.to_string())).filter(|s| !s.is_empty()).or_else(|| guess.as_ref().map(|g| g.0.clone())).unwrap_or_else(|| "Unknown Artist".into());
        let duration = props.as_ref().map(|p| p.duration().as_secs_f64()).unwrap_or(0.0);
        let mut bitrate = props.as_ref().and_then(|p| p.audio_bitrate());
        if bitrate.map_or(true, |b| !(32..=6000).contains(&b)) && duration > 0.0 {
            bitrate = Some(((meta.len() as f64 * 8.0) / duration / 1000.0).round() as u32);
        }
        let (cover, thumb) = tag.and_then(|t| t.pictures().first()).map(|p| self.save_cover(p.data())).unwrap_or((None, None));
        let codec = tagged.as_ref().map(|t| format!("{:?}", t.file_type()).to_uppercase()).unwrap_or_default();
        Some(Track {
            id: id_for(file),
            path: std::path::absolute(file).unwrap_or(file.to_path_buf()).to_string_lossy().into_owned(),
            title,
            artist,
            album: tag.and_then(|t| t.album().map(|s| s.to_string())).unwrap_or_default(),
            album_artist: get(&ItemKey::AlbumArtist).unwrap_or_default(),
            year: tag.and_then(|t| t.year()),
            track: tag.and_then(|t| t.track()),
            genre: tag.and_then(|t| t.genre().map(|s| s.to_string())).unwrap_or_default(),
            duration,
            bitrate,
            codec: match codec.as_str() { "MPEG" => "MP3".into(), "MP4" => "AAC".into(), c => c.to_string() },
            sample_rate: props.as_ref().and_then(|p| p.sample_rate()),
            cover,
            thumb,
            mtime: mtime_ms(file),
            size: meta.len(),
            added_at: store::now_ms(),
            v: SCHEMA,
            ..Default::default()
        })
    }

    /// Cover art stored once per unique image: ≤512px for the deck, 192px for lists.
    fn save_cover(&self, data: &[u8]) -> (Option<String>, Option<String>) {
        let mut h = Sha1::new();
        h.update(data);
        let hash: String = h.finalize().iter().take(10).map(|b| format!("{b:02x}")).collect();
        let (name, tname) = (format!("{hash}.jpg"), format!("t_{hash}.jpg"));
        let (out, tout) = (self.cover_dir.join(&name), self.cover_dir.join(&tname));
        if !out.exists() || !tout.exists() {
            let Ok(img) = image::load_from_memory(data) else { return (None, None) };
            let save = |im: image::DynamicImage, path: &Path| {
                if let Ok(f) = std::fs::File::create(path) {
                    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(f), 85);
                    let _ = enc.encode_image(&im.to_rgb8());
                }
            };
            if !out.exists() {
                let im = if img.width() > 512 { img.resize(512, 512, image::imageops::FilterType::Triangle) } else { img.clone() };
                save(im, &out);
            }
            if !tout.exists() {
                save(img.resize(192, 192, image::imageops::FilterType::Triangle), &tout);
            }
        }
        (Some(name), Some(tname))
    }

    /// A playlist cover from image bytes (your picture, or the source's): square-cropped to
    /// 300 px. `name` = the file name to use (None = named by its content). Returns the name.
    pub fn save_playlist_cover(&self, data: &[u8], name: Option<String>) -> Option<String> {
        let img = image::load_from_memory(data).ok()?;
        let side = img.width().min(img.height());
        let sq = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side).resize_exact(300, 300, image::imageops::FilterType::Triangle);
        let name = name.unwrap_or_else(|| format!("pl_{}.jpg", short_hash(data)));
        save_jpeg(&sq, &self.cover_dir.join(&name)).then_some(name)
    }

    /// Builds the 2x2 mosaic cover from 4 cover files.
    pub fn make_mosaic(&self, covers: &[String]) -> Option<String> {
        let name = mosaic_name(covers);
        let mut out = image::RgbImage::new(300, 300);
        for (i, c) in covers.iter().take(4).enumerate() {
            let im = image::open(self.cover_dir.join(c)).ok()?.resize_to_fill(150, 150, image::imageops::FilterType::Triangle).to_rgb8();
            image::imageops::replace(&mut out, &im, (i as i64 % 2) * 150, (i as i64 / 2) * 150);
        }
        save_jpeg(&image::DynamicImage::ImageRgb8(out), &self.cover_dir.join(&name)).then_some(name)
    }

    /// Delete cover art / waveform files no track or playlist uses (older than 10 minutes).
    pub fn cleanup_files(&self) {
        let d = self.data.read();
        let mut used: HashSet<String> = HashSet::new();
        for t in d.tracks.values() {
            used.extend(t.cover.clone());
            used.extend(t.thumb.clone());
        }
        for p in &d.playlists {
            used.extend(p.custom_cover.clone());
            used.extend(p.cover.as_deref().filter(|c| c.starts_with("http")).map(remote_cover_name));
            let c = first_covers(&d, p);
            if c.len() == 4 {
                used.insert(mosaic_name(&c));
            }
        }
        let ids: HashSet<&String> = d.tracks.keys().collect();
        let older = |p: &Path, secs: u64| p.metadata().and_then(|m| m.modified()).map(|t| t.elapsed().map(|e| e.as_secs() > secs).unwrap_or(false)).unwrap_or(false);
        let old = |p: &Path| older(p, 600);
        for e in std::fs::read_dir(&self.cover_dir).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            // (mosaics of Home's mixes aren't any playlist's: they're kept for a week)
            if !used.contains(&n) && older(&e.path(), if n.starts_with("mz_") { 7 * 86400 } else { 600 }) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        for e in std::fs::read_dir(data_dir().join("waves")).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().trim_end_matches(".bin").to_string();
            if !ids.contains(&n) && old(&e.path()) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    // ------------------------------------------------------------ queries & edits
    pub fn track(&self, id: &str) -> Option<Track> {
        self.data.read().tracks.get(id).cloned()
    }

    pub fn stat(&self, id: &str) -> Stat {
        self.data.read().stats.get(id).cloned().unwrap_or_default()
    }

    pub fn toggle_like(&self, id: &str) {
        let mut d = self.data.write();
        let s = d.stats.entry(id.to_string()).or_default();
        s.liked = !s.liked;
        drop(d);
        self.changed();
    }

    pub fn bump_play(&self, id: &str) {
        if self.private.load(Ordering::Relaxed) {
            return;
        }
        let mut d = self.data.write();
        let s = d.stats.entry(id.to_string()).or_default();
        s.plays += 1;
        s.last_played = store::now_ms();
        drop(d);
        self.quiet_changed();
        self.gen.fetch_add(1, Ordering::Relaxed);
    }

    pub fn bump_skip(&self, id: &str) {
        if self.private.load(Ordering::Relaxed) {
            return;
        }
        self.data.write().stats.entry(id.to_string()).or_default().skips += 1;
        self.quiet_changed();
    }

    pub fn set_gain(&self, id: &str, db: f32) {
        if let Some(t) = self.data.write().tracks.get_mut(id) {
            t.gain_v2 = Some(db);
        }
        self.quiet_changed();
    }

    pub fn add_history(&self, row: (String, i64, i64, u8)) {
        if self.private.load(Ordering::Relaxed) {
            return;
        }
        let mut h = self.history.write();
        h.events.push(row);
        let n = h.events.len();
        if n > 100_000 {
            h.events.drain(0..n - 100_000);
        }
        self.history_dirty.store(true, Ordering::Relaxed);
    }

    pub fn remove_track(&self, id: &str) {
        let mut d = self.data.write();
        d.tracks.remove(id);
        for p in d.playlists.iter_mut() {
            p.track_ids.retain(|t| t != id);
        }
        drop(d);
        self.changed();
    }

    /// Add songs to the end of a playlist (skipping ones already in it). Returns how many.
    pub fn playlist_add(&self, pid: &str, ids: &[String]) -> usize {
        let mut d = self.data.write();
        let keys: Vec<String> = ids.iter().filter_map(|i| d.tracks.get(i)).flat_map(keys_of).collect();
        let Some(p) = d.playlists.iter_mut().find(|p| p.id == pid) else { return 0 };
        let before = p.track_ids.len();
        for id in ids {
            if !p.track_ids.contains(id) {
                p.track_ids.push(id.clone());
            }
        }
        let n = p.track_ids.len() - before;
        if p.is_imported() && n > 0 {
            p.edited = true;
            p.removed.retain(|k| !keys.contains(k));
        }
        drop(d);
        self.changed();
        n
    }

    /// Remove songs from a playlist; a synced playlist remembers them so sync won't re-add them.
    pub fn playlist_remove(&self, pid: &str, ids: &[String]) {
        let mut d = self.data.write();
        let keys: Vec<String> = ids.iter().filter_map(|i| d.tracks.get(i)).flat_map(keys_of).collect();
        let Some(p) = d.playlists.iter_mut().find(|p| p.id == pid) else { return };
        p.track_ids.retain(|x| !ids.contains(x));
        if p.is_imported() {
            p.edited = true;
            for k in keys {
                if !p.removed.contains(&k) {
                    p.removed.push(k);
                }
            }
        }
        drop(d);
        self.changed();
    }

    /// Move songs (keeping their relative order) to just before `before` (None = the end).
    pub fn playlist_move(&self, pid: &str, moving: &[String], before: Option<&str>) {
        let mut d = self.data.write();
        let Some(p) = d.playlists.iter_mut().find(|p| p.id == pid) else { return };
        if before.map(|b| moving.iter().any(|m| m == b)).unwrap_or(false) {
            return;
        }
        let picked: Vec<String> = p.track_ids.iter().filter(|x| moving.contains(x)).cloned().collect();
        p.track_ids.retain(|x| !moving.contains(x));
        let at = before.and_then(|b| p.track_ids.iter().position(|x| x == b)).unwrap_or(p.track_ids.len());
        p.track_ids.splice(at..at, picked);
        if p.is_imported() {
            p.edited = true;
        }
        drop(d);
        self.changed();
    }

    /// Copies of the same song (same artist + title, lengths within 4 s), best copy first:
    /// lossless, then higher bitrate, then has cover art, then most played.
    pub fn duplicate_groups(&self) -> Vec<Vec<String>> {
        let d = self.data.read();
        let mut by: HashMap<String, Vec<&Track>> = HashMap::new();
        for t in d.tracks.values().filter(|t| !t.title.trim().is_empty()) {
            by.entry(ta_key(&t.artist, &t.title)).or_default().push(t);
        }
        let quality = |t: &Track| {
            let lossless = matches!(t.codec.to_lowercase().as_str(), c if c.contains("flac") || c.contains("alac") || c.contains("wav") || c.contains("pcm") || c.contains("aiff"));
            (lossless, t.bitrate.unwrap_or(0), t.cover.is_some(), d.stats.get(&t.id).map(|s| s.plays).unwrap_or(0))
        };
        let mut out: Vec<Vec<String>> = Vec::new();
        let mut take = |g: &mut Vec<&Track>| {
            if g.len() > 1 {
                g.sort_by(|a, b| quality(b).cmp(&quality(a)));
                out.push(g.iter().map(|t| t.id.clone()).collect());
            }
            g.clear();
        };
        for (_, mut v) in by.into_iter().filter(|(_, v)| v.len() > 1) {
            v.sort_by(|a, b| a.duration.total_cmp(&b.duration));
            let mut g: Vec<&Track> = Vec::new();
            for t in v {
                if g.last().map(|l| t.duration - l.duration > 4.0).unwrap_or(false) {
                    take(&mut g);
                }
                g.push(t);
            }
            take(&mut g);
        }
        out.sort_by_cached_key(|g| d.tracks.get(&g[0]).map(|t| (t.artist.to_lowercase(), t.title.to_lowercase())).unwrap_or_default());
        out
    }

    /// Keep `keep` and fold the other copies into it (plays, likes, playlists, source keys),
    /// dropping them from the library. Returns the dropped copies' files.
    pub fn merge_duplicates(&self, keep: &str, others: &[String]) -> Vec<PathBuf> {
        let mut d = self.data.write();
        let mut paths = Vec::new();
        for o in others.iter().filter(|o| *o != keep) {
            let Some(t) = d.tracks.remove(o) else { continue };
            paths.push(PathBuf::from(&t.path));
            if let Some(k) = d.tracks.get_mut(keep) {
                k.spotify_id = k.spotify_id.take().or(t.spotify_id);
                k.youtube_id = k.youtube_id.take().or(t.youtube_id);
                k.source_key = k.source_key.take().or(t.source_key);
            }
            if let Some(s) = d.stats.remove(o) {
                let k = d.stats.entry(keep.to_string()).or_default();
                k.plays += s.plays;
                k.skips += s.skips;
                k.liked |= s.liked;
                k.last_played = k.last_played.max(s.last_played);
            }
            for p in d.playlists.iter_mut() {
                if let Some(i) = p.track_ids.iter().position(|x| x == o) {
                    if p.track_ids.iter().any(|x| x == keep) {
                        p.track_ids.remove(i);
                    } else {
                        p.track_ids[i] = keep.to_string();
                    }
                }
            }
        }
        drop(d);
        self.changed();
        paths
    }

    pub fn set_liked(&self, ids: &[String], liked: bool) {
        let mut d = self.data.write();
        for id in ids {
            d.stats.entry(id.clone()).or_default().liked = liked;
        }
        drop(d);
        self.changed();
    }

    pub fn upsert_playlist(&self, pl: Playlist) -> String {
        let mut d = self.data.write();
        let id = pl.id.clone();
        if let Some(p) = d.playlists.iter_mut().find(|p| p.id == pl.id) {
            *p = pl;
        } else {
            d.playlists.push(pl);
        }
        drop(d);
        self.changed();
        id
    }

    pub fn edit_playlist(&self, id: &str, f: impl FnOnce(&mut Playlist)) {
        if let Some(p) = self.data.write().playlists.iter_mut().find(|p| p.id == id) {
            f(p);
        }
        self.changed();
    }

    /// Puts a song into a playlist right after `after` (or first when that isn't in it); does
    /// nothing if the song is already there.
    pub fn playlist_insert(&self, pid: &str, id: &str, after: Option<&str>) {
        let mut d = self.data.write();
        let Some(p) = d.playlists.iter_mut().find(|p| p.id == pid) else { return };
        if p.track_ids.iter().any(|x| x == id) {
            return;
        }
        let at = after.and_then(|a| p.track_ids.iter().position(|x| x == a)).map(|i| i + 1).unwrap_or(0);
        p.track_ids.insert(at, id.to_string());
        drop(d);
        self.changed();
    }

    pub fn new_playlist(&self, name: &str, ids: Vec<String>) -> String {
        let id = format!("{:016x}", fastrand::u64(..));
        self.upsert_playlist(Playlist { id: id.clone(), name: name.into(), track_ids: ids, source: Some("user".into()), created_at: store::now_ms(), ..Default::default() })
    }

    pub fn delete_playlist(&self, id: &str) {
        self.data.write().playlists.retain(|p| p.id != id);
        self.changed();
    }

    // ------------------------------------------------------------ hidden songs
    pub fn is_hidden(&self, id: &str) -> bool {
        self.data.read().stats.get(id).map(|s| s.hidden).unwrap_or(false)
    }

    pub fn set_hidden(&self, ids: &[String], hidden: bool) {
        let mut d = self.data.write();
        for id in ids {
            d.stats.entry(id.clone()).or_default().hidden = hidden;
        }
        drop(d);
        self.changed();
    }

    // ------------------------------------------------------------ sidebar: folders, pins, order
    pub fn new_folder(&self, name: &str) -> String {
        let id = format!("f{:015x}", fastrand::u64(..) >> 4);
        self.data.write().folders.push(Folder { id: id.clone(), name: name.into(), collapsed: false });
        self.changed();
        id
    }

    pub fn edit_folder(&self, id: &str, f: impl FnOnce(&mut Folder)) {
        if let Some(x) = self.data.write().folders.iter_mut().find(|x| x.id == id) {
            f(x);
        }
        self.changed();
    }

    /// Removes a folder; its playlists stay (outside any folder).
    pub fn delete_folder(&self, id: &str) {
        let mut d = self.data.write();
        d.folders.retain(|f| f.id != id);
        for p in d.playlists.iter_mut().filter(|p| p.folder.as_deref() == Some(id)) {
            p.folder = None;
        }
        drop(d);
        self.changed();
    }

    /// The playlists in a folder, in sidebar order.
    pub fn folder_playlists(&self, id: &str, sort: &str) -> Vec<Playlist> {
        let d = self.data.read();
        sorted_playlists(&d, sort).into_iter().filter(|p| p.folder.as_deref() == Some(id)).cloned().collect()
    }

    /// Puts a playlist into `folder` (None = no folder), pinned or not; with `near` = (another
    /// playlist, after it?) it also moves there in your own order.
    pub fn place_playlist(&self, pid: &str, folder: Option<String>, pinned: bool, near: Option<(&str, bool)>) {
        let mut d = self.data.write();
        let folder = folder.filter(|f| d.folders.iter().any(|x| x.id == *f));
        if let Some((other, after)) = near.filter(|n| n.0 != pid) {
            if !d.custom_order {
                // keep what you see: the default order becomes your own order
                let order: Vec<String> = sorted_playlists(&d, "custom").iter().map(|p| p.id.clone()).collect();
                d.playlists.sort_by_key(|p| order.iter().position(|o| *o == p.id));
                d.custom_order = true;
            }
            if let Some(i) = d.playlists.iter().position(|p| p.id == pid) {
                let p = d.playlists.remove(i);
                let at = d.playlists.iter().position(|p| p.id == other).map(|j| j + after as usize).unwrap_or(d.playlists.len());
                d.playlists.insert(at, p);
            }
        }
        if let Some(p) = d.playlists.iter_mut().find(|p| p.id == pid) {
            p.folder = folder;
            p.pinned = pinned;
        }
        drop(d);
        self.changed();
    }

    /// Moves a folder to just before `before` (None = the end).
    pub fn move_folder(&self, id: &str, before: Option<&str>) {
        let mut d = self.data.write();
        if before == Some(id) {
            return;
        }
        let Some(i) = d.folders.iter().position(|f| f.id == id) else { return };
        let f = d.folders.remove(i);
        let at = before.and_then(|b| d.folders.iter().position(|f| f.id == b)).unwrap_or(d.folders.len());
        d.folders.insert(at, f);
        drop(d);
        self.changed();
    }

    /// You played this playlist (for "recently played"; not while listening privately).
    pub fn touch_playlist(&self, pid: &str) {
        if self.private.load(Ordering::Relaxed) {
            return;
        }
        if let Some(p) = self.data.write().playlists.iter_mut().find(|p| p.id == pid) {
            p.last_played = store::now_ms();
        }
        self.changed();
    }

    // ------------------------------------------------------------ undo
    /// The playlists `pids` (and any holding songs `tids`) and songs `tids` (with their stats)
    /// as they are now, to put back if the change about to happen is undone.
    pub fn snapshot(&self, label: impl Into<String>, pids: &[String], tids: &[String]) -> Undo {
        let d = self.data.read();
        Undo {
            label: label.into(),
            playlists: d.playlists.iter().enumerate().filter(|(_, p)| pids.contains(&p.id) || p.track_ids.iter().any(|t| tids.contains(t))).map(|(i, p)| (i, p.clone())).collect(),
            tracks: tids.iter().filter_map(|t| d.tracks.get(t).cloned()).collect(),
            stats: tids.iter().map(|t| (t.clone(), d.stats.get(t).cloned())).collect(),
            trashed: 0,
        }
    }

    pub fn push_undo(&self, u: Undo) {
        let mut v = self.undo.lock();
        v.push(u);
        if v.len() > UNDO_MAX {
            v.remove(0);
        }
    }

    /// Puts back what the last undoable change touched; returns it (None = nothing to undo).
    pub fn undo(&self) -> Option<Undo> {
        let u = self.undo.lock().pop()?;
        let mut d = self.data.write();
        for (i, p) in &u.playlists {
            match d.playlists.iter_mut().find(|x| x.id == p.id) {
                Some(x) => *x = p.clone(),
                None => {
                    let at = (*i).min(d.playlists.len());
                    d.playlists.insert(at, p.clone());
                }
            }
        }
        for t in &u.tracks {
            d.tracks.insert(t.id.clone(), t.clone());
        }
        for (id, s) in &u.stats {
            match s {
                Some(s) => d.stats.insert(id.clone(), s.clone()),
                None => d.stats.remove(id),
            };
        }
        drop(d);
        self.changed();
        Some(u)
    }

    /// sourceKey / Spotify / YouTube / artist+title -> track id
    pub fn key_index(&self) -> HashMap<String, String> {
        let d = self.data.read();
        let mut m = HashMap::new();
        for t in d.tracks.values() {
            if let Some(k) = &t.source_key { m.insert(k.clone(), t.id.clone()); }
            if let Some(k) = &t.spotify_id { m.insert(format!("sp:{k}"), t.id.clone()); }
            if let Some(k) = &t.youtube_id { m.insert(format!("yt:{k}"), t.id.clone()); }
            m.entry(ta_key(&t.artist, &t.title)).or_insert_with(|| t.id.clone());
        }
        m
    }
}

fn merge_track(map: &mut HashMap<String, Track>, mut t: Track) {
    if let Some(prev) = map.get(&t.id) {
        t.added_at = prev.added_at;
        t.gain = t.gain.or(prev.gain);
        t.gain_v2 = t.gain_v2.or(prev.gain_v2);
        t.spotify_id = t.spotify_id.take().or_else(|| prev.spotify_id.clone());
        t.youtube_id = t.youtube_id.take().or_else(|| prev.youtube_id.clone());
        t.source_key = t.source_key.take().or_else(|| prev.source_key.clone());
    }
    map.insert(t.id.clone(), t);
}

fn save_jpeg(im: &image::DynamicImage, path: &Path) -> bool {
    let Ok(f) = std::fs::File::create(path) else { return false };
    image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(f), 85).encode_image(&im.to_rgb8()).is_ok()
}

fn mtime_ms(p: &Path) -> f64 {
    p.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, artist: &str, title: &str, dur: f64, codec: &str, kbps: u32) -> Track {
        Track { id: id.into(), path: format!("/x/{id}.m4a"), artist: artist.into(), title: title.into(), duration: dur, codec: codec.into(), bitrate: Some(kbps), spotify_id: Some(format!("S{id}")), ..Default::default() }
    }

    #[test]
    fn playlist_edits_and_duplicates() {
        let dir = std::env::temp_dir().join(format!("dkfm-libtest-{}", std::process::id()));
        std::env::set_var("DKFM_USER_DATA", &dir);
        let lib = Library::load();
        {
            let mut d = lib.data.write();
            for t in [track("a", "Sade", "Smooth Operator", 258.0, "AAC", 128), track("b", "Sade", "Smooth Operator", 259.5, "FLAC", 900), track("c", "Sade", "Smooth Operator (Live)", 400.0, "AAC", 128), track("d", "Drake", "Hold On", 230.0, "AAC", 256), track("e", "Drake", "Hold On", 300.0, "AAC", 256)] {
                d.tracks.insert(t.id.clone(), t);
            }
            d.stats.insert("a".into(), Stat { plays: 5, liked: true, ..Default::default() });
            d.playlists.push(Playlist { id: "p".into(), name: "P".into(), track_ids: vec!["a".into(), "d".into(), "e".into()], spotify_url: Some("https://open.spotify.com/playlist/x".into()), ..Default::default() });
        }
        // reorder: move e before a
        lib.playlist_move("p", &["e".into()], Some("a"));
        assert_eq!(lib.data.read().playlists[0].track_ids, ["e", "a", "d"]);
        // a shared playlist's downloads land in their place
        lib.data.write().playlists.push(Playlist { id: "s".into(), name: "S".into(), track_ids: vec!["d".into()], ..Default::default() });
        lib.playlist_insert("s", "a", None);
        lib.playlist_insert("s", "c", Some("a"));
        lib.playlist_insert("s", "e", Some("d"));
        lib.playlist_insert("s", "c", Some("d"));
        assert_eq!(lib.data.read().playlists[1].track_ids, ["a", "c", "d", "e"]);
        lib.data.write().playlists.retain(|p| p.id != "s");
        // remove marks a synced playlist edited and remembers the song's keys
        lib.playlist_remove("p", &["d".into()]);
        let p = lib.data.read().playlists[0].clone();
        assert!(p.edited && p.track_ids == ["e", "a"] && p.removed.contains(&"sp:Sd".to_string()));
        // adding it back forgets the removal
        assert_eq!(lib.playlist_add("p", &["d".into(), "a".into()]), 1);
        assert!(!lib.data.read().playlists[0].removed.contains(&"sp:Sd".to_string()));
        // duplicates: a+b (same length, FLAC first); live version and the 70 s longer "Hold On" stay apart
        let g = lib.duplicate_groups();
        assert_eq!(g, vec![vec!["b".to_string(), "a".to_string()]]);
        // merge keeps b, carries plays/like, swaps a for b in the playlist
        let files = lib.merge_duplicates("b", &g[0]);
        assert_eq!(files, vec![PathBuf::from("/x/a.m4a")]);
        let d = lib.data.read();
        assert!(!d.tracks.contains_key("a") && d.stats["b"].plays == 5 && d.stats["b"].liked);
        assert_eq!(d.playlists[0].track_ids, ["e", "b", "d"]);
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folders_pins_sort_undo_private() {
        let dir = std::env::temp_dir().join(format!("dkfm-libtest2-{}", std::process::id()));
        std::env::set_var("DKFM_USER_DATA", &dir);
        let lib = Library::load();
        let pl = |id: &str, name: &str, at: f64, url: Option<&str>| Playlist { id: id.into(), name: name.into(), created_at: at, spotify_url: url.map(String::from), ..Default::default() };
        {
            let mut d = lib.data.write();
            for t in [track("a", "Sade", "Smooth Operator", 258.0, "AAC", 128), track("b", "Drake", "Hold On", 230.0, "AAC", 256)] {
                d.tracks.insert(t.id.clone(), t);
            }
            d.playlists = vec![pl("mine", "Zed", 1.0, None), pl("imp", "Imported", 2.0, Some("https://open.spotify.com/playlist/x")), pl("liked", "Liked Songs", 3.0, Some("spotify:liked")), pl("new", "Alpha", 4.0, None)];
            d.playlists[0].track_ids = vec!["a".into(), "b".into()];
        }
        let order = |sort: &str| sorted_playlists(&lib.data.read(), sort).iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        let find = |id: &str| lib.data.read().playlists.iter().find(|p| p.id == id).cloned().unwrap();
        // default: Spotify Liked Songs, imported, then yours; the other sorts keep Liked Songs first
        assert_eq!(order("custom"), ["liked", "imp", "mine", "new"]);
        assert_eq!(order("name"), ["liked", "new", "imp", "mine"]);
        assert_eq!(order("added"), ["liked", "new", "imp", "mine"]);
        lib.touch_playlist("mine");
        assert_eq!(order("played"), ["liked", "mine", "imp", "new"]);
        // dragging "new" above "liked" makes the order your own
        lib.place_playlist("new", None, false, Some(("liked", false)));
        assert_eq!(order("custom"), ["new", "liked", "imp", "mine"]);
        assert!(lib.data.read().custom_order);
        lib.place_playlist("liked", None, false, Some(("", true))); // to the end
        assert_eq!(order("custom"), ["new", "imp", "mine", "liked"]);
        // folders and pins
        let f = lib.new_folder("Moods");
        let g = lib.new_folder("Gym");
        lib.place_playlist("mine", Some(f.clone()), false, None);
        lib.place_playlist("imp", Some(f.clone()), true, None);
        lib.place_playlist("new", Some("gone".into()), false, None); // unknown folder: none
        assert_eq!(lib.folder_playlists(&f, "custom").iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["imp", "mine"]);
        assert_eq!(find("new").folder, None);
        assert_eq!(sorted_folders(&lib.data.read(), "name").iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Gym", "Moods"]);
        lib.move_folder(&g, Some(&f));
        assert_eq!(sorted_folders(&lib.data.read(), "custom")[0].id, g);
        lib.edit_folder(&f, |x| x.collapsed = true);
        lib.set_hidden(&["b".into()], true);
        // everything survives a save + load
        let json = serde_json::to_string(&*lib.data.read()).unwrap();
        let back: LibraryData = serde_json::from_str(&json).unwrap();
        assert!(back.custom_order && back.folders.len() == 2 && back.folders.iter().find(|x| x.id == f).unwrap().collapsed);
        assert_eq!(back.playlists.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["new", "imp", "mine", "liked"]);
        let imp = back.playlists.iter().find(|p| p.id == "imp").unwrap();
        assert!(imp.pinned && imp.folder.as_deref() == Some(f.as_str()) && back.stats["b"].hidden);
        assert!(back.playlists.iter().find(|p| p.id == "mine").unwrap().last_played > 0.0);
        // deleting a folder keeps its playlists
        lib.delete_folder(&f);
        assert!(lib.data.read().playlists.iter().all(|p| p.folder.is_none()) && lib.data.read().playlists.len() == 4);
        // old files without the new fields still load
        let old: LibraryData = serde_json::from_str(r#"{"tracks":{},"playlists":[{"id":"x","name":"X"}],"stats":{"t":{"plays":2}}}"#).unwrap();
        assert!(!old.custom_order && old.folders.is_empty() && !old.playlists[0].pinned && !old.stats["t"].hidden);

        // undo: remove from playlist, delete a playlist, remove from library; newest first
        let u = lib.snapshot("removed from Zed", &["mine".into()], &[]);
        lib.playlist_remove("mine", &["a".into()]);
        lib.push_undo(u);
        let u = lib.snapshot("deleted Alpha", &["new".into()], &[]);
        lib.delete_playlist("new");
        lib.push_undo(u);
        lib.data.write().stats.insert("a".into(), Stat { plays: 3, ..Default::default() });
        let u = lib.snapshot("removed from library", &[], &["b".into()]);
        lib.remove_track("b");
        lib.push_undo(u);
        assert!(lib.track("b").is_none() && find("mine").track_ids.is_empty());
        assert_eq!(lib.undo().unwrap().label, "removed from library");
        assert!(lib.track("b").is_some() && lib.stat("b").hidden);
        assert_eq!(find("mine").track_ids, ["b"]);
        assert_eq!(lib.undo().unwrap().label, "deleted Alpha");
        assert_eq!(order("custom"), ["new", "imp", "mine", "liked"]); // back in its place
        lib.undo();
        assert_eq!(find("mine").track_ids, ["a", "b"]);
        assert!(lib.undo().is_none());
        // a merge comes back with the dropped copy's entry and stats
        lib.data.write().tracks.insert("a2".into(), track("a2", "Sade", "Smooth Operator", 258.0, "AAC", 96));
        lib.data.write().stats.insert("a2".into(), Stat { plays: 4, liked: true, ..Default::default() });
        lib.edit_playlist("new", |p| p.track_ids = vec!["a2".into()]);
        let mut u = lib.snapshot("merged", &[], &["a".into(), "a2".into()]);
        lib.merge_duplicates("a", &["a".into(), "a2".into()]);
        u.trashed = 1;
        lib.push_undo(u);
        assert!(lib.stat("a").plays == 7 && lib.track("a2").is_none());
        assert_eq!(lib.undo().unwrap().trashed, 1);
        assert!(lib.stat("a").plays == 3 && !lib.stat("a").liked && lib.stat("a2").plays == 4 && lib.track("a2").is_some());
        assert_eq!(find("new").track_ids, ["a2"]);
        // the stack keeps the last 20
        for i in 0..25 {
            lib.push_undo(lib.snapshot(format!("{i}"), &[], &[]));
        }
        assert_eq!(lib.undo().unwrap().label, "24");
        let mut n = 1;
        while lib.undo().is_some() {
            n += 1;
        }
        assert_eq!(n, 20);

        // private listening: nothing is recorded
        let (plays, hist) = (lib.stat("b").plays, lib.history.read().events.len());
        lib.private.store(true, Ordering::Relaxed);
        lib.bump_play("b");
        lib.bump_skip("b");
        lib.add_history(("b".into(), 1, 200, 0));
        let before = find("new").last_played;
        lib.touch_playlist("new");
        assert!(lib.stat("b").plays == plays && lib.stat("b").skips == 0 && lib.history.read().events.len() == hist);
        assert_eq!(find("new").last_played, before);
        lib.private.store(false, Ordering::Relaxed);
        lib.bump_play("b");
        lib.add_history(("b".into(), 1, 200, 0));
        assert!(lib.stat("b").plays == plays + 1 && lib.history.read().events.len() == hist + 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

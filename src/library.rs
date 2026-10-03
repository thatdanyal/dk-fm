//! Local library: folder scanning (parallel), tag/cover reading, playlists, play stats, history.
use crate::store::{self, data_dir, History, LibraryData, Playlist, Stat, Track};
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
    dirty: AtomicBool,
    history_dirty: AtomicBool,
    cover_dir: PathBuf,
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
            dirty: AtomicBool::new(false),
            history_dirty: AtomicBool::new(false),
            cover_dir,
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

    /// Delete cover art / waveform files no track uses (older than 10 minutes).
    pub fn cleanup_files(&self) {
        let d = self.data.read();
        let mut used: HashSet<String> = HashSet::new();
        for t in d.tracks.values() {
            used.extend(t.cover.clone());
            used.extend(t.thumb.clone());
        }
        let ids: HashSet<&String> = d.tracks.keys().collect();
        let old = |p: &Path| p.metadata().and_then(|m| m.modified()).map(|t| t.elapsed().map(|e| e.as_secs() > 600).unwrap_or(false)).unwrap_or(false);
        for e in std::fs::read_dir(&self.cover_dir).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if !used.contains(&n) && old(&e.path()) {
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
        let mut d = self.data.write();
        let s = d.stats.entry(id.to_string()).or_default();
        s.plays += 1;
        s.last_played = store::now_ms();
        drop(d);
        self.quiet_changed();
        self.gen.fetch_add(1, Ordering::Relaxed);
    }

    pub fn bump_skip(&self, id: &str) {
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

    pub fn new_playlist(&self, name: &str, ids: Vec<String>) -> String {
        let id = format!("{:016x}", fastrand::u64(..));
        self.upsert_playlist(Playlist { id: id.clone(), name: name.into(), track_ids: ids, source: Some("user".into()), created_at: store::now_ms(), ..Default::default() })
    }

    pub fn delete_playlist(&self, id: &str) {
        self.data.write().playlists.retain(|p| p.id != id);
        self.changed();
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

fn mtime_ms(p: &Path) -> f64 {
    p.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

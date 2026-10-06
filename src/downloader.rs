//! Download pipeline (track -> search & rank on YouTube Music | direct link -> yt-dlp -> tag with
//! lofty -> library) and background auto-sync of imported playlists.
use crate::library::{ta_key, Library};
use crate::sources::{self, Cand, Collection, Creds, Fetched, ITrack};
use crate::store::{self, Playlist, Settings};
use crate::ytdlp;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum TStatus {
    Queued,
    Searching,
    Downloading,
    Tagging,
    Done,
    Failed,
    Skipped,
    Cancelled,
}

impl TStatus {
    pub fn active(&self) -> bool {
        matches!(self, TStatus::Queued | TStatus::Searching | TStatus::Downloading | TStatus::Tagging)
    }
}

#[derive(Clone, Debug)]
pub struct JTrack {
    pub t: ITrack,
    pub status: TStatus,
    pub progress: f32,
    pub error: Option<String>,
    pub note: Option<String>,
    pub matched: Option<Cand>,
    pub candidates: Vec<Cand>,
    pub low_confidence: bool,
    pub track_id: Option<String>,
    pub forced: Option<String>,
    /// a sound quality upgrade: the library song whose file this download replaces
    pub replace: Option<String>,
}

pub struct Job {
    pub id: String,
    pub col: Collection,
    pub folder: PathBuf,
    pub fmt: String,
    /// existing library playlist that finished songs are added to (e.g. "find new songs")
    pub target_playlist: Option<String>,
    pub cancel: Arc<AtomicBool>,
    pub tracks: Vec<JTrack>,
}

pub struct Downloader {
    pub jobs: Mutex<Vec<Job>>,
    queue: Mutex<VecDeque<(String, usize)>>,
    active: AtomicUsize,
    lib: Arc<Library>,
    settings: Arc<Mutex<Settings>>,
    pub gen: std::sync::atomic::AtomicU64,
    pub notices: Mutex<Vec<String>>,
    pub repaint: Mutex<Option<Box<dyn Fn() + Send>>>,
    syncing: AtomicBool,
    /// keys of the songs being downloaded right now
    inflight: Mutex<HashSet<String>>,
}

/// A song's keys, taken in `inflight` until it's done.
struct Claim<'a>(&'a Mutex<HashSet<String>>, Vec<String>);
impl Drop for Claim<'_> {
    fn drop(&mut self) {
        let mut f = self.0.lock();
        self.1.iter().for_each(|k| {
            f.remove(k);
        });
    }
}

fn sanitize(s: &str) -> String {
    let t = clean(s);
    if t.is_empty() { "Untitled".into() } else { t }
}

/// Text that's safe inside a file or folder name on every OS (≤ 120 characters, may be empty).
fn clean(s: &str) -> String {
    let t: String = s.chars().filter(|c| !"<>:\"/\\|?*".contains(*c) && !c.is_control()).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    // (cut first: a cut can end in a dot or space too, which Windows can't have)
    t.chars().take(120).collect::<String>().trim_end_matches(['.', ' ']).to_string()
}

/// What a downloaded song's file name is built from.
pub struct NameParts<'a> {
    pub artist: &'a str,
    pub album: &'a str,
    pub title: &'a str,
    pub track: Option<u32>,
    pub year: Option<u32>,
    /// the playlist / album name, or "Downloads" (songs you got one at a time)
    pub folder: &'a str,
}

pub const NAME_TOKENS: [&str; 6] = ["{artist}", "{album}", "{title}", "{track}", "{year}", "{folder}"];

/// File path (relative to the download folder, without extension) from a pattern such as
/// `{artist}/{album}/{track} {title}`: "/" makes folders, empty folders are skipped, and every
/// part is made safe for Windows (no reserved characters or names, no trailing dots).
pub fn render_name(pattern: &str, p: &NameParts) -> PathBuf {
    let pattern = if pattern.trim().is_empty() { store::DEFAULT_PATTERN } else { pattern };
    let vals = [
        clean(p.artist).or_if_empty("Unknown"),
        clean(p.album),
        clean(p.title).or_if_empty("Untitled"),
        p.track.map(|n| format!("{n:02}")).unwrap_or_default(),
        p.year.map(|y| y.to_string()).unwrap_or_default(),
        clean(p.folder).or_if_empty("Untitled"),
    ];
    let segs: Vec<&str> = pattern.split(['/', '\\']).collect();
    let mut out = PathBuf::new();
    for (i, seg) in segs.iter().enumerate() {
        let mut s = String::new();
        let mut rest = *seg;
        while !rest.is_empty() {
            match NAME_TOKENS.iter().enumerate().find(|(_, t)| rest.starts_with(**t)) {
                Some((k, t)) => {
                    s.push_str(&vals[k]);
                    rest = &rest[t.len()..];
                }
                None => {
                    let c = rest.chars().next().unwrap();
                    if !"<>:\"|?*".contains(c) && !c.is_control() {
                        s.push(c);
                    }
                    rest = &rest[c.len_utf8()..];
                }
            }
        }
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        let s: String = s.trim_matches(['-', ' ']).chars().take(200).collect();
        let mut s = s.trim_end_matches(['.', ' ']).to_string();
        let last = i + 1 == segs.len();
        if s.is_empty() || s.chars().all(|c| c == '.') {
            if !last {
                continue;
            }
            s = "Untitled".into();
        }
        let base = s.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL") || (base.len() == 4 && (base.starts_with("COM") || base.starts_with("LPT")) && base.as_bytes()[3].is_ascii_digit()) {
            s.insert(0, '_');
        }
        out.push(s);
    }
    out
}

trait OrDefault {
    fn or_if_empty(self, d: &str) -> String;
}
impl OrDefault for String {
    fn or_if_empty(self, d: &str) -> String {
        if self.is_empty() { d.to_string() } else { self }
    }
}

/// `{folder}`: the playlist or album name; songs you get one at a time (FIND MUSIC, Songs like
/// this, Discover) go to "Downloads" (before 2.1: "Singles" / "Discovered").
pub const DOWNLOADS: &str = "Downloads";
/// `start_to` target that likes each song as it arrives (instead of adding it to a playlist)
pub const LIKED: &str = "__liked__";
fn folder_name(kind: &str, name: &str) -> String {
    match kind { "track" | "radio" => DOWNLOADS.into(), _ => name.into() }
}

/// Import matching strictness ->(minimum score to try a result, score to accept the best one
/// anyway, score below which a match is marked "check this").
pub fn thresholds(strictness: &str) -> (i32, i32, i32) {
    match strictness {
        "relaxed" => (10, 25, 35),
        "strict" => (35, 50, 60),
        _ => (20, 35, 45),
    }
}

/// Sound quality (Settings > Downloads > Sound quality) -> (file extension, yt-dlp arguments).
/// YouTube's best audio is Opus at about 160 kbps (AAC 256 kbps on YouTube Music with Premium);
/// YouTube has no lossless audio, so LOSSLESS keeps exactly what that best stream has.
pub fn fmt_args(fmt: &str) -> (&'static str, Vec<&'static str>) {
    match fmt {
        // the ~128 kbps AAC stream as it is: smallest files, never re-encoded
        "standard" => ("m4a", vec!["-f", "bestaudio[ext=m4a]/bestaudio", "-x", "--audio-format", "m4a", "--audio-quality", "0"]),
        // the best stream decoded once into FLAC: nothing more is lost on the way (about 5x larger)
        // (16-bit: the source has nothing a 24-bit file would keep, and it halves the size)
        "lossless" => ("flac", vec!["-f", "bestaudio", "-x", "--audio-format", "flac", "--postprocessor-args", "ExtractAudio:-sample_fmt s16"]),
        "mp3-320" => ("mp3", vec!["-f", "bestaudio", "-x", "--audio-format", "mp3", "--audio-quality", "320K"]),
        "mp3-v0" => ("mp3", vec!["-f", "bestaudio", "-x", "--audio-format", "mp3", "--audio-quality", "0"]),
        // "high" (the default; older settings saved "m4a"): the best stream, kept if it's already
        // 256 kbps AAC, else made into 256 kbps AAC so the Opus original's quality survives
        _ => ("m4a", vec!["-f", "bestaudio[acodec^=mp4a][abr>=190]/bestaudio", "-x", "--audio-format", "m4a", "--audio-quality", "256K"]),
    }
}

/// Short name of a sound quality setting, for the import screen and Settings.
pub fn quality_label(fmt: &str) -> &'static str {
    match fmt {
        "standard" => "STANDARD",
        "lossless" => "LOSSLESS (FLAC)",
        "mp3-320" => "MP3 320",
        "mp3-v0" => "MP3 V0",
        _ => "HIGH",
    }
}

impl Downloader {
    pub fn new(lib: Arc<Library>, settings: Arc<Mutex<Settings>>) -> Arc<Self> {
        let d = Arc::new(Self {
            jobs: Mutex::new(Vec::new()),
            queue: Mutex::new(VecDeque::new()),
            active: AtomicUsize::new(0),
            lib,
            settings,
            gen: Default::default(),
            notices: Mutex::new(Vec::new()),
            repaint: Mutex::new(None),
            syncing: AtomicBool::new(false),
            inflight: Mutex::new(HashSet::new()),
        });
        for i in 0..6 {
            let dd = d.clone();
            std::thread::Builder::new().name(format!("dl{i}")).spawn(move || dd.worker()).ok();
        }
        let ds = d.clone();
        std::thread::Builder::new().name("sync".into()).spawn(move || ds.sync_loop()).ok();
        d
    }

    fn touch(&self) {
        self.gen.fetch_add(1, Ordering::Relaxed);
        if let Some(f) = self.repaint.lock().as_ref() {
            f();
        }
    }

    pub fn creds(&self) -> Creds {
        let s = self.settings.lock();
        let settings = self.settings.clone();
        Creds {
            id: s.spotify_client_id.clone(),
            secret: s.spotify_client_secret.clone(),
            refresh: s.spotify_refresh_token.clone(),
            on_refresh: Some(Arc::new(move |r: String| {
                let mut s = settings.lock();
                s.spotify_refresh_token = r;
                s.save();
            })),
        }
    }

    /// Queue a collection; `selected` = indices to download (None = all).
    pub fn start(&self, col: Collection, selected: Option<Vec<usize>>) -> String {
        self.start_to(col, selected, None)
    }

    /// Like `start`, but every song that finishes is also added to the playlist `target`.
    pub fn start_to(&self, col: Collection, selected: Option<Vec<usize>>, target: Option<String>) -> String {
        let job_id = format!("{}-{}", col.kind, col.id);
        self.cancel(&job_id);
        self.jobs.lock().retain(|j| j.id != job_id);
        let (dir, fmt, pattern) = { let s = self.settings.lock(); (s.download_dir.clone(), s.download_format.clone(), s.name_pattern.clone()) };
        // the folder "open folder" shows (songs may go into subfolders of it, per the name pattern)
        let folder = if pattern.trim().is_empty() || pattern.starts_with("{folder}") { PathBuf::from(dir).join(sanitize(&folder_name(&col.kind, &col.name))) } else { PathBuf::from(dir) };
        // Spotify's Liked Songs: into DK.FM's own Liked (♥), and kept in sync from now on
        let spotify_liked = col.url == sources::LIKED_URL;
        let target = if spotify_liked {
            let mut s = self.settings.lock();
            if !s.spotify_liked_sync {
                s.spotify_liked_sync = true;
                s.save();
            }
            Some(LIKED.to_string())
        } else {
            target
        };
        if (col.kind == "playlist" || col.kind == "album") && !spotify_liked {
            let pid = format!("sp-{job_id}");
            // a re-sync keeps your changes: songs, order, name, cover, folder, pin
            let prev = self.lib.data.read().playlists.iter().find(|p| p.id == pid).cloned();
            let had = prev.is_some();
            let prev = prev.unwrap_or_default();
            self.lib.upsert_playlist(Playlist {
                id: pid,
                name: if had && !prev.name.is_empty() { prev.name.clone() } else { col.name.clone() },
                source: Some(col.source.clone()),
                source_url: Some(col.url.clone()),
                spotify_url: if col.source == "spotify" { Some(col.url.clone()) } else { None },
                cover: col.cover.clone().or(prev.cover.clone()),
                auto_sync: Some(prev.auto_sync.unwrap_or(true)),
                last_sync: Some(store::now_ms()),
                created_at: if had && prev.created_at > 0.0 { prev.created_at } else { store::now_ms() },
                ..prev
            });
        }
        let tracks: Vec<JTrack> = col
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| JTrack {
                t: t.clone(),
                status: if selected.as_ref().map(|s| s.contains(&i)).unwrap_or(true) { TStatus::Queued } else { TStatus::Skipped },
                progress: 0.0,
                error: None,
                note: None,
                matched: None,
                candidates: Vec::new(),
                low_confidence: false,
                track_id: None,
                forced: None,
                replace: None,
            })
            .collect();
        let mut q = self.queue.lock();
        for (i, t) in tracks.iter().enumerate() {
            if t.status == TStatus::Queued {
                q.push_back((job_id.clone(), i));
            }
        }
        drop(q);
        self.jobs.lock().push(Job { id: job_id.clone(), col, folder, fmt, target_playlist: target, cancel: Arc::new(AtomicBool::new(false)), tracks });
        // songs you already have go into the playlist now (and learn their Spotify ids), even
        // when nothing needs downloading
        self.mirror_job(&job_id);
        self.touch();
        job_id
    }

    /// Songs DK.FM downloaded in a lower quality than `fmt` (they can be downloaded again in it).
    pub fn upgradable(&self, fmt: &str) -> Vec<String> {
        let playing = self.lib.data.read();
        playing.tracks.values().filter(|t| t.youtube_id.is_some() && below(t, fmt)).map(|t| t.id.clone()).collect()
    }

    /// Download these songs again in the sound quality set now, each replacing its old file
    /// (playlists, likes and plays stay; an old file of another type goes to the Recycle Bin).
    pub fn upgrade(&self, ids: &[String]) -> String {
        let job_id = "upgrade-quality".to_string();
        self.cancel(&job_id);
        self.jobs.lock().retain(|j| j.id != job_id);
        let (dir, fmt) = { let s = self.settings.lock(); (s.download_dir.clone(), s.download_format.clone()) };
        let lib = self.lib.data.read();
        let tracks: Vec<JTrack> = ids
            .iter()
            .filter_map(|id| lib.tracks.get(id))
            .filter_map(|t| {
                let y = t.youtube_id.clone()?;
                let it = ITrack {
                    title: t.title.clone(),
                    artists: vec![t.artist.clone()],
                    album: t.album.clone(),
                    album_artist: t.album_artist.clone(),
                    year: t.year,
                    track_no: t.track,
                    duration_ms: (t.duration > 0.0).then(|| (t.duration * 1000.0) as u64),
                    spotify_id: t.spotify_id.clone(),
                    youtube_id: Some(y.clone()),
                    source_key: t.source_key.clone().unwrap_or_else(|| format!("yt:{y}")),
                    raw_title: t.title.clone(),
                    ..Default::default()
                };
                Some(JTrack { t: it, status: TStatus::Queued, progress: 0.0, error: None, note: None, matched: None, candidates: Vec::new(), low_confidence: false, track_id: None, forced: Some(y), replace: Some(t.id.clone()) })
            })
            .collect();
        drop(lib);
        let n = tracks.len();
        let col = Collection { kind: "upgrade".into(), id: "quality".into(), name: format!("Sound quality upgrade to {} ({n} songs)", quality_label(&fmt)), tracks: tracks.iter().map(|t| t.t.clone()).collect(), complete: true, ..Default::default() };
        self.queue.lock().extend((0..n).map(|i| (job_id.clone(), i)));
        self.jobs.lock().push(Job { id: job_id.clone(), col, folder: PathBuf::from(dir), fmt, target_playlist: None, cancel: Arc::new(AtomicBool::new(false)), tracks });
        self.touch();
        job_id
    }

    pub fn retry(&self, job_id: &str, idx: usize, forced: Option<String>) {
        let mut jobs = self.jobs.lock();
        let Some(j) = jobs.iter_mut().find(|j| j.id == job_id) else { return };
        if j.cancel.load(Ordering::Relaxed) {
            j.cancel = Arc::new(AtomicBool::new(false));
        }
        let Some(t) = j.tracks.get_mut(idx) else { return };
        if matches!(t.status, TStatus::Searching | TStatus::Downloading | TStatus::Tagging) {
            return;
        }
        t.status = TStatus::Queued;
        t.progress = 0.0;
        t.error = None;
        t.forced = forced;
        drop(jobs);
        self.queue.lock().push_back((job_id.to_string(), idx));
        self.touch();
    }

    pub fn cancel(&self, job_id: &str) {
        let mut jobs = self.jobs.lock();
        if let Some(j) = jobs.iter_mut().find(|j| j.id == job_id) {
            j.cancel.store(true, Ordering::Relaxed);
            for t in j.tracks.iter_mut() {
                if t.status.active() {
                    t.status = TStatus::Cancelled;
                }
            }
        }
        drop(jobs);
        self.queue.lock().retain(|(j, _)| j != job_id);
        self.touch();
    }

    pub fn clear(&self, job_id: &str) {
        self.cancel(job_id);
        self.jobs.lock().retain(|j| j.id != job_id);
        self.touch();
    }

    pub fn active_count(&self) -> usize {
        self.jobs.lock().iter().map(|j| j.tracks.iter().filter(|t| t.status.active()).count()).sum()
    }

    fn worker(self: Arc<Self>) {
        loop {
            let max = self.settings.lock().download_concurrency.clamp(1, 6) as usize;
            let task = if self.active.load(Ordering::Relaxed) < max { self.queue.lock().pop_front() } else { None };
            let Some((job_id, idx)) = task else {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            };
            self.active.fetch_add(1, Ordering::Relaxed);
            self.process(&job_id, idx);
            self.active.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Updates a song of this job (`cancel` tells it apart from a newer import of the same list).
    fn with_track<R>(&self, job_id: &str, cancel: &Arc<AtomicBool>, idx: usize, f: impl FnOnce(&mut JTrack) -> R) -> Option<R> {
        let mut jobs = self.jobs.lock();
        let r = jobs.iter_mut().find(|j| j.id == job_id && Arc::ptr_eq(&j.cancel, cancel)).and_then(|j| j.tracks.get_mut(idx)).map(f);
        drop(jobs);
        self.touch();
        r
    }

    fn process(&self, job_id: &str, idx: usize) {
        let (t, fmt, kind, job_cover, job_name, cancel, forced, replace) = {
            let jobs = self.jobs.lock();
            let Some((j, jt)) = jobs.iter().find(|j| j.id == job_id).and_then(|j| Some((j, j.tracks.get(idx)?))) else { return };
            if jt.status != TStatus::Queued || j.cancel.load(Ordering::Relaxed) {
                return;
            }
            (jt.t.clone(), j.fmt.clone(), j.col.kind.clone(), j.col.cover.clone(), j.col.name.clone(), j.cancel.clone(), jt.forced.clone(), jt.replace.clone())
        };
        // an upgrade: the song's file now (next to it goes the new one)
        let replacing: Option<(String, PathBuf)> = replace.as_ref().and_then(|id| self.lib.track(id).map(|t| (id.clone(), PathBuf::from(t.path))));
        if replace.is_some() && replacing.is_none() {
            self.with_track(job_id, &cancel, idx, |jt| {
                jt.status = TStatus::Skipped;
                jt.note = Some("No longer in your library".into());
            });
            return;
        }
        // a song you already have (from another list, or a file the scan found) isn't downloaded again
        if forced.is_none() {
            let kidx = self.lib.key_index();
            let have = lookup(&kidx, &t).filter(|id| self.lib.data.read().tracks.get(*id).is_some_and(|lt| Path::new(&lt.path).exists())).cloned();
            if let Some(id) = have {
                if !exact(&kidx, &t) {
                    self.lib.link_ids(&[(id.clone(), t.spotify_id.clone(), t.youtube_id.clone(), t.source_key.clone())]);
                }
                self.with_track(job_id, &cancel, idx, |jt| {
                    jt.status = TStatus::Done;
                    jt.note = Some("Already in your library".into());
                    jt.track_id = Some(id.clone());
                });
                self.deliver(job_id, idx, &[id]);
                self.mirror_job(job_id);
                return;
            }
        }
        // the same song in two lists imported at once: the second one waits, then finds it above
        let keys = itrack_keys(&t);
        let claim = {
            let mut f = self.inflight.lock();
            (!keys.iter().any(|k| f.contains(k))).then(|| {
                f.extend(keys.iter().cloned());
                Claim(&self.inflight, keys)
            })
        };
        let Some(_claim) = claim else {
            self.queue.lock().push_back((job_id.to_string(), idx));
            std::thread::sleep(Duration::from_millis(300));
            return;
        };
        let res = (|| -> Result<(), String> {
            let (ext, args) = fmt_args(&fmt);
            let (dir, pattern, strictness) = { let s = self.settings.lock(); (s.download_dir.clone(), s.name_pattern.clone(), s.match_strictness.clone()) };
            let (min_score, accept_score, sure_score) = thresholds(&strictness);
            let first_artist = t.artists.first().cloned().unwrap_or_else(|| "Unknown".into());
            let album = if !t.album.is_empty() { t.album.clone() } else if kind == "playlist" || kind == "album" { job_name.clone() } else { t.title.clone() };
            let rel = render_name(&pattern, &NameParts { artist: &first_artist, album: &album, title: &t.title, track: t.track_no, year: t.year, folder: &folder_name(&kind, &job_name) });
            let base = match &replacing {
                Some((_, old)) => old.with_extension(""),
                None => PathBuf::from(dir).join(&rel),
            };
            let mut out_file = PathBuf::from(format!("{}.{ext}", base.display()));
            // an upgrade to the same file type: saved beside it first, then put in its place
            let same = replacing.as_ref().is_some_and(|(_, old)| *old == out_file);
            if same {
                out_file = PathBuf::from(format!("{}.dkfm-new.{ext}", base.display()));
            }
            let _ = std::fs::create_dir_all(out_file.parent().unwrap_or(Path::new(".")));
            // already downloaded, maybe at another sound quality before it was changed
            let existing = if replacing.is_some() { None } else { std::iter::once(ext).chain(["m4a", "flac", "mp3"]).map(|e| PathBuf::from(format!("{}.{e}", base.display()))).find(|p| p.exists()) };

            if let (Some(out_file), None) = (existing, &forced) {
                let ids = self.lib.add_files(std::slice::from_ref(&out_file), |lt| apply_keys(lt, &t, None));
                self.with_track(job_id, &cancel, idx, |jt| {
                    jt.status = TStatus::Done;
                    jt.note = Some("Already downloaded".into());
                    jt.track_id = ids.first().cloned();
                });
                self.deliver(job_id, idx, &ids);
                self.mirror_job(job_id);
                return Ok(());
            }

            ytdlp::ensure()?;
            self.with_track(job_id, &cancel, idx, |jt| jt.status = TStatus::Searching);
            let want = t.duration_ms.map(|d| d as f64 / 1000.0);
            let order: Vec<Cand> = if let Some(f) = forced {
                vec![Cand { id: f, title: "(manual pick)".into(), source: "manual".into(), score: 100, ..Default::default() }]
            } else if let Some(u) = &t.direct_url {
                vec![Cand { id: t.youtube_id.clone().unwrap_or_default(), title: u.clone(), channel: first_artist.clone(), source: "direct".into(), score: 100, ..Default::default() }]
            } else {
                let cands = sources::rank(search(&t, &cancel)?, &t);
                self.with_track(job_id, &cancel, idx, |jt| jt.candidates = cands.iter().take(6).cloned().collect());
                let mut o: Vec<Cand> = cands.iter().filter(|c| c.score >= min_score && (want.is_none() || c.duration.is_none() || (c.duration.unwrap() - want.unwrap()).abs() <= 20.0)).take(3).cloned().collect();
                if o.is_empty() && cands.first().map(|c| c.score >= accept_score).unwrap_or(false) {
                    o.push(cands[0].clone());
                }
                if o.is_empty() {
                    return Err("No confident match found on YouTube".into());
                }
                o
            };

            // download: unknown-length candidates get a length filter inside the same yt-dlp call
            let tmp = std::env::temp_dir().join(format!("dkfm-{}", fastrand::u64(..)));
            std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
            let result = (|| {
                let mut picked: Option<(Cand, PathBuf)> = None;
                for c in &order {
                    self.with_track(job_id, &cancel, idx, |jt| {
                        jt.status = TStatus::Downloading;
                        jt.progress = 0.0;
                        jt.low_confidence = c.score < sure_score;
                        jt.matched = Some(c.clone());
                    });
                    let url = if c.source == "direct" { t.direct_url.clone().unwrap() } else { format!("https://music.youtube.com/watch?v={}", c.id) };
                    let mut a: Vec<String> = vec!["--no-playlist".into()];
                    if let (Some(w), None, false) = (want, c.duration, c.source == "manual" || c.source == "direct") {
                        a.push("--match-filters".into());
                        a.push(format!("duration>={} & duration<={}", (w - 12.0).floor(), (w + 12.0).ceil()));
                    }
                    a.extend(args.iter().map(|s| s.to_string()));
                    a.extend(["--write-thumbnail", "--convert-thumbnails", "jpg", "--newline", "--progress-template", "download:DKP %(progress._percent_str)s"].map(String::from));
                    a.push("-o".into());
                    a.push(tmp.join("audio.%(ext)s").display().to_string());
                    a.push("-o".into());
                    a.push(format!("thumbnail:{}", tmp.join("thumb.%(ext)s").display()));
                    a.push(url);
                    let mut last = std::time::Instant::now();
                    let mut on_line = |line: &str| {
                        if let Some(p) = line.split("DKP").nth(1).and_then(|r| r.trim().trim_end_matches('%').trim().parse::<f32>().ok()) {
                            if last.elapsed() > Duration::from_millis(300) {
                                last = std::time::Instant::now();
                                self.with_track(job_id, &cancel, idx, |jt| jt.progress = p.min(99.0));
                            }
                        }
                    };
                    let r = ytdlp::run_with(&a, Some(&mut on_line), Some(&cancel));
                    if let Err(e) = &r {
                        if cancel.load(Ordering::Relaxed) || order.len() == 1 {
                            return Err(e.clone());
                        }
                    }
                    let audio = std::fs::read_dir(&tmp).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.file_name().map(|n| n.to_string_lossy().starts_with("audio.")).unwrap_or(false) && p.extension().map(|e| e == ext).unwrap_or(false)));
                    if let Some(a) = audio {
                        picked = Some((c.clone(), a));
                        break;
                    }
                }
                let (pick, audio) = picked.ok_or("No version with the right length found on YouTube")?;
                self.with_track(job_id, &cancel, idx, |jt| {
                    jt.status = TStatus::Tagging;
                    jt.progress = 100.0;
                });
                // cover: Spotify / album art, else the YouTube thumbnail cropped square
                let cover_url = t.cover.clone().or(if kind == "album" { job_cover.clone() } else { None });
                let mut cover: Option<Vec<u8>> = cover_url.and_then(|u| crate::net::get_bytes(&u).ok());
                if cover.is_none() {
                    if let Ok(img) = image::open(tmp.join("thumb.jpg")) {
                        let s = img.width().min(img.height());
                        let sq = img.crop_imm((img.width() - s) / 2, (img.height() - s) / 2, s, s).resize(600, 600, image::imageops::FilterType::Triangle);
                        let mut buf = Vec::new();
                        if image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 88).encode_image(&sq.to_rgb8()).is_ok() {
                            cover = Some(buf);
                        }
                    }
                }
                let comment = format!("DK.FM · {}{}", if pick.id.is_empty() { t.direct_url.clone().unwrap_or_default() } else { format!("youtube:{}", pick.id) }, t.spotify_id.as_ref().map(|s| format!(" · spotify:{s}")).unwrap_or_default());
                write_tags(&audio, &t, &album, &comment, Some(pick.id.as_str()).filter(|i| !i.is_empty()), cover)?;
                // cancelled while it was being saved: it doesn't go into the library after all
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                if std::fs::rename(&audio, &out_file).is_err() {
                    std::fs::copy(&audio, &out_file).map_err(|e| e.to_string())?;
                }
                let yid = if pick.id.is_empty() { None } else { Some(pick.id.clone()) };
                let ids = match &replacing {
                    // same type: the new file takes the old one's name, so it's the same song
                    Some((_, old)) if same && std::fs::rename(&out_file, old).is_ok() => self.lib.add_files(std::slice::from_ref(old), |lt| apply_keys(lt, &t, yid.clone())),
                    // FLAC for an M4A (or the old file is busy): the new song takes over the old
                    // one's playlists, likes and plays, and the old file goes to the Recycle Bin
                    Some((old_id, _)) => {
                        let ids = self.lib.add_files(std::slice::from_ref(&out_file), |lt| apply_keys(lt, &t, yid.clone()));
                        if let Some(new_id) = ids.first().filter(|n| *n != old_id) {
                            for f in self.lib.merge_duplicates(new_id, std::slice::from_ref(old_id)) {
                                let _ = crate::system::trash(&f);
                            }
                        }
                        ids
                    }
                    None => self.lib.add_files(std::slice::from_ref(&out_file), |lt| apply_keys(lt, &t, yid.clone())),
                };
                self.with_track(job_id, &cancel, idx, |jt| {
                    jt.status = TStatus::Done;
                    jt.track_id = ids.first().cloned();
                });
                self.deliver(job_id, idx, &ids);
                Ok(())
            })();
            let _ = std::fs::remove_dir_all(&tmp);
            result?;
            self.mirror_job(job_id);
            Ok(())
        })();
        if let Err(e) = res {
            self.with_track(job_id, &cancel, idx, |jt| {
                if cancel.load(Ordering::Relaxed) {
                    jt.status = TStatus::Cancelled;
                } else {
                    jt.status = TStatus::Failed;
                    jt.error = Some(e);
                }
            });
        }
    }

    /// A finished song goes into the job's target playlist, if it has one. For a list of songs
    /// (a shared playlist) it goes in its place: after the nearest earlier song already there.
    fn deliver(&self, job_id: &str, idx: usize, ids: &[String]) {
        let Some(id) = ids.first() else { return };
        let Some((pid, ordered)) = self.jobs.lock().iter().find(|j| j.id == job_id).and_then(|j| Some((j.target_playlist.clone()?, j.tracks.len() > 1))) else { return };
        if pid == LIKED {
            self.lib.set_liked(&ids[..1], true);
            return;
        }
        if !ordered {
            self.lib.playlist_add(&pid, &ids[..1]);
            return;
        }
        let kidx = self.lib.key_index();
        let inpl: HashSet<String> = self.lib.data.read().playlists.iter().find(|p| p.id == pid).map(|p| p.track_ids.iter().cloned().collect()).unwrap_or_default();
        let after = {
            let jobs = self.jobs.lock();
            jobs.iter().find(|j| j.id == job_id).and_then(|j| j.tracks[..idx.min(j.tracks.len())].iter().rev().find_map(|jt| lookup(&kidx, &jt.t).filter(|i| inpl.contains(*i)).cloned()))
        };
        self.lib.playlist_insert(&pid, id, after.as_deref());
    }

    fn mirror_job(&self, job_id: &str) {
        let col = {
            let jobs = self.jobs.lock();
            let Some(j) = jobs.iter().find(|j| j.id == job_id) else { return };
            j.col.clone()
        };
        self.mirror(&col);
    }

    /// Make the library playlist match the source: same songs, same order. If the listing was
    /// truncated (Spotify's public page stops at 100) songs beyond it are kept.
    pub fn mirror(&self, col: &Collection) {
        if col.kind != "playlist" && col.kind != "album" {
            return;
        }
        let idx = self.lib.key_index();
        if col.url == sources::LIKED_URL {
            // Liked Songs: the ones you have get ♥ (nothing is unliked here)
            let ids: Vec<String> = col.tracks.iter().filter_map(|t| lookup(&idx, t).cloned()).collect();
            let new: Vec<String> = { let d = self.lib.data.read(); ids.into_iter().filter(|id| !d.stats.get(id).is_some_and(|s| s.liked)).collect() };
            if !new.is_empty() {
                self.lib.set_liked(&new, true);
            }
            return;
        }
        let pid = format!("sp-{}-{}", col.kind, col.id);
        let mut ordered: Vec<String> = Vec::new();
        let mut keys: Vec<Vec<String>> = Vec::new();
        let mut links = Vec::new();
        for t in &col.tracks {
            if let Some(id) = lookup(&idx, t) {
                if !exact(&idx, t) {
                    links.push((id.clone(), t.spotify_id.clone(), t.youtube_id.clone(), t.source_key.clone()));
                }
                if !ordered.contains(id) {
                    ordered.push(id.clone());
                    keys.push(itrack_keys(t));
                }
            }
        }
        self.lib.link_ids(&links);
        let complete = col.complete;
        self.lib.edit_playlist(&pid, |p| {
            if p.edited {
                // you edited it in DK.FM: keep your order and removals, only add songs that are new
                for (id, k) in ordered.iter().zip(&keys) {
                    if !p.track_ids.contains(id) && !k.iter().any(|k| p.removed.contains(k)) {
                        p.track_ids.push(id.clone());
                    }
                }
            } else if complete {
                p.track_ids = ordered;
            } else {
                let rest: Vec<String> = p.track_ids.iter().filter(|id| !ordered.contains(id)).cloned().collect();
                ordered.extend(rest);
                p.track_ids = ordered;
            }
            p.last_sync = Some(store::now_ms());
        });
    }

    // ------------------------------------------------------------ auto-sync
    fn sync_loop(self: Arc<Self>) {
        std::thread::sleep(Duration::from_secs(60));
        loop {
            let hours = self.settings.lock().sync_hours;
            if hours > 0 {
                self.sync_all(false);
            }
            let wait = if hours > 0 { hours as u64 * 3600 } else { 600 };
            let mut slept = 0;
            while slept < wait {
                std::thread::sleep(Duration::from_secs(30));
                slept += 30;
                if self.settings.lock().sync_hours != hours {
                    break;
                }
            }
        }
    }

    pub fn sync_all(&self, manual: bool) -> usize {
        if self.syncing.swap(true, Ordering::SeqCst) {
            return 0;
        }
        let mut list: Vec<Playlist> = self.lib.data.read().playlists.iter().filter(|p| p.is_imported() && (manual || p.auto_sync != Some(false))).cloned().collect();
        // Spotify's Liked Songs, synced into Liked
        if self.settings.lock().spotify_liked_sync {
            list.push(Playlist { id: "sp-playlist-liked".into(), name: "Liked Songs".into(), source: Some("spotify".into()), source_url: Some(sources::LIKED_URL.into()), ..Default::default() });
        }
        let mut added = 0;
        for p in list {
            added += self.sync_one(&p).unwrap_or(0);
        }
        self.syncing.store(false, Ordering::SeqCst);
        added
    }

    pub fn sync_one(&self, p: &Playlist) -> Result<usize, String> {
        let Some(url) = p.url() else { return Ok(0) };
        let job_id = p.id.trim_start_matches("sp-").to_string();
        if self.jobs.lock().iter().any(|j| j.id == job_id && j.tracks.iter().any(|t| t.status.active())) {
            return Ok(0);
        }
        let Fetched::Collection(col) = sources::fetch_any(url, &self.creds())? else { return Ok(0) };
        let idx = self.lib.key_index();
        let missing: Vec<usize> = col.tracks.iter().enumerate().filter(|(_, t)| lookup(&idx, t).is_none() && !itrack_keys(t).iter().any(|k| p.removed.contains(k))).map(|(i, _)| i).collect();
        let n = missing.len();
        if n > 0 {
            self.notices.lock().push(format!("Auto-sync: {n} new song{} from \"{}\"", if n == 1 { "" } else { "s" }, p.name));
            self.start(col, Some(missing));
        } else {
            self.mirror(&col);
        }
        Ok(n)
    }
}

/// This song's file is a lower sound quality than `fmt` would download now.
fn below(t: &store::Track, fmt: &str) -> bool {
    let ext = Path::new(&t.path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match fmt {
        "lossless" => ext != "flac",
        // HIGH is 256 kbps AAC: the 128 kbps STANDARD files and older MP3s are below it
        "standard" | "mp3-320" | "mp3-v0" => false,
        _ => ext != "flac" && t.bitrate.is_some_and(|b| b < 200),
    }
}

/// The library song an import track is (source key, Spotify id, YouTube id, artist + title).
fn lookup<'a>(idx: &'a HashMap<String, String>, t: &ITrack) -> Option<&'a String> {
    idx.get(&t.source_key)
        .or_else(|| t.spotify_id.as_ref().and_then(|s| idx.get(&format!("sp:{s}"))))
        .or_else(|| t.youtube_id.as_ref().and_then(|y| idx.get(&format!("yt:{y}"))))
        .or_else(|| idx.get(&ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title)))
}

/// Found by its own id (not just artist + title).
fn exact(idx: &HashMap<String, String>, t: &ITrack) -> bool {
    (!t.source_key.is_empty() && idx.contains_key(&t.source_key)) || t.spotify_id.as_ref().is_some_and(|s| idx.contains_key(&format!("sp:{s}"))) || t.youtube_id.as_ref().is_some_and(|y| idx.contains_key(&format!("yt:{y}")))
}

fn itrack_keys(t: &ITrack) -> Vec<String> {
    crate::library::track_keys(Some(&t.source_key), t.spotify_id.as_deref(), t.youtube_id.as_deref(), t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title)
}

fn apply_keys(lt: &mut store::Track, t: &ITrack, yid: Option<String>) {
    if let Some(s) = &t.spotify_id {
        lt.spotify_id = Some(s.clone());
    }
    if let Some(y) = yid.or(t.youtube_id.clone()) {
        lt.youtube_id = Some(y);
    }
    if !t.source_key.is_empty() {
        lt.source_key = Some(t.source_key.clone());
    }
}

/// YouTube Music songs + regular search results, fetched in parallel.
fn search(t: &ITrack, cancel: &Arc<AtomicBool>) -> Result<Vec<Cand>, String> {
    let q = format!("{} {}", t.artists.iter().take(2).cloned().collect::<Vec<_>>().join(" "), t.title);
    let enc = crate::net::enc(&q);
    let c1 = cancel.clone();
    let music_url = format!("https://music.youtube.com/search?q={enc}#songs");
    let h = std::thread::spawn(move || ytdlp::run_with(&["--flat-playlist".into(), "-J".into(), "--playlist-end".into(), "5".into(), music_url], None, Some(&c1)).ok());
    let plain = ytdlp::run_with(&["--flat-playlist".into(), "-J".into(), format!("ytsearch8:{q}")], None, Some(cancel)).ok();
    let music = h.join().ok().flatten();
    let mut out = Vec::new();
    for (raw, source) in [(music, "music"), (plain, "yt")] {
        let Some(raw) = raw else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else { continue };
        for (rank, e) in v["entries"].as_array().cloned().unwrap_or_default().iter().filter(|e| e["id"].as_str().map(|i| i.len() == 11).unwrap_or(false)).enumerate() {
            out.push(Cand {
                id: e["id"].as_str().unwrap().into(),
                title: e["title"].as_str().unwrap_or("").into(),
                channel: e["channel"].as_str().or(e["uploader"].as_str()).unwrap_or("").into(),
                duration: e["duration"].as_f64(),
                source: source.into(),
                rank: Some(rank),
                score: 0,
            });
        }
    }
    if out.is_empty() {
        return Err("YouTube search returned nothing".into());
    }
    Ok(out)
}

/// The song's ids in its own tags, so a library rebuilt from the files still knows which
/// Spotify / YouTube song it is.
fn set_ids(tag: &mut lofty::tag::Tag, spotify: Option<&str>, youtube: Option<&str>) {
    for (name, v) in [(crate::library::SPOTIFY_TAG, spotify), (crate::library::YOUTUBE_TAG, youtube)] {
        if let Some(v) = v {
            let key = crate::library::id_tag_key(tag.tag_type(), name);
            tag.insert_unchecked(lofty::tag::TagItem::new(key, lofty::tag::ItemValue::Text(v.to_string())));
        }
    }
}

fn write_tags(path: &Path, t: &ITrack, album: &str, comment: &str, youtube_id: Option<&str>, cover: Option<Vec<u8>>) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::*;
    use lofty::tag::{ItemKey, Tag};
    let mut tf = lofty::read_from_path(path).map_err(|e| format!("Tagging failed: {e}"))?;
    if tf.primary_tag().is_none() {
        let tt = tf.primary_tag_type();
        tf.insert_tag(Tag::new(tt));
    }
    let tag = tf.primary_tag_mut().unwrap();
    tag.set_title(t.title.clone());
    tag.set_artist(t.artists.join(", "));
    tag.set_album(album.to_string());
    tag.insert_text(ItemKey::AlbumArtist, if t.album_artist.is_empty() { t.artists.first().cloned().unwrap_or_default() } else { t.album_artist.clone() });
    if let Some(y) = t.year {
        tag.set_year(y);
    }
    if let Some(n) = t.track_no {
        tag.set_track(n);
    }
    tag.set_comment(comment.to_string());
    set_ids(tag, t.spotify_id.as_deref(), youtube_id.or(t.youtube_id.as_deref()));
    if let Some(c) = cover {
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(Picture::new_unchecked(PictureType::CoverFront, Some(MimeType::Jpeg), None, c));
    }
    tf.save_to_path(path, WriteOptions::default()).map_err(|e| format!("Tagging failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_downloads_from_the_pattern() {
        let p = NameParts { artist: "AC/DC", album: "Back in Black", title: "Hells Bells", track: Some(1), year: Some(1980), folder: "My: Mix" };
        let r = |pat: &str| render_name(pat, &p).to_string_lossy().replace('\\', "/");
        // default = how DK.FM always named files
        assert_eq!(r(store::DEFAULT_PATTERN), format!("{}/{} - {}", sanitize("My: Mix"), sanitize("AC/DC"), sanitize("Hells Bells")));
        assert_eq!(r(""), "My Mix/ACDC - Hells Bells");
        assert_eq!(r("{artist}/{album}/{track} {title}"), "ACDC/Back in Black/01 Hells Bells");
        assert_eq!(r("{year} - {album}\\{track}. {title}"), "1980 - Back in Black/01. Hells Bells");
        // missing values: empty folders are skipped, dangling separators trimmed
        let q = NameParts { artist: "", album: "", title: "", track: None, year: None, folder: "Downloads" };
        assert_eq!(render_name("{album}/{track} - {title}", &q).to_string_lossy(), "Untitled");
        assert_eq!(render_name("{artist}/{year}/{title}", &q).to_string_lossy().replace('\\', "/"), "Unknown/Untitled");
        // Windows: reserved characters and names, trailing dots, no escaping the folder
        let w = NameParts { artist: "CON", album: "a?b*", title: "Wait...", track: None, year: None, folder: "x" };
        assert_eq!(render_name("{artist}/{album}/{title}", &w).to_string_lossy().replace('\\', "/"), "_CON/ab/Wait");
        assert_eq!(render_name("../../{title}:<x>", &w).to_string_lossy(), "Waitx");
        assert_eq!(render_name("C:/Windows/{title}", &w).to_string_lossy().replace('\\', "/"), "C/Windows/Wait");
        // cut to length, still no trailing dot or space
        let long = format!("{}. Part 2", "a".repeat(119));
        assert_eq!(clean(&long), "a".repeat(119));
        assert!(!render_name(&format!("{}. {{title}}", "b".repeat(199)), &w).to_string_lossy().ends_with(['.', ' ']));
    }

    #[test]
    fn strictness_thresholds_order() {
        let (r, n, s) = (thresholds("relaxed"), thresholds("normal"), thresholds("strict"));
        assert_eq!(n, (20, 35, 45));
        assert!(r.0 < n.0 && n.0 < s.0 && r.1 < n.1 && n.1 < s.1);
        assert_eq!(thresholds("???"), n);
    }

    #[test]
    fn finished_song_goes_into_target_playlist() {
        // a file that's already downloaded finishes without yt-dlp or network
        let (_profile, dir) = crate::store::test_profile("dltest");
        let music = dir.join("Music");
        std::fs::create_dir_all(music.join("Downloads")).unwrap();
        std::fs::write(music.join("Downloads").join("Daft Punk - One More Time.m4a"), b"not really audio").unwrap();
        let lib = Library::load();
        let pid = lib.new_playlist("Mine", vec![]);
        let settings = Arc::new(Mutex::new(Settings { download_dir: music.display().to_string(), sync_hours: 0, ..Default::default() }));
        let dl = Downloader::new(lib.clone(), settings);
        let t = ITrack { title: "One More Time".into(), artists: vec!["Daft Punk".into()], youtube_id: Some("abcdefghijk".into()), direct_url: Some("https://music.youtube.com/watch?v=abcdefghijk".into()), source_key: "yt:abcdefghijk".into(), ..Default::default() };
        let col = Collection { kind: "track".into(), id: "ytabcdefghijk".into(), name: t.title.clone(), tracks: vec![t], complete: true, source: "youtube".into(), ..Default::default() };
        let job = dl.start_to(col, None, Some(pid.clone()));
        let t0 = std::time::Instant::now();
        while dl.active_count() > 0 && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let tid = dl.jobs.lock().iter().find(|j| j.id == job).and_then(|j| j.tracks[0].track_id.clone()).expect("song finished");
        let d = lib.data.read();
        assert_eq!(d.playlists.iter().find(|p| p.id == pid).unwrap().track_ids, vec![tid.clone()]);
        assert_eq!(d.tracks[&tid].youtube_id.as_deref(), Some("abcdefghijk"));
        // a plain import (no target) doesn't touch your playlists
        assert_eq!(d.playlists.len(), 1);
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn song_ids_survive_in_the_file_tags() {
        use lofty::config::WriteOptions;
        use lofty::file::TaggedFileExt;
        use lofty::prelude::*;
        let (_profile, dir) = crate::store::test_profile("tagtest");
        let lib = Library::load();
        // a tiny MP3: 20 silent MPEG-1 layer III frames (128 kbps, 44.1 kHz, 417 bytes each)
        let path = dir.join("song.mp3");
        let frame: Vec<u8> = [0xFF, 0xFB, 0x90, 0x00].into_iter().chain(std::iter::repeat_n(0, 413)).collect();
        std::fs::write(&path, frame.repeat(20)).unwrap();
        let (sp, yt) = ("4cOdK2wGLETKBW3PvgPWqT", "FGBhQbmPwH8");
        let t = ITrack { title: "One More Time".into(), artists: vec!["Daft Punk".into()], spotify_id: Some(sp.into()), source_key: format!("sp:{sp}"), ..Default::default() };
        write_tags(&path, &t, "Discovery", &format!("DK.FM · youtube:{yt} · spotify:{sp}"), Some(yt), None).unwrap();
        let ids = |lib: &Library| lib.read_track(&path).map(|t| (t.spotify_id, t.youtube_id, t.source_key)).unwrap();
        assert_eq!(ids(&lib), (Some(sp.into()), Some(yt.into()), Some(format!("sp:{sp}"))));
        // the TXXX fields alone are enough (comment gone)
        let mut tf = lofty::read_from_path(&path).unwrap();
        tf.primary_tag_mut().unwrap().remove_comment();
        tf.save_to_path(&path, WriteOptions::default()).unwrap();
        assert_eq!(ids(&lib).0.as_deref(), Some(sp));
        // untagged files saved by yt-dlp: "<title> [<id>]"
        for (name, want) in [("Song [dQw4w9WgXcQ].mp3", Some("dQw4w9WgXcQ")), ("Song [Instrumentl].mp3", None)] {
            std::fs::write(dir.join(name), frame.repeat(20)).unwrap();
            let t = lib.read_track(&dir.join(name)).unwrap();
            assert_eq!((t.youtube_id.as_deref(), t.source_key), (want, want.map(|y| format!("yt:{y}"))));
        }
        // songs downloaded before: the "DK.FM · youtube:… · spotify:…" comment alone
        let mut tag = lofty::tag::Tag::new(lofty::tag::TagType::Mp4Ilst);
        tag.set_comment(format!("DK.FM · youtube:{yt} · spotify:{sp}"));
        assert_eq!(crate::library::ids_from_tag(&tag), (Some(sp.into()), Some(yt.into())));
        tag.set_comment("DK.FM · https://soundcloud.com/a/b".into());
        assert_eq!(crate::library::ids_from_tag(&tag), (None, None));
        // MP4: iTunes freeform atoms, and back
        let mut tag = lofty::tag::Tag::new(lofty::tag::TagType::Mp4Ilst);
        set_ids(&mut tag, Some(sp), Some(yt));
        let ilst: lofty::mp4::Ilst = tag.into();
        let atom = lofty::mp4::AtomIdent::Freeform { mean: "com.apple.iTunes".into(), name: "SPOTIFY_TRACK_ID".into() };
        assert!(ilst.get(&atom).is_some());
        let back: lofty::tag::Tag = ilst.into();
        assert_eq!(crate::library::ids_from_tag(&back), (Some(sp.into()), Some(yt.into())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_links_songs_you_already_have_to_spotify() {
        let (_profile, dir) = crate::store::test_profile("linktest");
        let lib = Library::load();
        // a song from a rebuilt library: no Spotify id, only artist + title
        lib.data.write().tracks.insert("a".into(), store::Track { id: "a".into(), path: "/x/a.m4a".into(), artist: "Daft Punk".into(), title: "One More Time".into(), ..Default::default() });
        let settings = Arc::new(Mutex::new(Settings { download_dir: dir.join("Music").display().to_string(), sync_hours: 0, ..Default::default() }));
        let dl = Downloader::new(lib.clone(), settings);
        let sp = "0DiWol3AO6WpXZgp0goxAV";
        let t = ITrack { title: "One More Time".into(), artists: vec!["Daft Punk".into()], spotify_id: Some(sp.into()), source_key: format!("sp:{sp}"), ..Default::default() };
        let col = Collection { kind: "playlist".into(), id: "x".into(), name: "Mix".into(), tracks: vec![t], complete: true, source: "spotify".into(), ..Default::default() };
        // nothing to download (you have it): it still lands in the playlist and learns its id
        dl.start(col, Some(vec![]));
        let d = lib.data.read();
        assert_eq!(d.playlists.iter().find(|p| p.id == "sp-playlist-x").unwrap().track_ids, ["a"]);
        assert_eq!((d.tracks["a"].spotify_id.as_deref(), d.tracks["a"].source_key.clone()), (Some(sp), Some(format!("sp:{sp}"))));
        drop(d);
        assert_eq!(lib.key_index().get(&format!("sp:{sp}")).map(|s| s.as_str()), Some("a"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn importing_again_never_downloads_twice() {
        use lofty::file::TaggedFileExt;
        let dir = std::env::temp_dir().join(format!("dkfm-reimport-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("DKFM_USER_DATA", &dir);
        let music = dir.join("Music");
        std::fs::create_dir_all(music.join("Old")).unwrap();
        // a song downloaded long ago (somewhere else, other file name): only its tags say what it is
        let old = music.join("Old").join("track01.mp3");
        let frame: Vec<u8> = [0xFF, 0xFB, 0x90, 0x00].into_iter().chain(std::iter::repeat(0).take(413)).collect();
        std::fs::write(&old, frame.repeat(20)).unwrap();
        let sp = |n: u8| format!("{}{n}", "0DiWol3AO6WpXZgp0goxA"); // 22 characters
        let song = |n: u8, a: &str, t: &str| ITrack { title: t.into(), artists: vec![a.into()], spotify_id: Some(sp(n)), source_key: format!("sp:{}", sp(n)), ..Default::default() };
        let (a, b, c) = (song(1, "Daft Punk", "Aerodynamic"), song(2, "Sade", "Kiss of Life"), song(3, "Drake", "Hold On"));
        write_tags(&old, &a, "Discovery", "", None, None).unwrap();
        assert!(lofty::read_from_path(&old).unwrap().primary_tag().is_some());
        let lib = Library::load();
        // library.json lost: the scan finds it by the id in its tags
        lib.scan(vec![music.clone()]);
        let t0 = std::time::Instant::now();
        while (lib.scanning.load(Ordering::SeqCst) || lib.data.read().tracks.is_empty()) && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let a_id = crate::library::id_for(&old);
        // c: you have it under another name (artist + title); b: its file is already where it goes
        lib.data.write().tracks.insert("c".into(), store::Track { id: "c".into(), path: old.display().to_string(), artist: "Drake".into(), title: "Hold On".into(), ..Default::default() });
        std::fs::create_dir_all(music.join("Mix")).unwrap();
        std::fs::write(music.join("Mix").join("Sade - Kiss of Life.m4a"), b"x").unwrap();
        let settings = Arc::new(Mutex::new(Settings { download_dir: music.display().to_string(), sync_hours: 0, ..Default::default() }));
        let dl = Downloader::new(lib.clone(), settings);
        let col = Collection { kind: "playlist".into(), id: "mix".into(), name: "Mix".into(), tracks: vec![b.clone(), a.clone(), c.clone()], complete: true, source: "spotify".into(), ..Default::default() };
        for round in 0..2 {
            // everything ticked (as if you pressed ALL): nothing needs yt-dlp or the network
            let job = dl.start(col.clone(), None);
            let t0 = std::time::Instant::now();
            while dl.active_count() > 0 && t0.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(20));
            }
            let jobs = dl.jobs.lock();
            assert_eq!(jobs.len(), 1, "one job per list");
            let notes: Vec<_> = jobs[0].tracks.iter().map(|t| (t.status.clone(), t.note.clone().unwrap_or_default())).collect();
            assert!(notes.iter().all(|(s, _)| *s == TStatus::Done), "round {round}: {notes:?}");
            assert_eq!(notes[1].1, "Already in your library");
            assert_eq!(jobs[0].id, job);
            drop(jobs);
            let d = lib.data.read();
            assert_eq!(d.playlists.len(), 1, "no second playlist");
            let b_id = crate::library::id_for(&music.join("Mix").join("Sade - Kiss of Life.m4a"));
            assert_eq!(d.playlists[0].track_ids, [b_id, a_id.clone(), "c".into()], "source order");
            assert_eq!(d.tracks.len(), 3);
            assert_eq!(d.tracks["c"].spotify_id.as_deref(), Some(sp(3).as_str()), "linked for next time");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Real search + download into a playlist (network): `cargo test -- --ignored` with
    /// DKFM_USER_DATA pointing at a test profile (songs land in its Music folder).
    #[test]
    #[ignore]
    fn downloads_search_result_into_playlist() {
        let dir = std::path::PathBuf::from(std::env::var("DKFM_USER_DATA").expect("use a test profile"));
        let lib = Library::load();
        let pid = lib.new_playlist("Found online", vec![]);
        let settings = Arc::new(Mutex::new(Settings { download_dir: dir.join("Music").display().to_string(), sync_hours: 0, ..Default::default() }));
        let dl = Downloader::new(lib.clone(), settings);
        let t = sources::search("daft punk one more time radio edit", "songs", 1).unwrap().remove(0);
        let col = Collection { kind: "track".into(), id: format!("yt{}", t.youtube_id.clone().unwrap()), name: t.title.clone(), tracks: vec![t], complete: true, source: "youtube".into(), ..Default::default() };
        dl.start_to(col, None, Some(pid.clone()));
        let t0 = std::time::Instant::now();
        while dl.active_count() > 0 && t0.elapsed() < Duration::from_secs(180) {
            std::thread::sleep(Duration::from_millis(500));
        }
        let j = dl.jobs.lock()[0].tracks[0].clone();
        assert_eq!(j.status, TStatus::Done, "{:?}", j.error);
        let d = lib.data.read();
        let tr = &d.tracks[j.track_id.as_ref().unwrap()];
        eprintln!("{} - {} [{}] {}", tr.artist, tr.title, tr.album, tr.path);
        assert!(std::path::Path::new(&tr.path).starts_with(std::path::absolute(&dir).unwrap()));
        assert_eq!(d.playlists.iter().find(|p| p.id == pid).unwrap().track_ids, vec![tr.id.clone()]);
        drop(d);
        lib.delete_playlist(&pid);
        lib.flush();
    }

    #[test]
    fn knows_which_songs_a_better_quality_would_upgrade() {
        let t = |path: &str, kbps: Option<u32>| store::Track { path: path.into(), bitrate: kbps, ..Default::default() };
        // LOSSLESS: everything that isn't FLAC yet
        assert!(below(&t("a.m4a", Some(256)), "lossless") && below(&t("a.mp3", Some(320)), "lossless") && !below(&t("a.flac", Some(900)), "lossless"));
        // HIGH: the 128 kbps files, not the 256 ones or FLAC
        assert!(below(&t("a.m4a", Some(128)), "high") && !below(&t("a.m4a", Some(256)), "high") && !below(&t("a.flac", Some(900)), "high"));
        // a lower setting never "upgrades" anything
        assert!(!below(&t("a.m4a", Some(128)), "standard") && !below(&t("a.m4a", Some(128)), "mp3-320"));
    }
}

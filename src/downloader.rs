//! Download pipeline (track -> search & rank on YouTube Music | direct link -> yt-dlp -> tag with
//! lofty -> library) and background auto-sync of imported playlists.
use crate::library::{ta_key, Library};
use crate::sources::{self, Cand, Collection, Creds, Fetched, ITrack};
use crate::store::{self, Playlist, Settings};
use crate::ytdlp;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::VecDeque;
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
}

pub struct Job {
    pub id: String,
    pub col: Collection,
    pub folder: PathBuf,
    pub fmt: String,
    pub created: f64,
    pub silent: bool,
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
}

fn sanitize(s: &str) -> String {
    let t: String = s.chars().filter(|c| !"<>:\"/\\|?*".contains(*c) && !c.is_control()).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim_end_matches(['.', ' ']).chars().take(120).collect::<String>();
    if t.is_empty() { "Untitled".into() } else { t }
}

fn fmt_args(fmt: &str) -> (&'static str, Vec<&'static str>) {
    match fmt {
        "mp3-320" => ("mp3", vec!["-x", "--audio-format", "mp3", "--audio-quality", "320K"]),
        "mp3-v0" => ("mp3", vec!["-x", "--audio-format", "mp3", "--audio-quality", "0"]),
        _ => ("m4a", vec!["-f", "bestaudio[ext=m4a]/bestaudio", "-x", "--audio-format", "m4a", "--audio-quality", "0"]),
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
    pub fn start(&self, col: Collection, selected: Option<Vec<usize>>, silent: bool) -> String {
        let job_id = format!("{}-{}", col.kind, col.id);
        self.cancel(&job_id);
        self.jobs.lock().retain(|j| j.id != job_id);
        let (dir, fmt) = { let s = self.settings.lock(); (s.download_dir.clone(), s.download_format.clone()) };
        let sub = match col.kind.as_str() { "track" => "Singles".to_string(), "radio" => "Discovered".to_string(), _ => col.name.clone() };
        let folder = PathBuf::from(dir).join(sanitize(&sub));
        if col.kind == "playlist" || col.kind == "album" {
            let pid = format!("sp-{job_id}");
            let prev = self.lib.data.read().playlists.iter().find(|p| p.id == pid).cloned();
            self.lib.upsert_playlist(Playlist {
                id: pid,
                name: col.name.clone(),
                source: Some(col.source.clone()),
                source_url: Some(col.url.clone()),
                spotify_url: if col.source == "spotify" { Some(col.url.clone()) } else { None },
                cover: col.cover.clone(),
                auto_sync: Some(prev.as_ref().and_then(|p| p.auto_sync).unwrap_or(true)),
                last_sync: Some(store::now_ms()),
                edited: prev.as_ref().map(|p| p.edited).unwrap_or(false),
                removed: prev.as_ref().map(|p| p.removed.clone()).unwrap_or_default(),
                track_ids: prev.map(|p| p.track_ids).unwrap_or_default(),
                created_at: store::now_ms(),
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
            })
            .collect();
        let mut q = self.queue.lock();
        for (i, t) in tracks.iter().enumerate() {
            if t.status == TStatus::Queued {
                q.push_back((job_id.clone(), i));
            }
        }
        drop(q);
        self.jobs.lock().push(Job { id: job_id.clone(), col, folder, fmt, created: store::now_ms(), silent, cancel: Arc::new(AtomicBool::new(false)), tracks });
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
            let res = self.process(&job_id, idx);
            self.active.fetch_sub(1, Ordering::Relaxed);
            if let Err(e) = res {
                let mut jobs = self.jobs.lock();
                if let Some(j) = jobs.iter_mut().find(|j| j.id == job_id) {
                    let cancelled = j.cancel.load(Ordering::Relaxed);
                    if let Some(t) = j.tracks.get_mut(idx) {
                        if cancelled {
                            t.status = TStatus::Cancelled;
                        } else {
                            t.status = TStatus::Failed;
                            t.error = Some(e);
                        }
                    }
                }
                drop(jobs);
                self.touch();
            }
        }
    }

    fn with_track<R>(&self, job_id: &str, idx: usize, f: impl FnOnce(&mut JTrack) -> R) -> Option<R> {
        let mut jobs = self.jobs.lock();
        let r = jobs.iter_mut().find(|j| j.id == job_id).and_then(|j| j.tracks.get_mut(idx)).map(f);
        drop(jobs);
        self.touch();
        r
    }

    fn process(&self, job_id: &str, idx: usize) -> Result<(), String> {
        let (t, folder, fmt, kind, job_cover, job_name, cancel, forced) = {
            let jobs = self.jobs.lock();
            let j = jobs.iter().find(|j| j.id == job_id).ok_or("job gone")?;
            let jt = j.tracks.get(idx).ok_or("track gone")?;
            if jt.status != TStatus::Queued || j.cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            (jt.t.clone(), j.folder.clone(), j.fmt.clone(), j.col.kind.clone(), j.col.cover.clone(), j.col.name.clone(), j.cancel.clone(), jt.forced.clone())
        };
        ytdlp::ensure()?;
        let (ext, args) = fmt_args(&fmt);
        let _ = std::fs::create_dir_all(&folder);
        let first_artist = t.artists.first().cloned().unwrap_or_else(|| "Unknown".into());
        let out_file = folder.join(format!("{} - {}.{ext}", sanitize(&first_artist), sanitize(&t.title)));

        if out_file.exists() && forced.is_none() {
            let ids = self.lib.add_files(&[out_file.clone()], |lt| apply_keys(lt, &t, None));
            self.with_track(job_id, idx, |jt| {
                jt.status = TStatus::Done;
                jt.note = Some("Already downloaded".into());
                jt.track_id = ids.first().cloned();
            });
            self.mirror_job(job_id);
            return Ok(());
        }

        self.with_track(job_id, idx, |jt| jt.status = TStatus::Searching);
        let want = t.duration_ms.map(|d| d as f64 / 1000.0);
        let order: Vec<Cand> = if let Some(f) = forced {
            vec![Cand { id: f, title: "(manual pick)".into(), source: "manual".into(), score: 100, ..Default::default() }]
        } else if let Some(u) = &t.direct_url {
            vec![Cand { id: t.youtube_id.clone().unwrap_or_default(), title: u.clone(), channel: first_artist.clone(), source: "direct".into(), score: 100, ..Default::default() }]
        } else {
            let cands = sources::rank(search(&t, &cancel)?, &t);
            self.with_track(job_id, idx, |jt| jt.candidates = cands.iter().take(6).cloned().collect());
            let mut o: Vec<Cand> = cands.iter().filter(|c| c.score >= 20 && (want.is_none() || c.duration.is_none() || (c.duration.unwrap() - want.unwrap()).abs() <= 20.0)).take(3).cloned().collect();
            if o.is_empty() && cands.first().map(|c| c.score >= 35).unwrap_or(false) {
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
                self.with_track(job_id, idx, |jt| {
                    jt.status = TStatus::Downloading;
                    jt.progress = 0.0;
                    jt.low_confidence = c.score < 45;
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
                            self.with_track(job_id, idx, |jt| jt.progress = p.min(99.0));
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
            self.with_track(job_id, idx, |jt| {
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
            let album = if !t.album.is_empty() { t.album.clone() } else if kind == "playlist" || kind == "album" { job_name.clone() } else { t.title.clone() };
            let comment = format!("DK.FM · {}{}", if pick.id.is_empty() { t.direct_url.clone().unwrap_or_default() } else { format!("youtube:{}", pick.id) }, t.spotify_id.as_ref().map(|s| format!(" · spotify:{s}")).unwrap_or_default());
            write_tags(&audio, &t, &album, &comment, cover)?;
            if std::fs::rename(&audio, &out_file).is_err() {
                std::fs::copy(&audio, &out_file).map_err(|e| e.to_string())?;
            }
            let yid = if pick.id.is_empty() { None } else { Some(pick.id.clone()) };
            let ids = self.lib.add_files(&[out_file.clone()], |lt| apply_keys(lt, &t, yid.clone()));
            self.with_track(job_id, idx, |jt| {
                jt.status = TStatus::Done;
                jt.track_id = ids.first().cloned();
            });
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&tmp);
        result?;
        self.mirror_job(job_id);
        Ok(())
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
        let pid = format!("sp-{}-{}", col.kind, col.id);
        let idx = self.lib.key_index();
        let mut ordered: Vec<String> = Vec::new();
        let mut keys: Vec<Vec<String>> = Vec::new();
        for t in &col.tracks {
            let id = idx.get(&t.source_key).or_else(|| t.spotify_id.as_ref().and_then(|s| idx.get(&format!("sp:{s}")))).or_else(|| idx.get(&ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title)));
            if let Some(id) = id {
                if !ordered.contains(id) {
                    ordered.push(id.clone());
                    keys.push(itrack_keys(t));
                }
            }
        }
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
        let list: Vec<Playlist> = self.lib.data.read().playlists.iter().filter(|p| p.is_imported() && (manual || p.auto_sync != Some(false))).cloned().collect();
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
        let have = |t: &ITrack| idx.contains_key(&t.source_key) || t.spotify_id.as_ref().map(|s| idx.contains_key(&format!("sp:{s}"))).unwrap_or(false) || idx.contains_key(&ta_key(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title));
        let missing: Vec<usize> = col.tracks.iter().enumerate().filter(|(_, t)| !have(t) && !itrack_keys(t).iter().any(|k| p.removed.contains(k))).map(|(i, _)| i).collect();
        let n = missing.len();
        if n > 0 {
            self.notices.lock().push(format!("Auto-sync: {n} new song{} from \"{}\"", if n == 1 { "" } else { "s" }, p.name));
            self.start(col, Some(missing), true);
        } else {
            self.mirror(&col);
        }
        Ok(n)
    }
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

fn write_tags(path: &Path, t: &ITrack, album: &str, comment: &str, cover: Option<Vec<u8>>) -> Result<(), String> {
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
    if let Some(c) = cover {
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(Picture::new_unchecked(PictureType::CoverFront, Some(MimeType::Jpeg), None, c));
    }
    tf.save_to_path(path, WriteOptions::default()).map_err(|e| format!("Tagging failed: {e}"))
}

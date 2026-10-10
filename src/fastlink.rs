//! Faster listening before downloading. Getting a song's audio link from YouTube is what makes ▶
//! slow (yt-dlp: ~9 s for one song, but only ~1 s more for each extra song in the same run), so
//! songs on screen have their links looked up ahead, in the background, several per run. ▶ then
//! only fetches the file, in 1 MB pieces at once (under a second), instead of waiting on yt-dlp.
//! Links last a few hours; anything that goes wrong falls back to the normal way.
use crate::sources::ITrack;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// songs per yt-dlp run, and runs at once
const BATCH: usize = 6;
const WORKERS: usize = 2;
/// songs waiting to be looked up (the newest on screen first; older ones drop off)
const QUEUE_MAX: usize = 36;
const PIECE: u64 = 1 << 20;
const PIECES_AT_ONCE: usize = 6;

struct Link {
    url: String,
    size: u64,
    expires: i64,
}

#[derive(Default)]
struct State {
    links: HashMap<String, Link>,
    queue: VecDeque<String>,
    busy: HashSet<String>,
    failed: HashSet<String>,
    workers: usize,
}

static S: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

fn now() -> i64 {
    crate::store::now_secs()
}

fn fresh(l: &Link) -> bool {
    l.expires > now() + 120
}

/// Look up this song's audio link in the background (YouTube songs; once per song).
pub fn warm(t: &ITrack) {
    let Some(y) = t.youtube_id.clone() else { return };
    let mut s = S.lock();
    if s.links.get(&y).is_some_and(fresh) || s.busy.contains(&y) || s.failed.contains(&y) {
        return;
    }
    if let Some(i) = s.queue.iter().position(|q| *q == y) {
        if i < BATCH {
            return;
        }
        s.queue.remove(i);
    }
    s.queue.push_front(y);
    s.queue.truncate(QUEUE_MAX);
    if s.workers < WORKERS {
        s.workers += 1;
        std::thread::Builder::new().name("links".into()).spawn(worker).ok();
    }
}

fn worker() {
    loop {
        let batch: Vec<String> = {
            let mut s = S.lock();
            let n = s.queue.len().min(BATCH);
            let b: Vec<String> = s.queue.drain(..n).collect();
            if b.is_empty() {
                s.workers -= 1;
                return;
            }
            s.busy.extend(b.iter().cloned());
            b
        };
        let found = resolve(&batch);
        let mut s = S.lock();
        for y in batch {
            s.busy.remove(&y);
            if !found.contains_key(&y) {
                s.failed.insert(y);
            }
        }
        s.links.retain(|_, l| fresh(l));
        s.links.extend(found);
    }
}

/// One yt-dlp run for several songs: id -> link (M4A audio only: what DK.FM plays as it is).
fn resolve(ids: &[String]) -> HashMap<String, Link> {
    if crate::ytdlp::ensure().is_err() {
        return HashMap::new();
    }
    let mut args: Vec<String> = ["--no-playlist", "--ignore-errors", "-f", "bestaudio[ext=m4a]", "--print", "%(id)s %(filesize)s %(url)s"].iter().map(|s| s.to_string()).collect();
    args.extend(ids.iter().map(|y| format!("https://music.youtube.com/watch?v={y}")));
    let out = crate::ytdlp::run_with(&args, None, None).unwrap_or_default();
    parse(&out)
}

fn parse(out: &str) -> HashMap<String, Link> {
    let mut m = HashMap::new();
    for line in out.lines() {
        let mut p = line.trim().splitn(3, ' ');
        let (Some(id), Some(size), Some(url)) = (p.next(), p.next(), p.next()) else { continue };
        if !url.starts_with("https://") {
            continue;
        }
        let param = |k: &str| url.split(['?', '&']).find_map(|kv| kv.strip_prefix(k)).and_then(|v| v.parse::<u64>().ok());
        let Some(size) = size.parse::<u64>().ok().or_else(|| param("clen=")).filter(|n| *n > 0) else { continue };
        let expires = param("expire=").map(|e| e as i64).unwrap_or_else(|| now() + 3600);
        m.insert(id.to_string(), Link { url: url.to_string(), size, expires });
    }
    m
}

/// Fetch the song to `dest` (an .m4a) using its looked-up link, waiting up to `wait` for a look-up
/// already under way. None = no link for it: use the normal way.
pub fn fetch(t: &ITrack, dest: &Path, wait: Duration) -> Option<Result<(), String>> {
    let y = t.youtube_id.clone()?;
    let t0 = Instant::now();
    loop {
        {
            let mut s = S.lock();
            if let Some(l) = s.links.get(&y).filter(|l| fresh(l)) {
                let (url, size) = (l.url.clone(), l.size);
                drop(s);
                let r = download(&url, size, dest);
                if r.is_err() {
                    S.lock().links.remove(&y);
                }
                return Some(r);
            }
            let waiting = s.busy.contains(&y) || s.queue.contains(&y);
            if !waiting || t0.elapsed() > wait {
                return None;
            }
            // pressed ▶: it goes next
            if let Some(i) = s.queue.iter().position(|q| *q == y) {
                s.queue.remove(i);
                s.queue.push_front(y.clone());
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The file in 1 MB pieces, several at once (YouTube slows down one long download).
fn download(url: &str, size: u64, dest: &Path) -> Result<(), String> {
    let n = size.div_ceil(PIECE);
    let pieces: Vec<Mutex<Option<Result<Vec<u8>, String>>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicU64::new(0);
    std::thread::scope(|sc| {
        for _ in 0..PIECES_AT_ONCE.min(n as usize) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= n {
                    return;
                }
                let (a, b) = (i * PIECE, ((i + 1) * PIECE).min(size) - 1);
                let r = crate::net::get_bytes(&format!("{url}&range={a}-{b}")).and_then(|v| if v.len() as u64 == b - a + 1 { Ok(v) } else { Err("cut short".into()) });
                let bad = r.is_err();
                *pieces[i as usize].lock() = Some(r);
                if bad {
                    next.store(n, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
            });
        }
    });
    let tmp = dest.with_extension("part");
    let mut all = Vec::with_capacity(size as usize);
    for p in pieces {
        all.extend(p.into_inner().unwrap_or_else(|| Err("missing piece".into()))?);
    }
    std::fs::write(&tmp, all).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_ytdlp_lines() {
        let out = "abcdefghijk 5801081 https://rr5.googlevideo.com/videoplayback?expire=1791635393&clen=5801081&x=1\nERROR: nope\nzzzzzzzzzzz NA https://rr1.googlevideo.com/videoplayback?expire=1791635000&clen=42\nbad line\n";
        let m = super::parse(out);
        assert_eq!(m["abcdefghijk"].size, 5801081);
        assert_eq!(m["abcdefghijk"].expires, 1791635393);
        assert_eq!(m["zzzzzzzzzzz"].size, 42, "size from the link when yt-dlp doesn't know it");
        assert_eq!(m.len(), 2);
    }

    /// DKFM_LINK_TEST=1 cargo test --release fetches_a_song_fast -- --ignored --nocapture
    #[test]
    #[ignore]
    fn fetches_a_song_fast() {
        let t = crate::sources::ITrack { youtube_id: Some("fJ9rUzIMcZQ".into()), ..Default::default() };
        let t0 = std::time::Instant::now();
        super::warm(&t);
        let dest = std::env::temp_dir().join("dkfm-linktest.m4a");
        let r = super::fetch(&t, &dest, std::time::Duration::from_secs(60));
        println!("look-up + fetch: {:?} {:?}", t0.elapsed(), r);
        let t1 = std::time::Instant::now();
        let r = super::fetch(&t, &dest, std::time::Duration::from_secs(60));
        println!("fetch with the link known: {:?} {:?}, {} bytes", t1.elapsed(), r, std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0));
        let _ = std::fs::remove_file(dest);
    }
}

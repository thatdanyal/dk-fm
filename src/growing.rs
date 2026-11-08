//! Play while it downloads: a file still being fetched (a preview, a mix's new song) is
//! registered here with its full size and how much of its start is on disk so far. Anything
//! that opens it through `open` reads it as a normal file, waiting a moment whenever it reaches
//! the part that hasn't arrived yet, so ▶ starts as soon as the first piece is in instead of
//! after the whole song.
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use symphonia::core::io::MediaSource;

/// How long a read waits for a piece that hasn't arrived before giving up.
const STALL: Duration = Duration::from_secs(20);

pub struct Progress {
    pub size: u64,
    /// bytes from the start that are on disk (no gaps)
    avail: AtomicU64,
    failed: AtomicBool,
}

impl Progress {
    pub fn set_avail(&self, n: u64) {
        self.avail.fetch_max(n.min(self.size), Ordering::Release);
    }
    pub fn avail(&self) -> u64 {
        self.avail.load(Ordering::Acquire)
    }
    pub fn fail(&self) {
        self.failed.store(true, Ordering::Release);
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub fn complete(&self) -> bool {
        self.avail() >= self.size
    }
}

static FILES: LazyLock<Mutex<HashMap<PathBuf, Arc<Progress>>>> = LazyLock::new(Default::default);

/// A file of `size` bytes is about to be written at `path`, from its start (see `Progress`).
pub fn start(path: &Path, size: u64) -> Arc<Progress> {
    let p = Arc::new(Progress { size, avail: AtomicU64::new(0), failed: AtomicBool::new(false) });
    FILES.lock().insert(path.to_path_buf(), p.clone());
    p
}

/// It's all there (or it failed): readers that already have it keep their handle.
pub fn finish(path: &Path) {
    FILES.lock().remove(path);
}

pub fn get(path: &Path) -> Option<Arc<Progress>> {
    FILES.lock().get(path).cloned()
}

/// A file being written: reads wait for the bytes they need.
struct Growing {
    f: std::fs::File,
    p: Arc<Progress>,
    pos: u64,
}

impl Read for Growing {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.pos >= self.p.size {
            return Ok(0);
        }
        let t0 = Instant::now();
        while self.p.avail() <= self.pos {
            if self.p.failed() || t0.elapsed() > STALL {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the song stopped downloading"));
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        let n = (buf.len() as u64).min(self.p.avail() - self.pos) as usize;
        let got = self.f.read(&mut buf[..n])?;
        self.pos += got as u64;
        Ok(got)
    }
}

impl Seek for Growing {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let p = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(d) => self.p.size as i64 + d,
            SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if p < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = self.f.seek(SeekFrom::Start(p as u64))?;
        Ok(self.pos)
    }
}

impl MediaSource for Growing {
    /// Not until it's all there: opening an MP4 that can seek reads every fragment's header
    /// (spread over the whole file) before the first sound, which would wait for the whole
    /// download. Unseekable, the decoder starts at the first fragment and finds the rest as it
    /// plays; going back works again once the download is done.
    fn is_seekable(&self) -> bool {
        self.p.complete()
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.p.size)
    }
}

/// Open a song's file for decoding: as it is, or (still downloading) one that waits for its bytes.
pub fn open(path: &Path) -> std::io::Result<Box<dyn MediaSource>> {
    let f = std::fs::File::open(path)?;
    Ok(match get(path) {
        Some(p) if !p.complete() => Box::new(Growing { f, p, pos: 0 }),
        _ => Box::new(f),
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, SeekFrom, Write};

    #[test]
    fn reads_wait_for_the_rest() {
        let path = std::env::temp_dir().join(format!("dkfm-growing-{}.bin", std::process::id()));
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&data[..1000]).unwrap();
        f.flush().unwrap();
        let p = super::start(&path, data.len() as u64);
        p.set_avail(1000);
        let mut r = super::open(&path).unwrap();
        assert_eq!(r.byte_len(), Some(10_000));
        let (d2, p2) = (data.clone(), p.clone());
        let w = std::thread::spawn(move || {
            for c in d2[1000..].chunks(1500) {
                std::thread::sleep(std::time::Duration::from_millis(20));
                f.write_all(c).unwrap();
                f.flush().unwrap();
                p2.set_avail(p2.avail() + c.len() as u64);
            }
        });
        let mut got = Vec::new();
        r.read_to_end(&mut got).unwrap();
        w.join().unwrap();
        assert_eq!(got, data, "the whole file, read while it was written");
        r.seek(SeekFrom::End(-10)).unwrap();
        let mut tail = Vec::new();
        r.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, &data[data.len() - 10..]);
        super::finish(&path);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_failed_download_ends_the_read() {
        let path = std::env::temp_dir().join(format!("dkfm-growing-fail-{}.bin", std::process::id()));
        std::fs::write(&path, [1u8; 10]).unwrap();
        let p = super::start(&path, 100);
        p.set_avail(10);
        p.fail();
        let mut r = super::open(&path).unwrap();
        let mut buf = Vec::new();
        assert!(r.read_to_end(&mut buf).is_err());
        assert_eq!(buf.len(), 10);
        super::finish(&path);
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod decode_tests {
    use std::io::Write;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    /// DKFM_FRAG=/path/to/fragmented.m4a cargo test plays_while_downloading -- --ignored --nocapture
    /// (YouTube's audio is a fragmented MP4: the decoder must start with only its first piece)
    #[test]
    #[ignore]
    fn plays_while_downloading() {
        let src = std::env::var("DKFM_FRAG").expect("DKFM_FRAG");
        let data = std::fs::read(src).unwrap();
        let path = std::env::temp_dir().join(format!("dkfm-growing-dec-{}.m4a", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        let first = (data.len() / 8).max(4096);
        f.write_all(&data[..first]).unwrap();
        f.flush().unwrap();
        let p = super::start(&path, data.len() as u64);
        p.set_avail(first as u64);
        let mut hint = Hint::new();
        hint.with_extension("m4a");
        let t0 = std::time::Instant::now();
        let mss = MediaSourceStream::new(super::open(&path).unwrap(), Default::default());
        let mut reader = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).unwrap().format;
        let opened = t0.elapsed();
        assert!(!p.complete(), "opened with only the first piece");
        let track = reader.default_track().unwrap().clone();
        let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).unwrap();
        // the rest arrives while it plays
        let (d2, p2) = (data.clone(), p.clone());
        let w = std::thread::spawn(move || {
            for c in d2[first..].chunks(64 * 1024) {
                std::thread::sleep(std::time::Duration::from_millis(5));
                f.write_all(c).unwrap();
                f.flush().unwrap();
                p2.set_avail(p2.avail() + c.len() as u64);
            }
        });
        let mut frames = 0u64;
        while let Ok(pk) = reader.next_packet() {
            frames += dec.decode(&pk).map(|b| b.frames() as u64).unwrap_or(0);
        }
        w.join().unwrap();
        let secs = frames as f64 / track.codec_params.sample_rate.unwrap() as f64;
        println!("opened in {opened:?}, decoded {secs:.1} s");
        assert!(secs > 115.0, "the whole song played: {secs}");
        super::finish(&path);
        let _ = std::fs::remove_file(path);
    }
}

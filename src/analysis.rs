//! One decode per song, ever: waveform peaks (seek bar, cached in waves/<id>.bin like before)
//! and loudness for volume matching (gated block RMS, EBU R128 style). Runs on its own thread.
use crate::library::Library;
use crate::store::data_dir;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const PEAKS: usize = 1200;
const TARGET_DB: f32 = -17.0;

pub type GainHook = Box<dyn Fn(&str, f32) + Send>;

pub struct Analyzer {
    tx: Sender<(String, PathBuf)>,
    pub peaks: Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>>,
    pending: Arc<Mutex<HashSet<String>>>,
    lib: Arc<Library>,
}

impl Analyzer {
    pub fn start(lib: Arc<Library>, on_gain: GainHook) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<(String, PathBuf)>();
        let peaks = Arc::new(Mutex::new(HashMap::new()));
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let (p, pend, l) = (peaks.clone(), pending.clone(), lib.clone());
        std::thread::Builder::new().name("analysis".into()).spawn(move || worker(rx, p, pend, l, on_gain)).ok();
        Self { tx, peaks, pending, lib }
    }

    /// Make sure peaks (and loudness) exist for a song; cheap if already known.
    pub fn request(&self, id: &str) {
        if self.peaks.lock().contains_key(id) && self.lib.track(id).map(|t| t.gain_v2.is_some()).unwrap_or(true) {
            return;
        }
        if !self.pending.lock().insert(id.to_string()) {
            return;
        }
        if let Some(t) = self.lib.track(id) {
            let _ = self.tx.send((id.to_string(), PathBuf::from(t.path)));
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<Vec<u8>>> {
        self.peaks.lock().get(id).cloned()
    }
}

fn worker(rx: Receiver<(String, PathBuf)>, cache: Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>>, pending: Arc<Mutex<HashSet<String>>>, lib: Arc<Library>, on_gain: GainHook) {
    let wave_dir = data_dir().join("waves");
    while let Ok((id, path)) = rx.recv() {
        let file = wave_dir.join(format!("{id}.bin"));
        let known_gain = lib.track(&id).and_then(|t| t.gain_v2);
        let saved = std::fs::read(&file).ok().filter(|b| b.len() == PEAKS);
        if let (Some(p), Some(_)) = (&saved, known_gain) {
            cache.lock().insert(id.clone(), Arc::new(p.clone()));
        // (a damaged file that trips up the decoder is skipped, not the end of all analysis)
        } else if let Some((peaks, gain)) = std::panic::catch_unwind(|| analyze(&path)).ok().flatten() {
            let _ = std::fs::write(&file, &peaks);
            cache.lock().insert(id.clone(), Arc::new(peaks));
            lib.set_gain(&id, gain);
            on_gain(&id, gain);
        }
        let mut c = cache.lock();
        if c.len() > 120 {
            if let Some(k) = c.keys().next().cloned() {
                c.remove(&k);
            }
        }
        drop(c);
        pending.lock().remove(&id);
    }
}

/// Decode the whole file once: 50 ms RMS blocks -> waveform buckets + gated loudness.
fn analyze(path: &PathBuf) -> Option<(Vec<u8>, f32)> {
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let mut reader = probed.format;
    let track = reader.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;
    let tid = track.id;
    let sr = track.codec_params.sample_rate.unwrap_or(44100) as usize;
    let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).ok()?;
    let block = (sr / 20).max(1); // 50 ms
    let mut blocks: Vec<f32> = Vec::new(); // mean square per block
    let (mut acc, mut n) = (0f64, 0usize);
    let mut peak = 0f32;
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    loop {
        let pkt = match reader.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if pkt.track_id() != tid {
            continue;
        }
        let Ok(d) = dec.decode(&pkt) else { continue };
        let spec = *d.spec();
        let ch = spec.channels.count().max(1);
        if sbuf.as_ref().map(|b| b.capacity() < d.capacity() * ch).unwrap_or(true) {
            sbuf = Some(SampleBuffer::new(d.capacity() as u64, spec));
        }
        let sb = sbuf.as_mut().unwrap();
        sb.copy_interleaved_ref(d);
        for fr in sb.samples().chunks(ch) {
            let mut ms = 0f32;
            for &s in fr.iter().take(2) {
                ms += s * s;
                peak = peak.max(s.abs());
            }
            acc += (ms / fr.len().min(2) as f32) as f64;
            n += 1;
            if n == block {
                blocks.push((acc / n as f64) as f32);
                acc = 0.0;
                n = 0;
            }
        }
    }
    if n > 0 {
        blocks.push((acc / n as f64) as f32);
    }
    if blocks.is_empty() {
        return None;
    }
    // waveform: RMS per bucket, normalised
    let per = blocks.len() as f32 / PEAKS as f32;
    let mut wave = vec![0f32; PEAKS];
    for (i, w) in wave.iter_mut().enumerate() {
        let a = (i as f32 * per) as usize;
        let b = (((i + 1) as f32 * per) as usize).max(a + 1).min(blocks.len());
        let a = a.min(b.saturating_sub(1));
        let m: f32 = blocks[a..b].iter().sum::<f32>() / (b - a).max(1) as f32;
        *w = m.sqrt();
    }
    let max = wave.iter().cloned().fold(0f32, f32::max).max(1e-9);
    let peaks: Vec<u8> = wave.iter().map(|v| ((v / max) * 255.0).round() as u8).collect();
    // loudness: 400 ms blocks, absolute gate -60 dB, relative gate -10 dB
    let lb: Vec<f32> = blocks.chunks(8).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    let db = |x: f32| 10.0 * x.max(1e-12).log10();
    let abs: Vec<f32> = lb.iter().cloned().filter(|&x| db(x) > -60.0).collect();
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let rel: Vec<f32> = abs.iter().cloned().filter(|&x| db(x) > db(mean(&abs)) - 10.0).collect();
    let mut gain = if rel.is_empty() { 0.0 } else { TARGET_DB - db(mean(&rel)) };
    // never boost past the song's own peak, keep within a sane range
    gain = gain.min(-20.0 * peak.max(1e-6).log10() - 1.0).min(6.0).max(-12.0);
    Some((peaks, (gain * 10.0).round() / 10.0))
}

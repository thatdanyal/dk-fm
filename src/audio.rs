//! Audio engine: a mixer thread decodes up to two "decks" (current + next for gapless playback and
//! crossfades), runs the DSP chain (per-song trim -> preamp/10-band EQ -> leveler -> limiter) and
//! feeds a lock-free ring buffer that the sound-card callback drains. Volume is applied in the
//! callback so it reacts instantly.
use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

pub const EQ_FREQS: [f32; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
pub const SCOPE_LEN: usize = 4096;

pub enum Cmd {
    Play { id: String, path: PathBuf, start: f64, gain_db: f32, autoplay: bool },
    Preload { id: String, path: PathBuf, gain_db: f32 },
    ClearNext,
    Pause,
    Resume,
    Stop,
    Seek(f64),
    SetEq { enabled: bool, gains: [f32; 10], preamp: f32 },
    SetCrossfade(f32),
    SetGain { id: String, db: f32 },
    SetLimiter(bool),
    SetLeveler(bool),
}

#[derive(Debug, Clone)]
pub enum Event {
    /// A new current song (after Play, a gapless advance, or a crossfade start).
    Current,
    /// Moved on to the preloaded song on its own (gapless / crossfade).
    Advanced(String),
    /// Reached the end with nothing preloaded.
    Ended,
    Error(String, String),
    /// Now playing on this output device (the default changed or the old one went away), or
    /// "" = there's no sound device to play on.
    Device(String),
}

#[derive(Default, Clone)]
pub struct Status {
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub current: Option<String>,
}

pub struct Shared {
    pub status: Mutex<Status>,
    /// Last SCOPE_LEN stereo frames after the DSP chain (before volume): visualizer + VU.
    pub scope: Mutex<VecDeque<[f32; 2]>>,
    pub volume: AtomicU32,
    pub muted: AtomicBool,
    flush: AtomicU64,
    pub sample_rate: AtomicU32,
}

pub struct Engine {
    tx: Sender<Cmd>,
    pub events: Receiver<Event>,
    pub shared: Arc<Shared>,
}

impl Engine {
    pub fn start() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (etx, erx) = crossbeam_channel::unbounded();
        let shared = Arc::new(Shared {
            status: Mutex::new(Status::default()),
            scope: Mutex::new(VecDeque::with_capacity(SCOPE_LEN)),
            volume: AtomicU32::new(0.64f32.to_bits()),
            muted: AtomicBool::new(false),
            flush: AtomicU64::new(0),
            sample_rate: AtomicU32::new(48000),
        });
        let sh = shared.clone();
        std::thread::Builder::new().name("mixer".into()).spawn(move || mixer_thread(rx, etx, sh)).expect("mixer thread");
        Self { tx, events: erx, shared }
    }
    pub fn sender(&self) -> Sender<Cmd> {
        self.tx.clone()
    }
    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
    /// Perceptual volume 0..1 (applied squared, like most players).
    pub fn set_volume(&self, v: f32) {
        self.shared.volume.store((v * v).to_bits(), Ordering::Relaxed);
    }
    pub fn set_muted(&self, m: bool) {
        self.shared.muted.store(m, Ordering::Relaxed);
    }
    pub fn status(&self) -> Status {
        self.shared.status.lock().clone()
    }
}

// ------------------------------------------------------------------------------------------ deck

struct Deck {
    id: String,
    path: PathBuf,
    gain_db: f32,
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    src_rate: u32,
    dst_rate: u32,
    resampler: Option<rubato::FastFixedIn<f32>>,
    pending: [Vec<f32>; 2],
    out: VecDeque<f32>,
    frames_out: u64,
    base: f64,
    duration: f64,
    eof: bool,
    trim: f32,
    trim_target: f32,
    sample_buf: Option<SampleBuffer<f32>>,
}

impl Deck {
    fn open(id: &str, path: &PathBuf, dst_rate: u32, gain_db: f32) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
            .map_err(|e| format!("unsupported file: {e}"))?;
        let reader = probed.format;
        let track = reader.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or("no audio track")?;
        let params = track.codec_params.clone();
        let track_id = track.id;
        let decoder = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).map_err(|e| format!("unsupported codec: {e}"))?;
        let src_rate = params.sample_rate.unwrap_or(44100);
        let duration = match (params.n_frames, params.time_base) {
            (Some(n), Some(tb)) => {
                let t = tb.calc_time(n);
                t.seconds as f64 + t.frac
            }
            (Some(n), None) => n as f64 / src_rate as f64,
            _ => 0.0,
        };
        let resampler = if src_rate != dst_rate {
            use rubato::{FastFixedIn, PolynomialDegree};
            FastFixedIn::<f32>::new(dst_rate as f64 / src_rate as f64, 1.0, PolynomialDegree::Cubic, 1024, 2).ok()
        } else {
            None
        };
        let trim = db_to_lin(gain_db);
        Ok(Self {
            id: id.to_string(),
            path: path.clone(),
            gain_db,
            reader,
            decoder,
            track_id,
            src_rate,
            dst_rate,
            resampler,
            pending: [Vec::new(), Vec::new()],
            out: VecDeque::with_capacity(16384),
            frames_out: 0,
            base: 0.0,
            duration,
            eof: false,
            trim,
            trim_target: trim,
            sample_buf: None,
        })
    }

    fn position(&self) -> f64 {
        self.base + self.frames_out as f64 / self.dst_rate as f64
    }

    fn remaining(&self) -> f64 {
        if self.duration > 0.0 { self.duration - self.position() } else { f64::INFINITY }
    }

    fn seek(&mut self, secs: f64) {
        let secs = secs.max(0.0);
        let to = SeekTo::Time { time: Time::new(secs.trunc() as u64, secs.fract()), track_id: Some(self.track_id) };
        if self.reader.seek(SeekMode::Accurate, to).is_ok() {
            self.decoder.reset();
            self.out.clear();
            self.pending = [Vec::new(), Vec::new()];
            if let Some(r) = self.resampler.as_mut() {
                use rubato::Resampler;
                r.reset();
            }
            self.base = secs;
            self.frames_out = 0;
            self.eof = false;
        }
    }

    /// Decode one packet into the output queue (stereo, device rate). Returns false at end of file.
    fn decode_more(&mut self) -> bool {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(p) => p,
                Err(_) => {
                    self.flush_resampler();
                    return false;
                }
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue, // skip a corrupt frame
                Err(_) => {
                    self.flush_resampler();
                    return false;
                }
            };
            let spec = *decoded.spec();
            let frames = decoded.frames();
            if frames == 0 {
                continue;
            }
            let sb = match &mut self.sample_buf {
                Some(b) if b.capacity() >= frames * spec.channels.count() => b,
                _ => {
                    self.sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
                    self.sample_buf.as_mut().unwrap()
                }
            };
            sb.copy_interleaved_ref(decoded);
            let ch = spec.channels.count().max(1);
            let s = sb.samples();
            let (mut l, mut r) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
            for f in 0..frames {
                let a = s[f * ch];
                let b = if ch > 1 { s[f * ch + 1] } else { a };
                l.push(a);
                r.push(b);
            }
            self.push_stereo(l, r);
            return true;
        }
    }

    fn push_stereo(&mut self, l: Vec<f32>, r: Vec<f32>) {
        if self.resampler.is_none() {
            for (a, b) in l.into_iter().zip(r) {
                self.out.push_back(a);
                self.out.push_back(b);
            }
            return;
        }
        use rubato::Resampler;
        self.pending[0].extend(l);
        self.pending[1].extend(r);
        let rs = self.resampler.as_mut().unwrap();
        loop {
            let need = rs.input_frames_next();
            if self.pending[0].len() < need {
                break;
            }
            let chunk = [self.pending[0][..need].to_vec(), self.pending[1][..need].to_vec()];
            self.pending[0].drain(..need);
            self.pending[1].drain(..need);
            if let Ok(o) = rs.process(&chunk, None) {
                for i in 0..o[0].len() {
                    self.out.push_back(o[0][i]);
                    self.out.push_back(o[1][i]);
                }
            }
        }
    }

    fn flush_resampler(&mut self) {
        use rubato::Resampler;
        if let Some(rs) = self.resampler.as_mut() {
            if !self.pending[0].is_empty() {
                let chunk = [std::mem::take(&mut self.pending[0]), std::mem::take(&mut self.pending[1])];
                if let Ok(o) = rs.process_partial(Some(&chunk), None) {
                    for i in 0..o[0].len() {
                        self.out.push_back(o[0][i]);
                        self.out.push_back(o[1][i]);
                    }
                }
            }
        }
        let _ = self.src_rate;
    }

    /// Up to `frames` stereo frames; fewer means the song ended.
    fn read(&mut self, frames: usize, dst: &mut Vec<f32>) {
        dst.clear();
        while self.out.len() < frames * 2 && !self.eof {
            if !self.decode_more() {
                self.eof = true;
            }
        }
        let n = (frames * 2).min(self.out.len());
        dst.extend(self.out.drain(..n));
        // smooth per-song trim changes over the chunk (no clicks)
        let step = (self.trim_target - self.trim) / (dst.len().max(1) as f32 / 2.0);
        for f in dst.chunks_mut(2) {
            self.trim += step;
            f[0] *= self.trim;
            if f.len() > 1 {
                f[1] *= self.trim;
            }
        }
        self.trim = self.trim_target;
        self.frames_out += (n / 2) as u64;
    }

    fn finished(&self) -> bool {
        self.eof && self.out.is_empty()
    }
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

// ------------------------------------------------------------------------------------------ DSP

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32, b1: f32, b2: f32, a1: f32, a2: f32,
    z1: [f32; 2], z2: [f32; 2],
}

impl Biquad {
    /// RBJ cookbook filters: kind 0 = low shelf, 1 = peaking, 2 = high shelf.
    fn design(kind: u8, freq: f32, gain_db: f32, q: f32, sr: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * (freq / sr).min(0.49);
        let (sin, cos) = w0.sin_cos();
        let (b0, b1, b2, a0, a1, a2);
        match kind {
            1 => {
                let alpha = sin / (2.0 * q);
                b0 = 1.0 + alpha * a;
                b1 = -2.0 * cos;
                b2 = 1.0 - alpha * a;
                a0 = 1.0 + alpha / a;
                a1 = -2.0 * cos;
                a2 = 1.0 - alpha / a;
            }
            _ => {
                let alpha = sin / 2.0 * ((a + 1.0 / a) * (1.0 / 0.9 - 1.0) + 2.0).sqrt();
                let sq = 2.0 * a.sqrt() * alpha;
                if kind == 0 {
                    b0 = a * ((a + 1.0) - (a - 1.0) * cos + sq);
                    b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos);
                    b2 = a * ((a + 1.0) - (a - 1.0) * cos - sq);
                    a0 = (a + 1.0) + (a - 1.0) * cos + sq;
                    a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos);
                    a2 = (a + 1.0) + (a - 1.0) * cos - sq;
                } else {
                    b0 = a * ((a + 1.0) + (a - 1.0) * cos + sq);
                    b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos);
                    b2 = a * ((a + 1.0) + (a - 1.0) * cos - sq);
                    a0 = (a + 1.0) - (a - 1.0) * cos + sq;
                    a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos);
                    a2 = (a + 1.0) - (a - 1.0) * cos - sq;
                }
            }
        }
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: [0.0; 2], z2: [0.0; 2] }
    }

    #[inline]
    fn run(&mut self, x: f32, c: usize) -> f32 {
        let y = self.b0 * x + self.z1[c];
        self.z1[c] = self.b1 * x - self.a1 * y + self.z2[c];
        self.z2[c] = self.b2 * x - self.a2 * y;
        y
    }

    /// Magnitude response in dB at `f` (for drawing the EQ curve).
    pub fn response_db(&self, f: f32, sr: f32) -> f32 {
        let w = 2.0 * std::f32::consts::PI * f / sr;
        let (c1, s1) = (w.cos(), w.sin());
        let (c2, s2) = ((2.0 * w).cos(), (2.0 * w).sin());
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).max(1e-12).log10()
    }
}

/// EQ response curve for the UI (dB at each frequency), matching what the engine applies.
pub fn eq_curve(gains: &[f32; 10], preamp: f32, freqs: &[f32]) -> Vec<f32> {
    let sr = 48000.0;
    let filters: Vec<Biquad> = EQ_FREQS.iter().enumerate().map(|(i, &f)| Biquad::design(if i == 0 { 0 } else if i == 9 { 2 } else { 1 }, f, gains[i], 1.1, sr)).collect();
    freqs.iter().map(|&f| preamp + filters.iter().map(|b| b.response_db(f, sr)).sum::<f32>()).collect()
}

struct Dsp {
    sr: f32,
    eq_on: bool,
    preamp: f32,
    filters: Vec<Biquad>,
    leveler: bool,
    lev_env: f32,
    limiter: bool,
    lim_gain: f32,
}

impl Dsp {
    fn new(sr: f32) -> Self {
        Self { sr, eq_on: false, preamp: 1.0, filters: Vec::new(), leveler: false, lev_env: 0.0, limiter: true, lim_gain: 1.0 }
    }
    fn set_eq(&mut self, enabled: bool, gains: [f32; 10], preamp: f32) {
        self.eq_on = enabled;
        self.preamp = db_to_lin(preamp);
        let old = std::mem::take(&mut self.filters);
        self.filters = EQ_FREQS
            .iter()
            .enumerate()
            .map(|(i, &f)| {
                let mut b = Biquad::design(if i == 0 { 0 } else if i == 9 { 2 } else { 1 }, f, gains[i], 1.1, self.sr);
                if let Some(o) = old.get(i) {
                    b.z1 = o.z1;
                    b.z2 = o.z2;
                }
                b
            })
            .collect();
    }
    fn process(&mut self, buf: &mut [f32]) {
        for fr in buf.chunks_mut(2) {
            let (mut l, mut r) = (fr[0], fr[1]);
            if self.eq_on {
                l *= self.preamp;
                r *= self.preamp;
                for f in self.filters.iter_mut() {
                    l = f.run(l, 0);
                    r = f.run(r, 1);
                }
            }
            if self.leveler {
                // gentle RMS compressor (-22 dB threshold, 3.5:1) with make-up gain
                let p = 0.5 * (l * l + r * r);
                self.lev_env += (p - self.lev_env) * 0.0005;
                let lvl = 10.0 * self.lev_env.max(1e-9).log10();
                let over = (lvl + 22.0).max(0.0);
                let g = db_to_lin(-over * (1.0 - 1.0 / 3.5)) * 1.4;
                l *= g;
                r *= g;
            }
            if self.limiter {
                // transparent peak limiter at -1 dB: instant attack, smooth release
                let peak = l.abs().max(r.abs()) * self.lim_gain;
                let thr = 0.891;
                if peak > thr {
                    self.lim_gain *= thr / peak;
                } else {
                    self.lim_gain += (1.0 - self.lim_gain) * 0.0004;
                }
                l *= self.lim_gain;
                r *= self.lim_gain;
            }
            fr[0] = l;
            fr[1] = r;
        }
    }
}

// ------------------------------------------------------------------------------------------ mixer

/// The sound card stream DK.FM plays into, and the ring buffer that feeds it.
struct Output {
    stream: cpal::Stream,
    prod: rtrb::Producer<f32>,
    sr: u32,
    ring_len: usize,
    /// the device's name, to notice when Windows / macOS / Linux switches the default output
    name: String,
    dead: Arc<AtomicBool>,
}

/// Open the system's default output device (playing).
fn open_output(host: &cpal::Host, shared: &Arc<Shared>) -> Option<Output> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let device = host.default_output_device()?;
    let name = device.name().unwrap_or_default();
    let config = device.default_output_config().ok()?;
    let (sr, channels) = (config.sample_rate().0, config.channels() as usize);
    let ring_len = (sr as usize / 6) * 2; // ~170 ms of stereo audio
    let (prod, mut cons) = rtrb::RingBuffer::<f32>::new(ring_len);
        let sh = shared.clone();
        let mut last_flush = sh.flush.load(Ordering::Relaxed);
        let mut vol = 0.0f32;
        // the device went away (unplugged, Bluetooth/wireless headset off): the mixer opens the new default
        let dead = Arc::new(AtomicBool::new(false));
        let dead2 = dead.clone();
        let err = move |e| {
            eprintln!("audio stream error: {e}");
            dead2.store(true, Ordering::Relaxed);
        };
        let stream_cfg: cpal::StreamConfig = config.clone().into();
        // DKFM_AUDIO_DEBUG=1: print the loudness actually sent to the sound card, once a second
        let debug = std::env::var_os("DKFM_AUDIO_DEBUG").is_some();
        let (mut dbg_sum, mut dbg_n) = (0.0f64, 0usize);
        let mut fill = move |out: &mut [f32]| {
            let f = sh.flush.load(Ordering::Relaxed);
            if f != last_flush {
                last_flush = f;
                let n = cons.slots();
                if let Ok(ch) = cons.read_chunk(n) {
                    ch.commit_all();
                }
            }
            let target = if sh.muted.load(Ordering::Relaxed) { 0.0 } else { f32::from_bits(sh.volume.load(Ordering::Relaxed)) };
            for frame in out.chunks_mut(channels) {
                let l = cons.pop().unwrap_or(0.0);
                let r = cons.pop().unwrap_or(0.0);
                vol += (target - vol) * 0.002;
                for (i, s) in frame.iter_mut().enumerate() {
                    *s = if i % 2 == 0 { l * vol } else { r * vol };
                }
            }
            if debug {
                dbg_sum += out.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>();
                dbg_n += out.len();
                if dbg_n >= sr as usize * channels {
                    eprintln!("audio out: rms {:.5} vol {:.3} target {:.3}", (dbg_sum / dbg_n as f64).sqrt(), vol, target);
                    (dbg_sum, dbg_n) = (0.0, 0);
                }
            }
        };
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(&stream_cfg, move |o: &mut [f32], _| fill(o), err, None).ok()?,
            cpal::SampleFormat::I16 => {
                let mut tmp = Vec::new();
                device
                    .build_output_stream(
                        &stream_cfg,
                        move |o: &mut [i16], _| {
                            tmp.resize(o.len(), 0.0);
                            fill(&mut tmp);
                            for (d, s) in o.iter_mut().zip(&tmp) {
                                *d = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                            }
                        },
                        err,
                        None,
                    )
                    .ok()?
            }
            _ => {
                let mut tmp = Vec::new();
                device
                    .build_output_stream(
                        &stream_cfg,
                        move |o: &mut [u16], _| {
                            tmp.resize(o.len(), 0.0);
                            fill(&mut tmp);
                            for (d, s) in o.iter_mut().zip(&tmp) {
                                *d = ((s.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16;
                            }
                        },
                        err,
                        None,
                    )
                    .ok()?
            }
        };
        stream.play().ok()?;
    shared.sample_rate.store(sr, Ordering::Relaxed);
    Some(Output { stream, prod, sr, ring_len, name, dead })
}

/// Name of the system's default output device right now.
fn default_output_name(host: &cpal::Host) -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    host.default_output_device().and_then(|d| d.name().ok())
}

fn mixer_thread(rx: Receiver<Cmd>, events: Sender<Event>, shared: Arc<Shared>) {
    let host = cpal::default_host();
    let mut out = open_output(&host, &shared);
    let mut sr = out.as_ref().map(|o| o.sr).unwrap_or(48000);
    let mut last_check = std::time::Instant::now();
    let mut eq_state: Option<(bool, [f32; 10], f32)> = None;
    let mut warned_none = false;

    let mut cur: Option<Deck> = None;
    let mut next: Option<Deck> = None;
    let mut playing = false;
    // the sound card stream is stopped after a few seconds of silence (paused, nothing loaded):
    // an idle DK.FM (e.g. in the tray) then costs no CPU, and the OS audio engine can sleep
    let mut idle_since: Option<std::time::Instant> = Some(std::time::Instant::now());
    let mut stream_on = out.is_some();
    let mut xfade = 0.0f32;
    let mut fading: Option<(f64, f64)> = None; // (elapsed, length) seconds
    let mut dsp = Dsp::new(sr as f32);
    const CHUNK: usize = 512;
    let mut a = Vec::with_capacity(CHUNK * 2);
    let mut b = Vec::with_capacity(CHUNK * 2);
    let mut mix = vec![0f32; CHUNK * 2];
    let flush = |sh: &Shared| {
        sh.flush.fetch_add(1, Ordering::Relaxed);
    };

    loop {
        // commands: block briefly when idle, otherwise just drain
        let room = out.as_ref().map(|o| o.prod.slots()).unwrap_or(0);
        let wait = if playing && out.is_none() {
            Duration::from_millis(250) // no sound device: look for one again shortly
        } else if playing && room < CHUNK * 2 {
            Duration::from_millis(4)
        } else if playing {
            Duration::ZERO
        } else if stream_on {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(1000)
        };
        let first = if wait.is_zero() {
            rx.try_recv().ok()
        } else {
            match rx.recv_timeout(wait) {
                Ok(c) => Some(c),
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                Err(_) => None,
            }
        };
        let mut cmds: Vec<Cmd> = first.into_iter().collect();
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        for c in cmds {
            match c {
                Cmd::Play { id, path, start, gain_db, autoplay } => {
                    flush(&shared);
                    fading = None;
                    // reuse the preloaded deck when it's the same song (instant start)
                    let reuse = next.as_ref().map(|n| n.id == id).unwrap_or(false) && start == 0.0;
                    let deck = if reuse { next.take().map(Ok) } else { Some(Deck::open(&id, &path, sr, gain_db)) };
                    match deck {
                        Some(Ok(mut d)) => {
                            if start > 0.0 {
                                d.seek(start);
                            }
                            cur = Some(d);
                            playing = autoplay;
                            let _ = events.send(Event::Current);
                        }
                        Some(Err(e)) => {
                            cur = None;
                            playing = false;
                            let _ = events.send(Event::Error(id, e));
                        }
                        None => {}
                    }
                }
                Cmd::Preload { id, path, gain_db } => {
                    // (mid-crossfade, `next` is the song fading in: leave it)
                    if fading.is_none() && next.as_ref().is_none_or(|n| n.id != id) {
                        next = Deck::open(&id, &path, sr, gain_db).ok();
                    }
                }
                Cmd::ClearNext => {
                    if fading.is_none() {
                        next = None;
                    }
                }
                Cmd::Pause => {
                    playing = false;
                    flush(&shared);
                }
                Cmd::Resume => playing = cur.is_some(),
                Cmd::Stop => {
                    playing = false;
                    cur = None;
                    next = None;
                    fading = None;
                    flush(&shared);
                }
                Cmd::Seek(t) => {
                    if let Some(d) = cur.as_mut() {
                        if fading.is_some() {
                            // finish the crossfade instantly
                            fading = None;
                        }
                        d.seek(t.min((d.duration - 0.2).max(0.0)));
                        flush(&shared);
                    }
                }
                Cmd::SetEq { enabled, gains, preamp } => {
                    eq_state = Some((enabled, gains, preamp));
                    dsp.set_eq(enabled, gains, preamp);
                }
                Cmd::SetCrossfade(s) => xfade = s,
                Cmd::SetGain { id, db } => {
                    for d in [cur.as_mut(), next.as_mut()].into_iter().flatten() {
                        if d.id == id {
                            d.trim_target = db_to_lin(db);
                        }
                    }
                }
                Cmd::SetLimiter(on) => dsp.limiter = on,
                Cmd::SetLeveler(on) => dsp.leveler = on,
            }
        }

        // follow the default output device: a headset or AirPods connecting, a device unplugged or
        // switched in the system's sound settings. Checked about once a second while playing (and
        // right away when playback starts), never while idle.
        let starting = playing && !stream_on;
        let lost = out.as_ref().is_some_and(|o| o.dead.load(Ordering::Relaxed));
        if lost || (playing && (starting || out.is_none() || last_check.elapsed() > Duration::from_secs(1))) {
            last_check = std::time::Instant::now();
            let want = default_output_name(&host);
            let have = out.as_ref().map(|o| o.name.clone());
            if lost || (want.is_some() && want != have) {
                // what's still in the old ring was never heard: pick the song up from where we are
                let heard = match (cur.as_ref(), out.as_ref()) {
                    (Some(c), Some(o)) if !lost => (c.position() - (o.ring_len - o.prod.slots()) as f64 / 2.0 / sr as f64).max(0.0),
                    (Some(c), _) => c.position(),
                    _ => 0.0,
                };
                out = None;
                stream_on = false;
                if let Some(o) = open_output(&host, &shared) {
                    stream_on = true;
                    if o.sr != sr {
                        sr = o.sr;
                        let (limiter, leveler) = (dsp.limiter, dsp.leveler);
                        dsp = Dsp::new(sr as f32);
                        (dsp.limiter, dsp.leveler) = (limiter, leveler);
                        if let Some((e, g, p)) = eq_state {
                            dsp.set_eq(e, g, p);
                        }
                        next = None; // reloaded by the player when it's needed
                        fading = None;
                    }
                    if let Some(c) = cur.take() {
                        let trim = c.trim_target;
                        cur = match Deck::open(&c.id, &c.path, sr, c.gain_db) {
                            Ok(mut d) => {
                                d.trim_target = trim;
                                d.trim = trim;
                                d.seek(heard);
                                Some(d)
                            }
                            Err(_) => Some(c),
                        };
                    }
                    let _ = events.send(Event::Device(o.name.clone()));
                    out = Some(o);
                }
            }
            if out.is_none() && !warned_none {
                warned_none = true;
                let _ = events.send(Event::Device(String::new()));
            } else if out.is_some() {
                warned_none = false;
            }
        }
        // start / stop the sound card stream
        if let Some(stream) = out.as_ref().map(|o| &o.stream) {
            use cpal::traits::StreamTrait;
            if playing {
                idle_since = None;
                if !stream_on {
                    stream_on = stream.play().is_ok();
                    if !stream_on {
                        playing = false; // the device went away
                    }
                }
            } else if stream_on {
                let since = *idle_since.get_or_insert_with(std::time::Instant::now);
                if since.elapsed() > Duration::from_secs(3) && stream.pause().is_ok() {
                    stream_on = false;
                }
            }
        }
        // produce audio while there's room in the ring
        enum Act {
            None,
            FadeDone,
            FadeAbort,
            Gapless(usize),
            Ended,
        }
        while playing && out.as_ref().is_some_and(|o| o.prod.slots() >= CHUNK * 2) {
            if cur.is_none() {
                playing = false;
                break;
            }
            mix.iter_mut().for_each(|s| *s = 0.0);
            let mut act = Act::None;
            {
                let c = cur.as_mut().unwrap();
                c.read(CHUNK, &mut a);
                if let Some((elapsed, len)) = fading.as_mut() {
                    // equal-power crossfade between the outgoing (cur) and incoming (next) decks
                    if let Some(n) = next.as_mut() {
                        n.read(CHUNK, &mut b);
                        let sr_f = sr as f64;
                        for i in 0..CHUNK {
                            let t = ((*elapsed + i as f64 / sr_f) / *len).clamp(0.0, 1.0) as f32;
                            let (go, gi) = ((t * std::f32::consts::FRAC_PI_2).cos(), (t * std::f32::consts::FRAC_PI_2).sin());
                            let (ia, ib) = (i * 2, i * 2 + 1);
                            mix[ia] = a.get(ia).copied().unwrap_or(0.0) * go + b.get(ia).copied().unwrap_or(0.0) * gi;
                            mix[ib] = a.get(ib).copied().unwrap_or(0.0) * go + b.get(ib).copied().unwrap_or(0.0) * gi;
                        }
                        *elapsed += CHUNK as f64 / sr_f;
                        if *elapsed >= *len || c.finished() {
                            act = Act::FadeDone;
                        }
                    } else {
                        act = Act::FadeAbort;
                    }
                } else {
                    mix[..a.len()].copy_from_slice(&a);
                    if xfade > 0.0 && next.is_some() && c.remaining() <= xfade as f64 && c.remaining() > 0.3 {
                        fading = Some((0.0, c.remaining()));
                        if let Some(n) = next.as_ref() {
                            let _ = events.send(Event::Advanced(n.id.clone()));
                            let _ = events.send(Event::Current);
                        }
                    } else if c.finished() {
                        act = if next.is_some() { Act::Gapless(a.len()) } else { Act::Ended };
                    }
                }
            }
            match act {
                Act::None => {}
                Act::FadeDone => {
                    cur = next.take();
                    fading = None;
                }
                Act::FadeAbort => fading = None,
                Act::Gapless(have) => {
                    // fill the rest of this chunk from the next song: no gap at all
                    let mut n = next.take().unwrap();
                    n.read(CHUNK - have / 2, &mut b);
                    mix[have..have + b.len()].copy_from_slice(&b);
                    let id = n.id.clone();
                    cur = Some(n);
                    let _ = events.send(Event::Advanced(id.clone()));
                    let _ = events.send(Event::Current);
                }
                Act::Ended => {
                    cur = None;
                    playing = false;
                    let _ = events.send(Event::Ended);
                }
            }
            dsp.process(&mut mix);
            {
                let mut sc = shared.scope.lock();
                for fr in mix.chunks(2) {
                    if sc.len() >= SCOPE_LEN {
                        sc.pop_front();
                    }
                    sc.push_back([fr[0], fr[1]]);
                }
            }
            if let Some(Ok(mut ch)) = out.as_mut().map(|o| o.prod.write_chunk_uninit(CHUNK * 2)) {
                let (s1, s2) = ch.as_mut_slices();
                let n1 = s1.len();
                for (i, s) in s1.iter_mut().enumerate() {
                    s.write(mix[i]);
                }
                for (i, s) in s2.iter_mut().enumerate() {
                    s.write(mix[n1 + i]);
                }
                unsafe { ch.commit_all() };
            }
        }

        // publish status
        let mut st = shared.status.lock();
        st.playing = playing;
        match cur.as_ref() {
            Some(c) => {
                let queued = out.as_ref().map(|o| (o.ring_len - o.prod.slots()) as f64 / 2.0 / sr as f64).unwrap_or(0.0);
                st.position = (c.position() - if playing { queued } else { 0.0 }).max(0.0);
                st.duration = c.duration;
                st.current = Some(c.id.clone());
            }
            None => {
                st.current = None;
                st.position = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// DKFM_DECODE_DIR=<folder> cargo test --release decode_folder -- --ignored --nocapture
    /// decodes every song in a folder to the end, like the player does, and reports any that fail
    #[test]
    #[ignore]
    fn decode_folder() {
        let dir = std::env::var("DKFM_DECODE_DIR").expect("DKFM_DECODE_DIR");
        let mut files = Vec::new();
        let mut stack = vec![std::path::PathBuf::from(dir)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if matches!(p.extension().and_then(|x| x.to_str()), Some("m4a" | "mp3" | "flac" | "opus" | "ogg" | "wav" | "webm")) {
                    files.push(p);
                }
            }
        }
        let (mut ok, mut bad) = (0, 0);
        for p in files {
            let r = std::panic::catch_unwind(|| {
                let mut d = super::Deck::open("t", &p, 48000, 0.0)?;
                let mut buf = Vec::new();
                let mut frames = 0u64;
                while !d.finished() {
                    d.read(512, &mut buf);
                    if buf.iter().any(|s| !s.is_finite()) {
                        return Err("non-finite sample".to_string());
                    }
                    frames += buf.len() as u64 / 2;
                }
                let secs = frames as f64 / 48000.0;
                if d.duration > 0.0 && secs < d.duration - 2.0 {
                    return Err(format!("stopped at {secs:.0}s of {:.0}s", d.duration));
                }
                Ok(())
            });
            match r {
                Ok(Ok(())) => ok += 1,
                Ok(Err(e)) => {
                    bad += 1;
                    println!("FAIL {}: {e}", p.display());
                }
                Err(_) => {
                    bad += 1;
                    println!("PANIC {}", p.display());
                }
            }
        }
        println!("{ok} ok, {bad} bad");
    }
}

//! Sing-along score (THEATER, with sing-along on): the microphone listens while you sing and each
//! lyric line gets a grade. Two things count, half each:
//! - timing: you sing while the line is being sung (not in the breaks);
//! - pitch: the note you sing is one sounding in the song right then (any octave, so a low voice
//!   singing along with a high one isn't marked down; harmonies that fit the chord count too).
//!
//! Twenty times a second a background thread takes the last ~40 ms of the microphone (your note:
//! normalised autocorrelation, 80-1000 Hz) and of the song (how strong each of the 12 notes is:
//! Goertzel filters over three octaves), and stores (where in the song, singing?, how well the
//! note fits). Lines are graded from those when they end. Headphones give a fair score: with
//! speakers the microphone hears the song too.
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Analysis sample rate.
const SR: f32 = 16_000.0;

/// One look at you and the song: (seconds into the song, your voice is on?, how well your note
/// fits 0..1 — None when there's no clear note, like rapping or talking).
pub type Tick = (f64, bool, Option<f32>);

/// The microphone, listened to while sing-along scoring is on (dropped = stops).
pub struct Listener {
    stop: Arc<AtomicBool>,
    pub ticks: Arc<Mutex<Vec<Tick>>>,
    /// why it stopped (no microphone, blocked…)
    pub error: Arc<Mutex<Option<String>>>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Starts listening; `song` is the player's shared state (its last output frames and position).
pub fn listen(song: Arc<crate::audio::Shared>) -> Listener {
    let stop = Arc::new(AtomicBool::new(false));
    let ticks = Arc::new(Mutex::new(Vec::new()));
    let error = Arc::new(Mutex::new(None));
    let (s, t, e) = (stop.clone(), ticks.clone(), error.clone());
    std::thread::Builder::new()
        .name("karaoke".into())
        .spawn(move || {
            if let Err(err) = run(&s, &t, &song) {
                *e.lock() = Some(err);
            }
        })
        .ok();
    Listener { stop, ticks, error }
}

fn run(stop: &AtomicBool, ticks: &Mutex<Vec<Tick>>, song: &crate::audio::Shared) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let dev = cpal::default_host().default_input_device().ok_or("No microphone found: plug one in (a headset is best)")?;
    let cfg = dev.default_input_config().map_err(|e| format!("Couldn't use the microphone: {e}"))?;
    let (rate, ch) = (cfg.sample_rate().0 as f32, cfg.channels().max(1) as usize);
    let mic: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::with_capacity(8192)));
    let push = {
        let mic = mic.clone();
        move |it: &mut dyn Iterator<Item = f32>| {
            let mut m = mic.lock();
            let mut acc = 0.0;
            for (i, s) in it.enumerate() {
                acc += s;
                if i % ch == ch - 1 {
                    if m.len() >= 8192 {
                        m.pop_front();
                    }
                    m.push_back(acc / ch as f32);
                    acc = 0.0;
                }
            }
        }
    };
    let sc: cpal::StreamConfig = cfg.clone().into();
    let err = |e| eprintln!("karaoke mic: {e}");
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => dev.build_input_stream(&sc, move |d: &[f32], _| push(&mut d.iter().copied()), err, None),
        cpal::SampleFormat::I16 => dev.build_input_stream(&sc, move |d: &[i16], _| push(&mut d.iter().map(|s| *s as f32 / 32768.0)), err, None),
        cpal::SampleFormat::U16 => dev.build_input_stream(&sc, move |d: &[u16], _| push(&mut d.iter().map(|s| (*s as f32 - 32768.0) / 32768.0)), err, None),
        cpal::SampleFormat::I32 => dev.build_input_stream(&sc, move |d: &[i32], _| push(&mut d.iter().map(|s| *s as f32 / 2_147_483_648.0)), err, None),
        f => return Err(format!("Unsupported microphone format {f:?}")),
    }
    .map_err(|e| {
        let e = e.to_string();
        if e.to_lowercase().contains("denied") {
            "The microphone is blocked: allow it for desktop apps in Windows Settings > Privacy & security > Microphone".to_string()
        } else {
            format!("Couldn't use the microphone: {e}")
        }
    })?;
    stream.play().map_err(|e| e.to_string())?;
    // the quietest the microphone gets (the room): singing is well above it
    let mut floor = 0.002f32;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
        let (playing, pos) = {
            let st = song.status.lock();
            (st.playing, st.position)
        };
        if !playing {
            continue;
        }
        let you: Vec<f32> = { let m = mic.lock(); m.iter().rev().take((rate * 0.064) as usize).rev().copied().collect() };
        let you = down(&you, rate);
        let srate = song.sample_rate.load(Ordering::Relaxed).max(8000) as f32;
        let mix: Vec<f32> = song.scope.lock().iter().rev().take((srate * 0.064) as usize).rev().map(|f| (f[0] + f[1]) * 0.5).collect();
        let mix = down(&mix, srate);
        let level = rms(&you);
        floor = if level < floor { level.max(1e-4) } else { floor * 1.002 };
        let voice = level > (floor * 4.0).max(0.006);
        let fit = if voice { pitch(&you).map(|f| fits(f, &chroma(&mix))) } else { None };
        let mut t = ticks.lock();
        t.push((pos, voice, fit));
        if t.len() > 20 * 60 * 12 {
            t.drain(..20 * 60);
        }
    }
    drop(stream);
    Ok(())
}

/// `x` at `rate` down to about 16 kHz (averaging neighbours, which also filters).
fn down(x: &[f32], rate: f32) -> Vec<f32> {
    let k = (rate / SR).round().max(1.0) as usize;
    x.chunks(k).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect()
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

/// The note in `x` (16 kHz), 80-1000 Hz, if it's clearly one note (a voice, not noise).
pub fn pitch(x: &[f32]) -> Option<f32> {
    let n = x.len();
    let (lo, hi) = ((SR / 1000.0) as usize, (SR / 80.0) as usize);
    if n < hi * 2 {
        return None;
    }
    let mean = x.iter().sum::<f32>() / n as f32;
    let x: Vec<f32> = x.iter().map(|s| s - mean).collect();
    // normalised square difference (McLeod): 1 at a perfect repeat
    let nsdf = |lag: usize| {
        let (mut r, mut m) = (0.0f32, 0.0f32);
        for i in 0..n - lag {
            r += x[i] * x[i + lag];
            m += x[i] * x[i] + x[i + lag] * x[i + lag];
        }
        if m > 0.0 { 2.0 * r / m } else { 0.0 }
    };
    let v: Vec<f32> = (lo..=hi).map(nsdf).collect();
    let best = v.iter().cloned().fold(0.0f32, f32::max);
    if best < 0.6 {
        return None;
    }
    // the first peak close to the best (not a multiple of the period)
    let i = (1..v.len() - 1).find(|&i| v[i] >= best * 0.9 && v[i] >= v[i - 1] && v[i] >= v[i + 1])?;
    // a little more exact between samples
    let (a, b, c) = (v[i - 1], v[i], v[i + 1]);
    let shift = if a - 2.0 * b + c != 0.0 { 0.5 * (a - c) / (a - 2.0 * b + c) } else { 0.0 };
    Some(SR / ((lo + i) as f32 + shift))
}

/// How strong each of the 12 notes (C = 0) is in `x` (16 kHz), over three octaves, largest = 1.
pub fn chroma(x: &[f32]) -> [f32; 12] {
    let mut c = [0f32; 12];
    if x.len() < 256 {
        return c;
    }
    // A2 (110 Hz) up three octaves
    for k in 0..36 {
        let f = 110.0 * 2f32.powf(k as f32 / 12.0);
        let w = 2.0 * (std::f32::consts::PI * f / SR).cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &s in x {
            let s0 = s + w * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let p = s1 * s1 + s2 * s2 - w * s1 * s2;
        c[(k + 9) % 12] += p.max(0.0); // (A is 9 semitones above C)
    }
    let max = c.iter().cloned().fold(0.0f32, f32::max);
    if max > 0.0 {
        for v in c.iter_mut() {
            *v /= max;
        }
    }
    c
}

/// How well a sung note `f` fits the song's notes now: 1 = the strongest note, a little off key
/// still counts for something.
pub fn fits(f: f32, chroma: &[f32; 12]) -> f32 {
    let semis = 12.0 * (f / 440.0).log2() + 9.0; // from C
    let n = semis.round();
    let off = (semis - n).abs(); // 0..0.5 of a semitone
    let pc = (n as i32).rem_euclid(12) as usize;
    (chroma[pc] * (1.0 - off)).clamp(0.0, 1.0)
}

/// A line's score (0-100) from what was heard while it was sung.
pub fn line_score(ticks: &[Tick]) -> u32 {
    if ticks.is_empty() {
        return 0;
    }
    let sung: Vec<&Tick> = ticks.iter().filter(|t| t.1).collect();
    let timing = (sung.len() as f32 / ticks.len() as f32 / 0.6).min(1.0);
    if timing < 0.15 {
        return 0;
    }
    let notes: Vec<f32> = sung.iter().filter_map(|t| t.2).collect();
    // a rapped (or spoken) line has few clear notes: it's judged on timing
    let tune = if notes.len() * 3 >= sung.len() {
        // (singing in tune most of the time is enough for full marks)
        (notes.iter().sum::<f32>() / notes.len() as f32 / 0.7).min(1.0)
    } else {
        timing.min(0.85)
    };
    ((timing * 0.5 + tune * 0.5) * 100.0).round() as u32
}

pub fn grade(score: u32) -> &'static str {
    match score {
        85.. => "PERFECT!",
        65..=84 => "GREAT!",
        40..=64 => "GOOD",
        _ => "MISS",
    }
}

/// Your best score per song (data folder, karaoke.json).
pub fn best(id: &str) -> Option<u32> {
    let v: std::collections::HashMap<String, u32> = crate::store::load_json(&crate::store::data_dir().join("karaoke.json"));
    v.get(id).copied()
}

/// Saves `score` for the song if it beats your best; returns the best before it.
pub fn save_best(id: &str, score: u32) -> Option<u32> {
    let path = crate::store::data_dir().join("karaoke.json");
    let mut v: std::collections::HashMap<String, u32> = crate::store::load_json(&path);
    let before = v.get(id).copied();
    if before.map(|b| score > b).unwrap_or(true) {
        v.insert(id.to_string(), score);
        crate::store::save_json(&path, &v);
    }
    before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freqs: &[f32], n: usize) -> Vec<f32> {
        (0..n).map(|i| freqs.iter().map(|f| (2.0 * std::f32::consts::PI * f * i as f32 / SR).sin()).sum::<f32>() * 0.3).collect()
    }

    #[test]
    fn hears_the_note_you_sing() {
        for f in [110.0, 196.0, 261.6, 440.0, 659.3] {
            // a voice has overtones: the note is still its lowest
            let x = tone(&[f, f * 2.0, f * 3.0], 1024);
            let p = pitch(&x).unwrap();
            assert!((p / f - 1.0).abs() < 0.02, "{f}: {p}");
        }
        // noise isn't a note
        let mut seed = 1u32;
        let noise: Vec<f32> = (0..1024).map(|_| { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); (seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5 }).collect();
        assert!(pitch(&noise).is_none());
    }

    #[test]
    fn notes_that_fit_the_song_score() {
        // the song plays a C major chord (C4 E4 G4)
        let c = chroma(&tone(&[261.6, 329.6, 392.0], 1365));
        // singing C (any octave) or E fits; F# doesn't
        assert!(fits(130.8, &c) > 0.6 && fits(523.3, &c) > 0.6 && fits(329.6, &c) > 0.6, "{c:?}");
        assert!(fits(370.0, &c) < 0.3, "{c:?}");
        // slightly flat still counts, less
        assert!(fits(261.6 * 2f32.powf(-0.3 / 12.0), &c) < fits(261.6, &c));
    }

    #[test]
    fn lines_are_graded() {
        let sung = |fit: f32| (0.0, true, Some(fit));
        assert_eq!(line_score(&[]), 0);
        assert_eq!(line_score(&[(0.0, false, None); 20]), 0);
        assert!(line_score(&[sung(0.9); 20]) >= 95);
        // rapping on time (a voice, hardly any clear notes) scores well too
        assert!(line_score(&[(0.0, true, None); 20]) >= 85);
        let half: Vec<Tick> = (0..20).map(|i| if i % 2 == 0 { sung(0.8) } else { (0.0, false, None) }).collect();
        assert!((80..=100).contains(&line_score(&half)), "{}", line_score(&half));
        assert!(line_score(&[sung(0.1); 20]) < 65);
        assert_eq!(grade(90), "PERFECT!");
        assert_eq!(grade(10), "MISS");
    }
}

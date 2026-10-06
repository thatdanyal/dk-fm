//! SHAZAM: listens for a few seconds, either to what the PC is playing (WASAPI loopback on
//! Windows; a "monitor" input, else the default input, elsewhere) or to the microphone (one at a
//! time: two songs mixed into one recording match neither), makes a Shazam-style audio signature
//! (spectral peaks in four frequency bands, 16 kHz mono) and looks it up.
use crate::net;
use crate::ui::scope::Fft;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

pub const RATE: u32 = 16_000;

/// A song it heard.
#[derive(Clone, Debug, Default)]
pub struct Heard {
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// What SHAZAM listens to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ear {
    /// what this PC is playing
    #[default]
    Pc,
    /// the room, through the default microphone
    Mic,
}

/// Records `secs` from `ear`, as 16 kHz mono.
pub fn capture(secs: f32, ear: Ear) -> Result<Vec<f32>, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let (dev, cfg) = if ear == Ear::Mic {
        let d = host.default_input_device().ok_or("No microphone found: plug one in or turn it on in Windows sound settings")?;
        let c = d.default_input_config().map_err(|e| format!("Couldn't use the microphone: {e}"))?;
        (d, c)
    } else {
        pc_device(&host)?
    };
    let (rate, ch) = (cfg.sample_rate().0, cfg.channels().max(1) as usize);
    let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let max = (secs * rate as f32) as usize;
    let push = {
        let buf = buf.clone();
        move |frames: &mut dyn Iterator<Item = f32>| {
            let mut b = buf.lock();
            let mut acc = 0.0;
            for (i, s) in frames.enumerate() {
                acc += s;
                if i % ch == ch - 1 {
                    if b.len() < max {
                        b.push(acc / ch as f32);
                    }
                    acc = 0.0;
                }
            }
        }
    };
    let err = |e| eprintln!("listen: {e}");
    let sc: cpal::StreamConfig = cfg.clone().into();
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => {
            let p = push;
            dev.build_input_stream(&sc, move |d: &[f32], _| p(&mut d.iter().copied()), err, None)
        }
        cpal::SampleFormat::I16 => {
            let p = push;
            dev.build_input_stream(&sc, move |d: &[i16], _| p(&mut d.iter().map(|s| *s as f32 / 32768.0)), err, None)
        }
        cpal::SampleFormat::U16 => {
            let p = push;
            dev.build_input_stream(&sc, move |d: &[u16], _| p(&mut d.iter().map(|s| (*s as f32 - 32768.0) / 32768.0)), err, None)
        }
        cpal::SampleFormat::I32 => {
            let p = push;
            dev.build_input_stream(&sc, move |d: &[i32], _| p(&mut d.iter().map(|s| *s as f32 / 2_147_483_648.0)), err, None)
        }
        f => return Err(format!("Unsupported sound format {f:?}")),
    }
    .map_err(|e| {
        let e = e.to_string();
        if ear == Ear::Mic && e.to_lowercase().contains("denied") {
            "The microphone is blocked: allow it for desktop apps in Windows Settings > Privacy & security > Microphone".to_string()
        } else {
            format!("Couldn't listen: {e}")
        }
    })?;
    stream.play().map_err(|e| e.to_string())?;
    let end = std::time::Instant::now() + Duration::from_secs_f32(secs + 2.0);
    while buf.lock().len() < max && std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(stream);
    let mono = std::mem::take(&mut *buf.lock());
    Ok(resample(&mono, rate, RATE))
}

/// The device whose sound is "what this PC is playing".
fn pc_device(host: &cpal::Host) -> Result<(cpal::Device, cpal::SupportedStreamConfig), String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    // Windows: an input stream on the speakers is a loopback of everything they play
    #[cfg(windows)]
    {
        let d = host.default_output_device().ok_or("No speakers found")?;
        let c = d.default_output_config().map_err(|e| e.to_string())?;
        Ok((d, c))
    }
    #[cfg(not(windows))]
    {
        let monitor = host.input_devices().ok().and_then(|mut l| l.find(|d| d.name().map(|n| n.to_lowercase().contains("monitor")).unwrap_or(false)));
        let d = monitor.or_else(|| host.default_input_device()).ok_or("Nothing to listen with: no input device")?;
        let c = d.default_input_config().map_err(|e| e.to_string())?;
        Ok((d, c))
    }
}

/// Brings a quiet recording (a microphone across the room, the PC turned down) up to a level
/// where its peaks clear the signature's floor (silence was already turned away).
fn level(s: &mut [f32]) {
    let peak = s.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
    if peak > 0.0 && peak < 0.5 {
        let g = 0.8 / peak;
        s.iter_mut().for_each(|x| *x *= g);
    }
}

/// Box-filtered linear resampling (plenty for peak-picking up to 5.5 kHz).
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let w = (ratio.round() as usize).max(1);
    // low-pass: running mean over `w` samples
    let mut lp = Vec::with_capacity(x.len());
    let mut sum = 0.0;
    for i in 0..x.len() {
        sum += x[i];
        if i >= w {
            sum -= x[i - w];
        }
        lp.push(sum / w.min(i + 1) as f32);
    }
    let n = (x.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let p = i as f64 * ratio;
            let j = p as usize;
            let f = (p - j as f64) as f32;
            let a = lp[j.min(lp.len() - 1)];
            let b = lp[(j + 1).min(lp.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

// ------------------------------------------------------------------------------- signature

#[derive(Clone, Copy, Debug, PartialEq)]
struct Peak {
    pass: u32,
    magnitude: u16,
    bin: u16,
}

const SPREAD_BACK: [usize; 3] = [1, 3, 6];
const NEIGHBOURS: [i32; 8] = [-10, -7, -4, -3, 1, 2, 5, 8];
const OTHER_FFTS: [i32; 14] = [-53, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249];

/// Peaks of 16 kHz mono audio by frequency band (0: 250-520 Hz, 1: -1450, 2: -3500, 3: -5500).
fn peaks(samples: &[f32]) -> BTreeMap<u32, Vec<Peak>> {
    let fft = Fft::new(2048);
    let window: Vec<f32> = (0..2048).map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * (i + 1) as f32 / 2049.0).cos())).collect();
    let mut ring = vec![0.0f32; 2048];
    let mut ring_at = 0;
    let mut ffts = vec![vec![0.0f32; 1025]; 256];
    let mut spread = vec![vec![0.0f32; 1025]; 256];
    let mut at = 0usize; // next write in ffts / spread
    let mut done = 0u32;
    let mut out: BTreeMap<u32, Vec<Peak>> = BTreeMap::new();
    let idx = |at: usize, off: i32| (at as i32 + off).rem_euclid(256) as usize;
    let (mut re, mut im) = (vec![0.0f32; 2048], vec![0.0f32; 2048]);
    for chunk in samples.chunks_exact(128) {
        // FFT of the last 2048 samples
        for s in chunk {
            ring[ring_at] = s * 32767.0;
            ring_at = (ring_at + 1) % 2048;
        }
        for i in 0..2048 {
            re[i] = ring[(ring_at + i) % 2048] * window[i];
            im[i] = 0.0;
        }
        fft.process(&mut re, &mut im);
        let power: Vec<f32> = (0..1025).map(|i| ((re[i] * re[i] + im[i] * im[i]) / (1u32 << 17) as f32).max(1e-10)).collect();
        // spread each peak over its neighbours in frequency, then back in time
        let mut sp = power.clone();
        for i in 0..1023 {
            sp[i] = sp[i].max(sp[i + 1]).max(sp[i + 2]);
        }
        for back in SPREAD_BACK {
            let f = &mut spread[idx(at, -(back as i32))];
            for i in 0..1025 {
                f[i] = f[i].max(sp[i]);
            }
        }
        ffts[at] = power;
        spread[at] = sp;
        at = (at + 1) % 256;
        done += 1;
        if done < 46 {
            continue;
        }
        // peaks of the FFT 46 passes ago that stand out from everything around them
        let f46 = &ffts[idx(at, -46)];
        let f49 = &spread[idx(at, -49)];
        for bin in 10..1015usize {
            let v = f46[bin];
            if v < 1.0 / 64.0 || v < f49[bin - 1] {
                continue;
            }
            let mut around = NEIGHBOURS.iter().map(|o| f49[(bin as i32 + o) as usize]).fold(0.0f32, f32::max);
            if v <= around {
                continue;
            }
            for o in OTHER_FFTS {
                around = around.max(spread[idx(at, o)][bin - 1]);
            }
            if v <= around {
                continue;
            }
            let mag = |x: f32| x.max(1.0 / 64.0).ln() * 1477.3 + 6144.0;
            let (m, before, after) = (mag(v), mag(f46[bin - 1]), mag(f46[bin + 1]));
            let var1 = m * 2.0 - before - after;
            let var2 = (after - before) * 32.0 / var1;
            let fbin = bin as f32 * 64.0 + var2;
            let hz = fbin * (RATE as f32 / 2.0 / 1024.0 / 64.0);
            let band = match hz {
                h if (250.0..520.0).contains(&h) => 0,
                h if (520.0..1450.0).contains(&h) => 1,
                h if (1450.0..3500.0).contains(&h) => 2,
                h if (3500.0..=5500.0).contains(&h) => 3,
                _ => continue,
            };
            out.entry(band).or_default().push(Peak { pass: done - 46, magnitude: m as u16, bin: fbin as u16 });
        }
    }
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for b in data {
        c ^= *b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

/// The binary signature format the lookup takes.
fn encode(bands: &BTreeMap<u32, Vec<Peak>>, n_samples: usize) -> Vec<u8> {
    let mut b: Vec<u8> = Vec::new();
    let u32le = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
    u32le(&mut b, 0xcafe_2580); // magic
    u32le(&mut b, 0); // crc, below
    u32le(&mut b, 0); // size after the header, below
    u32le(&mut b, 0x9411_9c00); // magic
    for _ in 0..3 {
        u32le(&mut b, 0);
    }
    u32le(&mut b, 3 << 27); // 16 kHz
    for _ in 0..2 {
        u32le(&mut b, 0);
    }
    u32le(&mut b, (n_samples as f32 + RATE as f32 * 0.24) as u32);
    u32le(&mut b, (15 << 19) + 0x40000);
    // (48-byte header done)
    u32le(&mut b, 0x4000_0000);
    u32le(&mut b, 0); // size again, below
    for (band, list) in bands {
        let mut p: Vec<u8> = Vec::new();
        let mut pass = 0u32;
        for k in list {
            if k.pass - pass >= 255 {
                p.push(0xff);
                p.extend_from_slice(&k.pass.to_le_bytes());
                pass = k.pass;
            }
            p.push((k.pass - pass) as u8);
            p.extend_from_slice(&k.magnitude.to_le_bytes());
            p.extend_from_slice(&k.bin.to_le_bytes());
            pass = k.pass;
        }
        u32le(&mut b, 0x6003_0040 + band);
        u32le(&mut b, p.len() as u32);
        let pad = (4 - p.len() % 4) % 4;
        b.extend_from_slice(&p);
        b.extend(std::iter::repeat_n(0u8, pad));
    }
    let size = (b.len() - 48) as u32;
    b[8..12].copy_from_slice(&size.to_le_bytes());
    b[52..56].copy_from_slice(&size.to_le_bytes());
    let crc = crc32(&b[8..]);
    b[4..8].copy_from_slice(&crc.to_le_bytes());
    b
}

/// 16 kHz mono audio -> the signature as a data URI, and its length in ms.
pub fn signature(samples: &[f32]) -> (String, u64) {
    let bands = peaks(samples);
    (format!("data:audio/vnd.shazam.sig;base64,{}", base64(&encode(&bands, samples.len()))), samples.len() as u64 * 1000 / RATE as u64)
}

fn uuid() -> String {
    let h: String = (0..32).map(|_| format!("{:x}", fastrand::u8(..16))).collect();
    format!("{}-{}-4{}-a{}-{}", &h[..8], &h[8..12], &h[13..16], &h[17..20], &h[20..])
}

/// Looks the audio up. Ok(None) = no match.
pub fn lookup(samples: &[f32]) -> Result<Option<Heard>, String> {
    let (uri, ms) = signature(samples);
    let now = crate::store::now_ms() as u64;
    let url = format!("https://amp.shazam.com/discovery/v5/en/US/android/-/tag/{}/{}?sync=true&webv3=true&sampling=true&connected=&shazamapiversion=v3&sharehub=true&video=v3", uuid().to_uppercase(), uuid());
    let body = json!({ "geolocation": { "altitude": 300, "latitude": 45, "longitude": 2 }, "signature": { "samplems": ms, "timestamp": now, "uri": uri }, "timestamp": now, "timezone": "Europe/London" });
    let v: Value = net::post_json(&url, &[("User-Agent", "Dalvik/2.1.0 (Linux; U; Android 5.0.2; VS980 4G Build/LRX22G)"), ("Content-Language", "en_US")], &body)?;
    let t = &v["track"];
    if v["matches"].as_array().map(|m| m.is_empty()).unwrap_or(true) || t["title"].as_str().is_none() {
        return Ok(None);
    }
    let album = t["sections"].as_array().and_then(|s| s.iter().flat_map(|x| x["metadata"].as_array().cloned().unwrap_or_default()).find(|m| m["title"] == "Album")).and_then(|m| m["text"].as_str().map(String::from)).unwrap_or_default();
    Ok(Some(Heard { title: t["title"].as_str().unwrap_or("").into(), artist: t["subtitle"].as_str().unwrap_or("").into(), album }))
}

/// Listen to `ear` and look up; up to two tries (the second with a longer recording) before
/// giving up.
pub fn identify(ear: Ear) -> Result<Option<Heard>, String> {
    for secs in [8.0, 12.0] {
        let mut s = capture(secs, ear)?;
        let quiet = if ear == Ear::Mic { 0.0005 } else { 0.003 };
        if s.iter().map(|x| x.abs()).fold(0.0f32, f32::max) < quiet {
            if ear == Ear::Pc {
                return Err("Heard silence: play the song on this PC, then try again.".into());
            }
            use cpal::traits::{DeviceTrait, HostTrait};
            let name = cpal::default_host().default_input_device().and_then(|d| d.name().ok()).unwrap_or_default();
            return Err(format!("The microphone ({name}) heard nothing: check it's on and not muted, or pick another one as the default input in Windows sound settings."));
        }
        level(&mut s);
        if let Some(h) = lookup(&s)? {
            return Ok(Some(h));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_of_tones() {
        // a few seconds of chords: peaks in the bands, a well-formed header and checksum
        let n = RATE as usize * 4;
        let s: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let f = if (t * 2.0) as u32 % 2 == 0 { [330.0, 880.0, 2000.0, 4000.0] } else { [440.0, 1100.0, 2500.0, 5000.0] };
                f.iter().map(|f| (2.0 * std::f32::consts::PI * f * t).sin()).sum::<f32>() * 0.2
            })
            .collect();
        let bands = peaks(&s);
        assert!((0..4).all(|b| bands.get(&b).map(|v| !v.is_empty()).unwrap_or(false)), "{:?}", bands.keys().collect::<Vec<_>>());
        let bin = encode(&bands, n);
        assert_eq!(&bin[0..4], &0xcafe_2580u32.to_le_bytes());
        assert_eq!(u32::from_le_bytes(bin[8..12].try_into().unwrap()) as usize, bin.len() - 48);
        assert_eq!(u32::from_le_bytes(bin[4..8].try_into().unwrap()), crc32(&bin[8..]));
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        // 48 kHz -> 16 kHz keeps the length in time
        assert_eq!(resample(&vec![0.0; 48_000], 48_000, RATE).len(), 16_000);
        // a quiet recording is brought up, a loud one left alone
        let mut q = vec![0.01, -0.02];
        level(&mut q);
        assert!((q[1] + 0.8).abs() < 1e-6);
        let mut l = vec![0.9, -0.6];
        level(&mut l);
        assert_eq!(l, vec![0.9, -0.6]);
    }
}

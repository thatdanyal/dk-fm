//! Visualizer: LED spectrum bars, phosphor oscilloscope, twin analog VU meters, and a colour
//! waterfall (the sound falling from the top, bass in the middle, in the song's own colours).
//! Draws into a small pixel buffer (half resolution) that is uploaded as one texture per frame.
use super::theme::px;
use super::widgets::{fill, frame_rect};
use super::App;
use crate::audio::SCOPE_LEN;
use crate::player::Player;
use eframe::egui::{Color32, ColorImage, Pos2, Rect, Sense, TextureHandle, TextureOptions, Ui, Vec2};

pub const MODES: [(&str, &str); 5] = [("bars", "BARS"), ("scope", "SCOPE"), ("vu", "VU"), ("spectro", "WATERFALL"), ("off", "OFF")];
const FFT: usize = 2048;

pub struct ScopeState {
    pub mode: String,
    /// set when the visualizer was actually on screen this frame (not hidden behind another tab)
    pub drawn: bool,
    tex: Option<TextureHandle>,
    w: usize,
    h: usize,
    buf: Vec<Color32>,
    glow: Vec<f32>,
    peaks: Vec<f32>,
    vu: [f32; 2],
    hold: [u32; 2],
    fft: Fft,
    re: Vec<f32>,
    im: Vec<f32>,
    spec: Vec<f32>,
    loaded: bool,
    /// Settings > Look: custom colours (low, mid, peak) and bar count (0 = fit)
    pub colors: Option<[Color32; 3]>,
    pub bars: u32,
    /// waterfall: rows still owed (it falls at a steady speed, however often it's drawn), when it
    /// was last drawn, and the song's two hues (0..1) with the cover they're from
    flow: f32,
    last: Option<f64>,
    hues: (f32, f32),
    hues_for: Option<String>,
}

impl ScopeState {
    pub fn new() -> Self {
        let fft = Fft::new(FFT);
        Self { mode: "bars".into(), drawn: false, tex: None, w: 0, h: 0, buf: Vec::new(), glow: Vec::new(), peaks: Vec::new(), vu: [0.0; 2], hold: [0; 2], fft, re: vec![0.0; FFT], im: vec![0.0; FFT], spec: vec![0.0; FFT / 2], loaded: false, colors: None, bars: 0, flow: 0.0, last: None, hues: (0.5, 0.75), hues_for: None }
    }
    pub fn invalidate(&mut self) {
        self.w = 0;
    }
    /// Drop the texture and pixel buffers (rebuilt when it's on screen again).
    pub fn release(&mut self) {
        self.tex = None;
        self.buf = Vec::new();
        self.glow = Vec::new();
        self.w = 0;
    }
    pub fn cycle(&mut self, player: &Player) {
        let i = MODES.iter().position(|m| m.0 == self.mode).unwrap_or(0);
        self.mode = MODES[(i + 1) % MODES.len()].0.into();
        let fps = player.st.lock().opts.vis_fps;
        player.set_visualizer(&self.mode, fps);
        self.invalidate();
    }
}

pub fn show(app: &mut App, ui: &mut Ui) {
    app.scope.drawn = true;
    if app.scope.mode == "spectro" {
        song_hues(app);
    }
    if !app.scope.loaded {
        app.scope.mode = app.player.st.lock().opts.visualizer.clone();
        app.scope.loaded = true;
    }
    let pal = app.pal;
    let outer = ui.available_rect_before_wrap().shrink(6.0);
    let resp = ui.allocate_rect(outer, Sense::click());
    fill(ui.painter(), outer, pal.lcd_bg);
    frame_rect(ui.painter(), outer, 2.0, pal.line_hi);
    let r = outer.shrink(2.0);
    if resp.clicked() {
        app.scope.cycle(&app.player);
    }
    // (half resolution; never past the 2048 px textures are capped at)
    let (w, h) = ((r.width() / 2.0).clamp(1.0, 2048.0) as usize, (r.height() / 2.0).clamp(1.0, 2048.0) as usize);
    let s = &mut app.scope;
    let col = Cols::new(&pal, s.colors);
    if s.w != w || s.h != h {
        s.w = w;
        s.h = h;
        s.buf = vec![col.bg; w * h];
        s.glow = vec![0.0; w * h];
    }
    if s.mode != "off" {
        let frames: Vec<[f32; 2]> = app.player.engine.shared.scope.lock().iter().copied().collect();
        let sr = app.player.engine.shared.sample_rate.load(std::sync::atomic::Ordering::Relaxed) as f32;
        match s.mode.as_str() {
            "scope" => scope(s, &frames, &col),
            "vu" => vu(s, &frames, &col),
            "spectro" => {
                spectrum(s, &frames);
                // a steady fall: rows by the clock, not by how often the screen is drawn
                let now = ui.input(|i| i.time);
                let playing = app.player.engine.status().playing;
                let dt = s.last.map(|l| (now - l) as f32).unwrap_or(0.0).clamp(0.0, 0.25);
                s.last = Some(now);
                if playing {
                    s.flow += dt * FALL;
                }
                let rows = s.flow as usize;
                s.flow -= rows as f32;
                waterfall(s, sr, rows);
            }
            _ => {
                spectrum(s, &frames);
                bars(s, &col, sr)
            }
        }
    } else {
        s.buf.fill(col.bg);
    }
    let img = ColorImage { size: [w, h], pixels: s.buf.clone() };
    match &mut s.tex {
        Some(t) => t.set(img, TextureOptions::NEAREST),
        None => s.tex = Some(ui.ctx().load_texture("scope", img, TextureOptions::NEAREST)),
    }
    if let Some(t) = &s.tex {
        ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }
    if s.mode == "spectro" {
        // older rows fade as they fall
        let mut m = eframe::egui::Mesh::default();
        let (clear, bg) = (Color32::TRANSPARENT, super::widgets::with_alpha(pal.lcd_bg, 235));
        for (p, c) in [(r.left_top(), clear), (r.right_top(), clear), (r.right_bottom(), bg), (r.left_bottom(), bg)] {
            m.colored_vertex(p, c);
        }
        m.add_triangle(0, 1, 2);
        m.add_triangle(0, 2, 3);
        ui.painter().add(m);
    }
    if s.mode == "off" {
        ui.painter().text(r.center(), eframe::egui::Align2::CENTER_CENTER, "VISUALIZER OFF · CLICK TO TURN ON", px(6.0), pal.faint);
    }
    // mode buttons while the mouse is over the visualizer. Not `resp.hovered()`: the buttons
    // take the hover from it, so they'd vanish under the mouse every other frame (flicker).
    if ui.rect_contains_pointer(outer) {
        let mut x = r.right() - 4.0;
        for (id, label) in MODES.iter().rev() {
            let g = ui.painter().layout_no_wrap(label.to_string(), px(5.0), pal.text);
            let br = Rect::from_min_size(Pos2::new(x - g.size().x - 8.0, r.top() + 4.0), g.size() + Vec2::new(8.0, 6.0));
            let on = app.scope.mode == *id;
            fill(ui.painter(), br, if on { pal.accent } else { pal.panel });
            ui.painter().galley(br.min + Vec2::new(4.0, 3.0), g, if on { pal.ink } else { pal.dim });
            if ui.interact(br, ui.id().with(id), Sense::click()).clicked() {
                app.scope.mode = id.to_string();
                let fps = app.player.st.lock().opts.vis_fps;
                app.player.set_visualizer(id, fps);
            }
            x = br.left() - 2.0;
        }
    }
}

struct Cols {
    bg: Color32,
    a: Color32,
    b: Color32,
    text: Color32,
    dim: Color32,
    heat: Vec<Color32>,
}

impl Cols {
    fn new(p: &super::theme::Pal, custom: Option<[Color32; 3]>) -> Self {
        let [a, b, top] = custom.unwrap_or([p.accent, p.accent2, p.text]);
        let stops = [p.lcd_bg, a, b, top];
        let heat = (0..256)
            .map(|i| {
                let t = i as f32 / 255.0 * 3.0;
                let k = (t as usize).min(2);
                let f = t - k as f32;
                let (c0, c1) = (stops[k], stops[k + 1]);
                let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f) as u8;
                Color32::from_rgb(m(c0.r(), c1.r()), m(c0.g(), c1.g()), m(c0.b(), c1.b()))
            })
            .collect();
        Self { bg: p.lcd_bg, a, b, text: top, dim: p.faint, heat }
    }
}

fn rect(s: &mut ScopeState, x: i32, y: i32, w: i32, h: i32, c: Color32) {
    for yy in y.max(0)..(y + h).min(s.h as i32) {
        for xx in x.max(0)..(x + w).min(s.w as i32) {
            s.buf[yy as usize * s.w + xx as usize] = c;
        }
    }
}

fn dot(s: &mut ScopeState, x: f32, y: f32, c: Color32) {
    let (x, y) = (x as i32, y as i32);
    if x >= 0 && y >= 0 && (x as usize) < s.w && (y as usize) < s.h {
        s.buf[y as usize * s.w + x as usize] = c;
    }
}

fn spectrum(s: &mut ScopeState, frames: &[[f32; 2]]) {
    s.re.fill(0.0);
    s.im.fill(0.0);
    let start = frames.len().saturating_sub(FFT);
    for (i, f) in frames[start..].iter().enumerate() {
        s.re[i] = (f[0] + f[1]) * 0.5 * s.fft.window[i];
    }
    s.fft.process(&mut s.re, &mut s.im);
    for i in 0..FFT / 2 {
        // dB scaled to 0..1 (-90..-10 dB), then smoothed like an analyser
        let mag = (s.re[i] * s.re[i] + s.im[i] * s.im[i]).sqrt();
        let db = 20.0 * (mag / (FFT as f32 / 4.0)).max(1e-9).log10();
        let v = ((db + 90.0) / 80.0).clamp(0.0, 1.0);
        s.spec[i] = (s.spec[i] * 0.72).max(v);
    }
}

fn band(s: &ScopeState, i: usize, n: usize, sr: f32) -> f32 {
    let nyq = sr / 2.0;
    let f0 = 30.0 * (16000.0f32 / 30.0).powf(i as f32 / n as f32);
    let f1 = 30.0 * (16000.0f32 / 30.0).powf((i + 1) as f32 / n as f32);
    let len = s.spec.len();
    let a = ((f0 / nyq) * len as f32) as usize;
    let b = (((f1 / nyq) * len as f32) as usize).max(a + 1).min(len);
    s.spec[a.min(len - 1)..b].iter().cloned().fold(0.0, f32::max)
}

fn bars(s: &mut ScopeState, c: &Cols, sr: f32) {
    s.buf.fill(c.bg);
    let (w, h) = (s.w as i32, s.h as i32);
    let (bw, n) = if s.bars > 0 {
        let n = (s.bars as i32).min((w - 2) / 2).max(4);
        (((w - 2) / n - 1).max(1), n as usize)
    } else {
        let bw = if w > 300 { 5 } else { 3 };
        (bw, ((w - 2) / (bw + 1)).max(8) as usize)
    };
    let segs = ((h - 4) / 3).max(1);
    if s.peaks.len() != n {
        s.peaks = vec![0.0; n];
    }
    let x0 = (w - n as i32 * (bw + 1)) / 2;
    for i in 0..n {
        let lit = (band(s, i, n, sr).powf(1.6) * segs as f32).round() as i32;
        let x = x0 + i as i32 * (bw + 1);
        for seg in 0..segs {
            let y = h - 2 - (seg + 1) * 3;
            if seg < lit {
                let f = seg as f32 / segs as f32;
                rect(s, x, y, bw, 2, if f > 0.8 { c.text } else if f > 0.55 { c.b } else { c.a });
            } else if seg % 2 == 0 {
                rect(s, x, y, bw, 2, c.dim);
            }
        }
        s.peaks[i] = (s.peaks[i] - 0.25).max(lit as f32);
        if s.peaks[i] > 0.5 {
            let py = h - 2 - s.peaks[i].ceil() as i32 * 3 - 3;
            rect(s, x, py, bw, 1, c.text);
        }
    }
}

fn scope(s: &mut ScopeState, frames: &[[f32; 2]], c: &Cols) {
    let (w, h) = (s.w, s.h);
    for g in s.glow.iter_mut() {
        *g *= 0.55;
    }
    let mono: Vec<f32> = frames.iter().map(|f| (f[0] + f[1]) * 0.5).collect();
    let mut start = 0;
    for i in 1..mono.len() / 2 {
        if mono[i - 1] < 0.0 && mono[i] >= 0.0 {
            start = i;
            break;
        }
    }
    let span = (mono.len() - start).clamp(1, 1024);
    let mut prev: Option<i32> = None;
    for x in 0..w {
        let v = mono.get(start + x * span / w).copied().unwrap_or(0.0);
        let y = ((0.5 - v * 0.5) * (h as f32 - 2.0)).round() as i32 + 1;
        let p = prev.unwrap_or(y);
        for yy in p.min(y)..=p.max(y) {
            if yy >= 0 && (yy as usize) < h {
                s.glow[yy as usize * w + x] = 1.0;
            }
        }
        prev = Some(y);
    }
    for i in 0..w * h {
        s.buf[i] = if s.glow[i] > 0.04 { c.heat[(s.glow[i] * 190.0) as usize] } else { c.bg };
    }
    for x in (0..w).step_by(16) {
        for y in (0..h).step_by(4) {
            if s.glow[y * w + x] < 0.1 {
                s.buf[y * w + x] = c.dim;
            }
        }
    }
    for y in (0..h).step_by(16) {
        for x in (0..w).step_by(4) {
            if s.glow[y * w + x] < 0.1 {
                s.buf[y * w + x] = c.dim;
            }
        }
    }
}

fn vu(s: &mut ScopeState, frames: &[[f32; 2]], c: &Cols) {
    let tail = &frames[frames.len().saturating_sub(1024)..];
    let rms = |ch: usize| (tail.iter().map(|f| f[ch] * f[ch]).sum::<f32>() / tail.len().max(1) as f32).sqrt();
    let needle = |r: f32| ((20.0 * r.max(1e-5).log10() + 30.0) / 28.0).clamp(0.0, 1.0);
    for ch in 0..2 {
        s.vu[ch] += (needle(rms(ch)) - s.vu[ch]) * 0.18;
    }
    s.buf.fill(c.bg);
    let (w, h) = (s.w as f32, s.h as f32);
    let two = w > h * 1.6;
    let (mw, mh) = if two { (w / 2.0, h) } else { (w, h / 2.0) };
    for ch in 0..2 {
        let (x, y) = if two { (ch as f32 * mw, 0.0) } else { (0.0, ch as f32 * mh) };
        let cx = x + mw / 2.0;
        let cy = y + mh * 0.92;
        let r = (mw * 0.42).min(mh * 0.78);
        let (a0, a1) = (std::f32::consts::PI * 1.22, std::f32::consts::PI * 1.78);
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            let a = a0 + (a1 - a0) * t;
            let len = if i % 5 == 0 { 5 } else { 2 };
            for k in 0..len {
                dot(s, cx + a.cos() * (r - k as f32), cy + a.sin() * (r - k as f32), if t > 0.78 { c.a } else { c.text });
            }
        }
        let mut t = 0.78;
        while t <= 1.0 {
            let a = a0 + (a1 - a0) * t;
            dot(s, cx + a.cos() * (r + 2.0), cy + a.sin() * (r + 2.0), c.a);
            dot(s, cx + a.cos() * (r + 3.0), cy + a.sin() * (r + 3.0), c.a);
            t += 0.005;
        }
        let a = a0 + (a1 - a0) * s.vu[ch];
        let mut k = 0.0;
        while k < r - 2.0 {
            dot(s, cx + a.cos() * k, cy + a.sin() * k, c.b);
            k += 1.0;
        }
        rect(s, cx as i32 - 2, cy as i32 - 2, 5, 5, c.text);
        s.hold[ch] = if s.vu[ch] > 0.85 { 20 } else { s.hold[ch].saturating_sub(1) };
        let led = if s.hold[ch] > 0 { c.a } else { c.dim };
        rect(s, (x + mw - 10.0) as i32, (y + 5.0) as i32, 4, 4, led);
        let glyph: [&str; 5] = if ch == 0 { ["100", "100", "100", "100", "111"] } else { ["110", "101", "110", "101", "101"] };
        for (j, row) in glyph.iter().enumerate() {
            for (i, p) in row.chars().enumerate() {
                if p == '1' {
                    dot(s, x + 5.0 + i as f32, y + 5.0 + j as f32, c.dim);
                }
            }
        }
    }
}

/// Waterfall rows a second (half-resolution pixels: twice that on screen).
const FALL: f32 = 60.0;

/// The song's two hues (0..1): its cover's main colours, or (no colourful cover) picked from the
/// song itself, so every song looks a little different.
fn song_hues(app: &mut App) {
    let t = app.current_track();
    let key = t.as_ref().map(|t| format!("{}|{}", t.id, t.thumb.clone().or(t.cover.clone()).unwrap_or_default()));
    if key == app.scope.hues_for {
        return;
    }
    app.scope.hues_for = key;
    let Some(t) = t else { return };
    if let Some([a, b, _]) = app.scope.colors {
        app.scope.hues = (hue(a), hue(b));
        return;
    }
    let from_cover = t.thumb.as_ref().or(t.cover.as_ref()).and_then(|c| image::open(app.lib.cover_path(c)).ok()).and_then(|img| super::covertheme::cover_colors(&img.resize_exact(48, 48, image::imageops::FilterType::Triangle).to_rgb8()));
    app.scope.hues = match from_cover {
        Some((a, Some(b))) => (hue(a), hue(b)),
        Some((a, None)) => (hue(a), (hue(a) + 0.18) % 1.0),
        None => {
            let h = t.id.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
            let a = (h % 360) as f32 / 360.0;
            (a, (a + 0.25) % 1.0)
        }
    };
}

fn hue(c: Color32) -> f32 {
    eframe::egui::ecolor::Hsva::from(c).h
}

/// The waterfall: `rows` new lines at the top (the sound now), the rest falls down. Mirrored,
/// bass in the middle and treble at the edges; colour goes from the song's first hue (bass) to
/// its second (treble) and shifts with how bright the sound is, loud parts glow toward white.
fn waterfall(s: &mut ScopeState, sr: f32, rows: usize) {
    let (w, h) = (s.w, s.h);
    if rows == 0 || w < 2 || h < 2 {
        return;
    }
    let rows = rows.min(h);
    s.buf.copy_within(0..(h - rows) * w, rows * w);
    let half = w.div_ceil(2).max(1);
    let vals: Vec<f32> = (0..half).map(|i| band(s, i, half, sr)).collect();
    // how much is high up (0 = all bass, 1 = all treble): the colour leans that way
    let total: f32 = vals.iter().sum::<f32>().max(1e-6);
    let bright = vals.iter().enumerate().map(|(i, v)| v * i as f32).sum::<f32>() / total / half as f32;
    let (h1, h2) = s.hues;
    // the shorter way round the colour wheel from one hue to the other
    let mut d = h2 - h1;
    if d > 0.5 {
        d -= 1.0;
    } else if d < -0.5 {
        d += 1.0;
    }
    let line: Vec<Color32> = (0..w)
        .map(|x| {
            let f = ((x as f32 + 0.5 - w as f32 / 2.0).abs() / (w as f32 / 2.0)).min(1.0);
            // (between two bands: blended, so the low end isn't a row of blocks)
            let at = f * (half - 1) as f32;
            let (i, k) = (at as usize, at.fract());
            let v = (vals[i] * (1.0 - k) + vals[(i + 1).min(half - 1)] * k).powf(1.6);
            let hh = (h1 + d * f + (bright - 0.3) * 0.12).rem_euclid(1.0);
            let sat = (0.95 - (v - 0.75).max(0.0) * 2.4).clamp(0.2, 0.95);
            Color32::from(eframe::egui::ecolor::Hsva::new(hh, sat, v.clamp(0.0, 1.0), 1.0))
        })
        .collect();
    for y in 0..rows {
        s.buf[y * w..y * w + w].copy_from_slice(&line);
    }
    let _ = SCOPE_LEN;
}

/// A small radix-2 FFT. The visualizer only ever needs one fixed size, so this replaces a
/// general-purpose FFT library (~800 KB of code) with precomputed tables.
pub(crate) struct Fft {
    n: usize,
    rev: Vec<u32>,
    twiddle: Vec<(f32, f32)>,
    /// Hann window
    window: Vec<f32>,
}

impl Fft {
    pub(crate) fn new(n: usize) -> Self {
        let bits = n.trailing_zeros();
        let tau = 2.0 * std::f32::consts::PI;
        Fft {
            n,
            rev: (0..n as u32).map(|i| i.reverse_bits() >> (32 - bits)).collect(),
            twiddle: (0..n / 2).map(|k| (-tau * k as f32 / n as f32).sin_cos()).map(|(s, c)| (c, s)).collect(),
            window: (0..n).map(|i| 0.5 - 0.5 * (tau * i as f32 / n as f32).cos()).collect(),
        }
    }

    /// In-place forward transform of (re, im).
    pub(crate) fn process(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let (half, step) = (len / 2, n / len);
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = self.twiddle[k * step];
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Fft;

    #[test]
    fn fft_finds_a_sine() {
        let n = 2048;
        let f = Fft::new(n);
        let mut re: Vec<f32> = (0..n).map(|i| (2.0 * std::f32::consts::PI * 100.0 * i as f32 / n as f32).sin()).collect();
        let mut im = vec![0.0; n];
        f.process(&mut re, &mut im);
        let mag: Vec<f32> = (0..n / 2).map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt()).collect();
        let peak = (0..n / 2).max_by(|&a, &b| mag[a].total_cmp(&mag[b])).unwrap();
        assert_eq!(peak, 100);
        assert!((mag[100] - n as f32 / 2.0).abs() < 1.0, "amplitude {}", mag[100]);
        assert!(mag[50] < 0.01 && mag[300] < 0.01);
    }
}

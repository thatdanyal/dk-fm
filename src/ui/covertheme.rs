//! THEME > Album Cover: Red Retro coloured by the song playing. The accents take the cover's
//! main colours, kept to a brightness and strength that always reads well (a muddy or neon cover
//! can't make DK.FM ugly); backgrounds and text only take a tint of it. Black-and-white and grey
//! covers turn DK.FM monochrome: white and light grey for a white cover, black and dark grey for a
//! dark one, with a hint of whatever little colour the cover has. Colours glide from one song to
//! the next.
use super::theme::{self, Pal};
use super::App;
use eframe::egui::{self, Color32};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

pub const KEY: &str = "album";

/// How much the cover colours DK.FM (Settings > Look): (setting, label, strength).
pub const STRENGTHS: [(&str, &str, f32); 3] = [("subtle", "SUBTLE", 0.35), ("balanced", "BALANCED", 0.65), ("bold", "BOLD", 1.0)];

pub fn strength(key: &str) -> f32 {
    STRENGTHS.iter().find(|s| s.0 == key).map(|s| s.2).unwrap_or(0.65)
}

/// A cover's main colour and, if it has one, a second clearly different one.
pub type Colors = (Color32, Option<Color32>);

/// What a cover looks like, for the theme.
#[derive(Clone, Copy, Debug)]
pub enum Look {
    Color(Colors),
    /// black and white / grey: how light it is overall (0..1) and its faint colour, if any
    Mono(f32, Option<Color32>),
}

/// Album Cover theme state, kept on the App.
#[derive(Default)]
pub struct State {
    /// what the palette was last made for: (cover file, strength)
    applied: Option<(Option<String>, String)>,
    /// how each cover file looks (None: it couldn't be read)
    cache: HashMap<String, Option<Look>>,
    /// worked out in the background: (cover file, look)
    pending: Arc<Mutex<Option<(String, Option<Look>)>>>,
    loading: Option<String>,
    /// the glide between two palettes: (from, to, started at)
    glide: Option<(Pal, Pal, f64)>,
}

impl State {
    /// Make the palette again (theme or settings changed).
    pub fn reset(&mut self) {
        self.applied = None;
        self.glide = None;
    }
}

const GLIDE_SECS: f64 = 0.8;

/// Each frame: follow the song playing while the Album Cover theme is on.
pub fn update(app: &mut App, ctx: &egui::Context) {
    if app.theme_key != KEY {
        return;
    }
    let cover = app.player.current_id().and_then(|id| app.lib.track(&id)).and_then(|t| t.thumb.or(t.cover));
    let strength_key = app.settings.lock().cover_strength.clone();
    let want = (cover.clone(), strength_key.clone());
    if let Some((file, cols)) = app.album.pending.lock().take() {
        app.album.cache.insert(file.clone(), cols);
        if app.album.loading.as_ref() == Some(&file) {
            app.album.loading = None;
            app.album.applied = None;
        }
    }
    if app.album.applied.as_ref() != Some(&want) {
        let base = theme::resolve(&app.settings.lock());
        let target = match &cover {
            None => Some(base),
            Some(c) => match app.album.cache.get(c) {
                Some(Some(Look::Color(cols))) => Some(tint(&base, *cols, strength(&strength_key))),
                Some(Some(Look::Mono(light, hint))) => Some(mono(*light, *hint, strength(&strength_key))),
                Some(None) => Some(base),
                None => {
                    // work the colours out in the background; keep the current ones meanwhile
                    if app.album.loading.as_ref() != Some(c) {
                        app.album.loading = Some(c.clone());
                        let (path, slot, file, ctx) = (app.lib.cover_path(c), app.album.pending.clone(), c.clone(), ctx.clone());
                        std::thread::spawn(move || {
                            let cols = image::open(&path).ok().map(|im| cover_look(&im.thumbnail(48, 48).to_rgb8()));
                            *slot.lock() = Some((file, cols));
                            ctx.request_repaint();
                        });
                    }
                    None
                }
            },
        };
        if let Some(to) = target {
            app.album.applied = Some(want);
            let now = ctx.input(|i| i.time);
            // the very first time (DK.FM just opened, or the theme was picked) there's nothing to glide from
            app.album.glide = Some((app.pal, to, now));
        }
    }
    if let Some((from, to, start)) = app.album.glide {
        let t = ((ctx.input(|i| i.time) - start) / GLIDE_SECS).clamp(0.0, 1.0) as f32;
        let e = t * t * (3.0 - 2.0 * t);
        app.pal = lerp_pal(&from, &to, e);
        theme::apply(ctx, &app.pal);
        if t >= 1.0 {
            app.album.glide = None;
            app.scope.invalidate();
        } else {
            ctx.request_repaint();
        }
    }
}

/// The cover's main colours (`img` is small, about 48 x 48): the most common colourful hue,
/// and a second one at least 60 degrees away if it's common enough. None for a grey cover.
pub fn cover_colors(img: &image::RgbImage) -> Option<Colors> {
    const BINS: usize = 36;
    let mut weight = [0f32; BINS];
    let mut sum = [[0f32; 3]; BINS];
    let n = (img.width() * img.height()).max(1) as f32;
    for p in img.pixels() {
        let (h, s, l) = hsl(Color32::from_rgb(p[0], p[1], p[2]));
        if s < 0.22 || !(0.12..=0.92).contains(&l) {
            continue;
        }
        // vivid mid-tones count most
        let w = s * (1.0 - (l - 0.5).abs() * 1.4);
        let b = ((h / 360.0 * BINS as f32) as usize).min(BINS - 1);
        weight[b] += w;
        for (k, v) in sum[b].iter_mut().zip([p[0], p[1], p[2]]) {
            *k += w * v as f32;
        }
    }
    if weight.iter().sum::<f32>() / n < 0.04 {
        return None;
    }
    // neighbouring hues count together
    let near = |b: usize| weight[(b + BINS - 1) % BINS] + weight[b] + weight[(b + 1) % BINS];
    let avg = |b: usize| {
        let (mut w, mut c) = (0f32, [0f32; 3]);
        for i in [(b + BINS - 1) % BINS, b, (b + 1) % BINS] {
            w += weight[i];
            for k in 0..3 {
                c[k] += sum[i][k];
            }
        }
        Color32::from_rgb((c[0] / w) as u8, (c[1] / w) as u8, (c[2] / w) as u8)
    };
    let first = (0..BINS).max_by(|a, b| near(*a).total_cmp(&near(*b)))?;
    let apart = |b: usize| {
        let d = b.abs_diff(first);
        d.min(BINS - d) >= 6
    };
    let second = (0..BINS).filter(|b| apart(*b)).max_by(|a, b| near(*a).total_cmp(&near(*b))).filter(|b| near(*b) >= near(first) * 0.25);
    Some((avg(first), second.map(avg)))
}

/// A cover's look: its colours, or for a black-and-white / grey one how light it is and the most
/// common of the little colour it has.
pub fn cover_look(img: &image::RgbImage) -> Look {
    if let Some(c) = cover_colors(img) {
        return Look::Color(c);
    }
    let n = (img.width() * img.height()).max(1) as f32;
    let light = img.pixels().map(|p| (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0).sum::<f32>() / n;
    // its faint colour: the most common hue among pixels with any colour at all
    const BINS: usize = 12;
    let mut weight = [0f32; BINS];
    let mut sum = [[0f32; 3]; BINS];
    for p in img.pixels() {
        let (h, s, l) = hsl(Color32::from_rgb(p[0], p[1], p[2]));
        if s < 0.12 || !(0.08..=0.95).contains(&l) {
            continue;
        }
        let b = ((h / 360.0 * BINS as f32) as usize).min(BINS - 1);
        weight[b] += s;
        for (k, v) in sum[b].iter_mut().zip([p[0], p[1], p[2]]) {
            *k += s * v as f32;
        }
    }
    let best = (0..BINS).max_by(|a, b| weight[*a].total_cmp(&weight[*b])).filter(|b| weight[*b] / n >= 0.004);
    let hint = best.map(|b| Color32::from_rgb((sum[b][0] / weight[b]) as u8, (sum[b][1] / weight[b]) as u8, (sum[b][2] / weight[b]) as u8));
    Look::Mono(light, hint)
}

/// Monochrome for a black-and-white cover: white and light greys for a white cover (`light` near
/// 1), black and dark greys for a dark one, with a hint of `hint` (`s` = 0..1 how much).
pub fn mono(light: f32, hint: Option<Color32>, s: f32) -> Pal {
    let g = |v: f32| {
        let v = v.round().clamp(0.0, 255.0) as u8;
        Color32::from_rgb(v, v, v)
    };
    let mut p = theme::palette("mono", None);
    if light >= 0.6 {
        // white side: the whiter the cover, the whiter DK.FM
        let w = ((light - 0.6) / 0.3).clamp(0.0, 1.0);
        let at = |a: f32, b: f32| g(lerp(a, b, w));
        (p.bg, p.bg2, p.panel, p.panel_hi) = (at(204.0, 246.0), at(192.0, 234.0), at(214.0, 252.0), at(196.0, 238.0));
        (p.line, p.line_hi, p.faint) = (at(160.0, 204.0), at(108.0, 140.0), at(150.0, 182.0));
        (p.text, p.dim, p.accent, p.accent2) = (g(18.0), at(78.0, 96.0), g(22.0), g(96.0));
        (p.ink, p.lcd_bg, p.lcd, p.shadow) = (g(250.0), g(24.0), g(240.0), at(150.0, 186.0));
        p.sel = Color32::from_rgba_unmultiplied(0, 0, 0, 30);
        p.dark = false;
    } else {
        // dark side: black for a black cover, charcoal greys for a greyer one
        let k = ((light - 0.08) / 0.5).clamp(0.0, 1.0);
        let at = |a: f32, b: f32| g(lerp(a, b, k));
        (p.bg, p.bg2, p.panel, p.panel_hi) = (at(5.0, 26.0), at(9.0, 32.0), at(13.0, 38.0), at(22.0, 50.0));
        (p.line, p.line_hi, p.faint) = (at(40.0, 72.0), at(80.0, 112.0), at(56.0, 86.0));
        (p.text, p.dim, p.accent, p.accent2) = (g(236.0), at(136.0, 152.0), g(242.0), g(158.0));
        (p.ink, p.lcd_bg, p.lcd, p.shadow) = (g(0.0), at(14.0, 30.0), g(245.0), Color32::BLACK);
        p.sel = Color32::from_rgba_unmultiplied(255, 255, 255, 26);
        p.dark = true;
    }
    if let Some(h) = hint {
        // the cover's bit of colour: the second accent, and a touch on the main one and the borders
        let (hue, sat, _) = hsl(h);
        let c = from_hsl(hue, sat.clamp(0.5, 0.85), if p.dark { 0.62 } else { 0.38 });
        let s = s.clamp(0.0, 1.0);
        let mix = |a: Color32, t: f32| lerp_pal_color(a, c, t);
        p.accent2 = mix(p.accent2, 0.55 + 0.45 * s);
        p.accent = mix(p.accent, 0.12 + 0.2 * s);
        p.line_hi = mix(p.line_hi, 0.25 * s);
        p.lcd = mix(p.lcd, 0.2 * s);
        p.sel = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 40);
    }
    p
}

fn lerp_pal_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |u: u8, v: u8| lerp(u as f32, v as f32, t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The Album Cover theme's swatch: a rainbow (it takes any cover's colours).
pub fn rainbow(p: &egui::Painter, r: egui::Rect) {
    const N: usize = 6;
    let w = r.width() / N as f32;
    for i in 0..N {
        let x = r.left() + i as f32 * w;
        let c = from_hsl(i as f32 * 360.0 / N as f32, 0.85, 0.58);
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(x, r.top()), egui::pos2(x + w + 0.5, r.bottom())), 0.0, c);
    }
}

/// `base` coloured by a cover's colours, `s` = 0..1 how much.
pub fn tint(base: &Pal, (c1, c2): Colors, s: f32) -> Pal {
    let s = s.clamp(0.0, 1.0);
    // a colour that always reads well on a dark background
    let tame = |c: Color32, light: (f32, f32)| {
        let (h, sat, l) = hsl(c);
        (h, sat.clamp(0.55, 0.92), l.clamp(light.0, light.1))
    };
    let main = tame(c1, (0.52, 0.64));
    let second = c2.map(|c| tame(c, (0.58, 0.70))).unwrap_or(((main.0 + 32.0) % 360.0, main.1, 0.64));
    // accents follow the cover most (but keep a little of the theme); backgrounds and text a tint
    let toward = |c: Color32, to: (f32, f32, f32), t: f32, keep_light: bool| {
        let (h, sat, l) = hsl(c);
        let a = c.a();
        let out = from_hsl(lerp_hue(h, to.0, t), lerp(sat, to.1, if keep_light { t * 0.3 } else { t }), if keep_light { l } else { lerp(l, to.2, t) });
        Color32::from_rgba_unmultiplied(out.r(), out.g(), out.b(), a)
    };
    let (ta, tb, tt) = (0.5 + 0.5 * s, s, 0.6 * s);
    let mut p = *base;
    p.accent = toward(base.accent, main, ta, false);
    p.accent2 = toward(base.accent2, second, ta, false);
    p.lcd = p.accent;
    p.sel = Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 46);
    for f in [&mut p.bg, &mut p.bg2, &mut p.panel, &mut p.panel_hi, &mut p.line, &mut p.line_hi, &mut p.faint, &mut p.lcd_bg, &mut p.ink] {
        *f = toward(*f, main, tb, true);
    }
    for f in [&mut p.text, &mut p.dim] {
        *f = toward(*f, main, tt, true);
    }
    p
}

/// Part way (`t` 0..1) from palette `a` to `b`.
pub fn lerp_pal(a: &Pal, b: &Pal, t: f32) -> Pal {
    let mix = |x: Color32, y: Color32| {
        let [r0, g0, b0, a0] = x.to_srgba_unmultiplied();
        let [r1, g1, b1, a1] = y.to_srgba_unmultiplied();
        let m = |u: u8, v: u8| (u as f32 + (v as f32 - u as f32) * t).round() as u8;
        Color32::from_rgba_unmultiplied(m(r0, r1), m(g0, g1), m(b0, b1), m(a0, a1))
    };
    let mut p = *b;
    for (name, _) in theme::FIELDS {
        let (mut aa, mut bb) = (*a, *b);
        if let (Some(x), Some(y), Some(out)) = (aa.field(name).copied(), bb.field(name).copied(), p.field(name)) {
            *out = mix(x, y);
        }
    }
    p
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Hue `t` of the way from `a` to `b`, the short way round.
fn lerp_hue(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + 540.0) % 360.0 - 180.0;
    (a + d * t + 360.0) % 360.0
}

/// (hue 0..360, saturation 0..1, lightness 0..1)
fn hsl(c: Color32) -> (f32, f32, f32) {
    let (r, g, b) = (c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r { ((g - b) / d).rem_euclid(6.0) } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    (h * 60.0, s.min(1.0), l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 % 6 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(to(r), to(g), to(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(f: impl Fn(u32, u32) -> [u8; 3]) -> image::RgbImage {
        image::RgbImage::from_fn(48, 48, |x, y| image::Rgb(f(x, y)))
    }

    #[test]
    fn finds_a_covers_colours() {
        // mostly blue with an orange stripe (a small corner alone is too little for a second colour)
        let (c1, c2) = cover_colors(&img(|x, _| if x < 18 { [240, 140, 30] } else { [30, 70, 220] })).unwrap();
        assert!(c1.b() > 150 && c1.r() < 80, "{c1:?}");
        let c2 = c2.unwrap();
        assert!(c2.r() > 200 && c2.b() < 80, "{c2:?}");
        // black and white / grey: no colours, it's monochrome
        assert!(cover_colors(&img(|x, _| if x % 2 == 0 { [10, 10, 10] } else { [200, 200, 200] })).is_none());
    }

    #[test]
    fn black_and_white_covers_go_monochrome() {
        // a white cover with a little red: light, white-ish DK.FM with a red hint
        let white = cover_look(&img(|x, y| if x < 4 && y < 4 { [200, 30, 30] } else { [240, 240, 240] }));
        let Look::Mono(l, Some(h)) = white else { panic!("{white:?}") };
        assert!(l > 0.85 && h.r() > 150 && h.g() < 80, "{l} {h:?}");
        let p = mono(l, Some(h), 0.65);
        assert!(!p.dark && hsl(p.bg).2 > 0.9 && hsl(p.text).2 < 0.15);
        let (hue, sat, _) = hsl(p.accent2);
        assert!(sat > 0.4 && !(30.0..330.0).contains(&hue), "{:?}", p.accent2);
        // a black cover: black and dark grey, no colour, light text
        let black = cover_look(&img(|x, _| if x % 7 == 0 { [70, 70, 70] } else { [8, 8, 8] }));
        let Look::Mono(l, None) = black else { panic!("{black:?}") };
        let p = mono(l, None, 0.65);
        assert!(p.dark && hsl(p.bg).2 < 0.04 && hsl(p.text).2 > 0.85 && hsl(p.accent).1 < 0.05);
        // a mid-grey one is greyer than the black one
        let q = mono(0.45, None, 0.65);
        assert!(q.dark && hsl(q.bg).2 > hsl(p.bg).2);
    }

    #[test]
    fn any_cover_stays_readable() {
        let base = theme::palette("red-retro", None);
        // a muddy brown, a neon green, a near-black navy: accents land in a readable band
        for c in [Color32::from_rgb(110, 80, 50), Color32::from_rgb(0, 255, 40), Color32::from_rgb(10, 15, 60)] {
            for s in [0.35, 0.65, 1.0] {
                let p = tint(&base, (c, None), s);
                let (_, sat, l) = hsl(p.accent);
                assert!((0.45..=0.72).contains(&l) && sat >= 0.45, "{c:?} {s}: {:?} l={l} s={sat}", p.accent);
                // backgrounds stay as dark as the theme's, text as light
                assert!((hsl(p.bg).2 - hsl(base.bg).2).abs() < 0.02 && (hsl(p.text).2 - hsl(base.text).2).abs() < 0.03);
            }
        }
        // the strength setting changes how far the accent moves from the theme's red
        let blue = Color32::from_rgb(40, 90, 230);
        let hue = |s: f32| hsl(tint(&base, (blue, None), s).accent).0;
        assert!((hue(1.0) - hsl(blue).0).abs() < 3.0);
        let (sub, bal) = ((hue(0.35) - hsl(blue).0).abs(), (hue(0.65) - hsl(blue).0).abs());
        assert!(sub > bal && bal > 3.0, "{sub} {bal}");
    }

    #[test]
    fn glides_between_palettes() {
        let (a, b) = (theme::palette("red-retro", None), theme::palette("ice", None));
        assert_eq!(lerp_pal(&a, &b, 0.0).accent, a.accent);
        assert_eq!(lerp_pal(&a, &b, 1.0).accent, b.accent);
        assert_eq!(lerp_pal(&a, &b, 1.0).sel, b.sel);
    }
}

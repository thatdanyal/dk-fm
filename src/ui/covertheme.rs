//! THEME > Album Cover: Red Retro coloured by the song playing. The accents take the cover's
//! main colours, kept to a brightness and strength that always reads well (a muddy or neon cover
//! can't make DK.FM ugly); backgrounds and text only take a tint of it. Grey and black-and-white
//! covers leave Red Retro as it is. Colours glide from one song to the next.
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

/// Album Cover theme state, kept on the App.
#[derive(Default)]
pub struct State {
    /// what the palette was last made for: (cover file, strength)
    applied: Option<(Option<String>, String)>,
    /// colours per cover file (None: a grey cover)
    cache: HashMap<String, Option<Colors>>,
    /// colours worked out in the background: (cover file, colours)
    pending: Arc<Mutex<Option<(String, Option<Colors>)>>>,
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
                Some(Some(cols)) => Some(tint(&base, *cols, strength(&strength_key))),
                Some(None) => Some(base),
                None => {
                    // work the colours out in the background; keep the current ones meanwhile
                    if app.album.loading.as_ref() != Some(c) {
                        app.album.loading = Some(c.clone());
                        let (path, slot, file, ctx) = (app.lib.cover_path(c), app.album.pending.clone(), c.clone(), ctx.clone());
                        std::thread::spawn(move || {
                            let cols = image::open(&path).ok().and_then(|im| cover_colors(&im.thumbnail(48, 48).to_rgb8()));
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
        // black and white / grey: no colours (Red Retro stays)
        assert!(cover_colors(&img(|x, _| if x % 2 == 0 { [10, 10, 10] } else { [200, 200, 200] })).is_none());
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

//! Themes (same palettes as the Electron version) and fonts.
use crate::store::{CustomTheme, Settings};
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke};
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Pal {
    pub bg: Color32,
    pub bg2: Color32,
    pub panel: Color32,
    pub panel_hi: Color32,
    pub line: Color32,
    pub line_hi: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub ink: Color32,
    pub lcd_bg: Color32,
    pub lcd: Color32,
    pub shadow: Color32,
    pub sel: Color32,
    pub dark: bool,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const THEMES: &[(&str, &str)] = &[
    ("red-retro", "Red Retro"),
    ("amber-crt", "Amber CRT"),
    ("green-phosphor", "Green Phosphor"),
    ("synthwave", "Synthwave"),
    ("ice", "Ice"),
    ("mono", "Mono"),
    ("paper", "Paper"),
    (super::covertheme::KEY, "Album Cover"),
];

pub fn palette(name: &str, accent: Option<&str>) -> Pal {
    let mut p = match name {
        "amber-crt" => Pal { bg: hex(0x0a0702), bg2: hex(0x110c03), panel: hex(0x151004), panel_hi: hex(0x1e1706), line: hex(0x3b2c0b), line_hi: hex(0x6b4f12), text: hex(0xffe2b0), dim: hex(0xa7834a), faint: hex(0x5a4519), accent: hex(0xffb000), accent2: hex(0xff6a1f), ink: hex(0x1a1000), lcd_bg: hex(0x1a1203), lcd: hex(0xffb000), shadow: Color32::BLACK, sel: Color32::from_rgba_unmultiplied(255, 176, 0, 40), dark: true },
        "green-phosphor" => Pal { bg: hex(0x020a04), bg2: hex(0x031006), panel: hex(0x041407), panel_hi: hex(0x071d0b), line: hex(0x0f3a18), line_hi: hex(0x1b6a2c), text: hex(0xc8ffd4), dim: hex(0x4f9e62), faint: hex(0x1f4f2b), accent: hex(0x33ff66), accent2: hex(0xb6ff3b), ink: hex(0x001a06), lcd_bg: hex(0x021505), lcd: hex(0x33ff66), shadow: Color32::BLACK, sel: Color32::from_rgba_unmultiplied(51, 255, 102, 36), dark: true },
        "synthwave" => Pal { bg: hex(0x0b0414), bg2: hex(0x10061d), panel: hex(0x140824), panel_hi: hex(0x1c0c33), line: hex(0x35185c), line_hi: hex(0x5e2aa0), text: hex(0xf6dcff), dim: hex(0x9a72bf), faint: hex(0x452866), accent: hex(0xff2bd6), accent2: hex(0x22e5ff), ink: hex(0x1a0216), lcd_bg: hex(0x170828), lcd: hex(0xff4fe0), shadow: Color32::BLACK, sel: Color32::from_rgba_unmultiplied(255, 43, 214, 40), dark: true },
        "ice" => Pal { bg: hex(0x03080d), bg2: hex(0x050d14), panel: hex(0x07111b), panel_hi: hex(0x0b1a28), line: hex(0x143049), line_hi: hex(0x22547e), text: hex(0xd5f1ff), dim: hex(0x5f8fae), faint: hex(0x1f3b52), accent: hex(0x3bc8ff), accent2: hex(0xa6f0ff), ink: hex(0x00141d), lcd_bg: hex(0x04121c), lcd: hex(0x4fd2ff), shadow: Color32::BLACK, sel: Color32::from_rgba_unmultiplied(59, 200, 255, 38), dark: true },
        "mono" => Pal { bg: hex(0x080808), bg2: hex(0x0d0d0d), panel: hex(0x121212), panel_hi: hex(0x1a1a1a), line: hex(0x2e2e2e), line_hi: hex(0x555555), text: hex(0xececec), dim: hex(0x8a8a8a), faint: hex(0x3a3a3a), accent: hex(0xf2f2f2), accent2: hex(0x9a9a9a), ink: hex(0x000000), lcd_bg: hex(0x111111), lcd: hex(0xf5f5f5), shadow: Color32::BLACK, sel: Color32::from_rgba_unmultiplied(255, 255, 255, 26), dark: true },
        "paper" => Pal { bg: hex(0xefe6d6), bg2: hex(0xe7dcc8), panel: hex(0xf6efe2), panel_hi: hex(0xece2cf), line: hex(0xcdb99a), line_hi: hex(0xa8865c), text: hex(0x2a0f0a), dim: hex(0x85614a), faint: hex(0xcdb99a), accent: hex(0xc8231b), accent2: hex(0x2a6f8f), ink: hex(0xfff6ea), lcd_bg: hex(0x2a0f0a), lcd: hex(0xff5a45), shadow: hex(0xa8865c), sel: Color32::from_rgba_unmultiplied(200, 35, 27, 30), dark: false },
        _ => Pal { bg: hex(0x0b0404), bg2: hex(0x120606), panel: hex(0x160808), panel_hi: hex(0x200c0b), line: hex(0x3b1512), line_hi: hex(0x6a231d), text: hex(0xffd9cf), dim: hex(0xa0645a), faint: hex(0x5c2a24), accent: hex(0xff3b2f), accent2: hex(0xffae3b), ink: hex(0x1a0303), lcd_bg: hex(0x1b0503), lcd: hex(0xff4a3a), shadow: Color32::BLACK, sel: Color32::from_rgba_unmultiplied(255, 59, 47, 46), dark: true },
    };
    if let Some(a) = accent.and_then(parse_hex) {
        p.accent = a;
        p.lcd = a;
    }
    p
}

/// "#rrggbb" or "#rrggbbaa".
pub fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).ok()?;
    match s.len() {
        6 => Some(hex(v)),
        8 => Some(Color32::from_rgba_unmultiplied((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)),
        _ => None,
    }
}

pub fn to_hex(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// Palette fields by name, for the theme editor and custom themes.
pub const FIELDS: [(&str, &str); 16] = [
    ("bg", "Background"), ("bg2", "Sidebar & title bar"), ("panel", "Panels"), ("panel_hi", "Hover / raised"),
    ("line", "Lines"), ("line_hi", "Borders"), ("text", "Text"), ("dim", "Dim text"),
    ("faint", "Faint text"), ("accent", "Accent"), ("accent2", "Second accent"), ("ink", "Text on accent"),
    ("lcd_bg", "Display background"), ("lcd", "Display text"), ("shadow", "Shadows"), ("sel", "Selection"),
];

/// The theme editor's main colours: (key, name, what it colours). Each sets its palette field and
/// the shades that go with it (see `Pal::set_role`).
pub const ROLES: [(&str, &str, &str); 6] = [
    ("main", "MAIN", "The background, with the panels and title bar a shade off it"),
    ("accent", "ACCENT 1", "Buttons, highlights, the playing song, the display"),
    ("accent2", "ACCENT 2", "Headings, links, buttons that are on"),
    ("text", "TEXT", "Text, with dimmer shades for less important text"),
    ("detail", "DETAIL", "Borders and lines"),
    ("display", "DISPLAY", "Behind the LCD-style numbers and cards"),
];

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn light(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

impl Pal {
    /// A main colour's current value.
    pub fn role(&self, role: &str) -> Color32 {
        match role {
            "main" => self.bg,
            "accent" => self.accent,
            "accent2" => self.accent2,
            "text" => self.text,
            "detail" => self.line_hi,
            "display" => self.lcd_bg,
            _ => self.accent,
        }
    }

    /// Sets a main colour and the shades that go with it.
    pub fn set_role(&mut self, role: &str, c: Color32) {
        match role {
            "main" => {
                self.bg = c;
                self.bg2 = mix(c, self.text, 0.03);
                self.panel = mix(c, self.text, 0.05);
                self.panel_hi = mix(c, self.text, 0.09);
                self.dark = light(c) < 0.5;
                self.shadow = if self.dark { Color32::BLACK } else { mix(c, Color32::BLACK, 0.3) };
            }
            "accent" => {
                self.accent = c;
                self.lcd = c;
                self.sel = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 40);
                // text on it: dark on a bright accent, light on a dark one
                self.ink = if light(c) > 0.45 { mix(c, Color32::BLACK, 0.9) } else { mix(c, Color32::WHITE, 0.92) };
            }
            "accent2" => self.accent2 = c,
            "text" => {
                self.text = c;
                self.dim = mix(c, self.bg, 0.4);
                self.faint = mix(c, self.bg, 0.68);
            }
            "detail" => {
                self.line_hi = c;
                self.line = mix(c, self.bg, 0.45);
            }
            "display" => self.lcd_bg = c,
            _ => {}
        }
    }

    pub fn field(&mut self, name: &str) -> Option<&mut Color32> {
        Some(match name {
            "bg" => &mut self.bg, "bg2" => &mut self.bg2, "panel" => &mut self.panel, "panel_hi" => &mut self.panel_hi,
            "line" => &mut self.line, "line_hi" => &mut self.line_hi, "text" => &mut self.text, "dim" => &mut self.dim,
            "faint" => &mut self.faint, "accent" => &mut self.accent, "accent2" => &mut self.accent2, "ink" => &mut self.ink,
            "lcd_bg" => &mut self.lcd_bg, "lcd" => &mut self.lcd, "shadow" => &mut self.shadow, "sel" => &mut self.sel,
            _ => return None,
        })
    }
    pub fn to_custom(mut self, name: &str) -> CustomTheme {
        let colors = FIELDS.iter().map(|(f, _)| (f.to_string(), to_hex(*self.field(f).unwrap()))).collect();
        CustomTheme { name: name.into(), dark: self.dark, colors }
    }
    pub fn from_custom(c: &CustomTheme) -> Pal {
        let mut p = palette("", None);
        p.dark = c.dark;
        for (f, v) in &c.colors {
            if let (Some(slot), Some(col)) = (p.field(f), parse_hex(v)) {
                *slot = col;
            }
        }
        p
    }
}

pub const CUSTOM: &str = "custom:";

/// The palette the settings ask for: a built-in theme (+ accent), or a custom one by name.
pub fn resolve(s: &Settings) -> Pal {
    if let Some(name) = s.theme.strip_prefix(CUSTOM) {
        if let Some(c) = s.custom_themes.iter().find(|c| c.name == name) {
            let mut p = Pal::from_custom(c);
            if let Some(a) = s.accent.as_deref().and_then(parse_hex) {
                p.accent = a;
                p.lcd = a;
            }
            return p;
        }
    }
    palette(&s.theme, s.accent.as_deref())
}

/// Every theme to pick from (key, name): built-ins, then custom ones, in the order you dragged
/// them into (themes you haven't moved keep their place after those).
pub fn all_themes(s: &Settings) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = THEMES.iter().map(|(k, n)| (k.to_string(), n.to_string())).chain(s.custom_themes.iter().map(|c| (format!("{CUSTOM}{}", c.name), c.name.clone()))).collect();
    if !s.theme_order.is_empty() {
        v.sort_by_key(|(k, _)| s.theme_order.iter().position(|o| o == k).unwrap_or(usize::MAX));
    }
    v
}

/// A theme's colour block in pickers: its three main colours side by side (background, accent,
/// second accent). Album Cover's is a rainbow.
pub fn swatch(p: &egui::Painter, r: egui::Rect, s: &Settings, key: &str) {
    if key == super::covertheme::KEY {
        super::covertheme::rainbow(p, r);
        return;
    }
    let t = theme_pal(s, key);
    let w = r.width() / 3.0;
    for (i, c) in [t.bg, t.accent, t.accent2].into_iter().enumerate() {
        p.rect_filled(egui::Rect::from_min_size(r.min + egui::vec2(w * i as f32, 0.0), egui::vec2(w, r.height())), 0.0, c);
    }
    p.rect_stroke(r, 0.0, Stroke::new(1.0_f32, t.line_hi), egui::StrokeKind::Inside);
}

pub fn theme_pal(s: &Settings, key: &str) -> Pal {
    // Album Cover changes with every song: shown with a sample cover's colours
    if key == super::covertheme::KEY {
        return super::covertheme::tint(&palette(key, None), (Color32::from_rgb(150, 70, 230), Some(Color32::from_rgb(40, 210, 190))), 0.65);
    }
    match key.strip_prefix(CUSTOM).and_then(|n| s.custom_themes.iter().find(|c| c.name == n)) {
        Some(c) => Pal::from_custom(c),
        None => palette(key, None),
    }
}

pub fn vt(size: f32) -> FontId {
    FontId::new(step(size), FontFamily::Name("vt".into()))
}
pub fn px(size: f32) -> FontId {
    FontId::new(step(size), FontFamily::Name("px".into()))
}

/// Text sizes in steps (whole pixels, 4 px apart above 32): egui draws each size's letters into
/// its font texture once and keeps them, so sizes that follow a panel's width (lyrics, the deck's
/// time, cover placeholders) used to add more memory with every resize.
fn step(size: f32) -> f32 {
    if size <= 32.0 { size.round().max(1.0) } else { (size / 4.0).round() * 4.0 }
}

/// Chosen font ("pixel" / "clean" / "sys:<file>") and whether headings stay pixel.
static FONT: Mutex<(String, bool)> = Mutex::new((String::new(), true));

/// Applies the font settings; does nothing (no font rebuild) when they haven't changed.
pub fn set_font(ctx: &egui::Context, choice: &str, pixel_headings: bool) {
    {
        let mut f = FONT.lock();
        if f.0 == choice && f.1 == pixel_headings {
            return;
        }
        *f = (choice.to_string(), pixel_headings);
    }
    ctx.set_fonts(font_definitions());
}

pub fn font_definitions() -> FontDefinitions {
    // Only our own fonts (egui's built-in set is 1.4 MB). Fallbacks, tried in order for any
    // character the retro fonts lack: accented/Greek/Cyrillic text, emoji, UI icons, then the
    // pixel font (arrows and triangles).
    let mut f = base_fonts();
    // another font for body text (and headings unless they stay pixel): the same bytes twice,
    // scaled to the sizes the retro fonts are used at
    let (choice, pixel_heads) = FONT.lock().clone();
    let bytes: Option<&'static [u8]> = match choice.as_str() {
        "clean" => Some(TEXT_FONT),
        c => c.strip_prefix("sys:").and_then(super::fonts::load),
    };
    if let Some(b) = bytes {
        for (name, scale) in [("body", 0.8), ("head", 1.55)] {
            let mut fd = FontData::from_static(b);
            fd.tweak.scale = scale;
            f.font_data.insert(name.into(), Arc::new(fd));
        }
        let chain = |first: &str| [first, "text", "emoji", "icons", "press"].iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for fam in [FontFamily::Name("vt".into()), FontFamily::Proportional, FontFamily::Monospace] {
            f.families.insert(fam, chain("body"));
        }
        if !pixel_heads {
            f.families.insert(FontFamily::Name("px".into()), chain("head"));
        }
    }
    // Japanese / Korean / Chinese system fonts loaded so far
    for (name, fd) in super::cjk::loaded() {
        f.font_data.insert(name.clone(), Arc::new(fd));
        for fam in f.families.values_mut() {
            fam.push(name.clone());
        }
    }
    f
}

/// Ubuntu (subset): fallback for accented text, and the "clean" font
const TEXT_FONT: &[u8] = include_bytes!("../../assets/fallback-text.ttf");

fn base_fonts() -> FontDefinitions {
    let mut f = FontDefinitions::empty();
    let fonts: [(&str, &'static [u8]); 5] = [
        ("vt323", include_bytes!("../../assets/VT323-Regular.ttf")),
        ("press", include_bytes!("../../assets/PressStart2P-Regular.ttf")),
        ("text", TEXT_FONT),
        ("icons", include_bytes!("../../assets/fallback-icons.ttf")),
        ("emoji", include_bytes!("../../assets/fallback-emoji.ttf")),
    ];
    for (name, bytes) in fonts {
        f.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    let chain = |first: &str| [first, "text", "emoji", "icons", "press"].iter().map(|s| s.to_string()).collect::<Vec<_>>();
    f.families.insert(FontFamily::Name("vt".into()), chain("vt323"));
    f.families.insert(FontFamily::Name("px".into()), chain("press"));
    f.families.insert(FontFamily::Proportional, chain("vt323"));
    f.families.insert(FontFamily::Monospace, chain("vt323"));
    f
}

pub fn apply(ctx: &egui::Context, p: &Pal) {
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.panel_hi;
    v.code_bg_color = p.bg;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent2;
    // selected options (dropdown entries, selected text) are outlined with a faint tint, not
    // filled solid: a solid accent fill hid the label in themes whose accent is close to the text
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 60);
    v.selection.stroke = Stroke::new(2.0_f32, p.accent);
    v.window_stroke = Stroke::new(2.0_f32, p.accent);
    v.window_corner_radius = egui::CornerRadius::ZERO;
    v.menu_corner_radius = egui::CornerRadius::ZERO;
    v.window_shadow = egui::epaint::Shadow { offset: [6, 6], blur: 0, spread: 0, color: p.shadow };
    v.popup_shadow = egui::epaint::Shadow { offset: [4, 4], blur: 0, spread: 0, color: p.shadow };
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = egui::CornerRadius::ZERO;
        w.fg_stroke = Stroke::new(1.0_f32, p.text);
    }
    v.widgets.noninteractive.bg_fill = p.panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.line);
    v.widgets.inactive.bg_fill = p.panel_hi;
    v.widgets.inactive.weak_bg_fill = p.panel_hi;
    v.widgets.inactive.bg_stroke = Stroke::new(2.0_f32, p.line_hi);
    v.widgets.hovered.bg_fill = p.panel_hi;
    v.widgets.hovered.weak_bg_fill = p.panel_hi;
    v.widgets.hovered.bg_stroke = Stroke::new(2.0_f32, p.accent);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.accent);
    v.widgets.active.bg_fill = p.accent;
    v.widgets.active.weak_bg_fill = p.accent;
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, p.ink);
    v.widgets.open.bg_fill = p.panel_hi;
    v.slider_trailing_fill = true;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        s.spacing.scroll.bar_width = 8.0;
        s.text_styles.insert(egui::TextStyle::Body, vt(19.0));
        s.text_styles.insert(egui::TextStyle::Button, vt(19.0));
        s.text_styles.insert(egui::TextStyle::Small, vt(16.0));
        s.text_styles.insert(egui::TextStyle::Heading, px(12.0));
        s.text_styles.insert(egui::TextStyle::Monospace, vt(18.0));
        s.interaction.selectable_labels = false;
    });
}

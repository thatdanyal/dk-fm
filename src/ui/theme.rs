//! Themes (same palettes as the Electron version) and fonts.
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke};
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

pub fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    u32::from_str_radix(s, 16).ok().map(hex)
}

pub fn to_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

pub fn vt(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("vt".into()))
}
pub fn px(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("px".into()))
}

pub fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
}

pub fn font_definitions() -> FontDefinitions {
    // Only our own fonts (egui's built-in set is 1.4 MB). Fallbacks, tried in order for any
    // character the retro fonts lack: accented/Greek/Cyrillic text, emoji, UI icons, then the
    // pixel font (arrows and triangles).
    let mut f = FontDefinitions::empty();
    let fonts: [(&str, &'static [u8]); 5] = [
        ("vt323", include_bytes!("../../assets/VT323-Regular.ttf")),
        ("press", include_bytes!("../../assets/PressStart2P-Regular.ttf")),
        ("text", include_bytes!("../../assets/fallback-text.ttf")),
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
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0_f32, p.ink);
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

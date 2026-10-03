//! Fonts installed on the computer, for Settings > Look. Listing reads only each file's name
//! table (on a background thread, once); a chosen font is memory-mapped like the CJK fonts.
use eframe::egui;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

pub struct SysFont {
    pub name: String,
    pub path: String,
}

static LIST: OnceLock<Vec<SysFont>> = OnceLock::new();
static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Installed fonts sorted by name; None while the list is still being read.
pub fn list(ctx: &egui::Context) -> Option<&'static [SysFont]> {
    if let Some(l) = LIST.get() {
        return Some(l);
    }
    if !STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = LIST.set(scan());
            ctx.request_repaint();
        });
    }
    None
}

fn scan() -> Vec<SysFont> {
    let mut out: Vec<SysFont> = Vec::new();
    for dir in dirs() {
        for e in walkdir::WalkDir::new(dir).max_depth(4).into_iter().flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).unwrap_or_default();
            if !matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                continue;
            }
            let Ok(bytes) = std::fs::File::open(p).and_then(|f| unsafe { memmap2::Mmap::map(&f) }) else { continue };
            if let Some(name) = font_name(&bytes) {
                if !out.iter().any(|f| f.name == name) {
                    out.push(SysFont { name, path: p.to_string_lossy().into_owned() });
                }
            }
        }
    }
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}

fn dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        v.push(PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts"));
        v.extend(dirs::data_local_dir().map(|d| d.join(r"Microsoft\Windows\Fonts")));
    }
    #[cfg(target_os = "macos")]
    {
        v.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
        v.extend(dirs::home_dir().map(|d| d.join("Library/Fonts")));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        v.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        v.extend(dirs::data_dir().map(|d| d.join("fonts")));
        v.extend(dirs::home_dir().map(|d| d.join(".fonts")));
    }
    v.retain(|d| d.is_dir());
    v
}

/// Full font name (name ID 4, else family ID 1) from the OpenType `name` table of the first face.
fn font_name(b: &[u8]) -> Option<String> {
    let u16_at = |o: usize| b.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize);
    let u32_at = |o: usize| b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize);
    let face = if b.starts_with(b"ttcf") { u32_at(12)? } else { 0 };
    let tables = u16_at(face + 4)?;
    let name = (0..tables).map(|i| face + 12 + i * 16).find(|&r| b.get(r..r + 4) == Some(b"name")).and_then(|r| u32_at(r + 8))?;
    let (count, strings) = (u16_at(name + 2)?, name + u16_at(name + 4)?);
    let mut best: Option<(u8, String)> = None;
    for i in 0..count {
        let r = name + 6 + i * 12;
        let (platform, encoding, lang, id, len, off) = (u16_at(r)?, u16_at(r + 2)?, u16_at(r + 4)?, u16_at(r + 6)?, u16_at(r + 8)?, u16_at(r + 10)?);
        let rank = match (id, platform) {
            (4, 3) if lang == 0x409 => 0,
            (4, 3) => 1,
            (1, 3) => 2,
            (4, 1) if encoding == 0 => 3,
            _ => continue,
        };
        if best.as_ref().map(|b| b.0 <= rank).unwrap_or(false) {
            continue;
        }
        let raw = b.get(strings + off..strings + off + len)?;
        let s = if platform == 3 { String::from_utf16_lossy(&raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>()) } else { raw.iter().map(|&c| c as char).collect() };
        let s = s.trim().to_string();
        if !s.is_empty() {
            best = Some((rank, s));
        }
    }
    best.map(|b| b.1)
}

/// The font file's bytes, mapped once per file for the rest of the run.
pub fn load(path: &str) -> Option<&'static [u8]> {
    static MAPPED: Mutex<Option<HashMap<String, &'static [u8]>>> = Mutex::new(None);
    let mut m = MAPPED.lock();
    let m = m.get_or_insert_with(HashMap::new);
    if let Some(b) = m.get(path) {
        return Some(b);
    }
    let b = super::cjk::map(&PathBuf::from(path))?;
    // a collection: only its first face is used
    if ab_glyph::FontRef::try_from_slice(b).is_err() {
        return None;
    }
    m.insert(path.to_string(), b);
    Some(b)
}

pub fn name_of(path: &str) -> String {
    LIST.get().and_then(|l| l.iter().find(|f| f.path == path)).map(|f| f.name.clone()).unwrap_or_else(|| PathBuf::from(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_system_fonts_with_names() {
        let l = super::scan();
        println!("{} fonts, e.g. {:?}", l.len(), l.iter().take(5).map(|f| &f.name).collect::<Vec<_>>());
        if cfg!(windows) {
            assert!(l.iter().any(|f| f.name.starts_with("Arial") || f.name.starts_with("Segoe")));
            let f = l.iter().find(|f| f.name == "Arial").or(l.first()).unwrap();
            assert!(super::load(&f.path).is_some());
        }
    }
}

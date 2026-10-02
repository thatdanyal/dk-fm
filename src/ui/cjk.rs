//! Japanese, Korean and Chinese text. Fonts for these alphabets are 5–20 MB, so none is bundled.
//! When the library, lyrics, an import or typed text contains one, a font that's already on the
//! computer is memory-mapped (file-backed, so it barely counts toward DK.FM's RAM) and added to
//! the fallback chain. Nothing happens for libraries without these alphabets.
use super::theme;
use eframe::egui::{self, FontData};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Script {
    Kana,
    Hangul,
    Han,
}

const ALL: [Script; 3] = [Script::Kana, Script::Hangul, Script::Han];

impl Script {
    fn of(c: char) -> Option<Script> {
        match c as u32 {
            0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => Some(Script::Kana),
            0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Some(Script::Hangul),
            0x3000..=0x303F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFF65 | 0x20000..=0x2FFFF => Some(Script::Han),
            _ => None,
        }
    }
    /// A character the font must have to count as covering this script.
    fn sample(self) -> char {
        match self {
            Script::Kana => 'あ',
            Script::Hangul => '한',
            Script::Han => '中',
        }
    }
}

struct State {
    /// scripts already handled (a font was found, or none exists on this computer)
    done: Vec<Script>,
    /// fonts loaded so far, as (file, collection index, bytes)
    loaded: Vec<(PathBuf, u32, &'static [u8])>,
}

static STATE: Mutex<State> = Mutex::new(State { done: Vec::new(), loaded: Vec::new() });

/// Makes sure every Japanese/Korean/Chinese character in `text` can be drawn. Cheap when there's
/// nothing new to do; safe to call from any thread.
pub fn ensure(ctx: &egui::Context, text: &str) {
    let mut st = STATE.lock();
    if st.done.len() == ALL.len() {
        return;
    }
    let mut need: Vec<Script> = Vec::new();
    for c in text.chars() {
        if let Some(s) = Script::of(c) {
            if !st.done.contains(&s) && !need.contains(&s) {
                need.push(s);
                if st.done.len() + need.len() == ALL.len() {
                    break;
                }
            }
        }
    }
    if need.is_empty() {
        return;
    }
    let mut added = false;
    for s in need {
        st.done.push(s);
        // a font loaded for another script may already cover this one (CJK fonts often do)
        if st.loaded.iter().any(|(_, i, b)| covers(b, *i, s.sample())) {
            continue;
        }
        for path in candidates(s) {
            if st.loaded.iter().any(|(p, _, _)| *p == path) {
                continue;
            }
            let Some(bytes) = map(&path) else { continue };
            let Some(index) = (0..font_count(bytes)).find(|&i| covers(bytes, i, s.sample())) else { continue };
            st.loaded.push((path, index, bytes));
            added = true;
            break;
        }
    }
    if added {
        let mut defs = theme::font_definitions();
        let names: Vec<String> = st
            .loaded
            .iter()
            .enumerate()
            .map(|(n, (_, index, bytes))| {
                let name = format!("cjk{n}");
                let mut fd = FontData::from_static(bytes);
                fd.index = *index;
                defs.font_data.insert(name.clone(), Arc::new(fd));
                name
            })
            .collect();
        for fam in defs.families.values_mut() {
            fam.extend(names.iter().cloned());
        }
        ctx.set_fonts(defs);
        ctx.request_repaint();
    }
}

fn covers(bytes: &[u8], index: u32, c: char) -> bool {
    use ab_glyph::Font;
    ab_glyph::FontRef::try_from_slice_and_index(bytes, index).map(|f| f.glyph_id(c).0 != 0).unwrap_or(false)
}

/// Number of fonts in a file (.ttc collections hold several).
fn font_count(bytes: &[u8]) -> u32 {
    if bytes.starts_with(b"ttcf") && bytes.len() >= 12 {
        u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]).min(16)
    } else {
        1
    }
}

/// Maps the file for the rest of the run. The mapping is file-backed: only the glyphs actually
/// drawn are read in, and the OS can drop them again under memory pressure.
fn map(path: &PathBuf) -> Option<&'static [u8]> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: system font files aren't modified while programs use them.
    let mmap = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    let mmap: &'static memmap2::Mmap = Box::leak(Box::new(mmap));
    Some(&mmap[..])
}

fn candidates(s: Script) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let dir = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts");
        let names: &[&str] = match s {
            Script::Kana => &["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"],
            Script::Hangul => &["malgun.ttf", "gulim.ttc"],
            Script::Han => &["msyh.ttc", "msyh.ttf", "YuGothM.ttc", "meiryo.ttc", "simsun.ttc", "msjh.ttc"],
        };
        names.iter().map(|n| dir.join(n)).filter(|p| p.exists()).collect()
    }
    #[cfg(target_os = "macos")]
    {
        let names: &[&str] = match s {
            Script::Kana => &["/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "/System/Library/Fonts/Hiragino Sans GB.ttc", "/Library/Fonts/Arial Unicode.ttf"],
            Script::Hangul => &["/System/Library/Fonts/AppleSDGothicNeo.ttc", "/Library/Fonts/Arial Unicode.ttf"],
            Script::Han => &["/System/Library/Fonts/Hiragino Sans GB.ttc", "/System/Library/Fonts/STHeiti Medium.ttc", "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "/Library/Fonts/Arial Unicode.ttf"],
        };
        names.iter().map(PathBuf::from).filter(|p| p.exists()).collect()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // ask fontconfig first, then try the usual Noto CJK / Droid locations
        let lang = match s {
            Script::Kana => "ja",
            Script::Hangul => "ko",
            Script::Han => "zh-cn",
        };
        let mut v: Vec<PathBuf> = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", &format!(":lang={lang}")])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .into_iter()
            .collect();
        for p in [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        ] {
            v.push(PathBuf::from(p));
        }
        v.retain(|p| p.exists());
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_scripts() {
        assert_eq!(Script::of('あ'), Some(Script::Kana));
        assert_eq!(Script::of('한'), Some(Script::Hangul));
        assert_eq!(Script::of('中'), Some(Script::Han));
        assert_eq!(Script::of('é'), None);
    }

    #[test]
    fn finds_a_system_font_for_each_script() {
        for s in ALL {
            let found = candidates(s).into_iter().any(|p| map(&p).map(|b| (0..font_count(b)).any(|i| covers(b, i, s.sample()))).unwrap_or(false));
            println!("{s:?}: {found}");
        }
    }
}

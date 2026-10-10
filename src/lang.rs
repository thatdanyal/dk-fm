//! Your language (asked on a new install, Settings > Language) and lyrics translated into it.
//! Lyrics in another language get a translation under each line (or instead of it). Google
//! Translate's free web endpoint does it in one request per song, and the answer is saved next to
//! the song's lyrics, so each song is translated once. Lines that come back the same (names,
//! "oh oh oh", a line in your language) get no translation under them.
use crate::store::data_dir;
use serde::{Deserialize, Serialize};

/// Languages to pick from: (code, name). Only alphabets DK.FM can draw (Latin, Greek, Cyrillic,
/// Japanese, Korean, Chinese).
pub const LANGS: &[(&str, &str)] = &[
    ("en", "English"),
    ("es", "Spanish (Español)"),
    ("fr", "French (Français)"),
    ("de", "German (Deutsch)"),
    ("it", "Italian (Italiano)"),
    ("pt", "Portuguese (Português)"),
    ("nl", "Dutch (Nederlands)"),
    ("sv", "Swedish (Svenska)"),
    ("no", "Norwegian (Norsk)"),
    ("da", "Danish (Dansk)"),
    ("fi", "Finnish (Suomi)"),
    ("pl", "Polish (Polski)"),
    ("cs", "Czech (Čeština)"),
    ("ro", "Romanian (Română)"),
    ("hu", "Hungarian (Magyar)"),
    ("tr", "Turkish (Türkçe)"),
    ("el", "Greek (Ελληνικά)"),
    ("ru", "Russian (Русский)"),
    ("uk", "Ukrainian (Українська)"),
    ("id", "Indonesian"),
    ("ms", "Malay"),
    ("tl", "Filipino"),
    ("vi", "Vietnamese (Tiếng Việt)"),
    ("sw", "Swahili"),
    ("ja", "Japanese (日本語)"),
    ("ko", "Korean (한국어)"),
    ("zh-CN", "Chinese, Simplified (简体中文)"),
    ("zh-TW", "Chinese, Traditional (繁體中文)"),
];

pub fn name(code: &str) -> &'static str {
    LANGS.iter().find(|l| l.0 == code).map(|l| l.1).unwrap_or("English")
}

/// A language's English name ("es" -> "SPANISH"), for "TRANSLATED FROM …".
pub fn short_name(code: &str) -> String {
    match LANGS.iter().find(|l| same(l.0, code)) {
        Some(l) => l.1.split(" (").next().unwrap_or(l.1).to_uppercase(),
        None => code.to_uppercase(),
    }
}

/// The computer's language, if it's one of `LANGS` (else English).
pub fn system() -> String {
    static SYS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SYS.get_or_init(|| pick(&sys_locale::get_locale().unwrap_or_default().replace('_', "-")).to_string()).clone()
}

/// A locale ("pt-BR", "zh-Hant-TW", "en-US") as one of `LANGS`.
fn pick(loc: &str) -> &'static str {
    let low = loc.to_lowercase();
    if low.starts_with("zh") {
        return if low.contains("tw") || low.contains("hk") || low.contains("hant") || low.contains("mo") { "zh-TW" } else { "zh-CN" };
    }
    let base = low.split('-').next().unwrap_or("");
    let base = match base {
        "nb" | "nn" => "no",
        "fil" => "tl",
        b => b,
    };
    LANGS.iter().find(|l| l.0 == base).map(|l| l.0).unwrap_or("en")
}

/// Do two language codes name the same language ("zh-CN" / "zh" count as the same)?
fn same(a: &str, b: &str) -> bool {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    a == b || a.split('-').next() == b.split('-').next() && !a.starts_with("zh")
}

/// A song's lyrics in your language: one entry per line (None: that line needs none).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Translation {
    /// the language the lyrics are in ("" unknown)
    pub from: String,
    pub lines: Vec<Option<String>>,
}

impl Translation {
    /// Nothing to show: the lyrics are in your language already.
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.is_none())
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    /// the lyrics it was made for (lyrics can be looked up again)
    key: u64,
    t: Translation,
}

fn cache_path(id: &str, to: &str) -> std::path::PathBuf {
    data_dir().join("lyrics").join(format!("{id}.{to}.tr.json"))
}

fn key_of(lines: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    lines.hash(&mut h);
    h.finish()
}

/// `lines` (a song's lyrics) in language `to` (blocking: call from a background thread).
pub fn translate(id: &str, lines: &[String], to: &str) -> Result<Translation, String> {
    let key = key_of(lines);
    let path = cache_path(id, to);
    if let Some(s) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Saved>(&b).ok()).filter(|s| s.key == key) {
        return Ok(s.t);
    }
    let t = fetch(lines, to)?;
    crate::store::save_json(&path, &Saved { key, t: t.clone() });
    Ok(t)
}

fn fetch(lines: &[String], to: &str) -> Result<Translation, String> {
    // only lines with words in them are sent (blank lines and ♪ stay as they are)
    let idx: Vec<usize> = (0..lines.len()).filter(|i| lines[*i].chars().any(|c| c.is_alphabetic())).collect();
    let mut out = Translation { from: String::new(), lines: vec![None; lines.len()] };
    if idx.is_empty() {
        return Ok(out);
    }
    // a few thousand characters per request
    let mut chunks: Vec<Vec<usize>> = vec![Vec::new()];
    let mut size = 0;
    for i in idx {
        let n = lines[i].len() + 1;
        if size + n > 4000 && !chunks.last().unwrap().is_empty() {
            chunks.push(Vec::new());
            size = 0;
        }
        size += n;
        chunks.last_mut().unwrap().push(i);
    }
    for chunk in chunks {
        let q: String = chunk.iter().map(|i| lines[*i].replace('\n', " ")).collect::<Vec<_>>().join("\n");
        let url = format!("https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl={to}&dt=t");
        let v = crate::net::post_form_json(&url, &[], &[("q", &q)])?;
        if out.from.is_empty() {
            out.from = v.get(2).and_then(|x| x.as_str()).unwrap_or("").to_string();
        }
        let text: String = v.get(0).and_then(|x| x.as_array()).map(|segs| segs.iter().filter_map(|s| s.get(0).and_then(|x| x.as_str())).collect()).unwrap_or_default();
        let got: Vec<&str> = text.split('\n').collect();
        if got.len() != chunk.len() {
            return Err("The translation came back in a different shape".into());
        }
        for (i, tr) in chunk.iter().zip(got) {
            let tr = tr.trim();
            // a line in your language already (or a name, "oh oh oh"): nothing under it
            if !tr.is_empty() && simplify(tr) != simplify(&lines[*i]) {
                out.lines[*i] = Some(tr.to_string());
            }
        }
    }
    if same(&out.from, to) {
        // your language already (a translation would only reword slang)
        out.lines = vec![None; lines.len()];
    }
    Ok(out)
}

fn simplify(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_map_to_languages() {
        assert_eq!(pick("en-US"), "en");
        assert_eq!(pick("pt-BR"), "pt");
        assert_eq!(pick("zh-Hant-TW"), "zh-TW");
        assert_eq!(pick("zh-CN"), "zh-CN");
        assert_eq!(pick("nb-NO"), "no");
        assert_eq!(pick("xx"), "en");
        assert!(same("en", "en-GB") && !same("zh-CN", "zh-TW") && !same("es", "en"));
    }
}

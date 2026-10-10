//! Finding lyrics for a song: what was found before (saved in the data folder), else lyrics that
//! light up line by line if there are any — synced lyrics in the file's tags, LRCLIB's synced
//! lyrics, then the captions of the song's video on YouTube (its own video first: the same audio,
//! so the timing matches exactly; a cover or a small artist's song usually only has those) —
//! and only then plain lyrics. What's found is saved, so each song is looked up once; "nothing
//! found" is remembered for a week.
use crate::store::{data_dir, now_secs, Track};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq)]
pub enum Lyr {
    Loading,
    None,
    Instrumental,
    Plain(String),
    Synced(Vec<(f64, String)>),
}

/// Where lyrics came from (shown under them).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum Source {
    #[default]
    Lrclib,
    File,
    YouTube,
    /// YouTube's automatic (speech-recognised) captions: often rough
    YouTubeAuto,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Lrclib => "LYRICS · LRCLIB",
            Source::File => "LYRICS · FROM THE FILE",
            Source::YouTube => "LYRICS · YOUTUBE CAPTIONS",
            Source::YouTubeAuto => "LYRICS · YOUTUBE AUTO-CAPTIONS (MAY BE ROUGH)",
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    /// "synced" | "plain" | "instrumental" | "none"
    kind: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    lines: Vec<(f64, String)>,
    #[serde(default)]
    source: Source,
    #[serde(default)]
    at: i64,
    /// 2: looked for synced lyrics everywhere before settling for plain ones
    #[serde(default)]
    v: u32,
}

fn cache_path(id: &str) -> std::path::PathBuf {
    data_dir().join("lyrics").join(format!("{id}.json"))
}

fn load(id: &str) -> Option<(Lyr, Source)> {
    let s: Saved = serde_json::from_slice(&std::fs::read(cache_path(id)).ok()?).ok()?;
    let l = match s.kind.as_str() {
        "synced" if !s.lines.is_empty() => Lyr::Synced(s.lines),
        // (plain lyrics saved before captions were tried for them: look again, once)
        "plain" if !s.text.is_empty() && s.v >= 2 => Lyr::Plain(s.text),
        "instrumental" => Lyr::Instrumental,
        // looked for lately and found nothing: try again after a week
        "none" if now_secs() - s.at < 7 * 86400 => Lyr::None,
        _ => return None,
    };
    Some((l, s.source))
}

fn save(id: &str, l: &Lyr, source: Source) {
    let mut s = Saved { source, at: now_secs(), v: 2, ..Default::default() };
    match l {
        Lyr::Synced(v) => (s.kind, s.lines) = ("synced".into(), v.clone()),
        Lyr::Plain(t) => (s.kind, s.text) = ("plain".into(), t.clone()),
        Lyr::Instrumental => s.kind = "instrumental".into(),
        Lyr::None => s.kind = "none".into(),
        Lyr::Loading => return,
    }
    crate::store::save_json(&cache_path(id), &s);
}

/// Every song's saved lyrics as (song id, lines): synced lines with their time, plain lyrics
/// line by line with no time (-1). For searching your library by a line you remember.
pub fn all_saved() -> Vec<(String, Vec<(f64, String)>)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(data_dir().join("lyrics")).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // (translations are saved beside them as <id>.<language>.tr.json)
        let Some(id) = name.strip_suffix(".json").filter(|n| !n.contains('.')) else { continue };
        let Some(s) = std::fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Saved>(&b).ok()) else { continue };
        let lines = match s.kind.as_str() {
            "synced" => s.lines,
            "plain" => s.text.lines().map(|l| (-1.0, l.to_string())).collect(),
            _ => continue,
        };
        out.push((id.to_string(), lines));
    }
    out
}

/// Look a song's lyrics up the light way (its file's tags, then LRCLIB; never YouTube captions,
/// which need yt-dlp) and save them. For filling in a whole library: true if it has lyrics now.
/// Nothing found isn't remembered, so playing it still tries everything.
pub fn find_light(t: &Track) -> bool {
    if let Some((l, _)) = load(&t.id) {
        return !matches!(l, Lyr::None);
    }
    let found = match from_file(t) {
        Some(l @ Lyr::Synced(_)) => Some((l, Source::File)),
        file => lrclib(t).map(|l| (l, Source::Lrclib)).or(file.map(|l| (l, Source::File))),
    };
    match found {
        Some((l, src)) => {
            save(&t.id, &l, src);
            !matches!(l, Lyr::Instrumental)
        }
        None => false,
    }
}

/// Forget what was found for a song (to look again).
pub fn forget(id: &str) {
    let _ = std::fs::remove_file(cache_path(id));
}

static RE_TIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[(\d+):(\d+(?:\.\d+)?)\]").unwrap());
static RE_TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]").unwrap());
static RE_FEAT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[\(\[]\s*(feat|ft|with)[^\)\]]*[\)\]]").unwrap());

pub fn parse_lrc(src: &str) -> Vec<(f64, String)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let text = RE_TAGS.replace_all(line, "").trim().to_string();
        for c in RE_TIME.captures_iter(line) {
            let t = c[1].parse::<f64>().unwrap_or(0.0) * 60.0 + c[2].parse::<f64>().unwrap_or(0.0);
            out.push((t, text.clone()));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// Look everything up (blocking: call from a background thread).
pub fn find(t: &Track) -> (Lyr, Source) {
    if let Some(hit) = load(&t.id) {
        return hit;
    }
    let (l, src) = look(t);
    save(&t.id, &l, src);
    (l, src)
}

/// Synced lyrics from anywhere beat plain ones (from the file or LRCLIB).
fn look(t: &Track) -> (Lyr, Source) {
    let file = from_file(t);
    if matches!(file, Some(Lyr::Synced(_))) {
        return (file.unwrap(), Source::File);
    }
    let lrc = lrclib(t);
    if matches!(lrc, Some(Lyr::Synced(_) | Lyr::Instrumental)) {
        return (lrc.unwrap(), Source::Lrclib);
    }
    if let Some(yt) = youtube(t) {
        return yt;
    }
    match (lrc, file) {
        (Some(l), _) => (l, Source::Lrclib),
        (None, Some(l)) => (l, Source::File),
        (None, None) => (Lyr::None, Source::Lrclib),
    }
}

/// Lyrics saved in the file's tags (plain, or LRC-style synced).
fn from_file(t: &Track) -> Option<Lyr> {
    use lofty::file::TaggedFileExt;
    let f = std::panic::catch_unwind(|| lofty::read_from_path(&t.path)).ok()?.ok()?;
    let text = f.tags().iter().find_map(|tag| tag.get_string(&lofty::tag::ItemKey::Lyrics).map(String::from)).filter(|s| !s.trim().is_empty())?;
    let synced = parse_lrc(&text);
    Some(if synced.len() >= 4 { Lyr::Synced(synced) } else { Lyr::Plain(text.trim().to_string()) })
}

fn lrclib(t: &Track) -> Option<Lyr> {
    let title = RE_FEAT.replace_all(&t.title, "").to_string();
    let v = crate::net::lyrics(&crate::library::main_artist(&t.artist), &title, &t.album, t.duration)?;
    if v["instrumental"].as_bool() == Some(true) {
        return Some(Lyr::Instrumental);
    }
    if let Some(s) = v["syncedLyrics"].as_str().filter(|s| !s.trim().is_empty()) {
        return Some(Lyr::Synced(parse_lrc(s)));
    }
    v["plainLyrics"].as_str().filter(|s| !s.trim().is_empty()).map(|s| Lyr::Plain(s.to_string()))
}

// ------------------------------------------------------------------------------- YouTube captions

/// Captions of the song's YouTube video: its own (downloaded from YouTube: exactly the same
/// audio, so in time) first, else the top search results with about the right length. Uploaded
/// captions win over automatic ones, except that the song's own automatic captions win over
/// another video's (which may be another recording, with other timing).
fn youtube(t: &Track) -> Option<(Lyr, Source)> {
    crate::ytdlp::ensure().ok()?;
    if let Some((lines, uploaded)) = t.youtube_id.as_deref().and_then(captions) {
        return Some((Lyr::Synced(lines), if uploaded { Source::YouTube } else { Source::YouTubeAuto }));
    }
    let mut ids: Vec<String> = t.youtube_id.iter().cloned().collect();
    let q = format!("{} - {}", crate::library::main_artist(&t.artist), RE_FEAT.replace_all(&t.title, ""));
    if let Ok(found) = crate::sources::search(&q, "youtube", 5) {
        for f in found {
            let secs = f.duration_ms.map(|d| d as f64 / 1000.0);
            let close = match (secs, t.duration > 0.0) {
                (Some(s), true) => (s - t.duration).abs() <= 25.0,
                _ => true,
            };
            if let Some(id) = f.youtube_id.filter(|i| close && !ids.contains(i)) {
                ids.push(id);
            }
        }
    }
    let mut auto: Option<Vec<(f64, String)>> = None;
    for id in ids.iter().filter(|i| Some(*i) != t.youtube_id.as_ref()).take(4) {
        let Some((lines, uploaded)) = captions(id) else { continue };
        if uploaded {
            return Some((Lyr::Synced(lines), Source::YouTube));
        }
        auto.get_or_insert(lines);
    }
    auto.map(|l| (Lyr::Synced(l), Source::YouTubeAuto))
}

/// A video's captions as timed lines, and whether they were uploaded (not automatic).
fn captions(video: &str) -> Option<(Vec<(f64, String)>, bool)> {
    let raw = crate::ytdlp::run_with(&["-J".into(), "--skip-download".into(), "--no-playlist".into(), format!("https://www.youtube.com/watch?v={video}")], None, None).ok()?;
    let j: Value = serde_json::from_str(&raw).ok()?;
    let lang = j["language"].as_str().unwrap_or("en").to_string();
    // uploaded captions: the video's language, then English, then any
    if let Some(subs) = j["subtitles"].as_object() {
        let mut keys: Vec<&String> = subs.keys().filter(|k| !k.starts_with("live_chat")).collect();
        keys.sort_by_key(|k| (!k.starts_with(&lang), !k.starts_with("en"), k.to_string()));
        for k in keys {
            if let Some(lines) = fetch_track(&subs[k.as_str()]) {
                return Some((lines, true));
            }
        }
    }
    let auto = j["automatic_captions"].as_object()?;
    // the original-language track (not a machine translation)
    let key = [format!("{lang}-orig"), lang.clone(), "en-orig".into(), "en".into()].into_iter().find(|k| auto.contains_key(k))?;
    fetch_track(&auto[key.as_str()]).map(|l| (l, false))
}

/// One caption track (a list of formats): json3 preferred, else WebVTT.
fn fetch_track(formats: &Value) -> Option<Vec<(f64, String)>> {
    let list = formats.as_array()?;
    let pick = |ext: &str| list.iter().find(|f| f["ext"].as_str() == Some(ext)).and_then(|f| f["url"].as_str());
    let lines = if let Some(u) = pick("json3") {
        parse_json3(&serde_json::from_str(&crate::net::get_text(u, &[("User-Agent", crate::net::UA)]).ok()?).ok()?)
    } else {
        parse_vtt(&crate::net::get_text(pick("vtt")?, &[("User-Agent", crate::net::UA)]).ok()?)
    };
    // a handful of lines is a title card or a caption saying "[Music]", not lyrics
    (lines.iter().filter(|l| !l.1.is_empty()).count() >= 6).then_some(lines)
}

/// "[Music]", "♪ la la ♪" -> "la la"; sound labels alone become a ♪ line (empty).
fn clean_caption(s: &str) -> String {
    let s = RE_TAGS.replace_all(s, " ");
    let s = s.replace(['♪', '♫', '\n'], " ");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Merge a run of timed lines: no empty duplicates, no line repeated straight after itself.
fn tidy(raw: Vec<(f64, String)>) -> Vec<(f64, String)> {
    let mut out: Vec<(f64, String)> = Vec::new();
    for (t, text) in raw {
        match out.last() {
            Some(last) if last.1 == text => continue,
            Some(last) if text.is_empty() && last.1.is_empty() => continue,
            _ => out.push((t, text)),
        }
    }
    while out.first().map(|l| l.1.is_empty()).unwrap_or(false) {
        out.remove(0);
    }
    out
}

pub fn parse_json3(v: &Value) -> Vec<(f64, String)> {
    let mut raw = Vec::new();
    for e in v["events"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let Some(segs) = e["segs"].as_array() else { continue };
        let text: String = segs.iter().filter_map(|s| s["utf8"].as_str()).collect();
        if text.trim().is_empty() {
            continue; // line breaks between auto-caption lines
        }
        let t = e["tStartMs"].as_f64().unwrap_or(0.0) / 1000.0;
        raw.push((t, clean_caption(&text)));
    }
    tidy(raw)
}

static RE_CUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:(\d+):)?(\d{1,2}):(\d{2})[.,](\d{3})\s+-->").unwrap());
static RE_INLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());

pub fn parse_vtt(src: &str) -> Vec<(f64, String)> {
    let mut raw = Vec::new();
    let mut lines = src.lines().peekable();
    while let Some(l) = lines.next() {
        let Some(c) = RE_CUE.captures(l.trim()) else { continue };
        let n = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<f64>().ok()).unwrap_or(0.0);
        let t = n(1) * 3600.0 + n(2) * 60.0 + n(3) + n(4) / 1000.0;
        let mut text = Vec::new();
        while let Some(x) = lines.peek() {
            if x.trim().is_empty() {
                break;
            }
            text.push(RE_INLINE.replace_all(x, "").to_string());
            lines.next();
        }
        // automatic captions repeat the previous line above the new one: keep the last line
        let last = text.last().cloned().unwrap_or_default();
        raw.push((t, clean_caption(&last)));
    }
    tidy(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_lines() {
        let v = parse_lrc("[ar:x]\n[00:12.50]Hello\n[00:05.00][01:00.00]Again\n");
        assert_eq!(v, vec![(5.0, "Again".into()), (12.5, "Hello".into()), (60.0, "Again".into())]);
    }

    #[test]
    fn json3_captions() {
        let v: Value = serde_json::from_str(r#"{"events":[
            {"tStartMs":0,"segs":[{"utf8":"[Music]"}]},
            {"tStartMs":1200,"segs":[{"utf8":"\n"}]},
            {"tStartMs":2000,"segs":[{"utf8":"♪ Hello"},{"utf8":" darkness"}]},
            {"tStartMs":2500,"segs":[{"utf8":"♪ Hello darkness"}]},
            {"tStartMs":4000,"segs":[{"utf8":"my old friend"}]},
            {"tStartMs":6000,"segs":[{"utf8":"[Music]"}]},
            {"tStartMs":7000,"segs":[{"utf8":"[Applause]"}]}
        ]}"#).unwrap();
        assert_eq!(parse_json3(&v), vec![(2.0, "Hello darkness".into()), (4.0, "my old friend".into()), (6.0, String::new())]);
    }

    #[test]
    fn vtt_captions() {
        let src = "WEBVTT\nKind: captions\n\n00:00:01.000 --> 00:00:03.000\n<c>first</c> line\n\n00:00:03.000 --> 00:00:05.000\nfirst line\nsecond<00:00:04.000><c> line</c>\n\n1:02:03.500 --> 1:02:04.000\nlate\n";
        assert_eq!(parse_vtt(src), vec![(1.0, "first line".into()), (3.0, "second line".into()), (3723.5, "late".into())]);
    }

    #[test]
    fn saved_lyrics_come_back() {
        let (_g, _dir) = crate::store::test_profile("lyrics");
        let l = Lyr::Synced(vec![(1.0, "a".into()), (2.0, "b".into())]);
        save("t1", &l, Source::YouTube);
        assert_eq!(load("t1"), Some((l, Source::YouTube)));
        save("t2", &Lyr::None, Source::Lrclib);
        assert_eq!(load("t2").map(|x| x.0), Some(Lyr::None));
        forget("t1");
        assert!(load("t1").is_none());
        // plain lyrics saved before synced ones were looked for everywhere: looked up again
        save("t3", &Lyr::Plain("words".into()), Source::Lrclib);
        assert_eq!(load("t3").map(|x| x.0), Some(Lyr::Plain("words".into())));
        crate::store::save_json(&cache_path("t4"), &serde_json::json!({ "kind": "plain", "text": "old", "at": now_secs() }));
        assert!(load("t4").is_none());
    }
}

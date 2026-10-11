//! Podcasts and audiobooks: shows you follow (found on Apple Podcasts' free search, or any RSS
//! feed link), their episodes, and where you stopped in each. An episode is downloaded when you
//! play it (into the data folder, not your music: it never shows up as a song) and plays at the
//! podcast speed (1×–2×, voices keep their pitch). A finished episode's file is deleted.
//! Saved in podcasts.json.
use crate::store::{data_dir, now_secs};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Show {
    pub id: String,
    pub feed: String,
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub episodes: Vec<Episode>,
    /// when the feed was last read (unix s)
    #[serde(default)]
    pub refreshed: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Episode {
    pub id: String,
    pub title: String,
    pub url: String,
    /// published (unix s)
    #[serde(default)]
    pub date: i64,
    /// seconds (0 = unknown)
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub about: String,
}

/// Where you are in an episode.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Progress {
    #[serde(default)]
    pub pos: f64,
    #[serde(default)]
    pub done: bool,
    /// last listened (unix s)
    #[serde(default)]
    pub at: i64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Podcasts {
    #[serde(default)]
    pub shows: Vec<Show>,
    #[serde(default)]
    pub progress: HashMap<String, Progress>,
    #[serde(default = "one")]
    pub speed: f32,
    /// seconds listened per year (Year in review)
    #[serde(default)]
    pub listened: HashMap<i32, f64>,
}

fn one() -> f32 {
    1.0
}

fn path() -> std::path::PathBuf {
    data_dir().join("podcasts.json")
}

pub fn files_dir() -> std::path::PathBuf {
    data_dir().join("podcasts")
}

impl Podcasts {
    pub fn load() -> Self {
        let mut p: Self = std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if p.speed <= 0.0 {
            p.speed = 1.0;
        }
        p
    }
    pub fn save(&self) {
        crate::store::save_json(&path(), self);
    }
    pub fn episode(&self, id: &str) -> Option<(&Show, &Episode)> {
        self.shows.iter().find_map(|s| s.episodes.iter().find(|e| e.id == id).map(|e| (s, e)))
    }
    /// Episodes started and not finished, last listened first.
    pub fn in_progress(&self) -> Vec<(&Show, &Episode)> {
        let mut v: Vec<(&Show, &Episode, i64)> = self
            .shows
            .iter()
            .flat_map(|s| s.episodes.iter().map(move |e| (s, e)))
            .filter_map(|(s, e)| self.progress.get(&e.id).filter(|p| !p.done && p.pos > 5.0).map(|p| (s, e, p.at)))
            .collect();
        v.sort_by_key(|x| -x.2);
        v.into_iter().map(|x| (x.0, x.1)).collect()
    }
}

/// An episode's file (downloaded when it's played).
pub fn file_of(e: &Episode) -> std::path::PathBuf {
    let ext = e.url.split('?').next().and_then(|u| u.rsplit('.').next()).filter(|x| matches!(x.to_lowercase().as_str(), "mp3" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "flac")).unwrap_or("mp3").to_lowercase();
    files_dir().join(format!("{}.{ext}", e.id))
}

fn short_hash(s: &str) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// A show found by Apple Podcasts' search: (title, author, feed, cover).
#[derive(Clone, Debug)]
pub struct Found {
    pub title: String,
    pub author: String,
    pub feed: String,
    pub cover: Option<String>,
}

pub fn search(q: &str) -> Result<Vec<Found>, String> {
    let v = crate::net::get_json(&format!("https://itunes.apple.com/search?media=podcast&limit=25&term={}", crate::net::enc(q)), &[])?;
    Ok(v["results"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| {
            Some(Found {
                title: x["collectionName"].as_str()?.to_string(),
                author: x["artistName"].as_str().unwrap_or("").to_string(),
                feed: x["feedUrl"].as_str()?.to_string(),
                cover: x["artworkUrl600"].as_str().or(x["artworkUrl100"].as_str()).map(String::from),
            })
        })
        .collect())
}

static RE_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<item[\s>].*?</item>").unwrap());
static RE_CDATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!\[CDATA\[(.*?)\]\]>").unwrap());
static RE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());

/// The text inside the first <tag>…</tag> (CDATA unwrapped, entities decoded, HTML tags gone).
fn tag(src: &str, name: &str) -> Option<String> {
    let re = Regex::new(&format!(r"(?s)<{}(?:\s[^>]*)?>(.*?)</{}>", regex::escape(name), regex::escape(name))).ok()?;
    let raw = re.captures(src)?.get(1)?.as_str();
    let s = RE_CDATA.replace_all(raw, "$1");
    let s = RE_TAG.replace_all(&s, " ");
    let s = entities(&s);
    // (show notes are often HTML escaped inside the XML: its tags appear once decoded)
    let s = RE_TAG.replace_all(&s, " ");
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    (!s.is_empty()).then_some(s)
}

/// An attribute of the first <tag …>.
fn attr(src: &str, name: &str, at: &str) -> Option<String> {
    let re = Regex::new(&format!(r#"(?s)<{}\s[^>]*?\b{}\s*=\s*["']([^"']*)["']"#, regex::escape(name), regex::escape(at))).ok()?;
    Some(entities(re.captures(src)?.get(1)?.as_str()))
}

fn entities(s: &str) -> String {
    let s = s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&nbsp;", " ").replace("&#8217;", "’").replace("&#8216;", "‘").replace("&#8220;", "“").replace("&#8221;", "”").replace("&#8211;", "–").replace("&#8212;", "—");
    s.replace("&amp;", "&")
}

/// "1:02:03", "62:03" or "3723" seconds.
fn duration(s: &str) -> f64 {
    s.trim().split(':').map(|p| p.trim().parse::<f64>().unwrap_or(0.0)).fold(0.0, |acc, x| acc * 60.0 + x)
}

/// A show from its RSS feed (its episodes newest first).
pub fn parse_feed(feed: &str, xml: &str) -> Result<Show, String> {
    let head = xml.split("<item").next().unwrap_or(xml);
    let title = tag(head, "title").ok_or("That link isn't a podcast feed")?;
    let cover = attr(head, "itunes:image", "href").or_else(|| tag(head, "url"));
    let author = tag(head, "itunes:author").unwrap_or_default();
    let mut episodes: Vec<Episode> = RE_ITEM
        .find_iter(xml)
        .filter_map(|m| {
            let it = m.as_str();
            let url = attr(it, "enclosure", "url")?;
            let title = tag(it, "title").unwrap_or_else(|| "Episode".into());
            let guid = tag(it, "guid").unwrap_or_else(|| url.clone());
            let date = tag(it, "pubDate").and_then(|d| chrono::DateTime::parse_from_rfc2822(d.trim()).ok()).map(|d| d.timestamp()).unwrap_or(0);
            let mut about = tag(it, "itunes:summary").or_else(|| tag(it, "description")).unwrap_or_default();
            if about.chars().count() > 400 {
                about = about.chars().take(400).collect::<String>() + "…";
            }
            Some(Episode { id: format!("{}{}", crate::audio::EPISODE, short_hash(&format!("{feed}|{guid}"))), title, url, date, duration: tag(it, "itunes:duration").map(|d| duration(&d)).unwrap_or(0.0), about })
        })
        .collect();
    if episodes.is_empty() && !xml.contains("<item") {
        return Err("That link isn't a podcast feed".into());
    }
    episodes.sort_by_key(|e| -e.date);
    episodes.truncate(300);
    Ok(Show { id: short_hash(feed), feed: feed.to_string(), title, author, cover, episodes, refreshed: now_secs() })
}

/// Read a show's feed (blocking).
pub fn fetch(feed: &str) -> Result<Show, String> {
    let resp = ureq::get(feed).set("User-Agent", crate::net::UA).timeout(std::time::Duration::from_secs(30)).call().map_err(|e| format!("Couldn't read the feed: {e}"))?;
    let mut xml = String::new();
    use std::io::Read;
    resp.into_reader().take(20 * 1024 * 1024).read_to_string(&mut xml).map_err(|e| e.to_string())?;
    parse_feed(feed, &xml)
}

/// Download an episode to its file, reporting progress (0-1). Blocking.
pub fn download(e: &Episode, progress: &dyn Fn(f32)) -> Result<std::path::PathBuf, String> {
    let dest = file_of(e);
    if dest.exists() {
        return Ok(dest);
    }
    std::fs::create_dir_all(files_dir()).map_err(|e| e.to_string())?;
    let resp = ureq::get(&e.url).set("User-Agent", crate::net::UA).call().map_err(|e| format!("Couldn't get the episode: {e}"))?;
    let total: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let tmp = dest.with_extension("part");
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut r = resp.into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    use std::io::{Read, Write};
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        if total > 0 {
            progress(got as f32 / total as f32);
        }
    }
    drop(f);
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_a_feed() {
        let xml = r#"<?xml version="1.0"?><rss xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel>
            <title><![CDATA[Tech &amp; Talk]]></title><itunes:author>Some One</itunes:author>
            <itunes:image href="https://x.test/cover.jpg"/>
            <item><title>Episode 1: Hello</title><guid>ep1</guid><pubDate>Mon, 06 Oct 2025 10:00:00 +0000</pubDate>
              <enclosure url="https://x.test/1.mp3?x=1" length="1" type="audio/mpeg"/><itunes:duration>1:02:03</itunes:duration>
              <description>&lt;p&gt;We talk.&lt;/p&gt;</description></item>
            <item><title>Episode 2</title><guid>ep2</guid><pubDate>Tue, 07 Oct 2025 10:00:00 +0000</pubDate>
              <enclosure url="https://x.test/2.m4a" type="audio/x-m4a"/><itunes:duration>754</itunes:duration></item>
        </channel></rss>"#;
        let s = super::parse_feed("https://x.test/feed", xml).unwrap();
        assert_eq!((s.title.as_str(), s.author.as_str(), s.cover.as_deref()), ("Tech & Talk", "Some One", Some("https://x.test/cover.jpg")));
        assert_eq!(s.episodes.len(), 2);
        assert_eq!(s.episodes[0].title, "Episode 2"); // newest first
        assert_eq!(s.episodes[1].duration, 3723.0);
        assert_eq!(s.episodes[1].about, "We talk.");
        assert!(s.episodes[0].id.starts_with(crate::audio::EPISODE));
        assert!(super::file_of(&s.episodes[1]).to_string_lossy().ends_with(".mp3"));
        assert!(super::file_of(&s.episodes[0]).to_string_lossy().ends_with(".m4a"));
        assert!(super::parse_feed("https://x.test", "<html><title>no</title></html>").is_err());
    }
}

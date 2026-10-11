//! Small blocking HTTP helpers (ureq + rustls): no async runtime, tiny footprint.
use serde_json::Value;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130 Safari/537.36";

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(60)).user_agent(&format!("DK.FM/{}", env!("CARGO_PKG_VERSION"))).build());

fn req(url: &str, headers: &[(&str, &str)]) -> ureq::Request {
    let mut r = AGENT.get(url);
    for (k, v) in headers {
        r = r.set(k, v);
    }
    r
}

fn err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        e => e.to_string(),
    }
}

pub fn get_text(url: &str, headers: &[(&str, &str)]) -> Result<String, String> {
    req(url, headers).call().map_err(err)?.into_string().map_err(|e| e.to_string())
}

pub fn get_json(url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    req(url, headers).call().map_err(err)?.into_json::<Value>().map_err(|e| e.to_string())
}

/// GET returning (status, json) without treating 4xx as an error (Spotify rate limits etc).
pub fn get_json_status(url: &str, headers: &[(&str, &str)]) -> Result<(u16, Option<String>, Value), String> {
    match req(url, headers).call() {
        Ok(r) => Ok((r.status(), r.header("retry-after").map(String::from), r.into_json().unwrap_or(Value::Null))),
        Err(ureq::Error::Status(code, r)) => Ok((code, r.header("retry-after").map(String::from), r.into_json().unwrap_or(Value::Null))),
        Err(e) => Err(e.to_string()),
    }
}

pub fn post_form_json(url: &str, headers: &[(&str, &str)], form: &[(&str, &str)]) -> Result<Value, String> {
    let mut r = AGENT.post(url);
    for (k, v) in headers {
        r = r.set(k, v);
    }
    r.send_form(form).map_err(err)?.into_json::<Value>().map_err(|e| e.to_string())
}

pub fn post_json(url: &str, headers: &[(&str, &str)], body: &Value) -> Result<Value, String> {
    let mut r = AGENT.post(url);
    for (k, v) in headers {
        r = r.set(k, v);
    }
    r.send_json(body).map_err(err)?.into_json::<Value>().map_err(|e| e.to_string())
}

pub fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut v = Vec::new();
    req(url, &[]).call().map_err(err)?.into_reader().take(50 * 1024 * 1024).read_to_end(&mut v).map_err(|e| e.to_string())?;
    Ok(v)
}

/// Download to a file (atomically), optionally gunzipping on the fly.
pub fn download(url: &str, dest: &Path, gunzip: bool) -> Result<(), String> {
    let resp = req(url, &[]).call().map_err(|e| format!("download failed: {}", err(e)))?;
    let tmp = dest.with_extension("part");
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut reader: Box<dyn Read> = Box::new(resp.into_reader());
    if gunzip {
        reader = Box::new(flate2::read::GzDecoder::new(reader));
    }
    std::io::copy(&mut reader, &mut f).map_err(|e| e.to_string())?;
    f.flush().map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

pub fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

/// Synced/plain lyrics from LRCLIB (free, no key).
pub fn lyrics(artist: &str, title: &str, album: &str, duration: f64) -> Option<Value> {
    let ua = [("User-Agent", "DK.FM (https://github.com/thatdanyal/dk-fm)")];
    let mut url = format!("https://lrclib.net/api/get?artist_name={}&track_name={}", enc(artist), enc(title));
    if !album.is_empty() {
        url += &format!("&album_name={}", enc(album));
    }
    if duration > 0.0 {
        url += &format!("&duration={}", duration.round() as i64);
    }
    if let Ok(v) = get_json(&url, &ua) {
        return Some(v);
    }
    let list = get_json(&format!("https://lrclib.net/api/search?artist_name={}&track_name={}", enc(artist), enc(title)), &ua).ok()?;
    let arr = list.as_array()?;
    // the same recording (about the same length: a live, extended or sped-up one is timed
    // differently), synced first
    let off = |x: &&Value| if duration > 0.0 { x["duration"].as_f64().map(|d| (d - duration).abs()).unwrap_or(99.0) } else { 0.0 };
    let synced = |x: &&Value| x.get("syncedLyrics").map(|s| s.is_string()).unwrap_or(false);
    let same: Vec<&Value> = arr.iter().filter(|x| off(x) <= 3.0).collect();
    match same.iter().filter(|x| synced(x)).min_by(|a, b| off(a).total_cmp(&off(b))) {
        Some(x) => Some((*x).clone()),
        // nothing that long with synced lyrics: plain lyrics (they have no timing to be wrong)
        None => same.first().copied().or(arr.first()).map(|x| {
            let mut x = x.clone();
            if off(&&x) > 3.0 {
                x["syncedLyrics"] = Value::Null;
            }
            x
        }),
    }
}

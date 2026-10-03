//! Universal importer: Spotify (playlist/album/track/profile), YouTube / YouTube Music
//! (video/playlist/album/mix), SoundCloud, "More like this" radio, and the YouTube match scorer.
use crate::net::{self, enc};
use crate::ytdlp;
use regex::Regex;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::sync::LazyLock;

#[derive(Clone, Debug, Default)]
pub struct ITrack {
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_artist: String,
    pub year: Option<u32>,
    pub track_no: Option<u32>,
    pub duration_ms: Option<u64>,
    pub explicit: bool,
    pub cover: Option<String>,
    pub spotify_id: Option<String>,
    pub youtube_id: Option<String>,
    pub direct_url: Option<String>,
    pub source_key: String,
}

#[derive(Clone, Debug, Default)]
pub struct Collection {
    pub kind: String, // playlist | album | track | radio
    pub id: String,
    pub name: String,
    pub owner: String,
    pub cover: Option<String>,
    pub tracks: Vec<ITrack>,
    pub complete: bool,
    pub via: String,    // api | embed | youtube | soundcloud
    pub source: String, // spotify | youtube | soundcloud
    pub url: String,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ProfilePlaylist {
    pub name: String,
    pub cover: Option<String>,
    pub total: u64,
    pub owner: String,
    pub url: String,
    /// PLAYLIST | ALBUM | LIKED
    pub kind: String,
    /// shown dimmed next to the row (e.g. why only part of it can be read)
    pub note: String,
}

#[derive(Clone, Debug)]
pub enum Fetched {
    Collection(Collection),
    Profile { name: String, cover: Option<String>, playlists: Vec<ProfilePlaylist> },
}

#[derive(Clone)]
pub struct Creds {
    pub id: String,
    pub secret: String,
    /// "Connect Spotify" refresh token (empty = not connected) and where to save a rotated one
    pub refresh: String,
    pub on_refresh: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
}

impl Creds {
    pub fn connected(&self) -> bool {
        !self.id.is_empty() && !self.refresh.is_empty()
    }
}

fn short_hash(s: &str) -> String {
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    h.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect()
}

static RE_SP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:spotify:(playlist|album|track):([A-Za-z0-9]+))|open\.spotify\.com/(?:intl-[a-z-]+/)?(?:embed/)?(playlist|album|track)/([A-Za-z0-9]+)").unwrap());
const LIKED_URL: &str = "spotify:liked";

static RE_SP_USER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"open\.spotify\.com/(?:intl-[a-z-]+/)?user/([^/?#]+)").unwrap());

pub fn detect(url: &str) -> Option<&'static str> {
    if url == LIKED_URL {
        Some("spotify-liked")
    } else if RE_SP_USER.is_match(url) {
        Some("spotify-profile")
    } else if RE_SP.is_match(url) {
        Some("spotify")
    } else if Regex::new(r"(youtube\.com|youtu\.be)/").unwrap().is_match(url) {
        Some("youtube")
    } else if url.contains("soundcloud.com/") {
        Some("soundcloud")
    } else {
        None
    }
}

pub fn fetch_any(url: &str, creds: &Creds) -> Result<Fetched, String> {
    let url = url.trim();
    match detect(url) {
        Some("spotify-liked") => liked(creds).map(Fetched::Collection),
        Some("spotify-profile") => spotify_profile(url, creds),
        Some("spotify") => spotify(url, creds).map(Fetched::Collection),
        Some(_) => generic(url, None, None, 500).map(Fetched::Collection),
        None => Err("Paste a Spotify, YouTube, YouTube Music or SoundCloud link.".into()),
    }
}

// ------------------------------------------------------------------------------- Spotify

fn best_image(imgs: Option<&Value>) -> Option<String> {
    let arr = imgs?.as_array()?;
    arr.iter().max_by_key(|i| i.get("width").and_then(|w| w.as_u64()).unwrap_or(0)).and_then(|i| i.get("url")).and_then(|u| u.as_str()).map(String::from)
}

fn sp_token(c: &Creds) -> Result<String, String> {
    use std::sync::Mutex;
    static CACHE: Mutex<(String, String, i64)> = Mutex::new((String::new(), String::new(), 0));
    let key = format!("{}:{}", c.id, c.secret);
    {
        let g = CACHE.lock().unwrap();
        if g.0 == key && crate::store::now_secs() < g.2 - 60 {
            return Ok(g.1.clone());
        }
    }
    let auth = format!("Basic {}", crate::spotify_auth::b64(key.as_bytes(), false));
    let j = net::post_form_json("https://accounts.spotify.com/api/token", &[("Authorization", &auth)], &[("grant_type", "client_credentials")])
        .map_err(|e| format!("Spotify auth failed ({e}). Check your Client ID / Secret in Settings."))?;
    let tok = j["access_token"].as_str().ok_or("Spotify auth failed")?.to_string();
    let exp = j["expires_in"].as_i64().unwrap_or(3600);
    *CACHE.lock().unwrap() = (key, tok.clone(), crate::store::now_secs() + exp);
    Ok(tok)
}

static USER_TOK: std::sync::Mutex<(String, String, i64)> = std::sync::Mutex::new((String::new(), String::new(), 0));

/// Access token for the connected account (cached; refreshed and rotated as needed).
fn user_token(c: &Creds) -> Result<String, String> {
    {
        let g = USER_TOK.lock().unwrap();
        // g.0 holds "old|new" after a rotation, so a caller still holding the old token also hits
        if g.0.split('|').any(|r| r == c.refresh) && crate::store::now_secs() < g.2 - 60 {
            return Ok(g.1.clone());
        }
    }
    let (tok, exp, rotated) = crate::spotify_auth::refresh(&c.id, &c.refresh)?;
    let key = match &rotated {
        Some(r) if *r != c.refresh => {
            if let Some(f) = &c.on_refresh {
                f(r.clone());
            }
            format!("{}|{r}", c.refresh)
        }
        _ => c.refresh.clone(),
    };
    *USER_TOK.lock().unwrap() = (key, tok.clone(), exp);
    Ok(tok)
}

/// Prime the cache right after "Connect Spotify" so the first import needs no refresh.
pub fn seed_user_token(refresh: &str, access: &str, expires: i64) {
    *USER_TOK.lock().unwrap() = (refresh.to_string(), access.to_string(), expires);
}

fn sp_api(token: &str, url: &str) -> Result<Value, String> {
    let full = if url.starts_with("http") { url.to_string() } else { format!("https://api.spotify.com/v1{url}") };
    let auth = format!("Bearer {token}");
    for _ in 0..4 {
        let (code, retry, v) = net::get_json_status(&full, &[("Authorization", &auth)])?;
        if code == 429 {
            let wait = retry.and_then(|r| r.parse::<u64>().ok()).unwrap_or(2).min(30);
            std::thread::sleep(std::time::Duration::from_secs(wait));
            continue;
        }
        if code == 401 {
            return Err("Spotify sign-in expired. Reconnect in Settings → Spotify.".into());
        }
        if code == 403 {
            return Err("Spotify refused (403). Your Spotify app's owner needs Premium, and your account must be listed under User Management in the Spotify dashboard.".into());
        }
        if !(200..300).contains(&code) {
            return Err(format!("Spotify API error {code}"));
        }
        return Ok(v);
    }
    Err("Spotify rate limit — try again in a minute.".into())
}

fn api_track(t: &Value, album: Option<&Value>) -> ITrack {
    let al = album.unwrap_or(&t["album"]);
    let id = t["id"].as_str().map(String::from);
    ITrack {
        title: t["name"].as_str().unwrap_or("").into(),
        artists: t["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect()).unwrap_or_default(),
        album: al["name"].as_str().unwrap_or("").into(),
        album_artist: al["artists"][0]["name"].as_str().unwrap_or("").into(),
        year: al["release_date"].as_str().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()),
        track_no: t["track_number"].as_u64().map(|n| n as u32),
        duration_ms: t["duration_ms"].as_u64(),
        explicit: t["explicit"].as_bool().unwrap_or(false),
        cover: best_image(al.get("images")),
        source_key: id.as_ref().map(|i| format!("sp:{i}")).unwrap_or_default(),
        spotify_id: id,
        ..Default::default()
    }
}

fn spotify(url: &str, creds: &Creds) -> Result<Collection, String> {
    let c = RE_SP.captures(url).ok_or("That does not look like a Spotify link.")?;
    let kind = c.get(1).or(c.get(3)).unwrap().as_str().to_string();
    let id = c.get(2).or(c.get(4)).unwrap().as_str().to_string();
    // connected account first (the only way to read full playlists since Spotify's Feb 2026 API
    // change), then app keys (albums/songs), then the public page (first 100 songs)
    let mut first_err = None;
    let mut tokens: Vec<Result<String, String>> = Vec::new();
    if creds.connected() {
        tokens.push(user_token(creds));
    }
    if !creds.id.is_empty() && !creds.secret.is_empty() && !(kind == "playlist" && creds.connected()) {
        tokens.push(sp_token(creds));
    }
    for tok in tokens {
        match tok.and_then(|t| spotify_api(&kind, &id, &t)) {
            Ok(mut col) => {
                col.url = url.into();
                return Ok(col);
            }
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    match spotify_embed(&kind, &id) {
        Ok(mut col) => {
            col.url = url.into();
            if !col.complete && kind == "playlist" {
                col.warning = Some(if creds.connected() {
                    "Spotify only shares the full song list of playlists you own or collaborate on, so this shows the first 100. Tip: in Spotify, copy the songs into a playlist of your own.".into()
                } else {
                    "Showing the first 100 songs. Connect your Spotify account (Settings → Spotify) to get every song of your playlists.".into()
                });
            } else if let Some(e) = first_err {
                col.warning = Some(e);
            }
            Ok(col)
        }
        Err(e) => Err(first_err.unwrap_or(e)),
    }
}

fn page_items(page: &Value, col: &mut Collection) {
    for it in page["items"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        // playlist items are `item` since Feb 2026 (was `track`); saved tracks are `track`
        let t = if it["item"].is_object() { &it["item"] } else { &it["track"] };
        if t["type"].as_str().unwrap_or("track") == "track" && t["id"].is_string() && !it["is_local"].as_bool().unwrap_or(false) {
            col.tracks.push(api_track(t, None));
        }
    }
}

fn spotify_api(kind: &str, id: &str, tok: &str) -> Result<Collection, String> {
    let mut col = Collection { kind: kind.into(), id: id.into(), complete: true, via: "api".into(), source: "spotify".into(), ..Default::default() };
    match kind {
        "track" => {
            let t = sp_api(tok, &format!("/tracks/{id}"))?;
            col.name = t["name"].as_str().unwrap_or("").into();
            col.owner = t["artists"][0]["name"].as_str().unwrap_or("").into();
            col.cover = best_image(t["album"].get("images"));
            col.tracks.push(api_track(&t, None));
        }
        "album" => {
            let a = sp_api(tok, &format!("/albums/{id}"))?;
            col.name = a["name"].as_str().unwrap_or("").into();
            col.owner = a["artists"][0]["name"].as_str().unwrap_or("").into();
            col.cover = best_image(a.get("images"));
            let mut page = a["tracks"].clone();
            loop {
                for t in page["items"].as_array().cloned().unwrap_or_default() {
                    col.tracks.push(api_track(&t, Some(&a)));
                }
                match page["next"].as_str() {
                    Some(n) => page = sp_api(tok, n)?,
                    None => break,
                }
            }
        }
        _ => {
            let p = sp_api(tok, &format!("/playlists/{id}?fields=name,owner(display_name),images"))?;
            col.name = p["name"].as_str().unwrap_or("").into();
            col.owner = p["owner"]["display_name"].as_str().unwrap_or("").into();
            col.cover = best_image(p.get("images"));
            let mut next = Some(format!("/playlists/{id}/items?limit=50&additional_types=track"));
            while let Some(u) = next {
                let page = sp_api(tok, &u)?;
                if page.get("items").is_none() {
                    return Err("Spotify only shares the songs of playlists you own or collaborate on.".into());
                }
                page_items(&page, &mut col);
                next = page["next"].as_str().map(String::from);
            }
        }
    }
    Ok(col)
}

fn need_connect(creds: &Creds) -> Result<String, String> {
    if !creds.connected() {
        return Err("Connect your Spotify account first (Settings → Spotify).".into());
    }
    user_token(creds)
}

/// The connected account's Liked Songs, as a playlist that keeps syncing.
fn liked(creds: &Creds) -> Result<Collection, String> {
    let tok = need_connect(creds)?;
    let mut col = Collection { kind: "playlist".into(), id: "liked".into(), name: "Liked Songs".into(), owner: "Your Spotify".into(), complete: true, via: "api".into(), source: "spotify".into(), url: LIKED_URL.into(), ..Default::default() };
    let mut next = Some("/me/tracks?limit=50".to_string());
    while let Some(u) = next {
        let page = sp_api(&tok, &u)?;
        page_items(&page, &mut col);
        next = page["next"].as_str().map(String::from);
    }
    col.cover = col.tracks.first().and_then(|t| t.cover.clone());
    if col.tracks.is_empty() {
        return Err("Your Liked Songs on Spotify is empty.".into());
    }
    Ok(col)
}

/// Everything in the connected account: Liked Songs, every playlist in the sidebar, saved albums.
pub fn library(creds: &Creds) -> Result<Fetched, String> {
    let tok = need_connect(creds)?;
    let me = sp_api(&tok, "/me")?;
    let my_id = me["id"].as_str().unwrap_or("").to_string();
    let mut playlists = Vec::new();
    let liked_total = sp_api(&tok, "/me/tracks?limit=1")?["total"].as_u64().unwrap_or(0);
    if liked_total > 0 {
        playlists.push(ProfilePlaylist { name: "Liked Songs".into(), cover: None, total: liked_total, owner: "you".into(), url: LIKED_URL.into(), kind: "LIKED".into(), note: String::new() });
    }
    let mut next = Some("/me/playlists?limit=50".to_string());
    while let Some(u) = next {
        let page = sp_api(&tok, &u)?;
        for p in page["items"].as_array().cloned().unwrap_or_default() {
            let Some(pid) = p["id"].as_str() else { continue };
            let total = p["items"]["total"].as_u64().or(p["tracks"]["total"].as_u64()).unwrap_or(0);
            let mine = p["owner"]["id"].as_str() == Some(my_id.as_str()) || p["collaborative"].as_bool().unwrap_or(false);
            playlists.push(ProfilePlaylist {
                name: p["name"].as_str().unwrap_or("").into(),
                cover: best_image(p.get("images")),
                total,
                owner: if mine { "you".into() } else { p["owner"]["display_name"].as_str().unwrap_or("").into() },
                url: p["external_urls"]["spotify"].as_str().map(String::from).unwrap_or_else(|| format!("https://open.spotify.com/playlist/{pid}")),
                kind: "PLAYLIST".into(),
                note: if !mine && total > 100 { "not yours: Spotify only shares the first 100".into() } else { String::new() },
            });
        }
        next = page["next"].as_str().map(String::from);
    }
    let mut next = Some("/me/albums?limit=50".to_string());
    while let Some(u) = next {
        let page = sp_api(&tok, &u)?;
        for it in page["items"].as_array().cloned().unwrap_or_default() {
            let a = &it["album"];
            let Some(aid) = a["id"].as_str() else { continue };
            playlists.push(ProfilePlaylist {
                name: a["name"].as_str().unwrap_or("").into(),
                cover: best_image(a.get("images")),
                total: a["total_tracks"].as_u64().unwrap_or(0),
                owner: a["artists"][0]["name"].as_str().unwrap_or("").into(),
                url: a["external_urls"]["spotify"].as_str().map(String::from).unwrap_or_else(|| format!("https://open.spotify.com/album/{aid}")),
                kind: "ALBUM".into(),
                note: String::new(),
            });
        }
        next = page["next"].as_str().map(String::from);
    }
    if playlists.is_empty() {
        return Err("Your Spotify library is empty.".into());
    }
    let name = me["display_name"].as_str().or(me["id"].as_str()).unwrap_or("Your").to_string();
    Ok(Fetched::Profile { name: format!("{name} · Spotify library"), cover: best_image(me.get("images")), playlists })
}

fn spotify_embed(kind: &str, id: &str) -> Result<Collection, String> {
    let html = net::get_text(&format!("https://open.spotify.com/embed/{kind}/{id}"), &[("User-Agent", net::UA)]).map_err(|e| format!("Spotify returned {e}. Is it public?"))?;
    let re = Regex::new(r#"(?s)<script id="__NEXT_DATA__" type="application/json">(.+?)</script>"#).unwrap();
    let json = re.captures(&html).ok_or("Could not read the Spotify page. Add API keys in Settings as a fallback.")?;
    let v: Value = serde_json::from_str(&json[1]).map_err(|e| e.to_string())?;
    let e = &v["props"]["pageProps"]["state"]["data"]["entity"];
    if e.is_null() {
        return Err("Spotify did not return any data for that link.".into());
    }
    let cover = best_image(e["coverArt"].get("sources")).or_else(|| best_image(e["visualIdentity"].get("image")));
    let mut col = Collection { kind: kind.into(), id: id.into(), name: e["name"].as_str().unwrap_or("").into(), cover: cover.clone(), via: "embed".into(), source: "spotify".into(), ..Default::default() };
    if kind == "track" {
        col.owner = e["artists"][0]["name"].as_str().unwrap_or("").into();
        col.tracks.push(ITrack {
            title: col.name.clone(),
            artists: e["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect()).unwrap_or_default(),
            duration_ms: e["duration"].as_u64(),
            cover,
            spotify_id: Some(id.into()),
            source_key: format!("sp:{id}"),
            ..Default::default()
        });
        col.complete = true;
        return Ok(col);
    }
    col.owner = e["subtitle"].as_str().unwrap_or("").into();
    for t in e["trackList"].as_array().cloned().unwrap_or_default() {
        let sid = t["uri"].as_str().and_then(|u| u.rsplit(':').next()).map(String::from);
        col.tracks.push(ITrack {
            title: t["title"].as_str().unwrap_or("").into(),
            artists: t["subtitle"].as_str().unwrap_or("").split(", ").filter(|s| !s.is_empty()).map(String::from).collect(),
            album: if kind == "album" { col.name.clone() } else { String::new() },
            album_artist: if kind == "album" { col.owner.clone() } else { String::new() },
            duration_ms: t["duration"].as_u64(),
            explicit: t["isExplicit"].as_bool().unwrap_or(false),
            cover: if kind == "album" { col.cover.clone() } else { None },
            source_key: sid.as_ref().map(|s| format!("sp:{s}")).unwrap_or_default(),
            spotify_id: sid,
            ..Default::default()
        });
    }
    col.complete = col.tracks.len() < 100;
    Ok(col)
}

fn spotify_profile(url: &str, creds: &Creds) -> Result<Fetched, String> {
    let uid = RE_SP_USER.captures(url).map(|c| urlencoding::decode(&c[1]).map(|s| s.into_owned()).unwrap_or_default()).ok_or("That is not a Spotify profile link.")?;
    // Spotify removed "list someone's playlists" for apps in Feb 2026; your own still works
    if creds.connected() {
        let tok = user_token(creds)?;
        if sp_api(&tok, "/me")?["id"].as_str() == Some(uid.as_str()) {
            return library(creds);
        }
        return Err("Spotify no longer lets apps list other people's playlists. Paste the playlist links one by one instead.".into());
    }
    Err("Spotify no longer lets apps list a profile's playlists. To bring over your own, connect your account in Settings → Spotify, then IMPORT MY WHOLE LIBRARY.".into())
}

// ------------------------------------------------------------------------------- YouTube / SoundCloud

static RE_JUNK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[\(\[][^\)\]]*\b(video|audio|lyrics?|visuali[sz]er|official|4k|8k|hd|hq|explicit|clean|oficial|directed|dir\.|shot by)\b[^\)\]]*[\)\]]").unwrap());
static RE_DASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.+?)\s+[-–—]\s+(.+)$").unwrap());

/// "Artist - Title (Official Video)" -> (artist, title); handles "Title - Artist" uploads too.
pub fn split_title(raw: &str, channel: &str) -> (String, String) {
    let mut title = RE_JUNK.replace_all(raw, "").split_whitespace().collect::<Vec<_>>().join(" ");
    let topic = channel.ends_with(" - Topic");
    let mut artist = channel.trim_end_matches(" - Topic").trim_end_matches("VEVO").trim().to_string();
    if !topic {
        if let Some(m) = RE_DASH.captures(&title.clone()) {
            let (a, b) = (m[1].trim().to_string(), m[2].trim().to_string());
            let la = artist.to_lowercase();
            let swapped = !la.is_empty() && b.to_lowercase().contains(&la) && !a.to_lowercase().contains(&la);
            if swapped {
                artist = b;
                title = a;
            } else {
                artist = a;
                title = b;
            }
        }
    }
    (if artist.is_empty() { "Unknown Artist".into() } else { artist }, if title.is_empty() { "Untitled".into() } else { title })
}

fn best_thumb(o: &Value) -> Option<String> {
    o["thumbnails"].as_array().and_then(|l| l.iter().max_by_key(|t| t["width"].as_u64().unwrap_or(0))).and_then(|t| t["url"].as_str()).or(o["thumbnail"].as_str()).map(String::from)
}

pub fn generic(url: &str, name: Option<String>, kind: Option<&str>, limit: usize) -> Result<Collection, String> {
    ytdlp::ensure()?;
    let is_sc = url.contains("soundcloud.com");
    let mut args: Vec<String> = Vec::new();
    if !is_sc {
        args.push("--flat-playlist".into());
    }
    args.extend(["-J".into(), "--playlist-end".into(), limit.to_string(), url.to_string()]);
    let raw = ytdlp::run_with(&args, None, None)?;
    let j: Value = serde_json::from_str(&raw).map_err(|e| format!("bad yt-dlp output: {e}"))?;
    let single = j.get("entries").is_none();
    let entries: Vec<Value> = if single { vec![j.clone()] } else { j["entries"].as_array().cloned().unwrap_or_default() };
    let mut tracks = Vec::new();
    for e in entries.iter().filter(|e| e["id"].is_string() || e["url"].is_string()) {
        let channel = e["channel"].as_str().or(e["uploader"].as_str()).or(j["uploader"].as_str()).unwrap_or("");
        let (artist, title) = match (e["artist"].as_str(), e["track"].as_str()) {
            (Some(a), Some(t)) => (a.to_string(), t.to_string()),
            _ => split_title(e["title"].as_str().unwrap_or(""), channel),
        };
        let yid = if !is_sc { e["id"].as_str().filter(|i| i.len() == 11).map(String::from) } else { None };
        let page = e["webpage_url"].as_str().map(String::from).or_else(|| yid.as_ref().map(|i| format!("https://music.youtube.com/watch?v={i}"))).or(e["url"].as_str().map(String::from));
        let split = Regex::new(r"(?i)\s*,\s*|\s+&\s+|\s+x\s+").unwrap();
        tracks.push(ITrack {
            title,
            artists: split.split(&artist).filter(|s| !s.is_empty()).take(4).map(String::from).collect(),
            album: e["album"].as_str().unwrap_or("").into(),
            duration_ms: e["duration"].as_f64().map(|d| (d * 1000.0) as u64),
            source_key: yid.as_ref().map(|i| format!("yt:{i}")).unwrap_or_else(|| format!("url:{}", page.clone().unwrap_or_default())),
            direct_url: page,
            youtube_id: yid,
            cover: if is_sc { best_thumb(e) } else { None },
            ..Default::default()
        });
    }
    if tracks.is_empty() {
        return Err("Nothing playable found at that link.".into());
    }
    let raw_title = j["title"].as_str().unwrap_or("").to_string();
    // YouTube Music names releases "Album - Title" / "EP - Title" / "Single - Title"
    let rel = Regex::new(r"^(Album|EP|Single) - (.+)$").unwrap().captures(&raw_title).map(|c| c[2].to_string());
    if let Some(r) = &rel {
        for t in tracks.iter_mut().filter(|t| t.album.is_empty()) {
            t.album = r.clone();
        }
    }
    let kind = kind.map(String::from).unwrap_or_else(|| if single { "track".into() } else if rel.is_some() { "album".into() } else { "playlist".into() });
    Ok(Collection {
        kind,
        id: format!("{}{}", if is_sc { "sc" } else { "yt" }, short_hash(url)),
        name: name.or(rel).unwrap_or_else(|| if raw_title.is_empty() { tracks[0].title.clone() } else { raw_title }),
        owner: j["uploader"].as_str().or(j["channel"].as_str()).unwrap_or("").trim_end_matches(" - Topic").into(),
        cover: best_thumb(&j).or_else(|| tracks[0].cover.clone()),
        tracks,
        complete: true,
        via: if is_sc { "soundcloud".into() } else { "youtube".into() },
        source: if is_sc { "soundcloud".into() } else { "youtube".into() },
        url: url.into(),
        warning: None,
    })
}

/// "More like this": YouTube Music's radio mix seeded from the song.
pub fn radio(title: &str, artist: &str, youtube_id: Option<&str>) -> Result<Collection, String> {
    ytdlp::ensure()?;
    let seed = match youtube_id {
        Some(s) => s.to_string(),
        None => {
            let q = enc(&format!("{artist} {title}"));
            let raw = ytdlp::run_with(&["--flat-playlist".into(), "-J".into(), "--playlist-end".into(), "1".into(), format!("https://music.youtube.com/search?q={q}#songs")], None, None)?;
            let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            v["entries"][0]["id"].as_str().ok_or("Could not find that song on YouTube Music.")?.to_string()
        }
    };
    let mut col = generic(&format!("https://music.youtube.com/watch?v={seed}&list=RDAMVM{seed}"), Some(format!("Radio · {title}")), Some("radio"), 51)?;
    col.tracks.retain(|t| t.youtube_id.as_deref() != Some(seed.as_str()));
    col.tracks.truncate(50);
    col.owner = format!("Songs like {title} — {artist}");
    col.id = format!("radio{seed}");
    Ok(col)
}

/// "Find new songs" search: YouTube Music songs (`source` = "songs"), regular YouTube videos
/// ("youtube") or SoundCloud ("soundcloud"). Results carry a direct link, so downloading them
/// needs no matching.
pub fn search(q: &str, source: &str, n: usize) -> Result<Vec<ITrack>, String> {
    let n = n.clamp(1, 50);
    if source == "songs" {
        // YouTube Music's own search API has artist / album / length (yt-dlp's flat listing only has titles)
        match music_search(q, n) {
            Ok(v) if !v.is_empty() => return Ok(v),
            _ => {}
        }
    }
    ytdlp::ensure()?;
    let url = match source {
        "youtube" => format!("ytsearch{n}:{q}"),
        // a few extra: 30-second previews of paid tracks are dropped
        "soundcloud" => format!("scsearch{}:{q}", n + 5),
        _ => format!("https://music.youtube.com/search?q={}#songs", enc(q)),
    };
    let raw = ytdlp::run_with(&["--flat-playlist".into(), "-J".into(), "--playlist-end".into(), (n + 5).to_string(), url], None, None)?;
    let j: Value = serde_json::from_str(&raw).map_err(|e| format!("bad yt-dlp output: {e}"))?;
    let out: Vec<ITrack> = j["entries"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            let channel = e["channel"].as_str().or(e["uploader"].as_str()).unwrap_or("");
            if source == "soundcloud" {
                let id = e["id"].as_str().map(String::from).or(e["id"].as_u64().map(|i| i.to_string()))?;
                let secs = e["duration"].as_f64().filter(|d| *d > 31.0)?;
                let link = e["webpage_url"].as_str()?;
                let cover = e["thumbnails"].as_array().and_then(|a| a.iter().find(|t| t["id"] == "t500x500")).and_then(|t| t["url"].as_str()).map(String::from);
                let (artist, title) = split_title(e["title"].as_str().unwrap_or(""), channel);
                return Some(ITrack { title, artists: vec![artist], duration_ms: Some((secs * 1000.0) as u64), cover, direct_url: Some(link.into()), source_key: format!("sc:{id}"), ..Default::default() });
            }
            let id = e["id"].as_str().filter(|i| i.len() == 11)?;
            let (artist, title) = split_title(e["title"].as_str().unwrap_or(""), channel);
            Some(yt_track(id, title, vec![artist], String::new(), e["duration"].as_f64(), None, source))
        })
        .take(n)
        .collect();
    if out.is_empty() {
        return Err("No results".into());
    }
    Ok(out)
}

fn yt_track(id: &str, title: String, artists: Vec<String>, album: String, secs: Option<f64>, cover: Option<String>, source: &str) -> ITrack {
    let host = if source == "youtube" { "www.youtube.com" } else { "music.youtube.com" };
    ITrack { title, artists, album, duration_ms: secs.map(|d| (d * 1000.0) as u64), cover, youtube_id: Some(id.into()), direct_url: Some(format!("https://{host}/watch?v={id}")), source_key: format!("yt:{id}"), ..Default::default() }
}

fn music_search(q: &str, n: usize) -> Result<Vec<ITrack>, String> {
    let body = serde_json::json!({ "context": { "client": { "clientName": "WEB_REMIX", "clientVersion": "1.20250101.01.00", "hl": "en" } }, "query": q, "params": "EgWKAQIIAWoKEAoQAxAEEAkQBQ==" });
    let v = net::post_json("https://music.youtube.com/youtubei/v1/search?prettyPrint=false", &[("User-Agent", net::UA), ("Origin", "https://music.youtube.com")], &body)?;
    let mut items = Vec::new();
    find_key(&v, "musicResponsiveListItemRenderer", &mut items);
    let runs = |r: &Value, col: usize| r["flexColumns"][col]["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"].as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for r in items {
        let title_runs = runs(r, 0);
        let Some(id) = r["playlistItemData"]["videoId"].as_str().or(title_runs.first().and_then(|t| t["navigationEndpoint"]["watchEndpoint"]["videoId"].as_str())) else { continue };
        let title: String = title_runs.iter().filter_map(|t| t["text"].as_str()).collect();
        // "Artist & Artist • Album • 3:45" (an unfiltered search starts with "Song •")
        let mut segs: Vec<Vec<Value>> = vec![Vec::new()];
        for run in runs(r, 1) {
            if run["text"].as_str().map(|t| t.trim() == "•").unwrap_or(false) {
                segs.push(Vec::new());
            } else {
                segs.last_mut().unwrap().push(run);
            }
        }
        let text = |s: &[Value]| s.iter().filter_map(|t| t["text"].as_str()).collect::<String>();
        if segs.len() > 2 && text(&segs[0]) == "Song" {
            segs.remove(0);
        }
        let artists: Vec<String> = segs[0].iter().filter_map(|t| t["text"].as_str()).map(str::trim).filter(|t| !t.is_empty() && *t != "&" && *t != ",").map(String::from).collect();
        let album = segs.iter().flatten().find(|t| t["navigationEndpoint"]["browseEndpoint"]["browseEndpointContextSupportedConfigs"]["browseEndpointContextMusicConfig"]["pageType"] == "MUSIC_PAGE_TYPE_ALBUM").and_then(|t| t["text"].as_str()).unwrap_or("").to_string();
        let secs = segs.last().map(|s| text(s)).and_then(|d| d.trim().split(':').try_fold(0.0, |acc, p| p.parse::<f64>().ok().map(|x| acc * 60.0 + x))).filter(|_| segs.len() > 1);
        let thumb = r["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"].as_array().and_then(|a| a.last()).and_then(|t| t["url"].as_str());
        // album art: ask for a 544px copy of the square thumbnail
        let cover = thumb.map(|u| Regex::new(r"=w\d+-h\d+").unwrap().replace(u, "=w544-h544").into_owned());
        let mut t = yt_track(id, title, if artists.is_empty() { vec!["Unknown Artist".into()] } else { artists }, album, secs, cover, "songs");
        t.explicit = r["badges"].to_string().contains("MUSIC_EXPLICIT_BADGE");
        out.push(t);
        if out.len() >= n {
            break;
        }
    }
    Ok(out)
}

fn find_key<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == key { out.push(x) } else { find_key(x, key, out) }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| find_key(x, key, out)),
        _ => {}
    }
}

// ------------------------------------------------------------------------------- matcher

#[derive(Clone, Debug, Default)]
pub struct Cand {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration: Option<f64>,
    pub source: String, // music | yt
    pub rank: Option<usize>,
    pub score: i32,
}

fn norm(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect::<String>().to_lowercase().replace('&', " and ").chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

const STOP: &[&str] = &["the", "a", "an", "feat", "ft", "featuring", "with", "and", "x", "official", "audio", "video", "music", "lyrics", "lyric", "hd", "hq", "4k", "remastered", "remaster", "version", "from", "of"];
const VARIANTS: &[&str] = &["live", "cover", "remix", "sped up", "speed up", "slowed", "reverb", "nightcore", "karaoke", "instrumental", "8d", "acoustic", "extended", "mashup", "bass boosted", "reaction", "tutorial", "1 hour", "loop", "clean version", "piano", "edit"];

fn tokens(s: &str) -> Vec<String> {
    norm(s).split(' ').filter(|t| !t.is_empty() && !STOP.contains(t)).map(String::from).collect()
}

fn core_title(t: &str) -> String {
    let a = Regex::new(r"(?i)\s*[\(\[](feat|ft|with)\.?[^\)\]]*[\)\]]").unwrap().replace_all(t, "").to_string();
    Regex::new(r"(?i)\s+-\s+(\d{4}\s+)?remaster(ed)?.*$").unwrap().replace(&a, "").trim().to_string()
}

pub fn score(c: &Cand, t: &ITrack) -> i32 {
    let want = t.duration_ms.map(|d| d as f64 / 1000.0);
    let ct = norm(&c.title);
    let cc = norm(&c.channel);
    let title_toks = tokens(&core_title(&t.title));
    let artist_toks: Vec<String> = t.artists.iter().flat_map(|a| tokens(a)).collect();
    let cand: std::collections::HashSet<String> = tokens(&c.title).into_iter().chain(tokens(&c.channel)).collect();
    let mut s = 0f64;
    let t_hit = if title_toks.is_empty() { 0.0 } else { title_toks.iter().filter(|x| cand.contains(*x)).count() as f64 / title_toks.len() as f64 };
    s += t_hit * 45.0;
    let a_hit = if artist_toks.is_empty() { 0.0 } else { artist_toks.iter().filter(|x| cand.contains(*x)).count() as f64 / artist_toks.len() as f64 };
    s += a_hit * 15.0;
    let main = norm(t.artists.first().map(|s| s.as_str()).unwrap_or(""));
    if !cc.is_empty() && (cc == main || cc.trim_end_matches(" topic") == main) {
        s += 15.0;
    }
    if c.source == "music" {
        s += 25.0 + [20.0, 10.0, 5.0].get(c.rank.unwrap_or(9)).copied().unwrap_or(0.0);
        if a_hit < 0.5 {
            s += 15.0;
        }
    }
    if let (Some(w), Some(d)) = (want, c.duration) {
        let diff = (d - w).abs();
        s += if diff <= 3.0 { 25.0 } else if diff <= 8.0 { 14.0 } else if diff <= 20.0 { -5.0 } else { -45.0 };
    }
    let ot = norm(&t.title);
    for v in VARIANTS {
        let re = Regex::new(&format!(r"\b{}\b", regex::escape(v))).unwrap();
        if re.is_match(&ct) && !re.is_match(&ot) {
            s -= 30.0;
        }
    }
    if Regex::new(r"\bofficial (audio|lyric)").unwrap().is_match(&ct) {
        s += 6.0;
    }
    if Regex::new(r"\b(music video|official video)\b").unwrap().is_match(&ct) {
        s -= 3.0;
    }
    s.round() as i32
}

pub fn rank(cands: Vec<Cand>, t: &ITrack) -> Vec<Cand> {
    let mut merged: Vec<Cand> = Vec::new();
    for c in cands {
        if let Some(p) = merged.iter_mut().find(|p| p.id == c.id) {
            if c.source == "music" {
                p.source = "music".into();
            }
            p.duration = p.duration.or(c.duration);
            if p.channel.is_empty() {
                p.channel = c.channel;
            }
            p.rank = p.rank.or(c.rank);
        } else {
            merged.push(c);
        }
    }
    for c in merged.iter_mut() {
        c.score = score(c, t);
    }
    merged.sort_by(|a, b| b.score.cmp(&a.score));
    merged
}

#[cfg(test)]
mod tests {
    /// Real online search on both tabs (network + yt-dlp): `cargo test -- --ignored`
    /// with DKFM_USER_DATA pointing at a test profile.
    #[test]
    #[ignore]
    fn finds_new_songs_online() {
        assert!(std::env::var("DKFM_USER_DATA").is_ok(), "use a test profile");
        for src in ["songs", "youtube"] {
            let r = super::search("daft punk one more time", src, 5).unwrap();
            for t in &r {
                eprintln!("{src}: {} | {} | {} | {:?} | {:?}", t.title, t.artists.join(", "), t.album, t.duration_ms, t.direct_url);
            }
            assert!(!r.is_empty() && r.len() <= 5);
            assert!(r.iter().all(|t| t.youtube_id.as_ref().map(|i| i.len() == 11).unwrap_or(false) && t.source_key.starts_with("yt:")));
            assert!(r.iter().any(|t| t.title.to_lowercase().contains("one more time") && t.duration_ms.is_some()));
        }
        assert!(super::search("daft punk one more time", "songs", 3).unwrap().iter().any(|t| t.artists.iter().any(|a| a == "Daft Punk")));
        // SoundCloud: direct links, no 30-second previews
        let sc = super::search("daft punk one more time", "soundcloud", 5).unwrap();
        for t in &sc {
            eprintln!("soundcloud: {} | {} | {:?} | {:?}", t.title, t.artists.join(", "), t.duration_ms, t.direct_url);
        }
        assert!(!sc.is_empty() && sc.len() <= 5);
        assert!(sc.iter().all(|t| t.source_key.starts_with("sc:") && t.direct_url.as_deref().unwrap_or("").contains("soundcloud.com") && t.duration_ms.unwrap_or(0) > 31_000));
    }
}

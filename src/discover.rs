//! Discovery without a server of our own: "Made from your library" mixes (built locally from your
//! plays, playlists and genres) and new releases from your top artists (YouTube Music, checked at
//! most once a day per artist, cached in releases.json).
use crate::library::{main_artist, ta_key};
use crate::sources::{self, Release};
use crate::store::{self, LibraryData};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const DAY: i64 = 86_400;
/// "new" = out within this many days
pub const WINDOW: i64 = 60;

#[derive(Clone, Debug)]
pub struct Mix {
    pub id: String,
    pub name: String,
    pub sub: String,
    pub ids: Vec<String>,
    /// where its online songs come from
    pub seed: Seed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Seed {
    /// the artist's top songs on YouTube Music (name)
    Artist(String),
    /// songs like the genre's favourite: (title, artist, YouTube id)
    Like(String, String, Option<String>),
    None,
}

fn akey(a: &str) -> String {
    main_artist(a).to_lowercase()
}

/// Your top artists as (key, name): most played first, then most songs. Unknown / Various skipped.
pub fn top_artists(d: &LibraryData, n: usize) -> Vec<(String, String)> {
    let mut by: HashMap<String, (u64, usize, String)> = HashMap::new();
    for t in d.tracks.values() {
        let name = main_artist(&t.artist);
        let k = name.to_lowercase();
        if k.is_empty() || k.starts_with("unknown") || k.starts_with("various") {
            continue;
        }
        let e = by.entry(k).or_insert((0, 0, name));
        e.0 += d.stats.get(&t.id).map(|s| s.plays as u64).unwrap_or(0);
        e.1 += 1;
    }
    let mut v: Vec<(String, (u64, usize, String))> = by.into_iter().collect();
    v.sort_by(|a, b| (b.1 .0, b.1 .1).cmp(&(a.1 .0, a.1 .1)).then(a.0.cmp(&b.0)));
    v.into_iter().take(n).map(|(k, (_, _, name))| (k, name)).collect()
}

/// 2 of `a`, 1 of `b`, … (the list order; shuffle mixes it further).
pub fn weave(a: &[&String], b: &[&String]) -> Vec<String> {
    let (mut out, mut i, mut j) = (Vec::with_capacity(a.len() + b.len()), 0, 0);
    while i < a.len() || j < b.len() {
        for _ in 0..2 {
            if i < a.len() {
                out.push(a[i].clone());
                i += 1;
            }
        }
        if j < b.len() {
            out.push(b[j].clone());
            j += 1;
        }
    }
    out
}

const MIN_MIX: usize = 8;

/// "Made from your library": a mix per top artist (their most played songs woven with artists that
/// share your playlists or their genre), per big genre, and forgotten favourites. Hidden songs are
/// left out. Deterministic for the same library.
/// Most played first (then by id, so it's stable).
fn ranked<'a>(d: &LibraryData, mut v: Vec<&'a String>) -> Vec<&'a String> {
    let plays = |id: &str| d.stats.get(id).map(|s| s.plays).unwrap_or(0);
    v.sort_by(|a, b| plays(b).cmp(&plays(a)).then(a.cmp(b)));
    v
}

pub fn mixes<'a>(d: &'a LibraryData, genres: &Genres, now_ms: f64) -> Vec<Mix> {
    let ok = |id: &String| !d.stats.get(id).map(|s| s.hidden).unwrap_or(false);
    let ranked = |v: Vec<&'a String>| ranked(d, v);
    // the file's own genre, or the artist's (looked up online: downloads carry none)
    let genre_of = |t: &crate::store::Track| {
        let g = t.genre.split([';', ',']).next().unwrap_or("").trim().to_string();
        if g.is_empty() { genres.of(&akey(&t.artist)).unwrap_or("").to_string() } else { g }
    };
    let mut by_artist: HashMap<String, (String, Vec<&String>)> = HashMap::new();
    let mut by_genre: HashMap<String, (String, Vec<&String>)> = HashMap::new();
    let mut artist_genres: HashMap<String, HashMap<String, usize>> = HashMap::new();
    // one copy of each song (the most played) when the library has several
    let plays = |id: &str| d.stats.get(id).map(|s| s.plays).unwrap_or(0);
    let mut uniq: HashMap<String, &crate::store::Track> = HashMap::new();
    for t in d.tracks.values().filter(|t| ok(&t.id)) {
        let e = uniq.entry(ta_key(&t.artist, &t.title)).or_insert(t);
        if (plays(&t.id), &e.id) > (plays(&e.id), &t.id) {
            *e = t;
        }
    }
    let one: HashSet<&String> = uniq.values().map(|t| &t.id).collect();
    for t in uniq.values() {
        let a = akey(&t.artist);
        by_artist.entry(a.clone()).or_insert_with(|| (main_artist(&t.artist), Vec::new())).1.push(&t.id);
        let g = genre_of(t);
        if !g.is_empty() {
            by_genre.entry(g.to_lowercase()).or_insert_with(|| (g.clone(), Vec::new())).1.push(&t.id);
            *artist_genres.entry(a).or_default().entry(g.to_lowercase()).or_default() += 1;
        }
    }
    let main_genre = |a: &str| artist_genres.get(a).and_then(|m| m.iter().max_by(|x, y| x.1.cmp(y.1).then(y.0.cmp(x.0))).map(|x| x.0.clone()));
    // playlists as sets of artists
    let pl_artists: Vec<HashSet<String>> = d.playlists.iter().map(|p| p.track_ids.iter().filter_map(|i| d.tracks.get(i)).map(|t| akey(&t.artist)).collect()).collect();
    let mut out = Vec::new();
    for (k, name) in top_artists(d, 6) {
        let Some((_, own)) = by_artist.get(&k) else { continue };
        let mut near: HashMap<&String, u32> = HashMap::new();
        for set in pl_artists.iter().filter(|s| s.contains(&k)) {
            for a in set.iter().filter(|a| **a != k) {
                *near.entry(a).or_default() += 2;
            }
        }
        if let Some(g) = main_genre(&k) {
            for a in artist_genres.keys().filter(|a| **a != k && main_genre(a).as_deref() == Some(g.as_str())) {
                *near.entry(a).or_default() += 1;
            }
        }
        let mut near: Vec<(&String, u32)> = near.into_iter().filter(|(a, _)| by_artist.contains_key(*a)).collect();
        near.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        near.truncate(8);
        let mine: Vec<&String> = ranked(own.clone()).into_iter().take(12).collect();
        let theirs: Vec<&String> = near.iter().flat_map(|(a, _)| ranked(by_artist[*a].1.clone()).into_iter().take(3)).take(18).collect();
        if mine.len() + theirs.len() < MIN_MIX {
            continue;
        }
        let names: Vec<&str> = near.iter().take(3).map(|(a, _)| by_artist[*a].0.as_str()).collect();
        let sub = if names.is_empty() { name.clone() } else { format!("{name}, {} and more", names.join(", ")) };
        out.push(Mix { id: format!("mix:artist:{k}"), name: format!("{name} Mix"), sub, ids: weave(&mine, &theirs), seed: Seed::Artist(name.clone()) });
    }
    let mut genres: Vec<(&String, &(String, Vec<&String>))> = by_genre.iter().filter(|(_, v)| v.1.len() >= 10).collect();
    genres.sort_by(|a, b| b.1 .1.len().cmp(&a.1 .1.len()).then(a.0.cmp(b.0)));
    for (k, (name, ids)) in genres.into_iter().take(4) {
        let ids = ranked(ids.clone());
        let mut arts: Vec<String> = Vec::new();
        for t in ids.iter().filter_map(|i| d.tracks.get(*i)) {
            let a = main_artist(&t.artist);
            if !arts.contains(&a) && arts.len() < 3 {
                arts.push(a);
            }
        }
        let seed = ids.first().and_then(|i| d.tracks.get(*i)).map(|t| Seed::Like(t.title.clone(), main_artist(&t.artist), t.youtube_id.clone())).unwrap_or(Seed::None);
        out.push(Mix { id: format!("mix:genre:{k}"), name: format!("{name} Mix"), sub: format!("{} and more", arts.join(", ")), ids: ids.into_iter().take(50).cloned().collect(), seed });
    }
    let old = now_ms - (WINDOW * DAY * 1000) as f64;
    let forgotten: Vec<&String> = ranked(d.stats.iter().filter(|(id, s)| s.plays >= 3 && s.last_played > 0.0 && s.last_played < old && one.contains(id)).map(|(id, _)| id).collect());
    if forgotten.len() >= MIN_MIX {
        out.push(Mix { id: "mix:forgotten".into(), name: "Forgotten Favourites".into(), sub: "Songs you loved but haven't played in a while".into(), ids: forgotten.into_iter().take(50).cloned().collect(), seed: Seed::None });
    }
    out
}

/// A mix's online songs (network): an artist's top songs on YouTube Music, or songs like a genre's
/// favourite. Songs you have are left out later, where the library is known.
pub fn mix_online(seed: &Seed) -> Result<Vec<sources::ITrack>, String> {
    match seed {
        Seed::Artist(name) => {
            let id = sources::artist_id(name)?.ok_or("Not found on YouTube Music")?;
            let mut songs = sources::artist_page(&id)?.songs;
            let want = name.to_lowercase();
            for t in sources::search(name, "songs", 20).unwrap_or_default() {
                if t.artists.iter().any(|a| a.to_lowercase() == want) && !songs.iter().any(|s| s.youtube_id == t.youtube_id) {
                    songs.push(t);
                }
            }
            Ok(songs)
        }
        Seed::Like(title, artist, yid) => Ok(sources::radio(title, artist, yid.as_deref())?.tracks),
        Seed::None => Ok(Vec::new()),
    }
}

// ------------------------------------------------------------------------------- genres

/// Artists' genres, from Apple's iTunes search (free, no account): files downloaded from YouTube
/// carry no genre, and the genre mixes and Discover need one. Looked up once per artist (unknown
/// ones again after 30 days), kept in genres.json.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Genres {
    /// artist key -> (genre, "" = unknown; when it was looked up)
    #[serde(default)]
    pub artists: BTreeMap<String, (String, i64)>,
}

impl Genres {
    fn path() -> std::path::PathBuf {
        store::data_dir().join("genres.json")
    }
    pub fn load() -> Self {
        store::load_json(&Self::path())
    }
    pub fn save(&self) {
        store::save_json(&Self::path(), self)
    }
    pub fn of(&self, artist_key: &str) -> Option<&str> {
        self.artists.get(artist_key).map(|g| g.0.as_str()).filter(|g| !g.is_empty())
    }
    /// These artists still need looking up.
    pub fn missing<'a>(&self, artists: &'a [(String, String)], now: i64) -> Vec<&'a (String, String)> {
        artists.iter().filter(|(k, _)| !self.artists.get(k).is_some_and(|(g, at)| !g.is_empty() || now - at < 30 * DAY)).collect()
    }
}

/// Looks up the genres not known yet, a few seconds apart (iTunes allows ~20 a minute), saving
/// and calling `progress` after each. Stops at the first network error (offline: next time).
pub fn fill_genres(artists: &[(String, String)], mut progress: impl FnMut(&Genres)) {
    let mut g = Genres::load();
    let now = store::now_secs();
    let todo: Vec<(String, String)> = g.missing(artists, now).into_iter().cloned().collect();
    for (k, name) in todo {
        let Ok(genre) = itunes_genre(&name) else { break };
        g.artists.insert(k, (genre, now));
        g.save();
        progress(&g);
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}

fn itunes_genre(name: &str) -> Result<String, String> {
    let v = crate::net::get_json(&format!("https://itunes.apple.com/search?term={}&entity=musicArtist&limit=5", crate::net::enc(name)), &[])?;
    let want = name.to_lowercase();
    Ok(v["results"].as_array().and_then(|r| r.iter().find(|a| a["artistName"].as_str().is_some_and(|n| n.to_lowercase() == want))).and_then(|a| a["primaryGenreName"].as_str()).unwrap_or("").to_string())
}

// ------------------------------------------------------------------------------- new releases

/// What was found for each artist, and the release days looked up so far.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Releases {
    /// artist key -> their releases
    #[serde(default)]
    pub artists: BTreeMap<String, Checked>,
    /// release playlist -> release day (unix s; 0 = unknown), looked up once
    #[serde(default)]
    pub dates: BTreeMap<String, i64>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Checked {
    /// YouTube Music channel ("" = not found)
    #[serde(default)]
    pub id: String,
    pub at: i64,
    #[serde(default)]
    pub releases: Vec<Release>,
}

/// Could this release be from the last WINDOW days, going by its year? Only those get their exact
/// day looked up.
pub fn maybe_new(r: &Release, now: i64) -> bool {
    use chrono::Datelike;
    let from = chrono::DateTime::from_timestamp(now - WINDOW * DAY, 0).map(|d| d.year() as u32).unwrap_or(0);
    r.year.map(|y| y >= from).unwrap_or(true)
}

/// You have it: an album of that name by that artist, or (a single) a song of that name.
pub fn have(keys: &HashSet<String>, r: &Release) -> bool {
    keys.contains(&ta_key(&r.artist, &r.title))
}

impl Releases {
    pub fn path() -> std::path::PathBuf {
        store::data_dir().join("releases.json")
    }
    pub fn load() -> Self {
        store::load_json(&Self::path())
    }
    pub fn save(&self) {
        store::save_json(&Self::path(), self);
    }

    /// Any of `artists` not checked in the last 24 hours?
    pub fn due(&self, artists: &[(String, String)], now: i64) -> bool {
        artists.iter().any(|(k, _)| self.artists.get(k).map(|c| now - c.at >= DAY).unwrap_or(true))
    }

    /// Releases of `artists` out in the last WINDOW days that you don't have, newest first.
    pub fn fresh(&self, artists: &[(String, String)], now: i64, have: impl Fn(&Release) -> bool) -> Vec<(Release, i64)> {
        let mut seen = HashSet::new();
        let mut v: Vec<(Release, i64)> = artists
            .iter()
            .filter_map(|(k, _)| self.artists.get(k))
            .flat_map(|c| c.releases.iter())
            .filter_map(|r| {
                let d = *self.dates.get(&r.playlist)?;
                // (the same release twice, e.g. explicit and clean: once)
                (d > 0 && now - d <= WINDOW * DAY && d <= now + DAY && !have(r) && seen.insert(r.playlist.clone()) && seen.insert(ta_key(&r.artist, &r.title))).then(|| (r.clone(), d))
            })
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.title.cmp(&b.0.title)));
        v
    }
}

/// Brings the cache up to date for `artists`, checking only those not checked for 24 hours:
/// `page(name, known channel)` -> (channel, releases); `date(playlist)` -> release day (None =
/// unknown, not asked again); `done` after each artist. Stops at the first error (offline: try
/// again later) keeping what it has. Returns how many artists were checked.
pub fn update(cache: &mut Releases, artists: &[(String, String)], now: i64, mut page: impl FnMut(&str, &str) -> Result<(String, Vec<Release>), String>, mut date: impl FnMut(&str) -> Result<Option<i64>, String>, mut done: impl FnMut(&Releases)) -> Result<usize, String> {
    let keep: HashSet<&String> = artists.iter().map(|a| &a.0).collect();
    cache.artists.retain(|k, c| keep.contains(k) || now - c.at < 30 * DAY);
    cache.dates.retain(|_, d| *d == 0 || now - *d < 400 * DAY);
    let mut n = 0;
    for (k, name) in artists {
        if cache.artists.get(k).map(|c| now - c.at < DAY).unwrap_or(false) {
            continue;
        }
        let known = cache.artists.get(k).map(|c| c.id.clone()).unwrap_or_default();
        let (id, releases) = page(name, &known)?;
        for r in releases.iter().filter(|r| maybe_new(r, now)) {
            if !cache.dates.contains_key(&r.playlist) {
                let d = date(&r.playlist)?;
                cache.dates.insert(r.playlist.clone(), d.unwrap_or(0));
            }
        }
        cache.artists.insert(k.clone(), Checked { id, at: now, releases });
        done(cache);
        n += 1;
    }
    Ok(n)
}

/// The online check (network; run it on a background thread): a short pause between requests,
/// saved (and handed to `progress`) after each artist. Returns the cache and the error that
/// stopped it, if any.
pub fn check_online(artists: &[(String, String)], mut progress: impl FnMut(&Releases)) -> (Releases, Option<String>) {
    let mut cache = Releases::load();
    let pause = || std::thread::sleep(std::time::Duration::from_millis(350));
    let r = update(
        &mut cache,
        artists,
        store::now_secs(),
        |name, known| {
            pause();
            let id = if known.is_empty() { sources::artist_id(name)?.unwrap_or_default() } else { known.to_string() };
            if id.is_empty() {
                return Ok((id, Vec::new()));
            }
            pause();
            Ok((id.clone(), sources::artist_page(&id)?.releases))
        },
        |pl| {
            pause();
            match sources::release_date(pl) {
                Ok(d) => Ok(Some(d)),
                // the release itself has no date: don't ask again; anything else is the network
                Err(e) if e.starts_with("empty release") || e.starts_with("no release date") || e.starts_with("bad date") => Ok(None),
                Err(e) => Err(e),
            }
        },
        |c| {
            c.save();
            progress(c);
        },
    );
    cache.save();
    prune_covers();
    (cache, r.err())
}

/// A wide picture (a YouTube thumbnail: the cover in the middle of a 16:9 frame) cut to the square
/// in its middle; anything else as it is.
pub fn square(b: Vec<u8>) -> Vec<u8> {
    let Ok(img) = image::load_from_memory(&b) else { return b };
    let (w, h) = (img.width(), img.height());
    if w * 10 <= h * 12 {
        return b;
    }
    let mut out = std::io::Cursor::new(Vec::new());
    match img.crop_imm((w - h) / 2, 0, h, h).to_rgb8().write_to(&mut out, image::ImageFormat::Jpeg) {
        Ok(()) => out.into_inner(),
        Err(_) => b,
    }
}

/// Folder for downloaded covers of things you don't have (new releases, artist pages).
pub fn covers_dir() -> std::path::PathBuf {
    store::data_dir().join("discover")
}

/// Removes covers not looked at for 30 days.
fn prune_covers() {
    for e in std::fs::read_dir(covers_dir()).into_iter().flatten().flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|a| a.as_secs() > 30 * DAY as u64).unwrap_or(false);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Playlist, Stat, Track};

    fn t(id: &str, artist: &str, genre: &str) -> Track {
        Track { id: id.into(), path: format!("/m/{id}.m4a"), title: format!("Song {id}"), artist: artist.into(), genre: genre.into(), ..Default::default() }
    }

    #[test]
    fn local_mixes() {
        let mut d = LibraryData::default();
        // Sade (most played, Soul), Anita Baker (Soul, shares a playlist), Drake (Hip-Hop), Quiet (Soul, few songs)
        for i in 0..8 {
            d.tracks.insert(format!("s{i}"), t(&format!("s{i}"), "Sade", "Soul"));
            d.stats.insert(format!("s{i}"), Stat { plays: 10 + i, ..Default::default() });
            d.tracks.insert(format!("a{i}"), t(&format!("a{i}"), "Anita Baker", "Soul; R&B"));
            d.tracks.insert(format!("d{i}"), t(&format!("d{i}"), "Drake feat. Rihanna", "Hip-Hop"));
        }
        d.tracks.insert("q0".into(), t("q0", "Quiet", "Soul"));
        // a second copy of a song: in mixes once (the most played copy)
        d.tracks.insert("s0copy".into(), Track { title: "Song s0".into(), ..t("s0copy", "Sade", "Soul") });
        d.stats.insert("d0".into(), Stat { plays: 3, ..Default::default() });
        d.stats.insert("a7".into(), Stat { plays: 1, hidden: true, ..Default::default() });
        d.playlists.push(Playlist { id: "p".into(), name: "Mood".into(), track_ids: vec!["s0".into(), "a1".into()], ..Default::default() });
        // forgotten favourites: played a lot, but 90 days ago
        let now = 1_800_000_000_000.0;
        for i in 0..8 {
            d.stats.get_mut(&format!("s{i}")).unwrap().last_played = now - 90.0 * 86_400_000.0;
        }
        let top = top_artists(&d, 3);
        assert_eq!(top, vec![("sade".to_string(), "Sade".to_string()), ("drake".into(), "Drake".into()), ("anita baker".into(), "Anita Baker".into())]);
        let m = mixes(&d, &Genres::default(), now);
        let sade = m.iter().find(|x| x.id == "mix:artist:sade").expect("Sade mix");
        assert_eq!(sade.name, "Sade Mix");
        assert!(sade.sub.starts_with("Sade, Anita Baker"), "{}", sade.sub); // shares a playlist + genre: first
        assert_eq!(&sade.ids[..3], ["s7", "s6", "a0"]); // most played first, 2 : 1 with related artists
        assert!(!sade.ids.contains(&"a7".to_string()) && !sade.ids.iter().any(|i| i.starts_with('d'))); // hidden / other genre left out
        assert!(sade.ids.contains(&"q0".to_string()) && sade.ids.contains(&"s0".to_string()) && !sade.ids.contains(&"s0copy".to_string()));
        // genre mixes need 10+ songs: Soul (8 + 7 + 1); Hip-Hop has 8
        let soul = m.iter().find(|x| x.id == "mix:genre:soul").expect("Soul mix");
        assert_eq!((soul.ids.len(), soul.ids[0].as_str()), (16, "s7"));
        assert!(!m.iter().any(|x| x.id == "mix:genre:hip-hop"));
        assert!(m.iter().any(|x| x.id == "mix:forgotten" && x.ids.len() == 8));
        // same library, same mixes
        let again = mixes(&d, &Genres::default(), now);
        assert_eq!(m.iter().map(|x| (&x.id, &x.ids)).collect::<Vec<_>>(), again.iter().map(|x| (&x.id, &x.ids)).collect::<Vec<_>>());
        // an empty library has none
        assert!(mixes(&LibraryData::default(), &Genres::default(), now).is_empty());
    }

    fn rel(title: &str, artist: &str, year: u32, pl: &str) -> Release {
        Release { title: title.into(), artist: artist.into(), kind: "Single".into(), year: Some(year), playlist: pl.into(), cover: None }
    }

    #[test]
    fn release_window_and_cache() {
        let now = sources::day_of("2026-10-03").unwrap();
        assert_eq!(sources::day_of("2026-06-07T12:00:36-07:00"), Some(now - 118 * DAY));
        let artists = vec![("sade".to_string(), "Sade".to_string()), ("drake".to_string(), "Drake".to_string())];
        let releases = |a: &str| match a {
            "Sade" => vec![rel("New One", "Sade", 2026, "OLAK1"), rel("Older", "Sade", 2026, "OLAK2"), rel("Have It", "Sade", 2026, "OLAK3"), rel("Classic", "Sade", 1984, "OLAK4")],
            _ => vec![rel("Fresh", "Drake", 2026, "OLAK5"), rel("Undated", "Drake", 2026, "OLAK6")],
        };
        let day = |pl: &str| match pl {
            "OLAK1" => Some(now - 5 * DAY),
            "OLAK2" => Some(now - 90 * DAY),
            "OLAK3" => Some(now - 10 * DAY),
            "OLAK5" => Some(now - 20 * DAY),
            "OLAK4" => panic!("1984 isn't looked up"),
            _ => None,
        };
        let (mut pages, mut dates) = (0, 0);
        let mut c = Releases::default();
        assert!(c.due(&artists, now));
        let n = update(&mut c, &artists, now, |a, _| { pages += 1; Ok((format!("UC{a}"), releases(a))) }, |p| { dates += 1; Ok(day(p)) }, |_| {}).unwrap();
        assert_eq!((n, pages, dates), (2, 2, 5));
        assert!(!c.due(&artists, now + DAY - 1));
        let keys: HashSet<String> = [ta_key("Sade", "Have It")].into();
        let fresh = c.fresh(&artists, now, |r| have(&keys, r));
        assert_eq!(fresh.iter().map(|f| f.0.title.as_str()).collect::<Vec<_>>(), ["New One", "Fresh"]);
        // a saved cache comes back the same
        let back: Releases = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.fresh(&artists, now, |r| have(&keys, r)).len(), 2);
        // within 24 hours nothing is fetched again
        let n = update(&mut c, &artists, now + 3600, |_, _| panic!("cached"), |_| panic!("cached"), |_| {}).unwrap();
        assert_eq!(n, 0);
        // a day later: the page again (with the known channel), but dates only for new releases
        let (mut pages, mut dates) = (0, 0);
        update(&mut c, &artists, now + DAY + 1, |a, known| {
            pages += 1;
            assert_eq!(known, format!("UC{a}"));
            let mut r = releases(a);
            r.push(rel("Newest", a, 2026, &format!("NEW{a}")));
            Ok((known.to_string(), r))
        }, |_| { dates += 1; Ok(Some(now + DAY)) }, |_| {}).unwrap();
        assert_eq!((pages, dates), (2, 2));
        assert_eq!(c.fresh(&artists, now + DAY + 1, |_| false)[0].0.title, "Newest");
        // 61 days on, "New One" has aged out
        assert!(!c.fresh(&artists, now + 61 * DAY, |_| false).iter().any(|f| f.0.title == "New One"));
        // offline: stops at the error, keeps what it had and checks again next time
        let mut c2 = c.clone();
        let err = update(&mut c2, &artists, now + 3 * DAY, |_, _| Err("Connection Failed".into()), |_| Ok(None), |_| panic!("nothing checked"));
        assert!(err.is_err() && c2.due(&artists, now + 3 * DAY));
        assert_eq!(c2.fresh(&artists, now + DAY + 1, |_| false).len(), c.fresh(&artists, now + DAY + 1, |_| false).len());
        // maybe_new: early in a year last year's releases still count
        let jan = sources::day_of("2027-01-20").unwrap();
        assert!(maybe_new(&rel("x", "y", 2026, "p"), jan) && !maybe_new(&rel("x", "y", 2025, "p"), jan));
        assert!(!maybe_new(&rel("x", "y", 2025, "p"), now));
    }

    /// Real lookups (network + yt-dlp): `cargo test --release -- --ignored discovers_online` with
    /// DKFM_USER_DATA pointing at a test profile (releases.json is written there).
    #[test]
    #[ignore]
    fn discovers_online() {
        assert!(std::env::var("DKFM_USER_DATA").is_ok(), "use a test profile");
        let now = store::now_secs();
        // artist page: top songs, and albums / singles with their playlists
        let id = sources::artist_id("Taylor Swift").unwrap().expect("artist found");
        assert_eq!(sources::artist_id("zzqx no such artist qqzz").unwrap(), None);
        let page = sources::artist_page(&id).unwrap();
        for t in &page.songs {
            eprintln!("song: {} | {} | {} | {:?}", t.title, t.artists.join(", "), t.album, t.youtube_id);
        }
        for r in page.releases.iter().take(6) {
            eprintln!("release: {} | {} | {:?} | {} | {:?}", r.title, r.kind, r.year, r.playlist, r.cover);
        }
        assert!(page.songs.len() >= 3 && page.songs.iter().all(|t| t.youtube_id.as_ref().map(|i| i.len() == 11).unwrap_or(false)));
        assert!(page.releases.iter().any(|r| r.kind == "Album") && page.releases.iter().any(|r| r.kind == "Single"));
        assert!(page.releases.iter().all(|r| r.playlist.starts_with("OLAK") && r.year.is_some() && r.cover.is_some()));
        // a release's day, and its songs as the importer reads them (what + GET downloads)
        let single = page.releases.iter().find(|r| r.kind == "Single").unwrap();
        let day = sources::release_date(&single.playlist).unwrap();
        eprintln!("{} came out {} days ago", single.title, (now - day) / DAY);
        assert!(day > sources::day_of("2006-01-01").unwrap() && day <= now + DAY);
        let col = sources::generic(&single.url(), Some(single.title.clone()), Some("album"), 300).unwrap();
        assert!(col.kind == "album" && !col.tracks.is_empty() && col.tracks.iter().all(|t| t.direct_url.is_some() && t.artists.iter().any(|a| a == "Taylor Swift")), "{:?}", col.tracks.first());
        // the daily check: saved, and not repeated within 24 hours
        let artists = vec![("taylor swift".to_string(), "Taylor Swift".to_string()), ("daft punk".to_string(), "Daft Punk".to_string())];
        let mut steps = 0;
        let (c, err) = check_online(&artists, |_| steps += 1);
        assert!(err.is_none(), "{err:?}");
        eprintln!("{steps} artists checked now");
        assert!(c.artists.len() >= 2 && c.artists["daft punk"].releases.len() > 3 && Releases::path().exists());
        for (r, d) in c.fresh(&artists, now, |_| false) {
            eprintln!("new: {} — {} ({}, {} days ago)", r.artist, r.title, r.kind, (now - d) / DAY);
        }
        let t0 = std::time::Instant::now();
        let (again, _) = check_online(&artists, |_| panic!("checked within 24 h"));
        assert!(t0.elapsed().as_secs() < 2 && again.artists["taylor swift"].at == c.artists["taylor swift"].at);
        // recommendations: the radio a playlist's section is built from
        let radio = sources::radio("One More Time", "Daft Punk", None).unwrap();
        eprintln!("radio: {:?}", radio.tracks.iter().take(5).map(|t| format!("{} — {}", t.artists.join(", "), t.title)).collect::<Vec<_>>());
        assert!(radio.tracks.len() >= 10);
    }
}


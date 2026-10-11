//! Word timing for sing-along: when each word of a synced line is sung.
//!
//! Exact times come from NetEase Cloud Music's word-by-word lyrics (most popular songs in any
//! language have them, timed by hand to the album recording): its words are lined up with the
//! song's own lyric lines (whatever source they came from) and each matched word takes its time.
//! A song NetEase doesn't have (or a different recording: its length must match within 2 s) gets
//! an estimate instead: each word's share of the line by its syllables, tuned against NetEase's
//! times (about 0.29 s off on average, against 0.40 s for the old letter count).
//!
//! Looked up once per song, when its lyrics are first sung along to, and saved beside its lyrics
//! (lyrics/<id>.words.json; "nothing found" is retried after 30 days).
use crate::store::{data_dir, now_secs, Track};
use serde::{Deserialize, Serialize};

/// Per lyric line: when each unit (see `units`) starts, then when the singing of the line ends.
/// Empty for a line with no exact times.
pub type Times = Vec<Vec<f64>>;

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    /// which lyrics these times belong to (they're looked up again if the lyrics change)
    hash: u64,
    #[serde(default)]
    lines: Times,
    at: i64,
}

fn path(id: &str) -> std::path::PathBuf {
    data_dir().join("lyrics").join(format!("{id}.words.json"))
}

fn hash(lines: &[(f64, String)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (t, s) in lines {
        ((t * 100.0).round() as i64).hash(&mut h);
        s.hash(&mut h);
    }
    h.finish()
}

/// Exact word times for these lyric lines, saved or looked up (blocking: call from a background
/// thread). None if NetEase doesn't have the song.
pub fn exact(t: &Track, lines: &[(f64, String)]) -> Option<Times> {
    let h = hash(lines);
    if let Some(s) = std::fs::read(path(&t.id)).ok().and_then(|b| serde_json::from_slice::<Saved>(&b).ok()).filter(|s| s.hash == h) {
        if !s.lines.is_empty() {
            return Some(s.lines);
        }
        if now_secs() - s.at < 30 * 86400 {
            return None;
        }
    }
    let found = netease(t).and_then(|words| align(lines, &words));
    crate::store::save_json(&path(&t.id), &Saved { hash: h, lines: found.clone().unwrap_or_default(), at: now_secs() });
    found
}

/// Forget a song's word times (its lyrics were looked up again, or it got other audio).
pub fn forget(id: &str) {
    let _ = std::fs::remove_file(path(id));
}

// --------------------------------------------------------------------------------------- units

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

/// What lights up one at a time: words, and each character of Chinese, Japanese and Korean (no
/// spaces between words there). Byte ranges into `text`.
pub fn units(text: &str) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                v.push((s, i));
            }
        } else if is_cjk(c) {
            if let Some(s) = start.take() {
                v.push((s, i));
            }
            v.push((i, i + c.len_utf8()));
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        v.push((s, text.len()));
    }
    v
}

/// Lowercase letters and digits only ("I'm," -> "im"), to match words written differently.
fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// About how many syllables a word has (vowel groups; a silent final e doesn't count).
fn syllables(w: &str) -> f64 {
    let w = norm(w);
    if w.chars().any(is_cjk) {
        return 1.0;
    }
    let mut n = 0;
    let mut in_vowel = false;
    for c in w.chars() {
        let v = matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'á' | 'é' | 'í' | 'ó' | 'ú' | 'à' | 'è' | 'ì' | 'ò' | 'ù' | 'ä' | 'ë' | 'ï' | 'ö' | 'ü');
        if v && !in_vowel {
            n += 1;
        }
        in_vowel = v;
    }
    if n > 1 && w.ends_with('e') && !(w.ends_with("le") || w.ends_with("ee") || w.ends_with("ye")) {
        n -= 1;
    }
    n.max(1) as f64
}

/// Estimated times for a line starting at `start` with `gap` seconds until the next one: each
/// unit's share by syllables (+1, words take time even when short), the last one held. Same shape
/// as a line of `Times`.
pub fn estimate(text: &str, start: f64, gap: f64) -> Vec<f64> {
    let u = units(text);
    if u.is_empty() {
        return vec![start, start + gap.min(1.0)];
    }
    let syl: Vec<f64> = u.iter().map(|r| syllables(&text[r.0..r.1])).collect();
    let total: f64 = syl.iter().sum();
    // time until the last unit starts (fitted: 0.36 s a syllable + 0.4 s, at most three
    // quarters of the gap less a breath)
    let span = (total * 0.36 + 0.4).min((gap - 0.3) * 0.75).max(0.3);
    let weights: f64 = syl[..syl.len() - 1].iter().map(|s| s + 1.0).sum::<f64>().max(1.0);
    let mut out = Vec::with_capacity(u.len() + 1);
    let mut acc = 0.0;
    for s in &syl {
        out.push(start + acc / weights * span);
        acc += s + 1.0;
    }
    let last = *out.last().unwrap();
    out.push((last + syl[syl.len() - 1] * 0.3 + 0.3).min(start + gap - 0.1).max(last + 0.2));
    out
}

/// Times for `text` sung evenly (by syllables) from `start` to `end`: a translation shown on its
/// own, sharing out the original line's time.
pub fn spread(text: &str, start: f64, end: f64) -> Vec<f64> {
    let u = units(text);
    let syl: Vec<f64> = u.iter().map(|r| syllables(&text[r.0..r.1]) + 1.0).collect();
    let total = syl.iter().sum::<f64>().max(1.0);
    let mut out = Vec::with_capacity(u.len() + 1);
    let mut acc = 0.0;
    for s in &syl {
        out.push(start + acc / total * (end - start));
        acc += s;
    }
    out.push(end);
    out
}

/// A line's times: exact ones if they fit its units, else the estimate.
pub fn line_times(exact: Option<&Vec<f64>>, text: &str, start: f64, gap: f64) -> Vec<f64> {
    match exact {
        Some(t) if t.len() == units(text).len() + 1 && t.len() > 1 => t.clone(),
        _ => estimate(text, start, gap),
    }
}

/// How far into `text` has been sung at `pos` (byte position: units light up whole, the moment
/// they start).
pub fn sung_upto(text: &str, times: &[f64], pos: f64) -> usize {
    let u = units(text);
    let n = u.iter().zip(times).take_while(|(_, t)| **t <= pos).count();
    if n == 0 { 0 } else { u[n - 1].1 }
}

// ------------------------------------------------------------------------------------- NetEase

/// One word NetEase has: (start, end, text).
type Word = (f64, f64, String);

const UA: [(&str, &str); 1] = [("User-Agent", crate::net::UA)];

/// NetEase's word-by-word lyrics for the song (the same recording: within 2 s of its length).
fn netease(t: &Track) -> Option<Vec<Word>> {
    let artist = crate::library::main_artist(&t.artist);
    let title = crate::lyricsrc::bare_title(&t.title);
    let q = crate::net::enc(&format!("{artist} {title}"));
    let v = crate::net::get_json(&format!("https://music.163.com/api/search/get?s={q}&type=1&limit=10"), &UA).ok()?;
    let (na, nt) = (norm(&artist), norm(&title));
    let pick = v["result"]["songs"].as_array()?.iter().find(|s| {
        let secs = s["duration"].as_f64().unwrap_or(0.0) / 1000.0;
        let close = t.duration <= 0.0 || (secs - t.duration).abs() <= 2.0;
        let name = norm(s["name"].as_str().unwrap_or(""));
        let named = !nt.is_empty() && (name.contains(&nt) || nt.contains(&name)) && !name.is_empty();
        let by = s["artists"].as_array().into_iter().flatten().any(|a| {
            let a = norm(a["name"].as_str().unwrap_or(""));
            !a.is_empty() && (a.contains(&na) || na.contains(&a))
        });
        close && (named || by)
    })?;
    let id = pick["id"].as_i64()?;
    let l = crate::net::get_json(&format!("https://music.163.com/api/song/lyric/v1?id={id}&lv=1&yv=1"), &UA).ok()?;
    let words = parse_yrc(l["yrc"]["lyric"].as_str()?);
    (words.len() >= 8).then_some(words)
}

/// NetEase's word-timed format: `[line ms,dur](word ms,dur,0)word(…)…` per line (lines in
/// JSON, the credits, are skipped).
pub fn parse_yrc(src: &str) -> Vec<Word> {
    let mut out = Vec::new();
    for line in src.lines() {
        if !line.starts_with('[') {
            continue;
        }
        let Some(body) = line.find(']').map(|i| &line[i + 1..]) else { continue };
        let mut rest = body;
        while let Some(open) = rest.find('(') {
            let Some(close) = rest[open..].find(')').map(|c| open + c) else { break };
            let nums: Vec<f64> = rest[open + 1..close].split(',').filter_map(|n| n.trim().parse().ok()).collect();
            let after = &rest[close + 1..];
            let end = after.find('(').unwrap_or(after.len());
            let text = after[..end].trim().to_string();
            if nums.len() >= 2 && !text.is_empty() {
                out.push((nums[0] / 1000.0, (nums[0] + nums[1]) / 1000.0, text));
            }
            rest = &after[end..];
        }
    }
    out
}

/// Line NetEase's words up with the lyric lines (in order, allowing words that differ, are
/// missing or extra) and give each line its times. Lines where most words were found get exact
/// times, the words in between share the gaps; the rest are left to the estimate. None if few
/// words matched or the two are timed differently (another recording).
fn align(lines: &[(f64, String)], words: &[Word]) -> Option<Times> {
    // the lyrics' units: (line, unit, normalised)
    let mut ours: Vec<(usize, usize, String)> = Vec::new();
    for (li, (_, text)) in lines.iter().enumerate() {
        for (ui, r) in units(text).iter().enumerate() {
            ours.push((li, ui, norm(&text[r.0..r.1])));
        }
    }
    // NetEase's words split the same way (a token can hold several units)
    let mut theirs: Vec<(f64, f64, String)> = Vec::new();
    for (s, e, w) in words {
        let u = units(w);
        let n = u.len().max(1) as f64;
        for (k, r) in u.iter().enumerate() {
            let (a, b) = (s + (e - s) * k as f64 / n, s + (e - s) * (k + 1) as f64 / n);
            theirs.push((a, b, norm(&w[r.0..r.1])));
        }
    }
    let (n, m) = (ours.len(), theirs.len());
    if n == 0 || m == 0 || n * m > 4_000_000 {
        return None;
    }
    // longest common subsequence of equal words
    let w = m + 1;
    let mut dp = vec![0u16; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * w + j] = if !ours[i].2.is_empty() && ours[i].2 == theirs[j].2 { dp[(i + 1) * w + j + 1] + 1 } else { dp[(i + 1) * w + j].max(dp[i * w + j + 1]) };
        }
    }
    let mut matched: Vec<Option<(f64, f64)>> = vec![None; n];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if !ours[i].2.is_empty() && ours[i].2 == theirs[j].2 {
            matched[i] = Some((theirs[j].0, theirs[j].1));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    let hits = matched.iter().filter(|m| m.is_some()).count();
    if hits < 8 || (hits as f64) < n as f64 * 0.4 {
        return None;
    }
    // the same recording: a line's first word starts about when the line does
    let mut lead: Vec<f64> = Vec::new();
    for (k, (li, ui, _)) in ours.iter().enumerate() {
        if *ui == 0 {
            if let Some((s, _)) = matched[k] {
                lead.push(s - lines[*li].0);
            }
        }
    }
    lead.sort_by(|a, b| a.total_cmp(b));
    if lead.is_empty() || lead[lead.len() / 2].abs() > 0.8 {
        return None;
    }
    // per line
    let mut out: Times = vec![Vec::new(); lines.len()];
    let mut k = 0;
    for (li, (start, text)) in lines.iter().enumerate() {
        let u = units(text);
        let span: Vec<Option<(f64, f64)>> = matched[k..k + u.len()].to_vec();
        k += u.len();
        let got = span.iter().filter(|m| m.is_some()).count();
        if u.is_empty() || (got as f64) < u.len() as f64 * 0.6 {
            continue;
        }
        let gap = lines.get(li + 1).map(|l| l.0 - start).unwrap_or(8.0);
        let est = estimate(text, *start, gap);
        let mut t: Vec<f64> = span.iter().map(|m| m.map(|x| x.0).unwrap_or(f64::NAN)).collect();
        // words not found: between their found neighbours, by the estimate's spacing
        for q in 0..t.len() {
            if !t[q].is_nan() {
                continue;
            }
            let before = (0..q).rev().find(|&p| !t[p].is_nan());
            let after = (q + 1..t.len()).find(|&p| !t[p].is_nan());
            t[q] = match (before, after) {
                (Some(b), Some(a)) => t[b] + (t[a] - t[b]) * (est[q] - est[b]) / (est[a] - est[b]).max(1e-6),
                (Some(b), None) => t[b] + (est[q] - est[b]),
                (None, Some(a)) => t[a] - (est[a] - est[q]),
                (None, None) => est[q],
            };
        }
        for q in 1..t.len() {
            t[q] = t[q].max(t[q - 1]);
        }
        let end = span.iter().rev().find_map(|m| m.map(|x| x.1)).unwrap_or(est[u.len()]).max(*t.last().unwrap());
        t.push(end);
        out[li] = t;
    }
    out.iter().any(|l| !l.is_empty()).then_some(out)
}

// ------------------------------------------------------------------------------ timing nudges

/// Your own nudge of a song's lyrics (seconds; more = the lyrics come later), for audio that's
/// a little ahead of or behind them.
pub fn offsets() -> std::collections::HashMap<String, f64> {
    crate::store::load_json(&data_dir().join("lyric-offsets.json"))
}

pub fn save_offsets(o: &std::collections::HashMap<String, f64>) {
    crate::store::save_json(&data_dir().join("lyric-offsets.json"), o);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_split_words_and_cjk() {
        let t = "Hello world";
        assert_eq!(units(t).iter().map(|r| &t[r.0..r.1]).collect::<Vec<_>>(), ["Hello", "world"]);
        let j = "君の名は hope";
        assert_eq!(units(j).iter().map(|r| &j[r.0..r.1]).collect::<Vec<_>>(), ["君", "の", "名", "は", "hope"]);
    }

    #[test]
    fn estimate_moves_forward_and_stays_in_the_line() {
        let line = "Never gonna give you up";
        let e = estimate(line, 10.0, 4.0);
        assert_eq!(e.len(), 6);
        assert_eq!(e[0], 10.0);
        assert!(e.windows(2).all(|w| w[1] >= w[0]));
        assert!(*e.last().unwrap() <= 13.9);
        // lights up whole words in order
        assert_eq!(sung_upto(line, &e, 9.0), 0);
        assert_eq!(sung_upto(line, &e, 10.0), 5);
        assert_eq!(sung_upto(line, &e, 99.0), line.len());
        assert_eq!(syllables("little"), 2.0);
        assert_eq!(syllables("make"), 1.0);
        assert_eq!(syllables("beautiful"), 3.0);
    }

    #[test]
    fn yrc_parses() {
        let w = parse_yrc("{\"t\":0,\"c\":[{\"tx\":\"credits\"}]}\n[26180,5250](26180,1170,0)Whenever (27350,810,0)I'm (28160,1260,0)alone\n");
        assert_eq!(w.len(), 3);
        assert_eq!(w[1], (27.35, 28.16, "I'm".to_string()));
    }

    #[test]
    fn netease_words_line_up_with_other_lyrics() {
        let lines = vec![(26.2, "Whenever I'm alone with you".to_string()), (34.4, "You make me feel like I am home again".to_string()), (40.0, String::new())];
        let words = parse_yrc("[26180,5250](26180,1170,0)Whenever (27350,810,0)I'm (28160,1260,0)alone (29420,450,0)with (29870,1560,0)you\n[34370,5280](34370,360,0)You (34730,450,0)make (35180,240,0)me (35420,840,0)feel (36260,600,0)like (36860,240,0)I (37100,660,0)am (37760,660,0)home (38420,1230,0)again");
        let t = align(&lines, &words).unwrap();
        assert_eq!(t[0], vec![26.18, 27.35, 28.16, 29.42, 29.87, 31.43]);
        assert_eq!(t[1].len(), 10);
        assert!(t[2].is_empty());
        // another recording (everything 5 s later): not used
        let late: Vec<Word> = words.iter().map(|(s, e, w)| (s + 5.0, e + 5.0, w.clone())).collect();
        assert!(align(&lines, &late).is_none());
    }

    /// DKFM_LIB=<library.json> DKFM_LYR=<lyrics folder> cargo test --release real_songs -- --ignored --nocapture
    /// looks up word times for songs with synced lyrics and says how many lines got exact times
    #[test]
    #[ignore]
    fn real_songs() {
        let (_g, _dir) = crate::store::test_profile("wordsync");
        let lib: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("DKFM_LIB").unwrap()).unwrap()).unwrap();
        let lyr = std::path::PathBuf::from(std::env::var("DKFM_LYR").unwrap());
        let tracks: Vec<Track> = lib["tracks"].as_object().unwrap().values().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect();
        let (mut songs, mut exact_songs, mut lines_all, mut lines_exact) = (0, 0, 0, 0);
        for t in tracks.iter().step_by(7) {
            let Ok(b) = std::fs::read(lyr.join(format!("{}.json", t.id))) else { continue };
            let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
            if v["kind"] != "synced" {
                continue;
            }
            let lines: Vec<(f64, String)> = serde_json::from_value(v["lines"].clone()).unwrap();
            songs += 1;
            let sung = lines.iter().filter(|l| !l.1.trim().is_empty()).count();
            lines_all += sung;
            if let Some(x) = exact(t, &lines) {
                exact_songs += 1;
                let n = x.iter().filter(|l| !l.is_empty()).count();
                lines_exact += n;
                println!("EXACT {n:3}/{sung:3} {} - {}", t.artist, t.title);
            } else {
                println!("est         {} - {}", t.artist, t.title);
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
            if songs >= 30 {
                break;
            }
        }
        println!("{exact_songs}/{songs} songs exact, {lines_exact}/{lines_all} lines");
    }
}

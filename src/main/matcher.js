// Scores YouTube candidates against a Spotify track to pick the most faithful upload.
// Prefers YouTube Music "song" results (official studio audio), checks the duration, and
// penalises variants the original isn't (live, remix, sped up, nightcore, cover...).

const norm = (s) =>
  String(s || '')
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/&/g, ' and ')
    .replace(/[^\p{L}\p{N}\s]/gu, ' ')
    .replace(/\s+/g, ' ')
    .trim();

const tokens = (s) => norm(s).split(' ').filter((t) => t && !STOP.has(t));
const STOP = new Set(['the', 'a', 'an', 'feat', 'ft', 'featuring', 'with', 'and', 'x', 'official', 'audio', 'video', 'music', 'lyrics', 'lyric', 'hd', 'hq', '4k', 'remastered', 'remaster', 'version', 'from', 'of']);

// Strip "(feat. X)", "- Remastered 2011" etc. from the core title for matching.
const coreTitle = (t) =>
  String(t || '')
    .replace(/\s*[([](feat|ft|with)\.?[^)\]]*[)\]]/gi, '')
    .replace(/\s+-\s+(\d{4}\s+)?remaster(ed)?.*$/i, '')
    .trim();

const VARIANTS = ['live', 'cover', 'remix', 'sped up', 'speed up', 'slowed', 'reverb', 'nightcore', 'karaoke', 'instrumental', '8d', 'acoustic', 'extended', 'mashup', 'bass boosted', 'reaction', 'tutorial', '1 hour', 'loop', 'clean version', 'piano', 'edit'];

function score(cand, track) {
  const want = track.durationMs ? track.durationMs / 1000 : null;
  const ct = norm(cand.title);
  const cc = norm(cand.channel);
  const titleToks = tokens(coreTitle(track.title));
  const artistToks = track.artists.flatMap(tokens);
  const candToks = new Set([...tokens(cand.title), ...tokens(cand.channel)]);
  let s = 0;
  const why = [];

  const tHit = titleToks.length ? titleToks.filter((t) => candToks.has(t)).length / titleToks.length : 0;
  s += tHit * 45;

  const mainArtist = norm(track.artists[0]);
  const aHit = artistToks.length ? artistToks.filter((t) => candToks.has(t)).length / artistToks.length : 0;
  s += aHit * 15;
  if (cc && (cc === mainArtist || cc === `${mainArtist} topic` || cc.replace(/ topic$/, '') === mainArtist)) {
    s += 15;
    why.push('artist channel');
  }

  if (cand.source === 'music') {
    // YT Music "songs" are official audio-only uploads (no video intros/skits). Flat search
    // results carry no channel/duration, so trust the artist and rely on the rank order;
    // verify() probes the real duration before anything is downloaded.
    s += 25 + ([20, 10, 5][cand.rank] ?? 0);
    if (aHit < 0.5) s += 15;
    why.push('YT Music song');
  }

  if (want && cand.duration) {
    const d = Math.abs(cand.duration - want);
    if (d <= 3) { s += 25; why.push('exact length'); }
    else if (d <= 8) s += 14;
    else if (d <= 20) s -= 5;
    else { s -= 45; why.push(`length off ${Math.round(d)}s`); }
  }

  const origT = norm(track.title);
  for (const v of VARIANTS) {
    const re = new RegExp(`\\b${v}\\b`);
    if (re.test(ct) && !re.test(origT)) {
      s -= 30;
      why.push(v);
    }
  }
  if (/\bofficial (audio|lyric)/.test(ct)) s += 6;
  if (/\b(music video|official video)\b/.test(ct)) s -= 3; // videos often have intros/outros

  return { score: Math.round(s), why };
}

function rank(cands, track) {
  const seen = new Map();
  for (const c of cands) {
    const prev = seen.get(c.id);
    // Merge YT Music (better source flag) with regular search (has duration/channel).
    seen.set(c.id, prev ? { ...prev, ...c, source: prev.source === 'music' || c.source === 'music' ? 'music' : 'yt', duration: prev.duration || c.duration, channel: prev.channel || c.channel, rank: prev.rank ?? c.rank } : c);
  }
  return [...seen.values()]
    .map((c) => ({ ...c, ...score(c, track) }))
    .sort((a, b) => b.score - a.score);
}

module.exports = { rank, score, norm };

// Universal importer: Spotify (playlist/album/track/profile), YouTube & YouTube Music
// (video/playlist/album/mix), SoundCloud (track/set), plus "More like this" radio.
// Every source resolves to the same "collection" shape the downloader understands.
const crypto = require('crypto');
const yt = require('./ytdlp');
const { fetchSpotify, fetchSpotifyProfile, parseSpotifyUrl } = require('./spotify');

const shortHash = (s) => crypto.createHash('sha1').update(String(s)).digest('hex').slice(0, 12);

function detect(url) {
  const u = String(url).trim();
  if (/open\.spotify\.com\/(intl-[a-z-]+\/)?user\//.test(u)) return 'spotify-profile';
  if (parseSpotifyUrl(u)) return 'spotify';
  if (/(youtube\.com|youtu\.be)\//.test(u)) return 'youtube';
  if (/soundcloud\.com\//.test(u)) return 'soundcloud';
  return null;
}

// "Artist - Title (Official Video)" -> { artist, title }
// Any bracketed bit about the upload itself: (Official HD Video), [Lyric Video], ( Video Lyric ),
// (Performance Video), (Visualizer), (4K Remaster), (Explicit)...
const JUNK = /\s*[([][^)\]]*\b(video|audio|lyrics?|visuali[sz]er|official|4k|8k|hd|hq|explicit|clean|oficial|directed|dir\.|shot by)\b[^)\]]*[)\]]/gi;
function splitTitle(raw, channel) {
  let title = String(raw || '').replace(JUNK, '').replace(/\s{2,}/g, ' ').trim();
  let artist = String(channel || '').replace(/\s*-\s*Topic$/i, '').replace(/VEVO$/i, '').trim();
  const m = title.match(/^(.+?)\s+[-–—]\s+(.+)$/);
  // "Artist - Title" uploads (not YT Music "Topic" channels, whose titles are already clean)
  if (m && !/ - Topic$/i.test(channel || '')) {
    const low = (x) => x.toLowerCase();
    // "Title - Artist" uploads: the channel name appears on the right-hand side.
    const swapped = artist && low(m[2]).includes(low(artist)) && !low(m[1]).includes(low(artist));
    artist = (swapped ? m[2] : m[1]).trim();
    title = (swapped ? m[1] : m[2]).trim();
  }
  return { artist: artist || 'Unknown Artist', title: title || 'Untitled' };
}

const bestThumb = (o) => {
  const list = o?.thumbnails || [];
  if (list.length) return [...list].sort((a, b) => (b.width || 0) - (a.width || 0))[0].url;
  return o?.thumbnail || null;
};

async function fetchGeneric(url, { name, type, limit = 500 } = {}) {
  await yt.ensure();
  const isSC = /soundcloud\.com/.test(url);
  // SoundCloud sets only return URLs in flat mode, so resolve them fully (slower but complete).
  const raw = await yt.run([...yt.baseArgs(), ...(isSC ? [] : ['--flat-playlist']), '-J', '--playlist-end', String(limit), url]);
  const j = JSON.parse(raw);
  const entries = (j.entries || [j]).filter((e) => e && (e.id || e.url));
  if (!entries.length) throw new Error('Nothing playable found at that link.');
  const tracks = entries.map((e) => {
    const { artist, title } = e.artist && e.track ? { artist: e.artist, title: e.track } : splitTitle(e.title, e.channel || e.uploader || j.uploader);
    const youtubeId = !isSC && e.id && e.id.length === 11 ? e.id : null;
    return {
      title,
      artists: artist.split(/\s*,\s*|\s+&\s+|\s+x\s+/i).filter(Boolean).slice(0, 4),
      album: e.album || '',
      durationMs: e.duration ? Math.round(e.duration * 1000) : null,
      directUrl: e.webpage_url || (youtubeId ? `https://music.youtube.com/watch?v=${youtubeId}` : e.url),
      youtubeId,
      sourceKey: youtubeId ? `yt:${youtubeId}` : `url:${e.webpage_url || e.url}`,
      cover: isSC ? bestThumb(e) : null,
    };
  });
  const single = !j.entries;
  // YouTube Music names albums "Album - Title" / "EP - Title" / "Single - Title".
  const rel = String(j.title || '').match(/^(Album|EP|Single) - (.+)$/);
  if (rel) tracks.forEach((t) => (t.album ||= rel[2]));
  return {
    type: type || (single ? 'track' : rel ? 'album' : 'playlist'),
    id: (isSC ? 'sc' : 'yt') + shortHash(url),
    name: name || (rel ? rel[2] : j.title) || tracks[0].title,
    owner: (j.uploader || j.channel || '').replace(/\s*-\s*Topic$/i, ''),
    cover: bestThumb(j) || (isSC ? tracks[0].cover : null),
    tracks,
    complete: true,
    via: isSC ? 'soundcloud' : 'youtube',
    source: isSC ? 'soundcloud' : 'youtube',
    url,
  };
}

async function fetchAny(url, creds) {
  const kind = detect(url);
  if (kind === 'spotify-profile') return { kind: 'profile', ...(await fetchSpotifyProfile(url, creds)) };
  if (kind === 'spotify') {
    const c = await fetchSpotify(url, creds);
    return { ...c, source: 'spotify', tracks: c.tracks.map((t) => ({ ...t, sourceKey: `sp:${t.spotifyId}` })) };
  }
  if (kind === 'youtube' || kind === 'soundcloud') return fetchGeneric(url);
  throw new Error('Paste a Spotify, YouTube, YouTube Music or SoundCloud link.');
}

// "More like this": YouTube Music's radio mix seeded from the song.
async function radio(track) {
  await yt.ensure();
  let seed = track.youtubeId;
  if (!seed) {
    const q = encodeURIComponent(`${track.artist} ${track.title}`);
    const raw = await yt.run([...yt.baseArgs(), '--flat-playlist', '-J', '--playlist-end', '1', `https://music.youtube.com/search?q=${q}#songs`]);
    seed = JSON.parse(raw).entries?.[0]?.id;
    if (!seed) throw new Error('Could not find that song on YouTube Music.');
  }
  const col = await fetchGeneric(`https://music.youtube.com/watch?v=${seed}&list=RDAMVM${seed}`, { name: `Radio · ${track.title}`, type: 'radio', limit: 51 });
  col.tracks = col.tracks.filter((t) => t.youtubeId !== seed).slice(0, 50);
  col.owner = `Songs like ${track.title} — ${track.artist}`;
  col.id = 'radio' + seed;
  return col;
}

module.exports = { detect, fetchAny, fetchGeneric, radio, splitTitle };

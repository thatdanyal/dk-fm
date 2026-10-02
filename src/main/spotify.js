// Spotify import. Two strategies:
//  1. Official Web API (client-credentials) when the user has entered a Client ID/Secret: full
//     playlists with album + cover art.
//  2. Zero-setup fallback: the public embed page. Capped by Spotify at 100 tracks, no album info.

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130 Safari/537.36';

function parseSpotifyUrl(input) {
  const s = String(input).trim();
  let m = s.match(/^spotify:(playlist|album|track):([A-Za-z0-9]+)/);
  if (m) return { type: m[1], id: m[2] };
  m = s.match(/open\.spotify\.com\/(?:intl-[a-z-]+\/)?(?:embed\/)?(playlist|album|track)\/([A-Za-z0-9]+)/);
  if (m) return { type: m[1], id: m[2] };
  return null;
}

let tokenCache = { key: '', token: '', exp: 0 };

async function getApiToken(clientId, clientSecret) {
  const key = clientId + ':' + clientSecret;
  if (tokenCache.key === key && Date.now() < tokenCache.exp - 60_000) return tokenCache.token;
  const r = await fetch('https://accounts.spotify.com/api/token', {
    method: 'POST',
    headers: {
      Authorization: 'Basic ' + Buffer.from(key).toString('base64'),
      'Content-Type': 'application/x-www-form-urlencoded',
    },
    body: 'grant_type=client_credentials',
  });
  if (!r.ok) throw new Error(`Spotify auth failed (${r.status}). Check your Client ID / Secret in Settings.`);
  const j = await r.json();
  tokenCache = { key, token: j.access_token, exp: Date.now() + j.expires_in * 1000 };
  return j.access_token;
}

async function api(token, url) {
  for (let attempt = 0; attempt < 4; attempt++) {
    const r = await fetch(url.startsWith('http') ? url : 'https://api.spotify.com/v1' + url, {
      headers: { Authorization: 'Bearer ' + token },
    });
    if (r.status === 429) {
      const wait = Number(r.headers.get('retry-after') || 2);
      await new Promise((res) => setTimeout(res, Math.min(wait, 30) * 1000));
      continue;
    }
    if (!r.ok) throw new Error(`Spotify API error ${r.status} for ${url}`);
    return r.json();
  }
  throw new Error('Spotify rate limit — try again in a minute.');
}

const bestImage = (imgs) => (imgs && imgs.length ? [...imgs].sort((a, b) => (b.width || 0) - (a.width || 0))[0].url : null);

function apiTrack(t, albumOverride) {
  const album = albumOverride || t.album || {};
  return {
    spotifyId: t.id,
    title: t.name,
    artists: (t.artists || []).map((a) => a.name),
    album: album.name || '',
    albumArtist: album.artists?.[0]?.name || '',
    year: (album.release_date || '').slice(0, 4) || null,
    trackNo: t.track_number || null,
    durationMs: t.duration_ms,
    explicit: !!t.explicit,
    isrc: t.external_ids?.isrc || null,
    cover: bestImage(album.images),
  };
}

async function fetchViaApi(ref, creds) {
  const token = await getApiToken(creds.clientId, creds.clientSecret);
  if (ref.type === 'track') {
    const t = await api(token, `/tracks/${ref.id}`);
    return { type: 'track', id: ref.id, name: t.name, owner: t.artists?.[0]?.name, cover: bestImage(t.album?.images), tracks: [apiTrack(t)], complete: true };
  }
  if (ref.type === 'album') {
    const a = await api(token, `/albums/${ref.id}`);
    const tracks = [];
    let page = a.tracks;
    while (page) {
      for (const t of page.items) tracks.push(apiTrack(t, a));
      page = page.next ? await api(token, page.next) : null;
    }
    return { type: 'album', id: ref.id, name: a.name, owner: a.artists?.[0]?.name, cover: bestImage(a.images), tracks, complete: true };
  }
  const p = await api(token, `/playlists/${ref.id}?fields=name,owner(display_name),images,tracks.total`);
  const tracks = [];
  let url = `/playlists/${ref.id}/tracks?limit=100&additional_types=track`;
  while (url) {
    const page = await api(token, url);
    for (const it of page.items) if (it.track && it.track.type === 'track' && !it.is_local) tracks.push(apiTrack(it.track));
    url = page.next;
  }
  return { type: 'playlist', id: ref.id, name: p.name, owner: p.owner?.display_name, cover: bestImage(p.images), tracks, complete: true };
}

async function fetchViaEmbed(ref) {
  const r = await fetch(`https://open.spotify.com/embed/${ref.type}/${ref.id}`, { headers: { 'user-agent': UA } });
  if (!r.ok) throw new Error(`Spotify returned ${r.status}. Is the playlist public?`);
  const html = await r.text();
  const m = html.match(/<script id="__NEXT_DATA__" type="application\/json">(.+?)<\/script>/s);
  if (!m) throw new Error('Could not read the Spotify page (format changed?). Add API keys in Settings as a fallback.');
  const e = JSON.parse(m[1])?.props?.pageProps?.state?.data?.entity;
  if (!e) throw new Error('Spotify did not return any data for that link.');
  const cover = bestImage(e.coverArt?.sources || e.visualIdentity?.image);
  if (ref.type === 'track') {
    return {
      type: 'track', id: ref.id, name: e.name, owner: e.artists?.[0]?.name, cover,
      tracks: [{ spotifyId: ref.id, title: e.name, artists: (e.artists || []).map((a) => a.name), album: '', durationMs: e.duration, cover }],
      complete: true,
    };
  }
  const tracks = (e.trackList || []).map((t) => ({
    spotifyId: (t.uri || '').split(':').pop(),
    title: t.title,
    artists: String(t.subtitle || '').split(/,\s*/).filter(Boolean),
    album: ref.type === 'album' ? e.name : '',
    albumArtist: ref.type === 'album' ? e.subtitle : '',
    durationMs: t.duration,
    explicit: !!t.isExplicit,
    cover: ref.type === 'album' ? cover : null,
  }));
  return { type: ref.type, id: ref.id, name: e.name, owner: e.subtitle, cover, tracks, complete: tracks.length < 100 };
}

async function fetchSpotify(url, creds) {
  const ref = parseSpotifyUrl(url);
  if (!ref) throw new Error('That does not look like a Spotify playlist, album or track link.');
  if (creds?.clientId && creds?.clientSecret) {
    try {
      return { ...(await fetchViaApi(ref, creds)), via: 'api', url };
    } catch (err) {
      const fb = await fetchViaEmbed(ref).catch(() => null);
      if (fb) return { ...fb, via: 'embed', url, warning: err.message };
      throw err;
    }
  }
  return { ...(await fetchViaEmbed(ref)), via: 'embed', url };
}

module.exports = { fetchSpotify, parseSpotifyUrl };

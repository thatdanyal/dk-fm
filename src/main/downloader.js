// Download pipeline: Spotify track -> search YouTube -> rank -> verify -> download -> tag -> library.
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawn } = require('child_process');
const yt = require('./ytdlp');
const { rank } = require('./matcher');

const FORMATS = {
  'mp3-320': { ext: 'mp3', args: ['-x', '--audio-format', 'mp3', '--audio-quality', '320K'] },
  'mp3-v0': { ext: 'mp3', args: ['-x', '--audio-format', 'mp3', '--audio-quality', '0'] },
  m4a: { ext: 'm4a', args: ['-f', 'bestaudio[ext=m4a]/bestaudio', '-x', '--audio-format', 'm4a', '--audio-quality', '0'] },
};

const sanitize = (s) => String(s || '').replace(/[<>:"/\\|?*\u0000-\u001f]/g, '').replace(/\s+/g, ' ').replace(/[. ]+$/, '').trim().slice(0, 120) || 'Untitled';

class Downloader {
  constructor({ emit, library, settings }) {
    this.emit = emit;
    this.library = library;
    this.settings = settings;
    this.jobs = new Map(); // jobId -> job
    this.queue = [];
    this.active = 0;
  }

  list() {
    return [...this.jobs.values()].map(publicJob);
  }

  // collection = result of fetchSpotify; selected = indices to download (default all)
  start(collection, selected) {
    const jobId = `${collection.type}-${collection.id}`;
    const existing = this.jobs.get(jobId);
    if (existing) this.cancel(jobId);
    const fmt = this.settings.get('downloadFormat') || 'mp3-320';
    const folder = path.join(this.settings.get('downloadDir'), sanitize(collection.type === 'track' ? 'Singles' : collection.name));
    const job = {
      id: jobId,
      name: collection.name,
      cover: collection.cover,
      type: collection.type,
      url: collection.url,
      folder,
      fmt,
      createdAt: Date.now(),
      abort: new AbortController(),
      tracks: collection.tracks.map((t, i) => ({
        ...t,
        idx: i,
        status: selected && !selected.includes(i) ? 'skipped' : 'queued',
        progress: 0,
      })),
    };
    this.jobs.set(jobId, job);
    // A playlist in the library mirrors the Spotify collection.
    if (collection.type !== 'track') {
      this.library.upsertPlaylist({ id: 'sp-' + jobId, name: collection.name, source: 'spotify', spotifyUrl: collection.url, cover: collection.cover, trackIds: this.library.store.data.playlists.find((p) => p.id === 'sp-' + jobId)?.trackIds || [] });
    }
    for (const t of job.tracks) if (t.status === 'queued') this.queue.push([job, t]);
    this.emit('dl:job', publicJob(job));
    this.pump();
    return publicJob(job);
  }

  retry(jobId, idx, forcedVideoId) {
    const job = this.jobs.get(jobId);
    const t = job?.tracks[idx];
    if (!t || ['searching', 'downloading', 'tagging'].includes(t.status)) return;
    if (job.abort.signal.aborted) job.abort = new AbortController();
    Object.assign(t, { status: 'queued', progress: 0, error: null, forcedVideoId: forcedVideoId || null });
    this.queue.push([job, t]);
    this.update(job, t);
    this.pump();
  }

  cancel(jobId) {
    const job = this.jobs.get(jobId);
    if (!job) return;
    job.abort.abort();
    this.queue = this.queue.filter(([j]) => j !== job);
    for (const t of job.tracks) if (['queued', 'searching', 'downloading', 'tagging'].includes(t.status)) { t.status = 'cancelled'; this.update(job, t); }
  }

  clear(jobId) {
    this.cancel(jobId);
    this.jobs.delete(jobId);
    this.emit('dl:removed', jobId);
  }

  pump() {
    const max = Math.max(1, Math.min(6, Number(this.settings.get('downloadConcurrency')) || 3));
    while (this.active < max && this.queue.length) {
      const [job, t] = this.queue.shift();
      if (job.abort.signal.aborted || t.status !== 'queued') continue;
      this.active++;
      this.process(job, t)
        .catch((e) => {
          if (job.abort.signal.aborted) t.status = 'cancelled';
          else { t.status = 'failed'; t.error = e.message; }
          this.update(job, t);
        })
        .finally(() => { this.active--; this.pump(); });
    }
  }

  update(job, t) {
    this.emit('dl:track', { jobId: job.id, track: publicTrack(t) });
  }

  async process(job, t) {
    const signal = job.abort.signal;
    await yt.ensure();
    const fmt = FORMATS[job.fmt] || FORMATS['mp3-320'];
    const artist = t.artists.join(', ');
    fs.mkdirSync(job.folder, { recursive: true });
    const outFile = path.join(job.folder, `${sanitize(t.artists[0])} - ${sanitize(t.title)}.${fmt.ext}`);

    if (fs.existsSync(outFile) && !t.forcedVideoId) {
      const [added] = await this.library.addFiles([outFile], { spotifyId: t.spotifyId });
      t.status = 'done';
      t.note = 'Already downloaded';
      t.trackId = added?.id;
      t.file = outFile;
      this.linkPlaylist(job);
      return this.update(job, t);
    }

    // ---- search & pick ----
    t.status = 'searching';
    this.update(job, t);
    const want = t.durationMs ? t.durationMs / 1000 : null;
    let order;
    if (t.forcedVideoId) {
      order = [{ id: t.forcedVideoId, title: '(manual pick)', source: 'manual', score: 100 }];
    } else {
      const cands = rank(await this.search(t, signal), t);
      t.candidates = cands.slice(0, 6).map(({ id, title, channel, duration, score, source }) => ({ id, title, channel, duration, score, source }));
      order = cands.filter((c) => c.score >= 20 && (!want || !c.duration || Math.abs(c.duration - want) <= 20)).slice(0, 3);
      if (!order.length && cands[0]?.score >= 35) order = [cands[0]];
      if (!order.length) throw new Error('No confident match found on YouTube');
    }

    // ---- download ----
    // Candidates whose length is unknown (YT Music search results) get a --match-filters
    // length check inside the download call itself, so a wrong version is rejected without an
    // extra yt-dlp launch; we then fall through to the next candidate.
    t.status = 'downloading';
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'dkfm-'));
    try {
      let pick = null;
      let audio = null;
      for (const c of order) {
        t.match = { id: c.id, title: c.title, channel: c.channel, score: c.score, source: c.source };
        t.lowConfidence = c.score < 45;
        t.progress = 0;
        this.update(job, t);
        const filter = want && !c.duration && c.source !== 'manual'
          ? ['--match-filters', `duration>=${Math.floor(want - 12)} & duration<=${Math.ceil(want + 12)}`]
          : [];
        let lastEmit = 0;
        await yt.run(
          [
            ...yt.baseArgs(),
            '--no-playlist',
            ...filter,
            ...fmt.args,
            '--write-thumbnail', '--convert-thumbnails', 'jpg',
            '--newline', '--progress-template', 'download:DKP %(progress._percent_str)s',
            '-o', path.join(tmpDir, 'audio.%(ext)s'),
            '-o', 'thumbnail:' + path.join(tmpDir, 'thumb.%(ext)s'),
            `https://music.youtube.com/watch?v=${c.id}`,
          ],
          {
            signal,
            onLine: (line) => {
              const m = line.match(/DKPs+([d.]+)%/);
              if (m && Date.now() - lastEmit > 300) {
                lastEmit = Date.now();
                t.progress = Math.min(99, parseFloat(m[1]));
                this.update(job, t);
              }
            },
          }
        ).catch((e) => { if (signal.aborted || order.length === 1) throw e; });
        audio = fs.readdirSync(tmpDir).find((f) => f.startsWith('audio.') && f.endsWith('.' + fmt.ext));
        if (audio) { pick = c; break; }
      }
      if (!audio) throw new Error('No version with the right length found on YouTube');

      // ---- tag ----
      t.status = 'tagging';
      t.progress = 100;
      this.update(job, t);
      let cover = null;
      let coverIsSquare = false;
      if (t.cover) {
        try {
          const r = await fetch(t.cover, { signal });
          if (r.ok) {
            cover = path.join(tmpDir, 'cover.jpg');
            fs.writeFileSync(cover, Buffer.from(await r.arrayBuffer()));
            coverIsSquare = true;
          }
        } catch { /* fall back to YouTube thumbnail */ }
      }
      if (!cover && fs.existsSync(path.join(tmpDir, 'thumb.jpg'))) cover = path.join(tmpDir, 'thumb.jpg');

      await tag(path.join(tmpDir, audio), outFile, {
        title: t.title,
        artist,
        album: t.album || (job.type === 'playlist' ? job.name : t.title),
        album_artist: t.albumArtist || t.artists[0],
        date: t.year || '',
        track: t.trackNo || '',
        comment: `DK.FM · youtube:${pick.id}${t.spotifyId ? ' · spotify:' + t.spotifyId : ''}`,
      }, cover, coverIsSquare, fmt.ext, signal);

      const [added] = await this.library.addFiles([outFile], { spotifyId: t.spotifyId, youtubeId: pick.id });
      t.trackId = added?.id;
      t.file = outFile;
      t.status = 'done';
      this.linkPlaylist(job);
      this.update(job, t);
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  }

  async search(t, signal) {
    const q = `${t.artists.slice(0, 2).join(' ')} ${t.title}`.replace(/\s+/g, ' ');
    const enc = encodeURIComponent(q);
    const [music, plain] = await Promise.all([
      yt.run([...yt.baseArgs(), '--flat-playlist', '-J', '--playlist-end', '5', `https://music.youtube.com/search?q=${enc}#songs`], { signal }).catch(() => null),
      yt.run([...yt.baseArgs(), '--flat-playlist', '-J', `ytsearch8:${q}`], { signal }).catch(() => null),
    ]);
    const out = [];
    const take = (raw, source) => {
      if (!raw) return;
      try {
        let rank = 0;
        for (const e of JSON.parse(raw).entries || []) {
          if (!e?.id || e.id.length !== 11) continue;
          out.push({ id: e.id, title: e.title || '', channel: e.channel || e.uploader || '', duration: e.duration || null, source, rank: rank++ });
        }
      } catch { /* ignore */ }
    };
    take(music, 'music');
    take(plain, 'yt');
    if (!out.length) throw new Error('YouTube search returned nothing');
    return out;
  }

  linkPlaylist(job) {
    if (job.type === 'track') return;
    const ids = job.tracks.filter((x) => x.status === 'done' && x.trackId).map((x) => x.trackId);
    const pl = this.library.store.data.playlists.find((p) => p.id === 'sp-' + job.id);
    // Keep any tracks from previous syncs that aren't part of this run.
    const keep = (pl?.trackIds || []).filter((id) => !ids.includes(id));
    this.library.upsertPlaylist({ id: 'sp-' + job.id, trackIds: [...ids, ...keep] });
  }
}

function tag(input, output, meta, cover, coverIsSquare, ext, signal) {
  const args = ['-y', '-hide_banner', '-loglevel', 'error', '-i', input];
  if (cover) args.push('-i', cover);
  args.push('-map', '0:a');
  if (cover) {
    args.push('-map', '1:v', '-c:a', 'copy', '-c:v', 'mjpeg', '-disposition:v', 'attached_pic');
    if (!coverIsSquare) args.push('-vf', "crop='min(iw,ih)':'min(iw,ih)',scale=600:600");
  } else {
    args.push('-c', 'copy');
  }
  args.push('-map_metadata', '-1');
  for (const [k, v] of Object.entries(meta)) if (v !== '' && v != null) args.push('-metadata', `${k}=${v}`);
  if (ext === 'mp3') args.push('-id3v2_version', '3', '-metadata:s:v', 'title=Album cover', '-metadata:s:v', 'comment=Cover (front)');
  const tmpOut = output + '.part.' + ext;
  args.push(tmpOut);
  return new Promise((resolve, reject) => {
    const p = spawn(yt.ffmpegPath(), args, { windowsHide: true });
    let err = '';
    p.stderr.on('data', (d) => (err += d));
    const abort = () => p.kill();
    signal?.addEventListener('abort', abort, { once: true });
    p.on('error', reject);
    p.on('close', (code) => {
      signal?.removeEventListener('abort', abort);
      if (code === 0) {
        fs.renameSync(tmpOut, output);
        resolve();
      } else {
        fs.rmSync(tmpOut, { force: true });
        reject(new Error('Tagging failed: ' + err.trim().split('\n').pop()));
      }
    });
  });
}

const publicTrack = (t) => {
  const { forcedVideoId, ...rest } = t;
  return rest;
};
const publicJob = (j) => ({ id: j.id, name: j.name, cover: j.cover, type: j.type, url: j.url, folder: j.folder, fmt: j.fmt, createdAt: j.createdAt, tracks: j.tracks.map(publicTrack) });

module.exports = { Downloader, FORMATS };

//! Queue + playback control. A controller thread reacts to engine events (gapless advances,
//! song ends, errors) and does the bookkeeping (play counts, listening history, sleep timer,
//! session save), so playback logic runs even while the window is hidden in the tray.
use crate::analysis::Analyzer;
use crate::audio::{Cmd, Engine, Event, Status};
use crate::library::{main_artist, Library};
use crate::store::{now_secs, Eq, LibraryData, PlayerOpts, Settings};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
pub enum Sleep {
    Off,
    EndOfTrack,
    At(Instant),
}

pub struct State {
    pub queue: Vec<String>,
    pub index: isize,
    unshuffled: Option<Vec<String>>,
    pub opts: PlayerOpts,
    pub eq: Eq,
    pub muted: bool,
    pub sleep: Sleep,
    pub queue_gen: u64,
    error_streak: u32,
    // listening bookkeeping
    listen_id: Option<String>,
    listen_start: i64,
    listened: f64,
    counted: bool,
    pre_analyzed: Option<String>,
}

pub struct Player {
    pub st: Mutex<State>,
    pub engine: Engine,
    lib: Arc<Library>,
    pub analyzer: Arc<Analyzer>,
    settings: Arc<Mutex<Settings>>,
    pub settings_dirty: Arc<AtomicBool>,
    pub notices: Mutex<Vec<String>>,
    pub repaint: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl Player {
    pub fn new(lib: Arc<Library>, settings: Arc<Mutex<Settings>>, settings_dirty: Arc<AtomicBool>) -> Arc<Self> {
        let engine = Engine::start();
        let (opts, eq, session) = {
            let s = settings.lock();
            (s.player.clone(), s.eq.clone(), s.session.clone())
        };
        // gains measured in the background are applied to whatever is playing / queued next
        let tx_engine = engine.sender();
        let match_flag = Arc::new(AtomicBool::new(opts.match_volume));
        let mf = match_flag.clone();
        let analyzer = Arc::new(Analyzer::start(
            lib.clone(),
            Box::new(move |id, db| {
                if mf.load(Ordering::Relaxed) {
                    let _ = tx_engine.send(Cmd::SetGain { id: id.to_string(), db });
                }
            }),
        ));
        let p = Arc::new(Self {
            st: Mutex::new(State {
                queue: Vec::new(),
                index: -1,
                unshuffled: None,
                opts: opts.clone(),
                eq: eq.clone(),
                muted: false,
                sleep: Sleep::Off,
                queue_gen: 0,
                error_streak: 0,
                listen_id: None,
                listen_start: 0,
                listened: 0.0,
                counted: false,
                pre_analyzed: None,
            }),
            engine,
            lib,
            analyzer,
            settings,
            settings_dirty,
            notices: Mutex::new(Vec::new()),
            repaint: Mutex::new(None),
        });
        MATCH.get_or_init(|| match_flag);
        p.engine.set_volume(opts.volume);
        p.apply_eq();
        p.engine.send(Cmd::SetCrossfade(opts.crossfade as f32));
        p.engine.send(Cmd::SetLeveler(opts.normalize));
        p.engine.send(Cmd::SetLimiter(opts.match_volume));
        // restore the last session, paused
        let q: Vec<String> = session.queue.into_iter().filter(|id| p.lib.track(id).is_some()).collect();
        if !q.is_empty() {
            let idx = session.index.clamp(0, q.len() as i64 - 1) as isize;
            let mut st = p.st.lock();
            st.queue = q;
            st.index = idx;
            drop(st);
            p.load(idx, false, session.position);
        }
        let pc = p.clone();
        std::thread::Builder::new().name("player".into()).spawn(move || pc.controller()).ok();
        p
    }

    fn notify(&self) {
        if let Some(f) = self.repaint.lock().as_ref() {
            f();
        }
    }

    pub fn status(&self) -> Status {
        self.engine.status()
    }

    pub fn current_id(&self) -> Option<String> {
        let st = self.st.lock();
        st.queue.get(st.index.max(0) as usize).filter(|_| st.index >= 0).cloned()
    }

    fn save_opts(&self) {
        let st = self.st.lock();
        let mut s = self.settings.lock();
        s.player = st.opts.clone();
        s.eq = st.eq.clone();
        self.settings_dirty.store(true, Ordering::Relaxed);
    }

    fn gain_for(&self, id: &str) -> f32 {
        if !self.st.lock().opts.match_volume {
            return 0.0;
        }
        self.lib.track(id).and_then(|t| t.gain_v2).unwrap_or(0.0)
    }

    // ------------------------------------------------------------ transport
    fn load(&self, index: isize, autoplay: bool, start: f64) {
        self.finish_listen(false);
        let id = {
            let mut st = self.st.lock();
            let Some(id) = st.queue.get(index as usize).cloned() else { return };
            st.index = index;
            st.counted = false;
            st.listened = 0.0;
            st.queue_gen += 1;
            id
        };
        let Some(t) = self.lib.track(&id) else { return };
        self.analyzer.request(&id);
        self.engine.send(Cmd::Play { id: id.clone(), path: PathBuf::from(&t.path), start, gain_db: self.gain_for(&id), autoplay });
        self.preload_next();
        self.notify();
    }

    fn next_index(&self) -> Option<isize> {
        let st = self.st.lock();
        if st.opts.repeat == "one" && st.index >= 0 {
            return Some(st.index);
        }
        first_unhidden(&self.lib.data.read(), &st.queue, st.index + 1, st.opts.repeat == "all")
    }

    fn preload_next(&self) {
        match self.next_index().and_then(|i| self.st.lock().queue.get(i as usize).cloned()) {
            Some(id) => {
                if let Some(t) = self.lib.track(&id) {
                    self.engine.send(Cmd::Preload { id: id.clone(), path: PathBuf::from(&t.path), gain_db: self.gain_for(&id) });
                }
            }
            None => self.engine.send(Cmd::ClearNext),
        }
    }

    /// Play a list from `start` (hidden songs are left out).
    pub fn play_list(&self, ids: Vec<String>, start: usize, shuffle: Option<bool>) {
        self.start_list(ids, start, shuffle, false);
    }

    /// Play a list starting at the song you picked (it plays even if it's hidden).
    pub fn play_pick(&self, ids: Vec<String>, start: usize, shuffle: Option<bool>) {
        self.start_list(ids, start, shuffle, true);
    }

    fn start_list(&self, ids: Vec<String>, start: usize, shuffle: Option<bool>, picked: bool) {
        let (ids, start) = playable(&self.lib.data.read(), ids, start, picked);
        if ids.is_empty() {
            self.notices.lock().push("Nothing to play: those songs are hidden".into());
            return;
        }
        let mut st = self.st.lock();
        let do_shuffle = shuffle.unwrap_or(st.opts.shuffle);
        let mut q = ids;
        let mut idx = start.min(q.len() - 1);
        st.unshuffled = None;
        if do_shuffle {
            st.unshuffled = Some(q.clone());
            let first = q.remove(idx);
            let rest = smart_shuffle(&self.lib, q, st.opts.smart_shuffle);
            q = std::iter::once(first).chain(rest).collect();
            idx = 0;
            if shuffle == Some(true) {
                st.opts.shuffle = true;
            }
        }
        st.queue = q;
        drop(st);
        self.load(idx as isize, true, 0.0);
        self.save_opts();
    }

    pub fn play_index(&self, i: usize) {
        self.load(i as isize, true, 0.0);
    }

    pub fn toggle(&self) {
        if self.status().playing { self.pause() } else { self.resume() }
    }

    pub fn pause(&self) {
        self.engine.send(Cmd::Pause);
        self.notify();
    }

    pub fn resume(&self) {
        let (idx, has) = { let st = self.st.lock(); (st.index, !st.queue.is_empty()) };
        if !has || idx < 0 {
            return;
        }
        if self.status().current.is_none() {
            self.load(idx, true, 0.0);
        } else {
            self.engine.send(Cmd::Resume);
        }
        self.notify();
    }

    pub fn next(&self, manual: bool) {
        let (len, idx, repeat, shuffle) = { let st = self.st.lock(); (st.queue.len() as isize, st.index, st.opts.repeat.clone(), st.opts.shuffle) };
        if len == 0 {
            return;
        }
        if !manual && repeat == "one" {
            return self.load(idx, true, 0.0);
        }
        let mut i = idx + 1;
        if i >= len {
            if repeat == "all" || manual {
                i = 0;
                if shuffle && len > 4 {
                    self.reshuffle_lap();
                }
            } else {
                self.engine.send(Cmd::Pause);
                self.notify();
                return;
            }
        }
        // songs hidden after they were queued are skipped
        let next = { let st = self.st.lock(); first_unhidden(&self.lib.data.read(), &st.queue, i, repeat == "all" || manual) };
        match next {
            Some(i) => self.load(i, true, 0.0),
            None => {
                self.engine.send(Cmd::Pause);
                self.notify();
            }
        }
    }

    /// Shuffle + repeat: deal a fresh order for the next lap without the last few songs repeating.
    fn reshuffle_lap(&self) {
        let mut st = self.st.lock();
        let recent: Vec<String> = st.queue.iter().rev().take(3).cloned().collect();
        let others: Vec<String> = st.queue.iter().filter(|id| !recent.contains(id)).cloned().collect();
        let fresh = smart_shuffle(&self.lib, others, st.opts.smart_shuffle);
        let mut q: Vec<String> = fresh.iter().take(3).cloned().collect();
        q.extend(smart_shuffle(&self.lib, recent, false));
        q.extend(fresh.into_iter().skip(3));
        st.queue = q;
        st.queue_gen += 1;
    }

    pub fn prev(&self) {
        if self.status().position > 3.0 {
            return self.seek(0.0);
        }
        let (idx, len, repeat) = { let st = self.st.lock(); (st.index, st.queue.len() as isize, st.opts.repeat.clone()) };
        let i = if idx - 1 < 0 { if repeat == "all" { len - 1 } else { 0 } } else { idx - 1 };
        self.load(i, true, 0.0);
    }

    pub fn seek(&self, t: f64) {
        self.engine.send(Cmd::Seek(t.max(0.0)));
        self.notify();
    }

    pub fn set_volume(&self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        self.st.lock().opts.volume = v;
        self.st.lock().muted = false;
        self.engine.set_muted(false);
        self.engine.set_volume(v);
        self.save_opts();
    }

    pub fn toggle_mute(&self) {
        let m = { let mut st = self.st.lock(); st.muted = !st.muted; st.muted };
        self.engine.set_muted(m);
    }

    pub fn toggle_shuffle(&self) {
        let mut st = self.st.lock();
        st.opts.shuffle = !st.opts.shuffle;
        if !st.queue.is_empty() && st.index >= 0 {
            let cur = st.queue[st.index as usize].clone();
            if st.opts.shuffle {
                st.unshuffled = Some(st.queue.clone());
                let upcoming: Vec<String> = st.queue[(st.index as usize + 1)..].to_vec();
                let sh = smart_shuffle(&self.lib, upcoming, st.opts.smart_shuffle);
                let keep = st.index as usize + 1;
                st.queue.truncate(keep);
                st.queue.extend(sh);
            } else if let Some(orig) = st.unshuffled.take() {
                st.index = orig.iter().position(|x| *x == cur).unwrap_or(0) as isize;
                st.queue = orig;
            }
            st.queue_gen += 1;
        }
        drop(st);
        self.preload_next();
        self.save_opts();
    }

    pub fn cycle_repeat(&self) {
        {
            let mut st = self.st.lock();
            st.opts.repeat = match st.opts.repeat.as_str() { "off" => "all", "all" => "one", _ => "off" }.into();
        }
        self.preload_next();
        self.save_opts();
    }

    pub fn enqueue(&self, ids: Vec<String>) {
        if self.st.lock().index < 0 {
            return self.play_list(ids, 0, Some(false));
        }
        let n = ids.len();
        {
            let mut st = self.st.lock();
            if let Some(u) = st.unshuffled.as_mut() { u.extend(ids.iter().cloned()); }
            st.queue.extend(ids);
            st.queue_gen += 1;
        }
        self.preload_next();
        self.notices.lock().push(format!("Added {} to queue", plural(n, "song")));
    }

    pub fn play_next(&self, ids: Vec<String>) {
        if self.st.lock().index < 0 {
            return self.play_list(ids, 0, Some(false));
        }
        let n = ids.len();
        {
            let mut st = self.st.lock();
            let at = (st.index + 1) as usize;
            if let Some(u) = st.unshuffled.as_mut() { u.extend(ids.iter().cloned()); }
            for (k, id) in ids.into_iter().enumerate() {
                st.queue.insert(at + k, id);
            }
            st.queue_gen += 1;
        }
        self.preload_next();
        self.notices.lock().push(format!("{} will play next", if n == 1 { "Song".into() } else { format!("{n} songs") }));
    }

    pub fn remove_at(&self, i: usize) {
        let (cur, playing) = (self.st.lock().index, self.status().playing);
        if i as isize == cur {
            let mut st = self.st.lock();
            st.queue.remove(i);
            st.queue_gen += 1;
            if st.queue.is_empty() {
                st.index = -1;
                drop(st);
                self.engine.send(Cmd::Stop);
                return;
            }
            let ni = i.min(st.queue.len() - 1) as isize;
            drop(st);
            return self.load(ni, playing, 0.0);
        }
        let mut st = self.st.lock();
        if i < st.queue.len() {
            st.queue.remove(i);
            if (i as isize) < st.index { st.index -= 1; }
            st.queue_gen += 1;
        }
        drop(st);
        self.preload_next();
    }

    pub fn move_item(&self, from: usize, to: usize) {
        {
            let mut st = self.st.lock();
            if from >= st.queue.len() || from == to { return; }
            let id = st.queue.remove(from);
            let to = to.min(st.queue.len());
            st.queue.insert(to, id);
            let idx = st.index;
            if idx == from as isize { st.index = to as isize; }
            else if (from as isize) < idx && to as isize >= idx { st.index -= 1; }
            else if (from as isize) > idx && to as isize <= idx { st.index += 1; }
            st.queue_gen += 1;
        }
        self.preload_next();
    }

    pub fn clear_upcoming(&self) {
        {
            let mut st = self.st.lock();
            let keep = (st.index + 1).max(0) as usize;
            st.queue.truncate(keep);
            st.unshuffled = None;
            st.queue_gen += 1;
        }
        self.preload_next();
    }

    // ------------------------------------------------------------ sound options
    pub fn apply_eq(&self) {
        let eq = self.st.lock().eq.clone();
        let mut g = [0f32; 10];
        for (i, v) in eq.gains.iter().take(10).enumerate() { g[i] = *v; }
        self.engine.send(Cmd::SetEq { enabled: eq.enabled, gains: g, preamp: eq.preamp });
    }
    pub fn set_eq(&self, eq: Eq) {
        self.st.lock().eq = eq;
        self.apply_eq();
        self.save_opts();
    }
    pub fn set_crossfade(&self, s: u32) {
        self.st.lock().opts.crossfade = s;
        self.engine.send(Cmd::SetCrossfade(s as f32));
        self.save_opts();
    }
    pub fn set_normalize(&self, on: bool) {
        self.st.lock().opts.normalize = on;
        self.engine.send(Cmd::SetLeveler(on));
        self.save_opts();
    }
    pub fn set_match_volume(&self, on: bool) {
        self.st.lock().opts.match_volume = on;
        if let Some(f) = MATCH.get() { f.store(on, Ordering::Relaxed); }
        self.engine.send(Cmd::SetLimiter(on));
        for id in [self.current_id()].into_iter().flatten() {
            self.engine.send(Cmd::SetGain { id: id.clone(), db: self.gain_for(&id) });
        }
        self.preload_next();
        self.save_opts();
    }
    pub fn set_smart_shuffle(&self, on: bool) {
        self.st.lock().opts.smart_shuffle = on;
        self.save_opts();
    }
    pub fn set_visualizer(&self, mode: &str, fps: u32) {
        { let mut st = self.st.lock(); st.opts.visualizer = mode.into(); st.opts.vis_fps = fps; }
        self.save_opts();
    }
    pub fn set_sleep(&self, s: Sleep) {
        self.st.lock().sleep = s;
    }

    // ------------------------------------------------------------ bookkeeping
    /// Close out the current listen for Stats + Smart Shuffle (ended = played to the end).
    fn finish_listen(&self, ended: bool) {
        let mut st = self.st.lock();
        let (Some(id), secs) = (st.listen_id.take(), st.listened.round() as i64) else { return };
        let skipped = !ended && !st.counted;
        let start = st.listen_start;
        st.listen_start = 0;
        drop(st);
        if secs >= 1 {
            self.lib.add_history((id.clone(), start, secs, skipped as u8));
            if skipped && secs < 30 {
                self.lib.bump_skip(&id);
            }
        }
    }

    fn controller(self: Arc<Self>) {
        let mut last = Instant::now();
        let mut last_save = Instant::now();
        loop {
            let ev = self.engine.events.recv_timeout(Duration::from_millis(250));
            match ev {
                Ok(Event::Current) => {
                    self.st.lock().error_streak = 0;
                    self.notify();
                }
                Ok(Event::Advanced(id)) => {
                    self.finish_listen(true);
                    {
                        let mut st = self.st.lock();
                        let n = st.queue.len() as isize;
                        let mut ni = if st.opts.repeat == "one" { st.index } else { st.index + 1 };
                        if ni >= n { ni = 0; }
                        if st.queue.get(ni as usize) != Some(&id) {
                            if let Some(p) = st.queue.iter().position(|x| *x == id) { ni = p as isize; }
                        }
                        st.index = ni;
                        st.counted = false;
                        st.listened = 0.0;
                        st.queue_gen += 1;
                        if st.sleep == Sleep::EndOfTrack {
                            st.sleep = Sleep::Off;
                            drop(st);
                            self.engine.send(Cmd::Pause);
                        }
                    }
                    self.analyzer.request(&id);
                    self.preload_next();
                    self.notify();
                }
                Ok(Event::Ended) => {
                    self.finish_listen(true);
                    let sleep = std::mem::replace(&mut self.st.lock().sleep, Sleep::Off);
                    if sleep != Sleep::EndOfTrack {
                        if sleep != Sleep::Off { self.st.lock().sleep = sleep; }
                        self.next(false);
                    }
                    self.notify();
                }
                Ok(Event::Error(id, e)) => {
                    let title = self.lib.track(&id).map(|t| t.title).unwrap_or_default();
                    self.notices.lock().push(format!("Can't play \"{title}\" — {e}"));
                    let streak = { let mut st = self.st.lock(); st.error_streak += 1; st.error_streak };
                    if streak < 5 {
                        std::thread::sleep(Duration::from_millis(400));
                        self.next(true);
                    }
                    self.notify();
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                Err(_) => {}
            }

            // ~4x a second: listening time, play counts, pre-analysis, sleep timer, session save
            let dt = last.elapsed().as_secs_f64();
            last = Instant::now();
            let s = self.status();
            if s.playing {
                if let Some(cur) = s.current.clone() {
                    let mut count = false;
                    {
                        let mut st = self.st.lock();
                        if st.listen_id.as_deref() != Some(cur.as_str()) {
                            st.listen_id = Some(cur.clone());
                            st.listen_start = now_secs();
                            st.listened = 0.0;
                        }
                        st.listened += dt;
                        if !st.counted && s.duration > 0.0 && st.listened >= (s.duration / 2.0).min(30.0) {
                            st.counted = true;
                            count = true;
                        }
                    }
                    if count {
                        self.lib.bump_play(&cur);
                    }
                    // measure the next song's loudness before it starts
                    if s.duration - s.position < 45.0 {
                        if let Some(nid) = self.next_index().and_then(|i| self.st.lock().queue.get(i as usize).cloned()) {
                            let mut st = self.st.lock();
                            if st.pre_analyzed.as_ref() != Some(&nid) {
                                st.pre_analyzed = Some(nid.clone());
                                drop(st);
                                self.analyzer.request(&nid);
                            }
                        }
                    }
                }
                if let Sleep::At(t) = self.st.lock().sleep {
                    if Instant::now() >= t {
                        self.st.lock().sleep = Sleep::Off;
                        self.engine.send(Cmd::Pause);
                        self.notify();
                    }
                }
            }
            if last_save.elapsed() > Duration::from_secs(5) {
                last_save = Instant::now();
                let st = self.st.lock();
                let mut set = self.settings.lock();
                let pos = s.position;
                if set.session.index != st.index as i64 || set.session.queue.len() != st.queue.len() || (set.session.position - pos).abs() > 1.0 {
                    set.session.queue = st.queue.iter().take(5000).cloned().collect();
                    set.session.index = st.index as i64;
                    set.session.position = pos;
                    self.settings_dirty.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn shutdown(&self) {
        self.finish_listen(true);
        let s = self.status();
        let st = self.st.lock();
        let mut set = self.settings.lock();
        set.session.queue = st.queue.iter().take(5000).cloned().collect();
        set.session.index = st.index as i64;
        set.session.position = s.position;
    }
}

/// Songs to queue: hidden ones ("Don't play this") are left out, except the one you picked to
/// start with. Returns the list and where to start in it.
pub fn playable(d: &LibraryData, ids: Vec<String>, start: usize, picked: bool) -> (Vec<String>, usize) {
    let hidden = |id: &String| d.stats.get(id).map(|s| s.hidden).unwrap_or(false);
    if !ids.iter().any(hidden) {
        return (ids, start);
    }
    let first = ids.get(start).cloned();
    let keep = first.clone().filter(|_| picked);
    let out: Vec<String> = ids.into_iter().filter(|id| !hidden(id) || Some(id) == keep.as_ref()).collect();
    // start at the picked song, or the first playable one after where you started
    let at = first.and_then(|f| out.iter().position(|x| *x == f)).unwrap_or(0);
    let at = if out.is_empty() { 0 } else { at.min(out.len() - 1) };
    (out, at)
}

/// The first song in the queue from `from` on (wrapping round to the start if `wrap`) that
/// isn't hidden.
fn first_unhidden(d: &LibraryData, queue: &[String], from: isize, wrap: bool) -> Option<isize> {
    let n = queue.len() as isize;
    if n == 0 || from < 0 {
        return None;
    }
    (from..from + n).take_while(|&i| wrap || i < n).map(|i| i % n).find(|&i| !d.stats.get(&queue[i as usize]).map(|s| s.hidden).unwrap_or(false))
}

static MATCH: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();

fn plural(n: usize, w: &str) -> String {
    format!("{n} {w}{}", if n == 1 { "" } else { "s" })
}

/// Smart shuffle: a true permutation, same-artist songs spread evenly apart, and songs you tend
/// to skip drift toward the end.
pub fn smart_shuffle(lib: &Library, mut ids: Vec<String>, smart: bool) -> Vec<String> {
    fastrand::shuffle(&mut ids);
    if !smart {
        return ids;
    }
    let d = lib.data.read();
    let mut groups: std::collections::HashMap<String, Vec<String>> = Default::default();
    for id in ids {
        let a = d.tracks.get(&id).map(|t| main_artist(&t.artist).to_lowercase()).unwrap_or_default();
        groups.entry(a).or_default().push(id);
    }
    let mut placed: Vec<(f32, String)> = Vec::new();
    for g in groups.into_values() {
        let n = g.len() as f32;
        let offset = fastrand::f32() / n;
        for (i, id) in g.into_iter().enumerate() {
            let st = d.stats.get(&id).cloned().unwrap_or_default();
            let skip_rate = st.skips as f32 / (st.plays + st.skips + 2) as f32;
            let pos = offset + i as f32 / n + (fastrand::f32() - 0.5) * (0.2 / n) + skip_rate * 0.6;
            placed.push((pos, id));
        }
    }
    placed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    placed.into_iter().map(|p| p.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Stat;

    #[test]
    fn hidden_songs_are_skipped() {
        let mut d = LibraryData::default();
        d.stats.insert("b".into(), Stat { hidden: true, ..Default::default() });
        d.stats.insert("d".into(), Stat { hidden: true, ..Default::default() });
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // playing a list leaves hidden songs out and starts at the next playable one
        assert_eq!(playable(&d, ids(&["a", "b", "c", "d"]), 0, false), (ids(&["a", "c"]), 0));
        assert_eq!(playable(&d, ids(&["a", "b", "c", "d"]), 1, false), (ids(&["a", "c"]), 0));
        assert_eq!(playable(&d, ids(&["a", "b", "c"]), 2, false), (ids(&["a", "c"]), 1));
        // ...but a song you picked yourself plays
        assert_eq!(playable(&d, ids(&["a", "b", "c", "d"]), 1, true), (ids(&["a", "b", "c"]), 1));
        assert_eq!(playable(&d, ids(&["b", "d"]), 0, false).0.len(), 0);
        // songs hidden after they were queued are skipped when moving on
        let q = ids(&["a", "b", "c", "d"]);
        assert_eq!(first_unhidden(&d, &q, 1, false), Some(2));
        assert_eq!(first_unhidden(&d, &q, 3, false), None);
        assert_eq!(first_unhidden(&d, &q, 3, true), Some(0));
        assert_eq!(first_unhidden(&d, &q, 4, true), Some(0));
        assert_eq!(first_unhidden(&d, &ids(&["b"]), 0, true), None);
    }
}

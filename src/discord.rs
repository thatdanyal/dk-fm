//! Discord status: "Listening to DK.FM" with the song, artist, album cover and time left, on your
//! Discord profile while Discord is open on this computer (Settings > Playback; off until you turn
//! it on, and never during private listening). Talks to the Discord app over its local IPC pipe /
//! socket (Rich Presence): nothing goes through a server of ours. Updates are sent from one
//! background thread, at most one every few seconds (Discord allows 5 per 20 s).
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// DK.FM's application on Discord (discord.com/developers): its name is what Discord shows,
/// "Listening to DK.FM". Empty: the feature is hidden.
pub const APP_ID: &str = "";

/// What to show (None: nothing, e.g. paused for a while or private listening).
#[derive(Clone, Debug, PartialEq)]
pub struct Now {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// a picture on the web for the song (else DK.FM's logo)
    pub cover: Option<String>,
    pub playing: bool,
    /// seconds into the song, and its length
    pub position: f64,
    pub duration: f64,
}

static TX: OnceLock<Sender<Option<Now>>> = OnceLock::new();

pub fn available() -> bool {
    !APP_ID.is_empty()
}

/// Show `now` on Discord (None clears it). Cheap: hands it to the background thread.
pub fn set(now: Option<Now>) {
    if !available() {
        return;
    }
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("discord".into()).spawn(move || run(rx)).ok();
        tx
    });
    let _ = tx.send(now);
}

const LOGO: &str = "https://raw.githubusercontent.com/thatdanyal/dk-fm/main/docs/logo.png";

fn activity(n: &Now) -> Value {
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let mut a = json!({
        "type": 2, // "Listening to"
        "details": clip(&n.title),
        "state": clip(&format!("by {}", n.artist)),
        "assets": {
            "large_image": n.cover.clone().unwrap_or_else(|| LOGO.to_string()),
            "large_text": clip(if n.album.is_empty() { "DK.FM" } else { &n.album }),
            "small_image": LOGO,
            "small_text": if n.playing { "Playing in DK.FM" } else { "Paused" },
        },
    });
    if n.playing && n.duration > 0.0 {
        let start = now_ms - (n.position * 1000.0) as i64;
        a["timestamps"] = json!({ "start": start, "end": start + (n.duration * 1000.0) as i64 });
    }
    a
}

/// Discord takes 2-128 characters.
fn clip(s: &str) -> String {
    let mut t: String = s.chars().take(120).collect();
    while t.chars().count() < 2 {
        t.push(' ');
    }
    t
}

trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

fn connect() -> Option<Box<dyn Pipe>> {
    for i in 0..10 {
        #[cfg(windows)]
        {
            if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")) {
                return Some(Box::new(f));
            }
        }
        #[cfg(unix)]
        {
            let dirs: Vec<std::path::PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"].iter().filter_map(|k| std::env::var_os(k).map(std::path::PathBuf::from)).chain([std::path::PathBuf::from("/tmp")]).collect();
            for d in dirs {
                // the app as installed, as a Flatpak, as a Snap
                for sub in ["", "app/com.discordapp.Discord", "snap.discord"] {
                    if let Ok(s) = std::os::unix::net::UnixStream::connect(d.join(sub).join(format!("discord-ipc-{i}"))) {
                        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                        return Some(Box::new(s));
                    }
                }
            }
        }
    }
    None
}

fn send(p: &mut Box<dyn Pipe>, op: u32, body: &Value) -> std::io::Result<Value> {
    let b = serde_json::to_vec(body).unwrap_or_default();
    let mut frame = Vec::with_capacity(8 + b.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&(b.len() as u32).to_le_bytes());
    frame.extend_from_slice(&b);
    p.write_all(&frame)?;
    // the reply (Discord closes the pipe if replies pile up unread)
    let mut head = [0u8; 8];
    p.read_exact(&mut head)?;
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    let mut buf = vec![0u8; len.min(1 << 20)];
    p.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf).unwrap_or(Value::Null))
}

fn handshake() -> Option<Box<dyn Pipe>> {
    let mut p = connect()?;
    let r = send(&mut p, 0, &json!({ "v": 1, "client_id": APP_ID })).ok()?;
    (r["evt"] == "READY").then_some(p)
}

fn run(rx: Receiver<Option<Now>>) {
    let mut pipe: Option<Box<dyn Pipe>> = None;
    let mut want: Option<Option<Now>> = None;
    let mut last_sent = Instant::now() - Duration::from_secs(60);
    let mut last_try = Instant::now() - Duration::from_secs(60);
    let mut nonce = 0u64;
    loop {
        // the newest wish wins (a burst of song changes sends only the last)
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(n) => want = Some(n),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(n) = rx.try_recv() {
            want = Some(n);
        }
        let Some(w) = want.clone() else { continue };
        if last_sent.elapsed() < Duration::from_secs(4) {
            std::thread::sleep(Duration::from_secs(4) - last_sent.elapsed());
            while let Ok(n) = rx.try_recv() {
                want = Some(n);
            }
        }
        let w = want.clone().unwrap_or(w);
        if pipe.is_none() {
            // Discord isn't open: try again now and then
            if last_try.elapsed() < Duration::from_secs(20) {
                continue;
            }
            last_try = Instant::now();
            pipe = handshake();
            if pipe.is_none() {
                continue;
            }
        }
        nonce += 1;
        let args = match &w {
            Some(n) => json!({ "pid": std::process::id(), "activity": activity(n) }),
            None => json!({ "pid": std::process::id() }),
        };
        let ok = pipe.as_mut().map(|p| send(p, 1, &json!({ "cmd": "SET_ACTIVITY", "args": args, "nonce": nonce.to_string() })).is_ok()).unwrap_or(false);
        last_sent = Instant::now();
        if ok {
            want = None;
        } else {
            // Discord was closed: reconnect later and send it again
            pipe = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_shape() {
        let n = Now { title: "Immortal".into(), artist: "21 Savage".into(), album: "".into(), cover: None, playing: true, position: 10.0, duration: 254.0 };
        let a = activity(&n);
        assert_eq!(a["type"], 2);
        assert_eq!(a["state"], "by 21 Savage");
        assert_eq!(a["assets"]["large_image"], LOGO);
        let ts = &a["timestamps"];
        assert_eq!(ts["end"].as_i64().unwrap() - ts["start"].as_i64().unwrap(), 254_000);
        // paused: no countdown
        assert!(activity(&Now { playing: false, ..n.clone() }).get("timestamps").is_none());
        assert_eq!(clip("x"), "x ");
    }
}

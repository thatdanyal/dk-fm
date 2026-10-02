//! Single instance: the first copy listens on a localhost port; launching DK.FM again just tells
//! the running copy to show its window, then exits. `DK.FM --play-pause | --next | --prev` sends
//! that command to the running copy instead (for hotkey tools, stream decks, scripts).
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn port() -> u16 {
    // a stable port per data folder, so isolated test profiles don't collide with the real app
    let d = crate::store::data_dir().to_string_lossy().to_lowercase();
    let h = d.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    47000 + (h % 1000) as u16
}

/// Returns None if another copy is running (after asking it to show itself).
pub fn acquire(command: Option<&str>) -> Option<Arc<AtomicBool>> {
    let addr = ("127.0.0.1", port());
    if let Ok(mut s) = TcpStream::connect_timeout(&addr.into_addr(), Duration::from_millis(300)) {
        let msg = command.map(|c| format!("DKFM-CMD:{c}")).unwrap_or_else(|| "DKFM-SHOW".into());
        let _ = s.write_all(msg.as_bytes());
        return None;
    }
    if command.is_some() {
        return None; // nothing running to control
    }
    let listener = TcpListener::bind(addr).ok()?;
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    std::thread::Builder::new()
        .name("single".into())
        .spawn(move || {
            for mut c in listener.incoming().flatten() {
                let mut buf = [0u8; 32];
                let _ = c.set_read_timeout(Some(Duration::from_secs(1)));
                if let Ok(n) = c.read(&mut buf) {
                    if &buf[..n] == b"DKFM-SHOW" {
                        f.store(true, Ordering::Relaxed);
                        if let Some(cb) = WAKE.lock().as_ref() {
                            cb();
                        }
                    } else if let Some(cmd) = buf[..n].strip_prefix(b"DKFM-CMD:") {
                        if let Some(cb) = COMMAND.lock().as_ref() {
                            cb(&String::from_utf8_lossy(cmd));
                        }
                    }
                }
            }
        })
        .ok();
    Some(flag)
}

pub static WAKE: parking_lot::Mutex<Option<Box<dyn Fn() + Send>>> = parking_lot::Mutex::new(None);
pub static COMMAND: parking_lot::Mutex<Option<Box<dyn Fn(&str) + Send>>> = parking_lot::Mutex::new(None);

trait IntoAddr {
    fn into_addr(self) -> std::net::SocketAddr;
}
impl IntoAddr for (&str, u16) {
    fn into_addr(self) -> std::net::SocketAddr {
        std::net::SocketAddr::from(([127, 0, 0, 1], self.1))
    }
}

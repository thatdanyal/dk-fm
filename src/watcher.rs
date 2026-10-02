//! Folder watching (OS notifications, no polling): files dropped into music folders appear in the
//! library within seconds; deleted files disappear. Events are debounced per file.
use crate::library::{is_audio, Library};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct FolderWatcher {
    w: Mutex<Option<RecommendedWatcher>>,
    pending: Arc<Mutex<HashMap<PathBuf, Instant>>>,
}

impl FolderWatcher {
    pub fn new(lib: Arc<Library>) -> Arc<Self> {
        let me = Arc::new(Self { w: Mutex::new(None), pending: Arc::new(Mutex::new(HashMap::new())) });
        let pending = me.pending.clone();
        std::thread::Builder::new()
            .name("watch".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(700));
                let due: Vec<PathBuf> = {
                    let mut p = pending.lock();
                    let d: Vec<PathBuf> = p.iter().filter(|(_, t)| t.elapsed() > Duration::from_millis(2500)).map(|(k, _)| k.clone()).collect();
                    for k in &d {
                        p.remove(k);
                    }
                    d
                };
                for f in due {
                    if f.is_file() {
                        lib.add_files(&[f], |_| {});
                    } else {
                        lib.remove_path(&f);
                    }
                }
            })
            .ok();
        me
    }

    pub fn watch(&self, folders: &[PathBuf]) {
        let pending = self.pending.clone();
        let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                for p in ev.paths {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    if is_audio(&p) && !name.contains(".part") && !name.starts_with('.') {
                        pending.lock().insert(p, Instant::now());
                    }
                }
            }
        });
        let Ok(mut w) = w else { return };
        for f in folders {
            let _ = std::fs::create_dir_all(f);
            let _ = w.watch(f, RecursiveMode::Recursive);
        }
        *self.w.lock() = Some(w);
    }
}

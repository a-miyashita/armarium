//! Monitoring of registered files (04. Workspace, section 3).
//!
//! Armarium watches the parent directory of each registered file one time, and the
//! directory of `workspace.json`. After a change (debounced), the engine checks the
//! fingerprints of loaded tables and reads `workspace.json` again if necessary.

use armarium_core::Engine;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

enum Msg {
    FileEvent,
    RefreshPaths,
}

/// Wakes the watcher thread.
#[derive(Clone)]
pub struct Trigger {
    tx: Arc<Mutex<Option<Sender<Msg>>>>,
    pending: Arc<AtomicBool>,
}

impl Trigger {
    pub fn new() -> Trigger {
        Trigger {
            tx: Arc::new(Mutex::new(None)),
            pending: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Asks the watcher to update the watched directories (after the workspace changed).
    pub fn refresh_paths(&self) {
        if let Some(tx) = self.tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = tx.send(Msg::RefreshPaths);
        } else {
            self.pending.store(true, Ordering::SeqCst);
        }
    }
}

fn wanted_dirs(engine: &Engine) -> BTreeSet<PathBuf> {
    let mut dirs = BTreeSet::new();
    if let Some(d) = engine.workspace_path().parent() {
        dirs.insert(d.to_path_buf());
    }
    if let Ok(sources) = engine.list_sources() {
        for s in sources {
            if let Some(d) = std::path::Path::new(&s.path).parent() {
                if d.is_dir() {
                    dirs.insert(d.to_path_buf());
                }
            }
        }
    }
    dirs
}

pub fn start(engine: Arc<Engine>, trigger: Trigger) {
    let (tx, rx) = channel::<Msg>();
    *trigger.tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx.clone());
    std::thread::Builder::new()
        .name("armarium-watcher".into())
        .spawn(move || {
            let tx2 = tx.clone();
            let mut watcher: RecommendedWatcher =
                match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                    if let Ok(ev) = res {
                        if !matches!(ev.kind, notify::EventKind::Access(_)) {
                            let _ = tx2.send(Msg::FileEvent);
                        }
                    }
                }) {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::warn!("file monitoring is not available: {e}");
                        return;
                    }
                };
            let mut watched: BTreeSet<PathBuf> = BTreeSet::new();
            let sync = |watcher: &mut RecommendedWatcher, watched: &mut BTreeSet<PathBuf>| {
                let wanted = wanted_dirs(&engine);
                for d in watched.difference(&wanted).cloned().collect::<Vec<_>>() {
                    let _ = watcher.unwatch(&d);
                    watched.remove(&d);
                }
                for d in wanted.difference(watched).cloned().collect::<Vec<_>>() {
                    match watcher.watch(&d, RecursiveMode::NonRecursive) {
                        Ok(()) => {
                            watched.insert(d);
                        }
                        Err(e) => tracing::debug!("cannot watch {}: {e}", d.display()),
                    }
                }
            };
            sync(&mut watcher, &mut watched);
            let mut dirty = trigger.pending.swap(false, Ordering::SeqCst);
            loop {
                // Debounce: wait until no event comes for 300 ms.
                let timeout = if dirty {
                    Duration::from_millis(300)
                } else {
                    Duration::from_secs(3600)
                };
                match rx.recv_timeout(timeout) {
                    Ok(Msg::FileEvent) => dirty = true,
                    Ok(Msg::RefreshPaths) => sync(&mut watcher, &mut watched),
                    Err(RecvTimeoutError::Timeout) => {
                        if dirty {
                            dirty = false;
                            if let Err(e) = engine.check_external_changes() {
                                tracing::warn!("cannot check external changes: {e}");
                            }
                            sync(&mut watcher, &mut watched);
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("cannot start the watcher thread");
}

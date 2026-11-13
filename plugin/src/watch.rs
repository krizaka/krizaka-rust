//! Watching a granted folder: a document added, renamed or removed in it tells the page (debounced), which lists the
//! folder again through the gate. The event carries the folder, never a file's content.

use crate::error::CommandError;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Runtime};

/// How long the folder must stay quiet before the page is told (a copy of many files is one event).
// @fixed: a debounce of the file system's events, not a setting
const QUIET: Duration = Duration::from_millis(400);

/// Dropping it stops the watch (the watcher closes its channel, the debounce thread ends).
pub struct Watch {
    _watcher: RecommendedWatcher,
}

pub fn start<R: Runtime>(
    app: AppHandle<R>,
    dir: PathBuf,
    event: &'static str,
) -> Result<Watch, CommandError> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if let Ok(e) = result {
            if matches!(
                e.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            ) {
                let _ = tx.send(());
            }
        }
    })?;
    watcher.watch(&dir, RecursiveMode::Recursive)?;
    let folder = dir.display().to_string();
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            while rx.recv_timeout(QUIET).is_ok() {}
            let _ = app.emit(event, &folder);
        }
    });
    Ok(Watch { _watcher: watcher })
}

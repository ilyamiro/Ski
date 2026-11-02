//! Watches `settings.json`, so an edit made by hand (or by another tool) takes effect while Ski runs.
//!
//! inotify tells when a writer is finished: `IN_CLOSE_WRITE` for a file written in place and
//! `IN_MOVED_TO` for one renamed over the old, so the file is read once, complete, with no waiting.
use std::sync::mpsc::{channel, Receiver};

use eframe::egui;
use notify::event::{AccessKind, AccessMode, ModifyKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub struct SettingsWatch {
    _watcher: RecommendedWatcher,
    events: Receiver<()>,
}

impl SettingsWatch {
    pub fn new(ctx: &egui::Context) -> Option<Self> {
        let file = crate::settings::file()?;
        let dir = file.parent()?.to_path_buf();
        let name = file.file_name()?.to_owned();
        if !dir.is_dir() && (crate::fs::admin::is_admin() || std::fs::create_dir_all(&dir).is_err()) {
            return None;
        }
        let (tx, events) = channel();
        let repaint = ctx.clone();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if finished_writing(&event.kind) && event.paths.iter().any(|p| p.file_name() == Some(name.as_os_str())) && tx.send(()).is_ok() {
                crate::app::wake_windows(&repaint);
            }
        })
        .ok()?;
        // The folder, not the file: editors save by renaming a new file over the old one
        watcher.watch(&dir, RecursiveMode::NonRecursive).ok()?;
        Some(Self { _watcher: watcher, events })
    }

    /// True when the file was written since the last call.
    pub fn changed(&self) -> bool {
        self.events.try_iter().count() > 0
    }
}

fn finished_writing(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Access(AccessKind::Close(AccessMode::Write))
            | EventKind::Modify(ModifyKind::Name(RenameMode::To | RenameMode::Both | RenameMode::Any))
    )
}

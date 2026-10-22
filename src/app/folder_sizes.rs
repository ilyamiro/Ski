//! Folder sizes in the listing: a small scheduler that measures at most two folders at once,
//! visible rows first, and drops all work when the folder changes. The numbers themselves live
//! in `fs::dirsize`.
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::fs::dirsize;
use crate::fs::props::FolderScan;
use crate::settings::FolderSizeMode;

/// Folders measured at the same time: more only makes a spinning disk seek.
const MAX_RUNNING: usize = 2;
/// With "Always", this many rows from the top are measured; the rest wait until they scroll up.
const AUTO_ROWS: usize = 120;

#[derive(Default)]
pub(super) struct SizeScheduler {
    running: Vec<(PathBuf, FolderScan)>,
    queue: VecDeque<PathBuf>,
    counting: Arc<AtomicBool>,
}

impl SizeScheduler {
    /// Asks for `path` to be measured, unless it is known or on its way.
    pub(super) fn request(&mut self, path: &Path) {
        if dirsize::totals(path).is_some() || dirsize::is_pending(path) {
            return;
        }
        dirsize::mark_pending(path);
        self.queue.push_back(path.to_path_buf());
    }

    /// Leaving the folder: stop everything and forget what was waiting.
    pub(super) fn cancel_all(&mut self) {
        self.running.clear();
        self.queue.clear();
        dirsize::drop_pending();
    }

    /// Stops the scans that cover `path` (a measurement made while it changes would be stale).
    pub(super) fn discard(&mut self, path: &Path) {
        let affected = |p: &Path| p.starts_with(path) || path.starts_with(p);
        self.running.retain(|(p, _)| !affected(p));
        self.queue.retain(|p| !affected(p));
    }

    pub(super) fn busy(&self) -> bool {
        !self.running.is_empty() || !self.queue.is_empty() || self.counting.load(Ordering::Relaxed)
    }

    /// Takes in finished scans and starts waiting ones; true when a size arrived.
    pub(super) fn tick(&mut self) -> bool {
        let mut arrived = false;
        self.running.retain(|(path, scan)| {
            let done = scan.state.done.load(Ordering::Relaxed);
            if done {
                dirsize::store(path, scan.state.totals());
                arrived = true;
            }
            !done
        });
        while self.running.len() < MAX_RUNNING {
            let Some(path) = self.queue.pop_front() else { break };
            self.running.push((path.clone(), FolderScan::start(path)));
        }
        arrived
    }
}

impl SkiApp {
    /// Once per frame: queue the folders that should be measured, start the scans, and sort again
    /// when sizes arrived and the listing is ordered by size.
    pub(super) fn tick_folder_sizes(&mut self, ctx: &egui::Context) {
        dirsize::set_size_on_disk(self.settings.size_on_disk);
        if !self.in_recent() && !self.sizes.counting.load(Ordering::Relaxed) {
            let missing: Vec<PathBuf> = self.items.iter().filter(|i| i.is_dir && !dirsize::has_count(&i.path)).take(400).map(|i| i.path.clone()).collect();
            if !missing.is_empty() {
                dirsize::count_in_background(missing, Arc::clone(&self.sizes.counting));
            }
        }
        if self.settings.folder_sizes == FolderSizeMode::Always {
            let wanted: Vec<PathBuf> = self.items.iter().filter(|i| i.is_dir).take(AUTO_ROWS).map(|i| i.path.clone()).collect();
            // The selection feeds the status line
            let selected: Vec<PathBuf> = self.selection.iter().filter_map(|&i| self.items.get(i)).filter(|i| i.is_dir).map(|i| i.path.clone()).collect();
            for path in wanted.into_iter().chain(selected) {
                self.sizes.request(&path);
            }
        }
        if self.sizes.tick() && self.sort_by == SortBy::Size {
            self.resort_items();
        }
        if self.sizes.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
    }

    /// "Calculate size" on folders.
    pub(super) fn calculate_sizes(&mut self, paths: &[PathBuf]) {
        for path in paths.iter().filter(|p| p.is_dir()) {
            self.sizes.discard(path);
            dirsize::forget(path);
            self.sizes.request(path);
        }
    }

    /// Changes on disk below these paths make the totals above them stale.
    pub(super) fn forget_sizes(&mut self, paths: impl IntoIterator<Item = impl AsRef<Path>>) {
        for path in paths {
            self.sizes.discard(path.as_ref());
            dirsize::invalidate(path.as_ref());
        }
    }

    /// The paths an undoable change touched, whose totals (and their parents') are out of date.
    pub(super) fn forget_entry_sizes(&mut self, entry: &UndoEntry) {
        for edit in &entry.edits {
            match edit {
                Edit::Trash(p) => dirsize::invalidate(p),
                Edit::Move { from, to } => {
                    dirsize::invalidate(from);
                    dirsize::invalidate(to);
                }
                Edit::Renames(pairs) => pairs.iter().for_each(|(a, b)| {
                    dirsize::invalidate(a);
                    dirsize::invalidate(b);
                }),
                // A restore brings back something from the trash; where to is not known here
                Edit::Restore(_) => dirsize::clear(),
                Edit::Chmod { .. } => {}
            }
        }
    }

    /// The selected folders' sizes added up for the status line, or `None` when it should show
    /// files only. A trailing "…" means some are still being measured.
    pub(super) fn selection_folder_bytes(&self, folders: &[&FileItem]) -> Option<(u64, bool, bool)> {
        if self.settings.folder_sizes == FolderSizeMode::Off {
            return None;
        }
        let (mut bytes, mut waiting, mut partial) = (0, false, false);
        for folder in folders {
            match dirsize::totals(&folder.path) {
                Some(t) => {
                    bytes += t.shown();
                    partial |= t.partial;
                }
                None => waiting = true,
            }
        }
        // Without "Always" nothing measures them on its own: show files only until the user asks
        if waiting && self.settings.folder_sizes != FolderSizeMode::Always {
            return None;
        }
        Some((bytes, waiting, partial))
    }
}

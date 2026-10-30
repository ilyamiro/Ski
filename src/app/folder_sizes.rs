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
/// With "Always", this many folders are measured, starting with the rows on screen; the rest wait until they scroll into view.
const AUTO_ROWS: usize = 120;

#[derive(Default)]
pub(super) struct SizeScheduler {
    running: Vec<(PathBuf, FolderScan)>,
    queue: VecDeque<PathBuf>,
    counting: Arc<AtomicBool>,
    /// A count was started and its results not sorted in yet.
    counts_unsorted: bool,
}

impl SizeScheduler {
    /// Asks for `path` to be measured, unless it is known or on its way.
    pub(super) fn request(&mut self, path: &Path) {
        self.request_all(std::iter::once(path));
    }

    /// `request` for many folders at once.
    pub(super) fn request_all<'a>(&mut self, paths: impl Iterator<Item = &'a Path>) {
        self.queue.extend(dirsize::claim(paths));
    }

    /// `request`, but ahead of everything that waits: what scrolled into view is measured next.
    pub(super) fn prioritize(&mut self, path: &Path) {
        if dirsize::totals(path).is_some() {
            return;
        }
        if !dirsize::is_pending(path) {
            dirsize::mark_pending(path);
        } else if let Some(at) = self.queue.iter().position(|p| p == path) {
            self.queue.remove(at);
        } else {
            return; // already being measured
        }
        self.queue.push_front(path.to_path_buf());
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

/// The rows whose rectangle is in `viewport`, as a range (the rows are in order).
pub(super) fn visible_rows(rects: &[(usize, egui::Rect)], viewport: egui::Rect) -> std::ops::Range<usize> {
    let mut seen = rects.iter().filter(|(_, r)| r.intersects(viewport)).map(|(i, _)| *i);
    match seen.next() {
        Some(first) => first..seen.next_back().unwrap_or(first) + 1,
        None => 0..0,
    }
}

impl SkiApp {
    /// The listing with the rows on screen first, then the ones above them, then the ones below.
    fn visible_first(&self) -> impl Iterator<Item = &FileItem> {
        let end = self.visible_rows.end.min(self.items.len());
        let start = self.visible_rows.start.min(end);
        self.items[start..end].iter().chain(&self.items[..start]).chain(&self.items[end..])
    }

    /// Once per frame: count and queue the folders that should be measured, start the scans, and
    /// sort again when item counts arrived and the listing is ordered by size (folders go by count).
    pub(super) fn tick_folder_sizes(&mut self, ctx: &egui::Context) {
        dirsize::set_size_on_disk(self.settings.size_on_disk);
        let counting = self.sizes.counting.load(Ordering::Relaxed);
        let counted = !counting && std::mem::take(&mut self.sizes.counts_unsorted);
        if !self.in_recent() && !counting {
            let missing = dirsize::uncounted(self.visible_first().filter(|i| i.is_dir && crate::fs::network::previews_allowed(&i.path)).map(|i| i.path.as_path()), 400);
            if !missing.is_empty() {
                dirsize::count_in_background(missing, Arc::clone(&self.sizes.counting));
                self.sizes.counts_unsorted = true;
            }
        }
        if self.settings.folder_sizes == FolderSizeMode::Always {
            let wanted: Vec<PathBuf> = self.visible_first().filter(|i| i.is_dir && crate::fs::network::previews_allowed(&i.path)).take(AUTO_ROWS).map(|i| i.path.clone()).collect();
            // Scrolling reorders what waits: the last one pushed to the front ends up first, so go backwards
            let end = self.visible_rows.end.min(self.items.len());
            for item in self.items[self.visible_rows.start.min(end)..end].iter().rev().filter(|i| i.is_dir && crate::fs::network::previews_allowed(&i.path)) {
                self.sizes.prioritize(&item.path);
            }
            // The selection feeds the status line
            let selected = self.selection.iter().filter_map(|&i| self.items.get(i)).filter(|i| i.is_dir && crate::fs::network::previews_allowed(&i.path)).map(|i| i.path.as_path());
            self.sizes.request_all(wanted.iter().map(PathBuf::as_path).chain(selected));
        }
        self.sizes.tick();
        if counted && self.sort_by == SortBy::Size {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_on_screen_become_a_range() {
        let rect = |y: f32| egui::Rect::from_min_size(egui::pos2(0.0, y), egui::vec2(100.0, 10.0));
        let rects: Vec<(usize, egui::Rect)> = (0..10).map(|i| (i, rect(i as f32 * 10.0))).collect();
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 35.0), egui::vec2(100.0, 30.0));
        assert_eq!(visible_rows(&rects, viewport), 3..7);
        assert_eq!(visible_rows(&rects, egui::Rect::from_min_size(egui::pos2(0.0, 500.0), egui::vec2(1.0, 1.0))), 0..0);
    }

    #[test]
    fn scrolled_to_folders_jump_the_queue() {
        let (a, b, c) = (Path::new("/ski-test/queue/a"), Path::new("/ski-test/queue/b"), Path::new("/ski-test/queue/c"));
        let mut scheduler = SizeScheduler::default();
        scheduler.request(a);
        scheduler.request(b);
        scheduler.request(c);
        scheduler.prioritize(c);
        scheduler.prioritize(b);
        assert_eq!(scheduler.queue, VecDeque::from([b.to_path_buf(), c.to_path_buf(), a.to_path_buf()]));
        scheduler.cancel_all();
    }
}

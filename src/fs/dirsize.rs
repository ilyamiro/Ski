//! What the listing knows about folders: how many items they hold and, once measured, how big
//! they are. A folder's own modification time does not change when a nested file does, so sizes
//! are never trusted by time; they live for the session and are dropped when Ski itself changes
//! something below them (or on refresh).
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use super::file_item::format_bytes;
use crate::i18n::trn;

/// The result of measuring a folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    /// Sum of file lengths.
    pub bytes: u64,
    /// Space the files occupy on disk (allocated blocks).
    pub disk: u64,
    pub files: u64,
    pub folders: u64,
    /// Something could not be read, so the real size is at least this.
    pub partial: bool,
}

#[derive(Default)]
struct Book {
    sizes: HashMap<PathBuf, Totals>,
    /// `None` for a folder that could not be read.
    counts: HashMap<PathBuf, Option<usize>>,
    /// Folders being measured or waiting for it.
    pending: HashSet<PathBuf>,
}

static BOOK: Mutex<Option<Book>> = Mutex::new(None);
static ON_DISK: AtomicBool = AtomicBool::new(false);

fn book() -> MutexGuard<'static, Option<Book>> {
    let mut guard = BOOK.lock().unwrap_or_else(|e| e.into_inner());
    guard.get_or_insert_with(Book::default);
    guard
}

fn with<R>(f: impl FnOnce(&mut Book) -> R) -> R {
    f(book().as_mut().expect("book exists"))
}

/// Show space used on disk instead of the sum of file lengths.
pub fn set_size_on_disk(on: bool) {
    ON_DISK.store(on, Ordering::Relaxed);
}

pub fn size_on_disk() -> bool {
    ON_DISK.load(Ordering::Relaxed)
}

impl Totals {
    /// The figure shown in the size column.
    pub fn shown(&self) -> u64 {
        if size_on_disk() { self.disk } else { self.bytes }
    }
}

pub fn totals(path: &Path) -> Option<Totals> {
    with(|b| b.sizes.get(path).copied())
}

/// The size to sort a folder by, when it is known.
pub fn known_bytes(path: &Path) -> Option<u64> {
    totals(path).map(|t| t.shown())
}

pub fn is_pending(path: &Path) -> bool {
    with(|b| b.pending.contains(path))
}

pub fn mark_pending(path: &Path) {
    with(|b| b.pending.insert(path.to_path_buf()));
}

pub fn store(path: &Path, totals: Totals) {
    with(|b| {
        b.pending.remove(path);
        b.sizes.insert(path.to_path_buf(), totals);
    });
}

pub fn store_count(path: &Path, count: Option<usize>) {
    with(|b| b.counts.insert(path.to_path_buf(), count));
}

pub fn has_count(path: &Path) -> bool {
    with(|b| b.counts.contains_key(path))
}

pub fn drop_pending() {
    with(|b| b.pending.clear());
}

/// Forget `path`, everything below it and every folder above it: their totals include it.
pub fn invalidate(path: &Path) {
    with(|b| {
        b.sizes.retain(|p, _| !(p.starts_with(path) || path.starts_with(p)));
        b.counts.retain(|p, _| !(p.starts_with(path) || path.starts_with(p)));
    });
}

/// Forget the measurement of exactly this folder, so it can be measured again.
pub fn forget(path: &Path) {
    with(|b| b.sizes.remove(path));
}

/// Forget everything (refresh).
pub fn clear() {
    with(|b| {
        b.sizes.clear();
        b.counts.clear();
    });
}

/// The text for a folder in the size column: its size, "…" while it is measured, the item count
/// when known, otherwise a dash.
pub fn label(path: &Path) -> String {
    with(|b| {
        if let Some(t) = b.sizes.get(path) {
            let text = format_bytes(t.shown());
            return if t.partial { format!("≥ {text}") } else { text };
        }
        if b.pending.contains(path) {
            return "…".to_owned();
        }
        match b.counts.get(path) {
            Some(Some(n)) => trn(*n, "{} item", "{} items"),
            _ => "—".to_owned(),
        }
    })
}

/// Counts the entries of each folder on a worker thread; `busy` stays true until it is done.
pub fn count_in_background(folders: Vec<PathBuf>, busy: std::sync::Arc<AtomicBool>) {
    busy.store(true, Ordering::Relaxed);
    std::thread::spawn(move || {
        for folder in folders.iter().filter(|f| !has_count(f)) {
            store_count(folder, std::fs::read_dir(folder).ok().map(|read| read.take(100_000).count()));
        }
        busy.store(false, Ordering::Relaxed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_what_is_known() {
        let p = Path::new("/ski-test/labels");
        assert_eq!(label(p), "—");
        store_count(p, Some(3));
        assert_eq!(label(p), "3 items");
        mark_pending(p);
        assert_eq!(label(p), "…");
        store(p, Totals { bytes: 2048, disk: 4096, files: 2, folders: 1, partial: false });
        assert_eq!(label(p), format_bytes(2048));
        assert_eq!(known_bytes(p), Some(2048));
        store(p, Totals { bytes: 10, disk: 10, files: 1, folders: 0, partial: true });
        assert_eq!(label(p), format!("≥ {}", format_bytes(10)));
    }

    #[test]
    fn invalidating_drops_the_folder_its_parents_and_its_children() {
        let t = Totals { bytes: 1, disk: 1, files: 1, folders: 0, partial: false };
        for p in ["/ski-test/inv", "/ski-test/inv/a", "/ski-test/inv/a/b", "/ski-test/inv/other", "/ski-test/elsewhere"] {
            store(Path::new(p), t);
        }
        invalidate(Path::new("/ski-test/inv/a"));
        assert!(totals(Path::new("/ski-test/inv")).is_none());
        assert!(totals(Path::new("/ski-test/inv/a")).is_none());
        assert!(totals(Path::new("/ski-test/inv/a/b")).is_none());
        assert!(totals(Path::new("/ski-test/inv/other")).is_some());
        assert!(totals(Path::new("/ski-test/elsewhere")).is_some());
    }
}

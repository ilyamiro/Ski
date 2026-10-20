//! Undo and redo. Every change that can be taken back is stored as the list of edits that
//! reverse it. Applying an entry returns the entry that reverses *that*, which goes on the
//! other stack, so one mechanism serves both directions.
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::{ops, trash};

/// One step that takes back (part of) a change.
#[derive(Debug, Clone)]
pub enum Edit {
    /// Move to the trash.
    Trash(PathBuf),
    /// Bring a trashed item back to where it came from.
    Restore(PathBuf),
    /// Move or rename to the exact path.
    Move { from: PathBuf, to: PathBuf },
    /// Rename several items at once; they may swap names.
    Renames(Vec<(PathBuf, PathBuf)>),
    Chmod { path: PathBuf, mode: u32 },
}

#[derive(Debug, Clone)]
pub struct UndoEntry {
    /// What the change was, for the toast and the menu: "Move of 3 items".
    pub label: String,
    pub edits: Vec<Edit>,
}

impl UndoEntry {
    pub fn new(label: impl Into<String>, edits: Vec<Edit>) -> Self {
        Self { label: label.into(), edits }
    }
}

const LIMIT: usize = 100;

#[derive(Default)]
pub struct History {
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
}

impl History {
    /// A new change: it can be undone, and anything that was undone before can no longer be redone.
    pub fn record(&mut self, entry: UndoEntry) {
        if entry.edits.is_empty() {
            return;
        }
        self.redo.clear();
        self.push_undo(entry);
    }

    pub fn push_undo(&mut self, entry: UndoEntry) {
        self.undo.push(entry);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn push_redo(&mut self, entry: UndoEntry) {
        self.redo.push(entry);
        if self.redo.len() > LIMIT {
            self.redo.remove(0);
        }
    }

    pub fn pop_undo(&mut self) -> Option<UndoEntry> {
        self.undo.pop()
    }

    pub fn pop_redo(&mut self) -> Option<UndoEntry> {
        self.redo.pop()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }
}

/// Carry out an entry. Returns the entry that reverses it, plus the first error if some edit
/// failed (the edits that worked are still reversible).
pub fn apply(entry: UndoEntry) -> (Option<UndoEntry>, Option<String>) {
    let mut inverse = Vec::new();
    let mut error = None;
    for edit in entry.edits {
        let result = match edit {
            Edit::Trash(path) => trash::trash(&path).map(Edit::Restore),
            Edit::Restore(trashed) => trash::restore(&trashed).map(Edit::Trash),
            Edit::Move { from, to } => {
                if to.symlink_metadata().is_ok() {
                    Err(io::Error::other(format!("“{}” already exists", to.display())))
                } else {
                    if let Some(parent) = to.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    ops::move_path(&from, &to).map(|_| Edit::Move { from: to, to: from })
                }
            }
            Edit::Renames(pairs) => rename_many(&pairs).map(|_| Edit::Renames(pairs.into_iter().map(|(a, b)| (b, a)).collect())),
            Edit::Chmod { path, mode } => std::fs::metadata(&path).and_then(|meta| {
                let old = meta.permissions().mode() & 0o7777;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).map(|_| Edit::Chmod { path, mode: old })
            }),
        };
        match result {
            Ok(reverse) => inverse.push(reverse),
            Err(err) => {
                error.get_or_insert(err.to_string());
            }
        }
    }
    inverse.reverse();
    (if inverse.is_empty() { None } else { Some(UndoEntry { label: entry.label, edits: inverse }) }, error)
}

/// Rename `(from, to)` pairs together, going through temporary names so names can be swapped.
pub fn rename_many(pairs: &[(PathBuf, PathBuf)]) -> io::Result<()> {
    let mut staged: Vec<(&Path, PathBuf, &Path)> = Vec::new();
    for (i, (from, to)) in pairs.iter().enumerate() {
        let temp = from.with_file_name(format!(".ski-rename-{}-{i}", std::process::id()));
        if let Err(err) = std::fs::rename(from, &temp) {
            for (old, temp, _) in staged.into_iter().rev() {
                let _ = std::fs::rename(temp, old);
            }
            return Err(err);
        }
        staged.push((from, temp, to));
    }
    let mut failure = None;
    for (from, temp, to) in staged {
        if let Err(err) = std::fs::rename(&temp, to) {
            let _ = std::fs::rename(&temp, from);
            failure.get_or_insert(err);
        }
    }
    failure.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_swap_and_chmod_round_trip() {
        let dir = std::env::temp_dir().join(format!("ski_history_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::write(&a, "A").unwrap();
        std::fs::write(&b, "B").unwrap();

        let entry = UndoEntry::new("swap", vec![Edit::Renames(vec![(a.clone(), b.clone()), (b.clone(), a.clone())])]);
        let (inverse, error) = apply(entry);
        assert!(error.is_none());
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "B");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "A");
        let (back, _) = apply(inverse.unwrap());
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "A");
        assert!(back.is_some());

        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (inverse, _) = apply(UndoEntry::new("x", vec![Edit::Chmod { path: a.clone(), mode: 0o755 }]));
        assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o755);
        apply(inverse.unwrap());
        assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o644);

        let mut history = History::default();
        history.record(UndoEntry::new("one", vec![Edit::Trash(a.clone())]));
        let undone = history.pop_undo().unwrap();
        history.push_redo(undone);
        assert_eq!(history.redo_label(), Some("one"));
        history.record(UndoEntry::new("two", vec![Edit::Trash(b.clone())]));
        assert!(history.redo_label().is_none(), "a new change drops the redo stack");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

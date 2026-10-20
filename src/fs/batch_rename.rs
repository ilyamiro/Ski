//! Renaming several files at once, from a template or by find and replace.
//!
//! Both modes work on the name without its extension, so a file never loses its type by accident.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const TOKEN_NAME: &str = "[Original name]";
pub const TOKEN_NUM_1: &str = "[1, 2, 3]";
pub const TOKEN_NUM_2: &str = "[01, 02, 03]";
pub const TOKEN_NUM_3: &str = "[001, 002, 003]";
pub const TOKEN_DATE: &str = "[Modified date]";

/// Tokens offered next to the template field, with the label shown on their buttons.
pub const TOKENS: [(&str, &str); 5] = [
    (TOKEN_NAME, "Original name"),
    (TOKEN_NUM_1, "1, 2, 3"),
    (TOKEN_NUM_2, "01, 02, 03"),
    (TOKEN_NUM_3, "001, 002, 003"),
    (TOKEN_DATE, "Modified date"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Template,
    Replace,
}

/// One file to rename, with what the tokens need to know about it.
#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub is_dir: bool,
    /// `YYYY-MM-DD`, empty when unknown.
    pub date: String,
}

/// What the user typed in the dialog.
pub struct Rules {
    pub mode: Mode,
    pub template: String,
    pub find: String,
    pub replace: String,
}

impl Rules {
    pub fn new() -> Self {
        Self { mode: Mode::Template, template: TOKEN_NAME.to_owned(), find: String::new(), replace: String::new() }
    }
}

/// Splits a file name into the part that gets renamed and the extension that stays.
pub fn split_name(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if !is_dir && i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// The name each entry gets, in the same order. Unchanged entries keep their name.
pub fn preview(entries: &[Entry], rules: &Rules) -> Vec<String> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let name = entry.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (stem, ext) = split_name(&name, entry.is_dir);
            let new_stem = match rules.mode {
                Mode::Template => expand(&rules.template, stem, index, &entry.date),
                Mode::Replace if rules.find.is_empty() => stem.to_owned(),
                Mode::Replace => stem.replace(&rules.find, &rules.replace),
            };
            format!("{new_stem}{ext}")
        })
        .collect()
}

fn expand(template: &str, stem: &str, index: usize, date: &str) -> String {
    let n = index + 1;
    // The original name goes in last so a name that looks like a token is left alone
    template
        .replace(TOKEN_NUM_3, &format!("{n:03}"))
        .replace(TOKEN_NUM_2, &format!("{n:02}"))
        .replace(TOKEN_NUM_1, &n.to_string())
        .replace(TOKEN_DATE, date)
        .replace(TOKEN_NAME, stem)
}

/// Why the result cannot be applied, if it cannot. Looks at names only, not at the disk.
pub fn problem(entries: &[Entry], names: &[String]) -> Option<String> {
    let mut seen = HashSet::new();
    for (entry, new) in entries.iter().zip(names) {
        let trimmed = new.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == ".." || trimmed.contains('/') {
            return Some("Some names would be empty or invalid".to_owned());
        }
        let dir = entry.path.parent().map(Path::to_path_buf).unwrap_or_default();
        if !seen.insert((dir, new.clone())) {
            return Some(format!("More than one file would be named “{new}”"));
        }
    }
    // A target that already exists on disk is fine only when that file is renamed too
    let renamed: HashSet<&Path> = entries.iter().map(|e| e.path.as_path()).collect();
    for (entry, new) in entries.iter().zip(names) {
        let target = entry.path.with_file_name(new);
        if target != entry.path && target.symlink_metadata().is_ok() && !renamed.contains(target.as_path()) {
            return Some(format!("“{new}” already exists"));
        }
    }
    None
}

/// Applies the names. Goes through temporary names so files can swap names. Returns the paths
/// that were renamed (old, new) and the first error, if any.
pub fn apply(entries: &[Entry], names: &[String]) -> (Vec<(PathBuf, PathBuf)>, Option<String>) {
    let changes: Vec<(&Entry, &String)> = entries
        .iter()
        .zip(names)
        .filter(|(e, n)| e.path.file_name().is_some_and(|old| old.to_string_lossy() != n.as_str()))
        .collect();

    let mut staged: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new(); // old, temp, final
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (i, (entry, new)) in changes.iter().enumerate() {
        let temp = entry.path.with_file_name(format!(".ski-rename-{}-{i}", std::process::id()));
        if let Err(err) = std::fs::rename(&entry.path, &temp) {
            // Put back what was already moved aside
            for (old, temp, _) in staged.into_iter().rev() {
                let _ = std::fs::rename(&temp, &old);
            }
            return (done, Some(err.to_string()));
        }
        staged.push((entry.path.clone(), temp, entry.path.with_file_name(new.as_str())));
    }
    let mut failure = None;
    for (old, temp, target) in staged {
        match std::fs::rename(&temp, &target) {
            Ok(()) => done.push((old, target)),
            Err(err) => {
                let _ = std::fs::rename(&temp, &old);
                failure.get_or_insert(err.to_string());
            }
        }
    }
    (done, failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> Entry {
        Entry { path: PathBuf::from(path), is_dir: false, date: "2026-01-02".to_owned() }
    }

    fn rules(template: &str) -> Rules {
        Rules { template: template.to_owned(), ..Rules::new() }
    }

    #[test]
    fn template_keeps_extension_and_numbers_files() {
        let entries = [entry("/d/a.pdf"), entry("/d/b.tar.gz"), entry("/d/c")];
        let names = preview(&entries, &rules("Trip [01, 02, 03] [Original name]"));
        assert_eq!(names, ["Trip 01 a.pdf", "Trip 02 b.tar.gz", "Trip 03 c"]);
    }

    #[test]
    fn plain_numbers_and_date() {
        let entries = [entry("/d/a.txt"), entry("/d/b.txt")];
        assert_eq!(preview(&entries, &rules("[Modified date]-[1, 2, 3]")), ["2026-01-02-1.txt", "2026-01-02-2.txt"]);
        assert_eq!(preview(&entries, &rules("x[001, 002, 003]")), ["x001.txt", "x002.txt"]);
    }

    #[test]
    fn find_and_replace_leaves_the_extension_alone() {
        let entries = [entry("/d/IMG_1.jpg"), entry("/d/jpg_notes.txt")];
        let rules = Rules { mode: Mode::Replace, find: "jpg".to_owned(), replace: "pic".to_owned(), ..Rules::new() };
        assert_eq!(preview(&entries, &rules), ["IMG_1.jpg", "pic_notes.txt"]);
        let empty = Rules { mode: Mode::Replace, ..Rules::new() };
        assert_eq!(preview(&entries, &empty), ["IMG_1.jpg", "jpg_notes.txt"]);
    }

    #[test]
    fn problems_are_reported() {
        let entries = [entry("/d/a.txt"), entry("/d/b.txt")];
        assert!(problem(&entries, &["x.txt".into(), "x.txt".into()]).is_some());
        assert!(problem(&entries, &["".into(), "y.txt".into()]).is_some());
        assert!(problem(&entries, &["a/b".into(), "y.txt".into()]).is_some());
        assert!(problem(&entries, &["b.txt".into(), "a.txt".into()]).is_none());
    }

    #[test]
    fn apply_renames_and_swaps() {
        let dir = std::env::temp_dir().join(format!("ski-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "A").unwrap();
        std::fs::write(dir.join("b.txt"), "B").unwrap();
        let entries = [
            Entry { path: dir.join("a.txt"), is_dir: false, date: String::new() },
            Entry { path: dir.join("b.txt"), is_dir: false, date: String::new() },
        ];
        let names = vec!["b.txt".to_owned(), "a.txt".to_owned()];
        assert!(problem(&entries, &names).is_none());
        let (done, err) = apply(&entries, &names);
        assert!(err.is_none());
        assert_eq!(done.len(), 2);
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "A");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "B");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

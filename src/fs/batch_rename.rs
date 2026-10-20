//! Renaming several files at once, from a template or by find and replace.
//!
//! Both modes work on the name without its extension, so a file never loses its type by accident.

use std::collections::HashSet;

use crate::i18n::{tr, trf};
use std::path::{Path, PathBuf};

pub const TOKEN_NAME: &str = "[Original name]";
pub const TOKEN_NUM_1: &str = "[1, 2, 3]";
pub const TOKEN_NUM_2: &str = "[01, 02, 03]";
pub const TOKEN_NUM_3: &str = "[001, 002, 003]";
pub const TOKEN_DATE: &str = "[Modified date]";
pub const TOKEN_FOLDER: &str = "[Parent folder]";
pub const TOKEN_EXT: &str = "[Extension]";

/// Tokens offered next to the template field, with the label shown on their buttons.
pub const TOKENS: [(&str, &str); 7] = [
    (TOKEN_NAME, "Original name"),
    (TOKEN_NUM_1, "1, 2, 3"),
    (TOKEN_NUM_2, "01, 02, 03"),
    (TOKEN_NUM_3, "001, 002, 003"),
    (TOKEN_DATE, "Modified date"),
    (TOKEN_FOLDER, "Parent folder"),
    (TOKEN_EXT, "Extension"),
];

/// Letter case applied to the new name, after the template or replacement.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Case {
    Keep,
    Lower,
    Upper,
    Title,
    Sentence,
}

impl Case {
    pub const ALL: [(Case, &'static str); 5] =
        [(Case::Keep, "Keep"), (Case::Lower, "lowercase"), (Case::Upper, "UPPERCASE"), (Case::Title, "Title Case"), (Case::Sentence, "Sentence case")];

    fn apply(self, text: &str) -> String {
        match self {
            Case::Keep => text.to_owned(),
            Case::Lower => text.to_lowercase(),
            Case::Upper => text.to_uppercase(),
            Case::Title => {
                let mut start = true;
                text.chars()
                    .map(|c| {
                        let out = if start { c.to_uppercase().collect::<String>() } else { c.to_lowercase().collect() };
                        start = !c.is_alphanumeric() && c != '\'';
                        out
                    })
                    .collect()
            }
            Case::Sentence => {
                let mut chars = text.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
                    None => String::new(),
                }
            }
        }
    }
}

/// What to do with spaces in the new name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Spaces {
    Keep,
    Underscore,
    Dash,
    Remove,
}

impl Spaces {
    pub const ALL: [(Spaces, &'static str); 4] = [(Spaces::Keep, "Keep"), (Spaces::Underscore, "→ _"), (Spaces::Dash, "→ -"), (Spaces::Remove, "Remove")];
}

/// Order in which files are numbered.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Order {
    AsListed,
    Name,
    Date,
}

impl Order {
    pub const ALL: [(Order, &'static str); 3] = [(Order::AsListed, "As listed"), (Order::Name, "By name"), (Order::Date, "By date")];
}

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
    /// Find and replace only matches the exact letter case.
    pub match_case: bool,
    pub case: Case,
    pub spaces: Spaces,
    pub prefix: String,
    pub suffix: String,
    /// Characters cut from the start / end of the original name, as typed.
    pub trim_start: String,
    pub trim_end: String,
    /// Number the first file gets, and how much each next one adds, as typed.
    pub start: String,
    pub step: String,
    pub order: Order,
    pub lowercase_ext: bool,
}

impl Rules {
    pub fn new() -> Self {
        Self {
            mode: Mode::Template,
            template: TOKEN_NAME.to_owned(),
            find: String::new(),
            replace: String::new(),
            match_case: true,
            case: Case::Keep,
            spaces: Spaces::Keep,
            prefix: String::new(),
            suffix: String::new(),
            trim_start: String::new(),
            trim_end: String::new(),
            start: "1".to_owned(),
            step: "1".to_owned(),
            order: Order::AsListed,
            lowercase_ext: false,
        }
    }

    fn start_number(&self) -> i64 {
        self.start.trim().parse().unwrap_or(1)
    }

    fn step_number(&self) -> i64 {
        self.step.trim().parse().unwrap_or(1)
    }
}

fn trim_chars(text: &str, start: usize, end: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let start = start.min(chars.len());
    let end = end.min(chars.len() - start);
    chars[start..chars.len() - end].iter().collect()
}

fn replace_text(text: &str, find: &str, with: &str, match_case: bool) -> String {
    if match_case {
        return text.replace(find, with);
    }
    let (lower_text, lower_find) = (text.to_lowercase(), find.to_lowercase());
    // Lowercasing can change byte lengths; fall back to an exact match when it does
    if lower_text.len() != text.len() || lower_find.len() != find.len() {
        return text.replace(find, with);
    }
    let mut out = String::new();
    let mut at = 0;
    while let Some(i) = lower_text[at..].find(&lower_find) {
        out.push_str(&text[at..at + i]);
        out.push_str(with);
        at += i + find.len();
    }
    out.push_str(&text[at..]);
    out
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
    // Numbers follow the chosen order, not the order of the list
    let mut ranked: Vec<usize> = (0..entries.len()).collect();
    match rules.order {
        Order::AsListed => {}
        Order::Name => ranked.sort_by_key(|&i| entries[i].path.file_name().map(|n| n.to_string_lossy().to_lowercase())),
        Order::Date => ranked.sort_by(|&a, &b| entries[a].date.cmp(&entries[b].date)),
    }
    let mut rank = vec![0usize; entries.len()];
    for (position, &i) in ranked.iter().enumerate() {
        rank[i] = position;
    }
    let (start, step) = (rules.start_number(), rules.step_number());
    let (cut_start, cut_end) = (rules.trim_start.trim().parse().unwrap_or(0), rules.trim_end.trim().parse().unwrap_or(0));

    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let name = entry.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (stem, ext) = split_name(&name, entry.is_dir);
            let folder = entry.path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let number = start + rank[index] as i64 * step;
            let base = trim_chars(stem, cut_start, cut_end);
            let mut new_stem = match rules.mode {
                Mode::Template => expand(&rules.template, &base, number, &entry.date, &folder, ext.trim_start_matches('.')),
                Mode::Replace if rules.find.is_empty() => base,
                Mode::Replace => replace_text(&base, &rules.find, &rules.replace, rules.match_case),
            };
            new_stem = match rules.spaces {
                Spaces::Keep => new_stem,
                Spaces::Underscore => new_stem.replace(' ', "_"),
                Spaces::Dash => new_stem.replace(' ', "-"),
                Spaces::Remove => new_stem.replace(' ', ""),
            };
            new_stem = rules.case.apply(&new_stem);
            let ext = if rules.lowercase_ext { ext.to_lowercase() } else { ext.to_owned() };
            format!("{}{new_stem}{}{ext}", rules.prefix, rules.suffix)
        })
        .collect()
}

fn expand(template: &str, stem: &str, number: i64, date: &str, folder: &str, ext: &str) -> String {
    // The original name goes in last so a name that looks like a token is left alone
    template
        .replace(TOKEN_NUM_3, &format!("{number:03}"))
        .replace(TOKEN_NUM_2, &format!("{number:02}"))
        .replace(TOKEN_NUM_1, &number.to_string())
        .replace(TOKEN_DATE, date)
        .replace(TOKEN_FOLDER, folder)
        .replace(TOKEN_EXT, ext)
        .replace(TOKEN_NAME, stem)
}

/// Why the result cannot be applied, if it cannot. Looks at names only, not at the disk.
pub fn problem(entries: &[Entry], names: &[String]) -> Option<String> {
    let mut seen = HashSet::new();
    for (entry, new) in entries.iter().zip(names) {
        let trimmed = new.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == ".." || trimmed.contains('/') {
            return Some(tr("Some names would be empty or invalid").to_owned());
        }
        let dir = entry.path.parent().map(Path::to_path_buf).unwrap_or_default();
        if !seen.insert((dir, new.clone())) {
            return Some(trf("More than one file would be named “{}”", &[&new]));
        }
    }
    // A target that already exists on disk is fine only when that file is renamed too
    let renamed: HashSet<&Path> = entries.iter().map(|e| e.path.as_path()).collect();
    for (entry, new) in entries.iter().zip(names) {
        let target = entry.path.with_file_name(new);
        if target != entry.path && target.symlink_metadata().is_ok() && !renamed.contains(target.as_path()) {
            return Some(trf("“{}” already exists", &[&new]));
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
    fn numbering_start_step_and_order() {
        let entries = [entry("/d/b.txt"), entry("/d/a.txt")];
        let r = Rules { template: "n[01, 02, 03]".to_owned(), start: "10".to_owned(), step: "5".to_owned(), order: Order::Name, ..Rules::new() };
        assert_eq!(preview(&entries, &r), ["n15.txt", "n10.txt"]);
    }

    #[test]
    fn case_spaces_trim_prefix_and_extension() {
        let entries = [entry("/d/hello big World.JPG")];
        let r = Rules { case: Case::Title, spaces: Spaces::Underscore, prefix: "p_".to_owned(), lowercase_ext: true, ..Rules::new() };
        assert_eq!(preview(&entries, &r), ["p_Hello_Big_World.jpg"]);
        let r = Rules { trim_start: "6".to_owned(), trim_end: "2".to_owned(), suffix: "!".to_owned(), ..Rules::new() };
        assert_eq!(preview(&entries, &r), ["big Wor!.JPG"]);
        let r = Rules { case: Case::Sentence, ..Rules::new() };
        assert_eq!(preview(&entries, &r), ["Hello big world.JPG"]);
    }

    #[test]
    fn folder_and_extension_tokens() {
        let entries = [entry("/d/photos/a.png")];
        assert_eq!(preview(&entries, &rules("[Parent folder]-[Original name]-[Extension]")), ["photos-a-png.png"]);
    }

    #[test]
    fn case_insensitive_replace() {
        let entries = [entry("/d/Foo foo FOO.txt")];
        let r = Rules { mode: Mode::Replace, find: "foo".to_owned(), replace: "x".to_owned(), match_case: false, ..Rules::new() };
        assert_eq!(preview(&entries, &r), ["x x x.txt"]);
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

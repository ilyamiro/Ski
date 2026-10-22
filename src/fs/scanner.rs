use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::Path;

use super::file_item::{FileItem, FileType};
use crate::i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortBy {
    Name,
    Size,
    Type,
    Modified,
    Created,
    Accessed,
    Owner,
    Permissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortOrder {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeFilter {
    All,
    Folders,
    Documents,
    Images,
    Audio,
    Video,
    Archives,
    Code,
}

impl TypeFilter {
    pub const ALL: [TypeFilter; 8] = [
        TypeFilter::All,
        TypeFilter::Folders,
        TypeFilter::Documents,
        TypeFilter::Images,
        TypeFilter::Audio,
        TypeFilter::Video,
        TypeFilter::Archives,
        TypeFilter::Code,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            TypeFilter::All => tr("All"),
            TypeFilter::Folders => tr("Folders"),
            TypeFilter::Documents => tr("Documents"),
            TypeFilter::Images => tr("Images"),
            TypeFilter::Audio => tr("Audio"),
            TypeFilter::Video => tr("Video"),
            TypeFilter::Archives => tr("Archives"),
            TypeFilter::Code => tr("Code"),
        }
    }

    /// Folders stay visible under every filter except `Folders` itself, so navigation never breaks.
    fn matches(&self, item: &FileItem) -> bool {
        let wanted = match self {
            TypeFilter::All => return true,
            TypeFilter::Folders => return item.is_dir,
            TypeFilter::Documents => FileType::Document,
            TypeFilter::Images => FileType::Image,
            TypeFilter::Audio => FileType::Audio,
            TypeFilter::Video => FileType::Video,
            TypeFilter::Archives => FileType::Archive,
            TypeFilter::Code => FileType::Code,
        };
        item.is_dir || item.file_type == wanted
    }
}

impl TypeFilter {
    /// Strict variant for mixed listings (search, recent): folders only count under `Folders`.
    pub fn accepts(&self, item: &FileItem) -> bool {
        match self {
            TypeFilter::All => true,
            TypeFilter::Folders => item.is_dir,
            _ => !item.is_dir && self.matches(item),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListOptions {
    pub show_hidden: bool,
    pub sort_by: SortBy,
    pub sort_order: SortOrder,
    pub folders_first: bool,
    pub type_filter: TypeFilter,
}

pub fn read_directory(path: &Path, opts: &ListOptions) -> io::Result<Vec<FileItem>> {
    let mut items = Vec::new();

    for entry in fs::read_dir(path)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue, // Skip unreadable entries gracefully
        };

        if let Some(item) = FileItem::from_path(&entry.path()) {
            if !opts.show_hidden && item.is_hidden {
                continue;
            }

            if !opts.type_filter.matches(&item) {
                continue;
            }

            items.push(item);
        }
    }

    sort_items(&mut items, opts);

    Ok(items)
}

/// Accented Latin letters grouped under the letter they are read as, the way locale-aware collation does.
const FOLDS: &[(&str, char)] = &[
    ("àáâãäåāăąǎ", 'a'), ("æ", 'a'), ("çćĉċč", 'c'), ("ďđ", 'd'), ("èéêëēĕėęě", 'e'),
    ("ĝğġģ", 'g'), ("ĥħ", 'h'), ("ìíîïĩīĭįıǐ", 'i'), ("ĵ", 'j'), ("ķ", 'k'), ("ĺļľŀł", 'l'),
    ("ñńņňŉ", 'n'), ("òóôõöøōŏőǒœ", 'o'), ("ŕŗř", 'r'), ("śŝşšß", 's'), ("ţťŧ", 't'),
    ("ùúûüũūŭůűųǔ", 'u'), ("ŵ", 'w'), ("ýÿŷ", 'y'), ("źżž", 'z'),
];

/// The letter a character sorts as: lowercase, with accents dropped. Combining marks have none.
fn collation_key(c: char) -> Option<char> {
    if ('\u{300}'..='\u{36f}').contains(&c) {
        return None;
    }
    let lower = c.to_lowercase().next().unwrap_or(c);
    if lower.is_ascii() {
        return Some(lower);
    }
    Some(FOLDS.iter().find(|(set, _)| set.contains(lower)).map_or(lower, |(_, base)| *base))
}

/// Whether `name` begins with `prefix`, ignoring case and accents.
pub fn starts_with_folded(name: &str, prefix: &str) -> bool {
    let mut name = name.chars().filter_map(collation_key);
    prefix.chars().filter_map(collation_key).all(|k| name.next() == Some(k))
}

/// Locale-style comparison: case and accents are ignored, and runs of digits read as numbers,
/// so `file2` sorts before `file10` and `école` among the `e`s. Decomposed accents (as macOS
/// writes them) sort the same as composed ones.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let keys = |s: &str| s.chars().filter_map(|c| collation_key(c).map(|k| (k, c))).collect::<Vec<_>>();
    let (a, b) = (keys(a), keys(b));
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (x, y) = (a[i].0, b[j].0);
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let run = |v: &[(char, char)], at: usize| v[at..].iter().take_while(|k| k.0.is_ascii_digit()).count();
            let (la, lb) = (run(&a, i), run(&b, j));
            let digits = |v: &[(char, char)], at: usize, len: usize| v[at..at + len].iter().map(|k| k.0).collect::<String>();
            let (da, db) = (digits(&a, i, la), digits(&b, j, lb));
            let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
            let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
            if ord != Ordering::Equal {
                return ord;
            }
            i += la;
            j += lb;
        } else {
            if x != y {
                return x.cmp(&y);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

fn extension_of(item: &FileItem) -> String {
    if item.is_dir {
        return String::new();
    }
    item.path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// The name as shown, falling back to the full name and then raw bytes so the order never depends on `read_dir`.
fn name_cmp(a: &FileItem, b: &FileItem) -> Ordering {
    natural_cmp(a.display_name(), b.display_name())
        .then_with(|| natural_cmp(&a.name, &b.name))
        .then_with(|| a.name.cmp(&b.name))
}

pub fn sort_items(items: &mut [FileItem], opts: &ListOptions) {
    let (sort_by, sort_order) = (opts.sort_by, opts.sort_order);
    let owners: std::collections::HashMap<u32, String> = if sort_by == SortBy::Owner {
        items.iter().map(|i| (i.uid, i.owner())).collect()
    } else {
        Default::default()
    };
    items.sort_by(|a, b| {
        // Group directories first when enabled
        if opts.folders_first && a.is_dir != b.is_dir {
            return if a.is_dir {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }

        let ordering = match sort_by {
            SortBy::Name => name_cmp(a, b),
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Type => extension_of(a).cmp(&extension_of(b)),
            SortBy::Modified => a.modified.cmp(&b.modified),
            SortBy::Created => a.created.cmp(&b.created),
            SortBy::Accessed => a.accessed.cmp(&b.accessed),
            SortBy::Owner => natural_cmp(&owners[&a.uid], &owners[&b.uid]),
            SortBy::Permissions => (a.mode & 0o7777).cmp(&(b.mode & 0o7777)),
        };

        // Ties on any column fall back to name, ascending whichever way the column is ordered
        match sort_order {
            SortOrder::Ascending => ordering,
            SortOrder::Descending => ordering.reverse(),
        }
        .then_with(|| name_cmp(a, b))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_sort_by_value() {
        let mut names = ["Episode 10", "episode 2", "Episode 1", "Episode 02a"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["Episode 1", "episode 2", "Episode 02a", "Episode 10"]);
    }

    #[test]
    fn accents_sort_with_their_letter() {
        let mut names = ["zebra", "école", "über", "apple", "Eagle"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["apple", "Eagle", "école", "über", "zebra"]);
        // composed and decomposed spellings of the same name tie, so the raw-name fallback decides
        assert_eq!(natural_cmp("e\u{301}cole", "\u{e9}cole"), Ordering::Equal);
    }

    #[test]
    fn prefixes_ignore_case_and_accents() {
        assert!(starts_with_folded("École.txt", "ec"));
        assert!(!starts_with_folded("e", "ec"));
    }

    #[test]
    fn prefix_sorts_first() {
        assert_eq!(natural_cmp("report", "report-notes"), Ordering::Less);
    }
}

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
    pub fn matches(&self, item: &FileItem) -> bool {
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

/// The listing of a folder as shown: for the trash folder it also holds what was deleted on mounted drives.
pub fn read_folder(path: &Path, opts: &ListOptions) -> io::Result<Vec<FileItem>> {
    let mut items = read_directory(path, opts)?;
    if super::trash::is_trash_folder(path) {
        for dir in super::trash::drive_files_dirs() {
            if let Ok(more) = read_directory(&dir, opts) {
                items.extend(more);
            }
        }
        sort_items(&mut items, opts);
    }
    Ok(items)
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
    // Walks both names in place: sorting a big folder compares names millions of times, so nothing is allocated here
    let mut a = a.chars().filter_map(collation_key).peekable();
    let mut b = b.chars().filter_map(collation_key).peekable();
    loop {
        let (x, y) = match (a.peek().copied(), b.peek().copied()) {
            (Some(x), Some(y)) => (x, y),
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        };
        if x.is_ascii_digit() && y.is_ascii_digit() {
            // Leading zeros carry no value; then the longer run is the bigger number, and runs of one length differ at their first unequal digit
            while a.next_if_eq(&'0').is_some() {}
            while b.next_if_eq(&'0').is_some() {}
            let mut first_difference = Ordering::Equal;
            loop {
                match (a.next_if(char::is_ascii_digit), b.next_if(char::is_ascii_digit)) {
                    (Some(x), Some(y)) => {
                        if first_difference == Ordering::Equal {
                            first_difference = x.cmp(&y);
                        }
                    }
                    (None, None) => break,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                }
            }
            if first_difference != Ordering::Equal {
                return first_difference;
            }
        } else {
            if x != y {
                return x.cmp(&y);
            }
            a.next();
            b.next();
        }
    }
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

/// What an item is sorted by besides its name, looked up once per item instead of on every comparison.
enum SortKey {
    None,
    /// `None` for a folder not measured yet.
    Size(Option<u64>),
    Text(String),
}

fn sort_key(item: &FileItem, sort_by: SortBy) -> SortKey {
    match sort_by {
        SortBy::Size if item.is_dir => SortKey::Size(super::dirsize::known_bytes(&item.path)),
        SortBy::Size => SortKey::Size(Some(item.size)),
        SortBy::Type => SortKey::Text(extension_of(item)),
        SortBy::Owner => SortKey::Text(item.owner()),
        _ => SortKey::None,
    }
}

pub fn sort_items(items: &mut [FileItem], opts: &ListOptions) {
    let (sort_by, sort_order) = (opts.sort_by, opts.sort_order);
    let keys: Vec<SortKey> = items.iter().map(|i| sort_key(i, sort_by)).collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&x, &y| {
        let (a, b) = (&items[x], &items[y]);
        // Group directories first when enabled
        if opts.folders_first && a.is_dir != b.is_dir {
            return if a.is_dir {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }

        let ordering = match (&keys[x], &keys[y]) {
            (SortKey::Size(sa), SortKey::Size(sb)) => {
                // Folders not measured yet follow everything with a size, whichever way the column is ordered
                let asc = sort_order == SortOrder::Ascending;
                match (sa, sb) {
                    (Some(sa), Some(sb)) => sa.cmp(sb),
                    (None, None) => Ordering::Equal,
                    (Some(_), None) => if asc { Ordering::Less } else { Ordering::Greater },
                    (None, Some(_)) => if asc { Ordering::Greater } else { Ordering::Less },
                }
            }
            (SortKey::Text(ta), SortKey::Text(tb)) if sort_by == SortBy::Owner => natural_cmp(ta, tb),
            (SortKey::Text(ta), SortKey::Text(tb)) => ta.cmp(tb),
            _ => match sort_by {
                SortBy::Modified => a.modified.cmp(&b.modified),
                SortBy::Created => a.created.cmp(&b.created),
                SortBy::Accessed => a.accessed.cmp(&b.accessed),
                SortBy::Permissions => (a.mode & 0o7777).cmp(&(b.mode & 0o7777)),
                _ => name_cmp(a, b),
            },
        };

        // Ties on any column fall back to name, ascending whichever way the column is ordered
        match sort_order {
            SortOrder::Ascending => ordering,
            SortOrder::Descending => ordering.reverse(),
        }
        .then_with(|| name_cmp(a, b))
    });
    // Put the items where the sorted order says, following each cycle of the permutation with swaps
    for start in 0..order.len() {
        let mut at = start;
        while order[at] != usize::MAX {
            let from = order[at];
            order[at] = usize::MAX;
            if from == start {
                break;
            }
            items.swap(at, from);
            at = from;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_not_measured_yet_sort_after_measured_ones_either_way() {
        use super::super::dirsize;
        let root = std::env::temp_dir().join(format!("ski-sortsize-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for name in ["big", "small", "unknown-a", "unknown-b"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        let totals = |bytes| dirsize::Totals { bytes, disk: bytes, files: 1, folders: 0, partial: false };
        dirsize::store(&root.join("big"), totals(9000));
        dirsize::store(&root.join("small"), totals(10));
        let items = || ["unknown-b", "big", "unknown-a", "small"].map(|n| FileItem::from_path(&root.join(n)).unwrap()).to_vec();
        let names = |items: &[FileItem]| items.iter().map(|i| i.name.clone()).collect::<Vec<_>>();
        let opts = |sort_order| ListOptions { show_hidden: false, sort_by: SortBy::Size, sort_order, folders_first: true, type_filter: TypeFilter::All };

        let mut ascending = items();
        sort_items(&mut ascending, &opts(SortOrder::Ascending));
        assert_eq!(names(&ascending), ["small", "big", "unknown-a", "unknown-b"]);
        let mut descending = items();
        sort_items(&mut descending, &opts(SortOrder::Descending));
        assert_eq!(names(&descending), ["big", "small", "unknown-a", "unknown-b"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

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

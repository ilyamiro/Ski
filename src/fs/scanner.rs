use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::Path;

use super::file_item::{FileItem, FileType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortBy {
    Name,
    Size,
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
            TypeFilter::All => "All",
            TypeFilter::Folders => "Folders",
            TypeFilter::Documents => "Documents",
            TypeFilter::Images => "Images",
            TypeFilter::Audio => "Audio",
            TypeFilter::Video => "Video",
            TypeFilter::Archives => "Archives",
            TypeFilter::Code => "Code",
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

fn sort_items(items: &mut [FileItem], opts: &ListOptions) {
    let (sort_by, sort_order) = (opts.sort_by, opts.sort_order);
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
            SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Modified => a.modified.cmp(&b.modified),
            SortBy::Created => a.created.cmp(&b.created),
            SortBy::Accessed => a.accessed.cmp(&b.accessed),
            SortBy::Owner => a.uid.cmp(&b.uid),
            SortBy::Permissions => a.mode.cmp(&b.mode),
        };

        match sort_order {
            SortOrder::Ascending => ordering,
            SortOrder::Descending => ordering.reverse(),
        }
    });
}

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::Path;

use super::file_item::FileItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Size,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

pub fn read_directory(
    path: &Path,
    show_hidden: bool,
    search_query: &str,
    sort_by: SortBy,
    sort_order: SortOrder,
) -> io::Result<Vec<FileItem>> {
    let mut items = Vec::new();

    for entry in fs::read_dir(path)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue, // Skip unreadable entries gracefully
        };

        if let Some(item) = FileItem::from_path(&entry.path()) {
            if !show_hidden && item.is_hidden {
                continue;
            }

            if !search_query.is_empty()
                && !item
                    .name
                    .to_lowercase()
                    .contains(&search_query.to_lowercase())
            {
                continue;
            }

            items.push(item);
        }
    }

    sort_items(&mut items, sort_by, sort_order);

    Ok(items)
}

fn sort_items(items: &mut [FileItem], sort_by: SortBy, sort_order: SortOrder) {
    items.sort_by(|a, b| {
        // Always group directories first
        if a.is_dir != b.is_dir {
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
        };

        match sort_order {
            SortOrder::Ascending => ordering,
            SortOrder::Descending => ordering.reverse(),
        }
    });
}

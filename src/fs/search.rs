//! Recursive filename search running on a background thread, streaming results in batches.
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use eframe::egui;

use super::file_item::FileItem;
use super::glob;
use super::scanner::TypeFilter;
use super::recent;

pub const MAX_RESULTS: usize = 400;
const MAX_DEPTH: usize = 8;
const BATCH: usize = 24;
/// Larger files are not read when searching inside files.
const CONTENT_LIMIT: u64 = 1024 * 1024;

/// Where a search looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SearchScope {
    /// The folder that was open when the search began.
    ThisFolder,
    /// The whole home folder.
    #[default]
    Everywhere,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub match_case: bool,
    /// Also find text files that contain the words, not only files named so.
    pub contents: bool,
}

/// Does the text file at `path` contain `needle`? Binary and large files never do.
fn file_contains(path: &std::path::Path, needle: &str, match_case: bool) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let mut bytes = Vec::new();
    if file.by_ref().take(CONTENT_LIMIT).read_to_end(&mut bytes).is_err() || bytes[..bytes.len().min(4096)].contains(&0) {
        return false;
    }
    let text = String::from_utf8_lossy(&bytes);
    if match_case { text.contains(needle) } else { text.to_lowercase().contains(&needle.to_lowercase()) }
}

pub struct Search {
    generation: Arc<AtomicU64>,
    rx: Option<mpsc::Receiver<Vec<FileItem>>>,
    finished: bool,
}

impl Search {
    pub fn new() -> Self {
        Self { generation: Arc::new(AtomicU64::new(0)), rx: None, finished: true }
    }

    /// Cancel any running search.
    pub fn cancel(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.rx = None;
        self.finished = true;
    }

    /// A search is still walking folders.
    pub fn is_running(&self) -> bool {
        self.rx.is_some() && !self.finished
    }

    pub fn start(&mut self, ctx: &egui::Context, root: PathBuf, query: String, show_hidden: bool, filter: TypeFilter, options: SearchOptions) {
        let my_gen = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let generation = self.generation.clone();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.finished = false;
        let ctx = ctx.clone();

        std::thread::spawn(move || {
            let cancelled = || generation.load(Ordering::Relaxed) != my_gen;
            let matches = |name: &str| glob::matches(&query, name, options.match_case);
            let mut seen: HashSet<PathBuf> = HashSet::new();
            let mut found = 0usize;

            // Stage 1: files the user opened lately answer instantly, before any directory walk
            let recents: Vec<FileItem> = recent::load()
                .into_iter()
                .filter(|p| p.starts_with(&root))
                .filter(|p| p.file_name().is_some_and(|n| matches(&n.to_string_lossy())))
                .filter(|p| show_hidden || !p.strip_prefix(&root).is_ok_and(|r| r.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'))))
                .filter_map(|p| FileItem::from_path(&p).filter(|_| p.exists()))
                .filter(|i| filter.accepts(i))
                .take(MAX_RESULTS)
                .collect();
            if !recents.is_empty() {
                seen.extend(recents.iter().map(|i| i.path.clone()));
                found += recents.len();
                let _ = tx.send(recents);
                ctx.request_repaint();
            }

            // Stage 2: breadth-first walk, most recently modified folders first
            let mut queue = VecDeque::from([(root, 0usize)]);
            let mut batch = Vec::new();

            while let Some((dir, depth)) = queue.pop_front() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                let mut subdirs: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
                for entry in entries.flatten() {
                    if cancelled() {
                        return;
                    }
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !show_hidden && name.starts_with('.') {
                        continue;
                    }
                    let path = entry.path();
                    let is_real_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    if is_real_dir && depth < MAX_DEPTH {
                        let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                        subdirs.push((mtime, path.clone()));
                    }
                    let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
                    let hit = matches(&name)
                        || options.contents && is_file && !seen.contains(&path)
                            && entry.metadata().is_ok_and(|m| m.len() <= CONTENT_LIMIT)
                            && file_contains(&path, &query, options.match_case);
                    if hit && !seen.contains(&path) {
                        if let Some(item) = FileItem::from_path(&path).filter(|i| filter.accepts(i)) {
                            batch.push(item);
                            found += 1;
                        }
                        if batch.len() >= BATCH {
                            let _ = tx.send(std::mem::take(&mut batch));
                            ctx.request_repaint();
                        }
                        if found >= MAX_RESULTS {
                            let _ = tx.send(batch);
                            ctx.request_repaint();
                            return;
                        }
                    }
                }
                subdirs.sort_by_key(|a| std::cmp::Reverse(a.0));
                queue.extend(subdirs.into_iter().map(|(_, p)| (p, depth + 1)));
            }
            let _ = tx.send(batch);
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) -> Vec<FileItem> {
        let mut out = Vec::new();
        if let Some(rx) = &self.rx {
            loop {
                match rx.try_recv() {
                    Ok(mut batch) => out.append(&mut batch),
                    Err(mpsc::TryRecvError::Empty) => break,
                    // The worker is done and dropped its sender
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.finished = true;
                        break;
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contents_match_text_files_only() {
        let dir = std::env::temp_dir().join(format!("ski_search_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), "Remember the Milk").unwrap();
        std::fs::write(dir.join("blob.bin"), b"milk\0\0\0").unwrap();
        assert!(file_contains(&dir.join("note.txt"), "milk", false));
        assert!(!file_contains(&dir.join("note.txt"), "milk", true));
        assert!(file_contains(&dir.join("note.txt"), "Milk", true));
        assert!(!file_contains(&dir.join("blob.bin"), "milk", false));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

//! Recursive filename search running on a background thread, streaming results in batches.
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use eframe::egui;

use super::file_item::FileItem;
use super::recent;

const MAX_RESULTS: usize = 400;
const MAX_DEPTH: usize = 8;
const BATCH: usize = 24;

pub struct Search {
    generation: Arc<AtomicU64>,
    rx: Option<mpsc::Receiver<Vec<FileItem>>>,
}

impl Search {
    pub fn new() -> Self {
        Self { generation: Arc::new(AtomicU64::new(0)), rx: None }
    }

    /// Cancel any running search.
    pub fn cancel(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.rx = None;
    }

    pub fn start(&mut self, ctx: &egui::Context, root: PathBuf, query: String, show_hidden: bool) {
        let my_gen = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let generation = self.generation.clone();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        let needle = query.to_lowercase();

        std::thread::spawn(move || {
            let cancelled = || generation.load(Ordering::Relaxed) != my_gen;
            let matches = |name: &str| name.to_lowercase().contains(&needle);
            let mut seen: HashSet<PathBuf> = HashSet::new();
            let mut found = 0usize;

            // Stage 1: files the user opened lately answer instantly, before any directory walk
            let recents: Vec<FileItem> = recent::load()
                .into_iter()
                .filter(|p| p.starts_with(&root))
                .filter(|p| p.file_name().is_some_and(|n| matches(&n.to_string_lossy())))
                .filter(|p| show_hidden || !p.strip_prefix(&root).is_ok_and(|r| r.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'))))
                .filter_map(|p| FileItem::from_path(&p).filter(|_| p.exists()))
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
                    if matches(&name) && !seen.contains(&path) {
                        if let Some(item) = FileItem::from_path(&path) {
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
                subdirs.sort_by(|a, b| b.0.cmp(&a.0));
                queue.extend(subdirs.into_iter().map(|(_, p)| (p, depth + 1)));
            }
            let _ = tx.send(batch);
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) -> Vec<FileItem> {
        let mut out = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(mut batch) = rx.try_recv() {
                out.append(&mut batch);
            }
        }
        out
    }
}

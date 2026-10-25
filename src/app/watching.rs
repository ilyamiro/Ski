//! Keeps the listings in step with the disk, the way Nautilus does: every folder on show has a
//! monitor, the open tab's and each parked tab's, and the items of a search or Recent view are
//! followed by watching the folders they sit in. `fs::watch` reports what changed; a save or a copy
//! is a burst of those, so they are collected until things go quiet and then applied together:
//! changed files are read again, new ones appear, removed ones go, and a preview that no longer
//! matches its file is drawn again.
use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use super::*;
use crate::fs::scanner::sort_items;
use crate::fs::watch::{self, Event, Watcher};

/// Events are applied once nothing new came for this long...
const QUIET: Duration = Duration::from_millis(150);
/// ...or, under a steady flow (a big copy), this long after the first.
const LONGEST: Duration = Duration::from_secs(1);
/// More changed names than this and the folder is read again instead.
const BURST: usize = 300;
/// A search or Recent view follows at most this many folders; items elsewhere are not followed.
const MIXED_FOLDERS: usize = 100;
/// The folders of a growing list of results are re-registered at most this often.
const MIXED_EVERY: Duration = Duration::from_millis(400);

pub(super) struct Watching {
    watcher: Option<Watcher>,
    /// The folders the open listing follows: its own, the trash folders of drives in the trash
    /// view, or the folders of a search's results.
    active: Vec<PathBuf>,
    /// Changed paths, and whether any change to each one finished writing it.
    pending: BTreeMap<PathBuf, bool>,
    overflow: bool,
    gone: Vec<PathBuf>,
    first: Option<Instant>,
    last: Option<Instant>,
    /// What the folders of a mixed listing were made from, to notice when results arrive.
    mixed_sig: (usize, Option<PathBuf>),
    mixed_at: Instant,
}

impl Watching {
    pub(super) fn new(ctx: &egui::Context) -> Self {
        Self {
            watcher: Watcher::new(ctx),
            active: Vec::new(),
            pending: BTreeMap::new(),
            overflow: false,
            gone: Vec::new(),
            first: None,
            last: None,
            mixed_sig: (0, None),
            mixed_at: Instant::now(),
        }
    }

    /// Watch these folders now: the open listing's and the parked tabs'. This comes before a folder
    /// is read, so a change between the read and the next frame is not lost.
    fn follow(&mut self, active: Vec<PathBuf>, parked: Vec<PathBuf>) {
        if let Some(watcher) = self.watcher.as_mut() {
            let mut all = active.clone();
            for dir in parked {
                if !all.contains(&dir) {
                    all.push(dir);
                }
            }
            watcher.set(&all);
        }
        self.active = active;
    }

    fn clear(&mut self) {
        self.pending.clear();
        self.overflow = false;
        self.gone.clear();
        self.first = None;
        self.last = None;
    }
}

impl SkiApp {
    /// The folders whose changes the open listing follows.
    fn watch_targets(&self) -> Vec<PathBuf> {
        if self.is_mixed_listing() {
            // A search or Recent: the folders the items are in
            let mut dirs: Vec<PathBuf> = Vec::new();
            // Recent also follows the lists it is made from, so another window's or app's use shows up
            if self.showing_recent_list() {
                dirs.extend(recent::sources().iter().filter_map(|f| f.parent().map(Path::to_path_buf)));
                dirs.dedup();
            }
            let mut seen = HashSet::new();
            for parent in self.items.iter().filter_map(|i| i.path.parent()) {
                if dirs.len() >= MIXED_FOLDERS {
                    break;
                }
                if seen.insert(parent) {
                    dirs.push(parent.to_path_buf());
                }
            }
            return dirs;
        }
        let mut dirs = vec![self.current_path.clone()];
        if trash::is_trash_folder(&self.current_path) {
            dirs.extend(trash::drive_files_dirs());
        }
        dirs
    }

    /// The Recent view itself (not Bookmarks, and not a search in it).
    fn showing_recent_list(&self) -> bool {
        self.current_path.as_os_str() == RECENT_PATH && self.search_query.trim().is_empty()
    }

    /// Point the monitors at what is on show: the open listing and every parked tab's folder.
    pub(super) fn resync_watch(&mut self) {
        let active = self.watch_targets();
        let parked = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != self.active_tab)
            .filter_map(|(_, slot)| slot.saved.as_ref()?.watched().map(Path::to_path_buf))
            .collect();
        self.watching.follow(active, parked);
    }

    /// Select the same files again once the listing that is about to be read arrives.
    pub(super) fn keep_selection_through_reload(&mut self) {
        let focus = self.selected_index.and_then(|i| self.items.get(i)).map(|i| i.path.clone());
        self.restore_selection = Some((self.selected_paths(), focus, false));
    }

    /// Once per frame: collect what the monitors report and, when it has gone quiet, apply it.
    pub(super) fn poll_watch(&mut self, ctx: &egui::Context) {
        let w = &mut self.watching;
        let Some(watcher) = w.watcher.as_mut() else { return };
        for event in watcher.events() {
            match event {
                Event::Path { path, settled } => *w.pending.entry(path).or_insert(false) |= settled,
                Event::Overflow => w.overflow = true,
                Event::Gone(dir) => w.gone.push(dir),
            }
            let now = Instant::now();
            w.first.get_or_insert(now);
            w.last = Some(now);
        }
        // Results keep arriving in a search: follow the folders they are in
        if self.is_mixed_listing() && self.watching.mixed_at.elapsed() >= MIXED_EVERY {
            let sig = (self.items.len(), self.items.first().map(|i| i.path.clone()));
            if sig != self.watching.mixed_sig {
                self.watching.mixed_sig = sig;
                self.watching.mixed_at = Instant::now();
                self.resync_watch();
            }
        }
        let (Some(first), Some(last)) = (self.watching.first, self.watching.last) else { return };
        if last.elapsed() < QUIET && first.elapsed() < LONGEST {
            ctx.request_repaint_after(QUIET.saturating_sub(last.elapsed()).max(Duration::from_millis(10)));
            return;
        }
        let (changed, overflow, gone) = (std::mem::take(&mut self.watching.pending), self.watching.overflow, std::mem::take(&mut self.watching.gone));
        self.watching.clear();
        self.apply_disk_changes(changed, overflow, gone);
    }

    fn apply_disk_changes(&mut self, changed: BTreeMap<PathBuf, bool>, overflow: bool, gone: Vec<PathBuf>) {
        self.patch_parked_tabs(&changed, overflow, &gone);
        let inside: Vec<&PathBuf> = changed.keys().filter(|p| watch::is_inside(p, &self.watching.active)).collect();
        self.forget_sizes(inside);
        if self.is_mixed_listing() {
            self.patch_mixed(&changed);
            return;
        }
        if gone.contains(&self.current_path) && !self.current_path.is_dir() {
            // The open folder was deleted, moved or unmounted: go to the nearest one that is left
            let mut up = self.current_path.clone();
            while !up.is_dir() && let Some(parent) = up.parent() {
                up = parent.to_path_buf();
            }
            self.navigate_to(up);
            return;
        }
        let ours = changed.keys().filter(|p| watch::is_inside(p, &self.watching.active)).count();
        // A read that is under way may have missed these, and a flood is cheaper to read than to patch
        if overflow || ours > BURST || self.pending_reload.is_some() || gone.iter().any(|g| self.watching.active.contains(g)) {
            self.keep_selection_through_reload();
            self.reload_items_later();
            return;
        }
        let mut fresh = Vec::new();
        let mut touched = Vec::new();
        for (path, settled) in &changed {
            if !watch::is_inside(path, &self.watching.active) {
                continue;
            }
            touched.push(path.clone());
            if path.symlink_metadata().is_err() {
                continue;
            }
            if let Some(item) = FileItem::from_path(path).filter(|item| self.listed(item)) {
                // A file still being written is read again once it is closed
                if *settled && self.settings.show_thumbnails {
                    self.thumbs.ensure(path);
                }
                fresh.push(item);
            }
        }
        if touched.is_empty() {
            return;
        }
        self.edit_listing(|items| {
            items.retain(|i| !touched.contains(&i.path));
            items.extend(fresh);
        });
        let here = self.current_path.clone();
        self.plugins.directory_changed(&here);
    }

    /// Search results, Recent and Bookmarks: only the items already listed follow the disk. A changed
    /// file is read again, a removed one goes; nothing new is added, since it was not asked for.
    fn patch_mixed(&mut self, changed: &BTreeMap<PathBuf, bool>) {
        // Another window or app used a file: Recent is made again. Ski's own click on a file does not
        // reorder the list under the pointer.
        if self.showing_recent_list() && recent::sources().iter().any(|s| changed.contains_key(s)) {
            if !recent::written_here_lately() {
                self.keep_selection_through_reload();
                self.reload_items();
            }
            return;
        }
        let listed: HashSet<&Path> = self.items.iter().map(|i| i.path.as_path()).collect();
        let mine: Vec<&PathBuf> = changed.keys().filter(|p| listed.contains(p.as_path())).collect();
        if mine.is_empty() {
            return;
        }
        let mut fresh = Vec::new();
        let mut gone = Vec::new();
        for path in mine {
            match path.symlink_metadata() {
                Ok(_) => fresh.extend(FileItem::from_path(path)),
                Err(_) => gone.push(path.clone()),
            }
            if changed[path] && self.settings.show_thumbnails {
                self.thumbs.ensure(path);
            }
        }
        // Recent and Bookmarks have an order of their own; search results are sorted
        let sort = !self.in_recent();
        self.edit_listing_sorted(sort, |items| {
            for item in fresh {
                if let Some(slot) = items.iter_mut().find(|i| i.path == item.path) {
                    *slot = item;
                }
            }
            items.retain(|i| !gone.contains(&i.path));
        });
    }

    /// The tabs that are not on show keep their listing current from the same events, so switching
    /// to one needs no read. One that may have missed something (lost events, a flood) is marked
    /// to be read again when it comes back.
    fn patch_parked_tabs(&mut self, changed: &BTreeMap<PathBuf, bool>, overflow: bool, gone: &[PathBuf]) {
        let (show_hidden, folders_first) = (self.show_hidden, self.folders_first);
        for idx in 0..self.tabs.len() {
            if idx == self.active_tab {
                continue;
            }
            let Some((dir, type_filter, sort_by, sort_order)) =
                self.tabs[idx].saved.as_ref().and_then(|s| Some((s.watched()?.to_path_buf(), s.type_filter, s.sort_by, s.sort_order)))
            else {
                continue;
            };
            if gone.contains(&dir) && !dir.is_dir() {
                // Its folder is gone: it will show the nearest one that is left
                let mut up = dir;
                while !up.is_dir() && let Some(parent) = up.parent() {
                    up = parent.to_path_buf();
                }
                if let Some(state) = self.tabs[idx].saved.as_mut() {
                    state.set_folder(up);
                }
                continue;
            }
            let mine: Vec<&PathBuf> = changed.keys().filter(|p| p.parent() == Some(dir.as_path())).collect();
            if overflow || mine.len() > BURST || gone.contains(&dir) {
                if let Some(state) = self.tabs[idx].saved.as_mut() {
                    state.stale = true;
                }
                continue;
            }
            if mine.is_empty() {
                continue;
            }
            let fresh: Vec<FileItem> = mine
                .iter()
                .filter(|p| p.symlink_metadata().is_ok())
                .filter_map(|p| FileItem::from_path(p))
                .filter(|i| (show_hidden || !i.is_hidden) && type_filter.matches(i) && self.chooser_accepts(i))
                .collect();
            let opts = ListOptions { show_hidden, sort_by, sort_order, folders_first, type_filter };
            if let Some(items) = self.tabs[idx].saved.as_mut().and_then(|s| s.items.as_mut()) {
                items.retain(|i| !mine.contains(&&i.path));
                items.extend(fresh);
                sort_items(items, &opts);
            }
        }
    }
}

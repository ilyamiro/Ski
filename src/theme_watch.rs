//! Watches the files a theme can come from, so a color generator that rewrites them is followed
//! while Ski runs: the user's theme files and Serpantinum's `qs_colors.json`.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Writers truncate and then fill a file, so a change is read once things have been quiet this long.
const SETTLE: Duration = Duration::from_millis(150);

pub struct ThemeWatch {
    _watcher: RecommendedWatcher,
    events: Receiver<()>,
    due: Option<Instant>,
}

impl ThemeWatch {
    pub fn new(ctx: &egui::Context) -> Option<Self> {
        let (tx, events) = channel();
        let repaint = ctx.clone();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            let touched = matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_));
            let ours = event.paths.iter().any(|p| p.extension().is_some_and(|e| e == "json"));
            if touched && ours && tx.send(()).is_ok() {
                repaint.request_repaint();
            }
        })
        .ok()?;
        // The folders, not the files: generators replace a file by renaming a new one over it
        let mut any = false;
        for dir in folders() {
            any |= watcher.watch(&dir, RecursiveMode::NonRecursive).is_ok();
        }
        any.then_some(Self { _watcher: watcher, events, due: None })
    }

    /// True once a watched file has changed and stopped changing; asks for another frame while waiting.
    pub fn changed(&mut self, ctx: &egui::Context) -> bool {
        if self.events.try_iter().count() > 0 {
            self.due = Some(Instant::now() + SETTLE);
        }
        match self.due {
            Some(at) if Instant::now() >= at => {
                self.due = None;
                true
            }
            Some(at) => {
                ctx.request_repaint_after(at - Instant::now());
                false
            }
            None => false,
        }
    }
}

fn folders() -> Vec<PathBuf> {
    let home = dirs::home_dir();
    let state = home.as_ref().map(|h| h.join(".local/state/serpantinum"));
    let config = home.as_ref().map(|h| h.join(".config/serpantinum"));
    let themes = crate::ui::theme::user_themes_dir();
    // The themes folder may not exist before the first theme is saved
    if let Some(dir) = &themes
        && !crate::fs::admin::is_admin()
    {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut out: Vec<PathBuf> = themes.into_iter().chain(state).chain(config).collect();
    out.retain(|d| Path::new(d).is_dir());
    out
}

//! Plugins bundled with Ski. Each one adds a feature through a few small hooks (a badge on a file,
//! a context menu entry, rows in Properties, a line of status) and can be switched off in
//! Settings → Plugins. There is no dynamic loading: a plugin is a type in this folder that is
//! listed in `Plugins::bundled`.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::fs::file_item::FileItem;

pub mod actions;
pub mod checksum;
pub mod git;
pub mod quick_look;

/// How a badge or status is coloured; the interface maps it to the current theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Bad,
}

/// A small letter drawn over a file's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    pub letter: char,
    pub tone: Tone,
}

/// A context menu entry contributed by a plugin. Entries with the same `group` fold into a submenu.
pub struct MenuItem {
    pub id: String,
    pub label: &'static str,
    pub icon: &'static str,
    pub group: Option<&'static str>,
}

/// One line of status a plugin wants shown over the listing.
pub struct Status {
    pub icon: &'static str,
    pub text: String,
}

/// Rows added to the Properties dialog under a heading.
pub struct PropertyGroup {
    pub title: &'static str,
    pub rows: Vec<(&'static str, String)>,
}

pub trait Plugin {
    /// Stable id, the key of the switch in `settings.json`.
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn icon(&self) -> &'static str;
    fn default_enabled(&self) -> bool {
        true
    }

    /// The listing now shows `dir` (also after a reload, so changes on disk are picked up).
    fn directory_changed(&mut self, _dir: &Path) {}
    /// Called every frame while enabled; returns whether something changed and the window should repaint.
    fn poll(&mut self) -> bool {
        false
    }
    /// Work is running in the background, so the window keeps polling until it is done.
    fn busy(&self) -> bool {
        false
    }
    fn badge(&self, _item: &FileItem) -> Option<Badge> {
        None
    }
    fn status(&self) -> Option<Status> {
        None
    }
    fn menu_items(&self, _selection: &[FileItem]) -> Vec<MenuItem> {
        Vec::new()
    }
    /// Runs the menu entry `id` on `paths`, from the folder `dir`.
    fn run(&mut self, _id: &str, _paths: &[PathBuf], _dir: &Path) {}
    /// Extra rows for the Properties dialog of `item`; called every frame while it is open.
    fn properties(&mut self, _item: &FileItem) -> Vec<PropertyGroup> {
        Vec::new()
    }
    /// Messages for the user (a finished action, a failure), taken once.
    fn messages(&mut self) -> Vec<String> {
        Vec::new()
    }
}

pub struct Plugins {
    list: Vec<Box<dyn Plugin>>,
    /// The switches from settings: plugins missing here keep their default.
    enabled: BTreeMap<String, bool>,
}

impl Plugins {
    pub fn bundled(enabled: &BTreeMap<String, bool>) -> Self {
        Self {
            list: vec![
                Box::new(git::Git::default()),
                Box::new(checksum::Checksum::default()),
                Box::new(actions::Actions::load()),
                Box::new(quick_look::QuickLook),
            ],
            enabled: enabled.clone(),
        }
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.get(id).copied().unwrap_or_else(|| self.list.iter().find(|p| p.id() == id).is_some_and(|p| p.default_enabled()))
    }

    pub fn set_enabled(&mut self, id: &str, on: bool) {
        self.enabled.insert(id.to_owned(), on);
    }

    pub fn enabled_map(&self) -> &BTreeMap<String, bool> {
        &self.enabled
    }

    /// Every bundled plugin with its switch, for the settings page.
    pub fn catalog(&self) -> Vec<(&'static str, &'static str, &'static str, &'static str, bool)> {
        self.list.iter().map(|p| (p.id(), p.icon(), p.name(), p.description(), self.is_enabled(p.id()))).collect()
    }

    fn active(&self) -> impl Iterator<Item = &dyn Plugin> {
        self.list.iter().filter(|p| self.is_enabled(p.id())).map(|p| p.as_ref())
    }

    fn active_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Plugin>> {
        let enabled = &self.enabled;
        self.list.iter_mut().filter(move |p| enabled.get(p.id()).copied().unwrap_or_else(|| p.default_enabled()))
    }

    pub fn directory_changed(&mut self, dir: &Path) {
        for plugin in self.active_mut() {
            plugin.directory_changed(dir);
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for plugin in self.active_mut() {
            changed |= plugin.poll();
        }
        changed
    }

    pub fn busy(&self) -> bool {
        self.active().any(|p| p.busy())
    }

    pub fn badge(&self, item: &FileItem) -> Option<Badge> {
        self.active().find_map(|p| p.badge(item))
    }

    pub fn statuses(&self) -> Vec<Status> {
        self.active().filter_map(|p| p.status()).collect()
    }

    /// Menu entries for the selection, with the id of the plugin each one belongs to.
    pub fn menu_items(&self, selection: &[FileItem]) -> Vec<(&'static str, MenuItem)> {
        self.active().flat_map(|p| p.menu_items(selection).into_iter().map(|item| (p.id(), item))).collect()
    }

    pub fn run(&mut self, plugin: &str, id: &str, paths: &[PathBuf], dir: &Path) {
        if let Some(p) = self.active_mut().find(|p| p.id() == plugin) {
            p.run(id, paths, dir);
        }
    }

    pub fn properties(&mut self, item: &FileItem) -> Vec<PropertyGroup> {
        self.active_mut().flat_map(|p| p.properties(item)).collect()
    }

    pub fn messages(&mut self) -> Vec<String> {
        self.active_mut().flat_map(|p| p.messages()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_override_defaults() {
        let mut plugins = Plugins::bundled(&BTreeMap::new());
        assert!(plugins.is_enabled("git"));
        assert!(!plugins.is_enabled("no-such-plugin"));
        plugins.set_enabled("git", false);
        assert!(!plugins.is_enabled("git"));
        assert_eq!(plugins.enabled_map().get("git"), Some(&false));
    }

    #[test]
    fn plugin_ids_are_unique() {
        let plugins = Plugins::bundled(&BTreeMap::new());
        let mut ids: Vec<_> = plugins.catalog().iter().map(|c| c.0).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total);
    }
}

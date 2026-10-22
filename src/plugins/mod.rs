//! Plugins bundled with Ski. Each one adds a feature through a few small hooks (a badge on a file,
//! a context menu entry, rows in Properties, a line of status) and can be switched off in
//! Settings → Plugins. There is no dynamic loading: a plugin is a type in this folder that is
//! listed in `Plugins::bundled`.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::fs::engine::{Outcome, Worker};
use crate::fs::file_item::FileItem;
use crate::ui::theme::SerpantinumTheme;

pub mod actions;
pub mod checksum;
pub mod git;
pub mod quick_look;
pub mod tags;

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

/// One mark on a file: a letter badge, or a small dot in any colour (a tag has many).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Letter(Badge),
    Dot([u8; 3]),
}

/// A context menu entry contributed by a plugin. Entries with the same `group` fold into a submenu.
pub struct MenuItem {
    pub id: String,
    pub label: &'static str,
    pub icon: &'static str,
    pub group: Option<&'static str>,
    /// When set, the app asks this question first and runs the entry only after a yes.
    pub confirm: Option<&'static str>,
}

impl MenuItem {
    pub fn new(id: impl Into<String>, label: &'static str, icon: &'static str) -> Self {
        Self { id: id.into(), label, icon, group: None, confirm: None }
    }

    pub fn group(mut self, group: &'static str) -> Self {
        self.group = Some(group);
        self
    }

    pub fn confirm(mut self, question: &'static str) -> Self {
        self.confirm = Some(question);
        self
    }
}

/// Long work a plugin hands to the app, which runs it as a file operation: it shows progress in the
/// operations list and can be paused and cancelled there.
pub struct Job {
    pub title: String,
    pub work: Box<dyn FnOnce(&mut Worker) -> Outcome + Send>,
}

/// What the app does after a plugin ran an entry or handled a click.
#[derive(Default)]
pub enum Reaction {
    #[default]
    None,
    /// Put this text in the search field and search.
    Search(String),
    /// Run this as a file operation.
    Job(Job),
}

/// A row a plugin adds to the sidebar.
pub struct SidebarEntry {
    pub id: String,
    pub label: String,
    pub icon: &'static str,
}

/// A change Ski made to files, reported to plugins once it is done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOpEvent {
    /// `(from, to)` for each renamed or moved path.
    Moved(Vec<(PathBuf, PathBuf)>),
    Removed(Vec<PathBuf>),
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
    /// Every mark for `item`; by default the single badge.
    fn marks(&self, item: &FileItem) -> Vec<Mark> {
        self.badge(item).map(Mark::Letter).into_iter().collect()
    }
    fn status(&self) -> Option<Status> {
        None
    }
    fn menu_items(&self, _selection: &[FileItem]) -> Vec<MenuItem> {
        Vec::new()
    }
    /// Runs the menu entry `id` on `paths`, from the folder `dir`.
    fn run(&mut self, _id: &str, _paths: &[PathBuf], _dir: &Path) -> Reaction {
        Reaction::None
    }
    /// The title of the dialog this plugin wants open right now, if any. The app draws the dialog
    /// and calls `window` every frame until the plugin stops asking.
    fn window_title(&self) -> Option<&'static str> {
        None
    }
    /// The user closed the dialog (button, Escape, a click outside).
    fn close_window(&mut self) {}
    /// Draws the dialog's content; returns false when the plugin is done with it.
    fn window(&mut self, _ui: &mut eframe::egui::Ui, _theme: &SerpantinumTheme) -> bool {
        false
    }
    /// Rows of the plugin's own section in the sidebar.
    fn sidebar(&self) -> Vec<SidebarEntry> {
        Vec::new()
    }
    fn sidebar_click(&mut self, _id: &str) -> Reaction {
        Reaction::None
    }
    /// Files were renamed, moved or removed by Ski.
    fn file_op_done(&mut self, _op: &FileOpEvent) {}
    /// A search query the plugin understands (`tag:work`): the paths it matches, or None when the
    /// query is not meant for it.
    fn search_paths(&self, _query: &str) -> Option<Vec<PathBuf>> {
        None
    }
    /// Extra rows for the Properties dialog of `item`; called every frame while it is open.
    fn properties(&mut self, _item: &FileItem) -> Vec<PropertyGroup> {
        Vec::new()
    }
    /// The plugin's own controls on its Settings → Plugins page.
    fn settings(&mut self, _ui: &mut eframe::egui::Ui, _theme: &SerpantinumTheme) {}
    /// Messages for the user (a finished action, a failure), taken once.
    fn messages(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// Menu labels are `&'static str`; a text built at run time (a tag name) is leaked once and reused.
pub fn intern(text: &str) -> &'static str {
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<std::collections::HashSet<&'static str>>> = OnceLock::new();
    let mut seen = SEEN.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&known) = seen.get(text) {
        return known;
    }
    let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
    seen.insert(leaked);
    leaked
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
                Box::new(tags::Tags::load()),
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

    pub fn marks(&self, item: &FileItem) -> Vec<Mark> {
        self.active().flat_map(|p| p.marks(item)).collect()
    }

    pub fn statuses(&self) -> Vec<Status> {
        self.active().filter_map(|p| p.status()).collect()
    }

    /// Menu entries for the selection, with the id of the plugin each one belongs to.
    pub fn menu_items(&self, selection: &[FileItem]) -> Vec<(&'static str, MenuItem)> {
        self.active().flat_map(|p| p.menu_items(selection).into_iter().map(|item| (p.id(), item))).collect()
    }

    pub fn run(&mut self, plugin: &str, id: &str, paths: &[PathBuf], dir: &Path) -> Reaction {
        match self.active_mut().find(|p| p.id() == plugin) {
            Some(p) => p.run(id, paths, dir),
            None => Reaction::None,
        }
    }

    /// The plugin that wants a dialog open, with its title.
    pub fn wanted_window(&self) -> Option<(&'static str, &'static str)> {
        self.active().find_map(|p| p.window_title().map(|t| (p.id(), t)))
    }

    /// Draws the dialog of `plugin`; false when it closed.
    pub fn window(&mut self, plugin: &str, ui: &mut eframe::egui::Ui, theme: &SerpantinumTheme) -> bool {
        self.active_mut().find(|p| p.id() == plugin).is_some_and(|p| p.window(ui, theme))
    }

    pub fn settings(&mut self, plugin: &str, ui: &mut eframe::egui::Ui, theme: &SerpantinumTheme) {
        if let Some(p) = self.active_mut().find(|p| p.id() == plugin) {
            p.settings(ui, theme);
        }
    }

    pub fn close_window(&mut self, plugin: &str) {
        if let Some(p) = self.active_mut().find(|p| p.id() == plugin) {
            p.close_window();
        }
    }

    pub fn sidebar(&self) -> Vec<(&'static str, SidebarEntry)> {
        self.active().flat_map(|p| p.sidebar().into_iter().map(|e| (p.id(), e))).collect()
    }

    pub fn sidebar_click(&mut self, plugin: &str, id: &str) -> Reaction {
        match self.active_mut().find(|p| p.id() == plugin) {
            Some(p) => p.sidebar_click(id),
            None => Reaction::None,
        }
    }

    pub fn file_op_done(&mut self, op: &FileOpEvent) {
        for plugin in self.active_mut() {
            plugin.file_op_done(op);
        }
    }

    /// The first plugin that understands `query` answers it.
    pub fn search_paths(&self, query: &str) -> Option<Vec<PathBuf>> {
        self.active().find_map(|p| p.search_paths(query))
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

    /// A plugin that records what it is told, to test the routing.
    struct Fake {
        id: &'static str,
        on: bool,
        events: std::rc::Rc<std::cell::RefCell<Vec<FileOpEvent>>>,
    }

    impl Plugin for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn description(&self) -> &'static str {
            ""
        }
        fn icon(&self) -> &'static str {
            ""
        }
        fn default_enabled(&self) -> bool {
            self.on
        }
        fn marks(&self, _item: &FileItem) -> Vec<Mark> {
            vec![Mark::Dot([1, 2, 3]), Mark::Dot([4, 5, 6])]
        }
        fn menu_items(&self, _selection: &[FileItem]) -> Vec<MenuItem> {
            vec![MenuItem::new("go", "Go", "").confirm("Sure?")]
        }
        fn run(&mut self, id: &str, _paths: &[PathBuf], dir: &Path) -> Reaction {
            if id == "go" { Reaction::Search(dir.display().to_string()) } else { Reaction::None }
        }
        fn file_op_done(&mut self, op: &FileOpEvent) {
            self.events.borrow_mut().push(op.clone());
        }
    }

    fn fakes(on: bool) -> (Plugins, std::rc::Rc<std::cell::RefCell<Vec<FileOpEvent>>>) {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let plugins = Plugins { list: vec![Box::new(Fake { id: "fake", on, events: events.clone() })], enabled: BTreeMap::new() };
        (plugins, events)
    }

    fn item() -> FileItem {
        FileItem::from_path(Path::new("/")).expect("root")
    }

    #[test]
    fn marks_come_back_in_order() {
        let (plugins, _) = fakes(true);
        assert_eq!(plugins.marks(&item()), vec![Mark::Dot([1, 2, 3]), Mark::Dot([4, 5, 6])]);
    }

    #[test]
    fn a_disabled_plugin_contributes_nothing() {
        let (mut plugins, events) = fakes(false);
        assert!(plugins.marks(&item()).is_empty());
        assert!(plugins.menu_items(&[item()]).is_empty());
        plugins.file_op_done(&FileOpEvent::Removed(vec![PathBuf::from("/x")]));
        assert!(events.borrow().is_empty());
        assert!(matches!(plugins.run("fake", "go", &[], Path::new("/")), Reaction::None));
    }

    #[test]
    fn menu_items_carry_the_confirmation_and_run_reports_a_reaction() {
        let (mut plugins, _) = fakes(true);
        let entries = plugins.menu_items(&[item()]);
        assert_eq!(entries[0].0, "fake");
        assert_eq!(entries[0].1.confirm, Some("Sure?"));
        assert!(matches!(plugins.run("fake", "go", &[], Path::new("/tmp")), Reaction::Search(q) if q == "/tmp"));
    }

    #[test]
    fn file_operations_reach_enabled_plugins() {
        let (mut plugins, events) = fakes(true);
        let op = FileOpEvent::Moved(vec![(PathBuf::from("/a"), PathBuf::from("/b"))]);
        plugins.file_op_done(&op);
        assert_eq!(*events.borrow(), vec![op]);
    }

    #[test]
    fn interning_leaks_each_text_once() {
        let a = intern("same text");
        let b = intern(&String::from("same text"));
        assert!(std::ptr::eq(a, b));
    }

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

//! Tags: coloured labels on files. The names are stored in the extended attribute
//! `user.xdg.tags` (comma separated), the one Dolphin reads, so tags show in both managers.
//! Colours are Ski's own (`~/.config/ski/tags.json`). Because there is no index, Ski remembers
//! the paths it has seen tagged (`~/.local/share/ski/tag-paths.json`) to answer `tag:work`.
use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use eframe::egui;

use super::{FileOpEvent, Mark, MenuItem, Plugin, PropertyGroup, Reaction, SidebarEntry, intern};
use crate::fs::file_item::FileItem;
use crate::fs::xattr;
use crate::i18n::{tr, trf, trn};
use crate::ui::icons;
use crate::ui::material;
use crate::ui::theme::SerpantinumTheme;

pub const ATTR: &str = "user.xdg.tags";
const MAX_NAME: usize = 64;
/// A folder scan for tags stops after this many entries.
const SCAN_LIMIT: usize = 200_000;

/// Colours offered in Settings; a tag without a choice gets one of these from a hash of its name.
pub const PALETTE: [[u8; 3]; 8] = [
    [0xe5, 0x57, 0x57], // red
    [0xf0, 0x92, 0x4a], // orange
    [0xe9, 0xc4, 0x4c], // yellow
    [0x5c, 0xb8, 0x5c], // green
    [0x3f, 0xb8, 0xaf], // teal
    [0x4a, 0x90, 0xe2], // blue
    [0x9b, 0x6b, 0xdf], // purple
    [0xe0, 0x6c, 0xb0], // pink
];

/// The names in an attribute value: comma separated, trimmed, without empty or repeated names.
pub fn parse(bytes: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in String::from_utf8_lossy(bytes).split(',') {
        let name = name.trim();
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
    }
    out
}

pub fn format(tags: &[String]) -> String {
    tags.join(",")
}

/// Tag names match without regard to case, in any script (the registry folds the same way).
fn same_tag(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

/// A name that fits the attribute: no comma (it separates names), no control characters, not empty.
pub fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    let ok = !name.is_empty() && name.chars().count() <= MAX_NAME && !name.chars().any(|c| c == ',' || c.is_control());
    ok.then(|| name.to_owned())
}

fn hash(name: &str) -> usize {
    name.to_lowercase().bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)) as usize
}

/// What reading or writing the tags of a file said, in words for the user.
fn describe(err: &std::io::Error, name: &str) -> String {
    if xattr::is_unsupported(err) {
        return tr("This disk can't store tags").to_owned();
    }
    match err.raw_os_error() {
        Some(libc::EPERM) => trf("Tags can't be set on {} (links and special files can't carry them)", &[&name]),
        Some(libc::EACCES) | Some(libc::EROFS) => trf("{} is read-only, so its tags can't change", &[&name]),
        _ => trf("Tags of {} failed: {}", &[&name, &err]),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

pub fn read_tags(path: &Path) -> std::io::Result<Vec<String>> {
    Ok(xattr::get(path, ATTR)?.map(|v| parse(&v)).unwrap_or_default())
}

pub fn write_tags(path: &Path, tags: &[String]) -> std::io::Result<()> {
    if tags.is_empty() { xattr::remove(path, ATTR) } else { xattr::set(path, ATTR, format(tags).as_bytes()) }
}

/// The paths Ski has seen tagged, with their tags.
#[derive(Default, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Registry(BTreeMap<PathBuf, Vec<String>>);

impl Registry {
    fn file() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("ski").join("tag-paths.json"))
    }

    fn load() -> Self {
        let mut registry: Registry = Self::file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        registry.validate(|p| read_tags(p).ok());
        registry
    }

    fn save(&self) {
        // As administrator the data folder belongs to someone else's home, as for the settings
        if crate::fs::admin::is_admin() {
            return;
        }
        let Some(file) = Self::file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(file, json);
        }
    }

    /// Drops paths that are gone and takes the tags from disk (`read` gives None when unreadable).
    pub fn validate(&mut self, read: impl Fn(&Path) -> Option<Vec<String>>) {
        self.0.retain(|path, _| path.symlink_metadata().is_ok());
        for (path, tags) in self.0.iter_mut() {
            if let Some(fresh) = read(path) {
                *tags = fresh;
            }
        }
        self.0.retain(|_, tags| !tags.is_empty());
    }

    /// Returns whether anything changed.
    pub fn record(&mut self, path: &Path, tags: &[String]) -> bool {
        if tags.is_empty() {
            return self.0.remove(path).is_some();
        }
        self.0.insert(path.to_path_buf(), tags.to_vec()).as_deref() != Some(tags)
    }

    /// `from` and everything inside it now lives at `to`.
    pub fn rename(&mut self, from: &Path, to: &Path) -> bool {
        let moved: Vec<PathBuf> = self.0.keys().filter(|p| p.starts_with(from)).cloned().collect();
        for old in &moved {
            if let Some(tags) = self.0.remove(old) {
                let new = to.join(old.strip_prefix(from).unwrap_or(Path::new("")));
                self.0.insert(new, tags);
            }
        }
        !moved.is_empty()
    }

    pub fn remove_tree(&mut self, path: &Path) -> bool {
        let before = self.0.len();
        self.0.retain(|p, _| !p.starts_with(path));
        self.0.len() != before
    }

    /// Every tag name in use, with the number of files that have it, ordered by name.
    pub fn names(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for tags in self.0.values() {
            for tag in tags {
                counts.entry(tag.to_lowercase()).or_insert_with(|| (tag.clone(), 0)).1 += 1;
            }
        }
        counts.into_values().collect()
    }

    pub fn with_tag(&self, name: &str) -> Vec<PathBuf> {
        let name = name.to_lowercase();
        self.0.iter().filter(|(_, tags)| tags.iter().any(|t| t.to_lowercase() == name)).map(|(p, _)| p.clone()).collect()
    }
}

#[derive(Clone)]
struct Entry {
    /// `(inode, ctime seconds, ctime nanoseconds)`: changing an attribute changes ctime, not mtime.
    key: (u64, i64, i64),
    tags: Vec<String>,
}

fn key_of(path: &Path) -> Option<(std::fs::Metadata, (u64, i64, i64))> {
    let meta = path.symlink_metadata().ok()?;
    let key = (meta.ino(), meta.ctime(), meta.ctime_nsec());
    Some((meta, key))
}

/// The tags of everything in `dir`, reusing `previous` where the file's ctime did not change.
fn read_folder(dir: &Path, previous: &HashMap<PathBuf, Entry>) -> HashMap<PathBuf, Entry> {
    let mut out = HashMap::new();
    let Ok(read) = std::fs::read_dir(dir) else { return out };
    for entry in read.flatten() {
        let path = entry.path();
        let Some((meta, key)) = key_of(&path) else { continue };
        if meta.file_type().is_symlink() {
            continue;
        }
        let tags = match previous.get(&path) {
            Some(old) if old.key == key => old.tags.clone(),
            _ => read_tags(&path).unwrap_or_default(),
        };
        out.insert(path, Entry { key, tags });
    }
    out
}

/// Walks `dir` and returns every file with tags.
fn scan_tree(dir: &Path) -> Vec<Tagged> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut seen = 0usize;
    while let Some(folder) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&folder) else { continue };
        for entry in read.flatten() {
            seen += 1;
            if seen > SCAN_LIMIT {
                return found;
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            if let Ok(tags) = read_tags(&path)
                && !tags.is_empty()
            {
                found.push((path.clone(), tags));
            }
            if kind.is_dir() {
                stack.push(path);
            }
        }
    }
    found
}

/// A folder read on a worker: the generation that asked, the folder and what it holds.
type FolderRead = (u64, PathBuf, HashMap<PathBuf, Entry>);
type Tagged = (PathBuf, Vec<String>);

struct Prompt {
    paths: Vec<PathBuf>,
    name: String,
    first_frame: bool,
}

pub struct Tags {
    registry: Registry,
    colors: BTreeMap<String, [u8; 3]>,
    cache: HashMap<PathBuf, Entry>,
    dir: Option<PathBuf>,
    generation: u64,
    folder_rx: Option<mpsc::Receiver<FolderRead>>,
    scan_rx: Option<mpsc::Receiver<Vec<Tagged>>>,
    prompt: Option<Prompt>,
    messages: Vec<String>,
}

fn colors_file() -> Option<PathBuf> {
    crate::fs::admin::config_override().or_else(dirs::config_dir).map(|d| d.join("ski").join("tags.json"))
}

impl Tags {
    pub fn load() -> Self {
        let colors = colors_file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self::with(Registry::load(), colors)
    }

    fn with(registry: Registry, colors: BTreeMap<String, [u8; 3]>) -> Self {
        Self { registry, colors, cache: HashMap::new(), dir: None, generation: 0, folder_rx: None, scan_rx: None, prompt: None, messages: Vec::new() }
    }

    pub fn color(&self, name: &str) -> [u8; 3] {
        self.colors.get(name).or_else(|| self.colors.get(&name.to_lowercase())).copied().unwrap_or(PALETTE[hash(name) % PALETTE.len()])
    }

    fn save_colors(&self) {
        let Some(file) = colors_file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.colors) {
            let _ = std::fs::write(file, json);
        }
    }

    /// Every tag the user can pick: those in use and those with a colour of their own.
    fn known(&self) -> Vec<String> {
        let mut names: Vec<String> = self.registry.names().into_iter().map(|(n, _)| n).collect();
        for name in self.colors.keys() {
            if !names.iter().any(|n| same_tag(n, name)) {
                names.push(name.clone());
            }
        }
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    fn tags_of(&self, path: &Path) -> Vec<String> {
        match self.cache.get(path) {
            Some(entry) => entry.tags.clone(),
            None => read_tags(path).unwrap_or_default(),
        }
    }

    fn remember(&mut self, path: &Path, tags: Vec<String>) {
        if let Some((_, key)) = key_of(path) {
            self.cache.insert(path.to_path_buf(), Entry { key, tags: tags.clone() });
        }
        if self.registry.record(path, &tags) {
            self.registry.save();
        }
    }

    /// Changes the tags of every path with `change`; the first failure is the one reported.
    fn edit(&mut self, paths: &[PathBuf], change: impl Fn(&mut Vec<String>)) {
        let mut touched = 0;
        for path in paths {
            let mut tags = match read_tags(path) {
                Ok(tags) => tags,
                Err(err) => {
                    self.messages.push(describe(&err, &file_name(path)));
                    return;
                }
            };
            let before = tags.clone();
            change(&mut tags);
            if tags == before {
                continue;
            }
            if let Err(err) = write_tags(path, &tags) {
                self.messages.push(describe(&err, &file_name(path)));
                return;
            }
            self.remember(path, tags);
            touched += 1;
        }
        if touched == 0 && paths.len() > 1 {
            self.messages.push(tr("Nothing changed").to_owned());
        }
    }

    fn start_prompt(&mut self, paths: &[PathBuf]) {
        self.prompt = Some(Prompt { paths: paths.to_vec(), name: String::new(), first_frame: true });
    }
}

impl Plugin for Tags {
    fn id(&self) -> &'static str {
        "tags"
    }

    fn name(&self) -> &'static str {
        tr("Tags")
    }

    fn description(&self) -> &'static str {
        tr("Coloured labels on files, stored in the file system so other file managers see them too")
    }

    fn icon(&self) -> &'static str {
        icons::TAG
    }

    fn directory_changed(&mut self, dir: &Path) {
        if self.dir.as_deref() != Some(dir) {
            self.cache.retain(|p, _| p.parent() != self.dir.as_deref());
        }
        self.dir = Some(dir.to_path_buf());
        self.generation += 1;
        let (tx, rx) = mpsc::channel();
        self.folder_rx = Some(rx);
        let previous: HashMap<PathBuf, Entry> = self.cache.iter().filter(|(p, _)| p.parent() == Some(dir)).map(|(p, e)| (p.clone(), e.clone())).collect();
        let (dir, generation) = (dir.to_path_buf(), self.generation);
        std::thread::spawn(move || {
            let read = read_folder(&dir, &previous);
            let _ = tx.send((generation, dir, read));
        });
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.folder_rx {
            match rx.try_recv() {
                Ok((generation, dir, read)) if generation == self.generation => {
                    self.folder_rx = None;
                    self.cache.retain(|p, _| p.parent() != Some(dir.as_path()));
                    let mut dirty = false;
                    for (path, entry) in &read {
                        dirty |= self.registry.record(path, &entry.tags);
                    }
                    // Remembered files of this folder that are gone
                    let gone: Vec<PathBuf> = self.registry.0.keys().filter(|p| p.parent() == Some(dir.as_path()) && !read.contains_key(*p)).cloned().collect();
                    for path in gone {
                        dirty |= self.registry.record(&path, &[]);
                    }
                    if dirty {
                        self.registry.save();
                    }
                    self.cache.extend(read);
                    changed = true;
                }
                Ok(_) | Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.folder_rx = None,
            }
        }
        if let Some(rx) = &self.scan_rx {
            match rx.try_recv() {
                Ok(found) => {
                    self.scan_rx = None;
                    let count = found.len();
                    for (path, tags) in found {
                        self.registry.record(&path, &tags);
                    }
                    self.registry.save();
                    self.messages.push(if count == 0 { tr("No tagged files found here").to_owned() } else { trn(count, "Found {} tagged file", "Found {} tagged files") });
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.scan_rx = None,
            }
        }
        changed
    }

    fn busy(&self) -> bool {
        self.folder_rx.is_some() || self.scan_rx.is_some()
    }

    fn marks(&self, item: &FileItem) -> Vec<Mark> {
        match self.cache.get(&item.path) {
            Some(entry) => entry.tags.iter().map(|t| Mark::Dot(self.color(t))).collect(),
            None => Vec::new(),
        }
    }

    fn menu_items(&self, selection: &[FileItem]) -> Vec<MenuItem> {
        // Links cannot carry attributes
        if selection.is_empty() || selection.iter().any(|i| i.is_symlink) {
            return Vec::new();
        }
        let per_file: Vec<Vec<String>> = selection.iter().map(|i| self.tags_of(&i.path)).collect();
        let group = tr("Tags");
        let mut items = Vec::new();
        for name in self.known() {
            let all = per_file.iter().all(|tags| tags.iter().any(|t| same_tag(t, &name)));
            items.push(MenuItem::new(format!("toggle:{name}"), intern(&name), if all { icons::CHECK } else { icons::TAG }).group(group));
        }
        items.push(MenuItem::new("new", tr("New tag…"), icons::ADD).group(group));
        if per_file.iter().any(|tags| !tags.is_empty()) {
            items.push(MenuItem::new("clear", tr("Remove all tags"), icons::CLOSE).group(group));
        }
        items
    }

    fn run(&mut self, id: &str, paths: &[PathBuf], _dir: &Path) -> Reaction {
        if let Some(name) = id.strip_prefix("toggle:") {
            let all = paths.iter().all(|p| self.tags_of(p).iter().any(|t| same_tag(t, name)));
            let name = name.to_owned();
            if all {
                self.edit(paths, |tags| tags.retain(|t| !same_tag(t, &name)));
            } else {
                self.edit(paths, |tags| {
                    if !tags.iter().any(|t| same_tag(t, &name)) {
                        tags.push(name.clone());
                    }
                });
            }
        } else if id == "new" {
            self.start_prompt(paths);
        } else if id == "clear" {
            self.edit(paths, |tags| tags.clear());
        }
        Reaction::None
    }

    fn window_title(&self) -> Option<&'static str> {
        self.prompt.as_ref().map(|_| tr("New tag"))
    }

    fn close_window(&mut self) {
        self.prompt = None;
    }

    fn window(&mut self, ui: &mut egui::Ui, theme: &SerpantinumTheme) -> bool {
        let Some(prompt) = self.prompt.as_mut() else { return false };
        let mut add = false;
        let valid = clean_name(&prompt.name).is_some();
        material::input_frame(theme).show(ui, |ui| {
            let out = egui::TextEdit::singleline(&mut prompt.name)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .hint_text(tr("Tag name"))
                .font(egui::FontId::proportional(material::FONT_BODY));
            let response = ui.add(out);
            if std::mem::take(&mut prompt.first_frame) {
                response.request_focus();
            }
            add |= response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        });
        if !valid && !prompt.name.trim().is_empty() {
            let problem = if prompt.name.trim().chars().count() > MAX_NAME { trf("Names can't be longer than {} characters", &[&MAX_NAME]) } else { tr("Names can't contain commas").to_owned() };
            ui.label(egui::RichText::new(problem).size(material::FONT_CAPTION).color(theme.red));
        }
        ui.add_space(4.0);
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 36.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            add |= material::button(ui, "tag_add", Some(icons::CHECK), tr("Add tag"), theme, material::ButtonKind::Filled, valid).clicked();
        });
        if add && let Some(name) = clean_name(&prompt.name) {
            let paths = std::mem::take(&mut prompt.paths);
            self.prompt = None;
            self.edit(&paths, |tags| {
                if !tags.iter().any(|t| same_tag(t, &name)) {
                    tags.push(name.clone());
                }
            });
            return false;
        }
        true
    }

    fn sidebar(&self) -> Vec<SidebarEntry> {
        let mut rows: Vec<SidebarEntry> = self
            .registry
            .names()
            .into_iter()
            .map(|(name, _)| SidebarEntry { id: format!("tag:{name}"), label: name, icon: icons::TAG })
            .collect();
        rows.push(SidebarEntry { id: "scan".to_owned(), label: tr("Find tags in this folder").to_owned(), icon: icons::SEARCH });
        rows
    }

    fn sidebar_click(&mut self, id: &str) -> Reaction {
        if id == "scan" {
            if let Some(dir) = self.dir.clone().filter(|_| self.scan_rx.is_none()) {
                let (tx, rx) = mpsc::channel();
                self.scan_rx = Some(rx);
                std::thread::spawn(move || {
                    let _ = tx.send(scan_tree(&dir));
                });
            }
            return Reaction::None;
        }
        Reaction::Search(id.to_owned())
    }

    fn file_op_done(&mut self, op: &FileOpEvent) {
        let mut dirty = false;
        match op {
            FileOpEvent::Moved(pairs) => {
                for (from, to) in pairs {
                    dirty |= self.registry.rename(from, to);
                    // A moved folder takes everything cached below it along
                    let moved: Vec<PathBuf> = self.cache.keys().filter(|p| p.starts_with(from)).cloned().collect();
                    for old in moved {
                        if let Some(entry) = self.cache.remove(&old) {
                            let new = to.join(old.strip_prefix(from).unwrap_or(Path::new("")));
                            self.cache.insert(new, entry);
                        }
                    }
                }
            }
            FileOpEvent::Removed(paths) => {
                for path in paths {
                    dirty |= self.registry.remove_tree(path);
                    self.cache.retain(|p, _| !p.starts_with(path));
                }
            }
        }
        if dirty {
            self.registry.save();
        }
    }

    fn search_paths(&self, query: &str) -> Option<Vec<PathBuf>> {
        let name = query.trim().strip_prefix("tag:").or_else(|| query.trim().strip_prefix("Tag:"))?.trim();
        // Trust the attribute over the remembered list; drop what is gone or no longer tagged
        Some(self.registry.with_tag(name).into_iter().filter(|p| read_tags(p).map_or_else(|e| e.kind() != std::io::ErrorKind::NotFound, |tags| tags.iter().any(|t| same_tag(t, name)))).collect())
    }

    fn properties(&mut self, item: &FileItem) -> Vec<PropertyGroup> {
        if item.is_symlink {
            return Vec::new();
        }
        let tags = self.tags_of(&item.path);
        if tags.is_empty() {
            return Vec::new();
        }
        vec![PropertyGroup { title: tr("Tags"), rows: vec![(tr("Tags"), tags.join(", "))] }]
    }

    fn settings(&mut self, ui: &mut egui::Ui, theme: &SerpantinumTheme) {
        material::hint_label_inset(ui, tr("Pick a colour for each tag. Other file managers see the names only."), theme, 14);
        let names = self.known();
        if names.is_empty() {
            material::hint_label_inset(ui, tr("No tags yet. Right-click a file and choose Tags."), theme, 14);
        }
        let mut chosen: Option<(String, [u8; 3])> = None;
        for name in &names {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(egui::RichText::new(name).size(material::FONT_BODY).color(theme.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(14.0);
                    let current = self.color(name);
                    for rgb in PALETTE.iter().rev() {
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                        let color = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                        ui.painter().circle_filled(rect.center(), 7.5, color);
                        if *rgb == current {
                            ui.painter().circle_stroke(rect.center(), 9.5, egui::Stroke::new(2.0, theme.text));
                        }
                        if response.clicked() {
                            chosen = Some((name.clone(), *rgb));
                        }
                    }
                });
            });
        }
        if let Some((name, rgb)) = chosen {
            self.colors.insert(name, rgb);
            self.save_colors();
        }
        ui.add_space(4.0);
    }

    fn messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::xattr::{self, tests::scratch_file};

    #[test]
    fn case_folding_covers_non_ascii_names() {
        assert!(same_tag("Übung", "übung"));
        assert!(same_tag("Работа", "работа"));
        assert!(!same_tag("a", "b"));
    }

    #[test]
    fn parses_and_formats_the_comma_list() {
        assert_eq!(parse("work, home,,work ,Ünï".as_bytes()), vec!["work", "home", "Ünï"]);
        assert_eq!(format(&["a".to_owned(), "b c".to_owned()]), "a,b c");
        assert!(parse(b"").is_empty());
    }

    #[test]
    fn names_with_commas_are_rejected() {
        assert_eq!(clean_name("  work  ").as_deref(), Some("work"));
        assert_eq!(clean_name("a,b"), None);
        assert_eq!(clean_name("   "), None);
        assert_eq!(clean_name("line\nbreak"), None);
        assert_eq!(clean_name(&"x".repeat(MAX_NAME + 1)), None);
    }

    #[test]
    fn colors_fall_back_to_a_stable_palette_entry() {
        let tags = Tags::with(Registry::default(), BTreeMap::from([("work".to_owned(), [1, 2, 3])]));
        assert_eq!(tags.color("work"), [1, 2, 3]);
        assert_eq!(tags.color("Home"), tags.color("home"));
        assert!(PALETTE.contains(&tags.color("home")));
    }

    #[test]
    fn tags_round_trip_through_the_attribute() {
        let Some((dir, file)) = scratch_file("tags") else { return };
        assert!(read_tags(&file).unwrap().is_empty());
        write_tags(&file, &["work".to_owned(), "home".to_owned()]).unwrap();
        assert_eq!(read_tags(&file).unwrap(), vec!["work", "home"]);
        assert_eq!(xattr::get(&file, ATTR).unwrap().as_deref(), Some(&b"work,home"[..]));
        write_tags(&file, &[]).unwrap();
        assert!(read_tags(&file).unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn toggling_through_the_plugin_tags_and_untags_files() {
        let Some((dir, file)) = scratch_file("toggle") else { return };
        let mut tags = Tags::with(Registry::default(), BTreeMap::new());
        tags.run("toggle:work", std::slice::from_ref(&file), &dir);
        assert_eq!(read_tags(&file).unwrap(), vec!["work"]);
        assert_eq!(tags.registry.with_tag("WORK"), vec![file.clone()]);
        assert_eq!(tags.search_paths("tag:work"), Some(vec![file.clone()]));
        assert_eq!(tags.search_paths("work"), None);
        tags.run("toggle:work", std::slice::from_ref(&file), &dir);
        assert!(read_tags(&file).unwrap().is_empty());
        assert!(tags.registry.with_tag("work").is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_disk_without_attributes_gets_one_clear_message() {
        let err = std::io::Error::from_raw_os_error(libc::ENOTSUP);
        assert_eq!(describe(&err, "a"), tr("This disk can't store tags"));
    }

    #[test]
    fn registry_drops_missing_paths_and_follows_renames() {
        let dir = std::env::temp_dir().join(format!("ski-tag-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a"), "x").unwrap();
        let mut registry = Registry::default();
        registry.record(&dir.join("sub/a"), &["one".to_owned()]);
        registry.record(&dir.join("gone"), &["one".to_owned(), "two".to_owned()]);
        registry.validate(|_| None);
        assert_eq!(registry.with_tag("one"), vec![dir.join("sub/a")]);
        // Tags are read from disk again; a file that lost them leaves the registry
        registry.validate(|_| Some(Vec::new()));
        assert!(registry.names().is_empty());

        registry.record(&dir.join("sub/a"), &["one".to_owned()]);
        assert!(registry.rename(&dir.join("sub"), &dir.join("moved")));
        assert_eq!(registry.with_tag("one"), vec![dir.join("moved/a")]);
        assert!(registry.remove_tree(&dir.join("moved")));
        assert!(registry.names().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_reading_skips_links_and_reuses_unchanged_entries() {
        let Some((dir, file)) = scratch_file("folder") else { return };
        write_tags(&file, &["work".to_owned()]).unwrap();
        std::os::unix::fs::symlink(&file, dir.join("link")).unwrap();
        let first = read_folder(&dir, &HashMap::new());
        assert_eq!(first[&file].tags, vec!["work"]);
        assert!(!first.contains_key(&dir.join("link")));
        // A cached entry with the same key is trusted without reading the attribute
        let mut cached = first.clone();
        cached.get_mut(&file).unwrap().tags = vec!["cached".to_owned()];
        assert_eq!(read_folder(&dir, &cached)[&file].tags, vec!["cached"]);
        // Changing the attribute changes ctime, so the entry is read again
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_tags(&file, &["work".to_owned(), "new".to_owned()]).unwrap();
        assert_eq!(read_folder(&dir, &cached)[&file].tags, vec!["work", "new"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scanning_finds_tagged_files_in_subfolders() {
        let Some((dir, file)) = scratch_file("scan") else { return };
        std::fs::create_dir_all(dir.join("deep/er")).unwrap();
        let deep = dir.join("deep/er/b.txt");
        std::fs::write(&deep, "x").unwrap();
        write_tags(&deep, &["x".to_owned()]).unwrap();
        let found = scan_tree(&dir);
        assert_eq!(found, vec![(deep, vec!["x".to_owned()])]);
        let _ = file;
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn menu_hides_for_links_and_marks_the_tags_all_files_share() {
        let Some((dir, file)) = scratch_file("menu") else { return };
        write_tags(&file, &["work".to_owned()]).unwrap();
        let tags = Tags::with(Registry(BTreeMap::from([(file.clone(), vec!["work".to_owned()])])), BTreeMap::new());
        let item = FileItem::from_path(&file).unwrap();
        let entries = tags.menu_items(std::slice::from_ref(&item));
        let toggle = entries.iter().find(|e| e.id == "toggle:work").unwrap();
        assert_eq!(toggle.icon, icons::CHECK);
        assert!(entries.iter().any(|e| e.id == "new") && entries.iter().any(|e| e.id == "clear"));
        let mut link = item.clone();
        link.is_symlink = true;
        assert!(tags.menu_items(&[link]).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

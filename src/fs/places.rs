use std::collections::HashMap;
use std::path::PathBuf;
use crate::ui::icons;

/// Stand-in location of the Recent view, which lists files from many folders.
pub const RECENT_PATH: &str = "ski:recent";
/// Stand-in location of the Bookmarks view, which lists the hearted files and folders.
pub const BOOKMARKS_PATH: &str = "ski:bookmarks";

/// Whether `path` names a view that lists items from many folders instead of a folder.
pub fn is_virtual(path: &std::path::Path) -> bool {
    path.as_os_str() == RECENT_PATH || path.as_os_str() == BOOKMARKS_PATH
}

/// What a sidebar destination opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    Folder,
    /// Files and folders used lately, from every folder.
    Recent,
    /// Files and folders the user marked with a heart.
    Bookmarks,
}

/// A place the sidebar always offers, which the user can hide and show again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SidebarPlace {
    Home,
    Recent,
    Bookmarks,
    Trash,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
    Computer,
    /// The Connect to server entry under the servers.
    ConnectToServer,
}

impl SidebarPlace {
    /// Off until the user turns it on, so it is kept in the list of places shown, not hidden.
    pub fn opt_in(self) -> bool {
        self == SidebarPlace::ConnectToServer
    }
}

/// Sidebar sections, separated by dividers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceGroup {
    Main,
    Folders,
    Devices,
}

#[derive(Debug, Clone)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
    pub icon: &'static str,
    pub kind: PlaceKind,
    pub group: PlaceGroup,
    /// The device behind a drive entry.
    pub drive: Option<super::drives::Drive>,
    /// Which built-in place this is; drives and the like are `None`.
    pub builtin: Option<SidebarPlace>,
}

impl Place {
    pub fn new(name: impl Into<String>, path: PathBuf, icon: &'static str, group: PlaceGroup) -> Self {
        Self { name: name.into(), path, icon, kind: PlaceKind::Folder, group, drive: None, builtin: None }
    }

    /// Pinned folders and files, saved in `~/.config/ski/pins.json`.
    pub fn load_pins() -> Vec<PathBuf> {
        load_paths("pins.json")
    }

    pub fn save_pins(pins: &[PathBuf]) {
        save_paths("pins.json", pins);
    }

    /// Bookmarked files and folders, saved in `$XDG_STATE_HOME/ski/bookmarks.json`.
    pub fn load_bookmarks() -> Vec<PathBuf> {
        bookmarks_file().map(|f| load_paths_from(&f)).unwrap_or_default()
    }

    pub fn save_bookmarks(bookmarks: &[PathBuf]) {
        if let Some(file) = bookmarks_file() {
            save_paths_to(&file, bookmarks);
        }
    }

    pub fn default_places() -> Vec<Place> {
        use PlaceGroup::{Devices, Folders, Main};
        let builtin = |place: Place, id: SidebarPlace| Place { builtin: Some(id), ..place };
        let mut places = Vec::new();

        if let Some(home) = dirs::home_dir() {
            places.push(builtin(Place::new("Home", home, icons::HOME, Main), SidebarPlace::Home));
        }
        places.push(Place {
            kind: PlaceKind::Recent,
            ..builtin(Place::new("Recent", PathBuf::from(RECENT_PATH), icons::RECENT, Main), SidebarPlace::Recent)
        });
        places.push(Place {
            kind: PlaceKind::Bookmarks,
            ..builtin(Place::new("Bookmarks", PathBuf::from(BOOKMARKS_PATH), icons::HEART, Main), SidebarPlace::Bookmarks)
        });
        if let Some(trash) = super::trash::files_dir() {
            // Created on demand, so the destination always exists
            let _ = std::fs::create_dir_all(&trash);
            places.push(builtin(Place::new("Trash", trash, icons::TRASH, Main), SidebarPlace::Trash));
        }

        let user_dirs = [
            ("Desktop", dirs::desktop_dir(), icons::DESKTOP, SidebarPlace::Desktop),
            ("Documents", dirs::document_dir(), icons::DOCUMENTS, SidebarPlace::Documents),
            ("Downloads", dirs::download_dir(), icons::DOWNLOAD, SidebarPlace::Downloads),
            ("Pictures", dirs::picture_dir(), icons::IMAGE, SidebarPlace::Pictures),
            ("Music", dirs::audio_dir(), icons::AUDIO, SidebarPlace::Music),
            ("Videos", dirs::video_dir(), icons::VIDEO, SidebarPlace::Videos),
        ];
        for (name, dir, icon, id) in user_dirs {
            if let Some(dir) = dir.filter(|d| d.exists()) {
                places.push(builtin(Place::new(name, dir, icon, Folders), id));
            }
        }

        // Drives follow from a background listing (see `drives_ui`), as `lsblk` would hold up the first frame
        places.push(builtin(Place::new("Computer", PathBuf::from("/"), icons::STORAGE, Devices), SidebarPlace::Computer));

        // Compare against canonical paths so symlinked folders still highlight
        for place in &mut places {
            if place.kind == PlaceKind::Folder
                && place.drive.as_ref().is_none_or(|d| d.mounted)
                && let Ok(canonical) = place.path.canonicalize() {
                    place.path = canonical;
                }
        }
        places
    }
}

/// Bookmarks are state rather than settings; platforms without a state directory use the data one.
fn bookmarks_file() -> Option<PathBuf> {
    dirs::state_dir().or_else(dirs::data_dir).map(|d| d.join("ski").join("bookmarks.json"))
}

fn load_paths(name: &str) -> Vec<PathBuf> {
    config_file(name).map(|f| load_paths_from(&f)).unwrap_or_default()
}

fn load_paths_from(file: &std::path::Path) -> Vec<PathBuf> {
    Some(file)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|s| serde_json::from_str::<Vec<PathBuf>>(&s).ok())
        // Folders on a server stay while it is not connected; opening one connects
        .map(|paths| paths.into_iter().filter(|p| p.exists() || super::network::address_of(p).is_some()).collect())
        .unwrap_or_default()
}

fn save_paths(name: &str, paths: &[PathBuf]) {
    if let Some(file) = config_file(name) {
        save_paths_to(&file, paths);
    }
}

fn save_paths_to(file: &std::path::Path, paths: &[PathBuf]) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(paths) {
        let _ = std::fs::write(file, json);
    }
}

/// Custom folder icons: folder path -> image path, saved in `~/.config/ski/folder_icons.json`.
pub fn load_folder_icons() -> HashMap<PathBuf, PathBuf> {
    config_file("folder_icons.json")
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_folder_icons(icons: &HashMap<PathBuf, PathBuf>) {
    let Some(file) = config_file("folder_icons.json") else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(icons) {
        let _ = std::fs::write(file, json);
    }
}

static FOLDER_GLYPHS: std::sync::RwLock<Option<HashMap<PathBuf, char>>> = std::sync::RwLock::new(None);

/// Custom folder glyphs: folder path -> a glyph of the bundled icon font, saved in
/// `~/.config/ski/folder_glyphs.json`. Held globally, like the icon style, so every place a folder's
/// icon is drawn can look it up.
fn with_glyphs<R>(f: impl FnOnce(&mut HashMap<PathBuf, char>) -> R) -> R {
    let mut guard = FOLDER_GLYPHS.write().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(|| {
        config_file("folder_glyphs.json")
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    });
    f(map)
}

pub fn folder_glyph(folder: &std::path::Path) -> Option<char> {
    with_glyphs(|m| m.get(folder).copied())
}

/// Set (or with `None` remove) a folder's glyph; true when something changed.
pub fn set_folder_glyph(folder: &std::path::Path, glyph: Option<char>) -> bool {
    with_glyphs(|m| {
        let changed = match glyph {
            Some(c) => m.insert(folder.to_path_buf(), c) != Some(c),
            None => m.remove(folder).is_some(),
        };
        if changed && let Some(file) = config_file("folder_glyphs.json") {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(json) = serde_json::to_string_pretty(&*m) {
                let _ = std::fs::write(file, json);
            }
        }
        changed
    })
}

/// A folder that was renamed or moved keeps its glyph.
pub fn move_folder_glyph(old: &std::path::Path, new: &std::path::Path) {
    if let Some(c) = folder_glyph(old) {
        set_folder_glyph(old, None);
        set_folder_glyph(new, Some(c));
    }
}

fn config_file(name: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ski").join(name))
}

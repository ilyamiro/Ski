use std::collections::HashMap;
use std::path::PathBuf;
use crate::ui::icons;

/// Stand-in location of the Recent view, which lists files from many folders.
pub const RECENT_PATH: &str = "ski:recent";

/// What a sidebar destination opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    Folder,
    /// Files and folders used lately, from every folder.
    Recent,
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
}

impl Place {
    pub fn new(name: impl Into<String>, path: PathBuf, icon: &'static str, group: PlaceGroup) -> Self {
        Self { name: name.into(), path, icon, kind: PlaceKind::Folder, group }
    }

    /// Pinned folders, saved in `~/.config/ski/pins.json`.
    pub fn load_pins() -> Vec<PathBuf> {
        pins_file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str::<Vec<PathBuf>>(&s).ok())
            .map(|pins| pins.into_iter().filter(|p| p.is_dir()).collect())
            .unwrap_or_default()
    }

    pub fn save_pins(pins: &[PathBuf]) {
        let Some(file) = pins_file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(pins) {
            let _ = std::fs::write(file, json);
        }
    }

    pub fn default_places() -> Vec<Place> {
        use PlaceGroup::{Devices, Folders, Main};
        let mut places = Vec::new();

        if let Some(home) = dirs::home_dir() {
            places.push(Place::new("Home", home, icons::HOME, Main));
        }
        places.push(Place {
            kind: PlaceKind::Recent,
            ..Place::new("Recent", PathBuf::from(RECENT_PATH), icons::RECENT, Main)
        });
        if let Some(trash) = super::trash::files_dir() {
            // Created on demand, so the destination always exists
            let _ = std::fs::create_dir_all(&trash);
            places.push(Place::new("Trash", trash, icons::TRASH, Main));
        }

        let user_dirs = [
            ("Desktop", dirs::desktop_dir(), icons::DESKTOP),
            ("Documents", dirs::document_dir(), icons::DOCUMENTS),
            ("Downloads", dirs::download_dir(), icons::DOWNLOAD),
            ("Pictures", dirs::picture_dir(), icons::IMAGE),
            ("Music", dirs::audio_dir(), icons::AUDIO),
            ("Videos", dirs::video_dir(), icons::VIDEO),
        ];
        for (name, dir, icon) in user_dirs {
            if let Some(dir) = dir.filter(|d| d.exists()) {
                places.push(Place::new(name, dir, icon, Folders));
            }
        }

        places.push(Place::new("Computer", PathBuf::from("/"), icons::STORAGE, Devices));
        places.extend(mounted_drives());

        // Compare against canonical paths so symlinked folders still highlight
        for place in &mut places {
            if place.kind == PlaceKind::Folder
                && let Ok(canonical) = place.path.canonicalize() {
                    place.path = canonical;
                }
        }
        places
    }
}

/// Removable drives the desktop mounted for the user (`/run/media/<user>/<label>` and friends).
fn mounted_drives() -> Vec<Place> {
    let user = std::env::var("USER").unwrap_or_default();
    let roots = [PathBuf::from("/run/media").join(&user), PathBuf::from("/media").join(&user), PathBuf::from("/media")];
    let mut drives: Vec<Place> = roots
        .iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flat_map(|rd| rd.flatten())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Place::new(name, p, icons::USB, PlaceGroup::Devices)
        })
        .collect();
    drives.sort_by_key(|d| d.name.to_lowercase());
    drives
}

fn pins_file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ski").join("pins.json"))
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

fn config_file(name: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ski").join(name))
}

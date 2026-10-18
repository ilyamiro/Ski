use std::collections::HashMap;
use std::path::PathBuf;
use crate::ui::icons;

#[derive(Debug, Clone)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
    pub icon: &'static str,
}

impl Place {
    pub fn new(name: impl Into<String>, path: PathBuf, icon: &'static str) -> Self {
        Self {
            name: name.into(),
            path,
            icon,
        }
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
        let mut places = Vec::new();

        if let Some(home) = dirs::home_dir() {
            places.push(Place::new("Home", home, icons::HOME));
        }

        if let Some(desktop) = dirs::desktop_dir() {
            if desktop.exists() {
                places.push(Place::new("Desktop", desktop, icons::DESKTOP));
            }
        }

        if let Some(docs) = dirs::document_dir() {
            if docs.exists() {
                places.push(Place::new("Documents", docs, icons::DOCUMENTS));
            }
        }

        if let Some(downloads) = dirs::download_dir() {
            if downloads.exists() {
                places.push(Place::new("Downloads", downloads, icons::DOWNLOAD));
            }
        }

        if let Some(pictures) = dirs::picture_dir() {
            if pictures.exists() {
                places.push(Place::new("Pictures", pictures, icons::IMAGE));
            }
        }

        if let Some(music) = dirs::audio_dir() {
            if music.exists() {
                places.push(Place::new("Music", music, icons::AUDIO));
            }
        }

        if let Some(videos) = dirs::video_dir() {
            if videos.exists() {
                places.push(Place::new("Videos", videos, icons::VIDEO));
            }
        }

        places.push(Place::new("Computer", PathBuf::from("/"), icons::STORAGE));

        // Compare against canonical paths so symlinked folders still highlight
        for place in &mut places {
            if let Ok(canonical) = place.path.canonicalize() {
                place.path = canonical;
            }
        }
        places
    }
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

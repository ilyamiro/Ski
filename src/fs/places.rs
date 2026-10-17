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

        places
    }
}

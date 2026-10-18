//! User preferences, stored as JSON in `~/.config/ski/settings.json`.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::app::ViewMode;
use crate::fs::scanner::{SortBy, SortOrder};
use crate::ui::material::Column;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Palette name: `system` follows the Serpantinum colors, otherwise a preset from `theme::PRESETS`.
    pub theme: String,
    /// Accent color as `#rrggbb`; `None` keeps the palette's own.
    pub accent: Option<String>,
    /// Font file for the interface; `None` is the bundled Adwaita Sans.
    pub font: Option<PathBuf>,
    pub font_name: Option<String>,
    pub scale: f32,

    pub view_mode: ViewMode,
    pub sort_by: SortBy,
    pub sort_order: SortOrder,
    pub show_hidden: bool,
    pub folders_first: bool,
    pub columns: Vec<Column>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".to_owned(),
            accent: None,
            font: None,
            font_name: None,
            scale: 1.0,
            view_mode: ViewMode::List,
            sort_by: SortBy::Name,
            sort_order: SortOrder::Ascending,
            show_hidden: false,
            folders_first: true,
            columns: vec![Column::Size, Column::Modified],
        }
    }
}

fn file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ski").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(file) = file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(file, json);
        }
    }
}

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
    /// Dragged list column widths, indexed like `Column::ALL`; 0 keeps the default.
    pub column_widths: [f32; 6],

    // Startup
    pub start_location: StartLocation,
    pub start_path: Option<PathBuf>,
    pub last_dir: Option<PathBuf>,
    // Behavior
    pub single_click_open: bool,
    pub vim_keys: bool,
    pub open_in_new_window: bool,
    pub confirm_delete: bool,
    pub type_to_filter: bool,
    // View
    pub density: Density,
    pub grid_size: GridSize,
    pub grid_name_lines: u8,
    pub show_sidebar: bool,
    pub show_status_bar: bool,
    pub show_thumbnails: bool,
    pub show_extensions: bool,
    pub relative_dates: bool,
    pub decimal_units: bool,
    pub animations: bool,
    pub remember_view_per_folder: bool,
    pub folder_views: std::collections::HashMap<PathBuf, ViewMode>,
    // Keybindings: overrides only; missing actions use `Action::default_binding`.
    pub keybinds: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StartLocation {
    #[default]
    Launch,
    Home,
    LastVisited,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
    Spacious,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Compact, Density::Comfortable, Density::Spacious];

    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Comfortable => "Comfortable",
            Density::Spacious => "Spacious",
        }
    }

    /// One step larger (`1`) or smaller (`-1`), stopping at the ends.
    pub fn step(self, delta: i32) -> Self {
        let at = Self::ALL.iter().position(|d| *d == self).unwrap_or(1) as i32;
        Self::ALL[(at + delta).clamp(0, Self::ALL.len() as i32 - 1) as usize]
    }

    pub fn row_height(self) -> f32 {
        match self {
            Density::Compact => 34.0,
            Density::Comfortable => 44.0,
            Density::Spacious => 54.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GridSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl GridSize {
    pub const ALL: [GridSize; 3] = [GridSize::Small, GridSize::Medium, GridSize::Large];

    pub fn label(self) -> &'static str {
        match self {
            GridSize::Small => "Small",
            GridSize::Medium => "Medium",
            GridSize::Large => "Large",
        }
    }

    /// One step larger (`1`) or smaller (`-1`), stopping at the ends.
    pub fn step(self, delta: i32) -> Self {
        let at = Self::ALL.iter().position(|g| *g == self).unwrap_or(1) as i32;
        Self::ALL[(at + delta).clamp(0, Self::ALL.len() as i32 - 1) as usize]
    }

    pub fn cell_size(self) -> (f32, f32) {
        match self {
            GridSize::Small => (92.0, 104.0),
            GridSize::Medium => (116.0, 128.0),
            GridSize::Large => (144.0, 156.0),
        }
    }

    pub fn icon_size(self) -> f32 {
        match self {
            GridSize::Small => 36.0,
            GridSize::Medium => 48.0,
            GridSize::Large => 64.0,
        }
    }

    pub fn picture_size(self) -> f32 {
        match self {
            GridSize::Small => 56.0,
            GridSize::Medium => 72.0,
            GridSize::Large => 96.0,
        }
    }
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
            column_widths: [0.0; 6],
            start_location: StartLocation::Launch,
            start_path: None,
            last_dir: None,
            single_click_open: false,
            vim_keys: true,
            open_in_new_window: false,
            confirm_delete: true,
            type_to_filter: false,
            density: Density::Comfortable,
            grid_size: GridSize::Medium,
            grid_name_lines: 2,
            show_sidebar: true,
            show_status_bar: true,
            show_thumbnails: true,
            show_extensions: true,
            relative_dates: false,
            decimal_units: false,
            animations: true,
            remember_view_per_folder: false,
            folder_views: Default::default(),
            keybinds: Default::default(),
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

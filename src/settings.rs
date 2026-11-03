//! User preferences, stored as JSON in `~/.config/ski/settings.json`.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::app::ViewMode;
use crate::i18n::tr;
use crate::fs::places::SidebarPlace;
use crate::fs::scanner::{SortBy, SortOrder};
use crate::ui::icon_style::IconStyle;
use crate::ui::material::Column;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Palette name: `system` follows the Serpantinum colors, `custom:<id>` is the user's theme in
    /// `~/.local/state/ski/themes/<id>.json`, otherwise the id of a file in `assets/themes`.
    pub theme: String,
    /// `system` follows the desktop language, otherwise a code from `i18n::Language`.
    pub language: String,
    /// Accent color as `#rrggbb`; `None` keeps the palette's own.
    pub accent: Option<String>,
    /// Font file for the interface; `None` is the bundled Adwaita Sans.
    pub font: Option<PathBuf>,
    pub font_name: Option<String>,
    pub scale: f32,
    /// How round the corners are, relative to the default: 1.0 is the Material look, 0.0 square.
    pub corner_scale: f32,
    /// How opaque the window is, 1 to 100 percent.
    pub opacity: u8,
    /// How folders, file icons and glyphs are drawn.
    pub icon_style: IconStyle,

    pub view_mode: ViewMode,
    /// The file dialog's own list or grid, kept apart from the main window's.
    pub chooser_view_mode: ViewMode,
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
    /// The folders of the open tabs and which one was shown, kept for `StartLocation::RestoreTabs`.
    pub session_tabs: Vec<PathBuf>,
    pub session_active: usize,
    // Behavior
    pub single_click_open: bool,
    pub vim_keys: bool,
    pub open_in_new_window: bool,
    /// Stay running after the window closes, so the next one opens at once.
    pub run_in_background: bool,
    pub confirm_delete: bool,
    /// Copies to the same disk run one after the other.
    pub queue_copies: bool,
    /// Command that opens a terminal; `{}` stands for the folder. Empty finds one by itself.
    pub terminal: String,
    /// Folder whose files are offered as templates for new files; `None` is the desktop's own.
    pub templates_dir: Option<PathBuf>,
    pub type_to_filter: bool,
    /// Typing a name selects the matching file of the folder (unless `type_to_filter` takes the keys).
    pub type_ahead: bool,
    /// A sideways two-finger swipe over the listing goes back or forward.
    pub swipe_navigation: bool,
    /// How far one notch of the mouse wheel scrolls, as a multiple of `SCROLL_STEP` points.
    pub scroll_speed: f32,
    /// Switches of the bundled plugins by id; a plugin not listed keeps its default.
    pub plugins: std::collections::BTreeMap<String, bool>,
    /// Where searches look, and whether they match case or look inside files.
    pub search_scope: crate::fs::search::SearchScope,
    pub search_case: bool,
    pub search_contents: bool,
    // View
    pub density: Density,
    pub grid_size: GridSize,
    pub grid_name_lines: u8,
    pub show_sidebar: bool,
    /// Every disk and partition in the sidebar's Devices, internal ones too; off lists only what is
    /// plugged in or mounted under `/run/media`, `/media` or `/mnt`, beside Computer.
    pub show_all_partitions: bool,
    /// Built-in sidebar places the user hid, from the sidebar's menu or Settings → Sidebar.
    pub hidden_places: Vec<SidebarPlace>,
    /// Places off by default (`SidebarPlace::opt_in`) that the user turned on.
    pub shown_places: Vec<SidebarPlace>,
    pub show_status_bar: bool,
    pub show_thumbnails: bool,
    /// Previews and folder item counts also for files on servers, which reads them over the network.
    pub remote_previews: bool,
    pub show_extensions: bool,
    pub relative_dates: bool,
    pub decimal_units: bool,
    pub folder_sizes: FolderSizeMode,
    /// Folder sizes show the space used on disk instead of the sum of file lengths.
    pub size_on_disk: bool,
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
    /// Every tab that was open when Ski closed.
    RestoreTabs,
}

/// When folders get a size in the listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FolderSizeMode {
    Off,
    /// Only the folders you ask about ("Calculate size").
    #[default]
    OnDemand,
    /// Every folder in the listing.
    Always,
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
            Density::Compact => tr("Compact"),
            Density::Comfortable => tr("Comfortable"),
            Density::Spacious => tr("Spacious"),
        }
    }

    /// One step larger (`1`) or smaller (`-1`), stopping at the ends.
    pub fn step(self, delta: i32) -> Self {
        let at = Self::ALL.iter().position(|d| *d == self).unwrap_or(1) as i32;
        Self::ALL[(at + delta).clamp(0, Self::ALL.len() as i32 - 1) as usize]
    }

    pub fn row_height(self) -> f32 {
        match self {
            Density::Compact => 30.0,
            Density::Comfortable => 38.0,
            Density::Spacious => 48.0,
        }
    }
}

/// Size of the picture box of a grid tile, in points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct GridSize(u32);

impl GridSize {
    pub const MIN: u32 = 32;
    pub const MAX: u32 = 256;
    const STEP: u32 = 16;

    /// Older settings stored the size as a name.
    fn from_name(name: &str) -> Option<Self> {
        Some(Self(match name {
            "Small" => 56,
            "Medium" => 72,
            "Large" => 96,
            _ => return None,
        }))
    }

    pub fn label(self) -> String {
        format!("{} px", self.0)
    }

    /// One step larger (`1`) or smaller (`-1`), stopping at the ends.
    pub fn step(self, delta: i32) -> Self {
        let next = self.0 as i32 + delta * Self::STEP as i32;
        Self(next.clamp(Self::MIN as i32, Self::MAX as i32) as u32)
    }

    pub fn can_step(self, delta: i32) -> bool {
        self.step(delta) != self
    }

    pub fn cell_size(self) -> (f32, f32) {
        let w = (self.0 as f32 + 44.0).max(92.0);
        (w, w + 12.0)
    }

    pub fn icon_size(self) -> f32 {
        self.0 as f32 * 0.92
    }

    pub fn picture_size(self) -> f32 {
        self.0 as f32
    }
}

impl Default for GridSize {
    fn default() -> Self {
        Self(72)
    }
}

impl<'de> Deserialize<'de> for GridSize {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Px(u32),
            Name(String),
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Px(px) => Self(px.clamp(Self::MIN, Self::MAX)),
            Raw::Name(name) => Self::from_name(&name).unwrap_or_default(),
        })
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".to_owned(),
            language: "system".to_owned(),
            terminal: String::new(),
            templates_dir: None,
            accent: None,
            font: None,
            font_name: None,
            scale: 1.0,
            corner_scale: 1.0,
            opacity: 100,
            icon_style: IconStyle::default(),
            view_mode: ViewMode::List,
            chooser_view_mode: ViewMode::List,
            sort_by: SortBy::Name,
            sort_order: SortOrder::Ascending,
            show_hidden: false,
            folders_first: true,
            columns: vec![Column::Size, Column::Modified],
            column_widths: [0.0; 6],
            start_location: StartLocation::Launch,
            start_path: None,
            last_dir: None,
            session_tabs: Vec::new(),
            session_active: 0,
            single_click_open: false,
            vim_keys: false,
            open_in_new_window: false,
            run_in_background: true,
            confirm_delete: true,
            queue_copies: true,
            type_to_filter: false,
            type_ahead: true,
            swipe_navigation: false,
            scroll_speed: SCROLL_SPEED_DEFAULT,
            plugins: Default::default(),
            search_scope: Default::default(),
            search_case: false,
            search_contents: false,
            density: Density::Comfortable,
            grid_size: GridSize::default(),
            grid_name_lines: 2,
            show_sidebar: true,
            show_all_partitions: true,
            hidden_places: Vec::new(),
            shown_places: Vec::new(),
            show_status_bar: true,
            show_thumbnails: true,
            remote_previews: false,
            show_extensions: true,
            relative_dates: true,
            decimal_units: false,
            folder_sizes: FolderSizeMode::default(),
            size_on_disk: false,
            animations: true,
            remember_view_per_folder: false,
            folder_views: Default::default(),
            keybinds: Default::default(),
        }
    }
}

/// Points one wheel notch scrolls at speed 1. egui's own 40 felt slow beside other file managers.
pub const SCROLL_STEP: f32 = 60.0;
pub const SCROLL_SPEED_DEFAULT: f32 = 1.0;
/// What the slider in the settings covers; `settings.json` may hold any value in `SCROLL_SPEED_RANGE`.
pub const SCROLL_SPEED_SLIDER: std::ops::RangeInclusive<f32> = 0.25..=2.25;
pub const SCROLL_SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.1..=5.0;

pub(crate) fn file() -> Option<PathBuf> {
    crate::fs::admin::config_override().or_else(dirs::config_dir).map(|d| d.join("ski").join("settings.json"))
}

/// Settings fixed from outside, next to `settings.json`: the home-manager module writes them
/// (`programs.ski.settings`). Ski never writes this file; what it sets wins over `settings.json`
/// on every read, so a setting it names can be changed in the window only until the next read.
pub(crate) fn managed_file() -> Option<PathBuf> {
    file().map(|f| f.with_file_name("managed.json"))
}

/// `over` laid onto `base`: objects merge key by key, anything else replaces what was there.
fn merge(base: &mut serde_json::Value, over: serde_json::Value) {
    match (base, over) {
        (serde_json::Value::Object(base), serde_json::Value::Object(over)) => {
            for (key, value) in over {
                merge(base.entry(key).or_insert(serde_json::Value::Null), value);
            }
        }
        (base, over) => *base = over,
    }
}

impl Settings {
    pub fn load() -> Self {
        Self::read().and_then(Result::ok).unwrap_or_default()
    }

    /// The settings as the files have them now, `managed.json` over `settings.json`: `None` when
    /// there is neither, an error when one is not valid JSON (say, half way through an edit by hand).
    pub fn read() -> Option<Result<Self, serde_json::Error>> {
        let own = file().and_then(|f| std::fs::read_to_string(f).ok());
        let managed = managed_file().and_then(|f| std::fs::read_to_string(f).ok());
        if own.is_none() && managed.is_none() {
            return None;
        }
        Some(Self::from_texts(own.as_deref(), managed.as_deref()))
    }

    /// `settings.json`'s text with `managed.json`'s laid over it.
    fn from_texts(own: Option<&str>, managed: Option<&str>) -> Result<Self, serde_json::Error> {
        let mut value = match own {
            Some(text) => serde_json::from_str(text)?,
            None => serde_json::Value::Object(Default::default()),
        };
        if let Some(managed) = managed {
            merge(&mut value, serde_json::from_str(managed)?);
        }
        serde_json::from_value(value)
    }

    /// The tabs of the last session that can still be opened, and the position of the one that was
    /// shown among them. Folders that no longer exist are dropped.
    pub fn restorable_session(&self) -> (Vec<PathBuf>, usize) {
        let kept: Vec<PathBuf> = self.session_tabs.iter().filter(|p| p.is_dir()).cloned().collect();
        let active = self.session_tabs.get(self.session_active).filter(|p| p.is_dir());
        let shown = active.map_or(0, |_| self.session_tabs[..self.session_active].iter().filter(|p| p.is_dir()).count());
        let shown = shown.min(kept.len().saturating_sub(1));
        (kept, shown)
    }

    pub fn save(&self) {
        // As administrator the settings belong to someone else's home
        if crate::fs::admin::is_admin() {
            return;
        }
        let Some(file) = file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(file, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_settings_win_key_by_key() {
        let own = r#"{ "theme": "nord", "show_hidden": true, "plugins": { "git": false, "tags": true } }"#;
        let managed = r#"{ "theme": "light", "icon_style": "System", "scroll_speed": 1.5, "plugins": { "git": true } }"#;
        let s = Settings::from_texts(Some(own), Some(managed)).unwrap();
        assert_eq!(s.theme, "light");
        assert!(s.show_hidden, "a key only settings.json has is kept");
        assert_eq!(s.icon_style, IconStyle::System);
        assert_eq!(s.scroll_speed, 1.5);
        assert_eq!(s.plugins.get("git"), Some(&true));
        assert_eq!(s.plugins.get("tags"), Some(&true), "maps merge instead of being replaced");
    }

    #[test]
    fn managed_settings_alone_fill_in_the_defaults() {
        let s = Settings::from_texts(None, Some(r#"{ "view_mode": "Grid" }"#)).unwrap();
        assert_eq!(s.view_mode, ViewMode::Grid);
        assert_eq!(s.scroll_speed, SCROLL_SPEED_DEFAULT);
        assert!(Settings::from_texts(Some("{ broken"), None).is_err());
        assert!(Settings::from_texts(None, Some("[1, 2")).is_err());
    }
}

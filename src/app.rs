use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use eframe::egui;

use crate::fs::{
    file_item::FileItem,
    apps::{self, AppEntry},
    places::{load_folder_icons, save_folder_icons, Place},
    props::{FolderScan, Properties},
    scanner::{read_directory, ListOptions, SortBy, SortOrder, TypeFilter},
    recent,
    search::Search,
};
use crate::fs::ops::{self, ArchiveFormat, Jobs};
use crate::settings::Settings;
use crate::thumbs::Thumbnails;
use crate::ui::{
    icons,
    material::{self, chip, file_row, file_tile, filled_icon_button, icon_button, menu_item, nav_item, Column, CONTROL_HEIGHT},
    fonts::SystemFont,
    theme::SerpantinumTheme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ViewMode {
    List,
    Grid,
}

/// What a right click landed on.
#[derive(Clone)]
enum MenuTarget {
    Item(FileItem),
    Folder,
    Pin(PathBuf),
    /// The hamburger menu next to the search field.
    Main,
}

struct ContextMenu {
    pos: egui::Pos2,
    target: MenuTarget,
    /// "Open with <default app>" for files that have one; resolved off the UI thread.
    open_label: Option<String>,
    label_rx: Option<mpsc::Receiver<Option<String>>>,
}

#[derive(Clone)]
enum MenuAction {
    Open(FileItem),
    OpenPath(PathBuf),
    CopyPath(PathBuf),
    Properties(PathBuf),
    OpenWith(PathBuf),
    SetIcon(PathBuf),
    Copy(Vec<PathBuf>),
    Cut(Vec<PathBuf>),
    Paste,
    SelectAll,
    CopyTo(Vec<PathBuf>),
    MoveTo(Vec<PathBuf>),
    Compress(Vec<PathBuf>),
    ExtractHere(Vec<PathBuf>),
    ExtractTo(Vec<PathBuf>),
    UseAsIcon(PathBuf),
    Pin(PathBuf),
    Unpin(PathBuf),
    NewFile,
    NewFolder,
    Refresh,
    NewWindow,
    OpenSettings,
    Shortcuts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferOp {
    Copy,
    Move,
    Extract,
}

pub(crate) enum Dialog {
    Create { is_dir: bool, name: String, error: Option<String>, first_frame: bool },
    Properties { props: Box<Properties>, scan: Option<FolderScan> },
    OpenWith {
        path: PathBuf,
        mime: String,
        apps: Vec<AppEntry>,
        /// Indexes into `apps`: the app currently used, the ones made for this type, everything else.
        default: Option<usize>,
        recommended: Vec<usize>,
        others: Vec<usize>,
        selected: Option<usize>,
        query: String,
        first_frame: bool,
        for_file: bool,
        for_type: bool,
    },
    PickFolder {
        op: TransferOp,
        items: Vec<PathBuf>,
        dir: PathBuf,
        path_input: String,
    },
    Compress {
        sources: Vec<PathBuf>,
        name: String,
        format: ArchiveFormat,
        first_frame: bool,
    },
    Settings {
        tab: usize,
        font_query: String,
        hex: String,
        picking: Option<usize>,
        defaults: Vec<Option<String>>,
        keybind_search: String,
        capture: Option<(crate::keymap::Action, Option<usize>)>,
        conflict_warning: Option<(String, crate::keymap::Action, crate::keymap::Action, Option<usize>)>,
    },
    Shortcuts,
    Icon { folder: PathBuf, input: String, error: Option<String>, candidates: Vec<PathBuf>, browse: Option<PathBuf> },
}

pub struct SkiApp {
    current_path: PathBuf,
    items: Vec<FileItem>,
    selected_index: Option<usize>,
    /// Every selected row (Ctrl / Shift click); `selected_index` is the focused one.
    selection: BTreeSet<usize>,
    anchor: Option<usize>,
    last_cursor: Option<usize>,
    /// Files copied or cut, waiting for a paste (`true` = cut).
    clipboard: Option<(Vec<PathBuf>, bool)>,
    jobs: Jobs,
    error_message: Option<String>,

    // Dynamic shell theme from Serpantinum
    pub(crate) theme: SerpantinumTheme,

    // Navigation history
    back_history: Vec<PathBuf>,
    forward_history: Vec<PathBuf>,

    // Search and display options
    search_query: String,
    pub(crate) show_hidden: bool,
    pub(crate) folders_first: bool,
    type_filter: TypeFilter,
    pub(crate) view_mode: ViewMode,
    pub(crate) sort_by: SortBy,
    pub(crate) sort_order: SortOrder,

    // Path edit mode (Ctrl+L)
    is_editing_path: bool,
    path_input: String,

    thumbs: Thumbnails,
    search: Search,

    // Places
    places: Vec<Place>,

    // Header search field and settings popup
    show_search: bool,
    focus_search: bool,
    show_settings: bool,
    settings_button_rect: egui::Rect,
    main_button_rect: egui::Rect,

    // Optional list columns, pinned folders, context menu and modal dialog
    pub(crate) columns: Vec<Column>,
    folder_icons: HashMap<PathBuf, PathBuf>,
    open_overrides: HashMap<PathBuf, String>,
    pub(crate) app_icons: HashMap<String, Option<egui::TextureHandle>>,

    // Persisted preferences, and what the next frame has to re-apply to egui
    pub(crate) settings: Settings,
    pub(crate) saved_settings: Settings,
    pub(crate) system_theme: SerpantinumTheme,
    pub(crate) system_fonts: Option<Vec<SystemFont>>,
    pub(crate) theme_dirty: bool,
    pub(crate) fonts_dirty: bool,
    pins: Vec<PathBuf>,
    menu: Option<ContextMenu>,
    pub(crate) dialog: Option<Dialog>,

    // Status toast message (text, timestamp)
    status_toast: Option<(String, std::time::Instant)>,

    // Flag to scroll to selected item on next frame
    scroll_to_selected: bool,

    pub(crate) capturing_key: bool,
}

impl SkiApp {
    pub fn new(initial_path: PathBuf, theme: SerpantinumTheme, settings: Settings, ctx: &egui::Context) -> Self {
        apps::preload();
        let places = Place::default_places();
        let canonical_path = initial_path.canonicalize().unwrap_or(initial_path);

        let mut settings = settings;
        settings.last_dir = Some(canonical_path.clone());
        crate::fs::file_item::set_format_options(settings.decimal_units, settings.relative_dates);
        crate::fs::file_item::set_show_extensions(settings.show_extensions);
        let view_mode = if settings.remember_view_per_folder {
            settings.folder_views.get(&canonical_path).copied().unwrap_or(settings.view_mode)
        } else {
            settings.view_mode
        };

        let mut app = Self {
            current_path: canonical_path.clone(),
            items: Vec::new(),
            selected_index: None,
            selection: BTreeSet::new(),
            anchor: None,
            last_cursor: None,
            clipboard: None,
            jobs: Jobs::new(ctx),
            error_message: None,
            theme,
            back_history: Vec::new(),
            forward_history: Vec::new(),
            search_query: String::new(),
            show_hidden: settings.show_hidden,
            folders_first: settings.folders_first,
            type_filter: TypeFilter::All,
            view_mode,
            sort_by: settings.sort_by,
            sort_order: settings.sort_order,
            is_editing_path: false,
            path_input: canonical_path.to_string_lossy().to_string(),
            thumbs: Thumbnails::new(ctx),
            search: Search::new(),
            places,
            show_search: false,
            focus_search: false,
            show_settings: false,
            settings_button_rect: egui::Rect::NOTHING,
            main_button_rect: egui::Rect::NOTHING,
            columns: settings.columns.clone(),
            folder_icons: load_folder_icons(),
            open_overrides: apps::load_overrides(),
            app_icons: HashMap::new(),
            saved_settings: settings.clone(),
            settings,
            system_theme: SerpantinumTheme::load(),
            system_fonts: None,
            theme_dirty: false,
            fonts_dirty: false,
            pins: Place::load_pins(),
            menu: None,
            dialog: None,
            status_toast: None,
            scroll_to_selected: false,
            capturing_key: false,
        };

        app.reload_items();
        app
    }

    pub fn navigate_to(&mut self, target: PathBuf) {
        if target == self.current_path {
            return;
        }

        let canonical = target.canonicalize().unwrap_or(target);
        if canonical.is_dir() {
            self.back_history.push(self.current_path.clone());
            self.forward_history.clear();
            recent::record(&canonical);
            self.current_path = canonical;
            self.settings.last_dir = Some(self.current_path.clone());
            if self.settings.remember_view_per_folder {
                if let Some(&saved_mode) = self.settings.folder_views.get(&self.current_path) {
                    self.view_mode = saved_mode;
                }
            }
            self.path_input = self.current_path.to_string_lossy().to_string();
            self.selected_index = None;
            self.search_query.clear();
            self.search.cancel();
            self.show_search = false;
            self.is_editing_path = false;
            self.reload_items();
        } else {
            self.open_file(&canonical);
        }
    }

    pub fn go_back(&mut self) {
        if let Some(prev) = self.back_history.pop() {
            self.forward_history.push(self.current_path.clone());
            self.current_path = prev;
            self.settings.last_dir = Some(self.current_path.clone());
            if self.settings.remember_view_per_folder {
                if let Some(&saved_mode) = self.settings.folder_views.get(&self.current_path) {
                    self.view_mode = saved_mode;
                }
            }
            self.path_input = self.current_path.to_string_lossy().to_string();
            self.selected_index = None;
            self.reload_items();
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(next) = self.forward_history.pop() {
            self.back_history.push(self.current_path.clone());
            self.current_path = next;
            self.settings.last_dir = Some(self.current_path.clone());
            if self.settings.remember_view_per_folder {
                if let Some(&saved_mode) = self.settings.folder_views.get(&self.current_path) {
                    self.view_mode = saved_mode;
                }
            }
            self.path_input = self.current_path.to_string_lossy().to_string();
            self.selected_index = None;
            self.reload_items();
        }
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            self.navigate_to(parent.to_path_buf());
        }
    }

    pub fn refresh(&mut self) {
        // Also reload the theme from disk in case the wallpaper / Matugen theme changed
        self.system_theme = SerpantinumTheme::load();
        self.rebuild_theme();
        self.reload_items();
        self.set_toast("Refreshed");
    }

    pub fn reload_items(&mut self) {
        let opts = ListOptions {
            show_hidden: self.show_hidden,
            sort_by: self.sort_by,
            sort_order: self.sort_order,
            folders_first: self.folders_first,
            type_filter: self.type_filter,
        };
        match read_directory(&self.current_path, &opts) {
            Ok(items) => {
                self.items = items;
                self.error_message = None;
                if self.settings.show_thumbnails {
                    self.thumbs.preload(self.items.iter().map(|i| i.path.as_path()));
                }
                for item in self.items.iter().filter(|i| i.is_dir) {
                    if let Some(icon) = self.folder_icons.get(&item.path) {
                        self.thumbs.ensure(icon);
                    }
                }
                if self.items.is_empty() {
                    self.selected_index = None;
                } else if let Some(idx) = self.selected_index {
                    if idx >= self.items.len() {
                        self.selected_index = Some(self.items.len() - 1);
                    }
                }
            }
            Err(err) => {
                self.items.clear();
                self.selected_index = None;
                self.error_message = Some(format!("Failed to open directory: {err}"));
            }
        }
        self.sync_selection(true);
    }

    fn open_file(&mut self, path: &Path) {
        recent::record(path);
        let chosen = self.open_overrides.get(path).cloned();
        if let Some(id) = chosen {
            let app = self.installed_apps().iter().find(|a| a.id == id).cloned();
            match app {
                Some(app) => {
                    if let Err(err) = apps::launch(&app, path) {
                        self.set_toast(format!("Failed to start {}: {err}", app.name));
                    }
                    return;
                }
                // The chosen app is gone; fall back to the system default
                None => {
                    self.open_overrides.remove(path);
                    apps::save_overrides(&self.open_overrides);
                }
            }
        }
        if let Err(err) = open::that(path) {
            self.set_toast(format!("Failed to open file: {err}"));
        }
    }

    /// Keep `selection` in step with the focused row when something else (keyboard, reload)
    /// moved it. Pass `force` after the listing itself changed.
    fn sync_selection(&mut self, force: bool) {
        if force || self.selected_index != self.last_cursor {
            self.selection = self.selected_index.into_iter().collect();
            self.anchor = self.selected_index;
            self.last_cursor = self.selected_index;
        }
        let len = self.items.len();
        self.selection.retain(|i| *i < len);
    }

    /// Plain click selects one row, Ctrl toggles it, Shift selects the range from the anchor.
    fn click_select(&mut self, idx: usize, modifiers: egui::Modifiers) {
        if modifiers.shift {
            let anchor = self.anchor.unwrap_or(idx);
            let (lo, hi) = (anchor.min(idx), anchor.max(idx));
            if !modifiers.command {
                self.selection.clear();
            }
            self.selection.extend(lo..=hi);
        } else if modifiers.command {
            if !self.selection.remove(&idx) {
                self.selection.insert(idx);
            }
            self.anchor = Some(idx);
        } else {
            self.selection = BTreeSet::from([idx]);
            self.anchor = Some(idx);
        }
        self.selected_index = Some(idx);
        self.last_cursor = Some(idx);
    }

    fn select_all(&mut self) {
        self.selection = (0..self.items.len()).collect();
        self.anchor = Some(0);
        if self.selected_index.is_none() && !self.items.is_empty() {
            self.selected_index = Some(0);
        }
        self.last_cursor = self.selected_index;
    }

    /// Paths of everything selected, in list order.
    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selection.iter().filter_map(|i| self.items.get(*i)).map(|i| i.path.clone()).collect()
    }

    /// The items a context menu on `item` acts on: the whole selection if `item` is part of it.
    fn menu_group(&self, item: &FileItem) -> Vec<FileItem> {
        let in_selection = self.selection.len() > 1
            && self.items.iter().position(|i| i.path == item.path).is_some_and(|idx| self.selection.contains(&idx));
        if in_selection {
            self.selection.iter().filter_map(|i| self.items.get(*i)).cloned().collect()
        } else {
            vec![item.clone()]
        }
    }

    fn describe_count(paths: &[PathBuf]) -> String {
        match paths {
            [one] => format!("“{}”", one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            many => format!("{} items", many.len()),
        }
    }

    fn set_clipboard(&mut self, paths: Vec<PathBuf>, cut: bool) {
        if paths.is_empty() {
            return;
        }
        let message = format!("{} {}", if cut { "Cut" } else { "Copied" }, Self::describe_count(&paths));
        self.clipboard = Some((paths, cut));
        self.set_toast(message);
    }

    fn paste(&mut self) {
        let Some((paths, cut)) = self.clipboard.clone() else {
            self.set_toast("Nothing to paste");
            return;
        };
        let dest = self.current_path.clone();
        self.transfer(if cut { TransferOp::Move } else { TransferOp::Copy }, paths, dest);
        if cut {
            self.clipboard = None;
        }
    }

    /// Copy, move or extract `items` into `dest` on a worker thread.
    fn transfer(&mut self, op: TransferOp, items: Vec<PathBuf>, dest: PathBuf) {
        let label = Self::describe_count(&items);
        let target = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dest.display().to_string());
        self.set_toast(match op {
            TransferOp::Copy => format!("Copying {label}…"),
            TransferOp::Move => format!("Moving {label}…"),
            TransferOp::Extract => format!("Extracting {label}…"),
        });
        self.jobs.spawn(move || {
            let mut failures = Vec::new();
            for item in &items {
                let result = match op {
                    TransferOp::Copy => ops::copy_into(item, &dest).map(|_| ()),
                    TransferOp::Move => ops::move_into(item, &dest).map(|_| ()),
                    TransferOp::Extract => ops::extract(item, &dest),
                };
                if let Err(err) = result {
                    failures.push(format!("{}: {err}", item.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                }
            }
            if failures.is_empty() {
                let verb = match op {
                    TransferOp::Copy => "Copied",
                    TransferOp::Move => "Moved",
                    TransferOp::Extract => "Extracted",
                };
                Ok(format!("{verb} {label} to “{target}”"))
            } else {
                Err(failures.join(" · "))
            }
        });
    }

    fn extract_beside(&mut self, archives: Vec<PathBuf>) {
        let archives: Vec<PathBuf> = archives.into_iter().filter(|p| ops::is_archive(p)).collect();
        if archives.is_empty() {
            return;
        }
        let label = Self::describe_count(&archives);
        self.set_toast(format!("Extracting {label}…"));
        self.jobs.spawn(move || {
            let mut failures = Vec::new();
            let mut last = None;
            for archive in &archives {
                match ops::extract_beside(archive) {
                    Ok(dir) => last = Some(dir),
                    Err(err) => failures.push(format!("{}: {err}", archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
                }
            }
            if failures.is_empty() {
                let folder = last.and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
                Ok(if archives.len() == 1 { format!("Extracted to “{folder}”") } else { format!("Extracted {label}") })
            } else {
                Err(failures.join(" · "))
            }
        });
    }

    fn open_compress_dialog(&mut self, sources: Vec<PathBuf>) {
        let base = self.current_path.clone();
        let sources: Vec<PathBuf> = sources.into_iter().filter(|p| p.parent() == Some(base.as_path())).collect();
        if sources.is_empty() {
            self.set_toast("Open the folder first to compress its files");
            return;
        }
        let name = match sources.as_slice() {
            [one] => ops::archive_stem(one),
            _ => base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".to_owned()),
        };
        self.dialog = Some(Dialog::Compress { sources, name, format: ArchiveFormat::Zip, first_frame: true });
    }

    fn open_pick_folder(&mut self, op: TransferOp, items: Vec<PathBuf>) {
        let dir = self.current_path.clone();
        self.dialog = Some(Dialog::PickFolder { op, path_input: dir.to_string_lossy().into_owned(), dir, items });
    }

    /// Called once per frame: report finished background jobs.
    fn poll_jobs(&mut self) {
        let outcomes = self.jobs.poll();
        if outcomes.is_empty() {
            return;
        }
        for outcome in outcomes {
            let message = if outcome.ok { outcome.message } else { format!("Failed: {}", outcome.message) };
            self.set_toast(message);
        }
        if self.search_query.trim().is_empty() {
            self.reload_items();
        }
    }

    /// Show the context menu right away. The "Open with <app>" label needs process calls
    /// (`xdg-mime`) and the application scan, so a worker resolves it and the menu fills it in later.
    fn open_menu(&mut self, ctx: &egui::Context, pos: egui::Pos2, target: MenuTarget) {
        let label_rx = match &target {
            MenuTarget::Item(item) if !item.is_dir => {
                let (tx, rx) = mpsc::channel();
                let path = item.path.clone();
                let chosen = self.open_overrides.get(&path).cloned();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let id = chosen.or_else(|| apps::default_for(&apps::mime_of(&path)));
                    let label = id.and_then(|id| apps::installed().iter().find(|a| a.id == id).map(|a| format!("Open with {}", a.name)));
                    let _ = tx.send(label);
                    ctx.request_repaint();
                });
                Some(rx)
            }
            _ => None,
        };
        self.menu = Some(ContextMenu { pos, target, open_label: None, label_rx });
    }

    fn toggle_main_menu(&mut self, ctx: &egui::Context) {
        if matches!(self.menu.as_ref().map(|m| &m.target), Some(MenuTarget::Main)) {
            self.menu = None;
        } else {
            let rect = self.main_button_rect;
            self.open_menu(ctx, egui::pos2(rect.left(), rect.bottom() + 6.0), MenuTarget::Main);
        }
    }

    /// Another Ski window on the current folder.
    fn new_window(&mut self) {
        let started = std::env::current_exe()
            .and_then(|exe| std::process::Command::new(exe).arg(&self.current_path).spawn());
        if let Err(err) = started {
            self.set_toast(format!("Could not open a new window: {err}"));
        }
    }

    pub(crate) fn rebuild_theme(&mut self) {
        let mut theme = SerpantinumTheme::from_settings(&self.settings);
        if self.settings.theme == "system" {
            let accent = self.settings.accent.as_deref().and_then(crate::ui::theme::parse_hex);
            theme = self.system_theme;
            if let Some(a) = accent {
                theme.mauve = a;
                theme.blue = a;
            }
        }
        self.theme = theme;
        self.theme_dirty = true;
    }

    /// Re-apply theme / font / scale to egui after a change, and write changed preferences to disk.
    fn sync_settings(&mut self, ctx: &egui::Context) {
        if self.theme_dirty {
            crate::ui::theme::apply(ctx, &self.theme);
            self.theme_dirty = false;
        }
        if self.fonts_dirty {
            crate::ui::fonts::install(ctx, self.settings.font.as_deref());
            self.fonts_dirty = false;
        }
        if (ctx.zoom_factor() - self.settings.scale).abs() > f32::EPSILON {
            ctx.set_zoom_factor(self.settings.scale);
        }
        ctx.all_styles_mut(|s| s.animation_time = if self.settings.animations { 0.25 } else { 0.0 });
        crate::fs::file_item::set_format_options(self.settings.decimal_units, self.settings.relative_dates);
        crate::fs::file_item::set_show_extensions(self.settings.show_extensions);

        if self.settings.remember_view_per_folder {
            self.settings.folder_views.insert(self.current_path.clone(), self.view_mode);
        }

        let current = Settings {
            view_mode: self.view_mode,
            sort_by: self.sort_by,
            sort_order: self.sort_order,
            show_hidden: self.show_hidden,
            folders_first: self.folders_first,
            columns: self.columns.clone(),
            ..self.settings.clone()
        };
        if current != self.saved_settings {
            current.save();
            self.saved_settings = current;
        }
    }

    pub fn anim_time(&self, d: f32) -> f32 {
        if self.settings.animations { d } else { 0.0 }
    }

    fn is_act_pressed(&self, ctx: &egui::Context, act: crate::keymap::Action) -> bool {
        crate::keymap::is_action_pressed(ctx, &self.settings.keybinds, act, self.settings.vim_keys)
    }

    pub(crate) fn installed_apps(&self) -> &'static [AppEntry] {
        apps::installed()
    }

    pub(crate) fn set_toast(&mut self, message: impl Into<String>) {
        self.status_toast = Some((message.into(), std::time::Instant::now()));
    }

    fn close_search(&mut self) {
        self.show_search = false;
        if !self.search_query.is_empty() {
            self.search_query.clear();
            self.search.cancel();
            self.reload_items();
        }
    }

    /// Search the whole home folder; results replace the folder listing until the query is cleared.
    fn on_query_changed(&mut self, ctx: &egui::Context) {
        if self.search_query.trim().is_empty() {
            self.search.cancel();
            self.reload_items();
        } else {
            self.items.clear();
            self.selected_index = None;
            self.sync_selection(true);
            self.error_message = None;
            let root = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            self.search.start(ctx, root, self.search_query.trim().to_owned(), self.show_hidden);
        }
    }

    fn move_selection(&mut self, delta: isize, extend: bool) {
        if self.items.is_empty() {
            self.selected_index = None;
            return;
        }

        let new_idx = match self.selected_index {
            Some(curr) => {
                let next = curr as isize + delta;
                if next < 0 {
                    0
                } else if next >= self.items.len() as isize {
                    self.items.len() - 1
                } else {
                    next as usize
                }
            }
            None => {
                if delta >= 0 {
                    0
                } else {
                    self.items.len() - 1
                }
            }
        };

        if extend {
            let anchor = self.anchor.or(self.selected_index).unwrap_or(new_idx);
            self.selection = (anchor.min(new_idx)..=anchor.max(new_idx)).collect();
            self.anchor = Some(anchor);
            self.selected_index = Some(new_idx);
            self.last_cursor = Some(new_idx);
        } else {
            self.selected_index = Some(new_idx);
        }
        self.scroll_to_selected = true;
    }

    fn open_selected(&mut self) {
        if let Some(idx) = self.selected_index {
            if let Some(item) = self.items.get(idx).cloned() {
                self.open_item(item);
            }
        }
    }

    fn handle_keyboard_inputs(&mut self, ctx: &egui::Context) {
        if self.capturing_key {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.dialog.take().is_some() || self.menu.take().is_some() {
                return;
            }
        }
        if self.dialog.is_some() {
            return;
        }

        if ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.is_editing_path = false;
                self.close_search();
            }
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.menu.take().is_some() {
                return;
            }
            if self.show_settings {
                self.show_settings = false;
                return;
            }
            if self.show_search || !self.search_query.is_empty() {
                self.close_search();
                return;
            }
        }

        let mut handled = false;

        if self.is_act_pressed(ctx, crate::keymap::Action::ShowShortcuts) {
            self.dialog = Some(Dialog::Shortcuts);
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::Filter) {
            self.show_search = true;
            self.focus_search = true;
            handled = true;
        }

        let shift = ctx.input(|i| i.modifiers.shift);
        if self.is_act_pressed(ctx, crate::keymap::Action::MoveDown) {
            self.move_selection(1, shift);
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::MoveUp) {
            self.move_selection(-1, shift);
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::Open) {
            self.open_selected();
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::ParentFolder) {
            self.go_up();
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::First) {
            if !self.items.is_empty() {
                self.selected_index = Some(0);
                self.scroll_to_selected = true;
            }
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::Last) {
            if !self.items.is_empty() {
                self.selected_index = Some(self.items.len() - 1);
                self.scroll_to_selected = true;
            }
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::Back) {
            self.go_back();
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::Forward) {
            self.go_forward();
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::ListView) {
            self.view_mode = ViewMode::List;
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::GridView) {
            self.view_mode = ViewMode::Grid;
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::ToggleHidden) {
            self.show_hidden = !self.show_hidden;
            self.reload_items();
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::PinFolder) {
            self.toggle_pin(self.current_path.clone());
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::Properties) {
            let target = self
                .selected_index
                .and_then(|idx| self.items.get(idx))
                .map(|item| item.path.clone())
                .unwrap_or_else(|| self.current_path.clone());
            self.open_properties(&target);
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::EditPath) {
            self.is_editing_path = true;
            self.path_input = self.current_path.to_string_lossy().to_string();
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::Refresh) {
            self.refresh();
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::CopyPath) {
            if let Some(item) = self.selected_index.and_then(|idx| self.items.get(idx)) {
                let path_str = item.path.to_string_lossy().to_string();
                ctx.copy_text(path_str);
                self.set_toast(format!("Copied path: {}", item.name));
            }
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::NewFolder) {
            self.open_create_dialog(true);
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::NewWindow) {
            self.new_window();
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::Settings) {
            self.open_settings_dialog();
            handled = true;
        }

        // Ctrl+C / X / V arrive as dedicated events on some platforms and as key presses on others
        let (ev_copy, ev_cut, ev_paste) = ctx.input(|i| {
            let event = |f: fn(&egui::Event) -> bool| i.events.iter().any(f);
            (
                event(|e| matches!(e, egui::Event::Copy)),
                event(|e| matches!(e, egui::Event::Cut)),
                event(|e| matches!(e, egui::Event::Paste(_))),
            )
        });
        if ev_copy || self.is_act_pressed(ctx, crate::keymap::Action::Copy) {
            self.set_clipboard(self.selected_paths(), false);
            handled = true;
        } else if ev_cut || self.is_act_pressed(ctx, crate::keymap::Action::Cut) {
            self.set_clipboard(self.selected_paths(), true);
            handled = true;
        } else if ev_paste || self.is_act_pressed(ctx, crate::keymap::Action::Paste) {
            self.paste();
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::SelectAll) {
            self.select_all();
            handled = true;
        }

        // type_to_filter: when on, printable key with no ctrl/alt and nothing focused opens the filter field with it
        if !handled && self.settings.type_to_filter && ctx.input(|i| !i.modifiers.command && !i.modifiers.alt) {
            let typed = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Text(text) = e {
                        let printable: String = text.chars().filter(|c| !c.is_control()).collect();
                        if !printable.is_empty() {
                            Some(printable)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            });
            if let Some(text) = typed {
                self.show_search = true;
                self.focus_search = true;
                self.search_query.push_str(&text);
                self.on_query_changed(ctx);
            }
        }
    }
}

/// Content width below which the sidebar is hidden.
const SIDEBAR_BREAKPOINT: f32 = 680.0;

impl eframe::App for SkiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_keyboard_inputs(&ctx);
        self.sync_selection(false);
        self.poll_jobs();
        self.sync_settings(&ctx);
        self.thumbs.poll(&ctx);

        let found = self.search.poll();
        if !found.is_empty() && !self.search_query.trim().is_empty() {
            self.items.extend(found);
        }

        if let Some((_, time)) = &self.status_toast {
            if time.elapsed().as_secs() > 3 && !self.jobs.running() {
                self.status_toast = None;
            }
        }

        let window_width = ui.available_width();
        let sidebar_visible = self.settings.show_sidebar && (window_width >= SIDEBAR_BREAKPOINT);

        if sidebar_visible {
            egui::Panel::left("sidebar_places")
                .exact_size(212.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(self.theme.mantle)
                        .inner_margin(egui::Margin::symmetric(10, 10)),
                )
                .show(ui, |ui| self.render_sidebar(ui));
        }

        egui::Panel::top("top_navigation")
            .frame(
                egui::Frame::new()
                    .fill(self.theme.base)
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| self.render_top_bar(ui, sidebar_visible));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(self.theme.base)
                    .inner_margin(egui::Margin { left: 6, right: 6, top: 0, bottom: 6 }),
            )
            .show(ui, |ui| {
                let background = ui.interact(ui.max_rect(), ui.make_persistent_id("list_background"), egui::Sense::click());
                if background.secondary_clicked() && self.search_query.trim().is_empty() {
                    if let Some(pos) = background.interact_pointer_pos() {
                        self.open_menu(ui.ctx(), pos, MenuTarget::Folder);
                    }
                }

                if let Some(ref err) = self.error_message {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new(err).color(self.theme.red));
                    });
                } else if self.items.is_empty() {
                    let msg = if !self.search_query.trim().is_empty() {
                        "Searching…"
                    } else if self.type_filter != TypeFilter::All {
                        "Nothing matches"
                    } else {
                        "This folder is empty"
                    };
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new(msg).size(14.0).color(self.theme.subtext1));
                    });
                } else {
                    match self.view_mode {
                        ViewMode::List => self.render_list_view(ui),
                        ViewMode::Grid => self.render_grid_view(ui),
                    }
                }
            });

        self.render_settings(&ctx);
        self.render_menu(&ctx);
        self.render_dialog(&ctx);
        self.render_toast(&ctx);
    }
}

impl SkiApp {
    /// One fixed-height row: every control is `CONTROL_HEIGHT` tall and centered on the same axis.
    fn render_top_bar(&mut self, ui: &mut egui::Ui, sidebar_visible: bool) {
        let gap = 2.0;

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), CONTROL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = gap;

                let can_back = !self.back_history.is_empty();
                if icon_button(ui, "btn_back", icons::ARROW_BACK, &self.theme, can_back, "Back").clicked() {
                    self.go_back();
                }
                let can_forward = !self.forward_history.is_empty();
                if icon_button(ui, "btn_fwd", icons::ARROW_FORWARD, &self.theme, can_forward, "Forward").clicked() {
                    self.go_forward();
                }

                // Search lives in the sidebar; without a sidebar it becomes a header button.
                let right_buttons = if sidebar_visible { 2.0 } else { 4.0 };
                let reserved = right_buttons * (CONTROL_HEIGHT + gap);
                let title_width = (ui.available_width() - reserved - gap).max(40.0);

                let (title_rect, title_resp) =
                    ui.allocate_exact_size(egui::vec2(title_width, CONTROL_HEIGHT), egui::Sense::click());

                if self.show_search && !sidebar_visible {
                    ui.painter().rect_filled(title_rect, egui::CornerRadius::same(17), self.theme.surface1);
                    self.render_search_field(ui, title_rect);
                } else if self.is_editing_path {
                    ui.painter().rect_filled(title_rect, egui::CornerRadius::same(17), self.theme.surface1);
                    self.render_path_field(ui, title_rect);
                } else {
                    self.render_title(ui, title_rect, &title_resp);
                }

                if !sidebar_visible {
                    let icon = if self.show_search { icons::CLOSE } else { icons::SEARCH };
                    if icon_button(ui, "btn_search", icon, &self.theme, true, "Search (/)").clicked() {
                        if self.show_search {
                            self.close_search();
                        } else {
                            self.show_search = true;
                            self.focus_search = true;
                            self.is_editing_path = false;
                        }
                    }
                }

                if !sidebar_visible {
                    let menu = icon_button(ui, "btn_main_menu", icons::MENU, &self.theme, true, "Main menu");
                    self.main_button_rect = menu.rect;
                    if menu.clicked() {
                        self.toggle_main_menu(ui.ctx());
                    }
                }

                let view_icon = match self.view_mode {
                    ViewMode::List => icons::VIEW_LIST,
                    ViewMode::Grid => icons::GRID_VIEW,
                };
                let view = icon_button(ui, "btn_view_options", view_icon, &self.theme, true, "View options");
                self.settings_button_rect = view.rect;
                if view.clicked() {
                    self.show_settings = !self.show_settings;
                }

                let close = filled_icon_button(ui, "btn_close", icons::CLOSE, &self.theme, 28.0, "Close", Some(self.theme.red));
                if close.clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            },
        );
    }

    fn render_search_field(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        material::paint_icon(
            ui.painter(),
            egui::pos2(rect.left() + 18.0, rect.center().y),
            icons::SEARCH,
            16.0,
            self.theme.subtext1,
        );
        let edit_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 36.0, rect.top()),
            egui::pos2(rect.right() - 12.0, rect.bottom()),
        );
        let resp = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut self.search_query)
                .frame(egui::Frame::NONE)
                .align(egui::Align2::LEFT_CENTER)
                .font(egui::FontId::proportional(13.5))
                .hint_text("Search files"),
        );
        if self.focus_search {
            resp.request_focus();
            self.focus_search = false;
        }
        if resp.changed() {
            let ctx = ui.ctx().clone();
            self.on_query_changed(&ctx);
        }
    }

    fn render_path_field(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let edit_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 16.0, rect.top()),
            egui::pos2(rect.right() - 12.0, rect.bottom()),
        );
        let resp = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut self.path_input)
                .frame(egui::Frame::NONE)
                .align(egui::Align2::LEFT_CENTER)
                .font(egui::FontId::proportional(13.5))
                .hint_text("Type a path and press Enter"),
        );
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }
        if resp.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let target = PathBuf::from(&self.path_input);
                self.is_editing_path = false;
                self.navigate_to(target);
            } else {
                self.is_editing_path = false;
            }
        }
    }

    /// The location as plain text: the current folder, with its parent as a quiet link in front.
    /// Clicking anywhere else on the title switches to typing a path.
    fn render_title(&mut self, ui: &mut egui::Ui, rect: egui::Rect, field: &egui::Response) {
        let cy = rect.center().y;
        let name_font = egui::FontId::proportional(15.0);
        let parent_font = egui::FontId::proportional(13.5);
        let mut x = rect.left() + 10.0;

        let searching = !self.search_query.trim().is_empty();
        let name = if searching {
            format!("Results for “{}”", self.search_query.trim())
        } else {
            self.current_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Computer".to_owned())
        };

        if !searching {
            let parent_name = self
                .current_path
                .parent()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
            let name_w = ui.painter().layout_no_wrap(name.clone(), name_font.clone(), self.theme.text).size().x;

            if let Some(parent) = parent_name {
                let pw = ui.painter().layout_no_wrap(parent.clone(), parent_font.clone(), self.theme.subtext1).size().x;
                let slash_w = 18.0;
                // Only show the parent when it fits next to the folder name
                if pw + slash_w + name_w + 20.0 < rect.width() {
                    let r = egui::Rect::from_min_size(egui::pos2(x - 6.0, cy - 14.0), egui::vec2(pw + 12.0, 28.0));
                    let resp = ui.interact(r, ui.make_persistent_id("title_parent"), egui::Sense::click());
                    let hover = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.18);
                    ui.painter().rect_filled(r, egui::CornerRadius::same(14), self.theme.with_alpha(self.theme.text, (hover * 20.0) as u8));
                    let color = material::lerp_color(self.theme.subtext1, self.theme.text, hover);
                    ui.painter().text(egui::pos2(x, cy), egui::Align2::LEFT_CENTER, &parent, parent_font.clone(), color);
                    if resp.clicked() {
                        if let Some(p) = self.current_path.parent() {
                            let p = p.to_path_buf();
                            self.navigate_to(p);
                        }
                    }
                    x += pw + 6.0;
                    ui.painter().text(egui::pos2(x, cy), egui::Align2::LEFT_CENTER, "/", parent_font.clone(), self.theme.surface2);
                    x += slash_w - 6.0;
                }
            }
        }

        let max_w = (rect.right() - x - 4.0).max(10.0);
        let mut job = egui::text::LayoutJob::simple_singleline(name, name_font, self.theme.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(x, cy - galley.size().y / 2.0), galley, self.theme.text);

        if field.clicked() && !searching {
            self.is_editing_path = true;
            self.path_input = self.current_path.to_string_lossy().to_string();
        }
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        self.render_sidebar_search(ui);

        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.y = 2.0;

        let browsing = self.search_query.trim().is_empty();
        let mut destination: Option<PathBuf> = None;
        let mut menu_at: Option<(egui::Pos2, MenuTarget)> = None;
        for (idx, place) in self.places.iter().enumerate() {
            let selected = browsing && self.current_path == place.path;
            if nav_item(ui, ("place", idx), place.icon, &place.name, &self.theme, selected).clicked() {
                destination = Some(place.path.clone());
            }
        }

        if !self.pins.is_empty() {
            ui.add_space(10.0);
            ui.label(egui::RichText::new("  Pinned").size(11.5).color(self.theme.subtext1));
            ui.add_space(2.0);
        }
        for (idx, pin) in self.pins.iter().enumerate() {
            let selected = browsing && self.current_path == *pin;
            let name = pin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| pin.display().to_string());
            let resp = nav_item(ui, ("pin", idx), icons::FOLDER, &name, &self.theme, selected);
            if resp.clicked() {
                destination = Some(pin.clone());
            }
            if resp.secondary_clicked() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    menu_at = Some((pos, MenuTarget::Pin(pin.clone())));
                }
            }
        }

        if let Some((pos, target)) = menu_at {
            self.open_menu(ui.ctx(), pos, target);
        }
        if let Some(dest) = destination {
            self.close_search();
            self.navigate_to(dest);
        }
    }

    /// Search icon (growing into the search field when clicked) with the main menu button at the right.
    fn render_sidebar_search(&mut self, ui: &mut egui::Ui) {
        let open = self.show_search || !self.search_query.is_empty();
        let open_t = ui.ctx().animate_bool_with_time_and_easing(egui::Id::unique("sidebar_search_open"), open, self.anim_time(0.3), material::quint_out);

        let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), CONTROL_HEIGHT), egui::Sense::hover());
        let menu_rect = egui::Rect::from_min_size(egui::pos2(row.right() - CONTROL_HEIGHT, row.top()), egui::vec2(CONTROL_HEIGHT, CONTROL_HEIGHT));
        let menu = ui.scope_builder(egui::UiBuilder::new().max_rect(menu_rect), |ui| {
            icon_button(ui, "sidebar_main_menu", icons::MENU, &self.theme, true, "Main menu")
        });
        self.main_button_rect = menu.inner.rect;
        if menu.inner.clicked() {
            self.toggle_main_menu(ui.ctx());
        }

        // The search grows over the free space left of the menu button
        let full = row.width() - CONTROL_HEIGHT - 4.0;
        let slot = egui::Rect::from_min_size(row.min, egui::vec2(full, CONTROL_HEIGHT));

        if !open && open_t <= 0.0 {
            let button = ui.scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| {
                icon_button(ui, "sidebar_search_btn", icons::SEARCH, &self.theme, true, "Search (/)")
            });
            if button.inner.clicked() {
                self.show_search = true;
                self.focus_search = true;
            }
            return;
        }

        let rect = egui::Rect::from_min_size(slot.min, egui::vec2(CONTROL_HEIGHT + (full - CONTROL_HEIGHT) * open_t, CONTROL_HEIGHT));

        let search_id = egui::Id::unique("sidebar_search");
        let focused = ui.memory(|m| m.has_focus(search_id));
        let fill = if focused { self.theme.surface2 } else { self.theme.surface1 };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(17), fill);
        material::paint_icon(ui.painter(), egui::pos2(rect.left() + 17.0, rect.center().y), icons::SEARCH, 16.0, self.theme.subtext1);

        if open_t < 0.6 {
            return;
        }
        let clear_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 18.0, rect.center().y), egui::vec2(26.0, 26.0));
        let edit_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 36.0, rect.top()),
            egui::pos2(clear_rect.left() - 4.0, rect.bottom()),
        );
        let resp = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut self.search_query)
                .id(search_id)
                .frame(egui::Frame::NONE)
                .align(egui::Align2::LEFT_CENTER)
                .font(egui::FontId::proportional(13.5))
                .hint_text("Search files"),
        );
        if self.focus_search {
            resp.request_focus();
            self.focus_search = false;
        }
        if resp.changed() {
            let ctx = ui.ctx().clone();
            self.on_query_changed(&ctx);
        }

        let clear = ui.interact(clear_rect, ui.make_persistent_id("sidebar_search_clear"), egui::Sense::click());
        let hover = ui.ctx().animate_bool_with_time(clear.id.with("h"), clear.hovered(), 0.18);
        ui.painter().rect_filled(clear_rect, egui::CornerRadius::same(13), self.theme.with_alpha(self.theme.text, (hover * 26.0) as u8));
        material::paint_icon(ui.painter(), clear_rect.center(), icons::CLOSE, 14.0, material::lerp_color(self.theme.subtext1, self.theme.text, hover));
        if clear.clicked() {
            self.close_search();
        }
    }

    fn open_item(&mut self, item: FileItem) {
        if item.is_dir {
            if self.settings.open_in_new_window {
                let started = std::env::current_exe()
                    .and_then(|exe| std::process::Command::new(exe).arg(&item.path).spawn());
                if let Err(err) = started {
                    self.set_toast(format!("Could not open a new window: {err}"));
                }
            } else {
                self.navigate_to(item.path);
            }
        } else {
            self.open_file(&item.path);
        }
    }

    /// Parent folder of a search result, with the home folder abbreviated.
    fn path_hint(&self, item: &FileItem) -> Option<String> {
        if self.search_query.trim().is_empty() {
            return None;
        }
        let parent = item.path.parent()?;
        let home = dirs::home_dir();
        Some(match home.as_ref().and_then(|h| parent.strip_prefix(h).ok()) {
            Some(rel) if rel.as_os_str().is_empty() => "~".to_owned(),
            Some(rel) => format!("~/{}", rel.display()),
            None => parent.display().to_string(),
        })
    }

    fn render_list_view(&mut self, ui: &mut egui::Ui) {
        let mut opened: Option<FileItem> = None;
        let should_scroll = std::mem::take(&mut self.scroll_to_selected);
        let row_height = self.settings.density.row_height();
        let show_thumbs = self.settings.show_thumbnails;

        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut menu_at: Option<(egui::Pos2, MenuTarget)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (idx, item) in self.items.iter().enumerate() {
                let selected = self.selection.contains(&idx);
                let hint = self.path_hint(item);
                let thumb = thumb_for(&self.thumbs, &self.folder_icons, item, show_thumbs);
                let resp = file_row(ui, idx, item, &self.theme, selected, should_scroll, thumb, hint.as_deref(), &self.columns, row_height);
                let mods = ui.input(|i| i.modifiers);
                if resp.clicked() {
                    if self.settings.single_click_open && !mods.command && !mods.shift {
                        opened = Some(item.clone());
                    } else {
                        clicked = Some((idx, mods));
                    }
                } else if resp.secondary_clicked() && !selected {
                    clicked = Some((idx, egui::Modifiers::NONE));
                }
                if resp.double_clicked() {
                    opened = Some(item.clone());
                }
                if resp.secondary_clicked() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        menu_at = Some((pos, MenuTarget::Item(item.clone())));
                    }
                }
            }
        });

        if let Some((idx, modifiers)) = clicked {
            self.click_select(idx, modifiers);
        }
        if let Some((pos, target)) = menu_at {
            self.open_menu(ui.ctx(), pos, target);
        }
        if let Some(item) = opened {
            self.open_item(item);
        }
    }

    fn render_grid_view(&mut self, ui: &mut egui::Ui) {
        let mut opened: Option<FileItem> = None;
        let should_scroll = std::mem::take(&mut self.scroll_to_selected);
        let (cell_w, cell_h) = self.settings.grid_size.cell_size();
        let cell = egui::vec2(cell_w, cell_h);
        let grid_size = self.settings.grid_size;
        let grid_lines = self.settings.grid_name_lines as usize;
        let show_thumbs = self.settings.show_thumbnails;

        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut menu_at: Option<(egui::Pos2, MenuTarget)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let spacing = 4.0;
            let cols = ((ui.available_width() + spacing) / (cell.x + spacing)).floor().max(1.0) as usize;
            ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);

            for (row_idx, row) in self.items.chunks(cols).enumerate() {
                ui.horizontal(|ui| {
                    for (col_idx, item) in row.iter().enumerate() {
                        let idx = row_idx * cols + col_idx;
                        let selected = self.selection.contains(&idx);
                        let thumb = thumb_for(&self.thumbs, &self.folder_icons, item, show_thumbs);
                        let resp = file_tile(ui, idx, item, &self.theme, selected, should_scroll, thumb, grid_size, grid_lines);
                        let mods = ui.input(|i| i.modifiers);
                        if resp.clicked() {
                            if self.settings.single_click_open && !mods.command && !mods.shift {
                                opened = Some(item.clone());
                            } else {
                                clicked = Some((idx, mods));
                            }
                        } else if resp.secondary_clicked() && !selected {
                            clicked = Some((idx, egui::Modifiers::NONE));
                        }
                        if resp.double_clicked() {
                            opened = Some(item.clone());
                        }
                        if resp.secondary_clicked() {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                menu_at = Some((pos, MenuTarget::Item(item.clone())));
                            }
                        }
                    }
                });
            }
        });

        if let Some((idx, modifiers)) = clicked {
            self.click_select(idx, modifiers);
        }
        if let Some((pos, target)) = menu_at {
            self.open_menu(ui.ctx(), pos, target);
        }
        if let Some(item) = opened {
            self.open_item(item);
        }
    }

    fn render_toast(&mut self, ctx: &egui::Context) {
        let Some((msg, _)) = &self.status_toast else { return };
        egui::Area::new(egui::Id::unique("toast"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -20.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.theme.surface2)
                    .corner_radius(egui::CornerRadius::same(18))
                    .inner_margin(egui::Margin::symmetric(18, 10))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(msg).size(13.5).color(self.theme.text));
                    });
            });
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }

    fn render_settings(&mut self, ctx: &egui::Context) {
        let t = ctx.animate_bool_with_time_and_easing(egui::Id::unique("settings_anim"), self.show_settings, self.anim_time(0.32), material::quint_out);
        if t <= 0.0 {
            return;
        }

        let screen = ctx.content_rect();
        let width = (screen.width() - 24.0).clamp(240.0, 340.0);
        let max_height = (screen.height() - 80.0).max(160.0);
        let mut reload = false;

        // Fades in while sliding down from the button; closing plays the same motion backwards.
        let area = egui::Area::new(egui::Id::unique("settings_popup"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 52.0 - (1.0 - t) * 14.0))
            .interactable(self.show_settings)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .corner_radius(egui::CornerRadius::same(material::RADIUS_CONTAINER))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(width - 32.0);
                        egui::ScrollArea::vertical().max_height(max_height - 32.0).show(ui, |ui| {
                            reload = self.view_options_contents(ui);
                        });
                    });
            });

        // Click outside closes the popup (the settings button handles its own toggle).
        let rect = area.response.rect;
        let outside_press = self.show_settings
            && ctx.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|p| !rect.contains(p) && !self.settings_button_rect.contains(p))
            });
        if outside_press {
            self.show_settings = false;
        }
        if reload {
            if self.search_query.trim().is_empty() {
                self.reload_items();
            } else {
                self.on_query_changed(ctx);
            }
        }
    }

    fn section_title(&self, ui: &mut egui::Ui, text: &str) {
        material::section_label(ui, text, &self.theme);
    }

    /// View, sorting, visibility and column options; shared by the popup and the settings dialog.
    /// Returns true when a setting that affects the file listing changed.
    fn file_options(&mut self, ui: &mut egui::Ui) -> bool {
        let mut reload = false;
        self.section_title(ui, "View");
        ui.horizontal_wrapped(|ui| {
            if chip(ui, "set_list", Some(icons::VIEW_LIST), "List", &self.theme, self.view_mode == ViewMode::List).clicked() {
                self.view_mode = ViewMode::List;
            }
            if chip(ui, "set_grid", Some(icons::GRID_VIEW), "Grid", &self.theme, self.view_mode == ViewMode::Grid).clicked() {
                self.view_mode = ViewMode::Grid;
            }
        });

        self.section_title(ui, "Sort by");
        ui.horizontal_wrapped(|ui| {
            for (field, label) in [(SortBy::Name, "Name"), (SortBy::Size, "Size"), (SortBy::Modified, "Modified")] {
                if chip(ui, ("set_sort", label), None, label, &self.theme, self.sort_by == field).clicked() {
                    self.sort_by = field;
                    reload = true;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            let asc = self.sort_order == SortOrder::Ascending;
            if chip(ui, "set_asc", Some(icons::SORT_ASC), "Ascending", &self.theme, asc).clicked() {
                self.sort_order = SortOrder::Ascending;
                reload = true;
            }
            if chip(ui, "set_desc", Some(icons::SORT_DESC), "Descending", &self.theme, !asc).clicked() {
                self.sort_order = SortOrder::Descending;
                reload = true;
            }
        });

        self.section_title(ui, "Show");
        ui.horizontal_wrapped(|ui| {
            let icon = if self.show_hidden { icons::VISIBILITY } else { icons::VISIBILITY_OFF };
            if chip(ui, "set_hidden", Some(icon), "Hidden files", &self.theme, self.show_hidden).clicked() {
                self.show_hidden = !self.show_hidden;
                reload = true;
            }
            if chip(ui, "set_folders_first", Some(icons::FOLDER), "Folders first", &self.theme, self.folders_first).clicked() {
                self.folders_first = !self.folders_first;
                reload = true;
            }
        });

        self.section_title(ui, "List columns");
        ui.horizontal_wrapped(|ui| {
            for column in Column::ALL {
                let on = self.columns.contains(&column);
                if chip(ui, ("set_col", column.label()), None, column.label(), &self.theme, on).clicked() {
                    self.columns = Column::ALL
                        .into_iter()
                        .filter(|c| (*c == column) != self.columns.contains(c))
                        .collect();
                }
            }
        });

        self.section_title(ui, "File type");
        ui.horizontal_wrapped(|ui| {
            for filter in TypeFilter::ALL {
                if chip(ui, ("set_type", filter.label()), None, filter.label(), &self.theme, self.type_filter == filter).clicked() {
                    self.type_filter = filter;
                    reload = true;
                }
            }
        });

        reload
    }

    /// Contents of the view options popup: only what changes how this folder is shown.
    /// Returns true when a setting that affects the file listing changed.
    fn view_options_contents(&mut self, ui: &mut egui::Ui) -> bool {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
        ui.label(egui::RichText::new("View").size(16.0).color(self.theme.text));
        self.file_options(ui)
    }

    fn shortcuts_contents(&mut self, ui: &mut egui::Ui) -> bool {
        let theme = self.theme;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Keyboard shortcuts").size(material::FONT_TITLE).color(theme.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if filled_icon_button(ui, "sc_close", icons::CLOSE, &theme, 30.0, "Close", Some(theme.red)).clicked() {
                    close = true;
                }
            });
        });
        let max_height = (ui.ctx().content_rect().height() - 220.0).clamp(200.0, 520.0);
        egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
            for group in ["Navigation", "Files", "View"] {
                ui.add_space(8.0);
                material::section_label(ui, group, &theme);
                egui::Grid::new(format!("shortcuts_grid_{group}")).num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                    for action in crate::keymap::Action::ALL {
                        if action.group() != group {
                            continue;
                        }
                        let bindings = crate::keymap::bindings(&self.settings.keybinds, action);
                        if bindings.is_empty() {
                            continue;
                        }
                        let key_str = bindings.join(" / ");
                        ui.label(egui::RichText::new(key_str).size(material::FONT_CHIP).color(theme.mauve));
                        ui.label(egui::RichText::new(action.label()).size(material::FONT_CHIP).color(theme.subtext0));
                        ui.end_row();
                    }
                });
            }
        });
        close
    }
}

/// Context menu, pinning, file creation and the properties dialog.
impl SkiApp {
    fn toggle_pin(&mut self, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        if let Some(idx) = self.pins.iter().position(|p| *p == path) {
            self.pins.remove(idx);
            self.set_toast("Unpinned from sidebar");
        } else if self.places.iter().any(|p| p.path == path) {
            self.set_toast("Already in the sidebar");
            return;
        } else {
            self.pins.push(path);
            self.set_toast("Pinned to sidebar");
        }
        Place::save_pins(&self.pins);
    }

    fn open_properties(&mut self, path: &Path) {
        match Properties::load(path) {
            Some(props) => {
                let scan = props.item.is_dir.then(|| FolderScan::start(path.to_path_buf()));
                self.dialog = Some(Dialog::Properties { props: Box::new(props), scan });
            }
            None => self.set_toast("Could not read properties"),
        }
    }

    fn open_create_dialog(&mut self, is_dir: bool) {
        let base = if is_dir { "New Folder" } else { "Untitled" };
        let mut name = base.to_owned();
        let mut n = 2;
        while self.current_path.join(&name).exists() {
            name = format!("{base} {n}");
            n += 1;
        }
        self.dialog = Some(Dialog::Create { is_dir, name, error: None, first_frame: true });
    }

    fn create_entry(&mut self, is_dir: bool, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err("Enter a valid name".to_owned());
        }
        let path = self.current_path.join(name);
        let result = if is_dir {
            std::fs::create_dir(&path)
        } else {
            std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map(|_| ())
        };
        result.map_err(|err| match err.kind() {
            std::io::ErrorKind::AlreadyExists => format!("“{name}” already exists"),
            _ => err.to_string(),
        })?;

        self.reload_items();
        if let Some(idx) = self.items.iter().position(|i| i.name == name) {
            self.selected_index = Some(idx);
            self.scroll_to_selected = true;
        }
        self.set_toast(format!("Created {name}"));
        Ok(())
    }

    fn run_action(&mut self, ctx: &egui::Context, action: MenuAction) {
        match action {
            MenuAction::Open(item) => self.open_item(item),
            MenuAction::OpenPath(path) => self.navigate_to(path),
            MenuAction::CopyPath(path) => {
                ctx.copy_text(path.to_string_lossy().to_string());
                self.set_toast("Copied path");
            }
            MenuAction::Properties(path) => self.open_properties(&path),
            MenuAction::OpenWith(path) => self.open_open_with(path),
            MenuAction::SetIcon(folder) => self.open_icon_dialog(folder),
            MenuAction::UseAsIcon(image) => {
                let folder = self.current_path.clone();
                self.apply_folder_icon(&folder, Some(&image));
            }
            MenuAction::Pin(path) | MenuAction::Unpin(path) => self.toggle_pin(path),
            MenuAction::NewFile => self.open_create_dialog(false),
            MenuAction::NewFolder => self.open_create_dialog(true),
            MenuAction::Refresh => self.refresh(),
            MenuAction::NewWindow => self.new_window(),
            MenuAction::OpenSettings => self.open_settings_dialog(),
            MenuAction::Shortcuts => self.dialog = Some(Dialog::Shortcuts),
            MenuAction::Copy(paths) => self.set_clipboard(paths, false),
            MenuAction::Cut(paths) => self.set_clipboard(paths, true),
            MenuAction::Paste => self.paste(),
            MenuAction::SelectAll => self.select_all(),
            MenuAction::CopyTo(paths) => self.open_pick_folder(TransferOp::Copy, paths),
            MenuAction::MoveTo(paths) => self.open_pick_folder(TransferOp::Move, paths),
            MenuAction::Compress(paths) => self.open_compress_dialog(paths),
            MenuAction::ExtractHere(paths) => self.extract_beside(paths),
            MenuAction::ExtractTo(paths) => {
                let archives: Vec<PathBuf> = paths.into_iter().filter(|p| ops::is_archive(p)).collect();
                self.open_pick_folder(TransferOp::Extract, archives);
            }
        }
    }

    fn menu_entries(&self, target: &MenuTarget) -> Vec<Vec<(&'static str, &'static str, MenuAction)>> {
        let pin_entry = |path: &PathBuf| {
            if self.pins.contains(path) {
                (icons::PIN_OFF, "Unpin from sidebar", MenuAction::Unpin(path.clone()))
            } else {
                (icons::PIN, "Pin to sidebar", MenuAction::Pin(path.clone()))
            }
        };
        let icon_entry = |path: &PathBuf| (icons::IMAGE, "Custom icon…", MenuAction::SetIcon(path.clone()));
        // Clipboard / transfer / archive entries shared by every selection
        let file_groups = |paths: Vec<PathBuf>, archives: bool| {
            let mut groups = vec![
                vec![
                    (icons::COPY, "Copy", MenuAction::Copy(paths.clone())),
                    (icons::CUT, "Cut", MenuAction::Cut(paths.clone())),
                ],
                vec![
                    (icons::FOLDER_COPY, "Copy to…", MenuAction::CopyTo(paths.clone())),
                    (icons::FOLDER_MOVE, "Move to…", MenuAction::MoveTo(paths.clone())),
                ],
            ];
            let mut pack = vec![(icons::COMPRESS, "Compress…", MenuAction::Compress(paths.clone()))];
            if archives {
                pack.push((icons::EXTRACT, "Extract here", MenuAction::ExtractHere(paths.clone())));
                pack.push((icons::EXTRACT, "Extract to…", MenuAction::ExtractTo(paths)));
            }
            groups.push(pack);
            groups
        };
        match target {
            MenuTarget::Item(item) if self.menu_group(item).len() > 1 => {
                let group = self.menu_group(item);
                let paths: Vec<PathBuf> = group.iter().map(|i| i.path.clone()).collect();
                let archives = group.iter().any(|i| !i.is_dir && ops::is_archive(&i.path));
                let mut groups = file_groups(paths, archives);
                groups.push(vec![(icons::INFO, "Properties", MenuAction::Properties(item.path.clone()))]);
                groups
            }
            MenuTarget::Item(item) if item.is_dir => {
                let mut groups = vec![
                    vec![(icons::OPEN, "Open", MenuAction::Open(item.clone()))],
                    vec![pin_entry(&item.path), icon_entry(&item.path)],
                ];
                groups.extend(file_groups(vec![item.path.clone()], false));
                groups.push(vec![
                    (icons::COPY, "Copy path", MenuAction::CopyPath(item.path.clone())),
                    (icons::INFO, "Properties", MenuAction::Properties(item.path.clone())),
                ]);
                groups
            }
            MenuTarget::Item(item) => {
                let mut groups = vec![vec![
                    (icons::OPEN, "Open", MenuAction::Open(item.clone())),
                    (icons::APPS, "Open with…", MenuAction::OpenWith(item.path.clone())),
                ]];
                if crate::thumbs::is_image(&item.path) {
                    groups.push(vec![(icons::IMAGE, "Use as folder icon", MenuAction::UseAsIcon(item.path.clone()))]);
                }
                groups.extend(file_groups(vec![item.path.clone()], ops::is_archive(&item.path)));
                groups.push(vec![
                    (icons::COPY, "Copy path", MenuAction::CopyPath(item.path.clone())),
                    (icons::INFO, "Properties", MenuAction::Properties(item.path.clone())),
                ]);
                groups
            }
            MenuTarget::Folder => vec![
                vec![
                    (icons::FILE_PLUS, "New file", MenuAction::NewFile),
                    (icons::FOLDER_PLUS, "New folder", MenuAction::NewFolder),
                ],
                {
                    let mut edit = vec![(icons::SELECT_ALL, "Select all", MenuAction::SelectAll)];
                    if self.clipboard.is_some() {
                        edit.insert(0, (icons::PASTE, "Paste", MenuAction::Paste));
                    }
                    edit
                },
                vec![pin_entry(&self.current_path), icon_entry(&self.current_path)],
                vec![(icons::REFRESH, "Refresh", MenuAction::Refresh)],
                vec![
                    (icons::COPY, "Copy path", MenuAction::CopyPath(self.current_path.clone())),
                    (icons::INFO, "Properties", MenuAction::Properties(self.current_path.clone())),
                ],
            ],
            MenuTarget::Main => vec![
                vec![(icons::NEW_WINDOW, "New window", MenuAction::NewWindow)],
                vec![
                    (icons::REFRESH, "Refresh and reload theme", MenuAction::Refresh),
                    (icons::SETTINGS, "Settings", MenuAction::OpenSettings),
                ],
                vec![(icons::KEYBOARD, "Keyboard shortcuts", MenuAction::Shortcuts)],
            ],
            MenuTarget::Pin(path) => vec![
                vec![(icons::OPEN, "Open", MenuAction::OpenPath(path.clone()))],
                vec![(icons::PIN_OFF, "Unpin from sidebar", MenuAction::Unpin(path.clone())), icon_entry(path)],
                vec![(icons::INFO, "Properties", MenuAction::Properties(path.clone()))],
            ],
        }
    }

    fn render_menu(&mut self, ctx: &egui::Context) {
        if let Some(menu) = self.menu.as_mut() {
            if let Some(Ok(label)) = menu.label_rx.as_ref().map(|rx| rx.try_recv()) {
                menu.open_label = label;
                menu.label_rx = None;
            }
        }
        let Some(menu) = self.menu.as_ref() else { return };
        let (pos, groups, open_label) = (menu.pos, self.menu_entries(&menu.target), menu.open_label.clone());

        let count: usize = groups.iter().map(Vec::len).sum();
        let screen = ctx.content_rect();
        let size = egui::vec2(250.0, count as f32 * 36.0 + groups.len().saturating_sub(1) as f32 * 11.0 + 16.0);
        let origin = egui::pos2(
            pos.x.min(screen.right() - size.x - 8.0).max(8.0),
            pos.y.min(screen.bottom() - size.y - 8.0).max(8.0),
        );

        let is_main = matches!(self.menu.as_ref().map(|m| &m.target), Some(MenuTarget::Main));
        let mut chosen = None;
        let area = egui::Area::new(egui::Id::unique("context_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(origin)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .corner_radius(egui::CornerRadius::same(material::RADIUS_CONTAINER))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(size.x - 16.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (g, group) in groups.iter().enumerate() {
                            if g > 0 {
                                material::divider(ui, &self.theme);
                            }
                            for (idx, (icon, label, action)) in group.iter().enumerate() {
                                let shown = match action {
                                    MenuAction::Open(item) if !item.is_dir => open_label.as_deref().unwrap_or(label),
                                    _ => label,
                                };
                                if menu_item(ui, ("menu", g, idx), icon, shown, &self.theme).clicked() {
                                    chosen = Some(action.clone());
                                }
                            }
                        }
                    });
            });

        let rect = area.response.rect;
        let main_button = self.main_button_rect;
        let outside_press = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !(is_main && main_button.contains(p)))
        });
        if chosen.is_some() || outside_press {
            self.menu = None;
        }
        if let Some(action) = chosen {
            self.run_action(ctx, action);
        }
    }

    fn render_dialog(&mut self, ctx: &egui::Context) {
        let t = ctx.animate_bool_with_time_and_easing(egui::Id::unique("dialog_anim"), self.dialog.is_some(), self.anim_time(0.25), material::quint_out);
        let Some(mut dialog) = self.dialog.take() else { return };

        if let Dialog::Icon { input, error, .. } = &mut dialog {
            if let Some(path) = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf())) {
                *input = path.to_string_lossy().into_owned();
                *error = None;
            }
        }

        // Dimmed backdrop; clicking it dismisses the dialog.
        let screen = ctx.content_rect();
        let mut close = false;
        egui::Area::new(egui::Id::unique("dialog_scrim"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(rect, 0, egui::Color32::from_black_alpha((110.0 * t) as u8));
                close |= resp.clicked();
            });

        let preferred = match &dialog {
            Dialog::Settings { .. } => 720.0,
            Dialog::OpenWith { .. } | Dialog::PickFolder { .. } => 480.0,
            _ => 440.0,
        };
        let width = (screen.width() - 32.0).clamp(240.0, preferred);
        egui::Area::new(egui::Id::unique("dialog"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, (1.0 - t) * 14.0))
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .corner_radius(egui::CornerRadius::same(material::RADIUS_WINDOW))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(20))
                    .show(ui, |ui| {
                        ui.set_width(width - 40.0);
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
                        close |= match &mut dialog {
                            Dialog::Create { .. } => self.create_dialog_contents(ui, &mut dialog),
                            Dialog::Properties { .. } => self.properties_contents(ui, &mut dialog),
                            Dialog::Icon { .. } => self.icon_dialog_contents(ui, &mut dialog),
                            Dialog::OpenWith { .. } => self.open_with_contents(ui, &mut dialog),
                            Dialog::Settings { .. } => self.settings_dialog_contents(ui, &mut dialog),
                            Dialog::PickFolder { .. } => self.pick_folder_contents(ui, &mut dialog),
                            Dialog::Compress { .. } => self.compress_contents(ui, &mut dialog),
                            Dialog::Shortcuts => self.shortcuts_contents(ui),
                        };
                    });
            });

        // A dialog opened from inside this one (e.g. Properties -> Custom icon) replaces it
        if !close && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    /// Returns true when the dialog should close.
    fn create_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Create { is_dir, name, error, first_frame } = dialog else { return false };
        let title = if *is_dir { "New folder" } else { "New file" };
        ui.label(egui::RichText::new(title).size(16.0).color(self.theme.text));
        ui.add_space(4.0);

        let mut submit = false;
        egui::Frame::new()
            .fill(self.theme.surface1)
            .corner_radius(egui::CornerRadius::same(18))
            .inner_margin(egui::Margin::symmetric(14, 8))
            .show(ui, |ui| {
                let output = egui::TextEdit::singleline(name)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::proportional(14.0))
                    .show(ui);
                if *first_frame {
                    output.response.request_focus();
                    let mut state = output.state.clone();
                    let end = egui::text::CCursor::new(name.chars().count());
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), end)));
                    state.store(ui.ctx(), output.response.id);
                    *first_frame = false;
                }
                if output.response.changed() {
                    *error = None;
                }
                if output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
            });

        if let Some(err) = error.as_ref() {
            ui.label(egui::RichText::new(err).size(12.5).color(self.theme.red));
        }

        let mut close = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if chip(ui, "dlg_create", Some(icons::CHECK), "Create", &self.theme, true).clicked() {
                submit = true;
            }
            if chip(ui, "dlg_cancel", None, "Cancel", &self.theme, false).clicked() {
                close = true;
            }
        });

        if submit {
            match self.create_entry(*is_dir, name) {
                Ok(()) => close = true,
                Err(err) => *error = Some(err),
            }
        }
        close
    }

    /// Folder chooser for Copy to / Move to / Extract to. Returns true when the dialog should close.
    fn pick_folder_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::PickFolder { op, items, dir, path_input } = dialog else { return false };
        let theme = self.theme;
        let mut close = false;
        let (title, action) = match op {
            TransferOp::Copy => ("Copy to…", "Copy here"),
            TransferOp::Move => ("Move to…", "Move here"),
            TransferOp::Extract => ("Extract to…", "Extract here"),
        };
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(title).size(16.0).color(theme.text));
                ui.label(egui::RichText::new(Self::describe_count(items)).size(12.5).color(theme.subtext1));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if filled_icon_button(ui, "pf_close", icons::CLOSE, &theme, 30.0, "Close", Some(theme.red)).clicked() {
                    close = true;
                }
            });
        });

        let mut goto: Option<PathBuf> = None;
        ui.horizontal_wrapped(|ui| {
            for (idx, place) in self.places.iter().enumerate() {
                if chip(ui, ("pf_place", idx), Some(place.icon), &place.name, &theme, *dir == place.path).clicked() {
                    goto = Some(place.path.clone());
                }
            }
        });

        egui::Frame::new().fill(theme.surface1).corner_radius(egui::CornerRadius::same(18)).inner_margin(egui::Margin::symmetric(14, 6)).show(ui, |ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(path_input)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .hint_text("Folder path")
                    .font(egui::FontId::proportional(13.5)),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let typed = PathBuf::from(path_input.trim());
                if typed.is_dir() {
                    goto = Some(typed);
                } else {
                    self.status_toast = Some(("That folder does not exist".to_owned(), std::time::Instant::now()));
                }
            }
        });

        let mut folders: Vec<PathBuf> = std::fs::read_dir(&*dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        folders.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
        let max_height = (ui.ctx().content_rect().height() - 320.0).clamp(140.0, 300.0);
        egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).id_salt("pf_scroll").show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if let Some(parent) = dir.parent() {
                if nav_item(ui, "pf_up", icons::ARROW_UP, "..", &theme, false).clicked() {
                    goto = Some(parent.to_path_buf());
                }
            }
            for (idx, folder) in folders.iter().enumerate() {
                let label = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if nav_item(ui, ("pf_folder", idx), icons::FOLDER, &label, &theme, false).clicked() {
                    goto = Some(folder.clone());
                }
            }
            if folders.is_empty() {
                ui.label(egui::RichText::new("No subfolders").size(12.5).color(theme.subtext1));
            }
        });
        if let Some(target) = goto {
            *path_input = target.to_string_lossy().into_owned();
            *dir = target;
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if chip(ui, "pf_go", Some(icons::CHECK), action, &theme, true).clicked() {
                self.transfer(*op, items.clone(), dir.clone());
                close = true;
            }
            if chip(ui, "pf_cancel", None, "Cancel", &theme, false).clicked() {
                close = true;
            }
        });
        close
    }

    /// Returns true when the dialog should close.
    fn compress_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Compress { sources, name, format, first_frame } = dialog else { return false };
        let theme = self.theme;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Compress").size(16.0).color(theme.text));
                ui.label(egui::RichText::new(Self::describe_count(sources)).size(12.5).color(theme.subtext1));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if filled_icon_button(ui, "cz_close", icons::CLOSE, &theme, 30.0, "Close", Some(theme.red)).clicked() {
                    close = true;
                }
            });
        });
        ui.add_space(2.0);

        let mut submit = false;
        egui::Frame::new().fill(theme.surface1).corner_radius(egui::CornerRadius::same(18)).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let output = egui::TextEdit::singleline(name)
                    .frame(egui::Frame::NONE)
                    .desired_width(ui.available_width() - 80.0)
                    .font(egui::FontId::proportional(14.0))
                    .show(ui);
                if *first_frame {
                    output.response.request_focus();
                    *first_frame = false;
                }
                if output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                ui.label(egui::RichText::new(format.extension()).size(14.0).color(theme.subtext1));
            });
        });
        ui.horizontal_wrapped(|ui| {
            for candidate in ArchiveFormat::ALL {
                if !candidate.available() {
                    continue;
                }
                if chip(ui, ("cz_format", candidate.extension()), None, candidate.extension(), &theme, *format == candidate).clicked() {
                    *format = candidate;
                }
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if chip(ui, "cz_go", Some(icons::COMPRESS), "Compress", &theme, true).clicked() {
                submit = true;
            }
            if chip(ui, "cz_cancel", None, "Cancel", &theme, false).clicked() {
                close = true;
            }
        });

        if submit && !name.trim().is_empty() {
            let file_name = format!("{}{}", name.trim().replace('/', "_"), format.extension());
            let out = ops::unique_path(&self.current_path, std::ffi::OsStr::new(&file_name), "copy");
            let (sources, format) = (sources.clone(), *format);
            let shown = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.set_toast(format!("Compressing into {shown}…"));
            self.jobs.spawn(move || {
                ops::compress(&sources, &out, format).map(|_| format!("Created “{shown}”")).map_err(|e| e.to_string())
            });
            close = true;
        }
        close
    }

    fn open_open_with(&mut self, path: PathBuf) {
        let mime = apps::mime_of(&path);
        let default_id = apps::default_for(&mime);
        let installed: Vec<AppEntry> = self.installed_apps().to_vec();
        let current = self.open_overrides.get(&path).cloned().or(default_id.clone());
        let default = current.as_deref().and_then(|id| installed.iter().position(|a| a.id == id));
        let recommended: Vec<usize> = apps::apps_for(&installed, &mime, None)
            .into_iter()
            .filter_map(|a| installed.iter().position(|b| b.id == a.id))
            .filter(|i| Some(*i) != default)
            .collect();
        let others: Vec<usize> = (0..installed.len()).filter(|i| Some(*i) != default && !recommended.contains(i)).collect();
        self.dialog = Some(Dialog::OpenWith {
            path,
            mime,
            apps: installed,
            default,
            recommended,
            others,
            selected: default,
            query: String::new(),
            first_frame: true,
            for_file: false,
            for_type: false,
        });
    }

    /// Returns true when the dialog should close.
    fn open_with_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::OpenWith { path, mime, apps: listed, default, recommended, others, selected, query, first_frame, for_file, for_type } = dialog else { return false };
        let theme = self.theme;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut close = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Open with").size(16.0).color(theme.text));
                ui.label(egui::RichText::new(format!("{name} · {mime}")).size(12.5).color(theme.subtext1));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if filled_icon_button(ui, "ow_close", icons::CLOSE, &theme, 30.0, "Close", Some(theme.red)).clicked() {
                    close = true;
                }
            });
        });
        ui.add_space(2.0);

        // Search over every installed application
        egui::Frame::new().fill(theme.surface1).corner_radius(egui::CornerRadius::same(18)).inner_margin(egui::Margin::symmetric(14, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(icons::SEARCH).font(icons::font(15.0)).color(theme.subtext1));
                let resp = ui.add(
                    egui::TextEdit::singleline(query)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .hint_text("Search applications")
                        .font(egui::FontId::proportional(13.5)),
                );
                if *first_frame {
                    resp.request_focus();
                    *first_frame = false;
                }
            });
        });
        ui.add_space(2.0);

        let needle = query.trim().to_lowercase();
        let matches = |i: &usize| needle.is_empty() || listed[*i].name.to_lowercase().contains(&needle);
        let sections: [(&str, Vec<usize>); 3] = [
            ("Default", default.iter().copied().filter(matches).collect()),
            ("Recommended", recommended.iter().copied().filter(matches).collect()),
            ("Other applications", others.iter().copied().filter(matches).collect()),
        ];
        if sections.iter().all(|(_, rows)| rows.is_empty()) {
            ui.label(egui::RichText::new("No applications found.").size(13.0).color(theme.subtext0));
        }

        let max_height = (ui.ctx().content_rect().height() - 300.0).clamp(160.0, 400.0);
        let mut launch = false;
        // The application list sits in a box of its own
        egui::Frame::new().fill(theme.surface0).corner_radius(egui::CornerRadius::same(material::RADIUS_CONTAINER)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).id_salt("ow_scroll").show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (title, rows) in &sections {
                if rows.is_empty() {
                    continue;
                }
                material::section_label(ui, title, &theme);
                for &idx in rows {
                    let app = &listed[idx];
                    let is_pinned = self.open_overrides.get(path.as_path()) == Some(&app.id);
                    let label = if is_pinned { format!("{}  ·  this file", app.name) } else { app.name.clone() };
                    // Icons are only resolved and decoded for rows that are actually on screen
                    let probe = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 44.0));
                    let tex = if ui.is_rect_visible(probe) {
                        self.app_icons
                            .entry(app.id.clone())
                            .or_insert_with(|| {
                                let image = app.icon.as_deref().and_then(apps::resolve_icon).and_then(|p| apps::load_icon_image(&p, 64))?;
                                Some(ui.ctx().load_texture(format!("app_icon_{}", app.id), image, egui::TextureOptions::LINEAR))
                            })
                            .clone()
                    } else {
                        None
                    };
                    let resp = app_row(ui, ("open_with_app", idx), tex.as_ref(), &label, &theme, *selected == Some(idx));
                    if resp.clicked() {
                        *selected = Some(idx);
                    }
                    if resp.double_clicked() {
                        launch = true;
                    }
                }
            }
        });
        });

        ui.add_space(4.0);
        egui::Frame::new().fill(theme.surface0).corner_radius(egui::CornerRadius::same(22)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if material::switch_row(ui, "ow_file", icons::FILE, "Always for this file", &theme, *for_file).clicked() {
                *for_file = !*for_file;
            }
            if material::switch_row(ui, "ow_type", icons::APPS, "Always for this file type", &theme, *for_type).clicked() {
                *for_type = !*for_type;
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let can_open = selected.is_some();
            if chip(ui, "ow_open", Some(icons::OPEN), "Open", &theme, can_open).clicked() && can_open {
                launch = true;
            }
            if self.open_overrides.contains_key(path.as_path()) && chip(ui, "ow_forget", None, "Forget file choice", &theme, false).clicked() {
                self.open_overrides.remove(path.as_path());
                apps::save_overrides(&self.open_overrides);
                self.set_toast("File choice removed");
                close = true;
            }
            if chip(ui, "ow_cancel", None, "Cancel", &theme, false).clicked() {
                close = true;
            }
        });

        if launch {
            if let Some(app) = selected.and_then(|i| listed.get(i)) {
                match apps::launch(app, path) {
                    Ok(()) => {
                        recent::record(path);
                        if *for_file {
                            self.open_overrides.insert(path.clone(), app.id.clone());
                            apps::save_overrides(&self.open_overrides);
                        }
                        if *for_type {
                            if apps::set_default(mime, &app.id) {
                                self.set_toast(format!("{} is now the default for {mime}", app.name));
                            } else {
                                self.set_toast("Could not change the default application");
                            }
                        }
                        close = true;
                    }
                    Err(err) => self.set_toast(format!("Failed to start {}: {err}", app.name)),
                }
            }
        }
        close
    }

    fn open_icon_dialog(&mut self, folder: PathBuf) {
        let candidates = std::fs::read_dir(&folder)
            .map(|rd| {
                let mut images: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| crate::thumbs::is_image(p)).collect();
                images.sort();
                images.truncate(6);
                images
            })
            .unwrap_or_default();
        let input = self.folder_icons.get(&folder).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        self.dialog = Some(Dialog::Icon { folder, input, error: None, candidates, browse: None });
    }

    /// Set (or with `None` remove) the custom icon of a folder.
    fn apply_folder_icon(&mut self, folder: &Path, image: Option<&Path>) {
        match image {
            Some(img) => {
                self.thumbs.ensure(img);
                self.folder_icons.insert(folder.to_path_buf(), img.to_path_buf());
                self.set_toast("Folder icon set");
            }
            None => {
                self.folder_icons.remove(folder);
                self.set_toast("Folder icon reset");
            }
        }
        save_folder_icons(&self.folder_icons);
    }

    /// Returns true when the dialog should close.
    fn icon_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Icon { folder, input, error, candidates, browse } = dialog else { return false };
        let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Computer".to_owned());
        ui.label(egui::RichText::new("Folder icon").size(16.0).color(self.theme.text));
        ui.label(egui::RichText::new(format!("Choose an image for “{name}”. You can also drop a file here.")).size(12.5).color(self.theme.subtext1));
        ui.add_space(4.0);

        let mut apply = false;
        egui::Frame::new()
            .fill(self.theme.surface1)
            .corner_radius(egui::CornerRadius::same(18))
            .inner_margin(egui::Margin::symmetric(14, 8))
            .show(ui, |ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(input)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .hint_text("No image chosen")
                        .font(egui::FontId::proportional(13.5)),
                );
                if resp.changed() {
                    *error = None;
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    apply = true;
                }
            });

        ui.horizontal(|ui| {
            if chip(ui, "icon_browse", Some(icons::FOLDER), "Choose image…", &self.theme, browse.is_some()).clicked() {
                *browse = match browse {
                    Some(_) => None,
                    None => {
                        let typed = PathBuf::from(input.trim());
                        let start = typed.parent().filter(|p| p.is_dir()).map(Path::to_path_buf);
                        Some(start.unwrap_or_else(|| if folder.is_dir() { folder.clone() } else { self.current_path.clone() }))
                    }
                };
            }
        });
        if let Some(dir) = browse.clone() {
            let mut entries: Vec<(PathBuf, bool)> = std::fs::read_dir(&dir)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                        .map(|e| e.path())
                        .filter_map(|p| if p.is_dir() { Some((p, true)) } else { crate::thumbs::is_image(&p).then_some((p, false)) })
                        .collect()
                })
                .unwrap_or_default();
            entries.sort_by_key(|(p, is_dir)| (!*is_dir, p.file_name().map(|n| n.to_string_lossy().to_lowercase())));
            ui.label(egui::RichText::new(dir.to_string_lossy()).size(11.5).color(self.theme.subtext1));
            egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).id_salt("icon_browser").show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if let Some(parent) = dir.parent() {
                    if nav_item(ui, "icon_browse_up", icons::ARROW_UP, "..", &self.theme, false).clicked() {
                        *browse = Some(parent.to_path_buf());
                    }
                }
                for (idx, (path, is_dir)) in entries.iter().enumerate() {
                    let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let glyph = if *is_dir { icons::FOLDER } else { icons::IMAGE };
                    let on = !*is_dir && *input == path.to_string_lossy();
                    if nav_item(ui, ("icon_browse_entry", idx), glyph, &label, &self.theme, on).clicked() {
                        if *is_dir {
                            *browse = Some(path.clone());
                        } else {
                            *input = path.to_string_lossy().into_owned();
                            *error = None;
                            *browse = None;
                        }
                    }
                }
            });
        } else if !candidates.is_empty() {
            material::section_label(ui, "Images in this folder", &self.theme);
            ui.horizontal_wrapped(|ui| {
                for (idx, path) in candidates.iter().enumerate() {
                    let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let on = *input == path.to_string_lossy();
                    if chip(ui, ("icon_candidate", idx), None, &label, &self.theme, on).clicked() {
                        *input = path.to_string_lossy().into_owned();
                        *error = None;
                    }
                }
            });
        }
        if let Some(err) = error.as_ref() {
            ui.label(egui::RichText::new(err).size(12.5).color(self.theme.red));
        }

        let mut close = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if chip(ui, "icon_apply", Some(icons::CHECK), "Apply", &self.theme, true).clicked() {
                apply = true;
            }
            if self.folder_icons.contains_key(folder.as_path())
                && chip(ui, "icon_reset", None, "Reset", &self.theme, false).clicked()
            {
                self.apply_folder_icon(folder, None);
                close = true;
            }
            if chip(ui, "icon_cancel", None, "Cancel", &self.theme, false).clicked() {
                close = true;
            }
        });

        if apply {
            let path = PathBuf::from(input.trim().trim_matches('\''));
            if path.is_file() && crate::thumbs::is_image(&path) {
                self.apply_folder_icon(folder, Some(&path));
                close = true;
            } else {
                *error = Some("Pick an existing png, jpg, gif, webp or bmp image".to_owned());
            }
        }
        close
    }

    /// Nautilus-style properties: a centered header, then grouped cards of label / value rows.
    /// Returns true when the dialog should close.
    fn properties_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Properties { props, scan } = dialog else { return false };
        let ctx = ui.ctx().clone();
        let item = &props.item;
        let theme = self.theme.clone();
        let mut groups = props.groups();

        // Folder totals come from the background scan
        let mut subtitle = props.kind.clone();
        if let Some(scan) = scan.as_ref() {
            use std::sync::atomic::Ordering::Relaxed;
            let s = &scan.state;
            let (size, files, folders, done) = (s.size.load(Relaxed), s.files.load(Relaxed), s.folders.load(Relaxed), s.done.load(Relaxed));
            let total = format!("{}{}", crate::fs::file_item::format_bytes(size), if done { "" } else { " so far…" });
            groups[0].1.push(("Total size", format!("{total} · {files} files, {folders} folders")));
            subtitle = format!("{} · {total}", props.kind);
            if !done {
                ctx.request_repaint();
            }
        } else if !item.is_dir {
            subtitle = format!("{} · {}", props.kind, item.formatted_size());
        }

        let mut copied: Option<String> = None;
        let mut change_icon = false;
        let mut close = false;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Properties").size(16.0).color(theme.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if filled_icon_button(ui, "dlg_close", icons::CLOSE, &theme, 30.0, "Close", Some(theme.red)).clicked() {
                    close = true;
                }
            });
        });
        ui.add_space(2.0);

        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 150.0).max(160.0))
            .auto_shrink([true, true])
            .show(ui, |ui| {
                let full = ui.available_width();
                ui.spacing_mut().item_spacing.y = 4.0;

                // Header: icon tile, name, one-line summary
                ui.vertical_centered(|ui| {
                    let (tile, _) = ui.allocate_exact_size(egui::vec2(88.0, 88.0), egui::Sense::hover());
                    ui.painter().rect_filled(tile, egui::CornerRadius::same(26), theme.surface1);
                    match thumb_for(&self.thumbs, &self.folder_icons, item, self.settings.show_thumbnails) {
                        Some(tex) => {
                            egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), egui::vec2(64.0, 64.0)))
                                .corner_radius(egui::CornerRadius::same(14))
                                .paint_at(ui, egui::Rect::from_center_size(tile.center(), egui::vec2(64.0, 64.0)));
                        }
                        None => {
                            let color = if item.is_dir { theme.blue } else { theme.subtext0 };
                            let glyph = if item.is_symlink { icons::SYMLINK } else { item.file_type.icon() };
                            material::paint_icon(ui.painter(), tile.center(), glyph, 44.0, color);
                        }
                    }
                    ui.add_space(6.0);
                    let mut job = egui::text::LayoutJob::simple_singleline(item.name.clone(), egui::FontId::proportional(17.0), theme.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(full - 16.0);
                    ui.label(job);
                    ui.label(egui::RichText::new(subtitle).size(12.5).color(theme.subtext1));
                    if item.is_dir {
                        ui.add_space(4.0);
                        if chip(ui, "props_icon", Some(icons::IMAGE), "Custom icon…", &theme, false).clicked() {
                            change_icon = true;
                        }
                    }
                });
                ui.add_space(8.0);

                for (title, rows) in &groups {
                    material::section_label(ui, title, &theme);
                    egui::Frame::new().fill(theme.surface1).corner_radius(egui::CornerRadius::same(material::RADIUS_CONTAINER)).show(ui, |ui| {
                        ui.set_width(full);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (idx, (label, value)) in rows.iter().enumerate() {
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(full, 38.0), egui::Sense::click());
                            let fx = material::fx(ui, resp.id, &resp, true, &material::FxSpec::ROW);
                            let painter = ui.painter();
                            let inner = rect.shrink2(egui::vec2(4.0, 2.0));
                            painter.rect_filled(inner, egui::CornerRadius::same(16), theme.with_alpha(theme.text, (fx.hover * 16.0) as u8));
                            if idx > 0 {
                                let line = egui::Rect::from_min_size(egui::pos2(rect.left() + 16.0, rect.top()), egui::vec2(full - 32.0, 1.0));
                                painter.rect_filled(line, 0, theme.with_alpha(theme.text, 14));
                            }
                            painter.text(egui::pos2(rect.left() + 16.0, rect.center().y), egui::Align2::LEFT_CENTER, *label, egui::FontId::proportional(12.5), theme.subtext1);
                            let mut job = egui::text::LayoutJob::simple_singleline(value.clone(), egui::FontId::proportional(13.5), theme.text);
                            job.wrap = egui::text::TextWrapping::truncate_at_width((full - 140.0 - 36.0).max(20.0));
                            let galley = painter.layout_job(job);
                            painter.galley(egui::pos2(rect.left() + 118.0, rect.center().y - galley.size().y / 2.0), galley, theme.text);
                            material::paint_icon(painter, egui::pos2(rect.right() - 22.0, rect.center().y), icons::COPY, 14.0, theme.with_alpha(theme.subtext1, (fx.hover * 255.0) as u8));
                            if resp.on_hover_text("Click to copy").clicked() {
                                copied = Some(value.clone());
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
            });

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if chip(ui, "dlg_copy_path", Some(icons::COPY), "Copy path", &theme, false).clicked() {
                copied = Some(props.item.path.to_string_lossy().into_owned());
            }
        });

        let folder = props.item.path.clone();
        if let Some(text) = copied {
            ctx.copy_text(text);
            self.set_toast("Copied to clipboard");
        }
        if change_icon {
            self.open_icon_dialog(folder);
            return false;
        }
        close
    }
}

/// Thumbnail for a row: the image itself, or a folder's custom icon.
fn thumb_for<'a>(
    thumbs: &'a Thumbnails,
    icons: &HashMap<PathBuf, PathBuf>,
    item: &FileItem,
    show_thumbnails: bool,
) -> Option<&'a egui::TextureHandle> {
    if item.is_dir {
        icons.get(&item.path).and_then(|img| thumbs.get(img))
    } else if show_thumbnails {
        thumbs.get(&item.path)
    } else {
        None
    }
}

/// "Open with" list row: the application's own icon (or a generic glyph) and its name.
pub(crate) fn app_row(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: Option<&egui::TextureHandle>,
    label: &str,
    theme: &SerpantinumTheme,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = material::fx(ui, id, &resp, true, &material::FxSpec::ROW);
    let sel = ui.ctx().animate_bool_with_time_and_easing(id.with("sel"), selected, 0.25, material::quint_out);
    let bg = material::lerp_color(theme.with_alpha(theme.text, (fx.hover * 16.0) as u8), theme.with_alpha(theme.mauve, 52), sel);
    ui.painter().rect_filled(rect, egui::CornerRadius::same(18), bg);

    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), egui::vec2(28.0, 28.0));
    match icon {
        Some(tex) => {
            egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), icon_rect.size())).paint_at(ui, icon_rect);
        }
        None => material::paint_icon(ui.painter(), icon_rect.center(), icons::EXECUTABLE, 22.0, theme.subtext0),
    }
    let color = material::lerp_color(theme.text, theme.mauve, sel);
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), egui::FontId::proportional(13.5), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 64.0).max(20.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(rect.left() + 48.0, rect.center().y - galley.size().y / 2.0), galley, color);
    resp
}

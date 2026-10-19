use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use eframe::egui;

use crate::fs::{
    file_item::FileItem,
    apps::{self, AppEntry},
    places::{load_folder_icons, save_folder_icons, Place, PlaceGroup, PlaceKind, RECENT_PATH},
    props::{FolderScan, Properties},
    scanner::{read_directory, ListOptions, TypeFilter},
    recent,
    search::Search,
    trash,
};
use crate::fs::ops::{self, ArchiveFormat, Jobs};
use crate::fs::scanner::{SortBy, SortOrder};
use crate::settings::Settings;
use crate::thumbs::Thumbnails;
use crate::ui::{
    icons,
    material::{self, chip, segmented, file_row, file_tile, icon_button, menu_item, nav_item, ButtonKind, Column, FooterSpec, CONTROL_HEIGHT},
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
    /// The Trash entry of the sidebar.
    TrashPlace(PathBuf),
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
    Rename(PathBuf),
    Trash(Vec<PathBuf>),
    DeleteForever(Vec<PathBuf>),
    Restore(Vec<PathBuf>),
    EmptyTrash,
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

/// What a confirmation dialog does once accepted.
pub(crate) enum ConfirmAction {
    DeleteForever(Vec<PathBuf>),
    EmptyTrash,
}

/// Action offered by a toast, such as undoing a move to the trash.
enum ToastAction {
    UndoTrash(Vec<PathBuf>),
}

struct Toast {
    message: String,
    since: std::time::Instant,
    action: Option<ToastAction>,
    /// Fading out; removed once the fade finished.
    leaving: bool,
}

/// A context menu that was just closed, kept on screen while it fades away.
struct MenuGhost {
    origin: egui::Pos2,
    groups: Vec<Vec<(&'static str, &'static str, MenuAction)>>,
    open_label: Option<String>,
}

pub(crate) enum Dialog {
    Create { is_dir: bool, name: String, error: Option<String>, first_frame: bool },
    Rename { path: PathBuf, name: String, error: Option<String>, first_frame: bool },
    Confirm { title: String, body: String, confirm: String, action: ConfirmAction },
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
    pub(crate) theme_dirty: bool,
    pub(crate) fonts_dirty: bool,
    pins: Vec<PathBuf>,
    menu: Option<ContextMenu>,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) closing_dialog: Option<Dialog>,
    /// Set when a menu click ran this frame, so it is not also read as a click outside a dialog it just opened.
    menu_click_consumed: bool,

    menu_ghost: Option<MenuGhost>,
    window_title: String,

    status_toast: Option<Toast>,

    // Flag to scroll to selected item on next frame
    scroll_to_selected: bool,

    pub(crate) capturing_key: bool,
}

impl SkiApp {
    pub fn new(initial_path: PathBuf, theme: SerpantinumTheme, settings: Settings, ctx: &egui::Context) -> Self {
        apps::preload();
        crate::ui::fonts::preload();
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
            theme_dirty: false,
            fonts_dirty: false,
            pins: Place::load_pins(),
            menu: None,
            dialog: None,
            closing_dialog: None,
            menu_click_consumed: false,
            menu_ghost: None,
            window_title: String::new(),
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

        let canonical = if target.as_os_str() == RECENT_PATH { target } else { target.canonicalize().unwrap_or(target) };
        if canonical.as_os_str() == RECENT_PATH || canonical.is_dir() {
            self.back_history.push(self.current_path.clone());
            self.forward_history.clear();
            if canonical.as_os_str() != RECENT_PATH {
                recent::record(&canonical);
            }
            self.arrive(canonical);
        } else {
            self.open_file(&canonical);
        }
    }

    /// Switch to `path` after the history was updated: reset everything that belonged to the old place.
    fn arrive(&mut self, path: PathBuf) {
        self.current_path = path;
        if !self.in_recent() {
            self.settings.last_dir = Some(self.current_path.clone());
            if self.settings.remember_view_per_folder
                && let Some(&saved_mode) = self.settings.folder_views.get(&self.current_path) {
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
    }

    /// Showing the Recent view rather than a folder.
    pub fn in_recent(&self) -> bool {
        self.current_path.as_os_str() == RECENT_PATH
    }

    fn in_trash(&self) -> bool {
        trash::contains(&self.current_path)
    }

    /// The listing mixes items from many folders (search results, Recent).
    fn is_mixed_listing(&self) -> bool {
        self.in_recent() || !self.search_query.trim().is_empty()
    }

    /// Whether new files can be created or pasted into what is shown.
    fn can_modify_here(&self) -> bool {
        !self.is_mixed_listing() && !self.in_trash()
    }

    pub fn go_back(&mut self) {
        if let Some(prev) = self.back_history.pop() {
            self.forward_history.push(self.current_path.clone());
            self.arrive(prev);
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(next) = self.forward_history.pop() {
            self.back_history.push(self.current_path.clone());
            self.arrive(next);
        }
    }

    pub fn go_up(&mut self) {
        if self.in_recent() {
            self.go_back();
        } else if trash::is_trash_folder(&self.current_path) {
            if let Some(home) = dirs::home_dir() {
                self.navigate_to(home);
            }
        } else if let Some(parent) = self.current_path.parent() {
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
        let listing = if self.in_recent() { Ok(self.recent_items()) } else { read_directory(&self.current_path, &opts) };
        match listing {
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
                } else if let Some(idx) = self.selected_index
                    && idx >= self.items.len() {
                        self.selected_index = Some(self.items.len() - 1);
                    }
            }
            Err(err) => {
                self.items.clear();
                self.selected_index = None;
                self.error_message = Some(err.to_string());
            }
        }
        self.sync_selection(true);
    }

    /// The files and folders used lately, most recent first.
    fn recent_items(&self) -> Vec<FileItem> {
        const LIMIT: usize = 150;
        recent::load()
            .into_iter()
            .filter(|p| self.show_hidden || !p.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')))
            .filter_map(|p| FileItem::from_path(&p).filter(|_| p.symlink_metadata().is_ok()))
            .take(LIMIT)
            .collect()
    }

    /// Bring the listing up to date after files changed under it.
    fn refresh_listing(&mut self) {
        if self.is_mixed_listing() {
            self.items.retain(|i| i.path.symlink_metadata().is_ok());
            self.selected_index = None;
            self.sync_selection(true);
        } else {
            self.reload_items();
        }
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
        if !self.can_modify_here() {
            self.set_toast("Open a regular folder to paste into");
            return;
        }
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
        material::set_animations(self.settings.animations);
        ctx.all_styles_mut(|s| s.animation_time = material::dur(material::ANIM));
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

    fn is_act_pressed(&self, ctx: &egui::Context, act: crate::keymap::Action) -> bool {
        crate::keymap::is_action_pressed(ctx, &self.settings.keybinds, act, self.settings.vim_keys)
    }

    pub(crate) fn installed_apps(&self) -> &'static [AppEntry] {
        apps::installed()
    }

    pub(crate) fn set_toast(&mut self, message: impl Into<String>) {
        self.status_toast = Some(Toast { message: message.into(), since: std::time::Instant::now(), action: None, leaving: false });
    }

    fn set_toast_action(&mut self, message: impl Into<String>, action: ToastAction) {
        self.status_toast = Some(Toast { message: message.into(), since: std::time::Instant::now(), action: Some(action), leaving: false });
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

    /// Files dropped on the window are copied into the folder being shown.
    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if dropped.is_empty() {
            return;
        }
        if self.can_modify_here() {
            let dest = self.current_path.clone();
            self.transfer(TransferOp::Copy, dropped, dest);
        } else {
            self.set_toast("Open a regular folder to drop files into");
        }
    }

    /// Name of the place being shown, for the window title.
    fn location_name(&self) -> String {
        if !self.search_query.trim().is_empty() {
            "Search".to_owned()
        } else if self.in_recent() {
            "Recent".to_owned()
        } else if trash::is_trash_folder(&self.current_path) {
            "Trash".to_owned()
        } else {
            self.current_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Computer".to_owned())
        }
    }

    fn open_selected(&mut self) {
        if let Some(idx) = self.selected_index
            && let Some(item) = self.items.get(idx).cloned() {
                self.open_item(item);
            }
    }

    fn handle_keyboard_inputs(&mut self, ctx: &egui::Context) {
        if self.capturing_key {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape))
            && (self.dialog.is_some() || self.menu.take().is_some()) {
                if let Some(d) = self.dialog.take() {
                    self.closing_dialog = Some(d);
                }
                return;
            }
        if self.dialog.is_some() {
            return;
        }

        // The extra mouse buttons walk through the history, like in every other file manager
        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) {
            self.go_back();
        }
        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra2)) {
            self.go_forward();
        }
        self.handle_drops(ctx);

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
            if !self.selection.is_empty() {
                self.selection.clear();
                self.selected_index = None;
                self.anchor = None;
                self.last_cursor = None;
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
            if self.in_recent() {
                self.set_toast("Open a folder to pin it");
            } else {
                self.toggle_pin(self.current_path.clone());
            }
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

        if self.is_act_pressed(ctx, crate::keymap::Action::Rename) {
            if let [one] = self.selected_paths().as_slice() {
                self.open_rename(one.clone());
            }
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::DeleteForever) {
            self.request_delete_forever(self.selected_paths());
            handled = true;
        } else if self.is_act_pressed(ctx, crate::keymap::Action::Trash) {
            if self.in_trash() {
                self.request_delete_forever(self.selected_paths());
            } else {
                self.trash_paths(self.selected_paths());
            }
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::NewFile) {
            self.open_create_dialog(false);
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::GoHome) {
            if let Some(home) = dirs::home_dir() {
                self.navigate_to(home);
            }
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::Quit) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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

/// One step of the breadcrumb path.
struct Crumb {
    icon: Option<&'static str>,
    label: String,
    path: PathBuf,
}

/// Width of context menus; wide enough for a label next to its shortcut.
const MENU_WIDTH: f32 = 264.0;

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

        let title = format!("{} — Ski", self.location_name());
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }

        let busy = self.jobs.running();
        if let Some(toast) = &mut self.status_toast {
            let lifetime = if toast.action.is_some() { 7.0 } else { 3.0 };
            if !toast.leaving && !busy && toast.since.elapsed().as_secs_f32() > lifetime {
                toast.leaving = true;
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

        if self.settings.show_status_bar {
            egui::Panel::bottom("status_bar")
                .exact_size(30.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(self.theme.base).inner_margin(egui::Margin::symmetric(16, 0)))
                .show(ui, |ui| self.render_status_bar(ui));
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(self.theme.base)
                    .inner_margin(egui::Margin { left: 6, right: 6, top: 0, bottom: 6 }),
            )
            .show(ui, |ui| {
                let background = ui.interact(ui.max_rect(), ui.make_persistent_id("list_background"), egui::Sense::click());
                if background.secondary_clicked() && self.search_query.trim().is_empty() && !self.in_recent()
                    && let Some(pos) = background.interact_pointer_pos() {
                        self.open_menu(ui.ctx(), pos, MenuTarget::Folder);
                    }
                if background.clicked() && !self.selection.is_empty() {
                    self.selection.clear();
                    self.selected_index = None;
                    self.anchor = None;
                    self.last_cursor = None;
                }

                if let Some(err) = &self.error_message {
                    material::empty_state(ui, &self.theme, icons::INFO, "This folder can't be opened", Some(err));
                } else if self.items.is_empty() {
                    let (icon, title, hint) = if !self.search_query.trim().is_empty() {
                        if self.search.is_running() {
                            (icons::SEARCH, "Searching…", None)
                        } else {
                            (icons::SEARCH_OFF, "No results", Some("Try a different name"))
                        }
                    } else if self.in_recent() {
                        (icons::RECENT, "Nothing recent yet", Some("Files you open show up here"))
                    } else if self.in_trash() {
                        (icons::TRASH, "The trash is empty", Some("Deleted files end up here"))
                    } else if self.type_filter != TypeFilter::All {
                        (icons::SEARCH_OFF, "Nothing matches", Some("Try another file type in the view options"))
                    } else {
                        (icons::FOLDER, "This folder is empty", Some("Right-click to create a file or folder"))
                    };
                    material::empty_state(ui, &self.theme, icon, title, hint);
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
        self.render_drop_overlay(&ctx);
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
                let right_buttons = if sidebar_visible { 1.0 } else { 3.0 };
                let reserved = right_buttons * (CONTROL_HEIGHT + gap);
                let title_width = (ui.available_width() - reserved - gap).max(40.0);

                let (title_rect, title_resp) =
                    ui.allocate_exact_size(egui::vec2(title_width, CONTROL_HEIGHT), egui::Sense::click());

                if self.show_search && !sidebar_visible {
                    ui.painter().rect_filled(title_rect, material::pill(title_rect), self.theme.surface1);
                    self.render_search_field(ui, title_rect);
                } else if self.is_editing_path {
                    ui.painter().rect_filled(title_rect, material::pill(title_rect), self.theme.surface1);
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

    fn render_title(&mut self, ui: &mut egui::Ui, rect: egui::Rect, field: &egui::Response) {
        let theme = self.theme;
        let cy = rect.center().y;
        let hover = material::animate(ui.ctx(), field.id.with("hover"), field.hovered());
        ui.painter().rect_filled(rect, material::pill(rect), material::lerp_color(theme.surface0, theme.surface1, hover));

        let searching = !self.search_query.trim().is_empty();
        if searching || self.in_recent() {
            let (icon, text) = if searching {
                (icons::SEARCH, format!("Results for “{}”", self.search_query.trim()))
            } else {
                (icons::RECENT, "Recent".to_owned())
            };
            material::paint_icon(ui.painter(), egui::pos2(rect.left() + 20.0, cy), icon, material::ICON_MD, theme.subtext1);
            let mut job = egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(material::FONT_SUBTITLE), theme.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 52.0).max(10.0));
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(egui::pos2(rect.left() + 38.0, cy - galley.size().y / 2.0), galley, theme.text);
            return;
        }

        // Breadcrumbs: every folder on the way is a button; leading ones fold into an ellipsis when short on room
        let mut crumbs = self.crumbs();
        let font = egui::FontId::proportional(material::FONT_SUBTITLE);
        let measure = |ui: &egui::Ui, c: &Crumb| {
            let text_w = ui.painter().layout_no_wrap(c.label.clone(), font.clone(), theme.text).size().x;
            text_w + if c.icon.is_some() { 26.0 } else { 0.0 } + 20.0
        };
        let mut widths: Vec<f32> = crumbs.iter().map(|c| measure(ui, c)).collect();
        let (sep, avail) = (20.0, rect.width() - 16.0);
        let total = |w: &[f32], folded: bool| w.iter().sum::<f32>() + sep * w.len().saturating_sub(1) as f32 + if folded { 32.0 + sep } else { 0.0 };
        let mut folded_to: Option<std::path::PathBuf> = None;
        while crumbs.len() > 1 && total(&widths, folded_to.is_some()) > avail {
            let gone = crumbs.remove(0);
            widths.remove(0);
            folded_to = Some(gone.path);
        }
        if let Some(last) = widths.last_mut() {
            *last = last.min(avail - if folded_to.is_some() { 32.0 + sep } else { 0.0 }).max(24.0);
        }

        let mut x = rect.left() + 8.0;
        let mut go: Option<std::path::PathBuf> = None;
        let mut edit = false;
        let crumb_button = |ui: &mut egui::Ui, id: egui::Id, cell: egui::Rect, current: bool| {
            let resp = ui.interact(cell, id, egui::Sense::click());
            let h = material::animate(ui.ctx(), id.with("h"), resp.hovered());
            ui.painter().rect_filled(cell, material::pill(cell), theme.with_alpha(theme.text, (h * material::ALPHA_HOVER as f32) as u8));
            (resp, material::lerp_color(if current { theme.text } else { theme.subtext1 }, theme.text, h))
        };
        let chevron = |ui: &egui::Ui, x: f32| {
            material::paint_icon(ui.painter(), egui::pos2(x + sep / 2.0, cy), icons::CHEVRON_RIGHT, material::ICON_XS, theme.with_alpha(theme.subtext1, 140));
        };

        if let Some(target) = folded_to {
            let cell = egui::Rect::from_min_size(egui::pos2(x, cy - 13.0), egui::vec2(32.0, 26.0));
            let (resp, color) = crumb_button(ui, ui.make_persistent_id("crumb_fold"), cell, false);
            ui.painter().text(cell.center(), egui::Align2::CENTER_CENTER, "…", font.clone(), color);
            if resp.clicked() {
                go = Some(target);
            }
            x += 32.0;
            chevron(ui, x);
            x += sep;
        }
        let last = crumbs.len().saturating_sub(1);
        for (i, (crumb, w)) in crumbs.iter().zip(&widths).enumerate() {
            let cell = egui::Rect::from_min_size(egui::pos2(x, cy - 13.0), egui::vec2(*w, 26.0));
            let (resp, color) = crumb_button(ui, ui.make_persistent_id(("crumb", i)), cell, i == last);
            let mut tx = cell.left() + 10.0;
            if let Some(icon) = crumb.icon {
                material::paint_icon(ui.painter(), egui::pos2(tx + 8.0, cy), icon, material::ICON_MD, color);
                tx += 26.0;
            }
            let mut job = egui::text::LayoutJob::simple_singleline(crumb.label.clone(), font.clone(), color);
            job.wrap = egui::text::TextWrapping::truncate_at_width((cell.right() - 10.0 - tx).max(10.0));
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(egui::pos2(tx, cy - galley.size().y / 2.0), galley, color);
            if resp.clicked() {
                if i == last { edit = true } else { go = Some(crumb.path.clone()) }
            }
            x += w;
            if i != last {
                chevron(ui, x);
                x += sep;
            }
        }

        if let Some(path) = go {
            self.navigate_to(path);
        } else if edit || field.clicked() {
            self.is_editing_path = true;
            self.path_input = self.current_path.to_string_lossy().to_string();
        }
    }

    /// The folders leading to the current one, starting at Home, the trash or the root.
    fn crumbs(&self) -> Vec<Crumb> {
        let canon = |p: PathBuf| p.canonicalize().unwrap_or(p);
        let path = &self.current_path;
        let trash_root = trash::files_dir().map(canon);
        let home = dirs::home_dir().map(canon);
        let (root, icon, name) = if let Some(t) = trash_root.filter(|t| path.starts_with(t)) {
            (t, icons::TRASH, "Trash")
        } else if let Some(h) = home.filter(|h| path.starts_with(h)) {
            (h, icons::HOME, "Home")
        } else {
            (PathBuf::from("/"), icons::STORAGE, "Computer")
        };
        let mut crumbs = vec![Crumb { icon: Some(icon), label: name.to_owned(), path: root.clone() }];
        let mut acc = root.clone();
        if let Ok(rel) = path.strip_prefix(&root) {
            for part in rel.components() {
                acc.push(part);
                crumbs.push(Crumb { icon: None, label: part.as_os_str().to_string_lossy().into_owned(), path: acc.clone() });
            }
        }
        crumbs
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        self.render_sidebar_search(ui);
        ui.add_space(8.0);

        let browsing = self.search_query.trim().is_empty();
        let mut destination: Option<PathBuf> = None;
        let mut menu_at: Option<(egui::Pos2, MenuTarget)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut first = true;
            // Sections: Main, user folders, pinned folders, devices; a divider sits between them
            for section in 0..4 {
                let places: Vec<(usize, &Place)> = match section {
                    0 => self.places.iter().enumerate().filter(|(_, p)| p.group == PlaceGroup::Main).collect(),
                    1 => self.places.iter().enumerate().filter(|(_, p)| p.group == PlaceGroup::Folders).collect(),
                    3 => self.places.iter().enumerate().filter(|(_, p)| p.group == PlaceGroup::Devices).collect(),
                    _ => Vec::new(),
                };
                if section == 2 && self.pins.is_empty() || section != 2 && places.is_empty() {
                    continue;
                }
                if !first {
                    material::divider(ui, &self.theme);
                }
                first = false;

                if section == 2 {
                    for (idx, pin) in self.pins.iter().enumerate() {
                        let selected = browsing && self.current_path == *pin;
                        let name = pin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| pin.display().to_string());
                        let resp = nav_item(ui, ("pin", idx), icons::FOLDER, &name, &self.theme, selected);
                        if resp.clicked() {
                            destination = Some(pin.clone());
                        }
                        if resp.secondary_clicked()
                            && let Some(pos) = resp.interact_pointer_pos() {
                                menu_at = Some((pos, MenuTarget::Pin(pin.clone())));
                            }
                    }
                    continue;
                }
                for (idx, place) in places {
                    let here = match place.kind {
                        PlaceKind::Recent => self.in_recent(),
                        PlaceKind::Folder => self.current_path == place.path,
                    };
                    let resp = nav_item(ui, ("place", idx), place.icon, &place.name, &self.theme, browsing && here);
                    if resp.clicked() {
                        destination = Some(place.path.clone());
                    }
                    if resp.secondary_clicked()
                        && place.kind == PlaceKind::Folder
                        && trash::is_trash_folder(&place.path)
                        && let Some(pos) = resp.interact_pointer_pos() {
                            menu_at = Some((pos, MenuTarget::TrashPlace(place.path.clone())));
                        }
                }
            }
        });

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
        let open_t = material::animate_for(ui.ctx(), egui::Id::unique("sidebar_search_open"), open, material::ANIM_SLOW);

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
        ui.painter().rect_filled(rect, material::pill(rect), fill);
        material::paint_icon(ui.painter(), egui::pos2(rect.left() + 17.0, rect.center().y), icons::SEARCH, material::ICON_SM, self.theme.subtext1);

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
        let hover = material::animate(ui.ctx(), clear.id.with("h"), clear.hovered());
        ui.painter().rect_filled(clear_rect, material::pill(clear_rect), self.theme.with_alpha(self.theme.text, (hover * material::ALPHA_PRESS as f32) as u8));
        material::paint_icon(ui.painter(), clear_rect.center(), icons::CLOSE, material::ICON_XS, material::lerp_color(self.theme.subtext1, self.theme.text, hover));
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

    fn path_hint(&self, item: &FileItem) -> Option<String> {
        if !self.is_mixed_listing() {
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

        if !self.is_mixed_listing()
            && let Some(key) = material::list_header(ui, &self.theme, &self.columns, self.sort_by, self.sort_order) {
                if key == self.sort_by {
                    self.sort_order = if self.sort_order == SortOrder::Ascending { SortOrder::Descending } else { SortOrder::Ascending };
                } else {
                    self.sort_by = key;
                    self.sort_order = if key == SortBy::Name { SortOrder::Ascending } else { SortOrder::Descending };
                }
                self.reload_items();
            }

        let cut: &[PathBuf] = match &self.clipboard {
            Some((paths, true)) => paths,
            _ => &[],
        };
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut menu_at: Option<(egui::Pos2, MenuTarget)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (idx, item) in self.items.iter().enumerate() {
                let selected = self.selection.contains(&idx);
                let hint = self.path_hint(item);
                let thumb = thumb_for(&self.thumbs, &self.folder_icons, item, show_thumbs);
                let draw = |ui: &mut egui::Ui| {
                    file_row(ui, idx, item, &self.theme, selected, should_scroll, thumb, hint.as_deref(), &self.columns, row_height)
                };
                // Items waiting to be moved are shown faded
                let resp = if cut.contains(&item.path) {
                    ui.scope(|ui| {
                        ui.set_opacity(0.5);
                        draw(ui)
                    })
                    .inner
                } else {
                    draw(ui)
                };
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
                if resp.secondary_clicked()
                    && let Some(pos) = resp.interact_pointer_pos() {
                        menu_at = Some((pos, MenuTarget::Item(item.clone())));
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

        let cut: &[PathBuf] = match &self.clipboard {
            Some((paths, true)) => paths,
            _ => &[],
        };
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut menu_at: Option<(egui::Pos2, MenuTarget)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let spacing = 4.0;
            let cols = ((ui.available_width() + spacing) / (cell.x + spacing)).floor().max(1.0) as usize;
            // Leftover width is shared between the columns, so the grid fills the view like Nautilus does
            let gap = ((ui.available_width() - cols as f32 * cell.x) / cols as f32).max(spacing);
            ui.spacing_mut().item_spacing = egui::vec2(gap, spacing);

            for (row_idx, row) in self.items.chunks(cols).enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(gap / 2.0 - spacing.min(gap / 2.0));
                    for (col_idx, item) in row.iter().enumerate() {
                        let idx = row_idx * cols + col_idx;
                        let selected = self.selection.contains(&idx);
                        let thumb = thumb_for(&self.thumbs, &self.folder_icons, item, show_thumbs);
                        let draw = |ui: &mut egui::Ui| file_tile(ui, idx, item, &self.theme, selected, should_scroll, thumb, grid_size, grid_lines);
                        let resp = if cut.contains(&item.path) {
                            ui.scope(|ui| {
                                ui.set_opacity(0.5);
                                draw(ui)
                            })
                            .inner
                        } else {
                            draw(ui)
                        };
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
                        if resp.secondary_clicked()
                            && let Some(pos) = resp.interact_pointer_pos() {
                                menu_at = Some((pos, MenuTarget::Item(item.clone())));
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

    /// One quiet line under the listing: what is shown or selected, and what waits in the clipboard.
    fn render_status_bar(&self, ui: &mut egui::Ui) {
        let theme = self.theme;
        let rect = ui.max_rect();
        let cy = rect.center().y;
        let font = egui::FontId::proportional(material::FONT_CAPTION);

        let total = self.items.len();
        let text = if self.selection.is_empty() {
            let noun = if self.is_mixed_listing() && !self.in_recent() { ("result", "results") } else { ("item", "items") };
            let more = if self.search.is_running() { "…" } else { "" };
            format!("{total} {}{more}", if total == 1 { noun.0 } else { noun.1 })
        } else {
            let selected: Vec<&FileItem> = self.selection.iter().filter_map(|i| self.items.get(*i)).collect();
            let size: u64 = selected.iter().filter(|i| !i.is_dir).map(|i| i.size).sum();
            let mut text = format!("{} of {total} selected", selected.len());
            if selected.iter().any(|i| !i.is_dir) {
                text.push_str(&format!(" · {}", crate::fs::file_item::format_bytes(size)));
            }
            text
        };
        ui.painter().text(egui::pos2(rect.left(), cy), egui::Align2::LEFT_CENTER, text, font.clone(), theme.subtext1);

        if let Some((paths, cut)) = &self.clipboard {
            let label = format!("{} {}", paths.len(), if *cut { "ready to move" } else { "ready to copy" });
            let galley = ui.painter().layout_no_wrap(label, font, theme.subtext1);
            let x = rect.right() - galley.size().x;
            ui.painter().galley(egui::pos2(x, cy - galley.size().y / 2.0), galley, theme.subtext1);
            material::paint_icon(ui.painter(), egui::pos2(x - 14.0, cy), if *cut { icons::CUT } else { icons::COPY }, material::ICON_XS, theme.subtext1);
        }
    }

    /// While files are dragged over the window: a tint and a hint about where they will land.
    fn render_drop_overlay(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) || self.dialog.is_some() {
            return;
        }
        let theme = self.theme;
        let screen = ctx.content_rect();
        let target = if self.can_modify_here() { format!("Drop to copy into “{}”", self.location_name()) } else { "Open a regular folder to drop files".to_owned() };
        egui::Area::new(egui::Id::unique("drop_overlay")).order(egui::Order::Foreground).fixed_pos(screen.min).interactable(false).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0, theme.with_alpha(theme.mauve, 28));
            let galley = ui.painter().layout_no_wrap(target, egui::FontId::proportional(material::FONT_SUBTITLE), theme.text);
            let pill = egui::Rect::from_center_size(rect.center(), galley.size() + egui::vec2(48.0, 24.0));
            ui.painter().rect_filled(pill, material::pill(pill), theme.surface2);
            ui.painter().galley(pill.center() - galley.size() / 2.0, galley, theme.text);
        });
    }

    fn render_toast(&mut self, ctx: &egui::Context) {
        let Some(toast) = &self.status_toast else { return };
        let t = material::animate(ctx, egui::Id::unique("toast_anim"), !toast.leaving);
        if toast.leaving && t <= 0.0 {
            self.status_toast = None;
            return;
        }
        let (message, has_action) = (toast.message.clone(), toast.action.is_some());
        let mut act = false;
        let theme = self.theme;
        egui::Area::new(egui::Id::unique("toast"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0 + (1.0 - t) * 14.0))
            .interactable(has_action && !toast.leaving)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                let right = if has_action { 8 } else { 20 };
                egui::Frame::new()
                    .fill(theme.surface2)
                    .corner_radius(egui::CornerRadius::same(material::RADIUS_LG))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin { left: 20, right, top: 6, bottom: 6 })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.set_min_height(material::BUTTON_HEIGHT);
                            ui.label(egui::RichText::new(message).size(material::FONT_BODY).color(theme.text));
                            if has_action {
                                ui.add_space(8.0);
                                act = material::button(ui, "toast_action", None, "Undo", &theme, ButtonKind::Text, true).clicked();
                            }
                        });
                    });
            });
        if act
            && let Some(toast) = self.status_toast.as_mut()
            && let Some(ToastAction::UndoTrash(paths)) = toast.action.take() {
                toast.leaving = true;
                self.restore_paths(paths);
            }
        ctx.request_repaint_after(std::time::Duration::from_millis(400));
    }

    fn render_settings(&mut self, ctx: &egui::Context) {
        let t = material::animate_for(ctx, egui::Id::unique("settings_anim"), self.show_settings, material::ANIM_SLOW);
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
                        // An Area only offers the height it had last frame, which would pin scrolling content short
                        ui.set_max_height(max_height - 32.0);
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

    fn file_options(&mut self, ui: &mut egui::Ui) -> bool {
        let theme = self.theme;
        let mut reload = false;
        self.section_title(ui, "View");
        if let Some(i) = segmented(ui, "vo_view", &["List", "Grid"], usize::from(self.view_mode == ViewMode::Grid), &theme) {
            self.view_mode = if i == 0 { ViewMode::List } else { ViewMode::Grid };
        }

        self.section_title(ui, "Sort by");
        let fields = [(SortBy::Name, "Name"), (SortBy::Size, "Size"), (SortBy::Modified, "Modified")];
        let labels: Vec<&str> = fields.iter().map(|(_, l)| *l).collect();
        let current = fields.iter().position(|(f, _)| *f == self.sort_by).unwrap_or(0);
        if let Some(i) = segmented(ui, "vo_sort", &labels, current, &theme) {
            self.sort_by = fields[i].0;
            reload = true;
        }
        let asc = self.sort_order == SortOrder::Ascending;
        if let Some(i) = segmented(ui, "vo_order", &["Ascending", "Descending"], usize::from(!asc), &theme) {
            self.sort_order = if i == 0 { SortOrder::Ascending } else { SortOrder::Descending };
            reload = true;
        }

        self.section_title(ui, "Show");
        ui.horizontal_wrapped(|ui| {
            let icon = if self.show_hidden { icons::VISIBILITY } else { icons::VISIBILITY_OFF };
            if chip(ui, "set_hidden", Some(icon), "Hidden files", &theme, self.show_hidden).clicked() {
                self.show_hidden = !self.show_hidden;
                reload = true;
            }
            if chip(ui, "set_folders_first", Some(icons::FOLDER), "Folders first", &theme, self.folders_first).clicked() {
                self.folders_first = !self.folders_first;
                reload = true;
            }
        });

        self.section_title(ui, "List columns");
        ui.horizontal_wrapped(|ui| {
            for column in Column::ALL {
                let on = self.columns.contains(&column);
                if chip(ui, ("set_col", column.label()), None, column.label(), &theme, on).clicked() {
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
                if chip(ui, ("set_type", filter.label()), None, filter.label(), &theme, self.type_filter == filter).clicked() {
                    self.type_filter = filter;
                    reload = true;
                }
            }
        });

        reload
    }

    fn view_options_contents(&mut self, ui: &mut egui::Ui) -> bool {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
        ui.label(egui::RichText::new("View").size(material::FONT_TITLE).color(self.theme.text));
        self.file_options(ui)
    }

    fn shortcuts_contents(&mut self, ui: &mut egui::Ui) -> bool {
        let theme = self.theme;
        let close = material::dialog_header(ui, "Keyboard shortcuts", None, &theme, "sc_close");
        let max_height = (ui.ctx().content_rect().height() - 200.0).clamp(200.0, 540.0);
        egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
            for group in ["Navigation", "Files", "View"] {
                material::section_label(ui, group, &theme);
                material::group_card(ui, &theme, |ui| {
                    for action in crate::keymap::Action::ALL {
                        if action.group() != group {
                            continue;
                        }
                        let bindings: Vec<String> = crate::keymap::bindings(&self.settings.keybinds, action)
                            .into_iter()
                            .filter(|b| self.settings.vim_keys || !crate::keymap::is_bare_letter(b))
                            .collect();
                        if bindings.is_empty() {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.set_min_height(34.0);
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(action.label()).size(material::FONT_BODY).color(theme.text));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(6.0);
                                for binding in bindings.iter().rev() {
                                    material::keycap(ui, &crate::keymap::display(binding), &theme);
                                }
                            });
                        });
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
        if !self.can_modify_here() {
            self.set_toast("Open a regular folder to create files in it");
            return;
        }
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

    fn open_rename(&mut self, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.dialog = Some(Dialog::Rename { path, name, error: None, first_frame: true });
    }

    /// Rename `path` inside its folder and keep the renamed item selected.
    fn rename_entry(&mut self, path: &Path, new_name: &str) -> Result<(), String> {
        let new_name = new_name.trim();
        if new_name.is_empty() || new_name == "." || new_name == ".." || new_name.contains('/') {
            return Err("Enter a valid name".to_owned());
        }
        if path.file_name().is_some_and(|n| n.to_string_lossy() == new_name) {
            return Ok(());
        }
        let target = path.with_file_name(new_name);
        if target.symlink_metadata().is_ok() {
            return Err(format!("“{new_name}” already exists"));
        }
        std::fs::rename(path, &target).map_err(|err| err.to_string())?;

        if let Some(idx) = self.pins.iter().position(|p| p.as_path() == path) {
            self.pins[idx] = target.clone();
            Place::save_pins(&self.pins);
        }
        if let Some(icon) = self.folder_icons.remove(path) {
            self.folder_icons.insert(target.clone(), icon);
            save_folder_icons(&self.folder_icons);
        }
        if let Some(app) = self.open_overrides.remove(path) {
            self.open_overrides.insert(target.clone(), app);
            apps::save_overrides(&self.open_overrides);
        }

        if self.is_mixed_listing() {
            if let Some(idx) = self.items.iter().position(|i| i.path == path)
                && let Some(item) = FileItem::from_path(&target) {
                    self.items[idx] = item;
                }
        } else {
            self.reload_items();
        }
        if let Some(idx) = self.items.iter().position(|i| i.path == target) {
            self.selected_index = Some(idx);
            self.sync_selection(true);
            self.scroll_to_selected = true;
        }
        self.set_toast(format!("Renamed to “{new_name}”"));
        Ok(())
    }

    /// Move to the trash, with an Undo in the toast.
    fn trash_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let label = Self::describe_count(&paths);
        let (mut trashed, mut failed) = (Vec::new(), Vec::new());
        for path in &paths {
            match trash::trash(path) {
                Ok(t) => trashed.push(t),
                Err(err) => failed.push(format!("{}: {err}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
            }
        }
        self.selected_index = None;
        self.refresh_listing();
        if trashed.is_empty() {
            self.set_toast(format!("Could not move to trash: {}", failed.join(" · ")));
            return;
        }
        let mut message = format!("Moved {label} to trash");
        if !failed.is_empty() {
            message.push_str(&format!(" · {} failed", failed.len()));
        }
        self.set_toast_action(message, ToastAction::UndoTrash(trashed));
    }

    fn restore_paths(&mut self, paths: Vec<PathBuf>) {
        let label = Self::describe_count(&paths);
        let mut failed = Vec::new();
        for path in &paths {
            if let Err(err) = trash::restore(path) {
                failed.push(err.to_string());
            }
        }
        self.selected_index = None;
        self.refresh_listing();
        if failed.is_empty() {
            self.set_toast(format!("Restored {label}"));
        } else {
            self.set_toast(format!("Could not restore: {}", failed.join(" · ")));
        }
    }

    /// Ask before deleting for good (unless the setting turned the question off).
    fn request_delete_forever(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if !self.settings.confirm_delete {
            self.delete_forever(paths);
            return;
        }
        self.dialog = Some(Dialog::Confirm {
            title: "Delete permanently?".to_owned(),
            body: format!("{} will be deleted for good. This can't be undone.", Self::describe_count(&paths)),
            confirm: "Delete".to_owned(),
            action: ConfirmAction::DeleteForever(paths),
        });
    }

    fn request_empty_trash(&mut self) {
        if trash::is_empty() {
            self.set_toast("The trash is already empty");
        } else if self.settings.confirm_delete {
            self.dialog = Some(Dialog::Confirm {
                title: "Empty the trash?".to_owned(),
                body: "Everything in the trash will be deleted for good. This can't be undone.".to_owned(),
                confirm: "Empty trash".to_owned(),
                action: ConfirmAction::EmptyTrash,
            });
        } else {
            self.empty_trash();
        }
    }

    fn delete_forever(&mut self, paths: Vec<PathBuf>) {
        let label = Self::describe_count(&paths);
        self.selected_index = None;
        self.set_toast(format!("Deleting {label}…"));
        self.jobs.spawn(move || {
            let failures: Vec<String> = paths
                .iter()
                .filter_map(|p| trash::delete_forever(p).err().map(|e| format!("{}: {e}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())))
                .collect();
            if failures.is_empty() { Ok(format!("Deleted {label}")) } else { Err(failures.join(" · ")) }
        });
    }

    fn empty_trash(&mut self) {
        self.selected_index = None;
        self.set_toast("Emptying the trash…");
        self.jobs.spawn(|| trash::empty().map(|_| "The trash is empty".to_owned()).map_err(|e| e.to_string()));
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
                if let Some(folder) = image.parent().map(Path::to_path_buf) {
                    self.apply_folder_icon(&folder, Some(&image));
                }
            }
            MenuAction::Rename(path) => self.open_rename(path),
            MenuAction::Trash(paths) => self.trash_paths(paths),
            MenuAction::DeleteForever(paths) => self.request_delete_forever(paths),
            MenuAction::Restore(paths) => self.restore_paths(paths),
            MenuAction::EmptyTrash => self.request_empty_trash(),
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
        let (in_trash, mixed) = (self.in_trash(), self.is_mixed_listing());
        // Clipboard / transfer / archive entries shared by every selection
        let file_groups = |paths: Vec<PathBuf>, archives: bool| {
            let mut manage = Vec::new();
            if let [one] = paths.as_slice() {
                manage.push((icons::RENAME, "Rename", MenuAction::Rename(one.clone())));
            }
            manage.push((icons::TRASH, "Move to trash", MenuAction::Trash(paths.clone())));
            let mut groups = vec![
                vec![
                    (icons::COPY, "Copy", MenuAction::Copy(paths.clone())),
                    (icons::CUT, "Cut", MenuAction::Cut(paths.clone())),
                ],
                vec![
                    (icons::FOLDER_COPY, "Copy to…", MenuAction::CopyTo(paths.clone())),
                    (icons::FOLDER_MOVE, "Move to…", MenuAction::MoveTo(paths.clone())),
                ],
                manage,
            ];
            let mut pack = vec![(icons::COMPRESS, "Compress…", MenuAction::Compress(paths.clone()))];
            if archives {
                pack.push((icons::EXTRACT, "Extract here", MenuAction::ExtractHere(paths.clone())));
                pack.push((icons::EXTRACT, "Extract to…", MenuAction::ExtractTo(paths)));
            }
            groups.push(pack);
            groups
        };
        // Results and Recent items live in many folders: offer a way to the one that holds them
        let open_group = |item: &FileItem, with: bool| {
            let mut group = vec![(icons::OPEN, "Open", MenuAction::Open(item.clone()))];
            if with {
                group.push((icons::APPS, "Open with…", MenuAction::OpenWith(item.path.clone())));
            }
            if mixed && let Some(parent) = item.path.parent() {
                group.push((icons::FOLDER, "Show in folder", MenuAction::OpenPath(parent.to_path_buf())));
            }
            group
        };
        match target {
            MenuTarget::Item(item) if in_trash => {
                let paths: Vec<PathBuf> = self.menu_group(item).into_iter().map(|i| i.path).collect();
                vec![
                    vec![
                        (icons::RESTORE, "Restore", MenuAction::Restore(paths.clone())),
                        (icons::DELETE_FOREVER, "Delete permanently", MenuAction::DeleteForever(paths)),
                    ],
                    vec![(icons::INFO, "Properties", MenuAction::Properties(item.path.clone()))],
                ]
            }
            MenuTarget::Item(item) if self.menu_group(item).len() > 1 => {
                let group = self.menu_group(item);
                let paths: Vec<PathBuf> = group.iter().map(|i| i.path.clone()).collect();
                let archives = group.iter().any(|i| !i.is_dir && ops::is_archive(&i.path));
                let mut groups = file_groups(paths, archives);
                groups.push(vec![(icons::INFO, "Properties", MenuAction::Properties(item.path.clone()))]);
                groups
            }
            MenuTarget::Item(item) if item.is_dir => {
                let mut groups = vec![open_group(item, false), vec![pin_entry(&item.path), icon_entry(&item.path)]];
                groups.extend(file_groups(vec![item.path.clone()], false));
                groups.push(vec![
                    (icons::COPY, "Copy path", MenuAction::CopyPath(item.path.clone())),
                    (icons::INFO, "Properties", MenuAction::Properties(item.path.clone())),
                ]);
                groups
            }
            MenuTarget::Item(item) => {
                let mut groups = vec![open_group(item, true)];
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
            MenuTarget::Folder if in_trash => vec![
                vec![(icons::DELETE_FOREVER, "Empty trash", MenuAction::EmptyTrash)],
                vec![(icons::SELECT_ALL, "Select all", MenuAction::SelectAll)],
                vec![(icons::REFRESH, "Refresh", MenuAction::Refresh)],
            ],
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
            MenuTarget::TrashPlace(path) => vec![
                vec![(icons::OPEN, "Open", MenuAction::OpenPath(path.clone()))],
                vec![(icons::DELETE_FOREVER, "Empty trash", MenuAction::EmptyTrash)],
            ],
            MenuTarget::Pin(path) => vec![
                vec![(icons::OPEN, "Open", MenuAction::OpenPath(path.clone()))],
                vec![(icons::PIN_OFF, "Unpin from sidebar", MenuAction::Unpin(path.clone())), icon_entry(path)],
                vec![(icons::INFO, "Properties", MenuAction::Properties(path.clone()))],
            ],
        }
    }

    /// Keyboard shortcut to show next to a menu entry.
    fn menu_hint(&self, action: &MenuAction) -> Option<String> {
        use crate::keymap::Action as A;
        let act = match action {
            MenuAction::Open(_) => A::Open,
            MenuAction::Copy(_) => A::Copy,
            MenuAction::Cut(_) => A::Cut,
            MenuAction::Paste => A::Paste,
            MenuAction::SelectAll => A::SelectAll,
            MenuAction::Properties(_) => A::Properties,
            MenuAction::NewFolder => A::NewFolder,
            MenuAction::NewFile => A::NewFile,
            MenuAction::Refresh => A::Refresh,
            MenuAction::NewWindow => A::NewWindow,
            MenuAction::OpenSettings => A::Settings,
            MenuAction::Shortcuts => A::ShowShortcuts,
            MenuAction::Rename(_) => A::Rename,
            MenuAction::Trash(_) => A::Trash,
            MenuAction::DeleteForever(_) => A::DeleteForever,
            MenuAction::CopyPath(_) => A::CopyPath,
            MenuAction::Pin(_) | MenuAction::Unpin(_) => A::PinFolder,
            _ => return None,
        };
        crate::keymap::hint(&self.settings.keybinds, act, self.settings.vim_keys)
    }

    fn render_menu(&mut self, ctx: &egui::Context) {
        if let Some(menu) = self.menu.as_mut()
            && let Some(Ok(label)) = menu.label_rx.as_ref().map(|rx| rx.try_recv()) {
                menu.open_label = label;
                menu.label_rx = None;
            }

        // A closed menu fades out from the snapshot taken while it was open
        let t = material::animate(ctx, egui::Id::unique("context_menu_anim"), self.menu.is_some());
        let live = self.menu.as_ref().map(|m| (m.pos, m.target.clone(), m.open_label.clone()));
        let interactive = live.is_some();
        let (origin, groups, open_label) = if let Some((pos, target, open_label)) = live {
            let groups = self.menu_entries(&target);
            let count: usize = groups.iter().map(Vec::len).sum();
            let dividers = groups.len().saturating_sub(1);
            let gaps = (count + dividers).saturating_sub(1);
            let height = count as f32 * material::ENTRY_HEIGHT + dividers as f32 * 9.0 + gaps as f32 * 2.0 + 16.0;
            let screen = ctx.content_rect();
            let origin = egui::pos2(
                pos.x.min(screen.right() - MENU_WIDTH - 8.0).max(8.0),
                pos.y.min(screen.bottom() - height - 8.0).max(8.0),
            );
            self.menu_ghost = Some(MenuGhost { origin, groups: groups.clone(), open_label: open_label.clone() });
            (origin, groups, open_label)
        } else if let Some(ghost) = self.menu_ghost.as_ref().filter(|_| t > 0.0) {
            (ghost.origin, ghost.groups.clone(), ghost.open_label.clone())
        } else {
            self.menu_ghost = None;
            return;
        };

        let is_main = matches!(self.menu.as_ref().map(|m| &m.target), Some(MenuTarget::Main));
        let hints: Vec<Vec<Option<String>>> =
            groups.iter().map(|g| g.iter().map(|(_, _, action)| self.menu_hint(action)).collect()).collect();
        let mut chosen = None;
        let area = egui::Area::new(egui::Id::unique("context_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(origin + egui::vec2(0.0, (1.0 - t) * -8.0))
            .interactable(interactive)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .corner_radius(egui::CornerRadius::same(material::RADIUS_CONTAINER))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(MENU_WIDTH - 16.0);
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
                                if menu_item(ui, ("menu", g, idx), icon, shown, hints[g][idx].as_deref(), &self.theme).clicked() && interactive {
                                    chosen = Some(action.clone());
                                }
                            }
                        }
                    });
            });

        let rect = area.response.rect;
        let main_button = self.main_button_rect;
        let outside_press = interactive
            && ctx.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !(is_main && main_button.contains(p)))
            });
        if chosen.is_some() || outside_press {
            self.menu = None;
        }
        if let Some(action) = chosen {
            self.menu_click_consumed = true;
            self.run_action(ctx, action);
        }
    }

    fn render_dialog(&mut self, ctx: &egui::Context) {
        let click_consumed = std::mem::take(&mut self.menu_click_consumed);
        let is_open = self.dialog.is_some();
        let t = material::animate(ctx, egui::Id::unique("dialog_anim"), is_open);

        if !is_open && t <= 0.0 {
            self.closing_dialog = None;
            return;
        }

        let mut dialog = if let Some(d) = self.dialog.take() {
            d
        } else if let Some(d) = self.closing_dialog.take() {
            d
        } else {
            return;
        };

        if is_open
            && let Dialog::Icon { input, error, .. } = &mut dialog
                && let Some(path) = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf())) {
                    *input = path.to_string_lossy().into_owned();
                    *error = None;
                }

        let screen = ctx.content_rect();
        let mut close = false;

        // Dimmed backdrop lives strictly in Order::Middle, preventing clicks from passing to the background
        // while never covering or stealing clicks from the dialog in Order::Foreground.
        let _scrim_area = egui::Area::new(egui::Id::unique("dialog_scrim"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .interactable(is_open)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(rect, 0, egui::Color32::from_black_alpha((110.0 * t) as u8));
                resp
            });

        let preferred = match &dialog {
            Dialog::Settings { .. } => 720.0,
            Dialog::OpenWith { .. } | Dialog::PickFolder { .. } => 480.0,
            Dialog::Confirm { .. } => 400.0,
            _ => 440.0,
        };
        let width = (screen.width() - 32.0).clamp(240.0, preferred);

        // Dialog card sits in Order::Foreground, always above the backdrop scrim
        let dialog_area = egui::Area::new(egui::Id::unique("dialog_window"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, (1.0 - t) * 14.0))
            .interactable(is_open)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .corner_radius(egui::CornerRadius::same(material::RADIUS_WINDOW))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(20))
                    .show(ui, |ui| {
                        ui.set_width(width - 40.0);
                        // An Area only offers the height it had last frame; let content grow again (search filters, folder scans)
                        ui.set_max_height(screen.height() - 40.0);
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
                        close |= match &mut dialog {
                            Dialog::Create { .. } => self.create_dialog_contents(ui, &mut dialog),
                            Dialog::Rename { .. } => self.rename_dialog_contents(ui, &mut dialog),
                            Dialog::Confirm { .. } => self.confirm_dialog_contents(ui, &mut dialog),
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

        // Click outside the dialog card dismisses it
        let dialog_rect = dialog_area.response.rect;
        let outside_click = is_open
            && !click_consumed
            && ctx.input(|i| {
                i.pointer.primary_clicked()
                    && i.pointer.interact_pos().is_some_and(|p| !dialog_rect.contains(p))
            });

        if outside_click {
            close = true;
        }

        if is_open {
            if close {
                self.closing_dialog = Some(dialog);
                self.dialog = None;
            } else if self.dialog.is_none() {
                // A child dialog opened inside this one replaces it
                self.dialog = Some(dialog);
            }
        } else {
            self.closing_dialog = Some(dialog);
        }
    }

    fn create_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Create { is_dir, name, error, first_frame } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, if *is_dir { "New folder" } else { "New file" }, None, &theme, "create_close");

        let (changed, mut submit) = name_field(ui, &theme, name, first_frame, !*is_dir);
        if changed {
            *error = None;
        }
        if let Some(err) = error.as_ref() {
            ui.label(egui::RichText::new(err).size(material::FONT_CAPTION).color(theme.red));
        }

        let footer = material::dialog_footer(ui, &theme, FooterSpec::new("create", icons::CHECK, "Create"));
        submit |= footer.primary;
        close |= footer.cancel;
        if submit {
            match self.create_entry(*is_dir, name) {
                Ok(()) => close = true,
                Err(err) => *error = Some(err),
            }
        }
        close
    }

    fn rename_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Rename { path, name, error, first_frame } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, "Rename", None, &theme, "rename_close");

        let (changed, mut submit) = name_field(ui, &theme, name, first_frame, !path.is_dir());
        if changed {
            *error = None;
        }
        if let Some(err) = error.as_ref() {
            ui.label(egui::RichText::new(err).size(material::FONT_CAPTION).color(theme.red));
        }

        let footer = material::dialog_footer(ui, &theme, FooterSpec::new("rename", icons::CHECK, "Rename"));
        submit |= footer.primary;
        close |= footer.cancel;
        if submit {
            match self.rename_entry(path, name) {
                Ok(()) => close = true,
                Err(err) => *error = Some(err),
            }
        }
        close
    }

    fn confirm_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Confirm { title, body, confirm, action } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, title, None, &theme, "confirm_close");
        ui.label(egui::RichText::new(body.as_str()).size(material::FONT_BODY).color(theme.subtext0));

        let footer = material::dialog_footer(ui, &theme, FooterSpec { danger: true, ..FooterSpec::new("confirm", icons::DELETE_FOREVER, confirm) });
        close |= footer.cancel;
        if footer.primary {
            match std::mem::replace(action, ConfirmAction::EmptyTrash) {
                ConfirmAction::DeleteForever(paths) => self.delete_forever(paths),
                ConfirmAction::EmptyTrash => self.empty_trash(),
            }
            close = true;
        }
        close
    }

    fn pick_folder_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::PickFolder { op, items, dir, path_input } = dialog else { return false };
        let theme = self.theme;
        let (title, action) = match op {
            TransferOp::Copy => ("Copy to…", "Copy here"),
            TransferOp::Move => ("Move to…", "Move here"),
            TransferOp::Extract => ("Extract to…", "Extract here"),
        };
        let mut close = material::dialog_header(ui, title, Some(&Self::describe_count(items)), &theme, "pf_close");

        let mut goto: Option<PathBuf> = None;
        ui.horizontal_wrapped(|ui| {
            for (idx, place) in self.places.iter().enumerate() {
                if place.kind != PlaceKind::Folder || trash::contains(&place.path) {
                    continue;
                }
                if chip(ui, ("pf_place", idx), Some(place.icon), &place.name, &theme, *dir == place.path).clicked() {
                    goto = Some(place.path.clone());
                }
            }
        });

        material::input_frame(&theme).show(ui, |ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(path_input)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .hint_text("Folder path")
                    .font(egui::FontId::proportional(material::FONT_BODY)),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let typed = PathBuf::from(path_input.trim());
                if typed.is_dir() {
                    goto = Some(typed);
                } else {
                    self.set_toast("That folder does not exist");
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
        let max_height = (ui.ctx().content_rect().height() - 340.0).clamp(140.0, 300.0);
        material::group_card(ui, &theme, |ui| {
            egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).id_salt("pf_scroll").show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if let Some(parent) = dir.parent()
                    && nav_item(ui, "pf_up", icons::ARROW_UP, "..", &theme, false).clicked() {
                        goto = Some(parent.to_path_buf());
                    }
                for (idx, folder) in folders.iter().enumerate() {
                    let label = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if nav_item(ui, ("pf_folder", idx), icons::FOLDER, &label, &theme, false).clicked() {
                        goto = Some(folder.clone());
                    }
                }
                if folders.is_empty() {
                    ui.add_space(4.0);
                    material::hint_label(ui, "No subfolders", &theme);
                    ui.add_space(4.0);
                }
            });
        });
        if let Some(target) = goto {
            *path_input = target.to_string_lossy().into_owned();
            *dir = target;
        }

        let footer = material::dialog_footer(ui, &theme, FooterSpec::new("pf", icons::CHECK, action));
        close |= footer.cancel;
        if footer.primary {
            self.transfer(*op, items.clone(), dir.clone());
            close = true;
        }
        close
    }

    fn compress_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Compress { sources, name, format, first_frame } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, "Compress", Some(&Self::describe_count(sources)), &theme, "cz_close");

        let mut submit = false;
        material::input_frame(&theme).show(ui, |ui| {
            ui.horizontal(|ui| {
                let font = egui::FontId::proportional(material::FONT_SUBTITLE);
                let ext_w = ui.painter().layout_no_wrap(format.extension().to_owned(), font.clone(), theme.subtext1).size().x;
                let output = egui::TextEdit::singleline(name)
                    .frame(egui::Frame::NONE)
                    .desired_width(ui.available_width() - ext_w - ui.spacing().item_spacing.x)
                    .font(font)
                    .show(ui);
                if *first_frame {
                    output.response.request_focus();
                    *first_frame = false;
                }
                if output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                ui.label(egui::RichText::new(format.extension()).size(material::FONT_SUBTITLE).color(theme.subtext1));
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

        let footer = material::dialog_footer(ui, &theme, FooterSpec { enabled: !name.trim().is_empty(), ..FooterSpec::new("cz", icons::COMPRESS, "Compress") });
        submit |= footer.primary;
        close |= footer.cancel;

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

    fn open_with_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::OpenWith { path, mime, apps: listed, default, recommended, others, selected, query, first_frame, for_file, for_type } = dialog else { return false };
        let theme = self.theme;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut close = material::dialog_header(ui, "Open with", Some(&format!("{name} · {mime}")), &theme, "ow_close");

        // Search over every installed application
        material::input_frame(&theme).show(ui, |ui| {
            ui.horizontal(|ui| {
                material::inline_icon(ui, icons::SEARCH, material::ICON_SM, theme.subtext1);
                let resp = ui.add(
                    egui::TextEdit::singleline(query)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .hint_text("Search applications")
                        .font(egui::FontId::proportional(material::FONT_BODY)),
                );
                if *first_frame {
                    resp.request_focus();
                    *first_frame = false;
                }
            });
        });

        let needle = query.trim().to_lowercase();
        let matches = |i: &usize| needle.is_empty() || listed[*i].name.to_lowercase().contains(&needle);
        let sections: [(&str, Vec<usize>); 3] = [
            ("Default", default.iter().copied().filter(matches).collect()),
            ("Recommended", recommended.iter().copied().filter(matches).collect()),
            ("Other applications", others.iter().copied().filter(matches).collect()),
        ];

        let max_height = (ui.ctx().content_rect().height() - 330.0).clamp(160.0, 380.0);
        let mut launch = false;
        // The application list sits in a box of its own
        material::group_card(ui, &theme, |ui| {
            if sections.iter().all(|(_, rows)| rows.is_empty()) {
                ui.add_space(6.0);
                material::hint_label(ui, "No applications found", &theme);
                ui.add_space(6.0);
            }
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

        material::group_card(ui, &theme, |ui| {
            if material::switch_row(ui, "ow_file", icons::FILE, "Always for this file", &theme, *for_file).clicked() {
                *for_file = !*for_file;
            }
            if material::switch_row(ui, "ow_type", icons::APPS, "Always for this file type", &theme, *for_type).clicked() {
                *for_type = !*for_type;
            }
        });

        let has_choice = self.open_overrides.contains_key(path.as_path());
        let footer = material::dialog_footer(
            ui,
            &theme,
            FooterSpec { enabled: selected.is_some(), extra: has_choice.then_some("Forget file choice"), ..FooterSpec::new("ow", icons::OPEN, "Open") },
        );
        close |= footer.cancel;
        launch |= footer.primary;
        if footer.extra {
            self.open_overrides.remove(path.as_path());
            apps::save_overrides(&self.open_overrides);
            self.set_toast("File choice removed");
            close = true;
        }

        if launch
            && let Some(app) = selected.and_then(|i| listed.get(i)) {
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

    fn icon_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Icon { folder, input, error, candidates, browse } = dialog else { return false };
        let theme = self.theme;
        let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Computer".to_owned());
        let mut close = material::dialog_header(
            ui,
            "Folder icon",
            Some(&format!("Choose an image for “{name}”. You can also drop a file here.")),
            &theme,
            "icon_close",
        );

        let mut apply = false;
        material::input_frame(&theme).show(ui, |ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(input)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .hint_text("No image chosen")
                    .font(egui::FontId::proportional(material::FONT_BODY)),
            );
            if resp.changed() {
                *error = None;
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                apply = true;
            }
        });

        ui.horizontal(|ui| {
            if chip(ui, "icon_browse", Some(icons::FOLDER), "Choose image…", &theme, browse.is_some()).clicked() {
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
            material::hint_label(ui, &dir.to_string_lossy(), &theme);
            material::group_card(ui, &theme, |ui| {
                egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).id_salt("icon_browser").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if let Some(parent) = dir.parent()
                        && nav_item(ui, "icon_browse_up", icons::ARROW_UP, "..", &theme, false).clicked() {
                            *browse = Some(parent.to_path_buf());
                        }
                    for (idx, (path, is_dir)) in entries.iter().enumerate() {
                        let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let glyph = if *is_dir { icons::FOLDER } else { icons::IMAGE };
                        let on = !*is_dir && *input == path.to_string_lossy();
                        if nav_item(ui, ("icon_browse_entry", idx), glyph, &label, &theme, on).clicked() {
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
            });
        } else if !candidates.is_empty() {
            material::section_label(ui, "Images in this folder", &theme);
            ui.horizontal_wrapped(|ui| {
                for (idx, path) in candidates.iter().enumerate() {
                    let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let on = *input == path.to_string_lossy();
                    if chip(ui, ("icon_candidate", idx), None, &label, &theme, on).clicked() {
                        *input = path.to_string_lossy().into_owned();
                        *error = None;
                    }
                }
            });
        }
        if let Some(err) = error.as_ref() {
            ui.label(egui::RichText::new(err).size(material::FONT_CAPTION).color(theme.red));
        }

        let has_icon = self.folder_icons.contains_key(folder.as_path());
        let footer = material::dialog_footer(ui, &theme, FooterSpec { extra: has_icon.then_some("Reset"), ..FooterSpec::new("icon", icons::CHECK, "Apply") });
        close |= footer.cancel;
        apply |= footer.primary;
        if footer.extra {
            self.apply_folder_icon(folder, None);
            close = true;
        }

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
        let theme = self.theme;
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

        close |= material::dialog_header(ui, "Properties", None, &theme, "dlg_close");
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
                    ui.painter().rect_filled(tile, egui::CornerRadius::same(material::RADIUS_WINDOW), theme.surface1);
                    match thumb_for(&self.thumbs, &self.folder_icons, item, self.settings.show_thumbnails) {
                        Some(tex) => {
                            egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), egui::vec2(64.0, 64.0)))
                                .corner_radius(egui::CornerRadius::same(material::RADIUS_MD))
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
                    ui.label(egui::RichText::new(subtitle).size(material::FONT_CAPTION).color(theme.subtext1));
                    if item.is_dir {
                        ui.add_space(6.0);
                        if material::button(ui, "props_icon", Some(icons::IMAGE), "Custom icon…", &theme, ButtonKind::Tonal, true).clicked() {
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
                            painter.rect_filled(inner, material::pill(inner), theme.with_alpha(theme.text, (fx.hover * material::ALPHA_HOVER as f32) as u8));
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

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if material::button(ui, "dlg_copy_path", Some(icons::COPY), "Copy path", &theme, ButtonKind::Tonal, true).clicked() {
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
    let sel = material::animate(ui.ctx(), id.with("sel"), selected);
    let bg = material::lerp_color(theme.with_alpha(theme.text, (fx.hover * material::ALPHA_HOVER as f32) as u8), theme.with_alpha(theme.mauve, material::ALPHA_SEL_BG), sel);
    ui.painter().rect_filled(rect, material::pill(rect), bg);

    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), egui::vec2(28.0, 28.0));
    match icon {
        Some(tex) => {
            egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), icon_rect.size())).paint_at(ui, icon_rect);
        }
        None => material::paint_icon(ui.painter(), icon_rect.center(), icons::EXECUTABLE, 22.0, theme.subtext0),
    }
    let color = material::lerp_color(theme.text, theme.mauve, sel);
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), egui::FontId::proportional(material::FONT_BODY), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 64.0).max(20.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(rect.left() + 48.0, rect.center().y - galley.size().y / 2.0), galley, color);
    resp
}

/// Single-line name entry shared by the new-item and rename dialogs. On the first frame it takes
/// focus and selects the name (without its extension when `stem_only`). Returns (changed, submitted).
fn name_field(ui: &mut egui::Ui, theme: &SerpantinumTheme, text: &mut String, first_frame: &mut bool, stem_only: bool) -> (bool, bool) {
    let (mut changed, mut submitted) = (false, false);
    material::input_frame(theme).show(ui, |ui| {
        let output = egui::TextEdit::singleline(text)
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY)
            .font(egui::FontId::proportional(material::FONT_SUBTITLE))
            .show(ui);
        if *first_frame {
            output.response.request_focus();
            let stem_end = match text.rfind('.') {
                Some(i) if stem_only && i > 0 => text[..i].chars().count(),
                _ => text.chars().count(),
            };
            let mut state = output.state.clone();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(stem_end))));
            state.store(ui.ctx(), output.response.id);
            *first_frame = false;
        }
        changed = output.response.changed();
        submitted = output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    });
    (changed, submitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives rename, trash, undo and permanent delete through the app, in a sandboxed data folder.
    #[test]
    fn rename_trash_restore_delete() {
        let base = std::env::temp_dir().join(format!("ski_app_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let work = base.join("work");
        std::fs::create_dir_all(work.join("sub")).unwrap();
        std::fs::write(work.join("a.txt"), "a").unwrap();
        std::fs::write(work.join("taken.txt"), "t").unwrap();
        // The trash and the settings live under these folders, never in the real home
        unsafe {
            std::env::set_var("XDG_DATA_HOME", base.join("data"));
            std::env::set_var("XDG_CONFIG_HOME", base.join("config"));
        }

        let ctx = egui::Context::default();
        let mut app = SkiApp::new(work.clone(), SerpantinumTheme::default(), Settings::default(), &ctx);
        let names = |app: &SkiApp| app.items.iter().map(|i| i.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&app), ["sub", "a.txt", "taken.txt"]);

        // Rename keeps the item selected, refuses clashes and bad names
        let a = app.current_path.join("a.txt");
        assert!(app.rename_entry(&a, "taken.txt").is_err());
        assert!(app.rename_entry(&a, "x/y").is_err());
        assert!(app.rename_entry(&a, "  ").is_err());
        app.rename_entry(&a, "b.txt").unwrap();
        assert_eq!(names(&app), ["sub", "b.txt", "taken.txt"]);
        assert_eq!(app.selected_index.map(|i| app.items[i].name.as_str()), Some("b.txt"));
        assert!(app.rename_entry(&app.current_path.join("b.txt"), "b.txt").is_ok(), "same name is a no-op");

        // Trash removes it from the listing, offers Undo, and Undo brings it back
        let b = app.current_path.join("b.txt");
        app.trash_paths(vec![b.clone()]);
        assert_eq!(names(&app), ["sub", "taken.txt"]);
        assert!(!b.exists());
        let Some(Toast { action: Some(ToastAction::UndoTrash(trashed)), .. }) = app.status_toast.take() else { panic!("no undo offered") };
        assert!(trashed[0].exists() && trash::contains(&trashed[0].canonicalize().unwrap()));
        app.restore_paths(trashed);
        assert_eq!(names(&app), ["sub", "b.txt", "taken.txt"]);
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "a");

        // Delete forever runs on a worker; the trash record goes with the file
        app.trash_paths(vec![b.clone()]);
        let Some(Toast { action: Some(ToastAction::UndoTrash(trashed)), .. }) = app.status_toast.take() else { panic!("no undo offered") };
        app.delete_forever(trashed.clone());
        for _ in 0..100 {
            if !app.jobs.running() { break; }
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.poll_jobs();
        }
        assert!(!trashed[0].exists());
        assert!(trash::is_empty());

        // The Recent view is a place of its own, and going back returns to the folder
        app.navigate_to(PathBuf::from(RECENT_PATH));
        assert!(app.in_recent() && !app.can_modify_here());
        app.go_back();
        assert_eq!(app.current_path, work.canonicalize().unwrap());
        assert!(app.can_modify_here());

        std::fs::remove_dir_all(&base).unwrap();
    }
}

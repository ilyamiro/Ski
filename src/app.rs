use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use eframe::egui;

use crate::i18n::{tr, tr_dyn, trf, trn};

mod chooser_ui;
mod dnd;
mod drives_ui;
mod file_actions;
mod jobs_ui;
mod perms;
mod tabs;
use tabs::TabSlot;

use crate::fs::{
    drives::Drive,
    file_item::FileItem,
    apps::{self, AppEntry},
    places::{load_folder_icons, save_folder_icons, Place, PlaceGroup, PlaceKind, BOOKMARKS_PATH, RECENT_PATH, is_virtual},
    props::{FolderScan, Properties},
    scanner::{read_directory, ListOptions, TypeFilter},
    recent,
    search::{Search, SearchOptions, SearchScope},
    trash,
};
use crate::fs::batch_rename::{self, Entry as RenameEntry, Mode as RenameMode, Rules as RenameRules};
use crate::fs::engine::{self, ConflictRequest, Effect, Engine, JobKind, Op, Outcome};
use crate::fs::history::{Edit, History, UndoEntry};
use crate::fs::templates::{self, Template};
use crate::fs::ops::{self, ArchiveFormat};
use crate::fs::scanner::{SortBy, SortOrder};
use crate::settings::{Density, Settings};
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
    /// A folder place of the sidebar.
    Place(PathBuf),
    /// A drive of the sidebar: its device, name and where it is mounted (empty when it is not).
    Drive(Drive, String, PathBuf),
}

type MenuEntry = (&'static str, &'static str, MenuAction);

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
    OpenInNewTab(PathBuf),
    OpenInNewWindow(PathBuf),
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
    RenameMany(Vec<PathBuf>),
    Trash(Vec<PathBuf>),
    DeleteForever(Vec<PathBuf>),
    Restore(Vec<PathBuf>),
    EmptyTrash,
    ExtractHere(Vec<PathBuf>),
    ExtractTo(Vec<PathBuf>),
    UseAsIcon(PathBuf),
    Pin(PathBuf),
    Unpin(PathBuf),
    Bookmark(PathBuf),
    Unbookmark(PathBuf),
    /// Entries that open to the side while the pointer rests on this one.
    Submenu(Vec<MenuEntry>),
    NewFile,
    NewFolder,
    Refresh,
    NewWindow,
    OpenSettings,
    Shortcuts,
    NewTab,
    Undo,
    Redo,
    Duplicate(Vec<PathBuf>),
    MakeLink(Vec<PathBuf>),
    OpenTerminal(PathBuf),
    InvertSelection,
    SelectPattern,
    Mount(String, String),
    Unmount(String, String, PathBuf),
    Eject(String, String, PathBuf),
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
    /// The dialog's choice replaces files that exist: each one asked about in turn, the answer sent once all agreed.
    ReplaceFile { queue: Vec<PathBuf>, answer: Vec<PathBuf> },
}

/// Action offered by a toast, such as undoing a move to the trash.
enum ToastAction {
    Undo,
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
    groups: Vec<Vec<MenuEntry>>,
    open_label: Option<String>,
}

pub(crate) enum Dialog {
    Create { is_dir: bool, name: String, error: Option<String>, first_frame: bool, templates: Vec<Template>, template: usize, edited: bool },
    Rename { path: PathBuf, name: String, error: Option<String>, first_frame: bool },
    Confirm { title: String, body: String, confirm: String, action: ConfirmAction },
    Alert { title: String, body: String },
    Properties { props: Box<Properties>, scan: Option<FolderScan>, tab: usize },
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
    BatchRename { entries: Vec<RenameEntry>, rules: RenameRules, first_frame: bool },
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
    /// A copy or move found its target name taken and waits for the answer.
    Conflict { request: Option<ConflictRequest>, all: bool },
    SelectPattern { pattern: String, case: bool, first_frame: bool },
}

/// Rubber-band drag selection started inside the file listing.
struct DragSelect {
    /// Origin in content coordinates (`screen_y + scroll_offset.y`).
    start_content: egui::Pos2,
    /// Current pointer position in screen coordinates.
    current_screen: egui::Pos2,
    /// Selection before the drag began, kept when Ctrl or Shift is held.
    base_selection: BTreeSet<usize>,
    /// True once pointer travel passed the drag threshold.
    active: bool,
}

/// The folder picker of Copy to / Move to / Extract to, shown in a window of its own.
struct Picker {
    op: TransferOp,
    items: Vec<PathBuf>,
    dir: PathBuf,
    path_input: String,
    /// A subfolder picked in the list; the destination when set, otherwise `dir`.
    chosen: Option<PathBuf>,
    /// Subfolders of `dir`, read when the folder changes.
    folders: Vec<PathBuf>,
    /// Set when choosing an image for this folder instead of a destination.
    icon_for: Option<PathBuf>,
    /// Images in `dir`, only read when choosing an icon.
    images: Vec<PathBuf>,
}

impl Picker {
    fn enter(&mut self, dir: PathBuf) {
        self.path_input = dir.to_string_lossy().into_owned();
        self.folders = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        self.folders.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
        self.images = if self.icon_for.is_some() {
            let mut images: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map(|rd| rd.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).map(|e| e.path()).filter(|p| crate::thumbs::is_image(p)).collect())
                .unwrap_or_default();
            images.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
            images
        } else {
            Vec::new()
        };
        self.dir = dir;
        self.chosen = None;
    }
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
    engine: Engine,
    history: History,
    /// The undo or redo that is running, so a second one waits for it.
    history_job: Option<u64>,
    pending_conflicts: VecDeque<ConflictRequest>,
    jobs_popover: bool,
    jobs_button_rect: egui::Rect,
    /// Where each listed item was drawn last frame (for the menu key).
    last_item_rects: Vec<(usize, egui::Rect)>,
    /// Names that running duplications will create, so a second one picks another.
    reserved: Vec<PathBuf>,
    /// A folder another part of the app wants shown (settings buttons), opened at the next frame.
    pub(crate) pending_open: Option<PathBuf>,
    drive_state: drives_ui::DriveState,
    /// Where each pinned row was drawn last frame, for reordering by dragging.
    pin_rects: Vec<egui::Rect>,
    dnd: Option<dnd::Dnd>,
    drop_zones: Vec<dnd::DropZone>,
    /// The part of the window the listing fills (drops on items must land inside it).
    dnd_view: egui::Rect,
    error_message: Option<String>,

    // Dynamic shell theme from Serpantinum
    pub(crate) theme: SerpantinumTheme,

    // Navigation history
    back_history: Vec<PathBuf>,
    forward_history: Vec<PathBuf>,

    // Search and display options
    search_query: String,
    /// The folder a search began in (the one open when the query was first typed).
    search_root: Option<PathBuf>,
    pub(crate) show_hidden: bool,
    pub(crate) folders_first: bool,
    type_filter: TypeFilter,
    pub(crate) view_mode: ViewMode,
    pub(crate) sort_by: SortBy,
    pub(crate) sort_order: SortOrder,

    // Path edit mode (Ctrl+L)
    is_editing_path: bool,
    /// The path field has been on screen since editing began.
    path_field_open: bool,
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
    from_theme: SerpantinumTheme,
    target_theme: SerpantinumTheme,
    theme_anim_start: Option<f64>,
    pub(crate) theme_dirty: bool,
    pub(crate) fonts_dirty: bool,
    pins: Vec<PathBuf>,
    bookmarks: Vec<PathBuf>,
    /// The submenu entry (group, index) whose flyout is open, and where the flyout was drawn.
    menu_sub: Option<(usize, usize)>,
    menu_sub_rect: Option<egui::Rect>,
    menu: Option<ContextMenu>,
    pub(crate) dialog: Option<Dialog>,
    picker: Option<Picker>,
    /// The selection summary the pill still shows while it fades out.
    pill_text: String,
    /// Set when Ski runs as the open / save dialog of other programs.
    pub(crate) chooser: Option<crate::chooser::Chooser>,
    pub(crate) closing_dialog: Option<Dialog>,
    /// Kind of the dialog on screen, to notice when a new one appears and should take keyboard focus.
    dialog_kind: Option<std::mem::Discriminant<Dialog>>,
    /// Set when a menu click ran this frame, so it is not also read as a click outside a dialog it just opened.
    menu_click_consumed: bool,

    menu_ghost: Option<MenuGhost>,
    window_title: String,

    status_toast: Option<Toast>,

    // Flag to scroll to selected item on next frame
    scroll_to_selected: bool,
    // Accumulated Ctrl+scroll / pinch zoom, in log units
    zoom_accum: f32,

    drag_select: Option<DragSelect>,
    last_rubberband: Option<egui::Rect>,

    pub(crate) capturing_key: bool,

    // Tabs: the one on screen is the app itself, the others are parked in their slot
    tabs: Vec<TabSlot>,
    active_tab: usize,
    next_tab_id: u64,
    /// A tab that was just opened, so it grows in.
    tab_enter: Option<u64>,
    /// A parked tab with a search comes back; the search runs again.
    restart_search: bool,
}

impl SkiApp {
    pub fn new(initial_path: PathBuf, theme: SerpantinumTheme, settings: Settings, ctx: &egui::Context) -> Self {
        apps::preload();
        crate::ui::fonts::preload();
        // Ctrl+= / Ctrl+- resize icons (see `zoom_icons`), not the whole interface
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
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
            engine: Engine::new(ctx),
            history: History::default(),
            history_job: None,
            pending_conflicts: VecDeque::new(),
            jobs_popover: false,
            jobs_button_rect: egui::Rect::NOTHING,
            last_item_rects: Vec::new(),
            reserved: Vec::new(),
            pending_open: None,
            drive_state: drives_ui::DriveState::new(),
            pin_rects: Vec::new(),
            dnd: None,
            drop_zones: Vec::new(),
            dnd_view: egui::Rect::NOTHING,
            error_message: None,
            theme,
            back_history: Vec::new(),
            forward_history: Vec::new(),
            search_query: String::new(),
            search_root: None,
            show_hidden: settings.show_hidden,
            folders_first: settings.folders_first,
            type_filter: TypeFilter::All,
            view_mode,
            sort_by: settings.sort_by,
            sort_order: settings.sort_order,
            is_editing_path: false,
            path_field_open: false,
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
            from_theme: theme,
            target_theme: theme,
            theme_anim_start: None,
            theme_dirty: false,
            fonts_dirty: false,
            pins: Place::load_pins(),
            bookmarks: Place::load_bookmarks(),
            menu_sub: None,
            menu_sub_rect: None,
            menu: None,
            dialog: None,
            picker: None,
            pill_text: String::new(),
            chooser: None,
            closing_dialog: None,
            dialog_kind: None,
            menu_click_consumed: false,
            menu_ghost: None,
            window_title: String::new(),
            status_toast: None,
            scroll_to_selected: false,
            zoom_accum: 0.0,
            drag_select: None,
            last_rubberband: None,
            capturing_key: false,
            tabs: vec![TabSlot::live(0)],
            active_tab: 0,
            next_tab_id: 1,
            tab_enter: None,
            restart_search: false,
        };

        app.reload_items();
        app
    }

    pub fn navigate_to(&mut self, target: PathBuf) {
        if target == self.current_path {
            return;
        }

        let canonical = if is_virtual(&target) { target } else { target.canonicalize().unwrap_or(target) };
        if is_virtual(&canonical) || canonical.is_dir() {
            self.back_history.push(self.current_path.clone());
            self.forward_history.clear();
            if !is_virtual(&canonical) {
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
            if self.settings.remember_view_per_folder && self.chooser.is_none()
                && let Some(&saved_mode) = self.settings.folder_views.get(&self.current_path) {
                    self.view_mode = saved_mode;
                }
        }
        self.path_input = self.current_path.to_string_lossy().to_string();
        self.selected_index = None;
        self.drag_select = None;
        self.last_rubberband = None;
        self.search_query.clear();
        self.search_root = None;
        self.search.cancel();
        self.show_search = false;
        self.is_editing_path = false;
        self.reload_items();
    }

    /// Showing Recent or Bookmarks, which list items from many folders, rather than a folder.
    pub fn in_recent(&self) -> bool {
        is_virtual(&self.current_path)
    }

    fn in_bookmarks(&self) -> bool {
        self.current_path.as_os_str() == BOOKMARKS_PATH
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
        self.set_toast(tr("Refreshed"));
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
            Ok(mut items) => {
                if let Some(ch) = &self.chooser {
                    items.retain(|i| if ch.spec.mode == crate::chooser::Mode::Folder { i.is_dir } else { i.is_dir || ch.accepts(&i.path) });
                }
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

    /// The files and folders used lately, most recent first, or the bookmarked ones in the Bookmarks view.
    fn recent_items(&self) -> Vec<FileItem> {
        const LIMIT: usize = 150;
        let paths = if self.in_bookmarks() { self.bookmarks.clone() } else { recent::load() };
        paths
            .into_iter()
            .filter(|p| self.show_hidden || !p.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')))
            .filter_map(|p| FileItem::from_path(&p).filter(|_| p.symlink_metadata().is_ok()))
            .filter(|i| self.type_filter.accepts(i))
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
                        self.set_toast(trf("Failed to start {}: {}", &[&app.name, &err]));
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
            self.set_toast(trf("Failed to open file: {}", &[&err]));
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
            many => trn(many.len(), "{} item", "{} items"),
        }
    }

    fn set_clipboard(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, cut: bool) {
        if paths.is_empty() {
            return;
        }
        let text = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        ctx.copy_text(text);
        let what = Self::describe_count(&paths);
        let message = if cut { trf("Cut {}", &[&what]) } else { trf("Copied {}", &[&what]) };
        self.clipboard = Some((paths, cut));
        self.set_toast(message);
    }

    fn paste(&mut self) {
        let Some((paths, cut)) = self.clipboard.clone() else {
            self.set_toast(tr("Nothing to paste"));
            return;
        };
        if !self.can_modify_here() {
            self.set_toast(tr("Open a regular folder to paste into"));
            return;
        }
        let dest = self.current_path.clone();
        if cut {
            let dest_canon = dest.canonicalize().unwrap_or_else(|_| dest.clone());
            let same_folder = paths.iter().any(|p| {
                p.parent()
                    .map(|parent| parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf()) == dest_canon)
                    .unwrap_or(false)
            });
            if same_folder {
                let any_dir = paths.iter().any(|p| p.is_dir());
                let (title, body) = if paths.len() == 1 && any_dir {
                    (
                        tr("Can't move a folder over itself").to_owned(),
                        tr("The source folder is already in this folder and would overwrite itself.").to_owned(),
                    )
                } else if paths.len() == 1 {
                    (
                        tr("Can't move a file over itself").to_owned(),
                        tr("The source file is already in this folder and would overwrite itself.").to_owned(),
                    )
                } else {
                    (
                        tr("Can't move items over themselves").to_owned(),
                        tr("The source items are already in this folder and would overwrite themselves.").to_owned(),
                    )
                };
                self.dialog = Some(Dialog::Alert { title, body });
                return;
            }
        }
        self.transfer(if cut { TransferOp::Move } else { TransferOp::Copy }, paths, dest);
        if cut {
            self.clipboard = None;
        }
    }

    /// Copy, move or extract `items` into `dest` on a worker thread.
    fn transfer(&mut self, op: TransferOp, items: Vec<PathBuf>, dest: PathBuf) {
        let label = Self::describe_count(&items);
        let target = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dest.display().to_string());
        let detail = trf("to “{}”", &[&target]);
        match op {
            TransferOp::Extract => {
                let title = trf("Extracting {}…", &[&label]);
                self.engine.spawn(JobKind::Extract, title, detail, move |w| {
                    w.shared.files_total.store(items.len() as u64, std::sync::atomic::Ordering::Relaxed);
                    let mut failures = Vec::new();
                    for item in &items {
                        if w.shared.checkpoint().is_err() {
                            break;
                        }
                        if let Err(err) = ops::extract(item, &dest) {
                            failures.push(format!("{}: {err}", item.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                        }
                        w.shared.files_done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    if failures.is_empty() { Outcome::done(trf("Extracted {} to “{}”", &[&label, &target])) } else { Outcome::failed(failures.join(" · ")) }
                });
            }
            TransferOp::Copy | TransferOp::Move => {
                let pairs: Vec<(PathBuf, PathBuf)> = items.iter().filter_map(|p| p.file_name().map(|n| (p.clone(), dest.join(n)))).collect();
                let (kind, engine_op, title) = if op == TransferOp::Copy {
                    (JobKind::Copy, Op::Copy, trf("Copying {}", &[&label]))
                } else {
                    (JobKind::Move, Op::Move, trf("Moving {}", &[&label]))
                };
                self.engine.spawn(kind, title, detail, move |w| engine::transfer(w, engine_op, pairs));
            }
        }
    }

    fn extract_beside(&mut self, archives: Vec<PathBuf>) {
        let archives: Vec<PathBuf> = archives.into_iter().filter(|p| ops::is_archive(p)).collect();
        if archives.is_empty() {
            return;
        }
        let label = Self::describe_count(&archives);
        let title = trf("Extracting {}…", &[&label]);
        self.engine.spawn(JobKind::Extract, title, String::new(), move |w| {
            use std::sync::atomic::Ordering::Relaxed;
            w.shared.files_total.store(archives.len() as u64, Relaxed);
            let mut failures = Vec::new();
            let mut made = Vec::new();
            for archive in &archives {
                if w.shared.checkpoint().is_err() {
                    break;
                }
                match ops::extract_beside(archive) {
                    Ok(dir) => made.push(dir),
                    Err(err) => failures.push(format!("{}: {err}", archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
                }
                w.shared.files_done.fetch_add(1, Relaxed);
            }
            let effect = if made.is_empty() {
                Effect::None
            } else {
                Effect::Record(UndoEntry::new(trf("Extraction of {}", &[&label]), made.iter().cloned().map(Edit::Trash).collect()))
            };
            if failures.is_empty() {
                let folder = made.last().and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
                Outcome::done(if archives.len() == 1 { trf("Extracted to “{}”", &[&folder]) } else { trf("Extracted {}", &[&label]) }).with(effect)
            } else {
                Outcome::failed(failures.join(" · ")).with(effect)
            }
        });
    }

    fn open_compress_dialog(&mut self, sources: Vec<PathBuf>) {
        let base = self.current_path.clone();
        let sources: Vec<PathBuf> = sources.into_iter().filter(|p| p.parent() == Some(base.as_path())).collect();
        if sources.is_empty() {
            self.set_toast(tr("Open the folder first to compress its files"));
            return;
        }
        let name = match sources.as_slice() {
            [one] => ops::archive_stem(one),
            _ => base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tr("Archive").to_owned()),
        };
        self.dialog = Some(Dialog::Compress { sources, name, format: ArchiveFormat::Zip, first_frame: true });
    }

    fn open_pick_folder(&mut self, op: TransferOp, items: Vec<PathBuf>) {
        let dir = self.current_path.clone();
        let mut picker = Picker { op, path_input: String::new(), dir: PathBuf::new(), items, chosen: None, folders: Vec::new(), icon_for: None, images: Vec::new() };
        picker.enter(dir);
        self.picker = Some(picker);
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
                    let label = id.and_then(|id| apps::installed().iter().find(|a| a.id == id).map(|a| trf("Open with {}", &[&a.name])));
                    let _ = tx.send(label);
                    ctx.request_repaint();
                });
                Some(rx)
            }
            _ => None,
        };
        material::reset_anim(ctx, egui::Id::unique("context_menu_anim"), 0.0);
        self.menu = Some(ContextMenu { pos, target, open_label: None, label_rx });
    }

    fn toggle_main_menu(&mut self, ctx: &egui::Context) {
        if matches!(self.menu.as_ref().map(|m| &m.target), Some(MenuTarget::Main)) {
            self.menu = None;
        } else {
            let rect = self.main_button_rect;
            self.open_menu(ctx, egui::pos2(rect.left(), rect.bottom() + POPUP_GAP), MenuTarget::Main);
        }
    }

    /// Another Ski window on the current folder.
    fn new_window(&mut self) {
        let here = self.current_path.clone();
        self.open_window(&here);
    }

    /// Another Ski window on `path`.
    fn open_window(&mut self, path: &Path) {
        let started = std::env::current_exe()
            .and_then(|exe| std::process::Command::new(exe).arg(path).spawn());
        if let Err(err) = started {
            self.set_toast(trf("Could not open a new window: {}", &[&err]));
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
        theme.border_radius = crate::ui::theme::BASE_RADIUS * self.settings.corner_scale;
        if theme != self.target_theme {
            self.from_theme = self.theme;
            self.target_theme = theme;
            self.theme_dirty = true;
        }
    }

    /// Re-apply theme / font / scale to egui after a change, and write changed preferences to disk.
    fn sync_settings(&mut self, ctx: &egui::Context) {
        crate::i18n::set_language(crate::i18n::resolve(&self.settings.language));
        material::set_animations(self.settings.animations);
        ctx.all_styles_mut(|s| s.animation_time = material::dur(material::ANIM));

        let now = ctx.input(|i| i.time);
        let dur_s = material::dur(material::ANIM);
        if self.theme_dirty {
            self.theme_dirty = false;
            if dur_s <= 0.0 {
                self.theme = self.target_theme;
                self.from_theme = self.target_theme;
                self.theme_anim_start = None;
                crate::ui::theme::apply(ctx, &self.theme);
            } else {
                self.theme_anim_start = Some(now);
            }
        }
        if let Some(start) = self.theme_anim_start {
            let t = if dur_s > 0.0 { ((now - start) as f32 / dur_s).clamp(0.0, 1.0) } else { 1.0 };
            let eased = material::quint_out(t);
            self.theme = self.from_theme.lerp(&self.target_theme, eased);
            crate::ui::theme::apply(ctx, &self.theme);
            if t >= 1.0 {
                self.theme = self.target_theme;
                self.from_theme = self.target_theme;
                self.theme_anim_start = None;
            } else {
                ctx.request_repaint();
            }
        }
        if self.fonts_dirty {
            crate::ui::fonts::install(ctx, self.settings.font.as_deref());
            self.fonts_dirty = false;
        }
        if (ctx.zoom_factor() - self.settings.scale).abs() > f32::EPSILON {
            ctx.set_zoom_factor(self.settings.scale);
        }
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

    /// Step the icon size of the grid, or the row height of the list.
    fn zoom_icons(&mut self, delta: i32) {
        let message = match self.view_mode {
            ViewMode::Grid => {
                let next = self.settings.grid_size.step(delta);
                if next == self.settings.grid_size {
                    return;
                }
                self.settings.grid_size = next;
                trf("Icons {}", &[&next.label()])
            }
            ViewMode::List => {
                let next = self.settings.density.step(delta);
                if next == self.settings.density {
                    return;
                }
                self.settings.density = next;
                trf("{} rows", &[&next.label()])
            }
        };
        self.set_toast(message);
    }

    fn close_search(&mut self) {
        self.show_search = false;
        if !self.search_query.is_empty() {
            self.search_query.clear();
            self.search_root = None;
            self.search.cancel();
            self.reload_items();
        }
    }

    /// Search the whole home folder; results replace the folder listing until the query is cleared.
    fn on_query_changed(&mut self, ctx: &egui::Context) {
        if self.search_query.trim().is_empty() {
            self.search_root = None;
            self.search.cancel();
            self.reload_items();
        } else {
            if self.search_root.is_none() {
                self.search_root = Some(self.current_path.clone());
            }
            self.items.clear();
            self.selected_index = None;
            self.sync_selection(true);
            self.error_message = None;
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            let here = self.search_root.clone().filter(|p| p.is_dir() && !trash::contains(p));
            let root = if self.settings.search_scope == SearchScope::ThisFolder { here.unwrap_or(home) } else { home };
            let options = SearchOptions { match_case: self.settings.search_case, contents: self.settings.search_contents };
            self.search.start(ctx, root, self.search_query.trim().to_owned(), self.show_hidden, self.type_filter, options);
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
        if dropped.is_empty() || self.chooser.is_some() {
            return;
        }
        if self.can_modify_here() {
            let dest = self.current_path.clone();
            self.transfer(TransferOp::Copy, dropped, dest);
        } else {
            self.set_toast(tr("Open a regular folder to drop files into"));
        }
    }

    /// Name of the place being shown, for the window title.
    fn location_name(&self) -> String {
        if !self.search_query.trim().is_empty() {
            tr("Search").to_owned()
        } else if self.in_bookmarks() {
            tr("Bookmarks").to_owned()
        } else if self.in_recent() {
            tr("Recent").to_owned()
        } else if trash::is_trash_folder(&self.current_path) {
            tr("Trash").to_owned()
        } else {
            self.current_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tr("Computer").to_owned())
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
            && !matches!(self.dialog, Some(Dialog::Conflict { .. }))
            && (self.dialog.is_some() || self.menu.take().is_some()) {
                if let Some(d) = self.dialog.take() {
                    self.closing_dialog = Some(d);
                }
                return;
            }
        if self.dialog.is_some() || self.menu.is_some() {
            return;
        }
        // The operations popover keeps the keyboard to itself, apart from the key that toggles it
        if self.jobs_popover {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || self.is_act_pressed(ctx, crate::keymap::Action::ShowJobs) {
                self.jobs_popover = false;
            }
            return;
        }

        // A widget the user reached with Tab owns Enter and Space, so they don't also open the selection
        if material::focus_visible(ctx)
            && ctx.memory(|m| m.focused()).is_some()
            && ctx.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space))
        {
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

        if self.is_act_pressed(ctx, crate::keymap::Action::ZoomIn) {
            self.zoom_icons(1);
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::ZoomOut) {
            self.zoom_icons(-1);
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::ToggleHidden) {
            self.show_hidden = !self.show_hidden;
            self.reload_items();
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::PinFolder) {
            let picked: Vec<PathBuf> = self.selection.iter().filter_map(|i| self.items.get(*i)).map(|i| i.path.clone()).collect();
            if !picked.is_empty() && !self.in_trash() {
                for path in picked {
                    self.toggle_pin(path);
                }
            } else if self.in_recent() {
                self.set_toast(tr("Open a folder to pin it"));
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

        if !self.picking() && self.is_act_pressed(ctx, crate::keymap::Action::Rename) {
            match self.selected_paths().as_slice() {
                [] => {}
                [one] => self.open_rename(one.clone()),
                many => self.open_batch_rename(many.to_vec()),
            }
            handled = true;
        }

        if self.picking() {
        } else if self.is_act_pressed(ctx, crate::keymap::Action::DeleteForever) {
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

        if !self.picking() && self.is_act_pressed(ctx, crate::keymap::Action::NewFile) {
            self.open_create_dialog(false);
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::GoHome) {
            if let Some(home) = dirs::home_dir() {
                self.navigate_to(home);
            }
            handled = true;
        }

        if !self.picking() && self.is_act_pressed(ctx, crate::keymap::Action::Undo) {
            self.undo();
            handled = true;
        }
        if !self.picking() && self.is_act_pressed(ctx, crate::keymap::Action::Redo) {
            self.redo();
            handled = true;
        }

        if !self.picking() && self.is_act_pressed(ctx, crate::keymap::Action::Duplicate) {
            self.duplicate(self.selected_paths());
            handled = true;
        }
        if !self.picking() && self.is_act_pressed(ctx, crate::keymap::Action::MakeLink) {
            self.make_links(self.selected_paths());
            handled = true;
        }
        if !self.picking() && self.is_act_pressed(ctx, crate::keymap::Action::OpenTerminal) {
            let here = self.current_path.clone();
            self.open_terminal(&here);
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::InvertSelection) {
            self.invert_selection();
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::SelectNone) {
            self.deselect_all();
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::SelectPattern) {
            self.open_select_pattern();
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::ShowJobs) && !self.engine.jobs.is_empty() {
            self.jobs_popover = !self.jobs_popover;
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::ContextMenu) {
            self.open_context_menu_here(ctx);
            handled = true;
        }

        if self.is_act_pressed(ctx, crate::keymap::Action::NewTab) {
            let here = self.current_path.clone();
            self.new_tab(here);
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::CloseTab) {
            self.close_tab(ctx, self.active_tab);
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::NextTab) {
            self.cycle_tab(1);
            handled = true;
        }
        if self.is_act_pressed(ctx, crate::keymap::Action::PrevTab) {
            self.cycle_tab(-1);
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
                self.set_toast(trf("Copied path: {}", &[&item.name]));
            }
            handled = true;
        }

        if self.may_create_folder() && self.is_act_pressed(ctx, crate::keymap::Action::NewFolder) {
            self.open_create_dialog(true);
            handled = true;
        }

        if !self.picking() && self.is_act_pressed(ctx, crate::keymap::Action::NewWindow) {
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
                event(|e| matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_))),
            )
        });
        if self.picking() {
        } else if ev_copy || self.is_act_pressed(ctx, crate::keymap::Action::Copy) {
            self.set_clipboard(ctx, self.selected_paths(), false);
            handled = true;
        } else if ev_cut || self.is_act_pressed(ctx, crate::keymap::Action::Cut) {
            self.set_clipboard(ctx, self.selected_paths(), true);
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
/// Narrowest the path pill may get before header buttons start to drop out.
const TITLE_MIN: f32 = 110.0;
/// Outer gutter shared by the sidebar, header, list and status bar so their edges line up.
const PAGE_MARGIN: i8 = 10;
/// Side margin of the header row: puts the round end buttons' icons on the sidebar's icon column.
const HEADER_MARGIN: i8 = PAGE_MARGIN + HEADER_INSET as i8;
const HEADER_INSET: f32 = material::INSET_ICON - material::CONTROL_HEIGHT / 2.0;
/// Where a row's icon starts inside its rounded background; the status bar text lines up with it.
const CONTENT_INSET: i8 = 16;
/// Space between the navigation buttons and the address pill, and between it and the right buttons.
const TITLE_GAP: f32 = 8.0;
/// Space between a header button and the menu or popup that hangs from it.
const POPUP_GAP: f32 = 6.0;
/// Breadcrumb buttons sit as far from the pill's left end as from its top and bottom, so the
/// first icon and label land on the sidebar's icon and label columns.
const CRUMB_HEIGHT: f32 = 26.0;
const CRUMB_INSET: f32 = (material::CONTROL_HEIGHT - CRUMB_HEIGHT) / 2.0;
const CRUMB_ICON_STEP: f32 = material::INSET_LABEL - material::INSET_ICON + 8.0;

impl eframe::App for SkiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.resume_search(&ctx);
        if let Some(dir) = self.pending_open.take() {
            self.dialog = None;
            self.navigate_to(dir);
        }
        material::track_input_mode(&ctx, self.dialog.is_some() || self.menu.is_some() || self.jobs_popover);
        let escape_is_taken = self.dialog.is_some() || self.menu.is_some() || self.jobs_popover || self.picker.is_some()
            || self.show_settings || self.capturing_key || self.is_editing_path || !self.search_query.is_empty() || !self.selection.is_empty();
        self.handle_keyboard_inputs(&ctx);
        self.chooser_escape(&ctx, escape_is_taken);
        self.sync_selection(false);
        self.poll_jobs();
        self.poll_drives(&ctx);
        self.drop_zones.clear();
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

        if let Some(toast) = &mut self.status_toast {
            let lifetime = if toast.action.is_some() { 7.0 } else { 3.0 };
            if !toast.leaving && toast.since.elapsed().as_secs_f32() > lifetime {
                toast.leaving = true;
            }
        }

        let window_width = ui.available_width();
        let sidebar_visible = self.settings.show_sidebar && (window_width >= SIDEBAR_BREAKPOINT);
        let sidebar_t = material::animate(ui.ctx(), egui::Id::unique("sidebar_panel_anim"), sidebar_visible);

        if sidebar_t > 0.001 {
            egui::Panel::left("sidebar_places")
                .exact_size((212.0 * sidebar_t).max(1.0))
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(self.theme.mantle)
                        .inner_margin(egui::Margin::symmetric(PAGE_MARGIN, 8)),
                )
                .show(ui, |ui| {
                    ui.set_opacity(sidebar_t);
                    self.render_sidebar(ui);
                });
        }

        egui::Panel::top("top_navigation")
            .frame(
                egui::Frame::new()
                    .fill(self.theme.base)
                    .inner_margin(egui::Margin::symmetric(HEADER_MARGIN, 8)),
            )
            .show(ui, |ui| self.render_top_bar(ui, sidebar_visible));
        self.render_tab_bar(ui);

        self.render_chooser_bar(ui);

        let central = egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(self.theme.base)
                    .inner_margin(egui::Margin { left: PAGE_MARGIN, right: PAGE_MARGIN, top: 0, bottom: 6 }),
            )
            .show(ui, |ui| {
                let background = ui.interact(ui.max_rect(), ui.make_persistent_id("list_background"), egui::Sense::click_and_drag());
                if background.secondary_clicked() && self.search_query.trim().is_empty() && !self.in_recent()
                    && let Some(pos) = background.interact_pointer_pos() {
                        self.open_menu(ui.ctx(), pos, MenuTarget::Folder);
                    }
                if background.clicked() && self.drag_select.is_none() && !self.selection.is_empty() {
                    self.selection.clear();
                    self.selected_index = None;
                    self.anchor = None;
                    self.last_cursor = None;
                }

                let pinch = ui.input(|i| i.zoom_delta());
                if pinch != 1.0 && ui.rect_contains_pointer(ui.max_rect()) {
                    self.zoom_accum += pinch.ln();
                    if self.zoom_accum.abs() >= 0.12 {
                        let delta = if self.zoom_accum > 0.0 { 1 } else { -1 };
                        self.zoom_accum = 0.0;
                        self.zoom_icons(delta);
                    }
                }

                let show_filter_bar = !self.search_query.trim().is_empty() || self.type_filter != TypeFilter::All;
                material::collapsible(ui, "filter_bar_collapse", show_filter_bar, |ui| {
                    self.render_filter_bar(ui);
                });

                if let Some(err) = &self.error_message {
                    material::empty_state(ui, &self.theme, icons::INFO, tr("This folder can't be opened"), Some(err));
                } else if self.items.is_empty() {
                    let (icon, title, hint) = if !self.search_query.trim().is_empty() {
                        if self.search.is_running() {
                            (icons::SEARCH, tr("Searching…"), None)
                        } else {
                            (icons::SEARCH_OFF, tr("No results"), Some(if self.type_filter == TypeFilter::All { tr("Try a different name") } else { tr("Try a different name or file type") }))
                        }
                    } else if self.in_bookmarks() {
                        (icons::HEART, tr("No bookmarks yet"), Some(tr("Mark files and folders with a heart to find them here")))
                    } else if self.in_recent() {
                        (icons::RECENT, tr("Nothing recent yet"), Some(tr("Files you open show up here")))
                    } else if self.in_trash() {
                        (icons::TRASH, tr("The trash is empty"), Some(tr("Deleted files end up here")))
                    } else if self.type_filter != TypeFilter::All {
                        (icons::SEARCH_OFF, tr("Nothing matches"), Some(tr("Try another file type in the view options")))
                    } else {
                        (icons::FOLDER, tr("This folder is empty"), Some(tr("Right-click to create a file or folder")))
                    };
                    material::empty_state(ui, &self.theme, icon, title, hint);
                } else {
                    let enter_id = ui.make_persistent_id(("view_enter", &self.current_path, self.view_mode as u8));
                    let enter_t = material::animate_enter(ui.ctx(), enter_id, material::ANIM_FAST);
                    ui.scope(|ui| {
                        ui.set_opacity(enter_t);
                        match self.view_mode {
                            ViewMode::List => self.render_list_view(ui),
                            ViewMode::Grid => self.render_grid_view(ui),
                        }
                    });
                }
            });
        self.render_selection_pill(&ctx, central.response.rect);

        self.render_settings(&ctx);
        self.render_jobs_popover(&ctx);
        self.render_menu(&ctx);
        self.render_dialog(&ctx);
        self.render_picker(&ctx);
        self.render_toast(&ctx);
        self.render_drop_overlay(&ctx);
        self.update_item_drag(&ctx);
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
                // Every gap is spelled out below, so nothing depends on the layout's own spacing
                ui.spacing_mut().item_spacing.x = 0.0;

                // Controls leave one by one as the window narrows, like in Nautilus: the view button first,
                // then search, then back / forward. The path and the main menu stay.
                let nav_w = 2.0 * CONTROL_HEIGHT + TITLE_GAP;
                let button_w = CONTROL_HEIGHT + gap;
                let (mut show_view, mut show_search_btn, mut show_nav) = (true, !sidebar_visible, true);
                let needed = |view: bool, search: bool, nav: bool| {
                    let menu = if sidebar_visible { 0.0 } else { button_w };
                    TITLE_MIN + TITLE_GAP + menu + if view { button_w } else { 0.0 } + if search { button_w } else { 0.0 } + if nav { nav_w } else { 0.0 }
                };
                let room = ui.available_width();
                if needed(show_view, show_search_btn, show_nav) > room {
                    show_view = false;
                }
                if needed(show_view, show_search_btn, show_nav) > room {
                    show_search_btn = false;
                }
                if needed(show_view, show_search_btn, show_nav) > room {
                    show_nav = false;
                }

                if show_nav {
                    let (back, forward) = material::nav_pair(ui, &self.theme, !self.back_history.is_empty(), !self.forward_history.is_empty());
                    if back.clicked() {
                        self.go_back();
                    }
                    if forward.clicked() {
                        self.go_forward();
                    }
                    // The address pill breathes: a wider gap on both sides than between the icon buttons
                    ui.add_space(TITLE_GAP);
                }

                // Search lives in the sidebar; without a sidebar it becomes a header button.
                let show_jobs = !self.engine.jobs.is_empty();
                let jobs_w = if show_jobs { button_w } else { 0.0 };
                let right_w = jobs_w + if sidebar_visible { 0.0 } else { button_w } + if show_search_btn { button_w } else { 0.0 } + if show_view { CONTROL_HEIGHT } else { 0.0 };
                let title_width = (ui.available_width() - right_w - TITLE_GAP).max(40.0);

                let (title_rect, title_resp) =
                    ui.allocate_exact_size(egui::vec2(title_width, CONTROL_HEIGHT), egui::Sense::click());
                ui.add_space(TITLE_GAP);

                let editing_top_pill = self.show_search || self.is_editing_path;
                let pill_hover = material::animate(ui.ctx(), title_resp.id.with("hover"), !editing_top_pill && title_resp.hovered());
                let pill_edit = material::animate(ui.ctx(), egui::Id::unique("top_pill_edit"), editing_top_pill);
                let pill_fill = material::lerp_color(
                    material::lerp_color(self.theme.surface0, self.theme.surface1, pill_hover),
                    self.theme.surface2,
                    pill_edit,
                );
                let pill_radius = if self.show_search { egui::CornerRadius::same(material::radius_md()) } else { material::pill(title_rect) };
                ui.painter().rect_filled(material::snap(ui.painter(), title_rect), pill_radius, pill_fill);

                if self.show_search {
                    self.render_search_field(ui, title_rect);
                } else if self.is_editing_path {
                    self.render_path_field(ui, title_rect);
                } else {
                    self.render_title(ui, title_rect, &title_resp);
                }

                if show_search_btn {
                    let icon = if self.show_search { icons::CLOSE } else { icons::SEARCH };
                    if icon_button(ui, "btn_search", icon, &self.theme, true, tr("Search (/)")).clicked() {
                        if self.show_search {
                            self.close_search();
                        } else {
                            self.show_search = true;
                            self.focus_search = true;
                            self.is_editing_path = false;
                        }
                    }
                    ui.add_space(gap);
                }
                if !sidebar_visible {
                    let menu = icon_button(ui, "btn_main_menu", icons::MENU, &self.theme, true, tr("Main menu"));
                    self.main_button_rect = menu.rect;
                    if menu.clicked() {
                        self.toggle_main_menu(ui.ctx());
                    }
                    ui.add_space(gap);
                }

                if show_jobs {
                    self.jobs_button(ui);
                    ui.add_space(gap);
                }
                if !show_view {
                    return;
                }
                let view_icon = match self.view_mode {
                    ViewMode::List => icons::VIEW_LIST,
                    ViewMode::Grid => icons::GRID_VIEW,
                };
                let view = icon_button(ui, "btn_view_options", view_icon, &self.theme, true, tr("View options"));
                self.settings_button_rect = view.rect;
                let filter_dot_t = material::animate(ui.ctx(), egui::Id::unique("view_filter_dot"), self.type_filter != TypeFilter::All);
                if filter_dot_t > 0.01 {
                    let dot = egui::pos2(view.rect.right() - 8.0, view.rect.top() + 8.0);
                    ui.painter().circle_filled(dot, 4.0 * filter_dot_t, self.theme.mauve.gamma_multiply(filter_dot_t));
                }
                if view.clicked() {
                    self.show_settings = !self.show_settings;
                }
            },
        );
    }

    fn render_search_field(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        material::paint_icon(
            ui.painter(),
            egui::pos2(rect.left() + material::INSET_ICON, rect.center().y),
            icons::SEARCH,
            material::ICON_MD,
            self.theme.subtext1,
        );
        let edit_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + material::INSET_LABEL, rect.top()),
            egui::pos2(rect.right() - 40.0, rect.bottom()),
        );
        let clear_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 20.0, rect.center().y), egui::vec2(26.0, 26.0));
        let clear_id = ui.make_persistent_id("search_clear");
        let clear = ui.interact(clear_rect, clear_id, egui::Sense::click());
        let fx = material::fx(ui, clear_id, &clear, true, &material::FxSpec::ICON);
        let r = material::scaled(clear_rect, fx.scale);
        material::paint_fill(ui.painter(), r, material::pill(r), material::row_layer(&self.theme, &fx), &fx);
        material::paint_icon(
            ui.painter(),
            clear_rect.center(),
            icons::CLOSE,
            material::ICON_XS,
            material::lerp_color(self.theme.subtext1, self.theme.text, fx.hover.max(fx.press)),
        );
        if clear.clicked() {
            self.close_search();
            return;
        }
        let resp = material::put_over(
            ui,
            edit_rect,
            egui::TextEdit::singleline(&mut self.search_query)
                .frame(egui::Frame::NONE)
                .align(egui::Align2::LEFT_CENTER)
                .font(egui::FontId::proportional(material::FONT_BODY))
                .hint_text(tr("Search files")),
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
        let resp = material::put_over(
            ui,
            edit_rect,
            egui::TextEdit::singleline(&mut self.path_input)
                .frame(egui::Frame::NONE)
                .align(egui::Align2::LEFT_CENTER)
                .font(egui::FontId::proportional(material::FONT_BODY))
                .hint_text(tr("Type a path and press Enter")),
        );
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }
        // The whole path is selected when editing starts, so typing replaces it
        if !self.path_field_open {
            self.path_field_open = true;
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                let end = self.path_input.chars().count();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(end))));
                state.store(ui.ctx(), resp.id);
            }
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
        self.path_field_open = false;
        let theme = self.theme;
        let cy = rect.center().y;

        let searching = !self.search_query.trim().is_empty();
        if searching || self.in_recent() {
            let (icon, text) = if searching {
                (icons::SEARCH, trf("Results for “{}”", &[&self.search_query.trim()]))
            } else if self.in_bookmarks() {
                (icons::HEART, tr("Bookmarks").to_owned())
            } else {
                (icons::RECENT, tr("Recent").to_owned())
            };
            material::paint_icon(ui.painter(), egui::pos2(rect.left() + material::INSET_ICON, cy), icon, material::ICON_MD, theme.subtext1);
            let mut job = egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(material::FONT_SUBTITLE), theme.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - material::INSET_LABEL - 16.0).max(10.0));
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(egui::pos2(rect.left() + material::INSET_LABEL, cy - galley.size().y / 2.0), galley, theme.text);
            return;
        }

        // Breadcrumbs: every folder on the way is a button; leading ones fold into an ellipsis when short on room
        let mut crumbs = self.crumbs();
        let font = egui::FontId::proportional(material::FONT_SUBTITLE);
        let measure = |ui: &egui::Ui, c: &Crumb| {
            let text_w = ui.painter().layout_no_wrap(c.label.clone(), font.clone(), theme.text).size().x;
            text_w + if c.icon.is_some() { CRUMB_ICON_STEP } else { 0.0 } + 20.0
        };
        let mut widths: Vec<f32> = crumbs.iter().map(|c| measure(ui, c)).collect();
        let (sep, avail) = (20.0, rect.width() - 2.0 * CRUMB_INSET);
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

        let mut x = rect.left() + CRUMB_INSET;
        let mut go: Option<std::path::PathBuf> = None;
        let mut edit = false;
        let crumb_button = |ui: &mut egui::Ui, id: egui::Id, cell: egui::Rect, current: bool| {
            let resp = ui.interact(cell, id, egui::Sense::click());
            let fx = material::fx(ui, id, &resp, true, &material::FxSpec::CLICK);
            let r = material::scaled(cell, fx.scale);
            material::paint_fill(ui.painter(), r, material::pill(r), material::row_layer(&theme, &fx), &fx);
            (resp, material::lerp_color(if current { theme.text } else { theme.subtext1 }, theme.text, fx.hover.max(fx.press)))
        };
        let chevron = |ui: &egui::Ui, x: f32| {
            material::paint_icon(ui.painter(), egui::pos2(x + sep / 2.0, cy), icons::CHEVRON_RIGHT, material::ICON_XS, theme.with_alpha(theme.subtext1, 140));
        };

        if let Some(target) = folded_to {
            let cell = egui::Rect::from_min_size(egui::pos2(x, cy - CRUMB_HEIGHT / 2.0), egui::vec2(32.0, CRUMB_HEIGHT));
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
        let mut crumb_zones = Vec::new();
        for (i, (crumb, w)) in crumbs.iter().zip(&widths).enumerate() {
            let cell = egui::Rect::from_min_size(egui::pos2(x, cy - CRUMB_HEIGHT / 2.0), egui::vec2(*w, CRUMB_HEIGHT));
            let (resp, color) = crumb_button(ui, ui.make_persistent_id(("crumb", &crumb.path)), cell, i == last);
            crumb_zones.push(dnd::DropZone { rect: cell, dest: crumb.path.clone() });
            let mut tx = cell.left() + 10.0;
            if let Some(icon) = crumb.icon {
                material::paint_icon(ui.painter(), egui::pos2(tx + 8.0, cy), icon, material::ICON_MD, color);
                tx += CRUMB_ICON_STEP;
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

        self.drop_zones.extend(crumb_zones);
        if let Some(path) = go {
            self.navigate_to(path);
        } else if edit || field.clicked() {
            self.is_editing_path = true;
            self.path_input = self.current_path.to_string_lossy().to_string();
        }
    }

    /// File-type chips above the results: they narrow a search, a folder or the recent files.
    fn render_filter_bar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            ui.add_space(f32::from(CONTENT_INSET));
            for filter in TypeFilter::ALL {
                if chip(ui, ("bar_type", filter.label()), None, filter.label(), &theme, self.type_filter == filter).clicked() {
                    picked = Some(filter);
                }
            }
        });
        let mut options_changed = false;
        if !self.search_query.trim().is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                ui.add_space(f32::from(CONTENT_INSET));
                for (scope, label) in [(SearchScope::ThisFolder, tr("This folder")), (SearchScope::Everywhere, tr("Everywhere"))] {
                    if chip(ui, ("bar_scope", label), None, label, &theme, self.settings.search_scope == scope).clicked() && self.settings.search_scope != scope {
                        self.settings.search_scope = scope;
                        options_changed = true;
                    }
                }
                if chip(ui, "bar_case", None, tr("Match case"), &theme, self.settings.search_case).clicked() {
                    self.settings.search_case = !self.settings.search_case;
                    options_changed = true;
                }
                if chip(ui, "bar_contents", None, tr("File contents"), &theme, self.settings.search_contents).clicked() {
                    self.settings.search_contents = !self.settings.search_contents;
                    options_changed = true;
                }
            });
        }
        ui.add_space(2.0);
        if options_changed && picked.is_none() {
            let ctx = ui.ctx().clone();
            self.on_query_changed(&ctx);
        }
        if let Some(filter) = picked {
            self.type_filter = filter;
            if self.search_query.trim().is_empty() {
                self.reload_items();
            } else {
                let ctx = ui.ctx().clone();
                self.on_query_changed(&ctx);
            }
        }
    }

    /// The folders leading to the current one, starting at Home, the trash or the root.
    fn crumbs(&self) -> Vec<Crumb> {
        let canon = |p: PathBuf| p.canonicalize().unwrap_or(p);
        let path = &self.current_path;
        let trash_root = trash::files_dir().map(canon);
        let home = dirs::home_dir().map(canon);
        let (root, icon, name) = if let Some(t) = trash_root.filter(|t| path.starts_with(t)) {
            (t, icons::TRASH, tr("Trash"))
        } else if let Some(h) = home.filter(|h| path.starts_with(h)) {
            (h, icons::HOME, tr("Home"))
        } else {
            (PathBuf::from("/"), icons::STORAGE, tr("Computer"))
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
        // The search row is followed by the usual item spacing, which counts towards the 8px
        ui.add_space(8.0 - ui.spacing().item_spacing.y);

        let browsing = self.search_query.trim().is_empty();
        let mut destination: Option<PathBuf> = None;
        let mut new_tab_at: Option<PathBuf> = None;
        let mut mount: Option<(String, String)> = None;
        let mut zones: Vec<dnd::DropZone> = Vec::new();
        let mut open_pinned: Option<PathBuf> = None;
        let mut reorder: Option<(usize, usize)> = None;
        let mut eject: Option<(String, String, PathBuf)> = None;
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
                    let mut rects = Vec::with_capacity(self.pins.len());
                    for (idx, pin) in self.pins.iter().enumerate() {
                        let is_dir = pin.is_dir();
                        let selected = browsing && is_dir && self.current_path == *pin;
                        let name = pin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| pin.display().to_string());
                        let icon = if is_dir { icons::FOLDER } else { FileItem::from_path(pin).map_or(icons::FILE, |i| i.file_type.icon()) };
                        let resp = nav_item(ui, ("pin", pin), icon, &name, &self.theme, selected);
                        rects.push(resp.rect);
                        if is_dir {
                            zones.push(dnd::DropZone { rect: resp.rect, dest: pin.clone() });
                        }
                        if resp.clicked() {
                            if is_dir { destination = Some(pin.clone()) } else { open_pinned = Some(pin.clone()) }
                        }
                        if resp.middle_clicked() && is_dir {
                            new_tab_at = Some(pin.clone());
                        }
                        if resp.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            if let Some(p) = resp.interact_pointer_pos()
                                && let Some(over) = self.pin_rects.iter().position(|r| p.y >= r.top() && p.y < r.bottom())
                                && over != idx {
                                    reorder = Some((idx, over));
                                }
                        }
                        if resp.secondary_clicked()
                            && let Some(pos) = resp.interact_pointer_pos() {
                                menu_at = Some((pos, MenuTarget::Pin(pin.clone())));
                            }
                    }
                    self.pin_rects = rects;
                    continue;
                }
                for (idx, place) in places {
                    let here = match place.kind {
                        PlaceKind::Recent => self.in_recent() && !self.in_bookmarks(),
                        PlaceKind::Bookmarks => self.in_bookmarks(),
                        PlaceKind::Folder => place.drive.as_ref().is_none_or(|d| d.mounted) && self.current_path == place.path,
                    };
                    // Removable drives keep room for the eject button, so the name does not run under it
                    let reserve = if place.drive.as_ref().is_some_and(|d| d.mounted && d.removable) { material::NAV_TRAILING } else { 0.0 };
                    let resp = material::nav_item_with(ui, ("place", idx), place.icon, &tr_dyn(&place.name), &self.theme, browsing && here, reserve);
                    if place.kind == PlaceKind::Folder && place.drive.as_ref().is_none_or(|d| d.mounted) {
                        zones.push(dnd::DropZone { rect: resp.rect, dest: place.path.clone() });
                    }
                    if let Some(drive) = &place.drive {
                        let busy = self.drive_state.busy.contains(&drive.dev);
                        if busy {
                            ui.ctx().request_repaint();
                        }
                        if drive.mounted && drive.removable && !busy
                            && drives_ui::row_eject_button(ui, resp.rect, resp.hovered(), ("eject", idx), &self.theme) {
                            eject = Some((drive.dev.clone(), place.name.clone(), place.path.clone()));
                            continue;
                        }
                        if resp.clicked() {
                            if drive.mounted {
                                destination = Some(place.path.clone());
                            } else if !busy {
                                mount = Some((drive.dev.clone(), place.name.clone()));
                            }
                        }
                        if resp.middle_clicked() && drive.mounted {
                            new_tab_at = Some(place.path.clone());
                        }
                        if resp.secondary_clicked()
                            && let Some(pos) = resp.interact_pointer_pos() {
                                menu_at = Some((pos, MenuTarget::Drive(drive.clone(), place.name.clone(), place.path.clone())));
                            }
                        continue;
                    }
                    if resp.clicked() {
                        destination = Some(place.path.clone());
                    }
                    if resp.middle_clicked() && place.kind == PlaceKind::Folder {
                        new_tab_at = Some(place.path.clone());
                    }
                    if resp.secondary_clicked()
                        && place.kind == PlaceKind::Folder
                        && let Some(pos) = resp.interact_pointer_pos() {
                            menu_at = Some((pos, if trash::is_trash_folder(&place.path) { MenuTarget::TrashPlace(place.path.clone()) } else { MenuTarget::Place(place.path.clone()) }));
                        }
                }
            }
        });

        self.drop_zones.extend(zones);
        if let Some((from, to)) = reorder {
            let pin = self.pins.remove(from);
            self.pins.insert(to, pin);
            Place::save_pins(&self.pins);
        }
        if let Some(path) = open_pinned {
            self.open_file(&path);
        }
        if let Some((pos, target)) = menu_at {
            self.open_menu(ui.ctx(), pos, target);
        }
        if let Some(dest) = destination {
            self.close_search();
            self.navigate_to(dest);
        }
        if let Some(path) = new_tab_at {
            self.new_tab(path);
        }
        if let Some((dev, name)) = mount {
            self.mount_drive(ui.ctx(), dev, name, true);
        }
        if let Some((dev, name, path)) = eject {
            self.unmount_drive(ui.ctx(), dev, name, path, true);
        }
    }

    /// Search button (the field itself lives in the header) with the main menu button at the right.
    fn render_sidebar_search(&mut self, ui: &mut egui::Ui) {
        let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), CONTROL_HEIGHT), egui::Sense::hover());
        // Round buttons are inset so their icons sit on the same column as the destinations' icons
        let menu_rect = egui::Rect::from_min_size(egui::pos2(row.right() - HEADER_INSET - CONTROL_HEIGHT, row.top()), egui::vec2(CONTROL_HEIGHT, CONTROL_HEIGHT));
        let menu = ui.scope_builder(egui::UiBuilder::new().max_rect(menu_rect), |ui| {
            icon_button(ui, "sidebar_main_menu", icons::MENU, &self.theme, true, tr("Main menu"))
        });
        self.main_button_rect = menu.inner.rect;
        if menu.inner.clicked() {
            self.toggle_main_menu(ui.ctx());
        }

        let search_rect = egui::Rect::from_min_size(egui::pos2(row.left() + HEADER_INSET, row.top()), egui::vec2(CONTROL_HEIGHT, CONTROL_HEIGHT));
        let button = ui.scope_builder(egui::UiBuilder::new().max_rect(search_rect), |ui| {
            icon_button(ui, "sidebar_search_btn", icons::SEARCH, &self.theme, true, tr("Search (/)"))
        });
        if button.inner.clicked() {
            if self.show_search {
                self.close_search();
            } else {
                self.show_search = true;
                self.focus_search = true;
                self.is_editing_path = false;
            }
        }
    }

    fn open_item(&mut self, item: FileItem) {
        if self.chooser.is_some() && !item.is_dir {
            self.chooser_activate(&item);
            return;
        }
        if item.is_dir {
            if self.settings.open_in_new_window && self.chooser.is_none() {
                let started = std::env::current_exe()
                    .and_then(|exe| std::process::Command::new(exe).arg(&item.path).spawn());
                if let Err(err) = started {
                    self.set_toast(trf("Could not open a new window: {}", &[&err]));
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
        let row_height = material::animate_value(ui.ctx(), egui::Id::unique("list_row_height"), self.settings.density.row_height(), material::ANIM);
        let show_thumbs = self.settings.show_thumbnails;

        if !self.is_mixed_listing()
            && let Some(key) = material::list_header(ui, &self.theme, &self.columns, &mut self.settings.column_widths, self.sort_by, self.sort_order) {
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
        let mut new_tab_at: Option<PathBuf> = None;
        let mut menu_at: Option<(egui::Pos2, MenuTarget)> = None;
        let mut item_rects: Vec<(usize, egui::Rect)> = Vec::with_capacity(self.items.len());
        let mut scroll_delta = 0.0;
        let tab_scroll_id = ("files_scroll", self.tabs[self.active_tab].id);
        let scroll_out = egui::ScrollArea::vertical().id_salt(tab_scroll_id).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (idx, item) in self.items.iter().enumerate() {
                let selected = self.selection.contains(&idx);
                let is_cut = cut.contains(&item.path);
                let hint = self.path_hint(item);
                let thumb = thumb_for(&self.thumbs, &self.folder_icons, item, show_thumbs);
                let resp = file_row(ui, idx, item, &self.theme, selected, is_cut, should_scroll, thumb, hint.as_deref(), &self.columns, &self.settings.column_widths, row_height);
                item_rects.push((idx, resp.rect));
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
                if resp.middle_clicked() && item.is_dir {
                    new_tab_at = Some(item.path.clone());
                }
                if resp.secondary_clicked()
                    && let Some(pos) = resp.interact_pointer_pos() {
                        menu_at = Some((pos, MenuTarget::Item(item.clone())));
                    }
            }
            if let Some(drag) = &self.drag_select
                && drag.active
            {
                scroll_delta = Self::drag_auto_scroll(ui, ui.clip_rect(), drag.current_screen.y);
            }
        });

        self.last_item_rects = item_rects.clone();
        let drag_consumed = self.handle_drag_select(ui, scroll_out.inner_rect, scroll_out.state.clamped_offset().y, scroll_delta, &item_rects);

        if !drag_consumed {
            if let Some((idx, modifiers)) = clicked {
                self.click_select(idx, modifiers);
            }
            if let Some(item) = opened {
                self.open_item(item);
            }
        }
        if let Some((pos, target)) = menu_at {
            self.open_menu(ui.ctx(), pos, target);
        }
        if let Some(path) = new_tab_at {
            self.new_tab(path);
        }
    }

    fn render_grid_view(&mut self, ui: &mut egui::Ui) {
        let mut opened: Option<FileItem> = None;
        let should_scroll = std::mem::take(&mut self.scroll_to_selected);
        let (target_w, target_h) = self.settings.grid_size.cell_size();
        let cell_w = material::animate_value(ui.ctx(), egui::Id::unique("grid_cell_w"), target_w, material::ANIM);
        let cell_h = material::animate_value(ui.ctx(), egui::Id::unique("grid_cell_h"), target_h, material::ANIM);
        let cell = egui::vec2(cell_w, cell_h);
        let grid_size = self.settings.grid_size;
        let grid_lines = self.settings.grid_name_lines as usize;
        let show_thumbs = self.settings.show_thumbnails;

        let cut: &[PathBuf] = match &self.clipboard {
            Some((paths, true)) => paths,
            _ => &[],
        };
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut new_tab_at: Option<PathBuf> = None;
        let mut menu_at: Option<(egui::Pos2, MenuTarget)> = None;
        let mut item_rects: Vec<(usize, egui::Rect)> = Vec::with_capacity(self.items.len());
        let mut scroll_delta = 0.0;
        let tab_scroll_id = ("files_scroll", self.tabs[self.active_tab].id);
        let scroll_out = egui::ScrollArea::vertical().id_salt(tab_scroll_id).auto_shrink([false, false]).show(ui, |ui| {
            let spacing = 4.0;
            let cols = ((ui.available_width() + spacing) / (cell.x + spacing)).floor().max(1.0) as usize;
            // Leftover width is shared between the columns, so the grid fills the view like Nautilus does
            let gap = ((ui.available_width() - cols as f32 * cell.x) / cols as f32).max(spacing);
            // Every tile sits at a position computed from its row and column, never from its neighbours
            let rows = self.items.len().div_ceil(cols);
            let (_, area) = ui.allocate_space(egui::vec2(ui.available_width(), rows as f32 * (cell.y + spacing)));

            for (row_idx, row) in self.items.chunks(cols).enumerate() {
                for (col_idx, item) in row.iter().enumerate() {
                    let origin = egui::pos2(
                        area.left() + gap / 2.0 + col_idx as f32 * (cell.x + gap),
                        area.top() + row_idx as f32 * (cell.y + spacing),
                    );
                    let slot = egui::Rect::from_min_size(origin, cell);
                    let idx = row_idx * cols + col_idx;
                    let selected = self.selection.contains(&idx);
                    let is_cut = cut.contains(&item.path);
                    let thumb = thumb_for(&self.thumbs, &self.folder_icons, item, show_thumbs);
                    let resp = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| {
                            file_tile(ui, idx, item, &self.theme, selected, is_cut, should_scroll, thumb, grid_size, grid_lines)
                        })
                        .inner;
                    item_rects.push((idx, resp.rect));
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
                    if resp.middle_clicked() && item.is_dir {
                        new_tab_at = Some(item.path.clone());
                    }
                    if resp.secondary_clicked()
                        && let Some(pos) = resp.interact_pointer_pos() {
                            menu_at = Some((pos, MenuTarget::Item(item.clone())));
                        }
                }
        }
            if let Some(drag) = &self.drag_select
                && drag.active
            {
                scroll_delta = Self::drag_auto_scroll(ui, ui.clip_rect(), drag.current_screen.y);
            }
        });

        self.last_item_rects = item_rects.clone();
        let drag_consumed = self.handle_drag_select(ui, scroll_out.inner_rect, scroll_out.state.clamped_offset().y, scroll_delta, &item_rects);

        if !drag_consumed {
            if let Some((idx, modifiers)) = clicked {
                self.click_select(idx, modifiers);
            }
            if let Some(item) = opened {
                self.open_item(item);
            }
        }
        if let Some((pos, target)) = menu_at {
            self.open_menu(ui.ctx(), pos, target);
        }
        if let Some(path) = new_tab_at {
            self.new_tab(path);
        }
    }

    fn drag_auto_scroll(ui: &mut egui::Ui, viewport: egui::Rect, pointer_y: f32) -> f32 {
        let edge_zone = 36.0;
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let max_speed = 650.0;
        let dy = if pointer_y < viewport.top() + edge_zone {
            let t = ((viewport.top() + edge_zone - pointer_y) / edge_zone).clamp(0.0, 2.5);
            t * max_speed * dt
        } else if pointer_y > viewport.bottom() - edge_zone {
            let t = ((pointer_y - (viewport.bottom() - edge_zone)) / edge_zone).clamp(0.0, 2.5);
            -t * max_speed * dt
        } else {
            0.0
        };
        if dy != 0.0 {
            ui.scroll_with_delta_animation(
                egui::vec2(0.0, dy),
                egui::style::ScrollAnimation::none(),
            );
            ui.ctx().request_repaint();
        }
        dy
    }

    /// Rubber-band drag selection from any point inside the file listing (empty space or item).
    /// Returns `true` when an active drag just finished or is in progress, so single clicks on the
    /// starting item are suppressed.
    fn handle_drag_select(
        &mut self,
        ui: &mut egui::Ui,
        viewport: egui::Rect,
        scroll_y: f32,
        _scroll_delta: f32,
        item_rects: &[(usize, egui::Rect)],
    ) -> bool {
        if self.dialog.is_some() || self.menu.is_some() || self.show_settings {
            self.drag_select = None;
            self.last_rubberband = None;
            self.dnd = None;
            return false;
        }
        self.dnd_view = viewport;

        let (pressed, down, released, pos, mods) = ui.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Primary),
                i.pointer.button_down(egui::PointerButton::Primary),
                i.pointer.button_released(egui::PointerButton::Primary),
                i.pointer.latest_pos(),
                i.modifiers,
            )
        });

        // Leave room at the right edge so dragging the scrollbar thumb never starts a rubberband
        let interactive_viewport = egui::Rect::from_min_max(
            viewport.min,
            egui::pos2((viewport.right() - 12.0).max(viewport.left()), viewport.bottom()),
        );

        if pressed
            && let Some(p) = pos
            && interactive_viewport.contains(p)
            && !mods.command
            && !mods.shift
            && self.begin_item_drag(ui, p, item_rects)
        {
            self.drag_select = None;
        } else if pressed
            && let Some(p) = pos
            && interactive_viewport.contains(p)
        {
            let base_selection = if mods.command || mods.shift {
                self.selection.clone()
            } else {
                BTreeSet::new()
            };
            self.drag_select = Some(DragSelect {
                start_content: egui::pos2(p.x, p.y + scroll_y),
                current_screen: p,
                base_selection,
                active: false,
            });
        }

        let mut is_active = false;
        let mut was_active = false;

        if let Some(mut drag) = self.drag_select.take() {
            if let Some(p) = pos {
                drag.current_screen = p;
            }

            let start_screen = egui::pos2(drag.start_content.x, drag.start_content.y - scroll_y);
            if !drag.active && drag.current_screen.distance(start_screen) >= 5.0 {
                drag.active = true;
            }

            if drag.active {
                let box_screen = egui::Rect::from_two_pos(start_screen, drag.current_screen);
                let mut next_selection = drag.base_selection.clone();
                let mut first_hit = None;
                let mut last_hit = None;

                for &(idx, rect) in item_rects {
                    if box_screen.intersects(rect) {
                        if first_hit.is_none() {
                            first_hit = Some(idx);
                        }
                        last_hit = Some(idx);
                        if mods.command && drag.base_selection.contains(&idx) {
                            next_selection.remove(&idx);
                        } else {
                            next_selection.insert(idx);
                        }
                    }
                }

                self.selection = next_selection;
                self.selected_index = last_hit.or_else(|| self.selection.iter().next_back().copied());
                self.anchor = first_hit.or(self.selected_index);
                self.last_cursor = self.selected_index;

                let clipped = box_screen.intersect(viewport);
                if clipped.is_positive() {
                    self.last_rubberband = Some(clipped);
                }
            }

            was_active = drag.active;
            if down && !released {
                is_active = drag.active;
                self.drag_select = Some(drag);
            }
        }

        let rb_t = material::animate_for(ui.ctx(), egui::Id::unique("rubberband_fade"), is_active, material::ANIM_FAST);
        if rb_t > 0.01
            && let Some(rect) = self.last_rubberband
        {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("rubberband_overlay"),
            ));
            material::paint_rubberband_faded(&painter, rect, &self.theme, rb_t);
        } else if !is_active {
            self.last_rubberband = None;
        }

        was_active || self.item_drag_active()
    }

    /// One quiet line under the listing: what is shown or selected, and what waits in the clipboard.
    /// What the selection holds, e.g. "2 folders, 5 files selected · 11.8 GB"; `None` when nothing is selected.
    fn selection_summary(&self) -> Option<String> {
        let selected: Vec<&FileItem> = self.selection.iter().filter_map(|i| self.items.get(*i)).collect();
        if selected.is_empty() {
            return None;
        }
        let folders = selected.iter().filter(|i| i.is_dir).count();
        let files = selected.len() - folders;
        let mut parts = Vec::new();
        if folders > 0 {
            parts.push(trn(folders, "{} folder", "{} folders"));
        }
        if files > 0 {
            parts.push(trn(files, "{} file", "{} files"));
        }
        let mut text = trf("{} selected", &[&parts.join(", ")]);
        if files > 0 {
            let size: u64 = selected.iter().filter(|i| !i.is_dir).map(|i| i.size).sum();
            text.push_str(&format!(" · {}", crate::fs::file_item::format_bytes(size)));
        }
        Some(text)
    }

    /// Floating pill over the lower left of the listing, there only while something is selected;
    /// a second one on the right while files wait to be pasted. Clicks pass through both.
    fn render_selection_pill(&mut self, ctx: &egui::Context, area: egui::Rect) {
        let theme = self.theme;
        let summary = self.selection_summary().filter(|_| self.settings.show_status_bar);
        if let Some(text) = &summary {
            self.pill_text.clone_from(text);
        }
        let t = material::animate(ctx, egui::Id::unique("selection_pill"), summary.is_some());
        let pill = |id: &'static str, pos: egui::Pos2, align: egui::Align2, t: f32, icon: Option<&str>, text: &str| {
            egui::Area::new(egui::Id::unique(id))
                .order(egui::Order::Foreground)
                .pivot(align)
                .fixed_pos(pos + egui::vec2(0.0, (1.0 - t) * 6.0))
                .interactable(false)
                .show(ctx, |ui| {
                    ui.set_opacity(t);
                    egui::Frame::new()
                        .fill(theme.surface1)
                        .corner_radius(egui::CornerRadius::same(255))
                        .shadow(ctx.global_style().visuals.popup_shadow)
                        .inner_margin(egui::Margin::symmetric(14, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                if let Some(icon) = icon {
                                    material::inline_icon(ui, icon, material::ICON_XS, theme.subtext1);
                                }
                                ui.label(egui::RichText::new(text).size(material::FONT_CAPTION).color(theme.text));
                            });
                        });
                });
        };
        if t > 0.001 {
            pill("selection_pill_area", area.left_bottom() + egui::vec2(14.0, -12.0), egui::Align2::LEFT_BOTTOM, t, None, &self.pill_text);
        }
        if let Some((paths, cut)) = &self.clipboard {
            let (label, icon) = if *cut { (trf("{} ready to move", &[&paths.len()]), icons::CUT) } else { (trf("{} ready to copy", &[&paths.len()]), icons::COPY) };
            pill("clipboard_pill_area", area.right_bottom() + egui::vec2(-14.0, -12.0), egui::Align2::RIGHT_BOTTOM, 1.0, Some(icon), &label);
        }
    }

    /// While files are dragged over the window: a tint and a hint about where they will land.
    fn render_drop_overlay(&self, ctx: &egui::Context) {
        let active = !ctx.input(|i| i.raw.hovered_files.is_empty()) && self.dialog.is_none();
        let t = material::animate(ctx, egui::Id::unique("drop_overlay_anim"), active);
        if t <= 0.0 {
            return;
        }
        let theme = self.theme;
        let screen = ctx.content_rect();
        let target = if self.can_modify_here() { trf("Drop to copy into “{}”", &[&self.location_name()]) } else { tr("Open a regular folder to drop files").to_owned() };
        egui::Area::new(egui::Id::unique("drop_overlay")).order(egui::Order::Foreground).fixed_pos(screen.min).interactable(false).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0, theme.tint((20.0 * t).round() as u8));
            let galley = ui.painter().layout_no_wrap(target, egui::FontId::proportional(material::FONT_SUBTITLE), theme.text.gamma_multiply(t));
            let scale = material::lerp(0.94, 1.0, t);
            let pill = egui::Rect::from_center_size(
                rect.center() + egui::vec2(0.0, (1.0 - t) * 8.0),
                (galley.size() + egui::vec2(48.0, 24.0)) * scale,
            );
            ui.painter().rect_filled(pill, material::pill(pill), theme.surface2.gamma_multiply(t));
            ui.painter().galley(pill.center() - galley.size() / 2.0, galley, theme.text.gamma_multiply(t));
        });
    }

    fn render_toast(&mut self, ctx: &egui::Context) {
        let open = self.status_toast.as_ref().is_some_and(|t| !t.leaving);
        let t = material::animate(ctx, egui::Id::unique("toast_anim"), open);
        let Some(toast) = &self.status_toast else { return };
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
                    .corner_radius(egui::CornerRadius::same(material::radius_lg()))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin { left: 20, right, top: 6, bottom: 6 })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.set_min_height(material::BUTTON_HEIGHT);
                            ui.label(egui::RichText::new(message).size(material::FONT_BODY).color(theme.text));
                            if has_action {
                                ui.add_space(8.0);
                                act = material::button(ui, "toast_action", None, tr("Undo"), &theme, ButtonKind::Text, true).clicked();
                            }
                        });
                    });
            });
        if act
            && let Some(toast) = self.status_toast.as_mut()
            && let Some(ToastAction::Undo) = toast.action.take() {
                toast.leaving = true;
                self.undo();
            }
        ctx.request_repaint_after(std::time::Duration::from_millis(400));
    }

    fn render_settings(&mut self, ctx: &egui::Context) {
        let t = material::animate(ctx, egui::Id::unique("settings_anim"), self.show_settings);
        if t <= 0.0 {
            return;
        }

        let screen = ctx.content_rect();
        let width = (screen.width() - 24.0).clamp(240.0, 300.0);
        let max_height = (screen.height() - 80.0).max(160.0);
        let mut reload = false;

        // Fades in while sliding down from the button; closing plays the same motion backwards.
        let area = egui::Area::new(egui::Id::unique("settings_popup"))
            .order(egui::Order::Foreground)
            // Pinned by its top-right corner at a known width, so the first frame (size not yet
            // measured) lands where the later ones do. It never rises above the button, or it
            // would swallow clicks meant for the button while closing.
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(egui::pos2(
                self.settings_button_rect.right(),
                self.settings_button_rect.bottom() + POPUP_GAP - (1.0 - t) * 8.0,
            ))
            .interactable(self.show_settings)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .corner_radius(egui::CornerRadius::same(material::radius_container()))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(width - 20.0);
                        // An Area only offers the height it had last frame, which would pin scrolling content short
                        ui.set_max_height(max_height - 20.0);
                        egui::ScrollArea::vertical().max_height(max_height - 20.0).show(ui, |ui| {
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
        let hidden_hint = crate::keymap::hint(&self.settings.keybinds, crate::keymap::Action::ToggleHidden, self.settings.vim_keys);
        ui.spacing_mut().item_spacing.y = 2.0;

        if let Some(i) = segmented(ui, "vo_view", &[tr("List"), tr("Grid")], usize::from(self.view_mode == ViewMode::Grid), &theme) {
            self.view_mode = if i == 0 { ViewMode::List } else { ViewMode::Grid };
        }

        match self.view_mode {
            ViewMode::Grid => {
                let size = self.settings.grid_size;
                let (less, more) = material::stepper_row(ui, "vo_icon_size", tr("Icon size"), &theme, &size.label(), size.can_step(-1), size.can_step(1));
                if less || more {
                    self.settings.grid_size = self.settings.grid_size.step(if more { 1 } else { -1 });
                }
            }
            ViewMode::List => {
                let at = Density::ALL.iter().position(|d| *d == self.settings.density).unwrap_or(1);
                let (less, more) = material::stepper_row(ui, "vo_row_height", tr("Row height"), &theme, self.settings.density.label(), at > 0, at + 1 < Density::ALL.len());
                if less || more {
                    self.settings.density = Density::ALL[if more { at + 1 } else { at - 1 }];
                }
            }
        }

        material::divider(ui, &theme);
        self.section_title(ui, tr("Sort by"));
        for (field, label) in [(SortBy::Name, tr("Name")), (SortBy::Size, tr("Size")), (SortBy::Modified, tr("Last modified"))] {
            if material::radio_row(ui, ("vo_sort", label), label, &theme, self.sort_by == field).clicked() && self.sort_by != field {
                self.sort_by = field;
                reload = true;
            }
        }
        let descending = self.sort_order == SortOrder::Descending;
        if material::toggle_row(ui, "vo_reverse", tr("Reverse order"), None, &theme, descending).clicked() {
            self.sort_order = if descending { SortOrder::Ascending } else { SortOrder::Descending };
            reload = true;
        }

        material::divider(ui, &theme);
        if material::toggle_row(ui, "vo_hidden", tr("Hidden files"), hidden_hint.as_deref(), &theme, self.show_hidden).clicked() {
            self.show_hidden = !self.show_hidden;
            reload = true;
        }
        if material::toggle_row(ui, "vo_folders_first", tr("Folders first"), None, &theme, self.folders_first).clicked() {
            self.folders_first = !self.folders_first;
            reload = true;
        }

        material::collapsible(ui, "vo_list_columns", self.view_mode == ViewMode::List, |ui| {
            material::divider(ui, &theme);
            self.section_title(ui, tr("List columns"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
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
        });

        material::divider(ui, &theme);
        self.section_title(ui, tr("File type"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
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
        self.file_options(ui)
    }

    fn shortcuts_contents(&mut self, ui: &mut egui::Ui) -> bool {
        let theme = self.theme;
        let close = material::dialog_header(ui, tr("Keyboard shortcuts"), None, &theme, "sc_close");
        let max_height = (ui.ctx().content_rect().height() - 200.0).clamp(200.0, 540.0);
        egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
            for group in [tr("Navigation"), tr("Files"), tr("View")] {
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
                            ui.set_min_height(material::ENTRY_HEIGHT);
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
            self.set_toast(tr("Unpinned from sidebar"));
        } else if self.places.iter().any(|p| p.path == path) {
            self.set_toast(tr("Already in the sidebar"));
            return;
        } else {
            self.pins.push(path);
            self.set_toast(tr("Pinned to sidebar"));
        }
        Place::save_pins(&self.pins);
    }

    fn toggle_bookmark(&mut self, path: PathBuf) {
        if let Some(idx) = self.bookmarks.iter().position(|p| *p == path) {
            self.bookmarks.remove(idx);
            self.set_toast(tr("Removed from bookmarks"));
        } else {
            self.bookmarks.push(path);
            self.set_toast(tr("Added to bookmarks"));
        }
        Place::save_bookmarks(&self.bookmarks);
        if self.in_bookmarks() {
            self.reload_items();
        }
    }

    fn open_properties(&mut self, path: &Path) {
        match Properties::load(path) {
            Some(props) => {
                let scan = props.item.is_dir.then(|| FolderScan::start(path.to_path_buf()));
                self.dialog = Some(Dialog::Properties { props: Box::new(props), scan, tab: 0 });
            }
            None => self.set_toast(tr("Could not read properties")),
        }
    }

    fn open_create_dialog(&mut self, is_dir: bool) {
        if !self.can_modify_here() {
            self.set_toast(tr("Open a regular folder to create files in it"));
            return;
        }
        let templates = if is_dir { Vec::new() } else { templates::load(self.settings.templates_dir.as_deref()) };
        let base = if is_dir { tr("New Folder") } else { templates.first().map_or(tr("Untitled"), |t| t.file_name.as_str()) };
        let name = self.free_name(base);
        self.dialog = Some(Dialog::Create { is_dir, name, error: None, first_frame: true, templates, template: 0, edited: false });
    }

    /// `name` if nothing in the folder has it, otherwise `name 2`, `name 3`… (before the extension).
    fn free_name(&self, name: &str) -> String {
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name, ""),
        };
        let mut candidate = name.to_owned();
        let mut n = 2;
        while self.current_path.join(&candidate).symlink_metadata().is_ok() {
            candidate = format!("{stem} {n}{ext}");
            n += 1;
        }
        candidate
    }

    fn create_entry(&mut self, is_dir: bool, name: &str, template: Option<&Template>) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err(tr("Enter a valid name").to_owned());
        }
        let path = self.current_path.join(name);
        let result = if is_dir {
            std::fs::create_dir(&path)
        } else if let Some(template) = template {
            templates::create(template, &path)
        } else {
            std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map(|_| ())
        };
        result.map_err(|err| match err.kind() {
            std::io::ErrorKind::AlreadyExists => trf("“{}” already exists", &[&name]),
            _ => err.to_string(),
        })?;

        self.reload_items();
        if let Some(idx) = self.items.iter().position(|i| i.name == name) {
            self.selected_index = Some(idx);
            self.scroll_to_selected = true;
        }
        self.history.record(UndoEntry::new(trf("Creation of {}", &[&name]), vec![Edit::Trash(path)]));
        self.set_toast_action(trf("Created {}", &[&name]), ToastAction::Undo);
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
            return Err(tr("Enter a valid name").to_owned());
        }
        if path.file_name().is_some_and(|n| n.to_string_lossy() == new_name) {
            return Ok(());
        }
        let target = path.with_file_name(new_name);
        if target.symlink_metadata().is_ok() {
            return Err(trf("“{}” already exists", &[&new_name]));
        }
        std::fs::rename(path, &target).map_err(|err| err.to_string())?;

        self.retarget_path(path, &target);

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
        self.history.record(UndoEntry::new(trf("Renaming of {}", &[&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()]), vec![Edit::Move { from: target.clone(), to: path.to_path_buf() }]));
        self.set_toast_action(trf("Renamed to “{}”", &[&new_name]), ToastAction::Undo);
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
            self.set_toast(trf("Could not move to trash: {}", &[&failed.join(" · ")]));
            return;
        }
        let mut message = trf("Moved {} to trash", &[&label]);
        if !failed.is_empty() {
            message.push_str(&format!(" · {}", trf("{} failed", &[&failed.len()])));
        }
        self.history.record(UndoEntry::new(trf("Move of {} to trash", &[&label]), trashed.into_iter().map(Edit::Restore).collect()));
        self.set_toast_action(message, ToastAction::Undo);
    }

    fn restore_paths(&mut self, paths: Vec<PathBuf>) {
        let label = Self::describe_count(&paths);
        let mut failed = Vec::new();
        let mut restored = Vec::new();
        for path in &paths {
            match trash::restore(path) {
                Ok(back) => restored.push(back),
                Err(err) => failed.push(err.to_string()),
            }
        }
        self.selected_index = None;
        self.refresh_listing();
        if !restored.is_empty() {
            self.history.record(UndoEntry::new(trf("Restoring {}", &[&label]), restored.into_iter().map(Edit::Trash).collect()));
        }
        if failed.is_empty() {
            self.set_toast(trf("Restored {}", &[&label]));
        } else {
            self.set_toast(trf("Could not restore: {}", &[&failed.join(" · ")]));
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
            title: tr("Delete permanently?").to_owned(),
            body: trf("{} will be deleted for good. This can't be undone.", &[&Self::describe_count(&paths)]),
            confirm: tr("Delete").to_owned(),
            action: ConfirmAction::DeleteForever(paths),
        });
    }

    fn request_empty_trash(&mut self) {
        if trash::is_empty() {
            self.set_toast(tr("The trash is already empty"));
        } else if self.settings.confirm_delete {
            self.dialog = Some(Dialog::Confirm {
                title: tr("Empty the trash?").to_owned(),
                body: tr("Everything in the trash will be deleted for good. This can't be undone.").to_owned(),
                confirm: tr("Empty trash").to_owned(),
                action: ConfirmAction::EmptyTrash,
            });
        } else {
            self.empty_trash();
        }
    }

    fn delete_forever(&mut self, paths: Vec<PathBuf>) {
        let label = Self::describe_count(&paths);
        self.selected_index = None;
        let title = trf("Deleting {}", &[&label]);
        self.engine.spawn(JobKind::Delete, title, String::new(), move |w| engine::delete_forever(w, paths));
    }

    fn empty_trash(&mut self) {
        self.selected_index = None;
        let title = tr("Emptying the trash").to_owned();
        self.engine.spawn(JobKind::Delete, title, String::new(), |_| match trash::empty() {
            Ok(()) => Outcome::done(tr("The trash is empty").to_owned()),
            Err(err) => Outcome::failed(err.to_string()),
        });
    }

    fn run_action(&mut self, ctx: &egui::Context, action: MenuAction) {
        if !self.allows(&action) {
            return;
        }
        match action {
            MenuAction::Open(item) => self.open_item(item),
            MenuAction::OpenPath(path) => self.navigate_to(path),
            MenuAction::OpenInNewTab(path) => self.new_tab(path),
            MenuAction::OpenInNewWindow(path) => self.open_window(&path),
            MenuAction::CopyPath(path) => {
                ctx.copy_text(path.to_string_lossy().to_string());
                self.set_toast(tr("Copied path"));
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
            MenuAction::RenameMany(paths) => self.open_batch_rename(paths),
            MenuAction::Trash(paths) => self.trash_paths(paths),
            MenuAction::DeleteForever(paths) => self.request_delete_forever(paths),
            MenuAction::Restore(paths) => self.restore_paths(paths),
            MenuAction::EmptyTrash => self.request_empty_trash(),
            MenuAction::Pin(path) | MenuAction::Unpin(path) => self.toggle_pin(path),
            MenuAction::Bookmark(path) | MenuAction::Unbookmark(path) => self.toggle_bookmark(path),
            MenuAction::Submenu(_) => {}
            MenuAction::NewFile => self.open_create_dialog(false),
            MenuAction::NewFolder => self.open_create_dialog(true),
            MenuAction::Refresh => self.refresh(),
            MenuAction::NewWindow => self.new_window(),
            MenuAction::NewTab => {
                let here = self.current_path.clone();
                self.new_tab(here);
            }
            MenuAction::Undo => self.undo(),
            MenuAction::Duplicate(paths) => self.duplicate(paths),
            MenuAction::MakeLink(paths) => self.make_links(paths),
            MenuAction::OpenTerminal(path) => self.open_terminal(&path),
            MenuAction::InvertSelection => self.invert_selection(),
            MenuAction::SelectPattern => self.open_select_pattern(),
            MenuAction::Mount(dev, name) => self.mount_drive(ctx, dev, name, true),
            MenuAction::Unmount(dev, name, mount) => self.unmount_drive(ctx, dev, name, mount, false),
            MenuAction::Eject(dev, name, mount) => self.unmount_drive(ctx, dev, name, mount, true),
            MenuAction::Redo => self.redo(),
            MenuAction::OpenSettings => self.open_settings_dialog(),
            MenuAction::Shortcuts => self.dialog = Some(Dialog::Shortcuts),
            MenuAction::Copy(paths) => self.set_clipboard(ctx, paths, false),
            MenuAction::Cut(paths) => self.set_clipboard(ctx, paths, true),
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

    /// Whether this is the system file dialog, which only browses and chooses: no moving, renaming or deleting.
    fn picking(&self) -> bool {
        self.chooser.is_some()
    }

    /// Folders can be made in a dialog that saves or picks a folder, as in GTK, but not in one that opens files.
    fn may_create_folder(&self) -> bool {
        self.chooser.as_ref().is_none_or(|c| c.spec.mode != crate::chooser::Mode::Open)
    }

    fn allows(&self, action: &MenuAction) -> bool {
        if !self.picking() {
            return true;
        }
        match action {
            MenuAction::NewFolder => self.may_create_folder(),
            _ => matches!(
                action,
                MenuAction::Open(_)
                    | MenuAction::OpenPath(_)
                    | MenuAction::OpenInNewTab(_)
                    | MenuAction::CopyPath(_)
                    | MenuAction::Properties(_)
                    | MenuAction::Pin(_)
                    | MenuAction::Unpin(_)
                    | MenuAction::Bookmark(_)
                    | MenuAction::Unbookmark(_)
                    | MenuAction::SelectAll
                    | MenuAction::InvertSelection
                    | MenuAction::SelectPattern
                    | MenuAction::Refresh
                    | MenuAction::NewTab
                    | MenuAction::OpenSettings
                    | MenuAction::Shortcuts
                    | MenuAction::Mount(..)
                    | MenuAction::Unmount(..)
                    | MenuAction::Eject(..)
            ),
        }
    }

    /// Drops the entries a picking dialog does not allow, also inside submenus; emptied submenus go too.
    fn retain_allowed(&self, entries: &mut Vec<MenuEntry>) {
        for (_, _, action) in entries.iter_mut() {
            if let MenuAction::Submenu(sub) = action {
                self.retain_allowed(sub);
            }
        }
        entries.retain(|(_, _, action)| match action {
            MenuAction::Submenu(sub) => !sub.is_empty(),
            _ => self.allows(action),
        });
    }

    fn menu_entries(&self, target: &MenuTarget) -> Vec<Vec<MenuEntry>> {
        let mut groups = self.menu_entries_all(target);
        for group in &mut groups {
            self.retain_allowed(group);
        }
        groups
    }

    fn menu_entries_all(&self, target: &MenuTarget) -> Vec<Vec<MenuEntry>> {
        // Folders that already sit in the sidebar cannot be pinned again
        let pin_entry = |path: &PathBuf| -> Option<MenuEntry> {
            if self.pins.contains(path) {
                Some((icons::PIN_OFF, tr("Unpin from sidebar"), MenuAction::Unpin(path.clone())))
            } else if self.places.iter().any(|p| p.path == *path) {
                None
            } else {
                Some((icons::PIN, tr("Pin to sidebar"), MenuAction::Pin(path.clone())))
            }
        };
        let bookmark_entry = |path: &PathBuf| -> MenuEntry {
            if self.bookmarks.contains(path) {
                (icons::HEART, tr("Remove from bookmarks"), MenuAction::Unbookmark(path.clone()))
            } else {
                (icons::HEART_OUTLINE, tr("Add to bookmarks"), MenuAction::Bookmark(path.clone()))
            }
        };
        let icon_entry = |path: &PathBuf| (icons::IMAGE, tr("Custom icon…"), MenuAction::SetIcon(path.clone()));
        // Several entries fold into a submenu; a lone one stays where it is
        let fold = |icon: &'static str, label: &'static str, entries: Vec<MenuEntry>| -> Vec<MenuEntry> {
            match entries.len() {
                0 | 1 => entries,
                _ => vec![(icon, label, MenuAction::Submenu(entries))],
            }
        };
        let (in_trash, mixed) = (self.in_trash(), self.is_mixed_listing());
        // Clipboard / transfer / archive entries shared by every selection; `keep` leaves out
        // everything that would move or remove the items themselves
        let file_groups = |paths: Vec<PathBuf>, archives: bool, keep: bool| {
            let mut clipboard = vec![(icons::COPY, tr("Copy"), MenuAction::Copy(paths.clone()))];
            let mut manage = Vec::new();
            let mut transfer = vec![(icons::FOLDER_COPY, tr("Copy to…"), MenuAction::CopyTo(paths.clone()))];
            if !keep {
                clipboard.push((icons::CUT, tr("Cut"), MenuAction::Cut(paths.clone())));
                transfer.push((icons::FOLDER_MOVE, tr("Move to…"), MenuAction::MoveTo(paths.clone())));
                match paths.as_slice() {
                    [one] => manage.push((icons::RENAME, tr("Rename"), MenuAction::Rename(one.clone()))),
                    many => manage.push((icons::RENAME, tr("Rename…"), MenuAction::RenameMany(many.to_vec()))),
                }
                manage.push((icons::TRASH, tr("Move to trash"), MenuAction::Trash(paths.clone())));
            }
            let mut more = vec![
                (icons::DUPLICATE, tr("Duplicate"), MenuAction::Duplicate(paths.clone())),
                (icons::LINK, tr("Create link"), MenuAction::MakeLink(paths.clone())),
                (icons::COMPRESS, tr("Compress…"), MenuAction::Compress(paths.clone())),
            ];
            if archives {
                more.push((icons::EXTRACT, tr("Extract here"), MenuAction::ExtractHere(paths.clone())));
                more.push((icons::EXTRACT, tr("Extract to…"), MenuAction::ExtractTo(paths)));
            }
            manage.extend(transfer);
            manage.extend(fold(icons::MORE, tr("More actions"), more));
            vec![clipboard, manage]
        };
        // Results and Recent items live in many folders: offer a way to the one that holds them
        let open_group = |item: &FileItem, with: bool, show_parent: bool| {
            let mut group = vec![(icons::OPEN, tr("Open"), MenuAction::Open(item.clone()))];
            if item.is_dir {
                group.extend(fold(icons::NEW_TAB, tr("Open in"), vec![
                    (icons::NEW_TAB, tr("New tab"), MenuAction::OpenInNewTab(item.path.clone())),
                    (icons::NEW_WINDOW, tr("New window"), MenuAction::OpenInNewWindow(item.path.clone())),
                    (icons::TERMINAL, tr("Terminal"), MenuAction::OpenTerminal(item.path.clone())),
                ]));
            }
            if with {
                group.push((icons::APPS, tr("Open with…"), MenuAction::OpenWith(item.path.clone())));
            }
            if (mixed || show_parent) && let Some(parent) = item.path.parent() {
                group.push((icons::FOLDER, tr("Show in folder"), MenuAction::OpenPath(parent.to_path_buf())));
            }
            group
        };
        // The menu of one file or folder. `keep` is for sidebar places, which must not be moved or removed from here.
        let single_groups = |item: &FileItem, show_parent: bool, keep: bool| {
            let mut groups = vec![open_group(item, !item.is_dir, show_parent)];
            let mut mark = vec![bookmark_entry(&item.path)];
            if item.is_dir {
                let customize: Vec<MenuEntry> = pin_entry(&item.path).into_iter().chain([icon_entry(&item.path)]).collect();
                mark.extend(fold(icons::PIN, tr("Customize"), customize));
            } else {
                mark.extend(pin_entry(&item.path));
                if crate::thumbs::is_image(&item.path) {
                    mark.push((icons::IMAGE, tr("Use as folder icon"), MenuAction::UseAsIcon(item.path.clone())));
                }
            }
            groups.push(mark);
            groups.extend(file_groups(vec![item.path.clone()], !item.is_dir && ops::is_archive(&item.path), keep));
            groups.push(vec![
                (icons::COPY, tr("Copy path"), MenuAction::CopyPath(item.path.clone())),
                (icons::INFO, tr("Properties"), MenuAction::Properties(item.path.clone())),
            ]);
            groups
        };
        match target {
            MenuTarget::Item(item) if in_trash => {
                let paths: Vec<PathBuf> = self.menu_group(item).into_iter().map(|i| i.path).collect();
                vec![
                    vec![
                        (icons::RESTORE, tr("Restore"), MenuAction::Restore(paths.clone())),
                        (icons::DELETE_FOREVER, tr("Delete permanently"), MenuAction::DeleteForever(paths)),
                    ],
                    vec![(icons::INFO, tr("Properties"), MenuAction::Properties(item.path.clone()))],
                ]
            }
            MenuTarget::Item(item) if self.menu_group(item).len() > 1 => {
                let group = self.menu_group(item);
                let paths: Vec<PathBuf> = group.iter().map(|i| i.path.clone()).collect();
                let archives = group.iter().any(|i| !i.is_dir && ops::is_archive(&i.path));
                let mut groups = file_groups(paths, archives, false);
                groups.push(vec![(icons::INFO, tr("Properties"), MenuAction::Properties(item.path.clone()))]);
                groups
            }
            MenuTarget::Item(item) => single_groups(item, false, false),
            MenuTarget::Folder if in_trash => vec![
                vec![(icons::DELETE_FOREVER, tr("Empty trash"), MenuAction::EmptyTrash)],
                vec![(icons::SELECT_ALL, tr("Select all"), MenuAction::SelectAll)],
                vec![(icons::REFRESH, tr("Refresh"), MenuAction::Refresh)],
            ],
            MenuTarget::Folder => vec![
                vec![
                    (icons::FILE_PLUS, tr("New file"), MenuAction::NewFile),
                    (icons::FOLDER_PLUS, tr("New folder"), MenuAction::NewFolder),
                ],
                self.history_entries(),
                {
                    let mut edit = Vec::new();
                    if self.clipboard.is_some() {
                        edit.push((icons::PASTE, tr("Paste"), MenuAction::Paste));
                    }
                    edit.extend(fold(icons::SELECT_ALL, tr("Select"), vec![
                        (icons::SELECT_ALL, tr("Select all"), MenuAction::SelectAll),
                        (icons::SELECT_INVERSE, tr("Invert selection"), MenuAction::InvertSelection),
                        (icons::SELECT_PATTERN, tr("Select matching…"), MenuAction::SelectPattern),
                    ]));
                    edit
                },
                {
                    let here = &self.current_path;
                    let customize: Vec<MenuEntry> = pin_entry(here).into_iter().chain([icon_entry(here)]).collect();
                    let mut mark = vec![bookmark_entry(here)];
                    mark.extend(fold(icons::PIN, tr("Customize"), customize));
                    mark.push((icons::TERMINAL, tr("Open in terminal"), MenuAction::OpenTerminal(here.clone())));
                    mark
                },
                vec![(icons::REFRESH, tr("Refresh"), MenuAction::Refresh)],
                vec![
                    (icons::COPY, tr("Copy path"), MenuAction::CopyPath(self.current_path.clone())),
                    (icons::INFO, tr("Properties"), MenuAction::Properties(self.current_path.clone())),
                ],
            ],
            MenuTarget::Main => vec![
                vec![(icons::NEW_WINDOW, tr("New window"), MenuAction::NewWindow), (icons::NEW_TAB, tr("New tab"), MenuAction::NewTab)],
                self.history_entries(),
                vec![
                    (icons::REFRESH, tr("Refresh"), MenuAction::Refresh),
                    (icons::SETTINGS, tr("Settings"), MenuAction::OpenSettings),
                ],
                vec![(icons::KEYBOARD, tr("Keyboard shortcuts"), MenuAction::Shortcuts)],
            ],
            MenuTarget::TrashPlace(path) => vec![
                vec![(icons::OPEN, tr("Open"), MenuAction::OpenPath(path.clone()))],
                vec![(icons::DELETE_FOREVER, tr("Empty trash"), MenuAction::EmptyTrash)],
            ],
            MenuTarget::Place(path) => match FileItem::from_path(path) {
                Some(item) => single_groups(&item, false, true),
                None => vec![vec![(icons::OPEN, tr("Open"), MenuAction::OpenPath(path.clone()))]],
            },
            MenuTarget::Drive(drive, name, mount) => {
                let (dev, name) = (drive.dev.clone(), name.clone());
                if !drive.mounted {
                    vec![vec![(icons::USB, tr("Mount"), MenuAction::Mount(dev, name))]]
                } else {
                    let mut leave = vec![(icons::UNMOUNT, tr("Unmount"), MenuAction::Unmount(dev.clone(), name.clone(), mount.clone()))];
                    if drive.removable {
                        leave.push((icons::EJECT, tr("Eject"), MenuAction::Eject(dev, name, mount.clone())));
                    }
                    vec![
                        vec![
                            (icons::OPEN, tr("Open"), MenuAction::OpenPath(mount.clone())),
                            (icons::NEW_TAB, tr("Open in new tab"), MenuAction::OpenInNewTab(mount.clone())),
                            (icons::NEW_WINDOW, tr("Open in new window"), MenuAction::OpenInNewWindow(mount.clone())),
                        ],
                        leave,
                        vec![(icons::INFO, tr("Properties"), MenuAction::Properties(mount.clone()))],
                    ]
                }
            }
            MenuTarget::Pin(path) => match FileItem::from_path(path) {
                Some(item) => single_groups(&item, true, false),
                None => vec![vec![(icons::PIN_OFF, tr("Unpin from sidebar"), MenuAction::Unpin(path.clone()))]],
            },
        }
    }

    /// Undo and redo entries of the menus: only those that have something to do.
    fn history_entries(&self) -> Vec<MenuEntry> {
        let mut entries = Vec::new();
        if self.history.undo_label().is_some() {
            entries.push((icons::UNDO, tr("Undo"), MenuAction::Undo));
        }
        if self.history.redo_label().is_some() {
            entries.push((icons::REDO, tr("Redo"), MenuAction::Redo));
        }
        entries
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
            MenuAction::NewTab => A::NewTab,
            MenuAction::Duplicate(_) => A::Duplicate,
            MenuAction::MakeLink(_) => A::MakeLink,
            MenuAction::OpenTerminal(_) => A::OpenTerminal,
            MenuAction::InvertSelection => A::InvertSelection,
            MenuAction::SelectPattern => A::SelectPattern,
            MenuAction::Undo => A::Undo,
            MenuAction::Redo => A::Redo,
            MenuAction::OpenSettings => A::Settings,
            MenuAction::Shortcuts => A::ShowShortcuts,
            MenuAction::Rename(_) | MenuAction::RenameMany(_) => A::Rename,
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
            let mut groups = self.menu_entries(&target);
            groups.retain(|g| !g.is_empty());
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

        let hints: Vec<Vec<Option<String>>> =
            groups.iter().map(|g| g.iter().map(|(_, _, action)| self.menu_hint(action)).collect()).collect();
        let mut chosen = None;
        let mut item_ids = Vec::new();
        // What the pointer rests on: a submenu entry (with its row), another entry, or nothing yet
        let mut sub_hover: Option<Option<(usize, usize, egui::Rect)>> = None;
        let area = egui::Area::new(egui::Id::unique("context_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(origin + egui::vec2(0.0, (1.0 - t) * -8.0))
            .interactable(interactive)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .corner_radius(egui::CornerRadius::same(material::radius_container()))
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
                                if matches!(action, MenuAction::Submenu(_)) {
                                    let open = self.menu_sub == Some((g, idx));
                                    let resp = material::menu_submenu_item(ui, ("menu", g, idx), icon, label, open, &self.theme);
                                    item_ids.push(resp.id);
                                    if interactive && (resp.hovered() || resp.clicked()) {
                                        sub_hover = Some(Some((g, idx, resp.rect)));
                                    }
                                    continue;
                                }
                                let shown = match action {
                                    MenuAction::Open(item) if !item.is_dir => open_label.as_deref().unwrap_or(label),
                                    _ => label,
                                };
                                let resp = menu_item(ui, ("menu", g, idx), icon, shown, hints[g][idx].as_deref(), &self.theme);
                                item_ids.push(resp.id);
                                if resp.hovered() && interactive && sub_hover.is_none() {
                                    sub_hover = Some(None);
                                }
                                if resp.clicked() && interactive {
                                    chosen = Some(action.clone());
                                }
                            }
                        }
                    });
            });

        let rect = area.response.rect;

        // The pointer resting on a submenu entry opens its flyout; moving to another entry closes it,
        // unless the pointer is already inside the flyout
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        let in_flyout = pointer.is_some_and(|p| self.menu_sub_rect.is_some_and(|r| r.expand(2.0).contains(p)));
        let mut sub_anchor = None;
        if !interactive {
            self.menu_sub = None;
        } else {
            match sub_hover {
                Some(Some((g, idx, row))) => {
                    self.menu_sub = Some((g, idx));
                    sub_anchor = Some(row);
                }
                Some(None) if !in_flyout => self.menu_sub = None,
                _ => {}
            }
        }
        let mut flyout_rect = None;
        if let Some((g, idx)) = self.menu_sub
            && let Some((_, _, MenuAction::Submenu(entries))) = groups.get(g).and_then(|grp| grp.get(idx))
        {
            // The row to line up with: the hovered one now, or where the flyout sat before
            let row_top = sub_anchor.map(|r| r.top()).or_else(|| self.menu_sub_rect.map(|r| r.top() + 8.0));
            if let Some(row_top) = row_top {
                let screen = ctx.content_rect();
                let height = entries.len() as f32 * material::ENTRY_HEIGHT + (entries.len() as f32 - 1.0) * 2.0 + 16.0;
                let right_x = rect.right() + 2.0;
                let x = if right_x + MENU_WIDTH <= screen.right() - 8.0 { right_x } else { (rect.left() - 2.0 - MENU_WIDTH).max(8.0) };
                let y = (row_top - 8.0).min(screen.bottom() - height - 8.0).max(8.0);
                let hints: Vec<Option<String>> = entries.iter().map(|(_, _, a)| self.menu_hint(a)).collect();
                let fly = egui::Area::new(egui::Id::unique("context_submenu").with(g).with(idx))
                    .order(egui::Order::Foreground)
                    .fixed_pos(egui::pos2(x, y))
                    .show(ctx, |ui| {
                        egui::Frame::new()
                            .fill(self.theme.mantle)
                            .corner_radius(egui::CornerRadius::same(material::radius_container()))
                            .shadow(ctx.global_style().visuals.window_shadow)
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_width(MENU_WIDTH - 16.0);
                                ui.spacing_mut().item_spacing.y = 2.0;
                                for (i, (icon, label, action)) in entries.iter().enumerate() {
                                    let resp = material::menu_item(ui, ("submenu", g, idx, i), icon, label, hints[i].as_deref(), &self.theme);
                                    if resp.clicked() {
                                        chosen = Some(action.clone());
                                    }
                                }
                            });
                    });
                flyout_rect = Some(fly.response.rect);
            }
        }
        self.menu_sub_rect = flyout_rect;

        if interactive {
            // Keyboard: the first arrow press lands on the first (or last) entry, Home and End jump
            ctx.memory_mut(|m| m.set_modal_layer(area.response.layer_id));
            let focused = ctx.memory(|m| m.focused());
            if !focused.is_some_and(|f| item_ids.contains(&f)) {
                let key = ctx.input(|i| {
                    [egui::Key::ArrowDown, egui::Key::Home, egui::Key::ArrowUp, egui::Key::End].into_iter().find(|k| i.key_pressed(*k))
                });
                let target = match key {
                    Some(egui::Key::ArrowDown | egui::Key::Home) => item_ids.first(),
                    Some(_) => item_ids.last(),
                    None => None,
                };
                if let Some(id) = target {
                    // The arrow that picked the entry must not also move on from it in this frame
                    ctx.memory_mut(|m| {
                        m.request_focus(*id);
                        m.move_focus(egui::FocusDirection::None);
                    });
                }
            } else if let Some(key) = ctx.input(|i| [egui::Key::Home, egui::Key::End].into_iter().find(|k| i.key_pressed(*k))) {
                let target = if key == egui::Key::Home { item_ids.first() } else { item_ids.last() };
                if let Some(id) = target {
                    ctx.memory_mut(|m| m.request_focus(*id));
                }
            }
        }
        // The modal layer swallows presses on the header button, so a press there just closes the menu
        let outside_press = interactive
            && ctx.input(|i| {
                i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !flyout_rect.is_some_and(|f| f.contains(p)))
            });
        if chosen.is_some() || outside_press {
            self.menu = None;
            self.menu_sub = None;
            self.menu_sub_rect = None;
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
            self.dialog_kind = None;
            return;
        }

        let mut dialog = if let Some(d) = self.dialog.take() {
            d
        } else if let Some(d) = self.closing_dialog.take() {
            d
        } else {
            return;
        };

        let kind = std::mem::discriminant(&dialog);
        let fresh = is_open && self.dialog_kind != Some(kind);
        self.dialog_kind = is_open.then_some(kind);
        material::set_dialog_fresh(ctx, fresh);

        // The fields in dialogs are single-line, so Up and Down leave them for the next row
        // instead of just jumping the caret to either end of the text
        if is_open && !self.capturing_key && ctx.text_edit_focused() {
            let direction = ctx.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    Some(egui::FocusDirection::Down)
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    Some(egui::FocusDirection::Up)
                } else {
                    None
                }
            });
            if let Some(direction) = direction {
                ctx.memory_mut(|m| m.move_focus(direction));
            }
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
            Dialog::OpenWith { .. } => 480.0,
            Dialog::BatchRename { .. } => 560.0,
            Dialog::Confirm { .. } | Dialog::Alert { .. } => 400.0,
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
                    .corner_radius(egui::CornerRadius::same(material::radius_window()))
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
                            Dialog::Alert { .. } => self.alert_dialog_contents(ui, &mut dialog),
                            Dialog::Properties { .. } => self.properties_contents(ui, &mut dialog),
                            Dialog::OpenWith { .. } => self.open_with_contents(ui, &mut dialog),
                            Dialog::Settings { .. } => self.settings_dialog_contents(ui, &mut dialog),
                            Dialog::BatchRename { .. } => self.batch_rename_contents(ui, &mut dialog),
                            Dialog::Compress { .. } => self.compress_contents(ui, &mut dialog),
                            Dialog::Shortcuts => self.shortcuts_contents(ui),
                            Dialog::Conflict { .. } => self.conflict_dialog_contents(ui, &mut dialog),
                            Dialog::SelectPattern { .. } => self.select_pattern_contents(ui, &mut dialog),
                        };
                    });
            });

        // Tab and the arrow keys stay inside the dialog
        if is_open {
            ctx.memory_mut(|m| m.set_modal_layer(dialog_area.response.layer_id));
        }

        // Click outside the dialog card dismisses it
        let dialog_rect = dialog_area.response.rect;
        let outside_click = is_open
            && !matches!(dialog, Dialog::Conflict { .. })
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
        let Dialog::Create { is_dir, name, error, first_frame, templates, template, edited } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, if *is_dir { tr("New folder") } else { tr("New file") }, None, &theme, "create_close");

        let (changed, mut submit) = name_field(ui, &theme, name, first_frame, !*is_dir);
        if changed {
            *error = None;
            *edited = true;
        }
        if let Some(err) = error.as_ref() {
            ui.label(egui::RichText::new(err).size(material::FONT_CAPTION).color(theme.red));
        }

        // Starting points for a file: nothing, a built-in skeleton, or a file from the templates folder
        if !*is_dir && templates.len() > 1 {
            material::section_label(ui, tr("Template"), &theme);
            let mut picked = None;
            material::group_card(ui, &theme, |ui| {
                egui::ScrollArea::vertical().max_height(material::ENTRY_HEIGHT * 5.0 + 8.0).auto_shrink([false, true]).id_salt("create_templates").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, t) in templates.iter().enumerate() {
                        if nav_item(ui, ("create_tpl", i), t.icon, &t.name, &theme, *template == i).clicked() {
                            picked = Some(i);
                        }
                    }
                });
            });
            if let Some(i) = picked {
                *template = i;
                let chosen = &templates[i];
                let stem_of = |n: &str| match n.rfind('.') {
                    Some(at) if at > 0 => n[..at].to_owned(),
                    _ => n.to_owned(),
                };
                // A name nobody has typed follows the template; a typed one only changes its extension
                let base = if *edited {
                    match chosen.extension() {
                        Some(ext) => format!("{}.{ext}", stem_of(name)),
                        None => stem_of(name),
                    }
                } else {
                    chosen.file_name.clone()
                };
                *name = if *edited { base } else { self.free_name(&base) };
                *first_frame = true;
            }
        }

        let footer = material::dialog_footer(ui, &theme, FooterSpec::new("create", icons::CHECK, tr("Create")));
        submit |= footer.primary;
        close |= footer.cancel;
        if submit {
            match self.create_entry(*is_dir, name, templates.get(*template)) {
                Ok(()) => close = true,
                Err(err) => *error = Some(err),
            }
        }
        close
    }

    fn rename_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Rename { path, name, error, first_frame } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, tr("Rename"), None, &theme, "rename_close");

        let (changed, mut submit) = name_field(ui, &theme, name, first_frame, !path.is_dir());
        if changed {
            *error = None;
        }
        if let Some(err) = error.as_ref() {
            ui.label(egui::RichText::new(err).size(material::FONT_CAPTION).color(theme.red));
        }

        let footer = material::dialog_footer(ui, &theme, FooterSpec::new("rename", icons::CHECK, tr("Rename")));
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

        let replacing = matches!(action, ConfirmAction::ReplaceFile { .. });
        let icon = if replacing { icons::CHECK } else { icons::DELETE_FOREVER };
        let footer = material::dialog_footer(ui, &theme, FooterSpec { danger: true, ..FooterSpec::new("confirm", icon, confirm) });
        close |= footer.cancel;
        if footer.primary {
            match std::mem::replace(action, ConfirmAction::EmptyTrash) {
                ConfirmAction::DeleteForever(paths) => self.delete_forever(paths),
                ConfirmAction::EmptyTrash => self.empty_trash(),
                ConfirmAction::ReplaceFile { mut queue, answer } => {
                    if queue.is_empty() {
                        self.chooser_answer_with(&answer);
                    }
                    let (next_title, next_body) = Self::replace_question(&queue.remove(0));
                    (*title, *body) = (next_title, next_body);
                    *action = ConfirmAction::ReplaceFile { queue, answer };
                    return false;
                }
            }
            close = true;
        }
        close
    }

    fn alert_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Alert { title, body } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, title, None, &theme, "alert_close");
        ui.label(egui::RichText::new(body.as_str()).size(material::FONT_BODY).color(theme.subtext0));

        ui.add_space(8.0);
        // Fixed height: a bare right-to-left layout would claim all remaining height and stretch the card
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 36.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close |= material::button(ui, "alert_ok", Some(icons::CHECK), tr("Got it"), &theme, ButtonKind::Filled, true).clicked();
        });
        close |= ui.input(|i| i.key_pressed(egui::Key::Enter));
        if close {
            // Dismissing the "can't move over itself" notice drops the pending cut, like Nautilus
            self.clipboard = None;
        }
        close
    }

    fn open_batch_rename(&mut self, paths: Vec<PathBuf>) {
        let entries = paths
            .into_iter()
            .map(|path| {
                let modified = path.symlink_metadata().ok().and_then(|m| m.modified().ok());
                let stamp = crate::fs::props::format_datetime(modified);
                let date = stamp.get(..10).filter(|d| d.as_bytes().get(4) == Some(&b'-')).unwrap_or("").to_owned();
                RenameEntry { is_dir: path.is_dir(), path, date }
            })
            .collect();
        self.dialog = Some(Dialog::BatchRename { entries, rules: RenameRules::new(), first_frame: true });
    }

    /// Pins, folder icons and default apps follow a path that was renamed.
    fn retarget_path(&mut self, old: &Path, target: &Path) {
        if let Some(idx) = self.pins.iter().position(|p| p.as_path() == old) {
            self.pins[idx] = target.to_path_buf();
            Place::save_pins(&self.pins);
        }
        if let Some(idx) = self.bookmarks.iter().position(|p| p.as_path() == old) {
            self.bookmarks[idx] = target.to_path_buf();
            Place::save_bookmarks(&self.bookmarks);
        }
        if let Some(icon) = self.folder_icons.remove(old) {
            self.folder_icons.insert(target.to_path_buf(), icon);
            save_folder_icons(&self.folder_icons);
        }
        if let Some(app) = self.open_overrides.remove(old) {
            self.open_overrides.insert(target.to_path_buf(), app);
            apps::save_overrides(&self.open_overrides);
        }
    }

    /// Refresh the listing after a batch rename and keep the renamed items selected.
    fn finish_batch_rename(&mut self, done: &[(PathBuf, PathBuf)]) {
        for (old, new) in done {
            self.retarget_path(old, new);
        }
        if !done.is_empty() {
            let back = done.iter().map(|(old, new)| (new.clone(), old.clone())).collect();
            self.history.record(UndoEntry::new(trn(done.len(), "Renaming of {} item", "Renaming of {} items"), vec![Edit::Renames(back)]));
        }
        if self.is_mixed_listing() {
            for (old, new) in done {
                if let Some(idx) = self.items.iter().position(|i| &i.path == old)
                    && let Some(item) = FileItem::from_path(new) {
                        self.items[idx] = item;
                    }
            }
        } else {
            self.reload_items();
        }
        self.selection = done.iter().filter_map(|(_, new)| self.items.iter().position(|i| &i.path == new)).collect();
        self.selected_index = self.selection.iter().next().copied();
        self.anchor = self.selected_index;
        self.last_cursor = self.selected_index;
    }

    fn batch_rename_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::BatchRename { entries, rules, first_frame } = dialog else { return false };
        let theme = self.theme;
        let names = batch_rename::preview(entries, rules);
        let problem = batch_rename::problem(entries, &names);
        let old_name = |e: &RenameEntry| e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let changed = entries.iter().zip(&names).any(|(e, n)| old_name(e) != *n);
        let (mut close, mut submit) = (false, false);

        // Cancel and Rename at the corners, the title between them
        let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), material::BUTTON_HEIGHT), egui::Sense::hover());
        ui.painter().text(
            row.center(),
            egui::Align2::CENTER_CENTER,
            trf("Rename {} items", &[&entries.len()]),
            egui::FontId::proportional(material::FONT_TITLE),
            theme.text,
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            close |= material::button(ui, "br_cancel", None, tr("Cancel"), &theme, ButtonKind::Tonal, true).clicked();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ready = problem.is_none() && changed;
                submit |= material::button(ui, "br_go", None, tr("Rename"), &theme, ButtonKind::Filled, ready).clicked() && ready;
            });
        });

        ui.vertical_centered(|ui| {
            let current = usize::from(rules.mode == RenameMode::Replace);
            if let Some(i) = segmented(ui, "br_mode", &[tr("Use a template"), tr("Find and replace")], current, &theme) {
                rules.mode = if i == 0 { RenameMode::Template } else { RenameMode::Replace };
                *first_frame = true;
            }
        });

        let font = egui::FontId::proportional(material::FONT_SUBTITLE);
        let field = |ui: &mut egui::Ui, text: &mut String, hint: &str, focus: bool| {
            let mut enter = false;
            material::input_frame(&theme).show(ui, |ui| {
                let out = egui::TextEdit::singleline(text)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .hint_text(hint)
                    .font(font.clone())
                    .show(ui);
                if focus {
                    out.response.request_focus();
                }
                enter = out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
            enter
        };
        let focus = std::mem::take(first_frame);
        match rules.mode {
            RenameMode::Template => {
                submit |= field(ui, &mut rules.template, tr("Template"), focus);
                ui.horizontal_wrapped(|ui| {
                    for (token, label) in batch_rename::TOKENS {
                        if chip(ui, ("br_token", label), None, &format!("+ {}", tr(label)), &theme, false).clicked() {
                            rules.template.push_str(token);
                        }
                    }
                });
            }
            RenameMode::Replace => {
                submit |= field(ui, &mut rules.find, tr("Text to find"), focus);
                submit |= field(ui, &mut rules.replace, tr("Replace with"), false);
            }
        }

        egui::CollapsingHeader::new(egui::RichText::new(tr("More options")).size(material::FONT_CAPTION).color(theme.subtext1))
            .id_salt("br_more")
            .show(ui, |ui| {
                if rules.mode == RenameMode::Replace {
                    ui.horizontal_wrapped(|ui| {
                        if chip(ui, "br_match_case", None, tr("Match case"), &theme, rules.match_case).clicked() {
                            rules.match_case = !rules.match_case;
                        }
                    });
                }
                ui.columns(2, |cols| {
                    field(&mut cols[0], &mut rules.prefix, tr("Add at start"), false);
                    field(&mut cols[1], &mut rules.suffix, tr("Add at end"), false);
                });
                ui.columns(2, |cols| {
                    field(&mut cols[0], &mut rules.trim_start, tr("Remove first N characters"), false);
                    field(&mut cols[1], &mut rules.trim_end, tr("Remove last N characters"), false);
                });
                material::section_label(ui, tr("Letter case"), &theme);
                ui.horizontal_wrapped(|ui| {
                    for (case, label) in batch_rename::Case::ALL {
                        if chip(ui, ("br_case", label), None, tr(label), &theme, rules.case == case).clicked() {
                            rules.case = case;
                        }
                    }
                });
                material::section_label(ui, tr("Spaces"), &theme);
                ui.horizontal_wrapped(|ui| {
                    for (spaces, label) in batch_rename::Spaces::ALL {
                        if chip(ui, ("br_spaces", label), None, tr(label), &theme, rules.spaces == spaces).clicked() {
                            rules.spaces = spaces;
                        }
                    }
                    if chip(ui, "br_lower_ext", None, tr("Lowercase extension"), &theme, rules.lowercase_ext).clicked() {
                        rules.lowercase_ext = !rules.lowercase_ext;
                    }
                });
                if rules.mode == RenameMode::Template {
                    material::section_label(ui, tr("Numbering"), &theme);
                    ui.horizontal_wrapped(|ui| {
                        for (order, label) in batch_rename::Order::ALL {
                            if chip(ui, ("br_order", label), None, tr(label), &theme, rules.order == order).clicked() {
                                rules.order = order;
                            }
                        }
                    });
                    ui.columns(2, |cols| {
                        field(&mut cols[0], &mut rules.start, tr("Start at"), false);
                        field(&mut cols[1], &mut rules.step, tr("Step"), false);
                    });
                }
            });

        // Old name → new name for every file
        let max_height = (ui.ctx().content_rect().height() - 380.0).clamp(120.0, 320.0);
        material::group_card(ui, &theme, |ui| {
            egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).id_salt("br_scroll").show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (entry, new) in entries.iter().zip(&names) {
                    let old = old_name(entry);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::hover());
                    let half = (rect.width() - 36.0) / 2.0;
                    let paint = |x: f32, text: &str, color: egui::Color32| {
                        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), egui::FontId::proportional(material::FONT_BODY), color);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(half.max(10.0));
                        let galley = ui.painter().layout_job(job);
                        ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
                    };
                    paint(rect.left() + 10.0, &old, theme.subtext1);
                    material::paint_icon(ui.painter(), egui::pos2(rect.left() + 10.0 + half + 14.0, rect.center().y), icons::ARROW_FORWARD, material::ICON_XS, theme.subtext1);
                    paint(rect.left() + 10.0 + half + 28.0, new, if old_name(entry) == *new { theme.subtext1 } else { theme.text });
                }
            });
        });
        if let Some(err) = &problem {
            ui.label(egui::RichText::new(err).size(material::FONT_CAPTION).color(theme.red));
        }

        if submit && problem.is_none() && changed {
            let (done, error) = batch_rename::apply(entries, &names);
            let count = done.len();
            self.finish_batch_rename(&done);
            match error {
                None => self.set_toast(trn(count, "Renamed {} item", "Renamed {} items")),
                Some(err) => self.set_toast(trf("Renamed {} items, then: {}", &[&count, &err])),
            }
            close = true;
        }
        close
    }

    /// Copy to / Move to / Extract to: a window of its own with the places on the left, a path
    /// bar on top and the subfolders of the current folder to browse.
    fn render_picker(&mut self, ctx: &egui::Context) {
        let Some(mut picker) = self.picker.take() else { return };
        let theme = self.theme;
        let icon_for = picker.icon_for.clone();
        let (title, verb) = match picker.op {
            _ if icon_for.is_some() => (tr("Folder icon"), tr("Set icon")),
            TransferOp::Copy => (tr("Copy to"), tr("Copy")),
            TransferOp::Move => (tr("Move to"), tr("Move")),
            TransferOp::Extract => (tr("Extract to"), tr("Extract")),
        };
        let what = match &icon_for {
            Some(f) => format!("“{}”", f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            None => Self::describe_count(&picker.items),
        };
        let has_icon = icon_for.as_ref().is_some_and(|f| self.folder_icons.contains_key(f));
        let mut reset = false;
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("{title} — {what}"))
            .with_inner_size([880.0, 600.0])
            .with_min_inner_size([560.0, 380.0]);

        let mut confirmed = false;
        let mut cancelled = false;
        let mut goto: Option<PathBuf> = None;
        let places = &self.places;
        let pins = &self.pins;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("ski_folder_picker"), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            if ctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                cancelled = true;
            }

            egui::Panel::left("pk_places")
                .exact_size(212.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(theme.mantle).inner_margin(egui::Margin::symmetric(PAGE_MARGIN, 8)))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        for (idx, place) in places.iter().enumerate() {
                            if place.kind != PlaceKind::Folder || trash::contains(&place.path) {
                                continue;
                            }
                            if nav_item(ui, ("pk_place", idx), place.icon, &tr_dyn(&place.name), &theme, picker.dir == place.path).clicked() {
                                goto = Some(place.path.clone());
                            }
                        }
                        if !pins.is_empty() {
                            material::divider(ui, &theme);
                        }
                        for pin in pins {
                            let name = pin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| pin.display().to_string());
                            if nav_item(ui, ("pk_pin", pin), icons::FOLDER, &name, &theme, picker.dir == *pin).clicked() {
                                goto = Some(pin.clone());
                            }
                        }
                    });
                });

            egui::Panel::top("pk_path")
                .frame(egui::Frame::new().fill(theme.base).inner_margin(egui::Margin::symmetric(HEADER_MARGIN, 8)))
                .show(ui, |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), CONTROL_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = TITLE_GAP;
                            let can_up = picker.dir.parent().is_some();
                            if icon_button(ui, "pk_up", icons::ARROW_UP, &theme, can_up, tr("Parent folder")).clicked()
                                && let Some(parent) = picker.dir.parent() {
                                    goto = Some(parent.to_path_buf());
                                }
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), CONTROL_HEIGHT), egui::Sense::hover());
                            ui.painter().rect_filled(rect, material::pill(rect), theme.surface1);
                            let edit_rect = egui::Rect::from_min_max(egui::pos2(rect.left() + 16.0, rect.top()), egui::pos2(rect.right() - 12.0, rect.bottom()));
                            let resp = material::put_over(
                                ui,
                                edit_rect,
                                egui::TextEdit::singleline(&mut picker.path_input)
                                    .frame(egui::Frame::NONE)
                                    .align(egui::Align2::LEFT_CENTER)
                                    .font(egui::FontId::proportional(material::FONT_BODY))
                                    .hint_text(tr("Type a folder path and press Enter")),
                            );
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                let typed = PathBuf::from(picker.path_input.trim());
                                if typed.is_dir() {
                                    goto = Some(typed.canonicalize().unwrap_or(typed));
                                } else {
                                    picker.path_input = picker.dir.to_string_lossy().into_owned();
                                }
                            }
                        },
                    );
                });

            egui::Panel::bottom("pk_footer")
                .frame(egui::Frame::new().fill(theme.mantle).inner_margin(egui::Margin::symmetric(CONTENT_INSET, 10)))
                .show(ui, |ui| {
                    let dest = picker.chosen.as_ref().unwrap_or(&picker.dir);
                    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dest.display().to_string());
                    // Choosing an icon needs an image; a destination can always be the open folder
                    let ready = icon_for.is_none() || picker.chosen.as_ref().is_some_and(|c| !c.is_dir());
                    ui.horizontal(|ui| {
                        if icon_for.is_some() {
                            let text = if ready { trf("Icon for {}: “{}”", &[&what, &name]) } else { trf("Choose an image for {}", &[&what]) };
                            ui.label(egui::RichText::new(text).size(material::FONT_BODY).color(theme.text));
                        } else {
                            ui.label(egui::RichText::new(trf("{} {} to", &[&verb, &what])).size(material::FONT_CAPTION).color(theme.subtext1));
                            ui.label(egui::RichText::new(format!("“{name}”")).size(material::FONT_BODY).color(theme.text));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            confirmed |= material::button(ui, "pk_go", Some(icons::CHECK), verb, &theme, ButtonKind::Filled, ready).clicked() && ready;
                            cancelled |= material::button(ui, "pk_cancel", None, tr("Cancel"), &theme, ButtonKind::Text, true).clicked();
                            if has_icon {
                                reset |= material::button(ui, "pk_reset", None, tr("Reset"), &theme, ButtonKind::Text, true).clicked();
                            }
                        });
                    });
                });

            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme.base).inner_margin(egui::Margin { left: PAGE_MARGIN, right: PAGE_MARGIN, top: 0, bottom: 6 }))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("pk_scroll").show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let mut pick = None;
                        for folder in &picker.folders {
                            let label = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            let resp = nav_item(ui, ("pk_folder", folder), icons::FOLDER, &label, &theme, picker.chosen.as_ref() == Some(folder));
                            if resp.double_clicked() {
                                goto = Some(folder.clone());
                            } else if resp.clicked() {
                                pick = Some(folder.clone());
                            }
                        }
                        for image in &picker.images {
                            let label = image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            let resp = nav_item(ui, ("pk_image", image), icons::IMAGE, &label, &theme, picker.chosen.as_ref() == Some(image));
                            if resp.double_clicked() {
                                picker.chosen = Some(image.clone());
                                confirmed = true;
                            } else if resp.clicked() {
                                pick = Some(image.clone());
                            }
                        }
                        if pick.is_some() {
                            picker.chosen = pick;
                        }
                        if picker.folders.is_empty() && picker.images.is_empty() {
                            material::empty_state(ui, &theme, icons::FOLDER, tr("No subfolders"), Some(tr("Choose this folder with the button below")));
                        }
                    });
                });
        });

        if let Some(target) = goto {
            picker.enter(target);
        }
        if let Some(folder) = icon_for {
            if reset {
                self.apply_folder_icon(&folder, None);
            } else if confirmed && let Some(image) = picker.chosen.as_ref().filter(|c| !c.is_dir()) {
                self.apply_folder_icon(&folder, Some(image));
            } else if !cancelled {
                self.picker = Some(picker);
            }
        } else if confirmed {
            let dest = picker.chosen.clone().unwrap_or_else(|| picker.dir.clone());
            self.transfer(picker.op, picker.items, dest);
        } else if !cancelled {
            self.picker = Some(picker);
        }
    }

    fn compress_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Compress { sources, name, format, first_frame } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, tr("Compress"), Some(&Self::describe_count(sources)), &theme, "cz_close");

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

        let footer = material::dialog_footer(ui, &theme, FooterSpec { enabled: !name.trim().is_empty(), ..FooterSpec::new("cz", icons::COMPRESS, tr("Compress")) });
        submit |= footer.primary;
        close |= footer.cancel;

        if submit && !name.trim().is_empty() {
            let file_name = format!("{}{}", name.trim().replace('/', "_"), format.extension());
            let out = ops::unique_path(&self.current_path, std::ffi::OsStr::new(&file_name), "copy");
            let (sources, format) = (sources.clone(), *format);
            let shown = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let title = trf("Compressing into {}", &[&shown]);
            self.engine.spawn(JobKind::Compress, title, String::new(), move |_| match ops::compress(&sources, &out, format) {
                Ok(()) => Outcome::done(trf("Created “{}”", &[&shown])).with(Effect::Record(UndoEntry::new(trf("Compression into {}", &[&shown]), vec![Edit::Trash(out)]))),
                Err(err) => Outcome::failed(err.to_string()),
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
        let mut close = material::dialog_header(ui, tr("Open with"), Some(&format!("{name} · {mime}")), &theme, "ow_close");

        // Search over every installed application
        material::input_frame(&theme).show(ui, |ui| {
            ui.horizontal(|ui| {
                material::inline_icon(ui, icons::SEARCH, material::ICON_SM, theme.subtext1);
                let resp = ui.add(
                    egui::TextEdit::singleline(query)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .hint_text(tr("Search applications"))
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
            (tr("Default"), default.iter().copied().filter(matches).collect()),
            (tr("Recommended"), recommended.iter().copied().filter(matches).collect()),
            (tr("Other applications"), others.iter().copied().filter(matches).collect()),
        ];

        let max_height = (ui.ctx().content_rect().height() - 330.0).clamp(160.0, 380.0);
        let mut launch = false;
        // The application list sits in a box of its own
        material::group_card(ui, &theme, |ui| {
            if sections.iter().all(|(_, rows)| rows.is_empty()) {
                ui.add_space(6.0);
                material::hint_label(ui, tr("No applications found"), &theme);
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
                        let label = if is_pinned { trf("{}  ·  this file", &[&app.name]) } else { app.name.clone() };
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
            if material::switch_row(ui, "ow_file", icons::FILE, tr("Always for this file"), &theme, *for_file).clicked() {
                *for_file = !*for_file;
            }
            if material::switch_row(ui, "ow_type", icons::APPS, tr("Always for this file type"), &theme, *for_type).clicked() {
                *for_type = !*for_type;
            }
        });

        let has_choice = self.open_overrides.contains_key(path.as_path());
        let footer = material::dialog_footer(
            ui,
            &theme,
            FooterSpec { enabled: selected.is_some(), extra: has_choice.then_some(tr("Forget file choice")), ..FooterSpec::new("ow", icons::OPEN, tr("Open")) },
        );
        close |= footer.cancel;
        launch |= footer.primary;
        if footer.extra {
            self.open_overrides.remove(path.as_path());
            apps::save_overrides(&self.open_overrides);
            self.set_toast(tr("File choice removed"));
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
                                self.set_toast(trf("{} is now the default for {}", &[&app.name, &mime]));
                            } else {
                                self.set_toast(tr("Could not change the default application"));
                            }
                        }
                        close = true;
                    }
                    Err(err) => self.set_toast(trf("Failed to start {}: {}", &[&app.name, &err])),
                }
            }
        close
    }

    /// Pick an image for a folder in the same window as Move to.
    fn open_icon_dialog(&mut self, folder: PathBuf) {
        let start = self
            .folder_icons
            .get(&folder)
            .and_then(|p| p.parent())
            .filter(|p| p.is_dir())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| if folder.is_dir() { folder.clone() } else { self.current_path.clone() });
        let mut picker = Picker {
            op: TransferOp::Copy,
            items: Vec::new(),
            dir: PathBuf::new(),
            path_input: String::new(),
            chosen: None,
            folders: Vec::new(),
            icon_for: Some(folder),
            images: Vec::new(),
        };
        picker.enter(start);
        self.picker = Some(picker);
    }

    /// Set (or with `None` remove) the custom icon of a folder.
    fn apply_folder_icon(&mut self, folder: &Path, image: Option<&Path>) {
        match image {
            Some(img) => {
                self.thumbs.ensure(img);
                self.folder_icons.insert(folder.to_path_buf(), img.to_path_buf());
                self.set_toast(tr("Folder icon set"));
            }
            None => {
                self.folder_icons.remove(folder);
                self.set_toast(tr("Folder icon reset"));
            }
        }
        save_folder_icons(&self.folder_icons);
    }

    /// Nautilus-style properties: a centered header, then grouped cards of label / value rows.
    /// Returns true when the dialog should close.
    fn properties_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Properties { props, scan, tab } = dialog else { return false };
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
            let total = format!("{}{}", crate::fs::file_item::format_bytes(size), if done { "" } else { tr(" so far…") });
            groups[0].1.push((tr("Total size"), format!("{total} · {} · {}", trn(files as usize, "{} file", "{} files"), trn(folders as usize, "{} folder", "{} folders"))));
            subtitle = format!("{} · {total}", props.kind);
            if !done {
                ctx.request_repaint();
            }
        } else if !item.is_dir {
            subtitle = format!("{} · {}", props.kind, item.formatted_size());
        }

        let mut copied: Option<String> = None;
        let mut perm_action = perms::PermAction::default();
        let mut change_icon = false;
        let mut reset_icon = false;
        let mut toggle_bookmark_path: Option<PathBuf> = None;
        let mut close = false;

        let bookmarked = self.bookmarks.contains(&item.path);
        let (closed, heart) = material::dialog_header_with_heart(ui, tr("Properties"), None, &theme, "dlg_close", Some(bookmarked));
        close |= closed;
        if heart {
            toggle_bookmark_path = Some(item.path.clone());
        }
        if !ctx.egui_wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_pressed(egui::Key::ArrowRight) {
                    *tab = 1;
                }
                if i.key_pressed(egui::Key::ArrowLeft) {
                    *tab = 0;
                }
            });
        }
        ui.add_space(2.0);

        let full = ui.available_width();
        ui.spacing_mut().item_spacing.y = 4.0;

        // Header: icon tile, name, one-line summary
        ui.vertical_centered(|ui| {
            let (tile, _) = ui.allocate_exact_size(egui::vec2(88.0, 88.0), egui::Sense::hover());
            ui.painter().rect_filled(tile, egui::CornerRadius::same(material::radius_window()), theme.surface1);
            match thumb_for(&self.thumbs, &self.folder_icons, item, self.settings.show_thumbnails) {
                Some(tex) => {
                    egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), egui::vec2(64.0, 64.0)))
                        .corner_radius(egui::CornerRadius::same(material::radius_md()))
                        .paint_at(ui, egui::Rect::from_center_size(tile.center(), egui::vec2(64.0, 64.0)));
                }
                None => {
                    let color = if item.is_dir { theme.blue } else { theme.subtext0 };
                    let glyph = item.file_type.icon();
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
                if material::button(ui, "props_icon", Some(icons::IMAGE), tr("Custom icon…"), &theme, ButtonKind::Tonal, true).clicked() {
                    change_icon = true;
                }
                if self.folder_icons.contains_key(&item.path)
                    && material::button(ui, "props_icon_reset", Some(icons::TRASH), tr("Reset icon"), &theme, ButtonKind::Text, true).clicked()
                {
                    reset_icon = true;
                }
            }
        });
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            if let Some(i) = segmented(ui, "props_tab", &[tr("Details"), tr("Permissions")], *tab, &theme) {
                *tab = i;
            }
        });
        ui.add_space(4.0);

        // The header stays put; only the cards scroll, with the same height on both tabs so the dialog does not jump
        let body_height = (ctx.content_rect().height() - 150.0 - 224.0).clamp(160.0, 440.0);
        egui::ScrollArea::vertical()
            .min_scrolled_height(body_height)
            .max_height(body_height)
            .auto_shrink([true, false])
            .show(ui, |ui| {
                let full = ui.available_width();
                ui.spacing_mut().item_spacing.y = 4.0;

                // Ownership belongs to the permissions tab, everything else to the details
                let ownership = tr("Ownership");
                for (title, rows) in groups.iter().filter(|(title, _)| (*title == ownership) == (*tab == 1)) {
                    material::section_label(ui, title, &theme);
                    egui::Frame::new().fill(theme.surface1).corner_radius(egui::CornerRadius::same(material::radius_container())).show(ui, |ui| {
                        ui.set_width(full);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (idx, (label, value)) in rows.iter().enumerate() {
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(full, 38.0), egui::Sense::click());
                            let fx = material::fx(ui, resp.id, &resp, true, &material::FxSpec::ROW);
                            let painter = ui.painter();
                            let inner = rect.shrink2(egui::vec2(4.0, 2.0));
                            material::paint_fill(painter, inner, material::pill(inner), material::row_layer(&theme, &fx), &fx);
                            if idx > 0 {
                                let line = egui::Rect::from_min_size(egui::pos2(rect.left() + 16.0, rect.top()), egui::vec2(full - 32.0, 1.0));
                                painter.rect_filled(material::snap(painter, line), 0, theme.with_alpha(theme.text, 14));
                            }
                            painter.text(egui::pos2(rect.left() + 16.0, rect.center().y), egui::Align2::LEFT_CENTER, *label, egui::FontId::proportional(12.5), theme.subtext1);
                            let mut job = egui::text::LayoutJob::simple_singleline(value.clone(), egui::FontId::proportional(material::FONT_BODY), theme.text);
                            job.wrap = egui::text::TextWrapping::truncate_at_width((full - 140.0 - 36.0).max(20.0));
                            let galley = painter.layout_job(job);
                            painter.galley(egui::pos2(rect.left() + 118.0, rect.center().y - galley.size().y / 2.0), galley, theme.text);
                            let copy_alpha = ((fx.hover.max(fx.press)) * 255.0).round() as u8;
                            if copy_alpha > 0 {
                                material::paint_icon(painter, egui::pos2(rect.right() - 22.0, rect.center().y), icons::COPY, 14.0, theme.with_alpha(theme.subtext1, copy_alpha));
                            }
                            if resp.on_hover_text(tr("Click to copy")).clicked() {
                                copied = Some(value.clone());
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
                if *tab == 1 && !item.is_symlink {
                    perm_action = perms::permission_card(ui, &theme, full, item.mode, item.is_dir, perms::can_change(item));
                }
            });

        ui.add_space(6.0);
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), material::BUTTON_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if material::button(ui, "dlg_copy_path", Some(icons::COPY), tr("Copy path"), &theme, ButtonKind::Tonal, true).clicked() {
                copied = Some(props.item.path.to_string_lossy().into_owned());
            }
        });

        let folder = props.item.path.clone();
        if let Some(mode) = perm_action.new_mode {
            self.set_mode(props, mode);
        }
        if perm_action.apply_to_contents {
            self.apply_mode_to_contents(folder.clone(), props.item.mode & 0o7777);
            self.set_toast(tr("Changing permissions…"));
        }
        if let Some(text) = copied {
            ctx.copy_text(text);
            self.set_toast(tr("Copied to clipboard"));
        }
        if let Some(path) = toggle_bookmark_path {
            self.toggle_bookmark(path);
        }
        if reset_icon {
            self.apply_folder_icon(&folder, None);
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
    let icon_t = material::animate(ui.ctx(), id.with("ico"), icon.is_some());
    let bg = material::selectable_bg(theme, &fx, sel);
    material::paint_fill(ui.painter(), rect, material::pill(rect), bg, &fx);

    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), egui::vec2(28.0, 28.0));
    if icon_t < 0.99 {
        material::paint_icon(ui.painter(), icon_rect.center(), icons::EXECUTABLE, 22.0, theme.subtext0.gamma_multiply(1.0 - icon_t));
    }
    if let Some(tex) = icon {
        ui.scope(|ui| {
            ui.set_opacity(icon_t);
            egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), icon_rect.size())).paint_at(ui, icon_rect);
        });
    }
    let color = material::lerp_color(material::lerp_color(theme.text, theme.text, fx.hover), theme.mauve, sel);
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

        let settle = |app: &mut SkiApp| {
            for _ in 0..200 {
                app.poll_jobs();
                if !app.engine.running() { break; }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            app.poll_jobs();
        };

        // Trash removes it from the listing, offers Undo, and Undo brings it back
        let b = app.current_path.join("b.txt");
        app.trash_paths(vec![b.clone()]);
        assert_eq!(names(&app), ["sub", "taken.txt"]);
        assert!(!b.exists());
        assert!(matches!(app.status_toast.take(), Some(Toast { action: Some(ToastAction::Undo), .. })), "no undo offered");
        app.undo();
        settle(&mut app);
        assert_eq!(names(&app), ["sub", "b.txt", "taken.txt"]);
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "a");

        // Redo trashes it again; rename can be undone too
        app.redo();
        settle(&mut app);
        assert_eq!(names(&app), ["sub", "taken.txt"]);
        app.undo();
        settle(&mut app);
        app.rename_entry(&b, "c.txt").unwrap();
        app.undo();
        settle(&mut app);
        assert_eq!(names(&app), ["sub", "b.txt", "taken.txt"]);

        // Delete forever runs as a job; the trash record goes with the file
        app.trash_paths(vec![b.clone()]);
        let trashed: Vec<PathBuf> = std::fs::read_dir(trash::files_dir().unwrap()).unwrap().flatten().map(|e| e.path()).collect();
        assert_eq!(trashed.len(), 1);
        app.delete_forever(trashed.clone());
        settle(&mut app);
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

    #[test]
    fn tabs_keep_their_own_folder_history_and_selection() {
        let base = std::env::temp_dir().join(format!("ski_tabs_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (one, two) = (base.join("one"), base.join("two"));
        std::fs::create_dir_all(one.join("inner")).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(one.join("a.txt"), "a").unwrap();
        std::fs::write(one.join("b.txt"), "b").unwrap();
        std::fs::write(two.join("c.txt"), "c").unwrap();

        let ctx = egui::Context::default();
        let mut app = SkiApp::new(one.clone(), SerpantinumTheme::default(), Settings::default(), &ctx);
        app.selection = BTreeSet::from([1, 2]);
        app.selected_index = Some(1);
        app.last_cursor = Some(1);

        app.new_tab(two.clone());
        assert_eq!((app.tabs.len(), app.active_tab), (2, 1));
        assert_eq!(app.current_path, two.canonicalize().unwrap());
        assert_eq!(app.items.len(), 1);
        assert!(app.selection.is_empty() && app.back_history.is_empty());

        app.switch_tab(0);
        assert_eq!(app.current_path, one.canonicalize().unwrap());
        assert_eq!(app.selected_paths().len(), 2, "selection survives a round trip");

        app.navigate_to(one.join("inner"));
        assert_eq!(app.back_history.len(), 1);
        app.switch_tab(1);
        assert!(app.back_history.is_empty(), "history belongs to the tab");
        app.switch_tab(0);
        assert_eq!(app.back_history.len(), 1);

        app.close_tab(&ctx, 0);
        assert_eq!((app.tabs.len(), app.active_tab), (1, 0));
        assert_eq!(app.current_path, two.canonicalize().unwrap());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn rubberband_drag_selects_from_any_point() {
        let base = std::env::temp_dir().join(format!("ski_drag_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        for name in ["alpha.txt", "beta.txt", "gamma.txt", "delta.txt"] {
            std::fs::write(base.join(name), name).unwrap();
        }

        let ctx = egui::Context::default();
        let mut app = SkiApp::new(base.clone(), SerpantinumTheme::default(), Settings::default(), &ctx);
        app.view_mode = ViewMode::Grid;

        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let item_rects = vec![
            (0, egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(100.0, 100.0))),
            (1, egui::Rect::from_min_size(egui::pos2(140.0, 20.0), egui::vec2(100.0, 100.0))),
            (2, egui::Rect::from_min_size(egui::pos2(260.0, 20.0), egui::vec2(100.0, 100.0))),
            (3, egui::Rect::from_min_size(egui::pos2(380.0, 20.0), egui::vec2(100.0, 100.0))),
        ];

        // Start dragging in the gap above item 0 (a press on an item itself picks the item up
        // instead, see app/dnd.rs) and drag across to item 2
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(egui::pos2(10.0, 10.0)));
        raw.events.push(egui::Event::PointerButton {
            pos: egui::pos2(10.0, 10.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        let mut out = ctx.run_ui(raw, |ui| {
            assert!(!app.handle_drag_select(ui, viewport, 0.0, 0.0, &item_rects));
        });
        out.textures_delta.clear();

        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(egui::pos2(300.0, 90.0)));
        let mut out = ctx.run_ui(raw, |ui| {
            assert!(app.handle_drag_select(ui, viewport, 0.0, 0.0, &item_rects));
        });
        out.textures_delta.clear();
        assert_eq!(app.selection, BTreeSet::from([0, 1, 2]));
        assert_eq!(app.selected_index, Some(2));

        // Release ends the drag while keeping the selection and suppressing click-select
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerButton {
            pos: egui::pos2(300.0, 90.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        let mut out = ctx.run_ui(raw, |ui| {
            assert!(app.handle_drag_select(ui, viewport, 0.0, 0.0, &item_rects));
        });
        out.textures_delta.clear();
        assert!(app.drag_select.is_none());
        assert_eq!(app.selection, BTreeSet::from([0, 1, 2]));

        std::fs::remove_dir_all(&base).unwrap();
    }
}

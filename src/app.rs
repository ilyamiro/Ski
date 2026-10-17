use std::path::{Path, PathBuf};
use eframe::egui;

use crate::fs::{
    file_item::FileItem,
    places::Place,
    scanner::{read_directory, ListOptions, SortBy, SortOrder, TypeFilter},
    search::Search,
};
use crate::thumbs::Thumbnails;
use crate::ui::{
    icons,
    material::{self, chip, file_row, file_tile, icon_button, icon_button_tinted, nav_item, CONTROL_HEIGHT},
    theme::SerpantinumTheme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Grid,
}

pub struct SkiApp {
    current_path: PathBuf,
    items: Vec<FileItem>,
    selected_index: Option<usize>,
    error_message: Option<String>,

    // Dynamic shell theme from Serpantinum
    theme: SerpantinumTheme,

    // Navigation history
    back_history: Vec<PathBuf>,
    forward_history: Vec<PathBuf>,

    // Search and display options
    search_query: String,
    show_hidden: bool,
    folders_first: bool,
    type_filter: TypeFilter,
    view_mode: ViewMode,
    sort_by: SortBy,
    sort_order: SortOrder,

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

    // Status toast message (text, timestamp)
    status_toast: Option<(String, std::time::Instant)>,

    // Flag to scroll to selected item on next frame
    scroll_to_selected: bool,
}

impl SkiApp {
    pub fn new(initial_path: PathBuf, theme: SerpantinumTheme, ctx: &egui::Context) -> Self {
        let places = Place::default_places();
        let canonical_path = initial_path.canonicalize().unwrap_or(initial_path);

        let mut app = Self {
            current_path: canonical_path.clone(),
            items: Vec::new(),
            selected_index: None,
            error_message: None,
            theme,
            back_history: Vec::new(),
            forward_history: Vec::new(),
            search_query: String::new(),
            show_hidden: false,
            folders_first: true,
            type_filter: TypeFilter::All,
            view_mode: ViewMode::List,
            sort_by: SortBy::Name,
            sort_order: SortOrder::Ascending,
            is_editing_path: false,
            path_input: canonical_path.to_string_lossy().to_string(),
            thumbs: Thumbnails::new(ctx),
            search: Search::new(),
            places,
            show_search: false,
            focus_search: false,
            show_settings: false,
            settings_button_rect: egui::Rect::NOTHING,
            status_toast: None,
            scroll_to_selected: false,
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
            self.current_path = canonical;
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
            self.path_input = self.current_path.to_string_lossy().to_string();
            self.selected_index = None;
            self.reload_items();
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(next) = self.forward_history.pop() {
            self.back_history.push(self.current_path.clone());
            self.current_path = next;
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
        self.theme = SerpantinumTheme::load();
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
                self.thumbs.preload(self.items.iter().map(|i| i.path.as_path()));
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
    }

    fn open_file(&mut self, path: &Path) {
        if let Err(err) = open::that(path) {
            self.set_toast(format!("Failed to open file: {err}"));
        }
    }

    fn set_toast(&mut self, message: impl Into<String>) {
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
            self.error_message = None;
            let root = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            self.search.start(ctx, root, self.search_query.trim().to_owned(), self.show_hidden);
        }
    }

    fn move_selection(&mut self, delta: isize) {
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

        self.selected_index = Some(new_idx);
        self.scroll_to_selected = true;
    }

    fn open_selected(&mut self) {
        if let Some(idx) = self.selected_index {
            if let Some(item) = self.items.get(idx).cloned() {
                if item.is_dir {
                    self.navigate_to(item.path);
                } else {
                    self.open_file(&item.path);
                }
            }
        }
    }

    fn handle_keyboard_inputs(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.is_editing_path = false;
                self.close_search();
            }
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.show_settings {
                self.show_settings = false;
                return;
            }
            if self.show_search || !self.search_query.is_empty() {
                self.close_search();
                return;
            }
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Questionmark)) {
            self.show_settings = !self.show_settings;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Slash) || (i.modifiers.command && i.key_pressed(egui::Key::F))) {
            self.show_search = true;
            self.focus_search = true;
        }

        // Vim & Arrow Keys
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::J)) {
            self.move_selection(1);
        }

        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::K)) {
            self.move_selection(-1);
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::L)) {
            self.open_selected();
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::H)) {
            self.go_up();
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Home) || (i.key_pressed(egui::Key::G) && !i.modifiers.shift)) {
            if !self.items.is_empty() {
                self.selected_index = Some(0);
                self.scroll_to_selected = true;
            }
        }

        if ctx.input(|i| i.key_pressed(egui::Key::End) || (i.key_pressed(egui::Key::G) && i.modifiers.shift)) {
            if !self.items.is_empty() {
                self.selected_index = Some(self.items.len() - 1);
                self.scroll_to_selected = true;
            }
        }

        if ctx.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft)) {
            self.go_back();
        }
        if ctx.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::ArrowRight)) {
            self.go_forward();
        }

        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Num1)) {
            self.view_mode = ViewMode::List;
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Num2)) {
            self.view_mode = ViewMode::Grid;
        }

        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::H)) {
            self.show_hidden = !self.show_hidden;
            self.reload_items();
        }

        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::L)) {
            self.is_editing_path = true;
            self.path_input = self.current_path.to_string_lossy().to_string();
        }

        if ctx.input(|i| i.key_pressed(egui::Key::F5) || (i.modifiers.command && i.key_pressed(egui::Key::R))) {
            self.refresh();
        }

        if ctx.input(|i| (i.modifiers.command && i.key_pressed(egui::Key::C)) || i.key_pressed(egui::Key::Y)) {
            if let Some(idx) = self.selected_index {
                if let Some(item) = self.items.get(idx) {
                    let path_str = item.path.to_string_lossy().to_string();
                    ctx.copy_text(path_str);
                    self.set_toast(format!("Copied: {}", item.name));
                }
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
        self.thumbs.poll(&ctx);

        let found = self.search.poll();
        if !found.is_empty() && !self.search_query.trim().is_empty() {
            self.items.extend(found);
        }

        if let Some((_, time)) = &self.status_toast {
            if time.elapsed().as_secs() > 3 {
                self.status_toast = None;
            }
        }

        let window_width = ui.available_width();
        let sidebar_visible = window_width >= SIDEBAR_BREAKPOINT;

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

        if self.show_settings {
            self.render_settings(&ctx);
        }
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
                let right_buttons = if sidebar_visible { 2.0 } else { 3.0 };
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

                let settings = icon_button(ui, "btn_settings", icons::SETTINGS, &self.theme, true, "Settings (?)");
                self.settings_button_rect = settings.rect;
                if settings.clicked() {
                    self.show_settings = !self.show_settings;
                }

                let close = icon_button_tinted(ui, "btn_close", icons::CLOSE, &self.theme, true, "Close", Some(self.theme.red));
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
        // Search field on top of the places
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::hover());
        let search_id = egui::Id::new("sidebar_search");
        let focused = ui.memory(|m| m.has_focus(search_id));
        let fill = if focused { self.theme.surface2 } else { self.theme.surface1 };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(18), fill);
        material::paint_icon(ui.painter(), egui::pos2(rect.left() + 20.0, rect.center().y), icons::SEARCH, 16.0, self.theme.subtext1);

        let clear_w = if self.search_query.is_empty() { 0.0 } else { 30.0 };
        let edit_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 36.0, rect.top()),
            egui::pos2(rect.right() - 12.0 - clear_w, rect.bottom()),
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
        if clear_w > 0.0 {
            let clear_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 20.0, rect.center().y), egui::vec2(26.0, 26.0));
            let clear = ui.interact(clear_rect, ui.make_persistent_id("sidebar_search_clear"), egui::Sense::click());
            let hover = ui.ctx().animate_bool_with_time(clear.id.with("h"), clear.hovered(), 0.18);
            ui.painter().rect_filled(clear_rect, egui::CornerRadius::same(13), self.theme.with_alpha(self.theme.text, (hover * 26.0) as u8));
            material::paint_icon(ui.painter(), clear_rect.center(), icons::CLOSE, 14.0, material::lerp_color(self.theme.subtext1, self.theme.text, hover));
            if clear.clicked() {
                self.close_search();
            }
        }

        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.y = 2.0;

        let mut destination: Option<PathBuf> = None;
        for (idx, place) in self.places.iter().enumerate() {
            let selected = self.search_query.trim().is_empty() && self.current_path == place.path;
            if nav_item(ui, ("place", idx), place.icon, &place.name, &self.theme, selected).clicked() {
                destination = Some(place.path.clone());
            }
        }

        if let Some(dest) = destination {
            self.close_search();
            self.navigate_to(dest);
        }
    }

    fn open_item(&mut self, item: FileItem) {
        if item.is_dir {
            self.navigate_to(item.path);
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
        let mut copied: Option<FileItem> = None;
        let should_scroll = std::mem::take(&mut self.scroll_to_selected);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (idx, item) in self.items.iter().enumerate() {
                let selected = self.selected_index == Some(idx);
                let hint = self.path_hint(item);
                let thumb = self.thumbs.get(&item.path);
                let resp = file_row(ui, idx, item, &self.theme, selected, should_scroll, thumb, hint.as_deref());
                if resp.clicked() || resp.secondary_clicked() {
                    self.selected_index = Some(idx);
                }
                if resp.double_clicked() {
                    opened = Some(item.clone());
                }
                if resp.secondary_clicked() {
                    copied = Some(item.clone());
                }
            }
        });

        self.finish_item_actions(ui, opened, copied);
    }

    fn render_grid_view(&mut self, ui: &mut egui::Ui) {
        let mut opened: Option<FileItem> = None;
        let should_scroll = std::mem::take(&mut self.scroll_to_selected);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let cell = material::GRID_CELL;
            let spacing = 4.0;
            let cols = ((ui.available_width() + spacing) / (cell.x + spacing)).floor().max(1.0) as usize;
            ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);

            for (row_idx, row) in self.items.chunks(cols).enumerate() {
                ui.horizontal(|ui| {
                    for (col_idx, item) in row.iter().enumerate() {
                        let idx = row_idx * cols + col_idx;
                        let selected = self.selected_index == Some(idx);
                        let thumb = self.thumbs.get(&item.path);
                        let resp = file_tile(ui, idx, item, &self.theme, selected, should_scroll, thumb);
                        if resp.clicked() {
                            self.selected_index = Some(idx);
                        }
                        if resp.double_clicked() {
                            opened = Some(item.clone());
                        }
                    }
                });
            }
        });

        self.finish_item_actions(ui, opened, None);
    }

    fn finish_item_actions(&mut self, ui: &egui::Ui, opened: Option<FileItem>, copied: Option<FileItem>) {
        if let Some(item) = opened {
            self.open_item(item);
        }
        if let Some(item) = copied {
            ui.ctx().copy_text(item.path.to_string_lossy().to_string());
            self.set_toast(format!("Copied path of {}", item.name));
        }
    }

    fn render_toast(&mut self, ctx: &egui::Context) {
        let Some((msg, _)) = &self.status_toast else { return };
        egui::Area::new(egui::Id::new("toast"))
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
        let screen = ctx.content_rect();
        let width = (screen.width() - 24.0).clamp(240.0, 340.0);
        let max_height = (screen.height() - 80.0).max(160.0);
        let mut reload = false;

        let area = egui::Area::new(egui::Id::new("settings_popup"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 52.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .corner_radius(egui::CornerRadius::same(22))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(width - 32.0);
                        egui::ScrollArea::vertical().max_height(max_height - 32.0).show(ui, |ui| {
                            reload = self.settings_contents(ui);
                        });
                    });
            });

        // Click outside closes the popup (the settings button handles its own toggle).
        let rect = area.response.rect;
        let outside_press = ctx.input(|i| {
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
        ui.add_space(6.0);
        ui.label(egui::RichText::new(text).size(11.5).color(self.theme.subtext1));
        ui.add_space(2.0);
    }

    /// Returns true when a setting that affects the file listing changed.
    fn settings_contents(&mut self, ui: &mut egui::Ui) -> bool {
        let mut reload = false;
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);

        ui.label(egui::RichText::new("Settings").size(16.0).color(self.theme.text));

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

        self.section_title(ui, "File type");
        ui.horizontal_wrapped(|ui| {
            for filter in TypeFilter::ALL {
                if chip(ui, ("set_type", filter.label()), None, filter.label(), &self.theme, self.type_filter == filter).clicked() {
                    self.type_filter = filter;
                    reload = true;
                }
            }
        });

        self.section_title(ui, "Shortcuts");
        egui::Grid::new("shortcuts_grid").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
            for (key, desc) in [
                ("j / k", "Move selection"),
                ("Enter / l", "Open"),
                ("Backspace / h", "Parent folder"),
                ("Alt + Left / Right", "History"),
                ("/", "Filter by name"),
                ("Ctrl + L", "Edit path"),
                ("Ctrl + H", "Hidden files"),
                ("Ctrl + 1 / 2", "List / grid"),
                ("y", "Copy path"),
                ("F5", "Refresh"),
            ] {
                ui.label(egui::RichText::new(key).size(12.5).color(self.theme.mauve));
                ui.label(egui::RichText::new(desc).size(12.5).color(self.theme.subtext0));
                ui.end_row();
            }
        });

        ui.add_space(6.0);
        if chip(ui, "set_refresh", None, "Refresh and reload theme", &self.theme, false).clicked() {
            self.refresh();
        }

        reload
    }
}

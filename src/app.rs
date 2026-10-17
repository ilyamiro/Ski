use std::path::{Component, Path, PathBuf};
use eframe::egui;

use crate::fs::{
    file_item::FileItem,
    places::Place,
    scanner::{read_directory, ListOptions, SortBy, SortOrder, TypeFilter},
};
use crate::ui::{
    icons,
    material::{self, chip, file_row, file_tile, icon_button, nav_item, CONTROL_HEIGHT},
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
    pub fn new(initial_path: PathBuf, theme: SerpantinumTheme) -> Self {
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
            query: self.search_query.clone(),
            sort_by: self.sort_by,
            sort_order: self.sort_order,
            folders_first: self.folders_first,
            type_filter: self.type_filter,
        };
        match read_directory(&self.current_path, &opts) {
            Ok(items) => {
                self.items = items;
                self.error_message = None;
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
            self.reload_items();
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
            if self.show_search {
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
const SIDEBAR_BREAKPOINT: f32 = 720.0;
/// Content width below which secondary header controls are hidden.
const COMPACT_BREAKPOINT: f32 = 520.0;

impl eframe::App for SkiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_keyboard_inputs(&ctx);

        if let Some((_, time)) = &self.status_toast {
            if time.elapsed().as_secs() > 3 {
                self.status_toast = None;
            }
        }

        let window_width = ui.available_width();

        if window_width >= SIDEBAR_BREAKPOINT {
            egui::Panel::left("sidebar_places")
                .exact_size(232.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(self.theme.mantle)
                        .inner_margin(egui::Margin::symmetric(12, 14)),
                )
                .show(ui, |ui| self.render_sidebar(ui));
        }

        egui::Panel::top("top_navigation")
            .frame(
                egui::Frame::new()
                    .fill(self.theme.base)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| self.render_top_bar(ui));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(self.theme.base)
                    .inner_margin(egui::Margin { left: 8, right: 8, top: 0, bottom: 8 }),
            )
            .show(ui, |ui| {
                if let Some(ref err) = self.error_message {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new(err).color(self.theme.red));
                    });
                } else if self.items.is_empty() {
                    let msg = if self.search_query.is_empty() && self.type_filter == TypeFilter::All {
                        "This folder is empty".to_owned()
                    } else {
                        "Nothing matches".to_owned()
                    };
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new(msg).size(15.0).color(self.theme.subtext1));
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
    fn render_top_bar(&mut self, ui: &mut egui::Ui) {
        let compact = ui.available_width() < COMPACT_BREAKPOINT;
        let gap = 4.0;

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
                let mut left_buttons = 2.0;
                if !compact {
                    let can_up = self.current_path.parent().is_some();
                    if icon_button(ui, "btn_up", icons::ARROW_UP, &self.theme, can_up, "Parent folder").clicked() {
                        self.go_up();
                    }
                    left_buttons += 1.0;
                }

                let right_buttons = 2.0;
                let reserved = right_buttons * (CONTROL_HEIGHT + gap);
                let _ = left_buttons;
                let field_width = (ui.available_width() - reserved - gap).max(40.0);

                let (field_rect, field_resp) =
                    ui.allocate_exact_size(egui::vec2(field_width, CONTROL_HEIGHT), egui::Sense::click());
                ui.painter().rect_filled(field_rect, egui::CornerRadius::same(20), self.theme.surface1);

                if self.show_search {
                    self.render_search_field(ui, field_rect);
                } else if self.is_editing_path {
                    self.render_path_field(ui, field_rect);
                } else {
                    self.render_breadcrumbs(ui, field_rect, &field_resp);
                }

                let search_icon = if self.show_search { icons::CLOSE } else { icons::SEARCH };
                if icon_button(ui, "btn_search", search_icon, &self.theme, true, "Filter by name (/)").clicked() {
                    if self.show_search {
                        self.close_search();
                    } else {
                        self.show_search = true;
                        self.focus_search = true;
                        self.is_editing_path = false;
                    }
                }

                let settings = icon_button(ui, "btn_settings", icons::SETTINGS, &self.theme, true, "Settings (?)");
                self.settings_button_rect = settings.rect;
                if settings.clicked() {
                    self.show_settings = !self.show_settings;
                }
            },
        );
    }

    fn render_search_field(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        material::paint_icon(
            ui.painter(),
            egui::pos2(rect.left() + 22.0, rect.center().y),
            icons::SEARCH,
            18.0,
            self.theme.subtext1,
        );
        let edit_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 42.0, rect.top()),
            egui::pos2(rect.right() - 16.0, rect.bottom()),
        );
        let resp = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut self.search_query)
                .frame(egui::Frame::NONE)
                .align(egui::Align2::LEFT_CENTER)
                .font(egui::FontId::proportional(14.0))
                .hint_text("Filter this folder"),
        );
        if self.focus_search {
            resp.request_focus();
            self.focus_search = false;
        }
        if resp.changed() {
            self.reload_items();
        }
    }

    fn render_path_field(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let edit_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 18.0, rect.top()),
            egui::pos2(rect.right() - 16.0, rect.bottom()),
        );
        let resp = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut self.path_input)
                .frame(egui::Frame::NONE)
                .align(egui::Align2::LEFT_CENTER)
                .font(egui::FontId::proportional(14.0))
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

    /// Path shown as clickable segments. Leading segments collapse into an ellipsis
    /// when the field is too narrow, so the current folder is always visible.
    fn render_breadcrumbs(&mut self, ui: &mut egui::Ui, rect: egui::Rect, field: &egui::Response) {
        struct Crumb {
            label: String,
            icon: Option<&'static str>,
            path: PathBuf,
            width: f32,
        }

        let mut crumbs: Vec<Crumb> = Vec::new();
        let mut acc = PathBuf::new();
        for comp in self.current_path.components() {
            match comp {
                Component::RootDir => {
                    acc.push("/");
                    crumbs.push(Crumb { label: String::new(), icon: Some(icons::STORAGE), path: acc.clone(), width: 0.0 });
                }
                Component::Normal(seg) => {
                    acc.push(seg);
                    crumbs.push(Crumb { label: seg.to_string_lossy().into_owned(), icon: None, path: acc.clone(), width: 0.0 });
                }
                _ => {}
            }
        }

        let font = egui::FontId::proportional(14.0);
        for c in &mut crumbs {
            c.width = match c.icon {
                Some(_) => 36.0,
                None => ui.painter().layout_no_wrap(c.label.clone(), font.clone(), self.theme.text).size().x + 22.0,
            };
        }

        let sep = 16.0;
        let ellipsis = 34.0;
        let avail = rect.width() - 16.0;

        // Walk from the current folder backwards while it still fits.
        let mut first = crumbs.len();
        let mut used = 0.0;
        while first > 0 {
            let w = crumbs[first - 1].width + if first < crumbs.len() { sep } else { 0.0 };
            let reserve = if first - 1 > 0 { ellipsis + sep } else { 0.0 };
            if used + w + reserve > avail && first < crumbs.len() {
                break;
            }
            used += w;
            first -= 1;
        }

        let mut target: Option<PathBuf> = None;
        let mut x = rect.left() + 8.0;
        let cy = rect.center().y;
        let last = crumbs.len().saturating_sub(1);

        if first > 0 {
            let r = egui::Rect::from_min_size(egui::pos2(x, cy - 15.0), egui::vec2(ellipsis, 30.0));
            let resp = ui.interact(r, ui.make_persistent_id("crumb_more"), egui::Sense::click());
            let hover = ui.ctx().animate_bool_responsive(resp.id.with("h"), resp.hovered());
            ui.painter().rect_filled(r, egui::CornerRadius::same(15), self.theme.with_alpha(self.theme.text, (hover * 22.0) as u8));
            material::paint_icon(ui.painter(), r.center(), icons::MORE, 18.0, self.theme.subtext0);
            if resp.clicked() {
                target = Some(crumbs[first - 1].path.clone());
            }
            x += ellipsis;
            material::paint_icon(ui.painter(), egui::pos2(x + sep / 2.0, cy), icons::CHEVRON_RIGHT, 16.0, self.theme.subtext1);
            x += sep;
        }

        for i in first..crumbs.len() {
            let c = &crumbs[i];
            if i > first {
                material::paint_icon(ui.painter(), egui::pos2(x + sep / 2.0, cy), icons::CHEVRON_RIGHT, 16.0, self.theme.subtext1);
                x += sep;
            }
            let w = c.width.min((rect.right() - 8.0 - x).max(0.0));
            let r = egui::Rect::from_min_size(egui::pos2(x, cy - 15.0), egui::vec2(w, 30.0));
            let resp = ui.interact(r, ui.make_persistent_id(("crumb", i)), egui::Sense::click());
            let hover = ui.ctx().animate_bool_responsive(resp.id.with("h"), resp.hovered());
            ui.painter().rect_filled(r, egui::CornerRadius::same(15), self.theme.with_alpha(self.theme.text, (hover * 22.0) as u8));

            let is_current = i == last;
            let color = if is_current { self.theme.text } else { self.theme.subtext0 };
            match c.icon {
                Some(ic) => material::paint_icon(ui.painter(), r.center(), ic, 18.0, color),
                None => {
                    ui.painter().text(egui::pos2(r.left() + 11.0, cy), egui::Align2::LEFT_CENTER, &c.label, font.clone(), color);
                }
            }
            if resp.clicked() {
                target = Some(c.path.clone());
            }
            x += c.width;
        }

        // Clicking the empty area of the field switches to typing a path.
        if field.clicked() && target.is_none() {
            self.is_editing_path = true;
            self.path_input = self.current_path.to_string_lossy().to_string();
        }
        if let Some(t) = target {
            self.navigate_to(t);
        }
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        let mut destination: Option<PathBuf> = None;
        ui.spacing_mut().item_spacing.y = 2.0;

        for (idx, place) in self.places.iter().enumerate() {
            let selected = self.current_path == place.path;
            if nav_item(ui, ("place", idx), place.icon, &place.name, &self.theme, selected).clicked() {
                destination = Some(place.path.clone());
            }
        }

        if let Some(dest) = destination {
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

    fn render_list_view(&mut self, ui: &mut egui::Ui) {
        let mut opened: Option<FileItem> = None;
        let mut copied: Option<FileItem> = None;
        let should_scroll = std::mem::take(&mut self.scroll_to_selected);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (idx, item) in self.items.iter().enumerate() {
                let selected = self.selected_index == Some(idx);
                let resp = file_row(ui, idx, item, &self.theme, selected, should_scroll);
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
                        let resp = file_tile(ui, idx, item, &self.theme, selected, should_scroll);
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
        let width = (screen.width() - 24.0).clamp(240.0, 380.0);
        let max_height = (screen.height() - 80.0).max(160.0);
        let mut reload = false;

        let area = egui::Area::new(egui::Id::new("settings_popup"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 60.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .corner_radius(egui::CornerRadius::same(24))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(20))
                    .show(ui, |ui| {
                        ui.set_width(width - 40.0);
                        egui::ScrollArea::vertical().max_height(max_height - 40.0).show(ui, |ui| {
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
            self.reload_items();
        }
    }

    fn section_title(&self, ui: &mut egui::Ui, text: &str) {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(text).size(12.0).color(self.theme.subtext1));
        ui.add_space(2.0);
    }

    /// Returns true when a setting that affects the file listing changed.
    fn settings_contents(&mut self, ui: &mut egui::Ui) -> bool {
        let mut reload = false;
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);

        ui.label(egui::RichText::new("Settings").size(18.0).color(self.theme.text));

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

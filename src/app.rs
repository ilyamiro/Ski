use std::path::{Component, Path, PathBuf};
use eframe::egui::{self, Color32};

use crate::fs::{
    file_item::FileItem,
    places::Place,
    scanner::{read_directory, SortBy, SortOrder},
};
use crate::ui::{
    icons,
    material::{
        lerp_color, serpantinum_chip_button, serpantinum_file_row, serpantinum_icon_button,
        serpantinum_nav_item,
    },
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
    view_mode: ViewMode,
    sort_by: SortBy,
    sort_order: SortOrder,

    // Path edit mode (Ctrl+L)
    is_editing_path: bool,
    path_input: String,

    // Places
    places: Vec<Place>,

    // Keyboard shortcuts help overlay
    show_help: bool,

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
            view_mode: ViewMode::List,
            sort_by: SortBy::Name,
            sort_order: SortOrder::Ascending,
            is_editing_path: false,
            path_input: canonical_path.to_string_lossy().to_string(),
            places,
            show_help: false,
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
        // Also refresh theme from disk in case user changed wallpaper / Matugen theme in Serpantinum!
        self.theme = SerpantinumTheme::load();
        self.reload_items();
        self.set_toast("Refreshed & reloaded theme");
    }

    pub fn reload_items(&mut self) {
        match read_directory(
            &self.current_path,
            self.show_hidden,
            &self.search_query,
            self.sort_by,
            self.sort_order,
        ) {
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
                if !self.search_query.is_empty() {
                    self.search_query.clear();
                    self.reload_items();
                }
            }
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Questionmark)) {
            self.show_help = !self.show_help;
        }

        if self.show_help {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.show_help = false;
            }
            return;
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

impl eframe::App for SkiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_keyboard_inputs(&ctx);

        if let Some((_, time)) = &self.status_toast {
            if time.elapsed().as_secs() > 3 {
                self.status_toast = None;
            }
        }

        // Top Navigation Bar (matching Serpantinum modular TopBar)
        egui::Panel::top("top_navigation")
            .frame(
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .inner_margin(egui::Margin::symmetric(16, 12)),
            )
            .show(ui, |ui| {
                self.render_top_bar(ui);
            });

        // Bottom Status Bar (matching Serpantinum crust footer)
        egui::Panel::bottom("bottom_status")
            .frame(
                egui::Frame::new()
                    .fill(self.theme.crust)
                    .inner_margin(egui::Margin::symmetric(18, 8)),
            )
            .show(ui, |ui| {
                self.render_bottom_bar(ui);
            });

        // Left Sidebar (matching Serpantinum SideBar / GuideSidebar)
        egui::Panel::left("sidebar_places")
            .default_size(220.0)
            .min_size(170.0)
            .max_size(300.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .inner_margin(egui::Margin::symmetric(14, 16)),
            )
            .show(ui, |ui| {
                self.render_sidebar(ui);
            });

        // Central Panel (File Canvas in base color)
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(self.theme.base)
                    .inner_margin(egui::Margin::symmetric(20, 16)),
            )
            .show(ui, |ui| {
                if let Some(ref err) = self.error_message {
                    ui.centered_and_justified(|ui| {
                        ui.colored_label(self.theme.red, format!("⚠ {err}"));
                    });
                } else if self.items.is_empty() {
                    ui.centered_and_justified(|ui| {
                        if !self.search_query.is_empty() {
                            ui.label(
                                egui::RichText::new(format!("No matches for \"{}\"", self.search_query))
                                    .size(15.0)
                                    .color(self.theme.subtext1),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new("Empty Directory")
                                    .size(15.0)
                                    .color(self.theme.subtext1),
                            );
                        }
                    });
                } else {
                    match self.view_mode {
                        ViewMode::List => self.render_list_view(ui),
                        ViewMode::Grid => self.render_grid_view(ui),
                    }
                }
            });

        // Shortcuts modal
        if self.show_help {
            self.render_help_modal(&ctx);
        }
    }
}

impl SkiApp {
    fn render_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Serpantinum IconButton replicas (42dp, scale animation 1.04/1.08, glass rim)
            let can_back = !self.back_history.is_empty();
            if serpantinum_icon_button(ui, "btn_back", icons::ARROW_BACK, &self.theme, can_back, "Back (Alt+Left)").clicked() {
                self.go_back();
            }

            let can_forward = !self.forward_history.is_empty();
            if serpantinum_icon_button(ui, "btn_fwd", icons::ARROW_FORWARD, &self.theme, can_forward, "Forward (Alt+Right)").clicked() {
                self.go_forward();
            }

            let can_up = self.current_path.parent().is_some();
            if serpantinum_icon_button(ui, "btn_up", icons::ARROW_UP, &self.theme, can_up, "Parent Directory (h / Backspace)").clicked() {
                self.go_up();
            }

            if serpantinum_icon_button(ui, "btn_refresh", icons::REFRESH, &self.theme, true, "Refresh & Reload Theme (F5)").clicked() {
                self.refresh();
            }

            ui.add_space(8.0);

            // Breadcrumbs / Path Editor
            if self.is_editing_path {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.path_input)
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::proportional(14.0))
                        .margin(egui::Margin::symmetric(14, 8))
                        .hint_text("Enter path and press Enter..."),
                );
                response.request_focus();

                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let target = PathBuf::from(&self.path_input);
                    self.navigate_to(target);
                }
            } else {
                self.render_breadcrumbs(ui);
            }

            // Right-aligned toolbar
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Help button
                if serpantinum_icon_button(ui, "btn_help", icons::HELP, &self.theme, true, "Shortcuts (?)").clicked() {
                    self.show_help = !self.show_help;
                }

                ui.add_space(3.0);

                // View Toggle buttons
                let is_grid = self.view_mode == ViewMode::Grid;
                let grid_btn = serpantinum_chip_button(
                    ui,
                    "btn_grid",
                    Some(icons::GRID_VIEW),
                    "Grid",
                    &self.theme,
                    is_grid,
                    "Grid View (Ctrl+2)",
                );
                if grid_btn.clicked() {
                    self.view_mode = ViewMode::Grid;
                }

                let list_btn = serpantinum_chip_button(
                    ui,
                    "btn_list",
                    Some(icons::VIEW_LIST),
                    "List",
                    &self.theme,
                    !is_grid,
                    "List View (Ctrl+1)",
                );
                if list_btn.clicked() {
                    self.view_mode = ViewMode::List;
                }

                ui.add_space(3.0);

                // Hidden files toggle
                let hidden_icon = if self.show_hidden { icons::VISIBILITY } else { icons::VISIBILITY_OFF };
                let hidden_btn = serpantinum_chip_button(
                    ui,
                    "btn_hidden",
                    Some(hidden_icon),
                    if self.show_hidden { "Hidden" } else { "Show" },
                    &self.theme,
                    self.show_hidden,
                    "Toggle Hidden Files (Ctrl+H)",
                );
                if hidden_btn.clicked() {
                    self.show_hidden = !self.show_hidden;
                    self.reload_items();
                }

                ui.add_space(8.0);

                // Serpantinum Input.qml replica
                let search_hint = format!("{}  Filter...", icons::SEARCH);
                egui::Frame::new()
                    .fill(self.theme.surface0)
                    .corner_radius(egui::CornerRadius::same(12))
                    .stroke(egui::Stroke::new(1.0, self.theme.surface1))
                    .inner_margin(egui::Margin::symmetric(14, 6))
                    .show(ui, |ui| {
                        let search_resp = ui.add(
                            egui::TextEdit::singleline(&mut self.search_query)
                                .font(egui::FontId::proportional(13.0))
                                .desired_width(140.0)
                                .frame(egui::Frame::new())
                                .hint_text(search_hint),
                        );
                        if search_resp.changed() {
                            self.reload_items();
                        }
                    });
            });
        });
    }

    fn render_breadcrumbs(&mut self, ui: &mut egui::Ui) {
        let mut target_path: Option<PathBuf> = None;

        egui::ScrollArea::horizontal()
            .id_salt("breadcrumb_scroll")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut accumulated = PathBuf::new();

                    for (i, comp) in self.current_path.components().enumerate() {
                        match comp {
                            Component::RootDir => {
                                accumulated.push("/");
                                let is_current = accumulated == self.current_path;
                                let chip = serpantinum_chip_button(
                                    ui,
                                    "crumb_root",
                                    Some(icons::STORAGE),
                                    "/",
                                    &self.theme,
                                    is_current,
                                    "Root filesystem",
                                );
                                if chip.clicked() {
                                    target_path = Some(PathBuf::from("/"));
                                }
                            }
                            Component::Normal(segment) => {
                                accumulated.push(segment);
                                let name = segment.to_string_lossy();
                                let path_clone = accumulated.clone();

                                if i > 1 {
                                    ui.label(
                                        egui::RichText::new(icons::CHEVRON_RIGHT)
                                            .size(16.0)
                                            .color(self.theme.subtext1),
                                    );
                                }

                                let is_current = accumulated == self.current_path;
                                let chip = serpantinum_chip_button(
                                    ui,
                                    format!("crumb_{i}_{name}"),
                                    None,
                                    name.as_ref(),
                                    &self.theme,
                                    is_current,
                                    "",
                                );

                                if chip.clicked() {
                                    target_path = Some(path_clone);
                                }
                            }
                            _ => {}
                        }
                    }

                    // Path edit button
                    if serpantinum_icon_button(ui, "btn_edit_path", icons::EDIT, &self.theme, true, "Edit Path (Ctrl+L)").clicked() {
                        self.is_editing_path = true;
                        self.path_input = self.current_path.to_string_lossy().to_string();
                    }
                });
            });

        if let Some(target) = target_path {
            self.navigate_to(target);
        }
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("FAVORITES")
                .size(11.0)
                .strong()
                .color(self.theme.subtext1),
        );
        ui.add_space(10.0);

        let mut destination: Option<PathBuf> = None;

        for (idx, place) in self.places.iter().enumerate() {
            let is_selected = self.current_path == place.path;
            let item_resp = serpantinum_nav_item(
                ui,
                format!("place_{idx}"),
                place.icon,
                &place.name,
                &self.theme,
                is_selected,
            );

            if item_resp.clicked() {
                destination = Some(place.path.clone());
            }
        }

        if let Some(dest) = destination {
            self.navigate_to(dest);
        }
    }

    fn render_list_view(&mut self, ui: &mut egui::Ui) {
        let mut clicked_item: Option<FileItem> = None;
        let mut secondary_clicked_item: Option<FileItem> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Column Headers
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                if ui.button(
                    egui::RichText::new(format!("NAME {}", self.sort_symbol(SortBy::Name)))
                        .size(11.5)
                        .strong()
                        .color(self.theme.subtext1),
                ).clicked() {
                    self.toggle_sort(SortBy::Name);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(18.0);
                    if ui.button(
                        egui::RichText::new(format!("MODIFIED {}", self.sort_symbol(SortBy::Modified)))
                            .size(11.5)
                            .strong()
                            .color(self.theme.subtext1),
                    ).clicked() {
                        self.toggle_sort(SortBy::Modified);
                    }

                    ui.add_space(32.0);
                    if ui.button(
                        egui::RichText::new(format!("SIZE {}", self.sort_symbol(SortBy::Size)))
                            .size(11.5)
                            .strong()
                            .color(self.theme.subtext1),
                    ).clicked() {
                        self.toggle_sort(SortBy::Size);
                    }
                });
            });

            ui.add_space(6.0);

            let should_scroll = self.scroll_to_selected;
            self.scroll_to_selected = false;

            for (idx, item) in self.items.iter().enumerate() {
                let is_selected = self.selected_index == Some(idx);

                let (_, clicked, double_clicked, secondary_clicked) = serpantinum_file_row(
                    ui,
                    idx,
                    item,
                    &self.theme,
                    is_selected,
                    should_scroll,
                );

                if clicked {
                    self.selected_index = Some(idx);
                }

                if double_clicked {
                    clicked_item = Some(item.clone());
                }

                if secondary_clicked {
                    self.selected_index = Some(idx);
                    secondary_clicked_item = Some(item.clone());
                }
            }
        });

        if let Some(item) = clicked_item {
            if item.is_dir {
                self.navigate_to(item.path);
            } else {
                self.open_file(&item.path);
            }
        }

        if let Some(item) = secondary_clicked_item {
            ui.ctx().copy_text(item.path.to_string_lossy().to_string());
            self.set_toast(format!("Copied: {}", item.name));
        }
    }

    fn render_grid_view(&mut self, ui: &mut egui::Ui) {
        let mut clicked_item: Option<FileItem> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            let available_width = ui.available_width();
            let item_width = 110.0;
            let item_height = 100.0;
            let cols = (available_width / item_width).floor().max(1.0) as usize;

            for (row_idx, row) in self.items.chunks(cols).enumerate() {
                ui.horizontal(|ui| {
                    for (col_idx, item) in row.iter().enumerate() {
                        let idx = row_idx * cols + col_idx;
                        let is_selected = self.selected_index == Some(idx);

                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(item_width, item_height),
                            egui::Sense::click(),
                        );

                        let id = ui.make_persistent_id(format!("grid_{idx}"));
                        let hover_t = ui.ctx().animate_bool_responsive(id.with("hov"), response.hovered());

                        let target_scale = if response.is_pointer_button_down_on() { 0.98 } else { 1.0 };
                        let current_scale = ui.ctx().animate_value_with_time(id.with("scale"), target_scale, 0.18);

                        let selection_bg = self.theme.with_alpha(self.theme.mauve, 50);
                        let hover_bg = self.theme.with_alpha(self.theme.surface0, 160);

                        let bg_color = if is_selected {
                            lerp_color(selection_bg, self.theme.with_alpha(self.theme.mauve, 75), hover_t)
                        } else {
                            lerp_color(Color32::TRANSPARENT, hover_bg, hover_t)
                        };

                        let animated_rect = egui::Rect::from_center_size(rect.center(), rect.size() * current_scale);

                        if bg_color != Color32::TRANSPARENT {
                            ui.painter().rect_filled(
                                animated_rect,
                                egui::CornerRadius::same(self.theme.border_radius as u8),
                                bg_color,
                            );
                        }

                        // Icon in rounded 36x36 container
                        let icon_box_rect = egui::Rect::from_center_size(
                            egui::pos2(animated_rect.center().x, animated_rect.top() + 34.0),
                            egui::vec2(38.0, 38.0),
                        );

                        let box_bg = if is_selected {
                            self.theme.with_alpha(self.theme.mauve, 85)
                        } else {
                            self.theme.surface2
                        };

                        ui.painter().rect_filled(
                            icon_box_rect,
                            egui::CornerRadius::same(10),
                            box_bg,
                        );

                        let icon_str = if item.is_symlink {
                            icons::SYMLINK
                        } else {
                            item.file_type.icon()
                        };

                        let icon_color = if is_selected {
                            self.theme.text
                        } else if item.is_dir {
                            self.theme.blue
                        } else {
                            self.theme.subtext0
                        };

                        ui.painter().text(
                            icon_box_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            icon_str,
                            egui::FontId::proportional(22.0),
                            icon_color,
                        );

                        // Label
                        let display_name = if item.name.len() > 12 {
                            format!("{}…", &item.name[..11])
                        } else {
                            item.name.clone()
                        };

                        let text_color = if is_selected {
                            self.theme.text
                        } else {
                            lerp_color(
                                self.theme.subtext0,
                                self.theme.text,
                                hover_t,
                            )
                        };

                        let text_pos = egui::pos2(animated_rect.center().x, animated_rect.bottom() - 16.0);
                        ui.painter().text(
                            text_pos,
                            egui::Align2::CENTER_CENTER,
                            display_name,
                            egui::FontId::proportional(12.0),
                            text_color,
                        );

                        if response.clicked() {
                            self.selected_index = Some(idx);
                        }

                        if response.double_clicked() {
                            clicked_item = Some(item.clone());
                        }
                    }
                });
                ui.add_space(8.0);
            }
        });

        if let Some(item) = clicked_item {
            if item.is_dir {
                self.navigate_to(item.path);
            } else {
                self.open_file(&item.path);
            }
        }
    }

    fn render_bottom_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{} items", self.items.len()))
                    .size(12.0)
                    .color(self.theme.subtext1),
            );

            if let Some(idx) = self.selected_index {
                if let Some(item) = self.items.get(idx) {
                    ui.label(egui::RichText::new("•").color(self.theme.surface2));
                    ui.label(
                        egui::RichText::new(&item.name)
                            .size(12.0)
                            .strong()
                            .color(self.theme.text),
                    );
                    ui.label(
                        egui::RichText::new(format!("({})", item.file_type.label()))
                            .size(12.0)
                            .color(self.theme.subtext1),
                    );
                    if !item.is_dir {
                        ui.label(
                            egui::RichText::new(item.formatted_size())
                                .size(12.0)
                                .color(self.theme.subtext0),
                        );
                    }
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some((ref toast, _)) = self.status_toast {
                    ui.colored_label(self.theme.green, format!("{} {toast}", icons::CHECK));
                } else {
                    ui.label(
                        egui::RichText::new("j/k: Navigate • Enter: Open • h: Up • ?: Shortcuts")
                            .size(12.0)
                            .color(self.theme.subtext1),
                    );
                }
            });
        });
    }

    fn render_help_modal(&mut self, ctx: &egui::Context) {
        egui::Window::new("Shortcuts")
            .collapsible(false)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(self.theme.mantle)
                    .corner_radius(egui::CornerRadius::same(self.theme.border_radius as u8))
                    .stroke(egui::Stroke::new(1.0, self.theme.surface1))
                    .inner_margin(egui::Margin::symmetric(24, 20)),
            )
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.add_space(4.0);
                egui::Grid::new("shortcuts_grid")
                    .num_columns(2)
                    .spacing([28.0, 10.0])
                    .show(ui, |ui| {
                        let shortcuts = [
                            ("j / ↓", "Move selection down"),
                            ("k / ↑", "Move selection up"),
                            ("Enter / l", "Open file / enter folder"),
                            ("Backspace / h", "Go to parent folder"),
                            ("Alt + ← / →", "Navigate history"),
                            ("Home / g", "Jump to top"),
                            ("End / G", "Jump to bottom"),
                            ("/ or Ctrl + F", "Filter files"),
                            ("Ctrl + L", "Direct path editing"),
                            ("Ctrl + H", "Toggle hidden files"),
                            ("Ctrl + 1 / 2", "List / Grid view"),
                            ("y / Ctrl + C", "Copy file path"),
                            ("F5 / Ctrl + R", "Refresh & reload theme"),
                            ("? / Esc", "Toggle this dialog"),
                        ];

                        for (key, desc) in shortcuts {
                            ui.label(
                                egui::RichText::new(key)
                                    .strong()
                                    .color(self.theme.mauve),
                            );
                            ui.label(
                                egui::RichText::new(desc)
                                    .color(self.theme.text),
                            );
                            ui.end_row();
                        }
                    });

                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    if ui.button(
                        egui::RichText::new("Close (Esc)")
                            .size(13.0)
                            .color(self.theme.mauve),
                    ).clicked() {
                        self.show_help = false;
                    }
                });
            });
    }

    fn sort_symbol(&self, field: SortBy) -> &'static str {
        if self.sort_by == field {
            match self.sort_order {
                SortOrder::Ascending => "▲",
                SortOrder::Descending => "▼",
            }
        } else {
            ""
        }
    }

    fn toggle_sort(&mut self, field: SortBy) {
        if self.sort_by == field {
            self.sort_order = match self.sort_order {
                SortOrder::Ascending => SortOrder::Descending,
                SortOrder::Descending => SortOrder::Ascending,
            };
        } else {
            self.sort_by = field;
            self.sort_order = SortOrder::Ascending;
        }
        self.reload_items();
    }
}

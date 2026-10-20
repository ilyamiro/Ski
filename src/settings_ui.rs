use std::path::PathBuf;

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Vec2};

use crate::app::{app_row, Dialog, SkiApp, ViewMode};
use crate::fs::apps::{self, AppEntry};
use crate::fs::scanner::{SortBy, SortOrder};
use crate::keymap::{self, Action};
use crate::settings::{Density, Settings, StartLocation};
use crate::ui::fonts::SystemFont;
use crate::ui::icons;
use crate::ui::material::{self, button, chip, choice_row, filled_icon_button, group_card, nav_item, switch_row_with, ButtonKind, Column};
use crate::ui::theme::{self, SerpantinumTheme, ACCENTS, PRESETS};

impl SkiApp {
    pub(crate) fn open_settings_dialog(&mut self) {
        self.capturing_key = false;
        self.dialog = Some(Dialog::Settings {
            tab: 0,
            font_query: String::new(),
            hex: self.settings.accent.clone().unwrap_or_default(),
            picking: None,
            defaults: Vec::new(),
            keybind_search: String::new(),
            capture: None,
            conflict_warning: None,
        });
    }

    pub(crate) fn persist_settings(&mut self) {
        self.settings.view_mode = self.view_mode;
        self.settings.sort_by = self.sort_by;
        self.settings.sort_order = self.sort_order;
        self.settings.show_hidden = self.show_hidden;
        self.settings.folders_first = self.folders_first;
        self.settings.columns = self.columns.clone();
        self.settings.save();
        self.saved_settings = self.settings.clone();
    }

    pub(crate) fn settings_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Settings {
            tab,
            font_query,
            hex,
            picking,
            defaults,
            keybind_search,
            capture,
            conflict_warning,
        } = dialog else {
            return false;
        };

        let ctx = ui.ctx().clone();
        let theme = self.theme;
        let close = material::dialog_header(ui, "Settings", None, &theme, "set_close");
        ui.add_space(4.0);

        let max_height = (ctx.content_rect().height() - 160.0).clamp(320.0, 540.0);

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            // The dialog keeps one height for every category
            ui.set_min_height(max_height);
            // Left pane: categories
            ui.vertical(|ui| {
                ui.set_width(168.0);
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);

                let categories = [
                    (0, icons::COG, "General"),
                    (1, icons::TUNE, "Appearance"),
                    (2, icons::VIEW_LIST, "Files & View"),
                    (3, icons::SETTINGS, "Behavior"),
                    (4, icons::KEYBOARD, "Keybindings"),
                    (5, icons::APPS, "Applications"),
                ];

                for (idx, icon, label) in categories {
                    if nav_item(ui, ("set_cat_nav", idx), icon, label, &theme, *tab == idx).clicked() {
                        *tab = idx;
                        if self.capturing_key {
                            self.capturing_key = false;
                            *capture = None;
                            *conflict_warning = None;
                        }
                    }
                }
            });

            // Right pane: the settings of the chosen category
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                let enter_t = material::animate_enter(ui.ctx(), ui.make_persistent_id(("set_tab_enter", *tab)), material::ANIM_FAST);
                ui.set_opacity(enter_t);
                egui::ScrollArea::vertical()
                    .max_height(max_height)
                    .auto_shrink([false, false])
                    .id_salt(("set_scroll", *tab))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
                        match *tab {
                            0 => self.general_tab(ui),
                            1 => self.appearance_tab(ui, font_query, hex),
                            2 => self.files_view_tab(ui),
                            3 => self.behavior_tab(ui),
                            4 => self.keybindings_tab(ui, keybind_search, capture, conflict_warning),
                            _ => self.applications_tab(ui, picking, defaults),
                        }
                    });
            });
        });

        if close {
            self.capturing_key = false;
        }

        close
    }

    fn general_tab(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;

        material::section_label(ui, "Startup", &theme);
        group_card(ui, &theme, |ui| {
            let locations = [StartLocation::Launch, StartLocation::Home, StartLocation::LastVisited, StartLocation::Custom];
            let labels = ["Launch folder", "Home", "Last visited", "Custom"];
            let current = locations.iter().position(|l| *l == self.settings.start_location).unwrap_or(0);
            if let Some(i) = choice_row(ui, "start_loc", icons::HOME, "Open Ski in", &labels, current, &theme) {
                self.settings.start_location = locations[i];
                self.persist_settings();
            }

            material::collapsible(ui, "start_custom_path", self.settings.start_location == StartLocation::Custom, |ui| {
                let mut path_str = self.settings.start_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                let valid = path_str.is_empty() || PathBuf::from(&path_str).is_dir();
                material::input_frame(&theme).show(ui, |ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut path_str)
                            .frame(egui::Frame::NONE)
                            .desired_width(f32::INFINITY)
                            .hint_text("/path/to/folder")
                            .text_color(if valid { theme.text } else { theme.red })
                            .font(egui::FontId::proportional(material::FONT_BODY)),
                    );
                    if resp.changed() {
                        self.settings.start_path = if path_str.trim().is_empty() { None } else { Some(PathBuf::from(path_str.trim())) };
                        self.persist_settings();
                    }
                });
                if !valid {
                    material::hint_label(ui, "That folder does not exist; Ski opens in Home instead", &theme);
                }
            });
        });

        material::section_label(ui, "Windows", &theme);
        group_card(ui, &theme, |ui| {
            if switch_row_with(ui, "open_in_new_win", icons::NEW_WINDOW, "Open folders in a new window", None, &theme, self.settings.open_in_new_window).clicked() {
                self.settings.open_in_new_window = !self.settings.open_in_new_window;
                self.persist_settings();
            }
        });

        material::section_label(ui, "File operations", &theme);
        group_card(ui, &theme, |ui| {
            let desc = Some("Ask before deleting for good or emptying the trash");
            if switch_row_with(ui, "confirm_delete", icons::DELETE_FOREVER, "Confirm permanent deletion", desc, &theme, self.settings.confirm_delete).clicked() {
                self.settings.confirm_delete = !self.settings.confirm_delete;
                self.persist_settings();
            }
        });
    }

    fn appearance_tab(&mut self, ui: &mut egui::Ui, font_query: &mut String, hex: &mut String) {
        let theme = self.theme;

        material::section_label(ui, "Theme", &theme);
        // The cards share the row evenly, so the last one never wraps onto a line of its own
        let gap = 8.0;
        let count = PRESETS.len() + 1;
        let card_w = ((ui.available_width() - gap * (count - 1) as f32) / count as f32).floor().clamp(80.0, 120.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            let mut choices: Vec<(&str, &str, SerpantinumTheme)> = vec![("system", "System", self.system_theme)];
            for (id, label, ..) in PRESETS.iter() {
                let preset = Settings {
                    theme: (*id).to_owned(),
                    accent: None,
                    ..Settings::default()
                };
                choices.push((id, label, SerpantinumTheme::from_settings(&preset)));
            }
            for (id, label, preview) in choices {
                if theme_card(ui, ("theme_card", id), card_w, label, &preview, &theme, self.settings.theme == id).clicked() {
                    self.settings.theme = id.to_owned();
                    self.rebuild_theme();
                    self.persist_settings();
                }
            }
        });

        material::section_label(ui, "Accent color", &theme);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            if chip(ui, "accent_default", None, "Theme default", &theme, self.settings.accent.is_none()).clicked() {
                self.settings.accent = None;
                hex.clear();
                self.rebuild_theme();
                self.persist_settings();
            }
            for color in ACCENTS {
                let c = theme::parse_hex(color).unwrap_or(Color32::GRAY);
                let on = self.settings.accent.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(color));
                if swatch(ui, ("accent", color), c, &theme, on).clicked() {
                    self.settings.accent = Some(color.to_owned());
                    *hex = color.to_owned();
                    self.rebuild_theme();
                    self.persist_settings();
                }
            }
        });
        ui.horizontal(|ui| {
            material::input_frame(&theme).show(ui, |ui| {
                ui.set_width(120.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(hex)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .hint_text("#rrggbb")
                        .font(egui::FontId::proportional(material::FONT_BODY)),
                );
                if resp.changed() {
                    let text = hex.trim().to_owned();
                    if text.len() == 7 && theme::parse_hex(&text).is_some() {
                        self.settings.accent = Some(text.to_lowercase());
                        self.rebuild_theme();
                        self.persist_settings();
                    }
                }
            });
            ui.label(egui::RichText::new("Custom hex color").size(material::FONT_CAPTION).color(theme.subtext1));
        });

        material::section_label(ui, "Interface", &theme);
        group_card(ui, &theme, |ui| {
            let scales = [0.85, 1.0, 1.15, 1.3];
            let current = scales.iter().position(|s| (self.settings.scale - s).abs() < 0.01).unwrap_or(1);
            if let Some(i) = choice_row(ui, "scale", icons::VIEW_LIST, "Size", &["Compact", "Default", "Large", "Larger"], current, &theme) {
                self.settings.scale = scales[i];
                ui.ctx().set_zoom_factor(scales[i]);
                self.persist_settings();
            }
            if switch_row_with(ui, "app_animations", icons::REFRESH, "Animations", None, &theme, self.settings.animations).clicked() {
                self.settings.animations = !self.settings.animations;
                self.persist_settings();
            }
        });

        material::section_label(ui, "Font", &theme);
        let current = self.settings.font_name.clone().unwrap_or_else(|| "Adwaita Sans (default)".to_owned());
        material::hint_label(ui, &format!("Current: {current}"), &theme);
        material::input_frame(&theme).show(ui, |ui| {
            ui.horizontal(|ui| {
                material::inline_icon(ui, icons::SEARCH, material::ICON_SM, theme.subtext1);
                ui.add(
                    egui::TextEdit::singleline(font_query)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .hint_text("Search installed fonts")
                        .font(egui::FontId::proportional(material::FONT_BODY)),
                );
            });
        });
        let needle = font_query.trim().to_lowercase();
        let all_fonts = crate::ui::fonts::system_fonts();
        let fonts: Vec<&SystemFont> = all_fonts
            .iter()
            .filter(|f| needle.is_empty() || f.family.to_lowercase().contains(&needle))
            .collect();
        let show_default = needle.is_empty() || "adwaita sans".contains(&needle);
        let total = fonts.len() + usize::from(show_default);
        let mut picked: Option<Option<SystemFont>> = None;
        group_card(ui, &theme, |ui| {
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .auto_shrink([false, true])
                .id_salt("font_list")
                .show_rows(ui, material::ENTRY_HEIGHT + 2.0, total, |ui, range| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for row in range {
                        if show_default && row == 0 {
                            if nav_item(ui, "font_default", icons::DOCUMENT, "Adwaita Sans (default)", &theme, self.settings.font.is_none()).clicked() {
                                picked = Some(None);
                            }
                            continue;
                        }
                        let font = fonts[row - usize::from(show_default)];
                        let on = self.settings.font.as_deref() == Some(font.path.as_path());
                        if nav_item(ui, ("font_row", &font.path), icons::DOCUMENT, &font.family, &theme, on).clicked() {
                            picked = Some(Some(font.clone()));
                        }
                    }
                });
        });
        if let Some(choice) = picked {
            self.settings.font = choice.as_ref().map(|f| f.path.clone());
            self.settings.font_name = choice.map(|f| f.family);
            self.fonts_dirty = true;
            self.persist_settings();
        }
    }

    fn files_view_tab(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;

        material::section_label(ui, "View", &theme);
        group_card(ui, &theme, |ui| {
            if let Some(i) = choice_row(ui, "fv_view", icons::VIEW_LIST, "Default view", &["List", "Grid"], usize::from(self.view_mode == ViewMode::Grid), &theme) {
                self.view_mode = if i == 0 { ViewMode::List } else { ViewMode::Grid };
                self.settings.view_mode = self.view_mode;
                self.persist_settings();
            }
            let fields = [SortBy::Name, SortBy::Size, SortBy::Modified];
            let current = fields.iter().position(|f| *f == self.sort_by).unwrap_or(0);
            if let Some(i) = choice_row(ui, "fv_sort_by", icons::SORT_ASC, "Sort by", &["Name", "Size", "Modified"], current, &theme) {
                self.sort_by = fields[i];
                self.settings.sort_by = fields[i];
                self.reload_items();
                self.persist_settings();
            }
            let asc = self.sort_order == SortOrder::Ascending;
            if let Some(i) = choice_row(ui, "fv_sort_order", icons::SORT_DESC, "Order", &["Ascending", "Descending"], usize::from(!asc), &theme) {
                self.sort_order = if i == 0 { SortOrder::Ascending } else { SortOrder::Descending };
                self.settings.sort_order = self.sort_order;
                self.reload_items();
                self.persist_settings();
            }
        });

        material::section_label(ui, "Layout", &theme);
        group_card(ui, &theme, |ui| {
            if switch_row_with(ui, "fv_show_hidden", icons::VISIBILITY, "Show hidden files", None, &theme, self.show_hidden).clicked() {
                self.show_hidden = !self.show_hidden;
                self.settings.show_hidden = self.show_hidden;
                self.reload_items();
                self.persist_settings();
            }
            if switch_row_with(ui, "fv_folders_first", icons::FOLDER, "Folders before files", None, &theme, self.folders_first).clicked() {
                self.folders_first = !self.folders_first;
                self.settings.folders_first = self.folders_first;
                self.reload_items();
                self.persist_settings();
            }
            if switch_row_with(ui, "fv_show_sidebar", icons::MENU, "Sidebar", None, &theme, self.settings.show_sidebar).clicked() {
                self.settings.show_sidebar = !self.settings.show_sidebar;
                self.persist_settings();
            }
            if switch_row_with(ui, "fv_show_status", icons::INFO, "Status bar", None, &theme, self.settings.show_status_bar).clicked() {
                self.settings.show_status_bar = !self.settings.show_status_bar;
                self.persist_settings();
            }
            let desc = Some("Each folder keeps its own list or grid choice");
            if switch_row_with(ui, "fv_remember_view", icons::FOLDER_MOVE, "Remember view per folder", desc, &theme, self.settings.remember_view_per_folder).clicked() {
                self.settings.remember_view_per_folder = !self.settings.remember_view_per_folder;
                self.persist_settings();
            }
        });

        material::section_label(ui, "List view", &theme);
        group_card(ui, &theme, |ui| {
            let densities = [Density::Compact, Density::Comfortable, Density::Spacious];
            let current = densities.iter().position(|d| *d == self.settings.density).unwrap_or(1);
            if let Some(i) = choice_row(ui, "fv_density", icons::VIEW_LIST, "Row height", &["Compact", "Comfortable", "Spacious"], current, &theme) {
                self.settings.density = densities[i];
                self.persist_settings();
            }
            ui.add_space(4.0);
            material::hint_label(ui, "Columns", &theme);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(4.0);
                for col in Column::ALL {
                    let on = self.columns.contains(&col);
                    if chip(ui, ("fv_col", col.label()), None, col.label(), &theme, on).clicked() {
                        self.columns = Column::ALL
                            .into_iter()
                            .filter(|c| (*c == col) != self.columns.contains(c))
                            .collect();
                        self.settings.columns = self.columns.clone();
                        self.persist_settings();
                    }
                }
            });
            ui.add_space(4.0);
        });

        material::section_label(ui, "Grid view", &theme);
        group_card(ui, &theme, |ui| {
            let size = self.settings.grid_size;
            let (less, more) = material::stepper_row(ui, "fv_grid_size", "Icon size", &theme, &size.label(), size.can_step(-1), size.can_step(1));
            if less || more {
                self.settings.grid_size = size.step(if more { 1 } else { -1 });
                self.persist_settings();
            }
            let lines = [1u8, 2, 3];
            let current = lines.iter().position(|l| *l == self.settings.grid_name_lines).unwrap_or(1);
            if let Some(i) = choice_row(ui, "fv_grid_lines", icons::TEXT, "Name lines", &["1", "2", "3"], current, &theme) {
                self.settings.grid_name_lines = lines[i];
                self.persist_settings();
            }
        });

        material::section_label(ui, "Details", &theme);
        group_card(ui, &theme, |ui| {
            if switch_row_with(ui, "fv_show_thumbs", icons::IMAGE, "Thumbnails", None, &theme, self.settings.show_thumbnails).clicked() {
                self.settings.show_thumbnails = !self.settings.show_thumbnails;
                self.persist_settings();
            }
            if switch_row_with(ui, "fv_show_exts", icons::DOCUMENT, "File extensions", None, &theme, self.settings.show_extensions).clicked() {
                self.settings.show_extensions = !self.settings.show_extensions;
                self.persist_settings();
            }
            let desc = Some("Show “2 hours ago” instead of a date");
            if switch_row_with(ui, "fv_relative_dates", icons::RECENT, "Relative dates", desc, &theme, self.settings.relative_dates).clicked() {
                self.settings.relative_dates = !self.settings.relative_dates;
                self.persist_settings();
            }
            let current = usize::from(self.settings.decimal_units);
            if let Some(i) = choice_row(ui, "fv_units", icons::INFO, "File size units", &["1024 B = 1 KiB", "1000 B = 1 KB"], current, &theme) {
                self.settings.decimal_units = i == 1;
                self.persist_settings();
            }
        });
    }

    fn behavior_tab(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;

        material::section_label(ui, "Mouse", &theme);
        group_card(ui, &theme, |ui| {
            let desc = Some("Otherwise double-click opens, a single click only selects");
            if switch_row_with(ui, "beh_single_click", icons::OPEN, "Open with a single click", desc, &theme, self.settings.single_click_open).clicked() {
                self.settings.single_click_open = !self.settings.single_click_open;
                self.persist_settings();
            }
        });

        material::section_label(ui, "Keyboard", &theme);
        group_card(ui, &theme, |ui| {
            let desc = Some("h j k l to move, g / G to jump, y to copy the path");
            if switch_row_with(ui, "beh_vim_keys", icons::KEYBOARD, "Vim navigation keys", desc, &theme, self.settings.vim_keys).clicked() {
                self.settings.vim_keys = !self.settings.vim_keys;
                self.persist_settings();
            }
            let desc = Some("Start typing to filter, without pressing / first");
            if switch_row_with(ui, "beh_type_to_filter", icons::SEARCH, "Type to filter", desc, &theme, self.settings.type_to_filter).clicked() {
                self.settings.type_to_filter = !self.settings.type_to_filter;
                self.persist_settings();
            }
        });
    }

    fn keybindings_tab(
        &mut self,
        ui: &mut egui::Ui,
        keybind_search: &mut String,
        capture: &mut Option<(Action, Option<usize>)>,
        conflict_warning: &mut Option<(String, Action, Action, Option<usize>)>,
    ) {
        let theme = self.theme;

        // Top bar: Search box and Reset all to defaults
        ui.horizontal(|ui| {
            material::input_frame(&theme).show(ui, |ui| {
                ui.horizontal(|ui| {
                    material::inline_icon(ui, icons::SEARCH, material::ICON_SM, theme.subtext1);
                    ui.add(
                        egui::TextEdit::singleline(keybind_search)
                            .frame(egui::Frame::NONE)
                            .desired_width(ui.available_width().min(210.0))
                            .hint_text("Search shortcuts")
                            .font(egui::FontId::proportional(material::FONT_BODY)),
                    );
                });
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, "reset_all_keybinds", Some(icons::REFRESH), "Reset all", &theme, ButtonKind::Tonal, true).clicked() {
                    self.settings.keybinds.clear();
                    self.persist_settings();
                    *capture = None;
                    *conflict_warning = None;
                    self.capturing_key = false;
                }
            });
        });

        // Conflict warning banner
        if let Some((binding, target_act, conf_act, target_idx)) = conflict_warning.clone() {
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(theme.with_alpha(theme.yellow, 30))
                .corner_radius(CornerRadius::same(material::RADIUS_MD))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        material::inline_icon(ui, icons::INFO, material::ICON_MD, theme.yellow);
                        ui.label(
                            egui::RichText::new(format!(
                                "“{}” is already used by “{}”. Replace it?",
                                keymap::display(&binding),
                                conf_act.label()
                            ))
                            .size(material::FONT_BODY)
                            .color(theme.text),
                        );
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if button(ui, "conf_replace", Some(icons::CHECK), "Replace", &theme, ButtonKind::Filled, true).clicked() {
                            // 1. Remove from conflicting action
                            let mut conf_bindings = keymap::bindings(&self.settings.keybinds, conf_act);
                            conf_bindings.retain(|b| b != &binding);
                            if conf_bindings == conf_act.default_bindings() {
                                self.settings.keybinds.remove(conf_act.id());
                            } else {
                                self.settings.keybinds.insert(conf_act.id().to_string(), conf_bindings);
                            }

                            // 2. Add / replace in target action
                            let mut target_bindings = keymap::bindings(&self.settings.keybinds, target_act);
                            target_bindings.retain(|b| b != &binding);
                            if let Some(idx) = target_idx {
                                if idx < target_bindings.len() {
                                    target_bindings[idx] = binding.clone();
                                } else {
                                    target_bindings.push(binding.clone());
                                }
                            } else {
                                target_bindings.push(binding.clone());
                            }
                            if target_bindings == target_act.default_bindings() {
                                self.settings.keybinds.remove(target_act.id());
                            } else {
                                self.settings.keybinds.insert(target_act.id().to_string(), target_bindings);
                            }

                            self.persist_settings();
                            *conflict_warning = None;
                            *capture = None;
                            self.capturing_key = false;
                        }
                        if button(ui, "conf_cancel", None, "Cancel", &theme, ButtonKind::Text, true).clicked() {
                            *conflict_warning = None;
                            *capture = None;
                            self.capturing_key = false;
                        }
                    });
                });
            ui.add_space(4.0);
        }

        // Active key capture handler
        if let Some((target_act, target_idx)) = *capture {
            self.capturing_key = true;
            let mut handled_key = None;
            let mut cancel = false;
            let mut remove_binding = false;

            ui.input(|i| {
                for event in &i.events {
                    if let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = event {
                        match key {
                            egui::Key::Escape => {
                                cancel = true;
                                break;
                            }
                            egui::Key::Backspace | egui::Key::Delete if modifiers.is_none() && target_idx.is_some() => {
                                remove_binding = true;
                                break;
                            }
                            egui::Key::ShiftLeft | egui::Key::ShiftRight |
                            egui::Key::ControlLeft | egui::Key::ControlRight |
                            egui::Key::AltLeft | egui::Key::AltRight |
                            egui::Key::SuperLeft | egui::Key::SuperRight => {
                                // Bare modifier: ignore
                                continue;
                            }
                            _ => {
                                let formatted = keymap::format_binding(*key, *modifiers);
                                handled_key = Some(formatted);
                                break;
                            }
                        }
                    }
                }
            });

            if cancel {
                *capture = None;
                self.capturing_key = false;
            } else if remove_binding {
                if let Some(idx) = target_idx {
                    let mut bindings = keymap::bindings(&self.settings.keybinds, target_act);
                    if idx < bindings.len() {
                        bindings.remove(idx);
                        if bindings == target_act.default_bindings() {
                            self.settings.keybinds.remove(target_act.id());
                        } else {
                            self.settings.keybinds.insert(target_act.id().to_string(), bindings);
                        }
                        self.persist_settings();
                    }
                }
                *capture = None;
                self.capturing_key = false;
            } else if let Some(new_bind) = handled_key {
                let mut conflicting = None;
                for other_act in Action::ALL {
                    if other_act == target_act {
                        continue;
                    }
                    let other_bindings = keymap::bindings(&self.settings.keybinds, other_act);
                    if other_bindings.contains(&new_bind) {
                        conflicting = Some(other_act);
                        break;
                    }
                }

                if let Some(conf_act) = conflicting {
                    *conflict_warning = Some((new_bind, target_act, conf_act, target_idx));
                    *capture = None;
                } else {
                    let mut bindings = keymap::bindings(&self.settings.keybinds, target_act);
                    if let Some(idx) = target_idx {
                        if idx < bindings.len() {
                            bindings[idx] = new_bind;
                        } else {
                            bindings.push(new_bind);
                        }
                    } else if !bindings.contains(&new_bind) {
                        bindings.push(new_bind);
                    }
                    if bindings == target_act.default_bindings() {
                        self.settings.keybinds.remove(target_act.id());
                    } else {
                        self.settings.keybinds.insert(target_act.id().to_string(), bindings);
                    }
                    self.persist_settings();
                    *capture = None;
                    self.capturing_key = false;
                }
            }
        }

        // Action list grouped by Action::group()
        let search = keybind_search.trim().to_lowercase();
        let groups = ["Navigation", "View", "Files"];

        for group_name in groups {
            let actions_in_group: Vec<Action> = Action::ALL
                .into_iter()
                .filter(|a| a.group() == group_name)
                .filter(|a| {
                    search.is_empty()
                        || a.label().to_lowercase().contains(&search)
                        || a.id().contains(&search)
                        || group_name.to_lowercase().contains(&search)
                })
                .collect();

            if actions_in_group.is_empty() {
                continue;
            }

            material::section_label(ui, group_name, &theme);

            group_card(ui, &theme, |ui| {
            for action in actions_in_group {
                let is_overridden = self.settings.keybinds.contains_key(action.id());

                ui.horizontal(|ui| {
                    ui.set_min_height(material::ENTRY_HEIGHT);
                    ui.add_space(8.0);

                    // Action label
                    let label_color = if is_overridden { theme.mauve } else { theme.text };
                    ui.label(egui::RichText::new(action.label()).size(material::FONT_BODY).color(label_color));

                    // Per-action reset button if overridden
                    if is_overridden
                        && filled_icon_button(ui, ("rst_act", action.id()), icons::REFRESH, &theme, 22.0, "Reset to default", None).clicked()
                    {
                        self.settings.keybinds.remove(action.id());
                        self.persist_settings();
                        if matches!(*capture, Some((a, _)) if a == action) {
                            *capture = None;
                            self.capturing_key = false;
                        }
                    }

                    // Keycap pills aligned to right
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        // "+ Add" button
                        let is_adding = *capture == Some((action, None));
                        let add_label = if is_adding { "Press keys…" } else { "+ Add" };
                        let (add_resp, _) = keycap_pill(ui, ("act_add", action.id()), add_label, &theme, is_adding, false);
                        if add_resp.clicked() {
                            if is_adding {
                                *capture = None;
                                self.capturing_key = false;
                            } else {
                                *capture = Some((action, None));
                                *conflict_warning = None;
                                self.capturing_key = true;
                            }
                        }

                        // Existing bindings
                        let current_bindings = keymap::bindings(&self.settings.keybinds, action);
                        for (idx, bind_str) in current_bindings.iter().enumerate().rev() {
                            let is_capturing_this = *capture == Some((action, Some(idx)));
                            let shown = keymap::display(bind_str);
                            let pill_text = if is_capturing_this { "Press keys…" } else { shown.as_str() };
                            let (pill_resp, rem_clicked) = keycap_pill(
                                ui,
                                ("act_pill", action.id(), idx),
                                pill_text,
                                &theme,
                                is_capturing_this,
                                true,
                            );

                            if rem_clicked {
                                let mut b = keymap::bindings(&self.settings.keybinds, action);
                                if idx < b.len() {
                                    b.remove(idx);
                                    if b == action.default_bindings() {
                                        self.settings.keybinds.remove(action.id());
                                    } else {
                                        self.settings.keybinds.insert(action.id().to_string(), b);
                                    }
                                    self.persist_settings();
                                }
                                if is_capturing_this {
                                    *capture = None;
                                    self.capturing_key = false;
                                }
                            } else if pill_resp.clicked() {
                                if is_capturing_this {
                                    *capture = None;
                                    self.capturing_key = false;
                                } else {
                                    *capture = Some((action, Some(idx)));
                                    *conflict_warning = None;
                                    self.capturing_key = true;
                                }
                            }
                        }
                    });
                });
            }
            });
        }
    }

    fn applications_tab(&mut self, ui: &mut egui::Ui, picking: &mut Option<usize>, defaults: &mut Vec<Option<String>>) {
        if defaults.is_empty() {
            *defaults = apps::CATEGORIES.iter().map(|c| apps::default_for(c.mimes[0])).collect();
        }
        let theme = self.theme;
        material::section_label(ui, "Default applications", &theme);
        material::hint_label(ui, "The application that opens each kind of file. This changes the system-wide defaults.", &theme);
        ui.spacing_mut().item_spacing.y = 2.0;
        let installed = self.installed_apps();
        for (idx, category) in apps::CATEGORIES.iter().enumerate() {
            let current = defaults.get(idx).and_then(|d| d.as_deref()).and_then(|id| installed.iter().find(|a| a.id == id));
            let value = current.map_or("Not set", |a| a.name.as_str());
            let is_open = *picking == Some(idx);
            if category_row(ui, ("cat_row", idx), category.icon, category.label, value, &theme, is_open).clicked() {
                *picking = if is_open { None } else { Some(idx) };
            }
            material::collapsible(ui, ("cat_collapse", idx), *picking == Some(idx), |ui| {
                let default_app = defaults.get(idx).and_then(|d| d.as_deref());
                let candidates = apps::apps_for(installed, category.mimes[0], default_app);
                let mut chosen: Option<&AppEntry> = None;
                for app in &candidates {
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
                    let on = defaults[idx].as_deref() == Some(app.id.as_str());
                    if app_row(ui, ("cat_app", idx, &app.id), tex.as_ref(), &app.name, &theme, on).clicked() {
                        chosen = Some(app);
                    }
                }
                if candidates.is_empty() {
                    material::hint_label(ui, "No installed application handles these files", &theme);
                }
                if let Some(app) = chosen {
                    apps::set_category_default(category, &app.id);
                    defaults[idx] = Some(app.id.clone());
                    self.set_toast(format!("{} is now the default for {}", app.name, category.label.to_lowercase()));
                    *picking = None;
                }
                ui.add_space(4.0);
            });
        }
    }
}

fn keycap_pill(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    text: &str,
    theme: &SerpantinumTheme,
    capturing: bool,
    can_remove: bool,
) -> (egui::Response, bool) {
    let font_id = egui::FontId::proportional(material::FONT_CAPTION);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font_id, theme.text);
    let remove_btn_w = if can_remove && !capturing { 20.0 } else { 0.0 };
    let total_w = galley.size().x + 20.0 + remove_btn_w;

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(total_w, material::KEYCAP_HEIGHT), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = material::fx(ui, id, &resp, true, &material::FxSpec::CLICK);
    let on = material::animate(ui.ctx(), id.with("cap"), capturing);
    let r = material::scaled(rect, fx.scale);

    // Flat tonal fill only: lighter on hover/press, accent tint while waiting for a key
    let idle = material::lerp_color(theme.surface2, theme.text, 0.10 * fx.hover + 0.06 * fx.press);
    let bg = material::lerp_color(idle, material::lerp_color(theme.surface2, theme.mauve, 0.28), on);
    material::paint_fill(ui.painter(), r, CornerRadius::same(material::RADIUS_SM), bg, &fx);

    let text_color = material::lerp_color(material::lerp_color(theme.subtext0, theme.text, fx.hover.max(fx.press)), theme.mauve, on);
    let text_pos = Pos2::new(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, text_color);

    let mut remove_clicked = false;
    if can_remove && !capturing {
        let remove_rect = Rect::from_center_size(Pos2::new(rect.right() - 12.0, rect.center().y), Vec2::splat(material::KEYCAP_HEIGHT - 4.0));
        let remove = ui.interact(remove_rect, id.with("rm"), egui::Sense::click()).on_hover_text("Remove shortcut");
        let rem_fx = material::fx(ui, id.with("rm"), &remove, true, &material::FxSpec::ICON);
        let rem_r = material::scaled(remove_rect, rem_fx.scale);
        let rem_bg = theme.with_alpha(theme.red, ((rem_fx.hover * 40.0) + (rem_fx.press * 24.0)).round() as u8);
        material::paint_fill(ui.painter(), rem_r, material::pill(rem_r), rem_bg, &rem_fx);
        material::paint_icon(ui.painter(), remove_rect.center(), icons::CLOSE, 12.0, material::lerp_color(theme.subtext1, theme.red, rem_fx.hover.max(rem_fx.press)));
        remove_clicked = remove.clicked();
    }

    (resp, remove_clicked)
}

fn swatch(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    color: egui::Color32,
    theme: &SerpantinumTheme,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = material::fx(ui, id, &resp, true, &material::FxSpec::ICON);
    let sel_t = material::animate(ui.ctx(), id.with("sel"), selected);
    let r = egui::Rect::from_center_size(rect.center(), rect.size() * fx.scale);
    ui.painter().circle_filled(r.center(), r.width() / 2.0, color);
    if fx.flash > 0.0 {
        ui.painter().circle_filled(r.center(), r.width() / 2.0, egui::Color32::from_white_alpha((fx.flash * 120.0) as u8));
    }
    if sel_t > 0.01 {
        let fg = material::on_color(theme, color).gamma_multiply(sel_t);
        material::paint_icon(ui.painter(), rect.center(), icons::CHECK, material::ICON_MD * material::lerp(0.6, 1.0, sel_t), fg);
    }
    resp
}

fn theme_card(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
    label: &str,
    preview: &SerpantinumTheme,
    theme: &SerpantinumTheme,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 82.0), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = material::fx(ui, id, &resp, true, &material::FxSpec::CLICK);
    let sel = material::animate(ui.ctx(), id.with("sel"), selected);
    let r = material::scaled(rect, fx.scale);

    let idle = material::lerp_color(theme.surface1, theme.surface2, fx.hover.max(fx.press * 0.8));
    let selected_fill = material::lerp_color(theme.surface1, theme.mauve, 0.35 + fx.hover * 0.06);
    let back = material::lerp_color(idle, selected_fill, sel);
    material::paint_fill(ui.painter(), r, CornerRadius::same(material::RADIUS_LG), back, &fx);
    let swatch_rect = egui::Rect::from_min_size(r.min + egui::vec2(8.0, 8.0), egui::vec2(r.width() - 16.0, 46.0));
    let painter = ui.painter();
    painter.rect_filled(swatch_rect, CornerRadius::same(material::RADIUS_MD), preview.base);
    let bar = egui::Rect::from_min_size(swatch_rect.min + egui::vec2(6.0, 6.0), egui::vec2(swatch_rect.width() * 0.5, 34.0));
    painter.rect_filled(bar, CornerRadius::same(material::RADIUS_SM), preview.surface1);
    painter.rect_filled(
        egui::Rect::from_min_size(bar.min + egui::vec2(6.0, 6.0), egui::vec2(bar.width() - 12.0, 6.0)),
        CornerRadius::same(3),
        preview.text,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(bar.min + egui::vec2(6.0, 16.0), egui::vec2((bar.width() - 12.0) * 0.6, 6.0)),
        CornerRadius::same(3),
        preview.subtext1,
    );
    painter.circle_filled(egui::pos2(swatch_rect.right() - 16.0, swatch_rect.center().y), 9.0, preview.mauve);
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 13.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(material::FONT_CAPTION),
        material::lerp_color(theme.subtext0, theme.text, sel.max(fx.hover)),
    );
    resp
}

fn category_row(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    value: &str,
    theme: &SerpantinumTheme,
    open: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = material::fx(ui, id, &resp, true, &material::FxSpec::ROW);
    let sel = material::animate(ui.ctx(), id.with("open"), open);
    let fill = material::lerp_color(
        material::lerp_color(theme.surface1, theme.surface2, fx.hover.max(sel)),
        material::lerp_color(theme.surface2, theme.text, 0.08),
        fx.press,
    );
    material::paint_fill(ui.painter(), rect, material::pill(rect), fill, &fx);
    material::paint_icon(
        ui.painter(),
        egui::pos2(rect.left() + material::INSET_ICON, rect.center().y),
        icon,
        material::ICON_MD,
        material::lerp_color(theme.subtext0, theme.mauve, sel),
    );
    ui.painter().text(
        egui::pos2(rect.left() + material::INSET_LABEL, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(material::FONT_BODY),
        theme.text,
    );
    let chevron = if open { icons::ARROW_UP } else { icons::CHEVRON_RIGHT };
    material::paint_icon(
        ui.painter(),
        egui::pos2(rect.right() - 22.0, rect.center().y),
        chevron,
        material::ICON_SM,
        material::lerp_color(theme.subtext1, theme.text, fx.hover.max(sel)),
    );
    ui.painter().text(
        egui::pos2(rect.right() - 42.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        value,
        egui::FontId::proportional(material::FONT_CAPTION),
        theme.subtext1,
    );
    resp
}

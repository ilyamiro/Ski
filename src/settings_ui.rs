use std::path::PathBuf;

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Vec2};

use crate::app::{app_row, Dialog, SkiApp, ViewMode};
use crate::fs::apps::{self, AppEntry};
use crate::fs::scanner::{SortBy, SortOrder};
use crate::keymap::{self, Action};
use crate::settings::{Density, GridSize, Settings, StartLocation};
use crate::ui::fonts::SystemFont;
use crate::ui::icons;
use crate::ui::material::{self, chip, filled_icon_button, nav_item, switch_row, Column};
use crate::ui::theme::{self, SerpantinumTheme, ACCENTS, PRESETS};

impl SkiApp {
    pub(crate) fn open_settings_dialog(&mut self) {
        let defaults = apps::CATEGORIES.iter().map(|c| apps::default_for(c.mimes[0])).collect();
        self.system_fonts.get_or_insert_with(crate::ui::fonts::system_fonts);
        self.capturing_key = false;
        self.dialog = Some(Dialog::Settings {
            tab: 0,
            font_query: String::new(),
            hex: self.settings.accent.clone().unwrap_or_default(),
            picking: None,
            defaults,
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

    /// Returns true when the dialog should close.
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

        let max_height = (ctx.content_rect().height() - 160.0).clamp(320.0, 560.0);

        ui.horizontal(|ui| {
            // Left pane: Categories navigation
            ui.vertical(|ui| {
                ui.set_width(165.0);
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 3.0);

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

            // Divider between panes
            let (sep_rect, _) = ui.allocate_exact_size(egui::vec2(12.0, max_height), egui::Sense::hover());
            ui.painter().vline(
                sep_rect.center().x,
                sep_rect.min.y + 4.0..=sep_rect.max.y - 4.0,
                egui::Stroke::new(1.0, theme.surface2),
            );

            // Right pane: Settings content
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
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
        ui.label(egui::RichText::new("Choose where Ski opens on launch").size(12.5).color(theme.subtext1));
        ui.horizontal_wrapped(|ui| {
            for (loc, label) in [
                (StartLocation::Launch, "Launch folder"),
                (StartLocation::Home, "Home"),
                (StartLocation::LastVisited, "Last visited"),
                (StartLocation::Custom, "Custom path"),
            ] {
                if chip(ui, ("start_loc", label), None, label, &theme, self.settings.start_location == loc).clicked() {
                    self.settings.start_location = loc;
                    self.persist_settings();
                }
            }
        });

        if self.settings.start_location == StartLocation::Custom {
            ui.horizontal(|ui| {
                let mut path_str = self
                    .settings
                    .start_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                material::input_frame(&theme).show(ui, |ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut path_str)
                            .frame(egui::Frame::NONE)
                            .desired_width(ui.available_width() - 20.0)
                            .hint_text("/path/to/custom/folder")
                            .font(egui::FontId::proportional(13.0)),
                    );
                    if resp.changed() {
                        self.settings.start_path = if path_str.trim().is_empty() {
                            None
                        } else {
                            Some(PathBuf::from(path_str.trim()))
                        };
                        self.persist_settings();
                    }
                });
            });
        }

        material::section_label(ui, "Windows & Tabs", &theme);
        if switch_row(ui, "open_in_new_win", icons::NEW_WINDOW, "Open folders in a new window", &theme, self.settings.open_in_new_window).clicked() {
            self.settings.open_in_new_window = !self.settings.open_in_new_window;
            self.persist_settings();
        }

        material::section_label(ui, "File Operations", &theme);
        if switch_row(ui, "confirm_overwrite", icons::INFO, "Confirm before overwriting files", &theme, self.settings.confirm_overwrite).clicked() {
            self.settings.confirm_overwrite = !self.settings.confirm_overwrite;
            self.persist_settings();
        }
    }

    fn appearance_tab(&mut self, ui: &mut egui::Ui, font_query: &mut String, hex: &mut String) {
        let theme = self.theme;

        material::section_label(ui, "Theme", &theme);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
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
                if theme_card(ui, ("theme_card", id), label, &preview, &theme, self.settings.theme == id).clicked() {
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
                ui.set_width(130.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(hex)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .hint_text("#rrggbb")
                        .font(egui::FontId::proportional(13.5)),
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
            ui.label(egui::RichText::new("Custom hex color").size(12.0).color(theme.subtext1));
        });

        material::section_label(ui, "Interface size", &theme);
        ui.horizontal_wrapped(|ui| {
            for (scale, label) in [(0.85, "Compact"), (1.0, "Default"), (1.15, "Large"), (1.3, "Larger")] {
                if chip(ui, ("scale", label), None, label, &theme, (self.settings.scale - scale).abs() < 0.01).clicked() {
                    self.settings.scale = scale;
                    ui.ctx().set_zoom_factor(scale);
                    self.persist_settings();
                }
            }
        });

        material::section_label(ui, "Animations", &theme);
        if switch_row(ui, "app_animations", icons::REFRESH, "Enable animations", &theme, self.settings.animations).clicked() {
            self.settings.animations = !self.settings.animations;
            self.persist_settings();
        }

        material::section_label(ui, "Font", &theme);
        let current = self.settings.font_name.clone().unwrap_or_else(|| "Adwaita Sans (default)".to_owned());
        ui.label(egui::RichText::new(format!("Current: {current}")).size(12.5).color(theme.subtext0));
        material::input_frame(&theme).show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(font_query)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .hint_text("Search installed fonts")
                    .font(egui::FontId::proportional(13.5)),
            );
        });
        let needle = font_query.trim().to_lowercase();
        let fonts: Vec<SystemFont> = self
            .system_fonts
            .get_or_insert_with(crate::ui::fonts::system_fonts)
            .iter()
            .filter(|f| needle.is_empty() || f.family.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        let show_default = needle.is_empty() || "adwaita sans".contains(&needle);
        let total = fonts.len() + usize::from(show_default);
        let mut picked: Option<Option<SystemFont>> = None;
        egui::ScrollArea::vertical()
            .max_height(190.0)
            .auto_shrink([false, true])
            .id_salt("font_list")
            .show_rows(ui, 38.0, total, |ui, range| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for row in range {
                    if show_default && row == 0 {
                        if nav_item(ui, "font_default", icons::DOCUMENT, "Adwaita Sans (default)", &theme, self.settings.font.is_none()).clicked() {
                            picked = Some(None);
                        }
                        continue;
                    }
                    let font = &fonts[row - usize::from(show_default)];
                    let on = self.settings.font.as_deref() == Some(font.path.as_path());
                    if nav_item(ui, ("font_row", row), icons::DOCUMENT, &font.family, &theme, on).clicked() {
                        picked = Some(Some(font.clone()));
                    }
                }
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

        material::section_label(ui, "Default View", &theme);
        ui.horizontal_wrapped(|ui| {
            if chip(ui, "fv_view_list", Some(icons::VIEW_LIST), "List", &theme, self.view_mode == ViewMode::List).clicked() {
                self.view_mode = ViewMode::List;
                self.settings.view_mode = ViewMode::List;
                self.persist_settings();
            }
            if chip(ui, "fv_view_grid", Some(icons::GRID_VIEW), "Grid", &theme, self.view_mode == ViewMode::Grid).clicked() {
                self.view_mode = ViewMode::Grid;
                self.settings.view_mode = ViewMode::Grid;
                self.persist_settings();
            }
        });

        material::section_label(ui, "Sorting", &theme);
        ui.horizontal_wrapped(|ui| {
            for (field, label) in [(SortBy::Name, "Name"), (SortBy::Size, "Size"), (SortBy::Modified, "Modified")] {
                if chip(ui, ("fv_sort_by", label), None, label, &theme, self.sort_by == field).clicked() {
                    self.sort_by = field;
                    self.settings.sort_by = field;
                    self.reload_items();
                    self.persist_settings();
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            let asc = self.sort_order == SortOrder::Ascending;
            if chip(ui, "fv_sort_asc", Some(icons::SORT_ASC), "Ascending", &theme, asc).clicked() {
                self.sort_order = SortOrder::Ascending;
                self.settings.sort_order = SortOrder::Ascending;
                self.reload_items();
                self.persist_settings();
            }
            if chip(ui, "fv_sort_desc", Some(icons::SORT_DESC), "Descending", &theme, !asc).clicked() {
                self.sort_order = SortOrder::Descending;
                self.settings.sort_order = SortOrder::Descending;
                self.reload_items();
                self.persist_settings();
            }
        });

        material::section_label(ui, "Visibility & Navigation", &theme);
        if switch_row(ui, "fv_show_hidden", icons::VISIBILITY, "Show hidden files", &theme, self.show_hidden).clicked() {
            self.show_hidden = !self.show_hidden;
            self.settings.show_hidden = self.show_hidden;
            self.reload_items();
            self.persist_settings();
        }
        if switch_row(ui, "fv_folders_first", icons::FOLDER, "Show folders before files", &theme, self.folders_first).clicked() {
            self.folders_first = !self.folders_first;
            self.settings.folders_first = self.folders_first;
            self.reload_items();
            self.persist_settings();
        }
        if switch_row(ui, "fv_show_sidebar", icons::MENU, "Show navigation sidebar", &theme, self.settings.show_sidebar).clicked() {
            self.settings.show_sidebar = !self.settings.show_sidebar;
            self.persist_settings();
        }
        if switch_row(ui, "fv_remember_view", icons::FOLDER_MOVE, "Remember view settings per folder", &theme, self.settings.remember_view_per_folder).clicked() {
            self.settings.remember_view_per_folder = !self.settings.remember_view_per_folder;
            self.persist_settings();
        }

        material::section_label(ui, "List View Options", &theme);
        ui.label(egui::RichText::new("Row density").size(12.5).color(theme.subtext1));
        ui.horizontal_wrapped(|ui| {
            for (density, label) in [
                (Density::Compact, "Compact"),
                (Density::Comfortable, "Comfortable"),
                (Density::Spacious, "Spacious"),
            ] {
                if chip(ui, ("fv_density", label), None, label, &theme, self.settings.density == density).clicked() {
                    self.settings.density = density;
                    self.persist_settings();
                }
            }
        });

        ui.label(egui::RichText::new("Columns to display").size(12.5).color(theme.subtext1));
        ui.horizontal_wrapped(|ui| {
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

        material::section_label(ui, "Grid View Options", &theme);
        ui.label(egui::RichText::new("Icon size").size(12.5).color(theme.subtext1));
        ui.horizontal_wrapped(|ui| {
            for (gs, label) in [
                (GridSize::Small, "Small"),
                (GridSize::Medium, "Medium"),
                (GridSize::Large, "Large"),
            ] {
                if chip(ui, ("fv_grid_size", label), None, label, &theme, self.settings.grid_size == gs).clicked() {
                    self.settings.grid_size = gs;
                    self.persist_settings();
                }
            }
        });

        ui.label(egui::RichText::new("Name wrap lines").size(12.5).color(theme.subtext1));
        ui.horizontal_wrapped(|ui| {
            for (lines, label) in [(1u8, "1 line"), (2u8, "2 lines"), (3u8, "3 lines")] {
                if chip(ui, ("fv_grid_lines", label), None, label, &theme, self.settings.grid_name_lines == lines).clicked() {
                    self.settings.grid_name_lines = lines;
                    self.persist_settings();
                }
            }
        });

        material::section_label(ui, "Display Details", &theme);
        if switch_row(ui, "fv_show_thumbs", icons::IMAGE, "Show file thumbnails", &theme, self.settings.show_thumbnails).clicked() {
            self.settings.show_thumbnails = !self.settings.show_thumbnails;
            self.persist_settings();
        }
        if switch_row(ui, "fv_show_exts", icons::DOCUMENT, "Show file extensions", &theme, self.settings.show_extensions).clicked() {
            self.settings.show_extensions = !self.settings.show_extensions;
            self.persist_settings();
        }
        if switch_row(ui, "fv_relative_dates", icons::INFO, "Use relative dates (e.g. '2 hours ago')", &theme, self.settings.relative_dates).clicked() {
            self.settings.relative_dates = !self.settings.relative_dates;
            self.persist_settings();
        }

        ui.label(egui::RichText::new("File size units").size(12.5).color(theme.subtext1));
        ui.horizontal_wrapped(|ui| {
            if chip(ui, "fv_unit_bin", None, "Binary (1024 B = 1 KiB)", &theme, !self.settings.decimal_units).clicked() {
                self.settings.decimal_units = false;
                self.persist_settings();
            }
            if chip(ui, "fv_unit_dec", None, "Decimal (1000 B = 1 KB)", &theme, self.settings.decimal_units).clicked() {
                self.settings.decimal_units = true;
                self.persist_settings();
            }
        });
    }

    fn behavior_tab(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;

        material::section_label(ui, "Click Behavior", &theme);
        if switch_row(ui, "beh_single_click", icons::OPEN, "Single-click to open files and folders", &theme, self.settings.single_click_open).clicked() {
            self.settings.single_click_open = !self.settings.single_click_open;
            self.persist_settings();
        }

        material::section_label(ui, "Keyboard Navigation", &theme);
        if switch_row(ui, "beh_vim_keys", icons::KEYBOARD, "Enable vim navigation keys (h/j/k/l, g/G, y)", &theme, self.settings.vim_keys).clicked() {
            self.settings.vim_keys = !self.settings.vim_keys;
            self.persist_settings();
        }

        material::section_label(ui, "Quick Filter", &theme);
        if switch_row(ui, "beh_type_to_filter", icons::SEARCH, "Type to filter current folder without Ctrl+F", &theme, self.settings.type_to_filter).clicked() {
            self.settings.type_to_filter = !self.settings.type_to_filter;
            self.persist_settings();
        }
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
                    ui.label(egui::RichText::new(icons::SEARCH).font(icons::font(15.0)).color(theme.subtext1));
                    ui.add(
                        egui::TextEdit::singleline(keybind_search)
                            .frame(egui::Frame::NONE)
                            .desired_width(220.0)
                            .hint_text("Search shortcuts...")
                            .font(egui::FontId::proportional(13.0)),
                    );
                });
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if chip(ui, "reset_all_keybinds", Some(icons::REFRESH), "Reset all to defaults", &theme, false).clicked() {
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
                .fill(theme.with_alpha(theme.yellow, 35))
                .stroke(egui::Stroke::new(1.0, theme.yellow))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(icons::INFO).font(icons::font(16.0)).color(theme.yellow));
                        ui.label(
                            egui::RichText::new(format!(
                                "Shortcut \"{}\" is already used by \"{}\". Replace?",
                                binding,
                                conf_act.label()
                            ))
                            .size(13.0)
                            .color(theme.text),
                        );
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if chip(ui, "conf_replace", Some(icons::CHECK), "Replace", &theme, true).clicked() {
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
                        if chip(ui, "conf_cancel", Some(icons::CLOSE), "Cancel", &theme, false).clicked() {
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

            for action in actions_in_group {
                let is_overridden = self.settings.keybinds.contains_key(action.id());

                ui.horizontal(|ui| {
                    ui.set_min_height(34.0);

                    // Action label
                    let label_color = if is_overridden { theme.mauve } else { theme.text };
                    ui.label(egui::RichText::new(action.label()).size(13.0).color(label_color));

                    // Per-action reset button if overridden
                    if is_overridden {
                        if filled_icon_button(ui, ("rst_act", action.id()), icons::REFRESH, &theme, 22.0, "Reset to default", None).clicked() {
                            self.settings.keybinds.remove(action.id());
                            self.persist_settings();
                            if matches!(*capture, Some((a, _)) if a == action) {
                                *capture = None;
                                self.capturing_key = false;
                            }
                        }
                    }

                    // Keycap pills aligned to right
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                            let pill_text = if is_capturing_this { "Press keys…" } else { bind_str.as_str() };
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
        }
    }

    fn applications_tab(&mut self, ui: &mut egui::Ui, picking: &mut Option<usize>, defaults: &mut [Option<String>]) {
        let theme = self.theme;
        material::section_label(ui, "Default Applications", &theme);
        ui.label(
            egui::RichText::new("Default application for each kind of file. This changes the system-wide defaults.")
                .size(12.5)
                .color(theme.subtext1),
        );
        ui.spacing_mut().item_spacing.y = 2.0;
        let installed: Vec<AppEntry> = self.installed_apps().to_vec();
        for (idx, category) in apps::CATEGORIES.iter().enumerate() {
            let current = defaults[idx].as_deref().and_then(|id| installed.iter().find(|a| a.id == id));
            let value = current.map_or("Not set", |a| a.name.as_str());
            if category_row(ui, ("cat_row", idx), category.icon, category.label, value, &theme, *picking == Some(idx)).clicked() {
                *picking = if *picking == Some(idx) { None } else { Some(idx) };
            }
            if *picking != Some(idx) {
                continue;
            }
            let candidates = apps::apps_for(&installed, category.mimes[0], defaults[idx].as_deref());
            let mut chosen: Option<&AppEntry> = None;
            for (row, app) in candidates.iter().enumerate() {
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
                if app_row(ui, ("cat_app", idx, row), tex.as_ref(), &app.name, &theme, on).clicked() {
                    chosen = Some(app);
                }
            }
            if candidates.is_empty() {
                ui.label(egui::RichText::new("No installed application handles these files.").size(12.5).color(theme.subtext0));
            }
            if let Some(app) = chosen {
                apps::set_category_default(category, &app.id);
                defaults[idx] = Some(app.id.clone());
                self.set_toast(format!("{} is now the default for {}", app.name, category.label.to_lowercase()));
                *picking = None;
            }
            ui.add_space(4.0);
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
    let font_id = egui::FontId::monospace(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font_id, theme.text);
    let remove_btn_w = if can_remove && !capturing { 18.0 } else { 0.0 };
    let total_w = galley.size().x + 18.0 + remove_btn_w;
    let height = 26.0;

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(total_w, height), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = material::fx(ui, id, &resp, true, &material::FxSpec::ROW);

    let bg = if capturing {
        theme.with_alpha(theme.mauve, 50)
    } else {
        material::lerp_color(theme.surface2, theme.surface1, fx.hover)
    };

    let stroke = if capturing {
        egui::Stroke::new(1.5, theme.mauve)
    } else if fx.hover > 0.0 {
        egui::Stroke::new(1.0, theme.with_alpha(theme.text, 40))
    } else {
        egui::Stroke::new(1.0, theme.surface1)
    };

    ui.painter().rect(rect, CornerRadius::same(6), bg, stroke, egui::StrokeKind::Inside);

    let text_color = if capturing {
        theme.mauve
    } else {
        material::lerp_color(theme.subtext0, theme.text, fx.hover)
    };

    let text_pos = Pos2::new(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, text_color);

    let mut remove_clicked = false;
    if can_remove && !capturing {
        let remove_rect = Rect::from_center_size(
            Pos2::new(rect.right() - 11.0, rect.center().y),
            Vec2::new(16.0, 16.0),
        );
        let rem_hover = ui.rect_contains_pointer(remove_rect);
        let rem_color = if rem_hover { theme.red } else { theme.subtext1 };
        material::paint_icon(ui.painter(), remove_rect.center(), icons::CLOSE, 12.0, rem_color);
        if rem_hover && ui.input(|i| i.pointer.primary_clicked()) {
            remove_clicked = true;
        }
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
    let r = egui::Rect::from_center_size(rect.center(), rect.size() * fx.scale);
    ui.painter().circle_filled(r.center(), r.width() / 2.0, color);
    if fx.flash > 0.0 {
        ui.painter().circle_filled(r.center(), r.width() / 2.0, egui::Color32::from_white_alpha((fx.flash * 120.0) as u8));
    }
    if selected {
        let on_color = if (color.r() as u32 + color.g() as u32 + color.b() as u32) > 380 {
            theme.crust
        } else {
            egui::Color32::WHITE
        };
        material::paint_icon(ui.painter(), r.center(), icons::CHECK, 17.0, on_color);
    }
    resp
}

fn theme_card(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    preview: &SerpantinumTheme,
    theme: &SerpantinumTheme,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(104.0, 82.0), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = material::fx(ui, id, &resp, true, &material::FxSpec::CLICK);
    let sel = ui.ctx().animate_bool_with_time_and_easing(id.with("sel"), selected, 0.25, material::quint_out);
    let r = egui::Rect::from_center_size(rect.center(), rect.size() * fx.scale);

    let back = material::lerp_color(theme.surface1, theme.with_alpha(theme.mauve, 255), 0.0);
    let back = material::lerp_color(back, material::lerp_color(theme.surface1, theme.mauve, 0.35), sel);
    ui.painter().rect_filled(r, egui::CornerRadius::same(18), back);
    let swatch_rect = egui::Rect::from_min_size(r.min + egui::vec2(8.0, 8.0), egui::vec2(r.width() - 16.0, 46.0));
    let painter = ui.painter();
    painter.rect_filled(swatch_rect, egui::CornerRadius::same(12), preview.base);
    let bar = egui::Rect::from_min_size(swatch_rect.min + egui::vec2(6.0, 6.0), egui::vec2(swatch_rect.width() * 0.5, 34.0));
    painter.rect_filled(bar, egui::CornerRadius::same(8), preview.surface1);
    painter.rect_filled(
        egui::Rect::from_min_size(bar.min + egui::vec2(6.0, 6.0), egui::vec2(bar.width() - 12.0, 6.0)),
        egui::CornerRadius::same(3),
        preview.text,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(bar.min + egui::vec2(6.0, 16.0), egui::vec2((bar.width() - 12.0) * 0.6, 6.0)),
        egui::CornerRadius::same(3),
        preview.subtext1,
    );
    painter.circle_filled(egui::pos2(swatch_rect.right() - 16.0, swatch_rect.center().y), 9.0, preview.mauve);
    painter.text(
        egui::pos2(r.center().x, r.bottom() - 13.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(12.5),
        material::lerp_color(theme.subtext0, theme.text, sel),
    );
    if fx.flash > 0.0 {
        painter.rect_filled(r, egui::CornerRadius::same(18), egui::Color32::from_white_alpha((fx.flash * 60.0) as u8));
    }
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
    let sel = ui.ctx().animate_bool_with_time_and_easing(id.with("open"), open, 0.25, material::quint_out);
    let fill = material::lerp_color(theme.surface1, theme.surface2, fx.hover.max(sel));
    ui.painter().rect_filled(rect, egui::CornerRadius::same(18), fill);
    material::paint_icon(
        ui.painter(),
        egui::pos2(rect.left() + 24.0, rect.center().y),
        icon,
        18.0,
        material::lerp_color(theme.subtext0, theme.mauve, sel),
    );
    ui.painter().text(
        egui::pos2(rect.left() + 46.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.5),
        theme.text,
    );
    let chevron = if open { icons::ARROW_UP } else { icons::CHEVRON_RIGHT };
    material::paint_icon(ui.painter(), egui::pos2(rect.right() - 20.0, rect.center().y), chevron, 16.0, theme.subtext1);
    ui.painter().text(
        egui::pos2(rect.right() - 40.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        value,
        egui::FontId::proportional(12.5),
        theme.subtext1,
    );
    resp
}

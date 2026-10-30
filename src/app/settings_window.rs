//! Settings in a window of its own, built from the same frame as the main window: the categories
//! down the side like the places, the chosen one's settings in the middle, and the theme editor
//! as a card over them.
use super::*;

impl SkiApp {
    pub(super) fn render_settings_window(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.settings_win.take() else {
            if self.theme_editor {
                self.close_theme_editor();
            }
            return;
        };
        let theme = self.theme;
        let popup_theme = self.popup_theme;
        let builder = chrome::viewport(tr("Settings")).with_inner_size([1000.0, 720.0]).with_min_inner_size([640.0, 440.0]);
        let mut close = false;
        let raise = std::mem::take(&mut self.settings_raise);
        let fresh = self.settings_fresh;
        self.settings_fresh = false;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("ski_settings"), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            material::set_dialog_fresh(&ctx, fresh);
            if raise {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            // Escape peels the layers off one at a time: a color picker, the theme editor, then the window
            if !self.capturing_key && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                if !self.theme_editor {
                    close = true;
                } else if self.theme_picker.take().is_none() {
                    self.close_theme_editor();
                }
            }
            Self::window_resize_edges(&ctx);

            // The fields are single-line, so Up and Down leave them for the next row
            // instead of just jumping the caret to either end of the text
            if !self.capturing_key && ctx.text_edit_focused() {
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

            chrome::sidebar(ui, "set_categories", chrome::SIDEBAR_WIDTH, &theme, |ui| {
                // The title shares the header's row, its left edge on the rows' icon column
                let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), CONTROL_HEIGHT), egui::Sense::hover());
                chrome::drag_area(ui, row, ui.scope_id().with("set_side_drag"));
                material::paint_label(ui, row.left() + material::INSET_ICON - 15.0, row.center().y, tr("Settings"), material::FONT_TITLE, theme.text, row.width());
                self.settings_sidebar(ui, &mut state);
            });

            chrome::header(ui, "set_header", &theme, |ui| {
                let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), CONTROL_HEIGHT), egui::Sense::hover());
                // No title bar of its own: the empty parts of the header drag the window
                chrome::drag_area(ui, row.expand2(egui::vec2(HEADER_MARGIN as f32, 4.0)), ui.scope_id().with("set_drag"));
                let left = row.left() - HEADER_MARGIN as f32 + 16.0;
                let right = row.right() + 8.0 - 16.0;
                material::paint_label(ui, left, row.center().y, Self::settings_category_name(&state), material::FONT_TITLE, theme.text, row.width() / 2.0);
                let close_rect = egui::Rect::from_center_size(egui::pos2(right - material::CLOSE_BUTTON / 2.0, row.center().y), egui::vec2(material::CLOSE_BUTTON, material::CLOSE_BUTTON));
                let closed = ui.scope_builder(egui::UiBuilder::new().max_rect(close_rect), |ui| {
                    material::filled_icon_button(ui, "set_close", icons::CLOSE, &theme, material::CLOSE_BUTTON, tr("Close"), None).clicked()
                });
                close |= closed.inner;
            });

            chrome::content(ui, &theme, |ui| self.settings_page(ui, &mut state));

            // Menus and cards stay solid however see-through the window is
            self.theme = popup_theme;
            self.render_theme_editor(&ctx);
            self.theme = theme;
        });

        if close {
            self.capturing_key = false;
            self.close_theme_editor();
        } else {
            self.settings_win = Some(state);
        }
    }
}

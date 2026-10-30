//! A card for choosing a folder's icon from the glyphs of the bundled icon font, opened over
//! Properties or the main window like the theme editor is over Settings.
use super::*;
use crate::ui::icon_style;

const CELL: f32 = 48.0;
const CARD_WIDTH: f32 = 480.0;
const CARD_HEIGHT: f32 = 520.0;
const CARD_MARGIN: f32 = 20.0;

/// The folder being given a glyph, and the glyph picked so far.
pub(crate) struct GlyphPicker {
    pub folder: PathBuf,
    pub chosen: Option<char>,
}

impl SkiApp {
    pub(super) fn open_glyph_picker(&mut self, folder: PathBuf) {
        let chosen = crate::fs::places::folder_glyph(&folder);
        self.glyph_picker = Some(GlyphPicker { folder, chosen });
    }

    /// Set (or with `None` remove) the glyph of a folder; an image set before it goes.
    fn apply_folder_glyph(&mut self, folder: &Path, glyph: Option<char>) {
        if glyph.is_some() && self.folder_icons.remove(folder).is_some() {
            save_folder_icons(&self.folder_icons);
        }
        crate::fs::places::set_folder_glyph(folder, glyph);
        self.set_toast(if glyph.is_some() { tr("Folder icon set") } else { tr("Folder icon reset") });
    }

    /// Escape closes the card before anything under it sees the key.
    pub(super) fn glyph_picker_escape(&mut self, ctx: &egui::Context) {
        if self.glyph_picker.is_some() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.glyph_picker = None;
        }
    }

    pub(super) fn render_glyph_picker(&mut self, ctx: &egui::Context) {
        let Some(mut picker) = self.glyph_picker.take() else { return };
        let theme = self.theme;
        let screen = ctx.content_rect();
        let name = picker.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let has_glyph = crate::fs::places::folder_glyph(&picker.folder).is_some();
        let glyphs = icon_style::all_glyphs();
        let (mut confirmed, mut cancelled, mut reset) = (false, false, false);

        // Dims what is behind the card; a click on it closes the card
        let scrim = egui::Area::new(egui::Id::from("glyph_picker_scrim"))
            .order(egui::Order::Tooltip)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(rect, 0, egui::Color32::from_black_alpha(120));
                resp
            });
        cancelled |= scrim.inner.clicked();

        let width = (screen.width() - 32.0).clamp(240.0, CARD_WIDTH);
        let height = (screen.height() - 48.0).clamp(300.0, CARD_HEIGHT);
        let card = egui::Area::new(egui::Id::from("glyph_picker_card"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme.mantle)
                    .corner_radius(egui::CornerRadius::same(material::radius_window()))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(CARD_MARGIN as i8))
                    .show(ui, |ui| {
                        let inner = egui::vec2(width - 2.0 * CARD_MARGIN, height - 2.0 * CARD_MARGIN);
                        ui.set_width(inner.x);
                        ui.set_min_height(inner.y);
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);

                        // Title and close button
                        let (row, _) = ui.allocate_exact_size(egui::vec2(inner.x, 36.0), egui::Sense::hover());
                        material::paint_label(ui, row.left(), row.center().y, &trf("Glyph for “{}”", &[&name]), material::FONT_TITLE, theme.text, row.width() - 48.0);
                        let close_rect = egui::Rect::from_center_size(egui::pos2(row.right() - material::CLOSE_BUTTON / 2.0, row.center().y), egui::vec2(material::CLOSE_BUTTON, material::CLOSE_BUTTON));
                        cancelled |= ui.scope_builder(egui::UiBuilder::new().max_rect(close_rect), |ui| {
                            material::filled_icon_button(ui, "gp_close", icons::CLOSE, &theme, material::CLOSE_BUTTON, tr("Close"), None).clicked()
                        }).inner;

                        // The grid fills what the buttons below leave
                        let grid_h = inner.y - 36.0 - material::BUTTON_HEIGHT - 3.0 * 8.0;
                        let per_row = ((inner.x / CELL).floor() as usize).max(1);
                        let rows = glyphs.len().div_ceil(per_row);
                        let pad = (inner.x - per_row as f32 * CELL) / 2.0;
                        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                        egui::ScrollArea::vertical().max_height(grid_h).min_scrolled_height(grid_h).auto_shrink([false, false]).id_salt("gp_scroll").show_rows(ui, CELL, rows, |ui, range| {
                            for row in range {
                                ui.horizontal(|ui| {
                                    ui.add_space(pad);
                                    for &c in glyphs.iter().skip(row * per_row).take(per_row) {
                                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(CELL, CELL), egui::Sense::click());
                                        let selected = picker.chosen == Some(c);
                                        if selected || resp.hovered() {
                                            let fill = if selected { theme.surface2 } else { theme.surface0 };
                                            ui.painter().rect_filled(rect.shrink(3.0), egui::CornerRadius::same(material::radius_md()), fill);
                                        }
                                        let color = if selected { theme.mauve } else { theme.text };
                                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, c, icons::font(22.0), color);
                                        if resp.double_clicked() {
                                            picker.chosen = Some(c);
                                            confirmed = true;
                                        } else if resp.clicked() {
                                            picker.chosen = Some(c);
                                        }
                                    }
                                });
                            }
                        });

                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if let Some(c) = picker.chosen {
                                ui.label(egui::RichText::new(c.to_string()).font(icons::font(24.0)).color(theme.mauve));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let ready = picker.chosen.is_some();
                                confirmed |= material::button(ui, "gp_go", Some(icons::CHECK), tr("Set icon"), &theme, ButtonKind::Filled, ready).clicked() && ready;
                                if has_glyph {
                                    reset |= material::button(ui, "gp_reset", None, tr("Reset"), &theme, ButtonKind::Text, true).clicked();
                                }
                            });
                        });
                    });
            });
        // Tab and the arrow keys stay in the card
        ctx.memory_mut(|m| m.set_modal_layer(card.response.layer_id));
        ctx.move_to_top(card.response.layer_id);

        if reset {
            self.apply_folder_glyph(&picker.folder, None);
        } else if confirmed && let Some(c) = picker.chosen {
            self.apply_folder_glyph(&picker.folder, Some(c));
        } else if !cancelled {
            self.glyph_picker = Some(picker);
        }
    }
}

//! A window of its own for choosing a folder's icon from the glyphs of the bundled icon font,
//! opened over Properties or from the folder's menu.
use super::*;
use crate::ui::icon_style;

const CELL: f32 = 52.0;

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

    pub(super) fn render_glyph_picker(&mut self, ctx: &egui::Context) {
        let Some(mut picker) = self.glyph_picker.take() else { return };
        let theme = self.theme;
        let name = picker.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let builder = chrome::viewport(format!("{} — “{name}”", tr("Folder icon"))).with_inner_size([640.0, 560.0]).with_min_inner_size([380.0, 360.0]);
        let has_glyph = crate::fs::places::folder_glyph(&picker.folder).is_some();
        let glyphs = icon_style::all_glyphs();
        let (mut confirmed, mut cancelled, mut reset) = (false, false, false);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("ski_glyph_picker"), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            if ctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                cancelled = true;
            }
            Self::window_resize_edges(&ctx);

            chrome::header(ui, "gp_header", &theme, |ui| {
                chrome::drag_area(ui, ui.max_rect().expand2(egui::vec2(HEADER_MARGIN as f32, 4.0)), ui.scope_id().with("gp_drag"));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(trf("Glyph for “{}”", &[&name])).size(material::FONT_TITLE).color(theme.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        cancelled |= material::filled_icon_button(ui, "gp_close", icons::CLOSE, &theme, material::CLOSE_BUTTON, tr("Close"), None).clicked();
                    });
                });
            });

            chrome::footer(ui, "gp_footer", &theme, |ui| {
                ui.horizontal(|ui| {
                    if let Some(c) = picker.chosen {
                        ui.label(egui::RichText::new(c.to_string()).font(icons::font(22.0)).color(theme.mauve));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ready = picker.chosen.is_some();
                        confirmed |= material::button(ui, "gp_go", Some(icons::CHECK), tr("Set icon"), &theme, ButtonKind::Filled, ready).clicked() && ready;
                        cancelled |= material::button(ui, "gp_cancel", None, tr("Cancel"), &theme, ButtonKind::Text, true).clicked();
                        if has_glyph {
                            reset |= material::button(ui, "gp_reset", None, tr("Reset"), &theme, ButtonKind::Text, true).clicked();
                        }
                    });
                });
            });

            chrome::content(ui, &theme, |ui| {
                let per_row = ((ui.available_width() / CELL).floor() as usize).max(1);
                let rows = glyphs.len().div_ceil(per_row);
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("gp_scroll").show_rows(ui, CELL, rows, |ui, range| {
                    for row in range {
                        ui.horizontal(|ui| {
                            for &c in glyphs.iter().skip(row * per_row).take(per_row) {
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(CELL, CELL), egui::Sense::click());
                                let selected = picker.chosen == Some(c);
                                let cell = rect.shrink(3.0);
                                if selected || resp.hovered() {
                                    let fill = if selected { theme.surface2 } else { theme.surface0 };
                                    ui.painter().rect_filled(cell, egui::CornerRadius::same(material::radius_md()), fill);
                                }
                                let color = if selected { theme.mauve } else { theme.text };
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, c, icons::font(24.0), color);
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
            });
        });

        if reset {
            self.apply_folder_glyph(&picker.folder, None);
        } else if confirmed && let Some(c) = picker.chosen {
            self.apply_folder_glyph(&picker.folder, Some(c));
        } else if !cancelled {
            self.glyph_picker = Some(picker);
        }
    }
}

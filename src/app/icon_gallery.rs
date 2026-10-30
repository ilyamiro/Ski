//! A window of its own showing every icon in an icon style: the styles down the side, and the
//! folders and a file of each kind drawn in the chosen one.
use super::*;
use crate::fs::file_item::ICON_SAMPLES;
use crate::ui::icon_style::{self, IconStyle};

/// The sizes a folder is shown at, from the list's smallest to the grid's largest.
const FOLDER_SIZES: [f32; 6] = [16.0, 24.0, 32.0, 48.0, 64.0, 96.0];

impl SkiApp {
    pub(super) fn render_icon_gallery(&mut self, ctx: &egui::Context) {
        let Some(mut style) = self.icon_gallery else { return };
        let theme = self.theme;
        let builder = chrome::viewport(tr("Icon style")).with_inner_size([900.0, 640.0]).with_min_inner_size([560.0, 400.0]);
        let mut close = false;
        let mut chosen = None;
        let in_use = self.settings.icon_style;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("ski_icon_gallery"), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            if ctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            Self::window_resize_edges(&ctx);

            chrome::sidebar(ui, "ig_styles", chrome::SIDEBAR_WIDTH, &theme, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(CONTROL_HEIGHT + 8.0);
                for s in IconStyle::ALL {
                    // Each style shows its own folder where a row's icon goes
                    let resp = nav_item(ui, ("ig_style", s as u8), "", s.label(), &theme, s == style);
                    let at = egui::pos2(resp.rect.left() + material::INSET_ICON, resp.rect.center().y);
                    icon_style::paint_folder(s, ui.painter(), at, 18.0, theme.blue);
                    if resp.clicked() {
                        style = s;
                    }
                }
            });

            chrome::header(ui, "ig_header", &theme, |ui| {
                // No title bar of its own: the empty parts of the header drag the window
                chrome::drag_area(ui, ui.max_rect().expand2(egui::vec2(HEADER_MARGIN as f32, 4.0)), ui.scope_id().with("ig_drag"));
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), CONTROL_HEIGHT), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = TITLE_GAP;
                    ui.label(egui::RichText::new(trf("{} icons", &[&style.label()])).size(material::FONT_TITLE).color(theme.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if material::filled_icon_button(ui, "ig_close", icons::CLOSE, &theme, material::CLOSE_BUTTON, tr("Close"), None).clicked() {
                            close = true;
                        }
                        ui.add_space(CLOSE_GAP - TITLE_GAP);
                        let label = if in_use == style { tr("In use") } else { tr("Use this style") };
                        if material::button(ui, "ig_use", Some(icons::CHECK), label, &theme, ButtonKind::Tonal, in_use != style).clicked() {
                            chosen = Some(style);
                        }
                    });
                });
            });

            chrome::content(ui, &theme, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("ig_scroll").show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
                    material::section_label(ui, tr("Folders"), &theme);
                    let width = FOLDER_SIZES.iter().map(|s| s * 1.2 + 16.0).sum::<f32>().min(ui.available_width());
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 96.0 * 1.15 + 16.0), egui::Sense::hover());
                    let painter = ui.painter().with_clip_rect(rect);
                    painter.rect_filled(rect, egui::CornerRadius::same(material::radius_md()), theme.surface0);
                    let mut x = rect.left() + 8.0;
                    for size in FOLDER_SIZES {
                        icon_style::paint_folder(style, &painter, egui::pos2(x + size * 0.6, rect.center().y), size, theme.blue);
                        x += size * 1.2 + 16.0;
                    }

                    material::section_label(ui, tr("Files"), &theme);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                        for (name, kind) in ICON_SAMPLES {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(92.0, 92.0), egui::Sense::hover());
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            ui.painter().rect_filled(rect, egui::CornerRadius::same(material::radius_md()), theme.surface0);
                            let item = FileItem::sample(name, *kind);
                            let stage = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), rect.height() - 22.0));
                            let icon = material::item_icon_animated(&item, &theme, 0.0);
                            material::paint_item_icon_in(style, &ui.painter().with_clip_rect(stage), &item, stage.center(), 44.0, icon, 1.0);
                            // The plain icon of a type is named after the type
                            let caption = if name.is_empty() { kind.label() } else { name };
                            let galley = material::truncated(ui, caption, material::FONT_CAPTION, theme.subtext0, rect.width() - 8.0);
                            let pos = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - 13.0 - galley.size().y / 2.0);
                            ui.painter().galley(pos, galley, theme.subtext0);
                        }
                    });
                });
            });
        });

        if let Some(style) = chosen {
            self.settings.icon_style = style;
            self.persist_settings();
        }
        self.icon_gallery = if close { None } else { Some(style) };
    }
}

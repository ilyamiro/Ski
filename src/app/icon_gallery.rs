//! A window of its own showing every icon in an icon style: the styles down the side, and the
//! folders and a file of each kind drawn in the chosen one.
use super::*;
use super::settings_window::AppPtr;
use crate::fs::file_item::{FileType, ICON_SAMPLES};
use crate::ui::icon_style::{self, IconStyle};

/// The sizes a folder is shown at, from the list's smallest up; the largest still leaves room
/// around it in a column the width of a file tile.
const FOLDER_SIZES: [f32; 6] = [16.0, 24.0, 32.0, 48.0, 64.0, 80.0];
/// Space between the window's edges and the header's title and close button, and the content
/// under them, so both start and end on the same lines.
const EDGE: f32 = 16.0;
/// Space between tiles, and between a section's label and what it labels.
const GAP: f32 = 8.0;
const TILE_MIN: f32 = 96.0;
const TILE_HEIGHT: f32 = 96.0;
/// Height of the caption strip along the bottom of a tile.
const CAPTION: f32 = 26.0;

impl SkiApp {
    /// Declares the icon gallery each frame of the Settings window, which it is opened from; its
    /// contents are drawn by its own redraws, so it keeps up when the other windows are out of sight.
    pub(super) fn render_icon_gallery(&mut self, ctx: &egui::Context) {
        if self.icon_gallery.is_none() {
            return;
        }
        let builder = chrome::viewport(tr("Icon style")).with_inner_size([920.0, 660.0]).with_min_inner_size([600.0, 420.0]);
        let app = AppPtr::new(self);
        ctx.show_viewport_deferred(egui::ViewportId::from_hash_of("ski_icon_gallery"), builder, move |ui, _class| {
            // SAFETY: see `AppPtr`; the app outlives every window
            let app = unsafe { &mut *app.get() };
            app.icon_gallery_ui(ui);
        });
    }

    fn icon_gallery_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(mut style) = self.icon_gallery else {
            chrome::close_own_viewport(&ctx);
            return;
        };
        let theme = self.theme;
        let mut close = false;
        let mut chosen = None;
        let in_use = self.settings.icon_style;
        if ctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        Self::window_resize_edges(&ctx);

        chrome::sidebar(ui, "ig_styles", chrome::SIDEBAR_WIDTH, &theme, |ui| {
            // The title shares the header's row, its left edge on the rows' icon column
            let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), CONTROL_HEIGHT), egui::Sense::hover());
            chrome::drag_area(ui, row, ui.scope_id().with("ig_side_drag"));
            material::paint_label(ui, row.left() + material::INSET_ICON - 15.0, row.center().y, tr("Icon style"), material::FONT_TITLE, theme.text, row.width());
            ui.add_space(8.0 - ui.spacing().item_spacing.y);
            ui.spacing_mut().item_spacing.y = 2.0;
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
            let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), CONTROL_HEIGHT), egui::Sense::hover());
            // No title bar of its own: the empty parts of the header drag the window
            chrome::drag_area(ui, row.expand2(egui::vec2(HEADER_MARGIN as f32, 4.0)), ui.scope_id().with("ig_drag"));
            // The header's own margins are uneven (made for the main window's buttons); these
            // put the title and the close button the same distance from the window's edges
            let left = row.left() - HEADER_MARGIN as f32 + EDGE;
            let right = row.right() + 8.0 - EDGE;
            material::paint_label(ui, left, row.center().y, &trf("{} icons", &[&style.label()]), material::FONT_TITLE, theme.text, row.width() / 2.0);
            let close_rect = egui::Rect::from_center_size(egui::pos2(right - material::CLOSE_BUTTON / 2.0, row.center().y), egui::vec2(material::CLOSE_BUTTON, material::CLOSE_BUTTON));
            let closed = ui.scope_builder(egui::UiBuilder::new().max_rect(close_rect), |ui| {
                material::filled_icon_button(ui, "ig_close", icons::CLOSE, &theme, material::CLOSE_BUTTON, tr("Close"), None).clicked()
            });
            close |= closed.inner;
            // The choice is a chip, the height of the row's other controls: lit while the style is in use
            let label = if in_use == style { tr("In use") } else { tr("Use this style") };
            let width = material::chip_natural_width(ui, Some(icons::CHECK), label, &theme);
            let chip_rect = egui::Rect::from_center_size(
                egui::pos2(close_rect.left() - CLOSE_GAP - width / 2.0, row.center().y),
                egui::vec2(width, material::CHIP_HEIGHT),
            );
            let clicked = ui.scope_builder(egui::UiBuilder::new().max_rect(chip_rect), |ui| {
                material::chip_width(ui, ("ig_use", style as u8), Some(icons::CHECK), label, &theme, in_use == style, Some(width)).clicked()
            });
            if clicked.inner && in_use != style {
                chosen = Some(style);
            }
        });

        chrome::content(ui, &theme, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("ig_scroll").show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                // The content panel keeps PAGE_MARGIN from the sides; the sections keep EDGE
                let inset = EDGE - PAGE_MARGIN as f32;
                let full = ui.available_width();
                let width = full - 2.0 * inset;
                let column = |ui: &mut egui::Ui, h: f32| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(full, h), egui::Sense::hover());
                    egui::Rect::from_min_max(egui::pos2(rect.left() + inset, rect.top()), egui::pos2(rect.right() - inset, rect.bottom()))
                };

                let label = column(ui, 28.0);
                section(ui, &theme, label, tr("Folders"));
                let strip = column(ui, strip_height());
                folders(ui, &theme, strip, style);

                ui.add_space(GAP);
                let label = column(ui, 28.0);
                section(ui, &theme, label, tr("Files"));
                // As many tiles as fit a row, stretched so every row ends on the same line
                let per_row = ((width + GAP) / (TILE_MIN + GAP)).floor().max(1.0) as usize;
                let tile_w = (width - GAP * (per_row - 1) as f32) / per_row as f32;
                for (r, row) in ICON_SAMPLES.chunks(per_row).enumerate() {
                    if r > 0 {
                        ui.add_space(GAP);
                    }
                    let line = column(ui, TILE_HEIGHT);
                    if !ui.is_rect_visible(line) {
                        continue;
                    }
                    for (i, (name, kind)) in row.iter().enumerate() {
                        let tile = egui::Rect::from_min_size(egui::pos2(line.left() + i as f32 * (tile_w + GAP), line.top()), egui::vec2(tile_w, TILE_HEIGHT));
                        file_tile(ui, &theme, tile, style, name, *kind);
                    }
                }
                ui.add_space(EDGE);
            });
        });

        if let Some(style) = chosen {
            self.settings.icon_style = style;
            self.persist_settings();
            // The other windows show the style too, and get their turn when they can
            ctx.request_repaint_of(egui::ViewportId::ROOT);
            ctx.request_repaint_of(egui::ViewportId::from_hash_of("ski_settings"));
        }
        if close {
            self.icon_gallery = None;
            chrome::close_own_viewport(&ctx);
        } else {
            self.icon_gallery = Some(style);
        }
    }
}

/// A section's label, on the left edge of what it labels and `GAP` above it.
fn section(ui: &egui::Ui, theme: &SerpantinumTheme, rect: egui::Rect, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), egui::FontId::proportional(material::FONT_SECTION), theme.subtext1);
    ui.painter().galley(egui::pos2(rect.left(), rect.bottom() - GAP - galley.size().y), galley, theme.subtext1);
}

/// Padding inside the folder strip, the same above the largest folder as at the sides.
const STRIP_PAD: f32 = 16.0;
/// From a folder's bottom edge to the middle of its size label under it, clear of the underside
/// some styles draw below the edge.
const SIZE_LABEL: f32 = 24.0;

/// Height of the folder strip: the largest folder (0.84 × its size tall) and the labels under it.
fn strip_height() -> f32 {
    STRIP_PAD + FOLDER_SIZES[FOLDER_SIZES.len() - 1] * 0.84 + SIZE_LABEL + STRIP_PAD
}

/// The folder at every size it is shown at, all standing on one line with their size under them,
/// in columns laid out like the file tiles' (six across with `GAP` between), so where the tiles
/// run six to a row each folder sits over a column of them.
fn folders(ui: &egui::Ui, theme: &SerpantinumTheme, rect: egui::Rect, style: IconStyle) {
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, egui::CornerRadius::same(material::radius_md()), theme.surface0);
    let n = FOLDER_SIZES.len() as f32;
    let largest = FOLDER_SIZES[FOLDER_SIZES.len() - 1];
    let baseline = rect.top() + STRIP_PAD + largest * 0.84;
    let column = (rect.width() - GAP * (n - 1.0)) / n;
    // Too narrow a window for even columns: they follow the sizes instead, so none overlap
    let even = column >= largest * 1.05;
    let total: f32 = FOLDER_SIZES.iter().sum();
    let mut x = rect.left();
    for size in FOLDER_SIZES {
        let share = if even { column } else { (rect.width() - GAP * (n - 1.0)) * size / total };
        let cx = x + share / 2.0;
        // A folder's box is 0.84 × its size tall, centred on where it is drawn
        icon_style::paint_folder(style, &painter, egui::pos2(cx, baseline - size * 0.42), size, theme.blue);
        painter.text(egui::pos2(cx, baseline + SIZE_LABEL), egui::Align2::CENTER_CENTER, format!("{size}"), egui::FontId::proportional(material::FONT_CAPTION), theme.subtext0);
        x += share + GAP;
    }
}

/// One file's icon over its name; the plain icon of a type is named after the type.
fn file_tile(ui: &egui::Ui, theme: &SerpantinumTheme, tile: egui::Rect, style: IconStyle, name: &str, kind: FileType) {
    ui.painter().rect_filled(tile, egui::CornerRadius::same(material::radius_md()), theme.surface0);
    let item = FileItem::sample(name, kind);
    let stage = egui::Rect::from_min_max(tile.min, egui::pos2(tile.right(), tile.bottom() - CAPTION));
    let icon = material::item_icon_animated(&item, theme, 0.0);
    material::paint_item_icon_in(style, &ui.painter().with_clip_rect(stage), &item, egui::pos2(stage.center().x, stage.center().y + 4.0), 44.0, icon, 1.0);
    let caption = if name.is_empty() { kind.label() } else { name };
    let galley = material::truncated(ui, caption, material::FONT_CAPTION, theme.subtext0, tile.width() - 12.0);
    let pos = egui::pos2(tile.center().x - galley.size().x / 2.0, tile.bottom() - CAPTION / 2.0 - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, theme.subtext0);
}

//! Flat Material 3 style widgets. Nothing here draws a stroke: shape and state are
//! communicated purely through fill colors (tonal surfaces and state layers).
use eframe::egui::{
    self, text::LayoutJob, text::TextWrapping, Align2, Color32, CornerRadius, Pos2, Rect, Vec2,
};
use super::theme::SerpantinumTheme;
use crate::fs::file_item::FileItem;
use crate::ui::icons;

/// Height shared by every control in the header so the row always lines up.
pub const CONTROL_HEIGHT: f32 = 40.0;

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 * inv + b.r() as f32 * t).round() as u8,
        (a.g() as f32 * inv + b.g() as f32 * t).round() as u8,
        (a.b() as f32 * inv + b.b() as f32 * t).round() as u8,
        (a.a() as f32 * inv + b.a() as f32 * t).round() as u8,
    )
}

pub fn paint_icon(painter: &egui::Painter, center: Pos2, icon: &str, size: f32, color: Color32) {
    painter.text(center, Align2::CENTER_CENTER, icon, icons::font(size), color);
}

/// Single-line text truncated with an ellipsis at `max_width`.
fn truncated(ui: &egui::Ui, text: &str, size: f32, color: Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), egui::FontId::proportional(size), color);
    job.wrap = TextWrapping::truncate_at_width(max_width.max(1.0));
    ui.painter().layout_job(job)
}

fn state_layer(theme: &SerpantinumTheme, hover_t: f32, press_t: f32) -> Color32 {
    let hover = theme.with_alpha(theme.text, 22);
    let press = theme.with_alpha(theme.text, 40);
    lerp_color(lerp_color(Color32::TRANSPARENT, hover, hover_t), press, press_t)
}

/// Round icon-only button.
pub fn icon_button(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    theme: &SerpantinumTheme,
    enabled: bool,
    tooltip: &str,
) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(CONTROL_HEIGHT), sense);
    let id = ui.make_persistent_id(id_source);

    let hover_t = ui.ctx().animate_bool_responsive(id.with("hov"), enabled && response.hovered());
    let press_t = ui.ctx().animate_bool_responsive(id.with("prs"), enabled && response.is_pointer_button_down_on());

    ui.painter().rect_filled(rect, CornerRadius::same(20), state_layer(theme, hover_t, press_t));

    let color = if enabled {
        lerp_color(theme.subtext0, theme.text, hover_t)
    } else {
        theme.with_alpha(theme.subtext1, 90)
    };
    paint_icon(ui.painter(), rect.center(), icon, 21.0, color);

    if tooltip.is_empty() { response } else { response.on_hover_text(tooltip) }
}

/// Pill shaped choice / toggle chip. Selected state is a tonal fill, never an outline.
pub fn chip(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: Option<&str>,
    label: &str,
    theme: &SerpantinumTheme,
    selected: bool,
) -> egui::Response {
    let height = 34.0;
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(13.5), theme.text);
    let icon_w = if icon.is_some() { 24.0 } else { 0.0 };
    let width = galley.size().x + 28.0 + icon_w;

    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let hover_t = ui.ctx().animate_bool_responsive(id.with("hov"), response.hovered());
    let sel_t = ui.ctx().animate_bool_responsive(id.with("sel"), selected);

    let idle = lerp_color(theme.surface1, theme.surface2, hover_t);
    let active = lerp_color(theme.with_alpha(theme.mauve, 60), theme.with_alpha(theme.mauve, 85), hover_t);
    let bg = lerp_color(idle, blend_over(theme.surface1, active), sel_t);
    ui.painter().rect_filled(rect, CornerRadius::same(17), bg);

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, hover_t), theme.mauve, sel_t);
    let mut x = rect.left() + 14.0;
    if let Some(ic) = icon {
        paint_icon(ui.painter(), Pos2::new(x + 8.0, rect.center().y), ic, 17.0, color);
        x += 24.0;
    }
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.5),
        color,
    );
    response
}

fn blend_over(under: Color32, over: Color32) -> Color32 {
    lerp_color(under, Color32::from_rgb(over.r(), over.g(), over.b()), over.a() as f32 / 255.0)
}

/// Sidebar destination (Material navigation drawer item).
pub fn nav_item(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    theme: &SerpantinumTheme,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let hover_t = ui.ctx().animate_bool_responsive(id.with("hov"), response.hovered());
    let sel_t = ui.ctx().animate_bool_responsive(id.with("sel"), selected);

    let bg = lerp_color(
        lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, 16), hover_t),
        theme.with_alpha(theme.mauve, 52),
        sel_t,
    );
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(21), bg);
    }

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, hover_t), theme.mauve, sel_t);
    paint_icon(ui.painter(), Pos2::new(rect.left() + 26.0, rect.center().y), icon, 20.0, color);

    let galley = truncated(ui, label, 14.0, color, rect.width() - 64.0);
    let pos = Pos2::new(rect.left() + 48.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
    response
}

fn item_icon(item: &FileItem) -> &'static str {
    if item.is_symlink { icons::SYMLINK } else { item.file_type.icon() }
}

fn item_icon_color(item: &FileItem, theme: &SerpantinumTheme, selected: bool) -> Color32 {
    if selected {
        theme.mauve
    } else if item.is_dir {
        theme.blue
    } else {
        theme.subtext0
    }
}

/// One row of the list view. Columns drop out as the row gets narrower.
pub fn file_row(
    ui: &mut egui::Ui,
    idx: usize,
    item: &FileItem,
    theme: &SerpantinumTheme,
    selected: bool,
    should_scroll: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), egui::Sense::click());
    let id = ui.make_persistent_id(("row", idx));
    let hover_t = ui.ctx().animate_bool_responsive(id.with("hov"), response.hovered());
    let sel_t = ui.ctx().animate_bool_responsive(id.with("sel"), selected);

    let bg = lerp_color(
        lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, 14), hover_t),
        theme.with_alpha(theme.mauve, 48),
        sel_t,
    );
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(14), bg);
    }

    paint_icon(
        ui.painter(),
        Pos2::new(rect.left() + 28.0, rect.center().y),
        item_icon(item),
        22.0,
        item_icon_color(item, theme, selected),
    );

    let show_date = rect.width() >= 560.0;
    let show_size = rect.width() >= 400.0;

    let mut right = rect.right() - 18.0;
    if show_date {
        ui.painter().text(right_pos(right, rect), Align2::RIGHT_CENTER, item.formatted_date(), egui::FontId::proportional(12.5), theme.subtext1);
        right -= 120.0;
    }
    if show_size {
        ui.painter().text(right_pos(right, rect), Align2::RIGHT_CENTER, item.formatted_size(), egui::FontId::proportional(12.5), theme.subtext1);
        right -= 84.0;
    }

    let name_color = lerp_color(lerp_color(theme.subtext0, theme.text, hover_t), theme.text, sel_t);
    let galley = truncated(ui, &item.name, 14.0, name_color, right - rect.left() - 64.0);
    let pos = Pos2::new(rect.left() + 56.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, name_color);

    if selected && should_scroll {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    response
}

fn right_pos(x: f32, rect: Rect) -> Pos2 {
    Pos2::new(x, rect.center().y)
}

pub const GRID_CELL: Vec2 = Vec2::new(116.0, 108.0);

/// One cell of the grid view.
pub fn file_tile(
    ui: &mut egui::Ui,
    idx: usize,
    item: &FileItem,
    theme: &SerpantinumTheme,
    selected: bool,
    should_scroll: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(GRID_CELL, egui::Sense::click());
    let id = ui.make_persistent_id(("tile", idx));
    let hover_t = ui.ctx().animate_bool_responsive(id.with("hov"), response.hovered());
    let sel_t = ui.ctx().animate_bool_responsive(id.with("sel"), selected);

    let bg = lerp_color(
        lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, 14), hover_t),
        theme.with_alpha(theme.mauve, 48),
        sel_t,
    );
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(16), bg);
    }

    paint_icon(
        ui.painter(),
        Pos2::new(rect.center().x, rect.top() + 40.0),
        item_icon(item),
        38.0,
        item_icon_color(item, theme, selected),
    );

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, hover_t), theme.text, sel_t);
    let galley = truncated(ui, &item.name, 12.5, color, rect.width() - 16.0);
    let pos = Pos2::new(rect.center().x - galley.size().x / 2.0, rect.bottom() - 28.0);
    ui.painter().galley(pos, galley, color);

    if selected && should_scroll {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    response
}

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Stroke, Vec2};
use super::theme::SerpantinumTheme;
use crate::fs::file_item::FileItem;
use crate::ui::icons;

/// Linearly interpolate between two colors
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

/// Exact replica of Serpantinum's `IconButton.qml`
///
/// Properties:
/// - size: 42dp
/// - cornerRadius: 12dp
/// - Scale animation: 1.0 (rest) -> 1.04 (hover) -> 1.08 (pressed) with OutQuint curve (250ms)
/// - Background: surface0 -> lighter(surface0, 1.12) -> darker(surface0, 1.12)
/// - Glass rim stroke: 1px subtle surface1 highlight
pub fn serpantinum_icon_button(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    theme: &SerpantinumTheme,
    enabled: bool,
    tooltip: &str,
) -> egui::Response {
    let size = Vec2::splat(42.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    let id = ui.make_persistent_id(id_source);

    if !enabled {
        let text_color = theme.with_alpha(theme.subtext1, 90);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(19.0),
            text_color,
        );
        return response;
    }

    // Scale animation: OutQuint easing (250ms)
    let target_scale = if response.is_pointer_button_down_on() {
        1.08
    } else if response.hovered() {
        1.04
    } else {
        1.0
    };
    let current_scale = ui.ctx().animate_value_with_time(id.with("scale"), target_scale, 0.25);

    // Color animation (180ms)
    let hover_t = ui.ctx().animate_bool_responsive(id.with("hov"), response.hovered());
    let press_t = ui.ctx().animate_bool_responsive(id.with("prs"), response.is_pointer_button_down_on());

    let base_color = theme.surface0;
    let hover_color = theme.lighter(theme.surface0, 1.14);
    let press_color = theme.darker(theme.surface0, 1.14);

    let bg_color = if response.is_pointer_button_down_on() {
        press_color
    } else {
        lerp_color(base_color, hover_color, hover_t)
    };

    let text_color = lerp_color(theme.subtext0, theme.text, hover_t.max(press_t));

    // Scaled rectangle centered on original rect
    let center = rect.center();
    let scaled_size = rect.size() * current_scale;
    let animated_rect = Rect::from_center_size(center, scaled_size);

    // Glass rim border
    let rim_color = lerp_color(
        theme.with_alpha(theme.surface1, 120),
        theme.with_alpha(theme.mauve, 160),
        hover_t,
    );

    ui.painter().rect_filled(animated_rect, CornerRadius::same(12), bg_color);
    ui.painter().rect_stroke(
        animated_rect,
        CornerRadius::same(12),
        Stroke::new(1.0, rim_color),
        egui::StrokeKind::Inside,
    );

    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(19.0),
        text_color,
    );

    if !tooltip.is_empty() {
        response.on_hover_text(tooltip)
    } else {
        response
    }
}

/// Exact replica of Serpantinum's `ClickButton.qml` (used for Breadcrumb Chips & Buttons)
///
/// Properties:
/// - height: 34dp
/// - cornerRadius: 10dp
/// - Scale animation: 1.0 (rest) -> 1.015 (hover) -> 0.985 (press)
/// - Background: surface0 (or tinted mauve when active)
pub fn serpantinum_chip_button(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: Option<&str>,
    text: &str,
    theme: &SerpantinumTheme,
    is_active: bool,
    tooltip: &str,
) -> egui::Response {
    let font_id = egui::FontId::proportional(13.0);
    let text_galley = ui.painter().layout_no_wrap(text.to_owned(), font_id, theme.text);

    let mut width = text_galley.size().x + 24.0;
    if icon.is_some() {
        width += 22.0;
    }
    let height = 34.0;
    let size = Vec2::new(width, height);

    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let id = ui.make_persistent_id(id_source);

    let target_scale = if response.is_pointer_button_down_on() {
        0.985
    } else if response.hovered() {
        1.015
    } else {
        1.0
    };
    let current_scale = ui.ctx().animate_value_with_time(id.with("scale"), target_scale, 0.25);

    let hover_t = ui.ctx().animate_bool_responsive(id.with("hov"), response.hovered());

    // Active mauve container vs standard surface0
    let base_bg = if is_active {
        theme.with_alpha(theme.mauve, 45)
    } else {
        theme.surface0
    };

    let bg_color = if is_active {
        lerp_color(base_bg, theme.with_alpha(theme.mauve, 70), hover_t)
    } else {
        lerp_color(base_bg, theme.lighter(theme.surface0, 1.12), hover_t)
    };

    let text_color = if is_active {
        theme.mauve
    } else {
        lerp_color(theme.subtext0, theme.text, hover_t)
    };

    let center = rect.center();
    let animated_rect = Rect::from_center_size(center, rect.size() * current_scale);

    let rim_color = if is_active {
        theme.with_alpha(theme.mauve, 160)
    } else {
        theme.with_alpha(theme.surface1, 100)
    };

    ui.painter().rect_filled(animated_rect, CornerRadius::same(10), bg_color);
    ui.painter().rect_stroke(
        animated_rect,
        CornerRadius::same(10),
        Stroke::new(1.0, rim_color),
        egui::StrokeKind::Inside,
    );

    let mut cursor_x = animated_rect.left() + 12.0;
    if let Some(ic) = icon {
        let icon_pos = Pos2::new(cursor_x + 8.0, center.y);
        ui.painter().text(
            icon_pos,
            egui::Align2::CENTER_CENTER,
            ic,
            egui::FontId::proportional(17.0),
            text_color,
        );
        cursor_x += 22.0;
    }

    let text_pos = Pos2::new(cursor_x, center.y - text_galley.size().y / 2.0);
    ui.painter().galley(text_pos, text_galley, text_color);

    if !tooltip.is_empty() {
        response.on_hover_text(tooltip)
    } else {
        response
    }
}

/// Serpantinum Sidebar Item (exact replica of `GuideSidebar.qml` tab delegate)
///
/// Properties:
/// - height: 40dp
/// - cornerRadius: 12dp
/// - 32x32 rounded icon container in surface1 / surface2
/// - Active selection in mauve tint
pub fn serpantinum_nav_item(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    theme: &SerpantinumTheme,
    is_selected: bool,
) -> egui::Response {
    let width = ui.available_width();
    let height = 40.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());

    let id = ui.make_persistent_id(id_source);
    let hover_t = ui.ctx().animate_bool_responsive(id.with("hov"), response.hovered());
    let select_t = ui.ctx().animate_bool_responsive(id.with("sel"), is_selected);

    let hover_color = theme.with_alpha(theme.surface0, 180);

    let bg_color = lerp_color(
        lerp_color(Color32::TRANSPARENT, hover_color, hover_t),
        theme.with_alpha(theme.mauve, 50),
        select_t,
    );

    if bg_color != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(12), bg_color);
    }

    // 32x32 Icon Box (matching Serpantinum's delegateIconArea)
    let icon_box_rect = Rect::from_min_size(
        Pos2::new(rect.left() + 6.0, rect.center().y - 14.0),
        Vec2::splat(28.0),
    );

    let box_bg = if is_selected {
        theme.with_alpha(theme.mauve, 80)
    } else {
        theme.surface1
    };

    ui.painter().rect_filled(icon_box_rect, CornerRadius::same(8), box_bg);

    let icon_color = if is_selected {
        theme.text
    } else {
        lerp_color(theme.subtext1, theme.subtext0, hover_t)
    };

    ui.painter().text(
        icon_box_rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(16.0),
        icon_color,
    );

    // Label text
    let label_color = if is_selected {
        theme.text
    } else {
        lerp_color(theme.subtext0, theme.text, hover_t)
    };

    let label_pos = Pos2::new(rect.left() + 44.0, rect.center().y);
    ui.painter().text(
        label_pos,
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.5),
        label_color,
    );

    response
}

/// Serpantinum File List Row (exact replica of `Launcher.qml` list item delegate)
///
/// Properties:
/// - height: 44dp
/// - radius: theme.borderRadius (16dp)
/// - Left margin shift on selection: +2px animation
/// - Press scale: 0.98
/// - Selected background: morphHighlight in theme.mauve tint (0.22 alpha)
/// - 32x32 rounded icon container with boxRadius: 8 in surface2
pub fn serpantinum_file_row(
    ui: &mut egui::Ui,
    idx: usize,
    item: &FileItem,
    theme: &SerpantinumTheme,
    is_selected: bool,
    should_scroll: bool,
) -> (egui::Response, bool, bool, bool) {
    let row_height = 44.0;
    let available_width = ui.available_width();

    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(available_width, row_height),
        egui::Sense::click(),
    );

    let id = ui.make_persistent_id(format!("row_{idx}"));
    let hover_t = ui.ctx().animate_bool_responsive(id.with("hov"), response.hovered());
    let select_t = ui.ctx().animate_bool_responsive(id.with("sel"), is_selected);

    // Scale animation: scale: ma.pressed ? 0.98 : 1.0 (duration: 180ms)
    let target_scale = if response.is_pointer_button_down_on() { 0.98 } else { 1.0 };
    let current_scale = ui.ctx().animate_value_with_time(id.with("scale"), target_scale, 0.18);

    let hover_bg = theme.with_alpha(theme.surface0, 140);
    let bg_color = lerp_color(
        lerp_color(Color32::TRANSPARENT, hover_bg, hover_t),
        theme.with_alpha(theme.mauve, 50),
        select_t,
    );

    let animated_rect = Rect::from_center_size(rect.center(), rect.size() * current_scale);

    if bg_color != Color32::TRANSPARENT {
        ui.painter().rect_filled(
            animated_rect,
            CornerRadius::same(theme.border_radius as u8),
            bg_color,
        );
    }

    // Left selection accent bar (similar to Serpantinum focus pop)
    if is_selected {
        let bar_rect = Rect::from_min_size(
            Pos2::new(animated_rect.left() + 3.0, animated_rect.center().y - 12.0),
            Vec2::new(3.5, 24.0),
        );
        ui.painter().rect_filled(bar_rect, CornerRadius::same(2), theme.mauve);
    }

    // 32x32 Icon Area with boxRadius 8
    let icon_box_rect = Rect::from_min_size(
        Pos2::new(animated_rect.left() + 10.0, animated_rect.center().y - 16.0),
        Vec2::splat(32.0),
    );

    let icon_box_color = if is_selected {
        theme.with_alpha(theme.mauve, 85)
    } else {
        theme.surface2
    };

    ui.painter().rect_filled(icon_box_rect, CornerRadius::same(8), icon_box_color);

    let icon_str = if item.is_symlink {
        icons::SYMLINK
    } else {
        item.file_type.icon()
    };

    let icon_color = if is_selected {
        theme.text
    } else if item.is_dir {
        theme.blue
    } else {
        theme.subtext0
    };

    ui.painter().text(
        icon_box_rect.center(),
        egui::Align2::CENTER_CENTER,
        icon_str,
        egui::FontId::proportional(18.0),
        icon_color,
    );

    // File name text
    let name_color = if is_selected {
        theme.text
    } else {
        lerp_color(theme.subtext0, theme.text, hover_t)
    };

    let name_pos = Pos2::new(animated_rect.left() + 52.0, animated_rect.center().y);
    ui.painter().text(
        name_pos,
        egui::Align2::LEFT_CENTER,
        &item.name,
        if item.is_dir {
            egui::FontId::proportional(14.0)
        } else {
            egui::FontId::proportional(13.5)
        },
        name_color,
    );

    // Right metadata: Date & Size
    let date_pos = Pos2::new(animated_rect.right() - 20.0, animated_rect.center().y);
    ui.painter().text(
        date_pos,
        egui::Align2::RIGHT_CENTER,
        item.formatted_date(),
        egui::FontId::proportional(12.0),
        theme.subtext1,
    );

    let size_pos = Pos2::new(animated_rect.right() - 140.0, animated_rect.center().y);
    ui.painter().text(
        size_pos,
        egui::Align2::RIGHT_CENTER,
        item.formatted_size(),
        egui::FontId::proportional(12.0),
        theme.subtext0,
    );

    if is_selected && should_scroll {
        response.scroll_to_me(Some(egui::Align::Center));
    }

    let clicked = response.clicked();
    let double_clicked = response.double_clicked();
    let secondary_clicked = response.secondary_clicked();

    (response, clicked, double_clicked, secondary_clicked)
}

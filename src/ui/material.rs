//! Flat Material 3 style widgets. Nothing here draws a stroke: shape and state are
//! communicated purely through fill colors (tonal surfaces and state layers).
use eframe::egui::{
    self, text::LayoutJob, text::TextWrapping, Align2, Color32, CornerRadius, Pos2, Rect, Vec2,
};
use super::theme::SerpantinumTheme;
use crate::fs::file_item::{specific_icon, FileItem, FileType, Tint};
use crate::ui::icons;

/// Height shared by every control in the header so the row always lines up.
pub const CONTROL_HEIGHT: f32 = 34.0;

/// Shared Corner Radii (Material 3 scale)
pub const RADIUS_SM: u8 = 8;         // Small elements, previews, thumbnails, chips
pub const RADIUS_MD: u8 = 12;        // Rows, cards, list items
pub const RADIUS_LG: u8 = 18;        // Tiles, input fields, badges, large controls
pub const RADIUS_CONTAINER: u8 = 22; // Menus, popups, sub-panels, scroll boxes
pub const RADIUS_WINDOW: u8 = 26;    // Dialogs and modal frames

/// Shared Font Sizes
pub const FONT_SECTION: f32 = 12.0;
pub const FONT_CAPTION: f32 = 12.5;
pub const FONT_CHIP: f32 = 13.0;
pub const FONT_BODY: f32 = 13.5;
pub const FONT_SUBTITLE: f32 = 14.0;
pub const FONT_TITLE: f32 = 16.0;

/// Shared State Layer / Tonal Fill Alphas
pub const ALPHA_HOVER: u8 = 16;
pub const ALPHA_PRESS: u8 = 32;
pub const ALPHA_SEL_BG: u8 = 85;
pub const ALPHA_SEL_HOVER: u8 = 110;
pub const ALPHA_SEL_OUTLINE: u8 = 120;
pub const ALPHA_DIVIDER: u8 = 24;

pub fn quint_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(5)
}

fn quad_out(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

/// Per-frame animation values for one interactive widget, mirroring the behaviour of
/// Serpantinum's `IconButton.qml` / `ClickButton.qml`: OutQuint scale towards a hover / press
/// target, a short pop on click, and a white flash that fades out.
pub struct Fx {
    pub hover: f32,
    pub press: f32,
    pub scale: f32,
    pub flash: f32,
}

pub struct FxSpec {
    pub hover_scale: f32,
    pub press_scale: f32,
    pub pop_scale: f32,
}

impl FxSpec {
    pub const ICON: FxSpec = FxSpec { hover_scale: 1.06, press_scale: 0.92, pop_scale: 1.1 };
    pub const CLICK: FxSpec = FxSpec { hover_scale: 1.02, press_scale: 0.97, pop_scale: 1.04 };
    pub const ROW: FxSpec = FxSpec { hover_scale: 1.0, press_scale: 1.0, pop_scale: 1.0 };
}

pub fn fx(ui: &egui::Ui, id: egui::Id, resp: &egui::Response, enabled: bool, spec: &FxSpec) -> Fx {
    let ctx = ui.ctx();
    let hover = ctx.animate_bool_with_time_and_easing(id.with("hov"), enabled && resp.hovered(), 0.25, quint_out);
    let press = ctx.animate_bool_with_time_and_easing(
        id.with("prs"),
        enabled && resp.is_pointer_button_down_on(),
        0.25,
        quint_out,
    );

    let now = ctx.input(|i| i.time);
    let pop_key = id.with("pop");
    if enabled && resp.clicked() {
        ctx.data_mut(|d| d.insert_temp(pop_key, now));
    }
    let elapsed = ctx
        .data(|d| d.get_temp::<f64>(pop_key))
        .map(|t| (now - t) as f32)
        .unwrap_or(f32::MAX);

    // Click pop: up over 110ms (OutQuad), back over 420ms (OutQuint)
    let pop = if elapsed < 0.11 {
        1.0 + (spec.pop_scale - 1.0) * quad_out(elapsed / 0.11)
    } else if elapsed < 0.53 {
        1.0 + (spec.pop_scale - 1.0) * (1.0 - quint_out((elapsed - 0.11) / 0.42))
    } else {
        1.0
    };
    // Flash: 0.4 -> 0 over 400ms (OutExpo)
    let flash = if elapsed < 0.4 {
        0.4 * (1.0 - egui::emath::easing::exponential_out(elapsed / 0.4))
    } else {
        0.0
    };
    if elapsed < 0.55 {
        ctx.request_repaint();
    }

    let scale = (1.0 + (spec.hover_scale - 1.0) * hover + (spec.press_scale - spec.hover_scale) * press) * pop;
    Fx { hover, press, scale, flash }
}

fn scaled(rect: Rect, scale: f32) -> Rect {
    Rect::from_center_size(rect.center(), rect.size() * scale)
}

fn flash_color(flash: f32) -> Color32 {
    Color32::from_white_alpha((flash * 255.0 * 0.5) as u8)
}

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

fn state_layer(theme: &SerpantinumTheme, fx: &Fx) -> Color32 {
    let hover = theme.with_alpha(theme.text, ALPHA_HOVER);
    let press = theme.with_alpha(theme.text, ALPHA_PRESS);
    lerp_color(lerp_color(Color32::TRANSPARENT, hover, fx.hover), press, fx.press)
}

fn paint_fill(painter: &egui::Painter, rect: Rect, radius: u8, base: Color32, fx: &Fx) {
    if base.a() > 0 {
        painter.rect_filled(rect, CornerRadius::same(radius), base);
    }
    if fx.flash > 0.0 {
        painter.rect_filled(rect, CornerRadius::same(radius), flash_color(fx.flash));
    }
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
    icon_button_tinted(ui, id_source, icon, theme, enabled, tooltip, None)
}

/// Same as `icon_button`, with an accent that the hover state fades towards.
pub fn icon_button_tinted(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    theme: &SerpantinumTheme,
    enabled: bool,
    tooltip: &str,
    accent: Option<Color32>,
) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(CONTROL_HEIGHT), sense);
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, enabled, &FxSpec::ICON);
    let r = scaled(rect, fx.scale);

    let layer = match accent {
        Some(a) => lerp_color(
            lerp_color(Color32::TRANSPARENT, theme.with_alpha(a, 50), fx.hover),
            theme.with_alpha(a, 90),
            fx.press,
        ),
        None => state_layer(theme, &fx),
    };
    paint_fill(ui.painter(), r, 17, layer, &fx);

    let hover_color = accent.unwrap_or(theme.text);
    let color = if enabled {
        lerp_color(theme.subtext1, hover_color, fx.hover)
    } else {
        theme.with_alpha(theme.subtext1, 70)
    };
    paint_icon(ui.painter(), r.center(), icon, 18.0, color);

    if enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
    if tooltip.is_empty() { response } else { response.on_hover_text(tooltip) }
}

/// Icon-only counterpart of `chip`: a filled round button (dialog and window close buttons).
pub fn filled_icon_button(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    theme: &SerpantinumTheme,
    size: f32,
    tooltip: &str,
    accent: Option<Color32>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::CLICK);
    let r = scaled(rect, fx.scale);

    let idle = theme.surface1;
    let hovered = match accent {
        Some(a) => blend_over(theme.surface1, theme.with_alpha(a, 70)),
        None => theme.surface2,
    };
    paint_fill(ui.painter(), r, (size / 2.0) as u8, lerp_color(idle, hovered, fx.hover), &fx);

    let color = lerp_color(theme.subtext1, accent.unwrap_or(theme.text), fx.hover);
    paint_icon(ui.painter(), r.center(), icon, size * 0.5, color);
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
    let height = 30.0;
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(FONT_CHIP), theme.text);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let width = galley.size().x + 24.0 + icon_w;

    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::CLICK);
    let sel_t = ui.ctx().animate_bool_with_time_and_easing(id.with("sel"), selected, 0.25, quint_out);
    let r = scaled(rect, fx.scale);

    let idle = lerp_color(theme.surface1, theme.surface2, fx.hover);
    let active = blend_over(theme.surface1, theme.with_alpha(theme.mauve, 70 + (fx.hover * 25.0) as u8));
    paint_fill(ui.painter(), r, 15, lerp_color(idle, active, sel_t), &fx);

    if sel_t > 0.001 {
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(15),
            egui::Stroke::new(1.0, theme.with_alpha(theme.mauve, (90.0 * sel_t) as u8)),
            egui::StrokeKind::Inside,
        );
    }

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, fx.hover), theme.mauve, sel_t);
    let mut x = r.left() + 12.0;
    if let Some(ic) = icon {
        paint_icon(ui.painter(), Pos2::new(x + 7.0, r.center().y), ic, 15.0, color);
        x += 22.0;
    }
    ui.painter().text(Pos2::new(x, r.center().y), Align2::LEFT_CENTER, label, egui::FontId::proportional(FONT_CHIP), color);
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
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    let sel_t = ui.ctx().animate_bool_with_time_and_easing(id.with("sel"), selected, 0.25, quint_out);
    let r = scaled(rect, fx.scale);

    let idle_bg = lerp_color(
        lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, ALPHA_HOVER), fx.hover),
        theme.with_alpha(theme.text, ALPHA_PRESS),
        fx.press,
    );
    let active_bg = theme.with_alpha(theme.mauve, ALPHA_SEL_BG + (fx.hover * 20.0) as u8);
    let bg = lerp_color(idle_bg, active_bg, sel_t);
    paint_fill(ui.painter(), r, RADIUS_LG, bg, &fx);

    if sel_t > 0.001 {
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(RADIUS_LG),
            egui::Stroke::new(1.5, theme.with_alpha(theme.mauve, (ALPHA_SEL_OUTLINE as f32 * sel_t) as u8)),
            egui::StrokeKind::Inside,
        );
        let bar_rect = Rect::from_center_size(
            Pos2::new(r.left() + 4.0, r.center().y),
            Vec2::new(3.0, 18.0 * sel_t),
        );
        ui.painter().rect_filled(
            bar_rect,
            CornerRadius::same(2),
            theme.with_alpha(theme.mauve, (230.0 * sel_t) as u8),
        );
    }

    let active_color = if theme.light {
        theme.mauve
    } else {
        lerp_color(theme.mauve, Color32::WHITE, 0.25)
    };
    let color = lerp_color(lerp_color(theme.subtext0, theme.text, fx.hover), active_color, sel_t);
    paint_icon(ui.painter(), Pos2::new(r.left() + 22.0, r.center().y), icon, 17.0, color);

    let galley = truncated(ui, label, FONT_BODY, color, r.width() - 56.0);
    let pos = Pos2::new(r.left() + 42.0, r.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
    response
}

/// Glyph and color of a file's type icon.
fn item_icon(item: &FileItem, theme: &SerpantinumTheme, selected: bool) -> (&'static str, Color32) {
    if item.is_dir {
        return (icons::FOLDER, if selected { theme.mauve } else { theme.blue });
    }
    if item.is_symlink {
        return (icons::SYMLINK, if selected { theme.mauve } else { theme.subtext0 });
    }
    if let Some((glyph, tint)) = specific_icon(&item.name) {
        let color = match tint {
            Tint::Red => theme.red,
            Tint::Peach => theme.peach,
            Tint::Green => theme.green,
            Tint::Blue => theme.blue,
            Tint::Mauve => theme.mauve,
            Tint::Muted => theme.subtext0,
        };
        return (glyph, color);
    }
    let color = match item.file_type {
        FileType::Image => theme.green,
        FileType::Audio => theme.peach,
        FileType::Video => theme.red,
        FileType::Archive => theme.peach,
        FileType::Executable => theme.green,
        FileType::Code => theme.blue,
        _ => theme.subtext0,
    };
    (item.file_type.icon(), color)
}

/// Paints a preview as a rounded picture that keeps its aspect ratio inside `rect`.
/// Document pages sit on a thin shadow so the white sheet reads as paper on dark themes.
fn paint_thumb(ui: &egui::Ui, tex: &egui::TextureHandle, rect: Rect, radius: u8) {
    let size = tex.size_vec2();
    let fit = (rect.width() / size.x).min(rect.height() / size.y);
    let shown = Rect::from_center_size(rect.center(), size * fit);
    egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), shown.size()))
        .corner_radius(CornerRadius::same(radius))
        .paint_at(ui, shown);
}

#[allow(dead_code)]
pub const ROW_HEIGHT: f32 = 44.0;

/// One row of the list view. Columns drop out as the row gets narrower.
/// `path_hint` replaces size/date (used by search results to show where the file lives).
pub fn file_row(
    ui: &mut egui::Ui,
    idx: usize,
    item: &FileItem,
    theme: &SerpantinumTheme,
    selected: bool,
    should_scroll: bool,
    thumb: Option<&egui::TextureHandle>,
    path_hint: Option<&str>,
    columns: &[Column],
    row_height: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), egui::Sense::click());
    let id = ui.make_persistent_id(("row", idx));
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    let sel_t = ui.ctx().animate_bool_with_time_and_easing(id.with("sel"), selected, 0.25, quint_out);
    let r = scaled(rect, fx.scale);

    let idle_bg = lerp_color(
        lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, ALPHA_HOVER), fx.hover),
        theme.with_alpha(theme.text, ALPHA_PRESS),
        fx.press,
    );
    let active_bg = theme.with_alpha(theme.mauve, ALPHA_SEL_BG + (fx.hover * 25.0) as u8);
    let bg = lerp_color(idle_bg, active_bg, sel_t);
    paint_fill(ui.painter(), r, RADIUS_MD, bg, &fx);

    if sel_t > 0.001 {
        // Subtle ~1.5px accent outline
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(RADIUS_MD),
            egui::Stroke::new(1.5, theme.with_alpha(theme.mauve, (ALPHA_SEL_OUTLINE as f32 * sel_t) as u8)),
            egui::StrokeKind::Inside,
        );
        // Accent left bar in list view
        let bar_height = 22.0 * sel_t;
        let bar_rect = Rect::from_center_size(
            Pos2::new(r.left() + 4.0, r.center().y),
            Vec2::new(3.5, bar_height),
        );
        ui.painter().rect_filled(
            bar_rect,
            CornerRadius::same(2),
            theme.with_alpha(theme.mauve, (230.0 * sel_t) as u8),
        );
    }

    let icon_center = Pos2::new(r.left() + 28.0, r.center().y);
    match thumb {
        Some(tex) => paint_thumb(ui, tex, Rect::from_center_size(icon_center, Vec2::splat(32.0)), RADIUS_SM),
        None => {
            let (glyph, base_color) = item_icon(item, theme, selected);
            let icon_color = lerp_color(base_color, theme.mauve, sel_t * 0.7);
            paint_icon(ui.painter(), icon_center, glyph, 24.0, icon_color);
        }
    }

    let mut right = r.right() - 16.0;
    let small = egui::FontId::proportional(FONT_CAPTION);
    let subtext_color = lerp_color(theme.subtext1, theme.subtext0, sel_t);
    if let Some(hint) = path_hint {
        let galley = truncated(ui, hint, FONT_CAPTION, subtext_color, (r.width() * 0.4).min(260.0));
        let pos = Pos2::new(right - galley.size().x, r.center().y - galley.size().y / 2.0);
        right = pos.x - 12.0;
        ui.painter().galley(pos, galley, subtext_color);
    } else {
        // Columns that don't fit next to a readable name are dropped, rightmost first
        let budget = r.width() - 54.0 - 16.0 - 160.0;
        let mut used = 0.0;
        let fitting: Vec<Column> = columns
            .iter()
            .copied()
            .take_while(|c| {
                used += c.width();
                used <= budget
            })
            .collect();
        for column in fitting.iter().rev() {
            ui.painter().text(Pos2::new(right, r.center().y), Align2::RIGHT_CENTER, column.value(item), small.clone(), subtext_color);
            right -= column.width();
        }
    }

    let name_color = if theme.light {
        lerp_color(theme.text, theme.mauve, sel_t * 0.3)
    } else {
        lerp_color(theme.text, Color32::WHITE, sel_t * 0.6)
    };
    let galley = truncated(ui, item.display_name(), FONT_SUBTITLE, name_color, right - r.left() - 60.0);
    let pos = Pos2::new(r.left() + 54.0, r.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, name_color);

    if selected && should_scroll {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    response
}

/// Optional detail columns of the list view, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Column {
    Size,
    Modified,
    Created,
    Accessed,
    Owner,
    Permissions,
}

impl Column {
    pub const ALL: [Column; 6] = [
        Column::Size,
        Column::Modified,
        Column::Created,
        Column::Accessed,
        Column::Owner,
        Column::Permissions,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Column::Size => "Size",
            Column::Modified => "Modified",
            Column::Created => "Created",
            Column::Accessed => "Accessed",
            Column::Owner => "Owner",
            Column::Permissions => "Permissions",
        }
    }

    fn width(self) -> f32 {
        match self {
            Column::Size => 76.0,
            Column::Modified => 100.0,
            Column::Created | Column::Accessed => 124.0,
            Column::Owner => 90.0,
            Column::Permissions => 96.0,
        }
    }

    fn value(self, item: &FileItem) -> String {
        match self {
            Column::Size => item.formatted_size(),
            Column::Modified => item.formatted_date(),
            Column::Created => crate::fs::props::format_datetime(item.created),
            Column::Accessed => crate::fs::props::format_datetime(item.accessed),
            Column::Owner => item.owner(),
            Column::Permissions => item.formatted_perms(),
        }
    }
}

/// Entry of a context menu.
pub fn menu_item(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    theme: &SerpantinumTheme,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    let r = scaled(rect, fx.scale);
    let fill = lerp_color(
        lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, ALPHA_HOVER), fx.hover),
        theme.with_alpha(theme.text, ALPHA_PRESS),
        fx.press,
    );
    paint_fill(ui.painter(), r, 17, fill, &fx);

    let color = lerp_color(theme.subtext0, theme.text, fx.hover);
    paint_icon(ui.painter(), Pos2::new(r.left() + 20.0, r.center().y), icon, 16.0, color);
    ui.painter().text(Pos2::new(r.left() + 40.0, r.center().y), Align2::LEFT_CENTER, label, egui::FontId::proportional(FONT_BODY), color);
    response
}

#[allow(dead_code)]
pub const GRID_CELL: Vec2 = Vec2::new(116.0, 128.0);

/// One cell of the grid view.
pub fn file_tile(
    ui: &mut egui::Ui,
    idx: usize,
    item: &FileItem,
    theme: &SerpantinumTheme,
    selected: bool,
    should_scroll: bool,
    thumb: Option<&egui::TextureHandle>,
    grid_size: crate::settings::GridSize,
    max_name_lines: usize,
) -> egui::Response {
    let (cell_w, cell_h) = grid_size.cell_size();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(cell_w, cell_h), egui::Sense::click());
    let id = ui.make_persistent_id(("tile", idx));
    let fx = fx(ui, id, &response, true, &FxSpec::CLICK);
    let sel_t = ui.ctx().animate_bool_with_time_and_easing(id.with("sel"), selected, 0.25, quint_out);
    let r = scaled(rect, fx.scale);

    let idle_bg = lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, ALPHA_HOVER), fx.hover);
    let active_bg = theme.with_alpha(theme.mauve, ALPHA_SEL_BG + (fx.hover * 25.0) as u8);
    let bg = lerp_color(idle_bg, active_bg, sel_t);
    paint_fill(ui.painter(), r, RADIUS_LG, bg, &fx);

    if sel_t > 0.001 {
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(RADIUS_LG),
            egui::Stroke::new(1.5, theme.with_alpha(theme.mauve, (ALPHA_SEL_OUTLINE as f32 * sel_t) as u8)),
            egui::StrokeKind::Inside,
        );
    }

    let pic_sz = grid_size.picture_size();
    let picture = Rect::from_center_size(Pos2::new(r.center().x, r.top() + pic_sz * 0.5 + 12.0), Vec2::splat(pic_sz));
    match thumb {
        Some(tex) => paint_thumb(ui, tex, picture, RADIUS_SM),
        None => {
            let (glyph, base_color) = item_icon(item, theme, selected);
            let icon_color = lerp_color(base_color, theme.mauve, sel_t * 0.7);
            paint_icon(ui.painter(), picture.center(), glyph, grid_size.icon_size(), icon_color);
        }
    }

    let name_color = if theme.light {
        lerp_color(theme.text, theme.mauve, sel_t * 0.3)
    } else {
        lerp_color(theme.text, Color32::WHITE, sel_t * 0.6)
    };

    // Names wrap over configured lines and are centered under the picture, like Nautilus.
    // Fixed: halign Center centers lines around 0 in the galley, so painting at r.center().x
    // positions the text exactly centered without any leftward offset subtraction.
    let mut job = LayoutJob::simple(item.display_name().to_string(), egui::FontId::proportional(FONT_CAPTION), name_color, r.width() - 14.0);
    job.wrap = TextWrapping { max_width: r.width() - 14.0, max_rows: max_name_lines, break_anywhere: true, overflow_character: Some('…') };
    job.halign = egui::Align::Center;
    let galley = ui.painter().layout_job(job);
    let pos = Pos2::new(r.center().x, picture.bottom() + 8.0);
    ui.painter().galley(pos, galley, name_color);

    if selected && should_scroll {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    response
}

/// Material switch row: a label on the left and an animated on/off switch on the right.
pub fn switch_row(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    theme: &SerpantinumTheme,
    on: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    let t = ui.ctx().animate_bool_with_time_and_easing(id.with("on"), on, 0.25, quint_out);
    let fill = lerp_color(
        lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, ALPHA_HOVER), fx.hover),
        theme.with_alpha(theme.text, ALPHA_PRESS),
        fx.press,
    );
    paint_fill(ui.painter(), rect, RADIUS_LG, fill, &fx);

    paint_icon(ui.painter(), Pos2::new(rect.left() + 24.0, rect.center().y), icon, 18.0, lerp_color(theme.subtext0, theme.mauve, t));
    ui.painter().text(Pos2::new(rect.left() + 46.0, rect.center().y), Align2::LEFT_CENTER, label, egui::FontId::proportional(FONT_BODY), theme.text);

    let track = Rect::from_center_size(Pos2::new(rect.right() - 36.0, rect.center().y), Vec2::new(46.0, 26.0));
    let track_color = lerp_color(theme.surface2, theme.mauve, t);
    ui.painter().rect_filled(track, CornerRadius::same(13), track_color);
    let knob_r = lerp(8.0, 10.0, t.max(fx.press));
    let knob_x = lerp(track.left() + 13.0, track.right() - 13.0, t);
    let knob_color = lerp_color(theme.subtext1, theme.crust, t);
    ui.painter().circle_filled(Pos2::new(knob_x, track.center().y), knob_r, knob_color);
    if t > 0.5 {
        paint_icon(ui.painter(), Pos2::new(knob_x, track.center().y), icons::CHECK, 12.0, theme.mauve.gamma_multiply(t));
    }
    response
}

/// Material 3 section label with consistent padding and typography.
pub fn section_label(ui: &mut egui::Ui, text: &str, theme: &SerpantinumTheme) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(text)
            .size(FONT_SECTION)
            .color(theme.subtext1),
    );
    ui.add_space(2.0);
}

/// Standard dialog header with title, optional subtitle, and close button.
/// Returns true if the close button was clicked.
pub fn dialog_header(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: Option<&str>,
    theme: &SerpantinumTheme,
    close_id: impl std::hash::Hash + std::fmt::Debug,
) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).size(FONT_TITLE).color(theme.text));
            if let Some(sub) = subtitle {
                ui.label(egui::RichText::new(sub).size(FONT_CAPTION).color(theme.subtext1));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if filled_icon_button(ui, close_id, icons::CLOSE, theme, 30.0, "Close", Some(theme.red)).clicked() {
                close = true;
            }
        });
    });
    close
}

/// Standard subtle horizontal divider.
pub fn divider(ui: &mut egui::Ui, theme: &SerpantinumTheme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 7.0), egui::Sense::hover());
    let line = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width() - 24.0, 1.0));
    ui.painter().rect_filled(line, 0, theme.with_alpha(theme.text, ALPHA_DIVIDER));
}

/// Tonal card / subpanel frame with consistent background and corner radius.
pub fn card_frame(theme: &SerpantinumTheme, radius: u8) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.surface1)
        .corner_radius(egui::CornerRadius::same(radius))
        .inner_margin(egui::Margin::same(12))
}

/// Input field frame with consistent surface color and corner radius.
pub fn input_frame(theme: &SerpantinumTheme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.surface1)
        .corner_radius(egui::CornerRadius::same(RADIUS_LG))
        .inner_margin(egui::Margin::symmetric(14, 6))
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}


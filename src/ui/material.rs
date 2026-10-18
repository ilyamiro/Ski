//! Flat Material 3 style widgets. Nothing here draws a stroke: shape and state are
//! communicated purely through fill colors (tonal surfaces and state layers).
use eframe::egui::{
    self, text::LayoutJob, text::TextWrapping, Align2, Color32, CornerRadius, Pos2, Rect, Vec2,
};
use super::theme::SerpantinumTheme;
use crate::fs::file_item::FileItem;
use crate::ui::icons;

/// Height shared by every control in the header so the row always lines up.
pub const CONTROL_HEIGHT: f32 = 34.0;

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
    let hover = theme.with_alpha(theme.text, 22);
    let press = theme.with_alpha(theme.text, 40);
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
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(13.0), theme.text);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let width = galley.size().x + 24.0 + icon_w;

    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::CLICK);
    let sel_t = ui.ctx().animate_bool_with_time_and_easing(id.with("sel"), selected, 0.25, quint_out);
    let r = scaled(rect, fx.scale);

    let idle = lerp_color(theme.surface1, theme.surface2, fx.hover);
    let active = blend_over(theme.surface1, theme.with_alpha(theme.mauve, 60 + (fx.hover * 25.0) as u8));
    paint_fill(ui.painter(), r, 15, lerp_color(idle, active, sel_t), &fx);

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, fx.hover), theme.mauve, sel_t);
    let mut x = r.left() + 12.0;
    if let Some(ic) = icon {
        paint_icon(ui.painter(), Pos2::new(x + 7.0, r.center().y), ic, 15.0, color);
        x += 22.0;
    }
    ui.painter().text(Pos2::new(x, r.center().y), Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), color);
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

    let bg = lerp_color(
        lerp_color(
            lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, 16), fx.hover),
            theme.with_alpha(theme.text, 30),
            fx.press,
        ),
        theme.with_alpha(theme.mauve, 52),
        sel_t,
    );
    paint_fill(ui.painter(), r, 18, bg, &fx);

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, fx.hover), theme.mauve, sel_t);
    paint_icon(ui.painter(), Pos2::new(r.left() + 22.0, r.center().y), icon, 17.0, color);

    let galley = truncated(ui, label, 13.5, color, r.width() - 56.0);
    let pos = Pos2::new(r.left() + 42.0, r.center().y - galley.size().y / 2.0);
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

/// Paints an image thumbnail as a rounded square (the texture is pre-cropped to a square).
fn paint_thumb(ui: &egui::Ui, tex: &egui::TextureHandle, rect: Rect, radius: u8) {
    egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), rect.size()))
        .corner_radius(CornerRadius::same(radius))
        .paint_at(ui, rect);
}

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
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), egui::Sense::click());
    let id = ui.make_persistent_id(("row", idx));
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    let sel_t = ui.ctx().animate_bool_with_time_and_easing(id.with("sel"), selected, 0.25, quint_out);
    let r = scaled(rect, fx.scale);

    let bg = lerp_color(
        lerp_color(
            lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, 14), fx.hover),
            theme.with_alpha(theme.text, 28),
            fx.press,
        ),
        theme.with_alpha(theme.mauve, 48),
        sel_t,
    );
    paint_fill(ui.painter(), r, 12, bg, &fx);

    let icon_center = Pos2::new(r.left() + 26.0, r.center().y);
    match thumb {
        Some(tex) => paint_thumb(ui, tex, Rect::from_center_size(icon_center, Vec2::splat(26.0)), 6),
        None => paint_icon(ui.painter(), icon_center, item_icon(item), 20.0, item_icon_color(item, theme, selected)),
    }

    let mut right = r.right() - 16.0;
    let small = egui::FontId::proportional(12.0);
    if let Some(hint) = path_hint {
        let galley = truncated(ui, hint, 12.0, theme.subtext1, (r.width() * 0.4).min(260.0));
        let pos = Pos2::new(right - galley.size().x, r.center().y - galley.size().y / 2.0);
        right = pos.x - 12.0;
        ui.painter().galley(pos, galley, theme.subtext1);
    } else {
        // Columns that don't fit next to a readable name are dropped, rightmost first
        let budget = r.width() - 48.0 - 16.0 - 160.0;
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
            ui.painter().text(Pos2::new(right, r.center().y), Align2::RIGHT_CENTER, column.value(item), small.clone(), theme.subtext1);
            right -= column.width();
        }
    }

    let name_color = lerp_color(lerp_color(theme.subtext0, theme.text, fx.hover), theme.text, sel_t);
    let galley = truncated(ui, &item.name, 13.5, name_color, right - r.left() - 52.0);
    let pos = Pos2::new(r.left() + 48.0, r.center().y - galley.size().y / 2.0);
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
    paint_fill(ui.painter(), r, 17, lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, 18), fx.hover), &fx);

    let color = lerp_color(theme.subtext0, theme.text, fx.hover);
    paint_icon(ui.painter(), Pos2::new(r.left() + 20.0, r.center().y), icon, 16.0, color);
    ui.painter().text(Pos2::new(r.left() + 40.0, r.center().y), Align2::LEFT_CENTER, label, egui::FontId::proportional(13.5), color);
    response
}

pub const GRID_CELL: Vec2 = Vec2::new(104.0, 96.0);

/// One cell of the grid view.
pub fn file_tile(
    ui: &mut egui::Ui,
    idx: usize,
    item: &FileItem,
    theme: &SerpantinumTheme,
    selected: bool,
    should_scroll: bool,
    thumb: Option<&egui::TextureHandle>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(GRID_CELL, egui::Sense::click());
    let id = ui.make_persistent_id(("tile", idx));
    let fx = fx(ui, id, &response, true, &FxSpec::CLICK);
    let sel_t = ui.ctx().animate_bool_with_time_and_easing(id.with("sel"), selected, 0.25, quint_out);
    let r = scaled(rect, fx.scale);

    let bg = lerp_color(
        lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, 14), fx.hover),
        theme.with_alpha(theme.mauve, 48),
        sel_t,
    );
    paint_fill(ui.painter(), r, 14, bg, &fx);

    let icon_center = Pos2::new(r.center().x, r.top() + 34.0);
    match thumb {
        Some(tex) => paint_thumb(ui, tex, Rect::from_center_size(icon_center, Vec2::splat(52.0)), 10),
        None => paint_icon(ui.painter(), icon_center, item_icon(item), 32.0, item_icon_color(item, theme, selected)),
    }

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, fx.hover), theme.text, sel_t);
    let galley = truncated(ui, &item.name, 12.0, color, r.width() - 14.0);
    let pos = Pos2::new(r.center().x - galley.size().x / 2.0, r.bottom() - 24.0);
    ui.painter().galley(pos, galley, color);

    if selected && should_scroll {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    response
}

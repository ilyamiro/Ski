//! Flat Material 3 style widgets. Nothing here draws a stroke: shape and state are
//! communicated purely through fill colors (tonal surfaces and state layers).
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, text::LayoutJob, text::TextWrapping, Align2, Color32, CornerRadius, Pos2, Rect, Vec2,
};
use super::theme::SerpantinumTheme;
use crate::fs::file_item::{specific_icon, FileItem, FileType, Tint};
use crate::fs::scanner::{SortBy, SortOrder};
use crate::ui::icons;

/// Height shared by every control in the header so the row always lines up.
pub const CONTROL_HEIGHT: f32 = 34.0;
/// Height of dialog buttons and segmented controls.
pub const BUTTON_HEIGHT: f32 = 36.0;
/// Height of a chip.
pub const CHIP_HEIGHT: f32 = 30.0;
/// Height of a menu entry or a sidebar destination.
pub const ENTRY_HEIGHT: f32 = 36.0;
/// Height of a settings row.
pub const SETTING_HEIGHT: f32 = 48.0;
/// Height of a key combination pill.
pub const KEYCAP_HEIGHT: f32 = 24.0;

/// Shared Corner Radii (Material 3 scale)
pub const RADIUS_SM: u8 = 8;         // Small elements, previews, thumbnails
pub const RADIUS_MD: u8 = 12;        // Rows, cards, list items
pub const RADIUS_LG: u8 = 18;        // Tiles, input fields, large controls
pub const RADIUS_CONTAINER: u8 = 22; // Menus, popups, sub-panels, scroll boxes
pub const RADIUS_WINDOW: u8 = 26;    // Dialogs and modal frames

/// Shared Font Sizes
pub const FONT_SECTION: f32 = 12.0;
pub const FONT_CAPTION: f32 = 12.5;
pub const FONT_CHIP: f32 = 13.0;
pub const FONT_BODY: f32 = 13.5;
pub const FONT_SUBTITLE: f32 = 14.0;
pub const FONT_TITLE: f32 = 16.0;

/// Shared icon sizes
pub const ICON_XS: f32 = 14.0;
pub const ICON_SM: f32 = 16.0;
pub const ICON_MD: f32 = 18.0;

/// Where icon and label sit inside a navigation-like row, so every list lines up.
pub const INSET_ICON: f32 = 22.0;
pub const INSET_LABEL: f32 = 46.0;

/// Shared State Layer / Tonal Fill Alphas
pub const ALPHA_HOVER: u8 = 20;
pub const ALPHA_PRESS: u8 = 36;
pub const ALPHA_SEL_BG: u8 = 58;
pub const ALPHA_SEL_HOVER: u8 = 74;
pub const ALPHA_DIVIDER: u8 = 24;

/// Shared motion
pub const ANIM: f32 = 0.25;
pub const ANIM_SLOW: f32 = 0.32;

static ANIMATIONS: AtomicBool = AtomicBool::new(true);

pub fn set_animations(on: bool) {
    ANIMATIONS.store(on, Ordering::Relaxed);
}

pub fn animations_enabled() -> bool {
    ANIMATIONS.load(Ordering::Relaxed)
}

/// Duration after the "animations" preference: `d` seconds, or an instant jump.
pub fn dur(d: f32) -> f32 {
    if animations_enabled() { d } else { 0.0 }
}

/// 0 -> 1 (or back) eased with the shared OutQuint curve and duration.
pub fn animate(ctx: &egui::Context, id: egui::Id, target: bool) -> f32 {
    ctx.animate_bool_with_time_and_easing(id, target, dur(ANIM), quint_out)
}

pub fn animate_for(ctx: &egui::Context, id: egui::Id, target: bool, seconds: f32) -> f32 {
    ctx.animate_bool_with_time_and_easing(id, target, dur(seconds), quint_out)
}

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
    let hover = animate(ctx, id.with("hov"), enabled && resp.hovered());
    let press = animate(ctx, id.with("prs"), enabled && resp.is_pointer_button_down_on());

    let now = ctx.input(|i| i.time);
    let pop_key = id.with("pop");
    if enabled && resp.clicked() {
        ctx.data_mut(|d| d.insert_temp(pop_key, now));
    }
    let elapsed = if animations_enabled() {
        ctx.data(|d| d.get_temp::<f64>(pop_key)).map(|t| (now - t) as f32).unwrap_or(f32::MAX)
    } else {
        f32::MAX
    };

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
    if scale == 1.0 { rect } else { Rect::from_center_size(rect.center(), rect.size() * scale) }
}

fn flash_color(flash: f32) -> Color32 {
    Color32::from_white_alpha((flash * 255.0 * 0.5) as u8)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    // Color32 stores premultiplied channels, and fading between them in that space is what makes
    // a translucent layer fade in and out correctly
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()), mix(a.a(), b.a()))
}

fn blend_over(under: Color32, over: Color32) -> Color32 {
    lerp_color(under, Color32::from_rgb(over.r(), over.g(), over.b()), over.a() as f32 / 255.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Text color that stays readable on top of `bg` (the accent, for filled buttons).
pub fn on_color(theme: &SerpantinumTheme, bg: Color32) -> Color32 {
    let luma = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if luma > 150.0 { theme.crust.gamma_multiply(1.0) } else { Color32::WHITE }
}

/// Fully rounded corners for a control of this height.
pub fn pill(rect: Rect) -> CornerRadius {
    CornerRadius::same((rect.height() / 2.0).round().min(255.0) as u8)
}

/// Rounds a rect to whole device pixels so flat fills have crisp edges at any interface scale.
fn snap(painter: &egui::Painter, rect: Rect) -> Rect {
    let ppp = painter.ctx().pixels_per_point();
    let round = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_max(Pos2::new(round(rect.min.x), round(rect.min.y)), Pos2::new(round(rect.max.x), round(rect.max.y)))
}

/// Tonal layer for the idle / hover / press states shared by every row.
fn row_layer(theme: &SerpantinumTheme, fx: &Fx) -> Color32 {
    lerp_color(
        lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, ALPHA_HOVER), fx.hover),
        theme.with_alpha(theme.text, ALPHA_PRESS),
        fx.press,
    )
}

/// Row background that fades to the accent tint when selected.
fn selectable_bg(theme: &SerpantinumTheme, fx: &Fx, sel_t: f32) -> Color32 {
    let sel_alpha = (ALPHA_SEL_BG as f32 + fx.hover * (ALPHA_SEL_HOVER - ALPHA_SEL_BG) as f32) as u8;
    lerp_color(row_layer(theme, fx), theme.with_alpha(theme.mauve, sel_alpha), sel_t)
}

fn paint_fill(painter: &egui::Painter, rect: Rect, radius: CornerRadius, base: Color32, fx: &Fx) {
    let rect = snap(painter, rect);
    if base.a() > 0 {
        painter.rect_filled(rect, radius, base);
    }
    if fx.flash > 0.0 {
        painter.rect_filled(rect, radius, flash_color(fx.flash));
    }
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

/// Paints single-line text, vertically centered on `y`, with its left edge at `x`.
pub fn paint_label(ui: &egui::Ui, x: f32, y: f32, text: &str, size: f32, color: Color32, max_width: f32) {
    let galley = truncated(ui, text, size, color, max_width);
    ui.painter().galley(Pos2::new(x, y - galley.size().y / 2.0), galley, color);
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
        None => row_layer(theme, &fx),
    };
    paint_fill(ui.painter(), r, pill(r), layer, &fx);

    let hover_color = accent.unwrap_or(theme.text);
    let color = if enabled {
        lerp_color(theme.subtext1, hover_color, fx.hover)
    } else {
        theme.with_alpha(theme.subtext1, 70)
    };
    paint_icon(ui.painter(), r.center(), icon, ICON_MD, color);

    if tooltip.is_empty() { response } else { response.on_hover_text(tooltip) }
}

/// Icon-only counterpart of `chip`: a filled round button (dialog close buttons, small resets).
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
    paint_fill(ui.painter(), r, pill(r), lerp_color(idle, hovered, fx.hover), &fx);

    let color = lerp_color(theme.subtext1, accent.unwrap_or(theme.text), fx.hover);
    paint_icon(ui.painter(), r.center(), icon, (size * 0.5).round(), color);
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
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(FONT_CHIP), theme.text);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let width = galley.size().x + 24.0 + icon_w;

    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, CHIP_HEIGHT), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::CLICK);
    let sel_t = animate(ui.ctx(), id.with("sel"), selected);
    let r = scaled(rect, fx.scale);

    let idle = lerp_color(theme.surface1, theme.surface2, fx.hover);
    let active = blend_over(theme.surface1, theme.with_alpha(theme.mauve, 64 + (fx.hover * 20.0) as u8));
    paint_fill(ui.painter(), r, pill(r), lerp_color(idle, active, sel_t), &fx);

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, fx.hover), theme.mauve, sel_t);
    let mut x = r.left() + 12.0;
    if let Some(ic) = icon {
        paint_icon(ui.painter(), Pos2::new(x + 7.0, r.center().y), ic, ICON_SM, color);
        x += 22.0;
    }
    ui.painter().text(Pos2::new(x, r.center().y), Align2::LEFT_CENTER, label, egui::FontId::proportional(FONT_CHIP), color);
    response
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// Solid accent fill: the one action a dialog is about.
    Filled,
    /// Solid red fill for actions that destroy something.
    Danger,
    /// Tonal fill for secondary actions.
    Tonal,
    /// Text only, for dismissing.
    Text,
}

/// Dialog button.
pub fn button(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: Option<&str>,
    label: &str,
    theme: &SerpantinumTheme,
    kind: ButtonKind,
    enabled: bool,
) -> egui::Response {
    let font = egui::FontId::proportional(FONT_BODY);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), theme.text);
    let icon_w = if icon.is_some() { 24.0 } else { 0.0 };
    let width = galley.size().x + 32.0 + icon_w;

    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, BUTTON_HEIGHT), sense);
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, enabled, &FxSpec::CLICK);
    let r = scaled(rect, fx.scale);

    let (fill, color) = match kind {
        ButtonKind::Filled => (
            lerp_color(theme.mauve, Color32::WHITE, fx.hover * 0.12),
            on_color(theme, theme.mauve),
        ),
        ButtonKind::Danger => (
            lerp_color(theme.red, Color32::WHITE, fx.hover * 0.12),
            on_color(theme, theme.red),
        ),
        ButtonKind::Tonal => (lerp_color(theme.surface1, theme.surface2, fx.hover), theme.text),
        ButtonKind::Text => (
            lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.mauve, 30), fx.hover),
            theme.mauve,
        ),
    };
    let fade = if enabled { 1.0 } else { 0.4 };
    paint_fill(ui.painter(), r, pill(r), fill.gamma_multiply(fade), &fx);

    let color = color.gamma_multiply(fade);
    let mut x = r.left() + 16.0;
    if let Some(ic) = icon {
        paint_icon(ui.painter(), Pos2::new(x + 8.0, r.center().y), ic, ICON_SM, color);
        x += 24.0;
    }
    ui.painter().text(Pos2::new(x, r.center().y), Align2::LEFT_CENTER, label, font, color);
    response
}

/// Material segmented button: mutually exclusive choices that share one container, with the
/// selection indicator sliding between them. Returns the index that was clicked.
pub fn segmented(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    options: &[&str],
    selected: usize,
    theme: &SerpantinumTheme,
) -> Option<usize> {
    let font = egui::FontId::proportional(FONT_CHIP);
    let widths: Vec<f32> = options
        .iter()
        .map(|o| (ui.painter().layout_no_wrap((*o).to_owned(), font.clone(), theme.text).size().x + 28.0).max(52.0))
        .collect();
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total + 8.0, CHIP_HEIGHT + 8.0), egui::Sense::hover());
    let id = ui.make_persistent_id(id_source);
    ui.painter().rect_filled(snap(ui.painter(), rect), pill(rect), theme.surface1);

    let inner = rect.shrink(4.0);
    let mut x = inner.left();
    let mut cells = Vec::with_capacity(options.len());
    for w in &widths {
        cells.push(Rect::from_min_size(Pos2::new(x, inner.top()), Vec2::new(*w, inner.height())));
        x += w;
    }

    // The indicator glides to the selected cell
    let target = cells.get(selected).copied().unwrap_or(inner);
    let ctx = ui.ctx();
    let ix = ctx.animate_value_with_time(id.with("ix"), target.left(), dur(ANIM));
    let iw = ctx.animate_value_with_time(id.with("iw"), target.width(), dur(ANIM));
    let indicator = Rect::from_min_size(Pos2::new(ix, inner.top()), Vec2::new(iw, inner.height()));
    ui.painter().rect_filled(snap(ui.painter(), indicator), pill(indicator), blend_over(theme.surface1, theme.with_alpha(theme.mauve, 72)));

    let mut clicked = None;
    for (i, (cell, label)) in cells.iter().zip(options).enumerate() {
        let resp = ui.interact(*cell, id.with(i), egui::Sense::click());
        let hover = animate(ui.ctx(), id.with(("hov", i)), resp.hovered() && i != selected);
        ui.painter().rect_filled(snap(ui.painter(), *cell), pill(*cell), theme.with_alpha(theme.text, (hover * ALPHA_HOVER as f32) as u8));
        let on = animate(ui.ctx(), id.with(("on", i)), i == selected);
        let color = lerp_color(lerp_color(theme.subtext0, theme.text, hover), theme.mauve, on);
        ui.painter().text(cell.center(), Align2::CENTER_CENTER, *label, font.clone(), color);
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
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
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ENTRY_HEIGHT), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    let sel_t = animate(ui.ctx(), id.with("sel"), selected);
    let r = scaled(rect, fx.scale);

    paint_fill(ui.painter(), r, pill(r), selectable_bg(theme, &fx, sel_t), &fx);

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, fx.hover), theme.mauve, sel_t);
    paint_icon(ui.painter(), Pos2::new(r.left() + INSET_ICON, r.center().y), icon, ICON_MD, color);
    paint_label(ui, r.left() + INSET_LABEL, r.center().y, label, FONT_BODY, color, r.width() - INSET_LABEL - 14.0);
    response
}

/// Glyph and color of a file's type icon.
pub fn item_icon(item: &FileItem, theme: &SerpantinumTheme, selected: bool) -> (&'static str, Color32) {
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
fn paint_thumb(ui: &egui::Ui, tex: &egui::TextureHandle, rect: Rect, radius: u8) {
    let size = tex.size_vec2();
    let fit = (rect.width() / size.x).min(rect.height() / size.y);
    let shown = Rect::from_center_size(rect.center(), size * fit);
    egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), shown.size()))
        .corner_radius(CornerRadius::same(radius))
        .paint_at(ui, shown);
}

/// Left edge of the name in a list row; the header lines its first label up with it.
const ROW_NAME_X: f32 = 54.0;
const ROW_PAD_RIGHT: f32 = 16.0;
/// Narrowest the name may get before detail columns start dropping out.
const ROW_NAME_MIN: f32 = 160.0;

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

    /// Slot width, including the gap to the column on its left.
    fn width(self) -> f32 {
        match self {
            Column::Size => 92.0,
            Column::Modified | Column::Created | Column::Accessed => 146.0,
            Column::Owner => 110.0,
            Column::Permissions => 116.0,
        }
    }

    /// The sort order this column header toggles, if the listing can be sorted by it.
    pub fn sort_key(self) -> Option<SortBy> {
        match self {
            Column::Size => Some(SortBy::Size),
            Column::Modified => Some(SortBy::Modified),
            _ => None,
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

/// Columns that fit next to a readable name in a row of `width`, dropping the rightmost first.
fn fitting_columns(columns: &[Column], width: f32) -> Vec<Column> {
    let budget = width - ROW_NAME_X - ROW_PAD_RIGHT - ROW_NAME_MIN;
    let mut used = 0.0;
    columns
        .iter()
        .copied()
        .take_while(|c| {
            used += c.width();
            used <= budget
        })
        .collect()
}

/// Right edge of every shown column, right-aligned to the end of the row.
fn column_edges(fitting: &[Column], row: Rect) -> Vec<(Column, f32)> {
    let mut right = row.right() - ROW_PAD_RIGHT;
    let mut edges: Vec<(Column, f32)> = fitting
        .iter()
        .rev()
        .map(|c| {
            let edge = (*c, right);
            right -= c.width();
            edge
        })
        .collect();
    edges.reverse();
    edges
}

/// Header of the list view: column titles that sort the listing when clicked.
/// Returns the sort field that was clicked.
pub fn list_header(
    ui: &mut egui::Ui,
    theme: &SerpantinumTheme,
    columns: &[Column],
    sort_by: SortBy,
    order: SortOrder,
) -> Option<SortBy> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), egui::Sense::hover());
    let fitting = fitting_columns(columns, rect.width());
    let mut clicked = None;

    let mut cell = |ui: &mut egui::Ui, id: &str, key: Option<SortBy>, label: &str, cell: Rect, align_right: bool| {
        let resp = ui.interact(cell, ui.make_persistent_id(("list_header", id)), if key.is_some() { egui::Sense::click() } else { egui::Sense::hover() });
        let hover = animate(ui.ctx(), resp.id.with("h"), key.is_some() && resp.hovered());
        let active = key == Some(sort_by);
        ui.painter().rect_filled(snap(ui.painter(), cell), pill(cell), theme.with_alpha(theme.text, (hover * ALPHA_HOVER as f32) as u8));
        let color = lerp_color(theme.subtext1, theme.text, hover.max(if active { 1.0 } else { 0.0 }));
        let font = egui::FontId::proportional(FONT_CAPTION);
        let arrow = if active { Some(if order == SortOrder::Ascending { icons::SORT_ASC } else { icons::SORT_DESC }) } else { None };
        let (anchor, pos) = if align_right {
            (Align2::RIGHT_CENTER, Pos2::new(cell.right() - 10.0, cell.center().y))
        } else {
            (Align2::LEFT_CENTER, Pos2::new(cell.left() + 10.0, cell.center().y))
        };
        let text_rect = ui.painter().text(pos, anchor, label, font, color);
        if let Some(a) = arrow {
            let x = if align_right { text_rect.left() - 10.0 } else { text_rect.right() + 10.0 };
            paint_icon(ui.painter(), Pos2::new(x, cell.center().y), a, ICON_XS, theme.mauve);
        }
        if resp.clicked() {
            clicked = key;
        }
    };

    let name_cell = Rect::from_min_size(
        Pos2::new(rect.left() + ROW_NAME_X - 10.0, rect.top() + 2.0),
        Vec2::new(72.0, rect.height() - 4.0),
    );
    cell(ui, "name", Some(SortBy::Name), "Name", name_cell, false);
    for (column, edge) in column_edges(&fitting, rect) {
        let width = (column.width() - 6.0).min(110.0);
        let slot = Rect::from_min_size(Pos2::new(edge + 10.0 - width, rect.top() + 2.0), Vec2::new(width, rect.height() - 4.0));
        cell(ui, column.label(), column.sort_key(), column.label(), slot, true);
    }
    clicked
}

/// One row of the list view. Columns drop out as the row gets narrower.
/// `path_hint` replaces size/date (used by search results to show where the file lives).
#[allow(clippy::too_many_arguments)]
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
    let sel_t = animate(ui.ctx(), id.with("sel"), selected);
    let r = scaled(rect, fx.scale);

    paint_fill(ui.painter(), r, CornerRadius::same(RADIUS_MD), selectable_bg(theme, &fx, sel_t), &fx);

    let icon_center = Pos2::new(r.left() + 28.0, r.center().y);
    match thumb {
        Some(tex) => paint_thumb(ui, tex, Rect::from_center_size(icon_center, Vec2::splat(32.0)), RADIUS_SM),
        None => {
            let (glyph, color) = item_icon(item, theme, selected);
            paint_icon(ui.painter(), icon_center, glyph, 24.0, color);
        }
    }

    let mut right = r.right() - ROW_PAD_RIGHT;
    let small = egui::FontId::proportional(FONT_CAPTION);
    let subtext_color = theme.subtext1;
    if let Some(hint) = path_hint {
        let galley = truncated(ui, hint, FONT_CAPTION, subtext_color, (r.width() * 0.4).min(260.0));
        let pos = Pos2::new(right - galley.size().x, r.center().y - galley.size().y / 2.0);
        right = pos.x - 12.0;
        ui.painter().galley(pos, galley, subtext_color);
    } else {
        let fitting = fitting_columns(columns, r.width());
        for (column, edge) in column_edges(&fitting, r) {
            ui.painter().text(Pos2::new(edge, r.center().y), Align2::RIGHT_CENTER, column.value(item), small.clone(), subtext_color);
        }
        right -= fitting.iter().map(|c| c.width()).sum::<f32>();
    }

    paint_label(ui, r.left() + ROW_NAME_X, r.center().y, item.display_name(), FONT_SUBTITLE, theme.text, right - r.left() - ROW_NAME_X - 6.0);

    if selected && should_scroll {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    response
}

/// Entry of a context menu. `hint` is the keyboard shortcut shown at the right edge.
pub fn menu_item(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    hint: Option<&str>,
    theme: &SerpantinumTheme,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ENTRY_HEIGHT), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    let r = scaled(rect, fx.scale);
    paint_fill(ui.painter(), r, pill(r), row_layer(theme, &fx), &fx);

    let color = lerp_color(theme.subtext0, theme.text, fx.hover);
    paint_icon(ui.painter(), Pos2::new(r.left() + INSET_ICON, r.center().y), icon, ICON_MD, color);

    let mut label_room = r.width() - INSET_LABEL - 14.0;
    if let Some(hint) = hint {
        let galley = ui.painter().layout_no_wrap(hint.to_owned(), egui::FontId::proportional(FONT_CAPTION), theme.subtext1);
        label_room -= galley.size().x + 12.0;
        ui.painter().galley(Pos2::new(r.right() - 16.0 - galley.size().x, r.center().y - galley.size().y / 2.0), galley, theme.subtext1);
    }
    paint_label(ui, r.left() + INSET_LABEL, r.center().y, label, FONT_BODY, color, label_room);
    response
}

/// One cell of the grid view.
#[allow(clippy::too_many_arguments)]
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
    let sel_t = animate(ui.ctx(), id.with("sel"), selected);
    let r = scaled(rect, fx.scale);

    paint_fill(ui.painter(), r, CornerRadius::same(RADIUS_LG), selectable_bg(theme, &fx, sel_t), &fx);

    let pic_sz = grid_size.picture_size();
    let picture = Rect::from_center_size(Pos2::new(r.center().x, r.top() + pic_sz * 0.5 + 12.0), Vec2::splat(pic_sz));
    match thumb {
        Some(tex) => paint_thumb(ui, tex, picture, RADIUS_SM),
        None => {
            let (glyph, color) = item_icon(item, theme, selected);
            paint_icon(ui.painter(), picture.center(), glyph, grid_size.icon_size(), color);
        }
    }

    // Names wrap over the configured lines and are centered under the picture, like Nautilus.
    // With halign Center the galley is centered on x = 0, so it is painted at the cell's center.
    let name_color = theme.text;
    let mut job = LayoutJob::simple(item.display_name().to_string(), egui::FontId::proportional(FONT_CAPTION), name_color, r.width() - 14.0);
    job.wrap = TextWrapping { max_width: r.width() - 14.0, max_rows: max_name_lines, break_anywhere: false, overflow_character: Some('…') };
    job.halign = egui::Align::Center;
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(Pos2::new(r.center().x, picture.bottom() + 8.0), galley, name_color);

    if selected && should_scroll {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    response
}

/// Material switch row: icon, label (with an optional description) and an animated on/off switch.
pub fn switch_row(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    theme: &SerpantinumTheme,
    on: bool,
) -> egui::Response {
    switch_row_with(ui, id_source, icon, label, None, theme, on)
}

pub fn switch_row_with(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    description: Option<&str>,
    theme: &SerpantinumTheme,
    on: bool,
) -> egui::Response {
    let height = if description.is_some() { SETTING_HEIGHT + 8.0 } else { SETTING_HEIGHT };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    let t = animate(ui.ctx(), id.with("on"), on);
    paint_fill(ui.painter(), rect, CornerRadius::same(RADIUS_LG), row_layer(theme, &fx), &fx);

    paint_icon(ui.painter(), Pos2::new(rect.left() + INSET_ICON + 2.0, rect.center().y), icon, ICON_MD, lerp_color(theme.subtext0, theme.mauve, t));
    let text_x = rect.left() + INSET_LABEL + 2.0;
    let room = rect.width() - (text_x - rect.left()) - 64.0;
    match description {
        Some(desc) => {
            paint_label(ui, text_x, rect.center().y - 9.0, label, FONT_BODY, theme.text, room);
            paint_label(ui, text_x, rect.center().y + 10.0, desc, FONT_CAPTION, theme.subtext1, room);
        }
        None => paint_label(ui, text_x, rect.center().y, label, FONT_BODY, theme.text, room),
    }

    paint_switch(ui, Pos2::new(rect.right() - 30.0, rect.center().y), t, fx.press, theme);
    response
}

/// Flat on/off switch: a thin track and a plain knob, no glyphs.
fn paint_switch(ui: &egui::Ui, center: Pos2, on: f32, press: f32, theme: &SerpantinumTheme) {
    let track = Rect::from_center_size(center, Vec2::new(34.0, 18.0));
    ui.painter().rect_filled(snap(ui.painter(), track), CornerRadius::same(9), lerp_color(theme.surface2, theme.mauve, on));
    let knob_r = lerp(5.5, 6.5, on.max(press));
    let knob_x = lerp(track.left() + 9.0, track.right() - 9.0, on);
    let knob = lerp_color(theme.subtext1, on_color(theme, theme.mauve), on);
    ui.painter().circle_filled(Pos2::new(knob_x, track.center().y), knob_r, knob);
}

const MENU_ROW_HEIGHT: f32 = 32.0;

fn menu_row_frame(ui: &mut egui::Ui, id_source: impl std::hash::Hash + std::fmt::Debug, theme: &SerpantinumTheme) -> (Rect, egui::Response, Fx) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), MENU_ROW_HEIGHT), egui::Sense::click());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    paint_fill(ui.painter(), rect, CornerRadius::same(RADIUS_SM), row_layer(theme, &fx), &fx);
    (rect, response, fx)
}

/// Compact menu row with a radio dot: one of several exclusive choices.
pub fn radio_row(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    theme: &SerpantinumTheme,
    selected: bool,
) -> egui::Response {
    let (rect, response, _) = menu_row_frame(ui, &id_source, theme);
    let t = animate(ui.ctx(), response.id.with("on"), selected);
    let center = Pos2::new(rect.left() + 18.0, rect.center().y);
    let ring = lerp_color(theme.subtext1, theme.mauve, t);
    ui.painter().circle_stroke(center, 7.0, egui::Stroke::new(1.5, ring));
    ui.painter().circle_filled(center, 3.5 * t, theme.mauve);
    paint_label(ui, rect.left() + 38.0, rect.center().y, label, FONT_BODY, theme.text, rect.width() - 48.0);
    response
}

/// Compact menu row with a small switch at the right and an optional shortcut next to it.
pub fn toggle_row(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    hint: Option<&str>,
    theme: &SerpantinumTheme,
    on: bool,
) -> egui::Response {
    let (rect, response, fx) = menu_row_frame(ui, &id_source, theme);
    let t = animate(ui.ctx(), response.id.with("on"), on);
    let switch_cx = rect.right() - 12.0 - 17.0;
    paint_switch(ui, Pos2::new(switch_cx, rect.center().y), t, fx.press, theme);
    let mut room = rect.width() - 24.0 - 34.0 - 8.0;
    if let Some(hint) = hint {
        let galley = ui.painter().layout_no_wrap(hint.to_owned(), egui::FontId::proportional(FONT_CAPTION), theme.subtext1);
        let x = switch_cx - 17.0 - 10.0 - galley.size().x;
        ui.painter().galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, theme.subtext1);
        room = x - rect.left() - 12.0 - 10.0;
    }
    paint_label(ui, rect.left() + 12.0, rect.center().y, label, FONT_BODY, theme.text, room);
    response
}

/// Menu row with a label and a pair of round minus / plus buttons. Returns (minus, plus) clicks.
pub fn stepper_row(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    theme: &SerpantinumTheme,
    can_dec: bool,
    can_inc: bool,
) -> (bool, bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), MENU_ROW_HEIGHT), egui::Sense::hover());
    let id = ui.make_persistent_id(id_source);
    paint_label(ui, rect.left() + 12.0, rect.center().y, label, FONT_BODY, theme.text, rect.width() - 100.0);
    let mut out = [false; 2];
    for (i, (icon, enabled)) in [(icons::REMOVE, can_dec), (icons::ADD, can_inc)].into_iter().enumerate() {
        let cell = Rect::from_center_size(Pos2::new(rect.right() - 25.0 - (1 - i) as f32 * 32.0, rect.center().y), Vec2::splat(26.0));
        let resp = ui.interact(cell, id.with(i), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
        let hover = animate(ui.ctx(), id.with(("h", i)), resp.hovered() && enabled);
        ui.painter().rect_filled(cell, pill(cell), lerp_color(theme.surface1, theme.surface2, hover));
        let color = if enabled { lerp_color(theme.subtext1, theme.text, hover) } else { theme.with_alpha(theme.subtext1, 70) };
        paint_icon(ui.painter(), cell.center(), icon, ICON_SM, color);
        out[i] = resp.clicked();
    }
    (out[0], out[1])
}

/// Back and forward as one joined pill: two flat halves, dimmed when there is nowhere to go.
pub fn nav_pair(ui: &mut egui::Ui, theme: &SerpantinumTheme, can_back: bool, can_forward: bool) -> (egui::Response, egui::Response) {
    const W: f32 = 32.0;
    const H: f32 = 30.0;
    let (slot, _) = ui.allocate_exact_size(Vec2::new(W * 2.0, CONTROL_HEIGHT), egui::Sense::hover());
    let body = Rect::from_center_size(slot.center(), Vec2::new(W * 2.0, H));
    ui.painter().rect_filled(snap(ui.painter(), body), pill(body), theme.surface0);
    let id = ui.make_persistent_id("nav_pair");
    let mut responses = Vec::with_capacity(2);
    for (i, (icon, enabled, tip)) in [(icons::ARROW_BACK, can_back, "Back"), (icons::ARROW_FORWARD, can_forward, "Forward")].into_iter().enumerate() {
        let cell = Rect::from_min_size(body.min + Vec2::new(W * i as f32, 0.0), Vec2::new(W, H));
        let resp = ui.interact(cell, id.with(i), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
        let hover = animate(ui.ctx(), id.with(("h", i)), resp.hovered() && enabled);
        let press = enabled && resp.is_pointer_button_down_on();
        let fill = theme.with_alpha(theme.text, (hover * ALPHA_HOVER as f32) as u8 + if press { ALPHA_PRESS - ALPHA_HOVER } else { 0 });
        ui.painter().rect_filled(snap(ui.painter(), cell.shrink(2.0)), pill(cell.shrink(2.0)), fill);
        let color = if enabled { lerp_color(theme.subtext1, theme.text, hover) } else { theme.with_alpha(theme.subtext1, 70) };
        paint_icon(ui.painter(), cell.center(), icon, ICON_SM, color);
        responses.push(resp.on_hover_text(tip));
    }
    let forward = responses.pop().unwrap();
    (responses.pop().unwrap(), forward)
}

/// Settings row with a segmented control: label on the left, choices on the right. When the
/// choices do not fit next to the label they wrap onto a line of their own.
/// Returns the index that was clicked.
pub fn choice_row(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    options: &[&str],
    selected: usize,
    theme: &SerpantinumTheme,
) -> Option<usize> {
    let id = ui.make_persistent_id(id_source);
    let font = egui::FontId::proportional(FONT_CHIP);
    let seg_w: f32 = options
        .iter()
        .map(|o| (ui.painter().layout_no_wrap((*o).to_owned(), font.clone(), theme.text).size().x + 28.0).max(52.0))
        .sum::<f32>()
        + 8.0;
    let label_w = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(FONT_BODY), theme.text).size().x;
    let width = ui.available_width();
    let inline = INSET_LABEL + label_w + 16.0 + seg_w + 8.0 <= width;

    let seg_h = CHIP_HEIGHT + 8.0;
    let height = if inline { SETTING_HEIGHT } else { SETTING_HEIGHT + seg_h + 2.0 };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());

    let head_cy = if inline { rect.center().y } else { rect.top() + SETTING_HEIGHT / 2.0 };
    paint_icon(ui.painter(), Pos2::new(rect.left() + INSET_ICON + 2.0, head_cy), icon, ICON_MD, theme.subtext0);
    paint_label(ui, rect.left() + INSET_LABEL + 2.0, head_cy, label, FONT_BODY, theme.text, width - INSET_LABEL - 12.0);

    let seg_rect = if inline {
        Rect::from_min_size(Pos2::new(rect.right() - seg_w - 8.0, rect.center().y - seg_h / 2.0), Vec2::new(seg_w, seg_h))
    } else {
        Rect::from_min_size(Pos2::new(rect.left() + INSET_LABEL + 2.0, rect.top() + SETTING_HEIGHT - 4.0), Vec2::new(seg_w, seg_h))
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(seg_rect), |ui| segmented(ui, id, options, selected, theme)).inner
}

/// Material 3 section label with consistent padding and typography.
pub fn section_label(ui: &mut egui::Ui, text: &str, theme: &SerpantinumTheme) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(text).size(FONT_SECTION).color(theme.subtext1));
    });
    ui.add_space(2.0);
}

/// Rounded tonal container that groups related settings rows.
pub fn group_card<R>(ui: &mut egui::Ui, theme: &SerpantinumTheme, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme.surface0)
        .corner_radius(CornerRadius::same(RADIUS_CONTAINER))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            add(ui)
        })
        .inner
}

/// Quiet explanatory line under a section.
pub fn hint_label(ui: &mut egui::Ui, text: &str, theme: &SerpantinumTheme) {
    ui.horizontal_wrapped(|ui| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(text).size(FONT_CAPTION).color(theme.subtext1));
    });
}

/// Standard dialog header with title, optional subtitle, and close button.
/// The title always shares its center line with the close button.
/// Returns true if the close button was clicked.
pub fn dialog_header(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: Option<&str>,
    theme: &SerpantinumTheme,
    close_id: impl std::hash::Hash + std::fmt::Debug,
) -> bool {
    const SIZE: f32 = 30.0;
    let mut close = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width() - SIZE - 8.0, SIZE), egui::Sense::hover());
            paint_label(ui, rect.left(), rect.center().y, title, FONT_TITLE, theme.text, rect.width());
            if let Some(sub) = subtitle {
                ui.add_space(1.0);
                ui.label(egui::RichText::new(sub).size(FONT_CAPTION).color(theme.subtext1));
                ui.add_space(4.0);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if filled_icon_button(ui, close_id, icons::CLOSE, theme, SIZE, "Close", Some(theme.red)).clicked() {
                close = true;
            }
        });
    });
    close
}

/// What the footer of a dialog reported.
#[derive(Default)]
pub struct Footer {
    pub primary: bool,
    pub cancel: bool,
    pub extra: bool,
}

/// What a dialog footer offers.
pub struct FooterSpec<'a> {
    pub id: &'a str,
    pub icon: Option<&'a str>,
    pub primary: &'a str,
    pub enabled: bool,
    /// The primary action destroys something: shown in red.
    pub danger: bool,
    /// Optional secondary action at the left edge.
    pub extra: Option<&'a str>,
}

impl<'a> FooterSpec<'a> {
    pub fn new(id: &'a str, icon: &'a str, primary: &'a str) -> Self {
        Self { id, icon: Some(icon), primary, enabled: true, danger: false, extra: None }
    }
}

/// Dialog actions, right aligned: the primary action, then Cancel. An optional secondary action
/// sits at the left edge.
pub fn dialog_footer(ui: &mut egui::Ui, theme: &SerpantinumTheme, spec: FooterSpec) -> Footer {
    let mut out = Footer::default();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if let Some(extra) = spec.extra {
            out.extra = button(ui, (spec.id, "extra"), None, extra, theme, ButtonKind::Tonal, true).clicked();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let kind = if spec.danger { ButtonKind::Danger } else { ButtonKind::Filled };
            out.primary = button(ui, (spec.id, "primary"), spec.icon, spec.primary, theme, kind, spec.enabled).clicked() && spec.enabled;
            out.cancel = button(ui, (spec.id, "cancel"), None, "Cancel", theme, ButtonKind::Text, true).clicked();
        });
    });
    out
}

/// Standard subtle horizontal divider.
pub fn divider(ui: &mut egui::Ui, theme: &SerpantinumTheme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 9.0), egui::Sense::hover());
    let line = Rect::from_center_size(rect.center(), egui::vec2(rect.width() - 24.0, 1.0));
    ui.painter().rect_filled(snap(ui.painter(), line), 0, theme.with_alpha(theme.text, ALPHA_DIVIDER));
}

/// Input field frame with consistent surface color and corner radius.
pub fn input_frame(theme: &SerpantinumTheme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.surface1)
        .corner_radius(CornerRadius::same(RADIUS_LG))
        .inner_margin(egui::Margin::symmetric(14, 8))
}

/// Centered placeholder for views without content: a large quiet icon, a title and a hint.
pub fn empty_state(ui: &mut egui::Ui, theme: &SerpantinumTheme, icon: &str, title: &str, hint: Option<&str>) {
    let rect = ui.max_rect();
    let center = rect.center() - Vec2::new(0.0, 16.0);
    paint_icon(ui.painter(), center - Vec2::new(0.0, 34.0), icon, 56.0, theme.surface2);
    ui.painter().text(center + Vec2::new(0.0, 12.0), Align2::CENTER_CENTER, title, egui::FontId::proportional(FONT_TITLE), theme.subtext0);
    if let Some(hint) = hint {
        ui.painter().text(center + Vec2::new(0.0, 36.0), Align2::CENTER_CENTER, hint, egui::FontId::proportional(FONT_CAPTION), theme.subtext1);
    }
}

/// A key combination drawn as a small flat pill.
pub fn keycap(ui: &mut egui::Ui, text: &str, theme: &SerpantinumTheme) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), egui::FontId::proportional(FONT_CAPTION), theme.subtext0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 20.0, KEYCAP_HEIGHT), egui::Sense::hover());
    ui.painter().rect_filled(snap(ui.painter(), rect), CornerRadius::same(RADIUS_SM), theme.surface2);
    ui.painter().galley(Pos2::new(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, theme.subtext0);
    response
}

/// Adds a widget at `rect` like `Ui::put`, but without advancing the parent layout's cursor,
/// so widgets drawn on top of an already allocated area cannot shift their neighbours.
pub fn put_over(ui: &mut egui::Ui, rect: Rect, widget: impl egui::Widget) -> egui::Response {
    let builder = egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::centered_and_justified(egui::Direction::TopDown));
    ui.new_child(builder).add(widget)
}

/// An icon inside a line of widgets (the magnifier in front of a search field).
pub fn inline_icon(ui: &mut egui::Ui, icon: &str, size: f32, color: Color32) {
    ui.label(egui::RichText::new(icon).font(icons::font(size)).color(color));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translucent_layers_fade_in_proportionally() {
        let accent = Color32::from_rgba_unmultiplied(200, 100, 50, 80);
        assert_eq!(lerp_color(Color32::TRANSPARENT, accent, 1.0), accent);
        assert_eq!(lerp_color(Color32::TRANSPARENT, accent, 0.0), Color32::TRANSPARENT);
        // Halfway is half the alpha, not a quarter of it
        let half = lerp_color(Color32::TRANSPARENT, accent, 0.5);
        assert_eq!(half.a(), 40);
        // Opaque colors blend as usual
        assert_eq!(lerp_color(Color32::BLACK, Color32::WHITE, 0.5), Color32::from_rgb(128, 128, 128));
    }
}

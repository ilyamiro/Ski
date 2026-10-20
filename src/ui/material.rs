//! Flat Material 3 style widgets. Nothing here draws a stroke: shape and state are
//! communicated purely through fill colors (tonal surfaces and state layers).
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, text::LayoutJob, text::TextWrapping, Align2, Color32, CornerRadius, Pos2, Rect, Vec2,
};
use super::theme::SerpantinumTheme;
use crate::i18n::tr;
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
pub const ANIM_FAST: f32 = 0.15;

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

#[derive(Clone, Copy, Debug)]
struct TweenState {
    from: f32,
    to: f32,
    start_time: f64,
    duration: f32,
    last_pass: u64,
}

/// 0 -> 1 (or back) eased with the shared OutQuint curve and duration.
/// Reversing mid-animation starts smoothly from the current visual value without jumping.
pub fn animate(ctx: &egui::Context, id: egui::Id, target: bool) -> f32 {
    animate_for(ctx, id, target, ANIM)
}

pub fn animate_for(ctx: &egui::Context, id: egui::Id, target: bool, seconds: f32) -> f32 {
    animate_value(ctx, id, if target { 1.0 } else { 0.0 }, seconds)
}

/// Smoothly animates an `f32` toward `target` over `seconds` using `quint_out`.
/// Mid-flight target changes continue seamlessly from the current interpolated value, and widgets
/// that were off-screen or inside a closed container snap cleanly on re-appearance.
pub fn animate_value(ctx: &egui::Context, id: egui::Id, target: f32, seconds: f32) -> f32 {
    let dur_s = dur(seconds);
    let pass = ctx.cumulative_pass_nr();
    let (now, dt) = ctx.input(|i| (i.time, i.predicted_dt.clamp(0.001, 0.05) as f64));

    if dur_s <= 0.0 || !target.is_finite() {
        ctx.data_mut(|d| {
            d.insert_temp(
                id,
                TweenState {
                    from: target,
                    to: target,
                    start_time: now,
                    duration: 0.0,
                    last_pass: pass,
                },
            );
        });
        return target;
    }

    let existing = ctx.data(|d| d.get_temp::<TweenState>(id));
    let Some(mut state) = existing.filter(|s| pass <= s.last_pass.saturating_add(2)) else {
        ctx.data_mut(|d| {
            d.insert_temp(
                id,
                TweenState {
                    from: target,
                    to: target,
                    start_time: now,
                    duration: dur_s,
                    last_pass: pass,
                },
            );
        });
        return target;
    };

    let raw_t = if state.duration > 0.0 {
        ((now - state.start_time) as f32 / state.duration).clamp(0.0, 1.0)
    } else {
        1.0
    };

    if (state.to - target).abs() > 1e-5 {
        let prev_time = (now - dt * 0.5).max(state.start_time);
        let prev_t = if state.duration > 0.0 {
            ((prev_time - state.start_time) as f32 / state.duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        state.from = lerp(state.from, state.to, quint_out(prev_t));
        state.to = target;
        state.start_time = prev_time;
        state.duration = dur_s;
    } else if (state.duration - dur_s).abs() > 1e-5 && raw_t < 1.0 {
        state.duration = dur_s;
    }

    let t = if state.duration > 0.0 {
        ((now - state.start_time) as f32 / state.duration).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let mut current = lerp(state.from, state.to, quint_out(t));

    state.last_pass = pass;
    let done = (current - target).abs() <= 1e-4 || ((now - state.start_time) as f32 >= state.duration);
    if done {
        current = target;
        state.from = target;
        state.to = target;
    } else {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, state));
    current
}

/// Resets an animation's current value on this pass so the next `animate` / `animate_value` call
/// transitions smoothly from `value`.
pub fn reset_anim(ctx: &egui::Context, id: egui::Id, value: f32) {
    let pass = ctx.cumulative_pass_nr();
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| {
        d.insert_temp(
            id,
            TweenState {
                from: value,
                to: value,
                start_time: now,
                duration: dur(ANIM),
                last_pass: pass,
            },
        );
    });
}

/// 0 -> 1 entrance progress whenever `id` newly appears on screen (was not rendered in the
/// previous frame), used for view/tab/empty-state transitions.
pub fn animate_enter(ctx: &egui::Context, id: egui::Id, seconds: f32) -> f32 {
    let pass = ctx.cumulative_pass_nr();
    if pass <= 1 || !animations_enabled() {
        reset_anim(ctx, id, 1.0);
        return 1.0;
    }
    let was_visible = ctx
        .data(|d| d.get_temp::<TweenState>(id))
        .is_some_and(|s| pass <= s.last_pass.saturating_add(2));
    if !was_visible {
        reset_anim(ctx, id, 0.0);
    }
    animate_value(ctx, id, 1.0, seconds)
}

pub fn quint_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(5)
}

fn quad_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
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
    pub const TILE: FxSpec = FxSpec { hover_scale: 1.015, press_scale: 0.98, pop_scale: 1.025 };
    pub const ROW: FxSpec = FxSpec { hover_scale: 1.0, press_scale: 1.0, pop_scale: 1.0 };
}

pub fn fx(ui: &egui::Ui, id: egui::Id, resp: &egui::Response, enabled: bool, spec: &FxSpec) -> Fx {
    let ctx = ui.ctx();
    let hover = animate(ctx, id.with("hov"), enabled && resp.hovered());
    let press = animate_for(ctx, id.with("prs"), enabled && resp.is_pointer_button_down_on(), ANIM_FAST);

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
    // Flash: soft ramp over 25ms, then smooth decay over 375ms (OutExpo)
    let flash = if elapsed < 0.025 {
        lerp(0.24, 0.38, quad_out(elapsed / 0.025))
    } else if elapsed < 0.4 {
        0.38 * (1.0 - egui::emath::easing::exponential_out((elapsed - 0.025) / 0.375))
    } else {
        0.0
    };
    if elapsed < 0.55 {
        ctx.request_repaint();
    }

    let base_scale = lerp(lerp(1.0, spec.hover_scale, hover), spec.press_scale, press);
    let scale = base_scale * pop;
    Fx { hover, press, scale, flash }
}

fn flash_color(flash: f32) -> Color32 {
    Color32::from_white_alpha((flash * 255.0 * 0.48).round() as u8)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    // Color32 stores premultiplied channels, and fading between them in that space is what makes
    // a translucent layer fade in and out correctly
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()), mix(a.a(), b.a()))
}

pub(crate) fn blend_over(under: Color32, over: Color32) -> Color32 {
    lerp_color(under, Color32::from_rgb(over.r(), over.g(), over.b()), over.a() as f32 / 255.0)
}

pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
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
pub(crate) fn snap(painter: &egui::Painter, rect: Rect) -> Rect {
    let ppp = painter.ctx().pixels_per_point();
    let round = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_max(Pos2::new(round(rect.min.x), round(rect.min.y)), Pos2::new(round(rect.max.x), round(rect.max.y)))
}

/// Tonal layer for the idle / hover / press states shared by every row.
pub(crate) fn row_layer(theme: &SerpantinumTheme, fx: &Fx) -> Color32 {
    lerp_color(
        lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.text, ALPHA_HOVER), fx.hover),
        theme.with_alpha(theme.text, ALPHA_PRESS),
        fx.press,
    )
}

/// Row background that fades to the accent tint when selected.
pub(crate) fn selectable_bg(theme: &SerpantinumTheme, fx: &Fx, sel_t: f32) -> Color32 {
    let sel_alpha = (ALPHA_SEL_BG as f32 + fx.hover * (ALPHA_SEL_HOVER - ALPHA_SEL_BG) as f32 + fx.press * 12.0).round() as u8;
    lerp_color(row_layer(theme, fx), theme.with_alpha(theme.mauve, sel_alpha), sel_t)
}

pub(crate) fn scaled(rect: Rect, scale: f32) -> Rect {
    if (scale - 1.0).abs() <= 1e-4 { rect } else { Rect::from_center_size(rect.center(), rect.size() * scale) }
}

pub(crate) fn paint_fill(painter: &egui::Painter, rect: Rect, radius: CornerRadius, base: Color32, fx: &Fx) {
    // Snap static edges to whole device pixels, while preserving subpixel smoothness during scale transitions
    let rect = if (fx.scale - 1.0).abs() <= 1e-3 { snap(painter, rect) } else { rect };
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
    let en_t = animate(ui.ctx(), id.with("en"), enabled);
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
    let active_color = lerp_color(theme.subtext1, hover_color, fx.hover.max(fx.press * 0.5));
    let disabled_color = theme.with_alpha(theme.subtext1, 70);
    let color = lerp_color(disabled_color, active_color, en_t);
    paint_icon(ui.painter(), rect.center(), icon, ICON_MD, color);

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
        Some(a) => blend_over(theme.surface1, theme.with_alpha(a, 70 + (fx.press * 30.0) as u8)),
        None => lerp_color(theme.surface2, theme.text, fx.press * 0.08),
    };
    paint_fill(ui.painter(), r, pill(r), lerp_color(idle, hovered, fx.hover.max(fx.press)), &fx);

    let color = lerp_color(theme.subtext1, accent.unwrap_or(theme.text), fx.hover.max(fx.press));
    paint_icon(ui.painter(), rect.center(), icon, (size * 0.5).round(), color);
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

    let idle = lerp_color(
        lerp_color(theme.surface1, theme.surface2, fx.hover),
        lerp_color(theme.surface2, theme.text, 0.08),
        fx.press,
    );
    let active_alpha = (64.0 + fx.hover * 20.0 + fx.press * 16.0).round() as u8;
    let active = blend_over(theme.surface1, theme.with_alpha(theme.mauve, active_alpha));
    paint_fill(ui.painter(), r, pill(r), lerp_color(idle, active, sel_t), &fx);

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, fx.hover), theme.mauve, sel_t);
    let mut x = rect.left() + 12.0;
    if let Some(ic) = icon {
        paint_icon(ui.painter(), Pos2::new(x + 7.0, rect.center().y), ic, ICON_SM, color);
        x += 22.0;
    }
    ui.painter().text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, label, egui::FontId::proportional(FONT_CHIP), color);
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
    let en_t = animate(ui.ctx(), id.with("en"), enabled);
    let r = scaled(rect, fx.scale);

    let (fill, color) = match kind {
        ButtonKind::Filled => (
            lerp_color(theme.mauve, Color32::WHITE, (fx.hover * 0.12 + fx.press * 0.08).min(0.2)),
            on_color(theme, theme.mauve),
        ),
        ButtonKind::Danger => (
            lerp_color(theme.red, Color32::WHITE, (fx.hover * 0.12 + fx.press * 0.08).min(0.2)),
            on_color(theme, theme.red),
        ),
        ButtonKind::Tonal => (
            lerp_color(
                lerp_color(theme.surface1, theme.surface2, fx.hover),
                lerp_color(theme.surface2, theme.text, 0.08),
                fx.press,
            ),
            theme.text,
        ),
        ButtonKind::Text => (
            lerp_color(
                lerp_color(Color32::TRANSPARENT, theme.with_alpha(theme.mauve, 30), fx.hover),
                theme.with_alpha(theme.mauve, 48),
                fx.press,
            ),
            theme.mauve,
        ),
    };
    let fade = lerp(0.4, 1.0, en_t);
    paint_fill(ui.painter(), r, pill(r), fill.gamma_multiply(fade), &fx);

    let color = color.gamma_multiply(fade);
    let mut x = rect.left() + 16.0;
    if let Some(ic) = icon {
        paint_icon(ui.painter(), Pos2::new(x + 8.0, rect.center().y), ic, ICON_SM, color);
        x += 24.0;
    }
    ui.painter().text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, label, font, color);
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

    let inner = snap(ui.painter(), rect.shrink(4.0));
    let mut x = inner.left();
    let mut cells = Vec::with_capacity(options.len());
    for w in &widths {
        cells.push(Rect::from_min_size(Pos2::new(x, inner.top()), Vec2::new(*w, inner.height())));
        x += w;
    }

    // The indicator glides to the selected cell with the same OutQuint curve as the text color
    let target = cells.get(selected).copied().unwrap_or(inner);
    let ctx = ui.ctx();
    // Animated relative to the control, so a popup that moves while it is being placed
    // doesn't make the indicator drift
    let target_rel_x = target.left() - inner.left();
    let target_w = target.width();
    let rel_x = animate_value(ctx, id.with("ix"), target_rel_x, ANIM);
    let iw = animate_value(ctx, id.with("iw"), target_w, ANIM);
    let ix = inner.left() + rel_x;
    let raw_indicator = Rect::from_min_size(Pos2::new(ix, inner.top()), Vec2::new(iw, inner.height()));
    let at_rest = (rel_x - target_rel_x).abs() <= 0.05 && (iw - target_w).abs() <= 0.05;
    let indicator = if at_rest { snap(ui.painter(), raw_indicator) } else { raw_indicator };
    ui.painter().rect_filled(indicator, pill(indicator), blend_over(theme.surface1, theme.with_alpha(theme.mauve, 72)));

    let mut clicked = None;
    for (i, (cell, label)) in cells.iter().zip(options).enumerate() {
        let resp = ui.interact(*cell, id.with(i), egui::Sense::click());
        let hover = animate(ui.ctx(), id.with(("hov", i)), resp.hovered() && i != selected);
        let press = animate_for(ui.ctx(), id.with(("prs", i)), resp.is_pointer_button_down_on() && i != selected, ANIM_FAST);
        let layer_alpha = (hover * ALPHA_HOVER as f32 + press * (ALPHA_PRESS - ALPHA_HOVER) as f32).round() as u8;
        if layer_alpha > 0 {
            ui.painter().rect_filled(snap(ui.painter(), *cell), pill(*cell), theme.with_alpha(theme.text, layer_alpha));
        }
        let on = animate(ui.ctx(), id.with(("on", i)), i == selected);
        let color = lerp_color(lerp_color(theme.subtext0, theme.text, hover.max(press)), theme.mauve, on);
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
    nav_item_with(ui, id_source, icon, label, theme, selected, 0.0)
}

/// Room a round row button (eject) takes at the right end of a sidebar entry.
pub const NAV_TRAILING: f32 = 36.0;

/// `nav_item` that keeps `trailing` pixels at the right edge free of the label.
pub fn nav_item_with(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    theme: &SerpantinumTheme,
    selected: bool,
    trailing: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ENTRY_HEIGHT), egui::Sense::click_and_drag());
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    let sel_t = animate(ui.ctx(), id.with("sel"), selected);
    let r = scaled(rect, fx.scale);

    paint_fill(ui.painter(), r, pill(r), selectable_bg(theme, &fx, sel_t), &fx);

    let color = lerp_color(lerp_color(theme.subtext0, theme.text, fx.hover), theme.mauve, sel_t);
    paint_icon(ui.painter(), Pos2::new(rect.left() + INSET_ICON, rect.center().y), icon, ICON_MD, color);
    paint_label(ui, rect.left() + INSET_LABEL, rect.center().y, label, FONT_BODY, color, rect.width() - INSET_LABEL - 14.0 - trailing);
    response
}

/// Glyph and smoothly interpolated selection color of a file's type icon.
pub fn item_icon_animated(item: &FileItem, theme: &SerpantinumTheme, sel_t: f32) -> (&'static str, Color32) {
    if item.is_dir {
        return (icons::FOLDER, lerp_color(theme.blue, theme.mauve, sel_t));
    }
    if item.is_symlink {
        return (icons::SYMLINK, lerp_color(theme.subtext0, theme.mauve, sel_t));
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
pub const ROW_NAME_X: f32 = 54.0;
const ROW_PAD_RIGHT: f32 = 16.0;
/// Narrowest the name may get before detail columns start dropping out.
const ROW_NAME_MIN: f32 = 160.0;

/// Smallest and largest a column can be dragged to.
const COLUMN_MIN: f32 = 60.0;
const COLUMN_MAX: f32 = 420.0;
/// Per-column widths indexed like `Column::ALL`; 0 means the default.
pub type ColumnWidths = [f32; 6];

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
            Column::Size => tr("Size"),
            Column::Modified => tr("Modified"),
            Column::Created => tr("Created"),
            Column::Accessed => tr("Accessed"),
            Column::Owner => tr("Owner"),
            Column::Permissions => tr("Permissions"),
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    /// Width a column starts with, including the gap to the column on its left.
    pub fn default_width(self) -> f32 {
        match self {
            Column::Size => 92.0,
            Column::Modified | Column::Created | Column::Accessed => 146.0,
            Column::Owner => 110.0,
            Column::Permissions => 116.0,
        }
    }

    /// Slot width as dragged by the user; unset (or invalid) falls back to the default.
    fn width(self, widths: &ColumnWidths) -> f32 {
        let w = widths[self.index()];
        if w.is_finite() && w >= COLUMN_MIN { w.min(COLUMN_MAX) } else { self.default_width() }
    }

    /// The sort order this column header toggles, if the listing can be sorted by it.
    pub fn sort_key(self) -> Option<SortBy> {
        match self {
            Column::Size => Some(SortBy::Size),
            Column::Modified => Some(SortBy::Modified),
            Column::Created => Some(SortBy::Created),
            Column::Accessed => Some(SortBy::Accessed),
            Column::Owner => Some(SortBy::Owner),
            Column::Permissions => Some(SortBy::Permissions),
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
fn fitting_columns(columns: &[Column], widths: &ColumnWidths, width: f32) -> Vec<Column> {
    let budget = width - ROW_NAME_X - ROW_PAD_RIGHT - ROW_NAME_MIN;
    let mut used = 0.0;
    columns
        .iter()
        .copied()
        .take_while(|c| {
            used += c.width(widths);
            used <= budget
        })
        .collect()
}

/// Right edge of every shown column; the block of columns ends at the end of the row.
fn column_edges(fitting: &[Column], widths: &ColumnWidths, row: Rect) -> Vec<(Column, f32)> {
    let mut right = row.right() - ROW_PAD_RIGHT;
    let mut edges: Vec<(Column, f32)> = fitting
        .iter()
        .rev()
        .map(|c| {
            let edge = (*c, right);
            right -= c.width(widths);
            edge
        })
        .collect();
    edges.reverse();
    edges
}

/// Header of the list view: column titles that sort the listing when clicked, and whose
/// left borders can be dragged to resize the column. Returns the sort field that was clicked.
pub fn list_header(
    ui: &mut egui::Ui,
    theme: &SerpantinumTheme,
    columns: &[Column],
    widths: &mut ColumnWidths,
    sort_by: SortBy,
    order: SortOrder,
) -> Option<SortBy> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), egui::Sense::hover());
    let fitting = fitting_columns(columns, widths, rect.width());
    let mut clicked = None;

    let mut cell = |ui: &mut egui::Ui, id: &str, key: Option<SortBy>, label: &str, cell: Rect| {
        let resp = ui.interact(cell, ui.make_persistent_id(("list_header", id)), if key.is_some() { egui::Sense::click() } else { egui::Sense::hover() });
        let hover = animate(ui.ctx(), resp.id.with("h"), key.is_some() && resp.hovered());
        let press = animate_for(ui.ctx(), resp.id.with("p"), key.is_some() && resp.is_pointer_button_down_on(), ANIM_FAST);
        let active = key == Some(sort_by);
        let active_t = animate(ui.ctx(), resp.id.with("act"), active);
        let bg_alpha = (hover * ALPHA_HOVER as f32 + press * (ALPHA_PRESS - ALPHA_HOVER) as f32).round() as u8;
        if bg_alpha > 0 {
            ui.painter().rect_filled(snap(ui.painter(), cell), CornerRadius::same(RADIUS_SM), theme.with_alpha(theme.text, bg_alpha));
        }
        let color = lerp_color(theme.subtext1, theme.text, hover.max(active_t));
        let font = egui::FontId::proportional(FONT_CAPTION);
        let text_rect = ui.painter().text(Pos2::new(cell.left() + 10.0, cell.center().y), Align2::LEFT_CENTER, label, font, color);
        if active_t > 0.01 {
            let arrow = if order == SortOrder::Ascending { icons::SORT_ASC } else { icons::SORT_DESC };
            paint_icon(
                ui.painter(),
                Pos2::new(text_rect.right() + 13.0, cell.center().y),
                arrow,
                ICON_XS,
                theme.mauve.gamma_multiply(active_t),
            );
        }
        if resp.clicked() {
            clicked = key;
        }
    };

    let edges = column_edges(&fitting, widths, rect);
    let name_right = edges.first().map_or(rect.right() - ROW_PAD_RIGHT, |(c, e)| e - c.width(widths));
    let name_cell = Rect::from_min_max(
        Pos2::new(rect.left() + ROW_NAME_X - 10.0, rect.top() + 2.0),
        Pos2::new(name_right - 2.0, rect.bottom() - 2.0),
    );
    cell(ui, "name", Some(SortBy::Name), tr("Name"), name_cell);
    for (column, edge) in edges {
        let width = column.width(widths);
        let slot = Rect::from_min_max(Pos2::new(edge - width + 2.0, rect.top() + 2.0), Pos2::new(edge, rect.bottom() - 2.0));
        cell(ui, column.label(), column.sort_key(), column.label(), slot);

        // Grab strip on the column's left border; dragging it left widens the column
        let grip = Rect::from_x_y_ranges((edge - width - 4.0)..=(edge - width + 4.0), rect.y_range());
        let resp = ui.interact(grip, ui.make_persistent_id(("list_header_grip", column.label())), egui::Sense::drag());
        let grip_active = resp.hovered() || resp.dragged();
        if grip_active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        let grip_t = animate_for(ui.ctx(), resp.id.with("g"), grip_active, ANIM_FAST);
        if grip_t > 0.01 {
            let x = edge - width;
            let half_h = lerp(4.0, (rect.height() - 12.0) * 0.5, grip_t);
            ui.painter().line_segment(
                [Pos2::new(x, rect.center().y - half_h), Pos2::new(x, rect.center().y + half_h)],
                egui::Stroke::new(1.5, theme.mauve.gamma_multiply(grip_t)),
            );
        }
        if resp.dragged() {
            widths[column.index()] = (width - resp.drag_delta().x).clamp(COLUMN_MIN, COLUMN_MAX);
        }
    }
    clicked
}

/// One row of the list view. Columns drop out as the row gets narrower.
/// `path_hint` replaces size/date (used by search results to show where the file lives).
#[allow(clippy::too_many_arguments)]
pub fn file_row(
    ui: &mut egui::Ui,
    _idx: usize,
    item: &FileItem,
    theme: &SerpantinumTheme,
    selected: bool,
    cut: bool,
    should_scroll: bool,
    thumb: Option<&egui::TextureHandle>,
    path_hint: Option<&str>,
    columns: &[Column],
    widths: &ColumnWidths,
    row_height: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), egui::Sense::click());
    let id = ui.make_persistent_id(("row", &item.path));
    let fx = fx(ui, id, &response, true, &FxSpec::ROW);
    let sel_t = animate(ui.ctx(), id.with("sel"), selected);
    let cut_t = animate(ui.ctx(), id.with("cut"), cut);
    let thumb_t = animate(ui.ctx(), id.with("thumb"), thumb.is_some());
    let r = scaled(rect, fx.scale);

    let bg = lerp_color(
        selectable_bg(theme, &fx, sel_t),
        theme.with_alpha(theme.peach, 28),
        cut_t * (1.0 - sel_t * 0.5),
    );
    paint_fill(ui.painter(), r, CornerRadius::same(RADIUS_MD), bg, &fx);

    let fade = lerp(1.0, 0.58, cut_t);
    let icon_center = Pos2::new(r.left() + 28.0, r.center().y);
    if thumb_t < 0.99 {
        let (glyph, color) = item_icon_animated(item, theme, sel_t);
        paint_icon(ui.painter(), icon_center, glyph, 24.0, color.gamma_multiply(fade * (1.0 - thumb_t)));
    }
    if let Some(tex) = thumb {
        ui.scope(|ui| {
            ui.set_opacity(fade * thumb_t);
            paint_thumb(ui, tex, Rect::from_center_size(icon_center, Vec2::splat(32.0)), RADIUS_SM);
        });
    }
    if cut_t > 0.01 {
        paint_cut_badge(ui.painter(), Pos2::new(icon_center.x + 11.0, icon_center.y + 9.0), 16.0, cut_t, theme);
    }

    let mut right = r.right() - ROW_PAD_RIGHT;
    let subtext_color = theme.subtext1.gamma_multiply(fade);
    if let Some(hint) = path_hint {
        let galley = truncated(ui, hint, FONT_CAPTION, subtext_color, (r.width() * 0.4).min(260.0));
        let pos = Pos2::new(right - galley.size().x, r.center().y - galley.size().y / 2.0);
        right = pos.x - 12.0;
        ui.painter().galley(pos, galley, subtext_color);
    } else {
        let fitting = fitting_columns(columns, widths, r.width());
        for (column, edge) in column_edges(&fitting, widths, r) {
            let width = column.width(widths);
            paint_label(ui, edge - width + 10.0, r.center().y, &column.value(item), FONT_CAPTION, subtext_color, width - 14.0);
        }
        right -= fitting.iter().map(|c| c.width(widths)).sum::<f32>();
    }

    if cut_t > 0.01 {
        let pill_w = 54.0;
        let pill_h = 20.0;
        let pill_rect = Rect::from_min_size(Pos2::new(right - pill_w * cut_t, r.center().y - pill_h / 2.0), Vec2::new(pill_w, pill_h));
        let pill_bg = theme.with_alpha(theme.peach, (44.0 * cut_t).round() as u8);
        let pill_fg = theme.peach.gamma_multiply(cut_t);
        ui.painter().rect_filled(snap(ui.painter(), pill_rect), pill(pill_rect), pill_bg);
        paint_icon(ui.painter(), Pos2::new(pill_rect.left() + 13.0, pill_rect.center().y), icons::CUT, 11.0, pill_fg);
        ui.painter().text(Pos2::new(pill_rect.left() + 24.0, pill_rect.center().y), Align2::LEFT_CENTER, tr("Cut"), egui::FontId::proportional(11.5), pill_fg);
        right -= (pill_w + 10.0) * cut_t;
    }

    paint_label(ui, r.left() + ROW_NAME_X, r.center().y, item.display_name(), FONT_SUBTITLE, theme.text.gamma_multiply(fade), right - r.left() - ROW_NAME_X - 6.0);

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

    let color = lerp_color(theme.subtext0, theme.text, fx.hover.max(fx.press));
    paint_icon(ui.painter(), Pos2::new(rect.left() + INSET_ICON, rect.center().y), icon, ICON_MD, color);

    let mut label_room = rect.width() - INSET_LABEL - 14.0;
    if let Some(hint) = hint {
        let galley = ui.painter().layout_no_wrap(hint.to_owned(), egui::FontId::proportional(FONT_CAPTION), theme.subtext1);
        label_room -= galley.size().x + 12.0;
        ui.painter().galley(Pos2::new(rect.right() - 16.0 - galley.size().x, rect.center().y - galley.size().y / 2.0), galley, theme.subtext1);
    }
    paint_label(ui, rect.left() + INSET_LABEL, rect.center().y, label, FONT_BODY, color, label_room);
    response
}

/// One cell of the grid view.
#[allow(clippy::too_many_arguments)]
pub fn file_tile(
    ui: &mut egui::Ui,
    _idx: usize,
    item: &FileItem,
    theme: &SerpantinumTheme,
    selected: bool,
    cut: bool,
    should_scroll: bool,
    thumb: Option<&egui::TextureHandle>,
    grid_size: crate::settings::GridSize,
    max_name_lines: usize,
) -> egui::Response {
    let (target_w, target_h) = grid_size.cell_size();
    let cell_w = animate_value(ui.ctx(), egui::Id::unique("grid_cell_w"), target_w, ANIM);
    let cell_h = animate_value(ui.ctx(), egui::Id::unique("grid_cell_h"), target_h, ANIM);
    let pic_sz = animate_value(ui.ctx(), egui::Id::unique("grid_pic_sz"), grid_size.picture_size(), ANIM);
    let icon_sz = animate_value(ui.ctx(), egui::Id::unique("grid_icon_sz"), grid_size.icon_size(), ANIM);

    let (rect, response) = ui.allocate_exact_size(Vec2::new(cell_w, cell_h), egui::Sense::click());
    let id = ui.make_persistent_id(("tile", &item.path));
    let fx = fx(ui, id, &response, true, &FxSpec::TILE);
    let sel_t = animate(ui.ctx(), id.with("sel"), selected);
    let cut_t = animate(ui.ctx(), id.with("cut"), cut);
    let thumb_t = animate(ui.ctx(), id.with("thumb"), thumb.is_some());
    let r = scaled(rect, fx.scale);

    let bg = lerp_color(
        selectable_bg(theme, &fx, sel_t),
        theme.with_alpha(theme.peach, 28),
        cut_t * (1.0 - sel_t * 0.5),
    );
    paint_fill(ui.painter(), r, CornerRadius::same(RADIUS_LG), bg, &fx);

    let fade = lerp(1.0, 0.58, cut_t);
    // Anchor layout positions to `rect` so multi-line text baselines stay rock-steady during hover/press scaling
    let picture = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + pic_sz * 0.5 + 12.0), Vec2::splat(pic_sz));
    let scaled_picture = scaled(picture, fx.scale);
    if thumb_t < 0.99 {
        let (glyph, color) = item_icon_animated(item, theme, sel_t);
        paint_icon(
            ui.painter(),
            picture.center(),
            glyph,
            icon_sz * fx.scale,
            color.gamma_multiply(fade * (1.0 - thumb_t)),
        );
    }
    if let Some(tex) = thumb {
        ui.scope(|ui| {
            ui.set_opacity(fade * thumb_t);
            paint_thumb(ui, tex, scaled_picture, RADIUS_SM);
        });
    }
    if cut_t > 0.01 {
        paint_cut_badge(ui.painter(), Pos2::new(picture.right() - 6.0, picture.bottom() - 6.0), 22.0, cut_t, theme);
    }

    // Names wrap over the configured lines and are centered under the picture, like Nautilus.
    // With halign Center the galley is centered on x = 0, so it is painted at the cell's center.
    let name_color = theme.text.gamma_multiply(fade);
    let max_text_w = (rect.width() - 14.0).max(16.0);
    let mut job = LayoutJob::simple(item.display_name().to_string(), egui::FontId::proportional(FONT_CAPTION), name_color, max_text_w);
    job.wrap = TextWrapping { max_width: max_text_w, max_rows: max_name_lines, break_anywhere: false, overflow_character: Some('…') };
    job.halign = egui::Align::Center;
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(Pos2::new(rect.center().x, picture.bottom() + 8.0), galley, name_color);

    if selected && should_scroll {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    response
}

/// Flat tonal scissors badge drawn on the corner of a cut item's icon.
fn paint_cut_badge(painter: &egui::Painter, center: Pos2, max_size: f32, cut_t: f32, theme: &SerpantinumTheme) {
    let size = max_size * cut_t;
    let pill_fill = blend_over(theme.surface2, theme.with_alpha(theme.peach, 72)).gamma_multiply(cut_t);
    painter.circle_filled(center, size * 0.5, pill_fill);
    paint_icon(painter, center, icons::CUT, (max_size * 0.56).round() * cut_t.max(0.2), theme.peach.gamma_multiply(cut_t));
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
    let track = snap(ui.painter(), Rect::from_center_size(center, Vec2::new(34.0, 18.0)));
    ui.painter().rect_filled(track, CornerRadius::same(9), lerp_color(theme.surface2, theme.mauve, on));
    let knob_r = lerp(5.5, 6.5, on) + press * 0.8;
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
    let (rect, response, fx) = menu_row_frame(ui, &id_source, theme);
    let t = animate(ui.ctx(), response.id.with("on"), selected);
    let center = Pos2::new(rect.left() + 18.0, rect.center().y);
    let ring = lerp_color(lerp_color(theme.subtext1, theme.text, fx.hover), theme.mauve, t);
    ui.painter().circle_stroke(center, 7.0, egui::Stroke::new(1.5, ring));
    if t > 0.01 {
        ui.painter().circle_filled(center, 3.5 * t, theme.mauve.gamma_multiply(t));
    }
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
    value: &str,
    can_dec: bool,
    can_inc: bool,
) -> (bool, bool) {
    stepper_row_with(ui, id_source, None, label, theme, value, can_dec, can_inc)
}

/// `stepper_row` for settings cards: leading icon, the height and the label edge of the other setting rows.
#[allow(clippy::too_many_arguments)]
pub fn setting_stepper_row(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    label: &str,
    theme: &SerpantinumTheme,
    value: &str,
    can_dec: bool,
    can_inc: bool,
) -> (bool, bool) {
    stepper_row_with(ui, id_source, Some(icon), label, theme, value, can_dec, can_inc)
}

#[allow(clippy::too_many_arguments)]
fn stepper_row_with(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    icon: Option<&str>,
    label: &str,
    theme: &SerpantinumTheme,
    value: &str,
    can_dec: bool,
    can_inc: bool,
) -> (bool, bool) {
    let height = if icon.is_some() { SETTING_HEIGHT } else { MENU_ROW_HEIGHT };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover());
    let id = ui.make_persistent_id(id_source);
    match icon {
        Some(icon) => {
            paint_icon(ui.painter(), Pos2::new(rect.left() + INSET_ICON + 2.0, rect.center().y), icon, ICON_MD, theme.subtext0);
            paint_label(ui, rect.left() + INSET_LABEL + 2.0, rect.center().y, label, FONT_BODY, theme.text, rect.width() - 150.0);
        }
        None => paint_label(ui, rect.left() + 12.0, rect.center().y, label, FONT_BODY, theme.text, rect.width() - 150.0),
    }
    // The current value sits between the two buttons
    let value_galley = ui.painter().layout_no_wrap(value.to_owned(), egui::FontId::proportional(FONT_CAPTION), theme.subtext1);
    ui.painter().galley(
        Pos2::new(rect.right() - 12.0 - 22.0 * 2.0 - 8.0 - value_galley.size().x - 6.0, rect.center().y - value_galley.size().y / 2.0),
        value_galley,
        theme.subtext1,
    );
    let mut out = [false; 2];
    for (i, (icon, enabled)) in [(icons::REMOVE, can_dec), (icons::ADD, can_inc)].into_iter().enumerate() {
        let cell = Rect::from_center_size(Pos2::new(rect.right() - 23.0 - (1 - i) as f32 * 26.0, rect.center().y), Vec2::splat(22.0));
        let btn_id = id.with(i);
        let resp = ui.interact(cell, btn_id, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
        let fx = fx(ui, btn_id, &resp, enabled, &FxSpec::ICON);
        let en_t = animate(ui.ctx(), id.with(("en", i)), enabled);
        let r = scaled(cell, fx.scale);
        let fill = lerp_color(
            lerp_color(theme.surface1, theme.surface2, fx.hover),
            lerp_color(theme.surface2, theme.text, 0.08),
            fx.press,
        );
        paint_fill(ui.painter(), r, pill(r), fill, &fx);
        let active_color = lerp_color(theme.subtext1, theme.text, fx.hover.max(fx.press));
        let color = lerp_color(theme.with_alpha(theme.subtext1, 70), active_color, en_t);
        paint_icon(ui.painter(), cell.center(), icon, ICON_XS, color);
        out[i] = resp.clicked();
    }
    (out[0], out[1])
}

/// Back and forward as one joined pill: two flat halves, dimmed when there is nowhere to go.
pub fn nav_pair(ui: &mut egui::Ui, theme: &SerpantinumTheme, can_back: bool, can_forward: bool) -> (egui::Response, egui::Response) {
    // Same height as every other header control; each half is as wide as a round icon button
    const W: f32 = CONTROL_HEIGHT;
    const H: f32 = CONTROL_HEIGHT;
    let (body, _) = ui.allocate_exact_size(Vec2::new(W * 2.0, H), egui::Sense::hover());
    ui.painter().rect_filled(snap(ui.painter(), body), pill(body), theme.surface0);
    let id = ui.make_persistent_id("nav_pair");
    let mut responses = Vec::with_capacity(2);
    for (i, (icon, enabled, tip)) in [(icons::CHEVRON_LEFT, can_back, tr("Back")), (icons::CHEVRON_RIGHT, can_forward, tr("Forward"))].into_iter().enumerate() {
        let cell = Rect::from_min_size(body.min + Vec2::new(W * i as f32, 0.0), Vec2::new(W, H));
        let btn_id = id.with(i);
        let resp = ui.interact(cell, btn_id, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
        let fx = fx(ui, btn_id, &resp, enabled, &FxSpec::CLICK);
        let en_t = animate(ui.ctx(), id.with(("en", i)), enabled);
        let inner = scaled(cell.shrink(2.0), fx.scale);
        paint_fill(ui.painter(), inner, pill(inner), row_layer(theme, &fx), &fx);
        let active_color = lerp_color(theme.subtext1, theme.text, fx.hover.max(fx.press * 0.5));
        let color = lerp_color(theme.with_alpha(theme.subtext1, 70), active_color, en_t);
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

/// Smoothly expands and collapses a vertical block of widgets when `open` toggles.
pub fn collapsible(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    open: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let id = ui.make_persistent_id(id_source);
    let t = animate(ui.ctx(), id.with("open"), open);
    if t <= 0.0 {
        return;
    }
    if t >= 1.0 {
        let resp = ui.vertical(|ui| {
            add_contents(ui);
        });
        ui.ctx().data_mut(|d| d.insert_temp(id.with("h"), resp.response.rect.height()));
        return;
    }

    let avail_w = ui.available_width();
    let start_pos = ui.cursor().min;
    let prev_h = ui.ctx().data(|d| d.get_temp::<f32>(id.with("h")));
    let clip_rect = match prev_h {
        Some(h) => Rect::from_min_size(start_pos, Vec2::new(avail_w, (h * t).max(1.0))).intersect(ui.clip_rect()),
        None => ui.clip_rect(),
    };

    let slide_y = -(1.0 - t) * 6.0;
    let child_max = Rect::from_min_size(start_pos + Vec2::new(0.0, slide_y), Vec2::new(avail_w, 4000.0));
    let mut builder = egui::UiBuilder::new()
        .max_rect(child_max)
        .layout(egui::Layout::top_down(egui::Align::Min));
    if !open {
        builder = builder.disabled();
    }
    let mut child = ui.new_child(builder);
    child.set_clip_rect(clip_rect);
    child.set_opacity(if prev_h.is_some() { t } else { t * t });
    add_contents(&mut child);

    let natural_h = (child.min_rect().bottom() - child_max.top()).max(0.0);
    ui.ctx().data_mut(|d| d.insert_temp(id.with("h"), natural_h));
    ui.allocate_exact_size(Vec2::new(avail_w, natural_h * t), egui::Sense::hover());
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
    hint_label_inset(ui, text, theme, 4);
}

/// `hint_label` with a chosen left edge, for hints that sit under a text field or a row.
/// Every wrapped line starts at the same x.
pub fn hint_label_inset(ui: &mut egui::Ui, text: &str, theme: &SerpantinumTheme, inset: i8) {
    egui::Frame::new().inner_margin(egui::Margin { left: inset, right: inset, top: 4, bottom: 4 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(egui::RichText::new(text).size(FONT_CAPTION).color(theme.subtext1)).wrap());
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
            if filled_icon_button(ui, close_id, icons::CLOSE, theme, SIZE, tr("Close"), Some(theme.red)).clicked() {
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
            out.cancel = button(ui, (spec.id, "cancel"), None, tr("Cancel"), theme, ButtonKind::Text, true).clicked();
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
    let id = ui.make_persistent_id(("empty_state", icon, title));
    let t = animate_enter(ui.ctx(), id, ANIM);
    let rect = ui.max_rect();
    let center = rect.center() - Vec2::new(0.0, 16.0 - (1.0 - t) * 8.0);
    paint_icon(ui.painter(), center - Vec2::new(0.0, 34.0), icon, 56.0, theme.surface2.gamma_multiply(t));
    ui.painter().text(
        center + Vec2::new(0.0, 12.0),
        Align2::CENTER_CENTER,
        title,
        egui::FontId::proportional(FONT_TITLE),
        theme.subtext0.gamma_multiply(t),
    );
    if let Some(hint) = hint {
        ui.painter().text(
            center + Vec2::new(0.0, 36.0),
            Align2::CENTER_CENTER,
            hint,
            egui::FontId::proportional(FONT_CAPTION),
            theme.subtext1.gamma_multiply(t),
        );
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

/// Paints the rubber-band drag selection rectangle using the flat Material 3 accent tonal fill
/// and a subtle accent border, with an animated opacity factor `t` (0.0..=1.0).
pub fn paint_rubberband_faded(painter: &egui::Painter, rect: Rect, theme: &SerpantinumTheme, t: f32) {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return;
    }
    let rect = snap(painter, rect);
    let radius = CornerRadius::same(RADIUS_MD);
    let fill = theme.with_alpha(theme.mauve, (34.0 * t).round() as u8);
    let stroke = egui::Stroke::new(1.0, theme.with_alpha(theme.mauve, (110.0 * t).round() as u8));
    painter.rect(rect, radius, fill, stroke, egui::StrokeKind::Inside);
}

/// Square tick box. Filled with the accent when ticked.
pub fn checkbox(ui: &mut egui::Ui, id_source: impl std::hash::Hash + std::fmt::Debug, on: bool, enabled: bool, theme: &SerpantinumTheme) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let id = ui.make_persistent_id(id_source);
    let fx = fx(ui, id, &response, enabled, &FxSpec::ICON);
    let t = animate(ui.ctx(), id.with("on"), on);
    let en_t = animate(ui.ctx(), id.with("en"), enabled);
    let r = scaled(Rect::from_center_size(rect.center(), Vec2::splat(24.0)), fx.scale);
    let off = lerp_color(theme.surface2, theme.surface2.gamma_multiply(1.0), fx.hover);
    let hover_layer = row_layer(theme, &fx);
    let fill = lerp_color(blend_over(off, hover_layer), theme.mauve, t);
    paint_fill(ui.painter(), r, CornerRadius::same(RADIUS_SM), fill.gamma_multiply(lerp(0.55, 1.0, en_t)), &fx);
    if t > 0.01 {
        paint_icon(ui.painter(), r.center(), icons::CHECK, 16.0 * (0.6 + 0.4 * t), on_color(theme, theme.mauve).gamma_multiply(t * lerp(0.6, 1.0, en_t)));
    }
    response
}

/// Slim progress bar. `None` slides a short segment along for work whose size is not known.
pub fn progress_bar(ui: &mut egui::Ui, id_source: impl std::hash::Hash + std::fmt::Debug, width: f32, fraction: Option<f32>, theme: &SerpantinumTheme, color: Color32) {
    const HEIGHT: f32 = 6.0;
    let id = ui.make_persistent_id(id_source);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), egui::Sense::hover());
    let track = snap(ui.painter(), rect);
    ui.painter().rect_filled(track, pill(track), theme.surface2);
    match fraction {
        Some(f) => {
            let f = animate_value(ui.ctx(), id, f.clamp(0.0, 1.0), ANIM);
            if f > 0.001 {
                let fill = Rect::from_min_size(rect.min, Vec2::new((rect.width() * f).max(HEIGHT), HEIGHT));
                ui.painter().rect_filled(fill, pill(fill), color);
            }
        }
        None => {
            let time = ui.input(|i| i.time) as f32;
            let phase = (time * 0.8).fract();
            let segment = rect.width() * 0.34;
            let x = lerp(rect.left() - segment, rect.right(), phase);
            let fill = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(segment, HEIGHT));
            let clipped = ui.painter().with_clip_rect(rect);
            clipped.rect_filled(fill, pill(fill), color);
            ui.ctx().request_repaint();
        }
    }
}

/// A ring that fills clockwise from the top; `None` spins a short arc.
pub fn progress_ring(painter: &egui::Painter, center: Pos2, radius: f32, fraction: Option<f32>, track: Color32, color: Color32, time: f64) {
    let stroke_w = 2.6;
    painter.circle_stroke(center, radius, egui::Stroke::new(stroke_w, track));
    let (start, sweep) = match fraction {
        Some(f) => (-std::f32::consts::FRAC_PI_2, f.clamp(0.0, 1.0) * std::f32::consts::TAU),
        None => (time as f32 * 4.2 - std::f32::consts::FRAC_PI_2, std::f32::consts::TAU * 0.28),
    };
    if sweep < 0.02 {
        return;
    }
    let steps = ((sweep / std::f32::consts::TAU) * 72.0).ceil().max(2.0) as usize;
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = start + sweep * i as f32 / steps as f32;
            Pos2::new(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(stroke_w, color)));
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

    #[test]
    fn mid_flight_reversal_is_continuous_without_jumps() {
        set_animations(true);
        let ctx = egui::Context::default();
        let id = egui::Id::unique("test_reversal");

        let step = |time: f64, target: bool| -> f32 {
            let mut raw = egui::RawInput::default();
            raw.time = Some(time);
            raw.predicted_dt = 0.016;
            let mut val = 0.0;
            let mut out = ctx.run_ui(raw, |ui| {
                val = animate(ui.ctx(), id, target);
            });
            out.textures_delta.clear();
            val
        };

        // Frame 0: initial state false -> 0.0
        assert_eq!(step(0.0, false), 0.0);
        // Frame 1..3: animate toward true (0.0 -> ~0.67 at t=50ms)
        let v1 = step(0.016, true);
        let v2 = step(0.032, true);
        let v3 = step(0.050, true);
        assert!(v1 > 0.0 && v2 > v1 && v3 > v2);
        assert!(v3 > 0.5, "OutQuint should pass 0.5 early in the transition, got {v3}");

        // Frame 4: reverse back to false mid-flight!
        // Must descend smoothly from `v3` rather than teleporting down to near 0.0.
        let v4 = step(0.066, false);
        assert!(
            v4 < v3 && v4 > v3 - 0.25,
            "mid-flight reversal should be continuous from {v3}, got {v4}"
        );
    }
}

//! The frame every Ski window is built from: the main window, the file dialog and the Copy to /
//! Move to picker all take their window settings and their panels from here, so a change to how
//! the window looks reaches all of them at once.

use eframe::egui;
use crate::ui::theme::SerpantinumTheme;
use super::{CONTENT_INSET, HEADER_MARGIN, PAGE_MARGIN};

/// Width of the places column on the left.
pub(super) const SIDEBAR_WIDTH: f32 = 212.0;

/// A window without a frame of its own: the header carries the title, the close button and the
/// drag, and the backgrounds may be translucent.
pub fn viewport(title: impl Into<String>) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_title(title)
        // Compositors without server-side decorations (niri) would add a bar
        .with_decorations(false)
        .with_transparent(true)
}

/// Closes the deferred window whose pass this is, without waiting on the window that declares it.
///
/// eframe only drops a deferred window when a pass of its parent no longer declares it, and the
/// parent (the main window) may be on another workspace, where it gets no frame callbacks and
/// runs no pass; Wayland cannot hide a window meanwhile, so it stays up as an empty pane. So the
/// window declares itself as its own child with nothing to draw, and asks for one more pass of
/// its own: that pass does not declare it, and egui drops it at the end like any unused child.
/// If its parent declares it again first (it was opened again), the parent takes it back.
pub(super) fn close_own_viewport(ctx: &egui::Context) {
    // An empty builder: none of the window's settings change, so it is neither altered nor remade
    ctx.show_viewport_deferred(ctx.viewport_id(), egui::ViewportBuilder::default(), |_, _| {});
    ctx.request_repaint();
}

/// Lets go of a deferred window that was closed but is still around.
///
/// A window that closed itself (`close_own_viewport`) is left as its own parent, and egui keeps a
/// window whose parent is alive. A repaint asked for it after it was dropped (its close asks for
/// two) brings it back with that parent, so it then never goes: an empty window stays up, the main
/// window redraws without pause, and closing the main window leaves it hanging. Its parent takes
/// it back for one pass with nothing to draw, and egui drops it on the next as an unused child.
pub(super) fn reap_closed_viewport(ctx: &egui::Context, id: egui::ViewportId) {
    if !ctx.input(|i| i.raw.viewports.contains_key(&id)) {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    let key = egui::Id::unique("ski_reaped_viewport").with(id);
    // Declared every pass it would be kept; every other pass lets it go
    if ctx.data(|d| d.get_temp::<u64>(key)).is_some_and(|last| last + 1 == pass) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(key, pass));
    ctx.show_viewport_deferred(id, egui::ViewportBuilder::default(), |_, _| {});
    ctx.request_repaint();
}

/// The places column, `width` wide while it slides in or out.
pub(super) fn sidebar<R>(ui: &mut egui::Ui, id: &str, width: f32, theme: &SerpantinumTheme, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let panel = egui::Panel::left(id)
        .show_separator_line(false)
        .exact_size(width.max(1.0))
        .resizable(false)
        // The top matches the header's, so the search and menu buttons sit on its centre line
        .frame(egui::Frame::new().inner_margin(egui::Margin { left: PAGE_MARGIN, right: PAGE_MARGIN, top: 4, bottom: 8 }));
    backdrop(ui, theme.mantle, |ui| panel.show(ui, add))
}

/// The row of controls along the top.
pub(super) fn header<R>(ui: &mut egui::Ui, id: &str, theme: &SerpantinumTheme, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let panel = egui::Panel::top(id)
        .show_separator_line(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin { left: HEADER_MARGIN, right: 8, top: 4, bottom: 4 }));
    backdrop(ui, theme.base, |ui| panel.show(ui, add))
}

/// A header on a `fill` of its own, taller than the plain one, for windows whose title needs
/// room above it and a band that sets it apart from the page.
pub(super) fn banner<R>(ui: &mut egui::Ui, id: &str, fill: egui::Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let panel = egui::Panel::top(id)
        .show_separator_line(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin { left: HEADER_MARGIN, right: 8, top: BANNER_TOP, bottom: 6 }));
    backdrop(ui, fill, |ui| panel.show(ui, add))
}

/// Space above a banner's row; whatever sits beside it must start as far down.
pub(super) const BANNER_TOP: i8 = 8;

/// A strip under the header of a fixed `height`, with the header's side margins.
pub(super) fn header_strip<R>(ui: &mut egui::Ui, id: &str, height: f32, theme: &SerpantinumTheme, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let panel = egui::Panel::top(id)
        .show_separator_line(false)
        .exact_size(height.max(1.0))
        .resizable(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(HEADER_MARGIN, 0)));
    backdrop(ui, theme.base, |ui| panel.show(ui, add))
}

/// The bar of a dialog's buttons along the bottom.
pub(super) fn footer<R>(ui: &mut egui::Ui, id: &str, theme: &SerpantinumTheme, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let panel = egui::Panel::bottom(id)
        .show_separator_line(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(CONTENT_INSET, 10)));
    backdrop(ui, theme.mantle, |ui| panel.show(ui, add))
}

/// What is left in the middle: the listing.
pub(super) fn content<R>(ui: &mut egui::Ui, theme: &SerpantinumTheme, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let panel = egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(egui::Margin { left: PAGE_MARGIN, right: PAGE_MARGIN, top: 0, bottom: 6 }));
    let slot = ui.painter().add(egui::Shape::Noop);
    let response = panel.show(ui, add);
    ui.painter().set(slot, flat_rect(ui, response.response.rect, theme.base));
    response
}

/// Paints a panel's background under what `show` draws. A frame's own fill is anti-aliased, so
/// where two translucent panels meet both half-cover the same pixels and the desktop shows
/// through as a seam; these rects are snapped to whole pixels and meet edge to edge instead.
fn backdrop<R>(ui: &mut egui::Ui, fill: egui::Color32, show: impl FnOnce(&mut egui::Ui) -> egui::InnerResponse<R>) -> R {
    let slot = ui.painter().add(egui::Shape::Noop);
    let response = show(ui);
    ui.painter().set(slot, flat_rect(ui, response.response.rect, fill));
    response.inner
}

fn flat_rect(ui: &egui::Ui, rect: egui::Rect, fill: egui::Color32) -> egui::Shape {
    let painter = ui.painter();
    let rect = crate::ui::material::snap(painter, rect);
    let mut mesh = egui::Mesh::default();
    let radius = window_corner_radius(ui.ctx());
    if radius <= 0.0 {
        mesh.add_colored_rect(rect, fill);
        return egui::Shape::mesh(mesh);
    }
    // The corners this backdrop shares with the window are rounded; the inner edges stay flat
    let window = crate::ui::material::snap(painter, ui.ctx().content_rect());
    let near = |a: f32, b: f32| (a - b).abs() < 0.5;
    let round = |x: f32, y: f32| near(x, if x < rect.center().x { window.left() } else { window.right() }) && near(y, if y < rect.center().y { window.top() } else { window.bottom() });
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (rect.left_top(), egui::vec2(1.0, 1.0)),
        (rect.right_top(), egui::vec2(-1.0, 1.0)),
        (rect.right_bottom(), egui::vec2(-1.0, -1.0)),
        (rect.left_bottom(), egui::vec2(1.0, -1.0)),
    ];
    // Outline clockwise from the top left; an arc where the corner is the window's
    let mut outline: Vec<(egui::Pos2, egui::Vec2, Option<usize>)> = Vec::new();
    for (i, (corner, inward)) in corners.into_iter().enumerate() {
        if !round(corner.x, corner.y) {
            outline.push((corner, egui::Vec2::ZERO, None));
            continue;
        }
        let centre = corner + inward * r;
        const STEPS: usize = 12;
        let start = std::f32::consts::PI + i as f32 * std::f32::consts::FRAC_PI_2;
        for step in 0..=STEPS {
            let angle = start + step as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
            let dir = egui::vec2(angle.cos(), angle.sin());
            outline.push((centre + dir * r, dir, Some(i)));
        }
    }
    // A fan from the middle fills the shape without anti-aliasing, so panels still meet edge to edge
    let middle = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.center(), fill);
    for (pos, ..) in &outline {
        mesh.colored_vertex(*pos, fill);
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(middle, middle + 1 + i, middle + 1 + (i + 1) % n);
    }
    // Only the arcs get a soft one-pixel edge, which keeps them smooth
    let feather = 1.0 / ui.ctx().pixels_per_point();
    let clear = egui::Color32::TRANSPARENT;
    for i in 0..n as usize {
        let (a, da, arc) = outline[i];
        let (b, db, next) = outline[(i + 1) % n as usize];
        if arc.is_none() || arc != next {
            continue;
        }
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(a, fill);
        mesh.colored_vertex(b, fill);
        mesh.colored_vertex(a + da * feather, clear);
        mesh.colored_vertex(b + db * feather, clear);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    egui::Shape::mesh(mesh)
}

/// How round the window's own corners are drawn. Ski's window has no frame, so on desktops that do
/// not round windows themselves (GNOME, KDE, most others) its backgrounds are cut round; Hyprland
/// rounds every window already, and a maximized or fullscreen window has square corners.
pub(super) fn window_corner_radius(ctx: &egui::Context) -> f32 {
    static COMPOSITOR_ROUNDS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *COMPOSITOR_ROUNDS.get_or_init(|| std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()) {
        return 0.0;
    }
    let edge_to_edge = ctx.input(|i| i.viewport().maximized == Some(true) || i.viewport().fullscreen == Some(true));
    if edge_to_edge { 0.0 } else { crate::ui::material::radius_window() as f32 }
}

/// Makes the empty part of a header around `rect` move the window, as a title bar would.
pub(super) fn drag_area(ui: &egui::Ui, rect: egui::Rect, id: egui::Id) {
    let drag = ui.interact(rect, id, egui::Sense::click_and_drag());
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

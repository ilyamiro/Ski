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
    let mut mesh = egui::Mesh::default();
    mesh.add_colored_rect(crate::ui::material::snap(ui.painter(), rect), fill);
    egui::Shape::mesh(mesh)
}

/// Makes the empty part of a header around `rect` move the window, as a title bar would.
pub(super) fn drag_area(ui: &egui::Ui, rect: egui::Rect, id: egui::Id) {
    let drag = ui.interact(rect, id, egui::Sense::click_and_drag());
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

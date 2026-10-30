//! Settings in a window of its own, built from the same frame as the main window: the categories
//! down the side like the places, the chosen one's settings in the middle, and the theme editor
//! as a card over them.
use super::*;

/// Window width below which the categories' column is hidden.
const SETTINGS_SIDEBAR_BREAKPOINT: f32 = 600.0;

const SETTINGS_SIZE: egui::Vec2 = egui::vec2(1000.0, 720.0);
const SETTINGS_MIN_SIZE: egui::Vec2 = egui::vec2(420.0, 400.0);

/// Appends a line to `~/.cache/ski/settings.log`, so a run started from anywhere leaves its trace.
pub(crate) fn settings_trace(line: &str) {
    use std::io::Write;
    let Some(dir) = std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache"))) else { return };
    let dir = dir.join("ski");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("settings.log")) {
        let _ = writeln!(file, "[settings] {line}");
    }
}

/// Frame-by-frame trace of the Settings window on stderr, for chasing resize trouble: a line
/// whenever the width or the sidebar's slide changes, and for every frame slower than 30ms.
fn settings_log(ctx: &egui::Context, width: f32, visible: bool, t: f32) {
    use std::sync::Mutex;
    static LAST: Mutex<(f32, f32, u64)> = Mutex::new((0.0, 0.0, 0));
    let pass = ctx.cumulative_pass_nr();
    let (dt, repaints) = ctx.input(|i| (i.unstable_dt, i.viewport().native_pixels_per_point));
    let mut last = LAST.lock().unwrap();
    let frames = last.2 + 1;
    let changed = (last.0 - width).abs() > 0.5 || (last.1 - t).abs() > 0.001;
    if changed || dt > 0.03 || frames % 120 == 0 {
        settings_trace(&format!("frame={frames} pass={pass} width={width:.1} sidebar_visible={visible} sidebar_t={t:.3} dt={:.1}ms ppp={repaints:?} changed={changed}", dt * 1000.0));
    }
    *last = (width, t, frames);
}

/// A pointer back to the app for the redraws of a window of its own.
pub(super) struct AppPtr(*mut SkiApp);

// SAFETY: eframe runs every viewport's UI on the one thread that owns the app, one frame at a
// time and never inside the main window's `update`, so the pointer is never used from two places.
unsafe impl Send for AppPtr {}
unsafe impl Sync for AppPtr {}

impl AppPtr {
    pub(super) fn new(app: &mut SkiApp) -> Self {
        Self(app)
    }

    pub(super) fn get(&self) -> *mut SkiApp {
        self.0
    }
}

impl SkiApp {
    /// Declares the Settings window each frame of the main window. Its contents are drawn by
    /// its own redraws, so that it keeps up when the main window is out of sight (on another
    /// workspace the compositor sends the main window no frame callbacks, and a window drawn
    /// inside the main window's frame would only move when that one did).
    pub(super) fn render_settings_window(&mut self, ctx: &egui::Context) {
        let id = egui::ViewportId::from_hash_of("ski_settings");
        if self.settings_win.is_none() {
            if self.theme_editor {
                self.close_theme_editor();
            }
            // The icon gallery is declared by Settings and goes with it
            self.icon_gallery = None;
            return;
        }
        if self.settings_raise {
            ctx.request_repaint_of(id);
        }
        let builder = chrome::viewport(tr("Settings")).with_inner_size(SETTINGS_SIZE).with_min_inner_size(SETTINGS_MIN_SIZE);
        let app = AppPtr::new(self);
        ctx.show_viewport_deferred(id, builder, move |ui, _class| {
            // SAFETY: see `AppPtr`; the app outlives every window
            let app = unsafe { &mut *app.get() };
            app.settings_viewport_ui(ui);
        });
    }

    fn settings_viewport_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(mut state) = self.settings_win.take() else {
            chrome::close_own_viewport(&ctx);
            return;
        };
        let theme = self.theme;
        let popup_theme = self.popup_theme;
        let mut close = false;
        let raise = std::mem::take(&mut self.settings_raise);
        let fresh = std::mem::take(&mut self.settings_fresh);
        let ctx = ui.ctx().clone();
        material::set_dialog_fresh(&ctx, fresh);
        if raise {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            close = true;
        }
        // Escape peels the layers off one at a time: a color picker, the theme editor, then the window
        if !self.capturing_key && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if !self.theme_editor {
                close = true;
            } else if self.theme_picker.take().is_none() {
                self.close_theme_editor();
            }
        }
        Self::window_resize_edges(&ctx);

        // The fields are single-line, so Up and Down leave them for the next row
        // instead of just jumping the caret to either end of the text
        if !self.capturing_key && ctx.text_edit_focused() {
            let direction = ctx.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    Some(egui::FocusDirection::Down)
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    Some(egui::FocusDirection::Up)
                } else {
                    None
                }
            });
            if let Some(direction) = direction {
                ctx.memory_mut(|m| m.move_focus(direction));
            }
        }

        // The categories slide away as the window narrows, like the places in the main window
        let sidebar_visible = ui.available_width() >= SETTINGS_SIDEBAR_BREAKPOINT;
        let sidebar_t = material::animate(&ctx, egui::Id::unique("set_sidebar_anim"), sidebar_visible);
        settings_log(&ctx, ui.available_width(), sidebar_visible, sidebar_t);
        if sidebar_t > 0.001 {
        chrome::sidebar(ui, "set_categories", chrome::SIDEBAR_WIDTH * sidebar_t, &theme, |ui| {
            ui.set_opacity(sidebar_t);
            ui.add_space((chrome::BANNER_TOP - 4) as f32);
            // The title shares the header's row, its left edge on the rows' icon column
            let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), CONTROL_HEIGHT), egui::Sense::hover());
            chrome::drag_area(ui, row, ui.scope_id().with("set_side_drag"));
            material::paint_label(ui, row.left() + material::INSET_ICON - 15.0, row.center().y, tr("Settings"), material::FONT_TITLE, theme.text, row.width());
            self.settings_sidebar(ui, &mut state);
        });
        }

        chrome::banner(ui, "set_header", theme.mantle, |ui| {
            let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), CONTROL_HEIGHT), egui::Sense::hover());
            // No title bar of its own: the empty parts of the header drag the window
            chrome::drag_area(ui, row.expand2(egui::vec2(HEADER_MARGIN as f32, 4.0)), ui.scope_id().with("set_drag"));
            let left = row.left() - HEADER_MARGIN as f32 + 16.0;
            let right = row.right() + 8.0 - 16.0;
            material::paint_label(ui, left, row.center().y, Self::settings_category_name(&state), material::FONT_TITLE, theme.text, row.width() / 2.0);
            let close_rect = egui::Rect::from_center_size(egui::pos2(right - material::CLOSE_BUTTON / 2.0, row.center().y), egui::vec2(material::CLOSE_BUTTON, material::CLOSE_BUTTON));
            let closed = ui.scope_builder(egui::UiBuilder::new().max_rect(close_rect), |ui| {
                material::filled_icon_button(ui, "set_close", icons::CLOSE, &theme, material::CLOSE_BUTTON, tr("Close"), None).clicked()
            });
            close |= closed.inner;
        });

        chrome::content(ui, &theme, |ui| self.settings_page(ui, &mut state));

        // Menus and cards stay solid however see-through the window is
        self.theme = popup_theme;
        self.render_theme_editor(&ctx);
        self.theme = theme;

        // What was changed here is drawn by the main window, which gets its turn when it can
        if ctx.input(|i| i.pointer.any_released() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::Text(_)))) {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }

        if close {
            self.capturing_key = false;
            self.close_theme_editor();
            self.icon_gallery = None;
            chrome::close_own_viewport(&ctx);
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        } else {
            self.settings_win = Some(state);
            // Declared here rather than by the main window, so it opens and keeps up while the
            // main window is out of sight
            self.render_icon_gallery(&ctx);
        }
    }
}

//! The theme editor: a card over Settings for making a custom theme or changing one. It has the
//! theme's name, a live preview, palettes to start from, and every color with its swatch (which
//! opens a color picker of its own) and its hex code.
use super::*;
use crate::ui::theme::{self, CustomTheme, ThemeColors};
use egui::ecolor::Hsva;
use egui::Color32;

/// Nearly the size of Settings itself, which this opens over.
const CARD_WIDTH: f32 = 820.0;
const CARD_HEIGHT: f32 = 700.0;
const CARD_MARGIN: f32 = 20.0;
const ROW_HEIGHT: f32 = 44.0;
const PICKER_WIDTH: f32 = 208.0;
const PICKER_SQUARE: f32 = 150.0;
const PICKER_BAR: f32 = 14.0;
/// The picker popup's height: its margins, the square, the bar and the footer, and the gaps.
const PICKER_HEIGHT: f32 = 12.0 + PICKER_SQUARE + 10.0 + PICKER_BAR + 10.0 + 24.0 + 12.0;

/// What the editor is working on: a new theme (no id yet) or a saved one, until it is saved.
#[derive(Clone)]
pub(crate) struct ThemeDraft {
    id: Option<String>,
    name: String,
    pub(crate) colors: ThemeColors,
    focus_name: bool,
}

enum Exit {
    Stay,
    Close,
    Save,
    Delete,
}

impl SkiApp {
    /// Opens the editor on a new theme, which starts from the colors in use, or on a saved one.
    pub(crate) fn open_theme_editor(&mut self, id: Option<&str>) {
        let saved = id.and_then(|id| theme::user_themes().into_iter().find(|c| c.id == id));
        self.theme_draft = Some(match saved {
            Some(c) => ThemeDraft { id: Some(c.id), name: c.name, colors: c.colors, focus_name: false },
            None => {
                let current = if self.settings.theme == "system" { self.system_theme } else { SerpantinumTheme::from_settings(&self.settings) };
                ThemeDraft { id: None, name: String::new(), colors: ThemeColors::from_theme(&current), focus_name: true }
            }
        });
        self.theme_picker = None;
        self.theme_editor = true;
        self.rebuild_theme();
    }

    /// Closes the editor and drops what was not saved.
    pub(crate) fn close_theme_editor(&mut self) {
        self.theme_editor = false;
        self.theme_picker = None;
        self.rebuild_theme();
    }

    fn save_theme_draft(&mut self) {
        let Some(draft) = self.theme_draft.clone() else { return };
        let name = draft.name.trim().to_owned();
        if name.is_empty() {
            return;
        }
        let id = draft.id.unwrap_or_else(|| unique_id(&name, &theme::user_themes()));
        theme::save_user_theme(&CustomTheme { id: id.clone(), name, colors: draft.colors });
        theme::reload_user_themes();
        self.settings.theme = format!("custom:{id}");
        self.close_theme_editor();
        self.persist_settings();
    }

    fn delete_theme_draft(&mut self) {
        let Some(id) = self.theme_draft.as_ref().and_then(|d| d.id.clone()) else { return };
        theme::delete_user_theme(&id);
        theme::reload_user_themes();
        if self.settings.theme == format!("custom:{id}") {
            self.settings.theme = "system".to_owned();
        }
        self.close_theme_editor();
        self.persist_settings();
    }

    pub(super) fn render_theme_editor(&mut self, ctx: &egui::Context) {
        if self.theme_editor && !matches!(self.dialog, Some(Dialog::Settings { .. })) {
            self.close_theme_editor();
        }
        let t = material::animate(ctx, egui::Id::unique("theme_editor_anim"), self.theme_editor);
        if !self.theme_editor && t <= 0.0 {
            self.theme_picker = None;
            self.theme_draft = None;
            return;
        }
        let theme = self.theme;
        let screen = ctx.content_rect();
        let open = self.theme_editor;

        // Dims Settings behind the card; a click on it goes back one layer
        let scrim = egui::Area::new(egui::Id::unique("theme_editor_scrim"))
            .order(egui::Order::Tooltip)
            .fixed_pos(screen.min)
            .interactable(open)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(rect, 0, Color32::from_black_alpha((120.0 * t) as u8));
                resp
            });
        if open && scrim.inner.clicked() && self.theme_picker.take().is_none() {
            self.close_theme_editor();
        }

        let width = (screen.width() - 32.0).clamp(240.0, CARD_WIDTH);
        let height = (screen.height() - 48.0).clamp(300.0, CARD_HEIGHT);
        let mut exit = Exit::Stay;
        let card = egui::Area::new(egui::Id::unique("theme_editor_card"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, (1.0 - t) * 14.0))
            .interactable(open)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(theme.mantle)
                    .corner_radius(egui::CornerRadius::same(material::radius_window()))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(CARD_MARGIN as i8))
                    .show(ui, |ui| {
                        let inner = egui::vec2(width - 2.0 * CARD_MARGIN, height - 2.0 * CARD_MARGIN);
                        ui.set_width(inner.x);
                        ui.set_min_height(inner.y);
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
                        exit = self.theme_editor_contents(ui, inner.y);
                    });
            });
        if open {
            // Tab and the arrow keys stay in the card, which a click on the backdrop must not sink under it
            ctx.memory_mut(|m| m.set_modal_layer(card.response.layer_id));
            ctx.move_to_top(card.response.layer_id);
        }
        if open {
            match exit {
                Exit::Stay => {}
                Exit::Close => self.close_theme_editor(),
                Exit::Save => self.save_theme_draft(),
                Exit::Delete => self.delete_theme_draft(),
            }
        }
    }

    /// The card's content, `height` tall. On top: the name, Save and close. Under them, beside each
    /// other: a live preview with the palettes to start from, and every color to edit.
    fn theme_editor_contents(&mut self, ui: &mut egui::Ui, height: f32) -> Exit {
        let theme = self.theme;
        let system = self.system_theme;
        let Some(mut draft) = self.theme_draft.clone() else { return Exit::Stay };
        let top = ui.cursor().top();
        let mut exit = Exit::Stay;

        let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::hover());
        let close_rect = egui::Rect::from_center_size(egui::pos2(row.right() - material::CLOSE_BUTTON / 2.0, row.center().y), egui::vec2(material::CLOSE_BUTTON, material::CLOSE_BUTTON));
        if ui.scope_builder(egui::UiBuilder::new().max_rect(close_rect), |ui| {
            material::filled_icon_button(ui, "theme_editor_close", icons::CLOSE, &theme, material::CLOSE_BUTTON, tr("Close"), None).clicked()
        }).inner {
            exit = Exit::Close;
        }
        let save_label = tr("Save");
        let save_w = ui.painter().layout_no_wrap(save_label.to_owned(), egui::FontId::proportional(material::FONT_BODY), theme.text).size().x + 32.0 + 24.0;
        let save_rect = egui::Rect::from_center_size(egui::pos2(close_rect.left() - 12.0 - save_w / 2.0, row.center().y), egui::vec2(save_w, material::BUTTON_HEIGHT));
        let named = !draft.name.trim().is_empty();
        if ui.scope_builder(egui::UiBuilder::new().max_rect(save_rect), |ui| {
            material::button(ui, "theme_editor_save", Some(icons::CHECK), save_label, &theme, ButtonKind::Filled, named).clicked()
        }).inner {
            exit = Exit::Save;
        }
        // A saved theme can be deleted, after a second click: an icon button right beside Save, red once armed
        let mut name_right = save_rect.left();
        if draft.id.is_some() {
            let armed_id = ui.make_persistent_id("theme_delete_armed");
            let armed = ui.data(|d| d.get_temp::<bool>(armed_id)).unwrap_or(false);
            let size = material::BUTTON_HEIGHT;
            let delete_rect = egui::Rect::from_center_size(egui::pos2(save_rect.left() - 8.0 - size / 2.0, row.center().y), egui::vec2(size, size));
            name_right = delete_rect.left();
            let (tip, tint) = if armed { (tr("Click again to delete"), Some(theme.red)) } else { (tr("Delete"), None) };
            let clicked = ui.scope_builder(egui::UiBuilder::new().max_rect(delete_rect), |ui| {
                material::filled_icon_button(ui, "theme_delete", icons::DELETE_FOREVER, &theme, size, tip, tint).clicked()
            }).inner;
            if clicked {
                if armed {
                    exit = Exit::Delete;
                }
                ui.data_mut(|d| d.insert_temp(armed_id, !armed));
            }
        }
        let name_rect = egui::Rect::from_min_max(row.min, egui::pos2(name_right - 12.0, row.max.y));
        let name = ui
            .scope_builder(egui::UiBuilder::new().max_rect(name_rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                material::input_frame(&theme)
                    .show(ui, |ui| {
                        ui.set_width(name_rect.width() - 30.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.name)
                                .frame(egui::Frame::NONE)
                                .desired_width(f32::INFINITY)
                                .char_limit(40)
                                .hint_text(tr("Theme name"))
                                .font(egui::FontId::proportional(material::FONT_TITLE)),
                        )
                    })
                    .inner
            })
            .inner;
        if std::mem::take(&mut draft.focus_name) {
            name.request_focus();
        }
        if name.lost_focus() && named && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            exit = Exit::Save;
        }

        let mut changed = false;
        let mut picker = self.theme_picker;
        let mut anchor = None;

        let full = ui.available_width();
        let body = (height - (ui.cursor().top() - top) - 4.0).max(120.0);
        let side = (full * 0.4).clamp(200.0, 300.0).min(full * 0.5);
        let origin = ui.cursor().min;
        ui.allocate_rect(egui::Rect::from_min_size(origin, egui::vec2(full, body)), egui::Sense::hover());
        let left = egui::Rect::from_min_size(origin, egui::vec2(side, body));
        let right = egui::Rect::from_min_size(origin + egui::vec2(side + 16.0, 0.0), egui::vec2(full - side - 16.0, body));

        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("theme_editor_side").show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
                material::section_label(ui, tr("Preview"), &theme);
                material::group_card(ui, &theme, |ui| preview(ui, &draft.colors.to_theme()));

                material::section_label(ui, tr("Start from"), &theme);
                if let Some(from) = start_from(ui, &theme, &system, &draft.colors) {
                    draft.colors = from;
                    changed = true;
                }

                ui.add_space(4.0);
                material::group_card(ui, &theme, |ui| {
                    if material::switch_row_with(ui, "custom_light", icons::PALETTE, tr("Light theme"), None, &theme, draft.colors.light).clicked() {
                        draft.colors.light = !draft.colors.light;
                        changed = true;
                    }
                });
            });
        });

        ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("theme_editor_colors").show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
                for (title, range) in [(tr("Backgrounds"), 0..6), (tr("Text"), 6..9), (tr("Accents"), 9..14)] {
                    material::section_label(ui, title, &theme);
                    material::group_card(ui, &theme, |ui| {
                        for (key, label, value) in draft.colors.fields().into_iter().enumerate().filter(|(n, _)| range.contains(n)).map(|(_, f)| f) {
                            let row = color_row(ui, key, tr(label), value, &theme, picker == Some(key));
                            if row.toggled {
                                picker = if picker == Some(key) { None } else { Some(key) };
                            }
                            if picker == Some(key) {
                                anchor = Some(row.swatch);
                            }
                            changed |= row.changed;
                        }
                    });
                }
                ui.add_space(2.0);
            });
        });

        // The picker floats over the card, beside the swatch it belongs to
        if let (Some(key), Some(anchor)) = (picker, anchor) {
            let ctx = ui.ctx().clone();
            if let Some((_, _, value)) = draft.colors.fields().into_iter().find(|(k, ..)| *k == key) {
                let (picked, dismiss) = color_picker_popup(&ctx, &theme, key, anchor, value);
                changed |= picked;
                if dismiss {
                    picker = None;
                }
            }
        }
        self.theme_picker = picker;
        self.theme_draft = Some(draft);
        if changed {
            self.rebuild_theme();
        }
        exit
    }
}

/// A short id for a new theme out of its name, not taken by another.
fn unique_id(name: &str, taken: &[CustomTheme]) -> String {
    let slug: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let slug = slug.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-");
    let base = if slug.is_empty() { "theme".to_owned() } else { slug };
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|c| c.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// A miniature window in the palette being edited: sidebar, list, selected row and the accents.
fn preview(ui: &mut egui::Ui, t: &SerpantinumTheme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 150.0), egui::Sense::hover());
    let painter = ui.painter();
    let round = |r: u8| egui::CornerRadius::same(r);
    painter.rect_filled(rect, round(material::radius_md()), t.base);

    let side = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + rect.width() * 0.3, rect.bottom()));
    painter.rect_filled(side, egui::CornerRadius { nw: material::radius_md(), sw: material::radius_md(), ne: 0, se: 0 }, t.mantle);
    for i in 0..4 {
        let row = egui::Rect::from_min_size(side.min + egui::vec2(8.0, 12.0 + i as f32 * 22.0), egui::vec2(side.width() - 16.0, 16.0));
        if i == 0 {
            painter.rect_filled(row, round(5), t.tint(60));
        }
        let bar = egui::Rect::from_min_size(row.min + egui::vec2(6.0, 5.0), egui::vec2((row.width() - 12.0) * if i == 0 { 0.8 } else { 0.55 }, 6.0));
        painter.rect_filled(bar, round(3), if i == 0 { t.mauve } else { t.subtext1 });
    }

    let list = egui::Rect::from_min_max(egui::pos2(side.right() + 10.0, rect.top() + 12.0), egui::pos2(rect.right() - 10.0, rect.bottom() - 40.0));
    for i in 0..3 {
        let row = egui::Rect::from_min_size(list.min + egui::vec2(0.0, i as f32 * 28.0), egui::vec2(list.width(), 24.0));
        painter.rect_filled(row, round(6), if i == 1 { t.surface2 } else { t.surface0 });
        painter.circle_filled(row.left_center() + egui::vec2(12.0, 0.0), 5.0, [t.mauve, t.peach, t.green][i]);
        let title = egui::Rect::from_min_size(row.min + egui::vec2(26.0, 6.0), egui::vec2(row.width() * 0.4, 5.0));
        painter.rect_filled(title, round(2), t.text);
        let sub = egui::Rect::from_min_size(row.min + egui::vec2(26.0, 14.0), egui::vec2(row.width() * 0.25, 4.0));
        painter.rect_filled(sub, round(2), t.subtext0);
    }

    // The accent as a button, and the status colors
    let button = egui::Rect::from_min_size(egui::pos2(list.left(), rect.bottom() - 32.0), egui::vec2(64.0, 22.0));
    painter.rect_filled(button, round(11), t.mauve);
    let label = egui::Rect::from_center_size(button.center(), egui::vec2(30.0, 5.0));
    painter.rect_filled(label, round(2), t.base);
    for (i, color) in [t.green, t.yellow, t.red].into_iter().enumerate() {
        painter.circle_filled(egui::pos2(list.right() - 8.0 - i as f32 * 18.0, button.center().y), 6.0, color);
    }
}

/// Every palette there is, searchable, to copy into the custom theme; the one that matches the
/// current colors is lit.
fn start_from(ui: &mut egui::Ui, theme: &SerpantinumTheme, system: &SerpantinumTheme, current: &ThemeColors) -> Option<ThemeColors> {
    let memory = ui.make_persistent_id("theme_start_query");
    let mut query = ui.data_mut(|d| d.get_temp::<String>(memory)).unwrap_or_default();
    material::input_frame(theme).show(ui, |ui| {
        ui.horizontal(|ui| {
            material::input_icon(ui, icons::SEARCH, material::ICON_SM, theme.subtext1);
            ui.add(
                egui::TextEdit::singleline(&mut query)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr("Pick a theme to start with"))
                    .font(egui::FontId::proportional(material::FONT_BODY)),
            );
        });
    });
    ui.data_mut(|d| d.insert_temp(memory, query.clone()));

    let needle = query.trim().to_lowercase();
    let mut options: Vec<(String, &'static str, ThemeColors)> = vec![("system".to_owned(), tr("System"), ThemeColors::from_theme(system))];
    options.extend(theme::presets().iter().map(|p| (p.id.to_owned(), tr(p.name), p.colors.clone())));
    options.retain(|(id, label, _)| needle.is_empty() || label.to_lowercase().contains(&needle) || id.contains(&needle));

    let mut picked = None;
    if options.is_empty() {
        material::hint_label(ui, tr("No matching themes"), theme);
        return None;
    }
    material::group_card(ui, theme, |ui| {
        egui::ScrollArea::vertical().max_height(material::ENTRY_HEIGHT * 6.0 + 8.0).auto_shrink([false, true]).id_salt("theme_start_list").show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (id, label, colors) in &options {
                let resp = material::nav_item(ui, ("theme_start", id), "", label, theme, colors == current);
                // Its background with the accent on it, where a row's icon goes
                let t = colors.to_theme();
                let dot = egui::Rect::from_center_size(egui::pos2(resp.rect.left() + material::INSET_ICON, resp.rect.center().y), egui::vec2(20.0, 20.0));
                ui.painter().rect_filled(dot, egui::CornerRadius::same(6), t.base);
                ui.painter().rect_stroke(dot, egui::CornerRadius::same(6), egui::Stroke::new(1.0, theme.text.gamma_multiply(0.18)), egui::StrokeKind::Inside);
                ui.painter().circle_filled(dot.center(), 4.5, t.mauve);
                if resp.clicked() {
                    picked = Some(colors.clone());
                }
            }
        });
    });
    picked
}

struct Row {
    changed: bool,
    toggled: bool,
    swatch: egui::Rect,
}

/// One color: its swatch, its name and its hex code, on a row of fixed height so every row lines up.
fn color_row(ui: &mut egui::Ui, key: &'static str, label: &str, value: &mut String, theme: &SerpantinumTheme, open: bool) -> Row {
    let ctx = ui.ctx().clone();
    let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::hover());
    let mut out = Row { changed: false, toggled: false, swatch: egui::Rect::NOTHING };

    let swatch = egui::Rect::from_center_size(egui::pos2(row.left() + 12.0 + 18.0, row.center().y), egui::vec2(36.0, 26.0));
    let resp = ui.interact(swatch, ui.make_persistent_id(("theme_swatch", key)), egui::Sense::click());
    let lit = material::animate(&ctx, resp.id.with("lit"), open || resp.hovered());
    let color = theme::parse_hex(value).unwrap_or(Color32::GRAY);
    let radius = egui::CornerRadius::same(material::radius_sm());
    ui.painter().rect_filled(swatch, radius, color);
    let edge = material::lerp_color(theme.text.gamma_multiply(0.18), theme.text, lit);
    ui.painter().rect_stroke(swatch, radius, egui::Stroke::new(1.0 + lit, edge), egui::StrokeKind::Inside);
    out.toggled = resp.clicked();
    out.swatch = swatch;

    material::paint_label(ui, swatch.right() + 14.0, row.center().y, label, material::FONT_BODY, theme.text, row.width() - 36.0 - 14.0 - 12.0 - 130.0);

    // The code: what is typed stays as typed until it is a whole color, then the field follows the
    // value again (the picker, a start palette or the file may change it meanwhile)
    let field = egui::Rect::from_min_size(egui::pos2(row.right() - 12.0 - 110.0, row.top()), egui::vec2(110.0, row.height()));
    let typing = ui.make_persistent_id(("theme_hex", key));
    ui.scope_builder(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        material::input_frame(theme).show(ui, |ui| {
            ui.set_width(70.0);
            let mut text = ctx.data(|d| d.get_temp::<String>(typing)).unwrap_or_else(|| value.clone());
            let resp = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .char_limit(7)
                    .hint_text("#rrggbb")
                    .font(egui::FontId::monospace(material::FONT_BODY)),
            );
            if resp.has_focus() {
                ctx.data_mut(|d| d.insert_temp(typing, text.clone()));
            } else {
                ctx.data_mut(|d| d.remove_temp::<String>(typing));
            }
            let typed = text.trim().to_lowercase();
            if resp.changed() && typed.len() == 7 && theme::parse_hex(&typed).is_some() && typed != *value {
                *value = typed;
                out.changed = true;
            }
        });
    });
    out
}

/// The floating color picker: a square for saturation and brightness, a bar for the hue and the
/// code of the result. Returns whether the color changed, and whether a click outside closed it.
fn color_picker_popup(ctx: &egui::Context, theme: &SerpantinumTheme, key: &str, anchor: egui::Rect, value: &mut String) -> (bool, bool) {
    let current = theme::parse_hex(value).unwrap_or(Color32::GRAY);
    let rgb = [current.r(), current.g(), current.b()];
    // The hue is kept between frames: grey and black have none to read back from the code
    let memory = egui::Id::unique("theme_picker_hsva").with(key);
    let mut hsva = ctx.data(|d| d.get_temp::<Hsva>(memory)).filter(|h| h.to_srgb() == rgb).unwrap_or_else(|| Hsva::from_srgb(rgb));

    let screen = ctx.content_rect();
    let width = PICKER_WIDTH + 24.0;
    let below = anchor.bottom() + 6.0 + PICKER_HEIGHT <= screen.bottom() - 8.0;
    let x = anchor.left().min(screen.right() - width - 8.0).max(screen.left() + 8.0);
    let (pos, pivot) = if below {
        (egui::pos2(x, anchor.bottom() + 6.0), egui::Align2::LEFT_TOP)
    } else {
        (egui::pos2(x, anchor.top() - 6.0), egui::Align2::LEFT_BOTTOM)
    };

    let fill = theme.surface1;
    let mut changed = false;
    // Above the card for good: egui raises whichever of two same-level layers was clicked last
    let popup = egui::Area::new(egui::Id::unique("theme_picker_popup")).order(egui::Order::Debug).pivot(pivot).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::new()
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, theme.surface2))
            .corner_radius(egui::CornerRadius::same(material::radius_lg()))
            .shadow(ctx.global_style().visuals.window_shadow)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 10.0);
                changed |= saturation_square(ui, &mut hsva, fill);
                changed |= hue_bar(ui, &mut hsva, fill);
                let [r, g, b] = hsva.to_srgb();
                let shown = Color32::from_rgb(r, g, b);
                let (footer, _) = ui.allocate_exact_size(egui::vec2(PICKER_WIDTH, 24.0), egui::Sense::hover());
                let chip = egui::Rect::from_min_size(footer.min, egui::vec2(36.0, 24.0));
                ui.painter().rect_filled(chip, egui::CornerRadius::same(material::radius_sm()), shown);
                ui.painter().rect_stroke(chip, egui::CornerRadius::same(material::radius_sm()), egui::Stroke::new(1.0, theme.text.gamma_multiply(0.18)), egui::StrokeKind::Inside);
                let galley = ui.painter().layout_no_wrap(theme::to_hex(shown), egui::FontId::monospace(material::FONT_BODY), theme.subtext0);
                ui.painter().galley(egui::pos2(chip.right() + 12.0, footer.center().y - galley.size().y / 2.0), galley, theme.subtext0);
            });
    });

    if changed {
        let [r, g, b] = hsva.to_srgb();
        *value = theme::to_hex(Color32::from_rgb(r, g, b));
    }
    ctx.data_mut(|d| d.insert_temp(memory, hsva));

    let outside = ctx.input(|i| i.pointer.primary_clicked() && i.pointer.interact_pos().is_some_and(|p| !popup.response.rect.contains(p) && !anchor.contains(p)));
    (changed, outside)
}

/// The square: saturation across, brightness down, in the current hue.
fn saturation_square(ui: &mut egui::Ui, hsva: &mut Hsva, backdrop: Color32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(PICKER_WIDTH, PICKER_SQUARE), egui::Sense::click_and_drag());
    let mut changed = false;
    if let Some(p) = resp.interact_pointer_pos() {
        hsva.s = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        hsva.v = 1.0 - ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        changed = true;
    }
    let [r, g, b] = Hsva::new(hsva.h, 1.0, 1.0, 1.0).to_srgb();
    let hue = Color32::from_rgb(r, g, b);
    let painter = ui.painter();
    gradient(painter, rect, [Color32::WHITE, hue, hue, Color32::WHITE]);
    gradient(painter, rect, [Color32::TRANSPARENT, Color32::TRANSPARENT, Color32::BLACK, Color32::BLACK]);
    round_off(painter, rect, f32::from(material::radius_md()), backdrop);

    // A ring in two tones reads on any color under it
    let at = egui::pos2(rect.left() + hsva.s * rect.width(), rect.top() + (1.0 - hsva.v) * rect.height());
    let [r, g, b] = hsva.to_srgb();
    painter.circle_filled(at, 7.0, Color32::from_rgb(r, g, b));
    painter.circle_stroke(at, 7.0, egui::Stroke::new(2.0, Color32::WHITE));
    painter.circle_stroke(at, 8.5, egui::Stroke::new(1.0, Color32::from_black_alpha(90)));
    changed
}

/// The bar under it: the hue, red through the rainbow and back to red.
fn hue_bar(ui: &mut egui::Ui, hsva: &mut Hsva, backdrop: Color32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(PICKER_WIDTH, PICKER_BAR), egui::Sense::click_and_drag());
    let mut changed = false;
    if let Some(p) = resp.interact_pointer_pos() {
        hsva.h = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        changed = true;
    }
    let painter = ui.painter();
    let mut mesh = egui::Mesh::default();
    const STEPS: u32 = 6;
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let [r, g, b] = Hsva::new(t, 1.0, 1.0, 1.0).to_srgb();
        let color = Color32::from_rgb(r, g, b);
        let x = rect.left() + t * rect.width();
        mesh.colored_vertex(egui::pos2(x, rect.top()), color);
        mesh.colored_vertex(egui::pos2(x, rect.bottom()), color);
        if i > 0 {
            let k = i * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    round_off(painter, rect, PICKER_BAR / 2.0, backdrop);

    let at = egui::pos2(rect.left() + hsva.h * rect.width(), rect.center().y);
    let [r, g, b] = Hsva::new(hsva.h, 1.0, 1.0, 1.0).to_srgb();
    painter.circle_filled(at, PICKER_BAR / 2.0 + 2.0, Color32::from_rgb(r, g, b));
    painter.circle_stroke(at, PICKER_BAR / 2.0 + 2.0, egui::Stroke::new(2.0, Color32::WHITE));
    painter.circle_stroke(at, PICKER_BAR / 2.0 + 3.5, egui::Stroke::new(1.0, Color32::from_black_alpha(90)));
    changed
}

/// A rectangle blending between the colors of its corners: top left, top right, bottom right, bottom left.
fn gradient(painter: &egui::Painter, rect: egui::Rect, colors: [Color32; 4]) {
    let mut mesh = egui::Mesh::default();
    for (pos, color) in [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].into_iter().zip(colors) {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Rounds the corners of whatever was just drawn in `rect` by covering what lies outside the arcs
/// with the color behind it; a mesh cannot be clipped to a rounded shape.
fn round_off(painter: &egui::Painter, rect: egui::Rect, radius: f32, backdrop: Color32) {
    const SEGMENTS: u32 = 10;
    let corners = [(rect.left_top(), 1.0, 1.0), (rect.right_top(), -1.0, 1.0), (rect.right_bottom(), -1.0, -1.0), (rect.left_bottom(), 1.0, -1.0)];
    for (corner, sx, sy) in corners {
        let center = corner + egui::vec2(sx * radius, sy * radius);
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(corner, backdrop);
        for k in 0..=SEGMENTS {
            let angle = k as f32 / SEGMENTS as f32 * std::f32::consts::FRAC_PI_2;
            mesh.colored_vertex(center + egui::vec2(-sx * radius * angle.cos(), -sy * radius * angle.sin()), backdrop);
        }
        for k in 0..SEGMENTS {
            mesh.add_triangle(0, k + 1, k + 2);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_come_from_names_and_stay_unique() {
        let taken = |id: &str| CustomTheme { id: id.to_owned(), name: String::new(), colors: ThemeColors::default() };
        assert_eq!(unique_id("My Theme!", &[]), "my-theme");
        assert_eq!(unique_id("???", &[]), "theme");
        assert_eq!(unique_id("Mine", &[taken("mine"), taken("mine-2")]), "mine-3");
    }
}

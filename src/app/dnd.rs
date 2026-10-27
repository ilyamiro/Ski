//! Dragging files inside the window: pick up the selection by an item's icon or name, drop it on a
//! folder, a sidebar place, a breadcrumb or a tab. Moves by default, copies with Ctrl. Files
//! dropped into the folder on show are selected where the sort puts them, and scrolled to.
use super::*;
use std::time::{Duration, Instant};

/// How far the pointer travels before a press on an item becomes a drag.
const START_DISTANCE: f32 = 6.0;
/// How long files dropped into the folder on show are waited for, to select them.
const ARRIVAL_WAIT: Duration = Duration::from_secs(60);
/// Side of a file's card in the picture that follows the pointer, and the shift of each behind it.
const CARD: f32 = 32.0;
const CARD_STEP: f32 = 4.0;

pub(super) struct Dnd {
    items: Vec<PathBuf>,
    /// The first few items, the one picked up first, for the picture that follows the pointer.
    looks: Vec<FileItem>,
    start: egui::Pos2,
    pub(super) active: bool,
}

/// Files on their way into the folder on show.
pub(super) struct Arrivals {
    folder: PathBuf,
    paths: HashSet<PathBuf>,
    /// The selection as it was last set (at the drop, then to the files that arrived); once the
    /// user picks something else, arrivals leave it alone.
    shown: HashSet<PathBuf>,
    until: Instant,
}

/// A spot of the window that takes a drop.
pub(super) struct DropZone {
    pub rect: egui::Rect,
    pub dest: PathBuf,
}

impl SkiApp {
    /// Part of an item that picks it up: the icon and name of a row, or a whole grid tile.
    fn drag_grip(&self, ui: &egui::Ui, idx: usize, rect: egui::Rect) -> egui::Rect {
        if self.view_mode == ViewMode::Grid {
            return rect;
        }
        let name = self.items.get(idx).map(|i| i.name.clone()).unwrap_or_default();
        let width = ui.painter().layout_no_wrap(name, egui::FontId::proportional(material::FONT_BODY), self.theme.text).size().x;
        egui::Rect::from_min_max(rect.min, egui::pos2((rect.left() + material::ROW_NAME_X + width + 8.0).min(rect.right()), rect.bottom()))
    }

    /// A press on the grip of an item starts a drag of the selection; true when it did.
    pub(super) fn begin_item_drag(&mut self, ui: &egui::Ui, press: egui::Pos2, item_rects: &[(usize, egui::Rect)]) -> bool {
        if self.in_trash() || self.picking() {
            return false;
        }
        let Some(&(idx, _)) = item_rects.iter().find(|(idx, rect)| self.drag_grip(ui, *idx, *rect).contains(press)) else { return false };
        if !self.selection.contains(&idx) {
            self.selection = BTreeSet::from([idx]);
            self.selected_index = Some(idx);
            self.anchor = Some(idx);
            self.last_cursor = Some(idx);
        }
        let items: Vec<PathBuf> = self.selection.iter().filter_map(|i| self.items.get(*i)).map(|i| i.path.clone()).collect();
        let looks = std::iter::once(idx).chain(self.selection.iter().copied().filter(|i| *i != idx)).take(3).filter_map(|i| self.items.get(i).cloned()).collect();
        self.dnd = Some(Dnd { items, looks, start: press, active: false });
        true
    }

    /// An active drag keeps the click that ends it from selecting or opening anything.
    pub(super) fn item_drag_active(&self) -> bool {
        self.dnd.as_ref().is_some_and(|d| d.active)
    }

    fn drop_target(&self, pos: egui::Pos2, items: &[PathBuf]) -> Option<(PathBuf, egui::Rect)> {
        let view = self.dnd_view;
        let listing = self.last_item_rects.iter().filter(|(idx, rect)| {
            self.items.get(*idx).is_some_and(|i| i.is_dir && !items.contains(&i.path)) && rect.intersect(view).contains(pos)
        });
        let folders = listing.map(|(idx, rect)| (self.items[*idx].path.clone(), *rect));
        let zones = self.drop_zones.iter().rev().map(|z| (z.dest.clone(), z.rect));
        let (dest, rect) = folders.chain(zones).find(|(dest, rect)| rect.contains(pos) && self.accepts(dest, items))?;
        Some((dest, rect))
    }

    /// Dropping is pointless on the folder the items already sit in, and impossible into themselves.
    fn accepts(&self, dest: &Path, items: &[PathBuf]) -> bool {
        if trash::is_trash_folder(dest) {
            return true;
        }
        if dest.as_os_str() == RECENT_PATH || !dest.is_dir() || trash::contains(dest) {
            return false;
        }
        !items.iter().all(|i| i.parent() == Some(dest)) && !items.iter().any(|i| dest.starts_with(i))
    }

    /// Follow the pointer, draw the chip and the highlighted target, and carry out the drop.
    pub(super) fn update_item_drag(&mut self, ctx: &egui::Context) {
        let Some(dnd) = &mut self.dnd else { return };
        let (pos, down, released, copy, escape) = ctx.input(|i| {
            let button = egui::PointerButton::Primary;
            (i.pointer.latest_pos(), i.pointer.button_down(button), i.pointer.button_released(button), i.modifiers.command, i.key_pressed(egui::Key::Escape))
        });
        let Some(pos) = pos.filter(|_| (down || released) && !escape) else {
            self.dnd = None;
            return;
        };
        if !dnd.active && pos.distance(dnd.start) >= START_DISTANCE {
            dnd.active = true;
        }
        if !dnd.active {
            return;
        }
        let items = dnd.items.clone();
        let looks = dnd.looks.clone();
        // Past the window's edge the drag goes on as a real one, which other windows can take
        if !ctx.content_rect().shrink(1.0).contains(pos) {
            let argb = |c: egui::Color32| 0xFF << 24 | (c.r() as u32) << 16 | (c.g() as u32) << 8 | c.b() as u32;
            let style = crate::wayland_dnd::IconStyle {
                card: argb(self.theme.surface2),
                badge: argb(self.theme.mauve),
                badge_text: argb(self.theme.base),
                icons: looks.iter().map(|i| material::item_icon_animated(i, &self.theme, 0.0)).map(|(glyph, color)| (glyph, argb(color))).collect(),
                scale: ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0).ceil().max(1.0) as i32,
            };
            if let Some(wl) = &mut self.wl_dnd && wl.start_drag(&items, style) {
                self.dnd = None;
                return;
            }
        }
        let target = self.drop_target(pos, &items);
        let theme = self.theme;
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        ctx.request_repaint();

        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("item_drag"));
        let painter = ctx.layer_painter(layer);
        if let Some((_, rect)) = &target {
            painter.rect_filled(*rect, egui::CornerRadius::same(material::radius_md()), theme.tint(34));
        }

        let what = Self::describe_count(&items);
        let line = match &target {
            Some((dest, _)) if trash::is_trash_folder(dest) => trf("Move {} to the trash", &[&what]),
            Some((dest, _)) => {
                let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tr("Computer").to_owned());
                if copy { trf("Copy {} to “{}”", &[&what, &name]) } else { trf("Move {} to “{}”", &[&what, &name]) }
            }
            None => what,
        };
        let stack = self.paint_drag_stack(&painter, pos + egui::vec2(12.0, 12.0), &looks, items.len());
        let galley = painter.layout_no_wrap(line, egui::FontId::proportional(material::FONT_BODY), theme.text);
        let chip_size = galley.size() + egui::vec2(36.0, 16.0);
        let chip = egui::Rect::from_min_size(egui::pos2(stack.right() + 6.0, stack.center().y - chip_size.y / 2.0), chip_size);
        painter.rect_filled(chip.translate(egui::vec2(0.0, 2.0)), material::pill(chip), egui::Color32::from_black_alpha(60));
        painter.rect_filled(chip, material::pill(chip), theme.surface2);
        let icon = if target.as_ref().is_some_and(|(d, _)| trash::is_trash_folder(d)) { icons::TRASH } else if copy { icons::COPY } else { icons::FOLDER_MOVE };
        material::paint_icon(&painter, egui::pos2(chip.left() + 16.0, chip.center().y), icon, material::ICON_SM, theme.mauve);
        painter.galley(egui::pos2(chip.left() + 30.0, chip.center().y - galley.size().y / 2.0), galley, theme.text);

        if released {
            self.dnd = None;
            if let Some((dest, _)) = target {
                self.drop_items(items, dest, copy);
            }
        }
    }

    /// Where a drop at `pos` lands: a folder or place under it, else the folder being shown.
    fn system_drop_target(&self, pos: egui::Pos2, items: &[PathBuf]) -> Option<(PathBuf, Option<egui::Rect>)> {
        if let Some((dest, rect)) = self.drop_target(pos, items) {
            return Some((dest, Some(rect)));
        }
        (self.dnd_view.contains(pos) && self.can_modify_here() && self.accepts(&self.current_path, items)).then(|| (self.current_path.clone(), None))
    }

    /// Drags coming from other windows (or back from this one): highlight the target under the
    /// pointer and carry out the drop.
    pub(super) fn update_system_dnd(&mut self, ctx: &egui::Context) {
        let Some(wl) = &mut self.wl_dnd else { return };
        let poll = wl.poll();
        let scale = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0) / ctx.pixels_per_point();
        let to_ui = |(x, y): (f64, f64)| egui::pos2(x as f32 * scale, y as f32 * scale);
        if poll.busy || poll.hover.is_some() {
            ctx.request_repaint();
        }
        if let Some(hover) = poll.hover {
            let target = self.system_drop_target(to_ui(hover.pos), &hover.paths);
            // The files are not known until they have been read; until then the spot counts as fine
            let ok = target.is_some() || hover.paths.is_empty();
            if let Some(wl) = &mut self.wl_dnd {
                wl.set_accepting(ok);
            }
            if let Some((_, Some(rect))) = target {
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("item_drag")));
                painter.rect_filled(rect, egui::CornerRadius::same(material::radius_md()), self.theme.tint(34));
            }
        }
        if let Some(drop) = poll.dropped {
            if let Some((dest, _)) = self.system_drop_target(to_ui(drop.pos), &drop.paths) {
                self.drop_items(drop.paths, dest, drop.copy);
            }
        }
    }

    /// The files being dragged as a small pile of cards, each with its preview or type icon, and
    /// their number on a badge when there are several. Returns the space the pile takes.
    fn paint_drag_stack(&self, painter: &egui::Painter, origin: egui::Pos2, looks: &[FileItem], count: usize) -> egui::Rect {
        let theme = &self.theme;
        let layers = looks.len().clamp(1, 3);
        let reach = CARD + CARD_STEP * (layers - 1) as f32;
        let area = egui::Rect::from_min_size(origin, egui::vec2(reach, reach));
        let radius = egui::CornerRadius::same(material::radius_md());
        // The front card sits at the bottom left, those behind it up and to the right
        for layer in (0..layers).rev() {
            let step = CARD_STEP * layer as f32;
            let card = egui::Rect::from_min_size(egui::pos2(area.left() + step, area.bottom() - CARD - step), egui::vec2(CARD, CARD));
            painter.rect_filled(card.translate(egui::vec2(0.0, 1.5)), radius, egui::Color32::from_black_alpha(60));
            painter.rect_filled(card, radius, theme.surface2);
            painter.rect_stroke(card, radius, egui::Stroke::new(1.0, theme.with_alpha(theme.base, 0xC0)), egui::StrokeKind::Inside);
            let Some(item) = looks.get(layer) else { continue };
            match thumb_for(&self.thumbs, &self.folder_icons, item, self.settings.show_thumbnails) {
                Some(tex) => {
                    let size = tex.size_vec2();
                    let room = CARD - 6.0;
                    let shown = egui::Rect::from_center_size(card.center(), size * (room / size.x).min(room / size.y));
                    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                    painter.image(tex.id(), shown, uv, egui::Color32::WHITE);
                }
                None => {
                    let (glyph, color) = material::item_icon_animated(item, theme, 0.0);
                    material::paint_icon(painter, card.center(), glyph, 20.0, color);
                }
            }
        }
        if count > 1 {
            let label = if count > 99 { "99+".to_owned() } else { count.to_string() };
            let galley = painter.layout_no_wrap(label, egui::FontId::proportional(material::FONT_CAPTION), theme.base);
            let size = egui::vec2((galley.size().x + 10.0).max(18.0), 18.0);
            let badge = egui::Rect::from_center_size(egui::pos2(area.right() - 2.0, area.top() + 2.0), size);
            painter.rect_filled(badge, material::pill(badge), theme.mauve);
            painter.galley(badge.center() - galley.size() / 2.0, galley, theme.base);
        }
        area
    }

    fn drop_items(&mut self, items: Vec<PathBuf>, dest: PathBuf, copy: bool) {
        if dest == self.current_path && !trash::is_trash_folder(&dest) {
            let paths = items.iter().filter_map(|p| p.file_name()).map(|n| dest.join(n)).collect();
            let shown = self.selected_paths().into_iter().collect();
            self.arrivals = Some(Arrivals { folder: dest.clone(), paths, shown, until: Instant::now() + ARRIVAL_WAIT });
        }
        if trash::is_trash_folder(&dest) {
            self.trash_paths(items);
        } else if copy {
            self.transfer(TransferOp::Copy, items, dest);
        } else {
            self.transfer(TransferOp::Move, items, dest);
        }
    }

    /// The listing changed: select the dropped files that are in it now, and scroll to them.
    pub(super) fn reveal_arrivals(&mut self) {
        let Some(arrivals) = &self.arrivals else { return };
        let selected: HashSet<PathBuf> = self.selected_paths().into_iter().collect();
        if arrivals.folder != self.current_path || Instant::now() > arrivals.until || selected != arrivals.shown {
            self.arrivals = None;
            return;
        }
        let found: BTreeSet<usize> = self.items.iter().enumerate().filter(|(_, i)| arrivals.paths.contains(&i.path)).map(|(n, _)| n).collect();
        if found.is_empty() {
            return;
        }
        let complete = found.len() == arrivals.paths.len();
        self.selection = found;
        self.selected_index = self.selection.iter().next().copied();
        self.anchor = self.selected_index;
        self.last_cursor = self.selected_index;
        self.scroll_to_selected = true;
        if complete {
            self.arrivals = None;
        } else {
            let shown = self.selected_paths().into_iter().collect();
            if let Some(arrivals) = &mut self.arrivals {
                arrivals.shown = shown;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use crate::ui::theme::SerpantinumTheme;

    #[test]
    fn drops_need_a_different_folder_that_is_not_inside_the_items() {
        let base = std::env::temp_dir().join(format!("ski_dnd_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("a/inner")).unwrap();
        std::fs::create_dir_all(base.join("b")).unwrap();
        std::fs::write(base.join("a/file.txt"), "x").unwrap();

        let ctx = egui::Context::default();
        let mut app = SkiApp::new(base.clone(), SerpantinumTheme::default(), Settings::default(), &ctx);
        let items = vec![base.join("a/file.txt")];
        assert!(app.accepts(&base.join("b"), &items));
        assert!(!app.accepts(&base.join("a"), &items), "already there");
        assert!(!app.accepts(&base.join("a/file.txt"), &items), "not a folder");
        let folder = vec![base.join("a")];
        assert!(!app.accepts(&base.join("a/inner"), &folder), "into itself");
        assert!(app.accepts(&base.join("b"), &folder));

        let zone = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 30.0));
        app.drop_zones.push(DropZone { rect: zone, dest: base.join("b") });
        assert_eq!(app.drop_target(egui::pos2(10.0, 10.0), &items).map(|t| t.0), Some(base.join("b")));
        assert!(app.drop_target(egui::pos2(500.0, 10.0), &items).is_none());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// Files dropped into the folder on show are selected where the sort puts them, unless the
    /// user picked something else in the meantime.
    #[test]
    fn dropped_files_are_selected_once_they_arrive() {
        let base = std::env::temp_dir().join(format!("ski_dnd_arrive_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (here, there) = (base.join("here"), base.join("there"));
        std::fs::create_dir_all(&here).unwrap();
        std::fs::create_dir_all(&there).unwrap();
        for name in ["a.txt", "z.txt"] {
            std::fs::write(here.join(name), "x").unwrap();
        }
        std::fs::write(there.join("m.txt"), "x").unwrap();
        std::fs::write(there.join("n.txt"), "x").unwrap();

        let ctx = egui::Context::default();
        let mut app = SkiApp::new(here.clone(), SerpantinumTheme::default(), Settings::default(), &ctx);
        app.reload_items();
        app.select_path(&here.join("a.txt"));
        let wait = |paths: &[&str], app: &SkiApp| Arrivals {
            folder: here.clone(),
            paths: paths.iter().map(|n| here.join(n)).collect(),
            shown: app.selected_paths().into_iter().collect(),
            until: Instant::now() + ARRIVAL_WAIT,
        };
        app.arrivals = Some(wait(&["m.txt"], &app));
        app.scroll_to_selected = false;
        std::fs::rename(there.join("m.txt"), here.join("m.txt")).unwrap();
        app.reload_items();
        assert_eq!(app.selected_paths(), vec![here.join("m.txt")]);
        assert_eq!(app.selected_index, Some(1), "sorted between a and z");
        assert!(app.scroll_to_selected && app.arrivals.is_none());

        // A click elsewhere before the file turns up keeps the user's choice
        app.arrivals = Some(wait(&["n.txt"], &app));
        app.select_path(&here.join("z.txt"));
        std::fs::rename(there.join("n.txt"), here.join("n.txt")).unwrap();
        app.keep_selection_through_reload();
        app.reload_items();
        assert_eq!(app.selected_paths(), vec![here.join("z.txt")]);
        assert!(app.arrivals.is_none());
        std::fs::remove_dir_all(&base).unwrap();
    }
}

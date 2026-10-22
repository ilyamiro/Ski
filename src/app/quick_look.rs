//! Quick Look: Space opens a floating preview of the selected file. Text and code are read from
//! disk, pictures and documents reuse the thumbnail, folders list their size. Up and Down (or
//! `j` and `k`) move to the neighbouring file without closing it; Space or Escape closes.
use std::io::Read;
use std::path::Path;

use eframe::egui;

use super::{Dialog, SkiApp};
use crate::fs::file_item::{FileItem, format_bytes};
use crate::fs::props::format_datetime;
use crate::i18n::{tr, trf, trn};
use crate::ui::{icons, material};

/// Text previews read this much of a file and show this many lines, so a huge log opens at once.
const MAX_BYTES: usize = 128 * 1024;
const MAX_LINES: usize = 600;

pub struct Preview {
    pub body: Body,
}

pub enum Body {
    Text { text: String, numbers: String, lines: usize, words: usize, cut: bool },
    /// A picture or a document page, drawn from the thumbnail; the size is the original's, if known.
    Picture { dimensions: Option<(u32, u32)> },
    Nothing,
}

impl Preview {
    pub fn load(item: &FileItem) -> Self {
        let body = if crate::thumbs::is_image(&item.path) || crate::thumbs::has_preview(&item.path) {
            Body::Picture { dimensions: image::image_dimensions(&item.path).ok() }
        } else {
            read_text(&item.path).unwrap_or(Body::Nothing)
        };
        Self { body }
    }
}

/// The start of a file that is text; `None` when it holds binary data or cannot be read.
fn read_text(path: &Path) -> Option<Body> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity(MAX_BYTES);
    file.by_ref().take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).ok()?;
    let mut cut = bytes.len() > MAX_BYTES;
    bytes.truncate(MAX_BYTES);
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return None;
    }
    let full = String::from_utf8_lossy(&bytes);
    let words = full.split_whitespace().count();
    let all_lines: Vec<&str> = full.lines().collect();
    let lines = all_lines.len();
    cut |= lines > MAX_LINES;
    let shown = &all_lines[..lines.min(MAX_LINES)];
    let numbers = (1..=shown.len()).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
    Some(Body::Text { text: shown.join("\n").replace('\t', "    "), numbers, lines, words, cut })
}

impl SkiApp {
    pub(crate) fn open_quick_look(&mut self) {
        let Some(item) = self.selected_index.and_then(|i| self.items.get(i)).filter(|i| !i.is_dir).cloned() else { return };
        self.thumbs.ensure(&item.path);
        let preview = Box::new(Preview::load(&item));
        self.dialog = Some(Dialog::QuickLook { item: Box::new(item), preview });
    }

    pub(super) fn quick_look_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::QuickLook { item, preview } = dialog else { return false };
        let ctx = ui.ctx().clone();
        let theme = self.theme;
        let mut close = material::dialog_header(ui, &item.name, None, &theme, "ql_close");

        let (step, space) = ctx.input(|i| {
            let down = i.key_pressed(egui::Key::ArrowDown) || (self.settings.vim_keys && i.key_pressed(egui::Key::J));
            let up = i.key_pressed(egui::Key::ArrowUp) || (self.settings.vim_keys && i.key_pressed(egui::Key::K));
            (down as isize - up as isize, i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Space, pressed: true, repeat: false, .. })))
        });
        // The press that opened the preview is still there on its first frame
        close |= space && !material::dialog_fresh(&ctx);
        if step != 0 {
            // Folders have no preview: step over them
            let before = self.selected_index;
            loop {
                let at = self.selected_index;
                self.move_selection(step, false);
                if self.selected_index == at || !self.selected_index.and_then(|i| self.items.get(i)).is_some_and(|i| i.is_dir) {
                    break;
                }
            }
            let landed_on_folder = self.selected_index.and_then(|i| self.items.get(i)).is_some_and(|i| i.is_dir);
            if landed_on_folder {
                // Nothing to show past the last file: stay where the preview was
                self.selected_index = before;
            } else if self.selected_index != before
                && let Some(next) = self.selected_index.and_then(|i| self.items.get(i)).cloned()
            {
                self.thumbs.ensure(&next.path);
                **preview = Preview::load(&next);
                **item = next;
                self.selection = self.selected_index.into_iter().collect();
            }
        }

        let summary = match &preview.body {
            Body::Picture { dimensions: Some((w, h)) } => format!("{w} × {h} · {}", format_bytes(item.size)),
            _ => format!("{} · {}", format_bytes(item.size), format_datetime(item.modified)),
        };
        ui.label(egui::RichText::new(summary).size(material::FONT_CAPTION).color(theme.subtext1));
        ui.add_space(4.0);

        let height = (ctx.content_rect().height() - 220.0).clamp(160.0, 520.0);
        let mut copy = None;
        match &preview.body {
            Body::Text { text, numbers, lines, words, cut } => {
                egui::Frame::new().fill(theme.surface0).corner_radius(egui::CornerRadius::same(material::radius_container())).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::both().id_salt("ql_text").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.0;
                            let mono = egui::FontId::monospace(12.5);
                            ui.add(egui::Label::new(egui::RichText::new(numbers).font(mono.clone()).color(theme.subtext0)).wrap_mode(egui::TextWrapMode::Extend));
                            ui.add(egui::Label::new(egui::RichText::new(text).font(mono).color(theme.text)).wrap_mode(egui::TextWrapMode::Extend).selectable(true));
                        });
                    });
                });
                let mut counts = format!("{} · {}", trn(*lines, "{} line", "{} lines"), trn(*words, "{} word", "{} words"));
                if *cut {
                    counts.push_str(&format!(" · {}", trf("showing the first {} lines", &[&MAX_LINES.min(*lines)])));
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(counts).size(material::FONT_CAPTION).color(theme.subtext1));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if material::button(ui, "ql_copy", Some(icons::COPY), tr("Copy text"), &theme, material::ButtonKind::Tonal, true).clicked() {
                            copy = Some(text.clone());
                        }
                    });
                });
            }
            Body::Picture { .. } => {
                let area = egui::vec2(ui.available_width(), height);
                let (rect, _) = ui.allocate_exact_size(area, egui::Sense::hover());
                ui.painter().rect_filled(rect, egui::CornerRadius::same(material::radius_container()), theme.surface0);
                match self.thumbs.get(&item.path) {
                    Some(tex) => {
                        let size = tex.size_vec2();
                        let fit = (rect.width() - 16.0) / size.x;
                        let fit = fit.min((rect.height() - 16.0) / size.y).min(2.0);
                        let drawn = egui::Rect::from_center_size(rect.center(), size * fit);
                        egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), size)).corner_radius(egui::CornerRadius::same(material::radius_sm())).paint_at(ui, drawn);
                    }
                    None => {
                        material::paint_icon(ui.painter(), rect.center(), item.file_type.icon(), 48.0, theme.subtext0);
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    }
                }
            }
            Body::Nothing => {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 160.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, egui::CornerRadius::same(material::radius_container()), theme.surface0);
                material::paint_icon(ui.painter(), rect.center() - egui::vec2(0.0, 12.0), item.file_type.icon(), 52.0, theme.subtext0);
                ui.painter().text(rect.center() + egui::vec2(0.0, 40.0), egui::Align2::CENTER_CENTER, tr("No preview available"), egui::FontId::proportional(material::FONT_BODY), theme.subtext1);
            }
        }
        ui.label(egui::RichText::new(tr("Space or Esc to close · ↑ ↓ to browse")).size(material::FONT_CAPTION).color(theme.subtext0));

        if let Some(text) = copy {
            ctx.copy_text(text);
            self.set_toast(tr("Copied to clipboard"));
        }
        close
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("ski-ql-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn reads_text_and_refuses_binary() {
        let path = temp("a.txt", "one two\n\tthree\n".as_bytes());
        let Some(Body::Text { text, numbers, lines, words, cut }) = read_text(&path) else { panic!("not text") };
        assert_eq!((text.as_str(), numbers.as_str(), lines, words, cut), ("one two\n    three", "1\n2", 2, 3, false));
        let bin = temp("b.bin", &[1, 2, 0, 3]);
        assert!(read_text(&bin).is_none());
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(bin);
    }

    #[test]
    fn long_files_are_cut() {
        let big = "x\n".repeat(MAX_LINES + 50);
        let path = temp("big.txt", big.as_bytes());
        let Some(Body::Text { numbers, cut, .. }) = read_text(&path) else { panic!("not text") };
        assert!(cut);
        assert_eq!(numbers.lines().count(), MAX_LINES);
        let _ = std::fs::remove_file(path);
    }
}

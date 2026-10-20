//! Chooser mode (`ski --chooser`): the open / save dialog behind the system file picker.
use super::*;
use crate::chooser::{self, Mode};

impl SkiApp {
    /// The folder of `path` is already shown; this puts the cursor on the file itself.
    pub fn select_path(&mut self, path: &Path) {
        if let Some(idx) = self.items.iter().position(|i| i.path == path) {
            self.selected_index = Some(idx);
            self.selection = BTreeSet::from([idx]);
            self.anchor = Some(idx);
            self.scroll_to_selected = true;
        }
    }

    /// A double-click or Enter on a file: it is the answer when opening, the name when saving.
    pub(super) fn chooser_activate(&mut self, item: &FileItem) {
        let Some(ch) = self.chooser.as_mut() else { return };
        match ch.spec.mode {
            Mode::Open => chooser::finish(std::slice::from_ref(&item.path)),
            Mode::Save => {
                ch.name_input = item.name.clone();
                ch.warned = None;
            }
            Mode::Folder | Mode::SaveFiles => {}
        }
    }

    /// The paths the Open / Save button would answer with, or `None` while it cannot answer yet.
    fn chooser_answer(&self) -> Option<Vec<PathBuf>> {
        let ch = self.chooser.as_ref()?;
        let selected = || self.selection.iter().filter_map(|i| self.items.get(*i));
        let browsing = !self.is_mixed_listing() && !self.in_trash();
        match ch.spec.mode {
            Mode::Open => {
                let files: Vec<PathBuf> = selected().filter(|i| !i.is_dir).map(|i| i.path.clone()).collect();
                let files = if ch.spec.multiple { files } else { files.into_iter().take(1).collect() };
                (!files.is_empty()).then_some(files)
            }
            Mode::Folder => {
                let dirs: Vec<PathBuf> = selected().filter(|i| i.is_dir).map(|i| i.path.clone()).collect();
                let dirs = if ch.spec.multiple { dirs } else { dirs.into_iter().take(1).collect() };
                if !dirs.is_empty() {
                    Some(dirs)
                } else {
                    browsing.then(|| vec![self.current_path.clone()])
                }
            }
            Mode::Save => {
                let name = Path::new(ch.name_input.trim()).file_name()?.to_owned();
                browsing.then(|| vec![self.current_path.join(name)])
            }
            Mode::SaveFiles => browsing.then(|| {
                if ch.spec.files.is_empty() {
                    vec![self.current_path.clone()]
                } else {
                    ch.spec.files.iter().map(|f| self.current_path.join(f)).collect()
                }
            }),
        }
    }

    fn chooser_accept(&mut self) {
        let Some(answer) = self.chooser_answer() else { return };
        let overwrite = self.chooser.as_ref().is_some_and(|c| c.spec.mode == Mode::Save)
            && answer[0].exists()
            && self.chooser.as_ref().is_some_and(|c| c.warned.as_ref() != Some(&answer[0]));
        if overwrite {
            let name = answer[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.set_toast(trf("{} exists. Press Save again to replace it.", &[&name]));
            if let Some(ch) = self.chooser.as_mut() {
                ch.warned = Some(answer[0].clone());
            }
            return;
        }
        chooser::finish(&answer);
    }

    /// Esc leaves the dialog once it has nothing else to close.
    pub(super) fn chooser_escape(&self, ctx: &egui::Context, busy: bool) {
        if self.chooser.is_some() && !busy && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            chooser::cancel();
        }
    }

    pub(super) fn render_chooser_bar(&mut self, ui: &mut egui::Ui) {
        let ready = self.chooser_answer().is_some();
        let Some(mut ch) = self.chooser.take() else { return };
        let theme = self.theme;
        let (mut accept, mut cancel) = (false, false);
        let label = match (&ch.spec.accept_label, ch.spec.mode) {
            (Some(l), _) => l.replace('_', ""),
            (None, Mode::Save | Mode::SaveFiles) => tr("Save").to_owned(),
            (None, Mode::Folder) => tr("Select").to_owned(),
            (None, Mode::Open) => tr("Open").to_owned(),
        };
        egui::Panel::bottom("chooser_bar")
            .frame(egui::Frame::new().fill(theme.mantle).inner_margin(egui::Margin::symmetric(CONTENT_INSET, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if ch.takes_name() {
                        ui.label(egui::RichText::new(tr("File name")).size(material::FONT_CAPTION).color(theme.subtext1));
                        let field = material::input_frame(&theme)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut ch.name_input)
                                        .frame(egui::Frame::NONE)
                                        .desired_width(240.0)
                                        .font(egui::FontId::proportional(material::FONT_BODY)),
                                )
                            })
                            .inner;
                        if field.changed() {
                            ch.warned = None;
                        }
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            accept = true;
                        }
                    }
                    if !ch.spec.filters.is_empty() {
                        let shown = ch.spec.filters.get(ch.filter).map(|f| f.label.clone()).unwrap_or_default();
                        let before = ch.filter;
                        egui::ComboBox::from_id_salt("chooser_filter").selected_text(shown).show_ui(ui, |ui| {
                            for (i, f) in ch.spec.filters.iter().enumerate() {
                                ui.selectable_value(&mut ch.filter, i, &f.label);
                            }
                        });
                        if ch.filter != before {
                            self.reload_items();
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        accept |= material::button(ui, "chooser_accept", Some(icons::CHECK), &label, &theme, ButtonKind::Filled, ready).clicked() && ready;
                        cancel |= material::button(ui, "chooser_cancel", None, tr("Cancel"), &theme, ButtonKind::Text, true).clicked();
                    });
                });
            });
        self.chooser = Some(ch);
        if cancel {
            chooser::cancel();
        }
        if accept {
            self.chooser_accept();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chooser::{Chooser, Spec};
    use crate::settings::Settings;
    use crate::ui::theme::SerpantinumTheme;

    fn app_in(dir: &Path, args: &[&str]) -> SkiApp {
        let ctx = egui::Context::default();
        let mut app = SkiApp::new(dir.to_path_buf(), SerpantinumTheme::default(), Settings::default(), &ctx);
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        app.chooser = Spec::parse(&args).map(Chooser::new);
        app.reload_items();
        app
    }

    #[test]
    fn answers_follow_the_mode() {
        let work = std::env::temp_dir().join(format!("ski_chooser_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(work.join("sub")).unwrap();
        std::fs::write(work.join("a.txt"), "a").unwrap();
        std::fs::write(work.join("b.png"), "b").unwrap();

        // Opening needs a selected file, and the filter hides the rest
        let mut app = app_in(&work, &["--chooser", "--mode", "open", "--filter", "Text\tg:*.txt"]);
        assert_eq!(app.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["sub", "a.txt"]);
        assert_eq!(app.chooser_answer(), None);
        app.select_path(&work.join("a.txt"));
        assert_eq!(app.chooser_answer(), Some(vec![work.join("a.txt")]));

        // A folder dialog answers with the open folder when nothing is selected
        let app = app_in(&work, &["--chooser", "--mode", "folder"]);
        assert_eq!(app.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["sub"]);
        assert_eq!(app.chooser_answer(), Some(vec![app.current_path.clone()]));

        // Saving needs a name and answers with the open folder joined to it
        let app = app_in(&work, &["--chooser", "--mode", "save", "--name", "new.txt"]);
        assert_eq!(app.chooser_answer(), Some(vec![app.current_path.join("new.txt")]));
        let app = app_in(&work, &["--chooser", "--mode", "save"]);
        assert_eq!(app.chooser_answer(), None);

        let _ = std::fs::remove_dir_all(&work);
    }
}

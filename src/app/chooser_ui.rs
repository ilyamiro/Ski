//! Chooser mode (`ski --chooser`): the open / save dialog behind the system file picker.
use super::*;
use crate::chooser::{self, Chooser, Mode};

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
        let Some(ch) = self.chooser.as_ref() else { return };
        match ch.spec.mode {
            Mode::Open => match self.chooser_answer() {
                // Enter on several selected files answers with all of them
                Some(files) if ch.spec.multiple && files.len() > 1 && self.selection.len() > 1 => self.chooser_answer_with(&files),
                _ => self.chooser_answer_with(std::slice::from_ref(&item.path)),
            },
            Mode::Save => self.chooser_fill_name(item),
            Mode::Folder | Mode::SaveFiles => {}
        }
    }

    /// Hands the paths to the program that asked and ends the dialog.
    pub(super) fn chooser_answer_with(&self, paths: &[PathBuf]) -> ! {
        let ch = self.chooser.as_ref().expect("only a chooser answers");
        let real = !self.is_mixed_listing() && !self.in_trash();
        ch.answer(paths, if real { &self.current_path } else { paths.first().and_then(|p| p.parent()).unwrap_or(Path::new("/")) })
    }

    fn chooser_fill_name(&mut self, item: &FileItem) {
        if let Some(ch) = self.chooser.as_mut() {
            ch.name_input = item.name.clone();
            ch.filled = Some(item.path.clone());
        }
    }

    /// Selecting an existing file while saving copies its name into the field, like GTK does.
    fn chooser_follow_selection(&mut self) {
        let single = (self.selection.len() == 1).then(|| self.selection.iter().next().and_then(|i| self.items.get(*i))).flatten();
        let Some(item) = single.filter(|i| !i.is_dir).cloned() else { return };
        if self.chooser.as_ref().is_some_and(|c| c.takes_name() && c.filled.as_ref() != Some(&item.path)) {
            self.chooser_fill_name(&item);
        }
    }

    /// Where a typed name points: `~/x`, `/abs/x` and `sub/x` are all understood.
    fn chooser_typed(&self, name: &str) -> Option<PathBuf> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let typed = match name.strip_prefix("~/") {
            Some(rest) => dirs::home_dir()?.join(rest),
            None => PathBuf::from(name),
        };
        Some(if typed.is_absolute() { typed } else { self.current_path.join(typed) })
    }

    fn chooser_target(&self, name: &str) -> Option<PathBuf> {
        let target = self.chooser_typed(name)?;
        (!name.trim().ends_with('/')).then_some(target).filter(|t| t.file_name().is_some())
    }

    /// A folder named in the save field: pressing Save goes into it instead of answering.
    fn chooser_typed_dir(&self) -> Option<PathBuf> {
        let ch = self.chooser.as_ref().filter(|c| c.spec.mode == Mode::Save)?;
        self.chooser_typed(&ch.name_input).filter(|p| p.is_dir())
    }

    pub(super) fn replace_question(target: &Path) -> (String, String) {
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let folder = target.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (trf("Replace “{}”?", &[&name]), trf("“{}” already exists in “{}”.", &[&name, &folder]))
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
                let target = self.chooser_target(&ch.name_input)?;
                // A path typed out in full doesn't depend on the folder that is open
                (browsing || target.is_absolute() && ch.name_input.trim().starts_with(['/', '~'])).then(|| vec![target])
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
        if let Some(dir) = self.chooser_typed_dir() {
            self.navigate_to(dir);
            if let Some(ch) = self.chooser.as_mut() {
                ch.name_input.clear();
            }
            return;
        }
        let Some(answer) = self.chooser_answer() else { return };
        let saving = self.chooser.as_ref().is_some_and(|c| matches!(c.spec.mode, Mode::Save | Mode::SaveFiles));
        if self.chooser.as_ref().is_some_and(|c| c.spec.mode == Mode::Save) && !answer[0].parent().is_some_and(|p| p.is_dir()) {
            self.set_toast(tr("That folder doesn't exist").to_owned());
            return;
        }
        // Every file the answer would replace is confirmed on its own
        let mut taken: Vec<PathBuf> = if saving { answer.iter().filter(|p| p.symlink_metadata().is_ok()).cloned().collect() } else { Vec::new() };
        if taken.is_empty() {
            self.chooser_answer_with(&answer);
        }
        let (title, body) = Self::replace_question(&taken.remove(0));
        self.dialog = Some(Dialog::Confirm { title, body, confirm: tr("Replace").to_owned(), action: ConfirmAction::ReplaceFile { queue: taken, answer } });
    }

    /// Esc leaves the dialog once it has nothing else to close.
    pub(super) fn chooser_escape(&self, ctx: &egui::Context, busy: bool) {
        if self.chooser.is_some() && !busy && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            chooser::cancel();
        }
    }

    pub(super) fn render_chooser_bar(&mut self, ui: &mut egui::Ui) {
        self.chooser_follow_selection();
        let ready = self.chooser_answer().is_some() || self.chooser_typed_dir().is_some();
        let Some(mut ch) = self.chooser.take() else { return };
        ch.remember_size(ui.ctx());
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
                                egui::TextEdit::singleline(&mut ch.name_input)
                                    .frame(egui::Frame::NONE)
                                    .desired_width(240.0)
                                    .font(egui::FontId::proportional(material::FONT_BODY))
                                    .show(ui)
                            })
                            .inner;
                        if ch.focus_name {
                            // Ready to type: focus the field with the name up to its extension selected
                            field.response.request_focus();
                            let end = match ch.name_input.rfind('.') {
                                Some(i) if i > 0 => ch.name_input[..i].chars().count(),
                                _ => ch.name_input.chars().count(),
                            };
                            let mut state = field.state.clone();
                            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(end))));
                            state.store(ui.ctx(), field.response.id);
                            ch.focus_name = false;
                        }
                        let field = field.response;
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            accept = true;
                        }
                    }
                    if !ch.spec.filters.is_empty() {
                        let labels: Vec<String> = ch.spec.filters.iter().map(|f| f.label.clone()).collect();
                        if material::dropdown(ui, "chooser_filter", &labels, &mut ch.filter, &theme) {
                            self.reload_items();
                        }
                    }
                    let Chooser { spec, choice_values, .. } = &mut ch;
                    for (choice, value) in spec.choices.iter().zip(choice_values.iter_mut()) {
                        if choice.options.is_empty() {
                            let mut on = value == "true";
                            if ui.checkbox(&mut on, choice.label.replace('_', "")).changed() {
                                *value = on.to_string();
                            }
                        } else {
                            ui.label(egui::RichText::new(choice.label.replace('_', "")).size(material::FONT_CAPTION).color(theme.subtext1));
                            let labels: Vec<String> = choice.options.iter().map(|(_, label)| label.clone()).collect();
                            let mut at = choice.options.iter().position(|(id, _)| id == value).unwrap_or(0);
                            if material::dropdown(ui, ("chooser_choice", &choice.id), &labels, &mut at, &theme) {
                                *value = choice.options[at].0.clone();
                            }
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
    use crate::chooser::Spec;
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

        // A typed path decides where the file goes, and a filter never renames it
        let app = app_in(&work, &["--chooser", "--mode", "save", "--name", "sub/x", "--filter", "PNG\tg:*.png"]);
        assert_eq!(app.chooser_answer(), Some(vec![app.current_path.join("sub/x")]));

        // A typed folder is gone into, not answered with
        let app = app_in(&work, &["--chooser", "--mode", "save", "--name", "sub/"]);
        assert_eq!(app.chooser_answer(), None);
        assert_eq!(app.chooser_typed_dir(), Some(app.current_path.join("sub")));
        let app = app_in(&work, &["--chooser", "--mode", "save", "--name", "/tmp/y.txt"]);
        assert_eq!(app.chooser_answer(), Some(vec![PathBuf::from("/tmp/y.txt")]));

        let _ = std::fs::remove_dir_all(&work);
    }
}

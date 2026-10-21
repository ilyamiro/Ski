//! Smaller file actions: duplicate, create link, open a terminal, and the selection tools.
use super::*;
use crate::fs::glob;

impl SkiApp {
    /// Copy each item next to itself under a free "(copy)" name.
    pub(super) fn duplicate(&mut self, paths: Vec<PathBuf>) {
        if self.in_trash() || paths.is_empty() {
            return;
        }
        let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
        for path in &paths {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { continue };
            // Names that running duplications are about to take count as taken
            let target = ops::unique_path_where(parent, name, "copy", |p| self.reserved.iter().any(|r| r == p));
            self.reserved.push(target.clone());
            pairs.push((path.clone(), target));
        }
        if pairs.is_empty() {
            return;
        }
        let label = Self::describe_count(&paths);
        let title = trf("Duplicating {}", &[&label]);
        self.engine.spawn(JobKind::Copy, title, String::new(), move |w| engine::transfer(w, Op::Copy, pairs));
    }

    /// A symbolic link to each item, next to it, named "Link to …".
    pub(super) fn make_links(&mut self, paths: Vec<PathBuf>) {
        if self.in_trash() {
            return;
        }
        let (mut made, mut failed) = (Vec::new(), Vec::new());
        for path in &paths {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { continue };
            let link_name = trf("Link to {}", &[&name.to_string_lossy()]);
            let link = ops::unique_path(parent, std::ffi::OsStr::new(&link_name), "link");
            let target = path.canonicalize().unwrap_or_else(|_| path.clone());
            match std::os::unix::fs::symlink(&target, &link) {
                Ok(()) => made.push(link),
                Err(err) => failed.push(format!("{}: {err}", name.to_string_lossy())),
            }
        }
        if made.is_empty() {
            if !failed.is_empty() {
                self.set_toast(trf("Could not create link: {}", &[&failed.join(" · ")]));
            }
            return;
        }
        self.reload_items();
        self.selection = made.iter().filter_map(|l| self.items.iter().position(|i| &i.path == l)).collect();
        self.selected_index = self.selection.iter().next().copied();
        self.anchor = self.selected_index;
        self.last_cursor = self.selected_index;
        self.scroll_to_selected = true;
        let label = Self::describe_count(&paths);
        self.history.record(UndoEntry::new(trf("Link to {}", &[&label]), made.iter().cloned().map(Edit::Trash).collect()));
        let message = if made.len() == 1 { trf("Created link to {}", &[&label]) } else { trf("Created {} links", &[&made.len()]) };
        self.set_toast_action(message, ToastAction::Undo);
    }

    pub(super) fn open_terminal(&mut self, dir: &Path) {
        let dir = if dir.is_dir() { dir.to_path_buf() } else { dir.parent().map(Path::to_path_buf).unwrap_or_else(|| self.current_path.clone()) };
        if let Err(err) = crate::fs::terminal::open_in(&dir, self.settings.terminal.trim()) {
            self.set_toast(err.to_string());
        }
    }

    /// Another Ski on `dir` that runs as root, after the password prompt of polkit.
    pub(super) fn open_as_admin(&mut self, dir: &Path) {
        if self.chooser.is_some() {
            return;
        }
        if crate::fs::admin::is_admin() {
            self.set_toast(tr("Ski already runs as administrator"));
            return;
        }
        let dir = if dir.is_dir() { dir.to_path_buf() } else { dir.parent().map(Path::to_path_buf).unwrap_or_else(|| self.current_path.clone()) };
        match crate::fs::admin::open_folder(&dir) {
            Ok(rx) => self.admin_launch = Some(rx),
            Err(err) => self.set_toast(err.to_string()),
        }
    }

    pub(super) fn invert_selection(&mut self) {
        let all: BTreeSet<usize> = (0..self.items.len()).collect();
        self.selection = all.difference(&self.selection).copied().collect();
        self.selected_index = self.selection.iter().next().copied();
        self.anchor = self.selected_index;
        self.last_cursor = self.selected_index;
        self.scroll_to_selected = self.selected_index.is_some();
    }

    pub(super) fn deselect_all(&mut self) {
        self.selection.clear();
        self.selected_index = None;
        self.anchor = None;
        self.last_cursor = None;
    }

    pub(super) fn open_select_pattern(&mut self) {
        if !self.items.is_empty() {
            self.dialog = Some(Dialog::SelectPattern { pattern: String::new(), case: false, first_frame: true });
        }
    }

    /// Names that match `pattern`, as indexes into the listing.
    fn matching_items(&self, pattern: &str, case: bool) -> BTreeSet<usize> {
        self.items.iter().enumerate().filter(|(_, i)| glob::matches(pattern, &i.name, case)).map(|(idx, _)| idx).collect()
    }

    pub(super) fn select_pattern_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::SelectPattern { pattern, case, first_frame } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, tr("Select matching items"), Some(tr("Use * for any text and ? for one character")), &theme, "sp_close");
        let mut submit = false;
        material::input_frame(&theme).show(ui, |ui| {
            let out = egui::TextEdit::singleline(pattern)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .hint_text(tr("*.png, report*"))
                .font(egui::FontId::proportional(material::FONT_SUBTITLE))
                .show(ui);
            if std::mem::take(first_frame) {
                out.response.request_focus();
            }
            submit = out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        });
        ui.horizontal_wrapped(|ui| {
            if chip(ui, "sp_case", None, tr("Match case"), &theme, *case).clicked() {
                *case = !*case;
            }
        });
        let hits = if pattern.trim().is_empty() { BTreeSet::new() } else { self.matching_items(pattern, *case) };
        let note = if pattern.trim().is_empty() { String::new() } else { trn(hits.len(), "{} item matches", "{} items match") };
        ui.label(egui::RichText::new(note).size(material::FONT_CAPTION).color(theme.subtext1));
        let footer = material::dialog_footer(ui, &theme, FooterSpec { enabled: !hits.is_empty(), ..FooterSpec::new("sp", icons::SELECT_PATTERN, tr("Select")) });
        close |= footer.cancel;
        if (footer.primary || submit) && !hits.is_empty() {
            self.selected_index = hits.iter().next().copied();
            self.anchor = self.selected_index;
            self.last_cursor = self.selected_index;
            self.selection = hits;
            self.scroll_to_selected = true;
            close = true;
        }
        close
    }

    /// The menu key: the context menu of what is selected, or of the folder itself.
    pub(super) fn open_context_menu_here(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let at = self.selected_index.and_then(|idx| self.items.get(idx).cloned().map(|item| (idx, item)));
        match at {
            Some((idx, item)) => {
                let pos = self
                    .last_item_rects
                    .iter()
                    .find(|(i, _)| *i == idx)
                    .map_or(screen.center(), |(_, r)| egui::pos2(r.left() + 120.0, r.center().y));
                self.open_menu(ctx, pos, MenuTarget::Item(item));
            }
            None if self.can_modify_here() => self.open_menu(ctx, egui::pos2(screen.center().x, screen.top() + 140.0), MenuTarget::Folder),
            None => {}
        }
    }
}

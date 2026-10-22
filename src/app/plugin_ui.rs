//! The app's side of the plugin hooks: marks over icons, dialogs plugins draw, entries that ask
//! first, and the file changes plugins are told about. Each hook in `app.rs` is a call into here.
use eframe::egui;

use super::{ConfirmAction, Dialog, SkiApp, paint_badge};
use crate::fs::history::{Edit, UndoEntry};
use crate::i18n::tr;
use crate::plugins::{FileOpEvent, Mark, Reaction};
use crate::ui::material;
use crate::ui::theme::SerpantinumTheme;

/// Dots shown for one file before the rest fold into "+N".
const MAX_DOTS: usize = 3;

/// Letters sit at `badge_at`; dots form a row starting at `dots_at`.
pub(super) fn paint_marks(ui: &egui::Ui, theme: &SerpantinumTheme, marks: &[Mark], badge_at: egui::Pos2, dots_at: egui::Pos2) {
    let painter = ui.painter();
    let mut dots = 0;
    for mark in marks {
        match mark {
            Mark::Letter(badge) => paint_badge(ui, theme, *badge, badge_at),
            Mark::Dot(rgb) => {
                if dots < MAX_DOTS {
                    let center = dots_at + egui::vec2(dots as f32 * 8.0, 0.0);
                    painter.circle_filled(center, 5.0, theme.base);
                    painter.circle_filled(center, 3.8, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                }
                dots += 1;
            }
        }
    }
    if dots > MAX_DOTS {
        let at = dots_at + egui::vec2(MAX_DOTS as f32 * 8.0 + 3.0, 0.0);
        painter.text(at, egui::Align2::LEFT_CENTER, format!("+{}", dots - MAX_DOTS), egui::FontId::proportional(9.5), theme.subtext0);
    }
}

impl SkiApp {
    /// Runs a plugin entry, after the question it asked, if any.
    pub(super) fn run_plugin_entry(&mut self, ctx: &egui::Context, plugin: &'static str, id: String, paths: Vec<std::path::PathBuf>, confirm: Option<&'static str>) {
        match confirm {
            Some(question) => {
                self.dialog = Some(Dialog::Confirm {
                    title: tr("Are you sure?").to_owned(),
                    body: question.to_owned(),
                    confirm: tr("Continue").to_owned(),
                    action: ConfirmAction::Plugin(plugin, id, paths),
                });
            }
            None => {
                self.run_plugin_confirmed(ctx, plugin, &id, &paths);
            }
        }
    }

    pub(super) fn run_plugin_confirmed(&mut self, ctx: &egui::Context, plugin: &'static str, id: &str, paths: &[std::path::PathBuf]) {
        let here = self.current_path.clone();
        let reaction = self.plugins.run(plugin, id, paths, &here);
        self.apply_reaction(ctx, reaction);
    }

    pub(super) fn apply_reaction(&mut self, ctx: &egui::Context, reaction: Reaction) {
        match reaction {
            Reaction::None => {}
            Reaction::Job(job) => {
                let id = self.engine.spawn(crate::fs::engine::JobKind::Other, job.title, String::new(), job.work);
                if !job.removes.is_empty() {
                    self.job_removals.insert(id, job.removes);
                }
            }
            Reaction::Search(query) => {
                self.show_search = true;
                self.search_query = query;
                self.on_query_changed(ctx);
            }
        }
    }

    /// Opens the dialog a plugin asks for when nothing else is open.
    pub(super) fn open_plugin_dialog(&mut self) {
        if self.dialog.is_none()
            && let Some((id, _)) = self.plugins.wanted_window()
        {
            self.dialog = Some(Dialog::Plugin(id));
        }
    }

    pub(super) fn plugin_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Plugin(id) = dialog else { return false };
        let theme = self.theme;
        let Some((_, title)) = self.plugins.wanted_window().filter(|(plugin, _)| plugin == id) else { return true };
        let mut close = material::dialog_header(ui, title, None, &theme, "plugin_close");
        close |= !self.plugins.window(id, ui, &theme);
        close
    }

    /// A job that was to remove files has finished: tell plugins which are gone and drop their sizes.
    pub(super) fn report_removals(&mut self, job: u64) {
        let Some(paths) = self.job_removals.remove(&job) else { return };
        let gone: Vec<std::path::PathBuf> = paths.into_iter().filter(|p| p.symlink_metadata().is_err()).collect();
        if gone.is_empty() {
            return;
        }
        self.forget_sizes(&gone);
        self.plugins.file_op_done(&FileOpEvent::Removed(gone));
    }

    /// The entries plugins add for the open folder (and the main menu); `None` when there are none.
    pub(super) fn plugin_folder_group(&self) -> Option<Vec<super::MenuEntry>> {
        let others = self.other_tab_folders();
        let mut paths = vec![self.current_path.clone()];
        paths.extend(others.iter().cloned());
        let entries: Vec<super::MenuEntry> = self
            .plugins
            .folder_menu_items(&self.current_path, &others)
            .into_iter()
            .map(|(plugin, entry)| (entry.icon, entry.label, super::MenuAction::Plugin(plugin, entry.id, paths.clone(), entry.confirm)))
            .collect();
        (!entries.is_empty()).then_some(entries)
    }

    /// Tells plugins about the moves an undoable change made (the entry lists how to take them back).
    pub(super) fn report_moves(&mut self, entry: &UndoEntry) {
        let mut moved = Vec::new();
        for edit in &entry.edits {
            match edit {
                Edit::Move { from, to } => moved.push((to.clone(), from.clone())),
                Edit::Renames(back) => moved.extend(back.iter().map(|(new, old)| (old.clone(), new.clone()))),
                _ => {}
            }
        }
        if !moved.is_empty() {
            self.plugins.file_op_done(&FileOpEvent::Moved(moved));
        }
    }
}

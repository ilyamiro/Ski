//! Running operations: the progress button in the header, the popover that lists them, the
//! question asked when a name is taken, and undo / redo.
use super::*;
use crate::fs::engine::{Decision, Effect, Event, JobKind, JobState, Outcome};
use crate::fs::file_item::format_bytes;
use crate::fs::history;

const POPOVER_WIDTH: f32 = 372.0;

/// What the popover needs to draw one job, copied out so the app can be changed while drawing.
struct JobRow {
    id: u64,
    kind: JobKind,
    title: String,
    detail: String,
    state: JobState,
    message: String,
    fraction: Option<f32>,
    status: String,
    paused: bool,
    waiting: bool,
    controllable: bool,
    /// For a job queued behind another: its place among those waiting for the same disk.
    queue: Option<(usize, usize)>,
}

/// What a click in the list of operations asks for.
enum JobAction {
    TogglePause(u64),
    Cancel(u64),
    StartNow(u64),
    Move(u64, i32),
}

fn job_icon(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Copy => icons::COPY,
        JobKind::Move => icons::FOLDER_MOVE,
        JobKind::Extract => icons::EXTRACT,
        JobKind::Compress => icons::COMPRESS,
        JobKind::Delete => icons::DELETE_FOREVER,
        JobKind::Other => icons::HISTORY,
    }
}

fn format_eta(eta: std::time::Duration) -> String {
    let secs = eta.as_secs();
    if secs < 60 {
        trf("{} s left", &[&secs.max(1)])
    } else if secs < 3600 {
        trf("{} min left", &[&(secs / 60 + 1)])
    } else {
        trf("{} h {} min left", &[&(secs / 3600), &(secs % 3600 / 60)])
    }
}

impl SkiApp {
    /// Called once per frame: take in what finished jobs report and open the next question.
    pub(super) fn poll_jobs(&mut self) {
        self.engine.sample();
        let mut changed = false;
        for event in self.engine.poll() {
            match event {
                Event::Conflict(request) => self.pending_conflicts.push_back(request),
                Event::Done { job, outcome } => {
                    changed = true;
                    if self.history_job == Some(job) {
                        self.history_job = None;
                    }
                    self.finish_job(outcome);
                    self.report_removals(job);
                }
            }
        }
        if self.dialog.is_none()
            && let Some(request) = self.pending_conflicts.pop_front()
        {
            // The job may have been cancelled while the question waited
            let alive = self.engine.jobs.iter().any(|j| j.id == request.job && j.state == JobState::Running);
            if alive {
                self.dialog = Some(Dialog::Conflict { request: Some(request), all: false });
            }
        }
        if !self.engine.running() {
            self.reserved.clear();
        }
        if !self.jobs_popover {
            self.engine.prune(std::time::Duration::from_secs(7));
        }
        if changed && self.search_query.trim().is_empty() {
            self.reload_items_later();
        } else if changed {
            self.items.retain(|i| i.path.symlink_metadata().is_ok());
        }
    }

    fn finish_job(&mut self, outcome: Outcome) {
        let mut undoable = false;
        match outcome.effect {
            Effect::Record(entry) => {
                undoable = outcome.ok;
                self.report_moves(&entry);
                self.forget_entry_sizes(&entry);
                self.history.record(entry);
            }
            Effect::Undone(entry) => {
                self.forget_entry_sizes(&entry);
                self.history.push_redo(entry);
            }
            Effect::Redone(entry) => {
                self.forget_entry_sizes(&entry);
                self.history.push_undo(entry);
            }
            // A job that changed things without an undo (deleting for good, a cancelled copy)
            Effect::None => crate::fs::dirsize::clear(),
        }
        let message = if outcome.cancelled || outcome.ok { outcome.message } else { trf("Failed: {}", &[&outcome.message]) };
        if undoable {
            self.set_toast_action(message, ToastAction::Undo);
        } else {
            self.set_toast(message);
        }
    }

    /// Take back the last change.
    pub(super) fn undo(&mut self) {
        self.run_history(false);
    }

    pub(super) fn redo(&mut self) {
        self.run_history(true);
    }

    fn run_history(&mut self, redo: bool) {
        if self.history_job.is_some() {
            return;
        }
        let entry = if redo { self.history.pop_redo() } else { self.history.pop_undo() };
        let Some(entry) = entry else {
            self.set_toast(if redo { tr("Nothing to redo") } else { tr("Nothing to undo") });
            return;
        };
        let label = entry.label.clone();
        let title = if redo { trf("Redoing {}…", &[&label]) } else { trf("Undoing {}…", &[&label]) };
        let id = self.engine.spawn(JobKind::Other, title, String::new(), move |_| {
            let (inverse, error) = history::apply(entry);
            let effect = match inverse {
                Some(inverse) if redo => Effect::Redone(inverse),
                Some(inverse) => Effect::Undone(inverse),
                None => Effect::None,
            };
            let message = if redo { trf("Redid: {}", &[&label]) } else { trf("Undid: {}", &[&label]) };
            match error {
                None => Outcome::done(message).with(effect),
                Some(err) => Outcome::failed(err).with(effect),
            }
        });
        self.history_job = Some(id);
    }

    fn job_rows(&self) -> Vec<JobRow> {
        use std::sync::atomic::Ordering::Relaxed;
        self.engine
            .jobs
            .iter()
            .map(|job| {
                let s = &job.shared;
                let (bytes_done, bytes_total) = (s.bytes_done.load(Relaxed), s.bytes_total.load(Relaxed));
                let (files_done, files_total) = (s.files_done.load(Relaxed), s.files_total.load(Relaxed));
                let current = s.current();
                let status = if bytes_total > 0 {
                    let mut text = trf("{} of {}", &[&format_bytes(bytes_done), &format_bytes(bytes_total)]);
                    if job.speed > 1.0 {
                        text.push_str(&format!(" · {}", trf("{}/s", &[&format_bytes(job.speed as u64)])));
                    }
                    if let Some(eta) = job.eta() {
                        text.push_str(&format!(" · {}", format_eta(eta)));
                    }
                    text
                } else if files_total > 1 {
                    trf("{} of {} items", &[&files_done, &files_total])
                } else if !current.is_empty() {
                    current
                } else {
                    tr("Working…").to_owned()
                };
                JobRow {
                    id: job.id,
                    kind: job.kind,
                    title: job.title.clone(),
                    detail: job.detail.clone(),
                    state: job.state,
                    message: job.message.clone(),
                    fraction: s.fraction(),
                    status,
                    paused: job.paused(),
                    waiting: s.waiting.load(Relaxed),
                    controllable: job.controllable(),
                    queue: self.engine.queue_position(job.id),
                }
            })
            .collect()
    }

    /// The round progress button of the header: a ring that fills as the work gets done.
    pub(super) fn jobs_button(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        let ctx = ui.ctx().clone();
        let running = self.engine.running();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(CONTROL_HEIGHT, CONTROL_HEIGHT), egui::Sense::click());
        self.jobs_button_rect = rect;
        let id = ui.make_persistent_id("jobs_button");
        let fx = material::fx(ui, id, &resp, true, &material::FxSpec::ICON);
        let r = material::scaled(rect, fx.scale);
        let open_t = material::animate(&ctx, id.with("open"), self.jobs_popover);
        let layer = material::lerp_color(material::row_layer(&theme, &fx), theme.with_alpha(theme.text, material::ALPHA_PRESS), open_t);
        material::paint_fill(ui.painter(), r, material::pill(r), layer, &fx);

        let all_ok = !running && self.engine.jobs.iter().all(|j| j.state == JobState::Done);
        let done_t = material::animate(&ctx, id.with("done"), !running);
        let time = ctx.input(|i| i.time);
        let ring_color = material::lerp_color(theme.mauve, theme.green, done_t);
        let fraction = if running { self.engine.overall() } else { Some(1.0) };
        material::progress_ring(ui.painter(), rect.center(), 10.0, fraction, theme.surface2, ring_color, time);
        let glyph = if running {
            let kind = self.engine.jobs.iter().find(|j| j.state == JobState::Running).map_or(JobKind::Other, |j| j.kind);
            job_icon(kind)
        } else if all_ok {
            icons::CHECK
        } else {
            icons::ALERT
        };
        material::paint_icon(ui.painter(), rect.center(), glyph, 10.0, material::lerp_color(theme.subtext1, theme.text, fx.hover));
        if running {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }
        if resp.on_hover_text(tr("File operations")).clicked() {
            self.jobs_popover = !self.jobs_popover;
        }
    }

    /// The list of operations, hanging from the header button.
    pub(super) fn render_jobs_popover(&mut self, ctx: &egui::Context) {
        if self.engine.jobs.is_empty() || self.dialog.is_some() {
            self.jobs_popover = false;
        }
        let t = material::animate(ctx, egui::Id::unique("jobs_popover_anim"), self.jobs_popover);
        if t <= 0.0 {
            return;
        }
        let theme = self.theme;
        let rows = self.job_rows();
        let mut action = None;
        let screen = ctx.content_rect();
        let width = POPOVER_WIDTH.min(screen.width() - 24.0);
        let anchor = self.jobs_button_rect;
        let area = egui::Area::new(egui::Id::unique("jobs_popover"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(egui::pos2((anchor.right() + 4.0).min(screen.right() - 8.0), anchor.bottom() + POPUP_GAP - (1.0 - t) * 8.0))
            .interactable(self.jobs_popover)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::new()
                    .fill(theme.mantle)
                    .corner_radius(egui::CornerRadius::same(material::radius_container()))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(width - 20.0);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        egui::ScrollArea::vertical().max_height((screen.height() - 120.0).max(120.0)).auto_shrink([false, true]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            for row in &rows {
                                if let Some(a) = Self::job_row(ui, &theme, row) {
                                    action.get_or_insert(a);
                                }
                            }
                        });
                    });
            });
        let rect = area.response.rect;
        if self.jobs_popover {
            // Tab and the arrow keys stay in the popover; a press on the header button only closes it
            ctx.memory_mut(|m| m.set_modal_layer(area.response.layer_id));
            if ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))) {
                self.jobs_popover = false;
            }
        }
        match action {
            Some(JobAction::TogglePause(id)) => self.engine.toggle_pause(id),
            Some(JobAction::Cancel(id)) => self.engine.cancel(id),
            Some(JobAction::StartNow(id)) => self.engine.start_now(id),
            Some(JobAction::Move(id, delta)) => self.engine.move_waiting(id, delta),
            None => {}
        }
    }

    /// One operation as a card: icon, name, status line, progress bar and its controls.
    fn job_row(ui: &mut egui::Ui, theme: &SerpantinumTheme, row: &JobRow) -> Option<JobAction> {
        let mut action = None;
        let running = row.state == JobState::Running;
        let height = if running { 76.0 } else { 56.0 };
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
        ui.painter().rect_filled(material::snap(ui.painter(), rect), egui::CornerRadius::same(material::radius_md()), theme.surface1);

        let (icon, tint) = match row.state {
            JobState::Waiting => (icons::HISTORY, theme.subtext1),
            JobState::Running => (job_icon(row.kind), theme.mauve),
            JobState::Done => (icons::CHECK, theme.green),
            JobState::Cancelled => (icons::CANCEL, theme.subtext1),
            JobState::Failed => (icons::ALERT, theme.red),
        };
        material::paint_icon(ui.painter(), egui::pos2(rect.left() + 22.0, rect.top() + 24.0), icon, material::ICON_MD, tint);

        // Controls at the right edge
        let mut right = rect.right() - 10.0;
        if row.state == JobState::Waiting {
            let mut button = |ui: &mut egui::Ui, id: &str, glyph: &'static str, tip: &'static str, tint: Option<egui::Color32>| {
                let at = egui::Rect::from_center_size(egui::pos2(right - 13.0, rect.top() + 24.0), egui::vec2(26.0, 26.0));
                right -= 32.0;
                ui.put(at, |ui: &mut egui::Ui| material::filled_icon_button(ui, (id, row.id), glyph, theme, 26.0, tip, tint)).clicked()
            };
            if button(ui, "job_cancel", icons::CLOSE, tr("Cancel"), None) {
                action = Some(JobAction::Cancel(row.id));
            }
            if button(ui, "job_start", icons::PLAY, tr("Start now"), None) {
                action = Some(JobAction::StartNow(row.id));
            }
            if let Some((at, total)) = row.queue {
                if at + 1 < total && button(ui, "job_down", icons::CHEVRON_DOWN, tr("Move down"), None) {
                    action = Some(JobAction::Move(row.id, 1));
                }
                if at > 0 && button(ui, "job_up", icons::ARROW_UP, tr("Move up"), None) {
                    action = Some(JobAction::Move(row.id, -1));
                }
            }
        }
        if running && row.controllable {
            let cancel_rect = egui::Rect::from_center_size(egui::pos2(right - 13.0, rect.top() + 24.0), egui::vec2(26.0, 26.0));
            if ui.put(cancel_rect, |ui: &mut egui::Ui| material::filled_icon_button(ui, ("job_cancel", row.id), icons::CLOSE, theme, 26.0, tr("Cancel"), None)).clicked() {
                action = Some(JobAction::Cancel(row.id));
            }
            right -= 32.0;
            if row.kind != JobKind::Delete {
                let pause_rect = egui::Rect::from_center_size(egui::pos2(right - 13.0, rect.top() + 24.0), egui::vec2(26.0, 26.0));
                let (glyph, tip) = if row.paused { (icons::PLAY, tr("Resume")) } else { (icons::PAUSE, tr("Pause")) };
                if ui.put(pause_rect, |ui: &mut egui::Ui| material::filled_icon_button(ui, ("job_pause", row.id), glyph, theme, 26.0, tip, None)).clicked() {
                    action = Some(JobAction::TogglePause(row.id));
                }
                right -= 32.0;
            }
        }

        let text_left = rect.left() + 44.0;
        let text_width = (right - text_left - 6.0).max(40.0);
        material::paint_label(ui, text_left, rect.top() + 24.0 - 8.0, &row.title, material::FONT_BODY, theme.text, text_width);
        let sub = match row.state {
            JobState::Waiting => tr("Waiting for the disk").to_owned(),
            JobState::Running if row.waiting => tr("Waiting for your answer").to_owned(),
            JobState::Running if row.paused => tr("Paused").to_owned(),
            JobState::Running => row.status.clone(),
            JobState::Cancelled => tr("Cancelled").to_owned(),
            _ if !row.message.is_empty() => row.message.clone(),
            _ => row.detail.clone(),
        };
        let sub_color = if row.state == JobState::Failed { theme.red } else { theme.subtext1 };
        material::paint_label(ui, text_left, rect.top() + 24.0 + 9.0, &sub, material::FONT_CAPTION, sub_color, text_width);
        if running {
            let bar = egui::Rect::from_min_size(egui::pos2(text_left, rect.bottom() - 20.0), egui::vec2(rect.right() - text_left - 14.0, 6.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
                let color = if row.paused { theme.subtext1 } else { theme.mauve };
                material::progress_bar(ui, ("job_bar", row.id), bar.width(), if row.waiting { None } else { row.fraction }, theme, color);
            });
        }
        action
    }

    /// "Replace or skip?" — the question a copy or move asks when a name is already taken.
    pub(super) fn conflict_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Conflict { request, all } = dialog else { return false };
        let theme = self.theme;
        let Some(req) = request.as_ref() else { return true };
        let name = req.target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let folder = req.target.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let merge = req.can_merge();
        let title = if merge { trf("Merge “{}”?", &[&name]) } else { trf("Replace “{}”?", &[&name]) };

        let mut decision = None;
        if material::dialog_header(ui, &title, Some(&trf("“{}” already exists in “{}”.", &[&name, &folder])), &theme, "cf_close") {
            decision = Some(Decision::Cancel);
        }

        let describe = |info: &crate::fs::engine::EntryInfo| {
            let size = if info.is_dir { tr("Folder").to_owned() } else { format_bytes(info.size) };
            format!("{size} · {}", crate::fs::props::format_datetime(info.modified))
        };
        let newer = req.source_info.modified.zip(req.target_info.modified).map(|(a, b)| a.cmp(&b));
        let tags = |incoming: bool| -> Vec<&'static str> {
            let mut tags = Vec::new();
            match (newer, incoming) {
                (Some(std::cmp::Ordering::Greater), true) | (Some(std::cmp::Ordering::Less), false) => tags.push(tr("Newer")),
                (Some(std::cmp::Ordering::Less), true) | (Some(std::cmp::Ordering::Greater), false) => tags.push(tr("Older")),
                _ => {}
            }
            if !req.source_info.is_dir && req.source_info.size != req.target_info.size {
                let bigger = (req.source_info.size > req.target_info.size) == incoming;
                tags.push(if bigger { tr("Larger") } else { tr("Smaller") });
            }
            tags
        };
        material::group_card(ui, &theme, |ui| {
            for (label, info, incoming) in [(tr("Existing"), &req.target_info, false), (tr("New"), &req.source_info, true)] {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 56.0), egui::Sense::hover());
                let glyph = if info.is_dir { icons::FOLDER } else { icons::FILE };
                material::paint_icon(ui.painter(), egui::pos2(rect.left() + material::INSET_ICON + 2.0, rect.center().y), glyph, 24.0, if info.is_dir { theme.blue } else { theme.subtext0 });
                let text_x = rect.left() + material::INSET_LABEL + 2.0;
                material::paint_label(ui, text_x, rect.center().y - 9.0, label, material::FONT_BODY, theme.text, rect.width() - 150.0);
                material::paint_label(ui, text_x, rect.center().y + 9.0, &describe(info), material::FONT_CAPTION, theme.subtext1, rect.width() - 62.0);
                let tag = tags(incoming).join(" · ");
                if !tag.is_empty() {
                    let galley = ui.painter().layout_no_wrap(tag, egui::FontId::proportional(material::FONT_CAPTION), theme.mauve);
                    ui.painter().galley(egui::pos2(rect.right() - 13.0 - galley.size().x, rect.center().y - 9.0 - galley.size().y / 2.0), galley, theme.mauve);
                }
            }
        });
        material::group_card(ui, &theme, |ui| {
            if material::switch_row(ui, "cf_all", icons::SELECT_ALL, tr("Apply to all conflicts"), &theme, *all).clicked() {
                *all = !*all;
            }
        });

        ui.add_space(4.0);
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), material::BUTTON_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if merge {
                if material::button(ui, "cf_merge", Some(icons::FOLDER_MOVE), tr("Merge"), &theme, ButtonKind::Filled, true).clicked() {
                    decision = Some(Decision::Merge);
                }
                if material::button(ui, "cf_replace", None, tr("Replace"), &theme, ButtonKind::Tonal, true).clicked() {
                    decision = Some(Decision::Replace);
                }
            } else if material::button(ui, "cf_replace", Some(icons::CHECK), tr("Replace"), &theme, ButtonKind::Filled, true).clicked() {
                decision = Some(Decision::Replace);
            }
            if material::button(ui, "cf_both", None, tr("Keep both"), &theme, ButtonKind::Tonal, true).clicked() {
                decision = Some(Decision::KeepBoth);
            }
            if material::button(ui, "cf_skip", None, tr("Skip"), &theme, ButtonKind::Tonal, true).clicked() {
                decision = Some(Decision::Skip);
            }
        });

        if let Some(decision) = decision {
            if let Some(req) = request.take() {
                req.answer(decision, *all && decision != Decision::Cancel);
            }
            return true;
        }
        false
    }
}

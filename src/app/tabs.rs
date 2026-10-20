//! Browser-style tabs. The tab being shown lives in the `SkiApp` fields everything else already
//! reads; the others are parked in `TabSlot::saved` and swapped in when picked.
use super::*;
use crate::fs::search::Search;

/// Everything that belongs to one tab: where it is, what is selected, its history and its search.
pub(super) struct TabState {
    current_path: PathBuf,
    selection: Vec<PathBuf>,
    focus: Option<PathBuf>,
    back_history: Vec<PathBuf>,
    forward_history: Vec<PathBuf>,
    search_query: String,
    type_filter: TypeFilter,
}

impl TabState {
    fn fresh(path: PathBuf) -> Self {
        Self {
            current_path: path,
            selection: Vec::new(),
            focus: None,
            back_history: Vec::new(),
            forward_history: Vec::new(),
            search_query: String::new(),
            type_filter: TypeFilter::All,
        }
    }
}

pub(super) struct TabSlot {
    pub id: u64,
    /// `None` for the tab on screen, whose state is in the app itself.
    saved: Option<TabState>,
}

impl TabSlot {
    pub fn live(id: u64) -> Self {
        Self { id, saved: None }
    }
}

/// Narrowest and widest a tab gets.
const TAB_MIN_W: f32 = 96.0;
const TAB_MAX_W: f32 = 220.0;
pub(super) const TAB_BAR_HEIGHT: f32 = 40.0;
const TAB_HEIGHT: f32 = 32.0;

impl SkiApp {
    /// Park the tab on screen so another can take its place.
    fn take_tab_state(&mut self) -> TabState {
        self.search.cancel();
        TabState {
            current_path: self.current_path.clone(),
            selection: self.selected_paths(),
            focus: self.selected_index.and_then(|i| self.items.get(i)).map(|i| i.path.clone()),
            back_history: std::mem::take(&mut self.back_history),
            forward_history: std::mem::take(&mut self.forward_history),
            search_query: std::mem::take(&mut self.search_query),
            type_filter: self.type_filter,
        }
    }

    /// Show a parked tab: its folder is read again, so changes made meanwhile are visible.
    fn put_tab_state(&mut self, state: TabState) {
        self.current_path = state.current_path;
        self.back_history = state.back_history;
        self.forward_history = state.forward_history;
        self.search_query = state.search_query;
        self.type_filter = state.type_filter;
        self.search = Search::new();
        self.path_input = self.current_path.to_string_lossy().into_owned();
        self.is_editing_path = false;
        self.show_search = !self.search_query.trim().is_empty();
        self.drag_select = None;
        self.last_rubberband = None;
        self.selected_index = None;
        self.items.clear();
        if self.search_query.trim().is_empty() {
            self.reload_items();
        } else {
            self.restart_search = true;
        }
        let at = |p: &PathBuf| self.items.iter().position(|i| &i.path == p);
        let selection: BTreeSet<usize> = state.selection.iter().filter_map(at).collect();
        let focus = state.focus.as_ref().and_then(at).or_else(|| selection.iter().next().copied());
        self.selection = selection;
        self.selected_index = focus;
        self.anchor = focus;
        self.last_cursor = focus;
        self.scroll_to_selected = focus.is_some();
        if !self.in_recent() {
            self.settings.last_dir = Some(self.current_path.clone());
        }
    }

    pub(super) fn switch_tab(&mut self, to: usize) {
        if to >= self.tabs.len() || to == self.active_tab {
            return;
        }
        let state = self.take_tab_state();
        self.tabs[self.active_tab].saved = Some(state);
        self.active_tab = to;
        if let Some(state) = self.tabs[to].saved.take() {
            self.put_tab_state(state);
        }
    }

    /// Open `path` in a new tab next to the current one and show it.
    pub(super) fn new_tab(&mut self, path: PathBuf) {
        let path = if path.as_os_str() == RECENT_PATH { path } else { path.canonicalize().unwrap_or(path) };
        let state = self.take_tab_state();
        self.tabs[self.active_tab].saved = Some(state);
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let at = self.active_tab + 1;
        self.tabs.insert(at, TabSlot::live(id));
        self.active_tab = at;
        if !path.as_os_str().is_empty() && path.as_os_str() != RECENT_PATH {
            recent::record(&path);
        }
        self.put_tab_state(TabState::fresh(path));
        self.tab_enter = Some(id);
    }

    /// Close a tab; closing the last one closes the window.
    pub(super) fn close_tab(&mut self, ctx: &egui::Context, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if idx == self.active_tab {
            // Show the neighbour that takes its place
            let next = if idx + 1 < self.tabs.len() { idx + 1 } else { idx - 1 };
            let state = self.tabs[next].saved.take();
            self.search.cancel();
            self.tabs.remove(idx);
            self.active_tab = if next > idx { next - 1 } else { next };
            if let Some(state) = state {
                self.put_tab_state(state);
            }
        } else {
            self.tabs.remove(idx);
            if idx < self.active_tab {
                self.active_tab -= 1;
            }
        }
    }

    pub(super) fn cycle_tab(&mut self, delta: isize) {
        let n = self.tabs.len() as isize;
        if n > 1 {
            self.switch_tab((self.active_tab as isize + delta).rem_euclid(n) as usize);
        }
    }

    fn move_tab(&mut self, from: usize, to: usize) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let slot = self.tabs.remove(from);
        self.tabs.insert(to, slot);
        if self.active_tab == from {
            self.active_tab = to;
        } else if from < self.active_tab && to >= self.active_tab {
            self.active_tab -= 1;
        } else if from > self.active_tab && to <= self.active_tab {
            self.active_tab += 1;
        }
    }

    /// Name and icon a tab shows: the folder, or the kind of place when it is not one.
    fn tab_label(&self, idx: usize) -> (&'static str, String) {
        let (path, query) = match &self.tabs[idx].saved {
            Some(s) => (&s.current_path, s.search_query.as_str()),
            None => (&self.current_path, self.search_query.as_str()),
        };
        if !query.trim().is_empty() {
            (icons::SEARCH, tr("Search").to_owned())
        } else if path.as_os_str() == RECENT_PATH {
            (icons::RECENT, tr("Recent").to_owned())
        } else if trash::is_trash_folder(path) {
            (icons::TRASH, tr("Trash").to_owned())
        } else if Some(path.as_path()) == dirs::home_dir().and_then(|h| h.canonicalize().ok()).as_deref() {
            (icons::HOME, tr("Home").to_owned())
        } else {
            (icons::FOLDER, path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tr("Computer").to_owned()))
        }
    }

    /// The strip of tabs under the header. It grows in when a second tab opens and folds away when
    /// only one is left, so a single tab never costs any room.
    pub(super) fn render_tab_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let theme = self.theme;
        let t = material::animate(&ctx, egui::Id::unique("tab_bar_anim"), self.tabs.len() > 1);
        if t <= 0.001 {
            return;
        }
        egui::Panel::top("tab_bar")
            .exact_size((TAB_BAR_HEIGHT * t).max(1.0))
            .resizable(false)
            .frame(egui::Frame::new().fill(theme.base).inner_margin(egui::Margin::symmetric(HEADER_MARGIN, 0)))
            .show(ui, |ui| {
                ui.set_opacity(t);
                let row = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(ui.max_rect().width(), TAB_BAR_HEIGHT));
                let plus_w = TAB_HEIGHT + 4.0;
                let strip = egui::Rect::from_min_max(row.min, egui::pos2(row.right() - plus_w, row.bottom()));
                let count = self.tabs.len();
                let width = ((strip.width() - 4.0 * (count as f32 - 1.0)) / count as f32).clamp(TAB_MIN_W, TAB_MAX_W);

                let mut switch_to = None;
                let mut close = None;
                let mut drag_to: Option<(usize, usize)> = None;
                let mut rects = Vec::with_capacity(count);
                let mut x = strip.left();
                let top = row.center().y - TAB_HEIGHT / 2.0;
                let active = self.active_tab;
                let entering = self.tab_enter.take();
                for idx in 0..count {
                    let id = self.tabs[idx].id;
                    let grow_id = ui.make_persistent_id(("tab_grow", id));
                    if entering == Some(id) {
                        material::reset_anim(&ctx, grow_id, 0.0);
                    }
                    let grow = material::animate_value(&ctx, grow_id, 1.0, material::ANIM);
                    let w = width * grow;
                    let rect = egui::Rect::from_min_size(egui::pos2(x, top), egui::vec2(w, TAB_HEIGHT));
                    x += w + 4.0 * grow;
                    rects.push(rect);
                    if rect.right() > strip.right() + 1.0 {
                        continue;
                    }
                    let (icon, title) = self.tab_label(idx);
                    let tab_id = ui.make_persistent_id(("tab", id));
                    let tab_path = self.tabs[idx].saved.as_ref().map_or(&self.current_path, |s| &s.current_path).clone();
                    self.drop_zones.push(super::dnd::DropZone { rect, dest: tab_path });
                    let resp = ui.interact(rect, tab_id, egui::Sense::click_and_drag());
                    let fx = material::fx(ui, tab_id, &resp, true, &material::FxSpec::ROW);
                    let on = material::animate(&ctx, tab_id.with("on"), idx == active);
                    let base = material::selectable_bg(&theme, &fx, on);
                    let rounded = egui::CornerRadius::same(material::radius_md());
                    material::paint_fill(ui.painter(), rect, rounded, base, &fx);

                    let color = material::lerp_color(material::lerp_color(theme.subtext0, theme.text, fx.hover), theme.mauve, on);
                    material::paint_icon(ui.painter(), egui::pos2(rect.left() + 16.0, rect.center().y), icon, material::ICON_SM, color);
                    let close_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 17.0, rect.center().y), egui::vec2(22.0, 22.0));
                    let close_resp = ui.interact(close_rect, tab_id.with("x"), egui::Sense::click());
                    let close_t = material::animate_for(&ctx, tab_id.with("xs"), resp.hovered() || close_resp.hovered() || idx == active, material::ANIM_FAST);
                    if close_t > 0.01 {
                        let cfx = material::fx(ui, tab_id.with("x"), &close_resp, true, &material::FxSpec::ICON);
                        let r = material::scaled(close_rect, cfx.scale);
                        material::paint_fill(ui.painter(), r, material::pill(r), material::row_layer(&theme, &cfx).gamma_multiply(close_t), &cfx);
                        material::paint_icon(ui.painter(), close_rect.center(), icons::CLOSE, material::ICON_XS, color.gamma_multiply(close_t));
                    }
                    let label_right = rect.right() - 8.0 - 22.0 * close_t.max(if idx == active { 1.0 } else { 0.0 }) - 4.0;
                    material::paint_label(ui, rect.left() + 32.0, rect.center().y, &title, material::FONT_BODY, color, (label_right - rect.left() - 32.0).max(12.0));

                    if close_resp.clicked() || resp.middle_clicked() {
                        close = Some(idx);
                    } else if resp.clicked() {
                        switch_to = Some(idx);
                    }
                    if resp.dragged() && let Some(p) = resp.interact_pointer_pos() {
                        if let Some(over) = rects.iter().position(|r| p.x >= r.left() && p.x < r.right()) && over != idx {
                            drag_to = Some((idx, over));
                        }
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                }

                let plus = egui::Rect::from_center_size(egui::pos2(strip.right() + plus_w / 2.0, row.center().y), egui::vec2(TAB_HEIGHT, TAB_HEIGHT));
                let plus_id = ui.make_persistent_id("tab_new");
                let resp = ui.interact(plus, plus_id, egui::Sense::click()).on_hover_text(tr("New tab"));
                let fx = material::fx(ui, plus_id, &resp, true, &material::FxSpec::ICON);
                let r = material::scaled(plus, fx.scale);
                material::paint_fill(ui.painter(), r, material::pill(r), material::row_layer(&theme, &fx), &fx);
                material::paint_icon(ui.painter(), plus.center(), icons::ADD, material::ICON_MD, material::lerp_color(theme.subtext1, theme.text, fx.hover));
                let new_tab = resp.clicked();

                if let Some((from, to)) = drag_to {
                    self.move_tab(from, to);
                }
                if let Some(idx) = close {
                    self.close_tab(&ctx, idx);
                } else if let Some(idx) = switch_to {
                    self.switch_tab(idx);
                }
                if new_tab {
                    let here = self.current_path.clone();
                    self.new_tab(here);
                }
            });
    }

    /// Run a search again after a tab was shown; its results are not kept while it is parked.
    pub(super) fn resume_search(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.restart_search) {
            self.on_query_changed(ctx);
        }
    }
}

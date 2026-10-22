//! Compare folders: lists what differs between two folder trees and copies chosen items across.
//!
//! Every file is classed by its path below the two folders: only on one side, newer on one side,
//! different with equal times, or the same. The default test is size and modification time (two
//! seconds apart still counts as equal, since FAT keeps coarse times); "Compare contents" reads the
//! files whose sizes match. A folder that exists on one side only is one row. Links are compared by
//! where they point and never followed.
//!
//! This is "compare and copy", not a sync: nothing is ever deleted on its own. Copying goes through
//! the file operations, so name clashes ask, progress shows, and the copy can be undone.
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime};

use eframe::egui;

use super::progress::{OnWorker, Progress};
use super::{Job as AppJob, MenuItem, Plugin, Reaction};
use crate::fs::engine::{self, Effect, Op, Outcome};
use crate::fs::file_item::{FileItem, format_bytes};
use crate::fs::history::{Edit, UndoEntry};
use crate::fs::trash;
use crate::i18n::{tr, trf, trn};
use crate::ui::icons;
use crate::ui::material;
use crate::ui::theme::SerpantinumTheme;

/// Times this close count as equal.
const TOLERANCE: Duration = Duration::from_secs(2);
/// Rows are handed to the window in batches this big, so a huge tree shows up while it is read.
const BATCH: usize = 1000;
const ROW_HEIGHT: f32 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    OnlyLeft,
    OnlyRight,
    NewerLeft,
    NewerRight,
    /// Not the same, but neither is newer.
    Different,
    Same,
}

impl Class {
    const ALL: [Class; 6] = [Class::OnlyLeft, Class::OnlyRight, Class::NewerLeft, Class::NewerRight, Class::Different, Class::Same];

    fn index(self) -> usize {
        Class::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    fn label(self) -> &'static str {
        match self {
            Class::OnlyLeft => tr("Only on the left"),
            Class::OnlyRight => tr("Only on the right"),
            Class::NewerLeft => tr("Newer on the left"),
            Class::NewerRight => tr("Newer on the right"),
            Class::Different => tr("Different"),
            Class::Same => tr("Identical"),
        }
    }

    fn color(self, theme: &SerpantinumTheme) -> egui::Color32 {
        match self {
            Class::OnlyLeft | Class::OnlyRight => theme.blue,
            Class::NewerLeft | Class::NewerRight => theme.green,
            Class::Different => theme.red,
            Class::Same => theme.subtext1,
        }
    }

    /// Whether copying left to right is what this row calls for.
    fn goes_right(self) -> bool {
        matches!(self, Class::OnlyLeft | Class::NewerLeft)
    }

    fn goes_left(self) -> bool {
        matches!(self, Class::OnlyRight | Class::NewerRight)
    }
}

/// What one side has at a path.
#[derive(Debug, Clone, PartialEq)]
pub struct Side {
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The path below the two folders.
    pub rel: PathBuf,
    pub class: Class,
    pub left: Option<Side>,
    pub right: Option<Side>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Read files of equal size and compare what is in them.
    pub contents: bool,
}

struct Node {
    side: Side,
    /// Where a link points.
    link: Option<PathBuf>,
}

fn read_nodes(dir: &Path) -> BTreeMap<OsString, Node> {
    let mut nodes = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return nodes };
    for entry in entries.flatten() {
        let (Ok(kind), Ok(meta)) = (entry.file_type(), entry.metadata()) else { continue };
        let node = if kind.is_symlink() {
            Node { side: Side { size: 0, modified: meta.modified().ok(), is_dir: false }, link: Some(std::fs::read_link(entry.path()).unwrap_or_default()) }
        } else if kind.is_dir() {
            Node { side: Side { size: 0, modified: meta.modified().ok(), is_dir: true }, link: None }
        } else if kind.is_file() {
            Node { side: Side { size: meta.len(), modified: meta.modified().ok(), is_dir: false }, link: None }
        } else {
            // Sockets, pipes and devices have no contents to compare
            continue;
        };
        nodes.insert(entry.file_name(), node);
    }
    nodes
}

fn close(a: Option<SystemTime>, b: Option<SystemTime>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.duration_since(b).or_else(|e| Ok::<_, ()>(e.duration())).is_ok_and(|d| d <= TOLERANCE),
        (None, None) => true,
        _ => false,
    }
}

fn files_equal(a: &Path, b: &Path, progress: &dyn Progress) -> io::Result<bool> {
    let (mut fa, mut fb) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    let (mut ba, mut bb) = (vec![0u8; 256 * 1024], vec![0u8; 256 * 1024]);
    loop {
        progress.checkpoint()?;
        let na = read_full(&mut fa, &mut ba)?;
        let nb = read_full(&mut fb, &mut bb)?;
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
        progress.bytes(na as u64);
    }
}

fn read_full(file: &mut std::fs::File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// How two things of the same kind at one path relate.
fn classify(l: &Node, r: &Node, lp: &Path, rp: &Path, options: Options, progress: &dyn Progress) -> io::Result<Class> {
    if l.link.is_some() || r.link.is_some() {
        return Ok(if l.link == r.link { Class::Same } else { Class::Different });
    }
    let (same_size, same_time) = (l.side.size == r.side.size, close(l.side.modified, r.side.modified));
    if same_size && (if options.contents { files_equal(lp, rp, progress)? } else { same_time }) {
        return Ok(Class::Same);
    }
    Ok(if same_time {
        Class::Different
    } else if l.side.modified > r.side.modified {
        Class::NewerLeft
    } else {
        Class::NewerRight
    })
}

struct Walk<'a> {
    options: Options,
    progress: &'a dyn Progress,
    emit: &'a mut dyn FnMut(Entry),
}

fn walk_pair(w: &mut Walk, left: &Path, right: &Path, rel: &Path) -> io::Result<()> {
    w.progress.checkpoint()?;
    w.progress.current(&rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| left.display().to_string()));
    let (ln, rn) = (read_nodes(left), read_nodes(right));
    let names: std::collections::BTreeSet<&OsString> = ln.keys().chain(rn.keys()).collect();
    for name in names {
        let path = rel.join(name);
        let entry = |class, l: Option<&Node>, r: Option<&Node>| Entry { rel: path.clone(), class, left: l.map(|n| n.side.clone()), right: r.map(|n| n.side.clone()) };
        match (ln.get(name), rn.get(name)) {
            (Some(l), None) => (w.emit)(entry(Class::OnlyLeft, Some(l), None)),
            (None, Some(r)) => (w.emit)(entry(Class::OnlyRight, None, Some(r))),
            (Some(l), Some(r)) if l.side.is_dir && r.side.is_dir => walk_pair(w, &left.join(name), &right.join(name), &path)?,
            // A folder against a file
            (Some(l), Some(r)) if l.side.is_dir != r.side.is_dir => (w.emit)(entry(Class::Different, Some(l), Some(r))),
            (Some(l), Some(r)) => {
                let class = classify(l, r, &left.join(name), &right.join(name), w.options, w.progress)?;
                (w.emit)(entry(class, Some(l), Some(r)));
            }
            (None, None) => {}
        }
    }
    Ok(())
}

/// Walks both folders and hands every difference (and every identical item) to `emit`, sorted by
/// path within each folder.
pub fn compare(left: &Path, right: &Path, options: Options, progress: &dyn Progress, emit: &mut dyn FnMut(Entry)) -> io::Result<()> {
    let mut walk = Walk { options, progress, emit };
    walk_pair(&mut walk, left, right, Path::new(""))
}

enum Msg {
    Rows { generation: u64, rows: Vec<Entry> },
    Finished { generation: u64, result: Result<(), String>, cancelled: bool },
    /// Files changed, so the lists are read again.
    Refresh { generation: u64 },
}

struct View {
    generation: u64,
    left: String,
    right: String,
    /// The folders the rows were read from.
    scanned: Option<(PathBuf, PathBuf)>,
    cancel: Arc<AtomicBool>,
    scanning: bool,
    entries: Vec<Entry>,
    selected: HashSet<usize>,
    visible: Vec<usize>,
    show: [bool; 6],
    contents: bool,
    error: Option<String>,
    cancelled: bool,
    counts: [usize; 6],
}

impl View {
    fn new(generation: u64, left: String, right: String) -> Self {
        Self {
            generation,
            left,
            right,
            scanned: None,
            cancel: Arc::default(),
            scanning: false,
            entries: Vec::new(),
            selected: HashSet::new(),
            visible: Vec::new(),
            // Identical items are hidden until asked for
            show: [true, true, true, true, true, false],
            contents: false,
            error: None,
            cancelled: false,
            counts: [0; 6],
        }
    }

    fn refilter(&mut self) {
        self.visible = (0..self.entries.len()).filter(|&i| self.show[self.entries[i].class.index()]).collect();
    }

    fn add(&mut self, rows: Vec<Entry>) {
        for row in &rows {
            self.counts[row.class.index()] += 1;
        }
        let start = self.entries.len();
        self.entries.extend(rows);
        let show = self.show;
        let new: Vec<usize> = (start..self.entries.len()).filter(|&i| show[self.entries[i].class.index()]).collect();
        self.visible.extend(new);
    }

    /// `(source, target)` pairs for the selected rows that go from `from_left`'s side to the other.
    fn pairs(&self, from_left: bool) -> Vec<(PathBuf, PathBuf)> {
        let Some((left, right)) = &self.scanned else { return Vec::new() };
        let (from, to) = if from_left { (left, right) } else { (right, left) };
        let mut rows: Vec<&Entry> = self.selected.iter().filter_map(|&i| self.entries.get(i)).filter(|e| if from_left { e.left.is_some() } else { e.right.is_some() }).collect();
        rows.sort_by(|a, b| a.rel.cmp(&b.rel));
        rows.into_iter().map(|e| (from.join(&e.rel), to.join(&e.rel))).collect()
    }

    /// The selected paths that exist on one side.
    fn paths_on(&self, left: bool) -> Vec<PathBuf> {
        let Some((l, r)) = &self.scanned else { return Vec::new() };
        let base = if left { l } else { r };
        let mut rows: Vec<&Entry> = self.selected.iter().filter_map(|&i| self.entries.get(i)).filter(|e| if left { e.left.is_some() } else { e.right.is_some() }).collect();
        rows.sort_by(|a, b| a.rel.cmp(&b.rel));
        rows.into_iter().map(|e| base.join(&e.rel)).collect()
    }
}

pub struct Compare {
    view: Option<View>,
    generation: u64,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    jobs: Vec<AppJob>,
}

impl Default for Compare {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { view: None, generation: 0, tx, rx, jobs: Vec::new() }
    }
}

fn expand(text: &str) -> PathBuf {
    let text = text.trim();
    match (text.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(text),
    }
}

impl Compare {
    fn open(&mut self, left: &Path, right: Option<&Path>) {
        self.generation += 1;
        if let Some(old) = &self.view {
            old.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let mut view = View::new(self.generation, left.display().to_string(), right.map(|r| r.display().to_string()).unwrap_or_default());
        if let Some(old) = self.view.take() {
            view.contents = old.contents;
        }
        self.view = Some(view);
        if right.is_some() {
            self.scan();
        }
    }

    /// Reads both folders again (a new job; the one still running is stopped).
    fn scan(&mut self) {
        let Some(view) = self.view.as_mut() else { return };
        view.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let (left, right) = (expand(&view.left), expand(&view.right));
        view.entries.clear();
        view.visible.clear();
        view.selected.clear();
        view.counts = [0; 6];
        view.error = None;
        view.cancelled = false;
        view.scanned = None;
        if !left.is_dir() || !right.is_dir() {
            view.error = Some(trf("“{}” is not a folder", &[&if left.is_dir() { right.display() } else { left.display() }]));
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        view.generation = generation;
        view.cancel = Arc::new(AtomicBool::new(false));
        view.scanning = true;
        view.scanned = Some((left.clone(), right.clone()));
        let (tx, cancel, options) = (self.tx.clone(), Arc::clone(&view.cancel), Options { contents: view.contents });
        let title = trf("Comparing {} and {}", &[&name_of(&left), &name_of(&right)]);
        self.jobs.push(AppJob::new(title, move |w| {
            let progress = OnWorker { worker: w, cancel: &cancel };
            let mut batch = Vec::new();
            let result = {
                let mut emit = |entry: Entry| {
                    batch.push(entry);
                    if batch.len() >= BATCH {
                        let _ = tx.send(Msg::Rows { generation, rows: std::mem::take(&mut batch) });
                    }
                };
                compare(&left, &right, options, &progress, &mut emit)
            };
            let _ = tx.send(Msg::Rows { generation, rows: batch });
            let cancelled = result.as_ref().is_err_and(|e| e.kind() == io::ErrorKind::Interrupted);
            let outcome = match &result {
                Ok(()) => Outcome::done(tr("Comparison finished").to_owned()),
                Err(_) if cancelled => Outcome::done(tr("Cancelled").to_owned()),
                Err(e) => Outcome::failed(e.to_string()),
            };
            let _ = tx.send(Msg::Finished { generation, result: result.map_err(|e| e.to_string()), cancelled });
            outcome
        }));
    }

    fn copy(&mut self, from_left: bool) {
        let Some(view) = &self.view else { return };
        let pairs = view.pairs(from_left);
        if pairs.is_empty() {
            return;
        }
        let (tx, generation) = (self.tx.clone(), view.generation);
        let title = trn(pairs.len(), "Copying {} item to the other folder", "Copying {} items to the other folder");
        self.jobs.push(AppJob::new(title, move |w| {
            let outcome = engine::transfer(w, Op::Copy, pairs);
            let _ = tx.send(Msg::Refresh { generation });
            outcome
        }));
    }

    fn trash(&mut self, left: bool) {
        let Some(view) = &self.view else { return };
        let paths = view.paths_on(left);
        if paths.is_empty() {
            return;
        }
        let (tx, generation) = (self.tx.clone(), view.generation);
        let title = trn(paths.len(), "Moving {} item to the trash", "Moving {} items to the trash");
        let removes = paths.clone();
        self.jobs.push(
            AppJob::new(title, move |w| {
                w.shared.files_total.store(paths.len() as u64, std::sync::atomic::Ordering::Relaxed);
                let (mut done, mut failed, mut stopped) = (Vec::new(), 0, false);
                for path in &paths {
                    if w.shared.checkpoint().is_err() {
                        stopped = true;
                        break;
                    }
                    w.shared.set_current(&name_of(path));
                    match trash::trash(path) {
                        Ok(to) => done.push(to),
                        Err(_) => failed += 1,
                    }
                    w.file_done();
                }
                let _ = tx.send(Msg::Refresh { generation });
                let mut message = trn(done.len(), "Moved {} item to the trash", "Moved {} items to the trash");
                if failed > 0 {
                    message.push_str(&format!(" · {}", trf("{} failed", &[&failed])));
                }
                let label = trn(done.len(), "{} item", "{} items");
                let effect = if done.is_empty() { Effect::None } else { Effect::Record(UndoEntry::new(trf("Move of {} to trash", &[&label]), done.into_iter().map(Edit::Restore).collect())) };
                let mut outcome = Outcome::done(message).with(effect);
                outcome.cancelled = stopped;
                outcome
            })
            .removing(removes),
        );
    }
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn side_text(side: &Option<Side>) -> String {
    match side {
        None => String::new(),
        Some(s) if s.is_dir => tr("Folder").to_owned(),
        Some(s) => format_bytes(s.size),
    }
}

impl Plugin for Compare {
    fn id(&self) -> &'static str {
        "compare"
    }

    fn name(&self) -> &'static str {
        tr("Compare folders")
    }

    fn description(&self) -> &'static str {
        tr("Lists what differs between two folders and copies the chosen items across; nothing is deleted on its own")
    }

    fn icon(&self) -> &'static str {
        icons::SYNC
    }

    fn busy(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.scanning)
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        for msg in self.rx.try_iter().collect::<Vec<_>>() {
            let Some(view) = self.view.as_mut() else { continue };
            match msg {
                Msg::Rows { generation, rows } if generation == view.generation => {
                    view.add(rows);
                    changed = true;
                }
                Msg::Finished { generation, result, cancelled } if generation == view.generation => {
                    view.scanning = false;
                    view.cancelled = cancelled;
                    view.error = result.err();
                    changed = true;
                }
                Msg::Refresh { generation } if generation == view.generation && view.scanned.is_some() => {
                    self.scan();
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }

    fn menu_items(&self, selection: &[FileItem]) -> Vec<MenuItem> {
        match selection {
            [a, b] if a.is_dir && b.is_dir && !a.is_symlink && !b.is_symlink => vec![MenuItem::new("compare", tr("Compare"), icons::SYNC)],
            _ => Vec::new(),
        }
    }

    fn folder_menu_items(&self, _here: &Path, others: &[PathBuf]) -> Vec<MenuItem> {
        let mut items = vec![MenuItem::new("compare_pick", tr("Compare folders…"), icons::SYNC)];
        if !others.is_empty() {
            items.insert(0, MenuItem::new("compare_tab", tr("Compare with the other tab"), icons::SYNC));
        }
        items
    }

    fn run(&mut self, id: &str, paths: &[PathBuf], _dir: &Path) -> Reaction {
        match (id, paths) {
            ("compare", [a, b]) | ("compare_tab", [a, b, ..]) => self.open(a, Some(b)),
            ("compare_pick", [here, ..]) => self.open(here, None),
            _ => {}
        }
        Reaction::None
    }

    fn reaction(&mut self) -> Reaction {
        if self.jobs.is_empty() { Reaction::None } else { Reaction::Job(self.jobs.remove(0)) }
    }

    fn window_title(&self) -> Option<&'static str> {
        self.view.as_ref().map(|_| tr("Compare folders"))
    }

    fn close_window(&mut self) {
        if let Some(view) = self.view.take() {
            view.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn window(&mut self, ui: &mut egui::Ui, theme: &SerpantinumTheme) -> bool {
        let Some(view) = self.view.as_mut() else { return false };
        let (mut rescan, mut copy_right, mut copy_left, mut trash_left, mut trash_right, mut open) = (false, false, false, false, false, false);

        for (label, text, id) in [(tr("Left folder"), 0, "left"), (tr("Right folder"), 1, "right")].map(|(l, t, i)| (l, t, i)) {
            ui.horizontal(|ui| {
                ui.add_sized([96.0, 30.0], egui::Label::new(egui::RichText::new(label).size(material::FONT_CAPTION).color(theme.subtext1)));
                material::input_frame(theme).show(ui, |ui| {
                    let field = if text == 0 { &mut view.left } else { &mut view.right };
                    let response = ui.add(egui::TextEdit::singleline(field).id_salt(("compare_path", id)).frame(egui::Frame::NONE).desired_width(f32::INFINITY).font(egui::FontId::proportional(material::FONT_BODY)));
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        rescan = true;
                    }
                });
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if material::button(ui, "compare_go", Some(icons::SYNC), tr("Compare"), theme, material::ButtonKind::Filled, true).clicked() {
                rescan = true;
            }
            let on = view.contents;
            ui.add_space(6.0);
            if material::chip(ui, "compare_contents", None, tr("Compare contents"), theme, on).clicked() {
                view.contents = !on;
                rescan = view.scanned.is_some();
            }
        });

        if let Some(error) = &view.error {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(error).size(material::FONT_BODY).color(theme.red));
        }
        if view.scanned.is_none() {
            if view.error.is_none() {
                material::hint_label(ui, tr("Choose two folders and press Compare"), theme);
            }
            if rescan {
                self.scan();
            }
            return true;
        }

        ui.add_space(6.0);
        let mut refilter = false;
        ui.horizontal_wrapped(|ui| {
            for class in Class::ALL {
                let i = class.index();
                if material::chip(ui, ("compare_show", i), None, &format!("{} {}", class.label(), view.counts[i]), theme, view.show[i]).clicked() {
                    view.show[i] = !view.show[i];
                    refilter = true;
                }
            }
        });
        if refilter {
            view.refilter();
        }
        if view.scanning {
            ui.label(egui::RichText::new(tr("Comparing…")).size(material::FONT_CAPTION).color(theme.subtext1));
        } else if view.cancelled {
            ui.label(egui::RichText::new(tr("Cancelled, the list is incomplete")).size(material::FONT_CAPTION).color(theme.subtext1));
        } else if view.visible.is_empty() {
            let different = view.counts[..5].iter().sum::<usize>();
            material::hint_label(ui, if different == 0 { tr("The folders are identical") } else { tr("Nothing to show with these filters") }, theme);
        }
        ui.add_space(4.0);

        let mut toggled: Option<(usize, bool)> = None;
        let (info_w, class_w) = (76.0, 128.0);
        egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show_rows(ui, ROW_HEIGHT, view.visible.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for &index in &view.visible[range] {
                let entry = &view.entries[index];
                let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::hover());
                let on = view.selected.contains(&index);
                let box_rect = egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 0.0), egui::vec2(30.0, ROW_HEIGHT));
                let clicked = ui.scope_builder(egui::UiBuilder::new().max_rect(box_rect), |ui| material::checkbox(ui, ("compare_pick", index), on, true, theme)).inner.clicked();
                if clicked {
                    toggled = Some((index, !on));
                }
                let y = rect.center().y;
                let mut x = rect.left() + 40.0;
                material::paint_label(ui, x, y, entry.class.label(), material::FONT_CAPTION, entry.class.color(theme), class_w - 8.0);
                x += class_w;
                let path_w = rect.right() - x - info_w * 2.0 - 12.0;
                material::paint_label(ui, x, y, &entry.rel.display().to_string(), material::FONT_BODY, theme.text, path_w);
                material::paint_label(ui, rect.right() - info_w * 2.0 - 4.0, y, &side_text(&entry.left), material::FONT_CAPTION, theme.subtext1, info_w - 6.0);
                material::paint_label(ui, rect.right() - info_w - 4.0, y, &side_text(&entry.right), material::FONT_CAPTION, theme.subtext1, info_w - 6.0);
                let dates = |side: &Option<Side>| side.as_ref().map_or_else(|| "—".to_owned(), |s| crate::fs::props::format_datetime(s.modified));
                response.on_hover_text(format!("{}: {}\n{}: {}", tr("Left"), dates(&entry.left), tr("Right"), dates(&entry.right)));
            }
        });
        if let Some((index, on)) = toggled {
            if on {
                view.selected.insert(index);
            } else {
                view.selected.remove(&index);
            }
        }

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if material::button(ui, "compare_pick_right", None, tr("Pick what goes right"), theme, material::ButtonKind::Text, !view.entries.is_empty()).clicked() {
                view.selected = (0..view.entries.len()).filter(|&i| view.entries[i].class.goes_right()).collect();
            }
            if material::button(ui, "compare_pick_left", None, tr("Pick what goes left"), theme, material::ButtonKind::Text, !view.entries.is_empty()).clicked() {
                view.selected = (0..view.entries.len()).filter(|&i| view.entries[i].class.goes_left()).collect();
            }
            if material::button(ui, "compare_pick_none", None, tr("Select none"), theme, material::ButtonKind::Text, !view.selected.is_empty()).clicked() {
                view.selected.clear();
            }
        });
        let (on_left, on_right) = (!view.paths_on(true).is_empty(), !view.paths_on(false).is_empty());
        ui.horizontal_wrapped(|ui| {
            copy_right = material::button(ui, "compare_copy_right", Some(icons::ARROW_FORWARD), tr("Copy to the right"), theme, material::ButtonKind::Filled, on_left).clicked() && on_left;
            copy_left = material::button(ui, "compare_copy_left", Some(icons::ARROW_BACK), tr("Copy to the left"), theme, material::ButtonKind::Filled, on_right).clicked() && on_right;
            trash_left = material::button(ui, "compare_trash_left", Some(icons::TRASH), tr("Trash the left copy"), theme, material::ButtonKind::Tonal, on_left).clicked() && on_left;
            trash_right = material::button(ui, "compare_trash_right", Some(icons::TRASH), tr("Trash the right copy"), theme, material::ButtonKind::Tonal, on_right).clicked() && on_right;
            open = material::button(ui, "compare_open", Some(icons::OPEN), tr("Open"), theme, material::ButtonKind::Tonal, view.selected.len() == 1).clicked() && view.selected.len() == 1;
        });

        if open {
            // The left copy, or the right one when that is all there is
            let path = view.paths_on(true).into_iter().next().or_else(|| view.paths_on(false).into_iter().next());
            if let Some(path) = path {
                let _ = open::that_detached(path);
            }
        }
        if rescan {
            self.scan();
        } else if copy_right {
            self.copy(true);
        } else if copy_left {
            self.copy(false);
        } else if trash_left {
            self.trash(true);
        } else if trash_right {
            self.trash(false);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Quiet;

    impl Progress for Quiet {
        fn checkpoint(&self) -> io::Result<()> {
            Ok(())
        }
        fn current(&self, _: &str) {}
        fn plan(&self, _: u64, _: u64) {}
        fn bytes(&self, _: u64) {}
        fn file_done(&self) {}
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ski-compare-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: &Path, text: &str, age_secs: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        let time = SystemTime::now() - Duration::from_secs(age_secs);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(time).unwrap();
    }

    fn run(left: &Path, right: &Path, options: Options) -> Vec<(String, Class)> {
        let mut rows = Vec::new();
        compare(left, right, options, &Quiet, &mut |e| rows.push((e.rel.display().to_string(), e.class))).unwrap();
        rows
    }

    #[test]
    fn every_class_is_found() {
        let base = folder("classes");
        let (l, r) = (base.join("l"), base.join("r"));
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        put(&l.join("only-left.txt"), "a", 100);
        put(&r.join("only-right.txt"), "b", 100);
        put(&l.join("same.txt"), "same", 500);
        put(&r.join("same.txt"), "same", 500);
        put(&l.join("newer-left.txt"), "new", 10);
        put(&r.join("newer-left.txt"), "new", 5000);
        put(&l.join("newer-right.txt"), "old", 5000);
        put(&r.join("newer-right.txt"), "new!", 10);
        put(&l.join("different.txt"), "aaaa", 300);
        put(&r.join("different.txt"), "bbb", 300);
        // A folder on one side is one row, however much is in it
        put(&l.join("lonely/deep/file.txt"), "x", 100);
        put(&l.join("lonely/other.txt"), "y", 100);
        // Folders on both sides are walked
        put(&l.join("both/a.txt"), "a", 100);
        put(&r.join("both/a.txt"), "a", 100);
        put(&r.join("both/extra.txt"), "e", 100);
        // Links compare by where they point
        std::os::unix::fs::symlink("target-one", l.join("link-same")).unwrap();
        std::os::unix::fs::symlink("target-one", r.join("link-same")).unwrap();
        std::os::unix::fs::symlink("target-one", l.join("link-diff")).unwrap();
        std::os::unix::fs::symlink("target-two", r.join("link-diff")).unwrap();

        let rows = run(&l, &r, Options::default());
        let class_of = |name: &str| rows.iter().find(|(n, _)| n == name).map(|(_, c)| *c);
        assert_eq!(class_of("only-left.txt"), Some(Class::OnlyLeft));
        assert_eq!(class_of("only-right.txt"), Some(Class::OnlyRight));
        assert_eq!(class_of("same.txt"), Some(Class::Same));
        assert_eq!(class_of("newer-left.txt"), Some(Class::NewerLeft));
        assert_eq!(class_of("newer-right.txt"), Some(Class::NewerRight));
        assert_eq!(class_of("different.txt"), Some(Class::Different));
        assert_eq!(class_of("lonely"), Some(Class::OnlyLeft));
        assert!(rows.iter().all(|(n, _)| !n.starts_with("lonely/")), "a one-sided folder is a single row");
        assert_eq!(class_of("both/a.txt"), Some(Class::Same));
        assert_eq!(class_of("both/extra.txt"), Some(Class::OnlyRight));
        assert_eq!(class_of("both"), None, "a folder both sides have is not a row of its own");
        assert_eq!(class_of("link-same"), Some(Class::Same));
        assert_eq!(class_of("link-diff"), Some(Class::Different));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn times_a_second_apart_are_equal_but_five_are_not() {
        let base = folder("tolerance");
        let (l, r) = (base.join("l"), base.join("r"));
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        let now = SystemTime::now() - Duration::from_secs(1000);
        for (dir, name, shift) in [(&l, "near", 0), (&r, "near", 1), (&l, "far", 0), (&r, "far", 5)] {
            let path = dir.join(name);
            std::fs::write(&path, "same size").unwrap();
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(now + Duration::from_secs(shift)).unwrap();
        }
        let rows = run(&l, &r, Options::default());
        assert_eq!(rows, vec![("far".to_owned(), Class::NewerRight), ("near".to_owned(), Class::Same)]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn comparing_contents_sees_through_equal_sizes_and_different_times() {
        let base = folder("contents");
        let (l, r) = (base.join("l"), base.join("r"));
        put(&l.join("copy.txt"), "same bytes", 5000);
        put(&r.join("copy.txt"), "same bytes", 10);
        put(&l.join("edited.txt"), "abcdefgh", 300);
        put(&r.join("edited.txt"), "abcdefgX", 300);
        // By size and time the first pair is newer on the right; by contents it is the same
        let quick = run(&l, &r, Options::default());
        assert_eq!(quick, vec![("copy.txt".to_owned(), Class::NewerRight), ("edited.txt".to_owned(), Class::Same)]);
        let deep = run(&l, &r, Options { contents: true });
        assert_eq!(deep, vec![("copy.txt".to_owned(), Class::Same), ("edited.txt".to_owned(), Class::Different)]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn selected_rows_become_copy_pairs() {
        let (l, r) = (PathBuf::from("/l"), PathBuf::from("/r"));
        let side = Some(Side { size: 1, modified: None, is_dir: false });
        let mut view = View::new(1, String::new(), String::new());
        view.scanned = Some((l, r));
        view.add(vec![
            Entry { rel: "b/only-left".into(), class: Class::OnlyLeft, left: side.clone(), right: None },
            Entry { rel: "a/only-right".into(), class: Class::OnlyRight, left: None, right: side.clone() },
            Entry { rel: "c/both".into(), class: Class::NewerLeft, left: side.clone(), right: side },
        ]);
        view.selected = HashSet::from([0, 1, 2]);
        assert_eq!(view.pairs(true), vec![("/l/b/only-left".into(), "/r/b/only-left".into()), ("/l/c/both".into(), "/r/c/both".into())]);
        assert_eq!(view.pairs(false), vec![("/r/a/only-right".into(), "/l/a/only-right".into()), ("/r/c/both".into(), "/l/c/both".into())]);
        assert_eq!(view.paths_on(true), vec![PathBuf::from("/l/b/only-left"), PathBuf::from("/l/c/both")]);
        // Identical items are hidden by default but counted
        view.add(vec![Entry { rel: "same".into(), class: Class::Same, left: None, right: None }]);
        assert_eq!(view.visible.len(), 3);
        assert_eq!(view.counts[Class::Same.index()], 1);
        view.show[Class::Same.index()] = true;
        view.refilter();
        assert_eq!(view.visible.len(), 4);
    }

    #[test]
    fn copying_across_goes_through_the_file_operations() {
        let base = folder("copy");
        let (l, r) = (base.join("l"), base.join("r"));
        put(&l.join("new/inner.txt"), "inner", 10);
        put(&l.join("file.txt"), "file", 10);
        std::fs::create_dir_all(&r).unwrap();
        let mut compare_plugin = Compare::default();
        compare_plugin.open(&l, Some(&r));
        // The job is queued for the app; run it the way the app would
        let Reaction::Job(job) = compare_plugin.reaction() else { panic!("no scan job") };
        let mut engine = crate::fs::engine::Engine::new(&egui::Context::default());
        engine.spawn(crate::fs::engine::JobKind::Other, job.title, String::new(), job.work);
        for _ in 0..500 {
            engine.poll();
            compare_plugin.poll();
            if !engine.running() && !compare_plugin.busy() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let view = compare_plugin.view.as_mut().unwrap();
        assert_eq!(view.entries.len(), 2);
        view.selected = (0..view.entries.len()).collect();
        compare_plugin.copy(true);
        let Reaction::Job(job) = compare_plugin.reaction() else { panic!("no copy job") };
        engine.spawn(crate::fs::engine::JobKind::Copy, job.title, String::new(), job.work);
        for _ in 0..500 {
            engine.poll();
            if !engine.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(r.join("new/inner.txt")).unwrap(), "inner");
        assert_eq!(std::fs::read_to_string(r.join("file.txt")).unwrap(), "file");
        // The copy asked for a fresh comparison
        compare_plugin.poll();
        assert!(matches!(compare_plugin.reaction(), Reaction::Job(_)));
        let _ = std::fs::remove_dir_all(&base);
    }
}

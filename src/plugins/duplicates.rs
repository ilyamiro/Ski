//! Duplicate finder: groups files with identical contents so the extra copies can go to the trash.
//!
//! The scan runs as a file operation (progress, pause and cancel in the operations list) and sends
//! its groups to the plugin's window. To keep the reading low it goes in steps: files of a size seen
//! once are dropped, then the first and last 4 KiB are compared with a quick hash, and only what is
//! still alike is read whole and hashed with SHA-256. Files with the same inode (hard links) are one
//! file, links are not followed and empty files are ignored.
//!
//! Nothing is selected for removal until you choose. One copy of every group always stays, and
//! before a file is trashed it must still have the size and modification time it had in the scan.
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::SystemTime;

use eframe::egui;

use super::checksum::Sha256;
use super::{Job as AppJob, MenuItem, Plugin, Reaction};
use crate::fs::engine::{Effect, Outcome, Worker};
use crate::fs::file_item::{FileItem, format_bytes};
use crate::fs::history::{Edit, UndoEntry};
use crate::fs::trash;
use crate::i18n::{tr, trf, trn};
use crate::ui::icons;
use crate::ui::material;
use crate::ui::theme::SerpantinumTheme;

/// How much of the start and the end of a file the quick comparison reads.
const SAMPLE: usize = 4096;
/// Files are hashed on this many threads.
const THREADS: usize = 2;
const ROW_HEIGHT: f32 = 30.0;

/// One of the equal files, as the scan saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Member {
    /// Whether the file on disk is still the one the scan saw.
    fn unchanged(&self) -> bool {
        std::fs::symlink_metadata(&self.path).is_ok_and(|m| m.is_file() && m.len() == self.size && m.modified().ok() == self.modified)
    }
}

/// Files with identical contents.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub size: u64,
    pub files: Vec<Member>,
}

impl Group {
    /// What removing all but one copy gives back.
    pub fn reclaimable(&self) -> u64 {
        self.size * (self.files.len() as u64 - 1)
    }
}

/// Which copy of each group stays when a rule picks the ones to remove.
#[derive(Debug, Clone, PartialEq)]
pub enum Keep {
    Newest,
    Oldest,
    ShortestPath,
    /// The copy inside this folder; groups without one keep their shortest path.
    InFolder(PathBuf),
}

fn shortest(group: &Group) -> usize {
    (0..group.files.len()).min_by_key(|&i| (group.files[i].path.as_os_str().len(), group.files[i].path.clone())).unwrap_or(0)
}

/// The index of the copy `rule` keeps.
pub fn keeper(group: &Group, rule: &Keep) -> usize {
    let time = |i: usize| group.files[i].modified.unwrap_or(SystemTime::UNIX_EPOCH);
    // Ties go to the shorter path, then to the lower one
    let tie = |i: usize| (group.files[i].path.as_os_str().len(), group.files[i].path.clone());
    let by_time = |newest: bool| {
        (0..group.files.len())
            .min_by(|&a, &b| {
                let order = if newest { time(b).cmp(&time(a)) } else { time(a).cmp(&time(b)) };
                order.then_with(|| tie(a).cmp(&tie(b)))
            })
            .unwrap_or(0)
    };
    match rule {
        Keep::Newest => by_time(true),
        Keep::Oldest => by_time(false),
        Keep::ShortestPath => shortest(group),
        Keep::InFolder(dir) => group.files.iter().position(|m| m.path.starts_with(dir)).unwrap_or_else(|| shortest(group)),
    }
}

/// What a scan or a removal tells the job it runs in: pause and cancel points, and progress.
pub trait Progress: Sync {
    fn checkpoint(&self) -> io::Result<()>;
    fn current(&self, name: &str);
    /// A new stage: how many files and bytes it will go through; counting starts again.
    fn plan(&self, files: u64, bytes: u64);
    fn bytes(&self, n: u64);
    fn file_done(&self);
}

/// The progress of a file operation, stopped early when the window that asked for it was closed.
struct OnWorker<'a> {
    worker: &'a Worker,
    cancel: &'a AtomicBool,
}

impl Progress for OnWorker<'_> {
    fn checkpoint(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        self.worker.shared.checkpoint()
    }

    fn current(&self, name: &str) {
        self.worker.shared.set_current(name);
    }

    fn plan(&self, files: u64, bytes: u64) {
        let shared = &self.worker.shared;
        shared.files_done.store(0, Ordering::Relaxed);
        shared.bytes_done.store(0, Ordering::Relaxed);
        shared.files_total.store(files, Ordering::Relaxed);
        shared.bytes_total.store(bytes, Ordering::Relaxed);
    }

    fn bytes(&self, n: u64) {
        self.worker.add_bytes(n);
    }

    fn file_done(&self) {
        self.worker.file_done();
    }
}

struct Candidate {
    path: PathBuf,
    dev: u64,
    ino: u64,
    size: u64,
    modified: Option<SystemTime>,
}

/// Every regular file below `roots`, grouped by size. Links are not followed, empty files are
/// skipped, and a file reached twice (hard links, overlapping roots) counts once.
fn walk(roots: &[PathBuf], progress: &dyn Progress) -> io::Result<HashMap<u64, Vec<Candidate>>> {
    let mut by_size: HashMap<u64, Vec<Candidate>> = HashMap::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        progress.checkpoint()?;
        progress.current(&dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string()));
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file()
                && let Ok(meta) = entry.metadata()
                && meta.len() > 0
            {
                by_size.entry(meta.len()).or_default().push(Candidate { path: entry.path(), dev: meta.dev(), ino: meta.ino(), size: meta.len(), modified: meta.modified().ok() });
            }
        }
    }
    // A file reached twice (hard links, overlapping roots) stays under its lowest path
    for files in by_size.values_mut() {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut seen: HashSet<(u64, u64)> = HashSet::new();
        files.retain(|c| seen.insert((c.dev, c.ino)));
    }
    by_size.retain(|_, files| files.len() > 1);
    Ok(by_size)
}

/// FNV-1a, enough to tell files apart cheaply.
fn quick_hash(bytes: &[u8], mut hash: u64) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The first and last `SAMPLE` bytes of the file, hashed.
fn sample_hash(path: &Path, size: u64) -> io::Result<u64> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; SAMPLE];
    let mut hash = 0xcbf29ce484222325;
    let n = read_full(&mut file, &mut buffer)?;
    hash = quick_hash(&buffer[..n], hash);
    if size > 2 * SAMPLE as u64 {
        file.seek(SeekFrom::Start(size - SAMPLE as u64))?;
        let n = read_full(&mut file, &mut buffer)?;
        hash = quick_hash(&buffer[..n], hash);
    }
    Ok(hash)
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

fn full_hash(path: &Path, progress: &dyn Progress) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        progress.checkpoint()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok(hasher.finish());
        }
        hasher.update(&buffer[..n]);
        progress.bytes(n as u64);
    }
}

/// Runs `f` on every item with `THREADS` threads, in order of the list. A file that can't be read
/// gives `None`; cancelling gives an error.
fn par_map<T: Sync, R: Send>(items: &[T], progress: &dyn Progress, f: impl Fn(&T) -> io::Result<R> + Sync) -> io::Result<Vec<Option<R>>> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Option<R>)>> = Mutex::new(Vec::with_capacity(items.len()));
    let stopped: Mutex<Option<io::Error>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..THREADS.min(items.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= items.len() || stopped.lock().is_ok_and(|s| s.is_some()) {
                        return;
                    }
                    if let Err(e) = progress.checkpoint() {
                        *stopped.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                        return;
                    }
                    let value = match f(&items[i]) {
                        Ok(v) => Some(v),
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                            *stopped.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                            return;
                        }
                        Err(_) => None,
                    };
                    progress.file_done();
                    results.lock().unwrap_or_else(|e| e.into_inner()).push((i, value));
                }
            });
        }
    });
    if let Some(e) = stopped.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(e);
    }
    let mut results = results.into_inner().unwrap_or_else(|e| e.into_inner());
    results.sort_by_key(|(i, _)| *i);
    Ok(results.into_iter().map(|(_, v)| v).collect())
}

/// Finds the files below `roots` that have identical contents.
pub fn find(roots: &[PathBuf], progress: &dyn Progress) -> io::Result<Vec<Group>> {
    let by_size = walk(roots, progress)?;
    // Reading in inode order keeps a spinning disk from jumping around
    let mut candidates: Vec<Candidate> = by_size.into_values().flatten().collect();
    candidates.sort_by_key(|c| (c.dev, c.ino));

    progress.plan(candidates.len() as u64, 0);
    progress.current(tr("Comparing the start and end of files"));
    let samples = par_map(&candidates, progress, |c| sample_hash(&c.path, c.size))?;
    let mut by_sample: HashMap<(u64, u64), Vec<Candidate>> = HashMap::new();
    for (candidate, sample) in candidates.into_iter().zip(samples) {
        if let Some(sample) = sample {
            by_sample.entry((candidate.size, sample)).or_default().push(candidate);
        }
    }
    let mut candidates: Vec<Candidate> = by_sample.into_values().filter(|files| files.len() > 1).flatten().collect();
    candidates.sort_by_key(|c| (c.dev, c.ino));

    progress.plan(candidates.len() as u64, candidates.iter().map(|c| c.size).sum());
    progress.current(tr("Reading the files that still look alike"));
    let hashes = par_map(&candidates, progress, |c| full_hash(&c.path, progress))?;
    let mut by_hash: HashMap<(u64, String), Vec<Member>> = HashMap::new();
    for (candidate, hash) in candidates.into_iter().zip(hashes) {
        if let Some(hash) = hash {
            by_hash.entry((candidate.size, hash)).or_default().push(Member { path: candidate.path, size: candidate.size, modified: candidate.modified });
        }
    }
    let mut groups: Vec<Group> = by_hash
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|((size, _), mut files)| {
            files.sort_by(|a, b| a.path.cmp(&b.path));
            Group { size, files }
        })
        .collect();
    groups.sort_by(|a, b| b.reclaimable().cmp(&a.reclaimable()).then_with(|| a.files[0].path.cmp(&b.files[0].path)));
    Ok(groups)
}

/// Copies to remove from one group, with the ones that stay.
#[derive(Debug, Clone, PartialEq)]
struct Planned {
    keepers: Vec<Member>,
    doomed: Vec<Member>,
}

/// Trashes what `plans` lists. A group is skipped when none of its keepers is still the file the
/// scan saw, and a copy that changed since is left alone. Returns `(original, trashed)` pairs and
/// how many were skipped, and whether it was stopped early. `remove` does the trashing.
fn trash_planned(plans: &[Planned], progress: &dyn Progress, remove: impl Fn(&Path) -> io::Result<PathBuf>) -> (Vec<(PathBuf, PathBuf)>, usize, bool) {
    let (mut trashed, mut skipped) = (Vec::new(), 0);
    for plan in plans {
        if !plan.keepers.iter().any(Member::unchanged) {
            skipped += plan.doomed.len();
            continue;
        }
        for member in &plan.doomed {
            if progress.checkpoint().is_err() {
                return (trashed, skipped, true);
            }
            progress.current(&member.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            if !member.unchanged() {
                skipped += 1;
                continue;
            }
            match remove(&member.path) {
                Ok(to) => trashed.push((member.path.clone(), to)),
                Err(_) => skipped += 1,
            }
            progress.file_done();
        }
    }
    (trashed, skipped, false)
}

enum Msg {
    Found { generation: u64, result: Result<Vec<Group>, String>, cancelled: bool },
    Trashed { generation: u64, paths: Vec<PathBuf> },
}

#[derive(PartialEq)]
enum Phase {
    Scanning,
    Done,
}

enum Row {
    Group(usize),
    File(usize, usize),
}

struct View {
    generation: u64,
    roots: Vec<PathBuf>,
    cancel: Arc<AtomicBool>,
    phase: Phase,
    groups: Vec<Group>,
    selected: HashSet<PathBuf>,
    rows: Vec<Row>,
    error: Option<String>,
    cancelled: bool,
}

impl View {
    fn rebuild_rows(&mut self) {
        self.rows = self.groups.iter().enumerate().flat_map(|(g, group)| std::iter::once(Row::Group(g)).chain((0..group.files.len()).map(move |f| Row::File(g, f)))).collect();
    }

    /// Selects every copy but the one `rule` keeps, in each group.
    fn select_all_but(&mut self, rule: &Keep) {
        self.selected.clear();
        for group in &self.groups {
            let stays = keeper(group, rule);
            self.selected.extend(group.files.iter().enumerate().filter(|(i, _)| *i != stays).map(|(_, m)| m.path.clone()));
        }
    }

    /// Whether `member` of group `g` may be ticked: one copy must stay unticked.
    fn can_select(&self, g: usize, member: &Member) -> bool {
        self.selected.contains(&member.path) || self.groups[g].files.iter().filter(|m| m.path != member.path && !self.selected.contains(&m.path)).count() > 0
    }

    /// The removal the ticks stand for. Even if every copy of a group were ticked, one stays.
    fn plan(&self) -> Vec<Planned> {
        self.groups
            .iter()
            .filter_map(|group| {
                let mut doomed: Vec<Member> = group.files.iter().filter(|m| self.selected.contains(&m.path)).cloned().collect();
                if doomed.is_empty() {
                    return None;
                }
                if doomed.len() == group.files.len() {
                    doomed.remove(0);
                }
                let keepers = group.files.iter().filter(|m| !doomed.contains(m)).cloned().collect();
                Some(Planned { keepers, doomed })
            })
            .collect()
    }

    fn selected_size(&self) -> (usize, u64) {
        self.plan().iter().flat_map(|p| &p.doomed).fold((0, 0), |(n, bytes), m| (n + 1, bytes + m.size))
    }

    fn forget(&mut self, gone: &[PathBuf]) {
        let gone: HashSet<&PathBuf> = gone.iter().collect();
        for group in &mut self.groups {
            group.files.retain(|m| !gone.contains(&m.path));
        }
        self.groups.retain(|g| g.files.len() > 1);
        self.selected.retain(|p| !gone.contains(p));
        self.rebuild_rows();
    }
}

pub struct Duplicates {
    view: Option<View>,
    generation: u64,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    job: Option<AppJob>,
    messages: Vec<String>,
}

impl Default for Duplicates {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { view: None, generation: 0, tx, rx, job: None, messages: Vec::new() }
    }
}

impl Duplicates {
    fn start(&mut self, roots: Vec<PathBuf>) -> Reaction {
        if roots.is_empty() {
            return Reaction::None;
        }
        self.generation += 1;
        let (generation, tx, cancel) = (self.generation, self.tx.clone(), Arc::new(AtomicBool::new(false)));
        self.view = Some(View {
            generation,
            roots: roots.clone(),
            cancel: Arc::clone(&cancel),
            phase: Phase::Scanning,
            groups: Vec::new(),
            selected: HashSet::new(),
            rows: Vec::new(),
            error: None,
            cancelled: false,
        });
        let title = tr("Looking for duplicate files").to_owned();
        Reaction::Job(AppJob::new(title, move |w| {
            let progress = OnWorker { worker: w, cancel: &cancel };
            let result = find(&roots, &progress);
            let (cancelled, outcome) = match &result {
                Ok(groups) => (false, Outcome::done(trn(groups.len(), "Found {} group of duplicates", "Found {} groups of duplicates"))),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => (true, Outcome::done(tr("Cancelled").to_owned())),
                Err(e) => (false, Outcome::failed(e.to_string())),
            };
            let _ = tx.send(Msg::Found { generation, result: result.map_err(|e| e.to_string()), cancelled });
            outcome
        }))
    }

    fn start_removal(&mut self) {
        let Some(view) = &self.view else { return };
        let plans = view.plan();
        if plans.is_empty() {
            return;
        }
        let (generation, tx) = (view.generation, self.tx.clone());
        let doomed: Vec<PathBuf> = plans.iter().flat_map(|p| p.doomed.iter().map(|m| m.path.clone())).collect();
        let count = doomed.len();
        let title = trn(count, "Moving {} duplicate to the trash", "Moving {} duplicates to the trash");
        self.job = Some(
            AppJob::new(title, move |w| {
                let never = AtomicBool::new(false);
                let progress = OnWorker { worker: w, cancel: &never };
                progress.plan(count as u64, 0);
                let (done, skipped, cancelled) = trash_planned(&plans, &progress, trash::trash);
                let _ = tx.send(Msg::Trashed { generation, paths: done.iter().map(|(from, _)| from.clone()).collect() });
                let mut message = trn(done.len(), "Moved {} duplicate to the trash", "Moved {} duplicates to the trash");
                if skipped > 0 {
                    message.push_str(&format!(" · {}", trf("{} skipped", &[&skipped])));
                }
                let label = trn(done.len(), "{} duplicate", "{} duplicates");
                let effect = if done.is_empty() { Effect::None } else { Effect::Record(UndoEntry::new(trf("Removal of {}", &[&label]), done.into_iter().map(|(_, to)| Edit::Restore(to)).collect())) };
                let mut outcome = Outcome::done(message).with(effect);
                outcome.cancelled = cancelled;
                outcome
            })
            .removing(doomed),
        );
    }
}

fn short(path: &Path) -> String {
    match dirs::home_dir().and_then(|h| path.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

impl Plugin for Duplicates {
    fn id(&self) -> &'static str {
        "duplicates"
    }

    fn name(&self) -> &'static str {
        tr("Duplicate finder")
    }

    fn description(&self) -> &'static str {
        tr("Finds files with identical contents in folders and moves the extra copies to the trash, always keeping one")
    }

    fn icon(&self) -> &'static str {
        icons::DUPLICATE
    }

    fn busy(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.phase == Phase::Scanning)
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        for msg in self.rx.try_iter().collect::<Vec<_>>() {
            match msg {
                Msg::Found { generation, result, cancelled } => {
                    let Some(view) = self.view.as_mut().filter(|v| v.generation == generation) else { continue };
                    view.phase = Phase::Done;
                    view.cancelled = cancelled;
                    match result {
                        Ok(groups) => view.groups = groups,
                        Err(e) => view.error = Some(e),
                    }
                    view.rebuild_rows();
                    changed = true;
                }
                Msg::Trashed { generation, paths } => {
                    if let Some(view) = self.view.as_mut().filter(|v| v.generation == generation) {
                        view.forget(&paths);
                    }
                    changed = true;
                }
            }
        }
        changed
    }

    fn menu_items(&self, selection: &[FileItem]) -> Vec<MenuItem> {
        if !selection.is_empty() && selection.iter().all(|i| i.is_dir && !i.is_symlink) {
            vec![MenuItem::new("scan", tr("Find duplicate files…"), icons::DUPLICATE)]
        } else {
            Vec::new()
        }
    }

    fn folder_menu_items(&self, _here: &Path, _others: &[PathBuf]) -> Vec<MenuItem> {
        vec![MenuItem::new("scan_here", tr("Find duplicate files…"), icons::DUPLICATE)]
    }

    fn run(&mut self, id: &str, paths: &[PathBuf], _dir: &Path) -> Reaction {
        match id {
            "scan" => self.start(paths.iter().filter(|p| p.is_dir()).cloned().collect()),
            "scan_here" => self.start(paths.iter().take(1).filter(|p| p.is_dir()).cloned().collect()),
            _ => Reaction::None,
        }
    }

    fn reaction(&mut self) -> Reaction {
        self.job.take().map_or(Reaction::None, Reaction::Job)
    }

    fn window_title(&self) -> Option<&'static str> {
        self.view.as_ref().map(|_| tr("Duplicate files"))
    }

    fn close_window(&mut self) {
        if let Some(view) = self.view.take() {
            view.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn window(&mut self, ui: &mut egui::Ui, theme: &SerpantinumTheme) -> bool {
        let Some(view) = self.view.as_mut() else { return false };
        let roots = view.roots.iter().map(|r| short(r)).collect::<Vec<_>>().join(", ");
        ui.label(egui::RichText::new(roots).size(material::FONT_CAPTION).color(theme.subtext1));
        ui.add_space(4.0);

        if view.phase == Phase::Scanning {
            ui.label(egui::RichText::new(tr("Looking for duplicate files…")).size(material::FONT_BODY).color(theme.text));
            material::hint_label(ui, tr("Progress is in the operations list. Closing this window stops the search."), theme);
            return true;
        }
        if let Some(error) = &view.error {
            ui.label(egui::RichText::new(error).size(material::FONT_BODY).color(theme.red));
            return true;
        }
        if view.cancelled {
            ui.label(egui::RichText::new(tr("Cancelled")).size(material::FONT_BODY).color(theme.subtext1));
            return true;
        }
        if view.groups.is_empty() {
            material::hint_label(ui, tr("No duplicate files found"), theme);
            return true;
        }

        let reclaimable: u64 = view.groups.iter().map(Group::reclaimable).sum();
        let summary = trf("{} groups of identical files, {} could be freed", &[&view.groups.len(), &format_bytes(reclaimable)]);
        ui.label(egui::RichText::new(summary).size(material::FONT_BODY).color(theme.text));
        ui.add_space(6.0);

        let mut rule: Option<Keep> = None;
        let mut clear = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tr("Keep")).size(material::FONT_CAPTION).color(theme.subtext1));
            for (id, label, keep) in [("newest", tr("Newest"), Keep::Newest), ("oldest", tr("Oldest"), Keep::Oldest), ("short", tr("Shortest path"), Keep::ShortestPath)] {
                if material::button(ui, ("dup_keep", id), None, label, theme, material::ButtonKind::Tonal, true).clicked() {
                    rule = Some(keep);
                }
            }
            for (i, root) in view.roots.iter().enumerate() {
                let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string());
                if material::button(ui, ("dup_keep_in", i), Some(icons::FOLDER), &trf("In {}", &[&name]), theme, material::ButtonKind::Tonal, true).clicked() {
                    rule = Some(Keep::InFolder(root.clone()));
                }
            }
            if material::button(ui, "dup_clear", None, tr("Select none"), theme, material::ButtonKind::Text, !view.selected.is_empty()).clicked() {
                clear = true;
            }
        });
        if let Some(rule) = rule {
            view.select_all_but(&rule);
        }
        if clear {
            view.selected.clear();
        }
        ui.add_space(6.0);

        let mut toggled: Option<(PathBuf, bool)> = None;
        egui::ScrollArea::vertical().max_height(340.0).auto_shrink([false, true]).show_rows(ui, ROW_HEIGHT, view.rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in &view.rows[range] {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::hover());
                match *row {
                    Row::Group(g) => {
                        let group = &view.groups[g];
                        let text = trf("{} copies of {} each, {} could be freed", &[&group.files.len(), &format_bytes(group.size), &format_bytes(group.reclaimable())]);
                        material::paint_label(ui, rect.left() + 4.0, rect.center().y, &text, material::FONT_BODY, theme.subtext1, rect.width() - 8.0);
                    }
                    Row::File(g, f) => {
                        let member = &view.groups[g].files[f];
                        let on = view.selected.contains(&member.path);
                        let enabled = view.can_select(g, member);
                        let box_rect = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 0.0), egui::vec2(30.0, ROW_HEIGHT));
                        let clicked = ui.scope_builder(egui::UiBuilder::new().max_rect(box_rect), |ui| material::checkbox(ui, ("dup_pick", g, f), on, enabled, theme)).inner.clicked();
                        if clicked && enabled {
                            toggled = Some((member.path.clone(), !on));
                        }
                        let date = crate::fs::props::format_datetime(member.modified);
                        let date_w = 140.0;
                        material::paint_label(ui, rect.right() - date_w, rect.center().y, &date, material::FONT_CAPTION, theme.subtext1, date_w - 4.0);
                        let (name, folder) = (
                            member.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                            member.path.parent().map(short).unwrap_or_default(),
                        );
                        let x = rect.left() + 46.0;
                        let room = rect.width() - 46.0 - date_w - 8.0;
                        let name_w = ui.painter().layout_no_wrap(name.clone(), egui::FontId::proportional(material::FONT_BODY), theme.text).size().x.min(room);
                        material::paint_label(ui, x, rect.center().y, &name, material::FONT_BODY, if on { theme.red } else { theme.text }, name_w);
                        if room - name_w > 40.0 {
                            material::paint_label(ui, x + name_w + 10.0, rect.center().y, &folder, material::FONT_CAPTION, theme.subtext1, room - name_w - 10.0);
                        }
                    }
                }
            }
        });
        if let Some((path, on)) = toggled {
            if on {
                view.selected.insert(path);
            } else {
                view.selected.remove(&path);
            }
        }

        ui.add_space(8.0);
        let (count, bytes) = view.selected_size();
        let mut remove = false;
        ui.horizontal(|ui| {
            let label = if count == 0 { tr("Nothing selected").to_owned() } else { trf("{} selected, {}", &[&trn(count, "{} file", "{} files"), &format_bytes(bytes)]) };
            ui.label(egui::RichText::new(label).size(material::FONT_BODY).color(theme.subtext1));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                remove = material::button(ui, "dup_remove", Some(icons::TRASH), tr("Move to trash"), theme, material::ButtonKind::Danger, count > 0).clicked() && count > 0;
            });
        });
        if remove {
            self.start_removal();
        }
        true
    }

    fn messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Progress that never stops anything.
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
        let dir = std::env::temp_dir().join(format!("ski-dups-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn member(path: &str, secs: u64) -> Member {
        Member { path: PathBuf::from(path), size: 10, modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs)) }
    }

    #[test]
    fn finds_equal_files_and_tells_near_copies_apart() {
        let dir = folder("find");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let big = vec![7u8; 100_000];
        std::fs::write(dir.join("a.bin"), &big).unwrap();
        std::fs::write(dir.join("sub/a-copy.bin"), &big).unwrap();
        // Same size, different last byte
        let mut tail = big.clone();
        *tail.last_mut().unwrap() = 8;
        std::fs::write(dir.join("tail.bin"), &tail).unwrap();
        // Same size, same start and end, different middle: only the full hash can tell
        let mut middle = big.clone();
        middle[50_000] = 9;
        std::fs::write(dir.join("middle.bin"), &middle).unwrap();
        // Empty files are ignored, small equal files are found
        std::fs::write(dir.join("e1"), b"").unwrap();
        std::fs::write(dir.join("e2"), b"").unwrap();
        std::fs::write(dir.join("s1.txt"), b"same").unwrap();
        std::fs::write(dir.join("sub/s2.txt"), b"same").unwrap();
        // A hard link is the same file (listed under its lowest path), a symlink is not followed
        std::fs::hard_link(dir.join("s1.txt"), dir.join("s1-link.txt")).unwrap();
        std::os::unix::fs::symlink(dir.join("a.bin"), dir.join("a-symlink.bin")).unwrap();

        let groups = find(std::slice::from_ref(&dir), &Quiet).unwrap();
        let names: Vec<Vec<String>> = groups.iter().map(|g| g.files.iter().map(|m| m.path.strip_prefix(&dir).unwrap().display().to_string()).collect()).collect();
        assert_eq!(names, vec![vec!["a.bin".to_owned(), "sub/a-copy.bin".to_owned()], vec!["s1-link.txt".to_owned(), "sub/s2.txt".to_owned()]]);
        assert_eq!(groups[0].reclaimable(), 100_000);
        // Roots that overlap don't list a file twice
        let again = find(&[dir.clone(), dir.join("sub")], &Quiet).unwrap();
        assert_eq!(again.len(), 2);
        assert!(again.iter().all(|g| g.files.len() == 2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancelling_stops_the_scan() {
        struct Stop;
        impl Progress for Stop {
            fn checkpoint(&self) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
            }
            fn current(&self, _: &str) {}
            fn plan(&self, _: u64, _: u64) {}
            fn bytes(&self, _: u64) {}
            fn file_done(&self) {}
        }
        let dir = folder("cancel");
        std::fs::write(dir.join("a"), b"x").unwrap();
        assert_eq!(find(std::slice::from_ref(&dir), &Stop).unwrap_err().kind(), io::ErrorKind::Interrupted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn view_with(groups: Vec<Group>) -> View {
        let mut view = View { generation: 1, roots: vec![PathBuf::from("/a")], cancel: Arc::default(), phase: Phase::Done, groups, selected: HashSet::new(), rows: Vec::new(), error: None, cancelled: false };
        view.rebuild_rows();
        view
    }

    #[test]
    fn rules_pick_the_copy_that_stays() {
        let group = Group { size: 10, files: vec![member("/a/old/file", 100), member("/a/new/file-copy", 300), member("/b/x", 200)] };
        assert_eq!(keeper(&group, &Keep::Newest), 1);
        assert_eq!(keeper(&group, &Keep::Oldest), 0);
        assert_eq!(keeper(&group, &Keep::ShortestPath), 2);
        assert_eq!(keeper(&group, &Keep::InFolder(PathBuf::from("/b"))), 2);
        assert_eq!(keeper(&group, &Keep::InFolder(PathBuf::from("/nowhere"))), 2, "no copy there: the shortest path stays");
        let mut view = view_with(vec![group]);
        view.select_all_but(&Keep::Oldest);
        assert_eq!(view.selected.len(), 2);
        assert!(!view.selected.contains(&PathBuf::from("/a/old/file")));
    }

    #[test]
    fn one_copy_of_a_group_always_stays() {
        let group = Group { size: 10, files: vec![member("/a/1", 1), member("/a/2", 2), member("/a/3", 3)] };
        let mut view = view_with(vec![group.clone()]);
        view.selected.insert(PathBuf::from("/a/1"));
        view.selected.insert(PathBuf::from("/a/2"));
        // The last unticked copy can't be ticked
        assert!(!view.can_select(0, &group.files[2]));
        assert!(view.can_select(0, &group.files[0]), "a ticked one can always be unticked");
        // Even a selection that ticks everything plans to keep one
        view.selected.insert(PathBuf::from("/a/3"));
        let plan = view.plan();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].doomed.len(), 2);
        assert_eq!(plan[0].keepers.len(), 1);
        assert_eq!(view.selected_size(), (2, 20));
        // Nothing ticked, nothing planned
        view.selected.clear();
        assert!(view.plan().is_empty());
    }

    #[test]
    fn removal_checks_the_files_again_and_keeps_a_copy() {
        let dir = folder("trash");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let make = |name: &str, text: &str| {
            let path = dir.join(name);
            std::fs::write(&path, text).unwrap();
            let meta = std::fs::metadata(&path).unwrap();
            Member { path, size: meta.len(), modified: meta.modified().ok() }
        };
        let (keep, copy, edited) = (make("keep", "same"), make("copy", "same"), make("edited", "same"));
        std::fs::write(&edited.path, "changed after the scan").unwrap();
        let fake_trash = |path: &Path| {
            let to = bin.join(path.file_name().unwrap());
            std::fs::rename(path, &to).map(|_| to)
        };

        let plan = Planned { keepers: vec![keep.clone()], doomed: vec![copy.clone(), edited.clone()] };
        let (trashed, skipped, _) = trash_planned(&[plan], &Quiet, fake_trash);
        assert_eq!(trashed.len(), 1);
        assert_eq!(trashed[0].0, copy.path);
        assert_eq!(skipped, 1, "the edited file is left alone");
        assert!(keep.path.exists() && edited.path.exists() && !copy.path.exists());

        // The keeper vanished since the scan: the group's other copies are not touched
        let other = make("other", "same");
        std::fs::remove_file(&keep.path).unwrap();
        let plan = Planned { keepers: vec![keep], doomed: vec![other.clone()] };
        let (trashed, skipped, _) = trash_planned(&[plan], &Quiet, fake_trash);
        assert!(trashed.is_empty());
        assert_eq!(skipped, 1);
        assert!(other.path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removed_files_leave_the_groups() {
        let mut view = view_with(vec![
            Group { size: 10, files: vec![member("/a/1", 1), member("/a/2", 2), member("/a/3", 3)] },
            Group { size: 10, files: vec![member("/b/1", 1), member("/b/2", 2)] },
        ]);
        view.selected.extend([PathBuf::from("/a/2"), PathBuf::from("/b/2")]);
        view.forget(&[PathBuf::from("/a/2"), PathBuf::from("/b/2")]);
        assert_eq!(view.groups.len(), 1, "a group left with one file is no group");
        assert_eq!(view.groups[0].files.len(), 2);
        assert!(view.selected.is_empty());
        assert_eq!(view.rows.len(), 3);
    }
}

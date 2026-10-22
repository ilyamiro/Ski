//! Background file operations. A job runs on its own thread and reports through shared counters
//! (for the progress popover), can be paused or cancelled, and asks the window what to do when a
//! name is taken. Finished jobs hand back the entry that undoes them.
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;

use super::history::{Edit, UndoEntry};
use super::ops::{self, unique_path};
use super::trash;
use crate::i18n::{tr, trf, trn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Copy,
    Move,
    Extract,
    Compress,
    Delete,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Copy,
    Move,
}

/// Counters a job updates while it works; the window reads them every frame.
#[derive(Default)]
pub struct Shared {
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
    pub files_done: AtomicU64,
    pub files_total: AtomicU64,
    pub cancel: AtomicBool,
    pub paused: AtomicBool,
    /// Stopped on a question the window has to answer.
    pub waiting: AtomicBool,
    pub current: Mutex<String>,
}

impl Shared {
    /// Called between chunks of work: waits while paused, and fails once cancelled.
    pub fn checkpoint(&self) -> io::Result<()> {
        loop {
            if self.cancel.load(Relaxed) {
                return Err(cancelled());
            }
            if !self.paused.load(Relaxed) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    fn set_current(&self, name: &str) {
        if let Ok(mut current) = self.current.lock() {
            *current = name.to_owned();
        }
    }

    pub fn current(&self) -> String {
        self.current.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// 0..1 when the size of the work is known.
    pub fn fraction(&self) -> Option<f32> {
        let (bytes_total, files_total) = (self.bytes_total.load(Relaxed), self.files_total.load(Relaxed));
        if bytes_total > 0 {
            Some((self.bytes_done.load(Relaxed) as f64 / bytes_total as f64).clamp(0.0, 1.0) as f32)
        } else if files_total > 0 {
            Some((self.files_done.load(Relaxed) as f64 / files_total as f64).clamp(0.0, 1.0) as f32)
        } else {
            None
        }
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled")
}

fn is_cancel(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::Interrupted
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Replace,
    Skip,
    KeepBoth,
    /// Put the contents of a folder into the folder that already exists.
    Merge,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct EntryInfo {
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl EntryInfo {
    fn of(path: &Path) -> Self {
        let meta = path.symlink_metadata().ok();
        Self {
            is_dir: meta.as_ref().is_some_and(|m| m.is_dir()),
            size: meta.as_ref().map_or(0, |m| m.len()),
            modified: meta.and_then(|m| m.modified().ok()),
        }
    }
}

/// A name is taken. The job waits until the window sends an answer.
pub struct ConflictRequest {
    pub job: u64,
    pub target: PathBuf,
    pub source_info: EntryInfo,
    pub target_info: EntryInfo,
    reply: mpsc::Sender<(Decision, bool)>,
}

impl ConflictRequest {
    /// `all` applies the answer to every further conflict of the same job.
    pub fn answer(self, decision: Decision, all: bool) {
        let _ = self.reply.send((decision, all));
    }

    /// Both are folders, so their contents can be merged.
    pub fn can_merge(&self) -> bool {
        self.source_info.is_dir && self.target_info.is_dir
    }
}

pub enum Effect {
    None,
    /// A new change that can be undone.
    Record(UndoEntry),
    /// An undo finished: the entry that redoes it.
    Undone(UndoEntry),
    /// A redo finished: the entry that undoes it again.
    Redone(UndoEntry),
}

pub struct Outcome {
    pub ok: bool,
    pub cancelled: bool,
    pub message: String,
    pub effect: Effect,
}

impl Outcome {
    pub fn done(message: String) -> Self {
        Self { ok: true, cancelled: false, message, effect: Effect::None }
    }

    pub fn failed(message: String) -> Self {
        Self { ok: false, cancelled: false, message, effect: Effect::None }
    }

    pub fn with(mut self, effect: Effect) -> Self {
        self.effect = effect;
        self
    }
}

pub enum Event {
    Conflict(ConflictRequest),
    Done { job: u64, outcome: Outcome },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    Done,
    Failed,
    Cancelled,
}

pub struct Job {
    pub id: u64,
    pub kind: JobKind,
    pub title: String,
    pub detail: String,
    pub shared: Arc<Shared>,
    pub state: JobState,
    pub message: String,
    pub finished: Option<Instant>,
    /// Bytes per second, smoothed.
    pub speed: f64,
    sample: (Instant, u64),
}

impl Job {
    /// Pausing and cancelling make sense where the work moves through files one at a time.
    pub fn controllable(&self) -> bool {
        matches!(self.kind, JobKind::Copy | JobKind::Move | JobKind::Delete)
    }

    pub fn paused(&self) -> bool {
        self.shared.paused.load(Relaxed)
    }

    pub fn eta(&self) -> Option<Duration> {
        let total = self.shared.bytes_total.load(Relaxed);
        let left = total.saturating_sub(self.shared.bytes_done.load(Relaxed));
        (self.speed > 1.0 && total > 0 && !self.paused()).then(|| Duration::from_secs_f64(left as f64 / self.speed))
    }
}

/// Everything a running job needs from the outside.
pub struct Worker {
    pub shared: Arc<Shared>,
    events: mpsc::Sender<Event>,
    ctx: egui::Context,
    job: u64,
    /// The answer given with "apply to all".
    remembered: Option<Decision>,
}

impl Worker {
    fn decide(&mut self, source: &Path, target: &Path) -> io::Result<Decision> {
        let (source_info, target_info) = (EntryInfo::of(source), EntryInfo::of(target));
        let both_dirs = source_info.is_dir && target_info.is_dir;
        if let Some(decision) = self.remembered {
            return Ok(match decision {
                Decision::Merge if !both_dirs => Decision::Replace,
                other => other,
            });
        }
        let (reply, answer) = mpsc::channel();
        let request = ConflictRequest { job: self.job, target: target.to_path_buf(), source_info, target_info, reply };
        self.events.send(Event::Conflict(request)).map_err(|_| cancelled())?;
        self.ctx.request_repaint();
        self.shared.waiting.store(true, Relaxed);
        let result = loop {
            match answer.recv_timeout(Duration::from_millis(100)) {
                Ok(reply) => break Some(reply),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self.shared.cancel.load(Relaxed) {
                        break None;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break None,
            }
        };
        self.shared.waiting.store(false, Relaxed);
        match result {
            Some((Decision::Cancel, _)) | None => Err(cancelled()),
            Some((decision, all)) => {
                if all {
                    self.remembered = Some(decision);
                }
                Ok(decision)
            }
        }
    }

    fn add_bytes(&self, n: u64) {
        self.shared.bytes_done.fetch_add(n, Relaxed);
    }

    fn file_done(&self) {
        self.shared.files_done.fetch_add(1, Relaxed);
    }
}

pub struct Engine {
    ctx: egui::Context,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    next: u64,
    pub jobs: Vec<Job>,
}

impl Engine {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { ctx: ctx.clone(), tx, rx, next: 1, jobs: Vec::new() }
    }

    pub fn running(&self) -> bool {
        self.jobs.iter().any(|j| j.state == JobState::Running)
    }

    /// Start `work` on a thread of its own. `title` names it in the popover, `detail` says where to.
    pub fn spawn(&mut self, kind: JobKind, title: String, detail: String, work: impl FnOnce(&mut Worker) -> Outcome + Send + 'static) -> u64 {
        let id = self.next;
        self.next += 1;
        let shared = Arc::new(Shared::default());
        let now = Instant::now();
        self.jobs.push(Job { id, kind, title, detail, shared: shared.clone(), state: JobState::Running, message: String::new(), finished: None, speed: 0.0, sample: (now, 0) });
        let mut worker = Worker { shared, events: self.tx.clone(), ctx: self.ctx.clone(), job: id, remembered: None };
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut outcome = work(&mut worker);
            if worker.shared.cancel.load(Relaxed) {
                outcome.cancelled = true;
            }
            let _ = tx.send(Event::Done { job: id, outcome });
            ctx.request_repaint();
        });
        id
    }

    /// Collect what the jobs reported since the last call.
    pub fn poll(&mut self) -> Vec<Event> {
        let events: Vec<Event> = self.rx.try_iter().collect();
        for event in &events {
            if let Event::Done { job, outcome } = event
                && let Some(job) = self.jobs.iter_mut().find(|j| j.id == *job)
            {
                job.state = if outcome.cancelled {
                    JobState::Cancelled
                } else if outcome.ok {
                    JobState::Done
                } else {
                    JobState::Failed
                };
                job.message = outcome.message.clone();
                job.finished = Some(Instant::now());
            }
        }
        events
    }

    /// Smooth the transfer speed of the running jobs; call once per frame.
    pub fn sample(&mut self) {
        let now = Instant::now();
        for job in self.jobs.iter_mut().filter(|j| j.state == JobState::Running) {
            let done = job.shared.bytes_done.load(Relaxed);
            let dt = now.duration_since(job.sample.0).as_secs_f64();
            if job.shared.paused.load(Relaxed) {
                job.sample = (now, done);
                job.speed = 0.0;
            } else if dt >= 0.3 {
                let instant = done.saturating_sub(job.sample.1) as f64 / dt;
                job.speed = if job.speed == 0.0 { instant } else { job.speed * 0.7 + instant * 0.3 };
                job.sample = (now, done);
            }
        }
    }

    /// Forget finished jobs that have been on show long enough.
    pub fn prune(&mut self, keep: Duration) {
        self.jobs.retain(|j| j.state == JobState::Running || j.finished.is_some_and(|f| f.elapsed() < keep));
    }

    pub fn cancel(&mut self, id: u64) {
        if let Some(job) = self.jobs.iter().find(|j| j.id == id) {
            job.shared.cancel.store(true, Relaxed);
        }
    }

    pub fn toggle_pause(&mut self, id: u64) {
        if let Some(job) = self.jobs.iter().find(|j| j.id == id) {
            let now = job.shared.paused.load(Relaxed);
            job.shared.paused.store(!now, Relaxed);
        }
    }

    /// Overall progress of everything running, if it can be told.
    pub fn overall(&self) -> Option<f32> {
        let running: Vec<&Job> = self.jobs.iter().filter(|j| j.state == JobState::Running).collect();
        let known: Vec<f32> = running.iter().filter_map(|j| j.shared.fraction()).collect();
        (!known.is_empty() && known.len() == running.len()).then(|| known.iter().sum::<f32>() / known.len() as f32)
    }
}

// ---------------------------------------------------------------------------------------------
// Copy and move

#[derive(Default)]
struct Record {
    /// New items (copies, links) an undo moves to the trash.
    created: Vec<PathBuf>,
    /// `(from, to)` of everything moved.
    moved: Vec<(PathBuf, PathBuf)>,
    /// Items replaced by newer ones, now in the trash.
    replaced: Vec<PathBuf>,
    skipped: usize,
    /// An item could only be deleted for good, so the undo cannot bring it back.
    lossy: bool,
}

fn measure(path: &Path, files: &mut u64, bytes: &mut u64) {
    let Ok(meta) = path.symlink_metadata() else { return };
    if meta.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                measure(&entry.path(), files, bytes);
            }
        }
    } else {
        *files += 1;
        *bytes += meta.len();
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.symlink_metadata(), b.symlink_metadata()) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn copy_file(w: &Worker, src: &Path, dst: &Path) -> io::Result<()> {
    let result = (|| {
        let mut reader = std::fs::File::open(src)?;
        let meta = reader.metadata()?;
        let mut writer = std::fs::OpenOptions::new().write(true).create_new(true).open(dst)?;
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            w.shared.checkpoint()?;
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n])?;
            w.add_bytes(n as u64);
        }
        // Tags, comments and ratings live in extended attributes; set them while the copy is still writable
        super::xattr::copy_user(src, dst);
        writer.set_permissions(meta.permissions())?;
        if let Ok(modified) = meta.modified() {
            let _ = writer.set_modified(modified);
        }
        Ok(())
    })();
    if result.is_err() {
        // A half-written copy is worse than none
        let _ = std::fs::remove_file(dst);
    }
    result
}

fn copy_tree(w: &Worker, src: &Path, dst: &Path) -> io::Result<()> {
    w.shared.checkpoint()?;
    let meta = src.symlink_metadata()?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dst)?;
        w.file_done();
        Ok(())
    } else if meta.is_dir() {
        std::fs::create_dir(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(w, &entry.path(), &dst.join(entry.file_name()))?;
        }
        super::xattr::copy_user(src, dst);
        std::fs::set_permissions(dst, meta.permissions())
    } else {
        w.shared.set_current(&name_of(src));
        copy_file(w, src, dst)?;
        w.file_done();
        Ok(())
    }
}

/// Take `target` out of the way: to the trash if possible, so the undo can bring it back.
fn clear_target(rec: &mut Record, target: &Path) -> io::Result<()> {
    match trash::trash(target) {
        Ok(trashed) => rec.replaced.push(trashed),
        Err(_) => {
            ops::remove_tree(target)?;
            rec.lossy = true;
        }
    }
    Ok(())
}

fn transfer_one(w: &mut Worker, op: Op, src: &Path, target: &Path, known: Option<(u64, u64)>, rec: &mut Record) -> io::Result<()> {
    w.shared.checkpoint()?;
    let parent = target.parent().ok_or_else(|| io::Error::other("no folder"))?;
    let name = target.file_name().ok_or_else(|| io::Error::other("no file name"))?.to_owned();
    if src.is_dir() && !src.is_symlink() && parent.canonicalize().is_ok_and(|p| src.canonicalize().is_ok_and(|s| p.starts_with(s))) {
        return Err(io::Error::other(trf("Cannot put {} inside itself", &[&name_of(src)])));
    }
    let mut target = target.to_path_buf();
    let mut merge = false;
    if target.symlink_metadata().is_ok() {
        if same_file(src, &target) {
            if op == Op::Move {
                return Ok(());
            }
            // Pasting into the folder it came from makes a copy beside it
            target = unique_path(parent, &name, "copy");
        } else {
            match w.decide(src, &target)? {
                Decision::Skip => {
                    let (files, bytes) = known.unwrap_or_else(|| {
                        let (mut f, mut b) = (0, 0);
                        measure(src, &mut f, &mut b);
                        (f, b)
                    });
                    w.shared.files_done.fetch_add(files, Relaxed);
                    w.add_bytes(bytes);
                    rec.skipped += 1;
                    return Ok(());
                }
                Decision::Cancel => return Err(cancelled()),
                Decision::KeepBoth => target = unique_path(parent, &name, "copy"),
                Decision::Replace => clear_target(rec, &target)?,
                Decision::Merge => merge = true,
            }
        }
    }
    w.shared.set_current(&name_of(src));

    if merge {
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            transfer_one(w, op, &entry.path(), &target.join(entry.file_name()), None, rec)?;
        }
        if op == Op::Move {
            let _ = std::fs::remove_dir(src);
        }
        return Ok(());
    }

    match op {
        Op::Copy => {
            copy_tree(w, src, &target)?;
            rec.created.push(target);
        }
        Op::Move => match std::fs::rename(src, &target) {
            Ok(()) => {
                let (files, bytes) = known.unwrap_or_else(|| {
                    let (mut f, mut b) = (0, 0);
                    measure(&target, &mut f, &mut b);
                    (f, b)
                });
                w.shared.files_done.fetch_add(files, Relaxed);
                w.add_bytes(bytes);
                rec.moved.push((src.to_path_buf(), target));
            }
            // Another filesystem: copy, then remove the original once the copy is whole
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                copy_tree(w, src, &target)?;
                ops::remove_tree(src)?;
                rec.moved.push((src.to_path_buf(), target));
            }
            Err(e) => return Err(e),
        },
    }
    Ok(())
}

/// Copy or move every `(source, target)` pair. A taken target name is a question for the window.
pub fn transfer(w: &mut Worker, op: Op, pairs: Vec<(PathBuf, PathBuf)>) -> Outcome {
    let sizes: Vec<(u64, u64)> = pairs
        .iter()
        .map(|(src, _)| {
            let (mut files, mut bytes) = (0, 0);
            measure(src, &mut files, &mut bytes);
            (files, bytes)
        })
        .collect();
    w.shared.files_total.store(sizes.iter().map(|s| s.0).sum(), Relaxed);
    w.shared.bytes_total.store(sizes.iter().map(|s| s.1).sum(), Relaxed);

    let mut rec = Record::default();
    let mut failures = Vec::new();
    let mut was_cancelled = false;
    for ((src, target), known) in pairs.iter().zip(&sizes) {
        match transfer_one(w, op, src, target, Some(*known), &mut rec) {
            Ok(()) => {}
            Err(e) if is_cancel(&e) => {
                was_cancelled = true;
                break;
            }
            Err(e) => failures.push(format!("{}: {e}", name_of(src))),
        }
    }

    let count = pairs.len();
    let label = if count == 1 { format!("“{}”", name_of(&pairs[0].0)) } else { trn(count, "{} item", "{} items") };
    let dest = pairs.first().and_then(|(_, t)| t.parent()).map(name_of).unwrap_or_default();

    let mut edits: Vec<Edit> = Vec::new();
    let undo_label = match op {
        Op::Copy => {
            edits.extend(rec.created.iter().cloned().map(Edit::Trash));
            trf("Copy of {}", &[&label])
        }
        Op::Move => {
            edits.extend(rec.moved.iter().rev().map(|(from, to)| Edit::Move { from: to.clone(), to: from.clone() }));
            trf("Move of {}", &[&label])
        }
    };
    edits.extend(rec.replaced.iter().cloned().map(Edit::Restore));
    let effect = if edits.is_empty() { Effect::None } else { Effect::Record(UndoEntry::new(undo_label, edits)) };

    let mut message = if was_cancelled {
        tr("Cancelled").to_owned()
    } else {
        match op {
            Op::Copy => trf("Copied {} to “{}”", &[&label, &dest]),
            Op::Move => trf("Moved {} to “{}”", &[&label, &dest]),
        }
    };
    if rec.skipped > 0 {
        message.push_str(&format!(" · {}", trf("{} skipped", &[&rec.skipped])));
    }
    if !failures.is_empty() {
        let mut outcome = Outcome::failed(failures.join(" · ")).with(effect);
        outcome.cancelled = was_cancelled;
        return outcome;
    }
    let mut outcome = Outcome::done(message).with(effect);
    outcome.cancelled = was_cancelled;
    outcome
}

/// Delete for good, one item at a time.
pub fn delete_forever(w: &mut Worker, paths: Vec<PathBuf>) -> Outcome {
    w.shared.files_total.store(paths.len() as u64, Relaxed);
    let mut failures = Vec::new();
    let mut was_cancelled = false;
    for path in &paths {
        if w.shared.checkpoint().is_err() {
            was_cancelled = true;
            break;
        }
        w.shared.set_current(&name_of(path));
        if let Err(err) = trash::delete_forever(path) {
            failures.push(format!("{}: {err}", name_of(path)));
        }
        w.file_done();
    }
    let label = if paths.len() == 1 { format!("“{}”", name_of(&paths[0])) } else { trn(paths.len(), "{} item", "{} items") };
    let mut outcome = if !failures.is_empty() {
        Outcome::failed(failures.join(" · "))
    } else if was_cancelled {
        Outcome::done(tr("Cancelled").to_owned())
    } else {
        Outcome::done(trf("Deleted {}", &[&label]))
    };
    outcome.cancelled = was_cancelled;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        Engine::new(&egui::Context::default())
    }

    fn wait(engine: &mut Engine) -> Vec<Event> {
        let mut all = Vec::new();
        for _ in 0..500 {
            all.extend(engine.poll());
            if !engine.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        all
    }

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ski_engine_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copy_reports_progress_and_an_undo() {
        let dir = sandbox("copy");
        std::fs::create_dir_all(dir.join("src/d")).unwrap();
        std::fs::write(dir.join("src/a.bin"), vec![7u8; 3_000_000]).unwrap();
        std::fs::write(dir.join("src/d/b.txt"), "b").unwrap();
        std::fs::create_dir_all(dir.join("dst")).unwrap();

        let mut e = engine();
        let pairs = vec![(dir.join("src/a.bin"), dir.join("dst/a.bin")), (dir.join("src/d"), dir.join("dst/d"))];
        let id = e.spawn(JobKind::Copy, "Copy".into(), String::new(), move |w| transfer(w, Op::Copy, pairs));
        let events = wait(&mut e);
        let Some(Event::Done { outcome, .. }) = events.into_iter().find(|ev| matches!(ev, Event::Done { .. })) else { panic!("no result") };
        assert!(outcome.ok, "{}", outcome.message);
        let job = e.jobs.iter().find(|j| j.id == id).unwrap();
        assert_eq!(job.shared.bytes_done.load(Relaxed), 3_000_001);
        assert_eq!(job.shared.files_done.load(Relaxed), 2);
        assert_eq!(std::fs::read_to_string(dir.join("dst/d/b.txt")).unwrap(), "b");
        let Effect::Record(entry) = outcome.effect else { panic!("no undo") };
        assert_eq!(entry.edits.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copies_keep_tags_and_other_user_attributes() {
        let dir = sandbox("xattr");
        std::fs::create_dir_all(dir.join("src/d")).unwrap();
        std::fs::create_dir_all(dir.join("dst")).unwrap();
        std::fs::write(dir.join("src/a.txt"), "a").unwrap();
        if crate::fs::xattr::set(&dir.join("src/a.txt"), "user.xdg.tags", b"work").is_err() {
            // This file system keeps no attributes
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        crate::fs::xattr::set(&dir.join("src/d"), "user.xdg.tags", b"folder").unwrap();
        let mut e = engine();
        let pairs = vec![(dir.join("src/a.txt"), dir.join("dst/a.txt")), (dir.join("src/d"), dir.join("dst/d"))];
        e.spawn(JobKind::Copy, "Copy".into(), String::new(), move |w| transfer(w, Op::Copy, pairs));
        let events = wait(&mut e);
        assert!(events.iter().any(|ev| matches!(ev, Event::Done { outcome, .. } if outcome.ok)));
        assert_eq!(crate::fs::xattr::get(&dir.join("dst/a.txt"), "user.xdg.tags").unwrap().as_deref(), Some(&b"work"[..]));
        assert_eq!(crate::fs::xattr::get(&dir.join("dst/d"), "user.xdg.tags").unwrap().as_deref(), Some(&b"folder"[..]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn conflicts_wait_for_an_answer_and_remember_it() {
        let dir = sandbox("conflict");
        std::fs::create_dir_all(dir.join("dst")).unwrap();
        for n in ["a", "b"] {
            std::fs::write(dir.join(n), format!("new {n}")).unwrap();
            std::fs::write(dir.join("dst").join(n), format!("old {n}")).unwrap();
        }
        let mut e = engine();
        let pairs = vec![(dir.join("a"), dir.join("dst/a")), (dir.join("b"), dir.join("dst/b"))];
        e.spawn(JobKind::Copy, "Copy".into(), String::new(), move |w| transfer(w, Op::Copy, pairs));

        // The first name asks; "apply to all" answers the second without asking
        let mut asked = 0;
        for _ in 0..500 {
            for event in e.poll() {
                if let Event::Conflict(request) = event {
                    asked += 1;
                    assert!(!request.can_merge());
                    request.answer(Decision::KeepBoth, true);
                }
            }
            if !e.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(asked, 1);
        assert_eq!(std::fs::read_to_string(dir.join("dst/a")).unwrap(), "old a");
        assert_eq!(std::fs::read_to_string(dir.join("dst/a (copy)")).unwrap(), "new a");
        assert_eq!(std::fs::read_to_string(dir.join("dst/b (copy)")).unwrap(), "new b");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancel_stops_a_job_and_pasting_beside_the_original_makes_a_copy() {
        let dir = sandbox("cancel");
        std::fs::write(dir.join("big"), vec![1u8; 50_000_000]).unwrap();
        std::fs::create_dir_all(dir.join("dst")).unwrap();
        let mut e = engine();
        let pairs = vec![(dir.join("big"), dir.join("dst/big"))];
        let id = e.spawn(JobKind::Copy, "Copy".into(), String::new(), move |w| transfer(w, Op::Copy, pairs));
        e.toggle_pause(id);
        std::thread::sleep(Duration::from_millis(30));
        e.cancel(id);
        let events = wait(&mut e);
        assert!(events.iter().any(|ev| matches!(ev, Event::Done { outcome, .. } if outcome.cancelled)));
        assert!(!dir.join("dst/big").exists(), "a cancelled copy leaves nothing half-written");

        let pairs = vec![(dir.join("big"), dir.join("big"))];
        e.spawn(JobKind::Copy, "Copy".into(), String::new(), move |w| transfer(w, Op::Copy, pairs));
        wait(&mut e);
        assert!(dir.join("big (copy)").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn move_merges_folders_and_records_the_way_back() {
        let dir = sandbox("merge");
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("x/a")).unwrap();
        std::fs::write(dir.join("a/new.txt"), "n").unwrap();
        std::fs::write(dir.join("x/a/old.txt"), "o").unwrap();
        let mut e = engine();
        let pairs = vec![(dir.join("a"), dir.join("x/a"))];
        e.spawn(JobKind::Move, "Move".into(), String::new(), move |w| transfer(w, Op::Move, pairs));
        for _ in 0..500 {
            for event in e.poll() {
                if let Event::Conflict(request) = event {
                    assert!(request.can_merge());
                    request.answer(Decision::Merge, false);
                }
            }
            if !e.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(dir.join("x/a/new.txt").exists() && dir.join("x/a/old.txt").exists());
        assert!(!dir.join("a").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

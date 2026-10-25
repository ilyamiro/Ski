//! Watches the folders on show with inotify, so what changes on disk (by another program, a
//! download, a drive's own writes) reaches the listing without a manual refresh.
//!
//! One thread reads the kernel's events and passes them on; the app collects them for a moment
//! (see `app::watching`) because a single save or copy is a burst of them. A folder inotify cannot
//! watch (no room left for watches, an odd filesystem) is checked by its modification time instead.
use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime};

use eframe::egui;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// Something in a watched folder was made, changed, moved or removed. `settled` means it is
    /// written out (closed, moved in, its attributes set), not still being written to.
    Path { path: PathBuf, settled: bool },
    /// The kernel dropped events: read the folder again.
    Overflow,
    /// A watched folder itself was removed, moved away or unmounted.
    Gone(PathBuf),
}

#[derive(Default)]
struct State {
    watches: HashMap<i32, PathBuf>,
    /// Folders watched by looking at their modification time, with the last one seen.
    polled: HashMap<PathBuf, Option<SystemTime>>,
}

pub struct Watcher {
    fd: i32,
    state: Arc<Mutex<State>>,
    rx: mpsc::Receiver<Event>,
    stop: Arc<AtomicBool>,
    dirs: Vec<PathBuf>,
}

const WATCHED: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_MODIFY
    | libc::IN_CLOSE_WRITE
    | libc::IN_ATTRIB
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF
    | libc::IN_ONLYDIR;
const SETTLED: u32 = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_ATTRIB;
/// How often a folder that cannot be watched is looked at.
const POLL_EVERY: Duration = Duration::from_secs(2);

impl Watcher {
    pub fn new(ctx: &egui::Context) -> Option<Self> {
        // SAFETY: plain system call; the descriptor is owned by this struct until it is dropped.
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (thread_state, thread_stop, ctx) = (state.clone(), stop.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("ski-watch".into())
            .spawn(move || run(fd, thread_state, thread_stop, tx, ctx))
            .ok()?;
        Some(Self { fd, state, rx, stop, dirs: Vec::new() })
    }

    /// Watch exactly these folders (nothing is done when they already are the ones watched).
    pub fn set(&mut self, dirs: &[PathBuf]) {
        if self.dirs == dirs {
            return;
        }
        self.dirs = dirs.to_vec();
        let Ok(mut state) = self.state.lock() else { return };
        for wd in state.watches.keys() {
            // SAFETY: `fd` is a live inotify descriptor; a stale `wd` only makes this fail.
            unsafe { libc::inotify_rm_watch(self.fd, *wd) };
        }
        state.watches.clear();
        state.polled.clear();
        for dir in dirs {
            let Ok(c_path) = CString::new(dir.as_os_str().as_bytes()) else { continue };
            // SAFETY: `c_path` is a NUL-terminated path that outlives the call.
            let wd = unsafe { libc::inotify_add_watch(self.fd, c_path.as_ptr(), WATCHED) };
            if wd >= 0 {
                state.watches.insert(wd, dir.clone());
            } else {
                state.polled.insert(dir.clone(), None);
            }
        }
    }

    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        self.rx.try_iter()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // The thread notices within a poll round and closes the descriptor
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(fd: i32, state: Arc<Mutex<State>>, stop: Arc<AtomicBool>, tx: mpsc::Sender<Event>, ctx: egui::Context) {
    let mut buf = vec![0u8; 64 * 1024];
    let mut polled_at = std::time::Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: `pfd` is one valid pollfd.
        let ready = unsafe { libc::poll(&mut pfd, 1, 500) };
        let mut sent = false;
        if ready > 0 {
            // SAFETY: `buf` is writable for its whole length.
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                sent |= decode(&buf[..n as usize], &state, &tx);
            }
        }
        if polled_at.elapsed() >= POLL_EVERY {
            polled_at = std::time::Instant::now();
            sent |= check_polled(&state, &tx);
        }
        if sent {
            ctx.request_repaint();
        }
    }
    // SAFETY: the descriptor is only used by this thread now.
    unsafe { libc::close(fd) };
}

/// Turn a buffer of `inotify_event` records into `Event`s; whether any were sent.
fn decode(buf: &[u8], state: &Mutex<State>, tx: &mpsc::Sender<Event>) -> bool {
    const HEADER: usize = 16;
    let Ok(state) = state.lock() else { return false };
    let mut sent = false;
    let mut at = 0;
    while at + HEADER <= buf.len() {
        let field = |from: usize| u32::from_ne_bytes(buf[at + from..at + from + 4].try_into().unwrap_or([0; 4]));
        let (wd, mask, len) = (field(0) as i32, field(4), field(12) as usize);
        let name = buf.get(at + HEADER..at + HEADER + len).unwrap_or_default();
        at += HEADER + len;
        let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
        let event = if mask & libc::IN_Q_OVERFLOW != 0 {
            Some(Event::Overflow)
        } else if let Some(dir) = state.watches.get(&wd) {
            if mask & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF | libc::IN_UNMOUNT) != 0 {
                Some(Event::Gone(dir.clone()))
            } else if name.is_empty() {
                None
            } else {
                let path = dir.join(std::ffi::OsString::from_vec(name.to_vec()));
                Some(Event::Path { path, settled: mask & SETTLED != 0 })
            }
        } else {
            None
        };
        if let Some(event) = event {
            sent |= tx.send(event).is_ok();
        }
    }
    sent
}

fn check_polled(state: &Mutex<State>, tx: &mpsc::Sender<Event>) -> bool {
    let dirs: Vec<(PathBuf, Option<SystemTime>)> = match state.lock() {
        Ok(state) => state.polled.iter().map(|(d, t)| (d.clone(), *t)).collect(),
        Err(_) => return false,
    };
    let mut sent = false;
    for (dir, seen) in dirs {
        let now = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
        if now == seen {
            continue;
        }
        if let Ok(mut state) = state.lock()
            && let Some(slot) = state.polled.get_mut(&dir)
        {
            *slot = now;
        }
        // The first look only records the time
        if seen.is_some() || now.is_none() {
            sent |= tx.send(if now.is_none() { Event::Gone(dir) } else { Event::Overflow }).is_ok();
        }
    }
    sent
}

/// Whether `path` is one of the watched folders' own entries: its parent is `dir`.
pub fn is_inside(path: &Path, dirs: &[PathBuf]) -> bool {
    path.parent().is_some_and(|p| dirs.iter().any(|d| d == p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(watcher: &Watcher, want: impl Fn(&Event) -> bool) -> bool {
        for _ in 0..100 {
            if watcher.events().any(|e| want(&e)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn changes_in_a_watched_folder_arrive_and_a_removed_folder_is_reported() {
        let dir = std::env::temp_dir().join(format!("ski-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut watcher = Watcher::new(&egui::Context::default()).expect("inotify");
        watcher.set(std::slice::from_ref(&dir));

        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(wait_for(&watcher, |e| matches!(e, Event::Path { path, settled: true } if *path == file)), "a new file is reported once written");
        std::fs::remove_file(&file).unwrap();
        assert!(wait_for(&watcher, |e| matches!(e, Event::Path { path, .. } if *path == file)), "and again when it goes");

        std::fs::remove_dir_all(&dir).unwrap();
        assert!(wait_for(&watcher, |e| *e == Event::Gone(dir.clone())));
    }

    #[test]
    fn a_folder_that_cannot_be_watched_is_polled() {
        let mut watcher = Watcher::new(&egui::Context::default()).expect("inotify");
        let missing = std::env::temp_dir().join(format!("ski-watch-missing-{}", std::process::id()));
        watcher.set(std::slice::from_ref(&missing));
        assert!(watcher.state.lock().unwrap().polled.contains_key(&missing));
    }
}

//! Background preview loader. Previewable files in and near the viewport are queued on demand:
//! images, videos, SVGs, the first page of PDFs and the covers of e-books and comics. Decoded
//! pictures are uploaded as textures on the UI thread, within bounded queue, channel, decode and
//! texture memory budgets.
//!
//! Finished previews are also kept on disk (see `cache`), so reopening a folder or scrolling back
//! to an evicted item, in this window or in the next file dialog, never runs ffmpeg or a decoder
//! again. Previews come in a few resolutions that follow the icon size: zooming in re-renders at
//! the next tier, once, and keeps the old picture on screen until the sharper one arrives.
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};

use eframe::egui;

/// Preview resolutions, by the longest side in pixels.
const TIERS: [u32; 3] = [128, 256, 512];
/// Document pages and video frames need more pixels than a small tile to stay readable, so they
/// never render below this.
const PAGE_MIN: u32 = 256;
/// Decoding threads: all but one core, so the window keeps one to itself, and at least two.
fn workers() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1)).clamp(2, 6)
}
/// Textures uploaded per frame; the rest wait for the next one so a big folder never stalls a frame.
const UPLOADS_PER_FRAME: usize = 6;
/// Decoded previews buffered between workers and the UI thread; bounds memory when workers outpace
/// frames or the window is hidden.
const RESULT_CHANNEL_CAP: usize = 24;
/// How many files at the top of a freshly opened folder are queued before the first layout frame
/// reports which rows are on screen.
const INITIAL_PRELOAD: usize = 64;
/// Previews kept in memory, in bytes of pixels. Past this, older previews of other folders and then
/// off-screen previews of the open folder are dropped; the disk cache brings them back quickly.
const TEXTURE_BUDGET: usize = 128 * 1024 * 1024;
/// ffmpeg and 7z get this long on one file before they are killed, and may write at most this much.
const VIDEO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_HELPER_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
/// E-books, archives and SVGs larger than this are not worth reading for a tiny preview.
const MAX_DOC_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SVG_BYTES: u64 = 32 * 1024 * 1024;
/// A PDF is read whole to find its first page, so very large files fall back to the file icon.
const MAX_PDF_BYTES: u64 = 128 * 1024 * 1024;
/// Combined source bytes allowed in flight across workers while decoding uncached files.
const DECODE_BYTE_BUDGET: u64 = 128 * 1024 * 1024;
/// Concurrent PDF renders allowed at once, since each PDF also holds parsed pages and raster state.
const MAX_PDF_CONCURRENCY: usize = 2;
/// Maximum heap allocation a single image decoder may make.
const MAX_IMAGE_ALLOC: u64 = 256 * 1024 * 1024;
/// Cap on remembered failed paths across folders so long-lived sessions do not grow unboundedly.
const MAX_FAILED_CACHE: usize = 4096;
/// Cumulative decoded bytes after which glibc is asked to return freed worker arena pages.
#[cfg(target_env = "gnu")]
const MALLOC_TRIM_BYTES: u64 = 32 * 1024 * 1024;

#[cfg(target_env = "gnu")]
static TRIM_ACCUMULATOR: AtomicU64 = AtomicU64::new(0);

fn maybe_trim_heap(bytes: u64) {
    #[cfg(target_env = "gnu")]
    {
        if bytes == 0 {
            return;
        }
        let acc = TRIM_ACCUMULATOR.fetch_add(bytes, Ordering::Relaxed) + bytes;
        if acc >= MALLOC_TRIM_BYTES {
            TRIM_ACCUMULATOR.store(0, Ordering::Relaxed);
            // SAFETY: malloc_trim is thread-safe in glibc and only releases free heap pages.
            unsafe {
                libc::malloc_trim(0);
            }
        }
    }
    #[cfg(not(target_env = "gnu"))]
    let _ = bytes;
}

/// Shared memory and concurrency gate for uncached decoding across workers.
struct DecodeBudget {
    state: Mutex<BudgetState>,
    wake: Condvar,
    byte_limit: u64,
    pdf_limit: usize,
}

#[derive(Default)]
struct BudgetState {
    bytes: u64,
    pdfs: usize,
}

struct DecodePermit {
    budget: Arc<DecodeBudget>,
    bytes: u64,
    is_pdf: bool,
}

impl Drop for DecodePermit {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().unwrap_or_else(|e| e.into_inner());
        state.bytes = state.bytes.saturating_sub(self.bytes);
        if self.is_pdf {
            state.pdfs = state.pdfs.saturating_sub(1);
        }
        self.budget.wake.notify_all();
    }
}

impl DecodeBudget {
    fn new(byte_limit: u64, pdf_limit: usize) -> Self {
        Self {
            state: Mutex::new(BudgetState::default()),
            wake: Condvar::new(),
            byte_limit: byte_limit.max(1),
            pdf_limit: pdf_limit.max(1),
        }
    }

    fn acquire(self: &Arc<Self>, bytes: u64, is_pdf: bool, cancelled: &dyn Fn() -> bool) -> Option<DecodePermit> {
        let cost = bytes.min(self.byte_limit);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if cancelled() {
                return None;
            }
            let pdf_ok = !is_pdf || state.pdfs < self.pdf_limit;
            let bytes_ok = state.bytes == 0 || state.bytes.saturating_add(cost) <= self.byte_limit;
            if pdf_ok && bytes_ok {
                state.bytes += cost;
                if is_pdf {
                    state.pdfs += 1;
                }
                return Some(DecodePermit { budget: Arc::clone(self), bytes: cost, is_pdf });
            }
            state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn notify_all(&self) {
        let _guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.wake.notify_all();
    }
}

/// What a file looked like when its preview was asked for: modification time and size. A preview
/// made for another stamp is out of date.
pub type Stamp = (Option<std::time::SystemTime>, u64);

fn stamp_of(path: &Path) -> Stamp {
    std::fs::metadata(path).map_or((None, 0), |m| (m.modified().ok(), m.len()))
}

struct Job {
    path: PathBuf,
    stamp: Stamp,
    generation: u64,
    tier: u32,
}

/// Jobs waiting for a worker. Workers take a job for something on screen before any other, so the
/// pictures being looked at appear first and the rest of the viewport window fills in behind them.
#[derive(Default)]
struct Queue {
    jobs: Mutex<Waiting>,
    wake: Condvar,
    /// The files on screen top to bottom, then those just past them in the way the view is likely
    /// to scroll, so a worker with nothing on screen left to do prepares the next screenful.
    visible: Mutex<Vec<PathBuf>>,
}

#[derive(Default)]
struct Waiting {
    jobs: std::collections::VecDeque<Job>,
    /// How many waiting jobs each file has, so a worker asks the few files on screen whether they
    /// wait instead of hashing every job of a big folder on each turn.
    count: HashMap<PathBuf, usize>,
    closed: bool,
}

impl Queue {
    fn push(&self, job: Job, front: bool) {
        if let Ok(mut waiting) = self.jobs.lock() {
            if waiting.closed {
                return;
            }
            *waiting.count.entry(job.path.clone()).or_default() += 1;
            if front {
                waiting.jobs.push_front(job);
            } else {
                waiting.jobs.push_back(job);
            }
        }
        self.wake.notify_one();
    }

    fn clear(&self) {
        let mut waiting = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        waiting.jobs.clear();
        waiting.count.clear();
    }

    /// Drops waiting jobs not matching `keep`, returning paths that no longer have any waiting job.
    fn retain(&self, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
        let mut waiting = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let mut dropped = Vec::new();
        let mut kept = std::collections::VecDeque::with_capacity(waiting.jobs.len());
        while let Some(job) = waiting.jobs.pop_front() {
            if keep(&job.path) {
                kept.push_back(job);
            } else if let Some(n) = waiting.count.get_mut(&job.path) {
                *n -= 1;
                if *n == 0 {
                    waiting.count.remove(&job.path);
                    dropped.push(job.path);
                }
            }
        }
        waiting.jobs = kept;
        dropped
    }

    fn close(&self) {
        let mut waiting = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        waiting.closed = true;
        waiting.jobs.clear();
        waiting.count.clear();
        self.wake.notify_all();
    }

    fn pop(&self) -> Option<Job> {
        let mut waiting = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if waiting.closed {
                return None;
            }
            if !waiting.jobs.is_empty() {
                let at = {
                    let visible = self.visible.lock().unwrap_or_else(|e| e.into_inner());
                    visible
                        .iter()
                        .find(|p| waiting.count.contains_key(*p))
                        .and_then(|p| waiting.jobs.iter().position(|j| &j.path == p))
                        .unwrap_or(0)
                };
                let job = waiting.jobs.remove(at).expect("index in range");
                if let Some(n) = waiting.count.get_mut(&job.path) {
                    *n -= 1;
                    if *n == 0 {
                        waiting.count.remove(&job.path);
                    }
                }
                return Some(job);
            }
            waiting = self.wake.wait(waiting).unwrap_or_else(|e| e.into_inner());
        }
    }
}

enum DecodeResult {
    Ready(image::DynamicImage),
    Missing,
    Cancelled,
}

enum Outcome {
    Ready(egui::ColorImage, u32),
    Failed,
}

struct TextureEntry {
    handle: egui::TextureHandle,
    tier: u32,
    bytes: usize,
    last_used: u64,
}

pub struct Thumbnails {
    /// Each texture with the tier it was rendered at, its pixel byte size, and last-used tick.
    textures: HashMap<PathBuf, TextureEntry>,
    texture_bytes: usize,
    texture_budget: usize,
    use_tick: u64,
    /// The stamp each texture was made for, and the newest stamp seen for each file: a result for any other is dropped.
    made: HashMap<PathBuf, Stamp>,
    wanted: HashMap<PathBuf, Stamp>,
    queued: HashMap<PathBuf, Stamp>,
    /// Resolution wanted for new and upgraded previews.
    tier: u32,
    /// Previewable files of the open folder with their stamps, queued on demand as the view scrolls.
    folder: HashMap<PathBuf, Stamp>,
    /// First files of the open folder in listing order, used before the first layout frame.
    head: Vec<PathBuf>,
    /// Paths currently in or near the viewport (from `set_visible`).
    visible: Vec<PathBuf>,
    visible_set: HashSet<PathBuf>,
    /// Extra pictures such as folder icons or Quick Look previews, kept outside the folder window.
    extras: HashSet<PathBuf>,
    /// Files that have no preview, with the stamp they failed at; tried again once the file changes.
    failed: HashMap<PathBuf, Stamp>,
    queue: Arc<Queue>,
    budget: Arc<DecodeBudget>,
    result_rx: mpsc::Receiver<(PathBuf, Stamp, Outcome)>,
    generation: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
}

/// Smallest tier that is at least `px` physical pixels, or the largest one.
fn tier_for(px: f32) -> u32 {
    TIERS.into_iter().find(|&t| t as f32 >= px).unwrap_or(TIERS[TIERS.len() - 1])
}

fn extension(path: &Path) -> Option<String> {
    path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase())
}

pub fn is_image(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("png" | "apng" | "jpg" | "jpeg" | "jpe" | "jfif" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff" | "tga" | "ppm" | "pgm" | "pbm" | "pnm" | "qoi" | "hdr" | "dds")
    )
}

/// Pictures the image decoder cannot read but ffmpeg can.
fn is_ffmpeg_still(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("heic" | "heif" | "avif" | "jxl" | "psd" | "exr" | "jp2" | "j2k" | "pcx"))
}

/// Camera RAW files, which carry a full-size JPEG next to the sensor data.
fn is_raw(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("cr2" | "cr3" | "crw" | "nef" | "nrw" | "arw" | "sr2" | "srf" | "dng" | "orf" | "rw2" | "raf" | "pef" | "srw" | "3fr" | "erf" | "kdc" | "dcr" | "mrw" | "x3f" | "raw")
    )
}

/// Music files show their embedded cover art, when they have one.
fn is_audio(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("mp3" | "flac" | "m4a" | "m4b" | "ogg" | "oga" | "opus" | "wma" | "aac" | "ape" | "wv" | "mka" | "aif" | "aiff" | "mpc" | "dsf" | "tta"))
}

/// Comic archives, which are a folder of page pictures in a zip, rar or 7z.
fn is_comic(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("cbz" | "cbr" | "cb7" | "cbt"))
}

fn is_fb2(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    name.ends_with(".fb2") || name.ends_with(".fb2.zip")
}

fn is_mobi(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("mobi" | "azw" | "azw3" | "prc"))
}

fn is_video(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("mp4" | "mkv" | "webm" | "avi" | "mov" | "flv" | "wmv" | "m4v" | "mpg" | "mpeg" | "ogv" | "3gp" | "3g2" | "m2ts" | "m2t" | "mts" | "vob" | "asf" | "rm" | "rmvb" | "divx" | "f4v" | "mxf" | "ogm" | "mpe" | "qt" | "dv" | "ts"))
}

/// `.ts` is also TypeScript, so a transport stream is told apart by its 0x47 sync bytes, found
/// every 188 bytes (every 192 in a `.m2ts`, after a 4-byte timestamp).
fn is_transport_stream(path: &Path) -> bool {
    let mut head = [0u8; 193];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && ((head[0] == 0x47 && head[188] == 0x47) || (head[4] == 0x47 && head[192] == 0x47))
}

fn is_svg(path: &Path) -> bool {
    extension(path).as_deref() == Some("svg")
}

fn is_page(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("pdf" | "epub" | "kra" | "ora")
    ) || is_comic(path)
        || is_fb2(path)
        || is_mobi(path)
}

/// Whether the file gets a picture of its contents instead of a type icon.
pub fn has_preview(path: &Path) -> bool {
    is_image(path) || is_ffmpeg_still(path) || is_raw(path) || is_video(path) || is_audio(path) || is_svg(path) || is_page(path)
}

impl Thumbnails {
    pub fn new(ctx: &egui::Context) -> Self {
        let queue = Arc::new(Queue::default());
        let budget = Arc::new(DecodeBudget::new(DECODE_BYTE_BUDGET, MAX_PDF_CONCURRENCY));
        let (result_tx, result_rx) = mpsc::sync_channel(RESULT_CHANNEL_CAP);
        let generation = Arc::new(AtomicU64::new(0));
        let closed = Arc::new(AtomicBool::new(false));

        for _ in 0..workers() {
            let queue = queue.clone();
            let budget = budget.clone();
            let result_tx = result_tx.clone();
            let generation = generation.clone();
            let closed = closed.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                // Decoding is background work: it yields to the UI thread instead of competing with it
                unsafe { libc::setpriority(libc::PRIO_PROCESS, libc::syscall(libc::SYS_gettid) as libc::id_t, 15) };
                while let Some(job) = queue.pop() {
                    if closed.load(Ordering::Relaxed) {
                        break;
                    }
                    // Skip work for folders the user already left
                    if job.generation != generation.load(Ordering::Relaxed) {
                        continue;
                    }
                    let cancelled = || closed.load(Ordering::Relaxed) || job.generation != generation.load(Ordering::Relaxed);
                    // A malformed document must not take the worker down with it
                    let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        cached_thumbnail(&job.path, job.tier, &budget, &cancelled)
                    }))
                    .ok()
                    .unwrap_or(DecodeResult::Missing);
                    if cancelled() {
                        continue;
                    }
                    let outcome = match decoded {
                        DecodeResult::Ready(img) => Outcome::Ready(to_color_image(&img), job.tier),
                        DecodeResult::Missing => Outcome::Failed,
                        DecodeResult::Cancelled => continue,
                    };
                    if result_tx.send((job.path, job.stamp, outcome)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }

        cache::trim_in_background();

        Self {
            textures: HashMap::new(),
            texture_bytes: 0,
            texture_budget: TEXTURE_BUDGET,
            use_tick: 0,
            made: HashMap::new(),
            wanted: HashMap::new(),
            queued: HashMap::new(),
            tier: TIERS[0],
            folder: HashMap::new(),
            head: Vec::new(),
            visible: Vec::new(),
            visible_set: HashSet::new(),
            extras: HashSet::new(),
            failed: HashMap::new(),
            queue,
            budget,
            result_rx,
            generation,
            closed,
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.use_tick = self.use_tick.wrapping_add(1);
        self.use_tick
    }

    fn enqueue(&mut self, path: &Path, stamp: Stamp, generation: u64, front: bool) {
        self.wanted.insert(path.to_path_buf(), stamp);
        let sharp_enough = self.textures.get(path).is_some_and(|entry| entry.tier >= self.tier) && self.made.get(path) == Some(&stamp);
        let pending = self.queued.get(path) == Some(&stamp);
        if has_preview(path) && !sharp_enough && !pending && self.failed.get(path) != Some(&stamp) {
            self.queued.insert(path.to_path_buf(), stamp);
            self.queue.push(Job { path: path.to_path_buf(), stamp, generation, tier: self.tier }, front);
        }
    }

    /// Tell the loader which files are on screen and nearby: their previews are prioritized and
    /// queued on demand, while queued jobs that scrolled out of view are dropped.
    pub fn set_visible<'a>(&mut self, paths: impl Iterator<Item = &'a Path>) {
        let paths: Vec<&Path> = paths.collect();
        // Called every frame: only rebuild the window when the view moved
        if self.visible.len() == paths.len() && self.visible.iter().zip(&paths).all(|(a, b)| a.as_path() == *b) {
            return;
        }
        self.visible.clear();
        self.visible.extend(paths.into_iter().map(Path::to_path_buf));
        self.visible_set.clear();
        self.visible_set.extend(self.visible.iter().cloned());

        if let Ok(mut visible) = self.queue.visible.lock() {
            visible.clear();
            visible.extend(self.visible.iter().cloned());
        }

        let tick = self.next_tick();
        for path in &self.visible {
            if let Some(entry) = self.textures.get_mut(path) {
                entry.last_used = tick;
            }
        }

        let dropped = self.queue.retain(|p| self.visible_set.contains(p) || self.extras.contains(p));
        for path in dropped {
            self.queued.remove(&path);
        }

        let generation = self.generation.load(Ordering::Relaxed);
        let to_queue: Vec<(PathBuf, Stamp)> = self
            .visible
            .iter()
            .filter_map(|p| self.folder.get(p).copied().map(|stamp| (p.clone(), stamp)))
            .collect();
        for (path, stamp) in to_queue {
            self.enqueue(&path, stamp, generation, false);
        }
    }

    /// Record the previewable files of a freshly opened folder and queue only the top screenful;
    /// the rest are queued on demand as the viewport moves. Cancels work queued for the previous folder.
    pub fn preload<'a>(&mut self, files: impl Iterator<Item = (&'a Path, Stamp)>) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.queue.clear();
        self.budget.notify_all();
        while self.result_rx.try_recv().is_ok() {}
        // Anything that was queued but not finished was dropped
        self.queued.clear();
        self.folder.clear();
        self.head.clear();
        self.visible.clear();
        self.visible_set.clear();
        if let Ok(mut visible) = self.queue.visible.lock() {
            visible.clear();
        }
        self.extras.clear();
        // Files on servers are read only when the setting asks for that
        for (path, stamp) in files.filter(|(p, _)| has_preview(p) && crate::fs::network::previews_allowed(p)) {
            let owned = path.to_path_buf();
            self.folder.insert(owned.clone(), stamp);
            self.wanted.insert(owned.clone(), stamp);
            if self.head.len() < INITIAL_PRELOAD {
                self.enqueue(path, stamp, generation, false);
                self.head.push(owned);
            }
        }
        self.trim_textures();
        self.wanted.retain(|p, _| self.folder.contains_key(p) || self.textures.contains_key(p) || self.extras.contains(p));
        if self.failed.len() > MAX_FAILED_CACHE {
            self.failed.retain(|p, _| self.folder.contains_key(p) || self.extras.contains(p));
        }
    }

    /// Cancel queued folder previews when thumbnails are turned off.
    pub fn clear(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.queue.clear();
        self.budget.notify_all();
        while self.result_rx.try_recv().is_ok() {}
        self.queued.clear();
        self.folder.clear();
        self.head.clear();
        self.visible.clear();
        self.visible_set.clear();
        if let Ok(mut visible) = self.queue.visible.lock() {
            visible.clear();
        }
    }

    /// Lets go of older previews once they take more memory than `texture_budget`: first other
    /// folders, then off-screen previews of the open folder, then off-screen extras. The disk cache
    /// brings evicted pictures back quickly if they are needed again.
    fn trim_textures(&mut self) {
        if self.texture_bytes <= self.texture_budget {
            return;
        }
        let target = self.texture_budget / 20 * 17;
        self.evict_matching(target, |this, path| !this.folder.contains_key(path) && !this.extras.contains(path));
        self.evict_matching(target, |this, path| !this.visible_set.contains(path) && !this.extras.contains(path));
        self.evict_matching(target, |this, path| !this.visible_set.contains(path));
    }

    fn evict_matching(&mut self, target: usize, can_evict: impl Fn(&Self, &Path) -> bool) {
        if self.texture_bytes <= target {
            return;
        }
        let mut victims: Vec<(u64, PathBuf)> = self
            .textures
            .iter()
            .filter(|(path, _)| can_evict(self, path.as_path()))
            .map(|(path, entry)| (entry.last_used, path.clone()))
            .collect();
        victims.sort_unstable_by_key(|(tick, _)| *tick);
        for (_, path) in victims {
            if self.texture_bytes <= target {
                break;
            }
            if let Some(removed) = self.textures.remove(&path) {
                self.texture_bytes = self.texture_bytes.saturating_sub(removed.bytes);
                self.made.remove(&path);
            }
        }
    }

    /// Queue one extra image (e.g. a custom folder icon or Quick Look) without cancelling the
    /// folder's own work. Also the way to force a re-render right away.
    pub fn ensure(&mut self, path: &Path) {
        let generation = self.generation.load(Ordering::Relaxed);
        self.extras.insert(path.to_path_buf());
        self.enqueue(path, stamp_of(path), generation, true);
    }

    /// Note a file added or changed on disk in the open folder: its stamp is updated right away,
    /// and it is queued now only if it is in the current viewport window (otherwise on scroll).
    pub fn refresh(&mut self, path: &Path) {
        if !has_preview(path) || !crate::fs::network::previews_allowed(path) {
            return;
        }
        let stamp = stamp_of(path);
        let owned = path.to_path_buf();
        self.folder.insert(owned.clone(), stamp);
        self.wanted.insert(owned, stamp);
        if self.visible.is_empty() || self.visible_set.contains(path) {
            let generation = self.generation.load(Ordering::Relaxed);
            self.enqueue(path, stamp, generation, true);
        }
    }

    /// Tell the loader how many physical pixels the largest preview is drawn at. Crossing into a
    /// bigger tier re-renders what is in view; off-screen files upgrade when scrolled to.
    pub fn want_pixels(&mut self, px: f32) {
        let tier = tier_for(px);
        if tier <= self.tier {
            self.tier = tier;
            return;
        }
        self.tier = tier;
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.queue.clear();
        self.budget.notify_all();
        while self.result_rx.try_recv().is_ok() {}
        self.queued.clear();
        let active: Vec<PathBuf> = if self.visible.is_empty() { self.head.clone() } else { self.visible.clone() };
        for path in active {
            if let Some(&stamp) = self.folder.get(&path) {
                self.enqueue(&path, stamp, generation, false);
            }
        }
        let extras: Vec<PathBuf> = self.extras.iter().cloned().collect();
        for path in extras {
            let stamp = self.wanted.get(&path).copied().unwrap_or_else(|| stamp_of(&path));
            self.enqueue(&path, stamp, generation, true);
        }
    }

    /// Upload finished thumbnails; call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let mut uploaded = 0;
        let mut more = true;
        while uploaded < UPLOADS_PER_FRAME {
            let Ok((path, stamp, outcome)) = self.result_rx.try_recv() else {
                more = false;
                break;
            };
            // The file changed or its folder closed after this job started: drop the stale result
            let current = self.wanted.get(&path) == Some(&stamp);
            match outcome {
                Outcome::Ready(img, tier) => {
                    if !current {
                        continue;
                    }
                    if tier >= self.tier {
                        self.queued.remove(&path);
                    }
                    // A slow low-resolution job can finish after its upgrade did
                    if self.made.get(&path) == Some(&stamp) && self.textures.get(&path).is_some_and(|entry| entry.tier >= tier) {
                        continue;
                    }
                    let tex = ctx.load_texture(path.to_string_lossy(), img, egui::TextureOptions::LINEAR);
                    let bytes = tex.size().iter().product::<usize>() * 4;
                    let tick = self.next_tick();
                    if let Some(old) = self.textures.insert(path.clone(), TextureEntry { handle: tex, tier, bytes, last_used: tick }) {
                        self.texture_bytes = self.texture_bytes.saturating_sub(old.bytes);
                    }
                    self.texture_bytes += bytes;
                    self.made.insert(path, stamp);
                    uploaded += 1;
                }
                Outcome::Failed => {
                    if current {
                        self.queued.remove(&path);
                        self.failed.insert(path, stamp);
                    }
                }
            }
        }
        if uploaded > 0 {
            self.trim_textures();
        }
        if more {
            // More finished previews are waiting: draw this frame first, then take the next batch
            ctx.request_repaint();
        }
    }

    pub fn get(&self, path: &Path) -> Option<&egui::TextureHandle> {
        self.textures.get(path).map(|entry| &entry.handle)
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.queue.close();
        self.budget.notify_all();
        while self.result_rx.try_recv().is_ok() {}
    }
}

fn to_color_image(img: &image::DynamicImage) -> egui::ColorImage {
    let rgba = img.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
}

/// The preview from the disk cache, or rendered now and cached for next time. Files that have no
/// preview are remembered too, so a folder of covers-less tracks does not spawn ffmpeg per file on
/// every visit.
fn cached_thumbnail(path: &Path, tier: u32, budget: &Arc<DecodeBudget>, cancelled: &dyn Fn() -> bool) -> DecodeResult {
    if cancelled() {
        return DecodeResult::Cancelled;
    }
    let shared = is_image(path) && !is_ffmpeg_still(path);
    if shared {
        if let Some(hit) = shared_cache::read(path, tier) {
            return DecodeResult::Ready(hit);
        }
    }
    let key = cache::key(path);
    if let Some(key) = &key {
        if let Some(hit) = cache::read(key, tier) {
            return DecodeResult::Ready(hit);
        }
        if cache::known_missing(key) {
            return DecodeResult::Missing;
        }
    }
    let is_pdf = extension(path).as_deref() == Some("pdf");
    let file_bytes = std::fs::metadata(path).map_or(0, |m| m.len());
    if (is_pdf && file_bytes > MAX_PDF_BYTES) || (is_svg(path) && file_bytes > MAX_SVG_BYTES) {
        if let Some(key) = &key {
            cache::mark_missing(key);
        }
        return DecodeResult::Missing;
    }
    let Some(_permit) = budget.acquire(file_bytes, is_pdf, cancelled) else {
        return DecodeResult::Cancelled;
    };
    let rendered = render_thumbnail(path, tier);
    drop(_permit);
    if is_pdf || file_bytes >= 4 * 1024 * 1024 {
        maybe_trim_heap(file_bytes);
    }
    if cancelled() {
        return DecodeResult::Cancelled;
    }
    match rendered {
        Some(img) => {
            if let Some(key) = &key {
                cache::write(key, tier, &img);
            }
            if shared {
                shared_cache::write(path, tier, &img);
            }
            DecodeResult::Ready(img)
        }
        None => {
            if let Some(key) = &key {
                cache::mark_missing(key);
            }
            DecodeResult::Missing
        }
    }
}

#[cfg(test)]
fn decode_thumbnail(path: &Path) -> Option<egui::ColorImage> {
    render_thumbnail(path, TIERS[0]).map(|img| to_color_image(&img))
}

fn decode_image(path: &Path) -> Option<image::DynamicImage> {
    let mut reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_IMAGE_ALLOC);
    reader.limits(limits);
    reader.decode().ok()
}

/// A preview with its longest side at most `tier` pixels, or `PAGE_MIN` where the shape is kept.
fn render_thumbnail(path: &Path, tier: u32) -> Option<image::DynamicImage> {
    let page_size = tier.max(PAGE_MIN);
    if is_svg(path) {
        return render_svg(path, tier);
    }
    if is_page(path) {
        let page = if extension(path).as_deref() == Some("pdf") {
            if std::fs::metadata(path).ok()?.len() > MAX_PDF_BYTES {
                return None;
            }
            render_pdf(path, page_size)?
        } else {
            book_or_document_cover(path)?
        };
        return Some(fit(page, page_size));
    }
    if is_audio(path) {
        return Some(ffmpeg_picture(path, tier)?.thumbnail(tier, tier));
    }
    if is_video(path) {
        if extension(path).as_deref() == Some("ts") && !is_transport_stream(path) {
            return None;
        }
        let frame = video_frame(path, page_size)?;
        return Some(film_strip(frame));
    }
    let img = if is_ffmpeg_still(path) {
        ffmpeg_picture(path, page_size)?
    } else if is_raw(path) {
        raw_preview(path)?
    } else {
        decode_image(path)?
    };
    Some(fit(img, tier))
}

/// `img` scaled to fit `side` pixels. A photo many times that size is first averaged down in
/// blocks to about twice the size: filtering every pixel of a 12-megapixel picture took longer than
/// decoding it.
fn fit(img: image::DynamicImage, side: u32) -> image::DynamicImage {
    let block = img.width().max(img.height()) / side.max(1) / 2;
    let img = if block >= 2 { box_shrink(&img, block as usize).unwrap_or(img) } else { img };
    img.thumbnail(side, side)
}

/// Each `block`x`block` square of pixels averaged into one, from at most 4x4 evenly spaced samples
/// of it, so the cost follows the size of the result rather than of the photo. Only 8-bit pictures.
fn box_shrink(img: &image::DynamicImage, block: usize) -> Option<image::DynamicImage> {
    use image::DynamicImage as D;
    const TAPS: usize = 4;
    let channels = match img {
        D::ImageLuma8(_) => 1,
        D::ImageLumaA8(_) => 2,
        D::ImageRgb8(_) => 3,
        D::ImageRgba8(_) => 4,
        _ => return None,
    };
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (ow, oh) = (w / block, h / block);
    if ow == 0 || oh == 0 {
        return None;
    }
    let taps = block.min(TAPS);
    // Sample offsets centred in each of `taps` equal parts of the block
    let offsets: Vec<usize> = (0..taps).map(|i| (2 * i + 1) * block / (2 * taps)).collect();
    let count = (taps * taps) as u32;
    let src = img.as_bytes();
    let mut out = Vec::with_capacity(ow * oh * channels);
    for oy in 0..oh {
        for ox in 0..ow {
            let mut sum = [0u32; 4];
            for &dy in &offsets {
                let row = (oy * block + dy) * w;
                for &dx in &offsets {
                    let at = (row + ox * block + dx) * channels;
                    for c in 0..channels {
                        sum[c] += src[at + c] as u32;
                    }
                }
            }
            for c in sum.iter().take(channels) {
                out.push(((c + count / 2) / count) as u8);
            }
        }
    }
    let (ow, oh) = (ow as u32, oh as u32);
    match channels {
        1 => image::GrayImage::from_raw(ow, oh, out).map(D::ImageLuma8),
        2 => image::GrayAlphaImage::from_raw(ow, oh, out).map(D::ImageLumaA8),
        3 => image::RgbImage::from_raw(ow, oh, out).map(D::ImageRgb8),
        _ => image::RgbaImage::from_raw(ow, oh, out).map(D::ImageRgba8),
    }
}

/// Read at most `limit` bytes from `path`, returning `None` if the file exceeds the limit.
fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

pub(crate) fn render_svg(path: &Path, px: u32) -> Option<image::DynamicImage> {
    let data = read_bounded(path, MAX_SVG_BYTES)?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    let mut pixmap = resvg::tiny_skia::Pixmap::new((size.width() * scale).ceil().max(1.0) as u32, (size.height() * scale).ceil().max(1.0) as u32)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let rgba: Vec<u8> = pixmap.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }).collect();
    image::RgbaImage::from_raw(pixmap.width(), pixmap.height(), rgba).map(image::DynamicImage::ImageRgba8)
}

/// A frame from near the start of a video, grabbed with ffmpeg. Skips the first second to avoid
/// black intros, and falls back to the very first frame for clips shorter than that.
fn video_frame(path: &Path, size: u32) -> Option<image::DynamicImage> {
    ["1", "0"].into_iter().find_map(|seek| ffmpeg_frame(path, seek, size))
}

/// One decoded frame at `seek` seconds. Seeks on the input without decoding up to the exact
/// position (the nearest keyframe is plenty for a preview), skips every non-video stream, runs
/// single-threaded because several workers already decode side by side, and writes an
/// uncompressed BMP so no time goes into PNG compression. Pixel aspect ratio is applied, so
/// anamorphic clips keep their shape; rotation metadata is applied by ffmpeg itself.
fn ffmpeg_frame(path: &Path, seek: &str, size: u32) -> Option<image::DynamicImage> {
    use std::process::Command;
    let filter = format!("scale=iw*sar:ih,scale={size}:{size}:force_original_aspect_ratio=decrease,setsar=1");
    let mut command = Command::new("ffmpeg");
    command
        .args(["-nostdin", "-hide_banner", "-v", "quiet", "-threads", "1", "-noaccurate_seek", "-ss", seek, "-i"])
        .arg(path)
        .args(["-an", "-sn", "-dn", "-frames:v", "1", "-vf", &filter, "-f", "image2pipe", "-c:v", "bmp", "-"]);
    image::load_from_memory(&run_with_timeout(command)?).ok()
}

// An audio file's attached cover is read with `ffmpeg_picture`. Most tracks have none, and every
// worker spends its time on the whole folder, so ffmpeg is held to the cheap path: it probes only
// the start of the file (tags sit there), never seeks or decodes audio, maps just the first video
// stream, and scales it down itself so a multi-megapixel cover never travels through the pipe.

/// The first picture stream of a file, scaled to fit `size`: an audio file's attached cover or a
/// still image in a format only ffmpeg reads.
fn ffmpeg_picture(path: &Path, size: u32) -> Option<image::DynamicImage> {
    use std::process::Command;
    let filter = format!("scale={size}:{size}:force_original_aspect_ratio=decrease");
    let mut command = Command::new("ffmpeg");
    command
        .args(["-nostdin", "-hide_banner", "-v", "quiet", "-threads", "1", "-probesize", "1M", "-analyzeduration", "0", "-i"])
        .arg(path)
        .args(["-map", "0:v:0?", "-an", "-sn", "-dn", "-frames:v", "1", "-vf", &filter, "-f", "image2pipe", "-c:v", "bmp", "-"]);
    image::load_from_memory(&run_with_timeout(command)?).ok()
}

/// The largest JPEG inside a RAW file. Every camera maker embeds one (a preview or the full
/// frame), so the file is scanned for JPEG start markers instead of parsing each vendor's container.
fn raw_preview(path: &Path) -> Option<image::DynamicImage> {
    const SOI: [u8; 3] = [0xFF, 0xD8, 0xFF];
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(48 * 1024 * 1024).read_to_end(&mut bytes).ok()?;
    let starts: Vec<usize> = bytes.windows(3).enumerate().filter(|(_, w)| *w == SOI).map(|(i, _)| i).collect();
    // A JPEG runs to the next one's start (or the end), so the largest span is the biggest picture
    let best = starts
        .iter()
        .enumerate()
        .map(|(n, &at)| (at, starts.get(n + 1).copied().unwrap_or(bytes.len())))
        .max_by_key(|(a, b)| b - a)?;
    let jpeg = &bytes[best.0..best.1];
    // The decoder stops at the end-of-image marker, so trailing sensor data does not matter
    image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()
}

/// Stdout of a helper process. A broken or stalled input must not hold a worker forever, so the
/// process is killed after `VIDEO_TIMEOUT`.
fn run_with_timeout(mut command: std::process::Command) -> Option<Vec<u8>> {
    use std::process::Stdio;
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let pid = child.id();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        if done_rx.recv_timeout(VIDEO_TIMEOUT).is_err() {
            // SAFETY: plain signal to the child this function spawned; a stale pid only fails
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        }
    });
    let mut bytes = Vec::new();
    let read = child.stdout.take()?.take(MAX_HELPER_OUTPUT_BYTES + 1).read_to_end(&mut bytes);
    let _ = child.wait();
    let _ = done_tx.send(());
    read.ok()?;
    (bytes.len() as u64 <= MAX_HELPER_OUTPUT_BYTES).then_some(bytes)
}

/// Paints sprocket-hole bands over the left and right edges, so a video reads as film.
fn film_strip(frame: image::DynamicImage) -> image::DynamicImage {
    use image::Rgba;
    let mut img = frame.to_rgba8();
    let (w, h) = img.dimensions();
    let band = (w as f32 * 0.09).round().max(6.0) as u32;
    let hole = (band as f32 * 0.5).round().max(3.0) as u32;
    let pitch = hole * 2;
    if w < band * 4 || h < pitch * 2 {
        return image::DynamicImage::ImageRgba8(img);
    }
    for x in (0..band).chain(w - band..w) {
        for y in 0..h {
            img.put_pixel(x, y, Rgba([8, 8, 8, 255]));
        }
    }
    let inset = (band - hole) / 2;
    let mut y = pitch / 2;
    while y + hole <= h {
        for left in [inset, w - band + inset] {
            for dy in 0..hole {
                for dx in 0..hole {
                    img.put_pixel(left + dx, y + dy, Rgba([235, 235, 235, 255]));
                }
            }
        }
        y += pitch;
    }
    image::DynamicImage::ImageRgba8(img)
}

/// First page of a PDF, rendered on the CPU.
fn render_pdf(path: &Path, size: u32) -> Option<image::DynamicImage> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{PixmapSettings, RenderCache, RenderSettings};

    let pdf = Pdf::new(read_bounded(path, MAX_PDF_BYTES)?).ok()?;
    let page = pdf.pages().first()?;
    let (width, height) = page.render_dimensions();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let scale = size as f32 / width.max(height);
    if (width * scale) as u16 == 0 || (height * scale) as u16 == 0 {
        return None;
    }
    let pixmap = hayro::render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE },
    );
    // The page is painted over an opaque white background, so premultiplied equals straight alpha
    let buf = image::RgbaImage::from_raw(pixmap.width() as u32, pixmap.height() as u32, pixmap.data_as_u8_slice().to_vec())?;
    Some(image::DynamicImage::ImageRgba8(buf))
}

/// The cover of an e-book or comic, whichever way its format stores one.
fn book_or_document_cover(path: &Path) -> Option<image::DynamicImage> {
    if is_fb2(path) {
        return fb2_cover(path);
    }
    if is_mobi(path) {
        return mobi_cover(path);
    }
    if is_comic(path) {
        // Comics are routinely misnamed: a .cbr is often a zip and a .cbz sometimes a rar
        return zip_comic_page(path).or_else(|| seven_zip_comic_page(path));
    }
    embedded_preview(path)
}

fn is_page_picture(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    !name.contains("__macosx") && [".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp"].iter().any(|e| name.ends_with(e))
}

/// EPUBs and drawing programs carry a ready-made picture of their first page or cover.
fn embedded_preview(path: &Path) -> Option<image::DynamicImage> {
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(path).ok()?)).ok()?;
    let names: Vec<String> = archive.file_names().filter_map(Result::ok).map(|n| n.into_owned()).collect();
    let pick = |wanted: &dyn Fn(&str) -> bool| names.iter().find(|n| wanted(&n.to_ascii_lowercase())).cloned();
    let entry = match extension(path).as_deref() {
        Some("epub") => epub_cover_entry(&mut archive, &names)?,
        Some("kra") => pick(&|n| n == "preview.png" || n == "mergedimage.png")?,
        _ => pick(&|n| n == "thumbnails/thumbnail.png")?,
    };
    read_zip_image(&mut archive, &entry)
}

fn read_zip_image<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>, entry: &str) -> Option<image::DynamicImage> {
    let mut bytes = Vec::new();
    archive.by_name(entry).ok()?.take(32 * 1024 * 1024).read_to_end(&mut bytes).ok()?;
    image::load_from_memory(&bytes).ok()
}

/// Value of `attr="..."` inside one XML tag.
fn xml_attr<'a>(tag: &'a str, attr: &str) -> Option<&'a str> {
    let at = tag.find(&format!("{attr}=\""))? + attr.len() + 2;
    let rest = &tag[at..];
    Some(&rest[..rest.find('"')?])
}

/// The cover named by the book's package file, falling back to any picture called "cover".
fn epub_cover_entry<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>, names: &[String]) -> Option<String> {
    let mut read_text = |name: &str| -> Option<String> {
        let mut text = String::new();
        archive.by_name(name).ok()?.take(4 * 1024 * 1024).read_to_string(&mut text).ok()?;
        Some(text)
    };
    let mut by_package = || -> Option<String> {
        let container = read_text("META-INF/container.xml")?;
        let opf_path = xml_attr(&container[container.find("<rootfile")?..], "full-path")?.to_string();
        let opf = read_text(&opf_path)?;
        let base = opf_path.rsplit_once('/').map_or("", |(dir, _)| dir);
        let items: Vec<&str> = opf.split("<item ").skip(1).filter_map(|i| i.split('>').next()).collect();
        let cover_id = opf
            .split("<meta ")
            .skip(1)
            .filter_map(|m| m.split('>').next())
            .find(|m| xml_attr(m, "name") == Some("cover"))
            .and_then(|m| xml_attr(m, "content"));
        let item = items
            .iter()
            .find(|i| xml_attr(i, "properties").is_some_and(|p| p.split_whitespace().any(|p| p == "cover-image")))
            .or_else(|| items.iter().find(|i| cover_id.is_some() && xml_attr(i, "id") == cover_id))?;
        let href = xml_attr(item, "href")?;
        let joined = if base.is_empty() { href.to_string() } else { format!("{base}/{href}") };
        // Package hrefs may be percent-encoded and use ../
        let mut parts: Vec<&str> = Vec::new();
        for part in joined.split('/') {
            match part {
                ".." => {
                    parts.pop();
                }
                "." | "" => {}
                p => parts.push(p),
            }
        }
        let wanted = parts.join("/").replace("%20", " ");
        names.iter().find(|n| **n == wanted).cloned()
    };
    by_package().or_else(|| {
        names
            .iter()
            .find(|n| {
                let n = n.to_ascii_lowercase();
                n.contains("cover") && is_page_picture(&n)
            })
            .cloned()
    })
}

/// First page of a comic stored as a zip.
fn zip_comic_page(path: &Path) -> Option<image::DynamicImage> {
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(path).ok()?)).ok()?;
    let first = archive.file_names().filter_map(Result::ok).filter(|n| is_page_picture(n)).map(|n| n.into_owned()).min_by_key(|n| n.to_ascii_lowercase())?;
    read_zip_image(&mut archive, &first)
}

/// First page of a comic stored as rar or 7z, via the `7z` tool when it is installed.
fn seven_zip_comic_page(path: &Path) -> Option<image::DynamicImage> {
    use std::process::Command;
    if std::fs::metadata(path).ok()?.len() > MAX_DOC_BYTES * 8 {
        return None;
    }
    let tool = ["7z", "7zz", "7za"].into_iter().find(|t| Command::new(t).arg("-h").output().is_ok())?;
    let mut list = Command::new(tool);
    list.args(["l", "-ba", "-slt", "-sccUTF-8"]).arg(path);
    let listing = String::from_utf8(run_with_timeout(list)?).ok()?;
    let first = listing
        .lines()
        .filter_map(|l| l.strip_prefix("Path = "))
        .filter(|n| is_page_picture(n))
        .min_by_key(|n| n.to_ascii_lowercase())?
        .to_string();
    let mut extract = Command::new(tool);
    extract.args(["e", "-so", "-y"]).arg(path).arg(&first);
    image::load_from_memory(&run_with_timeout(extract)?).ok()
}

/// Cover of a FictionBook: a base64 `<binary>` the `<coverpage>` points at. `.fb2.zip` holds the same file zipped.
fn fb2_cover(path: &Path) -> Option<image::DynamicImage> {
    use base64::Engine;
    if std::fs::metadata(path).ok()?.len() > MAX_DOC_BYTES {
        return None;
    }
    let raw = if extension(path).as_deref() == Some("zip") {
        let mut archive = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(path).ok()?)).ok()?;
        let name = archive.file_names().filter_map(Result::ok).find(|n| n.to_ascii_lowercase().ends_with(".fb2"))?.into_owned();
        let mut bytes = Vec::new();
        archive.by_name(&name).ok()?.take(MAX_DOC_BYTES).read_to_end(&mut bytes).ok()?;
        bytes
    } else {
        std::fs::read(path).ok()?
    };
    // Only the ASCII markup is searched, so a legacy encoding in the text does not matter
    let text = String::from_utf8_lossy(&raw);
    let cover = text.find("<coverpage").and_then(|at| {
        let page = &text[at..text[at..].find("</coverpage>").map_or(text.len(), |e| at + e)];
        // Matches l:href and xlink:href as well as a bare href
        xml_attr(&page[page.find("<image")?..], "href")
    });
    let id = cover.map(|c| c.trim_start_matches('#'));
    let mut rest = &text[..];
    while let Some(at) = rest.find("<binary") {
        let tag_end = rest[at..].find('>')? + at;
        let body_end = rest[tag_end..].find("</binary>")? + tag_end;
        let tag = &rest[at..tag_end];
        let is_cover = match id {
            Some(id) => xml_attr(tag, "id") == Some(id),
            None => xml_attr(tag, "content-type").is_some_and(|t| t.starts_with("image/")),
        };
        if is_cover {
            let data: String = rest[tag_end + 1..body_end].chars().filter(|c| !c.is_whitespace()).collect();
            let bytes = base64::engine::general_purpose::STANDARD.decode(data).ok()?;
            return image::load_from_memory(&bytes).ok();
        }
        rest = &rest[body_end..];
    }
    None
}

/// Cover of a Kindle/Mobipocket book. The file is a Palm database; an optional EXTH header names the
/// cover as an offset from the first image record.
fn mobi_cover(path: &Path) -> Option<image::DynamicImage> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let u32_at = |b: &[u8], at: usize| b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]));
    let mut head = [0u8; 78];
    file.read_exact(&mut head).ok()?;
    let count = u16::from_be_bytes([head[76], head[77]]) as usize;
    let mut table = vec![0u8; count * 8];
    file.read_exact(&mut table).ok()?;
    let offset = |i: usize| u32_at(&table, i * 8).map(u64::from);
    let len = file.metadata().ok()?.len();
    let record = |file: &mut std::fs::File, i: usize, cap: u64| -> Option<Vec<u8>> {
        let start = offset(i)?;
        let end = if i + 1 < count { offset(i + 1)? } else { len };
        let size = end.checked_sub(start)?.min(cap);
        let mut buf = vec![0u8; size as usize];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    let first = record(&mut file, 0, 1024 * 1024)?;
    if first.get(16..20)? != b"MOBI" {
        return None;
    }
    let header_len = u32_at(&first, 20)? as usize;
    let first_image = u32_at(&first, 108 + 16)? as usize;
    let has_exth = u32_at(&first, 128 + 16)? & 0x40 != 0;
    let mut cover = None;
    if has_exth {
        let exth = 16 + header_len;
        if first.get(exth..exth + 4)? == b"EXTH" {
            let mut at = exth + 12;
            for _ in 0..u32_at(&first, exth + 8)? {
                let (kind, size) = (u32_at(&first, at)?, u32_at(&first, at + 4)? as usize);
                if kind == 201 {
                    cover = u32_at(&first, at + 8);
                }
                if size < 8 {
                    break;
                }
                at += size;
            }
        }
    }
    let index = first_image + cover.unwrap_or(0) as usize;
    if index >= count {
        return None;
    }
    image::load_from_memory(&record(&mut file, index, 16 * 1024 * 1024)?).ok()
}

/// On-disk preview cache under `$XDG_CACHE_HOME/ski/thumbs`. An entry is named by a hash of the
/// file's path, size and modification time, so editing a file makes its old entry unreachable
/// instead of stale. Everything is best effort: any I/O error just means a cache miss.
/// The thumbnail cache shared with Nautilus and other file managers (freedesktop thumbnail
/// specification): `~/.cache/thumbnails/<size>/<md5 of the file's URI>.png`, with the URI and
/// modification time stored in the PNG. Only plain pictures go in, so what other programs find
/// there is the picture itself.
mod shared_cache {
    use std::path::{Path, PathBuf};

    use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

    /// What `g_filename_to_uri` leaves unescaped in a path.
    const KEEP: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~').remove(b'!').remove(b'$').remove(b'&').remove(b'\'').remove(b'(').remove(b')').remove(b'*').remove(b'+').remove(b',').remove(b';').remove(b'=').remove(b':').remove(b'@').remove(b'/');

    fn folder(tier: u32) -> Option<&'static str> {
        match tier {
            128 => Some("normal"),
            256 => Some("large"),
            512 => Some("x-large"),
            _ => None,
        }
    }

    fn uri(path: &Path) -> Option<String> {
        use std::os::unix::ffi::OsStrExt;
        let text = std::str::from_utf8(path.as_os_str().as_bytes()).ok()?;
        Some(format!("file://{}", utf8_percent_encode(text, KEEP)))
    }

    fn mtime(path: &Path) -> Option<u64> {
        std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
    }

    fn entry(uri: &str, tier: u32) -> Option<PathBuf> {
        let name = format!("{:x}.png", md5::compute(uri.as_bytes()));
        Some(dirs::cache_dir()?.join("thumbnails").join(folder(tier)?).join(name))
    }

    pub fn read(path: &Path, tier: u32) -> Option<image::DynamicImage> {
        let uri = uri(path)?;
        let file = entry(&uri, tier)?;
        let bytes = std::fs::read(&file).ok()?;
        let reader = png::Decoder::new(std::io::Cursor::new(&bytes)).read_info().ok()?;
        let text = |key: &str| reader.info().uncompressed_latin1_text.iter().find(|c| c.keyword == key).map(|c| c.text.clone());
        // Made for this very file as it is now, not for an older version or another one with the same name
        if text("Thumb::URI").is_some_and(|u| u != uri) || text("Thumb::MTime")?.trim().parse::<u64>().ok()? != mtime(path)? {
            return None;
        }
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()
    }

    pub fn write(path: &Path, tier: u32, img: &image::DynamicImage) {
        let (Some(uri), Some(mtime)) = (uri(path), mtime(path)) else { return };
        let Some(file) = entry(&uri, tier) else { return };
        let Some(dir) = file.parent() else { return };
        if !dir.exists() {
            use std::os::unix::fs::DirBuilderExt;
            if std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).is_err() {
                return;
            }
        }
        let rgba = img.to_rgba8();
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, rgba.width(), rgba.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        let ok = encoder.add_text_chunk("Thumb::URI".into(), uri).is_ok()
            && encoder.add_text_chunk("Thumb::MTime".into(), mtime.to_string()).is_ok()
            && encoder.add_text_chunk("Thumb::Size".into(), size.to_string()).is_ok()
            && encoder.write_header().and_then(|mut w| w.write_image_data(rgba.as_raw())).is_ok();
        if !ok {
            return;
        }
        // Written under a temporary name first, so a reader never sees half a file
        use std::os::unix::fs::OpenOptionsExt;
        let tmp = file.with_extension(format!("tmp{}", std::process::id()));
        let written = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| std::io::Write::write_all(&mut f, &out));
        if written.is_err() || std::fs::rename(&tmp, &file).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

mod cache {
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::{Duration, SystemTime};

    /// Bump when the way previews are drawn changes, which orphans every older entry.
    const VERSION: u8 = 3;
    /// The least recently used entries go once the cache passes this size.
    const MAX_BYTES: u64 = 768 * 1024 * 1024;
    /// A file with no preview is tried again after this long, in case ffmpeg was installed meanwhile.
    const MISSING_TTL: Duration = Duration::from_secs(24 * 60 * 60);

    pub struct Key(u64);

    fn root() -> Option<&'static Path> {
        static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
        ROOT.get_or_init(|| dirs::cache_dir().map(|d| d.join("ski").join("thumbs").join(format!("v{VERSION}")))).as_deref()
    }

    pub fn key(path: &Path) -> Option<Key> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?;
        // FNV-1a: unlike `DefaultHasher`, the value is the same in every build and process
        let mut hash = 0xcbf29ce484222325u64;
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                hash = (hash ^ *b as u64).wrapping_mul(0x100000001b3);
            }
        };
        feed(path.as_os_str().as_bytes());
        feed(&meta.len().to_le_bytes());
        feed(&modified.as_nanos().to_le_bytes());
        Some(Key(hash))
    }

    fn entry(key: &Key, bucket: &str, ext: &str) -> Option<PathBuf> {
        let name = format!("{:016x}", key.0);
        Some(root()?.join(bucket).join(&name[..2]).join(format!("{name}.{ext}")))
    }

    pub fn read(key: &Key, tier: u32) -> Option<image::DynamicImage> {
        let file = entry(key, &tier.to_string(), "png")?;
        let mut bytes = Vec::new();
        std::fs::File::open(&file).ok()?.read_to_end(&mut bytes).ok()?;
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok();
        match img {
            // Marks the entry as recently used for the trimmer
            Some(_) => {
                let _ = std::fs::File::options().write(true).open(&file).and_then(|f| f.set_modified(SystemTime::now()));
            }
            // A truncated write from a crash; let it be rendered again
            None => {
                let _ = std::fs::remove_file(&file);
            }
        }
        img
    }

    pub fn write(key: &Key, tier: u32, img: &image::DynamicImage) {
        let Some(file) = entry(key, &tier.to_string(), "png") else { return };
        let mut png = std::io::Cursor::new(Vec::new());
        let encoder = image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Adaptive);
        if img.write_with_encoder(encoder).is_ok() {
            store(&file, png.get_ref());
        }
    }

    pub fn known_missing(key: &Key) -> bool {
        let Some(marker) = entry(key, "none", "none") else { return false };
        std::fs::metadata(marker).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age < MISSING_TTL))
    }

    pub fn mark_missing(key: &Key) {
        if let Some(marker) = entry(key, "none", "none") {
            store(&marker, &[]);
        }
    }

    /// Written under a temporary name first, so a reader never sees half a file.
    fn store(file: &Path, bytes: &[u8]) {
        let Some(dir) = file.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let tmp = file.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&tmp, bytes).is_err() || std::fs::rename(&tmp, file).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Deletes the least recently used entries when the cache is over its limit.
    pub fn trim_in_background() {
        std::thread::spawn(|| {
            let Some(root) = root() else { return };
            let mut entries = Vec::new();
            let mut total = 0u64;
            let mut stack = vec![root.to_path_buf()];
            while let Some(dir) = stack.pop() {
                let Ok(read) = std::fs::read_dir(&dir) else { continue };
                for item in read.flatten() {
                    let Ok(meta) = item.metadata() else { continue };
                    if meta.is_dir() {
                        stack.push(item.path());
                    } else {
                        total += meta.len();
                        entries.push((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len(), item.path()));
                    }
                }
            }
            if total <= MAX_BYTES {
                return;
            }
            entries.sort();
            // Down to 80% so the next few writes do not trigger another sweep
            for (_, len, path) in entries {
                if total <= MAX_BYTES / 5 * 4 {
                    break;
                }
                if std::fs::remove_file(path).is_ok() {
                    total -= len;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PDF: &str = "%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 300]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj\n4 0 obj<</Length 60>>stream\nBT /F1 24 Tf 20 150 Td (Hello) Tj ET 0 0 1 rg 20 20 80 40 re f\nendstream endobj\n5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\ntrailer<</Root 1 0 R/Size 6>>\n%%EOF";

    #[test]
    fn renders_first_pdf_page() {
        let dir = std::env::temp_dir().join("ski-thumb-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample.pdf");
        std::fs::write(&file, PDF).unwrap();
        let img = decode_thumbnail(&file).expect("pdf preview");
        assert_eq!(img.size[1], PAGE_MIN as usize);
        assert!(img.size[0] < img.size[1]);
        assert!(img.pixels.iter().any(|p| p.b() > 200 && p.r() < 50), "blue rectangle missing");
    }

    #[test]
    fn grabs_a_video_frame() {
        let dir = std::env::temp_dir().join("ski-thumb-video-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clip.mp4");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "quiet", "-y", "-f", "lavfi", "-i", "color=c=red:s=320x180:d=0.5", "-pix_fmt", "yuv420p"])
            .arg(&file)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return; // no ffmpeg here, nothing to check
        }
        let img = decode_thumbnail(&file).expect("video preview");
        // Keeps the clip's 16:9 shape instead of a square crop, with sprocket bands on both sides
        assert_eq!(img.size[0], PAGE_MIN as usize);
        assert!(img.size[1] < img.size[0]);
        assert!(img.pixels.iter().any(|p| p.r() > 200 && p.g() < 60), "frame missing");
        assert!(img.pixels.iter().any(|p| p.r() > 220 && p.g() > 220 && p.b() > 220), "sprocket holes missing");
    }

    fn png(color: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(40, 60, image::Rgb(color));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn zip_of(file: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(file).unwrap());
        for (name, data) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    fn is_red(img: &egui::ColorImage) -> bool {
        img.pixels.iter().any(|p| p.r() > 200 && p.g() < 50 && p.b() < 50)
    }

    #[test]
    fn previews_are_cached_per_tier_and_follow_the_file() {
        let dir = std::env::temp_dir().join("ski-thumb-cache-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("pic-{}.png", std::process::id()));
        std::fs::write(&file, png([255, 0, 0])).unwrap();
        let budget = Arc::new(DecodeBudget::new(DECODE_BYTE_BUDGET, MAX_PDF_CONCURRENCY));
        let never = || false;
        let key = cache::key(&file).unwrap();
        assert!(cache::read(&key, 128).is_none());
        let DecodeResult::Ready(first) = cached_thumbnail(&file, 128, &budget, &never) else {
            panic!("preview");
        };
        let hit = cache::read(&key, 128).expect("written to the cache");
        assert_eq!(first.to_rgba8().into_raw(), hit.to_rgba8().into_raw());
        assert!(cache::read(&key, 256).is_none(), "tiers are cached separately");
        // Changing the file changes its key, so the old entry is never served
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, png([0, 0, 255])).unwrap();
        assert!(cache::read(&cache::key(&file).unwrap(), 128).is_none());
        let bare = dir.join(format!("bare-{}.mp3", std::process::id()));
        std::fs::write(&bare, b"not audio").unwrap();
        assert!(matches!(cached_thumbnail(&bare, 128, &budget, &never), DecodeResult::Missing));
        assert!(cache::known_missing(&cache::key(&bare).unwrap()));
    }

    #[test]
    fn a_changed_file_is_drawn_again_even_when_its_old_preview_is_loaded() {
        let ctx = egui::Context::default();
        let mut thumbs = Thumbnails::new(&ctx);
        let dir = std::env::temp_dir().join(format!("ski-thumb-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("pic.png");
        let broken = dir.join("later.png");
        std::fs::write(&broken, b"not a picture yet").unwrap();
        std::fs::write(&file, png([255, 0, 0])).unwrap();
        let wait = |thumbs: &mut Thumbnails, done: &dyn Fn(&Thumbnails) -> bool| {
            for _ in 0..200 {
                thumbs.poll(&ctx);
                if done(thumbs) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        };

        let (first, bad) = (stamp_of(&file), stamp_of(&broken));
        thumbs.preload([(file.as_path(), first), (broken.as_path(), bad)].into_iter());
        assert!(wait(&mut thumbs, &|t| t.made.get(&file) == Some(&first) && t.failed.contains_key(&broken)));

        // Changed while another folder was open: reopening the folder brings the new picture
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, png([0, 0, 255])).unwrap();
        std::fs::write(&broken, png([0, 255, 0])).unwrap();
        let (second, fixed) = (stamp_of(&file), stamp_of(&broken));
        assert_ne!(first, second);
        thumbs.preload([(file.as_path(), second), (broken.as_path(), fixed)].into_iter());
        assert!(wait(&mut thumbs, &|t| t.made.get(&file) == Some(&second) && t.made.contains_key(&broken)), "stale or failed previews are not retried");

        // Changed while the folder is open: asking for it again is enough, and the old picture stays until then
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, png([255, 255, 0])).unwrap();
        thumbs.ensure(&file);
        assert!(thumbs.get(&file).is_some());
        let third = stamp_of(&file);
        assert!(wait(&mut thumbs, &|t| t.made.get(&file) == Some(&third)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tier_follows_the_drawn_size() {
        assert_eq!(tier_for(32.0), 128);
        assert_eq!(tier_for(129.0), 256);
        assert_eq!(tier_for(256.0), 256);
        assert_eq!(tier_for(300.0), 512);
        assert_eq!(tier_for(5000.0), 512);
    }

    #[test]
    fn audio_shows_its_attached_cover() {
        let dir = std::env::temp_dir().join("ski-thumb-audio-test");
        std::fs::create_dir_all(&dir).unwrap();
        let cover = dir.join("cover.png");
        std::fs::write(&cover, png([255, 0, 0])).unwrap();
        let track = dir.join("track.mp3");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "quiet", "-y", "-f", "lavfi", "-i", "sine=d=0.5", "-i"])
            .arg(&cover)
            .args(["-map", "0:a", "-map", "1:v", "-c:v", "copy", "-id3v2_version", "3", "-disposition:v", "attached_pic"])
            .arg(&track)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return; // no ffmpeg or no mp3 encoder here
        }
        assert!(is_red(&decode_thumbnail(&track).expect("audio cover")));
        let bare = dir.join("bare.mp3");
        let made = std::process::Command::new("ffmpeg").args(["-v", "quiet", "-y", "-f", "lavfi", "-i", "sine=d=0.5"]).arg(&bare).status();
        if made.is_ok_and(|s| s.success()) {
            assert!(decode_thumbnail(&bare).is_none());
        }
    }

    #[test]
    fn comic_shows_its_first_page() {
        let dir = std::env::temp_dir().join("ski-thumb-comic-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("issue.cbz");
        zip_of(&file, &[("02.png", &png([0, 0, 255])), ("01.png", &png([255, 0, 0])), ("notes.txt", b"x")]);
        assert!(is_red(&decode_thumbnail(&file).expect("comic preview")));
    }

    #[test]
    fn epub_uses_the_cover_named_in_its_package() {
        let dir = std::env::temp_dir().join("ski-thumb-epub-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("book.epub");
        let container = br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;
        let opf = br#"<package><metadata><meta name="cover" content="c"/></metadata><manifest><item id="c" href="img/front.png" media-type="image/png"/></manifest></package>"#;
        zip_of(&file, &[("META-INF/container.xml", container), ("OEBPS/content.opf", opf), ("OEBPS/img/other.png", &png([0, 0, 255])), ("OEBPS/img/front.png", &png([255, 0, 0]))]);
        assert!(is_red(&decode_thumbnail(&file).expect("epub preview")));
    }

    #[test]
    fn fb2_shows_the_cover_binary() {
        use base64::Engine;
        let dir = std::env::temp_dir().join("ski-thumb-fb2-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("novel.fb2");
        let b64 = |c| base64::engine::general_purpose::STANDARD.encode(png(c));
        let xml = format!(
            r##"<FictionBook xmlns:l="http://www.w3.org/1999/xlink"><description><title-info><coverpage><image l:href="#cover.png"/></coverpage></title-info></description><body/><binary id="other.png" content-type="image/png">{}</binary><binary id="cover.png" content-type="image/png">{}</binary></FictionBook>"##,
            b64([0, 0, 255]),
            b64([255, 0, 0])
        );
        std::fs::write(&file, xml).unwrap();
        assert!(is_red(&decode_thumbnail(&file).expect("fb2 preview")));
    }

    /// Makes a half-second red clip in the container the extension implies; `None` without ffmpeg.
    fn red_clip(dir: &Path, name: &str, extra: &[&str]) -> Option<PathBuf> {
        let file = dir.join(name);
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "quiet", "-y", "-f", "lavfi", "-i", "color=c=red:s=160x90:d=0.5,format=yuv420p"])
            .args(extra)
            .arg(&file)
            .status();
        made.is_ok_and(|s| s.success()).then_some(file)
    }

    #[test]
    fn common_video_containers_get_a_frame() {
        let dir = std::env::temp_dir().join("ski-thumb-containers-test");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, extra) in [
            ("a.mkv", vec![]),
            ("b.avi", vec!["-c:v", "mpeg4"]),
            ("c.webm", vec!["-c:v", "libvpx"]),
            ("d.mov", vec![]),
            ("e.ts", vec!["-c:v", "mpeg2video", "-f", "mpegts"]),
            ("f.m2ts", vec!["-c:v", "mpeg2video", "-f", "mpegts"]),
            ("g.wmv", vec!["-c:v", "wmv2"]),
            ("h.flv", vec!["-c:v", "flv"]),
            ("i.mpg", vec!["-c:v", "mpeg1video"]),
            ("j.3gp", vec!["-c:v", "h263", "-s", "176x144"]),
        ] {
            let Some(file) = red_clip(&dir, name, &extra) else { continue }; // encoder not built in
            let img = decode_thumbnail(&file).unwrap_or_else(|| panic!("{name} has no preview"));
            assert!(img.pixels.iter().any(|p| p.r() > 150 && p.g() < 90), "{name} frame is not red");
        }
    }

    #[test]
    fn typescript_is_not_a_transport_stream() {
        let dir = std::env::temp_dir().join("ski-thumb-ts-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("app.ts");
        std::fs::write(&file, "export const x: number = 1;\n".repeat(20)).unwrap();
        assert!(has_preview(&file) && decode_thumbnail(&file).is_none());
    }

    #[test]
    fn pictures_in_other_formats_get_previews() {
        let dir = std::env::temp_dir().join("ski-thumb-stills-test");
        std::fs::create_dir_all(&dir).unwrap();
        // The image crate reads these itself
        for ext in ["tga", "ppm", "qoi", "ico", "tiff"] {
            let file = dir.join(format!("p.{ext}"));
            let rgba = image::RgbaImage::from_pixel(32, 32, image::Rgba([255, 0, 0, 255]));
            if image::DynamicImage::ImageRgba8(rgba).save(&file).is_ok() {
                assert!(is_red(&decode_thumbnail(&file).unwrap_or_else(|| panic!("{ext}"))), "{ext}");
            }
        }
        // ffmpeg reads these
        let file = dir.join("p.pcx");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "quiet", "-y", "-f", "lavfi", "-i", "color=c=red:s=64x48,format=rgb24", "-frames:v", "1"])
            .arg(&file)
            .status();
        if made.is_ok_and(|s| s.success()) {
            assert!(is_red(&decode_thumbnail(&file).expect("pcx")));
        }
    }

    #[test]
    fn raw_photo_shows_its_biggest_embedded_jpeg() {
        let jpeg = |side: u32, color: [u8; 3]| {
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(side, side, image::Rgb(color))).write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
            out.into_inner()
        };
        let mut raw = b"II*\0 sensor data ".to_vec();
        raw.extend(jpeg(16, [0, 0, 255]));
        raw.extend(b"more sensor data");
        raw.extend(jpeg(200, [255, 0, 0]));
        raw.extend(b"trailer");
        let dir = std::env::temp_dir().join("ski-thumb-raw-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shot.cr2");
        std::fs::write(&file, raw).unwrap();
        assert!(is_red(&decode_thumbnail(&file).expect("raw preview")));
    }

    fn wait_for(thumbs: &mut Thumbnails, ctx: &egui::Context, done: impl Fn(&Thumbnails) -> bool) -> bool {
        for _ in 0..250 {
            thumbs.poll(ctx);
            if done(thumbs) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn oversized_pdf_falls_back_without_decoding() {
        let dir = std::env::temp_dir().join(format!("ski-thumb-big-pdf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("huge.pdf");
        let f = std::fs::File::create(&file).unwrap();
        // Sparse file: uses no disk blocks while reporting a length past MAX_PDF_BYTES
        f.set_len(MAX_PDF_BYTES + 1).unwrap();
        drop(f);
        assert!(decode_thumbnail(&file).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn large_folder_preloads_bounded_window_and_loads_on_scroll() {
        let ctx = egui::Context::default();
        let mut thumbs = Thumbnails::new(&ctx);
        let dir = std::env::temp_dir().join(format!("ski-thumb-window-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let files: Vec<PathBuf> = (0..(INITIAL_PRELOAD + 60))
            .map(|i| {
                let p = dir.join(format!("doc-{i:04}.pdf"));
                std::fs::write(&p, PDF).unwrap();
                p
            })
            .collect();
        let entries: Vec<(PathBuf, Stamp)> = files.iter().map(|p| (p.clone(), stamp_of(p))).collect();

        thumbs.preload(entries.iter().map(|(p, s)| (p.as_path(), *s)));
        assert_eq!(thumbs.folder.len(), files.len());
        assert_eq!(thumbs.queued.len(), INITIAL_PRELOAD, "preload should only queue the initial window");

        // Scrolling far down prunes off-screen queued jobs and decodes the newly visible slice
        let far = &files[INITIAL_PRELOAD + 40..INITIAL_PRELOAD + 44];
        thumbs.set_visible(far.iter().map(PathBuf::as_path));
        assert!(wait_for(&mut thumbs, &ctx, |t| far.iter().all(|p| t.get(p).is_some())));
        // Untouched middle files were never decoded
        assert!(thumbs.get(&files[INITIAL_PRELOAD + 10]).is_none());
        assert!(!thumbs.made.contains_key(&files[INITIAL_PRELOAD + 10]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn texture_budget_evicts_off_screen_entries_of_the_active_folder() {
        let ctx = egui::Context::default();
        let mut thumbs = Thumbnails::new(&ctx);
        // Each 40x60 PNG scales to 85x128 RGBA (43,520 bytes); allow 4 textures in memory at once
        thumbs.texture_budget = 85 * 128 * 4 * 4;
        let dir = std::env::temp_dir().join(format!("ski-thumb-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let files: Vec<PathBuf> = (0..12)
            .map(|i| {
                let p = dir.join(format!("img-{i:02}.png"));
                std::fs::write(&p, png([(i * 20) as u8, 40, 120])).unwrap();
                p
            })
            .collect();
        let entries: Vec<(PathBuf, Stamp)> = files.iter().map(|p| (p.clone(), stamp_of(p))).collect();
        thumbs.preload(entries.iter().map(|(p, s)| (p.as_path(), *s)));

        // View the first 3 files, then scroll across the folder in 3-item windows
        for chunk in files.chunks(3) {
            thumbs.set_visible(chunk.iter().map(PathBuf::as_path));
            assert!(wait_for(&mut thumbs, &ctx, |t| chunk.iter().all(|p| t.get(p).is_some())));
        }
        assert!(thumbs.texture_bytes <= thumbs.texture_budget, "active-folder textures exceeded budget: {} > {}", thumbs.texture_bytes, thumbs.texture_budget);
        assert!(thumbs.textures.len() < files.len(), "off-screen active-folder textures were not evicted");
        // The currently visible last chunk must remain loaded
        for p in &files[9..12] {
            assert!(thumbs.get(p).is_some());
        }

        // Scrolling back to evicted files brings them back (from disk cache)
        let first = &files[0..3];
        thumbs.set_visible(first.iter().map(PathBuf::as_path));
        assert!(wait_for(&mut thumbs, &ctx, |t| first.iter().all(|p| t.get(p).is_some())));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn decode_budget_bounds_concurrency_and_cancels_cleanly() {
        let budget = Arc::new(DecodeBudget::new(100, 2));
        let never = || false;
        let p1 = budget.acquire(40, true, &never).expect("first pdf");
        let p2 = budget.acquire(40, true, &never).expect("second pdf");

        // A third concurrent PDF blocks until cancelled
        let cancel = Arc::new(AtomicBool::new(false));
        let b = Arc::clone(&budget);
        let c = Arc::clone(&cancel);
        let waiter = std::thread::spawn(move || b.acquire(10, true, &|| c.load(Ordering::Relaxed)).is_some());
        std::thread::sleep(std::time::Duration::from_millis(30));
        cancel.store(true, Ordering::Relaxed);
        budget.notify_all();
        assert!(!waiter.join().unwrap(), "cancelled waiter should not acquire a permit");

        // A large non-PDF that would exceed byte_limit also waits until existing permits drop
        let b = Arc::clone(&budget);
        let big = std::thread::spawn(move || b.acquire(80, false, &|| false).is_some());
        std::thread::sleep(std::time::Duration::from_millis(30));
        drop(p1);
        drop(p2);
        assert!(big.join().unwrap(), "large job should proceed once budget is released");
    }

    #[test]
    fn refresh_defers_off_screen_changes_until_scrolled_into_view() {
        let ctx = egui::Context::default();
        let mut thumbs = Thumbnails::new(&ctx);
        let dir = std::env::temp_dir().join(format!("ski-thumb-refresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let on_screen = dir.join("a.png");
        let off_screen = dir.join("b.png");
        std::fs::write(&on_screen, png([255, 0, 0])).unwrap();
        std::fs::write(&off_screen, png([0, 255, 0])).unwrap();
        thumbs.preload([(on_screen.as_path(), stamp_of(&on_screen)), (off_screen.as_path(), stamp_of(&off_screen))].into_iter());
        thumbs.set_visible([on_screen.as_path()].into_iter());
        assert!(wait_for(&mut thumbs, &ctx, |t| t.get(&on_screen).is_some()));

        // Modify the off-screen file and notify via refresh: should not decode while off screen
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&off_screen, png([0, 0, 255])).unwrap();
        let updated = stamp_of(&off_screen);
        thumbs.refresh(&off_screen);
        assert!(!thumbs.queued.contains_key(&off_screen));
        assert_eq!(thumbs.folder.get(&off_screen), Some(&updated));

        // Scrolling to it decodes the updated stamp
        thumbs.set_visible([off_screen.as_path()].into_iter());
        assert!(wait_for(&mut thumbs, &ctx, |t| t.made.get(&off_screen) == Some(&updated)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drop_shuts_down_when_result_channel_is_full() {
        let ctx = egui::Context::default();
        let mut thumbs = Thumbnails::new(&ctx);
        let dir = std::env::temp_dir().join(format!("ski-thumb-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Queue more files than RESULT_CHANNEL_CAP without polling, so workers hit channel backpressure
        let files: Vec<PathBuf> = (0..(RESULT_CHANNEL_CAP + 16))
            .map(|i| {
                let p = dir.join(format!("p-{i:02}.png"));
                std::fs::write(&p, png([200, (i * 5) as u8, 50])).unwrap();
                p
            })
            .collect();
        let entries: Vec<(PathBuf, Stamp)> = files.iter().map(|p| (p.clone(), stamp_of(p))).collect();
        thumbs.preload(entries.iter().map(|(p, s)| (p.as_path(), *s)));
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Dropping Thumbnails must unblock workers and return promptly
        let start = std::time::Instant::now();
        drop(thumbs);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}


//! Background image thumbnail loader. Every image in the current folder is queued as soon as
//! the folder opens; decoded square thumbnails are uploaded as textures on the UI thread.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use eframe::egui;

const THUMB_SIZE: u32 = 128;
const WORKERS: usize = 4;

struct Job {
    path: PathBuf,
    generation: u64,
}

pub struct Thumbnails {
    textures: HashMap<PathBuf, egui::TextureHandle>,
    queued: HashSet<PathBuf>,
    job_tx: mpsc::Sender<Job>,
    result_rx: mpsc::Receiver<(PathBuf, Option<egui::ColorImage>)>,
    generation: Arc<AtomicU64>,
}

pub fn is_image(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
    )
}

impl Thumbnails {
    pub fn new(ctx: &egui::Context) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (result_tx, result_rx) = mpsc::channel();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let generation = Arc::new(AtomicU64::new(0));

        for _ in 0..WORKERS {
            let job_rx = job_rx.clone();
            let result_tx = result_tx.clone();
            let generation = generation.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || loop {
                let job = match job_rx.lock().map(|rx| rx.recv()) {
                    Ok(Ok(job)) => job,
                    _ => return,
                };
                // Skip work for folders the user already left
                if job.generation != generation.load(Ordering::Relaxed) {
                    let _ = result_tx.send((job.path, None));
                    continue;
                }
                let image = decode_thumbnail(&job.path);
                let _ = result_tx.send((job.path, image));
                ctx.request_repaint();
            });
        }

        Self { textures: HashMap::new(), queued: HashSet::new(), job_tx, result_rx, generation }
    }

    /// Queue every image of a freshly opened folder. Cancels work queued for the previous one.
    pub fn preload<'a>(&mut self, paths: impl Iterator<Item = &'a Path>) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        // Anything that was queued but not finished was dropped by the workers
        self.queued.retain(|p| self.textures.contains_key(p));
        for path in paths {
            if is_image(path) && !self.textures.contains_key(path) && self.queued.insert(path.to_path_buf()) {
                let _ = self.job_tx.send(Job { path: path.to_path_buf(), generation });
            }
        }
    }

    /// Upload finished thumbnails; call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((path, image)) = self.result_rx.try_recv() {
            match image {
                Some(img) => {
                    let tex = ctx.load_texture(path.to_string_lossy(), img, egui::TextureOptions::LINEAR);
                    self.textures.insert(path, tex);
                }
                None => {
                    self.queued.remove(&path);
                }
            }
        }
    }

    pub fn get(&self, path: &Path) -> Option<&egui::TextureHandle> {
        self.textures.get(path)
    }
}

fn decode_thumbnail(path: &Path) -> Option<egui::ColorImage> {
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let cropped = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    let thumb = cropped.thumbnail(THUMB_SIZE, THUMB_SIZE).to_rgba8();
    let size = [thumb.width() as usize, thumb.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, thumb.as_raw()))
}

//! Background preview loader. Every previewable file in the current folder is queued as soon as
//! the folder opens: images, videos, SVGs, the first page of PDFs and the embedded preview of office
//! documents. Decoded pictures are uploaded as textures on the UI thread.
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use eframe::egui;

/// Square image previews.
const THUMB_SIZE: u32 = 128;
/// Document pages keep their aspect ratio and need more pixels to stay readable.
const PAGE_SIZE: u32 = 256;
const WORKERS: usize = 4;
/// PDFs and archives larger than this are not worth rendering for a tiny preview.
/// ffmpeg gets this long on one frame before it is killed.
const VIDEO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_DOC_BYTES: u64 = 64 * 1024 * 1024;

struct Job {
    path: PathBuf,
    generation: u64,
}

enum Outcome {
    Ready(egui::ColorImage),
    Cancelled,
    Failed,
}

pub struct Thumbnails {
    textures: HashMap<PathBuf, egui::TextureHandle>,
    queued: HashSet<PathBuf>,
    /// Files that have no preview; not retried until the app restarts.
    failed: HashSet<PathBuf>,
    job_tx: mpsc::Sender<Job>,
    result_rx: mpsc::Receiver<(PathBuf, Outcome)>,
    generation: Arc<AtomicU64>,
}

fn extension(path: &Path) -> Option<String> {
    path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase())
}

pub fn is_image(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"))
}

fn is_video(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("mp4" | "mkv" | "webm" | "avi" | "mov" | "flv" | "wmv" | "m4v" | "mpg" | "mpeg" | "ogv"))
}

fn is_svg(path: &Path) -> bool {
    extension(path).as_deref() == Some("svg")
}

fn is_page(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("pdf" | "odt" | "ods" | "odp" | "odg" | "docx" | "xlsx" | "pptx" | "epub")
    )
}

/// Whether the file gets a picture of its contents instead of a type icon.
pub fn has_preview(path: &Path) -> bool {
    is_image(path) || is_video(path) || is_svg(path) || is_page(path)
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
                    let _ = result_tx.send((job.path, Outcome::Cancelled));
                    continue;
                }
                // A malformed document must not take the worker down with it
                let decoded = std::panic::catch_unwind(|| decode_thumbnail(&job.path)).ok().flatten();
                let outcome = decoded.map_or(Outcome::Failed, Outcome::Ready);
                let _ = result_tx.send((job.path, outcome));
                ctx.request_repaint();
            });
        }

        Self {
            textures: HashMap::new(),
            queued: HashSet::new(),
            failed: HashSet::new(),
            job_tx,
            result_rx,
            generation,
        }
    }

    fn enqueue(&mut self, path: &Path, generation: u64) {
        if has_preview(path) && !self.textures.contains_key(path) && !self.failed.contains(path) && self.queued.insert(path.to_path_buf()) {
            let _ = self.job_tx.send(Job { path: path.to_path_buf(), generation });
        }
    }

    /// Queue every previewable file of a freshly opened folder. Cancels work queued for the previous one.
    pub fn preload<'a>(&mut self, paths: impl Iterator<Item = &'a Path>) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        // Anything that was queued but not finished was dropped by the workers
        self.queued.retain(|p| self.textures.contains_key(p));
        for path in paths {
            self.enqueue(path, generation);
        }
    }

    /// Queue one extra image (e.g. a custom folder icon) without cancelling the folder's own work.
    pub fn ensure(&mut self, path: &Path) {
        let generation = self.generation.load(Ordering::Relaxed);
        self.enqueue(path, generation);
    }

    /// Upload finished thumbnails; call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((path, outcome)) = self.result_rx.try_recv() {
            match outcome {
                Outcome::Ready(img) => {
                    let tex = ctx.load_texture(path.to_string_lossy(), img, egui::TextureOptions::LINEAR);
                    self.textures.insert(path, tex);
                }
                Outcome::Cancelled => {
                    self.queued.remove(&path);
                }
                Outcome::Failed => {
                    self.queued.remove(&path);
                    self.failed.insert(path);
                }
            }
        }
    }

    pub fn get(&self, path: &Path) -> Option<&egui::TextureHandle> {
        self.textures.get(path)
    }
}

fn to_color_image(img: &image::DynamicImage) -> egui::ColorImage {
    let rgba = img.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
}

fn decode_thumbnail(path: &Path) -> Option<egui::ColorImage> {
    if is_svg(path) {
        return crate::fs::apps::load_icon_image(path, THUMB_SIZE);
    }
    if is_page(path) {
        let size = std::fs::metadata(path).ok()?.len();
        if size > MAX_DOC_BYTES {
            return None;
        }
        let page = match extension(path).as_deref() {
            Some("pdf") => render_pdf(path)?,
            _ => embedded_preview(path)?,
        };
        return Some(to_color_image(&page.thumbnail(PAGE_SIZE, PAGE_SIZE)));
    }
    if is_video(path) {
        let frame = video_frame(path)?;
        return Some(to_color_image(&film_strip(frame)));
    }
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let cropped = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    Some(to_color_image(&cropped.thumbnail(THUMB_SIZE, THUMB_SIZE)))
}

/// A frame from near the start of a video, grabbed with ffmpeg. Skips the first second to avoid
/// black intros, and falls back to the very first frame for clips shorter than that.
fn video_frame(path: &Path) -> Option<image::DynamicImage> {
    ["1", "0"].into_iter().find_map(|seek| ffmpeg_frame(path, seek))
}

/// One decoded frame at `seek` seconds. Seeks on the input without decoding up to the exact
/// position (the nearest keyframe is plenty for a preview), skips every non-video stream, runs
/// single-threaded because several workers already decode side by side, and writes an
/// uncompressed BMP so no time goes into PNG compression. Pixel aspect ratio is applied, so
/// anamorphic clips keep their shape; rotation metadata is applied by ffmpeg itself.
fn ffmpeg_frame(path: &Path, seek: &str) -> Option<image::DynamicImage> {
    use std::process::{Command, Stdio};
    let size = PAGE_SIZE;
    let filter = format!("scale=iw*sar:ih,scale={size}:{size}:force_original_aspect_ratio=decrease,setsar=1");
    let mut child = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-v", "quiet", "-threads", "1", "-noaccurate_seek", "-ss", seek, "-i"])
        .arg(path)
        .args(["-an", "-sn", "-dn", "-frames:v", "1", "-vf", &filter, "-f", "image2pipe", "-c:v", "bmp", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // A broken or stalled file must not hold a worker forever
    let pid = child.id();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        if done_rx.recv_timeout(VIDEO_TIMEOUT).is_err() {
            // SAFETY: plain signal to the child this function spawned; a stale pid only fails
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        }
    });
    let mut bytes = Vec::new();
    let read = child.stdout.take()?.read_to_end(&mut bytes);
    let _ = child.wait();
    let _ = done_tx.send(());
    read.ok()?;
    image::load_from_memory(&bytes).ok()
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
fn render_pdf(path: &Path) -> Option<image::DynamicImage> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{PixmapSettings, RenderCache, RenderSettings};

    let pdf = Pdf::new(std::fs::read(path).ok()?).ok()?;
    let page = pdf.pages().first()?;
    let (width, height) = page.render_dimensions();
    let scale = PAGE_SIZE as f32 / width.max(height).max(1.0);
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

/// Office documents and e-books carry a ready-made picture of their first page or cover.
fn embedded_preview(path: &Path) -> Option<image::DynamicImage> {
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(path).ok()?)).ok()?;
    let names: Vec<String> = archive.file_names().filter_map(Result::ok).map(|n| n.into_owned()).collect();
    let pick = |wanted: &dyn Fn(&str) -> bool| names.iter().find(|n| wanted(&n.to_ascii_lowercase())).cloned();
    let entry = match extension(path).as_deref() {
        Some("epub") => pick(&|n| n.contains("cover") && (n.ends_with(".jpg") || n.ends_with(".jpeg") || n.ends_with(".png"))),
        Some("docx" | "xlsx" | "pptx") => pick(&|n| n.starts_with("docprops/thumbnail.")),
        _ => pick(&|n| n == "thumbnails/thumbnail.png"),
    }?;
    let mut bytes = Vec::new();
    archive.by_name(&entry).ok()?.take(16 * 1024 * 1024).read_to_end(&mut bytes).ok()?;
    image::load_from_memory(&bytes).ok()
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
        assert_eq!(img.size[1], PAGE_SIZE as usize);
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
        assert_eq!(img.size[0], PAGE_SIZE as usize);
        assert!(img.size[1] < img.size[0]);
        assert!(img.pixels.iter().any(|p| p.r() > 200 && p.g() < 60), "frame missing");
        assert!(img.pixels.iter().any(|p| p.r() > 220 && p.g() > 220 && p.b() > 220), "sprocket holes missing");
    }
}

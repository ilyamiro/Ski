//! Background preview loader. Every previewable file in the current folder is queued as soon as
//! the folder opens: images, videos, SVGs, the first page of PDFs and the covers of e-books and comics. Decoded pictures are uploaded as textures on the UI thread.
//!
//! Finished previews are also kept on disk (see `cache`), so reopening a folder, in this window or in
//! the next file dialog, never runs ffmpeg or a decoder again. Previews come in a few resolutions
//! that follow the icon size: zooming in re-renders at the next tier, once, and keeps the old
//! picture on screen until the sharper one arrives.
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use eframe::egui;

/// Preview resolutions, by the longest side in pixels.
const TIERS: [u32; 3] = [128, 256, 512];
/// Document pages and video frames keep their aspect ratio and need more pixels than a square
/// crop to stay readable, so they never render below this.
const PAGE_MIN: u32 = 256;
const WORKERS: usize = 4;
/// PDFs and archives larger than this are not worth rendering for a tiny preview.
/// ffmpeg gets this long on one frame before it is killed.
const VIDEO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_DOC_BYTES: u64 = 64 * 1024 * 1024;
/// A PDF is read whole to find its first page, so the limit is generous but not unbounded.
const MAX_PDF_BYTES: u64 = 1024 * 1024 * 1024;

struct Job {
    path: PathBuf,
    generation: u64,
    tier: u32,
}

enum Outcome {
    Ready(egui::ColorImage, u32),
    Cancelled,
    Failed,
}

pub struct Thumbnails {
    /// Each texture with the tier it was rendered at.
    textures: HashMap<PathBuf, (egui::TextureHandle, u32)>,
    queued: HashSet<PathBuf>,
    /// Resolution wanted for new and upgraded previews.
    tier: u32,
    /// Files of the open folder, and extra pictures such as folder icons, for re-queuing at a new tier.
    current: Vec<PathBuf>,
    extras: HashSet<PathBuf>,
    /// Files that have no preview; not retried until the app restarts.
    failed: HashSet<PathBuf>,
    job_tx: mpsc::Sender<Job>,
    result_rx: mpsc::Receiver<(PathBuf, Outcome)>,
    generation: Arc<AtomicU64>,
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
                let decoded = std::panic::catch_unwind(|| cached_thumbnail(&job.path, job.tier)).ok().flatten();
                let outcome = decoded.map_or(Outcome::Failed, |img| Outcome::Ready(to_color_image(&img), job.tier));
                let _ = result_tx.send((job.path, outcome));
                ctx.request_repaint();
            });
        }

        cache::trim_in_background();

        Self {
            textures: HashMap::new(),
            queued: HashSet::new(),
            tier: TIERS[0],
            current: Vec::new(),
            extras: HashSet::new(),
            failed: HashSet::new(),
            job_tx,
            result_rx,
            generation,
        }
    }

    fn enqueue(&mut self, path: &Path, generation: u64) {
        let sharp_enough = self.textures.get(path).is_some_and(|(_, tier)| *tier >= self.tier);
        if has_preview(path) && !sharp_enough && !self.failed.contains(path) && self.queued.insert(path.to_path_buf()) {
            let _ = self.job_tx.send(Job { path: path.to_path_buf(), generation, tier: self.tier });
        }
    }

    /// Queue every previewable file of a freshly opened folder. Cancels work queued for the previous one.
    pub fn preload<'a>(&mut self, paths: impl Iterator<Item = &'a Path>) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        // Anything that was queued but not finished was dropped by the workers
        self.queued.clear();
        self.current = paths.map(Path::to_path_buf).collect();
        self.extras.clear();
        for path in std::mem::take(&mut self.current) {
            self.enqueue(&path, generation);
            self.current.push(path);
        }
    }

    /// Queue one extra image (e.g. a custom folder icon) without cancelling the folder's own work.
    pub fn ensure(&mut self, path: &Path) {
        let generation = self.generation.load(Ordering::Relaxed);
        self.extras.insert(path.to_path_buf());
        self.enqueue(path, generation);
    }

    /// Tell the loader how many physical pixels the largest preview is drawn at. Crossing into a
    /// bigger tier re-renders what is already loaded; going smaller keeps the sharper pictures.
    pub fn want_pixels(&mut self, px: f32) {
        let tier = tier_for(px);
        if tier <= self.tier {
            self.tier = tier;
            return;
        }
        self.tier = tier;
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.queued.clear();
        let paths: Vec<PathBuf> = self.current.iter().chain(self.extras.iter()).cloned().collect();
        for path in paths {
            self.enqueue(&path, generation);
        }
    }

    /// Upload finished thumbnails; call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((path, outcome)) = self.result_rx.try_recv() {
            match outcome {
                Outcome::Ready(img, tier) => {
                    if tier >= self.tier {
                        self.queued.remove(&path);
                    }
                    // A slow low-resolution job can finish after its upgrade did
                    if self.textures.get(&path).is_some_and(|(_, have)| *have >= tier) {
                        continue;
                    }
                    let tex = ctx.load_texture(path.to_string_lossy(), img, egui::TextureOptions::LINEAR);
                    self.textures.insert(path, (tex, tier));
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
        self.textures.get(path).map(|(tex, _)| tex)
    }
}

fn to_color_image(img: &image::DynamicImage) -> egui::ColorImage {
    let rgba = img.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
}

/// The preview from the disk cache, or rendered now and cached for next time. Files that have no
/// preview are remembered too, so a folder of covers-less tracks does not spawn ffmpeg per file on
/// every visit.
fn cached_thumbnail(path: &Path, tier: u32) -> Option<image::DynamicImage> {
    let Some(key) = cache::key(path) else { return render_thumbnail(path, tier) };
    if let Some(hit) = cache::read(&key, tier) {
        return Some(hit);
    }
    if cache::known_missing(&key) {
        return None;
    }
    let rendered = render_thumbnail(path, tier);
    match &rendered {
        Some(img) => cache::write(&key, tier, img),
        None => cache::mark_missing(&key),
    }
    rendered
}

#[cfg(test)]
fn decode_thumbnail(path: &Path) -> Option<egui::ColorImage> {
    render_thumbnail(path, TIERS[0]).map(|img| to_color_image(&img))
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
        return Some(page.thumbnail(page_size, page_size));
    }
    if is_audio(path) {
        return Some(square(ffmpeg_picture(path, tier)?, tier));
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
        image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?
    };
    Some(square(img, tier))
}

/// Center crop to a square of at most `side` pixels.
fn square(img: image::DynamicImage, side: u32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    let edge = w.min(h);
    img.crop_imm((w - edge) / 2, (h - edge) / 2, edge, edge).thumbnail(side, side)
}

fn render_svg(path: &Path, px: u32) -> Option<image::DynamicImage> {
    let data = std::fs::read(path).ok()?;
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
    let read = child.stdout.take()?.read_to_end(&mut bytes);
    let _ = child.wait();
    let _ = done_tx.send(());
    read.ok()?;
    Some(bytes)
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

    let pdf = Pdf::new(std::fs::read(path).ok()?).ok()?;
    let page = pdf.pages().first()?;
    let (width, height) = page.render_dimensions();
    let scale = size as f32 / width.max(height).max(1.0);
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
mod cache {
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::{Duration, SystemTime};

    /// Bump when the way previews are drawn changes, which orphans every older entry.
    const VERSION: u8 = 2;
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
        let key = cache::key(&file).unwrap();
        assert!(cache::read(&key, 128).is_none());
        let first = cached_thumbnail(&file, 128).expect("preview");
        let hit = cache::read(&key, 128).expect("written to the cache");
        assert_eq!(first.to_rgba8().into_raw(), hit.to_rgba8().into_raw());
        assert!(cache::read(&key, 256).is_none(), "tiers are cached separately");
        // Changing the file changes its key, so the old entry is never served
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, png([0, 0, 255])).unwrap();
        assert!(cache::read(&cache::key(&file).unwrap(), 128).is_none());
        let bare = dir.join(format!("bare-{}.mp3", std::process::id()));
        std::fs::write(&bare, b"not audio").unwrap();
        assert!(cached_thumbnail(&bare, 128).is_none());
        assert!(cache::known_missing(&cache::key(&bare).unwrap()));
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
}

//! Detailed file information (the "Properties" view) plus the small helpers the list columns share.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use super::file_item::{format_bytes, FileItem};
use crate::i18n::{tr, trf, trn};

fn id_table(path: &str) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines() {
            let mut parts = line.split(':');
            if let (Some(name), _, Some(id)) = (parts.next(), parts.next(), parts.next())
                && let Ok(id) = id.parse::<u32>() {
                    map.entry(id).or_insert_with(|| name.to_owned());
                }
        }
    }
    map
}

pub fn user_name(uid: u32) -> String {
    static USERS: OnceLock<HashMap<u32, String>> = OnceLock::new();
    USERS.get_or_init(|| id_table("/etc/passwd")).get(&uid).cloned().unwrap_or_else(|| uid.to_string())
}

pub fn group_name(gid: u32) -> String {
    static GROUPS: OnceLock<HashMap<u32, String>> = OnceLock::new();
    GROUPS.get_or_init(|| id_table("/etc/group")).get(&gid).cloned().unwrap_or_else(|| gid.to_string())
}

/// `drwxr-xr-x` style permission string.
pub fn mode_string(mode: u32, is_dir: bool) -> String {
    let mut out = String::with_capacity(10);
    out.push(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    out
}

/// `2026-10-18 14:03` in local time.
pub fn format_datetime(time: Option<SystemTime>) -> String {
    let Some(time) = time else { return tr("Unknown").to_owned() };
    let secs = match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as libc::time_t,
        Err(e) => -(e.duration().as_secs() as libc::time_t),
    };
    // SAFETY: `tm` is plain data and `localtime_r` only writes into it.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            return tr("Unknown").to_owned();
        }
        tm
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

/// Live totals of a recursive folder scan running on a background thread.
#[derive(Default)]
pub struct ScanState {
    pub size: AtomicU64,
    pub files: AtomicU64,
    pub folders: AtomicU64,
    pub done: AtomicBool,
    cancel: AtomicBool,
}

pub struct FolderScan {
    pub state: Arc<ScanState>,
}

impl FolderScan {
    pub fn start(root: PathBuf) -> Self {
        let state = Arc::new(ScanState::default());
        let worker = Arc::clone(&state);
        std::thread::spawn(move || {
            walk(&root, &worker);
            worker.done.store(true, Ordering::Relaxed);
        });
        Self { state }
    }
}

impl Drop for FolderScan {
    fn drop(&mut self) {
        self.state.cancel.store(true, Ordering::Relaxed);
    }
}

fn walk(dir: &Path, state: &ScanState) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if state.cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            state.folders.fetch_add(1, Ordering::Relaxed);
            walk(&entry.path(), state);
        } else {
            state.files.fetch_add(1, Ordering::Relaxed);
            state.size.fetch_add(meta.len(), Ordering::Relaxed);
        }
    }
}

pub struct Properties {
    pub item: FileItem,
    pub kind: String,
    pub location: String,
    pub link_target: Option<String>,
    pub group: String,
    pub changed: Option<SystemTime>,
    pub child_count: Option<usize>,
    pub mime: String,
    /// Extra sections that depend on the kind of file (image size, line counts, ...).
    pub extra: Vec<(&'static str, Vec<(&'static str, String)>)>,
}

impl Properties {
    pub fn load(path: &Path) -> Option<Self> {
        let item = FileItem::from_path(path)?;

        #[cfg(unix)]
        let (gid, changed) = {
            use std::os::unix::fs::MetadataExt;
            match std::fs::metadata(path) {
                Ok(m) => (
                    m.gid(),
                    Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(m.ctime().max(0) as u64)),
                ),
                Err(_) => (0, None),
            }
        };
        #[cfg(not(unix))]
        let (gid, changed) = (0, None);

        let child_count = item
            .is_dir
            .then(|| std::fs::read_dir(path).map(|rd| rd.count()).ok())
            .flatten();

        let mime = if item.is_dir { String::new() } else { super::apps::mime_of(path) };
        let extra = type_details(&item, &mime);

        Some(Self {
            mime,
            extra,
            kind: describe(&item),
            location: path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
            link_target: item
                .is_symlink
                .then(|| std::fs::read_link(path).ok().map(|t| t.display().to_string()))
                .flatten(),
            group: group_name(gid),
            changed,
            child_count,
            item,
        })
    }

    /// Sections of label / value pairs in display order. Folder totals are added by the caller.
    pub fn groups(&self) -> Vec<(&'static str, Vec<(&'static str, String)>)> {
        let it = &self.item;
        let mut general = vec![(tr("Type"), self.kind.clone()), (tr("Location"), self.location.clone())];
        if let Some(target) = &self.link_target {
            general.push((tr("Links to"), target.clone()));
        }
        if !it.is_dir {
            general.insert(1, (tr("MIME type"), self.mime.clone()));
        }
        if it.is_dir {
            if let Some(n) = self.child_count {
                general.push((tr("Contents"), trn(n, "{} item", "{} items")));
            }
        } else {
            general.push((tr("Size"), trf("{} ({} bytes)", &[&format_bytes(it.size), &it.size])));
        }
        let mut groups = vec![(tr("General"), general)];
        groups.extend(self.extra.iter().cloned());
        groups.extend([
            (
                tr("Dates"),
                vec![
                    (tr("Modified"), format_datetime(it.modified)),
                    (tr("Created"), format_datetime(it.created)),
                    (tr("Accessed"), format_datetime(it.accessed)),
                    (tr("Changed"), format_datetime(self.changed)),
                ],
            ),
            (
                tr("Ownership"),
                vec![
                    (tr("Owner"), it.owner()),
                    (tr("Group"), self.group.clone()),
                ],
            ),
        ]);
        groups
    }
}

fn describe(item: &FileItem) -> String {
    let base = if item.is_dir {
        tr("Folder").to_owned()
    } else {
        let ext = item
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_uppercase())
            .unwrap_or_default();
        match (ext.is_empty(), item.file_type) {
            (true, t) => t.label().to_owned(),
            (false, super::file_item::FileType::Other) => trf("{} file", &[&ext]),
            (false, t) => format!("{ext} · {}", t.label()),
        }
    };
    if item.is_symlink { trf("Link to {}", &[&base.to_lowercase()]) } else { base }
}

type Section = (&'static str, Vec<(&'static str, String)>);

/// Sections specific to the kind of file: images get their pixel details, text files their size in lines.
fn type_details(item: &FileItem, mime: &str) -> Vec<Section> {
    if item.is_dir {
        return Vec::new();
    }
    let mut sections = Vec::new();
    if mime.starts_with("image/") {
        if let Some(rows) = image_details(&item.path) {
            sections.push((tr("Image"), rows));
        }
    } else if (mime.starts_with("text/") || mime.contains("json") || mime.contains("toml") || mime.contains("xml"))
        && let Some(rows) = text_details(&item.path, item.size) {
            sections.push((tr("Text"), rows));
        }
    if item.file_type == super::file_item::FileType::Executable {
        sections.push((tr("Program"), vec![(tr("Executable"), tr("Yes").to_owned())]));
    }
    sections
}

fn image_details(path: &Path) -> Option<Vec<(&'static str, String)>> {
    use image::ImageDecoder;
    let reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let format = reader.format();
    let decoder = reader.into_decoder().ok()?;
    let (w, h) = decoder.dimensions();
    let color = decoder.color_type();

    let channels = color.channel_count().max(1) as u32;
    let bits = color.bits_per_pixel() as u32 / channels;
    let model = match channels {
        1 => tr("Grayscale"),
        2 => tr("Grayscale + alpha"),
        3 => tr("RGB"),
        _ => tr("RGB + alpha"),
    };
    let divisor = gcd(w, h).max(1);
    let orientation = match w.cmp(&h) {
        std::cmp::Ordering::Greater => tr("Landscape"),
        std::cmp::Ordering::Less => tr("Portrait"),
        std::cmp::Ordering::Equal => tr("Square"),
    };

    let mut rows = vec![
        (tr("Dimensions"), format!("{w} × {h} px")),
        (tr("Megapixels"), format!("{:.2} MP", w as f64 * h as f64 / 1_000_000.0)),
        (tr("Aspect ratio"), format!("{}:{} · {orientation}", w / divisor, h / divisor)),
        (tr("Color"), trf("{}, {}-bit", &[&model, &bits])),
    ];
    if let Some(format) = format {
        rows.push((tr("Format"), format!("{format:?}").to_uppercase()));
    }
    Some(rows)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn text_details(path: &Path, size: u64) -> Option<Vec<(&'static str, String)>> {
    const LIMIT: u64 = 8 * 1024 * 1024;
    if size > LIMIT {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    Some(vec![
        (tr("Lines"), content.lines().count().to_string()),
        (tr("Words"), content.split_whitespace().count().to_string()),
        (tr("Characters"), content.chars().count().to_string()),
        (tr("Encoding"), "UTF-8".to_owned()),
    ])
}

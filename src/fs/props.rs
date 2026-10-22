//! Detailed file information (the "Properties" view) plus the small helpers the list columns share.
use std::collections::{HashMap, HashSet};
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
    /// Space on disk (allocated blocks) rather than the sum of file lengths.
    pub disk: AtomicU64,
    pub files: AtomicU64,
    pub folders: AtomicU64,
    pub done: AtomicBool,
    /// A folder could not be read, so the totals are a lower bound.
    pub partial: AtomicBool,
    cancel: AtomicBool,
}

impl ScanState {
    pub fn totals(&self) -> super::dirsize::Totals {
        super::dirsize::Totals {
            bytes: self.size.load(Ordering::Relaxed),
            disk: self.disk.load(Ordering::Relaxed),
            files: self.files.load(Ordering::Relaxed),
            folders: self.folders.load(Ordering::Relaxed),
            partial: self.partial.load(Ordering::Relaxed),
        }
    }
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

/// Adds up everything below `root` without recursion (deep trees cannot overflow the stack).
/// Links are not followed, a file with several hard links counts once, and other file systems
/// mounted inside the tree (`/proc`, a USB disk) are left out.
fn walk(root: &Path, state: &ScanState) {
    use std::os::unix::fs::MetadataExt;
    let root_dev = root.symlink_metadata().map(|m| m.dev()).ok();
    let mut linked: HashSet<(u64, u64)> = HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            state.partial.store(true, Ordering::Relaxed);
            continue;
        };
        for entry in entries {
            if state.cancel.load(Ordering::Relaxed) {
                return;
            }
            let Ok(entry) = entry else {
                state.partial.store(true, Ordering::Relaxed);
                continue;
            };
            // `DirEntry::metadata` does not follow a symbolic link
            let Ok(meta) = entry.metadata() else {
                state.partial.store(true, Ordering::Relaxed);
                continue;
            };
            if meta.is_dir() {
                if root_dev.is_some_and(|d| d != meta.dev()) {
                    continue;
                }
                state.folders.fetch_add(1, Ordering::Relaxed);
                state.disk.fetch_add(meta.blocks() * 512, Ordering::Relaxed);
                stack.push(entry.path());
            } else {
                if meta.nlink() > 1 && !linked.insert((meta.dev(), meta.ino())) {
                    continue;
                }
                state.files.fetch_add(1, Ordering::Relaxed);
                state.size.fetch_add(meta.len(), Ordering::Relaxed);
                state.disk.fetch_add(meta.blocks() * 512, Ordering::Relaxed);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ski-props-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn walk_counts_hard_links_once_and_does_not_follow_symlinks() {
        let dir = tree("walk");
        std::fs::create_dir_all(dir.join("sub/deeper")).unwrap();
        std::fs::write(dir.join("a.bin"), vec![1u8; 1000]).unwrap();
        std::fs::hard_link(dir.join("a.bin"), dir.join("sub/a-link.bin")).unwrap();
        std::fs::write(dir.join("sub/deeper/b.bin"), vec![2u8; 500]).unwrap();
        let outside = tree("outside");
        std::fs::write(outside.join("big.bin"), vec![3u8; 100_000]).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link-to-folder")).unwrap();
        std::os::unix::fs::symlink(outside.join("big.bin"), dir.join("link-to-file")).unwrap();

        let state = ScanState::default();
        walk(&dir, &state);
        let totals = state.totals();
        // The two names of one inode count once; the links count as links, not as their targets
        assert_eq!(totals.folders, 2);
        assert!(totals.bytes >= 1500 && totals.bytes < 2000, "{totals:?}");
        assert!(!totals.partial);
        assert!(totals.disk >= totals.bytes.min(4096));
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn an_unreadable_folder_makes_the_total_partial() {
        use std::os::unix::fs::PermissionsExt;
        // Root reads everything
        // SAFETY: geteuid has no preconditions
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let dir = tree("partial");
        std::fs::create_dir_all(dir.join("locked")).unwrap();
        std::fs::write(dir.join("ok.txt"), "12345").unwrap();
        std::fs::set_permissions(dir.join("locked"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let state = ScanState::default();
        walk(&dir, &state);
        std::fs::set_permissions(dir.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(state.totals().partial);
        assert_eq!(state.totals().bytes, 5);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cancelled_walk_stops_at_once() {
        let dir = tree("cancel");
        for i in 0..50 {
            std::fs::write(dir.join(format!("{i}.txt")), "x").unwrap();
        }
        let state = ScanState::default();
        state.cancel.store(true, Ordering::Relaxed);
        walk(&dir, &state);
        assert_eq!(state.files.load(Ordering::Relaxed), 0);

        // Dropping a running scan raises the flag its thread watches
        let scan = FolderScan::start(dir.clone());
        let state = Arc::clone(&scan.state);
        drop(scan);
        assert!(state.cancel.load(Ordering::Relaxed));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn deep_trees_do_not_overflow_the_stack() {
        let dir = tree("deep");
        let mut path = dir.clone();
        // Long relative path chain built level by level through the parent
        for _ in 0..200 {
            path.push("d");
            std::fs::create_dir_all(&path).unwrap();
        }
        let state = ScanState::default();
        walk(&dir, &state);
        assert_eq!(state.folders.load(Ordering::Relaxed), 200);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

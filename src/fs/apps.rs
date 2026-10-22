//! Installed applications (freedesktop `.desktop` entries), MIME lookup and "open with".
use std::collections::{HashMap, HashSet};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Debug, Clone)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    exec: String,
    mimes: Vec<String>,
    /// Raw `Icon=` value: an absolute path or a theme icon name.
    pub icon: Option<String>,
}

/// MIME type of a file, as the system's shared-mime-info database sees it.
pub fn mime_of(path: &Path) -> String {
    let queried = Command::new("xdg-mime")
        .args(["query", "filetype"])
        .arg(path)
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|m| !m.is_empty());
    queried.unwrap_or_else(|| "application/octet-stream".to_owned())
}

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("applications"));
    }
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
    dirs.extend(system.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("applications")));
    dirs
}

static APPS: RwLock<Option<Arc<Vec<AppEntry>>>> = RwLock::new(None);

/// The installed applications as last scanned. Blocks the calling thread when there is no scan yet.
pub fn installed() -> Arc<Vec<AppEntry>> {
    if let Some(apps) = APPS.read().ok().and_then(|a| a.clone()) {
        return apps;
    }
    rescan()
}

fn rescan() -> Arc<Vec<AppEntry>> {
    let apps = Arc::new(load_apps());
    if let Ok(mut slot) = APPS.write() {
        *slot = Some(apps.clone());
    }
    apps
}

/// Scan the `.desktop` files in the background so the first use does not stall the UI. Each window
/// rescans, as a resident Ski would otherwise miss the applications installed since it started.
pub fn preload() {
    std::thread::spawn(rescan);
}

/// Every graphical application in the XDG data dirs. Earlier dirs win.
pub fn load_apps() -> Vec<AppEntry> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in application_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(app) = parse_desktop(&id, &path) {
                apps.push(app);
            }
        }
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

fn parse_desktop(id: &str, path: &Path) -> Option<AppEntry> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut fields: HashMap<&str, &str> = HashMap::new();
    let mut in_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry
            && let Some((k, v)) = line.split_once('=') {
                fields.entry(k.trim()).or_insert(v.trim());
            }
    }
    let is_true = |key: &str| fields.get(key) == Some(&"true");
    if fields.get("Type") != Some(&"Application") || is_true("NoDisplay") || is_true("Hidden") || is_true("Terminal") {
        return None;
    }
    let mimes: Vec<String> = fields
        .get("MimeType")
        .map(|m| m.split(';').filter(|m| !m.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    Some(AppEntry {
        id: id.to_owned(),
        name: fields.get("Name")?.to_string(),
        exec: fields.get("Exec")?.to_string(),
        icon: fields.get("Icon").map(|i| i.to_string()).filter(|i| !i.is_empty()),
        mimes,
    })
}

/// `child parent` / `alias canonical` pairs from the shared-mime-info database.
fn mime_pairs(file: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for dir in application_dirs() {
        let Some(share) = dir.parent() else { continue };
        let Ok(content) = std::fs::read_to_string(share.join("mime").join(file)) else { continue };
        for line in content.lines() {
            if let Some((a, b)) = line.split_once(' ') {
                pairs.push((a.trim().to_owned(), b.trim().to_owned()));
            }
        }
    }
    pairs
}

/// `mime` followed by everything it is a kind of (aliases and parent types, most specific first).
/// A .docx is also an `application/zip`, which is why file managers offer archive tools for it.
pub fn mime_chain(mime: &str) -> Vec<String> {
    static ALIASES: OnceLock<HashMap<String, String>> = OnceLock::new();
    static PARENTS: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    let aliases = ALIASES.get_or_init(|| mime_pairs("aliases").into_iter().collect());
    let parents = PARENTS.get_or_init(|| {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (child, parent) in mime_pairs("subclasses") {
            map.entry(child).or_default().push(parent);
        }
        map
    });

    let start = aliases.get(mime).map(String::as_str).unwrap_or(mime);
    let mut chain = vec![start.to_owned()];
    let mut next = 0;
    while next < chain.len() {
        for parent in parents.get(&chain[next]).into_iter().flatten() {
            if !chain.contains(parent) {
                chain.push(parent.clone());
            }
        }
        next += 1;
    }
    if mime.starts_with("text/") && !chain.iter().any(|m| m == "text/plain") {
        chain.push("text/plain".to_owned());
    }
    chain
}

impl AppEntry {
    /// How specifically this app handles `chain` (0 = the exact type); `None` if it does not.
    fn rank(&self, chain: &[String]) -> Option<usize> {
        chain.iter().position(|c| self.mimes.iter().any(|m| m == c))
    }
}

/// Applications able to open `mime` (or any type it derives from): the system default first,
/// then the ones that handle the type most specifically.
pub fn apps_for<'a>(apps: &'a [AppEntry], mime: &str, default_id: Option<&str>) -> Vec<&'a AppEntry> {
    let chain = mime_chain(mime);
    let mut list: Vec<(usize, &AppEntry)> = apps.iter().filter_map(|a| a.rank(&chain).map(|r| (r, a))).collect();
    list.sort_by_key(|(rank, a)| (Some(a.id.as_str()) != default_id, *rank));
    list.into_iter().map(|(_, a)| a).collect()
}

/// Locate the image file of a desktop entry's `Icon=` value.
pub fn resolve_icon(icon: &str) -> Option<PathBuf> {
    let direct = Path::new(icon);
    if direct.is_absolute() {
        return direct.is_file().then(|| direct.to_path_buf());
    }
    const SIZES: [&str; 6] = ["64x64", "48x48", "128x128", "96x96", "256x256", "32x32"];
    static THEMES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    let themes = THEMES.get_or_init(|| {
        let mut themes = Vec::new();
        for dir in application_dirs() {
            let Some(share) = dir.parent() else { continue };
            let icons = share.join("icons");
            themes.push(icons.join("hicolor"));
            if let Ok(rd) = std::fs::read_dir(&icons) {
                themes.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.ends_with("hicolor")));
            }
        }
        themes
    });
    for theme in themes {
        for size in SIZES {
            for sub in ["apps", "mimetypes"] {
                let png = theme.join(size).join(sub).join(format!("{icon}.png"));
                if png.is_file() {
                    return Some(png);
                }
            }
        }
        for sub in ["scalable/apps", "apps/scalable", "scalable/mimetypes"] {
            let svg = theme.join(sub).join(format!("{icon}.svg"));
            if svg.is_file() {
                return Some(svg);
            }
        }
    }
    for dir in application_dirs() {
        let Some(share) = dir.parent() else { continue };
        for ext in ["png", "svg"] {
            let pix = share.join("pixmaps").join(format!("{icon}.{ext}"));
            if pix.is_file() {
                return Some(pix);
            }
        }
    }
    None
}

/// Decode a PNG / SVG icon file into a square-ish bitmap about `px` pixels on its long side.
pub fn load_icon_image(path: &Path, px: u32) -> Option<eframe::egui::ColorImage> {
    use eframe::egui::ColorImage;
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        let data = std::fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
        let size = tree.size();
        let scale = px as f32 / size.width().max(size.height());
        let mut pixmap = resvg::tiny_skia::Pixmap::new((size.width() * scale).ceil().max(1.0) as u32, (size.height() * scale).ceil().max(1.0) as u32)?;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        Some(ColorImage::from_rgba_premultiplied([pixmap.width() as usize, pixmap.height() as usize], pixmap.data()))
    } else {
        let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
        let img = img.thumbnail(px, px).to_rgba8();
        Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
    }
}

pub fn default_for(mime: &str) -> Option<String> {
    Command::new("xdg-mime")
        .args(["query", "default", mime])
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Make `app_id` the system-wide default for `mime` (writes `~/.config/mimeapps.list`).
pub fn set_default(mime: &str, app_id: &str) -> bool {
    Command::new("xdg-mime")
        .args(["default", app_id, mime])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut started) = (false, false);
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(cur);
    }
    args
}

pub fn launch(app: &AppEntry, file: &Path) -> std::io::Result<()> {
    let file = file.to_string_lossy();
    let mut used_file = false;
    let mut args: Vec<String> = Vec::new();
    for arg in split_exec(&app.exec) {
        let mut out = String::new();
        let mut chars = arg.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('f' | 'F' | 'u' | 'U') => {
                    out.push_str(&file);
                    used_file = true;
                }
                Some('%') => out.push('%'),
                _ => {}
            }
        }
        if !out.is_empty() {
            args.push(out);
        }
    }
    if !used_file {
        args.push(file.into_owned());
    }
    let Some((program, rest)) = args.split_first() else {
        return Err(std::io::Error::other("empty Exec line"));
    };
    let mut child = Command::new(program)
        .args(rest)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Per-file "always open with" choices: file path -> desktop entry id.
fn overrides_file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ski").join("open_with.json"))
}

pub fn load_overrides() -> HashMap<PathBuf, String> {
    overrides_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_overrides(map: &HashMap<PathBuf, String>) {
    let Some(file) = overrides_file() else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(map) {
        let _ = std::fs::write(file, json);
    }
}

/// Kinds of files the user picks one default application for.
pub struct Category {
    pub label: &'static str,
    pub icon: &'static str,
    pub mimes: &'static [&'static str],
}

pub const CATEGORIES: [Category; 6] = [
    Category {
        label: "Images",
        icon: crate::ui::icons::IMAGE,
        mimes: &["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp", "image/svg+xml", "image/tiff"],
    },
    Category {
        label: "Video",
        icon: crate::ui::icons::VIDEO,
        mimes: &["video/mp4", "video/x-matroska", "video/webm", "video/quicktime", "video/x-msvideo", "video/mpeg"],
    },
    Category {
        label: "Music",
        icon: crate::ui::icons::AUDIO,
        mimes: &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav", "audio/mp4", "audio/x-opus+ogg"],
    },
    Category {
        label: "Documents",
        icon: crate::ui::icons::DOCUMENT,
        mimes: &["application/pdf", "application/epub+zip"],
    },
    Category {
        label: "Text",
        icon: crate::ui::icons::CODE,
        mimes: &["text/plain", "text/markdown", "text/x-python", "text/x-csrc", "application/json", "application/x-shellscript"],
    },
    Category {
        label: "Archives",
        icon: crate::ui::icons::ARCHIVE,
        mimes: &["application/zip", "application/x-tar", "application/gzip", "application/x-7z-compressed", "application/vnd.rar", "application/x-xz-compressed-tar"],
    },
];

/// Make `app_id` the default for every type of `category` without blocking the UI.
pub fn set_category_default(category: &Category, app_id: &str) {
    let app_id = app_id.to_owned();
    let mimes = category.mimes;
    std::thread::spawn(move || {
        for mime in mimes {
            set_default(mime, &app_id);
        }
    });
}

/// Name patterns of the shared-mime-info database (`globs2`): `*.ext` ones by extension, the rest as globs.
struct MimeGlobs {
    by_extension: HashMap<String, Vec<(u32, String)>>,
    other: Vec<(u32, String, String)>,
}

fn mime_globs() -> &'static MimeGlobs {
    static GLOBS: OnceLock<MimeGlobs> = OnceLock::new();
    GLOBS.get_or_init(|| {
        let mut globs = MimeGlobs { by_extension: HashMap::new(), other: Vec::new() };
        let mut seen = std::collections::HashSet::new();
        for dir in application_dirs() {
            let Some(share) = dir.parent() else { continue };
            let Ok(content) = std::fs::read_to_string(share.join("mime").join("globs2")) else { continue };
            for line in content.lines().filter(|l| !l.starts_with('#')) {
                let mut parts = line.splitn(4, ':');
                let (Some(weight), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next()) else { continue };
                let Ok(weight) = weight.parse::<u32>() else { continue };
                if !seen.insert((mime.to_owned(), glob.to_owned())) {
                    continue;
                }
                match glob.strip_prefix("*.") {
                    Some(ext) if !ext.contains(['*', '?', '[']) => globs.by_extension.entry(ext.to_lowercase()).or_default().push((weight, mime.to_owned())),
                    _ => globs.other.push((weight, mime.to_owned(), glob.to_lowercase())),
                }
            }
        }
        globs
    })
}

/// MIME type of a file from its name alone, the way the shared-mime-info database assigns it
/// (longest matching extension first, then the heaviest pattern); `None` when no pattern matches.
pub fn mime_of_name(name: &str) -> Option<String> {
    let globs = mime_globs();
    let lower = name.to_lowercase();
    let mut best: Option<(u32, usize, &str)> = None;
    for (weight, mime, glob) in &globs.other {
        if super::glob::matches(glob, &lower, true) && best.is_none_or(|(w, l, _)| (*weight, glob.len()) > (w, l)) {
            best = Some((*weight, glob.len(), mime));
        }
    }
    // `a.tar.gz` is tried as `tar.gz` before `gz`
    for (i, _) in lower.match_indices('.') {
        if let Some(found) = globs.by_extension.get(&lower[i + 1..]) {
            let (weight, mime) = found.iter().max_by_key(|(w, _)| *w)?;
            let len = lower.len() - i;
            if best.is_none_or(|(w, l, _)| (*weight, len) > (w, l)) {
                best = Some((*weight, len, mime));
            }
            break;
        }
    }
    best.map(|(_, _, mime)| mime.to_owned())
}

/// Whether a file named `name` is of the type `pattern` (`image/png`, or `image/*` for a family),
/// counting the types it derives from: a `.docx` is an `application/zip`.
pub fn name_is_mime(name: &str, pattern: &str) -> bool {
    // Without a database nothing can be told apart, and hiding every file is worse than showing all
    if mime_globs().by_extension.is_empty() {
        return true;
    }
    let mime = mime_of_name(name).unwrap_or_else(|| "application/octet-stream".to_owned());
    let chain = mime_chain(&mime);
    match pattern.strip_suffix("/*") {
        Some(family) => chain.iter().any(|m| m.split_once('/').is_some_and(|(f, _)| f == family)),
        None => {
            let wanted = mime_chain(pattern).into_iter().next().unwrap_or_else(|| pattern.to_owned());
            chain.contains(&wanted)
        }
    }
}

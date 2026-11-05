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

static CATEGORY_DEFAULTS: RwLock<Option<Vec<Option<String>>>> = RwLock::new(None);

/// Query the current default of every category in the background (`xdg-mime` is a slow script, and
/// one run per category on the UI thread made the settings tab stall).
pub fn preload_category_defaults() {
    if let Ok(mut slot) = CATEGORY_DEFAULTS.write() {
        *slot = None;
    }
    std::thread::spawn(|| {
        let found = std::thread::scope(|s| {
            let jobs: Vec<_> = CATEGORIES.iter().map(|c| s.spawn(|| default_for(c.mimes[0]))).collect();
            jobs.into_iter().map(|j| j.join().ok().flatten()).collect::<Vec<_>>()
        });
        if let Ok(mut slot) = CATEGORY_DEFAULTS.write() {
            *slot = Some(found);
        }
    });
}

/// The defaults found by `preload_category_defaults`, once they are in.
pub fn category_defaults() -> Option<Vec<Option<String>>> {
    CATEGORY_DEFAULTS.read().ok().and_then(|d| d.clone())
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

/// The types a file named `name` may be, the way the shared-mime-info database assigns them: the
/// matches of the heaviest weight and then the longest pattern. Several when patterns tie (`.ogg` is
/// audio or video, `.ts` video or a translation), none when no pattern matches.
pub fn mime_candidates_of_name(name: &str) -> Vec<String> {
    let globs = mime_globs();
    let lower = name.to_lowercase();
    // (weight, pattern length, type)
    let mut found: Vec<(u32, usize, &str)> = Vec::new();
    for (weight, mime, glob) in &globs.other {
        // One pattern each, matching whole names: `*,v` is not a list, and `core` claims no `scorecard.png`
        let hit = if glob.contains(['*', '?', '[']) { super::glob::wildcard(glob, &lower) } else { *glob == lower };
        if hit {
            found.push((*weight, glob.len(), mime));
        }
    }
    // `a.tar.gz` is tried as `tar.gz` before `gz`
    for (i, _) in lower.match_indices('.') {
        if let Some(types) = globs.by_extension.get(&lower[i + 1..]) {
            // Measured as the pattern `*.ext` is, to compare with the other patterns
            let len = lower.len() - i + 1;
            found.extend(types.iter().map(|(weight, mime)| (*weight, len, mime.as_str())));
            break;
        }
    }
    let Some(best) = found.iter().map(|(w, l, _)| (*w, *l)).max() else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for (_, _, mime) in found.into_iter().filter(|(w, l, _)| (*w, *l) == best) {
        if !out.iter().any(|m| m == mime) {
            out.push(mime.to_owned());
        }
    }
    out
}

/// One test of a `magic` section: `value` (under `mask`) found at `offset`, or at any of the
/// `range` places from there.
struct MagicRule {
    indent: usize,
    offset: usize,
    range: usize,
    value: Vec<u8>,
    mask: Option<Vec<u8>>,
}

impl MagicRule {
    fn test(&self, data: &[u8]) -> bool {
        (self.offset..self.offset + self.range).any(|at| {
            let Some(window) = data.get(at..at + self.value.len()) else { return false };
            window.iter().zip(&self.value).enumerate().all(|(k, (byte, want))| {
                let mask = self.mask.as_ref().map_or(0xff, |m| m[k]);
                byte & mask == want & mask
            })
        })
    }
}

/// Whether one of `rules` (the first one's indent being the level) passes along with, when it
/// has any, one of the rules nested under it.
fn magic_rules_match(rules: &[MagicRule], data: &[u8]) -> bool {
    let mut i = 0;
    while i < rules.len() {
        let indent = rules[i].indent;
        let end = rules[i + 1..].iter().position(|r| r.indent <= indent).map_or(rules.len(), |p| i + 1 + p);
        let nested = &rules[i + 1..end];
        if rules[i].test(data) && (nested.is_empty() || magic_rules_match(nested, data)) {
            return true;
        }
        i = end;
    }
    false
}

/// One rule line of a `magic` file: `[indent]>offset=<u16 length><value>[&<mask>][~word][+range]\n`.
fn parse_magic_rule(s: &[u8]) -> Option<(MagicRule, &[u8])> {
    fn number(s: &[u8]) -> (usize, &[u8]) {
        let digits = s.iter().take_while(|b| b.is_ascii_digit()).count();
        let n = std::str::from_utf8(&s[..digits]).ok().and_then(|d| d.parse().ok()).unwrap_or(0);
        (n, &s[digits..])
    }
    let (indent, s) = number(s);
    let (offset, s) = number(s.strip_prefix(b">")?);
    let s = s.strip_prefix(b"=")?;
    let len = u16::from_be_bytes([*s.first()?, *s.get(1)?]) as usize;
    let mut value = s.get(2..2 + len)?.to_vec();
    let mut s = &s[2 + len..];
    let mut mask = None;
    if let Some(rest) = s.strip_prefix(b"&") {
        mask = Some(rest.get(..len)?.to_vec());
        s = &rest[len..];
    }
    let mut word = 1;
    if let Some(rest) = s.strip_prefix(b"~") {
        (word, s) = number(rest);
    }
    let mut range = 1;
    if let Some(rest) = s.strip_prefix(b"+") {
        (range, s) = number(rest);
    }
    // Values written as 16- or 32-bit host-order words are stored big-endian
    if cfg!(target_endian = "little") && word > 1 {
        for bytes in std::iter::once(&mut value).chain(mask.as_mut()) {
            bytes.chunks_mut(word).for_each(<[u8]>::reverse);
        }
    }
    // Anything a later version of the format adds is skipped up to the end of the line
    let end = s.iter().position(|&b| b == b'\n')?;
    Some((MagicRule { indent, offset, range: range.max(1), value, mask }, &s[end + 1..]))
}

/// The content tests of the shared-mime-info database (`magic`): (priority, type, rules), highest
/// priority first.
fn mime_magic() -> &'static [(u32, String, Vec<MagicRule>)] {
    static MAGIC: OnceLock<Vec<(u32, String, Vec<MagicRule>)>> = OnceLock::new();
    MAGIC.get_or_init(|| {
        let mut sections = Vec::new();
        for dir in application_dirs() {
            let Some(share) = dir.parent() else { continue };
            let Ok(bytes) = std::fs::read(share.join("mime").join("magic")) else { continue };
            let Some(mut rest) = bytes.strip_prefix(b"MIME-Magic\0\n") else { continue };
            while let Some(header) = rest.strip_prefix(b"[") {
                let Some(close) = header.iter().position(|&b| b == b']') else { break };
                let head = String::from_utf8_lossy(&header[..close]).into_owned();
                rest = header[close + 1..].strip_prefix(b"\n").unwrap_or(&header[close + 1..]);
                let mut rules = Vec::new();
                while !rest.is_empty() && rest[0] != b'[' {
                    let Some((rule, after)) = parse_magic_rule(rest) else { break };
                    rules.push(rule);
                    rest = after;
                }
                if let Some((priority, mime)) = head.split_once(':').and_then(|(p, m)| Some((p.parse().ok()?, m.to_owned()))) {
                    sections.push((priority, mime, rules));
                }
                if !rest.is_empty() && rest[0] != b'[' {
                    break;
                }
            }
        }
        sections.sort_by_key(|(priority, _, _)| std::cmp::Reverse(*priority));
        sections
    })
}

/// The type the content of a file shows, with the priority of the test that told it.
fn sniff_mime(data: &[u8]) -> Option<(u32, &'static str)> {
    mime_magic().iter().find(|(_, _, rules)| magic_rules_match(rules, data)).map(|(p, m, _)| (*p, m.as_str()))
}

/// Whether the start of a file reads as text: UTF-8 without control characters other than spacing.
fn looks_like_text(data: &[u8]) -> bool {
    let valid = match std::str::from_utf8(data) {
        Ok(_) => data.len(),
        // A character cut off at the end of the sample still counts
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => return false,
    };
    !data[..valid].iter().any(|&b| b < 0x20 && !b"\t\n\r\x0c\x08\x1b".contains(&b) || b == 0x7f)
}

/// The types a file may be, as GLib (and so GTK and Nautilus) decides: by name, and where the name
/// says nothing or is ambiguous, by the first 16 KiB of its content.
pub fn mime_candidates_of_path(path: &Path) -> Vec<String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let by_name = mime_candidates_of_name(&name);
    if by_name.len() == 1 {
        return by_name;
    }
    // Only regular files are read: a named pipe would never answer
    let mut data = Vec::new();
    if std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
        && let Ok(file) = std::fs::File::open(path)
    {
        use std::io::Read;
        let _ = file.take(16 * 1024).read_to_end(&mut data);
    }
    let sniffed = sniff_mime(&data).or_else(|| (!data.is_empty() && looks_like_text(&data)).then_some((0, "text/plain")));
    match sniffed {
        None if by_name.is_empty() => vec!["application/octet-stream".to_owned()],
        // Conflicting names the content cannot settle: any of them may be right, so all are kept
        None => by_name,
        Some((_, mime)) if by_name.is_empty() => vec![mime.to_owned()],
        Some((priority, mime)) if priority >= 80 => vec![mime.to_owned()],
        // A weaker content match breaks the tie between the names it agrees with
        Some((_, mime)) => match by_name.iter().find(|n| mime_chain(n).iter().any(|c| c == mime)) {
            Some(n) => vec![n.clone()],
            None => by_name,
        },
    }
}

/// Whether the file at `path` is of the type `pattern` (`image/png`, or `image/*` for a family),
/// counting the types it derives from: a `.docx` is an `application/zip`.
pub fn path_is_mime(path: &Path, pattern: &str) -> bool {
    // Without a database nothing can be told apart, and hiding every file is worse than showing all
    if mime_globs().by_extension.is_empty() {
        return true;
    }
    let wanted = mime_chain(pattern).into_iter().next().unwrap_or_else(|| pattern.to_owned());
    // Every file is a kind of `application/octet-stream`
    if wanted == "application/octet-stream" {
        return true;
    }
    mime_candidates_of_path(path).iter().any(|mime| {
        let chain = mime_chain(mime);
        match wanted.strip_suffix("/*") {
            Some(family) => chain.iter().any(|m| m.split_once('/').is_some_and(|(f, _)| f == family)),
            None => chain.contains(&wanted),
        }
    })
}

//! The System icon style: file and folder icons from the icon theme installed on the desktop
//! (Adwaita, Papirus, Breeze, ...), found the way the freedesktop icon theme specification
//! describes and picked by each file's MIME type from shared-mime-info, as Nautilus does.
//!
//! Lookups are kept, so each icon name is searched for once; pictures are rasterized on the UI
//! thread a few per frame, at the drawn size, and kept as textures.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

use crate::fs::file_item::{FileItem, FileType};
use crate::ui::icons;

/// New icons rasterized per frame; the others draw in the classic style until their turn.
const NEW_PER_FRAME: usize = 6;

// ---- The theme in use ----

/// The icon theme the desktop is set to: GNOME's setting through the portal, else GTK's or KDE's
/// configuration files, else Adwaita where it is installed, else the spec's own `hicolor`.
pub fn theme_name() -> String {
    if let Some(name) = crate::ui::appearance::current().icon_theme {
        return name;
    }
    static FROM_FILES: OnceLock<String> = OnceLock::new();
    FROM_FILES.get_or_init(|| from_config_files().unwrap_or_else(|| if theme_dir_exists("Adwaita") { "Adwaita".to_owned() } else { "hicolor".to_owned() })).clone()
}

fn from_config_files() -> Option<String> {
    let config = dirs::config_dir()?;
    let ini = |file: &str, section: &str, key: &str| -> Option<String> {
        let text = std::fs::read_to_string(config.join(file)).ok()?;
        let mut inside = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                inside = line == section;
            } else if inside && let Some((k, v)) = line.split_once('=') && k.trim() == key {
                let v = v.trim().trim_matches('"');
                return (!v.is_empty()).then(|| v.to_owned());
            }
        }
        None
    };
    ini("gtk-4.0/settings.ini", "[Settings]", "gtk-icon-theme-name")
        .or_else(|| ini("gtk-3.0/settings.ini", "[Settings]", "gtk-icon-theme-name"))
        .or_else(|| ini("kdeglobals", "[Icons]", "Theme"))
}

/// The folders icon themes live in, in the order the spec searches them.
fn base_dirs() -> &'static [PathBuf] {
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let mut out = Vec::new();
        if let Some(home) = dirs::home_dir() {
            out.push(home.join(".icons"));
        }
        out.extend(data_dirs().into_iter().map(|d| d.join("icons")));
        out.push(PathBuf::from("/usr/share/pixmaps"));
        out.retain(|d| d.is_dir());
        out.dedup();
        out
    })
}

/// `$XDG_DATA_HOME` and then `$XDG_DATA_DIRS`, with the spec's defaults.
fn data_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = dirs::data_dir().into_iter().collect();
    let system = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    out.extend(system.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    out
}

fn theme_dir_exists(name: &str) -> bool {
    base_dirs().iter().any(|b| b.join(name).join("index.theme").is_file())
}

// ---- Themes and lookup ----

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Debug, Clone)]
struct Subdir {
    path: String,
    size: u32,
    scale: u32,
    min: u32,
    max: u32,
    threshold: u32,
    kind: Kind,
}

impl Subdir {
    /// How far this folder's pictures are from `px` pixels; 0 when they fit.
    fn distance(&self, px: u32) -> u32 {
        let (size, min, max) = (self.size * self.scale, self.min * self.scale, self.max * self.scale);
        match self.kind {
            Kind::Fixed => size.abs_diff(px),
            Kind::Scalable if px < min => min - px,
            Kind::Scalable if px > max => px - max,
            Kind::Scalable => 0,
            Kind::Threshold if px + self.threshold * self.scale < size => size - self.threshold * self.scale - px,
            Kind::Threshold if px > size + self.threshold * self.scale => px - size - self.threshold * self.scale,
            Kind::Threshold => 0,
        }
    }
}

#[derive(Debug)]
struct Theme {
    subdirs: Vec<Subdir>,
    inherits: Vec<String>,
}

fn theme(name: &str) -> Option<Arc<Theme>> {
    static THEMES: OnceLock<Mutex<HashMap<String, Option<Arc<Theme>>>>> = OnceLock::new();
    let themes = THEMES.get_or_init(Default::default);
    if let Some(found) = themes.lock().unwrap_or_else(|e| e.into_inner()).get(name) {
        return found.clone();
    }
    let parsed = base_dirs().iter().find_map(|b| std::fs::read_to_string(b.join(name).join("index.theme")).ok()).map(|text| Arc::new(parse_index(&text)));
    themes.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_owned(), parsed.clone());
    parsed
}

fn parse_index(text: &str) -> Theme {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current = String::new();
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_owned();
        } else if let Some((k, v)) = line.split_once('=') {
            sections.entry(current.clone()).or_default().insert(k.trim().to_owned(), v.trim().to_owned());
        }
    }
    let main = sections.get("Icon Theme").cloned().unwrap_or_default();
    let list = |key: &str| main.get(key).map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
    let mut names = list("Directories");
    names.extend(list("ScaledDirectories"));
    let subdirs = names
        .into_iter()
        .filter_map(|path| {
            let section = sections.get(&path)?;
            let num = |key: &str| section.get(key).and_then(|v| v.parse::<u32>().ok());
            let size = num("Size")?;
            let kind = match section.get("Type").map(String::as_str) {
                Some("Fixed") => Kind::Fixed,
                Some("Scalable") => Kind::Scalable,
                _ => Kind::Threshold,
            };
            Some(Subdir { size, scale: num("Scale").unwrap_or(1).max(1), min: num("MinSize").unwrap_or(size), max: num("MaxSize").unwrap_or(size), threshold: num("Threshold").unwrap_or(2), kind, path })
        })
        .collect();
    Theme { subdirs, inherits: list("Inherits") }
}

/// The themes searched, in order: the chosen one, what it inherits from (depth first), `hicolor`.
fn chain(name: &str) -> Vec<(String, Arc<Theme>)> {
    let mut out: Vec<(String, Arc<Theme>)> = Vec::new();
    let mut stack = vec![name.to_owned()];
    while let Some(next) = stack.pop() {
        if out.iter().any(|(n, _)| *n == next) || out.len() > 16 {
            continue;
        }
        if let Some(theme) = theme(&next) {
            stack.extend(theme.inherits.iter().rev().cloned());
            out.push((next, theme));
        }
    }
    if !out.iter().any(|(n, _)| n == "hicolor") && let Some(hicolor) = theme("hicolor") {
        out.push(("hicolor".to_owned(), hicolor));
    }
    out
}

/// The picture for the first of `names` the theme has, the closest to `px` pixels.
fn find(theme_name: &str, names: &[&str], px: u32) -> Option<PathBuf> {
    type Key = (String, String, u32);
    static FOUND: OnceLock<Mutex<HashMap<Key, Option<PathBuf>>>> = OnceLock::new();
    let found = FOUND.get_or_init(Default::default);
    let key = (theme_name.to_owned(), names.join("|"), px);
    if let Some(hit) = found.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return hit.clone();
    }
    let themes = chain(theme_name);
    let result = names.iter().find_map(|name| {
        themes.iter().find_map(|(theme_name, theme)| lookup_in(base_dirs(), theme_name, theme, name, px)).or_else(|| pixmap(name))
    });
    found.lock().unwrap_or_else(|e| e.into_inner()).insert(key, result.clone());
    result
}

fn lookup_in(bases: &[PathBuf], theme_name: &str, theme: &Theme, name: &str, px: u32) -> Option<PathBuf> {
    // Symbolic icons are single-color glyphs, meant for buttons, not for files
    let symbolic = name.ends_with("-symbolic");
    let mut best: Option<(u32, PathBuf)> = None;
    for base in bases {
        let root = base.join(theme_name);
        if !root.is_dir() {
            continue;
        }
        for sub in theme.subdirs.iter().filter(|s| symbolic || !s.path.contains("symbolic")) {
            let distance = sub.distance(px);
            if best.as_ref().is_some_and(|(d, _)| *d <= distance) {
                continue;
            }
            for ext in ["svg", "png"] {
                let file = root.join(&sub.path).join(format!("{name}.{ext}"));
                if file.is_file() {
                    best = Some((distance, file));
                    break;
                }
            }
        }
    }
    best.map(|(_, path)| path)
}

/// The spec's last resort: a loose picture in `/usr/share/pixmaps`.
fn pixmap(name: &str) -> Option<PathBuf> {
    ["png", "svg"].into_iter().map(|ext| Path::new("/usr/share/pixmaps").join(format!("{name}.{ext}"))).find(|p| p.is_file())
}

// ---- MIME types ----

struct MimeDb {
    /// By lowercase suffix after a dot (`png`, `tar.gz`), and by whole lowercase file name.
    suffixes: HashMap<String, (u32, String)>,
    names: HashMap<String, (u32, String)>,
    /// A MIME type's own icon name, and its generic one.
    icons: HashMap<String, String>,
    generic: HashMap<String, String>,
}

fn mime_db() -> &'static MimeDb {
    static DB: OnceLock<MimeDb> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = MimeDb { suffixes: HashMap::new(), names: HashMap::new(), icons: HashMap::new(), generic: HashMap::new() };
        // Earlier data folders take precedence, so they are read last
        for dir in data_dirs().into_iter().rev().map(|d| d.join("mime")) {
            if let Ok(text) = std::fs::read_to_string(dir.join("globs2")) {
                for line in text.lines().filter(|l| !l.starts_with('#')) {
                    let mut parts = line.split(':');
                    let (Some(weight), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next()) else { continue };
                    let weight = weight.parse().unwrap_or(50);
                    let glob = glob.to_lowercase();
                    let (map, key) = match glob.strip_prefix("*.") {
                        Some(rest) if !rest.contains(['*', '?', '[']) => (&mut db.suffixes, rest.to_owned()),
                        _ if !glob.contains(['*', '?', '[']) => (&mut db.names, glob),
                        _ => continue,
                    };
                    let entry = map.entry(key).or_insert((0, String::new()));
                    if weight >= entry.0 {
                        *entry = (weight, mime.to_owned());
                    }
                }
            }
            for (file, map) in [("icons", &mut db.icons), ("generic-icons", &mut db.generic)] {
                if let Ok(text) = std::fs::read_to_string(dir.join(file)) {
                    for (mime, icon) in text.lines().filter_map(|l| l.split_once(':')) {
                        map.insert(mime.to_owned(), icon.to_owned());
                    }
                }
            }
        }
        db
    })
}

/// The MIME type of a file, from its name.
fn mime_of(name: &str) -> Option<&'static str> {
    let db = mime_db();
    let lower = name.to_lowercase();
    if let Some((_, mime)) = db.names.get(&lower) {
        return Some(mime);
    }
    // The longest matching suffix wins: `.tar.gz` over `.gz`
    lower.match_indices('.').find_map(|(at, _)| db.suffixes.get(&lower[at + 1..])).map(|(_, mime)| mime.as_str())
}

/// The icon names to try for a file, most specific first.
fn file_icon_names(item: &FileItem) -> Vec<String> {
    let mut out = Vec::new();
    if item.file_type == FileType::Symlink {
        out.push("inode-symlink".to_owned());
    }
    if let Some(mime) = mime_of(&item.name) {
        let db = mime_db();
        out.extend(db.icons.get(mime).cloned());
        out.push(mime.replace('/', "-"));
        out.extend(db.generic.get(mime).cloned());
        if let Some((media, _)) = mime.split_once('/') {
            out.push(format!("{media}-x-generic"));
        }
    }
    out.extend(type_icon_names(item.file_type).iter().map(|s| s.to_string()));
    out
}

/// Generic names for a kind of file, for files the MIME database does not know.
fn type_icon_names(kind: FileType) -> &'static [&'static str] {
    match kind {
        FileType::Directory => &["folder", "inode-directory"],
        FileType::Image => &["image-x-generic"],
        FileType::Audio => &["audio-x-generic"],
        FileType::Video => &["video-x-generic"],
        FileType::Archive => &["package-x-generic", "application-x-archive"],
        FileType::Code => &["text-x-script", "text-x-generic"],
        FileType::Document => &["x-office-document", "text-x-generic"],
        FileType::Executable => &["application-x-executable", "application-x-generic"],
        FileType::Symlink => &["inode-symlink", "application-x-generic"],
        FileType::Other => &["application-x-generic", "text-x-generic", "unknown"],
    }
}

/// The theme's name for a folder: the desktop's special folders have their own.
fn folder_icon_names(path: &Path) -> Vec<&'static str> {
    let special: [(Option<PathBuf>, &'static str); 9] = [
        (dirs::home_dir(), "user-home"),
        (dirs::desktop_dir(), "user-desktop"),
        (dirs::document_dir(), "folder-documents"),
        (dirs::download_dir(), "folder-download"),
        (dirs::audio_dir(), "folder-music"),
        (dirs::picture_dir(), "folder-pictures"),
        (dirs::video_dir(), "folder-videos"),
        (dirs::template_dir(), "folder-templates"),
        (dirs::public_dir(), "folder-publicshare"),
    ];
    let mut out: Vec<&'static str> = special.into_iter().filter(|(dir, _)| dir.as_deref() == Some(path)).map(|(_, name)| name).collect();
    out.extend(["folder", "inode-directory"]);
    out
}

/// Generic names for one of Ski's type glyphs, where only the glyph is known (samples, previews).
fn glyph_icon_names(glyph: &str) -> &'static [&'static str] {
    match glyph {
        g if g == icons::IMAGE => type_icon_names(FileType::Image),
        g if g == icons::AUDIO => type_icon_names(FileType::Audio),
        g if g == icons::VIDEO => type_icon_names(FileType::Video),
        g if g == icons::ARCHIVE => type_icon_names(FileType::Archive),
        g if g == icons::CODE => type_icon_names(FileType::Code),
        g if g == icons::DOCUMENT => type_icon_names(FileType::Document),
        g if g == icons::EXECUTABLE => type_icon_names(FileType::Executable),
        g if g == icons::SYMLINK => type_icon_names(FileType::Symlink),
        _ => type_icon_names(FileType::Other),
    }
}

// ---- Drawing ----

/// Rasterized icons of one window's egui context, by file and pixel size.
#[derive(Default)]
struct Textures {
    made: HashMap<(PathBuf, u32), Option<egui::TextureHandle>>,
    /// The frame the budget is counted for, and how much of it is used.
    frame: u64,
    spent: usize,
}

fn textures(ctx: &egui::Context) -> Arc<Mutex<Textures>> {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Arc<Mutex<Textures>>>(egui::Id::unique("ski_system_icons")).clone())
}

/// Sizes icons are rasterized at, so zooming does not make a texture per pixel.
fn bucket(px: f32) -> u32 {
    const SIZES: [u32; 10] = [16, 24, 32, 48, 64, 96, 128, 192, 256, 512];
    SIZES.into_iter().find(|&s| s as f32 >= px).unwrap_or(512)
}

/// Draws the first of `names` the theme has, filling a square of `side` points. False when there
/// is no such icon, or it is not ready yet (it is asked for again on the next frame).
fn paint_named(painter: &egui::Painter, names: &[&str], center: Pos2, side: f32, fade: f32) -> bool {
    let ctx = painter.ctx();
    let px = bucket(side * ctx.pixels_per_point());
    let Some(file) = find(&theme_name(), names, px) else { return false };
    let cache = textures(ctx);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    let key = (file, px);
    let tex = match cache.made.get(&key) {
        Some(tex) => tex.clone(),
        None => {
            let frame = ctx.cumulative_frame_nr();
            if cache.frame != frame {
                cache.frame = frame;
                cache.spent = 0;
            }
            if cache.spent >= NEW_PER_FRAME {
                ctx.request_repaint();
                return false;
            }
            cache.spent += 1;
            let tex = rasterize(&key.0, px).map(|img| ctx.load_texture(format!("system-icon:{}:{px}", key.0.display()), img, egui::TextureOptions::LINEAR));
            cache.made.insert(key, tex.clone());
            tex
        }
    };
    let Some(tex) = tex else { return false };
    let [w, h] = tex.size();
    let scale = side / w.max(h).max(1) as f32;
    let shown = Rect::from_center_size(center, Vec2::new(w as f32 * scale, h as f32 * scale));
    painter.image(tex.id(), shown, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE.gamma_multiply(fade));
    true
}

fn rasterize(file: &Path, px: u32) -> Option<egui::ColorImage> {
    let img = if file.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        crate::thumbs::render_svg(file, px)?
    } else {
        image::open(file).ok()?.thumbnail(px, px)
    };
    let rgba = img.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// Icons are drawn a little larger than the glyphs they stand in for, which leave room around them.
const SIDE: f32 = 1.25;

/// A file or folder's icon from the theme; false leaves it to the classic drawing.
pub fn paint_item(painter: &egui::Painter, item: &FileItem, center: Pos2, size: f32, fade: f32) -> bool {
    if item.is_dir {
        return paint_named(painter, &folder_icon_names(&item.path), center, size * SIDE, fade);
    }
    let names = file_icon_names(item);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    paint_named(painter, &names, center, size * SIDE, fade)
}

/// The theme's icon for one of Ski's type glyphs.
pub fn paint_glyph(painter: &egui::Painter, glyph: &str, center: Pos2, size: f32, fade: f32) -> bool {
    paint_named(painter, glyph_icon_names(glyph), center, size * SIDE, fade)
}

/// The theme's plain folder.
pub fn paint_folder(painter: &egui::Painter, center: Pos2, size: f32, fade: f32) -> bool {
    paint_named(painter, &["folder", "inode-directory"], center, size * SIDE, fade)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = "[Icon Theme]\nName=T\nInherits=Base,hicolor\nDirectories=16x16/mimetypes,scalable/places,symbolic/places\n\n[16x16/mimetypes]\nSize=16\nType=Fixed\n\n[scalable/places]\nSize=64\nMinSize=8\nMaxSize=512\nType=Scalable\n\n[symbolic/places]\nSize=16\nType=Scalable\n";

    #[test]
    fn index_files_are_read() {
        let theme = parse_index(INDEX);
        assert_eq!(theme.inherits, ["Base", "hicolor"]);
        assert_eq!(theme.subdirs.len(), 3);
        let fixed = &theme.subdirs[0];
        assert_eq!((fixed.kind, fixed.distance(16), fixed.distance(48)), (Kind::Fixed, 0, 32));
        let scalable = &theme.subdirs[1];
        assert_eq!((scalable.kind, scalable.distance(48), scalable.distance(4)), (Kind::Scalable, 0, 4));
    }

    #[test]
    fn a_theme_on_disk_is_searched_closest_size_first() {
        let root = std::env::temp_dir().join(format!("ski-icons-{}", std::process::id()));
        let theme = root.join("T");
        for dir in ["16x16/mimetypes", "scalable/places", "symbolic/places"] {
            std::fs::create_dir_all(theme.join(dir)).unwrap();
        }
        std::fs::write(theme.join("index.theme"), INDEX).unwrap();
        std::fs::write(theme.join("16x16/mimetypes/text-x-generic.png"), b"png").unwrap();
        std::fs::write(theme.join("scalable/places/text-x-generic.svg"), "<svg/>").unwrap();
        std::fs::write(theme.join("symbolic/places/folder.svg"), "<svg/>").unwrap();
        let parsed = parse_index(INDEX);
        let bases = [root.clone()];
        // A small icon comes from the fixed 16px folder, a large one from the scalable one
        assert_eq!(lookup_in(&bases, "T", &parsed, "text-x-generic", 16), Some(theme.join("16x16/mimetypes/text-x-generic.png")));
        assert_eq!(lookup_in(&bases, "T", &parsed, "text-x-generic", 96), Some(theme.join("scalable/places/text-x-generic.svg")));
        // Symbolic glyphs are never taken for a file icon
        assert_eq!(lookup_in(&bases, "T", &parsed, "folder", 48), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn files_get_mime_icon_names() {
        let item = FileItem::sample("photo.PNG", FileType::Image);
        let names = file_icon_names(&item);
        // Whatever the MIME database says, the generic picture is always among the choices
        assert!(names.iter().any(|n| n == "image-x-generic"), "{names:?}");
        if mime_of("a.png").is_some() {
            assert_eq!(mime_of("photo.PNG"), Some("image/png"));
            assert_eq!(names[0], "image-png");
        }
        if mime_of("a.tar.gz").is_some() {
            assert_eq!(mime_of("backup.tar.gz"), Some("application/x-compressed-tar"));
        }
    }

    #[test]
    fn sizes_are_bucketed() {
        assert_eq!(bucket(40.0), 48);
        assert_eq!(bucket(1000.0), 512);
    }
}

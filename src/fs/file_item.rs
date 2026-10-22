use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;
use crate::i18n::{tr, trf};
use crate::ui::icons;

static DECIMAL_UNITS: AtomicBool = AtomicBool::new(false);
static RELATIVE_DATES: AtomicBool = AtomicBool::new(false);
static SHOW_EXTENSIONS: AtomicBool = AtomicBool::new(true);

pub fn set_format_options(decimal_units: bool, relative_dates: bool) {
    DECIMAL_UNITS.store(decimal_units, Ordering::Relaxed);
    RELATIVE_DATES.store(relative_dates, Ordering::Relaxed);
}

pub fn set_show_extensions(show: bool) {
    SHOW_EXTENSIONS.store(show, Ordering::Relaxed);
}

pub fn decimal_units_enabled() -> bool {
    DECIMAL_UNITS.load(Ordering::Relaxed)
}

pub fn relative_dates_enabled() -> bool {
    RELATIVE_DATES.load(Ordering::Relaxed)
}

pub fn show_extensions_enabled() -> bool {
    SHOW_EXTENSIONS.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Directory,
    Document,
    Image,
    Audio,
    Video,
    Archive,
    Code,
    Executable,
    Symlink,
    Other,
}

/// Which theme color a file type icon is tinted with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    Red,
    Peach,
    Green,
    Blue,
    Mauve,
    Muted,
}

/// Word processor, spreadsheet and presentation files get a drawn page icon of their own.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Office {
    Document,
    Spreadsheet,
    Presentation,
}

pub fn office_kind(name: &str) -> Option<Office> {
    let lower = name.to_ascii_lowercase();
    Some(match lower.rsplit_once('.')?.1 {
        "doc" | "docx" | "docm" | "dot" | "dotx" | "odt" | "ott" | "rtf" => Office::Document,
        "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" | "ots" => Office::Spreadsheet,
        "ppt" | "pptx" | "pptm" | "pps" | "ppsx" | "potx" | "odp" | "otp" => Office::Presentation,
        _ => return None,
    })
}

/// A glyph and tint of its own for the file kinds people recognise at a glance.
/// Returns `None` for kinds that only have the generic per-category icon.
pub fn specific_icon(name: &str) -> Option<(&'static str, Tint)> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "makefile" | "cmakelists.txt" | "justfile" => return Some((icons::CONFIG, Tint::Muted)),
        "dockerfile" | "containerfile" => return Some((icons::DOCKER, Tint::Blue)),
        ".gitignore" | ".gitattributes" | ".gitmodules" => return Some((icons::GIT, Tint::Peach)),
        "license" | "licence" | "copying" | "readme" => return Some((icons::TEXT, Tint::Muted)),
        _ => {}
    }
    if lower.ends_with(".fb2.zip") {
        return Some((icons::EBOOK, Tint::Peach));
    }
    let ext = lower.rsplit_once('.')?.1;
    Some(match ext {
        "pdf" => (icons::PDF, Tint::Red),
        "html" | "htm" | "xhtml" => (icons::HTML, Tint::Peach),
        "iso" | "img" | "dmg" | "cue" | "nrg" => (icons::DISC, Tint::Blue),
        "doc" | "docx" | "docm" | "dot" | "dotx" | "odt" | "ott" | "rtf" | "pages" | "wpd" => (icons::WORD, Tint::Blue),
        "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" | "ots" | "numbers" => (icons::EXCEL, Tint::Green),
        "csv" | "tsv" => (icons::TABLE, Tint::Green),
        "ppt" | "pptx" | "pptm" | "pps" | "ppsx" | "potx" | "odp" | "otp" => (icons::POWERPOINT, Tint::Peach),
        "md" | "markdown" | "rst" | "adoc" => (icons::MARKDOWN, Tint::Muted),
        "txt" | "tex" | "nfo" | "org" => (icons::TEXT, Tint::Muted),
        "log" => (icons::LOG, Tint::Muted),
        "epub" | "mobi" | "azw" | "azw3" | "azw4" | "kfx" | "prc" | "fb2" | "djvu" | "djv" | "lit" | "lrf" | "chm" | "ibooks"
        | "cbz" | "cbr" | "cb7" | "cbt" => (icons::EBOOK, Tint::Peach),
        "py" | "pyc" | "ipynb" => (icons::PYTHON, Tint::Blue),
        "rs" => (icons::RUST, Tint::Peach),
        "js" | "mjs" | "cjs" | "jsx" => (icons::JAVASCRIPT, Tint::Peach),
        "ts" | "tsx" => (icons::TYPESCRIPT, Tint::Blue),
        "css" | "scss" | "sass" | "less" => (icons::CSS, Tint::Blue),
        "json" | "jsonc" | "json5" => (icons::JSON, Tint::Peach),
        "c" | "h" => (icons::LANG_C, Tint::Blue),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => (icons::LANG_CPP, Tint::Blue),
        "go" => (icons::GO, Tint::Blue),
        "java" | "jar" => (icons::JAVA, Tint::Red),
        "kt" | "kts" => (icons::KOTLIN, Tint::Mauve),
        "swift" => (icons::SWIFT, Tint::Peach),
        "lua" => (icons::LUA, Tint::Blue),
        "php" => (icons::PHP, Tint::Mauve),
        "rb" => (icons::RUBY, Tint::Red),
        "sh" | "bash" | "zsh" | "fish" => (icons::SHELL, Tint::Green),
        "nix" => (icons::NIX, Tint::Blue),
        "toml" | "yaml" | "yml" | "ini" | "conf" | "cfg" | "env" | "desktop" | "service" => (icons::CONFIG, Tint::Muted),
        "sql" | "db" | "sqlite" | "sqlite3" => (icons::DATABASE, Tint::Mauve),
        "svg" | "svgz" | "ai" | "eps" => (icons::VECTOR, Tint::Green),
        "ttf" | "otf" | "woff" | "woff2" => (icons::FONT, Tint::Muted),
        "pem" | "key" | "gpg" | "pub" | "asc" => (icons::KEY, Tint::Peach),
        "crt" | "cer" | "p12" => (icons::CERTIFICATE, Tint::Peach),
        "deb" | "rpm" | "apk" | "pkg" | "flatpakref" | "snap" => (icons::PACKAGE, Tint::Peach),
        "appimage" | "exe" | "msi" => (icons::APPLICATION, Tint::Green),
        "torrent" => (icons::TORRENT, Tint::Green),
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" | "tgz" | "tbz2" | "txz" | "lz" | "lzma" | "lz4" | "cab" | "cpio" | "zipx" => (icons::ZIP, Tint::Peach),
        "dxf" | "dwg" | "stl" | "obj" | "step" | "stp" | "3mf" | "blend" | "fcstd" => (icons::CAD, Tint::Mauve),
        _ => return None,
    })
}

impl FileType {
    pub fn icon(&self) -> &'static str {
        match self {
            FileType::Directory => icons::FOLDER,
            FileType::Document => icons::DOCUMENT,
            FileType::Image => icons::IMAGE,
            FileType::Audio => icons::AUDIO,
            FileType::Video => icons::VIDEO,
            FileType::Archive => icons::ARCHIVE,
            FileType::Code => icons::CODE,
            FileType::Executable => icons::EXECUTABLE,
            FileType::Symlink => icons::SYMLINK,
            FileType::Other => icons::FILE,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            FileType::Directory => tr("Folder"),
            FileType::Document => tr("Document"),
            FileType::Image => tr("Image"),
            FileType::Audio => tr("Audio"),
            FileType::Video => tr("Video"),
            FileType::Archive => tr("Archive"),
            FileType::Code => tr("Source Code"),
            FileType::Executable => tr("Executable"),
            FileType::Symlink => tr("Symlink"),
            FileType::Other => tr("File"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileItem {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub uid: u32,
    pub mode: u32,
    pub file_type: FileType,
    pub is_hidden: bool,
    /// The current user cannot write to it, so it gets a lock badge.
    pub is_locked: bool,
}

impl FileItem {
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        let is_hidden = name.starts_with('.');

        let symlink_meta = std::fs::symlink_metadata(path).ok();
        let is_symlink = symlink_meta.as_ref().map(|m| m.file_type().is_symlink()).unwrap_or(false);

        // Follow symlink for actual metadata if possible, otherwise use symlink metadata
        let metadata = std::fs::metadata(path).ok().or(symlink_meta);

        let (is_dir, size, modified, is_executable, created, accessed, uid, mode) = if let Some(ref meta) = metadata {
            let is_dir = meta.is_dir();
            let size = if is_dir { 0 } else { meta.len() };
            let modified = meta.modified().ok();
            let created = meta.created().ok();
            let accessed = meta.accessed().ok();

            #[cfg(unix)]
            let (uid, mode) = {
                use std::os::unix::fs::MetadataExt;
                (meta.uid(), meta.mode())
            };
            #[cfg(not(unix))]
            let (uid, mode) = (0, 0);

            #[cfg(unix)]
            let is_executable = {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0 && !is_dir
            };
            #[cfg(not(unix))]
            let is_executable = false;

            (is_dir, size, modified, is_executable, created, accessed, uid, mode)
        } else {
            (false, 0, None, false, None, None, 0, 0)
        };

        let file_type = if is_dir {
            FileType::Directory
        } else if is_symlink && metadata.is_none() {
            FileType::Symlink
        } else {
            // The executable bit alone proves nothing for known formats: files from FAT, NTFS or
            // network shares come out all-executable, and a book is still a book
            match detect_file_type(&name) {
                FileType::Other if is_executable => FileType::Executable,
                known => known,
            }
        };

        Some(Self {
            name,
            path: path.to_path_buf(),
            is_dir,
            is_symlink,
            size,
            modified,
            created,
            accessed,
            uid,
            mode,
            file_type,
            is_hidden,
            is_locked: metadata.is_some() && !writable(path),
        })
    }

    /// Shown as its target's kind with a link badge; a dangling link keeps the plain link icon.
    pub fn is_live_symlink(&self) -> bool {
        self.is_symlink && self.file_type != FileType::Symlink
    }

    pub fn display_name(&self) -> &str {
        if self.is_dir || show_extensions_enabled() {
            &self.name
        } else {
            match self.path.file_stem().and_then(|s| s.to_str()) {
                Some(stem) if !stem.is_empty() && stem != self.name => stem,
                _ => &self.name,
            }
        }
    }

    pub fn formatted_size(&self) -> String {
        if self.is_dir {
            "—".to_string()
        } else {
            format_bytes(self.size)
        }
    }

    pub fn owner(&self) -> String {
        super::props::user_name(self.uid)
    }

    pub fn formatted_perms(&self) -> String {
        super::props::mode_string(self.mode, self.is_dir)
    }

    pub fn formatted_date(&self) -> String {
        if relative_dates_enabled() {
            match self.modified {
                Some(time) => {
                    let duration = match SystemTime::now().duration_since(time) {
                        Ok(d) => d,
                        Err(_) => return tr("In the future").to_string(),
                    };
                    let secs = duration.as_secs();
                    if secs < 60 {
                        tr("Just now").to_string()
                    } else if secs < 3600 {
                        trf("{}m ago", &[&(secs / 60)])
                    } else if secs < 86400 {
                        trf("{}h ago", &[&(secs / 3600)])
                    } else if secs < 86400 * 7 {
                        trf("{}d ago", &[&(secs / 86400)])
                    } else {
                        trf("{}w ago", &[&(secs / (86400 * 7))])
                    }
                }
                None => tr("Unknown").to_string(),
            }
        } else {
            super::props::format_datetime(self.modified)
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    format_bytes_ext(bytes, decimal_units_enabled())
}

pub fn format_bytes_ext(bytes: u64, decimal: bool) -> String {
    let base = if decimal { 1000.0 } else { 1024.0 };
    let kb = base;
    let mb = kb * base;
    let gb = mb * base;
    let tb = gb * base;
    let b = bytes as f64;

    if b < kb {
        trf("{} B", &[&bytes])
    } else if b < mb {
        trf("{} KB", &[&format!("{:.1}", b / kb)])
    } else if b < gb {
        trf("{} MB", &[&format!("{:.1}", b / mb)])
    } else if b < tb {
        trf("{} GB", &[&format!("{:.2}", b / gb)])
    } else {
        trf("{} TB", &[&format!("{:.2}", b / tb)])
    }
}

fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    match std::ffi::CString::new(path.as_os_str().as_bytes()) {
        // SAFETY: a valid NUL-terminated path; access only reads it
        Ok(c) => unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 },
        Err(_) => true,
    }
}

fn detect_file_type(filename: &str) -> FileType {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".fb2.zip") {
        return FileType::Document;
    }
    let ext = match lower.rsplit_once('.') {
        Some((_, ext)) => ext,
        None => return FileType::Other,
    };

    match ext {
        "txt" | "md" | "pdf" | "doc" | "docx" | "docm" | "dot" | "dotx" | "odt" | "ott" | "rtf" | "tex" | "pages" | "wpd"
        | "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" | "ots" | "csv" | "tsv" | "numbers"
        | "ppt" | "pptx" | "pptm" | "pps" | "ppsx" | "potx" | "odp" | "otp" | "odg"
        | "epub" | "mobi" | "azw" | "azw3" | "azw4" | "kfx" | "prc" | "fb2" | "djvu" | "djv" | "lit" | "lrf" | "chm" | "ibooks"
        | "cbz" | "cbr" | "cb7" | "cbt" => FileType::Document,
        "png" | "apng" | "jpg" | "jpeg" | "jpe" | "jfif" | "gif" | "svg" | "svgz" | "webp" | "bmp" | "ico" | "tif" | "tiff" | "heic" | "heif"
        | "avif" | "jxl" | "psd" | "xcf" | "kra" | "ora" | "exr" | "hdr" | "tga" | "dds" | "qoi" | "ppm" | "pgm" | "pbm" | "pnm" | "jp2" | "j2k" | "pcx"
        | "raw" | "cr2" | "cr3" | "crw" | "nef" | "nrw" | "arw" | "sr2" | "srf" | "dng" | "orf" | "rw2" | "raf" | "pef" | "srw" | "3fr" | "erf" | "kdc" | "dcr" | "mrw" | "x3f" => FileType::Image,
        "mp3" | "flac" | "wav" | "ogg" | "oga" | "m4a" | "m4b" | "aac" | "opus" | "wma" | "aif" | "aiff" | "ape" | "wv" | "mid" | "midi" | "mka" | "mpc" | "dsf" | "tta" | "amr" | "ac3" | "dts" => FileType::Audio,
        "mp4" | "m4v" | "mkv" | "webm" | "avi" | "mov" | "flv" | "wmv" | "mpg" | "mpeg" | "ogv" | "3gp" | "3g2" | "m2ts" | "m2t" | "mts" | "vob" | "asf" | "rm" | "rmvb" | "divx" | "f4v" | "mxf" | "ogm" | "mpe" | "qt" | "dv" => FileType::Video,
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" | "tgz" | "tbz2" | "txz" | "lz" | "lzma" | "lz4" | "cab" | "cpio" | "zipx" => FileType::Archive,
        "rs" | "c" | "cpp" | "h" | "hpp" | "py" | "js" | "ts" | "jsx" | "tsx" | "html"
        | "css" | "scss" | "json" | "toml" | "yaml" | "yml" | "nix" | "sh" | "bash"
        | "lua" | "go" | "java" | "kt" | "swift" | "sql" => FileType::Code,
        _ => FileType::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes_ext() {
        assert_eq!(format_bytes_ext(500, false), "500 B");
        assert_eq!(format_bytes_ext(500, true), "500 B");
        assert_eq!(format_bytes_ext(1024, false), "1.0 KB");
        assert_eq!(format_bytes_ext(1000, true), "1.0 KB");
    }

    #[test]
    fn test_display_name_extension_toggle() {
        let item = FileItem {
            name: "document.pdf".to_string(),
            path: std::path::PathBuf::from("/tmp/document.pdf"),
            size: 1024,
            modified: None,
            created: None,
            accessed: None,
            uid: 1000,
            mode: 0o644,
            is_dir: false,
            file_type: FileType::Document,
            is_hidden: false,
            is_symlink: false,
            is_locked: false,
        };
        set_show_extensions(true);
        assert_eq!(item.display_name(), "document.pdf");
        set_show_extensions(false);
        assert_eq!(item.display_name(), "document");

        // Hidden file starting with dot
        let dotfile = FileItem {
            name: ".gitignore".to_string(),
            path: std::path::PathBuf::from("/tmp/.gitignore"),
            size: 10,
            modified: None,
            created: None,
            accessed: None,
            uid: 1000,
            mode: 0o644,
            is_dir: false,
            file_type: FileType::Code,
            is_hidden: true,
            is_symlink: false,
            is_locked: false,
        };
        assert_eq!(dotfile.display_name(), ".gitignore");

        // Reset to default
        set_show_extensions(true);
    }

    #[test]
    fn executable_bit_does_not_hide_a_known_format() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join("ski-item-kind-test");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, kind) in [("book.fb2", FileType::Document), ("comic.cbz", FileType::Document), ("run-me", FileType::Executable)] {
            let file = dir.join(name);
            std::fs::write(&file, b"x").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(FileItem::from_path(&file).unwrap().file_type, kind, "{name}");
        }
        assert!(specific_icon("book.fb2").is_some() && specific_icon("comic.cbz").is_some() && specific_icon("x.fb2.zip").is_some());
    }

    #[test]
    fn read_only_files_are_locked() {
        use std::os::unix::fs::PermissionsExt;
        let file = std::env::temp_dir().join("ski-item-lock-test");
        std::fs::write(&file, b"x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
        // Root can write anywhere, so there is nothing to lock for it
        let root = unsafe { libc::geteuid() } == 0;
        assert_eq!(FileItem::from_path(&file).unwrap().is_locked, !root);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!FileItem::from_path(&file).unwrap().is_locked);
    }
}

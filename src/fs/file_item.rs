use std::path::{Path, PathBuf};
use std::time::SystemTime;
use crate::ui::icons;

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
    let ext = lower.rsplit_once('.')?.1;
    Some(match ext {
        "pdf" => (icons::PDF, Tint::Red),
        "html" | "htm" | "xhtml" => (icons::HTML, Tint::Peach),
        "iso" | "img" | "dmg" | "cue" | "nrg" => (icons::DISC, Tint::Blue),
        "doc" | "docx" | "odt" | "rtf" | "dotx" => (icons::WORD, Tint::Blue),
        "xls" | "xlsx" | "ods" | "xlsm" => (icons::EXCEL, Tint::Green),
        "csv" | "tsv" => (icons::TABLE, Tint::Green),
        "ppt" | "pptx" | "odp" => (icons::POWERPOINT, Tint::Peach),
        "md" | "markdown" | "rst" => (icons::MARKDOWN, Tint::Muted),
        "txt" => (icons::TEXT, Tint::Muted),
        "log" => (icons::LOG, Tint::Muted),
        "epub" | "mobi" | "azw3" | "djvu" => (icons::EBOOK, Tint::Peach),
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
        "svg" => (icons::VECTOR, Tint::Green),
        "ttf" | "otf" | "woff" | "woff2" => (icons::FONT, Tint::Muted),
        "pem" | "key" | "gpg" | "pub" | "asc" => (icons::KEY, Tint::Peach),
        "crt" | "cer" | "p12" => (icons::CERTIFICATE, Tint::Peach),
        "deb" | "rpm" | "apk" | "pkg" | "flatpakref" | "snap" => (icons::PACKAGE, Tint::Peach),
        "appimage" | "exe" | "msi" => (icons::APPLICATION, Tint::Green),
        "torrent" => (icons::TORRENT, Tint::Green),
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" | "tgz" => (icons::ZIP, Tint::Peach),
        "dxf" | "dwg" | "stl" | "obj" | "step" | "stp" => (icons::CAD, Tint::Mauve),
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
            FileType::Directory => "Folder",
            FileType::Document => "Document",
            FileType::Image => "Image",
            FileType::Audio => "Audio",
            FileType::Video => "Video",
            FileType::Archive => "Archive",
            FileType::Code => "Source Code",
            FileType::Executable => "Executable",
            FileType::Symlink => "Symlink",
            FileType::Other => "File",
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
        } else if is_executable {
            FileType::Executable
        } else {
            detect_file_type(&name)
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
        })
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
        match self.modified {
            Some(time) => {
                let duration = match SystemTime::now().duration_since(time) {
                    Ok(d) => d,
                    Err(_) => return "In the future".to_string(),
                };
                let secs = duration.as_secs();
                if secs < 60 {
                    "Just now".to_string()
                } else if secs < 3600 {
                    format!("{}m ago", secs / 60)
                } else if secs < 86400 {
                    format!("{}h ago", secs / 3600)
                } else if secs < 86400 * 7 {
                    format!("{}d ago", secs / 86400)
                } else {
                    format!("{}w ago", secs / (86400 * 7))
                }
            }
            None => "Unknown".to_string(),
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes < TB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    }
}

fn detect_file_type(filename: &str) -> FileType {
    let ext = filename
        .rsplit('.')
        .next()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "txt" | "md" | "pdf" | "doc" | "docx" | "odt" | "rtf" | "epub" => FileType::Document,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "tiff" => FileType::Image,
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" => FileType::Audio,
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "flv" | "wmv" => FileType::Video,
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" => FileType::Archive,
        "rs" | "c" | "cpp" | "h" | "hpp" | "py" | "js" | "ts" | "jsx" | "tsx" | "html"
        | "css" | "scss" | "json" | "toml" | "yaml" | "yml" | "nix" | "sh" | "bash"
        | "lua" | "go" | "java" | "kt" | "swift" | "sql" => FileType::Code,
        _ => FileType::Other,
    }
}

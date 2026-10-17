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

        let (is_dir, size, modified, is_executable) = if let Some(ref meta) = metadata {
            let is_dir = meta.is_dir();
            let size = if is_dir { 0 } else { meta.len() };
            let modified = meta.modified().ok();

            #[cfg(unix)]
            let is_executable = {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0 && !is_dir
            };
            #[cfg(not(unix))]
            let is_executable = false;

            (is_dir, size, modified, is_executable)
        } else {
            (false, 0, None, false)
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

//! Chooser mode: `ski --chooser …` runs the file manager as an open / save dialog. The chosen
//! paths go to stdout as `file://` URIs, one per line; the exit code is 0 for a choice and 1 for
//! a cancel. The portal backend (`portal.rs`) starts it for every FileChooser request.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Pick existing files.
    Open,
    /// Pick existing folders.
    Folder,
    /// Name one new file.
    Save,
    /// Pick the folder that several named files go into.
    SaveFiles,
}

#[derive(Clone, Debug)]
pub enum Pattern {
    Glob(String),
    Mime(String),
}

#[derive(Clone, Debug)]
pub struct Filter {
    pub label: String,
    pub patterns: Vec<Pattern>,
}

impl Filter {
    pub fn accepts(&self, path: &Path) -> bool {
        if self.patterns.is_empty() {
            return true;
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        self.patterns.iter().any(|p| match p {
            Pattern::Glob(g) => crate::fs::glob::matches(g, &name, false),
            Pattern::Mime(m) => mime_has_extension(m, &ext),
        })
    }
}

/// Extensions behind the MIME types portal clients most often ask for. A type listed here
/// matches by extension; a type not listed matches everything rather than hiding files.
fn mime_has_extension(mime: &str, ext: &str) -> bool {
    const IMAGE: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "tif", "tiff", "avif", "heic", "ico"];
    const AUDIO: &[&str] = &["mp3", "flac", "ogg", "oga", "opus", "wav", "m4a", "aac", "wma"];
    const VIDEO: &[&str] = &["mp4", "mkv", "webm", "avi", "mov", "m4v", "ogv", "wmv", "flv"];
    const TEXT: &[&str] = &["txt", "md", "log", "csv", "json", "xml", "html", "htm", "toml", "yaml", "yml", "ini", "rs", "py", "c", "h", "js", "css", "sh"];
    const ARCHIVE: &[&str] = &["zip", "tar", "gz", "xz", "bz2", "zst", "7z", "rar"];
    let list: &[&str] = match mime {
        "image/*" => IMAGE,
        "audio/*" => AUDIO,
        "video/*" => VIDEO,
        "text/*" => TEXT,
        "image/png" => &["png"],
        "image/jpeg" => &["jpg", "jpeg"],
        "image/gif" => &["gif"],
        "image/webp" => &["webp"],
        "image/svg+xml" => &["svg"],
        "application/pdf" => &["pdf"],
        "application/zip" => &["zip"],
        "application/json" => &["json"],
        "text/plain" => &["txt"],
        "text/html" => &["html", "htm"],
        "audio/mpeg" => &["mp3"],
        "video/mp4" => &["mp4"],
        "application/x-tar" | "application/gzip" | "application/x-compressed-tar" => ARCHIVE,
        _ => return true,
    };
    list.contains(&ext)
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub mode: Mode,
    pub multiple: bool,
    pub title: Option<String>,
    pub accept_label: Option<String>,
    pub folder: Option<PathBuf>,
    pub name: Option<String>,
    /// Names for `SaveFiles`, which the chosen folder gets one URI each.
    pub files: Vec<String>,
    pub filters: Vec<Filter>,
    pub current_filter: usize,
    /// The requester's window as the portal names it (`wayland:<handle>`).
    pub parent: Option<String>,
}

impl Spec {
    /// Reads the arguments after `--chooser`; `None` when they are not a chooser request.
    pub fn parse(args: &[String]) -> Option<Self> {
        let mut spec = Spec {
            mode: Mode::Open,
            multiple: false,
            title: None,
            accept_label: None,
            folder: None,
            name: None,
            files: Vec::new(),
            filters: Vec::new(),
            current_filter: 0,
            parent: None,
        };
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--chooser" => {}
                "--multiple" => spec.multiple = true,
                "--mode" => {
                    spec.mode = match it.next()?.as_str() {
                        "open" => Mode::Open,
                        "folder" => Mode::Folder,
                        "save" => Mode::Save,
                        "save-files" => Mode::SaveFiles,
                        _ => return None,
                    }
                }
                "--title" => spec.title = it.next().cloned().filter(|s| !s.is_empty()),
                "--accept" => spec.accept_label = it.next().cloned().filter(|s| !s.is_empty()),
                "--folder" => spec.folder = it.next().map(PathBuf::from),
                "--name" => spec.name = it.next().cloned().filter(|s| !s.is_empty()),
                "--file" => spec.files.extend(it.next().cloned()),
                "--parent" => spec.parent = it.next().cloned().filter(|s| !s.is_empty()),
                "--current-filter" => spec.current_filter = it.next()?.parse().unwrap_or(0),
                // label, then patterns written `g:*.png` (glob) or `m:image/png` (MIME type)
                "--filter" => {
                    let raw = it.next()?;
                    let mut parts = raw.split('\t');
                    let label = parts.next().unwrap_or_default().to_owned();
                    let patterns = parts
                        .filter_map(|p| match p.split_at_checked(2)? {
                            ("g:", g) => Some(Pattern::Glob(g.to_owned())),
                            ("m:", m) => Some(Pattern::Mime(m.to_owned())),
                            _ => None,
                        })
                        .collect();
                    spec.filters.push(Filter { label, patterns });
                }
                _ => {}
            }
        }
        spec.current_filter = spec.current_filter.min(spec.filters.len().saturating_sub(1));
        Some(spec)
    }
}

pub struct Chooser {
    pub spec: Spec,
    pub name_input: String,
    pub filter: usize,
    /// The name an overwrite warning was shown for; pressing Save again confirms it.
    pub warned: Option<PathBuf>,
    /// Keeps the window tied to its parent while the dialog is open.
    pub parent: Option<crate::wayland_parent::Parent>,
}

impl Chooser {
    pub fn new(spec: Spec) -> Self {
        let name_input = spec.name.clone().unwrap_or_default();
        let filter = spec.current_filter;
        Self { spec, name_input, filter, warned: None, parent: None }
    }

    pub fn accepts(&self, path: &Path) -> bool {
        self.spec.filters.get(self.filter).is_none_or(|f| f.accepts(path))
    }

    pub fn takes_name(&self) -> bool {
        self.spec.mode == Mode::Save
    }
}

/// `file://` URI with the characters a URI cannot hold escaped.
pub fn uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Path behind a `file://` URI (or a plain path); `None` for other schemes.
pub fn from_uri(text: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let Some(rest) = text.strip_prefix("file://") else {
        return (!text.contains("://")).then(|| PathBuf::from(text));
    };
    // `file://host/path`: only the local host is ours
    let rest = rest.find('/').map_or("", |i| &rest[i..]);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = rest.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// Prints the choice and ends the process; the portal backend reads it from stdout.
pub fn finish(paths: &[PathBuf]) -> ! {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    for p in paths {
        let _ = writeln!(out, "{}", uri(p));
    }
    let _ = out.flush();
    std::process::exit(0);
}

pub fn cancel() -> ! {
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_round_trips() {
        let p = PathBuf::from("/home/a b/ü#1.txt");
        assert_eq!(from_uri(&uri(&p)), Some(p));
        assert_eq!(uri(Path::new("/a b")), "file:///a%20b");
    }

    #[test]
    fn filters_match_globs_and_mimes() {
        let f = Filter { label: "Images".into(), patterns: vec![Pattern::Mime("image/*".into()), Pattern::Glob("*.xyz".into())] };
        assert!(f.accepts(Path::new("/x/a.PNG")));
        assert!(f.accepts(Path::new("/x/a.xyz")));
        assert!(!f.accepts(Path::new("/x/a.txt")));
    }

    #[test]
    fn parses_arguments() {
        let args: Vec<String> = ["--chooser", "--mode", "save", "--name", "a.txt", "--filter", "Text\tg:*.txt\tm:text/plain"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let spec = Spec::parse(&args).unwrap();
        assert_eq!(spec.mode, Mode::Save);
        assert_eq!(spec.name.as_deref(), Some("a.txt"));
        assert_eq!(spec.filters[0].patterns.len(), 2);
    }
}

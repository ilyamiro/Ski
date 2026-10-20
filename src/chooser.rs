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
        self.patterns.iter().any(|p| match p {
            Pattern::Glob(g) => crate::fs::glob::matches(g, &name, false),
            Pattern::Mime(m) => crate::fs::apps::name_is_mime(&name, m),
        })
    }
}

/// An extra setting the requesting program asked for: a checkbox (no options) or a drop-down.
#[derive(Clone, Debug)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub options: Vec<(String, String)>,
    pub initial: String,
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
    /// The requesting program, to start where it last chose something.
    pub app_id: Option<String>,
    pub choices: Vec<Choice>,
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
            app_id: None,
            choices: Vec::new(),
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
                // Only the last component counts: a name is never a way out of the chosen folder
                "--file" => spec.files.extend(it.next().and_then(|f| Path::new(f).file_name()).map(|n| n.to_string_lossy().into_owned())),
                "--parent" => spec.parent = it.next().cloned().filter(|s| !s.is_empty()),
                "--app-id" => spec.app_id = it.next().cloned().filter(|s| !s.is_empty()),
                // id, label, initial value, then option id and label pairs
                "--choice" => {
                    let raw = it.next()?;
                    let mut parts = raw.split('\t').map(str::to_owned);
                    let (id, label, initial) = (parts.next()?, parts.next()?, parts.next()?);
                    let rest: Vec<String> = parts.collect();
                    let options = rest.chunks(2).filter(|pair| pair.len() == 2).map(|pair| (pair[0].clone(), pair[1].clone())).collect();
                    spec.choices.push(Choice { id, label, options, initial });
                }
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
    /// The current value of each of the spec's choices.
    pub choice_values: Vec<String>,
    /// The file whose name was last copied into the name field, so that is done once per click.
    pub filled: Option<PathBuf>,
    /// Set until the name field has taken focus and selected the part of the name before the dot.
    pub focus_name: bool,
    /// Keeps the window tied to its parent while the dialog is open.
    pub parent: Option<crate::wayland_parent::Parent>,
    /// The window size last seen and since when, so a resize is stored once it settles.
    size_seen: Option<([f32; 2], std::time::Instant)>,
    size_stored: Option<[f32; 2]>,
}

impl Chooser {
    pub fn new(spec: Spec) -> Self {
        let name_input = spec.name.clone().unwrap_or_default();
        let filter = spec.current_filter;
        let choice_values = spec.choices.iter().map(|c| c.initial.clone()).collect();
        let focus_name = spec.mode == Mode::Save;
        Self { spec, name_input, filter, choice_values, filled: None, focus_name, parent: None, size_seen: None, size_stored: None }
    }

    pub fn accepts(&self, path: &Path) -> bool {
        self.spec.filters.get(self.filter).is_none_or(|f| f.accepts(path))
    }

    /// Remembers the size the user gave the dialog, once a resize has settled. Kept in a file of its
    /// own: the settings file is whole-file rewritten by whichever Ski window saves last.
    pub fn remember_size(&mut self, ctx: &eframe::egui::Context) {
        let Some(rect) = ctx.input(|i| i.viewport().inner_rect) else { return };
        let zoom = ctx.zoom_factor();
        let size = [(rect.width() * zoom).round(), (rect.height() * zoom).round()];
        match self.size_seen {
            Some((seen, _)) if seen == size => {}
            _ => self.size_seen = Some((size, std::time::Instant::now())),
        }
        let Some((seen, since)) = self.size_seen else { return };
        if self.size_stored.is_none() {
            // The first frame is the size the dialog opened with
            self.size_stored = Some(seen);
        } else if self.size_stored != Some(seen) {
            if since.elapsed() >= SIZE_SETTLE {
                save_size(seen);
                self.size_stored = Some(seen);
            } else {
                ctx.request_repaint_after(SIZE_SETTLE);
            }
        }
    }

    pub fn takes_name(&self) -> bool {
        self.spec.mode == Mode::Save
    }
}

/// The size a dialog opens at when none was remembered, and the smallest it is restored to.
pub const DEFAULT_SIZE: [f32; 2] = [760.0, 520.0];
const SIZE_SETTLE: std::time::Duration = std::time::Duration::from_millis(400);

fn size_file() -> Option<PathBuf> {
    dirs::state_dir().or_else(dirs::config_dir).map(|d| d.join("ski").join("chooser-size"))
}

/// The size the dialog was last left at.
pub fn saved_size() -> Option<[f32; 2]> {
    let text = std::fs::read_to_string(size_file()?).ok()?;
    let (w, h) = text.trim().split_once('x')?;
    let (w, h) = (w.parse::<f32>().ok()?, h.parse::<f32>().ok()?);
    (w >= 360.0 && h >= 420.0 && w <= 8192.0 && h <= 8192.0).then_some([w, h])
}

fn save_size([w, h]: [f32; 2]) {
    let Some(file) = size_file() else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, format!("{w}x{h}"));
}

fn folders_file() -> Option<PathBuf> {
    dirs::state_dir().or_else(dirs::config_dir).map(|d| d.join("ski").join("chooser-folders"))
}

/// The folder a program last chose something in (`app_id` is empty for programs the portal cannot name).
pub fn remembered_folder(app_id: Option<&str>) -> Option<PathBuf> {
    let text = std::fs::read_to_string(folders_file()?).ok()?;
    let key = app_id.unwrap_or("");
    text.lines().find_map(|l| l.split_once('\t').filter(|(id, _)| *id == key).map(|(_, p)| PathBuf::from(p))).filter(|p| p.is_dir())
}

fn remember_folder(app_id: Option<&str>, folder: &Path) {
    let Some(file) = folders_file() else { return };
    let key = app_id.unwrap_or("");
    let mut lines: Vec<String> = std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.split_once('\t').is_some_and(|(id, _)| id != key))
        .map(str::to_owned)
        .collect();
    lines.push(format!("{key}\t{}", folder.display()));
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, lines.join("\n") + "\n");
}

impl Chooser {
    /// Answers with `paths`, the filter and the choices as they stand, remembering where it was done.
    pub fn answer(&self, paths: &[PathBuf], folder: &Path) -> ! {
        remember_folder(self.spec.app_id.as_deref(), folder);
        let mut extra = Vec::new();
        if !self.spec.filters.is_empty() {
            extra.push(format!("filter\t{}", self.filter));
        }
        for (choice, value) in self.spec.choices.iter().zip(&self.choice_values) {
            extra.push(format!("choice\t{}\t{value}", choice.id));
        }
        finish(paths, &extra)
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

/// Prints the choice and ends the process; the portal backend reads it from stdout. Lines that
/// are not URIs carry the rest of the answer: `filter<TAB>index` and `choice<TAB>id<TAB>value`.
pub fn finish(paths: &[PathBuf], extra: &[String]) -> ! {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    for p in paths {
        let _ = writeln!(out, "{}", uri(p));
    }
    for line in extra {
        let _ = writeln!(out, "{line}");
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
    fn mime_filters_follow_the_shared_database() {
        let mime = |m: &str| Filter { label: String::new(), patterns: vec![Pattern::Mime(m.into())] };
        // Skipped where no shared-mime-info is installed, which then shows everything by design
        if crate::fs::apps::mime_of_name("a.png").is_none() {
            return;
        }
        assert!(mime("application/zip").accepts(Path::new("/x/report.docx")));
        assert!(mime("text/*").accepts(Path::new("/x/main.rs")));
        assert!(!mime("image/*").accepts(Path::new("/x/a.tar.gz")));
        assert!(!mime("application/x-foo").accepts(Path::new("/x/a.png")));
    }

    #[test]
    fn file_names_cannot_leave_the_folder() {
        let args: Vec<String> = ["--chooser", "--mode", "save-files", "--file", "../../etc/passwd", "--file", "/abs/x.txt", "--file", "..", "--file", "ok"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(Spec::parse(&args).unwrap().files, ["passwd", "x.txt", "ok"]);
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

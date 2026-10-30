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
    Yellow,
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
        "makefile" | "cmakelists.txt" | "justfile" | "gemfile" | "procfile" | "cargo.lock" => return Some((icons::CONFIG, Tint::Muted)),
        "dockerfile" | "containerfile" | ".dockerignore" | "compose.yml" | "compose.yaml" => return Some((icons::DOCKER, Tint::Blue)),
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitconfig" => return Some((icons::GIT, Tint::Peach)),
        "readme" => return Some((icons::TEXT, Tint::Muted)),
        "package.json" | "package-lock.json" => return Some((icons::NPM, Tint::Red)),
        "cargo.toml" => return Some((icons::RUST, Tint::Peach)),
        "go.mod" => return Some((icons::GO, Tint::Blue)),
        "pyproject.toml" => return Some((icons::PYTHON, Tint::Blue)),
        "flake.lock" => return Some((icons::NIX, Tint::Blue)),
        ".editorconfig" => return Some((icons::TOOLING, Tint::Muted)),
        ".gitlab-ci.yml" | ".gitlab-ci.yaml" => return Some((icons::CI, Tint::Blue)),
        _ => {}
    }
    if lower.ends_with(".fb2.zip") {
        return Some((icons::EBOOK, Tint::Peach));
    }
    // The license and changelog, bare or as text (`LICENSE.md`, but not `license.rs`)
    let (stem, text) = match lower.split_once('.') {
        Some((stem, ext)) => (stem, matches!(ext, "md" | "txt" | "rst" | "adoc" | "org")),
        None => (lower.as_str(), true),
    };
    if text && matches!(stem, "license" | "licence" | "copying" | "unlicense") {
        return Some((icons::LICENSE, Tint::Yellow));
    }
    if text && matches!(stem, "changelog" | "changes") {
        return Some((icons::CHANGELOG, Tint::Green));
    }
    // Kinds known by how the name starts: every `.env.local` and `.env.production` along with
    // `.env`, and the tools' own dotfiles
    if lower == ".env" || lower.starts_with(".env.") {
        return Some((icons::SECRETS, Tint::Yellow));
    }
    if [".prettier", ".eslint", "prettier.config.", "eslint.config."].iter().any(|p| lower.starts_with(p)) {
        return Some((icons::TOOLING, Tint::Muted));
    }
    if lower.starts_with("docker-compose") && (lower.ends_with(".yml") || lower.ends_with(".yaml")) {
        return Some((icons::DOCKER, Tint::Blue));
    }
    // Backups an editor leaves (`notes.txt~`), and shared libraries with their version after the
    // extension (`libssl.so.3`)
    if lower.ends_with('~') {
        return Some((icons::BACKUP, Tint::Muted));
    }
    if lower.contains(".so.") {
        return Some((icons::LIBRARY, Tint::Muted));
    }
    let ext = lower.rsplit_once('.')?.1;
    Some(match ext {
        "pdf" => (icons::PDF, Tint::Red),
        "html" | "htm" | "xhtml" => (icons::HTML, Tint::Peach),
        "iso" | "img" | "dmg" | "nrg" => (icons::DISC, Tint::Blue),
        "doc" | "docx" | "docm" | "dot" | "dotx" | "odt" | "ott" | "rtf" | "pages" | "wpd" => (icons::WORD, Tint::Blue),
        "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" | "ots" | "numbers" => (icons::EXCEL, Tint::Green),
        "csv" | "tsv" => (icons::TABLE, Tint::Green),
        "ppt" | "pptx" | "pptm" | "pps" | "ppsx" | "potx" | "odp" | "otp" => (icons::POWERPOINT, Tint::Peach),
        "md" | "markdown" | "rst" | "adoc" => (icons::MARKDOWN, Tint::Blue),
        "txt" | "tex" | "nfo" | "org" => (icons::TEXT, Tint::Muted),
        "log" => (icons::LOG, Tint::Muted),
        "epub" | "mobi" | "azw" | "azw3" | "azw4" | "kfx" | "prc" | "fb2" | "djvu" | "djv" | "lit" | "lrf" | "chm" | "ibooks"
        | "cbz" | "cbr" | "cb7" | "cbt" => (icons::EBOOK, Tint::Peach),
        "py" | "pyc" | "pyw" => (icons::PYTHON, Tint::Blue),
        "ipynb" => (icons::JUPYTER, Tint::Peach),
        "rs" => (icons::RUST, Tint::Peach),
        "js" | "mjs" | "cjs" | "jsx" => (icons::JAVASCRIPT, Tint::Yellow),
        "ts" | "cts" | "tsx" => (icons::TYPESCRIPT, Tint::Blue),
        "css" | "scss" | "sass" | "less" => (icons::CSS, Tint::Blue),
        "json" | "jsonc" | "json5" => (icons::JSON, Tint::Yellow),
        "c" | "h" => (icons::LANG_C, Tint::Blue),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "c++" | "h++" => (icons::LANG_CPP, Tint::Blue),
        "cs" | "csx" => (icons::CSHARP, Tint::Mauve),
        "go" => (icons::GO, Tint::Blue),
        "java" | "jar" | "class" => (icons::JAVA, Tint::Red),
        "kt" | "kts" => (icons::KOTLIN, Tint::Mauve),
        "swift" => (icons::SWIFT, Tint::Peach),
        "lua" => (icons::LUA, Tint::Blue),
        "php" => (icons::PHP, Tint::Mauve),
        "rb" | "erb" | "gemspec" => (icons::RUBY, Tint::Red),
        "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "tcsh" => (icons::SHELL, Tint::Green),
        "ps1" | "psm1" | "psd1" => (icons::POWERSHELL, Tint::Blue),
        "nix" => (icons::NIX, Tint::Blue),
        "hs" | "lhs" => (icons::HASKELL, Tint::Mauve),
        "r" | "rmd" => (icons::LANG_R, Tint::Blue),
        "vue" => (icons::VUE, Tint::Green),
        "svelte" => (icons::SVELTE, Tint::Peach),
        "dart" => (icons::DART, Tint::Blue),
        "zig" => (icons::ZIG, Tint::Peach),
        "ex" | "exs" => (icons::ELIXIR, Tint::Mauve),
        "scala" | "sc" => (icons::SCALA, Tint::Red),
        "pl" | "pm" => (icons::PERL, Tint::Blue),
        "xml" | "xaml" | "xsl" | "xslt" | "plist" => (icons::XML, Tint::Peach),
        "toml" | "yaml" | "yml" | "ini" | "conf" | "cfg" | "env" | "service" | "properties" => (icons::CONFIG, Tint::Muted),
        "sql" | "db" | "sqlite" | "sqlite3" => (icons::DATABASE, Tint::Mauve),
        "svg" | "svgz" | "ai" | "eps" => (icons::VECTOR, Tint::Peach),
        "ttf" | "otf" | "woff" | "woff2" | "eot" => (icons::FONT, Tint::Muted),
        "pem" | "key" | "gpg" | "pub" | "asc" => (icons::KEY, Tint::Yellow),
        "crt" | "cer" | "p12" | "pfx" => (icons::CERTIFICATE, Tint::Yellow),
        "deb" | "rpm" | "apk" | "pkg" | "flatpakref" | "snap" => (icons::PACKAGE, Tint::Peach),
        "appimage" | "exe" | "msi" => (icons::APPLICATION, Tint::Green),
        "torrent" => (icons::TORRENT, Tint::Green),
        "zip" | "zipx" => (icons::ZIP, Tint::Peach),
        "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" | "tgz" | "tbz2" | "txz" | "lz" | "lzma" | "lz4" | "cab" | "cpio" => (icons::ARCHIVE, Tint::Peach),
        "dxf" | "dwg" | "stl" | "obj" | "step" | "stp" | "3mf" | "blend" | "fcstd" => (icons::CAD, Tint::Mauve),
        "raw" | "cr2" | "cr3" | "crw" | "nef" | "nrw" | "arw" | "sr2" | "srf" | "dng" | "orf" | "rw2" | "raf" | "pef" | "srw" | "3fr" | "erf" | "kdc" | "dcr" | "mrw" | "x3f" => (icons::CAMERA_RAW, Tint::Peach),
        "psd" | "xcf" | "kra" | "ora" => (icons::LAYERED_IMAGE, Tint::Blue),
        "gif" => (icons::GIF, Tint::Green),
        "mid" | "midi" => (icons::SHEET_MUSIC, Tint::Mauve),
        "srt" | "vtt" | "ass" | "ssa" => (icons::SUBTITLES, Tint::Yellow),
        "m3u" | "m3u8" | "pls" | "cue" | "xspf" => (icons::PLAYLIST, Tint::Peach),
        "ml" | "mli" => (icons::OCAML, Tint::Peach),
        "fs" | "fsi" | "fsx" => (icons::FSHARP, Tint::Blue),
        "clj" | "cljs" | "cljc" | "edn" => (icons::CLOJURE, Tint::Green),
        "erl" | "hrl" => (icons::ERLANG, Tint::Red),
        "jl" => (icons::JULIA, Tint::Mauve),
        "nim" | "nims" | "nimble" => (icons::NIM, Tint::Yellow),
        "v" | "vsh" => (icons::LANG_V, Tint::Blue),
        "odin" => (icons::ODIN, Tint::Blue),
        "asm" | "s" => (icons::ASSEMBLY, Tint::Red),
        "wasm" | "wat" => (icons::WASM, Tint::Mauve),
        "graphql" | "gql" => (icons::GRAPHQL, Tint::Mauve),
        "proto" => (icons::PROTOBUF, Tint::Green),
        "tf" | "tfvars" | "hcl" => (icons::TERRAFORM, Tint::Mauve),
        "diff" | "patch" => (icons::DIFF, Tint::Green),
        "parquet" | "avro" | "arrow" | "feather" | "h5" | "hdf5" | "npy" | "npz" => (icons::DATASET, Tint::Peach),
        "kdbx" | "kdb" => (icons::PASSWORDS, Tint::Green),
        "ics" | "ical" => (icons::CALENDAR, Tint::Red),
        "vcf" | "vcard" => (icons::CONTACT, Tint::Mauve),
        "eml" | "msg" | "mbox" => (icons::EMAIL, Tint::Blue),
        "url" | "webloc" | "desktop" => (icons::SHORTCUT, Tint::Blue),
        "qcow2" | "qcow" | "vdi" | "vmdk" | "vhd" | "vhdx" | "ova" => (icons::VIRTUAL_DISK, Tint::Mauve),
        "bin" | "so" | "dll" | "dylib" | "o" | "a" | "ko" => (icons::LIBRARY, Tint::Muted),
        "bak" | "swp" | "swo" | "tmp" | "temp" | "old" | "orig" => (icons::BACKUP, Tint::Muted),
        "part" | "crdownload" | "partial" => (icons::PARTIAL, Tint::Yellow),
        _ => return None,
    })
}

/// Like `specific_icon`, also knowing the kinds that their folder gives away: the workflows of
/// GitHub Actions are YAML files in `.github/workflows`.
pub fn specific_icon_at(path: &Path, name: &str) -> Option<(&'static str, Tint)> {
    let lower = name.to_ascii_lowercase();
    if (lower.ends_with(".yml") || lower.ends_with(".yaml")) && path.parent().is_some_and(|dir| dir.ends_with(".github/workflows")) {
        return Some((icons::CI, Tint::Blue));
    }
    specific_icon(name)
}

/// A file of every kind that has an icon of its own, for the icon gallery in Settings: the plain icon
/// of each type (named after the type), then each kind `specific_icon` gives a glyph of its own.
pub const ICON_SAMPLES: &[(&str, FileType)] = &[
    ("", FileType::Other),
    ("", FileType::Document),
    ("", FileType::Image),
    ("", FileType::Audio),
    ("", FileType::Video),
    ("", FileType::Archive),
    ("", FileType::Code),
    ("", FileType::Executable),
    ("", FileType::Symlink),
    ("report.pdf", FileType::Document),
    ("letter.docx", FileType::Document),
    ("budget.xlsx", FileType::Document),
    ("slides.pptx", FileType::Document),
    ("data.csv", FileType::Document),
    ("README.md", FileType::Document),
    ("book.epub", FileType::Document),
    ("app.log", FileType::Document),
    ("LICENSE", FileType::Other),
    ("CHANGELOG.md", FileType::Document),
    ("film.srt", FileType::Other),
    ("index.html", FileType::Code),
    ("style.css", FileType::Code),
    ("app.js", FileType::Code),
    ("app.ts", FileType::Code),
    ("tsconfig.json", FileType::Code),
    ("package.json", FileType::Code),
    ("layout.xml", FileType::Code),
    ("main.rs", FileType::Code),
    ("script.py", FileType::Code),
    ("main.c", FileType::Code),
    ("main.cpp", FileType::Code),
    ("Program.cs", FileType::Code),
    ("main.go", FileType::Code),
    ("Main.java", FileType::Code),
    ("Main.kt", FileType::Code),
    ("App.swift", FileType::Code),
    ("init.lua", FileType::Code),
    ("index.php", FileType::Code),
    ("app.rb", FileType::Code),
    ("run.sh", FileType::Code),
    ("setup.ps1", FileType::Code),
    ("shell.nix", FileType::Code),
    ("Main.hs", FileType::Code),
    ("plot.r", FileType::Code),
    ("App.vue", FileType::Code),
    ("App.svelte", FileType::Code),
    ("main.dart", FileType::Code),
    ("build.zig", FileType::Code),
    ("app.ex", FileType::Code),
    ("Main.scala", FileType::Code),
    ("tool.pl", FileType::Code),
    ("main.ml", FileType::Code),
    ("Program.fs", FileType::Code),
    ("core.clj", FileType::Code),
    ("server.erl", FileType::Code),
    ("model.jl", FileType::Code),
    ("app.nim", FileType::Code),
    ("main.v", FileType::Code),
    ("main.odin", FileType::Code),
    ("boot.asm", FileType::Code),
    ("module.wasm", FileType::Other),
    ("schema.graphql", FileType::Code),
    ("api.proto", FileType::Code),
    ("main.tf", FileType::Code),
    ("analysis.ipynb", FileType::Code),
    ("fix.patch", FileType::Other),
    ("config.toml", FileType::Other),
    ("store.db", FileType::Other),
    ("Dockerfile", FileType::Other),
    (".gitignore", FileType::Other),
    (".env", FileType::Other),
    (".editorconfig", FileType::Other),
    (".gitlab-ci.yml", FileType::Code),
    ("logo.svg", FileType::Image),
    ("IMG_0001.cr2", FileType::Image),
    ("poster.psd", FileType::Image),
    ("cat.gif", FileType::Image),
    ("song.mid", FileType::Audio),
    ("mix.m3u", FileType::Other),
    ("Inter.ttf", FileType::Other),
    ("id.pem", FileType::Other),
    ("cert.crt", FileType::Other),
    ("vault.kdbx", FileType::Other),
    ("disk.iso", FileType::Other),
    ("vm.qcow2", FileType::Other),
    ("photos.zip", FileType::Archive),
    ("app.deb", FileType::Archive),
    ("Setup.exe", FileType::Executable),
    ("linux.torrent", FileType::Other),
    ("model.stl", FileType::Other),
    ("table.parquet", FileType::Other),
    ("event.ics", FileType::Other),
    ("card.vcf", FileType::Other),
    ("message.eml", FileType::Other),
    ("site.url", FileType::Other),
    ("libfoo.so", FileType::Other),
    ("notes.bak", FileType::Other),
    ("movie.part", FileType::Other),
];

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
    /// A made-up file of a name and type, to show its icon; nothing on disk backs it.
    pub fn sample(name: &str, file_type: FileType) -> Self {
        FileItem {
            name: name.to_owned(),
            path: PathBuf::from(name),
            is_dir: file_type == FileType::Directory,
            is_symlink: file_type == FileType::Symlink,
            size: 0,
            modified: None,
            created: None,
            accessed: None,
            uid: 0,
            mode: 0,
            file_type,
            is_hidden: false,
            is_locked: false,
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        let is_hidden = name.starts_with('.');

        let symlink_meta = std::fs::symlink_metadata(path).ok();
        let is_symlink = symlink_meta.as_ref().map(|m| m.file_type().is_symlink()).unwrap_or(false);

        // Follow a symlink for the actual metadata if possible; anything else is already read (a second stat per file is slow on network and USB drives)
        let metadata = if is_symlink { std::fs::metadata(path).ok().or(symlink_meta) } else { symlink_meta };

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
            super::dirsize::label(&self.path)
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
        format_date(self.modified)
    }

    pub fn formatted_created(&self) -> String {
        format_date(self.created)
    }

    pub fn formatted_accessed(&self) -> String {
        format_date(self.accessed)
    }
}

/// A date the way the list shows it: relative to today or in full, as the setting says.
fn format_date(time: Option<SystemTime>) -> String {
    if relative_dates_enabled() { super::props::format_relative(time) } else { super::props::format_datetime(time) }
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
        "rs" | "c" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "hh" | "hxx" | "c++" | "h++"
        | "cs" | "csx" | "py" | "pyc" | "ipynb" | "pyw" | "js" | "mjs" | "cjs" | "ts" | "cts" | "jsx" | "tsx"
        | "html" | "htm" | "xhtml" | "css" | "scss" | "sass" | "less" | "json" | "jsonc" | "json5"
        | "toml" | "yaml" | "yml" | "nix" | "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "tcsh"
        | "ps1" | "psm1" | "psd1" | "lua" | "go" | "java" | "kt" | "kts" | "swift" | "sql"
        | "php" | "rb" | "erb" | "gemspec" | "hs" | "lhs" | "r" | "rmd" | "vue" | "svelte" | "dart"
        | "zig" | "ex" | "exs" | "scala" | "sc" | "pl" | "pm" | "xml" | "xaml"
        | "ml" | "mli" | "fs" | "fsi" | "fsx" | "clj" | "cljs" | "cljc" | "edn" | "erl" | "hrl" | "jl" | "nim" | "nims"
        | "v" | "vsh" | "odin" | "asm" | "s" | "wat" | "graphql" | "gql" | "proto" | "tf" | "tfvars" | "hcl" => FileType::Code,
        _ => FileType::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_samples_each_show_a_different_icon() {
        let mut seen = std::collections::HashSet::new();
        for (name, kind) in ICON_SAMPLES {
            let glyph = specific_icon(name).map_or(kind.icon(), |(g, _)| g);
            assert!(seen.insert(glyph), "{name:?} ({kind:?}) repeats an icon");
        }
    }

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

    #[test]
    fn specific_icons_and_tints_match_authentic_branding() {
        assert_eq!(specific_icon("app.js"), Some((icons::JAVASCRIPT, Tint::Yellow)));
        assert_eq!(specific_icon("module.mjs"), Some((icons::JAVASCRIPT, Tint::Yellow)));
        assert_eq!(specific_icon("config.json"), Some((icons::JSON, Tint::Yellow)));
        assert_eq!(specific_icon("id_rsa.pub"), Some((icons::KEY, Tint::Yellow)));
        assert_eq!(specific_icon("server.crt"), Some((icons::CERTIFICATE, Tint::Yellow)));
        assert_eq!(specific_icon("README.md"), Some((icons::MARKDOWN, Tint::Blue)));
        assert_eq!(specific_icon("ubuntu.torrent"), Some((icons::TORRENT, Tint::Green)));
        assert_eq!(specific_icon("backup.zip"), Some((icons::ZIP, Tint::Peach)));
        assert_eq!(specific_icon("archive.tar.gz"), Some((icons::ARCHIVE, Tint::Peach)));
        assert_eq!(specific_icon("main.rs"), Some((icons::RUST, Tint::Peach)));
        assert_eq!(specific_icon("Program.cs"), Some((icons::CSHARP, Tint::Mauve)));
        assert_eq!(specific_icon("App.vue"), Some((icons::VUE, Tint::Green)));
        assert_eq!(specific_icon("script.ps1"), Some((icons::POWERSHELL, Tint::Blue)));
        assert_eq!(specific_icon("logo.svg"), Some((icons::VECTOR, Tint::Peach)));
    }

    #[test]
    fn project_files_are_known_by_name() {
        assert_eq!(specific_icon("package.json"), Some((icons::NPM, Tint::Red)));
        assert_eq!(specific_icon("Cargo.toml"), Some((icons::RUST, Tint::Peach)));
        assert_eq!(specific_icon("go.mod"), Some((icons::GO, Tint::Blue)));
        assert_eq!(specific_icon("pyproject.toml"), Some((icons::PYTHON, Tint::Blue)));
        assert_eq!(specific_icon("flake.lock"), Some((icons::NIX, Tint::Blue)));
        assert_eq!(specific_icon("docker-compose.prod.yml"), Some((icons::DOCKER, Tint::Blue)));
        assert_eq!(specific_icon("compose.yaml"), Some((icons::DOCKER, Tint::Blue)));
        assert_eq!(specific_icon(".eslintrc.json"), Some((icons::TOOLING, Tint::Muted)));
        assert_eq!(specific_icon(".prettierrc"), Some((icons::TOOLING, Tint::Muted)));
        for name in [".env", ".env.local", ".ENV.production"] {
            assert_eq!(specific_icon(name), Some((icons::SECRETS, Tint::Yellow)), "{name}");
        }
        assert_eq!(specific_icon("my.env"), Some((icons::CONFIG, Tint::Muted)));
        for name in ["LICENSE", "LICENSE.md", "COPYING.txt"] {
            assert_eq!(specific_icon(name), Some((icons::LICENSE, Tint::Yellow)), "{name}");
        }
        assert_eq!(specific_icon("CHANGELOG.md"), Some((icons::CHANGELOG, Tint::Green)));
        // A source file that happens to share the name keeps its language
        assert_eq!(specific_icon("license.rs"), Some((icons::RUST, Tint::Peach)));
        assert_eq!(specific_icon("changes.py"), Some((icons::PYTHON, Tint::Blue)));
        assert_eq!(specific_icon("README"), Some((icons::TEXT, Tint::Muted)));
    }

    #[test]
    fn new_kinds_get_their_own_icons() {
        assert_eq!(specific_icon("disc.cue"), Some((icons::PLAYLIST, Tint::Peach)));
        assert_eq!(specific_icon("notebook.ipynb"), Some((icons::JUPYTER, Tint::Peach)));
        assert_eq!(specific_icon("ski.desktop"), Some((icons::SHORTCUT, Tint::Blue)));
        assert_eq!(specific_icon("libssl.so.3"), Some((icons::LIBRARY, Tint::Muted)));
        assert_eq!(specific_icon("notes.txt~"), Some((icons::BACKUP, Tint::Muted)));
        assert_eq!(specific_icon("DSC_1.NEF"), Some((icons::CAMERA_RAW, Tint::Peach)));
        assert_eq!(detect_file_type("main.odin"), FileType::Code);
        let workflow = Path::new("/repo/.github/workflows/ci.yml");
        assert_eq!(specific_icon_at(workflow, "ci.yml"), Some((icons::CI, Tint::Blue)));
        assert_eq!(specific_icon_at(Path::new("/repo/ci.yml"), "ci.yml"), Some((icons::CONFIG, Tint::Muted)));
    }
}

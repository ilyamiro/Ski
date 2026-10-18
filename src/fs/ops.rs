//! File operations: copy, move, compress and extract. Everything here blocks, so the app runs
//! it on a worker thread through [`Jobs`].
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;

use eframe::egui;

/// A name for `name` inside `dir` that does not exist yet: `a.txt`, `a (copy).txt`, `a (copy 2).txt`, ...
pub fn unique_path(dir: &Path, name: &std::ffi::OsStr, suffix: &str) -> PathBuf {
    let direct = dir.join(name);
    if !direct.exists() && direct.symlink_metadata().is_err() {
        return direct;
    }
    let name = name.to_string_lossy();
    let (stem, ext) = split_name(&name);
    for n in 1.. {
        let tag = if n == 1 { format!(" ({suffix})") } else { format!(" ({suffix} {n})") };
        let candidate = dir.join(format!("{stem}{tag}{ext}"));
        if candidate.symlink_metadata().is_err() {
            return candidate;
        }
    }
    unreachable!()
}

/// `("archive", ".tar.gz")` / `("photo", ".png")` / `("Makefile", "")`.
fn split_name(name: &str) -> (&str, &str) {
    for double in [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst"] {
        if let Some(stem) = name.strip_suffix(double)
            && !stem.is_empty() {
                return (stem, &name[stem.len()..]);
            }
    }
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = src.symlink_metadata()?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dst)
    } else if meta.is_dir() {
        std::fs::create_dir(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        std::fs::set_permissions(dst, meta.permissions())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

fn check_destination(src: &Path, dir: &Path) -> io::Result<()> {
    if !dir.is_dir() {
        return Err(io::Error::other(format!("{} is not a folder", dir.display())));
    }
    let dir = dir.canonicalize()?;
    if src.is_dir() && dir.starts_with(src.canonicalize()?) {
        return Err(io::Error::other(format!("Cannot put {} inside itself", src.display())));
    }
    Ok(())
}

/// Copy `src` into `dir`, never overwriting: a clash gets a " (copy)" name.
pub fn copy_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    check_destination(src, dir)?;
    let name = src.file_name().ok_or_else(|| io::Error::other("no file name"))?;
    let target = unique_path(dir, name, "copy");
    copy_tree(src, &target)?;
    Ok(target)
}

/// Move `src` into `dir`. Moving to where it already is does nothing.
pub fn move_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    check_destination(src, dir)?;
    if src.parent().is_some_and(|p| p.canonicalize().ok() == dir.canonicalize().ok()) {
        return Ok(src.to_path_buf());
    }
    let name = src.file_name().ok_or_else(|| io::Error::other("no file name"))?;
    let target = unique_path(dir, name, "moved");
    match std::fs::rename(src, &target) {
        Ok(()) => Ok(target),
        // Different filesystem: copy, then remove the original
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            copy_tree(src, &target)?;
            if src.is_dir() && !src.is_symlink() { std::fs::remove_dir_all(src)? } else { std::fs::remove_file(src)? }
            Ok(target)
        }
        Err(e) => Err(e),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
    TarXz,
    TarBz2,
    TarZst,
    SevenZ,
}

impl ArchiveFormat {
    pub const ALL: [ArchiveFormat; 6] = [
        ArchiveFormat::Zip,
        ArchiveFormat::TarGz,
        ArchiveFormat::TarXz,
        ArchiveFormat::TarBz2,
        ArchiveFormat::TarZst,
        ArchiveFormat::SevenZ,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => ".zip",
            ArchiveFormat::TarGz => ".tar.gz",
            ArchiveFormat::TarXz => ".tar.xz",
            ArchiveFormat::TarBz2 => ".tar.bz2",
            ArchiveFormat::TarZst => ".tar.zst",
            ArchiveFormat::SevenZ => ".7z",
        }
    }

    /// Zip and tar.gz are built in; the rest need the system's `tar` / `7z` and their compressors.
    pub fn available(self) -> bool {
        match self {
            ArchiveFormat::Zip | ArchiveFormat::TarGz => true,
            ArchiveFormat::TarXz => have("xz") && have("tar"),
            ArchiveFormat::TarBz2 => have("bzip2") && have("tar"),
            ArchiveFormat::TarZst => have("zstd") && have("tar"),
            ArchiveFormat::SevenZ => have("7z") || have("7za"),
        }
    }
}

fn have(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

fn seven_zip() -> &'static str {
    if have("7z") { "7z" } else { "7za" }
}

fn run(mut command: Command) -> io::Result<()> {
    let output = command.stdin(Stdio::null()).output()?;
    if output.status.success() {
        Ok(())
    } else {
        let text = String::from_utf8_lossy(if output.stderr.is_empty() { &output.stdout } else { &output.stderr });
        Err(io::Error::other(text.lines().last().unwrap_or("command failed").to_owned()))
    }
}

/// Whether `path` looks like an archive this module can unpack.
pub fn is_archive(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    [
        ".zip", ".jar", ".apk", ".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tar.zst", ".tzst",
        ".gz", ".xz", ".bz2", ".zst", ".7z", ".rar", ".iso", ".cab", ".deb", ".rpm",
    ]
    .iter()
    .any(|ext| name.ends_with(ext))
}

/// Folder name an archive extracts into: `photos.tar.gz` -> `photos`.
pub fn archive_stem(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for ext in [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tgz", ".txz", ".tbz2", ".tzst"] {
        if let Some(stem) = name.strip_suffix(ext) {
            return stem.to_owned();
        }
    }
    split_name(&name).0.to_owned()
}

/// Pack `sources` (all in the same folder) into the archive `out`.
pub fn compress(sources: &[PathBuf], out: &Path, format: ArchiveFormat) -> io::Result<()> {
    let base = sources
        .first()
        .and_then(|p| p.parent())
        .ok_or_else(|| io::Error::other("nothing to compress"))?
        .to_path_buf();
    let names: Vec<_> = sources.iter().filter_map(|p| p.file_name().map(|n| n.to_owned())).collect();
    let result = match format {
        ArchiveFormat::Zip => write_zip(&base, &names, out),
        ArchiveFormat::TarGz => {
            let encoder = flate2::write::GzEncoder::new(std::fs::File::create(out)?, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            builder.follow_symlinks(false);
            for name in &names {
                let path = base.join(name);
                if path.is_dir() && !path.is_symlink() {
                    builder.append_dir_all(name, &path)?;
                } else {
                    builder.append_path_with_name(&path, name)?;
                }
            }
            builder.into_inner()?.finish().map(|_| ())
        }
        ArchiveFormat::TarXz | ArchiveFormat::TarBz2 | ArchiveFormat::TarZst => {
            let flag = match format {
                ArchiveFormat::TarXz => "--xz",
                ArchiveFormat::TarBz2 => "--bzip2",
                _ => "--zstd",
            };
            let mut cmd = Command::new("tar");
            cmd.arg("-c").arg(flag).arg("-f").arg(out).arg("-C").arg(&base).arg("--").args(&names);
            run(cmd)
        }
        ArchiveFormat::SevenZ => {
            let mut cmd = Command::new(seven_zip());
            cmd.current_dir(&base).arg("a").arg("-y").arg(out).args(&names);
            run(cmd)
        }
    };
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result
}

fn write_zip(base: &Path, names: &[std::ffi::OsString], out: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use zip::write::SimpleFileOptions;

    fn add(zip: &mut zip::ZipWriter<std::fs::File>, path: &Path, name: &str) -> io::Result<()> {
        let meta = path.symlink_metadata()?;
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(meta.permissions().mode() & 0o7777)
            .large_file(meta.len() > u32::MAX as u64);
        if meta.is_dir() {
            zip.add_directory(format!("{name}/"), options).map_err(io::Error::other)?;
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                add(zip, &entry.path(), &format!("{name}/{}", entry.file_name().to_string_lossy()))?;
            }
        } else if meta.file_type().is_symlink() {
            zip.add_symlink(name, std::fs::read_link(path)?.to_string_lossy(), options).map_err(io::Error::other)?;
        } else {
            zip.start_file(name, options).map_err(io::Error::other)?;
            io::copy(&mut std::fs::File::open(path)?, zip)?;
        }
        Ok(())
    }

    let mut zip = zip::ZipWriter::new(std::fs::File::create(out)?);
    for name in names {
        add(&mut zip, &base.join(name), &name.to_string_lossy())?;
    }
    zip.finish().map(|_| ()).map_err(io::Error::other)
}

/// Unpack `archive` into the existing folder `dest`.
pub fn extract(archive: &Path, dest: &Path) -> io::Result<()> {
    let name = archive.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ends = |exts: &[&str]| exts.iter().any(|e| name.ends_with(e));
    if ends(&[".zip", ".jar", ".apk"]) {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?).map_err(io::Error::other)?;
        zip.extract(dest).map_err(io::Error::other)
    } else if ends(&[".tar.gz", ".tgz"]) {
        tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(archive)?)).unpack(dest)
    } else if ends(&[".tar"]) {
        tar::Archive::new(std::fs::File::open(archive)?).unpack(dest)
    } else if ends(&[".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tar.zst", ".tzst"]) && have("tar") {
        let mut cmd = Command::new("tar");
        cmd.arg("-xf").arg(archive).arg("-C").arg(dest);
        run(cmd)
    } else if have("7z") || have("7za") {
        let mut cmd = Command::new(seven_zip());
        let mut out = std::ffi::OsString::from("-o");
        out.push(dest);
        cmd.arg("x").arg("-y").arg(out).arg(archive);
        run(cmd)
    } else {
        Err(io::Error::other("No tool installed to unpack this kind of archive (install 7z)"))
    }
}

/// Extract into a fresh folder next to the archive (`<archive stem>`), returning that folder.
pub fn extract_beside(archive: &Path) -> io::Result<PathBuf> {
    let parent = archive.parent().ok_or_else(|| io::Error::other("no parent folder"))?;
    let dest = unique_path(parent, std::ffi::OsStr::new(&archive_stem(archive)), "extracted");
    std::fs::create_dir(&dest)?;
    match extract(archive, &dest) {
        Ok(()) => Ok(dest),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dest);
            Err(e)
        }
    }
}

/// Background runner: one worker thread per job, results polled once per frame.
pub struct Jobs {
    tx: mpsc::Sender<Outcome>,
    rx: mpsc::Receiver<Outcome>,
    running: usize,
    ctx: egui::Context,
}

pub struct Outcome {
    pub message: String,
    pub ok: bool,
}

impl Jobs {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, running: 0, ctx: ctx.clone() }
    }

    pub fn running(&self) -> bool {
        self.running > 0
    }

    /// Run `work` off the UI thread; it returns the message to show when it finishes.
    pub fn spawn(&mut self, work: impl FnOnce() -> Result<String, String> + Send + 'static) {
        self.running += 1;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let (message, ok) = match work() {
                Ok(m) => (m, true),
                Err(m) => (m, false),
            };
            let _ = tx.send(Outcome { message, ok });
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) -> Vec<Outcome> {
        let done: Vec<Outcome> = self.rx.try_iter().collect();
        self.running = self.running.saturating_sub(done.len());
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let root = std::env::temp_dir().join(format!("ski_ops_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src");
        std::fs::create_dir_all(src.join("d/e")).unwrap();
        std::fs::write(src.join("a.txt"), "hello").unwrap();
        std::fs::write(src.join("d/e/b.txt"), "world").unwrap();
        let srcs = vec![src.join("a.txt"), src.join("d")];
        for fmt in ArchiveFormat::ALL {
            if !fmt.available() { println!("skip {:?}", fmt); continue; }
            let out = src.join(format!("pack{}", fmt.extension()));
            compress(&srcs, &out, fmt).unwrap_or_else(|e| panic!("{fmt:?} compress: {e}"));
            let dest = extract_beside(&out).unwrap_or_else(|e| panic!("{fmt:?} extract: {e}"));
            assert_eq!(std::fs::read_to_string(dest.join("a.txt")).unwrap(), "hello", "{fmt:?}");
            assert_eq!(std::fs::read_to_string(dest.join("d/e/b.txt")).unwrap(), "world", "{fmt:?}");
            println!("ok {:?} -> {}", fmt, dest.display());
        }
        let c = copy_into(&src.join("d"), &src).unwrap();
        assert!(c.ends_with("d (copy)"));
        let other = root.join("other");
        std::fs::create_dir(&other).unwrap();
        let m = move_into(&src.join("a.txt"), &other).unwrap();
        assert!(m.exists() && !src.join("a.txt").exists());
        assert!(copy_into(&src.join("d"), &src.join("d/e")).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}

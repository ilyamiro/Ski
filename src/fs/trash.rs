//! The freedesktop.org trash in the home data folder (`~/.local/share/Trash`): deleted items move
//! to `files/` and a `info/<name>.trashinfo` file remembers where they came from, so every
//! other file manager can restore them too.
use std::io;
use std::path::{Path, PathBuf};

use super::ops::{move_path, remove_tree, unique_path};
use crate::i18n::tr;

pub fn root() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Trash"))
}

/// The folder trashed items live in, as the file manager shows it.
pub fn files_dir() -> Option<PathBuf> {
    root().map(|r| r.join("files"))
}

/// Whether `path` is the trash folder itself or something inside it.
pub fn contains(path: &Path) -> bool {
    files_dir().is_some_and(|files| {
        let files = files.canonicalize().unwrap_or(files);
        path.starts_with(files)
    })
}

/// Whether the trash folder is the folder being looked at.
pub fn is_trash_folder(path: &Path) -> bool {
    files_dir().is_some_and(|files| files.canonicalize().unwrap_or(files) == path)
}

/// Whether `path` sits directly in the trash folder, where its record is.
pub fn is_top_level(path: &Path) -> bool {
    files_dir().is_some_and(|files| path.parent().is_some_and(|parent| parent == files || files.canonicalize().is_ok_and(|f| f == parent)))
}

pub fn is_empty() -> bool {
    files_dir().and_then(|f| std::fs::read_dir(f).ok()).is_none_or(|mut rd| rd.next().is_none())
}

/// Move `path` to the trash, returning where it ended up.
pub fn trash(path: &Path) -> io::Result<PathBuf> {
    let root = root().ok_or_else(|| io::Error::other("no trash folder"))?;
    trash_in(&root, path)
}

fn trash_in(root: &Path, path: &Path) -> io::Result<PathBuf> {
    // A symlink is trashed as the link itself, so only its folder is made canonical
    let original = if path.is_symlink() {
        path.parent().unwrap_or(Path::new("/")).canonicalize()?.join(path.file_name().unwrap_or_default())
    } else {
        path.canonicalize()?
    };
    let (files, info) = (root.join("files"), root.join("info"));
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;

    let name = original.file_name().ok_or_else(|| io::Error::other("no file name"))?;
    let target = unique_path(&files, name, "copy");
    let record = info.join(format!("{}.trashinfo", target.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&record, format!("[Trash Info]\nPath={}\nDeletionDate={}\n", encode(&original), now()))?;
    if let Err(err) = move_path(path, &target) {
        let _ = std::fs::remove_file(&record);
        return Err(err);
    }
    Ok(target)
}

/// Put a trashed item back where it came from (next to it under a new name if that is taken).
pub fn restore(trashed: &Path) -> io::Result<PathBuf> {
    if trashed.parent().and_then(Path::file_name).is_none_or(|n| n != "files") {
        return Err(io::Error::other(tr("The trash has no record of where this came from")));
    }
    let root = trashed.parent().and_then(Path::parent).ok_or_else(|| io::Error::other("not in the trash"))?;
    let record = record_of(root, trashed);
    let text = std::fs::read_to_string(&record).map_err(|_| io::Error::other(tr("The trash has no record of where this came from")))?;
    let original = text
        .lines()
        .find_map(|l| l.strip_prefix("Path="))
        .map(|p| PathBuf::from(decode(p.trim())))
        .ok_or_else(|| io::Error::other(tr("The trash has no record of where this came from")))?;
    let folder = original.parent().ok_or_else(|| io::Error::other("invalid original location"))?;
    std::fs::create_dir_all(folder)?;
    let target = unique_path(folder, original.file_name().unwrap_or_default(), "restored");
    move_path(trashed, &target)?;
    let _ = std::fs::remove_file(record);
    Ok(target)
}

/// Delete an item for good; for trashed items this also drops their record.
pub fn delete_forever(path: &Path) -> io::Result<()> {
    remove_tree(path)?;
    if contains(path)
        && let Some(root) = path.parent().and_then(Path::parent) {
            let _ = std::fs::remove_file(record_of(root, path));
        }
    Ok(())
}

pub fn empty() -> io::Result<()> {
    let Some(root) = root() else { return Ok(()) };
    for sub in ["files", "info"] {
        let Ok(entries) = std::fs::read_dir(root.join(sub)) else { continue };
        for entry in entries.flatten() {
            remove_tree(&entry.path())?;
        }
    }
    Ok(())
}

fn record_of(root: &Path, trashed: &Path) -> PathBuf {
    root.join("info").join(format!("{}.trashinfo", trashed.file_name().unwrap_or_default().to_string_lossy()))
}

fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    // SAFETY: `tm` is plain data and `localtime_r` only writes into it.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// Percent-encode a path for a `.trashinfo` file, keeping `/` readable.
fn encode(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    std::ffi::OsString::from_vec(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_and_restore() {
        let base = std::env::temp_dir().join(format!("ski_trash_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, work) = (base.join("Trash"), base.join("work dir"));
        std::fs::create_dir_all(work.join("sub")).unwrap();
        std::fs::write(work.join("a b.txt"), "x").unwrap();
        std::fs::write(work.join("sub/in.txt"), "y").unwrap();

        let t1 = trash_in(&root, &work.join("a b.txt")).unwrap();
        assert!(t1.exists() && !work.join("a b.txt").exists());
        std::fs::write(work.join("a b.txt"), "second").unwrap();
        let t2 = trash_in(&root, &work.join("a b.txt")).unwrap();
        assert_ne!(t1, t2, "clashing names get a fresh name");
        let t3 = trash_in(&root, &work.join("sub")).unwrap();

        // Restore puts items back, and moves aside what is in the way
        std::fs::write(work.join("a b.txt"), "occupied").unwrap();
        let back = restore(&t1).unwrap();
        assert_eq!(std::fs::read_to_string(&back).unwrap(), "x");
        assert_ne!(back, work.join("a b.txt"));
        let back = restore(&t3).unwrap();
        assert_eq!(back, work.join("sub"));
        assert!(work.join("sub/in.txt").exists());
        assert!(!root.join("info").join(format!("{}.trashinfo", t1.file_name().unwrap().to_string_lossy())).exists());

        // Something inside a trashed folder has no record of its own and can't be restored alone
        let again = trash_in(&root, &work.join("sub")).unwrap();
        assert!(restore(&again.join("in.txt")).is_err());
        assert!(again.join("in.txt").exists());

        assert_eq!(decode(&encode(Path::new("/a b/ü%x"))), std::ffi::OsString::from("/a b/ü%x"));
        std::fs::remove_dir_all(&base).unwrap();
    }
}

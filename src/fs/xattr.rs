//! Extended attributes through `libc`, without following symbolic links (a link cannot carry
//! `user.*` attributes). Tags live in `user.xdg.tags`; copying a file must carry its attributes along.
use std::ffi::{CString, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

fn c_name(name: &str) -> io::Result<CString> {
    CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// The file system cannot store attributes (FAT, exFAT, some network shares).
pub fn is_unsupported(err: &io::Error) -> bool {
    err.raw_os_error() == Some(libc::ENOTSUP)
}

/// The value of `name`, or `None` when the file has no such attribute.
pub fn get(path: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let (path, name) = (c_path(path)?, c_name(name)?);
    loop {
        // SAFETY: both pointers are NUL-terminated and a null buffer with size 0 only asks for the length.
        let size = unsafe { libc::lgetxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) };
        if size < 0 {
            return match io::Error::last_os_error() {
                e if e.raw_os_error() == Some(libc::ENODATA) => Ok(None),
                e => Err(e),
            };
        }
        let mut buffer = vec![0u8; size as usize];
        // SAFETY: the buffer holds `size` writable bytes.
        let read = unsafe { libc::lgetxattr(path.as_ptr(), name.as_ptr(), buffer.as_mut_ptr().cast(), buffer.len()) };
        if read >= 0 {
            buffer.truncate(read as usize);
            return Ok(Some(buffer));
        }
        let err = io::Error::last_os_error();
        // The value grew between the two calls: ask again
        if err.raw_os_error() != Some(libc::ERANGE) {
            return Err(err);
        }
    }
}

pub fn set(path: &Path, name: &str, value: &[u8]) -> io::Result<()> {
    let (path, name) = (c_path(path)?, c_name(name)?);
    // SAFETY: the value pointer and length describe `value`.
    let result = unsafe { libc::lsetxattr(path.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0) };
    if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// Removes `name`; a missing attribute is not an error.
pub fn remove(path: &Path, name: &str) -> io::Result<()> {
    let (path, name) = (c_path(path)?, c_name(name)?);
    // SAFETY: both pointers are NUL-terminated.
    let result = unsafe { libc::lremovexattr(path.as_ptr(), name.as_ptr()) };
    match io::Error::last_os_error() {
        _ if result == 0 => Ok(()),
        e if e.raw_os_error() == Some(libc::ENODATA) => Ok(()),
        e => Err(e),
    }
}

/// Names of all attributes of the file.
pub fn list(path: &Path) -> io::Result<Vec<String>> {
    let path = c_path(path)?;
    loop {
        // SAFETY: a null buffer with size 0 only asks for the length.
        let size = unsafe { libc::llistxattr(path.as_ptr(), std::ptr::null_mut(), 0) };
        if size < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u8; size as usize];
        // SAFETY: the buffer holds `size` writable bytes.
        let read = unsafe { libc::llistxattr(path.as_ptr(), buffer.as_mut_ptr().cast(), buffer.len()) };
        if read >= 0 {
            buffer.truncate(read as usize);
            return Ok(buffer.split(|b| *b == 0).filter(|n| !n.is_empty()).map(|n| OsStr::from_bytes(n).to_string_lossy().into_owned()).collect());
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ERANGE) {
            return Err(err);
        }
    }
}

/// Copies the `user.*` attributes (tags, comments, ratings) from `src` to `dst`. Failures are
/// skipped: a copy onto a disk without attributes is still a good copy.
pub fn copy_user(src: &Path, dst: &Path) {
    let Ok(names) = list(src) else { return };
    for name in names.iter().filter(|n| n.starts_with("user.")) {
        if let Ok(Some(value)) = get(src, name) {
            let _ = set(dst, name, &value);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A temp file on a file system that stores attributes, or None when the test cannot run here.
    pub(crate) fn scratch_file(tag: &str) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        let dir = std::env::temp_dir().join(format!("ski-xattr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let file = dir.join("a.txt");
        std::fs::write(&file, "x").ok()?;
        match set(&file, "user.ski.probe", b"1") {
            Ok(()) => Some((dir, file)),
            Err(e) if is_unsupported(&e) => {
                let _ = std::fs::remove_dir_all(&dir);
                None
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn set_get_list_remove_round_trip() {
        let Some((dir, file)) = scratch_file("rt") else { return };
        assert_eq!(get(&file, "user.ski.probe").unwrap().as_deref(), Some(&b"1"[..]));
        assert_eq!(get(&file, "user.ski.missing").unwrap(), None);
        assert!(list(&file).unwrap().contains(&"user.ski.probe".to_owned()));
        remove(&file, "user.ski.probe").unwrap();
        remove(&file, "user.ski.probe").unwrap();
        assert_eq!(get(&file, "user.ski.probe").unwrap(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copy_user_carries_attributes_over() {
        let Some((dir, file)) = scratch_file("copy") else { return };
        set(&file, "user.xdg.tags", b"work,home").unwrap();
        let copy = dir.join("b.txt");
        std::fs::write(&copy, "x").unwrap();
        copy_user(&file, &copy);
        assert_eq!(get(&copy, "user.xdg.tags").unwrap().as_deref(), Some(&b"work,home"[..]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn symbolic_links_are_not_followed() {
        let Some((dir, file)) = scratch_file("link") else { return };
        set(&file, "user.xdg.tags", b"work").unwrap();
        let link = dir.join("l");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(get(&link, "user.xdg.tags").unwrap(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

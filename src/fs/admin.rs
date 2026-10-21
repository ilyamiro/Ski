//! Open a folder in a second Ski that runs as root. The person asks for the password through
//! polkit (`pkexec`), the same way graphical tools elevate, so Ski never sees it.
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;

/// Tells the elevated Ski where the settings of the person who started it live. They are read
/// from there but never written, so root does not leave files in someone else's home.
pub const CONFIG_ENV: &str = "SKI_ADMIN_CONFIG";

/// Variables the elevated process needs to reach the display, the session and the same libraries.
const PASS_ENV: &[&str] = &[
    "DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_RUNTIME_DIR", "XDG_SESSION_TYPE", "XDG_CURRENT_DESKTOP",
    "XDG_DATA_DIRS", "LANG", "LANGUAGE", "LC_ALL", "LD_LIBRARY_PATH", "XCURSOR_THEME", "XCURSOR_SIZE", "GDK_SCALE",
];

/// Whether this process already runs as root.
pub fn is_admin() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// Whether the elevated Ski was started from another one, and so reads that one's settings.
pub fn config_override() -> Option<PathBuf> {
    std::env::var_os(CONFIG_ENV).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

fn pkexec() -> Option<PathBuf> {
    let paths = std::env::var_os("PATH").unwrap_or_default();
    // NixOS and a few other systems keep the setuid wrapper outside the usual PATH of a desktop session
    let extra = ["/run/wrappers/bin", "/usr/bin", "/usr/local/bin"].map(PathBuf::from);
    std::env::split_paths(&paths).chain(extra).map(|dir| dir.join("pkexec")).find(|p| p.is_file())
}

/// Whether administrator access can be asked for at all.
pub fn available() -> bool {
    !is_admin() && pkexec().is_some()
}

/// Start Ski as root on `dir`. The result arrives once polkit is done: `Err` carries what went
/// wrong, and nothing is sent when the person simply dismissed the password prompt.
pub fn open_folder(dir: &Path) -> io::Result<mpsc::Receiver<io::Error>> {
    let pkexec = pkexec().ok_or_else(|| io::Error::other(crate::i18n::tr("pkexec was not found. Install polkit to use administrator mode")))?;
    let exe = std::env::current_exe()?;
    let mut command = Command::new(pkexec);
    command.arg("env");
    for name in PASS_ENV {
        if let Some(value) = std::env::var_os(name) {
            let mut pair = std::ffi::OsString::from(format!("{name}="));
            pair.push(value);
            command.arg(pair);
        }
    }
    if let Some(config) = dirs::config_dir() {
        let mut pair = std::ffi::OsString::from(format!("{CONFIG_ENV}="));
        pair.push(config);
        command.arg(pair);
    }
    let mut child = command
        .arg(exe)
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // pkexec: 126 is a dismissed or refused prompt, 127 is no agent or a failure to start
        if let Ok(status) = child.wait()
            && status.code() == Some(127)
        {
            let _ = tx.send(io::Error::other(crate::i18n::tr("Could not get administrator rights. Is a polkit agent running?")));
        }
    });
    Ok(rx)
}

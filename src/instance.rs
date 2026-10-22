//! Keeps Ski resident after its window closes, the way Nautilus stays running, so a later launch
//! only has to open a window: starting the process and loading the graphics driver are paid once.
//! A launch hands its request to the resident process over a socket in `$XDG_RUNTIME_DIR`. While
//! that process already shows a window, the launch opens its own as a separate process.
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// The activation tokens a launcher passes so the new window may take the focus.
const TOKEN_VARS: [&str; 2] = ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"];
const PROTOCOL: &[u8] = b"ski-1";

/// Answers to a launch. Anything else, or no answer, means "open the window yourself".
const SHOWN: u8 = b'y';
/// The resident process belongs to another build of Ski and has quit; start a fresh one.
const RETIRED: u8 = b'r';
const BUSY: u8 = b'n';

/// One launch: the folder it was started in, its arguments, and what it needs to open a window.
pub struct Request {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    tokens: [String; 2],
    exe: PathBuf,
    reply: Option<UnixStream>,
}

impl Request {
    /// The launch this process was started for.
    pub fn current(args: Vec<String>) -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            args,
            tokens: TOKEN_VARS.map(|var| std::env::var(var).unwrap_or_default()),
            exe: std::env::current_exe().unwrap_or_default(),
            reply: None,
        }
    }

    /// Tell the launch that handed this request over that its window is up.
    pub fn shown(&mut self) {
        if let Some(mut stream) = self.reply.take() {
            let _ = stream.write_all(&[SHOWN]);
        }
    }

    fn encode(&self) -> Vec<u8> {
        // NUL cannot occur in a path or an argument, so it separates the fields
        let mut fields: Vec<&[u8]> = vec![PROTOCOL, self.exe.as_os_str().as_bytes(), self.cwd.as_os_str().as_bytes()];
        fields.extend(self.tokens.iter().map(|t| t.as_bytes()));
        fields.extend(self.args.iter().map(|a| a.as_bytes()));
        fields.join(&0)
    }

    fn decode(data: &[u8]) -> Option<Self> {
        let mut fields = data.split(|b| *b == 0);
        if fields.next()? != PROTOCOL {
            return None;
        }
        let path = |field: &[u8]| PathBuf::from(OsStr::from_bytes(field));
        let text = |field: &[u8]| String::from_utf8_lossy(field).into_owned();
        let exe = path(fields.next()?);
        let cwd = path(fields.next()?);
        let tokens = [text(fields.next()?), text(fields.next()?)];
        Some(Self { cwd, args: fields.map(text).collect(), tokens, exe, reply: None })
    }

    /// Put the launcher's activation tokens where the window creation looks for them.
    fn apply_tokens(&self) {
        for (var, value) in TOKEN_VARS.iter().zip(&self.tokens) {
            // SAFETY: egui-winit reads and clears these variables in the same way when it creates the
            // window; they are set here on the main thread before the window exists.
            unsafe {
                if value.is_empty() {
                    std::env::remove_var(var);
                } else {
                    std::env::set_var(var, value);
                }
            }
        }
    }
}

/// The socket of the resident Ski for this user and display; `None` where there is no such place.
fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty())?;
    let display = ["WAYLAND_DISPLAY", "DISPLAY"].iter().find_map(|var| std::env::var(var).ok().filter(|d| !d.is_empty()))?;
    let display: String = display.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    Some(PathBuf::from(runtime).join(format!("ski-{display}.sock")))
}

/// Have the resident Ski open the window for `request`, starting it first when none runs.
/// False when this process should open the window itself.
pub fn hand_off(request: &Request) -> bool {
    let Some(path) = socket_path() else { return false };
    match send(&path, request) {
        Some(SHOWN) => return true,
        Some(RETIRED) | None => {}
        Some(_) => return false,
    }
    let Ok(mut resident) = start_resident() else { return false };
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match send(&path, request) {
            Some(answer) => return answer == SHOWN,
            // It quit without listening: another one won the race, or the socket cannot be made
            None if resident.try_wait().is_ok_and(|status| status.is_some()) => return send(&path, request) == Some(SHOWN),
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    false
}

/// Let an idle resident Ski quit, as the user turned it off.
pub fn dismiss(request: &Request) {
    if let Some(path) = socket_path() {
        send(&path, request);
    }
}

/// The resident process's answer to `request`; `None` when nothing listens.
fn send(path: &Path, request: &Request) -> Option<u8> {
    let mut stream = UnixStream::connect(path).ok()?;
    stream.write_all(&request.encode()).ok()?;
    stream.shutdown(Shutdown::Write).ok()?;
    // A cold window can take a while to come up, but a stuck process must not hold the launch forever
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let mut answer = [0u8];
    Some(match stream.read(&mut answer) {
        Ok(1) => answer[0],
        _ => BUSY,
    })
}

/// Start `ski --resident` in a session of its own, so it outlives the terminal or launcher.
fn start_resident() -> std::io::Result<Child> {
    let mut command = Command::new(std::env::current_exe()?);
    // Out of the launch folder, which may be a drive the user wants to unmount
    command.arg("--resident").current_dir("/").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    for var in TOKEN_VARS {
        command.env_remove(var);
    }
    // SAFETY: setsid is async-signal-safe and touches no memory of the parent.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    command.spawn()
}

/// Run as the resident process: open a window with `open` for each launch handed over, then wait
/// for the next. Returns once the user turned this off, or a window could not be opened.
pub fn serve(mut open: impl FnMut(Request) -> eframe::Result) {
    let Some(path) = socket_path() else { return };
    let Some(listener) = bind(&path) else { return };
    let showing = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    {
        let (path, showing) = (path.clone(), showing.clone());
        std::thread::spawn(move || listen(listener, &path, &showing, &tx));
    }

    for request in rx {
        request.apply_tokens();
        let result = open(request);
        // `showing` stays set until the decision, so a launch meanwhile opens its own window
        if result.is_err() || !crate::settings::Settings::load().run_in_background {
            let _ = std::fs::remove_file(&path);
            return;
        }
        release_memory();
        showing.store(false, Ordering::SeqCst);
    }
}

/// Claim the socket, taking over one left behind by a resident process that died.
fn bind(path: &Path) -> Option<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => Some(listener),
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse && UnixStream::connect(path).is_err() => {
            let _ = std::fs::remove_file(path);
            UnixListener::bind(path).ok()
        }
        // Another resident process already serves this display
        Err(_) => None,
    }
}

fn listen(listener: UnixListener, path: &Path, showing: &AtomicBool, tx: &mpsc::Sender<Request>) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut data = Vec::new();
        // A launch writes its request and closes its side right away; a stuck one must not block the rest
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        if (&mut stream).take(1 << 20).read_to_end(&mut data).is_err() {
            continue;
        }
        let Some(mut request) = Request::decode(&data) else { continue };
        // Quit once idle when a launch of another build (an update replaced this binary) wants a
        // resident process of its own, or the user turned this off since
        let other_build = std::env::current_exe().ok().as_deref() != Some(request.exe.as_path());
        if other_build || !crate::settings::Settings::load().run_in_background {
            if showing.load(Ordering::SeqCst) {
                let _ = stream.write_all(&[BUSY]);
                continue;
            }
            let _ = std::fs::remove_file(path);
            let _ = stream.write_all(&[if other_build { RETIRED } else { BUSY }]);
            std::process::exit(0);
        }
        if showing.swap(true, Ordering::SeqCst) {
            let _ = stream.write_all(&[BUSY]);
            continue;
        }
        request.reply = Some(stream);
        if tx.send(request).is_err() {
            return;
        }
    }
}

/// Hand the memory of the closed window back to the system while waiting.
fn release_memory() {
    #[cfg(target_env = "gnu")]
    // SAFETY: malloc_trim only releases free heap memory.
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_survive_the_wire() {
        let request = Request {
            cwd: PathBuf::from("/home/a b"),
            args: vec!["dir".into(), String::new(), "--x".into()],
            tokens: ["tok".into(), String::new()],
            exe: PathBuf::from("/usr/bin/ski"),
            reply: None,
        };
        let back = Request::decode(&request.encode()).unwrap();
        assert_eq!(back.cwd, request.cwd);
        assert_eq!(back.args, request.args);
        assert_eq!(back.tokens, request.tokens);
        assert_eq!(back.exe, request.exe);
        assert!(Request::decode(b"other\0x").is_none());
    }
}

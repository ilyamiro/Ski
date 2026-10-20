//! `ski --portal`: a session D-Bus service that makes Ski the system's file dialog and file
//! manager. It implements the FileChooser backend of xdg-desktop-portal
//! (`org.freedesktop.impl.portal.FileChooser`) and `org.freedesktop.FileManager1`, the interface
//! browsers use for "Show in folder". Each dialog request runs `ski --chooser …` in a window of
//! its own and reads the choice from its stdout (see `chooser.rs`).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use zbus::zvariant::{DeserializeDict, ObjectPath, OwnedObjectPath, SerializeDict, Type};
use zbus::{blocking as zblocking, interface};

const PORTAL_NAME: &str = "org.freedesktop.impl.portal.desktop.ski";
const FILE_MANAGER_NAME: &str = "org.freedesktop.FileManager1";

/// Portal response codes: 0 = chosen, 1 = cancelled by the user, 2 = something else ended it.
const RESPONSE_OK: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const RESPONSE_FAILED: u32 = 2;

type FilterSpec = (String, Vec<(u32, String)>);
/// A checkbox (no options) or a drop-down: id, label, the options as (id, label), the initial value.
type ChoiceSpec = (String, String, Vec<(String, String)>, String);

#[derive(DeserializeDict, Type, Debug, Default)]
#[zvariant(signature = "dict")]
struct Options {
    multiple: Option<bool>,
    directory: Option<bool>,
    filters: Option<Vec<FilterSpec>>,
    current_filter: Option<FilterSpec>,
    current_name: Option<String>,
    current_folder: Option<Vec<u8>>,
    current_file: Option<Vec<u8>>,
    accept_label: Option<String>,
    files: Option<Vec<Vec<u8>>>,
    choices: Option<Vec<ChoiceSpec>>,
}

#[derive(SerializeDict, Type, Debug, Default)]
#[zvariant(signature = "dict")]
struct Results {
    uris: Option<Vec<String>>,
    /// The filter that was active when the dialog ended: programs read it to learn the type chosen.
    current_filter: Option<FilterSpec>,
    choices: Option<Vec<(String, String)>>,
}

/// Paths travel as NUL-terminated byte strings.
fn path_from_bytes(mut bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

/// The filters offered, in the order the dialog numbers them: a filter named only as the current
/// one still has to be offered.
fn all_filters(options: &Options) -> Vec<FilterSpec> {
    let mut filters = options.filters.clone().unwrap_or_default();
    if let Some(current) = &options.current_filter
        && !filters.contains(current) {
            filters.push(current.clone());
        }
    filters
}

fn filter_args(options: &Options, args: &mut Vec<String>) {
    let encode = |(label, patterns): &FilterSpec| {
        let mut out = label.replace('\t', " ");
        for (kind, pattern) in patterns {
            out.push('\t');
            out.push_str(if *kind == 0 { "g:" } else { "m:" });
            out.push_str(pattern);
        }
        out
    };
    let filters = all_filters(options);
    for filter in &filters {
        args.push("--filter".into());
        args.push(encode(filter));
    }
    if let Some(at) = options.current_filter.as_ref().and_then(|c| filters.iter().position(|f| f == c)) {
        args.push("--current-filter".into());
        args.push(at.to_string());
    }
}

fn choice_args(options: &Options, args: &mut Vec<String>) {
    let clean = |text: &str| text.replace('\t', " ");
    for (id, label, choices, initial) in options.choices.iter().flatten() {
        let mut out = [id, label, initial].map(|t| clean(t)).join("\t");
        for (value, name) in choices {
            out.push_str(&format!("\t{}\t{}", clean(value), clean(name)));
        }
        args.extend(["--choice".into(), out]);
    }
}

/// What a closed dialog hands back.
struct Outcome {
    uris: Vec<String>,
    filter: Option<usize>,
    choices: Vec<(String, String)>,
}

/// The running dialog of one request, which the portal can end through `Request.Close`.
#[derive(Default)]
struct Running {
    pid: AtomicU32,
    closed: AtomicBool,
}

impl Running {
    fn stop(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let pid = self.pid.load(Ordering::SeqCst);
        if pid != 0 {
            // SAFETY: plain signal to the child process this request started
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        }
    }
}

/// `org.freedesktop.impl.portal.Request`, exported at the request's handle so the program can
/// withdraw the request (it closed its window, or gave up) and the dialog goes away with it.
struct Request(Arc<Running>);

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        self.0.stop();
    }
}

/// Runs the dialog and returns the chosen URIs, or `None` when it was cancelled.
fn run_dialog(mode: &str, title: &str, parent: &str, app_id: &str, options: &Options, running: &Running) -> Result<Option<Outcome>, String> {
    let mut args: Vec<String> = vec!["--chooser".into(), "--mode".into(), mode.into()];
    if options.multiple == Some(true) {
        args.push("--multiple".into());
    }
    if !title.is_empty() {
        args.extend(["--title".into(), title.to_owned()]);
    }
    if !parent.is_empty() {
        args.extend(["--parent".into(), parent.to_owned()]);
    }
    if !app_id.is_empty() {
        args.extend(["--app-id".into(), app_id.to_owned()]);
    }
    if let Some(label) = &options.accept_label {
        args.extend(["--accept".into(), label.clone()]);
    }
    if let Some(name) = &options.current_name {
        args.extend(["--name".into(), name.clone()]);
    }
    let folder = options.current_folder.clone().map(path_from_bytes).or_else(|| {
        // A current file also tells where to start (and, when saving, what to call it)
        options.current_file.clone().map(path_from_bytes).and_then(|f| f.parent().map(PathBuf::from))
    });
    if let Some(folder) = folder {
        args.extend(["--folder".into(), folder.to_string_lossy().into_owned()]);
    }
    if options.current_name.is_none()
        && let Some(file) = options.current_file.clone().map(path_from_bytes)
        && let Some(name) = file.file_name() {
            args.extend(["--name".into(), name.to_string_lossy().into_owned()]);
        }
    for file in options.files.iter().flatten() {
        if let Some(name) = path_from_bytes(file.clone()).file_name() {
            args.extend(["--file".into(), name.to_string_lossy().into_owned()]);
        }
    }
    filter_args(options, &mut args);
    choice_args(options, &mut args);

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let child = Command::new(exe)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| e.to_string())?;
    running.pid.store(child.id(), Ordering::SeqCst);
    if running.closed.load(Ordering::SeqCst) {
        // Closed before the dialog was even up
        running.stop();
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    running.pid.store(0, Ordering::SeqCst);

    let text = String::from_utf8_lossy(&output.stdout);
    let mut outcome = Outcome { uris: Vec::new(), filter: None, choices: Vec::new() };
    for line in text.lines().filter(|l| !l.is_empty()) {
        let mut parts = line.split('\t');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("filter"), Some(index), None) => outcome.filter = index.parse().ok(),
            (Some("choice"), Some(id), Some(value)) => outcome.choices.push((id.to_owned(), value.to_owned())),
            _ => outcome.uris.push(line.to_owned()),
        }
    }
    Ok((output.status.success() && !outcome.uris.is_empty()).then_some(outcome))
}

async fn respond(server: &zbus::ObjectServer, handle: ObjectPath<'_>, mode: &'static str, app_id: String, title: String, parent: String, options: Options) -> (u32, Results) {
    let running = Arc::new(Running::default());
    let handle = OwnedObjectPath::from(handle);
    // Without the request object the dialog still works, it just cannot be withdrawn
    let exported = server.at(&handle, Request(running.clone())).await.unwrap_or(false);
    let filters = all_filters(&options);
    // The dialog can stay open for minutes; keep the connection's executor free meanwhile
    let answer = {
        let running = running.clone();
        ::blocking::unblock(move || run_dialog(mode, &title, &parent, &app_id, &options, &running)).await
    };
    if exported {
        let _ = server.remove::<Request, _>(&handle).await;
    }
    match answer {
        Ok(Some(outcome)) => (
            RESPONSE_OK,
            Results {
                uris: Some(outcome.uris),
                current_filter: outcome.filter.and_then(|i| filters.get(i).cloned()),
                choices: (!outcome.choices.is_empty()).then_some(outcome.choices),
            },
        ),
        Ok(None) => (RESPONSE_CANCELLED, Results::default()),
        Err(err) => {
            eprintln!("ski portal: could not start the dialog: {err}");
            (RESPONSE_FAILED, Results::default())
        }
    }
}

struct FileChooser;

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    async fn open_file(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        options: Options,
    ) -> (u32, Results) {
        let mode = if options.directory == Some(true) { "folder" } else { "open" };
        respond(server, handle, mode, app_id.to_owned(), title.to_owned(), parent_window.to_owned(), options).await
    }

    async fn save_file(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        options: Options,
    ) -> (u32, Results) {
        respond(server, handle, "save", app_id.to_owned(), title.to_owned(), parent_window.to_owned(), options).await
    }

    async fn save_files(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        options: Options,
    ) -> (u32, Results) {
        respond(server, handle, "save-files", app_id.to_owned(), title.to_owned(), parent_window.to_owned(), options).await
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        3
    }
}

struct FileManager;

impl FileManager {
    fn show(uris: &[String]) {
        let Ok(exe) = std::env::current_exe() else { return };
        for path in uris.iter().filter_map(|u| crate::chooser::from_uri(u)) {
            let _ = Command::new(&exe).arg(path).stdin(Stdio::null()).spawn();
        }
    }
}

#[interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn show_folders(&self, uris: Vec<String>, _startup_id: &str) {
        Self::show(&uris);
    }

    /// Opens the folder that holds each item and selects the item.
    fn show_items(&self, uris: Vec<String>, _startup_id: &str) {
        Self::show(&uris);
    }

    fn show_item_properties(&self, uris: Vec<String>, _startup_id: &str) {
        Self::show(&uris);
    }
}

pub fn serve() -> Result<(), String> {
    let connection = zblocking::connection::Builder::session()
        .and_then(|b| b.name(PORTAL_NAME))
        .and_then(|b| b.serve_at("/org/freedesktop/portal/desktop", FileChooser))
        .and_then(|b| b.serve_at("/org/freedesktop/FileManager1", FileManager))
        .and_then(|b| b.build())
        .map_err(|e| format!("could not claim {PORTAL_NAME}: {e}"))?;
    // Another file manager may already own this name; the portal works without it
    if let Err(err) = connection.request_name(FILE_MANAGER_NAME) {
        eprintln!("ski portal: {FILE_MANAGER_NAME} is not available: {err}");
    }
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_paths_lose_their_terminator() {
        assert_eq!(path_from_bytes(b"/home/a\0".to_vec()), PathBuf::from("/home/a"));
    }

    #[test]
    fn filters_become_arguments() {
        let options = Options {
            filters: Some(vec![("Images".into(), vec![(1, "image/png".into()), (0, "*.jpg".into())])]),
            current_filter: Some(("Images".into(), vec![(1, "image/png".into()), (0, "*.jpg".into())])),
            ..Default::default()
        };
        let mut args = Vec::new();
        filter_args(&options, &mut args);
        assert_eq!(args, ["--filter", "Images\tm:image/png\tg:*.jpg", "--current-filter", "0"]);
    }
}

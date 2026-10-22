//! Custom actions: your own commands in the context menu. Every `*.json` file and every
//! executable script in `~/.config/ski/actions/` is one entry.
//!
//! ```json
//! { "name": "Convert to WebP", "command": "for f in \"$@\"; do cwebp \"$f\" -o \"${f%.*}.webp\"; done",
//!   "extensions": ["jpg", "png"] }
//! ```
//!
//! `command` runs in `sh -c` inside the open folder with the selected files as arguments (`"$@"`).
//! Both kinds also get `SKI_SELECTED_FILES` (one path per line) and `SKI_CURRENT_DIR`.
//! Optional filters: `extensions`, `dirs_only`, `files_only`, `single` (exactly one item).
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

use serde::Deserialize;

use super::{MenuItem, Plugin};
use crate::fs::file_item::FileItem;
use crate::i18n::{tr, trf};
use crate::ui::icons;

#[derive(Debug, Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
pub struct Definition {
    pub name: String,
    pub command: String,
    pub extensions: Vec<String>,
    pub dirs_only: bool,
    pub files_only: bool,
    pub single: bool,
}

struct Entry {
    def: Definition,
    /// Menu labels are `&'static str`; the few actions of a user are leaked once at startup.
    label: &'static str,
    /// A script run by its own path instead of through `sh -c`.
    script: Option<PathBuf>,
}

pub struct Actions {
    entries: Vec<Entry>,
    done_tx: mpsc::Sender<String>,
    done_rx: mpsc::Receiver<String>,
    running: Arc<AtomicUsize>,
}

pub fn dir() -> Option<PathBuf> {
    crate::fs::admin::config_override().or_else(dirs::config_dir).map(|d| d.join("ski").join("actions"))
}

impl Actions {
    pub fn load() -> Self {
        let (done_tx, done_rx) = mpsc::channel();
        let entries = dir().map(|d| read_dir(&d)).unwrap_or_default();
        Self { entries, done_tx, done_rx, running: Arc::default() }
    }
}

fn read_dir(dir: &Path) -> Vec<Entry> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    files.into_iter().filter_map(|path| read_entry(&path)).collect()
}

fn read_entry(path: &Path) -> Option<Entry> {
    std::fs::metadata(path).ok()?.is_file().then_some(())?;
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let is_json = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let (def, script) = if is_json {
        let def: Definition = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        if def.command.trim().is_empty() {
            return None;
        }
        (def, None)
    } else if std::fs::metadata(path).ok()?.permissions().mode() & 0o111 != 0 {
        (Definition::default(), Some(path.to_path_buf()))
    } else {
        return None;
    };
    let name = if def.name.trim().is_empty() { stem } else { def.name.clone() };
    Some(Entry { label: Box::leak(name.into_boxed_str()), def, script })
}

impl Definition {
    pub fn applies(&self, selection: &[FileItem]) -> bool {
        if selection.is_empty() || (self.single && selection.len() != 1) {
            return false;
        }
        selection.iter().all(|item| {
            if self.dirs_only && !item.is_dir || self.files_only && item.is_dir {
                return false;
            }
            self.extensions.is_empty()
                || item.path.extension().is_some_and(|ext| self.extensions.iter().any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext.to_string_lossy())))
        })
    }
}

impl Plugin for Actions {
    fn id(&self) -> &'static str {
        "actions"
    }

    fn name(&self) -> &'static str {
        tr("Custom actions")
    }

    fn description(&self) -> &'static str {
        tr("Adds your scripts and commands from ~/.config/ski/actions to the context menu")
    }

    fn icon(&self) -> &'static str {
        icons::TERMINAL
    }

    fn menu_items(&self, selection: &[FileItem]) -> Vec<MenuItem> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.def.applies(selection))
            .map(|(i, e)| MenuItem { id: i.to_string(), label: e.label, icon: icons::TERMINAL, group: Some(tr("Actions")) })
            .collect()
    }

    fn run(&mut self, id: &str, paths: &[PathBuf], dir: &Path) {
        let Some(entry) = id.parse::<usize>().ok().and_then(|i| self.entries.get(i)) else { return };
        let mut command = match &entry.script {
            Some(script) => {
                let mut c = Command::new(script);
                c.args(paths);
                c
            }
            None => {
                let mut c = Command::new("sh");
                c.arg("-c").arg(&entry.def.command).arg("ski-action").args(paths);
                c
            }
        };
        let selected = paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n");
        command.current_dir(dir).env("SKI_SELECTED_FILES", selected).env("SKI_CURRENT_DIR", dir).stdin(std::process::Stdio::null());
        let (tx, label, running) = (self.done_tx.clone(), entry.label, Arc::clone(&self.running));
        running.fetch_add(1, Ordering::Relaxed);
        std::thread::spawn(move || {
            let message = match command.output() {
                Ok(out) if out.status.success() => trf("Finished: {}", &[&label]),
                Ok(out) => {
                    let error = String::from_utf8_lossy(&out.stderr);
                    let error = error.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned();
                    trf("{} failed: {}", &[&label, &error])
                }
                Err(e) => trf("{} failed: {}", &[&label, &e]),
            };
            let _ = tx.send(message);
            running.fetch_sub(1, Ordering::Relaxed);
        });
    }

    fn busy(&self) -> bool {
        self.running.load(Ordering::Relaxed) > 0
    }

    fn messages(&mut self) -> Vec<String> {
        self.done_rx.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, is_dir: bool) -> FileItem {
        let mut item = FileItem::from_path(Path::new("/")).unwrap();
        item.name = name.to_owned();
        item.path = PathBuf::from("/x").join(name);
        item.is_dir = is_dir;
        item
    }

    #[test]
    fn filters() {
        let webp = Definition { extensions: vec!["JPG".into(), ".png".into()], ..Default::default() };
        assert!(webp.applies(&[item("a.jpg", false), item("b.PNG", false)]));
        assert!(!webp.applies(&[item("a.jpg", false), item("b.txt", false)]));
        assert!(!webp.applies(&[]));
        let dirs = Definition { dirs_only: true, ..Default::default() };
        assert!(dirs.applies(&[item("d", true)]));
        assert!(!dirs.applies(&[item("f", false)]));
        let files = Definition { files_only: true, single: true, ..Default::default() };
        assert!(files.applies(&[item("f", false)]));
        assert!(!files.applies(&[item("f", false), item("g", false)]));
        assert!(!files.applies(&[item("d", true)]));
    }

    #[test]
    fn reads_json_and_scripts_and_runs_them() {
        let dir = std::env::temp_dir().join(format!("ski-actions-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.json"), r#"{"name":"List","command":"printf '%s\n' \"$@\" > out.txt; echo \"$SKI_CURRENT_DIR\" >> out.txt"}"#).unwrap();
        std::fs::write(dir.join("broken.json"), "not json").unwrap();
        std::fs::write(dir.join("notes.txt"), "not executable").unwrap();
        std::fs::write(dir.join("tool.sh"), "#!/bin/sh\ntrue\n").unwrap();
        std::fs::set_permissions(dir.join("tool.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let entries = read_dir(&dir);
        let names: Vec<_> = entries.iter().map(|e| e.label).collect();
        assert_eq!(names, ["List", "tool"]);

        let mut actions = Actions::load();
        actions.entries = entries;
        assert!(!actions.busy());
        actions.run("0", &[PathBuf::from("one"), PathBuf::from("two words")], &dir);
        let message = actions.done_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(message.contains("List"), "{message}");
        let out = std::fs::read_to_string(dir.join("out.txt")).unwrap();
        assert_eq!(out, format!("one\ntwo words\n{}\n", dir.display()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

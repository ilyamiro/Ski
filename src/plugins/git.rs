//! Git awareness: a badge on every changed file of a repository and the branch in the status line,
//! plus stage, unstage, discard and a diff window in the context menu. Every git command runs on
//! a worker thread, so a big repository never stalls the window.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;

use super::{Badge, MenuItem, Plugin, Reaction, Status, Tone, intern};
use crate::fs::file_item::FileItem;
use crate::i18n::{tr, trf, trn};
use crate::ui::icons;
use crate::ui::theme::SerpantinumTheme;

/// Reloads of the same folder closer together than this share one `git status`.
const THROTTLE: Duration = Duration::from_millis(400);

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Repo {
    pub root: PathBuf,
    pub branch: String,
    /// `[ahead 1, behind 2]` from `git status -b`, empty when in sync or without upstream.
    pub tracking: String,
    pub files: HashMap<PathBuf, Badge>,
    /// The two status letters (index, work tree) of each changed path, without the folders above it.
    pub states: HashMap<PathBuf, (char, char)>,
    pub changed: usize,
}

/// Which Git entries make sense for a selection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Applies {
    pub stage: bool,
    pub unstage: bool,
    pub discard: bool,
    pub diff: bool,
}

impl Repo {
    /// Reads the status letters of everything at or below the selected paths.
    pub fn applies(&self, selected: &[&Path]) -> Applies {
        let mut out = Applies::default();
        for (path, &(x, y)) in &self.states {
            if !selected.iter().any(|s| path.starts_with(s)) {
                continue;
            }
            out.stage |= y != ' ';
            out.unstage |= !matches!(x, ' ' | '?' | '!' | 'U') && y != 'U';
            out.discard |= matches!(y, 'M' | 'D') && x != 'U';
            out.diff |= x != '?';
        }
        // Git reports a folder with nothing tracked in it as one entry, so what is inside is untracked too
        out.stage |= selected.iter().any(|s| self.in_untracked_folder(s));
        // One diff window shows one thing
        out.diff &= selected.len() == 1;
        out
    }

    /// The paths with a status at or below them that satisfies `wanted`; git refuses a whole command
    /// when one path is not known to it.
    fn keep(&self, paths: &[PathBuf], wanted: impl Fn(char, char) -> bool) -> Vec<PathBuf> {
        paths.iter().filter(|p| self.states.iter().any(|(path, &(x, y))| path.starts_with(p) && wanted(x, y))).cloned().collect()
    }

    /// Whether a folder above `path` is listed as untracked as a whole.
    pub fn in_untracked_folder(&self, path: &Path) -> bool {
        path.ancestors().skip(1).take_while(|a| a.starts_with(&self.root) && *a != self.root).any(|a| self.states.get(a) == Some(&('?', '?')))
    }
}

/// What a Git menu entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Stage,
    Unstage,
    Discard,
    Diff,
}

impl Action {
    fn id(self) -> &'static str {
        match self {
            Action::Stage => "stage",
            Action::Unstage => "unstage",
            Action::Discard => "discard",
            Action::Diff => "diff",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        [Action::Stage, Action::Unstage, Action::Discard, Action::Diff].into_iter().find(|a| a.id() == id)
    }
}

/// The text of a diff, as lines to draw.
struct DiffView {
    title: String,
    lines: Vec<String>,
    /// Lines past `MAX_DIFF_LINES` were left out.
    cut: bool,
    binary: bool,
}

/// A diff window shows this many lines at most.
const MAX_DIFF_LINES: usize = 4000;

enum Done {
    /// A stage, unstage or discard finished: the message to show.
    Changed(Result<String, String>, Action),
    Diff(Result<DiffView, String>),
}

#[derive(Default)]
pub struct Git {
    repo: Option<Repo>,
    dir: Option<PathBuf>,
    asked: Option<Instant>,
    generation: u64,
    rx: Option<mpsc::Receiver<(u64, Option<Repo>)>>,
    /// Results of the commands started from the menu.
    done: Option<(mpsc::Sender<Done>, mpsc::Receiver<Done>)>,
    running: usize,
    diff: Option<DiffView>,
    messages: Vec<String>,
    refresh: bool,
}

impl Plugin for Git {
    fn id(&self) -> &'static str {
        "git"
    }

    fn name(&self) -> &'static str {
        tr("Git")
    }

    fn description(&self) -> &'static str {
        tr("Marks changed, staged and untracked files in repositories and shows the branch")
    }

    fn icon(&self) -> &'static str {
        icons::SYNC
    }

    fn directory_changed(&mut self, dir: &Path) {
        let same = self.dir.as_deref() == Some(dir);
        if same && self.asked.is_some_and(|t| t.elapsed() < THROTTLE) {
            return;
        }
        if !same {
            self.repo = None;
        }
        self.dir = Some(dir.to_path_buf());
        self.read_status();
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok((generation, repo)) if generation == self.generation => {
                    self.rx = None;
                    self.repo = repo;
                    changed = true;
                }
                Ok(_) | Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.rx = None,
            }
        }
        let results: Vec<Done> = self.done.as_ref().map(|(_, rx)| rx.try_iter().collect()).unwrap_or_default();
        for result in results {
            match result {
                Done::Changed(outcome, action) => {
                    self.running = self.running.saturating_sub(1);
                    // Restoring files puts back what the listing shows
                    self.refresh |= action == Action::Discard && outcome.is_ok();
                    self.messages.push(outcome.unwrap_or_else(|e| e));
                    // The badges are out of date now: ask again without waiting for the throttle
                    self.read_status();
                }
                Done::Diff(outcome) => {
                    self.running = self.running.saturating_sub(1);
                    match outcome {
                        Ok(view) => self.diff = Some(view),
                        Err(error) => self.messages.push(error),
                    }
                }
            }
            changed = true;
        }
        changed
    }

    fn busy(&self) -> bool {
        self.rx.is_some() || self.running > 0
    }

    fn menu_items(&self, selection: &[FileItem]) -> Vec<MenuItem> {
        let Some(repo) = self.repo.as_ref().filter(|r| !selection.is_empty() && selection.iter().all(|i| i.path.starts_with(&r.root))) else {
            return Vec::new();
        };
        let paths: Vec<&Path> = selection.iter().map(|i| i.path.as_path()).collect();
        let applies = repo.applies(&paths);
        let group = tr("Git");
        let mut items = Vec::new();
        if applies.stage {
            items.push(MenuItem::new(Action::Stage.id(), tr("Stage"), icons::ADD).group(group));
        }
        if applies.unstage {
            items.push(MenuItem::new(Action::Unstage.id(), tr("Unstage"), icons::REMOVE).group(group));
        }
        if applies.diff {
            items.push(MenuItem::new(Action::Diff.id(), tr("Show diff"), icons::CODE).group(group));
        }
        if applies.discard {
            let files = repo.states.iter().filter(|(p, (_, y))| matches!(y, 'M' | 'D') && paths.iter().any(|s| p.starts_with(s))).count();
            let question = intern(&trn(files, "Discard the changes to {} file? They can't be brought back.", "Discard the changes to {} files? They can't be brought back."));
            items.push(MenuItem::new(Action::Discard.id(), tr("Discard changes"), icons::UNDO).group(group).confirm(question));
        }
        items
    }

    fn run(&mut self, id: &str, paths: &[PathBuf], _dir: &Path) -> Reaction {
        let (Some(action), Some(root)) = (Action::from_id(id), self.repo.as_ref().map(|r| r.root.clone())) else { return Reaction::None };
        let tx = self.done.get_or_insert_with(mpsc::channel).0.clone();
        let mut paths = paths.to_vec();
        if let Some(repo) = &self.repo {
            match action {
                Action::Unstage => paths = repo.keep(&paths, |x, y| !matches!(x, ' ' | '?' | '!' | 'U') && y != 'U'),
                Action::Discard => paths = repo.keep(&paths, |x, y| matches!(y, 'M' | 'D') && x != 'U'),
                _ => {}
            }
        }
        if paths.is_empty() {
            return Reaction::None;
        }
        self.running += 1;
        std::thread::spawn(move || {
            let _ = tx.send(match action {
                Action::Diff => Done::Diff(diff(&root, &paths)),
                _ => Done::Changed(change(&root, action, &paths), action),
            });
        });
        Reaction::None
    }

    fn reaction(&mut self) -> Reaction {
        if std::mem::take(&mut self.refresh) { Reaction::Refresh } else { Reaction::None }
    }

    fn window_title(&self) -> Option<&'static str> {
        self.diff.as_ref().map(|_| tr("Changes"))
    }

    fn close_window(&mut self) {
        self.diff = None;
    }

    fn window(&mut self, ui: &mut egui::Ui, theme: &SerpantinumTheme) -> bool {
        let Some(view) = &self.diff else { return false };
        ui.label(egui::RichText::new(&view.title).size(crate::ui::material::FONT_CAPTION).color(theme.subtext1));
        if view.binary {
            ui.label(egui::RichText::new(tr("Binary file")).color(theme.subtext0));
            return true;
        }
        if view.lines.is_empty() {
            ui.label(egui::RichText::new(tr("No changes")).color(theme.subtext0));
            return true;
        }
        egui::ScrollArea::both().max_height(420.0).auto_shrink([false, true]).show_rows(ui, 15.0, view.lines.len(), |ui, rows| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for line in &view.lines[rows] {
                let color = match line.as_bytes().first() {
                    Some(b'+') if !line.starts_with("+++") => theme.green,
                    Some(b'-') if !line.starts_with("---") => theme.red,
                    Some(b'@') => theme.blue,
                    _ => theme.subtext0,
                };
                ui.label(egui::RichText::new(line).monospace().size(12.5).color(color));
            }
        });
        if view.cut {
            ui.label(egui::RichText::new(trf("Showing the first {} lines", &[&MAX_DIFF_LINES])).size(crate::ui::material::FONT_CAPTION).color(theme.subtext1));
        }
        true
    }

    fn messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    fn badge(&self, item: &FileItem) -> Option<Badge> {
        let repo = self.repo.as_ref()?;
        repo.files.get(&item.path).copied().or_else(|| repo.in_untracked_folder(&item.path).then_some(Badge { letter: '?', tone: Tone::Bad }))
    }

    fn status(&self) -> Option<Status> {
        let repo = self.repo.as_ref()?;
        let mut text = repo.branch.clone();
        if !repo.tracking.is_empty() {
            text.push_str(&format!(" {}", repo.tracking));
        }
        if repo.changed > 0 {
            text.push_str(&format!(" · {}", trf("{} changed", &[&repo.changed])));
        }
        Some(Status { icon: icons::SYNC, text })
    }
}

impl Git {
    /// Runs `git status` on a worker, bypassing the throttle.
    fn read_status(&mut self) {
        let Some(dir) = self.dir.clone() else { return };
        self.asked = Some(Instant::now());
        self.generation += 1;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let generation = self.generation;
        std::thread::spawn(move || {
            let _ = tx.send((generation, read_repo(&dir)));
        });
    }
}

/// Runs git in `root` with these arguments (never through a shell). On failure the last
/// non-empty line of stderr is the error, as for custom actions.
fn run_git(root: &Path, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
    let out = Command::new("git").arg("-C").arg(root).args(args).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let error = String::from_utf8_lossy(&out.stderr);
    Err(error.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("git failed").trim().to_owned())
}

fn os(text: &str) -> &std::ffi::OsStr {
    std::ffi::OsStr::new(text)
}

/// Whether the repository has a commit yet (`HEAD` resolves).
fn has_head(root: &Path) -> bool {
    run_git(root, &[os("rev-parse"), os("--verify"), os("--quiet"), os("HEAD")]).is_ok()
}

/// Stage, unstage or discard `paths`. The paths come after `--`, so a name starting with a dash
/// is still a path.
fn change(root: &Path, action: Action, paths: &[PathBuf]) -> Result<String, String> {
    let mut args: Vec<&std::ffi::OsStr> = match action {
        Action::Stage => vec![os("add"), os("--")],
        Action::Unstage if has_head(root) => vec![os("restore"), os("--staged"), os("--")],
        // Nothing to restore from before the first commit: take the files out of the index instead
        Action::Unstage => vec![os("rm"), os("--cached"), os("-r"), os("-q"), os("--")],
        Action::Discard => vec![os("restore"), os("--")],
        Action::Diff => return Err(String::new()),
    };
    args.extend(paths.iter().map(|p| p.as_os_str()));
    run_git(root, &args)?;
    Ok(match action {
        Action::Stage => trn(paths.len(), "Staged {} item", "Staged {} items"),
        Action::Unstage => trn(paths.len(), "Unstaged {} item", "Unstaged {} items"),
        _ => trn(paths.len(), "Discarded changes in {} item", "Discarded changes in {} items"),
    })
}

/// `git diff` of one path against the last commit (staged and unstaged together).
fn diff(root: &Path, paths: &[PathBuf]) -> Result<DiffView, String> {
    let path = paths.first().ok_or_else(String::new)?;
    let mut args: Vec<&std::ffi::OsStr> = vec![os("diff"), os("--no-color"), os("--no-ext-diff")];
    args.push(if has_head(root) { os("HEAD") } else { os("--cached") });
    args.push(os("--"));
    args.push(path.as_os_str());
    let text = String::from_utf8_lossy(&run_git(root, &args)?).into_owned();
    let binary = text.lines().any(|l| l.starts_with("Binary files ") || l == "GIT binary patch");
    let mut lines: Vec<String> = text.lines().take(MAX_DIFF_LINES + 1).map(str::to_owned).collect();
    let cut = lines.len() > MAX_DIFF_LINES;
    lines.truncate(MAX_DIFF_LINES);
    let name = path.strip_prefix(root).unwrap_or(path).display().to_string();
    Ok(DiffView { title: name, lines, cut, binary })
}

fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_OPTIONAL_LOCKS", "0").output().ok()?;
    out.status.success().then_some(out.stdout)
}

fn read_repo(dir: &Path) -> Option<Repo> {
    let top = PathBuf::from(String::from_utf8(git(dir, &["rev-parse", "--show-toplevel"])?).ok()?.trim());
    // Git names the real folder; items below a link keep the path they were reached by
    let root = dir.ancestors().find(|a| a.canonicalize().is_ok_and(|c| c == top)).map_or(top, Path::to_path_buf);
    let status = git(dir, &["status", "--porcelain=v1", "-z", "-b", "--untracked-files=normal"])?;
    Some(parse_status(&root, &String::from_utf8_lossy(&status)))
}

/// Parses `git status --porcelain=v1 -z -b`: a `## branch` entry, then `XY path` entries, where a
/// rename or copy is followed by one more entry holding the old name.
pub fn parse_status(root: &Path, text: &str) -> Repo {
    let mut repo = Repo { root: root.to_path_buf(), ..Repo::default() };
    let mut entries = text.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        if let Some(head) = entry.strip_prefix("## ") {
            let (branch, tracking) = split_branch(head);
            repo.branch = branch;
            repo.tracking = tracking;
            continue;
        }
        let mut chars = entry.chars();
        let (Some(x), Some(y)) = (chars.next(), chars.next()) else { continue };
        let Some(path) = entry.get(3..) else { continue };
        if matches!(x, 'R' | 'C') {
            entries.next();
        }
        let badge = match (x, y) {
            ('?', '?') => Badge { letter: '?', tone: Tone::Bad },
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => Badge { letter: '!', tone: Tone::Bad },
            (_, 'D') => Badge { letter: 'D', tone: Tone::Bad },
            (_, y) if y != ' ' => Badge { letter: 'M', tone: Tone::Warn },
            (x, _) => Badge { letter: if x == 'M' { 'S' } else { x }, tone: Tone::Good },
        };
        repo.changed += 1;
        let path = path.trim_end_matches('/');
        let mut full = root.join(path);
        repo.files.insert(full.clone(), badge);
        repo.states.insert(full.clone(), (x, y));
        // Every folder on the way shows that something inside it changed
        while full.pop() && full != root && full.starts_with(root) {
            repo.files.entry(full.clone()).or_insert(Badge { letter: 'M', tone: Tone::Warn });
        }
    }
    repo
}

/// `main...origin/main [ahead 1]` into `("main", "[ahead 1]")`; a detached head or a new
/// repository (`No commits yet on main`) is shown as it is.
fn split_branch(head: &str) -> (String, String) {
    if let Some(name) = head.strip_prefix("No commits yet on ") {
        return (name.to_owned(), String::new());
    }
    if head.starts_with("HEAD (no branch)") {
        return (tr("detached HEAD").to_owned(), String::new());
    }
    let (names, tracking) = match head.find(" [") {
        Some(at) => (&head[..at], head[at + 1..].to_owned()),
        None => (head, String::new()),
    };
    let branch = names.split("...").next().unwrap_or(names).to_owned();
    (branch, tracking)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_and_file_states() {
        let root = Path::new("/r");
        let text = "## main...origin/main [ahead 2]\0 M src/a.rs\0A  new.txt\0?? notes/\0R  to.rs\0from.rs\0UU merge.rs\0 D gone.rs\0";
        let repo = parse_status(root, text);
        assert_eq!(repo.branch, "main");
        assert_eq!(repo.tracking, "[ahead 2]");
        assert_eq!(repo.changed, 6);
        let letter = |p: &str| repo.files.get(Path::new(p)).map(|b| b.letter);
        assert_eq!(letter("/r/src/a.rs"), Some('M'));
        assert_eq!(letter("/r/src"), Some('M'));
        assert_eq!(letter("/r/new.txt"), Some('A'));
        assert_eq!(letter("/r/notes"), Some('?'));
        assert_eq!(letter("/r/to.rs"), Some('R'));
        assert_eq!(letter("/r/from.rs"), None);
        assert_eq!(letter("/r/merge.rs"), Some('!'));
        assert_eq!(letter("/r/gone.rs"), Some('D'));
        assert_eq!(letter("/r"), None);
    }

    #[test]
    fn files_in_an_untracked_folder_can_be_staged() {
        let repo = parse_status(Path::new("/r"), "## main\0?? notes/\0 M a.rs\0");
        assert!(repo.in_untracked_folder(Path::new("/r/notes/x.txt")));
        assert!(!repo.in_untracked_folder(Path::new("/r/notes")));
        assert!(!repo.in_untracked_folder(Path::new("/r/a.rs")));
        assert!(repo.applies(&[Path::new("/r/notes/x.txt")]).stage);
    }

    #[test]
    fn staged_and_edited_again_counts_as_modified() {
        let repo = parse_status(Path::new("/r"), "## dev\0MM a\0M  b\0");
        assert_eq!(repo.files[Path::new("/r/a")].tone, Tone::Warn);
        assert_eq!(repo.files[Path::new("/r/b")], Badge { letter: 'S', tone: Tone::Good });
        assert_eq!(repo.branch, "dev");
    }

    #[test]
    fn branch_without_upstream_or_commits() {
        assert_eq!(split_branch("feature/x"), ("feature/x".to_owned(), String::new()));
        assert_eq!(split_branch("No commits yet on main"), ("main".to_owned(), String::new()));
    }

    #[test]
    fn reads_a_real_repository() {
        let dir = std::env::temp_dir().join(format!("ski-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if git(&dir, &["init", "-q"]).is_none() {
            return;
        }
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let repo = read_repo(&dir).expect("repo");
        let root = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(repo.files.get(&root.join("a.txt")).map(|b| b.letter), Some('?'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A repository in a temp folder with `a.txt` committed, then edited, and `b.txt` untracked.
    fn sandbox(name: &str) -> Option<(PathBuf, PathBuf)> {
        let dir = std::env::temp_dir().join(format!("ski-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        git(&dir, &["init", "-q"])?;
        let root = std::fs::canonicalize(&dir).ok()?;
        std::fs::write(root.join("a.txt"), "one\n").ok()?;
        let commit = |args: &[&str]| {
            Command::new("git").arg("-C").arg(&root).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).output().ok().filter(|o| o.status.success())
        };
        commit(&["add", "a.txt"])?;
        commit(&["commit", "-q", "-m", "first"])?;
        std::fs::write(root.join("a.txt"), "one\nchanged\n").ok()?;
        std::fs::write(root.join("b.txt"), "new\n").ok()?;
        Some((dir, root))
    }

    fn state(root: &Path, name: &str) -> Option<(char, char)> {
        read_repo(root)?.states.get(&root.join(name)).copied()
    }

    #[test]
    fn stage_unstage_and_discard_change_the_repository() {
        let Some((dir, root)) = sandbox("actions") else { return };
        assert_eq!(state(&root, "a.txt"), Some((' ', 'M')));
        assert_eq!(state(&root, "b.txt"), Some(('?', '?')));

        change(&root, Action::Stage, &[root.join("a.txt"), root.join("b.txt")]).unwrap();
        assert_eq!(state(&root, "a.txt"), Some(('M', ' ')));
        assert_eq!(state(&root, "b.txt"), Some(('A', ' ')));

        change(&root, Action::Unstage, &[root.join("a.txt"), root.join("b.txt")]).unwrap();
        assert_eq!(state(&root, "a.txt"), Some((' ', 'M')));
        assert_eq!(state(&root, "b.txt"), Some(('?', '?')));

        let message = change(&root, Action::Discard, &[root.join("a.txt")]).unwrap();
        assert!(message.contains('1'), "{message}");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\n");
        assert_eq!(state(&root, "a.txt"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn names_that_look_like_options_are_still_paths() {
        let Some((dir, root)) = sandbox("dash") else { return };
        std::fs::write(root.join("--force"), "x").unwrap();
        change(&root, Action::Stage, &[root.join("--force")]).unwrap();
        assert_eq!(state(&root, "--force"), Some(('A', ' ')));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unstaging_before_the_first_commit_works_and_errors_say_why() {
        let dir = std::env::temp_dir().join(format!("ski-git-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if git(&dir, &["init", "-q"]).is_none() {
            return;
        }
        let root = std::fs::canonicalize(&dir).unwrap();
        std::fs::write(root.join("f"), "x").unwrap();
        change(&root, Action::Stage, &[root.join("f")]).unwrap();
        assert_eq!(state(&root, "f"), Some(('A', ' ')));
        change(&root, Action::Unstage, &[root.join("f")]).unwrap();
        assert_eq!(state(&root, "f"), Some(('?', '?')));
        // Outside a repository git's own last line is the message
        let outside = std::env::temp_dir().join(format!("ski-git-none-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        let error = change(&outside, Action::Stage, &[outside.join("x")]).unwrap_err();
        assert!(!error.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn diff_shows_changes_and_flags_binary_files() {
        let Some((dir, root)) = sandbox("diff") else { return };
        let view = diff(&root, &[root.join("a.txt")]).unwrap();
        assert!(view.lines.iter().any(|l| l == "+changed"), "{:?}", view.lines);
        assert!(!view.binary && !view.cut);
        // Staged changes are part of the diff too
        change(&root, Action::Stage, &[root.join("a.txt")]).unwrap();
        assert!(diff(&root, &[root.join("a.txt")]).unwrap().lines.iter().any(|l| l == "+changed"));

        std::fs::write(root.join("pic.bin"), [0u8, 159, 146, 150, 0]).unwrap();
        let commit = Command::new("git").arg("-C").arg(&root).args(["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-am", "x"]).output().unwrap();
        assert!(commit.status.success());
        change(&root, Action::Stage, &[root.join("pic.bin")]).unwrap();
        std::fs::write(root.join("pic.bin"), [0u8, 1, 2, 3, 0, 0]).unwrap();
        assert!(diff(&root, &[root.join("pic.bin")]).unwrap().binary);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn menu_entries_follow_the_state_of_the_selection() {
        let root = Path::new("/r");
        let repo = parse_status(root, "## main\0 M edited.rs\0M  staged.rs\0MM both.rs\0?? new.txt\0 D gone.rs\0");
        let applies = |names: &[&str]| {
            let paths: Vec<PathBuf> = names.iter().map(|n| root.join(n)).collect();
            repo.applies(&paths.iter().map(|p| p.as_path()).collect::<Vec<_>>())
        };
        assert_eq!(applies(&["edited.rs"]), Applies { stage: true, unstage: false, discard: true, diff: true });
        assert_eq!(applies(&["staged.rs"]), Applies { stage: false, unstage: true, discard: false, diff: true });
        assert_eq!(applies(&["both.rs"]), Applies { stage: true, unstage: true, discard: true, diff: true });
        // Untracked: can be staged, nothing to discard or compare
        assert_eq!(applies(&["new.txt"]), Applies { stage: true, unstage: false, discard: false, diff: false });
        assert!(applies(&["gone.rs"]).discard);
        // Several files: no diff window
        assert!(!applies(&["edited.rs", "both.rs"]).diff);
        // Clean files offer nothing
        assert_eq!(applies(&["clean.rs"]), Applies::default());
    }

    #[test]
    fn discard_asks_first_and_menu_actions_refresh_the_badges() {
        let Some((dir, root)) = sandbox("menu") else { return };
        let mut plugin = Git::default();
        plugin.directory_changed(&root);
        let wait = |plugin: &mut Git, until: &dyn Fn(&Git) -> bool| {
            for _ in 0..400 {
                plugin.poll();
                if until(plugin) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("timed out");
        };
        wait(&mut plugin, &|p| p.repo.is_some());

        let a = FileItem::from_path(&root.join("a.txt")).unwrap();
        let entries = plugin.menu_items(std::slice::from_ref(&a));
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["stage", "diff", "discard"]);
        let discard = entries.iter().find(|e| e.id == "discard").unwrap();
        assert!(discard.confirm.is_some_and(|q| q.contains('1')), "{:?}", discard.confirm);
        assert!(entries.iter().filter(|e| e.id != "discard").all(|e| e.confirm.is_none()));
        // Files outside the repository get nothing
        assert!(plugin.menu_items(&[FileItem::from_path(Path::new("/")).unwrap()]).is_empty());

        // Staging runs on a worker, reports, and the badge follows without waiting for the throttle
        plugin.run("stage", std::slice::from_ref(&a.path), &root);
        wait(&mut plugin, &|p| p.repo.as_ref().is_some_and(|r| r.states.get(&a.path) == Some(&('M', ' '))));
        let messages = plugin.messages();
        assert_eq!(messages.len(), 1, "{messages:?}");
        let ids: Vec<String> = plugin.menu_items(std::slice::from_ref(&a)).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["unstage", "diff"]);

        // The diff window opens with the changes and closes on request
        plugin.run("diff", std::slice::from_ref(&a.path), &root);
        wait(&mut plugin, &|p| p.diff.is_some());
        assert_eq!(plugin.window_title(), Some(tr("Changes")));
        plugin.close_window();
        assert_eq!(plugin.window_title(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

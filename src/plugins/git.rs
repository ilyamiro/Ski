//! Git awareness: a badge on every changed file of a repository and the branch in the status line.
//! `git status` runs on a worker thread, so a big repository never stalls the window.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{Badge, Plugin, Status, Tone};
use crate::fs::file_item::FileItem;
use crate::i18n::{tr, trf};
use crate::ui::icons;

/// Reloads of the same folder closer together than this share one `git status`.
const THROTTLE: Duration = Duration::from_millis(400);

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Repo {
    pub branch: String,
    /// `[ahead 1, behind 2]` from `git status -b`, empty when in sync or without upstream.
    pub tracking: String,
    pub files: HashMap<PathBuf, Badge>,
    pub changed: usize,
}

#[derive(Default)]
pub struct Git {
    repo: Option<Repo>,
    dir: Option<PathBuf>,
    asked: Option<Instant>,
    generation: u64,
    rx: Option<mpsc::Receiver<(u64, Option<Repo>)>>,
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
        self.asked = Some(Instant::now());
        self.generation += 1;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let (dir, generation) = (dir.to_path_buf(), self.generation);
        std::thread::spawn(move || {
            let _ = tx.send((generation, read_repo(&dir)));
        });
    }

    fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok((generation, repo)) if generation == self.generation => {
                self.rx = None;
                self.repo = repo;
                true
            }
            Ok(_) | Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                false
            }
        }
    }

    fn busy(&self) -> bool {
        self.rx.is_some()
    }

    fn badge(&self, item: &FileItem) -> Option<Badge> {
        self.repo.as_ref()?.files.get(&item.path).copied()
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

fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_OPTIONAL_LOCKS", "0").output().ok()?;
    out.status.success().then_some(out.stdout)
}

fn read_repo(dir: &Path) -> Option<Repo> {
    let root = PathBuf::from(String::from_utf8(git(dir, &["rev-parse", "--show-toplevel"])?).ok()?.trim());
    let status = git(dir, &["status", "--porcelain=v1", "-z", "-b", "--untracked-files=normal"])?;
    Some(parse_status(&root, &String::from_utf8_lossy(&status)))
}

/// Parses `git status --porcelain=v1 -z -b`: a `## branch` entry, then `XY path` entries, where a
/// rename or copy is followed by one more entry holding the old name.
pub fn parse_status(root: &Path, text: &str) -> Repo {
    let mut repo = Repo::default();
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
}

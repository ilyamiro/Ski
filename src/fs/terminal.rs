//! Open a terminal in a folder. Terminal emulators disagree on how to be told where to start, so the
//! common ones are listed with their flag; anything else starts in the working directory of the
//! process we launch, which most terminals use.
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// Name, and how its "start in this folder" option is spelled (`{}` is the folder; empty means
/// the terminal starts in the working directory of the process).
const TERMINALS: &[(&str, &[&str])] = &[
    ("ptyxis", &["--working-directory={}", "--new-window"]),
    ("kgx", &["--working-directory={}"]),
    ("gnome-terminal", &["--working-directory={}"]),
    ("konsole", &["--workdir", "{}"]),
    ("xfce4-terminal", &["--working-directory={}"]),
    ("kitty", &["--directory", "{}"]),
    ("alacritty", &["--working-directory", "{}"]),
    ("foot", &["--working-directory={}"]),
    ("wezterm", &["start", "--cwd", "{}"]),
    ("tilix", &["--working-directory={}"]),
    ("terminator", &["--working-directory={}"]),
    ("lxterminal", &["--working-directory={}"]),
    ("mate-terminal", &["--working-directory={}"]),
    ("urxvt", &[]),
    ("xterm", &[]),
    ("st", &[]),
    ("x-terminal-emulator", &[]),
];

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The terminal that would be started, for the settings hint.
pub fn detect(custom: &str) -> Option<String> {
    if let Some(program) = custom.split_whitespace().next() {
        return Some(program.to_owned());
    }
    if let Ok(env) = std::env::var("TERMINAL")
        && let Some(program) = env.split_whitespace().next()
        && on_path(program)
    {
        return Some(program.to_owned());
    }
    TERMINALS.iter().map(|(name, _)| *name).find(|name| on_path(name)).map(str::to_owned)
}

fn fill(arg: &str, dir: &Path) -> String {
    arg.replace("{}", &dir.to_string_lossy())
}

/// Start a terminal in `dir`. `custom` is the command from the settings: `{}` stands for the
/// folder, and without it the terminal is started inside the folder.
pub fn open_in(dir: &Path, custom: &str) -> io::Result<()> {
    let mut parts: Vec<String> = custom.split_whitespace().map(|p| fill(p, dir)).collect();
    if parts.is_empty() {
        let env = std::env::var("TERMINAL").ok().filter(|t| t.split_whitespace().next().is_some_and(on_path));
        let known = |program: &str| TERMINALS.iter().find(|(name, _)| *name == program).map(|(_, args)| *args);
        if let Some(env) = env {
            parts = env.split_whitespace().map(str::to_owned).collect();
            if let Some(args) = parts.first().and_then(|p| known(p)) {
                parts.extend(args.iter().map(|a| fill(a, dir)));
            }
        } else if let Some((name, args)) = TERMINALS.iter().find(|(name, _)| on_path(name)) {
            parts = std::iter::once((*name).to_owned()).chain(args.iter().map(|a| fill(a, dir))).collect();
        }
    }
    let Some((program, args)) = parts.split_first() else {
        return Err(io::Error::other(crate::i18n::tr("No terminal found. Set one in Settings → Behavior")));
    };
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_placeholder_is_filled() {
        assert_eq!(fill("--working-directory={}", Path::new("/a b")), "--working-directory=/a b");
        assert_eq!(detect("mycterm --flag").as_deref(), Some("mycterm"));
    }
}

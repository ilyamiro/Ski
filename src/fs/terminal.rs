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

/// How each terminal is told to run a command instead of a shell: the arguments before the command.
const RUNS: &[(&str, &[&str])] = &[
    ("ptyxis", &["--new-window", "--"]),
    ("kgx", &["--"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("foot", &[]),
    ("wezterm", &["start", "--"]),
    ("tilix", &["-e"]),
    ("terminator", &["-x"]),
    ("mate-terminal", &["-x"]),
    ("urxvt", &["-e"]),
    ("xterm", &["-e"]),
    ("st", &["-e"]),
    ("x-terminal-emulator", &["-e"]),
];

/// Start a terminal that runs `command`: the one of the settings, of `$TERMINAL`, or the first one
/// installed. `None` when that terminal is not known to take a command.
pub fn run(command: &[String], custom: &str) -> Option<io::Result<()>> {
    let env = std::env::var("TERMINAL").ok().and_then(|t| t.split_whitespace().next().map(str::to_owned)).filter(|t| on_path(t));
    let chosen = custom
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .or(env)
        .or_else(|| RUNS.iter().map(|(name, _)| *name).find(|name| on_path(name)).map(str::to_owned))?;
    let name = Path::new(&chosen).file_name()?.to_string_lossy().into_owned();
    let (_, before) = RUNS.iter().find(|(known, _)| *known == name)?;
    Some(
        Command::new(&chosen)
            .args(*before)
            .args(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ()),
    )
}

/// The ssh command that logs in to `host` and opens a shell in `dir` there.
pub fn ssh_command(user: Option<&str>, host: &str, port: Option<u16>, dir: &str) -> Vec<String> {
    let mut command = vec!["ssh".to_owned(), "-t".to_owned()];
    if let Some(port) = port {
        command.extend(["-p".to_owned(), port.to_string()]);
    }
    command.push(match user {
        Some(user) => format!("{user}@{host}"),
        None => host.to_owned(),
    });
    let quoted = format!("'{}'", dir.replace('\'', "'\\''"));
    command.push(format!("cd {quoted} 2>/dev/null; exec \"$SHELL\" -l"));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_placeholder_is_filled() {
        assert_eq!(fill("--working-directory={}", Path::new("/a b")), "--working-directory=/a b");
        assert_eq!(detect("mycterm --flag").as_deref(), Some("mycterm"));
    }

    #[test]
    fn ssh_opens_the_folder_on_the_server() {
        let command = ssh_command(Some("me"), "example.com", Some(2222), "/srv/it's here");
        assert_eq!(command[..5], ["ssh", "-t", "-p", "2222", "me@example.com"]);
        assert_eq!(command[5], "cd '/srv/it'\\''s here' 2>/dev/null; exec \"$SHELL\" -l");
    }
}

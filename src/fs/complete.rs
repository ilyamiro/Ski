//! Folder-name completion for the address bar.

use std::path::Path;

const MAX_SUGGESTIONS: usize = 50;

/// The folders whose names continue what has been typed, as full paths in the typed spelling.
/// Hidden folders only show up once the typed name starts with a dot.
pub fn dir_completions(input: &str) -> Vec<String> {
    let Some(slash) = input.rfind('/') else { return Vec::new() };
    let (head, prefix) = input.split_at(slash + 1);
    let dir = if head == "/" { "/".to_owned() } else { expand_tilde(&head[..slash]) };
    let Ok(entries) = std::fs::read_dir(Path::new(&dir)) else { return Vec::new() };
    let prefix_lower = prefix.to_lowercase();
    let show_hidden = prefix.starts_with('.');

    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if (!show_hidden && name.starts_with('.')) || !name.to_lowercase().starts_with(&prefix_lower) {
                return None;
            }
            let is_dir = match entry.file_type() {
                Ok(t) if t.is_symlink() => entry.path().is_dir(),
                Ok(t) => t.is_dir(),
                Err(_) => false,
            };
            is_dir.then_some(name)
        })
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.truncate(MAX_SUGGESTIONS);
    names.into_iter().map(|n| format!("{head}{n}")).collect()
}

/// `~` at the start of a typed path stands for the home folder.
pub fn expand_tilde(input: &str) -> String {
    let rest = match input.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => rest,
        _ => return input.to_owned(),
    };
    match dirs::home_dir() {
        Some(home) => format!("{}{rest}", home.to_string_lossy()),
        None => input.to_owned(),
    }
}

/// What Tab turns the input into: the longest start shared by all suggestions, and a trailing
/// slash when only one folder is left.
pub fn extend(input: &str, suggestions: &[String]) -> Option<String> {
    let first = suggestions.first()?;
    if suggestions.len() == 1 {
        return Some(format!("{first}/"));
    }
    let mut common = first.len();
    for s in &suggestions[1..] {
        common = first.chars().zip(s.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a.len_utf8()).sum::<usize>().min(common);
    }
    let common = &first[..common];
    (common.len() > input.len()).then(|| common.to_owned())
}

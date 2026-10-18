//! Recently used paths, so searches can look at files the user touched lately before walking
//! the whole disk. Combines Ski's own history with the desktop-wide `recently-used.xbel`.
use std::path::PathBuf;

const HISTORY_LIMIT: usize = 200;

fn history_file() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("ski").join("recent.json"))
}

fn load_history() -> Vec<PathBuf> {
    history_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Remember a file that was opened or a folder that was entered (most recent first).
pub fn record(path: &std::path::Path) {
    let Some(file) = history_file() else { return };
    let mut history = load_history();
    history.retain(|p| p != path);
    history.insert(0, path.to_path_buf());
    history.truncate(HISTORY_LIMIT);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(&history) {
        let _ = std::fs::write(file, json);
    }
}

/// Most recently used first: Ski's history, then everything other apps registered.
pub fn load() -> Vec<PathBuf> {
    let mut all = load_history();
    let mut shared = dirs::data_dir()
        .and_then(|d| std::fs::read_to_string(d.join("recently-used.xbel")).ok())
        .map(|xml| parse_xbel(&xml))
        .unwrap_or_default();
    // ISO timestamps sort lexicographically
    shared.sort_by(|a, b| b.0.cmp(&a.0));
    all.extend(shared.into_iter().map(|(_, p)| p));
    let mut seen = std::collections::HashSet::new();
    all.retain(|p| seen.insert(p.clone()));
    all
}

fn parse_xbel(xml: &str) -> Vec<(String, PathBuf)> {
    xml.lines()
        .filter_map(|line| {
            let href = attr(line, "href")?;
            let path = percent_decode(href.strip_prefix("file://")?);
            let stamp = attr(line, "visited").or_else(|| attr(line, "modified")).unwrap_or_default();
            Some((stamp.to_owned(), PathBuf::from(path)))
        })
        .collect()
}

fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name}=\"");
    let start = line.find(&marker)? + marker.len();
    let len = line[start..].find('"')?;
    Some(&line[start..start + len])
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

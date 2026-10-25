//! Recently used paths, so searches can look at files the user touched lately before walking
//! the whole disk. Combines Ski's own history with the desktop-wide `recently-used.xbel`.
use std::path::PathBuf;

const HISTORY_LIMIT: usize = 200;

fn history_file() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("ski").join("recent.json"))
}

fn shared_file() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("recently-used.xbel"))
}

/// The files the Recent view is made from: Ski's own history and the desktop-wide list.
pub fn sources() -> Vec<PathBuf> {
    history_file().into_iter().chain(shared_file()).collect()
}

static LAST_OWN_WRITE: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Whether Ski itself wrote the history a moment ago, as opposed to another window or app: the Recent
/// view stays put when its own click is what changed it.
pub fn written_here_lately() -> bool {
    LAST_OWN_WRITE.lock().ok().and_then(|t| *t).is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Entry {
    path: PathBuf,
    /// Seconds since the epoch.
    at: u64,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Ski's own history, newest first. Files written before entries carried times get the
/// file's modification time, one second apart, so they still interleave sensibly with other apps.
fn load_history() -> Vec<Entry> {
    let Some(file) = history_file() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(&file) else { return Vec::new() };
    if let Ok(entries) = serde_json::from_str::<Vec<Entry>>(&text) {
        return entries;
    }
    let base = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    serde_json::from_str::<Vec<PathBuf>>(&text)
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, path)| Entry { path, at: base.saturating_sub(i as u64) })
        .collect()
}

/// Remember a file that was opened or a folder that was entered (most recent first).
pub fn record(path: &std::path::Path) {
    let Some(file) = history_file() else { return };
    if let Ok(mut last) = LAST_OWN_WRITE.lock() {
        *last = Some(std::time::Instant::now());
    }
    let mut history = load_history();
    history.retain(|e| e.path != path);
    history.insert(0, Entry { path: path.to_path_buf(), at: now() });
    history.truncate(HISTORY_LIMIT);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(&history) {
        let _ = std::fs::write(file, json);
    }
}

/// Most recently used first, by when each path was last used in Ski or any other app.
pub fn load() -> Vec<PathBuf> {
    let history = load_history().into_iter().map(|e| (e.at, e.path)).collect();
    let shared = dirs::data_dir()
        .and_then(|_| shared_file())
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|xml| parse_xbel(&xml))
        .unwrap_or_default();
    merge(history, shared)
}

/// Both sources newest first (Ski's wins a tie), each path once.
fn merge(mut all: Vec<(u64, PathBuf)>, shared: Vec<(u64, PathBuf)>) -> Vec<PathBuf> {
    all.extend(shared);
    all.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut seen = std::collections::HashSet::new();
    all.into_iter().map(|(_, p)| p).filter(|p| seen.insert(p.clone())).collect()
}

fn parse_xbel(xml: &str) -> Vec<(u64, PathBuf)> {
    xml.lines()
        .filter_map(|line| {
            let href = attr(line, "href")?;
            let path = percent_decode(href.strip_prefix("file://")?);
            let stamp = attr(line, "visited").or_else(|| attr(line, "modified")).map_or(0, iso_seconds);
            Some((stamp, PathBuf::from(path)))
        })
        .collect()
}

/// Seconds since the epoch of a UTC `2024-05-17T09:30:00.123Z` stamp; 0 if it can't be read.
fn iso_seconds(stamp: &str) -> u64 {
    let num = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    let parse = || {
        let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
        let (h, min, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
        // Days since 1970-01-01 from the civil date (Howard Hinnant's algorithm)
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
        u64::try_from(days * 86400 + h * 3600 + min * 60 + s).ok()
    };
    parse().unwrap_or(0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_become_epoch_seconds() {
        assert_eq!(iso_seconds("1970-01-02T00:00:01.5Z"), 86401);
        assert_eq!(iso_seconds("2024-03-01T00:00:00Z"), 1709251200);
        assert_eq!(iso_seconds("junk"), 0);
    }

    #[test]
    fn a_write_by_ski_counts_as_ours_only_for_a_moment() {
        *LAST_OWN_WRITE.lock().unwrap() = None;
        assert!(!written_here_lately());
        *LAST_OWN_WRITE.lock().unwrap() = Some(std::time::Instant::now());
        assert!(written_here_lately());
        *LAST_OWN_WRITE.lock().unwrap() = Some(std::time::Instant::now() - std::time::Duration::from_secs(10));
        assert!(!written_here_lately());
        assert!(sources().iter().all(|s| s.is_absolute()));
    }

    #[test]
    fn newest_use_wins_across_sources() {
        let p = |s: &str| PathBuf::from(s);
        let merged = merge(vec![(100, p("/old-in-ski")), (500, p("/both"))], vec![(900, p("/in-gimp")), (400, p("/both"))]);
        assert_eq!(merged, [p("/in-gimp"), p("/both"), p("/old-in-ski")]);
    }
}

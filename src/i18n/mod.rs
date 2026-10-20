//! Interface translations. The English text is the key: `tr("Open")` returns the English string
//! unchanged, or its translation in the language chosen in Settings. Anything a catalog does not
//! cover falls back to English, so a missing entry never breaks the interface.
//!
//! * `tr(key)` – a plain label.
//! * `trf(key, args)` – a label with `{}` (next argument) or `{0}`, `{1}` (explicit argument) holes.
//! * `trn(n, one, other)` – a count: `trn(3, "{} item", "{} items")`. Catalogs store the plural
//!   forms of the `other` text separated by `|`, in the order of the language's plural rule.
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

mod de;
mod es;
mod fr;
mod ru;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Russian,
    German,
    Spanish,
    French,
}

type Catalog = &'static [(&'static str, &'static str)];

impl Language {
    pub const ALL: [Language; 5] = [Language::English, Language::Russian, Language::German, Language::Spanish, Language::French];

    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Russian => "ru",
            Language::German => "de",
            Language::Spanish => "es",
            Language::French => "fr",
        }
    }

    /// The language's own name, as it is listed in Settings.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Russian => "Русский",
            Language::German => "Deutsch",
            Language::Spanish => "Español",
            Language::French => "Français",
        }
    }

    pub fn from_code(code: &str) -> Option<Language> {
        let code = code.trim().to_ascii_lowercase();
        let code = code.split(['_', '-', '.', '@']).next().unwrap_or("");
        Language::ALL.into_iter().find(|l| l.code() == code)
    }

    fn index(self) -> usize {
        Language::ALL.iter().position(|l| *l == self).unwrap_or(0)
    }

    fn catalog(self) -> Catalog {
        match self {
            Language::English => &[],
            Language::Russian => ru::CATALOG,
            Language::German => de::CATALOG,
            Language::Spanish => es::CATALOG,
            Language::French => fr::CATALOG,
        }
    }

    /// Index into the `|`-separated plural forms for a count of `n`.
    fn plural_form(self, n: usize) -> usize {
        match self {
            Language::English | Language::German | Language::Spanish => usize::from(n != 1),
            Language::French => usize::from(n > 1),
            Language::Russian => {
                if n % 10 == 1 && n % 100 != 11 {
                    0
                } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
                    1
                } else {
                    2
                }
            }
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_language(language: Language) {
    CURRENT.store(language.index() as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    Language::ALL[(CURRENT.load(Ordering::Relaxed) as usize).min(Language::ALL.len() - 1)]
}

/// The language of the desktop session (`LC_ALL`, `LC_MESSAGES`, `LANG`), English when unknown.
pub fn system_language() -> Language {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .and_then(|v| Language::from_code(&v))
        .unwrap_or(Language::English)
}

/// The language a settings value stands for: `system` follows the desktop, otherwise a code.
pub fn resolve(setting: &str) -> Language {
    if setting == "system" { system_language() } else { Language::from_code(setting).unwrap_or(Language::English) }
}

fn tables() -> &'static [HashMap<&'static str, &'static str>] {
    static TABLES: OnceLock<Vec<HashMap<&'static str, &'static str>>> = OnceLock::new();
    TABLES.get_or_init(|| Language::ALL.iter().map(|l| l.catalog().iter().copied().collect()).collect())
}

fn lookup(key: &'static str) -> Option<&'static str> {
    match language() {
        Language::English => None,
        lang => tables()[lang.index()].get(key).copied().filter(|t| !t.is_empty()),
    }
}

/// Texts that reach `tr` through a variable or a table (place names, categories, batch-rename
/// chips), so the source scan cannot see them. They still need an entry in every catalog.
#[cfg(test)]
pub const INDIRECT: &[&str] = &[
    "Home", "Recent", "Trash", "Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos", "Computer",
    "Images", "Video", "Text", "Archives", "System", "Light",
    "Original name", "Modified date", "Parent folder", "Extension",
    "Keep", "lowercase", "UPPERCASE", "Title Case", "Sentence case", "Remove", "As listed", "By name", "By date",
];

/// Translate a text that is only known at run time (a place name stored as English). Unknown
/// texts, such as the name of a drive, come back unchanged.
pub fn tr_dyn(text: &str) -> String {
    match language() {
        Language::English => text.to_owned(),
        lang => tables()[lang.index()].get(text).copied().filter(|t| !t.is_empty()).unwrap_or(text).to_owned(),
    }
}

/// Translate a label.
pub fn tr(key: &'static str) -> &'static str {
    lookup(key).unwrap_or(key)
}

/// Fill the holes of a (translated) template: `{}` takes the next argument, `{n}` the n-th.
pub fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) if after[..end].chars().all(|c| c.is_ascii_digit()) => {
                let index = if end == 0 {
                    next += 1;
                    next - 1
                } else {
                    after[..end].parse().unwrap_or(0)
                };
                if let Some(arg) = args.get(index) {
                    out.push_str(&arg.to_string());
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Translate a label that has holes and fill them.
pub fn trf(key: &'static str, args: &[&dyn Display]) -> String {
    fill(tr(key), args)
}

/// A count with the right plural form: `trn(n, "{} item", "{} items")`.
pub fn trn(n: usize, one: &'static str, other: &'static str) -> String {
    let lang = language();
    let template = match lookup(other) {
        Some(forms) => {
            let forms: Vec<&str> = forms.split('|').collect();
            forms.get(lang.plural_form(n)).or(forms.last()).copied().unwrap_or(other)
        }
        None if n == 1 => one,
        None => other,
    };
    fill(template, &[&n])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tr("…")`, `trf("…"` and `trn(n, "…", "…")` key written in the sources.
    fn keys_in_sources() -> Vec<(String, bool)> {
        fn string_at(src: &str, quote: usize) -> Option<(String, usize)> {
            let bytes = src.as_bytes();
            let (mut out, mut i) = (String::new(), quote + 1);
            while i < bytes.len() {
                match bytes[i] {
                    b'"' => return Some((out, i + 1)),
                    b'\\' => {
                        let c = *bytes.get(i + 1)?;
                        out.push(match c { b'n' => '\n', b't' => '\t', other => other as char });
                        i += 2;
                    }
                    _ => {
                        let ch = src[i..].chars().next()?;
                        out.push(ch);
                        i += ch.len_utf8();
                    }
                }
            }
            None
        }
        fn walk(dir: &std::path::Path, out: &mut Vec<(String, bool)>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == "i18n") {
                        continue;
                    }
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let src = std::fs::read_to_string(&path).unwrap();
                    let src = src.split("#[cfg(test)]").next().unwrap();
                    for (call, is_plural) in [("tr(\"", false), ("trf(\"", false), ("trn(", true)] {
                        for (at, _) in src.match_indices(call) {
                            if src[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                                continue;
                            }
                            if is_plural {
                                // trn(count, "one", "other")
                                let Some(first) = src[at..].find('"').map(|p| at + p) else { continue };
                                let Some((_, after_one)) = string_at(src, first) else { continue };
                                let Some(second) = src[after_one..].find('"').map(|p| after_one + p) else { continue };
                                if let Some((key, _)) = string_at(src, second) {
                                    out.push((key, true));
                                }
                            } else if let Some((key, _)) = string_at(src, at + call.len() - 1) {
                                out.push((key, false));
                            }
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
        out.extend(INDIRECT.iter().map(|k| (k.to_string(), false)));
        out
    }

    fn holes(s: &str) -> usize {
        s.matches('{').count()
    }

    #[test]
    fn catalogs_cover_every_key() {
        let keys = keys_in_sources();
        assert!(keys.len() > 100, "found only {} keys; the scanner is broken", keys.len());
        let mut missing = Vec::new();
        for lang in Language::ALL.into_iter().filter(|l| *l != Language::English) {
            let table: HashMap<_, _> = lang.catalog().iter().copied().collect();
            for (key, plural) in &keys {
                match table.get(key.as_str()) {
                    None => missing.push(format!("{}: {key:?}", lang.code())),
                    Some(t) if *plural && t.split('|').count() < if lang == Language::Russian { 3 } else { 2 } => {
                        missing.push(format!("{}: plural forms of {key:?}", lang.code()))
                    }
                    Some(t) if !*plural && holes(t) != holes(key) => missing.push(format!("{}: holes of {key:?}", lang.code())),
                    _ => {}
                }
            }
        }
        assert!(missing.is_empty(), "{} problems:\n{}", missing.len(), missing.join("\n"));
    }

    #[test]
    fn catalogs_have_no_stale_or_duplicate_entries() {
        let keys: std::collections::HashSet<String> = keys_in_sources().into_iter().map(|(k, _)| k).collect();
        for lang in Language::ALL.into_iter().filter(|l| *l != Language::English) {
            let mut seen = std::collections::HashSet::new();
            for (key, _) in lang.catalog() {
                assert!(seen.insert(*key), "{}: duplicate {key:?}", lang.code());
                assert!(keys.contains(*key), "{}: unused entry {key:?}", lang.code());
            }
        }
    }

    #[test]
    fn fill_and_plurals() {
        assert_eq!(fill("{} of {}", &[&1, &2]), "1 of 2");
        assert_eq!(fill("{1} / {0}", &[&"a", &"b"]), "b / a");
        assert_eq!(fill("no holes {x}", &[]), "no holes {x}");
        assert_eq!(Language::Russian.plural_form(1), 0);
        assert_eq!(Language::Russian.plural_form(3), 1);
        assert_eq!(Language::Russian.plural_form(11), 2);
        assert_eq!(Language::Russian.plural_form(21), 0);
        assert_eq!(Language::French.plural_form(0), 0);
        assert_eq!(Language::from_code("ru_RU.UTF-8"), Some(Language::Russian));
    }
}

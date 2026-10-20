//! Name patterns for "select matching" and search: `*` matches any run of characters, `?` one
//! character, and several patterns can be given separated by commas or semicolons. A pattern
//! without wildcards matches names that contain it.

pub fn matches(patterns: &str, name: &str, case_sensitive: bool) -> bool {
    let (name, fold) = if case_sensitive { (name.to_owned(), false) } else { (name.to_lowercase(), true) };
    patterns
        .split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .any(|p| {
            let p = if fold { p.to_lowercase() } else { p.to_owned() };
            if p.contains(['*', '?']) { wildcard(&p, &name) } else { name.contains(&p) }
        })
}

fn wildcard(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn wildcards_lists_and_plain_text() {
        assert!(matches("*.png", "a.PNG", false));
        assert!(!matches("*.png", "a.PNG", true));
        assert!(matches("*.png, *.jpg", "b.jpg", true));
        assert!(matches("report?.txt", "report1.txt", true));
        assert!(!matches("report?.txt", "report12.txt", true));
        assert!(matches("holiday", "My Holiday 2024.mov", false), "plain text matches anywhere");
        assert!(matches("a*b*c", "aXXbYYc", true));
        assert!(!matches("a*b*c", "aXXbYY", true));
        assert!(!matches("  ,; ", "anything", true), "empty patterns match nothing");
    }
}

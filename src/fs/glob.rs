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
            if p.contains(['*', '?', '[']) { wildcard(&p, &name) } else { name.contains(&p) }
        })
}

fn wildcard(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        let step = match p.get(pi) {
            Some('?') => Some(1),
            Some('[') => class_len(&p[pi..], t[ti]),
            Some(&c) if c == t[ti] && c != '*' => Some(1),
            _ => None,
        };
        if let Some(step) = step {
            pi += step;
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

/// Length of the `[...]` class at the start of `p` if it matches `c`; a class never closed is the literal `[`.
fn class_len(p: &[char], c: char) -> Option<usize> {
    let Some(end) = p.iter().skip(2).position(|&x| x == ']').map(|i| i + 2) else {
        return (c == '[').then_some(1);
    };
    let (negated, body) = match p[1] {
        '!' | '^' => (true, &p[2..end]),
        _ => (false, &p[1..end]),
    };
    let mut hit = false;
    let mut i = 0;
    while i < body.len() {
        if i + 2 < body.len() && body[i + 1] == '-' {
            hit |= (body[i]..=body[i + 2]).contains(&c);
            i += 3;
        } else {
            hit |= body[i] == c;
            i += 1;
        }
    }
    (hit != negated).then_some(end + 1)
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
        assert!(matches("*.[pP][nN][gG]", "a.PNG", true));
        assert!(matches("*.[a-c]x", "f.bx", true));
        assert!(!matches("*.[!a-c]x", "f.bx", true));
        assert!(!matches("*.[pP][nN][gG]", "a.jpg", true));
        assert!(!matches("  ,; ", "anything", true), "empty patterns match nothing");
    }
}

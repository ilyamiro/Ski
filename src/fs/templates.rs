//! File templates for "New file": a few built-in ones, and every file found in the templates
//! folder (the desktop's `Templates` folder unless the settings name another).
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::i18n::tr;
use crate::ui::icons;

#[derive(Debug, Clone)]
pub enum Source {
    Builtin { content: &'static str, executable: bool },
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Template {
    /// What the list shows.
    pub name: String,
    /// The name a new file starts with, extension included.
    pub file_name: String,
    pub icon: &'static str,
    pub source: Source,
}

impl Template {
    pub fn extension(&self) -> Option<&str> {
        self.file_name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()).map(|(_, ext)| ext)
    }
}

/// The folder user templates live in.
pub fn user_dir(custom: Option<&Path>) -> Option<PathBuf> {
    custom.map(Path::to_path_buf).or_else(dirs::template_dir).or_else(|| dirs::home_dir().map(|h| h.join("Templates")))
}

fn builtin() -> Vec<Template> {
    let make = |name: &str, file: &str, icon, content, executable| Template {
        name: name.to_owned(),
        file_name: file.to_owned(),
        icon,
        source: Source::Builtin { content, executable },
    };
    let untitled = tr("Untitled");
    vec![
        make(tr("Empty file"), untitled, icons::FILE, "", false),
        make(tr("Text document"), &format!("{untitled}.txt"), icons::TEXT, "", false),
        make(tr("Markdown document"), &format!("{untitled}.md"), icons::MARKDOWN, "# \n", false),
        make(tr("Shell script"), "script.sh", icons::SHELL, "#!/usr/bin/env bash\nset -euo pipefail\n\n", true),
        make(tr("Python script"), "script.py", icons::PYTHON, "#!/usr/bin/env python3\n\n\nif __name__ == \"__main__\":\n    pass\n", true),
        make(tr("HTML page"), "index.html", icons::HTML, "<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  <title></title>\n</head>\n<body>\n\n</body>\n</html>\n", false),
        make(tr("JSON file"), &format!("{untitled}.json"), icons::JSON, "{}\n", false),
    ]
}

/// Built-in templates first, then the files of the templates folder (one level of subfolders
/// is looked into), alphabetically.
pub fn load(custom: Option<&Path>) -> Vec<Template> {
    let mut all = builtin();
    let Some(dir) = user_dir(custom) else { return all };
    let mut found = Vec::new();
    collect(&dir, 0, &mut found);
    found.sort_by(|a, b| super::scanner::natural_cmp(&a.name, &b.name));
    all.extend(found);
    all
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<Template>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if file_name.starts_with('.') || out.len() >= 200 {
            continue;
        }
        if path.is_dir() {
            if depth < 1 {
                collect(&path, depth + 1, out);
            }
            continue;
        }
        let icon = crate::fs::file_item::specific_icon(&file_name).map_or(icons::FILE, |(icon, _)| icon);
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| file_name.clone());
        out.push(Template { name, file_name, icon, source: Source::File(path) });
    }
}

/// Write the template's content to the new file `target`, which must not exist yet.
pub fn create(template: &Template, target: &Path) -> io::Result<()> {
    match &template.source {
        Source::Builtin { content, executable } => {
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(target)?;
            file.write_all(content.as_bytes())?;
            if *executable {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
            }
            Ok(())
        }
        Source::File(from) => {
            // `copy` would overwrite, and a new file must never replace anything
            let mut src = std::fs::File::open(from)?;
            let mut dst = std::fs::OpenOptions::new().write(true).create_new(true).open(target)?;
            io::copy(&mut src, &mut dst)?;
            dst.set_permissions(src.metadata()?.permissions())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_and_user_templates_create_files() {
        let dir = std::env::temp_dir().join(format!("ski_tpl_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tpl/sub")).unwrap();
        std::fs::write(dir.join("tpl/Invoice.odt"), "odt").unwrap();
        std::fs::write(dir.join("tpl/sub/Letter.txt"), "dear").unwrap();
        std::fs::write(dir.join("tpl/.hidden"), "x").unwrap();

        let all = load(Some(&dir.join("tpl")));
        let names: Vec<&str> = all.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Invoice") && names.contains(&"Letter") && !names.iter().any(|n| n.contains("hidden")));
        assert_eq!(all[0].file_name, "Untitled");

        let sh = all.iter().find(|t| t.file_name == "script.sh").unwrap();
        create(sh, &dir.join("a.sh")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join("a.sh")).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(create(sh, &dir.join("a.sh")).is_err(), "an existing file is never replaced");

        let invoice = all.iter().find(|t| t.name == "Invoice").unwrap();
        create(invoice, &dir.join("new.odt")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("new.odt")).unwrap(), "odt");
        assert_eq!(invoice.extension(), Some("odt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

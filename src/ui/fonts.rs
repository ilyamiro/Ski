//! Interface font installation and discovery of the fonts installed on the system.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use eframe::egui;

use super::icons;

/// Install the interface font (the bundled Adwaita Sans unless `custom` points at a font file).
/// Adwaita Sans stays behind a custom font as a fallback for glyphs it lacks.
pub fn install(ctx: &egui::Context, custom: Option<&Path>) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "adwaita_sans".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!("../../assets/fonts/AdwaitaSans-Regular.ttf"))),
    );
    let proportional = fonts.families.entry(egui::FontFamily::Proportional).or_default();
    proportional.insert(0, "adwaita_sans".to_owned());

    if let Some(data) = custom.and_then(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("custom_ui".to_owned(), Arc::new(egui::FontData::from_owned(data)));
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "custom_ui".to_owned());
    }

    // Icons: the Symbols Nerd Font Mono Serpantinum pins, in a family of its own with no
    // fallback, so every icon comes from one font with identical metrics.
    fonts.font_data.insert(
        "nerd_symbols".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!("../../assets/fonts/SymbolsNerdFontMono-Regular.ttf"))),
    );
    fonts.families.insert(egui::FontFamily::Name(icons::FAMILY.into()), vec!["nerd_symbols".to_owned()]);

    ctx.set_fonts(fonts);
}

/// One installed font family and the file of its regular style.
#[derive(Debug, Clone)]
pub struct SystemFont {
    pub family: String,
    pub path: PathBuf,
}

/// Installed font families (via fontconfig), regular style only, sorted by name.
pub fn system_fonts() -> Vec<SystemFont> {
    let Some(output) = Command::new("fc-list")
        .args(["--format", "%{file}\t%{family[0]}\t%{style[0]}\n"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);

    let mut best: std::collections::HashMap<String, (u8, PathBuf)> = std::collections::HashMap::new();
    for line in text.lines() {
        let mut parts = line.split('\t');
        let (Some(file), Some(family), style) = (parts.next(), parts.next(), parts.next().unwrap_or("")) else { continue };
        let ext = Path::new(file).extension().map(|e| e.to_string_lossy().to_lowercase());
        if !matches!(ext.as_deref(), Some("ttf" | "otf")) || family.is_empty() {
            continue;
        }
        let style = style.to_lowercase();
        let score = match style.as_str() {
            "regular" | "book" | "roman" | "normal" => 0,
            "" => 1,
            s if s.contains("bold") || s.contains("italic") || s.contains("oblique") => 3,
            _ => 2,
        };
        let entry = best.entry(family.to_owned()).or_insert((u8::MAX, PathBuf::new()));
        if score < entry.0 {
            *entry = (score, PathBuf::from(file));
        }
    }
    let mut list: Vec<SystemFont> = best
        .into_iter()
        .filter(|(_, (score, _))| *score < 3)
        .map(|(family, (_, path))| SystemFont { family, path })
        .collect();
    list.sort_by_key(|f| f.family.to_lowercase());
    list
}

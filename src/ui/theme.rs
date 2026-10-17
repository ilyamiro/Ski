use std::fs;
use eframe::egui::Color32;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SerpantinumColorsRaw {
    pub base: Option<String>,
    pub mantle: Option<String>,
    pub crust: Option<String>,
    pub text: Option<String>,
    pub subtext0: Option<String>,
    pub subtext1: Option<String>,
    pub surface0: Option<String>,
    pub surface1: Option<String>,
    pub surface2: Option<String>,
    pub blue: Option<String>,
    pub mauve: Option<String>,
    pub peach: Option<String>,
    pub green: Option<String>,
    pub red: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct SerpantinumTheme {
    pub base: Color32,
    pub mantle: Color32,
    pub crust: Color32,
    pub text: Color32,
    pub subtext0: Color32,
    pub subtext1: Color32,
    pub surface0: Color32,
    pub surface1: Color32,
    pub surface2: Color32,
    pub blue: Color32,
    pub mauve: Color32,
    pub peach: Color32,
    pub green: Color32,
    pub red: Color32,
    pub border_radius: f32,
}

impl Default for SerpantinumTheme {
    fn default() -> Self {
        Self {
            base: parse_hex("#0f0d13").unwrap_or(Color32::from_rgb(15, 13, 19)),
            mantle: parse_hex("#1c1b20").unwrap_or(Color32::from_rgb(28, 27, 32)),
            crust: parse_hex("#141318").unwrap_or(Color32::from_rgb(20, 19, 24)),
            text: parse_hex("#e6e1e9").unwrap_or(Color32::from_rgb(230, 225, 233)),
            subtext0: parse_hex("#cac4cf").unwrap_or(Color32::from_rgb(202, 196, 207)),
            subtext1: parse_hex("#938f99").unwrap_or(Color32::from_rgb(147, 143, 153)),
            surface0: parse_hex("#201f24").unwrap_or(Color32::from_rgb(32, 31, 36)),
            surface1: parse_hex("#2b292f").unwrap_or(Color32::from_rgb(43, 41, 47)),
            surface2: parse_hex("#36343a").unwrap_or(Color32::from_rgb(54, 52, 58)),
            blue: parse_hex("#ccbeff").unwrap_or(Color32::from_rgb(204, 190, 255)),
            mauve: parse_hex("#ccbeff").unwrap_or(Color32::from_rgb(204, 190, 255)),
            peach: parse_hex("#eeb8cb").unwrap_or(Color32::from_rgb(238, 184, 203)),
            green: parse_hex("#cac3dc").unwrap_or(Color32::from_rgb(202, 195, 220)),
            red: parse_hex("#ffb4ab").unwrap_or(Color32::from_rgb(255, 180, 171)),
            border_radius: 16.0,
        }
    }
}

impl SerpantinumTheme {
    pub fn load() -> Self {
        let mut theme = Self::default();

        // 1. Try to load colors from ~/.local/state/serpantinum/qs_colors.json
        let candidate_paths = [
            dirs::home_dir().map(|h| h.join(".local/state/serpantinum/qs_colors.json")),
            dirs::home_dir().map(|h| h.join(".config/serpantinum/qs_colors.json")),
        ];

        for path in candidate_paths.into_iter().flatten() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(raw) = serde_json::from_str::<SerpantinumColorsRaw>(&content) {
                    if let Some(c) = raw.base.and_then(|s| parse_hex(&s)) { theme.base = c; }
                    if let Some(c) = raw.mantle.and_then(|s| parse_hex(&s)) { theme.mantle = c; }
                    if let Some(c) = raw.crust.and_then(|s| parse_hex(&s)) { theme.crust = c; }
                    if let Some(c) = raw.text.and_then(|s| parse_hex(&s)) { theme.text = c; }
                    if let Some(c) = raw.subtext0.and_then(|s| parse_hex(&s)) { theme.subtext0 = c; }
                    if let Some(c) = raw.subtext1.and_then(|s| parse_hex(&s)) { theme.subtext1 = c; }
                    if let Some(c) = raw.surface0.and_then(|s| parse_hex(&s)) { theme.surface0 = c; }
                    if let Some(c) = raw.surface1.and_then(|s| parse_hex(&s)) { theme.surface1 = c; }
                    if let Some(c) = raw.surface2.and_then(|s| parse_hex(&s)) { theme.surface2 = c; }
                    if let Some(c) = raw.blue.and_then(|s| parse_hex(&s)) { theme.blue = c; }
                    if let Some(c) = raw.mauve.and_then(|s| parse_hex(&s)) { theme.mauve = c; }
                    if let Some(c) = raw.peach.and_then(|s| parse_hex(&s)) { theme.peach = c; }
                    if let Some(c) = raw.green.and_then(|s| parse_hex(&s)) { theme.green = c; }
                    if let Some(c) = raw.red.and_then(|s| parse_hex(&s)) { theme.red = c; }
                    break;
                }
            }
        }

        // 2. Try to read borderRadius from ~/.config/serpantinum/settings.json
        if let Some(home) = dirs::home_dir() {
            let settings_path = home.join(".config/serpantinum/settings.json");
            if let Ok(content) = fs::read_to_string(settings_path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(r) = val.get("theme").and_then(|t| t.get("borderRadius")).and_then(|b| b.as_f64()) {
                        theme.border_radius = r as f32;
                    }
                }
            }
        }

        theme
    }

    /// Darken color by factor (e.g. 1.12 => divide RGB by 1.12)
    pub fn darker(&self, color: Color32, factor: f32) -> Color32 {
        let f = factor.max(0.1);
        Color32::from_rgba_unmultiplied(
            ((color.r() as f32) / f).clamp(0.0, 255.0) as u8,
            ((color.g() as f32) / f).clamp(0.0, 255.0) as u8,
            ((color.b() as f32) / f).clamp(0.0, 255.0) as u8,
            color.a(),
        )
    }

    /// Lighten color by factor (e.g. 1.12 => multiply RGB by 1.12)
    pub fn lighter(&self, color: Color32, factor: f32) -> Color32 {
        let f = factor.max(0.1);
        Color32::from_rgba_unmultiplied(
            ((color.r() as f32) * f).clamp(0.0, 255.0) as u8,
            ((color.g() as f32) * f).clamp(0.0, 255.0) as u8,
            ((color.b() as f32) * f).clamp(0.0, 255.0) as u8,
            color.a(),
        )
    }

    /// Semi-transparent color
    pub fn with_alpha(&self, color: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
    }
}

pub fn parse_hex(hex: &str) -> Option<Color32> {
    let clean = hex.trim().trim_start_matches('#');
    if clean.len() == 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        Some(Color32::from_rgb(r, g, b))
    } else if clean.len() == 8 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        let a = u8::from_str_radix(&clean[6..8], 16).ok()?;
        Some(Color32::from_rgba_unmultiplied(r, g, b, a))
    } else {
        None
    }
}

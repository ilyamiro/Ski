use std::fs;
use eframe::egui::{self, Color32};
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
    pub light: bool,
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
            light: false,
        }
    }
}

/// Named palettes: base, mantle, crust, text, subtext0, subtext1, surface0-2, accent, peach, green, red.
pub const PRESETS: [(&str, &str, [&str; 13], bool); 4] = [
    ("midnight", "Midnight", ["#0b0e14", "#151a23", "#10141b", "#dfe6f3", "#b4bfd3", "#7d889c", "#181e29", "#212938", "#2b3446", "#8ab4f8", "#f2b8a2", "#9bd3a8", "#f28b82"], false),
    ("nord", "Nord", ["#242933", "#2e3440", "#20242d", "#eceff4", "#d8dee9", "#8c95a6", "#343b49", "#3b4252", "#434c5e", "#88c0d0", "#d08770", "#a3be8c", "#bf616a"], false),
    ("gruvbox", "Gruvbox", ["#1d2021", "#282828", "#181a1b", "#ebdbb2", "#d5c4a1", "#928374", "#32302f", "#3c3836", "#504945", "#d3869b", "#fe8019", "#b8bb26", "#fb4934"], false),
    ("light", "Light", ["#fef7ff", "#f3edf7", "#ece6f0", "#1d1b20", "#49454f", "#79747e", "#f7f2fa", "#e7e0ec", "#ddd6e2", "#6750a4", "#7d5260", "#625b71", "#b3261e"], true),
];

/// Quick accent choices offered in the settings.
pub const ACCENTS: [&str; 9] = ["#ccbeff", "#8ab4f8", "#88c0d0", "#a3d9a5", "#f5d76e", "#ffb784", "#ff9aa2", "#e0a3ff", "#f28b82"];

impl SerpantinumTheme {
    /// Palette picked in the settings (`system` follows Serpantinum), with the accent applied.
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let mut theme = match PRESETS.iter().find(|(id, ..)| *id == settings.theme) {
            Some((_, _, c, light)) => {
                let col = |i: usize| parse_hex(c[i]).unwrap_or(Color32::GRAY);
                Self {
                    base: col(0), mantle: col(1), crust: col(2), text: col(3), subtext0: col(4), subtext1: col(5),
                    surface0: col(6), surface1: col(7), surface2: col(8), blue: col(9), mauve: col(9),
                    peach: col(10), green: col(11), red: col(12), light: *light,
                    ..Self::default()
                }
            }
            None => Self::load(),
        };
        if let Some(accent) = settings.accent.as_deref().and_then(parse_hex) {
            theme.mauve = accent;
            theme.blue = accent;
        }
        theme
    }

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

/// Flat Material look: no strokes anywhere, state is shown by fills only.
pub fn apply(ctx: &egui::Context, theme: &SerpantinumTheme) {
    let mut visuals = if theme.light { egui::Visuals::light() } else { egui::Visuals::dark() };
    visuals.window_corner_radius = egui::CornerRadius::same(24);
    visuals.menu_corner_radius = egui::CornerRadius::same(16);
    visuals.window_stroke = egui::Stroke::NONE;
    visuals.popup_shadow = egui::Shadow::NONE;
    visuals.window_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(if theme.light { 50 } else { 110 }),
    };
    let w = &mut visuals.widgets;
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.bg_stroke = egui::Stroke::NONE;
        state.fg_stroke.color = theme.text;
    }
    visuals.selection.stroke = egui::Stroke::NONE;
    visuals.selection.bg_fill = theme.with_alpha(theme.mauve, 90);
    visuals.panel_fill = theme.base;
    visuals.window_fill = theme.mantle;
    visuals.extreme_bg_color = theme.surface1;
    visuals.override_text_color = Some(theme.text);
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.interaction.selectable_labels = false;
        for (ts, size) in [
            (egui::TextStyle::Body, 14.0),
            (egui::TextStyle::Button, 14.0),
            (egui::TextStyle::Small, 12.0),
            (egui::TextStyle::Heading, 18.0),
        ] {
            if let Some(f) = style.text_styles.get_mut(&ts) {
                f.size = size;
            }
        }
    });
}

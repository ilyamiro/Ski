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
    pub yellow: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
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
    pub yellow: Color32,
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
            yellow: parse_hex("#f5d76e").unwrap_or(Color32::from_rgb(245, 215, 110)),
            border_radius: 16.0,
            light: false,
        }
    }
}

/// The `border_radius` of the unchanged Material look; the corner setting scales it.
pub const BASE_RADIUS: f32 = 16.0;

/// Named palettes: base, mantle, crust, text, subtext0, subtext1, surface0-2, accent, peach, green, red, yellow.
pub const PRESETS: [(&str, &str, [&str; 14], bool); 13] = [
    ("midnight", "Midnight", ["#0b0e14", "#151a23", "#10141b", "#dfe6f3", "#b4bfd3", "#7d889c", "#181e29", "#212938", "#2b3446", "#8ab4f8", "#f2b8a2", "#9bd3a8", "#f28b82", "#fdd663"], false),
    ("nord", "Nord", ["#242933", "#2e3440", "#20242d", "#eceff4", "#d8dee9", "#8c95a6", "#343b49", "#3b4252", "#434c5e", "#88c0d0", "#d08770", "#a3be8c", "#bf616a", "#ebcb8b"], false),
    ("gruvbox", "Gruvbox", ["#1d2021", "#282828", "#181a1b", "#ebdbb2", "#d5c4a1", "#928374", "#32302f", "#3c3836", "#504945", "#d3869b", "#fe8019", "#b8bb26", "#fb4934", "#fabd2f"], false),
    ("light", "Light", ["#fef7ff", "#f3edf7", "#ece6f0", "#1d1b20", "#49454f", "#79747e", "#f7f2fa", "#e7e0ec", "#ddd6e2", "#6750a4", "#7d5260", "#625b71", "#b3261e", "#7d5800"], true),
    ("catppuccin", "Catppuccin Mocha", ["#181825", "#1e1e2e", "#11111b", "#cdd6f4", "#bac2de", "#8087a2", "#313244", "#45475a", "#585b70", "#cba6f7", "#fab387", "#a6e3a1", "#f38ba8", "#f9e2af"], false),
    ("dracula", "Dracula", ["#1e1f29", "#282a36", "#191a21", "#f8f8f2", "#cfcfc2", "#8a93c0", "#313442", "#3b3f51", "#464b60", "#bd93f9", "#ffb86c", "#50fa7b", "#ff5555", "#f1fa8c"], false),
    ("tokyonight", "Tokyo Night", ["#16161e", "#1a1b26", "#13131a", "#c0caf5", "#a9b1d6", "#7a82a8", "#24283b", "#2f334d", "#3b4261", "#7aa2f7", "#ff9e64", "#9ece6a", "#f7768e", "#e0af68"], false),
    ("rosepine", "Rosé Pine", ["#191724", "#1f1d2e", "#16141f", "#e0def4", "#c4c1dc", "#908caa", "#26233a", "#403d52", "#524f67", "#c4a7e7", "#f6c177", "#9ccfd8", "#eb6f92", "#f6c177"], false),
    ("onedark", "One Dark", ["#1e2227", "#282c34", "#1b1f23", "#abb2bf", "#9aa1ad", "#7f8794", "#2f343e", "#3a404c", "#454c5a", "#61afef", "#d19a66", "#98c379", "#e06c75", "#e5c07b"], false),
    ("everforest", "Everforest", ["#272e33", "#2d353b", "#232a2e", "#d3c6aa", "#bfb49a", "#859289", "#343f44", "#3d484d", "#475258", "#a7c080", "#e69875", "#83c092", "#e67e80", "#dbbc7f"], false),
    ("solarized-dark", "Solarized Dark", ["#00212b", "#002b36", "#001e26", "#e1e5df", "#93a1a1", "#7a8b8d", "#073642", "#0f4452", "#1a5361", "#268bd2", "#e0733c", "#a1b32a", "#f0524f", "#b58900"], false),
    ("latte", "Catppuccin Latte", ["#eff1f5", "#e6e9ef", "#dce0e8", "#4c4f69", "#5c5f77", "#7c7f93", "#f2f4f8", "#d9dde6", "#ccd0da", "#8839ef", "#fe640b", "#40a02b", "#d20f39", "#df8e1d"], true),
    ("solarized-light", "Solarized Light", ["#fdf6e3", "#eee8d5", "#e6dfca", "#3b4a50", "#586e75", "#6f8184", "#f7f0dc", "#e3dcc6", "#d6cfb8", "#268bd2", "#cb4b16", "#859900", "#dc322f", "#857000"], true),
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
                    peach: col(10), green: col(11), red: col(12), yellow: col(13), light: *light,
                    ..Self::default()
                }
            }
            None => Self::load(),
        };
        if let Some(accent) = settings.accent.as_deref().and_then(parse_hex) {
            theme.mauve = accent;
            theme.blue = accent;
        }
        theme.border_radius = BASE_RADIUS * settings.corner_scale;
        theme.ensure_readable();
        theme
    }

    /// Nudges text and accent colors that are too faint on this palette's backgrounds toward the
    /// strongest end (white on dark palettes, black on light ones). Colors that already read well
    /// are left as the palette defines them.
    fn ensure_readable(&mut self) {
        let all = [self.base, self.mantle, self.crust, self.surface0, self.surface1, self.surface2];
        // Secondary text and accents sit on the panels and rows, not on the lightest hover fill
        let main = [self.base, self.mantle, self.surface0, self.surface1];
        let toward = if self.light { Color32::BLACK } else { Color32::WHITE };
        let fix = |color: Color32, min: f32, backgrounds: &[Color32]| {
            let mut out = color;
            for step in 1..=20 {
                if backgrounds.iter().all(|bg| contrast(out, *bg) >= min) {
                    break;
                }
                out = super::material::lerp_color(color, toward, step as f32 * 0.05);
            }
            out
        };
        self.text = fix(self.text, 5.5, &all);
        self.subtext0 = fix(self.subtext0, 4.5, &main);
        self.subtext1 = fix(self.subtext1, 3.5, &main);
        for accent in [&mut self.mauve, &mut self.blue, &mut self.peach, &mut self.green, &mut self.red, &mut self.yellow] {
            *accent = fix(*accent, 3.5, &main);
        }
    }

    pub fn load() -> Self {
        let mut theme = Self::default();

        // 1. Try to load colors from ~/.local/state/serpantinum/qs_colors.json
        let candidate_paths = [
            dirs::home_dir().map(|h| h.join(".local/state/serpantinum/qs_colors.json")),
            dirs::home_dir().map(|h| h.join(".config/serpantinum/qs_colors.json")),
        ];

        for path in candidate_paths.into_iter().flatten() {
            if let Ok(content) = fs::read_to_string(&path)
                && let Ok(raw) = serde_json::from_str::<SerpantinumColorsRaw>(&content) {
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
                    if let Some(c) = raw.yellow.and_then(|s| parse_hex(&s)) { theme.yellow = c; }
                    break;
                }
        }

        // 2. Try to read borderRadius from ~/.config/serpantinum/settings.json
        if let Some(home) = dirs::home_dir() {
            let settings_path = home.join(".config/serpantinum/settings.json");
            if let Ok(content) = fs::read_to_string(settings_path)
                && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
                    && let Some(r) = val.get("theme").and_then(|t| t.get("borderRadius")).and_then(|b| b.as_f64()) {
                        theme.border_radius = r as f32;
                    }
        }

        theme
    }

    /// The accent softened toward the text color, for the selection, focus and hover tints: the
    /// same calm shade everywhere instead of the full accent at several strengths.
    pub fn tint(&self, alpha: u8) -> Color32 {
        self.with_alpha(super::material::lerp_color(self.mauve, self.text, 0.22), alpha)
    }

    /// Makes the backgrounds translucent: `opacity` is a percentage, 100 leaves them solid.
    pub fn fade(&mut self, opacity: u8) {
        let a = (opacity.clamp(1, 100) as f32 * 2.55).round() as u8;
        // Whatever shows through is unknown, so the dimmer texts move toward the main one to stay legible
        let see_through = (100 - opacity.clamp(1, 100)) as f32 / 100.0;
        self.subtext0 = super::material::lerp_color(self.subtext0, self.text, see_through);
        self.subtext1 = super::material::lerp_color(self.subtext1, self.text, see_through);
        for c in [&mut self.base, &mut self.mantle, &mut self.crust, &mut self.surface0, &mut self.surface1, &mut self.surface2] {
            *c = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);
        }
    }

    /// Semi-transparent color
    pub fn with_alpha(&self, color: Color32, alpha: u8) -> Color32 {
        // r/g/b of a Color32 are premultiplied, which only matches the color itself while it is solid
        let [r, g, b, _] = color.to_srgba_unmultiplied();
        Color32::from_rgba_unmultiplied(r, g, b, alpha)
    }

    /// Smoothly blends between two palettes.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        use super::material::{lerp, lerp_color};
        let t = t.clamp(0.0, 1.0);
        Self {
            base: lerp_color(self.base, other.base, t),
            mantle: lerp_color(self.mantle, other.mantle, t),
            crust: lerp_color(self.crust, other.crust, t),
            text: lerp_color(self.text, other.text, t),
            subtext0: lerp_color(self.subtext0, other.subtext0, t),
            subtext1: lerp_color(self.subtext1, other.subtext1, t),
            surface0: lerp_color(self.surface0, other.surface0, t),
            surface1: lerp_color(self.surface1, other.surface1, t),
            surface2: lerp_color(self.surface2, other.surface2, t),
            blue: lerp_color(self.blue, other.blue, t),
            mauve: lerp_color(self.mauve, other.mauve, t),
            peach: lerp_color(self.peach, other.peach, t),
            green: lerp_color(self.green, other.green, t),
            red: lerp_color(self.red, other.red, t),
            yellow: lerp_color(self.yellow, other.yellow, t),
            border_radius: lerp(self.border_radius, other.border_radius, t),
            light: if t < 0.5 { self.light } else { other.light },
        }
    }
}

fn luminance(color: Color32) -> f32 {
    let channel = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

/// WCAG contrast ratio of two opaque colors, 1 (identical) to 21.
fn contrast(a: Color32, b: Color32) -> f32 {
    let (hi, lo) = (luminance(a).max(luminance(b)), luminance(a).min(luminance(b)));
    (hi + 0.05) / (lo + 0.05)
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
    super::material::set_radius_scale(theme.border_radius / BASE_RADIUS);
    let mut visuals = if theme.light { egui::Visuals::light() } else { egui::Visuals::dark() };
    visuals.window_corner_radius = egui::CornerRadius::same(super::material::radius_window());
    visuals.menu_corner_radius = egui::CornerRadius::same(super::material::radius_container());
    visuals.window_stroke = egui::Stroke::NONE;
    visuals.popup_shadow = egui::Shadow::NONE;
    visuals.window_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 32,
        spread: 0,
        // A shadow would show through translucent surfaces
        color: Color32::from_black_alpha(if theme.base.a() < 255 { 0 } else if theme.light { 50 } else { 110 }),
    };
    let w = &mut visuals.widgets;
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.bg_stroke = egui::Stroke::NONE;
        state.fg_stroke.color = theme.text;
    }
    w.noninteractive.bg_fill = theme.surface0;
    w.inactive.bg_fill = theme.with_alpha(theme.subtext1, 64);
    w.hovered.bg_fill = theme.with_alpha(theme.subtext0, 110);
    w.active.bg_fill = theme.with_alpha(theme.mauve, 150);
    w.open.bg_fill = theme.surface2;
    // Flat look: no outline, but the stroke color is also what selected text is drawn in
    visuals.selection.stroke = egui::Stroke::new(0.0, theme.text);
    visuals.selection.bg_fill = theme.tint(super::material::ALPHA_SEL_BG);
    visuals.panel_fill = theme.base;
    visuals.window_fill = theme.mantle;
    visuals.extreme_bg_color = theme.surface1;
    visuals.override_text_color = Some(theme.text);
    ctx.set_visuals(visuals);
    super::material::set_focus_color(ctx, theme.mauve);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.interaction.selectable_labels = false;
        for (ts, size) in [
            (egui::TextStyle::Body, super::material::FONT_BODY),
            (egui::TextStyle::Button, super::material::FONT_BODY),
            (egui::TextStyle::Small, super::material::FONT_CAPTION),
            (egui::TextStyle::Heading, super::material::FONT_TITLE),
        ] {
            if let Some(f) = style.text_styles.get_mut(&ts) {
                f.size = size;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_readable() {
        for (id, ..) in PRESETS {
            let settings = crate::settings::Settings { theme: id.to_owned(), ..Default::default() };
            let t = SerpantinumTheme::from_settings(&settings);
            for bg in [t.base, t.mantle, t.surface0, t.surface1] {
                assert!(contrast(t.text, bg) >= 5.5, "{id} text");
                assert!(contrast(t.subtext1, bg) >= 3.5, "{id} subtext1");
                assert!(contrast(t.mauve, bg) >= 3.5, "{id} accent");
                assert!(contrast(t.yellow, bg) >= 3.5, "{id} yellow");
            }
            assert!(contrast(t.text, t.surface2) >= 5.5, "{id} text on surface2");
        }
    }
}

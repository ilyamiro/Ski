use std::fs;
use eframe::egui::{self, Color32};
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use serde::{Deserialize, Serialize};

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
        ThemeColors::default().to_theme()
    }
}

/// The `border_radius` of the unchanged Material look; the corner setting scales it.
pub const BASE_RADIUS: f32 = 16.0;

/// The colors of a palette as `#rrggbb`: the shape of every file in `assets/themes` and of
/// `custom_theme` in the settings. A color that is missing or unreadable keeps the Dark palette's.
/// `accent` colors both the accent slots of the theme.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeColors {
    pub light: bool,
    pub base: String,
    pub mantle: String,
    pub crust: String,
    pub text: String,
    pub subtext0: String,
    pub subtext1: String,
    pub surface0: String,
    pub surface1: String,
    pub surface2: String,
    pub accent: String,
    pub peach: String,
    pub green: String,
    pub red: String,
    pub yellow: String,
}

impl Default for ThemeColors {
    /// The Dark palette; kept in code as the fallback every other palette is read against.
    fn default() -> Self {
        let hex = |s: &str| s.to_owned();
        Self {
            light: false,
            base: hex("#0f0d13"),
            mantle: hex("#1c1b20"),
            crust: hex("#141318"),
            text: hex("#e6e1e9"),
            subtext0: hex("#cac4cf"),
            subtext1: hex("#938f99"),
            surface0: hex("#201f24"),
            surface1: hex("#2b292f"),
            surface2: hex("#36343a"),
            accent: hex("#ccbeff"),
            peach: hex("#eeb8cb"),
            green: hex("#cac3dc"),
            red: hex("#ffb4ab"),
            yellow: hex("#f5d76e"),
        }
    }
}

impl ThemeColors {
    pub fn to_theme(&self) -> SerpantinumTheme {
        let dark = Self::default();
        let col = |mine: &str, dark: &str| parse_hex(mine).or_else(|| parse_hex(dark)).unwrap_or(Color32::GRAY);
        let accent = col(&self.accent, &dark.accent);
        SerpantinumTheme {
            base: col(&self.base, &dark.base),
            mantle: col(&self.mantle, &dark.mantle),
            crust: col(&self.crust, &dark.crust),
            text: col(&self.text, &dark.text),
            subtext0: col(&self.subtext0, &dark.subtext0),
            subtext1: col(&self.subtext1, &dark.subtext1),
            surface0: col(&self.surface0, &dark.surface0),
            surface1: col(&self.surface1, &dark.surface1),
            surface2: col(&self.surface2, &dark.surface2),
            blue: accent,
            mauve: accent,
            peach: col(&self.peach, &dark.peach),
            green: col(&self.green, &dark.green),
            red: col(&self.red, &dark.red),
            yellow: col(&self.yellow, &dark.yellow),
            border_radius: BASE_RADIUS,
            light: self.light,
        }
    }

    /// The palette of a finished theme, to start a custom one from.
    pub fn from_theme(t: &SerpantinumTheme) -> Self {
        Self {
            light: t.light,
            base: to_hex(t.base),
            mantle: to_hex(t.mantle),
            crust: to_hex(t.crust),
            text: to_hex(t.text),
            subtext0: to_hex(t.subtext0),
            subtext1: to_hex(t.subtext1),
            surface0: to_hex(t.surface0),
            surface1: to_hex(t.surface1),
            surface2: to_hex(t.surface2),
            accent: to_hex(t.mauve),
            peach: to_hex(t.peach),
            green: to_hex(t.green),
            red: to_hex(t.red),
            yellow: to_hex(t.yellow),
        }
    }

    /// The color fields by their key in the file, with the name the editor shows for each.
    pub fn fields(&mut self) -> [(&'static str, &'static str, &mut String); 14] {
        [
            ("base", "Background", &mut self.base),
            ("mantle", "Panels", &mut self.mantle),
            ("crust", "Deepest background", &mut self.crust),
            ("surface0", "Cards", &mut self.surface0),
            ("surface1", "Raised cards", &mut self.surface1),
            ("surface2", "Hover fill", &mut self.surface2),
            ("text", "Text", &mut self.text),
            ("subtext0", "Secondary text", &mut self.subtext0),
            ("subtext1", "Faint text", &mut self.subtext1),
            ("accent", "Accent", &mut self.accent),
            ("peach", "Warm accent", &mut self.peach),
            ("green", "Green", &mut self.green),
            ("red", "Red", &mut self.red),
            ("yellow", "Yellow", &mut self.yellow),
        ]
    }
}

/// A palette the user made: a file in the state directory (see `user_themes_dir`) whose name,
/// without `.json`, is its id; it is selected as `custom:<id>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomTheme {
    /// Not in the file: the file's name.
    #[serde(default, skip_serializing)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(flatten)]
    pub colors: ThemeColors,
}

/// The user's themes as last read from their folder, for `from_settings` and the settings page.
static USER_THEMES: RwLock<Vec<CustomTheme>> = RwLock::new(Vec::new());

/// Where the user's themes are kept, a JSON file each: `~/.local/state/ski/themes`.
pub fn user_themes_dir() -> Option<PathBuf> {
    let state = match crate::fs::admin::config_override() {
        // As administrator: the state folder next to the config folder of whoever started Ski
        Some(config) => config.parent().map(|home| home.join(".local/state")),
        None => dirs::state_dir().or_else(dirs::data_local_dir),
    };
    state.map(|dir| dir.join("ski").join("themes"))
}

fn read_user_themes() -> Vec<CustomTheme> {
    let Some(dir) = user_themes_dir() else { return Vec::new() };
    let mut themes: Vec<CustomTheme> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let id = path.file_stem()?.to_str()?.to_owned();
            if path.extension()? != "json" {
                return None;
            }
            let mut theme: CustomTheme = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
            if theme.name.trim().is_empty() {
                theme.name = id.clone();
            }
            theme.id = id;
            Some(theme)
        })
        .collect();
    themes.sort_by_key(|t| (t.name.to_lowercase(), t.id.clone()));
    themes
}

/// Reads the folder again; true when the themes are not what they were.
pub fn reload_user_themes() -> bool {
    let read = read_user_themes();
    let mut current = USER_THEMES.write().unwrap_or_else(|e| e.into_inner());
    let changed = *current != read;
    *current = read;
    changed
}

pub fn user_themes() -> Vec<CustomTheme> {
    USER_THEMES.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Writes the theme to its file, whole or not at all.
pub fn save_user_theme(theme: &CustomTheme) {
    // As administrator the folder belongs to someone else
    if crate::fs::admin::is_admin() {
        return;
    }
    let Some(dir) = user_themes_dir() else { return };
    let _ = fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string_pretty(theme) {
        let file = dir.join(format!("{}.json", theme.id));
        let partial = dir.join(format!("{}.json.tmp", theme.id));
        if fs::write(&partial, json).is_ok() {
            let _ = fs::rename(partial, file);
        }
    }
}

pub fn delete_user_theme(id: &str) {
    if crate::fs::admin::is_admin() {
        return;
    }
    if let Some(dir) = user_themes_dir() {
        let _ = fs::remove_file(dir.join(format!("{id}.json")));
    }
}

/// A palette bundled in `assets/themes`; the file name is its id.
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    /// Listed under "Browse more themes" instead of right in the settings.
    pub more: bool,
    pub colors: ThemeColors,
}

#[derive(Deserialize)]
struct ThemeFile {
    name: String,
    #[serde(default)]
    group: String,
    #[serde(flatten)]
    colors: ThemeColors,
}

/// Dark and Light first, then the curated palettes, then those behind "Browse more themes".
const BUNDLED: &[(&str, &str)] = &[
    ("dark", include_str!("../../assets/themes/dark.json")),
    ("light", include_str!("../../assets/themes/light.json")),
    ("midnight", include_str!("../../assets/themes/midnight.json")),
    ("nord", include_str!("../../assets/themes/nord.json")),
    ("gruvbox", include_str!("../../assets/themes/gruvbox.json")),
    ("catppuccin", include_str!("../../assets/themes/catppuccin.json")),
    ("dracula", include_str!("../../assets/themes/dracula.json")),
    ("tokyonight", include_str!("../../assets/themes/tokyonight.json")),
    ("rosepine", include_str!("../../assets/themes/rosepine.json")),
    ("onedark", include_str!("../../assets/themes/onedark.json")),
    ("everforest", include_str!("../../assets/themes/everforest.json")),
    ("latte", include_str!("../../assets/themes/latte.json")),
    ("catppuccin-frappe", include_str!("../../assets/themes/catppuccin-frappe.json")),
    ("catppuccin-macchiato", include_str!("../../assets/themes/catppuccin-macchiato.json")),
    ("tokyonight-storm", include_str!("../../assets/themes/tokyonight-storm.json")),
    ("tokyonight-moon", include_str!("../../assets/themes/tokyonight-moon.json")),
    ("rosepine-moon", include_str!("../../assets/themes/rosepine-moon.json")),
    ("gruvbox-material", include_str!("../../assets/themes/gruvbox-material.json")),
    ("kanagawa", include_str!("../../assets/themes/kanagawa.json")),
    ("kanagawa-dragon", include_str!("../../assets/themes/kanagawa-dragon.json")),
    ("nightfox", include_str!("../../assets/themes/nightfox.json")),
    ("carbonfox", include_str!("../../assets/themes/carbonfox.json")),
    ("duskfox", include_str!("../../assets/themes/duskfox.json")),
    ("terafox", include_str!("../../assets/themes/terafox.json")),
    ("nordfox", include_str!("../../assets/themes/nordfox.json")),
    ("monokai", include_str!("../../assets/themes/monokai.json")),
    ("monokai-pro", include_str!("../../assets/themes/monokai-pro.json")),
    ("ayu-dark", include_str!("../../assets/themes/ayu-dark.json")),
    ("ayu-mirage", include_str!("../../assets/themes/ayu-mirage.json")),
    ("material-ocean", include_str!("../../assets/themes/material-ocean.json")),
    ("material-darker", include_str!("../../assets/themes/material-darker.json")),
    ("palenight", include_str!("../../assets/themes/palenight.json")),
    ("night-owl", include_str!("../../assets/themes/night-owl.json")),
    ("nightfly", include_str!("../../assets/themes/nightfly.json")),
    ("moonfly", include_str!("../../assets/themes/moonfly.json")),
    ("oceanic-next", include_str!("../../assets/themes/oceanic-next.json")),
    ("base16-ocean", include_str!("../../assets/themes/base16-ocean.json")),
    ("horizon", include_str!("../../assets/themes/horizon.json")),
    ("synthwave84", include_str!("../../assets/themes/synthwave84.json")),
    ("cobalt2", include_str!("../../assets/themes/cobalt2.json")),
    ("github-dark", include_str!("../../assets/themes/github-dark.json")),
    ("github-dimmed", include_str!("../../assets/themes/github-dimmed.json")),
    ("sonokai", include_str!("../../assets/themes/sonokai.json")),
    ("edge-dark", include_str!("../../assets/themes/edge-dark.json")),
    ("oxocarbon", include_str!("../../assets/themes/oxocarbon.json")),
    ("vesper", include_str!("../../assets/themes/vesper.json")),
    ("poimandres", include_str!("../../assets/themes/poimandres.json")),
    ("andromeda", include_str!("../../assets/themes/andromeda.json")),
    ("iceberg", include_str!("../../assets/themes/iceberg.json")),
    ("zenburn", include_str!("../../assets/themes/zenburn.json")),
    ("tomorrow-night", include_str!("../../assets/themes/tomorrow-night.json")),
    ("doom-one", include_str!("../../assets/themes/doom-one.json")),
    ("melange-dark", include_str!("../../assets/themes/melange-dark.json")),
    ("mellow", include_str!("../../assets/themes/mellow.json")),
    ("spaceduck", include_str!("../../assets/themes/spaceduck.json")),
    ("cyberdream", include_str!("../../assets/themes/cyberdream.json")),
    ("flexoki-dark", include_str!("../../assets/themes/flexoki-dark.json")),
    ("everblush", include_str!("../../assets/themes/everblush.json")),
    ("solarized-dark", include_str!("../../assets/themes/solarized-dark.json")),
    ("gruvbox-light", include_str!("../../assets/themes/gruvbox-light.json")),
    ("rosepine-dawn", include_str!("../../assets/themes/rosepine-dawn.json")),
    ("ayu-light", include_str!("../../assets/themes/ayu-light.json")),
    ("github-light", include_str!("../../assets/themes/github-light.json")),
    ("one-light", include_str!("../../assets/themes/one-light.json")),
    ("tokyonight-day", include_str!("../../assets/themes/tokyonight-day.json")),
    ("everforest-light", include_str!("../../assets/themes/everforest-light.json")),
    ("kanagawa-lotus", include_str!("../../assets/themes/kanagawa-lotus.json")),
    ("dayfox", include_str!("../../assets/themes/dayfox.json")),
    ("flexoki-light", include_str!("../../assets/themes/flexoki-light.json")),
    ("tomorrow", include_str!("../../assets/themes/tomorrow.json")),
    ("light-owl", include_str!("../../assets/themes/light-owl.json")),
    ("melange-light", include_str!("../../assets/themes/melange-light.json")),
    ("material-lighter", include_str!("../../assets/themes/material-lighter.json")),
    ("nord-light", include_str!("../../assets/themes/nord-light.json")),
    ("papercolor", include_str!("../../assets/themes/papercolor.json")),
    ("solarized-light", include_str!("../../assets/themes/solarized-light.json")),
];

/// Every bundled palette, in the order of `BUNDLED`.
pub fn presets() -> &'static [Preset] {
    static PRESETS: OnceLock<Vec<Preset>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        BUNDLED
            .iter()
            .filter_map(|(id, json)| {
                let file: ThemeFile = serde_json::from_str(json).ok()?;
                // Built once and kept for the life of the program
                let name: &'static str = Box::leak(file.name.into_boxed_str());
                Some(Preset { id, name, more: file.group == "more", colors: file.colors })
            })
            .collect()
    })
}

pub fn preset(id: &str) -> Option<&'static Preset> {
    presets().iter().find(|p| p.id == id)
}

/// Quick accent choices offered in the settings.
pub const ACCENTS: [&str; 9] = ["#ccbeff", "#8ab4f8", "#88c0d0", "#a3d9a5", "#f5d76e", "#ffb784", "#ff9aa2", "#e0a3ff", "#f28b82"];

impl SerpantinumTheme {
    /// The same palette with every surface one step down, for controls that sit straight on a
    /// popup instead of on a card.
    pub fn lowered(&self) -> Self {
        Self { surface2: self.surface1, surface1: self.surface0, surface0: self.mantle, ..*self }
    }

    /// Palette picked in the settings (`system` follows Serpantinum, `custom:<id>` is one of the user's own), with the accent applied.
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let custom = settings.theme.strip_prefix("custom:").and_then(|id| user_themes().into_iter().find(|c| c.id == id));
        let mut theme = if let Some(custom) = &custom {
            custom.colors.to_theme()
        } else if let Some(preset) = preset(&settings.theme) {
            preset.colors.to_theme()
        } else {
            Self::load()
        };
        if let Some(accent) = settings.accent.as_deref().and_then(parse_hex) {
            theme.mauve = accent;
            theme.blue = accent;
        }
        theme.border_radius = BASE_RADIUS * settings.corner_scale;
        // A custom palette is shown exactly as written
        if custom.is_none() {
            theme.ensure_readable();
        }
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

pub fn to_hex(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
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
/// `popup` is the solid palette that menus and tooltips are drawn in, whatever the window's opacity.
pub fn apply(ctx: &egui::Context, theme: &SerpantinumTheme, popup: &SerpantinumTheme) {
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
        // Only popups cast it, and they stay solid
        color: Color32::from_black_alpha(if popup.light { 50 } else { 110 }),
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
    visuals.window_fill = popup.mantle;
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
        for preset in presets() {
            let id = preset.id;
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

    #[test]
    fn a_custom_theme_is_picked_by_its_id() {
        let file: CustomTheme = serde_json::from_str(r##"{ "name": "Mine", "base": "#102030" }"##).unwrap();
        USER_THEMES.write().unwrap().push(CustomTheme { id: "test-mine".to_owned(), ..file });
        let settings = crate::settings::Settings { theme: "custom:test-mine".to_owned(), ..Default::default() };
        let t = SerpantinumTheme::from_settings(&settings);
        assert_eq!(t.base, parse_hex("#102030").unwrap());
        // What the file leaves out keeps the Dark palette's
        assert_eq!(t.text, parse_hex(&ThemeColors::default().text).unwrap());
    }

    #[test]
    fn every_bundled_file_loads() {
        assert_eq!(presets().len(), BUNDLED.len());
        let mut ids: Vec<_> = presets().iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), BUNDLED.len());
        for p in presets() {
            let mut colors = p.colors.clone();
            for (key, _, hex) in colors.fields() {
                assert!(parse_hex(hex).is_some(), "{} {key}", p.id);
            }
        }
    }
}

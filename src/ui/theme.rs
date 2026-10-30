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

/// A named palette: id, name, colors (base, mantle, crust, text, subtext0, subtext1, surface0-2,
/// accent, peach, green, red, yellow) and whether it is light.
pub type Preset = (&'static str, &'static str, [&'static str; 14], bool);

/// Palettes offered right in the settings: Dark and Light first, then a curated few.
pub const PRESETS: &[Preset] = &[
    ("dark", "Dark", ["#0f0d13", "#1c1b20", "#141318", "#e6e1e9", "#cac4cf", "#938f99", "#201f24", "#2b292f", "#36343a", "#ccbeff", "#eeb8cb", "#cac3dc", "#ffb4ab", "#f5d76e"], false),
    ("light", "Light", ["#fef7ff", "#f3edf7", "#ece6f0", "#1d1b20", "#49454f", "#79747e", "#f7f2fa", "#e7e0ec", "#ddd6e2", "#6750a4", "#7d5260", "#625b71", "#b3261e", "#7d5800"], true),
    ("midnight", "Midnight", ["#0b0e14", "#151a23", "#10141b", "#dfe6f3", "#b4bfd3", "#7d889c", "#181e29", "#212938", "#2b3446", "#8ab4f8", "#f2b8a2", "#9bd3a8", "#f28b82", "#fdd663"], false),
    ("nord", "Nord", ["#242933", "#2e3440", "#20242d", "#eceff4", "#d8dee9", "#8c95a6", "#343b49", "#3b4252", "#434c5e", "#88c0d0", "#d08770", "#a3be8c", "#bf616a", "#ebcb8b"], false),
    ("gruvbox", "Gruvbox", ["#1d2021", "#282828", "#181a1b", "#ebdbb2", "#d5c4a1", "#928374", "#32302f", "#3c3836", "#504945", "#d3869b", "#fe8019", "#b8bb26", "#fb4934", "#fabd2f"], false),
    ("catppuccin", "Catppuccin Mocha", ["#181825", "#1e1e2e", "#11111b", "#cdd6f4", "#bac2de", "#8087a2", "#313244", "#45475a", "#585b70", "#cba6f7", "#fab387", "#a6e3a1", "#f38ba8", "#f9e2af"], false),
    ("dracula", "Dracula", ["#1e1f29", "#282a36", "#191a21", "#f8f8f2", "#cfcfc2", "#8a93c0", "#313442", "#3b3f51", "#464b60", "#bd93f9", "#ffb86c", "#50fa7b", "#ff5555", "#f1fa8c"], false),
    ("tokyonight", "Tokyo Night", ["#16161e", "#1a1b26", "#13131a", "#c0caf5", "#a9b1d6", "#7a82a8", "#24283b", "#2f334d", "#3b4261", "#7aa2f7", "#ff9e64", "#9ece6a", "#f7768e", "#e0af68"], false),
    ("rosepine", "Rosé Pine", ["#191724", "#1f1d2e", "#16141f", "#e0def4", "#c4c1dc", "#908caa", "#26233a", "#403d52", "#524f67", "#c4a7e7", "#f6c177", "#9ccfd8", "#eb6f92", "#f6c177"], false),
    ("onedark", "One Dark", ["#1e2227", "#282c34", "#1b1f23", "#abb2bf", "#9aa1ad", "#7f8794", "#2f343e", "#3a404c", "#454c5a", "#61afef", "#d19a66", "#98c379", "#e06c75", "#e5c07b"], false),
    ("everforest", "Everforest", ["#272e33", "#2d353b", "#232a2e", "#d3c6aa", "#bfb49a", "#859289", "#343f44", "#3d484d", "#475258", "#a7c080", "#e69875", "#83c092", "#e67e80", "#dbbc7f"], false),
    ("latte", "Catppuccin Latte", ["#eff1f5", "#e6e9ef", "#dce0e8", "#4c4f69", "#5c5f77", "#7c7f93", "#f2f4f8", "#d9dde6", "#ccd0da", "#8839ef", "#fe640b", "#40a02b", "#d20f39", "#df8e1d"], true),
];

/// Palettes behind "Browse more themes".
pub const MORE_PRESETS: &[Preset] = &[
    ("catppuccin-frappe", "Catppuccin Frappé", ["#292c3c", "#303446", "#232634", "#c6d0f5", "#b5bfe2", "#838ba7", "#414559", "#51576d", "#626880", "#ca9ee6", "#ef9f76", "#a6d189", "#e78284", "#e5c890"], false),
    ("catppuccin-macchiato", "Catppuccin Macchiato", ["#1e2030", "#24273a", "#181926", "#cad3f5", "#b8c0e0", "#8087a2", "#363a4f", "#494d64", "#5b6078", "#c6a0f6", "#f5a97f", "#a6da95", "#ed8796", "#eed49f"], false),
    ("tokyonight-storm", "Tokyo Night Storm", ["#1f2335", "#24283b", "#1b1e2d", "#c0caf5", "#a9b1d6", "#737aa2", "#292e42", "#343a55", "#414868", "#7aa2f7", "#ff9e64", "#9ece6a", "#f7768e", "#e0af68"], false),
    ("tokyonight-moon", "Tokyo Night Moon", ["#1e2030", "#222436", "#191b29", "#c8d3f5", "#b4c2f0", "#7a88cf", "#2f334d", "#3b4064", "#444a73", "#82aaff", "#ff966c", "#c3e88d", "#ff757f", "#ffc777"], false),
    ("rosepine-moon", "Rosé Pine Moon", ["#232136", "#2a273f", "#1d1b2e", "#e0def4", "#c4c1dc", "#908caa", "#393552", "#44415a", "#56526e", "#c4a7e7", "#ea9a97", "#9ccfd8", "#eb6f92", "#f6c177"], false),
    ("gruvbox-material", "Gruvbox Material", ["#1d2021", "#282828", "#1b1b1b", "#d4be98", "#ddc7a1", "#928374", "#32302f", "#3c3836", "#45403d", "#7daea3", "#e78a4e", "#a9b665", "#ea6962", "#d8a657"], false),
    ("kanagawa", "Kanagawa", ["#16161d", "#1f1f28", "#121218", "#dcd7ba", "#c8c093", "#727169", "#2a2a37", "#363646", "#54546d", "#7e9cd8", "#ffa066", "#98bb6c", "#ff5d62", "#e6c384"], false),
    ("kanagawa-dragon", "Kanagawa Dragon", ["#12120f", "#181616", "#0d0c0c", "#c5c9c5", "#a6a69c", "#737c73", "#282727", "#393836", "#625e5a", "#8ba4b0", "#b6927b", "#87a987", "#c4746e", "#c4b28a"], false),
    ("nightfox", "Nightfox", ["#131a24", "#192330", "#0e141c", "#cdcecf", "#aeafb0", "#71839b", "#212e3f", "#29394f", "#39506d", "#719cd6", "#f4a261", "#81b29a", "#c94f6d", "#dbc074"], false),
    ("carbonfox", "Carbonfox", ["#0c0c0c", "#161616", "#080808", "#f2f4f8", "#b6b8bb", "#7b7c7e", "#252525", "#2a2a2a", "#404040", "#78a9ff", "#ffab91", "#25be6a", "#ee5396", "#f1c21b"], false),
    ("duskfox", "Duskfox", ["#191726", "#232136", "#14121f", "#e0def4", "#cdcbe0", "#6e6a86", "#2d2a45", "#373354", "#4b4673", "#c4a7e7", "#ea9a97", "#a3be8c", "#eb6f92", "#f6c177"], false),
    ("terafox", "Terafox", ["#0f1c1e", "#152528", "#0b1517", "#e6eaea", "#cbd9d8", "#587b7b", "#1d3337", "#254147", "#2d4f56", "#a1cdd8", "#ff8349", "#7aa4a1", "#e85c51", "#f0c674"], false),
    ("nordfox", "Nordfox", ["#232831", "#2e3440", "#1e222a", "#cdcecf", "#abb1bb", "#7e8188", "#39404f", "#444c5e", "#5a657d", "#81a1c1", "#c9826b", "#a3be8c", "#bf616a", "#ebcb8b"], false),
    ("monokai", "Monokai", ["#1e1f1c", "#272822", "#191a17", "#f8f8f2", "#cfcfc2", "#90908a", "#34352f", "#3e3d32", "#49483e", "#66d9ef", "#fd971f", "#a6e22e", "#f92672", "#e6db74"], false),
    ("monokai-pro", "Monokai Pro", ["#221f22", "#2d2a2e", "#19181a", "#fcfcfa", "#c1c0c0", "#939293", "#363337", "#403e41", "#5b595c", "#ab9df2", "#fc9867", "#a9dc76", "#ff6188", "#ffd866"], false),
    ("ayu-dark", "Ayu Dark", ["#0b0e14", "#0d1017", "#090b10", "#bfbdb6", "#a5a39c", "#6c7380", "#141821", "#1c212b", "#262b36", "#e6b450", "#ff8f40", "#aad94c", "#f07178", "#ffb454"], false),
    ("ayu-mirage", "Ayu Mirage", ["#1a1f29", "#1f2430", "#171b24", "#cccac2", "#b8b4a8", "#707a8c", "#242936", "#2d3343", "#383e4f", "#73d0ff", "#ffa659", "#d5ff80", "#f28779", "#ffd173"], false),
    ("material-ocean", "Material Ocean", ["#090b10", "#0f111a", "#070910", "#babed8", "#a6accd", "#717cb4", "#1a1c25", "#232637", "#2f3344", "#82aaff", "#f78c6c", "#c3e88d", "#f07178", "#ffcb6b"], false),
    ("material-darker", "Material Darker", ["#1a1a1a", "#212121", "#151515", "#eeffff", "#b0bec5", "#8a8a8a", "#2a2a2a", "#323232", "#3d3d3d", "#80cbc4", "#f78c6c", "#c3e88d", "#f07178", "#ffcb6b"], false),
    ("palenight", "Palenight", ["#232534", "#292d3e", "#1b1e2b", "#d6daf2", "#a6accd", "#676e95", "#32374d", "#3a3f58", "#444a64", "#c792ea", "#f78c6c", "#c3e88d", "#ff5370", "#ffcb6b"], false),
    ("night-owl", "Night Owl", ["#010e17", "#011627", "#01111d", "#d6deeb", "#a7b4c4", "#637777", "#0b2942", "#1d3b53", "#234d70", "#82aaff", "#f78c6c", "#addb67", "#ef5350", "#ffeb95"], false),
    ("nightfly", "Nightfly", ["#01111f", "#011627", "#000d18", "#c3ccdc", "#acb4c2", "#7c8f8f", "#092236", "#0e293f", "#1d3b53", "#82aaff", "#f78c6c", "#a1cd5e", "#fc514e", "#e3d18a"], false),
    ("moonfly", "Moonfly", ["#080808", "#121212", "#050505", "#c6c6c6", "#b2b2b2", "#808080", "#1c1c1c", "#262626", "#323437", "#80a0ff", "#de935f", "#8cc85f", "#ff5454", "#e3c78a"], false),
    ("oceanic-next", "Oceanic Next", ["#16232a", "#1b2b34", "#121d23", "#d8dee9", "#c0c5ce", "#a7adba", "#243540", "#343d46", "#4f5b66", "#6699cc", "#f99157", "#99c794", "#ec5f67", "#fac863"], false),
    ("base16-ocean", "Ocean", ["#252a34", "#2b303b", "#20242d", "#dfe1e8", "#c0c5ce", "#a7adba", "#343d46", "#3e4752", "#4f5b66", "#b48ead", "#d08770", "#a3be8c", "#bf616a", "#ebcb8b"], false),
    ("horizon", "Horizon", ["#1a1c23", "#1c1e26", "#16161c", "#d5d8da", "#bbbbbb", "#6c6f93", "#232530", "#2e303e", "#3d4050", "#e95678", "#f09383", "#29d398", "#f43e5c", "#fab795"], false),
    ("synthwave84", "Synthwave '84", ["#241b2f", "#262335", "#1e1828", "#f0eff1", "#d2cfe0", "#848bbd", "#2a2139", "#34294f", "#463465", "#ff7edb", "#f97e72", "#72f1b8", "#fe4450", "#fede5d"], false),
    ("cobalt2", "Cobalt2", ["#15232d", "#193549", "#122738", "#ffffff", "#e1efff", "#9aa9b8", "#1e3f57", "#234a66", "#2b5876", "#ffc600", "#ff9d00", "#3ad900", "#ff628c", "#fad000"], false),
    ("github-dark", "GitHub Dark", ["#010409", "#0d1117", "#010409", "#e6edf3", "#c9d1d9", "#8b949e", "#161b22", "#21262d", "#30363d", "#58a6ff", "#ffa657", "#3fb950", "#f85149", "#d29922"], false),
    ("github-dimmed", "GitHub Dimmed", ["#1c2128", "#22272e", "#1c2128", "#adbac7", "#909dab", "#768390", "#2d333b", "#373e47", "#444c56", "#539bf5", "#f69d50", "#57ab5a", "#e5534b", "#c69026"], false),
    ("sonokai", "Sonokai", ["#222327", "#2c2e34", "#181819", "#e2e2e3", "#b9babc", "#7f8490", "#33353f", "#363944", "#414550", "#76cce0", "#f39660", "#9ed072", "#fc5d7c", "#e7c664"], false),
    ("edge-dark", "Edge Dark", ["#202023", "#2c2e34", "#1b1b1e", "#c5cdd9", "#b0b8c4", "#758094", "#33353f", "#363a4e", "#414550", "#6cb6eb", "#d99a6c", "#a0c980", "#ec7279", "#deb974"], false),
    ("oxocarbon", "Oxocarbon", ["#111111", "#161616", "#0b0b0b", "#f2f4f8", "#dde1e6", "#8d8d8d", "#212121", "#262626", "#393939", "#be95ff", "#ff7eb6", "#42be65", "#ee5396", "#ffe97b"], false),
    ("vesper", "Vesper", ["#0a0a0a", "#101010", "#070707", "#ffffff", "#c8c8c8", "#8b8b8b", "#1c1c1c", "#232323", "#343434", "#ffc799", "#ffb07a", "#99ffe4", "#ff8080", "#ffd28f"], false),
    ("poimandres", "Poimandres", ["#171922", "#1b1e28", "#13151c", "#e4f0fb", "#a6accd", "#767c9d", "#252b37", "#303340", "#3c4152", "#5de4c7", "#f087bd", "#5fb3a1", "#d0679d", "#fffac2"], false),
    ("andromeda", "Andromeda", ["#1b1d23", "#23262e", "#16181d", "#d5ced9", "#b9b3be", "#7c7682", "#2b2f38", "#333844", "#3d4250", "#00e8c6", "#f39c12", "#96e072", "#ee5d43", "#ffe66d"], false),
    ("iceberg", "Iceberg", ["#0f1117", "#161821", "#0c0d12", "#c6c8d1", "#b0b2bd", "#6b7089", "#1e2132", "#272c42", "#33374c", "#84a0c6", "#e2a478", "#b4be82", "#e27878", "#e2c792"], false),
    ("zenburn", "Zenburn", ["#2b2b2b", "#3f3f3f", "#262626", "#dcdccc", "#c0c0b0", "#9f9f8f", "#474747", "#4f4f4f", "#5f5f5f", "#8cd0d3", "#dfaf8f", "#7f9f7f", "#cc9393", "#f0dfaf"], false),
    ("tomorrow-night", "Tomorrow Night", ["#181a1b", "#1d1f21", "#151617", "#d6d8d6", "#c5c8c6", "#969896", "#282a2e", "#373b41", "#41454c", "#81a2be", "#de935f", "#b5bd68", "#cc6666", "#f0c674"], false),
    ("doom-one", "Doom One", ["#21242b", "#282c34", "#1c1f24", "#bbc2cf", "#a7aebb", "#848a96", "#2e323a", "#383d46", "#464b55", "#51afef", "#da8548", "#98be65", "#ff6c6b", "#ecbe7b"], false),
    ("melange-dark", "Melange", ["#221e1c", "#292522", "#1c1917", "#ece1d7", "#d0c0b0", "#a08a74", "#34302c", "#403a36", "#4d4540", "#e49b5d", "#e8a87c", "#85b695", "#d47766", "#ebc06d"], false),
    ("mellow", "Mellow", ["#111112", "#161617", "#0d0d0e", "#c9c7cd", "#b4b2bb", "#8a8892", "#1d1d20", "#27272a", "#353539", "#aca1cf", "#f5a191", "#90b99f", "#ea83a5", "#e6b99d"], false),
    ("spaceduck", "Spaceduck", ["#0b0d16", "#0f111b", "#080a12", "#ecf0c1", "#c1c3cc", "#818596", "#16172d", "#1b1c36", "#30365f", "#7a5ccc", "#e39400", "#5ccc96", "#e33400", "#f2ce00"], false),
    ("cyberdream", "Cyberdream", ["#111214", "#16181a", "#0d0e10", "#ffffff", "#d0d4dc", "#7b8496", "#1e2124", "#292c31", "#3c4048", "#5ea1ff", "#ffbd5e", "#5eff6c", "#ff6e5e", "#f1ff5e"], false),
    ("flexoki-dark", "Flexoki Dark", ["#100f0f", "#1c1b1a", "#0c0b0b", "#cecdc3", "#b7b5ac", "#878580", "#282726", "#343331", "#403e3c", "#4385be", "#da702c", "#879a39", "#d14d41", "#d0a215"], false),
    ("everblush", "Everblush", ["#10171a", "#141b1e", "#0d1315", "#dadada", "#b3b9b8", "#8a9294", "#1e2528", "#232a2d", "#2d3437", "#67b0e8", "#e9967e", "#8ccf7e", "#e57474", "#e5c76b"], false),
    ("solarized-dark", "Solarized Dark", ["#00212b", "#002b36", "#001e26", "#e1e5df", "#93a1a1", "#7a8b8d", "#073642", "#0f4452", "#1a5361", "#268bd2", "#e0733c", "#a1b32a", "#f0524f", "#b58900"], false),
    ("gruvbox-light", "Gruvbox Light", ["#fbf1c7", "#f2e5bc", "#ebdbb2", "#3c3836", "#504945", "#7c6f64", "#f9f5d7", "#ebdbb2", "#d5c4a1", "#076678", "#af3a03", "#79740e", "#9d0006", "#b57614"], true),
    ("rosepine-dawn", "Rosé Pine Dawn", ["#faf4ed", "#f4ede8", "#ebe4df", "#575279", "#6e6a8e", "#797593", "#fffaf3", "#f2e9e1", "#dfdad9", "#907aa9", "#d7827e", "#56949f", "#b4637a", "#ea9d34"], true),
    ("ayu-light", "Ayu Light", ["#fcfcfc", "#f3f4f5", "#e8eaec", "#5c6166", "#6c7175", "#8a9199", "#f8f9fa", "#e7eaed", "#dadde1", "#399ee6", "#fa8d3e", "#86b300", "#f07171", "#f2ae49"], true),
    ("github-light", "GitHub Light", ["#ffffff", "#f6f8fa", "#eaeef2", "#1f2328", "#424a53", "#656d76", "#ffffff", "#eaeef2", "#d0d7de", "#0969da", "#bc4c00", "#1a7f37", "#cf222e", "#9a6700"], true),
    ("one-light", "One Light", ["#fafafa", "#f0f0f1", "#e5e5e6", "#383a42", "#50525c", "#696c77", "#fbfbfb", "#e5e5e6", "#dbdbdc", "#4078f2", "#986801", "#50a14f", "#e45649", "#c18401"], true),
    ("tokyonight-day", "Tokyo Night Day", ["#e1e2e7", "#d0d5e3", "#c4c8da", "#3760bf", "#6172b0", "#848cb5", "#e9e9ed", "#c4c8da", "#b6bfe2", "#2e7de9", "#b15c00", "#587539", "#f52a65", "#8c6c3e"], true),
    ("everforest-light", "Everforest Light", ["#fdf6e3", "#f4f0d9", "#efebd4", "#5c6a72", "#708089", "#829181", "#fffbef", "#e6e2cc", "#e0dcc7", "#8da101", "#f57d26", "#35a77c", "#f85552", "#dfa000"], true),
    ("kanagawa-lotus", "Kanagawa Lotus", ["#f2ecbc", "#e5ddb0", "#dcd5ac", "#545464", "#43436c", "#8a8980", "#f6f1cc", "#e7dba0", "#d5cea3", "#4d699b", "#cc6d00", "#6f894e", "#c84053", "#77713f"], true),
    ("dayfox", "Dayfox", ["#f6f2ee", "#ebe5df", "#e4dcd4", "#3d2b5a", "#643f61", "#824d5b", "#faf7f4", "#dbd1dd", "#d3c7bb", "#2848a9", "#955f61", "#396847", "#a5222f", "#ac5402"], true),
    ("flexoki-light", "Flexoki Light", ["#fffcf0", "#f2f0e5", "#e6e4d9", "#100f0f", "#403e3c", "#6f6e69", "#fdfbf3", "#e6e4d9", "#dad8ce", "#205ea6", "#bc5215", "#66800b", "#af3029", "#ad8301"], true),
    ("tomorrow", "Tomorrow", ["#ffffff", "#f4f4f4", "#e8e8e8", "#4d4d4c", "#626261", "#8e908c", "#fafafa", "#e6e6e6", "#d6d6d6", "#4271ae", "#f5871f", "#718c00", "#c82829", "#eab700"], true),
    ("light-owl", "Light Owl", ["#fbfbfb", "#f0f0f0", "#e4e4e4", "#403f53", "#5e5d72", "#7a8181", "#ffffff", "#e4e7ea", "#d3d8dc", "#4876d6", "#c96765", "#08916a", "#de3d3b", "#daaa01"], true),
    ("melange-light", "Melange Light", ["#f4f0ed", "#e9e1db", "#ddd5cf", "#54433a", "#6b574c", "#7d6658", "#faf6f3", "#ddd3cc", "#d0c6bf", "#7892bd", "#bc5c00", "#3a684a", "#bf0021", "#a06d00"], true),
    ("material-lighter", "Material Lighter", ["#fafafa", "#f0f0f0", "#e6e6e6", "#37474f", "#546e7a", "#7e939e", "#ffffff", "#e7e7e8", "#d8d8da", "#39adb5", "#f76d47", "#91b859", "#e53935", "#ffb62c"], true),
    ("nord-light", "Nord Light", ["#eceff4", "#e5e9f0", "#d8dee9", "#2e3440", "#3b4252", "#4c566a", "#f2f4f8", "#d8dee9", "#cdd3de", "#5e81ac", "#d08770", "#a3be8c", "#bf616a", "#ebcb8b"], true),
    ("papercolor", "PaperColor", ["#f7f7f7", "#eeeeee", "#e4e4e4", "#444444", "#575757", "#878787", "#fafafa", "#e0e0e0", "#d0d0d0", "#005f87", "#d75f00", "#718c00", "#d70000", "#af8700"], true),
    ("solarized-light", "Solarized Light", ["#fdf6e3", "#eee8d5", "#e6dfca", "#3b4a50", "#586e75", "#6f8184", "#f7f0dc", "#e3dcc6", "#d6cfb8", "#268bd2", "#cb4b16", "#859900", "#dc322f", "#857000"], true),
];

/// Every palette, the curated ones first.
pub fn all_presets() -> impl Iterator<Item = &'static Preset> {
    PRESETS.iter().chain(MORE_PRESETS)
}

/// Quick accent choices offered in the settings.
pub const ACCENTS: [&str; 9] = ["#ccbeff", "#8ab4f8", "#88c0d0", "#a3d9a5", "#f5d76e", "#ffb784", "#ff9aa2", "#e0a3ff", "#f28b82"];

impl SerpantinumTheme {
    /// The same palette with every surface one step down, for controls that sit straight on a
    /// popup instead of on a card.
    pub fn lowered(&self) -> Self {
        Self { surface2: self.surface1, surface1: self.surface0, surface0: self.mantle, ..*self }
    }

    /// Palette picked in the settings (`system` follows Serpantinum), with the accent applied.
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let mut theme = match all_presets().find(|(id, ..)| *id == settings.theme) {
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
        for (id, ..) in all_presets() {
            let settings = crate::settings::Settings { theme: (*id).to_owned(), ..Default::default() };
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

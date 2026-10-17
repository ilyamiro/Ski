mod app;
mod fs;
mod ui;

use std::env;
use std::path::PathBuf;
use app::SkiApp;
use eframe::egui;
use ui::theme::SerpantinumTheme;

fn main() -> eframe::Result {
    let initial_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let theme = SerpantinumTheme::load();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ski")
            .with_inner_size([1080.0, 720.0])
            .with_min_inner_size([620.0, 420.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ski-file-manager",
        native_options,
        Box::new(move |cc| {
            // Load and register Serpantinum fonts
            let mut fonts = egui::FontDefinitions::default();

            fonts.font_data.insert(
                "adwaita_mono".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/AdwaitaMonoNerdFont-Regular.ttf"
                ))),
            );

            fonts.font_data.insert(
                "nerd_symbols".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/SymbolsNerdFontMono-Regular.ttf"
                ))),
            );

            fonts.font_data.insert(
                "material_symbols".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/MaterialSymbolsRounded.ttf"
                ))),
            );

            // Primary font: Adwaita Mono (Nerd Font) matching Serpantinum
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .clear();

            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .extend([
                    "adwaita_mono".to_owned(),
                    "nerd_symbols".to_owned(),
                    "material_symbols".to_owned(),
                ]);

            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .clear();

            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .extend([
                    "adwaita_mono".to_owned(),
                    "nerd_symbols".to_owned(),
                    "material_symbols".to_owned(),
                ]);

            cc.egui_ctx.set_fonts(fonts);

            // Configure visual styling to match Serpantinum shell
            let mut visuals = egui::Visuals::dark();
            visuals.window_corner_radius = egui::CornerRadius::same(theme.border_radius as u8);
            visuals.menu_corner_radius = egui::CornerRadius::same(12);

            visuals.panel_fill = theme.base;
            visuals.window_fill = theme.surface0;
            visuals.extreme_bg_color = theme.crust;

            cc.egui_ctx.set_visuals(visuals);

            Ok(Box::new(SkiApp::new(initial_path, theme)))
        }),
    )
}

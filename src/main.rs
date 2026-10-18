mod app;
mod fs;
mod keymap;
mod settings;
mod settings_ui;
mod thumbs;
mod ui;

use std::env;
use std::path::PathBuf;
use app::SkiApp;
use eframe::egui;
use ui::theme::SerpantinumTheme;

fn main() -> eframe::Result {
    let settings = settings::Settings::load();
    let theme = SerpantinumTheme::from_settings(&settings);

    let initial_path = if let Some(arg) = env::args().nth(1) {
        PathBuf::from(arg)
    } else {
        match settings.start_location {
            settings::StartLocation::Launch => env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            settings::StartLocation::Home => dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            settings::StartLocation::LastVisited => settings
                .last_dir
                .clone()
                .filter(|p| p.is_dir())
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))),
            settings::StartLocation::Custom => settings
                .start_path
                .clone()
                .filter(|p| p.is_dir())
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))),
        }
    };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ski")
            .with_inner_size([1080.0, 720.0])
            .with_min_inner_size([360.0, 420.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ski-file-manager",
        native_options,
        Box::new(move |cc| {
            ui::fonts::install(&cc.egui_ctx, settings.font.as_deref());
            ui::theme::apply(&cc.egui_ctx, &theme);
            cc.egui_ctx.set_zoom_factor(settings.scale);

            Ok(Box::new(SkiApp::new(initial_path, theme, settings, &cc.egui_ctx)))
        }),
    )
}

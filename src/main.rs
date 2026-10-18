mod app;
mod fs;
mod settings;
mod thumbs;
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

    let settings = settings::Settings::load();
    let theme = SerpantinumTheme::from_settings(&settings);

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

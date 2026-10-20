mod app;
mod chooser;
mod fs;
mod i18n;
mod keymap;
mod portal;
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
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--portal") {
        if let Err(err) = portal::serve() {
            eprintln!("ski: {err}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let chooser = if args.iter().any(|a| a == "--chooser") { chooser::Spec::parse(&args) } else { None };

    let settings = settings::Settings::load();
    i18n::set_language(i18n::resolve(&settings.language));
    let theme = SerpantinumTheme::from_settings(&settings);

    // A file given instead of a folder (a browser's "Show in folder") opens its folder with the file selected
    let mut select = None;
    let given = if chooser.is_some() { None } else { args.first().filter(|a| !a.starts_with("--")).cloned() };
    let initial_path = if let Some(arg) = given {
        let path = PathBuf::from(arg);
        match path.parent() {
            Some(parent) if path.exists() && !path.is_dir() => {
                select = Some(path.clone());
                parent.to_path_buf()
            }
            _ => path,
        }
    } else if let Some(folder) = chooser.as_ref().and_then(|c| c.folder.clone()).filter(|p| p.is_dir()) {
        folder
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

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(chooser.as_ref().and_then(|c| c.title.clone()).unwrap_or_else(|| "Ski".to_owned()))
        .with_inner_size([1080.0, 720.0])
        .with_min_inner_size([360.0, 420.0]);
    if chooser.is_some() {
        // A dialog: smaller, and with an app id of its own so window rules can float and center it
        viewport = viewport.with_inner_size([900.0, 600.0]).with_app_id("ski-chooser");
    }
    let native_options = eframe::NativeOptions {
        viewport,
        // Honoured on X11 and Windows; Wayland leaves the position to the compositor
        centered: chooser.is_some(),
        ..Default::default()
    };

    eframe::run_native(
        "ski-file-manager",
        native_options,
        Box::new(move |cc| {
            ui::fonts::install(&cc.egui_ctx, settings.font.as_deref());
            ui::theme::apply(&cc.egui_ctx, &theme);
            cc.egui_ctx.set_zoom_factor(settings.scale);

            let mut app = SkiApp::new(initial_path, theme, settings, &cc.egui_ctx);
            if let Some(path) = select {
                app.select_path(&path);
            }
            app.chooser = chooser.map(chooser::Chooser::new);
            Ok(Box::new(app))
        }),
    )
}

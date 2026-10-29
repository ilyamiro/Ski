mod app;
mod chooser;
mod clipboard;
mod fs;
mod i18n;
mod instance;
mod keymap;
mod plugins;
mod portal;
mod settings;
mod settings_ui;
mod thumbs;
mod ui;
mod wayland_dnd;
mod wayland_parent;
mod x11_parent;

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
    if args.first().is_some_and(|a| a == "--resident") {
        instance::serve(open_window);
        return Ok(());
    }
    let request = instance::Request::current(args);
    // A file dialog belongs to its requester, and an administrator's window to its own process.
    // As a container's first process (the Docker image), leaving would stop the container.
    if !request.args.iter().any(|a| a == "--chooser") && !fs::admin::is_admin() && std::process::id() != 1 {
        if !settings::Settings::load().run_in_background {
            instance::dismiss(&request);
        } else if instance::hand_off(&request) {
            return Ok(());
        }
    }
    open_window(request)
}

/// Open a window for one launch and run it until it closes.
fn open_window(mut request: instance::Request) -> eframe::Result {
    let args = std::mem::take(&mut request.args);
    let chooser = if args.iter().any(|a| a == "--chooser") { chooser::Spec::parse(&args) } else { None };

    let settings = settings::Settings::load();
    i18n::set_language(i18n::resolve(&settings.language));
    let theme = SerpantinumTheme::from_settings(&settings);

    // A file given instead of a folder (a browser's "Show in folder") opens its folder with the file selected
    let mut select = None;
    // Tabs of the last session, which only a normal launch without a folder argument brings back
    let mut session = None;
    let given = if chooser.is_some() { None } else { args.first().filter(|a| !a.starts_with("--")).cloned() };
    // Several items dropped on the launcher at once: the first fills this window, each other gets its own
    if chooser.is_none() {
        for extra in args.iter().skip(1).filter(|a| !a.starts_with("--")) {
            let _ = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg(extra).current_dir(&request.cwd).spawn());
        }
    }
    // Tabs to keep beside a folder given on the command line, so that launch does not replace the saved session
    let mut kept_session = None;
    let initial_path = if let Some(arg) = given {
        if chooser.is_none() && settings.start_location == settings::StartLocation::RestoreTabs {
            kept_session = Some(settings.restorable_session().0);
        }
        // A launcher passes what was dropped on it as `file://` URIs
        let path = chooser::from_uri(&arg).unwrap_or_else(|| PathBuf::from(arg));
        // Relative to where the launch was made, which a resident process does not share
        let path = if fs::places::is_virtual(&path) { path } else { request.cwd.join(path) };
        match path.parent() {
            Some(parent) if path.exists() && !path.is_dir() => {
                select = Some(path.clone());
                parent.to_path_buf()
            }
            _ => path,
        }
    } else if let Some(folder) = chooser.as_ref().and_then(|c| c.folder.clone().or_else(|| chooser::remembered_folder(c.app_id.as_deref()))).filter(|p| p.is_dir()) {
        folder
    } else {
        match settings.start_location {
            settings::StartLocation::Launch => request.cwd.clone(),
            settings::StartLocation::Home => dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            settings::StartLocation::LastVisited => settings
                .last_dir
                .clone()
                .filter(|p| p.is_dir())
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))),
            settings::StartLocation::RestoreTabs => {
                let (paths, active) = settings.restorable_session();
                match paths.first().cloned() {
                    Some(first) => {
                        session = Some((paths, active));
                        first
                    }
                    None => request.cwd.clone(),
                }
            }
            settings::StartLocation::Custom => settings
                .start_path
                .clone()
                .filter(|p| p.is_dir())
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))),
        }
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(chooser.as_ref().and_then(|c| c.title.clone()).unwrap_or_else(|| if fs::admin::is_admin() { format!("Ski ({})", i18n::tr("Administrator")) } else { "Ski".to_owned() }))
        .with_inner_size([1080.0, 720.0])
        .with_min_inner_size([360.0, 420.0])
        // The header draws its own close button; compositors without server-side decorations (niri) would add a bar
        .with_decorations(false)
        .with_transparent(true);
    if let Some(spec) = &chooser {
        viewport = viewport.with_inner_size(chooser::saved_size().unwrap_or(chooser::DEFAULT_SIZE)).with_app_id("ski-chooser");
        // Tied to its requester (see wayland_parent.rs) it is floated and centered as a dialog and can
        // stay resizable; without a parent, a fixed size is what makes tiling compositors float it
        if !spec.parent.as_deref().is_some_and(|p| p.starts_with("wayland:") || p.starts_with("x11:")) {
            viewport = viewport.with_resizable(false);
        }
    }
    let native_options = eframe::NativeOptions {
        viewport,
        // Honoured on X11 and Windows; Wayland leaves the position to the compositor
        centered: chooser.is_some(),
        // Starts several times faster than wgpu, which has to bring up Vulkan first
        renderer: eframe::Renderer::Glow,
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
            if fs::admin::is_admin() {
                app.set_toast(i18n::tr("Running as administrator. Changes to settings are not saved."));
            }
            if let Some((paths, active)) = session.filter(|_| chooser.is_none()) {
                app.restore_tabs(&paths, active);
            }
            if let Some(paths) = kept_session.filter(|_| chooser.is_none()) {
                app.add_session_tabs(&paths);
            }
            if let Some(path) = select {
                app.select_path(&path);
            }
            app.chooser = chooser.map(|spec| {
                if let Some(handle) = spec.parent.as_deref() {
                    x11_parent::attach(cc, handle);
                }
                let parent = spec.parent.as_deref().and_then(|handle| wayland_parent::attach(cc, handle));
                // A dialog opens as a list with sortable column headers, whatever the file manager last used
                app.view_mode = app::ViewMode::List;
                let mut chooser = chooser::Chooser::new(spec);
                chooser.parent = parent;
                chooser
            });
            // The folder was listed before the chooser existed, and the chooser's filter applies to it
            if app.chooser.is_some() {
                app.reload_items();
            }
            if app.chooser.is_none() {
                app.wl_dnd = wayland_dnd::WaylandDnd::new(cc, &cc.egui_ctx);
            }
            request.shown();
            Ok(Box::new(app))
        }),
    )
}

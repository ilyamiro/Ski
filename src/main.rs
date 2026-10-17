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
            .with_min_inner_size([360.0, 420.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ski-file-manager",
        native_options,
        Box::new(move |cc| {
            let mut fonts = egui::FontDefinitions::default();

            // Text: Adwaita Sans, in front of egui's defaults (kept as fallback for rare glyphs)
            fonts.font_data.insert(
                "adwaita_sans".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/AdwaitaSans-Regular.ttf"
                ))),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "adwaita_sans".to_owned());

            // Icons: the same Symbols Nerd Font Mono Serpantinum pins, in a family of its own
            // with no fallback, so every icon comes from one font with identical metrics.
            fonts.font_data.insert(
                "nerd_symbols".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/SymbolsNerdFontMono-Regular.ttf"
                ))),
            );
            fonts.families.insert(
                egui::FontFamily::Name(ui::icons::FAMILY.into()),
                vec!["nerd_symbols".to_owned()],
            );

            cc.egui_ctx.set_fonts(fonts);

            // Flat Material look: no strokes anywhere, state is shown by fills only
            let mut visuals = egui::Visuals::dark();
            visuals.window_corner_radius = egui::CornerRadius::same(24);
            visuals.menu_corner_radius = egui::CornerRadius::same(16);
            visuals.window_stroke = egui::Stroke::NONE;
            visuals.popup_shadow = egui::Shadow::NONE;
            visuals.window_shadow = egui::Shadow {
                offset: [0, 8],
                blur: 32,
                spread: 0,
                color: egui::Color32::from_black_alpha(110),
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
            cc.egui_ctx.set_visuals(visuals);

            cc.egui_ctx.all_styles_mut(|style| {
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

            Ok(Box::new(SkiApp::new(initial_path, theme)))
        }),
    )
}

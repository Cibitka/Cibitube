mod app;
mod browser_auth;
mod discord;
mod downloader;
mod innertube;
mod library;
mod lyrics;
mod models;
mod player;
mod settings;

use app::MetrolistApp;
use eframe::egui::Vec2;

fn setup_custom_fonts(ctx: &eframe::egui::Context) {
    let mut fonts = eframe::egui::FontDefinitions::default();

    // Load DejaVuSans.ttf for complete unicode symbols (arrows, media buttons, hearts, musical notes)
    if let Ok(bytes) = std::fs::read("/usr/share/fonts/TTF/DejaVuSans.ttf") {
        fonts.font_data.insert(
            "dejavu_sans".to_owned(),
            std::sync::Arc::new(eframe::egui::FontData::from_owned(bytes)),
        );
        if let Some(prop) = fonts.families.get_mut(&eframe::egui::FontFamily::Proportional) {
            prop.push("dejavu_sans".to_owned());
        }
        if let Some(mono) = fonts.families.get_mut(&eframe::egui::FontFamily::Monospace) {
            mono.push("dejavu_sans".to_owned());
        }
    }

    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result<()> {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Cibitube — YouTube Music Client")
            .with_app_id("cibitube")
            .with_inner_size(Vec2::new(1040.0, 700.0))
            .with_min_inner_size(Vec2::new(850.0, 520.0)),
        ..Default::default()
    };

    eframe::run_native(
        "Cibitube",
        native_options,
        Box::new(|cc| {
            setup_custom_fonts(&cc.egui_ctx);
            Ok(Box::new(MetrolistApp::new(cc)))
        }),
    )
}

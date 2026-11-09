mod app;
mod browser_auth;
mod discord;
mod donate;
mod downloader;
mod i18n;
mod innertube;
mod library;
mod lyrics;
mod models;
mod player;
mod settings;
mod sync;
mod updater;

use app::MetrolistApp;
use eframe::egui::Vec2;
use std::sync::Arc;

fn load_app_icon() -> Option<Arc<eframe::egui::IconData>> {
    let bytes = include_bytes!("../assets/icon.png");
    if let Ok(img) = image::load_from_memory(bytes) {
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        Some(Arc::new(eframe::egui::IconData {
            rgba: rgba.into_raw(),
            width,
            height,
        }))
    } else {
        None
    }
}

fn setup_custom_fonts(ctx: &eframe::egui::Context) {
    let mut fonts = eframe::egui::FontDefinitions::default();

    // Load DejaVuSans.ttf for complete unicode symbols (arrows, media buttons, hearts, musical notes)
    if let Ok(bytes) = std::fs::read("/usr/share/fonts/TTF/DejaVuSans.ttf") {
        fonts.font_data.insert(
            "dejavu_sans".to_owned(),
            std::sync::Arc::new(eframe::egui::FontData::from_owned(bytes)),
        );
        if let Some(prop) = fonts
            .families
            .get_mut(&eframe::egui::FontFamily::Proportional)
        {
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

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Cibitube — YouTube Music Client")
        .with_app_id("cibitube")
        .with_inner_size(Vec2::new(1040.0, 700.0))
        .with_min_inner_size(Vec2::new(850.0, 520.0));

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
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

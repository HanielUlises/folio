#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[cfg(feature = "drive")]
mod drive;
mod engine;
mod icon;
mod integrate;
mod model;
mod pdf;

use eframe::egui;

/// Decode the bundled PNG into an eframe window icon (used for the title bar,
/// taskbar and window-switcher thumbnail on every platform).
fn window_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../icons/128x128.png")).ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(egui::IconData { rgba: rgba.into_raw(), width, height })
}

fn main() -> eframe::Result<()> {
    // Register in the OS app menu on first run (best-effort, per-user).
    integrate::ensure_registered();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([900.0, 600.0])
        .with_title("Folio");
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Folio",
        native_options,
        Box::new(|cc| Ok(Box::new(app::Folio::new(cc)))),
    )
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod engine;
mod icon;
mod model;
mod pdf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Folio"),
        ..Default::default()
    };

    eframe::run_native(
        "Folio",
        native_options,
        Box::new(|cc| Ok(Box::new(app::Folio::new(cc)))),
    )
}

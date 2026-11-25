//! Aureate — a local Markdown writer that only exports PDF.
//! Pure Rust: egui interface, Typst typesetting, no webview, no JavaScript.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod fonts;
mod ops;
mod prefs;
mod preview;
mod state;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Aureate")
            .with_inner_size([1320.0, 860.0])
            .with_min_inner_size([980.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Aureate",
        options,
        Box::new(|cc| Ok(Box::new(app::AureateApp::new(cc)))),
    )
}

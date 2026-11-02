#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apc;
mod app;
mod core;
mod dmx;
mod engine;
mod fixture;
mod shot;
mod tempo;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1360.0, 820.0]).with_min_inner_size([900.0, 600.0]).with_title("SPOT4 — ムービング ×2 + RGBWA LED ×2"),
        ..Default::default()
    };
    eframe::run_native("spot4", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}

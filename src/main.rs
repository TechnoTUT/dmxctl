#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apcmini;
mod app;
mod color;
mod engine;
#[cfg(test)]
mod engine_tests;
mod fixture;
mod led;
mod link;
mod look;
#[cfg(test)]
mod bpm_tests;
#[cfg(test)]
mod color_tests;
#[cfg(test)]
mod led_tests;
#[cfg(test)]
mod master_tests;
#[cfg(test)]
mod perf_tests;
#[cfg(test)]
mod pair_tests;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod pixel_tests;
#[cfg(test)]
mod palette_tests;
#[cfg(test)]
mod sync_tests;
mod midi;
mod opendmx;
mod osc;
mod output;
mod pixel;
mod project;
mod ui_binding;
mod ui_control;
mod ui_map;
mod ui_midi;
mod ui_output;
mod ui_pair;
mod ui_patch;
mod ui_profile;
mod ui_sequence;
mod ui_show;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_title("DMX Controller"),
        ..Default::default()
    };
    eframe::run_native("dmxctl", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}

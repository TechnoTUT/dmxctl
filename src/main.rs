#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apcmini;
mod app;
mod color;
mod engine;
#[cfg(test)]
mod engine_tests;
mod fixture;
mod layout;
#[cfg(test)]
mod layout_tests;
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
mod shot;
mod theme;
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
    // `dmxctl --apply-layout`: 保存してあるプロジェクトを、おすすめのMIDI配置にする(画面は開かない)
    if std::env::args().any(|a| a == "--apply-layout") {
        if let Err(e) = layout::cli() {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    // 開発・確認用: DMXCTL_SIZE=幅x高さ でウィンドウの大きさを指定
    let size = std::env::var("DMXCTL_SIZE").ok().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some([w.parse::<f32>().ok()?, h.parse::<f32>().ok()?])
    });
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size(size.unwrap_or([1280.0, 800.0])).with_min_inner_size([960.0, 620.0]).with_title("DMX Controller"),
        ..Default::default()
    };
    eframe::run_native("dmxctl", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}

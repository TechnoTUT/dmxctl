#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apcmini;
mod audio;
mod app;
#[cfg(test)]
mod binding_tests;
mod color;
mod engine;
#[cfg(test)]
mod engine_tests;
mod fixture;
mod keys;
#[cfg(all(target_os = "linux", feature = "kmsdrm"))]
mod kmsdrm;
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
mod safety_tests;
#[cfg(test)]
mod sync_tests;
mod midi;
mod motion;
mod ndi;
mod opendmx;
mod osc;
mod output;
mod pixel;
mod project;
mod shot;
mod theme;
mod ui_binding;
mod ui_control;
mod ui_keys;
mod ui_laser;
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
    // 落ちたとき・終了のシグナルを受けたときに、安全な値(レーザーOFF・明るさ0)を送ってから終わる
    engine::install_safety_hooks();
    // デスクトップ環境の無いLinux(またはDMXCTL_KMSDRM=1 / --kmsdrm)では、画面に直接(KMS/DRM)出す
    #[cfg(all(target_os = "linux", feature = "kmsdrm"))]
    if kmsdrm::wanted() {
        if let Err(e) = kmsdrm::run() {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    // kmsdrm機能なしで作ったものを、デスクトップ環境の無いLinuxで起動したとき(下のeframeは分かりにくいエラーで終わる)
    #[cfg(all(target_os = "linux", not(feature = "kmsdrm")))]
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("デスクトップ環境(X11 / Wayland)が見つかりません。画面に直接出すには、kmsdrm機能つきで作ってください(./scripts/build-linux.sh)");
    }
    // 開発・確認用: DMXCTL_SIZE=幅x高さ でウィンドウの大きさを指定
    let size = std::env::var("DMXCTL_SIZE").ok().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some([w.parse::<f32>().ok()?, h.parse::<f32>().ok()?])
    });
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size(size.unwrap_or([1280.0, 800.0])).with_min_inner_size([960.0, 620.0]).with_title("DMX Controller").with_app_id("dmxctl"),
        ..Default::default()
    };
    eframe::run_native("dmxctl", options, Box::new(|cc| Ok(Box::new(app::App::new(&cc.egui_ctx)))))
}

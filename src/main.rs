#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dmxctl::{app, engine, layout};
#[cfg(all(target_os = "linux", feature = "kmsdrm"))]
use dmxctl::kmsdrm;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg|arg == "serve" || arg == "ctl") {
        if let Err(error) = dmxctl::cli::run(args) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let launch = dmxctl::control::Launch::parse(&args, false).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(1);
    });
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
    // 画面を出さずに動かす(モニターの無いPCで、Beat Link Triggerなどと一緒にオートで動かす)
    if std::env::args().any(|a| a == "--headless") || std::env::var_os("DMXCTL_HEADLESS").is_some_and(|v| v == "1") {
        app::App::run_headless_with_options(launch);
    }
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
        eprintln!("デスクトップ環境(X11 / Wayland)が見つかりません。画面に直接出すには、同梱の dmxctl-kmsdrm を使ってください(./scripts/build-linux.sh で作れます)");
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
    eframe::run_native("dmxctl", options, Box::new(move |cc| {
        app::App::with_options(&cc.egui_ctx, false, launch)
            .map(|app|Box::new(app) as Box<dyn eframe::App>).map_err(|error|error.into())
    }))
}

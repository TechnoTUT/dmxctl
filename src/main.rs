#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dmxctl::core::engine;
use dmxctl_backend::entry::{self, Mode};
use dmxctl_ui::app;
#[cfg(target_os = "linux")]
use dmxctl_ui::kmsdrm;

const USAGE: &str = "dmxctl [--project FILE] [--control] [--listen IP:PORT] [--output none|live] [--inputs none|configured] [--no-autosave]
    画面を開いて起動する(引数なし)
dmxctl headless [同じオプション]
    画面を出さずに動かす(OSC受信・Ableton Link・オートを必ずONにする)
dmxctl apply-layout
    保存してあるプロジェクトを、おすすめのMIDI配置にする(先にバックアップを残す)
dmxctl kmsdrm
    デスクトップ環境の無いLinuxで、画面に直接(KMS/DRM)出す(Linuxだけ)
dmxctl connect HOST[:PORT]
    サーバー(別のPCの dmxctl)につなぎ、画面とMIDIコントローラーだけを手元で動かす(ポートの既定は 9300)
    サーバーは --remote-listen ADDR を付けて起動する(例: dmxctl headless --remote-listen 192.168.1.10)
    クライアントから機材定義の安全の設定も変えるなら、サーバーに --remote-allow-safety-edit も付ける
dmxctl carabiner install
    Ableton Link に参加する Carabiner を、配布元から取って、設定フォルダに入れる。Linkを使うときに1回だけ要る
dmxctl serve|ctl|inspect|simulate|run|watch|log|diff ...
    自動操作・検証用のCLI(JSONで返す)。詳しくは下

以前の書き方(--headless / --apply-layout / --kmsdrm / DMXCTL_HEADLESS=1)も使えます。
";

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, rest) = Mode::of(&args);
    if !matches!(mode, Mode::Gui) {
        entry::attach_console();
    }
    if entry::run_noninteractive(&mode, rest, USAGE) {
        return Ok(());
    }
    let launch = entry::parse_launch(&mode, rest);
    // 画面を出さずに動かす(モニターの無いPCで、Beat Link Triggerなどと一緒にオートで動かす)
    if matches!(mode, Mode::Headless) {
        entry::run_headless(launch);
    }
    // 落ちたとき・終了のシグナルを受けたときに、安全な値(レーザーOFF・明るさ0)を送ってから終わる
    engine::install_safety_hooks();
    // デスクトップ環境の無いLinux(または kmsdrm / DMXCTL_KMSDRM=1)では、画面に直接(KMS/DRM)出す
    #[cfg(target_os = "linux")]
    if kmsdrm::wanted(matches!(mode, Mode::Kmsdrm)) {
        if let Err(e) = kmsdrm::run(launch.clone()) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    if matches!(mode, Mode::Kmsdrm) {
        eprintln!("kmsdrm は Linux だけで使えます");
        std::process::exit(1);
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

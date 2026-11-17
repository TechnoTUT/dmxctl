#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dmxctl::{app, engine, layout};
#[cfg(target_os = "linux")]
use dmxctl::kmsdrm;

const USAGE: &str = "dmxctl [--project FILE] [--control] [--listen IP:PORT] [--output none|live] [--inputs none|configured] [--no-autosave]
    画面を開いて起動する(引数なし)
dmxctl headless [同じオプション]
    画面を出さずに動かす(OSC受信・Ableton Link・オートを必ずONにする)
dmxctl apply-layout
    保存してあるプロジェクトを、おすすめのMIDI配置にする(先にバックアップを残す)
dmxctl kmsdrm
    デスクトップ環境の無いLinuxで、画面に直接(KMS/DRM)出す(Linuxだけ)
dmxctl serve|ctl|inspect|simulate|run|watch|log|diff ...
    自動操作・検証用のCLI(JSONで返す)。詳しくは下

以前の書き方(--headless / --apply-layout / --kmsdrm / DMXCTL_HEADLESS=1)も使えます。
";

/// 最初の引数で決まる動かし方
enum Mode { Gui, Headless, ApplyLayout, Kmsdrm, Cli, Help }

impl Mode {
    fn of(args: &[String]) -> (Self, &[String]) {
        let rest = args.get(1..).unwrap_or_default();
        match args.first().map(String::as_str) {
            Some("help" | "--help" | "-h") => (Self::Help, rest),
            Some("headless") => (Self::Headless, rest),
            Some("apply-layout") => (Self::ApplyLayout, rest),
            Some("kmsdrm") => (Self::Kmsdrm, rest),
            Some(command) if dmxctl::cli::COMMANDS.contains(&command) => (Self::Cli, args),
            // 以前の書き方(フラグ・環境変数)。systemd のサービスなど、インストール済みのものが使っている
            _ if args.iter().any(|a| a == "--apply-layout") => (Self::ApplyLayout, args),
            _ if args.iter().any(|a| a == "--headless") || std::env::var_os("DMXCTL_HEADLESS").is_some_and(|v| v == "1") => (Self::Headless, args),
            _ if args.iter().any(|a| a == "--kmsdrm") => (Self::Kmsdrm, args),
            _ => (Self::Gui, args),
        }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, rest) = Mode::of(&args);
    if !matches!(mode, Mode::Gui) {
        attach_console();
    }
    match mode {
        Mode::Help => {
            println!("{USAGE}\n{}", dmxctl::cli::HELP);
            return Ok(());
        }
        Mode::Cli => {
            if let Err(error) = dmxctl::cli::run(rest.to_vec()) {
                eprintln!("{}", serde_json::json!({"error": error.to_string()}));
                std::process::exit(1);
            }
            return Ok(());
        }
        Mode::ApplyLayout => {
            if let Err(e) = layout::cli() {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Mode::Gui | Mode::Headless | Mode::Kmsdrm => {}
    }
    let launch = dmxctl::control::Launch::parse(rest, false).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(1);
    });
    // 落ちたとき・終了のシグナルを受けたときに、安全な値(レーザーOFF・明るさ0)を送ってから終わる
    engine::install_safety_hooks();
    // 画面を出さずに動かす(モニターの無いPCで、Beat Link Triggerなどと一緒にオートで動かす)
    if matches!(mode, Mode::Headless) {
        app::App::run_headless_with_options(launch);
    }
    // デスクトップ環境の無いLinux(または kmsdrm / DMXCTL_KMSDRM=1)では、画面に直接(KMS/DRM)出す
    #[cfg(target_os = "linux")]
    if kmsdrm::wanted(matches!(mode, Mode::Kmsdrm)) {
        if let Err(e) = kmsdrm::run() {
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

/// Windowsのリリース版は画面用(windows_subsystem = "windows")なので、コンソールが付かない。
/// 端末から CLI・画面なしで起動したときは、起動元の端末に出力をつなぐ。
/// (パイプやファイルに出力を向けているときは、もともとつながっているので何もしない)
#[cfg(windows)]
fn attach_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // 起動元に端末が無い(エクスプローラーから起動など)ときは失敗するが、そのままでよい
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS); }
}

#[cfg(not(windows))]
fn attach_console() {}

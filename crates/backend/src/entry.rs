//! 起動の振り分け。画面付きの `dmxctl` と、画面なしの `dmxctl-server` が同じ規則で最初の引数を読む。
//! 画面を出す動かし方 (`Gui`・`Kmsdrm`・`Connect`) は、画面を持つ側だけが実行する。
use crate::control::Launch;
use std::io::Write;

/// 最初の引数で決まる動かし方
pub enum Mode {
    Gui,
    Headless,
    ApplyLayout,
    Kmsdrm,
    Connect,
    Cli,
    Help,
}

impl Mode {
    pub fn of(args: &[String]) -> (Self, &[String]) {
        let rest = args.get(1..).unwrap_or_default();
        match args.first().map(String::as_str) {
            Some("help" | "--help" | "-h") => (Self::Help, rest),
            Some("headless") => (Self::Headless, rest),
            Some("apply-layout") => (Self::ApplyLayout, rest),
            Some("kmsdrm") => (Self::Kmsdrm, rest),
            Some("connect") => (Self::Connect, rest),
            Some(command) if crate::cli::COMMANDS.contains(&command) => (Self::Cli, args),
            // 以前の書き方(フラグ・環境変数)。systemd のサービスなど、インストール済みのものが使っている
            _ if args.iter().any(|a| a == "--apply-layout") => (Self::ApplyLayout, args),
            _ if args.iter().any(|a| a == "--headless") || std::env::var_os("DMXCTL_HEADLESS").is_some_and(|v| v == "1") => (Self::Headless, args),
            _ if args.iter().any(|a| a == "--kmsdrm") => (Self::Kmsdrm, args),
            _ => (Self::Gui, args),
        }
    }

    /// 画面を出さない動かし方か
    pub fn is_interactive(&self) -> bool {
        matches!(self, Mode::Gui | Mode::Kmsdrm | Mode::Connect)
    }
}

/// 画面を使わずに終わる動かし方 (ヘルプ・CLI・おすすめ配置) を実行する。実行したら `true`
pub fn run_noninteractive(mode: &Mode, rest: &[String], usage: &str) -> bool {
    match mode {
        Mode::Help => {
            if let Err(error) = crate::cli::stdout_line(format_args!("{usage}\n{}", crate::cli::HELP)) {
                let _ = writeln!(std::io::stderr().lock(), "{error}");
                std::process::exit(1);
            }
        }
        Mode::Cli => {
            if let Err(error) = crate::cli::run(rest.to_vec()) {
                let _ = writeln!(std::io::stderr().lock(), "{}", serde_json::json!({"error": error.to_string()}));
                std::process::exit(1);
            }
        }
        Mode::ApplyLayout => {
            if let Err(e) = dmxctl_core::layout::cli() {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Mode::Gui | Mode::Headless | Mode::Kmsdrm | Mode::Connect => return false,
    }
    true
}

/// 起動オプションを読む。`connect` では最初の引数が接続先
pub fn parse_launch(mode: &Mode, rest: &[String]) -> Launch {
    let (target, rest) = match mode {
        Mode::Connect => match rest.split_first() {
            Some((t, r)) if !t.starts_with("--") => (Some(t.clone()), r),
            _ => {
                eprintln!("接続先を指定してください: dmxctl connect HOST[:PORT]");
                std::process::exit(1);
            }
        },
        _ => (None, rest),
    };
    let mut launch = Launch::parse(rest, false).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(1);
    });
    launch.connect = target;
    launch
}

/// 自分自身を、別の引数で起動し直す。呼ぶ側は、先に自分を終了させる。
/// Windows と Mac の利用者が、端末で引数を付けずに `connect` などへ切り替えるために利用する
pub fn spawn_self(args: &[String]) -> std::io::Result<()> {
    std::process::Command::new(std::env::current_exe()?).args(args).spawn().map(|_| ())
}

/// 画面なしで動かす。落ちたとき・終了のシグナルで、安全な値を送ってから終わる。戻らない
pub fn run_headless(launch: Launch) -> ! {
    dmxctl_core::engine::install_safety_hooks();
    crate::host::Host::run_headless(launch)
}

/// Windowsのリリース版は画面用なので、コンソールが付かない。
/// 端末から CLI・画面なしで起動したときは、起動元の端末に出力をつなぐ。
/// パイプやファイルに出力を向けているときは、もともとつながっているので何もしない
#[cfg(windows)]
pub fn attach_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // 起動元に端末が無い(エクスプローラーから起動など)ときは失敗するが、そのままでよい
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS); }
}

#[cfg(not(windows))]
pub fn attach_console() {}

//! 画面なしの実行ファイル `dmxctl-server` が、画面の要らない動かし方だけを受け付けることを確かめる。
//! 本番の設定に触れないよう、設定のフォルダは毎回別にする。

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    let dir = std::env::temp_dir().join(format!("dmxctl-server-test-{}-{}", std::process::id(), args.join("_").len()));
    std::fs::create_dir_all(&dir).unwrap();
    Command::new(env!("CARGO_BIN_EXE_dmxctl-server"))
        .args(args)
        .env("DMXCTL_CONFIG_DIR", &dir)
        .env("DMXCTL_NO_MIDI", "1")
        .output()
        .unwrap()
}

#[test]
fn help_lists_only_headless_modes() {
    let out = run(&["help"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("dmxctl-server headless"));
    assert!(!text.contains("dmxctl kmsdrm"));
}

#[test]
fn modes_that_need_a_screen_are_refused() {
    for args in [&[][..], &["kmsdrm"], &["connect", "127.0.0.1"]] {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("画面を持ちません"), "{args:?}");
    }
}

#[test]
fn cli_commands_run_without_a_screen() {
    let project = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/cli/project.json");
    let out = run(&["inspect", "--project", project]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(json["fixtures"].is_array());
}

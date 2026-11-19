//! クレートの依存の向きを守っているかを調べる。参照してよいのは ui → backend → core だけ。
//! ソースの参照はコンパイラが止めるので、ここでは Cargo.toml に余計な依存が入っていないかを見る。

use std::fs;
use std::path::Path;

/// `[dependencies]` 系の表に書かれたクレート名を集める
fn dependencies(krate: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates").join(krate).join("Cargo.toml");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut names = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line.contains("dependencies");
            continue;
        }
        if in_deps && let Some((name, _)) = line.split_once('=') {
            names.push(name.trim().to_string());
        }
    }
    names
}

fn assert_without(krate: &str, forbidden: &[&str]) {
    let deps = dependencies(krate);
    let found: Vec<_> = forbidden.iter().filter(|f| deps.iter().any(|d| d == *f)).collect();
    assert!(found.is_empty(), "{krate} が依存してはいけないクレートを使っている: {found:?}");
}

/// 画面を出すためのクレート。画面の層にだけ置く
const GUI: [&str; 6] = ["eframe", "egui", "egui_glow", "glow", "sdl2", "rfd"];

/// 機器とやりとりするためのクレート。モデル層には置かない
const DEVICE: [&str; 6] = ["cpal", "midir", "serialport", "libloading", "moip-ndi", "sha2"];

#[test]
fn model_depends_on_nothing_above_it() {
    assert_without("model", &["dmxctl-core", "dmxctl-backend", "dmxctl-ui"]);
    assert_without("model", &GUI);
    assert_without("model", &DEVICE);
}

#[test]
fn core_depends_on_neither_upper_layers_nor_gui() {
    assert_without("core", &["dmxctl-backend", "dmxctl-ui"]);
    assert_without("core", &GUI);
}

#[test]
fn backend_depends_on_neither_ui_nor_gui() {
    assert_without("backend", &["dmxctl-ui"]);
    assert_without("backend", &GUI);
}

#[test]
fn server_depends_on_neither_ui_nor_gui() {
    assert_without("server", &["dmxctl-ui"]);
    assert_without("server", &GUI);
}

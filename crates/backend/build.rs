//! 操作パネルの画面 (リポジトリ直下の web/ にある React) を単一のHTMLにビルドし、OUT_DIR/web/index.html に置く。
//!
//! Node.js と npm が無い環境でも、ビルドとテストは止めない。そのときは、未ビルドであることを知らせるページを埋め込む。
//! 配布物では未ビルドのページが入らないよう、`DMXCTL_REQUIRE_WEB=1` なら npm が無いことをエラーにする。
//! `DMXCTL_CHECK_ONLY=1` の型チェックでは npm を呼ばない。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="ja"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>DMX Controller</title></head>
<body style="font-family:sans-serif;padding:24px">
<h1>操作パネルは未ビルドです</h1>
<p>このビルドには、操作パネルの画面が含まれていません。Node.js 20 系は 20.19 以降、または 22.12 以降を導入してから、もう一度ビルドしてください。</p>
<p>API の <code>/api/state</code> と <code>/api/cmd</code> は利用できます。</p>
</body></html>
"#;

fn env_set(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0")
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// npm を実行する。起動できない (npm が無い) ときは Ok(false)、実行して失敗したときは panic
fn npm(web: &Path, args: &[&str], out: Option<&Path>) -> bool {
    // Windows では npm の実体が npm.cmd
    let program = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(web);
    if let Some(out) = out {
        cmd.env("DMXCTL_WEB_OUT", out);
    }
    match cmd.status() {
        Ok(s) if s.success() => true,
        Ok(s) => panic!("`npm {}` failed ({s}). The web panel in web/ could not be built.", args.join(" ")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => panic!("cannot run npm: {e}"),
    }
}

/// node_modules が package-lock.json より古い (git pull で依存が変わった) ときも入れ直す
fn deps_stale(web: &Path) -> bool {
    let installed = web.join("node_modules/.package-lock.json");
    match (modified(&installed), modified(&web.join("package-lock.json"))) {
        (Some(i), Some(l)) => i < l,
        (None, _) => true,
        _ => false,
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let web = manifest.join("../../web").canonicalize().expect("web/ directory not found");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("web");

    for path in ["src", "index.html", "package.json", "package-lock.json", "tsconfig.json", "vite.config.ts"] {
        println!("cargo:rerun-if-changed={}", web.join(path).display());
    }
    for name in ["DMXCTL_CHECK_ONLY", "DMXCTL_REQUIRE_WEB", "PATH"] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let built = if env_set("DMXCTL_CHECK_ONLY") {
        false
    } else {
        (!deps_stale(&web) || npm(&web, &["ci"], None)) && npm(&web, &["run", "build"], Some(&out))
    };
    if built {
        return;
    }
    if env_set("DMXCTL_REQUIRE_WEB") {
        panic!("npm was not found, but DMXCTL_REQUIRE_WEB is set. Install Node.js 20.19+ (20.x) or 22.12+ to build the web panel in web/. See docs/development.md.");
    }
    if !env_set("DMXCTL_CHECK_ONLY") {
        println!("cargo:warning=npm was not found. The web panel in web/ is not built, and a placeholder page is embedded. Install Node.js 20.19+ (20.x) or 22.12+ to build it.");
    }
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("index.html"), PLACEHOLDER).unwrap();
}

//! 操作パネルの画面 (リポジトリ直下の web/ にある React) を単一のHTMLにビルドし、OUT_DIR/web/index.html に置く。
//! Node.js と npm が必要。

use std::path::PathBuf;
use std::process::Command;

fn npm(web: &PathBuf, args: &[&str], out: Option<&PathBuf>) {
    // Windows では npm の実体が npm.cmd
    let program = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(web);
    if let Some(out) = out {
        cmd.env("DMXCTL_WEB_OUT", out);
    }
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("`npm {}` failed ({s}). The web panel in web/ could not be built.", args.join(" ")),
        Err(e) => panic!("cannot run npm ({e}). Node.js 20 or later is required to build the web panel in web/. See docs/development.md."),
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let web = manifest.join("../../web").canonicalize().expect("web/ directory not found");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("web");

    for path in ["src", "index.html", "package.json", "package-lock.json", "tsconfig.json", "vite.config.ts"] {
        println!("cargo:rerun-if-changed={}", web.join(path).display());
    }

    if !web.join("node_modules").is_dir() {
        npm(&web, &["ci"], None);
    }
    npm(&web, &["run", "build"], Some(&out));
}

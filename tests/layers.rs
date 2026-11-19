//! 層の向きを守っているかを調べる。参照してよいのは `ui` → `backend` → `core` だけ。

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `layer` の中のコードが、`forbidden` の層を参照していたら、その場所を返す
fn violations(layer: &str, forbidden: &[&str]) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![root.join(format!("{layer}.rs"))];
    rust_files(&root.join(layer), &mut files);
    let mut found = Vec::new();
    for f in files {
        let Ok(text) = fs::read_to_string(&f) else { continue };
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for bad in forbidden {
                if code.contains(&format!("crate::{bad}")) || code.contains(&format!("dmxctl::{bad}")) {
                    found.push(format!("{}:{}: {}", f.strip_prefix(&root).unwrap().display(), i + 1, line.trim()));
                }
            }
        }
    }
    found
}

#[test]
fn core_does_not_depend_on_backend_or_ui() {
    let v = violations("core", &["backend", "ui"]);
    assert!(v.is_empty(), "core が上の層を参照している:\n{}", v.join("\n"));
}

#[test]
fn backend_does_not_depend_on_ui() {
    let v = violations("backend", &["ui"]);
    assert!(v.is_empty(), "backend が ui を参照している:\n{}", v.join("\n"));
}

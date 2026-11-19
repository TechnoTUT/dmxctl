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

/// コードの中の `crate::X` と `dmxctl::X` の X を、波括弧でまとめた書き方も含めて集める
fn referenced_layers(code: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for root in ["crate::", "dmxctl::"] {
        for (at, _) in code.match_indices(root) {
            // 直前が識別子の続きなら別のパス
            if code[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let rest = &code[at + root.len()..];
            let heads: Vec<String> = if let Some(body) = rest.strip_prefix('{') {
                let mut depth = 0;
                let mut list = String::new();
                for c in body.chars() {
                    match c {
                        '{' => depth += 1,
                        '}' if depth == 0 => break,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    list.push(c);
                }
                // 入れ子の中は、先頭の層から見えているので、最上位の項目だけ見る
                let mut tops = Vec::new();
                let mut depth = 0;
                let mut item = String::new();
                for c in list.chars() {
                    match c {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        ',' if depth == 0 => {
                            tops.push(std::mem::take(&mut item));
                            continue;
                        }
                        _ => {}
                    }
                    item.push(c);
                }
                tops.push(item);
                tops
            } else {
                vec![rest.to_string()]
            };
            for h in heads {
                let name: String = h.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !name.is_empty() {
                    found.push((at, name));
                }
            }
        }
    }
    found
}

/// `layer` の中のコードが、`forbidden` の層を参照していたら、その場所を返す
fn violations(layer: &str, forbidden: &[&str]) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![root.join(format!("{layer}.rs"))];
    rust_files(&root.join(layer), &mut files);
    let mut found = Vec::new();
    for f in files {
        let Ok(text) = fs::read_to_string(&f) else { continue };
        // コメントを除く。行の位置を保つため、行ごとに消す
        let code: String = text.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
        for (at, name) in referenced_layers(&code) {
            if forbidden.contains(&name.as_str()) {
                let line = code[..at].matches('\n').count() + 1;
                found.push(format!("{}:{}: {}", f.strip_prefix(&root).unwrap().display(), line, text.lines().nth(line - 1).unwrap_or("").trim()));
            }
        }
    }
    found
}

/// 検査自体が動いているかを、わざと違反する例で確かめる
#[test]
fn detects_braced_and_plain_references() {
    let names = |code: &str| referenced_layers(code).into_iter().map(|(_, n)| n).collect::<Vec<_>>();
    assert_eq!(names("use crate::backend::host::Host;"), ["backend"]);
    assert_eq!(names("use crate::{core::engine, ui::app::App, project};"), ["core", "ui", "project"]);
    assert_eq!(names("use crate::{core::{a, b}, backend};"), ["core", "backend"]);
    assert!(names("use my_crate::ui::x;").is_empty());
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

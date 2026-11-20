#!/bin/bash
# Mac用のアプリ(DMX Controller.app)を作る。Macのターミナルで実行する:
#   ./scripts/build-mac.sh
# 必要なもの: Rust(https://rustup.rs)と、Xcodeのコマンドラインツール(xcode-select --install)
#   操作パネルのビルドに、Node.js 20.19以上(https://nodejs.org)も必要
# できるもの: dist/mac/DMX Controller.app と dist/mac/DMX-Controller-mac.zip
# (Apple Silicon と Intel の両方で動く、ユニバーサル版)
set -euo pipefail
cd "$(dirname "$0")/.."
# NDIのクレート(サブモジュール moip-engine/)が無ければ取ってくる(--recursive を付けずに clone したとき)
[ -f moip-engine/ndi/Cargo.toml ] || git submodule update --init moip-engine

APP_NAME="DMX Controller"
OUT="dist/mac"
APP="$OUT/$APP_NAME.app"

rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# 2つを1つのファイルにまとめる
lipo -create \
  target/aarch64-apple-darwin/release/dmxctl \
  target/x86_64-apple-darwin/release/dmxctl \
  -output "$APP/Contents/MacOS/dmxctl"
# CLI(dmxctl ctl など)も同じファイル: "$APP/Contents/MacOS/dmxctl" ctl status
cp scripts/mac/Info.plist "$APP/Contents/Info.plist"

# 署名(開発者IDなしの簡易署名)。Apple Silicon では署名が無いと起動できない
codesign --force --deep --sign - "$APP"

(cd "$OUT" && rm -f DMX-Controller-mac.zip && ditto -c -k --keepParent "$APP_NAME.app" DMX-Controller-mac.zip)
echo "Built: $APP"
echo "Package: $OUT/DMX-Controller-mac.zip"

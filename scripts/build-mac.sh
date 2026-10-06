#!/bin/bash
# Mac用のアプリ(DMX Controller.app)を作る。Macのターミナルで実行する:
#   ./scripts/build-mac.sh
# 必要なもの: Rust(https://rustup.rs)と、Xcodeのコマンドラインツール(xcode-select --install)
# できるもの: dist/mac/DMX Controller.app と dist/mac/DMX-Controller-mac.zip
# (Apple Silicon と Intel の両方で動く、ユニバーサル版)
set -euo pipefail
cd "$(dirname "$0")/.."

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
cp scripts/mac/Info.plist "$APP/Contents/Info.plist"

# 署名(開発者IDなしの簡易署名)。Apple Silicon では署名が無いと起動できない
codesign --force --deep --sign - "$APP"

(cd "$OUT" && rm -f DMX-Controller-mac.zip && ditto -c -k --keepParent "$APP_NAME.app" DMX-Controller-mac.zip)
echo "できました: $APP"
echo "配布用: $OUT/DMX-Controller-mac.zip"

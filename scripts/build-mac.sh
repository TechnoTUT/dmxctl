#!/bin/bash
# Mac用のアプリ(DMX Controller.app)を作る。Macのターミナルで実行する:
#   ./scripts/build-mac.sh
# 必要なもの: Rust(https://rustup.rs)と、Xcodeのコマンドラインツール(xcode-select --install)
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
cp scripts/mac/Info.plist "$APP/Contents/Info.plist"
lipo -create \
  target/aarch64-apple-darwin/release/dmxctl-cli \
  target/x86_64-apple-darwin/release/dmxctl-cli \
  -output "$OUT/dmxctl-cli"
codesign --force --sign - "$OUT/dmxctl-cli"

# FTDIのD2XXライブラリを同梱する(third_party/ftd2xx/mac/libftd2xx.dylib があれば)
D2XX="third_party/ftd2xx/mac/libftd2xx.dylib"
if [ -f "$D2XX" ]; then
  mkdir -p "$APP/Contents/Frameworks"
  # ダウンロード時の印(quarantine)は持ち込まない
  ditto --noextattr "$D2XX" "$APP/Contents/Frameworks/libftd2xx.dylib"
fi

# 署名(開発者IDなしの簡易署名)。Apple Silicon では署名が無いと起動できない
codesign --force --deep --sign - "$APP"

(cd "$OUT" && rm -f DMX-Controller-mac.zip && ditto -c -k --keepParent "$APP_NAME.app" DMX-Controller-mac.zip)
(cd "$OUT" && zip -q DMX-Controller-mac.zip dmxctl-cli)
echo "できました: $APP"
echo "配布用: $OUT/DMX-Controller-mac.zip"

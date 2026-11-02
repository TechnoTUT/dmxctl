#!/bin/bash
# Mac用のアプリ(SPOT4.app)を作る: ./scripts/build-mac.sh
# できるもの: dist/mac/SPOT4.app(Apple Silicon / Intel 両対応)
set -euo pipefail
cd "$(dirname "$0")/.."

APP="dist/mac/SPOT4.app"
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create target/aarch64-apple-darwin/release/spot4 target/x86_64-apple-darwin/release/spot4 -output "$APP/Contents/MacOS/spot4"
cp scripts/mac/Info.plist "$APP/Contents/Info.plist"
# 開発者IDなしの簡易署名(Apple Silicon では署名が無いと起動できない)
codesign --force --deep --sign - "$APP"
echo "できました: $APP"
# 「アプリケーション」フォルダに入れてあれば、そちらも新しくする(古い版を起動してしまわないように)
if [ -d "/Applications/SPOT4.app" ]; then
  osascript -e 'quit app "SPOT4"' 2>/dev/null || true
  rm -rf "/Applications/SPOT4.app"
  ditto "$APP" "/Applications/SPOT4.app"
  echo "アプリケーションフォルダの SPOT4.app も更新しました"
fi

#!/bin/bash
# MacからWindows用の dmxctl.exe を作る(MinGW-w64 を使う)。
# 必要なもの: Rust、MinGW-w64(brew install mingw-w64)
# できるもの: dist/windows/dmxctl.exe と dist/windows/DMX-Controller-windows.zip
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="dist/windows"
rustup target add x86_64-pc-windows-gnu
CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++ AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar \
  cargo build --release --target x86_64-pc-windows-gnu
mkdir -p "$OUT"
cp target/x86_64-pc-windows-gnu/release/dmxctl.exe "$OUT/dmxctl.exe"
(cd "$OUT" && rm -f DMX-Controller-windows.zip && zip -q DMX-Controller-windows.zip dmxctl.exe)
echo "できました: $OUT/dmxctl.exe"
echo "配布用: $OUT/DMX-Controller-windows.zip"

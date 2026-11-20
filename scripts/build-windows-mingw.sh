#!/bin/bash
# Mac・LinuxからWindows用の dmxctl.exe を作る(MinGW-w64 を使う)。
# 必要なもの: Rust、MinGW-w64(Mac: brew install mingw-w64 / Debian・Ubuntu: sudo apt install mingw-w64)
#   操作パネルのビルドに、Node.js 20系は20.19以降、または22.12以降(https://nodejs.org)も必要
# できるもの: dist/windows/dmxctl.exe(単体で動く。追加のDLLは不要)
set -euo pipefail
# 操作パネルが未ビルドの配布物を作らないよう、npm が無いときはエラーにする
export DMXCTL_REQUIRE_WEB=1
cd "$(dirname "$0")/.."
# NDIのクレート(サブモジュール moip-engine/)が無ければ取ってくる(--recursive を付けずに clone したとき)
[ -f moip-engine/ndi/Cargo.toml ] || git submodule update --init moip-engine
OUT="dist/windows"
rustup target add x86_64-pc-windows-gnu
CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++ AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar \
  cargo build --release --target x86_64-pc-windows-gnu
mkdir -p "$OUT"
cp target/x86_64-pc-windows-gnu/release/dmxctl.exe "$OUT/dmxctl.exe"
echo "Built: $OUT/dmxctl.exe"

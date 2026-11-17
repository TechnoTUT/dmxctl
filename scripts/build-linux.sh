#!/bin/bash
# Linux用の dmxctl を作る。Linuxのターミナルで実行する:
#   ./scripts/build-linux.sh
# 必要なもの: Rust(https://rustup.rs)と、C++コンパイラ・開発用ライブラリ
#   Ubuntu/Debian: sudo apt install build-essential pkg-config cmake libasound2-dev libudev-dev libdrm-dev libgbm-dev libegl-dev libgles-dev
#   Fedora:        sudo dnf install gcc-c++ pkgconf cmake alsa-lib-devel systemd-devel libatomic libdrm-devel mesa-libgbm-devel mesa-libEGL-devel mesa-libGLES-devel
#   Arch:          sudo pacman -S base-devel cmake alsa-lib libdrm mesa
# dmxctl 1つで、デスクトップ(X11 / Wayland)・デスクトップ環境の無い画面(KMS/DRM)・画面なし(dmxctl headless)・CLI(dmxctl ctl など)
# のすべてに使う。SDL2 は実行ファイルに内蔵するので、動かすPCに入れる必要は無い
# できるもの: dist/linux/dmxctl
set -euo pipefail
cd "$(dirname "$0")/.."
# NDIのクレート(サブモジュール moip-engine/)が無ければ取ってくる(--recursive を付けずに clone したとき)
[ -f moip-engine/ndi/Cargo.toml ] || git submodule update --init moip-engine
OUT="dist/linux"
mkdir -p "$OUT"
cargo build --release
cp target/release/dmxctl "$OUT/dmxctl"
echo "Built: $OUT/dmxctl"

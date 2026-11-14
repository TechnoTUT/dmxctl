#!/bin/bash
# Linux用の dmxctl を作る。Linuxのターミナルで実行する:
#   ./scripts/build-linux.sh
# 必要なもの: Rust(https://rustup.rs)と、C++コンパイラ・開発用ライブラリ
#   Ubuntu/Debian: sudo apt install build-essential pkg-config libasound2-dev libudev-dev libsdl2-dev
#   Fedora:        sudo dnf install gcc-c++ pkgconf alsa-lib-devel systemd-devel libatomic SDL2-devel
#   Arch:          sudo pacman -S base-devel alsa-lib sdl2
# デスクトップ環境の無い画面(KMS/DRM)でも動くよう、SDL2を使う機能(kmsdrm)を入れて作る。
# SDL2を入れたくないときは DMXCTL_NO_KMSDRM=1 ./scripts/build-linux.sh(デスクトップ環境の上でだけ動く)
# できるもの: dist/linux/dmxctl と dist/linux/DMX-Controller-linux.tar.gz
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="dist/linux"
if [ -n "${DMXCTL_NO_KMSDRM:-}" ]; then
  cargo build --release
else
  cargo build --release --features kmsdrm
fi
mkdir -p "$OUT"
cp target/release/dmxctl "$OUT/dmxctl"
cp scripts/linux/dmxctl.desktop "$OUT/dmxctl.desktop"
(cd "$OUT" && rm -f DMX-Controller-linux.tar.gz && tar czf DMX-Controller-linux.tar.gz dmxctl dmxctl.desktop)
echo "できました: $OUT/dmxctl"
echo "配布用: $OUT/DMX-Controller-linux.tar.gz"

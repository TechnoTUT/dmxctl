#!/bin/bash
# Linux用の dmxctl を作る。Linuxのターミナルで実行する:
#   ./scripts/build-linux.sh
# 必要なもの: Rust(https://rustup.rs)と、C++コンパイラ・開発用ライブラリ
#   Ubuntu/Debian: sudo apt install build-essential pkg-config libasound2-dev libudev-dev libsdl2-dev
#   Fedora:        sudo dnf install gcc-c++ pkgconf alsa-lib-devel systemd-devel libatomic SDL2-devel
#   Arch:          sudo pacman -S base-devel alsa-lib sdl2
# 2つ作る:
#   dmxctl        … デスクトップ環境(X11 / Wayland)用。SDL2は要らない
#   dmxctl-kmsdrm … デスクトップ環境の無い画面(KMS/DRM)にも出せる版(kmsdrm機能。動かすPCに SDL2 が要る)
# SDL2を入れたくないときは DMXCTL_NO_KMSDRM=1 ./scripts/build-linux.sh(dmxctl だけ作る)
# できるもの: dist/linux/dmxctl・dmxctl-kmsdrm と dist/linux/DMX-Controller-linux.tar.gz
set -euo pipefail
cd "$(dirname "$0")/.."
# NDIのクレート(サブモジュール moip-engine/)が無ければ取ってくる(--recursive を付けずに clone したとき)
[ -f moip-engine/ndi/Cargo.toml ] || git submodule update --init moip-engine
OUT="dist/linux"
mkdir -p "$OUT"
rm -f "$OUT/dmxctl-kmsdrm"
cargo build --release
cp target/release/dmxctl "$OUT/dmxctl"
FILES="dmxctl dmxctl.desktop"
if [ -z "${DMXCTL_NO_KMSDRM:-}" ]; then
  cargo build --release --features kmsdrm
  cp target/release/dmxctl "$OUT/dmxctl-kmsdrm"
  FILES="$FILES dmxctl-kmsdrm"
fi
cp scripts/linux/dmxctl.desktop "$OUT/dmxctl.desktop"
(cd "$OUT" && rm -f DMX-Controller-linux.tar.gz && tar czf DMX-Controller-linux.tar.gz $FILES)
echo "できました: $OUT/dmxctl"
echo "配布用: $OUT/DMX-Controller-linux.tar.gz"

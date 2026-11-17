#!/bin/bash
# モニターの無いLinuxで常駐させる版(常駐版)を、1つの tar.gz にまとめる。
#   ./scripts/package-linux-headless.sh x86_64   (または arm64)
# 先に dist/linux/dmxctl を作っておく(./scripts/build-linux.sh、または cargo build --release して置く)。
# できるもの: dist/linux/DMX-Controller-linux-headless-<arch>.tar.gz
#   展開すると dmxctl-headless/ に、dmxctl 本体・install.sh・systemd の設定・BLT 用のスクリプト・README.txt
set -euo pipefail
cd "$(dirname "$0")/.."
ARCH="${1:?x86_64 か arm64 を指定してください}"
OUT="dist/linux"
[ -x "$OUT/dmxctl" ] || { echo "$OUT/dmxctl がありません"; exit 1; }
STAGE="$OUT/dmxctl-headless"
rm -rf "$STAGE"
mkdir -p "$STAGE"
install -m 755 "$OUT/dmxctl" "$STAGE/dmxctl"
install -m 755 scripts/linux/install-headless.sh "$STAGE/install.sh"
install -m 755 scripts/linux/blt-headless.sh "$STAGE/blt-headless.sh"
install -m 644 scripts/linux/dmxctl-headless.service scripts/linux/beat-link-trigger.service "$STAGE/"
install -m 644 scripts/linux/headless-README.txt "$STAGE/README.txt"
TAR="DMX-Controller-linux-headless-$ARCH.tar.gz"
(cd "$OUT" && rm -f "$TAR" && tar czf "$TAR" dmxctl-headless)
rm -rf "$STAGE"
echo "配布用: $OUT/$TAR"

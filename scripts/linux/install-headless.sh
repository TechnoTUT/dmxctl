#!/bin/bash
# モニターの無いLinuxで、dmxctl(オート)と Beat Link Trigger を、電源を入れたら自動で動くようにする。
#   ./scripts/linux/install-headless.sh [--no-blt] [dmxctlの場所]
#     --no-blt     dmxctl だけ(BLT は別のPCで動かすとき)
#     dmxctlの場所 Release から落とした dmxctl-linux-x86_64 / dmxctl-linux-arm64 など。
#                  省略すると ./scripts/build-linux.sh で作った dist/linux/dmxctl を使う
# Beat Link Trigger は公式の .deb(https://github.com/Deep-Symmetry/beat-link-trigger/releases)を先に入れておく
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
WITH_BLT=1
BIN=""
for arg in "$@"; do
  case "$arg" in
    --no-blt) WITH_BLT=0 ;;
    *) BIN="$arg" ;;
  esac
done
[ -n "$BIN" ] || BIN="$HERE/../../dist/linux/dmxctl"
if [ ! -f "$BIN" ]; then
  echo "dmxctl が見つかりません($BIN)。Release から落とした dmxctl-linux-* の場所を指定するか、先に ./scripts/build-linux.sh を実行してください"
  exit 1
fi

# USB-DMX(dialout)を使えるように
if ! id -nG | grep -qw dialout; then
  echo "USB-DMX を使えるよう、dialout グループに入れます(sudo)"
  sudo usermod -aG dialout "$USER"
fi
if [ "$WITH_BLT" = 1 ] && ! command -v Xvfb >/dev/null; then
  echo "仮想の画面(Xvfb)と、設定用の VNC(x11vnc)を入れます(sudo)"
  sudo apt-get install -y xvfb x11vnc
fi

mkdir -p "$HOME/dmxctl" "$HOME/.config/systemd/user"
install -m 755 "$BIN" "$HOME/dmxctl/dmxctl"
install -m 644 "$HERE/dmxctl-headless.service" "$HOME/.config/systemd/user/"
UNITS="dmxctl-headless.service"
if [ "$WITH_BLT" = 1 ]; then
  install -m 755 "$HERE/blt-headless.sh" "$HOME/dmxctl/blt-headless.sh"
  install -m 644 "$HERE/beat-link-trigger.service" "$HOME/.config/systemd/user/"
  UNITS="$UNITS beat-link-trigger.service"
fi

# ログインしていなくても(電源を入れただけで)ユーザーサービスを動かす
sudo loginctl enable-linger "$USER"
systemctl --user daemon-reload
systemctl --user enable --now $UNITS
echo "できました。状態: systemctl --user status $UNITS"
echo "dmxctl の様子: journalctl --user -u dmxctl-headless -f"

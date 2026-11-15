#!/bin/bash
# モニターの無いLinuxで、dmxctl(オート)と Beat Link Trigger を、電源を入れたら自動で動くようにする。
#   ./scripts/linux/install-headless.sh            dmxctl と BLT の両方
#   ./scripts/linux/install-headless.sh --no-blt   dmxctl だけ
# 先に ./scripts/build-linux.sh で dist/linux/dmxctl を作っておく(または Release の tar.gz を展開した dmxctl をこのフォルダの dist/linux に置く)。
# Beat Link Trigger は公式の .deb(https://github.com/Deep-Symmetry/beat-link-trigger/releases)を先に入れておく
set -euo pipefail
cd "$(dirname "$0")/../.."
WITH_BLT=1
[ "${1:-}" = "--no-blt" ] && WITH_BLT=0

[ -x dist/linux/dmxctl ] || { echo "dist/linux/dmxctl がありません。先に ./scripts/build-linux.sh を実行してください"; exit 1; }

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
install -m 755 dist/linux/dmxctl "$HOME/dmxctl/dmxctl"
install -m 644 scripts/linux/dmxctl-headless.service "$HOME/.config/systemd/user/"
UNITS="dmxctl-headless.service"
if [ "$WITH_BLT" = 1 ]; then
  install -m 755 scripts/linux/blt-headless.sh "$HOME/dmxctl/blt-headless.sh"
  install -m 644 scripts/linux/beat-link-trigger.service "$HOME/.config/systemd/user/"
  UNITS="$UNITS beat-link-trigger.service"
fi

# ログインしていなくても(電源を入れただけで)ユーザーサービスを動かす
sudo loginctl enable-linger "$USER"
systemctl --user daemon-reload
systemctl --user enable --now $UNITS
echo "できました。状態: systemctl --user status $UNITS"
echo "dmxctl の様子: journalctl --user -u dmxctl-headless -f"

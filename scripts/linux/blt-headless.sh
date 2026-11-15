#!/bin/bash
# 仮想の画面(Xvfb :99)を作って、その中で Beat Link Trigger を動かす。
# BLT_VNC=1 なら、その画面を VNC(localhost:5900)で見られるようにする(設定するときだけ)
#   ssh -L 5900:localhost:5900 ユーザー@このPC  のあと、手元の VNC ビューアで localhost:5900 へ
set -u
DISP=":99"
# BLT の起動コマンド(.deb で入れた場所。違うときは ~/dmxctl/blt.env に BLT_CMD=... を書く)
BLT_CMD="/opt/beat-link-trigger/bin/Beat Link Trigger"
[ -f "$HOME/dmxctl/blt.env" ] && . "$HOME/dmxctl/blt.env"

Xvfb "$DISP" -screen 0 1280x800x24 -nolisten tcp &
XVFB=$!
trap 'kill $XVFB ${VNC:-} 2>/dev/null' EXIT
sleep 1
if [ "${BLT_VNC:-0}" = "1" ]; then
  x11vnc -display "$DISP" -localhost -forever -nopw -quiet &
  VNC=$!
fi
DISPLAY="$DISP" "$BLT_CMD"

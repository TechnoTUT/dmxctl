#!/bin/bash
# 仮想の画面(Xvfb :99)を作って、その中で Beat Link Trigger を動かす。
# BLT_VNC=1 なら、その画面を VNC(localhost:5900)で見られるようにする(設定するときだけ)
#   ssh -L 5900:localhost:5900 ユーザー@このPC  のあと、手元の VNC ビューアで localhost:5900 へ
set -u
DISP=":99"
# BLT の jar の場所(GitHub の Releases から落とした beat-link-trigger-*.jar を、この名前で置く。
# 場所が違うときは ~/dmxctl/blt.env に BLT_JAR=... を書く)。動かすには Java 21 以上が要る
BLT_JAR="$HOME/dmxctl/beat-link-trigger.jar"
[ -f "$HOME/dmxctl/blt.env" ] && . "$HOME/dmxctl/blt.env"
if [ ! -f "$BLT_JAR" ]; then
  echo "BLT の jar が見つかりません($BLT_JAR)。Releases から落として、この場所に置いてください"
  exit 1
fi

Xvfb "$DISP" -screen 0 1280x800x24 -nolisten tcp &
XVFB=$!
trap 'kill $XVFB ${VNC:-} 2>/dev/null' EXIT
sleep 1
if [ "${BLT_VNC:-0}" = "1" ]; then
  x11vnc -display "$DISP" -localhost -forever -nopw -quiet &
  VNC=$!
fi
DISPLAY="$DISP" java -jar "$BLT_JAR"

#!/bin/bash
# 仮想の画面(Xvfb :99)を作って、その中で Beat Link Trigger を動かす。
# BLT_VNC=1 なら、その画面を VNC(localhost:5900)で見られるようにする(設定するときだけ)
#   ssh -L 5900:localhost:5900 ユーザー@このPC  のあと、手元の VNC ビューアで localhost:5900 へ
set -u
DISP=":99"
# BLT の jar の場所(GitHub の Releases から落とした beat-link-trigger-*.jar を、この名前で置く。
# 場所が違うときは ~/dmxctl/blt.env に BLT_JAR=... を書く)。動かすには Java 21 以上が要る
# 以前の .deb などで入れた BLT を使い続けるときは、blt.env に BLT_CMD="起動コマンドの場所" を書く。
# BLT_CMD が書いてあれば、jar よりそちらを使う
BLT_JAR="$HOME/dmxctl/beat-link-trigger.jar"
BLT_CMD=""
[ -f "$HOME/dmxctl/blt.env" ] && . "$HOME/dmxctl/blt.env"
if [ -n "$BLT_CMD" ]; then
  BLT_RUN=("$BLT_CMD")
elif [ -f "$BLT_JAR" ]; then
  BLT_RUN=(java -jar "$BLT_JAR")
else
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
DISPLAY="$DISP" "${BLT_RUN[@]}"

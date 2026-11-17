dmxctl 常駐版(モニターの無いLinuxで、電源を入れたら自動で照明を動かす)
=====================================================================

DJ機器(CDJ / XDJ)から Beat Link Trigger(BLT)経由でテンポ・波形の色と大きさ・フレーズを受け取り、
照明を自動で動かします。曲が止まれば薄い水色で待機します。画面は出しません。

動くPC
  - x86_64 版: Debian 13 / Ubuntu 24.04 以降
  - arm64 版 : Raspberry Pi OS(Bookworm 以降, 64bit)/ Debian 12 以降 / Ubuntu 24.04 以降

入れ方
  1. 照明の設定(機材・出力・シーン・オートのプリセット)を、画面のあるPCの dmxctl で作り、
     ~/.config/dmxctl/ をこのPCの同じ場所にコピーします
     (Windows で作ったときは %APPDATA%\dmxctl\ の中身)
  2. BLT も使うときは、BLT の公式の .deb を入れておきます
       sudo apt install ./beat-link-trigger_*.deb
     .deb が入らないPC(arm64 など)では、Java と BLT の .jar を入れ、起動するスクリプトを作って
     ~/dmxctl/blt.env に BLT_CMD="そのスクリプトの場所" と書きます
  3. このフォルダで
       ./install.sh            dmxctl と BLT の両方を常駐させる
       ./install.sh --no-blt   dmxctl だけ(BLT は別のPCで動かすとき)
     USB-DMX を使えるようにグループに入ったときは、一度再起動してください

様子を見る
  journalctl --user -u dmxctl-headless -f
  スマホ・PCのブラウザで http://このPCのIPアドレス:8080/ を開くと操作パネル

止める
  systemctl --user stop dmxctl-headless      (安全な値を送ってから止まります)
  systemctl --user disable --now dmxctl-headless beat-link-trigger   (自動で動かさない)

BLT の最初の設定(VNC で仮想の画面に入る方法)や詳しいことは、README の
「モニターの無いLinuxでオート(BLT と一緒に常駐)」を見てください。

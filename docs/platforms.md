# Mac・Linux で使う

Windows と同じ機能が Mac・Linux でも使えます。ここでは、OSごとの違いと注意をまとめます。ソースからビルドする手順は [開発ガイド](development.md) にあります。

## Mac で使う

Release の `DMX-Controller-mac.zip` は、Apple Silicon と Intel の両方で動くユニバーサル版です。

- 設定の保存場所は `~/Library/Application Support/dmxctl/` です。中身は Windows の `%APPDATA%\dmxctl\` と同じなので、`project.json` をコピーすればそのまま使えます
- 保存のショートカットは ⌘S です。ほかのキーは同じです
- FT232系の USB-DMX は、Mac標準のドライバで `/dev/cu.usbserial-…` として見え、「出力」タブのCOMポートの一覧に出ます。D2XX を使う場合は FTDI の `libftd2xx.dylib` を入れてください
- 簡易署名のアプリなので、ダウンロードしたものを初めて開くときは「開発元を確認できません」と出ます。Finderでアプリを右クリック →「開く」で開けます。macOS 15 以降で開けないときは、「システム設定」→「プライバシーとセキュリティ」の「このまま開く」を押します
- 初めて Art-Net・OSC を使うとき、「ローカルネットワーク上のデバイスの検索と接続」の許可を求められます。許可してください

## Linux で使う

X11 と Wayland のどちらでも動きます。コンソールだけのPCや Raspberry Pi など、**デスクトップ環境の無いLinux** でも、画面に直接出して使えます。詳しくは [デスクトップ環境の無いLinuxで使う](#デスクトップ環境の無いlinuxで使う)。

Release の Linux 版 `dmxctl-linux-x86_64` / `dmxctl-linux-arm64` は Debian 12 環境でビルドしているため、**Debian 12・Raspberry Pi OS Bookworm・Ubuntu 24.04 以降** で動作します。古い環境では `GLIBC_2.xx not found` で起動できないため、その環境でビルドしてください。手順は [開発ガイド](development.md#linux) にあります。デスクトップ環境の無い画面で使う SDL2 は `dmxctl` に内蔵しているので、動かすPCに入れるものはありません。

**Linuxでの違い・注意**

- 設定の保存場所は `~/.config/dmxctl/` です。`XDG_CONFIG_HOME` があればその下になります。中身は Windows・Mac と同じです
- 保存のショートカットは Ctrl+S です
- 日本語フォントは、Noto Sans CJK があればそれを使い、無ければ fontconfig が選ぶ日本語フォントを使います。文字が□になるときは `sudo apt install fonts-noto-cjk` などで入れてください
- FT232系の USB-DMX は、標準のドライバで `/dev/ttyUSB0` などとして見えます。開けないときは `sudo usermod -aG dialout $USER` のあとログインし直してください
- D2XX を使う場合は FTDI の `libftd2xx.so` を入れ、`sudo rmmod ftdi_sio` で標準のドライバを外します。ふつうは COMポート方式で十分です
- MIDI・音声入力は ALSA を使います。PipeWire や PulseAudio の環境でもそのまま動きます
- メニューに出したいときは、`dmxctl` を PATH の通った場所に置き、`scripts/linux/dmxctl.desktop` を `~/.local/share/applications/` にコピーします
- ufw や firewalld でファイアウォールを有効にしている場合は、使う機能のUDPポートを開けてください。OSC受信は既定で 9000、Ableton Link は 20808 です。Link の通信をするのは Carabiner です。dmxctl が Carabiner につなぐ 17000 は同じPC内だけなので、開けなくて大丈夫です。Art-Netの送信だけなら要りません
- Ctrl+C・`kill`・ログアウト・シャットダウンで終了のシグナルを受けたときも、レーザーOFF・明るさ0の安全な値を送ってから止まります
- NDI を使うときは NDI SDK のライブラリが要ります。次の [Linux で NDI を使う](#linux-で-ndi-を使う) を見てください

### Linux で NDI を使う

Linux には NDI Tools が無いため、NDI を使う場合のみ **NDI SDK for Linux** の `libndi.so.6` を導入します。配布条件によりアプリへは同梱していませんが、無くても起動します。

```sh
curl -fLO https://downloads.ndi.tv/SDK/NDI_SDK_Linux/Install_NDI_SDK_v6_Linux.tar.gz
tar -xzf Install_NDI_SDK_v6_Linux.tar.gz
sh Install_NDI_SDK_v6_Linux.sh       # 使用許諾を読んで同意するなら y。今いるフォルダに「NDI SDK for Linux」ができる
sudo cp -P "NDI SDK for Linux/lib/x86_64-linux-gnu/"libndi.so* /usr/local/lib/
sudo ldconfig
```

- 入ったかどうかは、`/sbin/ldconfig -p | grep libndi` に `libndi.so.6` が出るかで確かめます。入っていれば、dmxctl の2ページ目の設定パネルで NDI のソースが選べるようになります
- 64bit の Raspberry Pi OS では、`x86_64-linux-gnu` の代わりに `aarch64-rpi4-linux-gnueabi` のフォルダのものを使います
- `sudo` を使わずに試すときは、置いた場所を直接指定して起動します: `DMXCTL_NDI_LIB="$PWD/NDI SDK for Linux/lib/x86_64-linux-gnu/libndi.so.6" ./dmxctl`
- 入れたあとは、ダウンロードしたファイルと「NDI SDK for Linux」のフォルダは消してかまいません

### デスクトップ環境の無いLinuxで使う

X11 や Wayland のデスクトップ環境が無いPCでは、`dmxctl` を **コンソールの tty から起動するだけ** で、画面全体に直接表示します。SDL2 の KMSDRM を使って、KMS/DRM で描きます。照明卓専用の小さなPCや Raspberry Pi で、余計なものを入れずに動かせます。

**準備**

1. Release の `dmxctl-linux-x86_64` / `dmxctl-linux-arm64` か、`./scripts/build-linux.sh` で作ったものを使います。動かすPCには、OpenGL ES の動く Mesa などのGPUドライバと、日本語フォント `fonts-noto-cjk` を入れます。GPUドライバはふつう入っています
2. 画面とキーボード・マウスを使えるよう、ユーザーを `video` と `input` のグループに入れて、ログインし直します。`dialout` は USB-DMX 用です

   ```
   sudo usermod -aG video,input,dialout $USER
   ```

   `input` に入っていないと、画面は出てもキーボード・マウスが効きません。このときは画面下に知らせが出ます

3. コンソールにログインして `./dmxctl` で起動します。SSH から起動することもでき、画面はそのPCにつないだモニターに出ます

**操作・違い**

- 画面全体に表示します。拡大率は画面の大きさから自動で決め、1920×1080 なら1.25倍です。変えたいときは `DMXCTL_SCALE=1.5 ./dmxctl` のように指定します
- 閉じるボタンが無いので、「ファイル」メニューの「終了」か Ctrl+Q で終了します。起動した端末で Ctrl+C を押しても安全に止まります
- 別名で保存・開くのファイルを選ぶ画面は出せないので、「ファイル」メニューには出ません。設定は今までどおり自動保存され、「バックアップから戻す」は一覧から選べます
- タッチパネルは、触った所をクリックします。触ったまま動かすとドラッグです
- トラックパッドは1本指で移動、タップで左クリック、2本指タップで右クリック、2本指ドラッグでスクロールできます。押し込みクリックも使えます
- 日本語入力は使えません。名前の欄などには英数字を入力してください
- デスクトップ環境など、ほかのアプリが画面を使っている間は起動できません
- KMS/DRM を使うかどうかは、ふつうは自動で決まります。環境変数 `DISPLAY` と `WAYLAND_DISPLAY` が無ければ使います。`dmxctl kmsdrm` か `DMXCTL_KMSDRM=1` で必ず使い、`DMXCTL_KMSDRM=0` で使いません

**電源を入れたら自動で起動する例**

1. tty1 に自動ログインするよう設定します。次のコマンドで開いたところに、下の内容を書いて保存します。`ユーザー名` は自分のものにします

   ```
   sudo systemctl edit getty@tty1
   ```

   ```
   [Service]
   ExecStart=
   ExecStart=-/sbin/agetty --autologin ユーザー名 --noclear %I $TERM
   ```

2. `~/.bash_profile` の最後に、tty1 のときだけ起動するよう書きます。パスは `dmxctl` を置いた場所に合わせます

   ```
   if [ "$(tty)" = /dev/tty1 ]; then ~/dmxctl/dmxctl; fi
   ```

   アプリを終了するとコンソールに戻ります。`~/dmxctl/dmxctl` の前に `exec` を付けると、終了しても自動でもう一度起動します

## USB-DMX について

FT232系の USB-DMX は、BREAK→スタートコード→スロットという DMX のフレームを、PC側が作って送り続ける方式です。つなぎ方は2通りあります。

- **COMポート**: FTDIドライバが作る仮想COMポートに、250000bps 8N2 と BREAK で送ります。まずこちらを試してください
- **D2XX**: FTDIのライブラリを実行時に読み込んで、直接制御します。ライブラリは、Windows では FTDIドライバに同梱の `ftd2xx.dll`、Mac ではアプリに同梱の `libftd2xx.dylib`、Linux では `libftd2xx.so` です。ライブラリが無くてもアプリ自体は起動します

抜き差しや電源断で切れても、自動でつなぎ直します。

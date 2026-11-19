# Mac・Linux で使う

Windows と同じ機能が Mac・Linux でも利用できます。ここでは、OS別の違いと注意点をまとめます。ソースからビルドする手順は [開発ガイド](development.md) にあります。

## Mac で使う

Release の `DMX-Controller-mac.zip` は、Apple Silicon と Intel の両方で動作するユニバーサル版です。

- 設定の保存場所は `~/Library/Application Support/dmxctl/` です。中身は Windows の `%APPDATA%\dmxctl\` と同じため、`project.json` をコピーすればそのまま利用できます
- 保存のショートカットは ⌘S です。他のキーは同じです
- FT232系の USB-DMX は、Mac 標準のドライバで `/dev/cu.usbserial-…` として認識され、「出力」タブの COMポートの一覧に表示されます。D2XX を利用する場合は、[FTDI の配布元](https://ftdichip.com/drivers/d2xx-drivers/) から `libftd2xx.dylib` を取得し、`/usr/local/lib/` か設定フォルダに配置してください。このアプリには同梱していません
- 簡易署名のアプリのため、ダウンロードしたものを初回に開くと「開発元を確認できません」と表示されます。Finder でアプリを右クリック →「開く」で開けます。macOS 15 以降で開けないときは、「システム設定」→「プライバシーとセキュリティ」の「このまま開く」を押します
- 初めて Art-Net・sACN・OSC を利用するとき、「ローカルネットワーク上のデバイスの検索と接続」の許可を求められます。許可してください

## Linux で使う

X11 と Wayland のどちらでも動作します。コンソールのみの PC や Raspberry Pi など、**デスクトップ環境の無い Linux** でも、画面に直接出力して利用できます。詳しくは [デスクトップ環境の無い Linux で使う](#デスクトップ環境の無いlinuxで使う)。

Release の Linux 版 `dmxctl-linux-x86_64` / `dmxctl-linux-arm64` は Debian 12 環境でビルドしているため、**Debian 12・Raspberry Pi OS Bookworm・Ubuntu 24.04 以降** で動作します。古い環境では `GLIBC_2.xx not found` で起動できないため、その環境でビルドしてください。手順は [開発ガイド](development.md#linux) にあります。デスクトップ環境の無い画面で利用する SDL2 は `dmxctl` に内蔵しているので、動作させる PC に導入するものはありません。

**Linux での違い・注意点**

- 設定の保存場所は `~/.config/dmxctl/` です。`XDG_CONFIG_HOME` が設定されていればその下になります。中身は Windows・Mac と同じです
- 保存のショートカットは Ctrl+S です
- 日本語フォントは、Noto Sans CJK があればそれを使い、無ければ fontconfig が選択する日本語フォントを使います。文字が□で表示されるときは `sudo apt install fonts-noto-cjk` などで導入してください
- FT232系の USB-DMX は、標準のドライバで `/dev/ttyUSB0` などとして認識されます。開けないときは `sudo usermod -aG dialout $USER` を実行したあと、ログインし直してください
- D2XX を利用する場合は FTDI の `libftd2xx.so` を導入し、`sudo rmmod ftdi_sio` で標準のドライバを外します。通常は COMポート方式で十分です
- MIDI・音声入力は ALSA を利用します。PipeWire や PulseAudio の環境でもそのまま動作します
- メニューに表示したいときは、`dmxctl` を PATH の通った場所に置き、`scripts/linux/dmxctl.desktop` を `~/.local/share/applications/` にコピーします
- ufw や firewalld でファイアウォールを有効にしている場合は、利用する機能の UDP ポートを開放してください。OSC 受信は既定で 9000、Ableton Link は 20808 です。Link の通信を担当するのは Carabiner です。dmxctl が Carabiner へ接続する 17000 は同じ PC 内だけで使うため、開放は不要です。Art-Net・sACN の送信だけであれば開放は不要です
- Ctrl+C・`kill`・ログアウト・シャットダウンで終了のシグナルを受信したときも、レーザーOFF・明るさ0の安全な値を送信してから停止します
- NDI を利用するには NDI SDK のライブラリが必要です。次の [Linux で NDI を使う](#linux-で-ndi-を使う) を確認してください

### Linux で NDI を使う

Linux には NDI Tools が無いため、NDI を利用する場合に限り **NDI SDK for Linux** の `libndi.so.6` を導入します。配布条件によりアプリには同梱していませんが、無くても起動します。

```sh
curl -fLO https://downloads.ndi.tv/SDK/NDI_SDK_Linux/Install_NDI_SDK_v6_Linux.tar.gz
tar -xzf Install_NDI_SDK_v6_Linux.tar.gz
sh Install_NDI_SDK_v6_Linux.sh       # 使用許諾を確認して同意する場合は y。現在のフォルダに「NDI SDK for Linux」ができる
sudo cp -P "NDI SDK for Linux/lib/x86_64-linux-gnu/"libndi.so* /usr/local/lib/
sudo ldconfig
```

- 導入できたかは、`/sbin/ldconfig -p | grep libndi` に `libndi.so.6` が表示されるかで確認します。導入済みであれば、dmxctl の2ページ目の設定パネルで NDI のソースを選択できるようになります
- 64bit の Raspberry Pi OS では、`x86_64-linux-gnu` の代わりに `aarch64-rpi4-linux-gnueabi` のフォルダのものを利用します
- `sudo` を使わずに試すときは、配置した場所を直接指定して起動します: `DMXCTL_NDI_LIB="$PWD/NDI SDK for Linux/lib/x86_64-linux-gnu/libndi.so.6" ./dmxctl`
- 導入後は、ダウンロードしたファイルと「NDI SDK for Linux」のフォルダを削除して構いません

### デスクトップ環境の無いLinuxで使う

X11 や Wayland のデスクトップ環境が無い PC では、`dmxctl` を **コンソールの tty から起動するだけ** で、画面全体に直接表示します。SDL2 の KMSDRM を利用して、KMS/DRM で描画します。照明卓専用の小型 PC や Raspberry Pi で、余計なものを導入せずに動作させられます。

**準備**

1. Release の `dmxctl-linux-x86_64` / `dmxctl-linux-arm64` か、`./scripts/build-linux.sh` で作成したものを利用します。動作させる PC には、OpenGL ES に対応した Mesa などの GPU ドライバと、日本語フォント `fonts-noto-cjk` を導入します。GPU ドライバは通常、導入済みです
2. 画面とキーボード・マウスを利用できるよう、ユーザーを `video` と `input` のグループに追加して、ログインし直します。`dialout` は USB-DMX 用です

   ```
   sudo usermod -aG video,input,dialout $USER
   ```

   `input` に所属していないと、画面は表示されてもキーボード・マウスが効きません。このときは画面下に通知が表示されます

3. コンソールにログインして `./dmxctl` で起動します。SSH から起動することもでき、画面はその PC に接続したモニターに表示されます

**操作・違い**

- 画面全体に表示します。拡大率は画面の大きさから自動で決定し、1920×1080 なら1.25倍です。変更したいときは `DMXCTL_SCALE=1.5 ./dmxctl` のように指定します
- 閉じるボタンが無いので、「ファイル」メニューの「終了」か Ctrl+Q で終了します。起動した端末で Ctrl+C を押しても安全に停止します
- 「別名で保存」や「開く」のファイル選択画面は表示できないため、「ファイル」メニューには表示されません。設定は従来どおり自動保存され、「バックアップから戻す」は一覧から選択できます
- タッチパネルは、触れた箇所をクリックとして扱います。触れたまま動かすとドラッグになります
- トラックパッドは1本指で移動、タップで左クリック、2本指タップで右クリック、2本指ドラッグでスクロールできます。押し込みクリックも利用できます
- 日本語入力は利用できません。名前の欄などには英数字を入力してください
- デスクトップ環境など、他のアプリが画面を使用している間は起動できません
- KMS/DRM を利用するかどうかは、通常は自動で決まります。環境変数 `DISPLAY` と `WAYLAND_DISPLAY` が無ければ利用します。`dmxctl kmsdrm` か `DMXCTL_KMSDRM=1` で必ず利用し、`DMXCTL_KMSDRM=0` で利用しません

**電源を入れたら自動で起動する例**

1. tty1 に自動ログインするよう設定します。次のコマンドで開いた画面に、下の内容を記述して保存します。`ユーザー名` は自分のユーザー名に置き換えます

   ```
   sudo systemctl edit getty@tty1
   ```

   ```
   [Service]
   ExecStart=
   ExecStart=-/sbin/agetty --autologin ユーザー名 --noclear %I $TERM
   ```

2. `~/.bash_profile` の末尾に、tty1 のときだけ起動するよう記述します。パスは `dmxctl` を配置した場所に合わせます

   ```
   if [ "$(tty)" = /dev/tty1 ]; then ~/dmxctl/dmxctl; fi
   ```

   アプリを終了するとコンソールに戻ります。`~/dmxctl/dmxctl` の前に `exec` を付けると、終了後も自動で再起動します

## USB-DMX について

FT232系の USB-DMX は、BREAK→スタートコード→スロットという DMX のフレームを、PC 側で生成して送信し続ける方式です。接続方法は2通りあります。

- **COMポート**: FTDI ドライバが作成する仮想 COMポートに、250000bps 8N2 と BREAK で送信します。まずこちらを試してください
- **D2XX**: FTDI のライブラリを実行時に読み込んで、直接制御します。ライブラリは、Windows では FTDI ドライバに同梱の `ftd2xx.dll`、Mac では利用者が導入した `libftd2xx.dylib`、Linux では `libftd2xx.so` です。ライブラリが無くてもアプリ自体は起動します

抜き差しや電源断で切断されても、自動で再接続します。

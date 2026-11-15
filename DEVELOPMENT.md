# 開発環境メモ(別のPCで作り直すとき)

## コピーするもの

| もの | 場所 | 備考 |
|---|---|---|
| ソース一式(履歴つき) | `dmxctl` フォルダ(`.git` を含む) | `target/`(ビルドの中間ファイル、2GB以上)と `dist/`(できあがったアプリ)はコピー不要。作り直せる |
| 設定(パッチ・機材定義・割り当て) | Mac: `~/Library/Application Support/dmxctl/`<br>Windows: `%APPDATA%\dmxctl\`<br>Linux: `~/.config/dmxctl/` | `project.json` と `fixtures/` が本体。`backups/` は任意 |

新しいPCでは、設定フォルダの中身を上の場所にそのまま置く(Mac・Windows・Linuxで中身は同じ形式)。

## 新しいPCで入れるもの

### Windows
1. **Rust**: https://rustup.rs の `rustup-init.exe`(既定のまま。MSVC版になる)
2. **Visual Studio Build Tools 2022** の「C++ によるデスクトップ開発」(Ableton Link のC++部分をビルドするため)
3. (任意)**Git**: 履歴を使う・GitHubに置くなら
4. **FTDIドライバ**: USB-DMX(FT232R)を挿すと、ふつうはWindows Updateで入る(COMポート方式・D2XX方式の両方)
5. (NDIを使うなら)**NDI Tools**: https://ndi.video/tools/

ビルドと起動:
```
cargo build --release
target\release\dmxctl.exe
```

### Mac
1. **Rust**: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. **Xcode のコマンドラインツール**: `xcode-select --install`
3. (NDIを使うなら)**NDI Tools**
4. (MacからWindows版も作るなら)**MinGW-w64**: `brew install mingw-w64`

### Linux
1. **Rust**: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. **ビルドに要るもの**(Debian / Ubuntu): `sudo apt install build-essential pkg-config libasound2-dev libudev-dev libsdl2-dev`(Fedora / Arch は README の「Linux で使う」)
3. **日本語フォント**: `sudo apt install fonts-noto-cjk`
4. **権限**: `sudo usermod -aG dialout,video,input $USER`(USB-DMX、KMSDRMの画面・キーボード/マウス)
5. (NDIを使うなら)**NDI SDK for Linux** のライブラリ: README の「Linux で NDI を使う」

ビルド:
```
bash ./scripts/build-mac.sh            # dist/mac/DMX Controller.app(ユニバーサル版)
bash ./scripts/build-windows-mingw.sh  # dist/windows/dmxctl.exe(MacからWindows版)
bash ./scripts/build-linux.sh          # dist/linux/dmxctl と dmxctl-kmsdrm(Linuxの上で。後者だけkmsdrm機能入り)
cargo run --release                    # 開発中の確認
```

### テスト
```
cargo test --release
cargo test --release --features kmsdrm   # Linux: KMSDRMの部分(キーの変換・拡大率)も
```

## 新しいPCで最初にやること

- 「出力」タブ: USB-DMXのポート名を選び直す(Mac `/dev/cu.usbserial-…` / Windows `COM3` / Linux `/dev/ttyUSB0` など)
- 2ページ目の設定パネル: NDIのソースを選び直す(ソース名にPC名が入るため)
- 初回のネットワーク利用時の確認(Macの「ローカルネットワーク」、Windowsのファイアウォール)は「許可」

## 今の構成(2026-10)

- ユニバース1(USB-DMX、COMポート方式): MINI SPOT 30 ×4(9ch、アドレス1/10/19/28)、Twinkle Ball(8ch、37)、GLP-185L ×2(45/50)、LASER SCAN 300 ×2(55/72、ARMが必要)
- LEDバー16本(90ピクセルRGBW=360ch): アプリ内のユニバース2〜17 → Art-Net ユニバース1〜16、送り先 192.168.11.10
- MIDIコントローラー: AKAI APC mini mk2(1ページ目=演出、2ページ目=設定。右の列5番目で切り替え)

## コードの地図

| ファイル | 中身 |
|---|---|
| `src/engine.rs` | 共有状態(`Core`)、値の合成(`render`)・出力ループ |
| `src/engine/` | `Core`の操作を分野ごとに: `binding.rs`(MIDIの受信・割り当ての実行) / `tempo.rs`(BPM・Link・MIDIクロック・音声・チェイス) / `palette.rs`(色・メイン/サブ色) / `scene.rs`(シーン・位置・フェード) / `pair.rs`(ペア) |
| `src/project.rs` | 保存する設定(`project.json`)と割り当ての種類(`Action`) |
| `src/layout.rs` | おすすめのMIDI配置(APC mini mk2) |
| `src/pixel.rs` | LEDバーの模様(面向けプリセット4セット) |
| `src/ndi.rs` | NDI受信(映像の平均色・音の色) |
| `src/ui_map.rs` | MIDI画面(メイン画面)。`src/ui_map/` に図の描画(`pad.rs`)と右のパネル(`stage.rs` / `settings.rs`) |
| `src/ui_laser.rs` | レーザータブ(位置・大きさ・動いてよい範囲をマウスで合わせる) |
| `src/motion.rs` / `src/engine/motion.rs` | ムービングの動き(SPOT4方式: 形×1周の長さ×揃え方×向き。傾きで計算) / それをパン/チルトに書き込む(`render`の中、エフェクトの後) |
| `src/keys.rs` / `src/ui_keys.rs` | キーボードの割り当て(`TriggerKind::Key`。番号=段×16+位置) / キーボードタブ(キーの絵) |
| `src/theme.rs` | 見た目(色・フォント) |
| `src/opendmx.rs` / `src/output.rs` | USB-DMX / Art-Net の出力 |
| `src/main.rs` / `src/app.rs` | 起動(eframe か KMSDRM かを選ぶ) / アプリ本体。`App::new`・`App::frame`・`App::shutdown` は eframe に頼らないので、どちらの土台からも呼べる |
| `src/kmsdrm.rs` / `src/kmsdrm/keys.rs` | Linux・`kmsdrm`機能のときだけ: デスクトップ環境の無い画面に、SDL2(KMSDRM)+ OpenGL ES(egui_glow)で描くループ / SDLのキーをeguiのキーに直す表 |

## 開発用の環境変数・コマンド

| もの | 使い方 |
|---|---|
| `DMXCTL_CONFIG_DIR=フォルダ` | 本番の設定に触れずに、別の設定で起動 |
| `DMXCTL_NO_MIDI=1` | MIDI機器に自動接続しない(本番と2つ起動するとき) |
| `DMXCTL_SHOT=保存先.png` + `DMXCTL_TAB=タブ名` / `DMXCTL_PAGE=2` / `DMXCTL_SIZE=1280x800` / `DMXCTL_UNIVERSE=2` | 画面を撮って保存して終了(見た目の確認用) |
| `DMXCTL_PERF=1` | 描画の時間を表示 |
| `DMXCTL_KMSDRM=1` / `=0` / `--kmsdrm` | Linux(`kmsdrm`機能入り): KMSDRMで必ず起動する / 使わない。指定が無ければ `DISPLAY` と `WAYLAND_DISPLAY` が無いときに使う |
| `DMXCTL_SCALE=1.5` | KMSDRMの画面の拡大率(指定が無ければ画面の大きさから自動) |
| `DMXCTL_D2XX_LIB` / `DMXCTL_NDI_LIB` | ライブラリの場所を直接指定 |
| `DMXCTL_CHECK_ONLY=1 cargo check --target …` | C++をビルドせずに、他のOS向けに型チェックだけ |
| `dmxctl --apply-layout` | 保存してある設定を、おすすめのMIDI配置にする(先にバックアップを残す) |

## 実機の確認用テスト(`--ignored` で実行)

```
DMXCTL_TEST_PORT=/dev/cu.usbserial-XXXX cargo test --release hw_serial -- --ignored --nocapture    # USB-DMXに3秒送る
DMXCTL_D2XX_LIB=third_party/ftd2xx/mac/libftd2xx.dylib cargo test --release hw_d2xx -- --ignored --nocapture
DMXCTL_TEST_PORT=… cargo test --release hw_spot_sweep -- --ignored --nocapture        # ムービングのモード確認(9ch/11ch/内蔵)
DMXCTL_TEST_PORT=… DMXCTL_FX=円運動 cargo test --release real_moves_demo -- --ignored --nocapture  # 保存した設定の動きを実機に流す
DMXCTL_NDI_SOURCE=ソース名の一部 cargo test --release hw_ndi -- --ignored --nocapture
DMXCTL_SHEET_DIR=保存先 cargo test --release preset_sheet -- --ignored               # LEDバーの模様の一覧画像
```

## はまりどころ

- **USB-DMXが開けない(Resource busy)**: QLC+ など別の照明ソフトが使っていないか確認する
- **NDIのライブラリは同梱しない**(配布条件のため)。インストール済みの NDI Tools(Linux は NDI SDK)のものを読み込む
- **D2XX**: Mac版は `third_party/ftd2xx/mac/libftd2xx.dylib` をアプリに同梱。Windowsは FTDIドライバの `ftd2xx.dll`
- **レーザーの安全処理**(`engine.rs` の `guard_values` / `emergency_stop`): 機材定義の `forbid`(オートショー・サウンド・パン/チルトの自動の動き)と、機材ごとの `limits`(動いてよい範囲)は、出力の直前に必ず直す。内蔵の定義を上書きしても `forbid` は引き継ぐ。落ちたとき・終了のシグナル・シャットダウン(Windowsは見えないウィンドウで `WM_ENDSESSION` を受ける。`watch_windows_shutdown`)・Mac/Linuxで画面を通らずに exit() したとき(`atexit`)では、安全なフレーム(レーザーOFF・明るさ0)を約0.3秒送ってから終わる
- **KMSDRM**: 画面が出ない(「画面を開けませんでした」)ときは、`video` グループに入っているか、デスクトップ環境など他のアプリが画面を使っていないかを確認。キー・マウスが効かないときは `input` グループ。`DMXCTL_SHOT` での画面の撮影は KMSDRM でも使える(SSHから確かめるのに便利)
- **ムービングのチルト**: 128が真上。動きのプリセットはチルトを大きく振る作り
- 実行中のアプリは5秒ごとに設定を自動保存するので、`project.json` を直接書き換えるときは先にアプリを閉じる

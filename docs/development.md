# 開発ガイド

ソースからビルドする手順、テスト、コードの構成をまとめます。守ることや書き方などの開発のルールは [AGENTS.md](../AGENTS.md) にあります。

## ソースを取ってくる

```
git clone --recursive https://github.com/TechnoTUT/dmxctl
```

NDI 機能はサブモジュール `moip-engine/` 内の `ndi` クレートを使うため、`--recursive` が必要です。付け忘れた場合は `git submodule update --init` を実行します。`scripts/build-*.sh` は、無ければ自動で取ってきます。

一度取得すればオフラインでもビルドでき、ネットの無いPCへフォルダごとコピーして使えます。crates.io のクレートは `~/.cargo` のキャッシュを参照するため、2GB以上ある `target/` と `dist/` はコピー不要です。

## ビルドに要るもの

### Windows

1. **Rust**: https://rustup.rs の `rustup-init.exe` を既定のまま入れます。MSVC版になります
2. **Visual Studio Build Tools 2022** の「C++ によるデスクトップ開発」: Rust の MSVC版がリンクに使います
3. **FTDIドライバ**: FT232R の USB-DMX を挿すと、ふつうは Windows Update で入ります。COMポート方式と D2XX方式の両方が使えます
4. NDI を使うなら **NDI Tools**: https://ndi.video/tools/

### Mac

1. **Rust**: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. **Xcode のコマンドラインツール**: `xcode-select --install`
3. NDI を使うなら **NDI Tools**
4. Mac から Windows版も作るなら **MinGW-w64**: `brew install mingw-w64`

### Linux

1. **Rust**: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. **ビルドに要るもの**
   - Ubuntu / Debian: `sudo apt install build-essential pkg-config cmake libasound2-dev libudev-dev libdrm-dev libgbm-dev libegl-dev libgles-dev libxext-dev`
   - Fedora: `sudo dnf install gcc-c++ pkgconf cmake alsa-lib-devel systemd-devel libatomic libdrm-devel mesa-libgbm-devel mesa-libEGL-devel mesa-libGLES-devel libXext-devel`
   - Arch: `sudo pacman -S base-devel cmake alsa-lib libdrm mesa libxext`
3. **日本語フォント**: `sudo apt install fonts-noto-cjk`
4. **権限**: `sudo usermod -aG dialout,video,input $USER`。USB-DMX と、KMS/DRM の画面・キーボード・マウスに要ります
5. NDI を使うなら **NDI SDK for Linux** のライブラリ。入れ方は [Linux で NDI を使う](platforms.md#linux-で-ndi-を使う)
6. Linux から Windows版も作るなら **MinGW-w64**: `sudo apt install mingw-w64`

## ビルド

```
cargo run --release                    # 開発中の確認
bash ./scripts/build-mac.sh            # Macの上で dist/mac/DMX Controller.app を作る。ユニバーサル版
bash ./scripts/build-windows-mingw.sh  # Mac・Linux から dist/windows/dmxctl.exe を作る
bash ./scripts/build-linux.sh          # Linuxの上で dist/linux/dmxctl を作る
```

- Windows の上では `cargo build --release` で `target\release\dmxctl.exe` ができます。追加のDLLは要らず、exe 1つで動きます
- Linux 版は SDL2 を静的リンクしているので、1つの実行ファイルで、デスクトップ環境の有る画面と無い画面の両方に対応します
- `cargo check --target …` で、ほかのOS向けの型チェックだけができます

### CI と Release

`.github/workflows/build.yml` で次を行います。

| きっかけ | すること |
|---|---|
| push | テスト |
| PR | テストと、Windows 用・Linux 用のビルドが通るかの確認 |
| `v0.2.0` のような `v` で始まるタグ | Mac 用・Windows 用・Linux 用をビルドし、GitHub の Release に添付する。Windows と Linux は実行ファイルそのまま、Mac は .app の zip |

Mac が手元に無くても、タグを push すれば Mac 用ができます。

## テスト

```
cargo test --release
```

チャンネル定義の抜け・重複、16bit値、ARM・ブラックアウト、レーザーの禁止値・動いてよい範囲、MIDI・キーボードの割り当て、ムービングの動き、シーンのフェード、エフェクト、Art-Netパケットなどを検証します。Linux では、KMS/DRM のキーの変換と拡大率も含みます。

画面を見ずに動作を確かめるには、CLI で起動中のアプリや仮想時間のシミュレーションを操作し、最終DMX値を検証します。使い方は [CLI](cli.md) に、例は [examples/cli/](../examples/cli/scenario.json) にあります。

### 実機の確認用テスト

`--ignored` を付けたときだけ動くテストです。

```
DMXCTL_TEST_PORT=/dev/cu.usbserial-XXXX cargo test --release hw_serial -- --ignored --nocapture    # USB-DMXに3秒送る
DMXCTL_D2XX_LIB=/usr/local/lib/libftd2xx.dylib cargo test --release hw_d2xx -- --ignored --nocapture
DMXCTL_TEST_PORT=… cargo test --release hw_spot_sweep -- --ignored --nocapture        # ムービングの 9ch / 11ch / 内蔵 のモードを確かめる
DMXCTL_TEST_PORT=… DMXCTL_FX=円運動 cargo test --release real_moves_demo -- --ignored --nocapture  # 保存した設定の動きを実機に流す
DMXCTL_NDI_SOURCE=ソース名の一部 cargo test --release hw_ndi -- --ignored --nocapture
DMXCTL_SHEET_DIR=保存先 cargo test --release preset_sheet -- --ignored               # LEDバーの模様の一覧画像を作る
cargo test --release perf -- --ignored --nocapture                                   # ベンチマーク
```

## 開発用の環境変数・コマンド

| もの | 使い方 |
|---|---|
| `DMXCTL_CONFIG_DIR=フォルダ` | 本番の設定に触れずに、別の設定で起動する |
| `DMXCTL_NO_MIDI=1` | MIDI機器に自動でつながない。本番と2つ起動するときに使う |
| `DMXCTL_SHOT=保存先.png` + `DMXCTL_TAB=タブ名` / `DMXCTL_PAGE=2` / `DMXCTL_SIZE=1280x800` / `DMXCTL_UNIVERSE=2` | 画面を撮って保存して終了する。見た目の確認用で、KMS/DRM でも使える。`DMXCTL_SHOT_FRAMES=150` で、撮るまでのフレーム数を延ばせる。操作を入れてから撮るときに使う |
| `DMXCTL_KEY_COLOR=1` | キーボードタブを色の段で開く |
| `DMXCTL_PERF=1` | 画面下に、描画の回数/秒と1回あたりの時間を出す |
| `DMXCTL_KMSDRM=1` / `=0` / `dmxctl kmsdrm` | Linux で、KMS/DRM で必ず起動する / 使わない。指定が無ければ `DISPLAY` と `WAYLAND_DISPLAY` が無いときに使う |
| `DMXCTL_SCALE=1.5` | KMS/DRM の画面の拡大率。指定が無ければ画面の大きさから自動で決める |
| `dmxctl headless` | 画面を出さずに動かす。`Host::run_headless` で、OSC受信・Link・オートを必ずONにする |
| `DMXCTL_D2XX_LIB` / `DMXCTL_NDI_LIB` | ライブラリの場所を直接指定する |
| `DMXCTL_CARABINER_PORT=17000` | Ableton Link の窓口の Carabiner につなぐポート。指定が無ければ 17000 |
| `dmxctl apply-layout` | 保存してある設定を、おすすめのMIDI配置にする。先にバックアップを残す |
| `--remote-listen ADDR` / `dmxctl connect HOST[:PORT]` | サーバーとしてクライアントを待ち受ける。アドレスは必須で、ポートの既定は 9300 / サーバーにつなぐクライアントとして起動する。同じPCで試すなら `dmxctl serve --project examples/cli/project.json --remote-listen 127.0.0.1:9300` と `DMXCTL_NO_MIDI=1 dmxctl connect 127.0.0.1` |
| `--remote-allow-safety-edit` | サーバーで、クライアントから機材定義の安全の設定を変えることを許す。既定では自動操作の窓口と同じく変えられない |

起動オプションの一覧は `dmxctl help` で出ます。`--headless`・`--apply-layout`・`--kmsdrm`・`DMXCTL_HEADLESS=1` という以前の書き方も使えます。

## コードの地図

ソースは3つの層に分けています。参照してよい向きは `ui` → `backend` → `core` だけです。`tests/layers.rs` が、逆向きの参照が入っていないかを調べます。層ごとに別々に保守できるので、画面を直すときに出力の中身を、機器の対応を足すときに画面を読む必要はありません。

| 層 | 役割 |
|---|---|
| `src/core/` | 設定の形、値の合成、機器とプロトコルの入出力。NDI・OSC・USB-DMX・Art-Net・MIDI・音声・Link はここ |
| `src/backend/` | 画面なしで動かす部分。起動と保存、遠隔操作、ブラウザの操作パネル、自動操作用のCLI |
| `src/ui/` | 画面。eframe と KMS/DRM、タブごとの描画、見た目 |

### core

| ファイル | 中身 |
|---|---|
| `core/engine.rs` | 共有状態の `Core`、値の合成 `render`、出力ループ |
| `core/engine/` | `Core` の操作を分野ごとに分けたもの。`binding.rs` は MIDIの受信と割り当ての実行、`tempo.rs` は BPM・Link・MIDIクロック・音声・チェイス、`palette.rs` は色とメイン/サブ色、`scene.rs` はシーン・位置・フェード、`pair.rs` はペア |
| `core/engine/auto.rs` | オート。OSCで曲の色・明るさ・フレーズが届いているかを見て、届かないときは待機の見た目を出力の直前、フラッシュの後に重ねる。フレーズのプリセットの見た目を当てる `apply_look` と、BLTの様子を返す `blt_state` もここ |
| `core/motion.rs` / `core/engine/motion.rs` | ムービングの動きを、形×1周の長さ×揃え方×向きで傾きとして計算する / それを `render` の中、エフェクトの後でパン/チルトに書き込む |
| `core/live.rs` | 画面・MIDIから実行中の状態を変える操作 `LiveOp`。必ず `Core::live` を通す。クライアントでは、トレイト `LiveForward` を通してサーバーへ送る |
| `core/project.rs` | 保存する設定 `project.json` と、割り当ての種類 `Action` |
| `core/fixture.rs` | 機材定義。内蔵のものはリポジトリの `fixtures/` から取り込む |
| `core/keys.rs` | キーボードの割り当て `TriggerKind::Key`。番号は 段×16+位置 |
| `core/color.rs` / `core/look.rs` / `core/pixel.rs` | 色 / フレーズの見た目 / LEDバーの模様のプリセット4セット |
| `core/led.rs` / `core/layout.rs` | パッドLEDの計算 / APC mini mk2 のおすすめのMIDI配置 |
| `core/recorder.rs` / `core/trace.rs` / `core/hold.rs` | 実行の記録の口 / 指定した機材の合成の段階の記録 / 遠隔操作で押している間の記録。`Core` が持つので、ここに置く。ファイルへの書き込みや問い合わせは `backend` 側 |
| `core/io/opendmx.rs` / `core/io/output.rs` | USB-DMX / Art-Net の出力 |
| `core/io/ndi.rs` | NDI受信で、映像の平均色と音の色を取る。読み込みと呼び出しは、Cargo.toml の `moip-ndi` にある moip-engine の `ndi` クレートで行う。上げるときは `git -C moip-engine checkout <コミット>` してから `git add moip-engine` |
| `core/io/osc.rs` | OSCの受信と、アドレスごとの操作 |
| `core/io/midi.rs` / `core/io/apcmini.rs` | MIDI機器の入出力 / APC mini mk2 のパッド・ボタン・フェーダーの番号。AKAI の Communications Protocol v1.0 に基づく |
| `core/io/audio.rs` / `core/io/link.rs` / `core/io/carabiner.rs` | 音声の入力 / Ableton Link / Link の窓口の Carabiner への接続とダウンロード |

### backend

| ファイル | 中身 |
|---|---|
| `backend/host.rs` | 画面によらずに動く部分の `Host`。Core・出力ループ・MIDI/OSC/Link/NDI/音声/操作パネル・自動操作の窓口・保存を持つ。画面は `Host` を1つ持ち、headless と serve は `Host` だけで動く |
| `backend/project_patch.rs` | 画面の1フレームで変わったプロジェクトの差分 `Change` を作って当てる。クライアントの編集をサーバーへ送るのに使う |
| `backend/remote.rs` | `--remote-listen` のサーバーと `dmxctl connect` のクライアントの通信。食い違いを防ぐ仕組みは先頭のコメントにある |
| `backend/cli.rs` / `backend/control.rs` / `backend/commands.rs` / `backend/editing.rs` / `backend/recording.rs` / `backend/diagnostics.rs` | 自動操作・検証用の [CLI](cli.md)。順に、サブコマンド / 制御用の接続 / 操作 / 設定の編集と検証 / 記録のファイル書き込み / `explain` などの診断 |
| `backend/controls.rs` | 制御用の接続やパネルから、割り当てを押したり離したりする操作 |
| `backend/web.rs` / `backend/web/panel.html` | ブラウザの操作パネル。std の TCP で最小限の HTTP を話す。`GET /` でページ、`GET /api/state` で様子、`POST /api/cmd` で操作。合言葉は `X-Pin` ヘッダー |

### ui

| ファイル | 中身 |
|---|---|
| `main.rs` / `ui/app.rs` | 起動 / 画面。起動では最初の引数で動かし方を選び、eframe か KMS/DRM かを決める。`App::new`・`App::frame`・`App::shutdown` は eframe に頼らないので、どちらの土台からも呼べる |
| `ui/map.rs` | メイン画面のMIDIタブ。`ui/map/` に、図の描画 `pad.rs` と右のパネル `stage.rs` / `settings.rs` がある |
| `ui/keys.rs` | キーボードの絵を出すキーボードタブ |
| `ui/laser.rs` / `ui/auto.rs` | レーザータブ / オートタブ。オートタブは、フレーズごとの見た目のプリセット `AutoPreset` / `PhraseLook` の表 |
| `ui/` の `binding.rs`・`control.rs`・`midi.rs`・`output.rs`・`pair.rs`・`patch.rs`・`profile.rs`・`sequence.rs`・`show.rs` | 残りのタブ。ファイル名がタブに対応する |
| `ui/theme.rs` / `ui/shot.rs` | 色・フォントなどの見た目 / 画面を撮って保存する |
| `ui/kmsdrm.rs` / `ui/kmsdrm/` | Linuxだけ。デスクトップ環境の無い画面に、SDL2 の KMSDRM と OpenGL ES の egui_glow で描くループ / キーの変換・タッチ |

### そのほか

| ファイル | 中身 |
|---|---|
| `Containerfile` / `.dockerignore` / `deploy/kubernetes/` | コンテナのイメージと Kubernetes のマニフェスト。画面を使わず `dmxctl serve` だけを動かす。手順は [コンテナで動かす](container.md) |

単体テストは層ごとに `src/core/tests/`・`src/backend/tests/`・`src/ui/tests/` に置きます。複数の層をまたぐ検証は `tests/` に置きます。

### 設計のポイント

| もの | 中身 |
|---|---|
| 機材定義 | 「機材定義」タブで、モード・チャンネル・値の範囲・16bit・安全値・カテゴリを作成・編集する。実体は設定フォルダの `fixtures/*.json` で、手書きもできる。内蔵の定義はリポジトリの `fixtures/` |
| 対象の指定 | 機材単体 / グループ / 同機種すべて / すべて。`dimmer` や `color` などの機能名で指定するので、機種が違っても同じ割り当てが効く |
| パッドの動作 | 連続のフェーダー / 押している間 / トグル / ワンショット |
| 割り当ての入力 | MIDI のノートと CC はページ別にできる。ほかにパソコンのキーボード、OSCアドレス |
| エフェクト | 対象チャンネルを 最小〜最大 で周期的に動かす。波形はサイン/三角/ノコギリ/矩形/ランダムで、位相と機材ごとのずれを付けられる |
| 出力 | 専用スレッドで40Hz固定。画面の更新頻度とは関係しない |

## 性能の目安

14論理コアのPCで、機材8台・エフェクト・チェイス・ペアを動かしながら Art-Net を40Hzで送る、負荷をかけた構成で測りました。

| 項目 | 結果 |
|---|---|
| 1フレームの合成。機材16台・割り当て64件・エフェクト4件・チェイス2件・ペア3件 | 約12µs。40Hzの周期25,000µsの0.05% |
| 80個のパッドLEDの計算 / MIDI 1メッセージ | 約30µs / 約0.4µs |
| DMX出力の間隔 | 平均25.0msで40回/秒。p99は26ms前後 |
| **CPUを全コア使い切っているとき** | 出力は40回/秒のまま。間隔の最悪は+10ms程度 |
| メモリ | 起動後 約90MB の Private で一定。100秒以上動かしても増えない |
| 1コア換算のCPU | コントロール/マップ表示 約2〜3% / **前面が他のアプリのとき 約1.4%** / 最小化 0% / モニター表示 約11% |
| LEDバー 32ピクセル×2本 + ピクセルエフェクト2つを追加した構成 | 1フレームの合成 約36µs。周期の0.14% |
| Link・OSCを有効にしたとき | コントロール表示 約1〜2% / Linkの通信で最小化 約1% / **OSCを60回/秒受け続けている間 約5%**。出力は40回/秒・p99 26ms のまま |

- DMXの出力は画面とは別のスレッドで動くので、画面の表示状態や、画面が重いことは出力の間隔に影響しません
- 描画は操作時・MIDI入力時・拍の頭に限定し、1回あたり約10〜15msのCPU負荷を抑えています。モニター表示中は10fps、ウィンドウが前面でないときは通常1秒に1回、モニターは4fpsです
- 映像などの重いアプリと同居するときは、このウィンドウを最小化するか、他のアプリを前に出しておくと、CPU使用はほぼゼロになります

## はまりどころ

- **USB-DMXが開けない、Resource busy と出る**: QLC+ など別の照明ソフトが使っていないか確かめる
- **NDIのライブラリは同梱しない**: 配布条件のためです。インストール済みの NDI Tools、Linux では NDI SDK のものを読み込みます。探す順は `DMXCTL_NDI_LIB` → macOS の NDI Tools のアプリの中 → `NDI_RUNTIME_DIR_V6`/`V5` → 各OSの標準の場所 です。後ろの2つは moip-engine の `ndi` クレートが探します
- **FTDIのD2XXライブラリは同梱しない**: 配布条件のためです。利用者がFTDIの配布元から入れたものを、実行時に読み込みます。探す順は `DMXCTL_D2XX_LIB` → 設定フォルダに置いたもの → 各OSの標準の場所 です。Windowsは FTDIドライバの `ftd2xx.dll` を使います
- **レーザーの安全処理**: 場所は `core/engine.rs` の `guard_values` と `emergency_stop` です。機材定義の `forbid` で禁止したオートショー・サウンド・パン/チルトの自動の動きと、機材ごとの `limits` で決めた動いてよい範囲は、出力の直前に必ず直します。内蔵の定義を上書きしても `forbid` は引き継ぎます。落ちたとき・終了のシグナル・シャットダウン・Mac/Linuxで画面を通らずに `exit()` したときは、レーザーOFF・明るさ0の安全なフレームを約0.3秒送ってから終わります。Windows のシャットダウンは、見えないウィンドウで `WM_ENDSESSION` を受けて検知します。`watch_windows_shutdown` です。`exit()` のときは `atexit` を使います
- **KMS/DRM**: 「画面を開けませんでした」と出て画面が出ないときは、`video` グループに入っているか、デスクトップ環境など他のアプリが画面を使っていないかを確かめる。キー・マウスが効かないときは `input` グループ
- **ムービングのチルト**: 128が真上です。動きのプリセットはチルトを大きく振る作りです
- 実行中のアプリは5秒ごとに設定を自動保存するので、`project.json` を直接書き換えるときは先にアプリを閉じる

## ライセンスについて

Ableton Link は GPLv2 以降なので、このアプリには入れていません。Link を使う部分は、別のプログラムの [Carabiner](https://github.com/Deep-Symmetry/carabiner) に TCP でつなぐ作りです。Carabiner は同梱せず、使う人がボタンか `dmxctl carabiner install` を押したときだけ、配布元から取ります。そのため、このアプリを配るときに、Link のソースを公開する必要はありません。

Carabiner を一緒に配りたいときは、Carabiner と Link のソースを公開し、GPL に従ってください。

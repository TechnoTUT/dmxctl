# 開発ガイド

ソースからビルドする手順、テスト、コードの構成をまとめます。守るべきことや書き方などの開発のルールは [AGENTS.md](../AGENTS.md) にあります。

## ソースを取得する

```
git clone --recursive https://github.com/TechnoTUT/dmxctl
```

NDI 機能はサブモジュール `moip-engine/` 内の `ndi` クレートを使うため、`--recursive` が必要です。付け忘れた場合は `git submodule update --init` を実行します。`scripts/build-*.sh` は、無ければ自動で取得します。

一度取得すればオフラインでもビルドでき、ネットワークに接続していない PC へフォルダごとコピーして利用できます。crates.io のクレートは `~/.cargo` のキャッシュを参照するため、2GB 以上ある `target/` と `dist/` はコピーする必要はありません。

## ビルドに必要なもの

### Windows

1. **Rust**: https://rustup.rs の `rustup-init.exe` を既定のまま導入します。MSVC 版になります
2. **Visual Studio Build Tools 2022** の「C++ によるデスクトップ開発」: Rust の MSVC 版がリンクに使用します
3. **FTDI ドライバ**: FT232R の USB-DMX を接続すると、通常は Windows Update で導入されます。COMポート方式と D2XX 方式の両方が利用できます
4. NDI を利用するなら **NDI Tools**: https://ndi.video/tools/

### Mac

1. **Rust**: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. **Xcode のコマンドラインツール**: `xcode-select --install`
3. NDI を利用するなら **NDI Tools**
4. Mac から Windows 版も作成するなら **MinGW-w64**: `brew install mingw-w64`

### Linux

1. **Rust**: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. **ビルドに必要なもの**
   - Ubuntu / Debian: `sudo apt install build-essential pkg-config cmake libasound2-dev libudev-dev libdrm-dev libgbm-dev libegl-dev libgles-dev libxext-dev`
   - Fedora: `sudo dnf install gcc-c++ pkgconf cmake alsa-lib-devel systemd-devel libatomic libdrm-devel mesa-libgbm-devel mesa-libEGL-devel mesa-libGLES-devel libXext-devel`
   - Arch: `sudo pacman -S base-devel cmake alsa-lib libdrm mesa libxext`
3. **日本語フォント**: `sudo apt install fonts-noto-cjk`
4. **権限**: `sudo usermod -aG dialout,video,input $USER`。USB-DMX と、KMS/DRM の画面・キーボード・マウスに必要です
5. NDI を利用するなら **NDI SDK for Linux** のライブラリ。導入方法は [Linux で NDI を使う](platforms.md#linux-で-ndi-を使う)
6. Linux から Windows 版も作成するなら **MinGW-w64**: `sudo apt install mingw-w64`

## ビルド

```
cargo run --release                    # 開発中の確認
bash ./scripts/build-mac.sh            # Mac の上で dist/mac/DMX Controller.app を作る。ユニバーサル版
bash ./scripts/build-windows-mingw.sh  # Mac・Linux から dist/windows/dmxctl.exe を作る
bash ./scripts/build-linux.sh          # Linux の上で dist/linux/dmxctl を作る
cargo build --release -p dmxctl-server # 画面を持たない dmxctl-server だけを作る。cmake や画面用のライブラリは不要
```

- Windows の上では `cargo build --release` で `target\release\dmxctl.exe` ができます。追加の DLL は不要で、exe 1つで動作します
- Linux 版は SDL2 を静的リンクしているので、1つの実行ファイルで、デスクトップ環境の有る画面と無い画面の両方に対応します
- `cargo check --target …` で、他のOS向けの型チェックだけができます

### CI と Release

`.github/workflows/build.yml` で次を行います。

| きっかけ | 処理 |
|---|---|
| push | テスト |
| PR | テストと、Windows 用・Linux 用のビルドが通るかの確認 |
| `v0.2.0` のような `v` で始まるタグ | Mac 用・Windows 用・Linux 用をビルドし、GitHub の Release に添付する。Windows と Linux は実行ファイルそのまま、Mac は .app の zip |

Mac が手元に無くても、タグを push すれば Mac 用が作成されます。

## テスト

```
cargo test --release
```

チャンネル定義の欠落・重複、16bit 値、ARM・ブラックアウト、レーザーの禁止値・動いてよい範囲、MIDI・キーボードの割り当て、ムービングの動き、シーンのフェード、エフェクト、Art-Net パケットなどを検証します。Linux では、KMS/DRM のキーの変換と拡大率も含みます。

画面を見ずに動作を確認するには、CLI で起動中のアプリや仮想時間のシミュレーションを操作し、最終 DMX 値を検証します。使い方は [CLI](cli.md) に、例は [examples/cli/](../examples/cli/scenario.json) にあります。

### 実機の確認用テスト

`--ignored` を付けたときだけ動作するテストです。

```
DMXCTL_TEST_PORT=/dev/cu.usbserial-XXXX cargo test --release hw_serial -- --ignored --nocapture    # USB-DMX に3秒送信する
DMXCTL_D2XX_LIB=/usr/local/lib/libftd2xx.dylib cargo test --release hw_d2xx -- --ignored --nocapture
DMXCTL_TEST_PORT=… cargo test --release hw_spot_sweep -- --ignored --nocapture        # ムービングの 9ch / 11ch / 内蔵 のモードを確認する
DMXCTL_TEST_PORT=… DMXCTL_FX=円運動 cargo test --release real_moves_demo -- --ignored --nocapture  # 保存した設定の動きを実機に流す
DMXCTL_NDI_SOURCE=ソース名の一部 cargo test --release hw_ndi -- --ignored --nocapture
DMXCTL_SHEET_DIR=保存先 cargo test --release preset_sheet -- --ignored               # LEDバーの模様の一覧画像を作成する
cargo test --release perf -- --ignored --nocapture                                   # ベンチマーク
```

## 開発用の環境変数・コマンド

| もの | 利用方法 |
|---|---|
| `DMXCTL_CONFIG_DIR=フォルダ` | 本番の設定に触れずに、別の設定で起動する |
| `DMXCTL_NO_MIDI=1` | MIDI 機器に自動で接続しない。本番と2つ起動するときに使う |
| `DMXCTL_SHOT=保存先.png` + `DMXCTL_TAB=タブ名` / `DMXCTL_PAGE=2` / `DMXCTL_SIZE=1280x800` / `DMXCTL_UNIVERSE=2` | 画面を撮影して保存して終了する。見た目の確認用で、KMS/DRM でも利用できる。`DMXCTL_SHOT_FRAMES=150` で、撮影までのフレーム数を延長できる。操作を入れてから撮影するときに使う |
| `DMXCTL_KEY_COLOR=1` | キーボードタブを色の段で開く |
| `DMXCTL_PERF=1` | 画面下に、描画の回数/秒と1回あたりの時間を表示する |
| `DMXCTL_KMSDRM=1` / `=0` / `dmxctl kmsdrm` | Linux で、KMS/DRM で必ず起動する / 利用しない。指定が無ければ `DISPLAY` と `WAYLAND_DISPLAY` が無いときに利用する |
| `DMXCTL_SCALE=1.5` | KMS/DRM の画面の拡大率。指定が無ければ画面の大きさから自動で決定する |
| `dmxctl headless` | 画面を出さずに動作させる。`Host::run_headless` で、OSC 受信・Link・オートを必ず ON にする |
| `DMXCTL_D2XX_LIB` / `DMXCTL_NDI_LIB` | ライブラリの場所を直接指定する |
| `DMXCTL_NDI_DIR` / `NDI_ACCEPT_LICENSE=yes` / `NDI_SDK_URL` | コンテナの `dmxctl-fetch-ndi` が利用する。NDI のライブラリの置き場所 `/data/ndi` / 使用許諾に同意した印 / ダウンロード元 |
| `DMXCTL_CARABINER_PORT=17000` | Ableton Link の窓口の Carabiner に接続するポート。指定が無ければ 17000 |
| `DMXCTL_CARABINER_HOST=ホスト名` | 別の PC や別の Pod で動作している Carabiner に接続する。指定が無ければ同じ PC の Carabiner に接続し、導入済みなら起動する。指定したときは起動しない。ポートは `DMXCTL_CARABINER_PORT` |
| `dmxctl apply-layout` | 保存してある設定を、おすすめの MIDI 配置にする。先にバックアップを残す |
| `--remote-listen ADDR` / `dmxctl connect HOST[:PORT]` | サーバーとしてクライアントからの接続を待ち受ける。アドレスは必須で、ポートの既定は 9300 / サーバーに接続するクライアントとして起動する。同じ PC で試すなら `dmxctl serve --project examples/cli/project.json --remote-listen 127.0.0.1:9300` と `DMXCTL_NO_MIDI=1 dmxctl connect 127.0.0.1` |
| `--remote-allow-safety-edit` | サーバーで、クライアントから機材定義の安全の設定を変更することを許可する。既定では自動操作の窓口と同様に変更できない |

起動オプションの一覧は `dmxctl help` で表示されます。`--headless`・`--apply-layout`・`--kmsdrm`・`DMXCTL_HEADLESS=1` という以前の書き方も利用できます。

## コードの地図

ソースは5つのクレートに分けています。参照してよい向きは `ui` → `backend` → `core` → `io` → `model` だけです。逆向きの参照はコンパイルが通らず、`tests/layers.rs` が、モデルに機器のクレートが、`core` に画面のクレートが入り込んでいないかを調べます。層ごとに別々に保守できるので、画面を修正するときに出力の中身を、機器の対応を追加するときに画面を読む必要はありません。

| 層 | 役割 |
|---|---|
| `crates/model/` | 保存する設定・機材定義・色・動き・キーの表。機器にも画面にも依存しないデータと規則 |
| `crates/io/` | 機器とプロトコルの、エンジンの状態を知らない部分。USB-DMX・Art-Net の送出、OSC・MIDI・NDI・音声の入出力、Carabiner への接続。`core` の `io` は、これらを `Core` につなぐ橋渡し |
| `crates/core/` | 設定の形、値の合成、機器とプロトコルの入出力。NDI・OSC・USB-DMX・Art-Net・MIDI・音声・Link はここ |
| `crates/backend/` | 画面なしで動作する部分。起動と保存、遠隔操作、ブラウザの操作パネル、自動操作用のCLI |
| `crates/ui/` | 画面。eframe と KMS/DRM、タブごとの描画、見た目 |
| `crates/server/` | 画面を持たない実行ファイル `dmxctl-server`。`core` と `backend` だけに依存し、画面のクレートを含まない。コンテナはこれを使う |

### core

| ファイル | 中身 |
|---|---|
| `crates/core/src/engine.rs` | 共有状態の `Core`。合成と出力の部品は `engine/` の `runtime.rs`・`render.rs`・`guard.rs`・`safety.rs`・`output_loop.rs`・`glide.rs` に分けてある。順に、実行中の状態 / ユニバースごとのフレームの合成 / 出力の直前の値の補正 / 異常時の安全なフレーム / 40Hzの出力スレッド / OSCで受けた値の追従 |
| `crates/core/src/engine/` | `Core` の操作を分野ごとに分けたもの。`binding.rs` は MIDIの受信と割り当ての実行、`tempo.rs` は BPM・Link・MIDIクロック・音声・チェイス、`palette.rs` は色とメイン/サブ色、`scene.rs` はシーン・位置・フェード、`pair.rs` はペア |
| `crates/core/src/engine/auto.rs` | オート。OSCで曲の色・明るさ・フレーズが届いているかを見て、届かないときは待機の見た目を出力の直前、フラッシュの後に重ねる。フレーズのプリセットの見た目を当てる `apply_look` と、BLTの様子を返す `blt_state` もここ |
| `crates/model/src/motion.rs` / `crates/core/src/engine/motion.rs` | ムービングの動きを、形×1周の長さ×揃え方×向きで傾きとして計算する / それを `render` の中、エフェクトの後でパン/チルトに書き込む |
| `crates/core/src/live.rs` | 画面・MIDI から実行中の状態を変更する操作 `LiveOp`。必ず `Core::live` を通す。クライアントでは、トレイト `LiveForward` を通してサーバーへ送信する |
| `crates/model/src/project.rs` | 保存する設定 `project.json` と、割り当ての種類 `Action` |
| `crates/model/src/fixture.rs` | 機材定義。内蔵のものはリポジトリの `fixtures/` から取り込む |
| `crates/model/src/keys.rs` | キーボードの割り当て `TriggerKind::Key`。番号は 段×16+位置 |
| `crates/model/src/color.rs` / `crates/core/src/look.rs` / `crates/model/src/pixel.rs` | 色 / フレーズの見た目 / LEDバーの模様のプリセット4セット |
| `crates/core/src/led.rs` / `crates/model/src/layout.rs` | パッドLEDの計算 / APC mini mk2 のおすすめの MIDI 配置 |
| `crates/core/src/observe.rs` | 実行の観測 `Observe`。`Runtime` の `observe` が持つ。記録の口と診断をまとめ、観測の有無をほかの実行中の値と混ぜない |
| `crates/core/src/recorder.rs` / `crates/core/src/trace.rs` / `crates/core/src/hold.rs` | 実行の記録の口 / 指定した機材の合成の段階の記録 / 遠隔操作で押している間の記録。`Core` が保持するので、ここに置く。ファイルへの書き込みや問い合わせは `backend` 側 |
| `crates/io/src/opendmx.rs` / `crates/io/src/output.rs` | USB-DMX / Art-Net・sACN (E1.31) の出力。sACN の送信元の識別子(CID)は起動ごとに生成し、停止時は終了の合図を3回送信する |
| `crates/io/src/ndi.rs` / `crates/core/src/io/ndi.rs` | NDI受信で、映像の平均色と音の色を取る。読み込みと呼び出しは、`crates/io/Cargo.toml` の `moip-ndi` にある moip-engine の `ndi` クレートで行う。上げるときは `git -C moip-engine checkout <コミット>` してから `git add moip-engine` |
| `crates/core/src/io/osc.rs` | OSC の受信と、アドレスごとの操作 |
| `crates/io/src/midi.rs` / `crates/core/src/io/midi.rs` / `crates/model/src/apcmini.rs` | MIDI のポートの一覧・入出力とバイト列の解釈 / 受けた入力で `Core` を動かす接続の管理 / APC mini mk2 のパッド・ボタン・フェーダーの番号。AKAI の Communications Protocol v1.0 に基づく |
| `crates/io/src/audio.rs` / `crates/core/src/io/audio.rs` | 音声の入力とテンポの検出 / 検出の結果を `Core` に渡す橋渡し |
| `crates/io/src/link.rs` / `crates/io/src/carabiner.rs` | Ableton Link / Link の窓口の Carabiner への接続とダウンロード |

### backend

| ファイル | 中身 |
|---|---|
| `crates/backend/src/host.rs` | 画面によらずに動作する部分の `Host`。Core・出力ループ・MIDI/OSC/Link/NDI/音声/操作パネル・自動操作の窓口・保存を持つ。画面は `Host` を1つ持ち、headless と serve は `Host` だけで動作する |
| `crates/backend/src/project_patch.rs` | 画面の1フレームで変わったプロジェクトの差分 `Change` を作って当てる。クライアントの編集をサーバーへ送信するのに使う |
| `crates/backend/src/remote.rs` | `--remote-listen` のサーバーと `dmxctl connect` のクライアントの通信。`remote/` の `state.rs`・`server.rs`・`client.rs` に分けてある。食い違いを防ぐ仕組みは先頭のコメントにある |
| `crates/backend/src/cli.rs` / `crates/backend/src/control.rs` / `crates/backend/src/commands.rs` / `crates/backend/src/editing.rs` / `crates/backend/src/recording.rs` / `crates/backend/src/diagnostics.rs` | 自動操作・検証用の [CLI](cli.md)。順に、サブコマンド / 制御用の接続 / 操作 / 設定の編集と検証 / 記録のファイル書き込み / `explain` などの診断 |
| `crates/backend/src/controls.rs` | 制御用の接続やパネルから、割り当てを押したり離したりする操作 |
| `crates/backend/src/web.rs` / `crates/backend/src/web/panel.html` | ブラウザの操作パネル。std の TCP で最小限の HTTP を話す。`GET /` でページ、`GET /api/state` で様子、`POST /api/cmd` で操作。合言葉は `X-Pin` ヘッダー |

### ui

| ファイル | 中身 |
|---|---|
| `src/main.rs` / `crates/ui/src/app.rs` | 起動 / 画面。起動では最初の引数で動作方法を選択し、eframe か KMS/DRM かを決定する。`App::new`・`App::frame`・`App::shutdown` は eframe に依存しないので、どちらの土台からも呼べる |
| `crates/ui/src/map.rs` | メイン画面の MIDI タブ。`crates/ui/src/map/` に、図の描画 `pad.rs` と右のパネル `stage.rs` / `settings.rs` がある |
| `crates/ui/src/remote.rs` | 起動後にサーバーの待ち受けを始める欄と、「サーバーに接続…」の入力欄。接続は、確認してから `entry::spawn_self` で `connect` として起動し直す。待ち受けの設定は保存しない |
| `crates/ui/src/keys.rs` | キーの絵を出すキーボードタブ |
| `crates/ui/src/laser.rs` / `crates/ui/src/auto.rs` | レーザータブ / オートタブ。オートタブは、フレーズごとの見た目のプリセット `AutoPreset` / `PhraseLook` の表 |
| `crates/ui/src/` の `binding.rs`・`control.rs`・`midi.rs`・`output.rs`・`pair.rs`・`patch.rs`・`profile.rs`・`sequence.rs`・`show.rs` | 残りのタブ。ファイル名がタブに対応する |
| `crates/ui/src/theme.rs` / `crates/ui/src/shot.rs` | 色・フォントなどの見た目 / 画面を撮影して保存する |
| `crates/ui/src/kmsdrm.rs` / `crates/ui/src/kmsdrm/` | Linux だけ。デスクトップ環境の無い画面に、SDL2 の KMSDRM と OpenGL ES の egui_glow で描画するループ / キーの変換・タッチ |

### そのほか

| ファイル | 中身 |
|---|---|
| `Containerfile` / `.dockerignore` / `deploy/container/fetch-ndi.sh` / `deploy/kubernetes/` | コンテナのイメージと Kubernetes のマニフェスト。`fetch-ndi.sh` はイメージの中で `dmxctl-fetch-ndi` になり、NDI のライブラリを利用者が公式サイトから取得する。`deploy/kubernetes/components/ndi/` はそれを initContainer で実行する追加分。`dmxctl-server` をビルドし、画面を使わず `dmxctl serve` だけを動作させる。手順は [コンテナで動かす](container.md) |

単体テストは層ごとに `crates/core/src/tests/`・`crates/backend/src/tests/`・`crates/ui/src/tests/` に置きます。複数の層にまたがる検証は、ルートの `tests/` に置きます。ルートのクレートは、起動の `src/main.rs` と、結合テストが使う再公開だけを持ちます。他のクレートのテストが使う補助は、`dmxctl-core` の機能 `test-support` で公開します。

### 設計のポイント

| もの | 内容 |
|---|---|
| 機材定義 | 「機材定義」タブで、モード・チャンネル・値の範囲・16bit・安全値・カテゴリを作成・編集する。実体は設定フォルダの `fixtures/*.json` で、手書きもできる。内蔵の定義はリポジトリの `fixtures/` |
| 対象の指定 | 機材単体 / グループ / 同一機種すべて / すべて。`dimmer` や `color` などの機能名で指定するため、機種が異なっても同一の割り当てが効く |
| パッドの動作 | 連続のフェーダー / 押下中 / トグル / ワンショット |
| 割り当ての入力 | MIDI のノートと CC はページ別にできる。他にパソコンのキーボード、OSC アドレス |
| エフェクト | 対象チャンネルを 最小〜最大 で周期的に動かす。波形はサイン/三角/ノコギリ/矩形/ランダムで、位相と機材ごとのずれを付与できる |
| 出力 | 専用スレッドで40Hz 固定。画面の更新頻度とは関係しない |

## 性能の目安

14論理コアの PC で、機材8台・エフェクト・チェイス・ペアを動作させながら Art-Net を40Hz で送信する、負荷をかけた構成で測定しました。

| 項目 | 結果 |
|---|---|
| 1フレームの合成。機材16台・割り当て64件・エフェクト4件・チェイス2件・ペア3件 | 約12µs。40Hz の周期25,000µs の0.05% |
| 80個のパッド LED の計算 / MIDI 1メッセージ | 約30µs / 約0.4µs |
| DMX 出力の間隔 | 平均25.0ms で40回/秒。p99 は26ms 前後 |
| **CPU を全コア使い切っているとき** | 出力は40回/秒のまま。間隔の最悪は+10ms 程度 |
| メモリ | 起動後 約90MB の Private で一定。100秒以上動作させても増えない |
| 1コア換算の CPU | コントロール/マップ表示 約2〜3% / **前面が他のアプリのとき 約1.4%** / 最小化 0% / モニター表示 約11% |
| LEDバー 32ピクセル×2本 + ピクセルエフェクト2つを追加した構成 | 1フレームの合成 約36µs。周期の0.14% |
| Link・OSC を有効にしたとき | コントロール表示 約1〜2% / Link の通信で最小化 約1% / **OSC を60回/秒受信し続けている間 約5%**。出力は40回/秒・p99 26ms のまま |

- DMX の出力は画面とは別のスレッドで動作するため、画面の表示状態や、画面が重いことは出力の間隔に影響しません
- 描画は操作時・MIDI 入力時・拍の頭に限定し、1回あたり約10〜15msのCPU負荷に抑えています。モニター表示中は10fps、ウィンドウが前面でないときは通常1秒に1回、モニターは4fps です
- 映像などの重いアプリと同居するときは、このウィンドウを最小化するか、他のアプリを前に出しておくと、CPU 使用はほぼゼロになります

## はまりどころ

- **USB-DMX が開けない、Resource busy と表示される**: QLC+ など別の照明ソフトが利用していないか確認する
- **NDI のライブラリは同梱しない**: 配布条件のためです。インストール済みの NDI Tools、Linux では NDI SDK のものを読み込みます。探す順は `DMXCTL_NDI_LIB` → macOS の NDI Tools のアプリの中 → `NDI_RUNTIME_DIR_V6`/`V5` → 各OSの標準の場所 です。後ろの2つは moip-engine の `ndi` クレートが探します
- **FTDI の D2XX ライブラリは同梱しない**: 配布条件のためです。利用者が FTDI の配布元から導入したものを、実行時に読み込みます。探す順は `DMXCTL_D2XX_LIB` → 設定フォルダに置いたもの → 各OSの標準の場所 です。Windows は FTDI ドライバの `ftd2xx.dll` を利用します
- **レーザーの安全処理**: 場所は `crates/core/src/engine/guard.rs` の `guard_values` と、`crates/core/src/engine/safety.rs` の `emergency_stop` です。機材定義の `forbid` で禁止したオートショー・サウンド・パン/チルトの自動の動きと、機材ごとの `limits` で決めた動いてよい範囲は、出力の直前に必ず補正します。内蔵の定義を上書きしても `forbid` は引き継ぎます。異常終了時・終了シグナル・シャットダウン・Mac/Linux で画面を通らずに `exit()` したときは、レーザーOFF・明るさ0の安全なフレームを約0.3秒送信してから終了します。Windows のシャットダウンは、見えないウィンドウで `WM_ENDSESSION` を受信して検知します。`watch_windows_shutdown` です。`exit()` のときは `atexit` を使います
- **KMS/DRM**: 「画面を開けませんでした」と表示されて画面が出ないときは、`video` グループに所属しているか、デスクトップ環境など他のアプリが画面を使っていないかを確認する。キー・マウスが効かないときは `input` グループ
- **ムービングのチルト**: 128が真上です。動きのプリセットはチルトを大きく振る作りです
- 実行中のアプリは5秒ごとに設定を自動保存するため、`project.json` を直接書き換えるときは先にアプリを閉じる

## ライセンスについて

Ableton Link は GPLv2 以降のライセンスのため、このアプリには含めていません。Link を利用する部分は、別のプログラムの [Carabiner](https://github.com/Deep-Symmetry/carabiner) に TCP で接続する構成です。Carabiner は同梱せず、利用者がボタンか `dmxctl carabiner install` を実行したときだけ、配布元から取得します。そのため、このアプリを配布するときに、Link のソースを公開する必要はありません。

Carabiner を一緒に配布したいときは、Carabiner と Link のソースを公開し、GPL に従ってください。

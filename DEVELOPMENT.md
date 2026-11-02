# 開発環境メモ(別のPCで作り直すとき)

## コピーするもの

| もの | 場所 | 備考 |
|---|---|---|
| ソース一式(履歴つき) | `dmxctl` フォルダ(`.git` を含む) | `target/`(ビルドの中間ファイル、2GB以上)と `dist/`(できあがったアプリ)はコピー不要。作り直せる |
| 設定(パッチ・機材定義・割り当て) | Mac: `~/Library/Application Support/dmxctl/`<br>Windows: `%APPDATA%\dmxctl\` | `project.json` と `fixtures/` が本体。`backups/` は任意 |

新しいPCでは、設定フォルダの中身を上の場所にそのまま置く(Mac⇄Windowsで中身は同じ形式)。

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

ビルド:
```
bash ./scripts/build-mac.sh            # dist/mac/DMX Controller.app(ユニバーサル版)
bash ./scripts/build-windows-mingw.sh  # dist/windows/dmxctl.exe(MacからWindows版)
cargo run --release                    # 開発中の確認
```

### テスト
```
cargo test --release
```

## 新しいPCで最初にやること

- 「出力」タブ: USB-DMXのポート名を選び直す(Mac `/dev/cu.usbserial-…` / Windows `COM3` など)
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
| `src/theme.rs` | 見た目(色・フォント) |
| `src/opendmx.rs` / `src/output.rs` | USB-DMX / Art-Net の出力 |

## 開発用の環境変数・コマンド

| もの | 使い方 |
|---|---|
| `DMXCTL_CONFIG_DIR=フォルダ` | 本番の設定に触れずに、別の設定で起動 |
| `DMXCTL_NO_MIDI=1` | MIDI機器に自動接続しない(本番と2つ起動するとき) |
| `DMXCTL_SHOT=保存先.png` + `DMXCTL_TAB=タブ名` / `DMXCTL_PAGE=2` / `DMXCTL_SIZE=1280x800` / `DMXCTL_UNIVERSE=2` | 画面を撮って保存して終了(見た目の確認用) |
| `DMXCTL_PERF=1` | 描画の時間を表示 |
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
- **NDIのライブラリは同梱しない**(配布条件のため)。インストール済みの NDI Tools のものを読み込む
- **D2XX**: Mac版は `third_party/ftd2xx/mac/libftd2xx.dylib` をアプリに同梱。Windowsは FTDIドライバの `ftd2xx.dll`
- **ムービングのチルト**: 128が真上。動きのプリセットはチルトを大きく振る作り
- 実行中のアプリは5秒ごとに設定を自動保存するので、`project.json` を直接書き換えるときは先にアプリを閉じる

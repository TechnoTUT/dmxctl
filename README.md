# dmxctl

MIDIパッドやパソコンのキーボードで、DMX照明機材を操作するコントローラーです。Rust と egui で開発しています。Windows・Mac・Linux に対応し、1つの実行ファイルで動作します。

## 主な機能

照明オペレーターを配置できない現場や、照明専用のPCを用意できない現場で、少ない手間で照明を操作できるように開発しています。

- **映像ソフトと併用できる**: TouchDesigner のような高負荷の映像ソフトと同じPCで動作できるよう、常駐中のCPU・メモリ使用量を抑えています。DMXの出力は画面と別のスレッドで処理し、40Hzを維持します
- **パッド1つで演出**: AKAI APC mini mk2 は接続すると自動で認識されます。MIDIコントローラーが無くても、**パソコンのキーボードだけ** で操作できます。色・動き・LEDバーの模様・ストロボなどを、機材の種類によらず同じ操作で出力できます。他のMIDI機器や OSC も割り当て可能です
- **無人でも動作**: DJ機器の CDJ や XDJ から Beat Link Trigger 経由でテンポ・波形の色と大きさ・曲の構成を受信し、自動で照明を制御します。この機能をオートと呼んでいます。モニターの無い Linux に設置して電源を入れるだけで常駐し、曲の停止時は待機の色に切り替わり、人が操作すると手動を優先します
- **曲や映像と拍の同期**: TAP・Ableton Link・MIDIクロック・音声からの自動検出に対応します。エフェクトやチェイスは拍に合わせて動作します
- **レーザーの安全設計**: 起動のたびに SAFE 状態になり、ARM したうえで合図(押下中・常時出力・オートのフレーズ)がある間のみ出力します。禁止した値や動いてよい範囲は、どの操作から来た値でも出力の直前に必ず補正します。終了時や異常終了時は、安全な値を送信してから停止します
- **出力**: FT232系の USB-DMX と Art-Net に対応し、複数の出力を同時に利用できます
- **遠隔操作**: スマホのブラウザから操作パネルを開けます。別のPCの画面とMIDIコントローラーから、照明を出力しているPCを操作することもできます

## Quick start

1. **[Releases](https://github.com/TechnoTUT/dmxctl/releases/latest)** から、利用するPCに合ったファイルをダウンロードして起動します。ビルドは不要です

   | PC | ファイル | 起動方法 |
   |---|---|---|
   | Windows 10 / 11 | `dmxctl-windows-x86_64.exe` | ダブルクリック |
   | Mac | `DMX-Controller-mac.zip` | 展開して `DMX Controller.app` を開く。初回は右クリック →「開く」 |
   | Linux | `dmxctl-linux-x86_64` / `dmxctl-linux-arm64` | `chmod +x` で実行権限を付与して起動 |

2. **出力** タブで、USB-DMX か Art-Net の出力先を追加します
3. **パッチ** タブで、機材のアドレスを実機の設定と一致させます
4. **MIDI** タブの「配置」メニューから **おすすめ配置** を選ぶと、APC mini mk2 とキーボードの割り当てが、現在の機材に合わせて一括で作成されます
5. パッドかキーボードで操作します。キーの割り当ては **キーボード** タブに表示されます

設定は自動で保存されます。ムービング・レーザーの準備や、OS別の注意は [導入と最初の準備](docs/getting-started.md) を確認してください。

## ドキュメント

| | |
|---|---|
| [導入と最初の準備](docs/getting-started.md) | ダウンロード、初期設定、バックアップ、別のPCへの移行 |
| [本番での操作](docs/operation.md) | APC mini mk2・キーボードの配置、ムービングの動き、レーザー |
| [機能の詳しい説明](docs/features.md) | マップ、パッドのLED・ページ、機材定義、BPM・チェイス、色、LEDバーなど |
| [映像ソフト・DJ機器との連携](docs/integration.md) | Ableton Link、MIDIクロック、OSC、Beat Link Trigger |
| [オートと常駐](docs/auto.md) | 曲の情報による自動制御、モニターの無い Linux での常駐 |
| [スマホ・別のPCから操作する](docs/remote.md) | 操作パネル、サーバーとクライアント |
| [Mac・Linux で使う](docs/platforms.md) | OS別の注意、NDI、デスクトップ環境の無い Linux、USB-DMX |
| [CLI](docs/cli.md) | 起動中のアプリを操作し、DMXの値を検証する。AI や自動テスト向け |
| [MCP](docs/mcp.md) | Claude Code などの AI のクライアントから、起動中のアプリを操作する |
| [開発ガイド](docs/development.md) | ソースからのビルド、テスト、コードの構成 |
| [今後の予定](docs/roadmap.md) | 機能の候補、3Dプレビュー、Model Context Protocol (MCP) |

## 開発

```
git clone --recursive https://github.com/TechnoTUT/dmxctl
cd dmxctl
cargo run --release
```

ビルドに必要なものは [開発ガイド](docs/development.md) に、開発のルールは [AGENTS.md](AGENTS.md) にあります。

## ライセンスについて

Ableton Link は GPLv2 以降のライセンスのため、このアプリには含めていません。Link を利用するときは、別のプログラムの [Carabiner](https://github.com/Deep-Symmetry/carabiner) に TCP で接続します。詳しくは [開発ガイド](docs/development.md#ライセンスについて) にあります。

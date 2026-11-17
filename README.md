# dmxctl

MIDIパッドやパソコンのキーボードで、DMXの照明機材を操作するコントローラーです。Rust と egui で作っています。Windows・Mac・Linux で、1つの実行ファイルで動きます。

## 何ができるか

照明のオペレーターがいない現場や、照明だけにPCを1台割けない現場で、少ない手間で照明を動かすために作っています。

- **映像ソフトの横で動く**: TouchDesigner のような重い映像ソフトと同じPCで動かせるよう、常駐中のCPU・メモリを抑えています。DMXの出力は画面と別のスレッドで動き、40Hzを保ちます
- **パッド1つで演出**: AKAI APC mini mk2 は挿せば自動でつながります。MIDIコントローラーが無くても、**パソコンのキーボードだけ** で操作できます。色・動き・LEDバーの模様・ストロボなどを、機材の種類によらず同じ操作で出せます。ほかのMIDI機器や OSC も割り当てられます
- **人がいなくても動く**: DJ機器の CDJ や XDJ から Beat Link Trigger 経由でテンポ・波形の色と大きさ・曲の構成を受け取り、自動で照明を動かします。これをオートと呼んでいます。モニターの無い Linux に置いて電源を入れるだけで常駐し、曲が止まれば待機の色になり、人が操作すれば手動を優先します
- **曲や映像と拍を揃える**: TAP・Ableton Link・MIDIクロック・音声からの自動検出に対応します。エフェクトやチェイスは拍に合わせて動きます
- **レーザーを安全に**: 起動のたびに SAFE になり、ARM したときだけ出力します。禁止した値や動いてよい範囲は、どの操作から来た値でも出力の直前に必ず直します。落ちたときや終了するときは、安全な値を送ってから止まります
- **出力**: FT232系の USB-DMX と Art-Net に対応し、複数の出力を同時に使えます
- **離れて操作**: スマホのブラウザから操作パネルを開けます。別のPCの画面とMIDIコントローラーから、照明を出しているPCを操作することもできます

## Quick start

1. **[Releases](https://github.com/TechnoTUT/dmxctl/releases/latest)** から、使うPCに合ったファイルを落として起動します。ビルドは要りません

   | PC | ファイル | 起動のしかた |
   |---|---|---|
   | Windows 10 / 11 | `dmxctl-windows-x86_64.exe` | ダブルクリック |
   | Mac | `DMX-Controller-mac.zip` | 展開して `DMX Controller.app` を開く。初めて開くときは右クリック →「開く」 |
   | Linux | `dmxctl-linux-x86_64` / `dmxctl-linux-arm64` | `chmod +x` して起動 |

2. **出力** タブで、USB-DMX か Art-Net の出力先を追加します
3. **パッチ** タブで、機材のアドレスを実機の設定と合わせます
4. **MIDI** タブの「配置」メニューから **おすすめ配置** を選ぶと、APC mini mk2 とキーボードの割り当てが、今の機材に合わせてまとめて作られます
5. パッドかキーボードで操作します。キーの割り当ては **キーボード** タブに出ます

設定は自動で保存されます。ムービング・レーザーの準備や、OSごとの注意は [導入と最初の準備](docs/getting-started.md) を見てください。

## ドキュメント

| | |
|---|---|
| [導入と最初の準備](docs/getting-started.md) | ダウンロード、最初の設定、バックアップ、別のPCへの移し方 |
| [本番での操作](docs/operation.md) | APC mini mk2・キーボードの配置、ムービングの動き、レーザー |
| [機能の詳しい説明](docs/features.md) | マップ、パッドのLED・ページ、機材定義、BPM・チェイス、色、LEDバーなど |
| [映像ソフト・DJ機器との連携](docs/integration.md) | Ableton Link、MIDIクロック、OSC、Beat Link Trigger |
| [オートと常駐](docs/auto.md) | 曲の情報で自動で動かす、モニターの無い Linux で常駐させる |
| [スマホ・別のPCから操作する](docs/remote.md) | 操作パネル、サーバーとクライアント |
| [Mac・Linux で使う](docs/platforms.md) | OSごとの注意、NDI、デスクトップ環境の無い Linux、USB-DMX |
| [CLI](docs/cli.md) | 起動中のアプリを操作し、DMXの値を検証する。AI や自動テスト向け |
| [開発ガイド](docs/development.md) | ソースからのビルド、テスト、コードの地図 |
| [今後の予定](docs/roadmap.md) | 機能の候補、3Dプレビュー、Model Context Protocol (MCP) |

## 開発

```
git clone --recursive https://github.com/TechnoTUT/dmxctl
cd dmxctl
cargo run --release
```

ビルドに要るものは [開発ガイド](docs/development.md) に、開発のルールは [AGENTS.md](AGENTS.md) にあります。

## ライセンスについて

GPLv2 以降で配布されている Ableton Link を組み込んでいるため、このアプリを他の人に配る場合は、GPL に従ってソースコードを一緒に公開する必要があります。Link のソースとライセンス文書は `third_party/link/` にあります。

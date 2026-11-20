# AGENTS.md

dmxctl を開発する人と AI エージェント向けのルールです。ビルド手順とコードの地図は [docs/development.md](docs/development.md) にあります。

## 確かめ方

- コミットの前に `cargo test --release` を通す。ドキュメント (docs/*) は対象外。
- 他のOS向けのコードを変えたときは、`DMXCTL_CHECK_ONLY=1 cargo check --target …` で型チェックする。この場合 C++ はビルドしない
- ブラウザの操作パネルは `web/` の React で、画面のビルドに Node.js 20.19 以上、または 22.12 以上が必要になる。無い環境でも Rust 側のビルドとテストは通り、操作パネルは未ビルドのページになる。`web/` を変えたら `cd web && npm run typecheck` を通す。見た目は `npm run dev` で確かめる。手順は [docs/development.md](docs/development.md)
- 画面の見た目は `DMXCTL_SHOT=保存先.png DMXCTL_TAB=タブ名` で撮って確かめる
- 動作は CLI の `dmxctl serve` / `ctl` / `simulate` で最終DMX値を検証できる。使い方は [docs/cli.md](docs/cli.md)
- 本番の設定や機器に触れない。`DMXCTL_CONFIG_DIR` で別の設定を使い、本番と同じPCで試すときは `DMXCTL_NO_MIDI=1` を付ける。`serve` は既定で実機送信・自動保存・入力を切っている
- 実行中のアプリは5秒ごとに自動保存するので、`project.json` を直接書き換えるときは先にアプリを閉じる

## 安全

照明、特にレーザーは人に向くと危ないので、次は崩さない。

- 機材定義の `forbid` で禁止した値と、機材ごとの `limits` で決めた動いてよい範囲は、どの操作から来た値でも出力の直前に必ず直す。場所は `crates/core/src/engine/guard.rs` の `guard_values`。これを通らない出力の経路を作らない
- ARM は起動のたびに解除し、保存しない。CLI・入力の再現・操作パネル・MCP からは ARM にできない。ARM・レーザーの合図の割り当ても、CLI・MCP からは作成・変更・削除できない。場所は `crates/backend/src/editing/validate.rs` の `safety_unchanged`
- 落ちたとき・終了のシグナル・シャットダウンでは、レーザーOFF・明るさ0の安全なフレームを送ってから止まる。場所は `crates/core/src/engine/safety.rs` の `emergency_stop`
- 通常の操作から安全設定をバイパスできるようにしない。安全設定そのものを変える機能は、演出の操作と分けて扱う

## 構成のルール

- ソースは5つのクレートに分ける。`crates/model/` は設定の形・機材定義・色や動きの表、`crates/io/` は機器とプロトコルの、エンジンの状態に触れない部分、`crates/core/` は値の合成と、機器をエンジンにつなぐ橋渡し、`crates/backend/` は画面なしの起動・保存・遠隔操作・CLI、`crates/ui/` は画面である。参照してよい向きは `ui` → `backend` → `core` → `io` → `model` だけで、逆向きの参照はコンパイルが通らず、`tests/layers.rs` が余計なクレートの依存を検出する
- 新しい処理は、画面に依存しないなら `core` か `backend` に配置する。`core` の型は、上の層の型に依存させない。上の層の処理を呼びたいときは、`core` にトレイトを定義して上の層で実装する。例は `core::live::LiveForward`
- 画面によらずに動く部分は `crates/backend/src/host.rs` の `Host` に置く。headless と serve は `Host` だけで動くので、画面の `App` にしか無い処理に頼らない
- 画面や MIDI から実行中の状態を変えるときは、`Core` のメソッドや `rt` を直接触らず、`LiveOp` にして `Core::live` に渡す。クライアントの `dmxctl connect` では、ここでサーバーへ送るため。詳しくは `crates/core/src/live.rs`
- 機材・割り当て・シーンの追加などプロジェクトの編集は、`editing` の操作で行う。検証してからまとめて反映する
- 40Hz の DMX 出力スレッドを待たせない。画面・ログの書き込み・ネットワークの待ちを出力の経路に入れない
- 操作パネルのビルド済みのファイルはコミットしない。`crates/backend/build.rs` がビルド時に `web/` から生成する。API の形を変えるときは `crates/backend/src/web.rs` と `web/src/api.ts` を同じ変更で直す
- 画面の描画は、操作したとき・入力があったとき・拍の頭などに限る。映像ソフトと同じPCで動かすため、CPUを抑える

## 互換性

- 古い版で保存した `project.json` と機材定義は、そのまま読めるようにする。新しい項目には `#[serde(default)]` を付ける
- 新しい版で保存したファイルを古い版で開いたときは、読めない項目だけを飛ばして起動し、元のファイルを `project.broken-…json` に残す。この仕組みを壊さない
- `--headless` など、起動オプションの以前の書き方も受け付け続ける

## 書き方

- 画面の文字・ドキュメント・コードのコメントは日本語で、熟語・漢語を優先して書く。例: 使う→利用する、要らない→不要、合わせる→一致させる、作る→作成する
- ネットワーク用語は、漢語に言い換えず、英語かカタカナで書く。言い換えると分かりにくくなるため。例: IPアドレス、ポート、ファイアウォール、サーバー、クライアント
- 括弧書きの補足はできるだけ使わない。補足は文を分けるか、文の中に入れる。括弧を使ってよいのは「All-Photonics Network (APN)」のような略称の併記だけ。見出しにも括弧を入れない
- ビルドスクリプト・CI の表示と、CLI が返す JSON・ヘルプは英語
- 使い方が変わったら、同じ変更で `docs/` も直す。README.md は概要と Quick start だけにして、詳しいことは `docs/` に書く
- 環境変数・起動オプション・ファイルの役割を足したり変えたりしたら、[docs/development.md](docs/development.md) の表とコードの地図も直す

## コミットと PR

- コミットのタイトルは日本語の1文で、何ができるようになるか・何を直すかを書く。例: 「クライアントから別のファイルを開けるようにする」。1つのコミットには1つの変更
- PR の説明は、何が良くなるかを先に書く
- NDI と FTDI の D2XX のライブラリは配布条件のため同梱しない。利用者が入れたものを実行時に読み込む
- Ableton Link は GPLv2 以降なので、配布物にはソースの公開が要る

# AIから操作・検証するCLI

`dmxctl-cli` は、起動中のGUI・画面なしの常駐本体への操作と、独立したシミュレーションに対応します。常駐本体への接続が `ctl`、独立した検証が `simulate` / `run` です。常駐接続は第1段階で、全タブの作成・編集操作は今後追加します。

## 常駐本体へ接続する

画面なしで起動する場合は、1つ目の端末で次を実行します。停止を指示するまで動き続けます。

```powershell
cargo build --bins
./target/debug/dmxctl-cli serve --project examples/cli/project.json --inputs none
```

GUIを操作対象にする場合は、代わりに次で起動します。既にこのGUIを起動している場合、別の `serve` を追加で起動する必要はありません。

```powershell
./target/debug/dmxctl --control --project examples/cli/project.json --output none --inputs none --no-autosave
```

2つ目の端末から、同じ本体へ接続します。

```powershell
./target/debug/dmxctl-cli ctl status
./target/debug/dmxctl-cli ctl fixture list
./target/debug/dmxctl-cli ctl fixture set 1 --attr pan --value 80
./target/debug/dmxctl-cli ctl command --json '{"cmd":"motion","params":{"shape":"Circle","beats":4,"size":0.3}}'
./target/debug/dmxctl-cli ctl output snapshot --fixture 1 --attrs pan,tilt
./target/debug/dmxctl-cli ctl watch --fixture 1 --attrs pan,tilt --duration 5
./target/debug/dmxctl-cli ctl project save
./target/debug/dmxctl-cli ctl stop
```

- `serve` は既定で実機送信なし、自動保存なしです。MIDIなどの入力は保存設定に従い起動するため、デバッグ例では `--inputs none` を指定しています。実機送信は `--output live`、自動保存は `--autosave` で有効にできます。
- GUIの通常起動は従来どおりの出力・保存動作です。デバッグ時は上の例のように起動時オプションを指定します。
- `--output none` は保存済みの出力先を変更せず、送信だけを無効にします。GUIで出力先を追加・有効化しても、そのプロセスからは送信しません。40Hzの合成は続くため、最終DMX値を確認できます。
- `--no-autosave` の実行では、変更を残すときに `ctl project save` が必要です。終了時にも自動保存しません。保存する値は元のプロジェクト形式で、実行時に無効化した出力先や入力機器の設定も保持します。
- 接続先は既定で `127.0.0.1:9190` です。複数起動する場合は、本体の `--listen 127.0.0.1:9191` と、クライアントの `ctl --connect 127.0.0.1:9191 ...` を対応させます。制御用の接続先はループバックIPに限定しています。
- コマンドは本体の `App` で処理します。GUIのフレーム処理と画面なしの常駐ループが同じ要求を受け取り、同じ `Core` を変更します。GUIとCLIが別々の設定コピーを操作する構成ではありません。

`ctl command --json` では、下記の「操作できる値」にある既存の操作を利用できます。ARMや安全設定の変更は引き続き受け付けません。フラッシュは押下の有効期限を設け、接続が切れたりGUIの更新が止まった場合も、出力スレッドが解除します。

```powershell
./target/debug/dmxctl-cli ctl command --json '{"cmd":"flash","on":true,"hold_ms":500}'
```

`hold_ms` の既定は500ms、範囲は1〜5000msです。操作の応答にある `accepted_at_frame` は受付時点、`takes_effect_from_frame` はその変更を反映できる最初のエンジンフレームです。応答は実際の反映・送信を待つものではありません。適用後の値は `ctl output snapshot` や `ctl watch` で確認します。

### 起動中の記録

```powershell
New-Item -ItemType Directory -Force runs
./target/debug/dmxctl-cli ctl record start --file runs/live.jsonl
# GUIまたはCLIで操作する
./target/debug/dmxctl-cli ctl record stop
./target/debug/dmxctl-cli log summarize runs/live.jsonl
```

記録先の相対パスは、クライアントを実行したフォルダを基準に解決します。記録中に `ctl stop` で本体を終了した場合も、安全停止中のフレームを記録してからログを閉じます。記録しない間はログ出力用スレッドを作りません。

`ctl watch` は既定100ms間隔で最終値を取得する監視機能です。`--interval-ms` で25〜10000msを指定できます。40Hzすべてのフレームを残す用途では `ctl record start` を使ってください。ログの `frame` は記録内のフレーム番号、`engine_frame` はアプリ本体のフレーム番号です。

## 独立したシミュレーション

以下の `simulate` / `run` は、常駐本体へ接続せず独立して実行します。元の設定ファイルは保存・変更しません。

## 最初に試す

```powershell
cargo build --bin dmxctl-cli
New-Item -ItemType Directory -Force runs
./target/debug/dmxctl-cli inspect --project examples/cli/project.json
./target/debug/dmxctl-cli simulate --project examples/cli/project.json --scenario examples/cli/scenario.json --duration 3 --log runs/example.jsonl
./target/debug/dmxctl-cli log summarize runs/example.jsonl
./target/debug/dmxctl-cli log query runs/example.jsonl --fixture 2 --attrs pan,tilt --from-ms 900 --to-ms 1100
```

Linux/macOSでは `mkdir -p runs` で保存先を作ります。ログは既存ファイルを上書きしないため、再実行時は別のファイル名を指定します。

`--project` は必須です。機材定義は設定ファイルと同じフォルダの `fixtures/` と内蔵定義から読みます。別の場所を使う場合は `--fixtures DIR` を指定します。読み込めない項目を黙って飛ばさず、エラーにします。

## 実行方法

| コマンド | 動作 |
|---|---|
| `inspect` | 設定、機材ID、チャンネルの属性・禁止値・可動範囲をJSONで取得 |
| `simulate` | 仮想時間で40Hzの合成を実行。実際の時間を待たずに再現・比較 |
| `run` / `watch` | 実時間で40Hzの合成を実行。JSONLを実行中に読み取れる |
| `log summarize` | フレーム数、操作件数、安全補正件数、終了・欠落の有無を確認 |
| `log query` | 時間帯、機材・属性、ユニバース・アドレスで記録を抽出 |
| `diff` | 2つの記録をフレーム順に比較し、異なるチャンネルと値を取得 |

実行時間は `--duration SECONDS` で指定します。既定は10秒、上限は24時間で、25ms単位に切り上げます。シナリオは、実行範囲内の `at_ms` 順に並べた配列です。時刻が25ms単位でない操作は、その時刻以降の最初の処理で適用します。0msの操作は出力スレッドを開始する前に適用します。

各実行のテンポ入力は内部クロックです。保存設定で有効でも、MIDI・OSC・NDI・音声・Ableton Linkの入力は起動しません。`auto` は曲の入力がない状態で動作します。入力の録音・再生は未対応です。

実時間で操作する場合は `--stdin` を付け、1行ごとにJSONオブジェクトを渡します。

```powershell
'{"id":"pan-1","cmd":"set","fixture":1,"attr":"pan","value":80}' | ./target/debug/dmxctl-cli watch --project examples/cli/project.json --stdin --duration 3 --log runs/live.jsonl
```

AIはプロセスの標準入力を開いたまま操作を送り、標準出力のJSONLで結果を読み取れます。標準入力が閉じても、指定した実行時間までは継続します。標準入力の不正な操作は `command_error` として記録し、実行を継続します。シナリオの不正な操作は起動前に拒否します。

## 操作できる値

```json
{"id":"a","cmd":"set","fixture":1,"attr":"dimmer","value":180}
{"id":"b","cmd":"motion","params":{"shape":"Circle","beats":4,"size":0.3}}
{"id":"c","cmd":"effect","name":"既存のエフェクト名","params":{"enabled":true,"beats":2,"min":40,"max":180}}
{"id":"d","cmd":"blackout","on":true}
```

- `set`: 機材IDと属性名を指定。値は既存エンジンの0〜255スケールで、16bitの属性も同じスケールです。
- `motion`: `shape`, `beats`, `spread`, `position`, `size`, `points`, `glide` を変更できます。向き合わせの校正値は変更できません。形や揃え方の列挙値は `inspect` の設定と既存のJSON形式に従います。
- `effect`: 既存のエフェクトの `enabled`, `min`, `max`, `rate_hz`, `beats`, `phase`, `spread`, `wave` を変更できます。
- 既存の操作APIから、`blackout`, `master`, `fade`, `flash`, `bpm`, `auto`, `auto_resume`, `preset`, `look`, `color`, `swap`, `scene`, `tap`, `resync` も使えます。`on` は真偽値、数値には `value`、シーンには `name`、プリセットには0始まりの `index`、フレーズには `phrase` を渡します。色は `name`（例: `Red`）と `slot`（`main` / `sub`）を渡します。

ユニバースとDMXアドレスはログ上で1始まりです。`project.json` 内のユニバース番号とプリセット番号は既存形式の0始まりです。

## 安全制御と実機出力

既定では実機への送信を行いません。送信するときだけ `run --output` を付けます。設定ファイルに保存されている有効な出力先へ送るので、使用する出力先を設定した専用プロジェクトを指定してください。

CLIの操作からARM、禁止値、可動範囲、機材定義、パッチ、出力先は変更できません。ARMは起動時に解除され、CLIでは有効にできません。バイパス用のオプションもありません。実機出力は既存の出力スレッドと安全停止処理を使います。通常の実行終了時も約0.3秒間、安全なフレームを送ります。

合成後にブラックアウト・ミュート・ARM・可動範囲・禁止値を適用します。禁止値の置換先が可動範囲外になる場合は、両方の条件を満たす値を選びます。条件を同時に満たす値がない設定や、暗転・安全値と制限が矛盾する設定、アドレスが重複する有効な機材は、CLIでは起動前に拒否します。

これらは読み込んだ機材定義と制限に基づくソフトウェア上の制御です。記録するのはアプリの生成値と出力ステータスで、実機の受信・動作の確認ではありません。

## ログ形式と欠落

実行中の標準出力と `--log` の保存ファイルには、同じJSONLイベントを出します。開始時に設定・機材定義を記録しますが、WebパネルのPINは含めません。

| `type` | 内容 |
|---|---|
| `start` | 形式のバージョン、実行モード、設定、機材定義、開始日時 |
| `command` / `command_error` | 操作内容、ID、経過時間、反映対象の最初のフレーム番号 |
| `frame` | フレーム番号、経過ms、拍、DMX差分、安全補正、緊急停止中かどうか |
| `complete` | 実行時間を満了したことと、最終の出力ステータス |
| `end` | ログを閉じたことと、欠落イベント数 |

最初のフレームと40フレームごとに、対象ユニバースの512チャンネルすべてを記録します。それ以外は変化したチャンネルだけを記録します。機材のチャンネルには `fixture` と `attr` も付きます。`safety` は安全処理直前の値を `requested`、補正後を `applied` として記録します。エフェクトより前の操作値は `command` に残ります。補正件数はフレームごとの件数で、同じ補正が続けば毎フレーム数えます。

ログ書き込みは別スレッドで行います。実時間モードでは出力を待たせないため、256イベントのキューが満杯になると記録が欠落します。`seq` の飛びと最後の `dropped` で判別でき、次の全値フレームから状態を復元できます。仮想時間のシミュレーションでは書き込みを待ち、キューの満杯による欠落を防ぎます。標準出力の読み手が閉じた場合も、ファイルへの保存は継続します。

Ctrl+Cなどでプロセスが終了した場合、`complete` / `end` が残らないことがあります。ログは正常完了とみなさず、強制終了時は末尾のJSON行が途中になる可能性もあります。`diff` は正常終了し、欠落のない記録だけを受け付けます。比較の対応付けは記録フレーム順なので、同じ条件の仮想時間シミュレーションを比較する用途に適しています。

`log query` は指定した区間のイベントを抽出します。区間開始時の値を補う機能はなく、空の差分は「変化なし」を意味します。範囲内に `full: true` がなければ、それ以前の全値フレームと差分も参照してください。現時点の解析コマンドは記録ファイルをメモリに読み込むため、長時間の実行はファイルを分ける運用を想定しています。

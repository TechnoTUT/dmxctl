# CLI

`dmxctl` の `serve` / `ctl` / `inspect` / `simulate` などのサブコマンドは、起動中のGUI・画面なしの常駐本体を操作し、最終DMX値を検証するためのCLIです。AI や自動テストから使うことを想定しています。GUIと同じ実行ファイルで、Macでは `DMX Controller.app/Contents/MacOS/dmxctl` です。`ctl` で常駐本体に接続し、`simulate` / `run` で独立した検証を行います。

## 常駐本体へ接続する

画面なしで起動する場合は、1つ目の端末で次を実行します。停止を指示するまで動き続けます。

```powershell
cargo build
./target/debug/dmxctl serve --project examples/cli/project.json
```

GUIを操作対象にする場合は、代わりに次で起動します。既にこのGUIを起動している場合、別の `serve` を追加で起動する必要はありません。

```powershell
./target/debug/dmxctl --control --project examples/cli/project.json --output none --inputs none --no-autosave
```

2つ目の端末から、同じ本体へ接続します。

```powershell
./target/debug/dmxctl ctl status
./target/debug/dmxctl ctl fixture list
./target/debug/dmxctl ctl fixture set 1 --attr pan --value 80
./target/debug/dmxctl ctl command --json '{"cmd":"motion","params":{"shape":"Circle","beats":4,"size":0.3}}'
./target/debug/dmxctl ctl output snapshot --fixture 1 --attrs pan,tilt
./target/debug/dmxctl ctl watch --fixture 1 --attrs pan,tilt --duration 5
./target/debug/dmxctl ctl project save
./target/debug/dmxctl ctl stop
```

- `dmxctl carabiner install` は、Ableton Link に参加する Carabiner を配布元から取って、設定フォルダに入れます。成功すると、場所と版を JSON で返します。
- `serve` は既定で実機送信なし、自動保存なし、MIDI・OSC・Ableton Link・操作パネルの入力なしです。本番のGUIと同じPCで動かしても、コントローラーやポートを取り合いません。入力は `--inputs configured`、実機送信は `--output live`、自動保存は `--autosave` で有効にできます。
- GUIの通常起動は従来どおりの出力・保存動作です。デバッグ時は上の例のように起動時オプションを指定します。
- `--output none` は保存済みの出力先を変更せず、送信だけを無効にします。GUIで出力先を追加・有効化しても、そのプロセスからは送信しません。40Hzの合成は続くため、最終DMX値を確認できます。
- `--no-autosave` の実行では、変更を残すときに `ctl project save` が必要です。終了時にも自動保存しません。保存する値は元のプロジェクト形式で、実行時に無効化した出力先や入力機器の設定も保持します。
- 接続先は既定で `127.0.0.1:9190` です。複数起動する場合は、本体の `--listen 127.0.0.1:9191` と、クライアントの `ctl --connect 127.0.0.1:9191 ...` を対応させます。制御用の接続先はループバックIPに限定しています。
- コマンドは本体の `App` で処理します。GUIのフレーム処理と画面なしの常駐ループが同じ要求を受け取り、同じ `Core` を変更します。GUIとCLIが別々の設定コピーを操作する構成ではありません。

`ctl command --json` では、下記の「操作できる値」にある既存の操作を利用できます。ARMや安全設定の変更は引き続き受け付けません。フラッシュには押下の有効期限があり、接続が切れたりGUIの更新が止まったりしても、出力スレッドが自動で解除します。

```powershell
./target/debug/dmxctl ctl command --json '{"cmd":"flash","on":true,"hold_ms":500}'
```

`hold_ms` の既定は500ms、範囲は1〜5000msです。応答の `accepted_at_frame` は受付時点、`takes_effect_from_frame` は変更を反映できる最初のエンジンフレームです。`--wait` を付けると、そのフレーム以降の合成まで待ちます。実機の受信・動作を待つ機能ではありません。

### スキーマ・反映待ち・出力の検証

```powershell
./target/debug/dmxctl ctl schema motion
./target/debug/dmxctl ctl fixture set 1 --attr pan --value 80 --wait
./target/debug/dmxctl ctl assert --fixture 1 --attr pan --equals 80 --tolerance 0.1
./target/debug/dmxctl ctl wait --frames 2
./target/debug/dmxctl ctl wait --beats 4 --timeout-ms 10000
./target/debug/dmxctl ctl wait --fixture 1 --attr dimmer --min 100 --timeout-ms 5000
./target/debug/dmxctl ctl explain --fixture 1 --attr pan
```

`schema` はJSON Schema、現在値、読み取り専用項目、検証規則を返します。対象を省略するとプロジェクト全体を取得します。参照先やパッチの整合性は `edit --dry-run` で検証します。

`assert` は合成済みの出力を検証し、不一致ならJSONを出して終了コードを非0にします。`--equals`、`--min`、`--max` と、許容差 `--tolerance` を指定できます。16bit属性は粗値＋細値/256で比較し、同じ属性の複数チャンネルはすべて条件を満たす必要があります。

`wait` は指定したフレーム数・拍数・`--ms` で指定する時間・出力条件を待ちます。条件を併用すると、すべてを満たすまで待ちます。`--after-frame N` はN以降のフレームを対象にします。待機の既定上限は5000ms、`--timeout-ms` の範囲は1〜300000msです。緊急停止や本体の再起動を検出すると失敗します。

`explain` は次の実フレームについて、基準値、オート・フェード、エフェクト、動き、ピクセル、ペア、光量、フラッシュ、待機表示、安全処理、最終出力を返します。同時に取得できるのは1件で、未取得の結果は5秒後に新しい要求で置き換えられます。通常時はトレースを保持しません。

配列位置を使う編集などで競合を検出するには、`ctl version` で取得した `session` と `revision` を編集時の `--expected-session`、`--expected-revision` に渡します。バージョンは設定を観測した時点の変更を示し、GUIの編集やフェードによる保存値の変化も対象です。

### 起動中の記録

```powershell
New-Item -ItemType Directory -Force runs
./target/debug/dmxctl ctl record start --file runs/live.jsonl
# GUIまたはCLIで操作する
./target/debug/dmxctl ctl record stop
./target/debug/dmxctl log summarize runs/live.jsonl
```

記録先の相対パスはクライアントを実行したフォルダを基準に解決します。記録中に `ctl stop` で本体を終了した場合も、安全停止中のフレームを記録してからログを閉じます。記録しない間はログ出力用スレッドを作りません。

`ctl watch` は既定100ms間隔で最終値を取得する監視機能です。`--interval-ms` で25〜10000msを指定できます。40Hzすべてのフレームを残す用途では `ctl record start` を使ってください。ログの `frame` は記録内のフレーム番号、`engine_frame` はアプリ本体のフレーム番号です。

### ボタン・スライダーと入力の再現

```powershell
./target/debug/dmxctl ctl button list
./target/debug/dmxctl ctl button click binding:42
./target/debug/dmxctl ctl button hold binding:42 --hold-ms 500
./target/debug/dmxctl ctl button press binding:42 --hold-ms 1000
./target/debug/dmxctl ctl button release binding:42
./target/debug/dmxctl ctl button set binding:1 --value 0.5
./target/debug/dmxctl ctl input midi --kind Note --channel 9 --number 40 --value 127
./target/debug/dmxctl ctl input key --json '{"number":16,"pressed":true}'
./target/debug/dmxctl ctl input osc --json '{"address":"/dmx/bpm","args":[128]}'
```

IDは実行中の設定に合わせて `button list` から取得してください。割り当てに保存するIDなので、並べ替えや他の割り当ての削除で変わりません。古い設定は読み込み時にIDを付与し、保存すると次回も引き継ぎます。IDのない割り当てを新規作成すると新しいIDが付きます。

ボタンIDの直接操作はページに依存しません。MIDI・キー・OSCの再現では、現在のページや割り当てを含めてGUIと同じ入力処理を実行します。MIDIのチャンネルは0〜15、番号と値は0〜127です。キーの番号はASCIIではなく既存のキー配置コードで、16はQです。

`click` は既定50ms押してから解放、`press` / `hold` は既定1000ms、`--hold-ms` は1〜5000msです。`press` を繰り返すと期限を更新します。トグルやワンショットはGUI同様、解放しても設定した状態が残ります。スライダーの `set` は0〜1を指定し、値はそのまま維持します。ARMのボタンは一覧に `remote_allowed: false` と表示され、入力再現からも起動できません。入力再現はMIDI/OSCラーン中には受け付けません。

期限切れの解放は40Hzの出力スレッドで処理します。GUIの更新が止まった場合も期限を管理し、割り当てを削除・変更した場合でも押下時の動作を保持して解放します。

### 設定の作成・編集・削除

常駐中の設定を編集できます。`fixture`, `group`, `binding`, `scene`, `position`, `effect`, `pixel`, `chase`, `pair`, `output`, `auto_preset` が対象です。

```powershell
./target/debug/dmxctl ctl group create --json '{"name":"Spots","fixtures":[1]}'
./target/debug/dmxctl ctl effect create --json '{"name":"Pulse","sel":{"Group":"Spots"},"attr":"dimmer","beats":2}'
./target/debug/dmxctl ctl effect update Pulse --json '{"enabled":true,"beats":4}'
./target/debug/dmxctl ctl scene capture Intro --json '{"fade":1,"fixtures":[1]}'
./target/debug/dmxctl ctl scene update Intro --json '{"fade":2}'
./target/debug/dmxctl ctl settings update --json '{"bpm":128,"auto":{"enabled":false}}'
./target/debug/dmxctl ctl scene list
./target/debug/dmxctl ctl binding list
./target/debug/dmxctl ctl scene delete Intro
```

対象の指定は、機材は数値ID、割り当ては `binding:ID`、その他は名前です。出力先や、同じ名前でまとめたエフェクトなどは `--index N` で一覧の0始まりの位置を指定します。並びを変更した場合は、一覧を取り直してください。名前が複数一致する場合は変更を拒否します。`fixture list` は従来の機材一覧、`fixture get` は編集用の全フィールドを返します。

`--json` の代わりに `--file FILE.json` も使えます。編集データは `project.json` と同じ構造です。ユニバース番号、機材モード、ページ、プリセット番号、`--index` は0始まりです。DMXアドレスは1始まりです。`scene capture` はGUIの「現在の状態を保存」と同じく、エフェクト合成前の基準値を保存します。

複数の変更をまとめる場合は、編集ファイルを使います。

```powershell
./target/debug/dmxctl ctl edit --file examples/cli/edits.json --dry-run
./target/debug/dmxctl ctl edit --file examples/cli/edits.json
```

ファイルは次の形式の配列です。`create` の `data` には省略可能な既定値を補い、`update` は指定したフィールドだけを変更します。

```json
[
  {"entity":"scene","op":"update","key":"Intro","data":{"name":"Intro 2"}},
  {"entity":"chase","op":"update","key":"Chase","data":{"steps":["Intro 2"]}}
]
```

検証するのは変更全体を適用した後の状態です。アドレスを2台で入れ替える場合など、途中だけ重複する変更も一括で扱えます。参照切れ、アドレス重複、ペアの循環、不正な数値などがあれば、設定と実行状態は変更しません。名前の変更や削除では、その名前を参照する割り当てやチェイスなども同じバッチで修正してください。参照先を自動で削除する機能はありません。

`--dry-run` は検証だけを行い、機材・割り当てのIDも消費しません。ボタンを押したまま、またはMIDI/OSCラーン中には編集を受け付けません。先に解放・終了してください。

GUIとCLIは、機材・グループ追加、シーン保存・削除、割り当て削除、機材定義の保存・削除、プロジェクト読込、保存時の検証を共有します。GUIの数値・名前の直接編集は引き続きタブごとの処理を使います。

CLIで機材を削除するときは、その機材を指すグループ・シーン・割り当てなどを同じバッチで直す必要があります。GUIのパッチタブで機材を削除したときは、それらを自動で片付けます。グループ・ペア・シーン・位置プリセットからはその機材だけを外し、その機材だけを対象にしたエフェクトと割り当ては削除します。消したものは画面の下に表示します。

### 差分・再読込・復元

```powershell
./target/debug/dmxctl ctl project diff
./target/debug/dmxctl ctl project save
./target/debug/dmxctl ctl project reload --dry-run
./target/debug/dmxctl ctl project reload
./target/debug/dmxctl ctl project restore --file runs/backups/before-load-....json
```

`diff` は保存済みファイルと現在の設定との差分を返します。WebパネルのPINは表示・記録せず、変更の有無だけを `redacted_changes.web_pin` に返します。

再読込・復元では、ファイル全体を検証してから、現在の未保存状態をプロジェクトと同じフォルダの `backups/` へ保存し、実行中の設定を置き換えます。読込や検証に失敗した場合は元の状態を維持します。保存先は実行中のプロジェクトのままです。自動保存を無効にしている場合、復元した設定を残すには `project save` が必要です。ARMは解除します。

### 機材定義

```powershell
./target/debug/dmxctl ctl profile list
./target/debug/dmxctl ctl profile save --file fixture.json --dry-run
./target/debug/dmxctl ctl profile save --file fixture.json
./target/debug/dmxctl ctl profile delete user/example
```

機材定義は個別のJSONファイルに明示的に保存します。プロジェクトの編集バッチには含められません。使用中、または保存済みプロジェクトが参照している定義は、そのまま削除できません。使用中のチャンネル構造を変更するときは、先に機材をパッチから外してプロジェクトを保存してください。パッチを外す際に、参照するシーン・割り当てなども修正する必要があります。名前や範囲のラベルなど、チャンネル構造を変えない編集は使用中でも行えます。

安全制限のある機材はCLIから削除・再パッチ・有効状態の変更を行えません。ARM、可動範囲、禁止値・安全値の定義、ムービングの校正値も通常のCLI編集から変更できません。既存の安全設定をGUIで変更する操作とは区別しています。

## 独立したシミュレーション

`simulate` / `run` は常駐本体へ接続せず独立して実行します。元の設定ファイルは保存・変更しません。

### 最初に試す

```powershell
cargo build
New-Item -ItemType Directory -Force runs
./target/debug/dmxctl inspect --project examples/cli/project.json
./target/debug/dmxctl simulate --project examples/cli/project.json --scenario examples/cli/scenario.json --duration 3 --log runs/example.jsonl
./target/debug/dmxctl log summarize runs/example.jsonl
./target/debug/dmxctl log query runs/example.jsonl --fixture 2 --attrs pan,tilt --from-ms 900 --to-ms 1100
```

Linux/macOSでは `mkdir -p runs` で保存先を作ります。ログは既存ファイルを上書きしないため、再実行時は別のファイル名を指定します。

`--project` は必須です。機材定義は設定ファイルと同じフォルダの `fixtures/` と内蔵定義から読みます。別の場所を使う場合は `--fixtures DIR` を指定します。読み込めない項目があるとエラーにします。

### 実行方法

| コマンド | 動作 |
|---|---|
| `inspect` | 設定、機材ID、チャンネルの属性・禁止値・可動範囲をJSONで取得 |
| `simulate` | 仮想時間で40Hzの合成を実行。実際の時間を待たずに再現・比較 |
| `run` / `watch` | 実時間で40Hzの合成を実行。JSONLを実行中に読み取れる |
| `log summarize` | フレーム数、操作件数、安全補正件数、終了・欠落の有無を確認 |
| `log query` | 時間帯、機材・属性、ユニバース・アドレスで記録を抽出 |
| `diff` | 2つの記録をフレーム順に比較し、異なるチャンネルと値を取得 |

実行時間は `--duration SECONDS` で指定します。既定は10秒、上限は24時間で、25ms単位に切り上げます。シナリオは実行範囲内の `at_ms` 順に並べた配列です。時刻が25ms単位でない操作は、その時刻以降の最初の処理で適用します。0msの操作は出力スレッドを開始する前に適用します。

各実行のテンポ入力は内部クロックです。保存設定で有効でも、MIDI・OSC・NDI・音声・Ableton Linkの入力サービスは起動しません。`auto` は曲の入力がない状態で動作します。シナリオの `input` はMIDI・キー・OSCの操作を再現できますが、音声・映像・外部クロックは再現しません。

実時間で操作する場合は `--stdin` を付け、1行ごとにJSONオブジェクトを渡します。

```powershell
'{"id":"pan-1","cmd":"set","fixture":1,"attr":"pan","value":80}' | ./target/debug/dmxctl watch --project examples/cli/project.json --stdin --duration 3 --log runs/live.jsonl
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
- 既存の操作APIから、`blackout`, `master`, `fade`, `flash`, `bpm`, `auto`, `auto_resume`, `preset`, `look`, `color`, `swap`, `scene`, `tap`, `resync` も使えます。`on` は真偽値、数値には `value`、シーンには `name`、プリセットには0始まりの `index`、フレーズには `phrase` を渡します。色は `Red` のような `name` と、`main` か `sub` の `slot` を渡します。

ユニバースとDMXアドレスはログ上で1始まりです。`project.json` 内のユニバース番号とプリセット番号は既存形式の0始まりです。

## 安全制御と実機出力

既定では実機への送信を行いません。送信するときだけ `run --output` を付けます。設定ファイルに保存されている有効な出力先へ送るので、使用する出力先を設定した専用プロジェクトを指定してください。

ARMは起動時に解除され、CLIでは有効にできません。禁止値、可動範囲などの安全設定も通常のCLI操作から変更できません。パッチ・出力先・機材定義の編集には常駐接続の専用コマンドを使います。実機出力は既存の出力スレッドと安全停止処理を使い、通常の実行終了時も約0.3秒間、安全なフレームを送ります。

合成後にブラックアウト・ミュート・ARM・可動範囲・禁止値を適用します。禁止値の置換先が可動範囲外になる場合は、両方の条件を満たす値を選びます。条件を同時に満たす値がない設定や、暗転・安全値と制限が矛盾する設定、アドレスが重複する有効な機材は、CLIでは起動前に拒否します。

これらは読み込んだ機材定義と制限に基づくソフトウェア上の制御です。記録するのはアプリの生成値と出力ステータスで、実機の受信・動作の確認ではありません。

## ログ形式と欠落

実行中の標準出力と `--log` の保存ファイルには、同じJSONLイベントを出します。開始時に設定・機材定義を記録しますが、WebパネルのPINは含めません。

| `type` | 内容 |
|---|---|
| `start` | 形式のバージョン、実行モード、設定、機材定義、開始日時 |
| `command` / `command_error` | 操作内容、ID、経過時間、反映対象の最初のフレーム番号 |
| `state_change` | GUIフレーム中や設定読込で観測した設定・状態の変化。実行可能な操作とは区別 |
| `frame` | フレーム番号、経過ms、拍、DMX差分、安全補正、緊急停止中かどうか |
| `complete` | 実行時間を満了したことと、最終の出力ステータス |
| `end` | ログを閉じたことと、欠落イベント数 |

最初のフレームと40フレームごとに、対象ユニバースの512チャンネルすべてを記録します。それ以外は変化したチャンネルだけを記録します。機材のチャンネルには `fixture` と `attr` も付きます。`safety` は安全処理直前の値を `requested`、補正後を `applied` として記録します。エフェクトより前の操作値は `command` に残ります。補正件数はフレームごとの件数で、同じ補正が続けば毎フレーム数えます。

ログ書き込みは別スレッドで行います。実時間モードでは出力を待たせないため、256イベントのキューが満杯になると記録が欠落します。`seq` の飛びと最後の `dropped` で判別でき、次の全値フレームから状態を復元できます。仮想時間のシミュレーションでは書き込みを待ち、キューの満杯による欠落を防ぎます。標準出力の読み手が閉じた場合も、ファイルへの保存は継続します。

Ctrl+Cなどでプロセスが終了した場合、`complete` / `end` が残らないことがあります。ログは正常完了とみなさず、強制終了時は末尾のJSON行が途中になる可能性もあります。`diff` は正常終了し、欠落のない記録だけを受け付けます。比較の対応付けは記録フレーム順なので、同じ条件の仮想時間シミュレーションを比較する用途に適しています。

`log query` は指定した区間のイベントを抽出します。区間開始時の値を補う機能はなく、空の差分は「変化なし」を意味します。範囲内に `full: true` がなければ、それ以前の全値フレームと差分も参照してください。現時点の解析コマンドは記録ファイルをメモリに読み込むため、長時間の実行はファイルを分ける運用を想定しています。

### 操作履歴とシナリオ出力

```powershell
./target/debug/dmxctl log scenario runs/example.jsonl --out runs/repro
./target/debug/dmxctl simulate --project runs/repro/project.json --scenario runs/repro/scenario.json --duration 3 --log runs/replay.jsonl
```

記録中はMIDI・キー・OSCの入力も `command` に残します。GUIフレームの前後で観測した変化は `state_change` に残します。GUIフレーム中にはバックグラウンド処理も進むため、これは個々のボタン操作を特定する履歴ではありません。記録開始前の操作は含みません。

`log scenario` は開始時の設定・機材定義と操作列を、新しいフォルダへ出力します。実行時間の目安は `manifest.json` の `duration_seconds` に記録します。欠落、操作エラー、ラーン、`state_change` があるログは、再現できない変化を省略しないよう出力を拒否します。ARMなど、通常のCLIで拒否する操作も再現できません。

再実行は内部クロックの初期状態から始まります。実時間の記録にある拍の位相、進行中のフェード、入力サービスの内部状態は復元しません。出力値の一致を比較する用途には、同じ条件の仮想時間シミュレーションを使います。

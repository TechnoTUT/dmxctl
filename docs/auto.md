# オートと常駐

## オート

**オート** タブでオートを ON にすると、人が操作しなくても照明が動作します。曲の情報が届けばそれに合わせて動作し、届かなければ待機の色で待機します。

- **曲の情報が届いている間**: 波形の色と大きさで色と明るさが変化し、フレーズで見た目が切り替わります。テンポは Ableton Link から取得します。曲の情報とは、Beat Link Trigger (BLT) から届く `/dmx/main/rgb`・`/dmx/level`・`/dmx/phrase` のことです
- **フレーズのプリセット**: イントロ・バース・ブリッジ・ビルドアップ・サビ・ダウン・アウトロのそれぞれで、どう動作するかを **オート** タブの表で設定します。フレーズが届くと、その行の見た目になります

  | 項目 | 内容 |
  |---|---|
  | 動き・1周・揃え方・向き | ムービングの動きの形・1周の長さ・揃え方・向き。[ムービングの動き](operation.md#ムービングの動き) と同じもの |
  | 色・色の間隔 | メイン/サブ色を、そのまま / 拍で入れ替え / グラデーション のどれにするかと、その間隔 |
  | 明るさ | 全体光量に掛ける。イントロは暗め、サビは100% のように設定する。1秒かけて変化する |
  | 波形の色 | OFF にすると、そのフレーズの間は波形の色を使わず、パレットの色のまま |
  | 入った瞬間に全開 | そのフレーズに入った瞬間、1拍だけ現在の色で全開にする。レーザーは除く |
  | レーザー | ON にすると、そのフレーズの間のみ ARM 済みのレーザーを出力する。見本のプリセットではサビのみ ON。[レーザー](operation.md#レーザー) |
  | シーン | 呼び出すシーン |
  | エフェクト | ON にするエフェクト・チェイス・ピクセルエフェクト。次のフレーズで、そこに無いものは OFF に戻す |

  - **プリセットは必要な数だけ**作成でき、複製・名前の変更・削除ができます。見本として「標準」と「激しめ」が入っています。「標準」は盛り上がるほど速く大きく、静かな箇所はゆっくり暗めに動きます。使うプリセットを選び替えれば、曲調やイベントに合わせて全体の雰囲気を変えられます
  - 「**試す**」を押すと、オートが OFF でもすぐその見た目にできるため、現場で確認しながら作成できます。現在のフレーズの行には ● が付きます
  - `/dmx/phrase/<名前>` に割り当てがあるフレーズは、そちらが優先です。プリセットでシーンを選択していないフレーズでは、`サビ` のようにフレーズと同じ名前のシーンがあれば、それも呼び出します

- **フレーズの情報が届かない曲**: rekordbox でフレーズを解析していない曲などです。16拍ごとに、波形の大きさで見た目を切り替えます。`/dmx/level` を数秒ならした「盛り上がり」が大きければサビ、中くらいならバース、小さければダウンになります
- **手動優先**: MIDI コントローラーやキーボードで操作すると、既定で2分間はオートがフレーズ・色・明るさ・待機の色を変更しません。時間は設定で変更でき、0 にすると無効になります。時間が経過するか「オートに戻す」を押すと戻り、画面のクリックは対象外です
- **BLT が見つからないことの通知**: Link の仲間も曲の情報も60秒届かなければ、画面の下の帯に「⚠ BLTが見つかりません」と表示します。時間は設定で変更でき、0 にすると通知しません。帯を押すとオートタブが開き、タブの上に確認すべき内容が表示されます。`dmxctl headless` ではログに出力し、見つかるまで10分ごとに繰り返します。ログは `journalctl --user -u dmxctl-headless -f` で確認できます。Link の仲間がいて曲の情報だけが無いときは「BLT 再生待ち」と表示し、警告は出しません
- **届かなくなったとき**: 曲の情報が既定で5秒届かないと、全体を **薄い水色** の明るさ35%で待機し、ムービングを停止しレーザーを消灯します。色・明るさ・待ち時間は画面で変更でき、待機中も全体光量とブラックアウトは効きます

## モニターの無いLinuxでオート

照明卓用の小型 PC を DJ 機器と同じネットワークに接続して設置しておくだけで、電源を入れれば自動で動作する構成です。BLT も同じ PC に常駐させます。

```
CDJ / XDJ ── Pro DJ Link・LAN ── Linux PC ── USB-DMX または Art-Net ── 照明
                                  ├ Beat Link Trigger: 仮想の画面の中で動作
                                  │   ├ テンポ・拍 → Ableton Link
                                  │   └ 波形の色・大きさ・フレーズ → OSC で 127.0.0.1:9000 へ
                                  └ dmxctl headless: オートで動作
```

- `dmxctl headless` は、画面を出さずに動作します。モニター・X11・SDL2 は不要です。起動すると、OSC 受信・Ableton Link のテンポ・オートを必ず ON にします。待機中か受信中か・フレーズ・BPM・Link の台数を標準出力に表示します
- Beat Link Trigger は画面のあるアプリのため、Xvfb の仮想の画面の中で動作させます。画面を出さずに動作する [Open Beat Control](https://github.com/Deep-Symmetry/open-beat-control) もありますが、波形・フレーズは扱えないため、ここでは BLT を利用します

**準備**

1. 機材・出力・シーンなどの設定は画面のある PC で作成し、`~/.config/dmxctl/` をこの PC へコピーします。同じ PC にモニターを接続して作成しても構いません。フレーズごとの動きは **オート** タブのプリセットで設定しておきます。シーン・エフェクトも選択できます
2. [Releases](https://github.com/TechnoTUT/dmxctl/releases) から `dmxctl-linux-x86_64` をダウンロードします。Raspberry Pi などは `dmxctl-linux-arm64` です。このリポジトリも `git clone` しておきます。Debian 12・64bit の Raspberry Pi OS Bookworm・Ubuntu 24.04 以降で動作します
3. Beat Link Trigger (BLT) を導入します。BLT の [Releases](https://github.com/Deep-Symmetry/beat-link-trigger/releases) から `beat-link-trigger-*.jar` をダウンロードして、`~/dmxctl/beat-link-trigger.jar` という名前で配置します。**XDJ-AZ を利用するときは、正式版ではなくプレビュー版の jar を利用します。** XDJ-AZ への対応は、正式版の 8.0.0 にはまだ含まれておらず、プレビュー版で進んでいます。プレビュー版は Releases の [latest-preview](https://github.com/Deep-Symmetry/beat-link-trigger/releases/tag/latest-preview) にあり、名前は `beat-link-trigger-8.1.0-Preview.jar` のようになっています。対応状況は [CHANGELOG](https://github.com/Deep-Symmetry/beat-link-trigger/blob/main/CHANGELOG.md) で確認できます。BLT 8 を動作させるには Java 21 以上が必要です。BLT は仮想の画面に表示する画面のあるアプリなので、名前に headless の付かない JRE を導入します。headless の付く JRE には画面の部品が無く、BLT が起動できません。Ubuntu 24.04 と Debian 13 なら `sudo apt install openjdk-21-jre` で導入できます。Debian 12 と Raspberry Pi OS Bookworm は apt で導入できる Java が 17 までなので、[Adoptium](https://adoptium.net/installation/linux/) の Temurin 21 を導入するか、Java 11 以上で動作する BLT 7 の jar を `sudo apt install openjdk-17-jre` と組み合わせて利用します。BLT 7 は XDJ-AZ に対応していないため、XDJ-AZ を利用するときは Temurin 21 を導入します。jar を別の場所に配置するときは、`~/dmxctl/blt.env` に `BLT_JAR="jarの場所"` と記述します。以前の `.deb` などで導入した BLT を継続して利用するときは、`blt.env` に `BLT_CMD="起動コマンドの場所"` と記述すると、jar の代わりにそちらを起動します
4. リポジトリのフォルダで、取得したファイルの場所を付けて `./scripts/linux/install-headless.sh ~/Downloads/dmxctl-linux-arm64` を実行します。これで、電源を入れたら dmxctl と BLT がユーザーのサービスとして動作するようになります。`--no-blt` を付けると dmxctl だけになります。USB-DMX 用のグループに追加されたときは、一度再起動してください。ソースから利用するときは、`./scripts/build-linux.sh` のあと、場所を付けずに `./scripts/linux/install-headless.sh` を実行します

**BLT の設定**

最初に1回だけ、仮想の画面に VNC で接続して設定します。設定は BLT が保持し、次回からはそのまま動作します。

1. この PC で、VNC つきで BLT を再起動します

   ```
   systemctl --user stop beat-link-trigger
   BLT_VNC=1 ~/dmxctl/blt-headless.sh
   ```

2. 手元の PC から `ssh -L 5900:localhost:5900 ユーザー@このPC` で接続し、VNC ビューアで `localhost:5900` を開きます
3. BLT で次を設定します
   - Ableton Link: マスタープレイヤーのテンポと拍を Link に流す設定にします。dmxctl は BLT が動かす Carabiner にそのまま接続します。BLT のバージョンによっては、Carabiner を別に導入する必要があります
   - トリガーを1つ作成し、Watch を Master Player にします。Global Setup と Tracked Update に、波形の色・大きさを先読みで送信する式を入力します。式は [Beat Link Trigger との連携](integration.md#beat-link-trigger-との連携) にあります
   - フレーズを使うなら、Show に Phrase Trigger を作成し、Started で `/dmx/phrase` を送信します
4. BLT を終了すると設定が保存されます。端末で Ctrl+C を押し、`systemctl --user start beat-link-trigger` で元に戻します

**動作中の確認**

スマホや PC のブラウザで **`http://このPCのIPアドレス:8080/`** を開くと、操作パネルが表示されます。アドレスは起動時のログにも出力されます。操作パネルについては [スマホから操作する](remote.md#スマホから操作する) を確認してください。

```
journalctl --user -u dmxctl-headless -f
```

ログは `待機(薄い水色)` から、DJ 機器が曲を再生すると `曲の情報を受信中(フレーズ: chorus …)` のように変化します。

- 停止するときは `systemctl --user stop dmxctl-headless` を実行します。安全な値を送信してから停止します。設定を画面で変更したいときは、停止してから画面のある PC で起動します
- DJ 機器と同じネットワークに接続してください。BLT は rekordbox と同じ PC では動作させられません

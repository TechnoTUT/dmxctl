# オートと常駐

## オート

**オート** タブでオートをONにすると、人が操作しなくても照明が動きます。曲の情報が届けばそれに合わせて動き、届かなければ待機の色で待ちます。

- **曲の情報が届いている間**: 波形の色と大きさで色と明るさが変わり、フレーズで見た目が切り替わります。テンポは Ableton Link から取ります。曲の情報とは、Beat Link Trigger (BLT) から届く `/dmx/main/rgb`・`/dmx/level`・`/dmx/phrase` のことです
- **フレーズのプリセット**: イントロ・バース・ブリッジ・ビルドアップ・サビ・ダウン・アウトロのそれぞれで、どう動くかを **オート** タブの表で決めます。フレーズが届くと、その行の見た目になります

  | 項目 | 内容 |
  |---|---|
  | 動き・1周・揃え方・向き | ムービングの動きの形・1周の長さ・揃え方・向き。[ムービングの動き](operation.md#ムービングの動き) と同じもの |
  | 色・色の間隔 | メイン/サブ色を、そのまま / 拍で入れ替え / グラデーション のどれにするかと、その間隔 |
  | 明るさ | 全体光量に掛ける。イントロは暗め、サビは100% のように決める。1秒かけて変わる |
  | 波形の色 | OFFにすると、そのフレーズの間は波形の色を使わず、パレットの色のまま |
  | 入った瞬間に全開 | そのフレーズに入った瞬間、1拍だけ今の色で全開にする。レーザーは除く |
  | シーン | 呼び出すシーン |
  | エフェクト | ONにするエフェクト・チェイス・ピクセルエフェクト。次のフレーズで、そこに無いものはOFFに戻す |

  - **プリセットはいくつでも**作れ、複製・名前の変更・削除ができます。見本として「標準」と「激しめ」が入っています。「標準」は盛り上がるほど速く大きく、静かなところはゆっくり暗めに動きます。使うプリセットを選び替えれば、曲調やイベントに合わせて全体の雰囲気を変えられます
  - 「**試す**」を押すと、オートがOFFでもすぐその見た目にできるので、現場で確かめながら作れます。今のフレーズの行には ● が付きます
  - `/dmx/phrase/<名前>` に割り当てがあるフレーズは、そちらが優先です。プリセットでシーンを選んでいないフレーズでは、`サビ` のようにフレーズと同じ名前のシーンがあれば、それも呼び出します

- **フレーズの情報が来ない曲**: rekordbox でフレーズを解析していない曲などです。16拍ごとに、波形の大きさで見た目を選びます。`/dmx/level` を数秒ならした「盛り上がり」が大きければサビ、中くらいならバース、小さければダウンになります
- **手動優先**: MIDIコントローラーやキーボードで操作すると、既定で2分間はオートがフレーズ・色・明るさ・待機の色を変えません。時間は設定で変えられ、0 にすると無効になります。時間が経つか「オートに戻す」を押すと戻り、画面のクリックは対象外です
- **BLT が見つからないと知らせる**: Link の仲間も曲の情報も60秒届かなければ、画面の下の帯に「⚠ BLTが見つかりません」と出します。時間は設定で変えられ、0 にすると知らせません。帯を押すとオートタブが開き、タブの上に確かめることが出ます。`dmxctl headless` ではログに出し、見つかるまで10分ごとに繰り返します。ログは `journalctl --user -u dmxctl-headless -f` で見られます。Link の仲間がいて曲の情報だけが無いときは「BLT 再生待ち」と表示し、警告は出しません
- **届かなくなったら**: 曲の情報が既定で5秒届かないと、全体を **薄い水色** の明るさ35%で待機し、ムービングを止めレーザーを消します。色・明るさ・待ち時間は画面で変えられ、待機中も全体光量とブラックアウトは効きます

## モニターの無いLinuxでオート

照明卓用の小さなPCを DJ機器と同じネットワークにつないで置いておくだけで、電源を入れれば自動で動く構成です。BLT も同じPCに常駐させます。

```
CDJ / XDJ ── Pro DJ Link・LAN ── Linux PC ── USB-DMX または Art-Net ── 照明
                                  ├ Beat Link Trigger: 仮想の画面の中で動く
                                  │   ├ テンポ・拍 → Ableton Link
                                  │   └ 波形の色・大きさ・フレーズ → OSC で 127.0.0.1:9000 へ
                                  └ dmxctl headless: オートで動く
```

- `dmxctl headless` は、画面を出さずに動きます。モニター・X11・SDL2 は要りません。起動すると、OSC受信・Ableton Link のテンポ・オートを必ずONにします。待機中か受信中か・フレーズ・BPM・Link の台数を標準出力に出します
- Beat Link Trigger は画面のあるアプリなので、Xvfb の仮想の画面の中で動かします。画面を出さずに動く [Open Beat Control](https://github.com/Deep-Symmetry/open-beat-control) もありますが、波形・フレーズは扱えないので、ここでは BLT を使います

**準備**

1. 機材・出力・シーンなどの設定は画面のあるPCで作成し、`~/.config/dmxctl/` をこのPCへコピーします。同じPCにモニターをつないで作ってもかまいません。フレーズごとの動きは **オート** タブのプリセットで決めておきます。シーン・エフェクトも選べます
2. [Releases](https://github.com/TechnoTUT/dmxctl/releases) から `dmxctl-linux-x86_64` を落とします。Raspberry Pi などは `dmxctl-linux-arm64` です。このリポジトリも `git clone` しておきます。Debian 12・64bit の Raspberry Pi OS Bookworm・Ubuntu 24.04 以降で動きます
3. Beat Link Trigger (BLT) を入れます。BLT の [Releases](https://github.com/Deep-Symmetry/beat-link-trigger/releases) から `beat-link-trigger-*.jar` を落として、`~/dmxctl/beat-link-trigger.jar` という名前で置きます。BLT 8 を動かすには Java 21 以上が要ります。BLT は仮想の画面に出す画面のアプリなので、名前に headless の付かない JRE を入れます。headless の付く JRE には画面の部品が無く、BLT が起動できません。Ubuntu 24.04 と Debian 13 なら `sudo apt install openjdk-21-jre` で入ります。Debian 12 と Raspberry Pi OS Bookworm は apt で入る Java が 17 までなので、[Adoptium](https://adoptium.net/installation/linux/) の Temurin 21 を入れるか、Java 11 以上で動く BLT 7 の jar を `sudo apt install openjdk-17-jre` と組み合わせて使います。jar を別の場所に置くときは、`~/dmxctl/blt.env` に `BLT_JAR="jarの場所"` と書きます。以前の `.deb` などで入れた BLT を使い続けるときは、`blt.env` に `BLT_CMD="起動コマンドの場所"` と書くと、jar の代わりにそちらを起動します
4. リポジトリのフォルダで、落としたファイルの場所を付けて `./scripts/linux/install-headless.sh ~/Downloads/dmxctl-linux-arm64` を実行します。これで、電源を入れたら dmxctl と BLT がユーザーのサービスとして動くようになります。`--no-blt` を付けると dmxctl だけです。USB-DMX 用のグループに追加されたときは、一度再起動してください。ソースから使うときは、`./scripts/build-linux.sh` のあと、場所を付けずに `./scripts/linux/install-headless.sh` を実行します

**BLT の設定**

最初に1回だけ、仮想の画面に VNC で入って設定します。設定は BLT が覚えていて、次からはそのまま動きます。

1. このPCで、VNC つきで BLT を起動し直します

   ```
   systemctl --user stop beat-link-trigger
   BLT_VNC=1 ~/dmxctl/blt-headless.sh
   ```

2. 手元のPCから `ssh -L 5900:localhost:5900 ユーザー@このPC` でつなぎ、VNC ビューアで `localhost:5900` を開きます
3. BLT で次を設定します
   - Ableton Link: マスタープレイヤーのテンポと拍を Link に流す設定にします。dmxctl は BLT が動かす Carabiner にそのままつながります。BLT のバージョンによっては、Carabiner を別に入れる必要があります
   - トリガーを1つ作り、Watch を Master Player にします。Global Setup と Tracked Update に、波形の色・大きさを先読みで送る式を入れます。式は [Beat Link Trigger との連携](integration.md#beat-link-trigger-との連携) にあります
   - フレーズを使うなら、Show に Phrase Trigger を作り、Started で `/dmx/phrase` を送ります
4. BLT を終了すると設定が保存されます。端末で Ctrl+C を押し、`systemctl --user start beat-link-trigger` で元に戻します

**動かしているときの確認**

スマホやPCのブラウザで **`http://このPCのIPアドレス:8080/`** を開くと、操作パネルが出ます。アドレスは起動したときのログにも出ます。操作パネルについては [スマホから操作する](remote.md#スマホから操作する) を見てください。

```
journalctl --user -u dmxctl-headless -f
```

ログは `待機(薄い水色)` から、DJ機器が曲を再生すると `曲の情報を受信中(フレーズ: chorus …)` のように変わります。

- 止めるときは `systemctl --user stop dmxctl-headless` を実行します。安全な値を送ってから止まります。設定を画面で直したいときは、止めてから画面のあるほうで起動します
- DJ機器と同じネットワークにつないでください。BLT は rekordbox と同じPCでは動かせません

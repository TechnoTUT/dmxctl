# 映像ソフト・DJ機器との連携

## テンポの同期

「BPM/チェイス」タブの **同期 → テンポの元** で選びます。上部のBPMの横に状態が出ます。`Link 2` はつながっているアプリの数、`CLK` は MIDIクロックを受信中という意味です。

| テンポの元 | 動き |
|---|---|
| 内部 | 既定。このアプリで TAP や手入力で決めたBPMを使う |
| **Ableton Link** | 同じPCや同じネットワークにある Link 対応アプリと、**テンポと小節の頭**が揃います。TouchDesigner の **Ableton Link CHOP**、Resolume、Ableton Live、DJソフトなどが対応しています。相手のテンポ変更についていき、ここでの TAP・BPM変更・頭出しも相手に伝わります |
| MIDIクロック | 外から来る、1拍24回の MIDIクロックでテンポと拍を決めます。Start の 0xFA で小節の頭に揃います。クロックが途切れても、最後のテンポのまま内部で走り続けるので、照明は止まりません。入力は、メインのMIDI入力か、loopMIDI のような仮想ポートなど別のポートを選べます。クロック中はBPMの手入力とTAPは効きません |

- Link は、公式の C++ 実装の Ableton Link 4 をそのまま組み込んでいます。初めて Link や OSC を有効にしたとき、Windows ではファイアウォールの確認が出ます。exe の場所ごとに1回です。映像用PCなど別のPCとつなぐ場合は「許可」が必要です。Linux でファイアウォールを有効にしている場合は [Linux で使う](platforms.md#linux-で使う) を見てください
- TouchDesigner では、Ableton Link CHOP の `Enable` をONにするだけで、このアプリとテンポ・拍が揃います。Beat CHOP などで作った映像の動きと、照明のBPM同期エフェクト・チェイスが同じ拍で動きます

## OSC受信

「MIDI/OSC」タブの **OSCを受信** をONにします。UDP で、既定のポートは 9000 です。TouchDesigner なら OSC Out CHOP か OSC Out DAT から送ります。

- **割り当てにOSCアドレスを付ける**: マップ・MIDI・OSC タブの割り当ての「OSC」欄にアドレスを書くか、「OSCラーン」を押してから送ると登録されます。MIDIと同じ動作を OSC でも実行でき、MIDI と OSC の両方を付けてもかまいません。値は 0〜1 です。フェーダー系はそのまま使い、ボタン系は 0.5 以上で押した扱い、値が無ければ1回押した扱いになります。OSCの割り当てはページに関係なく反応します
- **決まったアドレス**: 割り当てなしで使えるアドレスです。一覧は画面の「OSCで使えるアドレス」にも出ます

| アドレス | 内容 |
|---|---|
| `/dmx/master f` | 全体光量。0〜1 |
| `/dmx/blackout f` | 1 で ON、0 で OFF |
| `/dmx/bpm f` / `/dmx/tap` / `/dmx/resync` | BPMの数値 / タップ / 頭出し |
| `/dmx/scene s` / `/dmx/position s` | シーン / 位置プリセットを名前で呼び出す |
| `/dmx/color s` / `/dmx/hue f` | 色を `赤` や `red` などの名前で / 色相を 0〜1 で一周。全体/個別スイッチに従う |
| `/dmx/main s` / `/dmx/sub s` | メイン色 / サブ色。色相は `/dmx/main/hue f`・`/dmx/sub/hue f` |
| `/dmx/swap` / `/dmx/swap/auto f` | メイン⇄サブ入れ替え / 拍で自動入れ替えを 1 か 0 で |
| `/dmx/main/rgb f f f [秒]` / `/dmx/sub/rgb f f f [秒]` | メイン色 / サブ色を RGB で。値は 0〜1 で、0〜255 でもよい。受けた色へ**なめらかに**近づく。4つ目の数値で近づく速さを秒で指定する |
| `/dmx/main/rgb/at f f f 秒` / `/dmx/sub/rgb/at …` | 先読みした色。その秒数後に**ちょうどその色**になるよう、まっすぐ近づく。遅れを打ち消すのに使う。下の Beat Link Trigger の例を参照 |
| `/dmx/rgb/smooth f` / `/dmx/rgb/off` | RGB で受けた色へ近づく標準の速さを秒で。0 ならすぐ変わる。画面でも変えられる / RGB の色をやめてパレットの色に戻す |
| `/dmx/level f [秒]` / `/dmx/level/at f 秒` | 波形の大きさなどの明るさ。0〜1 で、全体光量に掛ける。色と同じく、なめらかに、または先読みで近づく |
| `/dmx/level/depth f` / `/dmx/level/off` | 明るさの効き具合。0 なら効かず、1 なら明るさ0 のとき真っ暗。標準は 0.7 で、画面でも変えられる / やめる |
| `/dmx/phrase s` | 曲のフレーズ。`intro` / `verse` / `chorus` / `bridge` / `up` / `down` / `outro`。下の [Beat Link Trigger との連携](#beat-link-trigger-との連携) を参照 |
| `/dmx/effect s f` / `/dmx/chase s f` | エフェクト / チェイスの ON/OFF。名前と 1 か 0 を送る |
| `/dmx/page i` | パッドのページ。1〜8 |
| `/dmx/fixture/<ID>/<機能> f` | 機材1台のチャンネル。0〜1。例: `/dmx/fixture/1/dimmer` |
| `/dmx/all/<機能> f` | 全機材のチャンネル。0〜1。例: `/dmx/all/pan` |

- `#bundle` のバンドルにも対応しています。OSCを連続で受けていても、画面の更新は0.25秒に1回までに抑えています
- RGB で受けた色は保存しません。パッドや画面で手で色を選ぶと、RGB の受信はやめた扱いになり、手で選んだ色が効きます。NDI の色を使っている系統では、NDI の色が優先です

## Beat Link Trigger との連携

Beat Link Trigger (BLT) を使うと、CDJ や XDJ などのDJ機器から、テンポ・波形の色・曲の構成を受け取って照明に使えます。曲の構成は、イントロやサビなどのフレーズのことです。BLT は [GitHub](https://github.com/Deep-Symmetry/beat-link-trigger) から入手できます。

- **テンポ**: BLT の Ableton Link の機能で、マスタープレイヤーのテンポと拍を Link に流します。このアプリでは、テンポの元を「Ableton Link」にします。詳しくは [テンポの同期](#テンポの同期)
- **波形の色をメイン色に**: BLT から、**少し先の位置**の波形の色を `/dmx/main/rgb/at r g b 秒` で送ります。このアプリは「その秒数後にちょうどその色」になるよう、まっすぐ色を変えていきます。BLT が状態を受ける約0.2秒の間隔・ネットワーク・照明の反応の遅れを先読みで打ち消すので、曲と色がずれません。次の色が届くと、そこからまたつながるので、途切れずになめらかに変わります
  - 先読みしない `/dmx/main/rgb` でも使えます。受けた色へなめらかに近づきますが、そのぶん少し遅れます
- **波形の大きさを明るさに**: 同じように `/dmx/level/at 大きさ 秒` で送ると、曲の大きい所で明るく、静かな所で暗くなります。全体光量に掛けます。どこまで暗くするかは「MIDI/OSC」タブの「効き具合」で決めます。標準は70%で、100%なら無音のとき真っ暗です
- **フレーズで照明を切り替える**: BLT から `/dmx/phrase` にフレーズの名前を送ります。次のどちらかが動きます
  1. `/dmx/phrase/<名前>` を OSC アドレスに付けた割り当て。たとえば `/dmx/phrase/chorus` をストロボのパッドの動作に付けます。OSCラーンを押して、BLT から1回送って登録してもかまいません
  2. 割り当てが無ければ、オートのプリセットの見た目と、同じ名前のシーンが動きます。オートについては [オート](auto.md#オート) を見てください。名前は英語の `chorus` でも、日本語の `サビ`・`イントロ`・`バース`・`ブリッジ`・`アップ`・`ダウン`・`アウトロ` でもかまいません。`verse2` のような番号つきは `verse` や `バース` として扱います
- 「MIDI/OSC」タブに、受けている RGB の色と、最後に受けたフレーズが出ます

BLT 側の設定例です。BLT の式は Clojure で書きます。関数の名前などは BLT のバージョンで変わることがあるので、BLT のドキュメントの「Expressions」「Shows」も見てください。

```clojure
;; トリガーの Global Setup: OSCの送り先。このアプリのPCのIPとポート
(swap! globals assoc :dmx (osc/osc-client "127.0.0.1" 9000))

;; トリガーの Tracked Update: 再生中、状態が届くたびに動く。Watch は Master Player にする
;; 遅れを打ち消すため、LEAD 秒先の波形の色を読み、「LEAD 秒後にこの色」として送る
(let [lead   0.3     ; 先読みする秒数。状態が届く間隔の約0.2秒より長くする
      light  0.05    ; DMXの送信や機材の反応で、照明が遅れて光るぶんの秒数
      n      (.getDeviceNumber status)
      ms     (.getTimeFor (org.deepsymmetry.beatlink.data.TimeFinder/getInstance) n)
      speed  (org.deepsymmetry.beatlink.Util/pitchToMultiplier (.getPitch status))
      detail (.getLatestDetailFor (org.deepsymmetry.beatlink.data.WaveformFinder/getInstance) n)]
  (when (and detail (.isPlaying status) (pos? ms))
    ;; 曲の中の時間は再生速度のぶん速く進むので、先読みする量にも速度を掛ける
    (let [ahead (+ ms (* lead 1000 speed))
          start (org.deepsymmetry.beatlink.Util/timeToHalfFrame ahead)
          segs  (range (max 0 (- start 8)) (min (+ start 8) (.getFrameCount detail)))  ; その前後 約0.1秒を平均する
          cols  (map #(.segmentColor detail % false) segs)
          avg   (fn [f] (/ (reduce + (map f cols)) (max 1 (count cols)) 255.0))
          ;; 0〜31 の波形の高さの平均を 0〜1 にする
          size  (/ (reduce + (map #(.segmentHeight detail % false) segs)) (max 1 (count segs)) 31.0)]
      (when (seq cols)
        (osc/osc-send (:dmx @globals) "/dmx/main/rgb/at"
                      (float (avg #(.getRed %))) (float (avg #(.getGreen %))) (float (avg #(.getBlue %)))
                      (float (- lead light)))
        (osc/osc-send (:dmx @globals) "/dmx/level/at" (float (min 1.0 size)) (float (- lead light)))))))
```

フレーズは、BLT の **Show** にサビ用・イントロ用などの **Phrase Trigger** を作り、その「Started」の式で送ります。

```clojure
(osc/osc-send (:dmx @globals) "/dmx/phrase" "chorus")
```

- 波形の色は、rekordbox で解析した曲の RGB カラー波形から取ります。3バンド波形の曲では色の意味が変わります
- 波形の色には暗い所や白っぽい所もあるので、鮮やかな色にしたいときは BLT の式で彩度・明るさを上げてから送ってください

## TechnoTUT ネットワークの BLT を使う

TechnoTUT ネットワークには、Beat Link Trigger がすでに動いている環境があります。これを使えば、dmxctl のPCに BLT を入れる必要はありません。環境の使い方は [Beat Link Trigger の案内](https://network.technotut.net/service/beatlinktrigger/) にあります。

```
CDJ / XDJ ── Pro DJ Link ── BLT: TechnoTUT ネットワークのコンテナ、Debian のデスクトップ
                              ├ テンポ・拍 → Ableton Link。Carabiner で設定済み
                              └ 波形の色・大きさ・フレーズ → OSC → dmxctl のPC、ポート 9000
```

**1. BLT を起動する**

案内のページの手順です。

1. TechnoTUT ネットワークにつなぐ
2. 次のどれかで、BLT の環境に入る
   - ブラウザの noVNC: `http://web.blt.svc.technotut.net/vnc.html`
   - RDP: `rdp.blt.svc.technotut.net`
   - VNC: `blt.svc.technotut.net:5901`
3. デスクトップの `blt-start.sh` を実行する。止まったときは、ウィンドウを閉じてもう一度実行する

**2. テンポを Ableton Link で揃える**

この BLT は、DJ機器とつながると Carabiner で Link に接続し、マスタープレイヤーのテンポに合わせて**小節の頭まで揃える**設定になっています。Came Online の式で設定してあります。dmxctl 側は、テンポの元を **Ableton Link** にするだけです。`dmxctl headless` なら自動で Link になります。

- 上部のBPMの横が `Link 1` 以上なら、つながっています。`Link 0` のままなら、同じネットワーク内のマルチキャストで流れる Link の通信が、dmxctl のPCまで届いていません。そのときは下の Beat の式で、拍を OSC で送ります

**3. BLT にトリガーを1つ足す**

色・明るさ・フレーズを dmxctl へ送るトリガーです。BLT の Triggers のウィンドウでトリガーを1つ作り、**Watch** を **Master Player** にして、次の式を入れます。`<dmxctlのIP>` は、dmxctl を動かしているPCの TechnoTUT ネットワークでのIPアドレスです。OSCのポートは画面で設定したもので、既定は 9000 です。

Setup の式です。トリガーを作ったときと、BLT を起動したときに動きます。

```clojure
(swap! locals assoc :dmx (osc/osc-client "<dmxctlのIP>" 9000))
```

Tracked Update の式は、再生中に状態が届くたびに動きます。[Beat Link Trigger との連携](#beat-link-trigger-との連携) の、先読みで波形の色・大きさを送る式を使います。ただし、2か所ある `(:dmx @globals)` を `(:dmx @locals)` に替えます。

Beat の式は拍ごとに動きます。**`Link 0` のときだけ** 入れてください。Link が届いているなら要りません。

```clojure
;; Link の代わりに、テンポを送り、小節の頭で頭出しする
(osc/osc-send (:dmx @locals) "/dmx/bpm" (float effective-tempo))
(when (= beat-within-bar 1)
  (osc/osc-send (:dmx @locals) "/dmx/resync"))
```

Shutdown の式です。トリガーを消したときと、BLT を終了するときに動きます。

```clojure
(osc/osc-close (:dmx @locals))
```

フレーズは、BLT の **Show** に **Phrase Trigger** を作り、その「Started」の式で送ります。送り先は `(:dmx @locals)` ではなく、Show の Global Setup で作ったものを使います。

```clojure
(osc/osc-send (:dmx @globals) "/dmx/phrase" "chorus")
```

**4. dmxctl を動かす**

- 画面のあるPCでは、「MIDI/OSC」タブで **OSCを受信** をONにし、**オート** タブで **オート** をONにします
- モニターの無い Linux では、`./scripts/linux/install-headless.sh --no-blt` で dmxctl だけを常駐させます。詳しくは [モニターの無いLinuxでオート](auto.md#モニターの無いlinuxでオート)

BLT が止まっている間や、DJ機器が再生していない間は、dmxctl は薄い水色で待ちます。BLT を起動し直せば、自動で元に戻ります。

**注意**

- 案内のページが参照している BLT のユーザーガイドは 0.6.3 版です。BLT が古い版だと、波形の色を取る `segmentColor` や、BLT 7 からの Show の Phrase Trigger が使えないことがあります。BLT の版は、BLT のメニューの About で確かめてください。使えないときは、Link のテンポと、波形の大きさ・拍だけで動かします。オートは、フレーズが来ない曲では波形の大きさで見た目を選びます
- dmxctl のPCのファイアウォールで、OSC の UDP 9000 と Link の UDP 20808 を受け取れるようにしてください

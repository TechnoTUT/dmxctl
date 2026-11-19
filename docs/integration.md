# 映像ソフト・DJ機器との連携

## テンポの同期

「BPM/チェイス」タブの **同期 → テンポの元** で選択します。上部の BPM の横に状態が表示されます。`Link 2` は接続しているアプリの数、`CLK` は MIDI クロックを受信中という意味です。

| テンポの元 | 動作 |
|---|---|
| 内部 | 既定。このアプリで TAP や手入力で決定した BPM を利用する |
| **Ableton Link** | 同じ PC や同じネットワークにある Link 対応アプリと、**テンポと小節の頭**が一致します。TouchDesigner の **Ableton Link CHOP**、Resolume、Ableton Live、DJ ソフトなどが対応しています。相手のテンポ変更に追従し、ここでの TAP・BPM 変更・頭出しも相手に伝わります |
| MIDI クロック | 外部から入力される、1拍24回の MIDI クロックでテンポと拍を決定します。Start の 0xFA で小節の頭に合わせます。クロックが途切れても、最後のテンポのまま内部で動き続けるため、照明は停止しません。入力は、メインの MIDI 入力か、loopMIDI のような仮想ポートなど別のポートを選択できます。クロック受信中は BPM の手入力と TAP は効きません |

- **Carabiner が必要です**: Link はライセンスの都合でこのアプリに含めておらず、別のプログラムの [Carabiner](https://github.com/Deep-Symmetry/carabiner/releases) が Link に参加します。このアプリは、同じ PC で動作する Carabiner に、ポート 17000 で接続します。ポートを変更したときは、環境変数 `DMXCTL_CARABINER_PORT` に同じ番号を設定します
- **導入方法**: テンポの元を Link にして、「Carabinerをダウンロードして入れる」を押します。画面を使わない PC では `dmxctl carabiner install` で導入します。配布元の GitHub から取得し、検証した値と一致したときだけ、設定フォルダの `carabiner/` に配置します。自動では取得しません。Windows・Mac・Linux 用があります。Linux は x86_64 と arm64 です
- **起動**: 導入後は、Link を利用するとき、このアプリが自動で Carabiner を起動し、終了時に停止します。既に誰かが Carabiner を動かしていれば、それに接続するだけです。自分で導入した Carabiner を利用するなら、先に起動しておいてください
- Carabiner に接続していないときは、「BPM/チェイス」タブに「Carabinerにつながっていません」と表示されます。その間は、内部の時計で拍が進みます。接続できるようになれば、自動で切り替わります
- 再接続したときに Link の仲間がいなければ、切断前のテンポに戻します。仲間がいれば、そのセッションのテンポに合わせます
- **Beat Link Trigger を利用しているとき**: BLT も Carabiner を利用します。BLT が Carabiner を動かしていれば、このアプリはそれにそのまま接続するため、別に起動する必要はありません
- Windows では、Carabiner や OSC を初めて有効にしたときにファイアウォールの確認が表示されます。別の PC と接続する場合は「許可」が必要です。Linux でファイアウォールを有効にしている場合は [Linux で使う](platforms.md#linux-で使う) を確認してください
- TouchDesigner では、Ableton Link CHOP の `Enable` を ON にするだけで、このアプリとテンポ・拍が一致します。Beat CHOP などで作成した映像の動きと、照明の BPM 同期エフェクト・チェイスが同じ拍で動きます

## OSC受信

「MIDI/OSC」タブの **OSCを受信** を ON にします。UDP で、既定のポートは 9000 です。TouchDesigner なら OSC Out CHOP か OSC Out DAT から送信します。

- **割り当てに OSC アドレスを付ける**: マップ・MIDI・OSC タブの割り当ての「OSC」欄にアドレスを記入するか、「OSCラーン」を押してから送信すると登録されます。MIDI と同じ動作を OSC でも実行でき、MIDI と OSC の両方を付けても構いません。値は 0〜1 です。フェーダー系はそのまま使い、ボタン系は 0.5 以上で押下扱い、値が無ければ1回押下扱いになります。OSC の割り当てはページに関係なく反応します
- **固定のアドレス**: 割り当てなしで利用できるアドレスです。一覧は画面の「OSCで使えるアドレス」にも表示されます

| アドレス | 内容 |
|---|---|
| `/dmx/master f` | 全体光量。0〜1 |
| `/dmx/blackout f` | 1 で ON、0 で OFF |
| `/dmx/bpm f` / `/dmx/tap` / `/dmx/resync` | BPM の数値 / タップ / 頭出し |
| `/dmx/scene s` / `/dmx/position s` | シーン / 位置プリセットを名前で呼び出す |
| `/dmx/color s` / `/dmx/hue f` | 色を `赤` や `red` などの名前で / 色相を 0〜1 で一周。全体/個別スイッチに従う |
| `/dmx/main s` / `/dmx/sub s` | メイン色 / サブ色。色相は `/dmx/main/hue f`・`/dmx/sub/hue f` |
| `/dmx/swap` / `/dmx/swap/auto f` | メイン⇄サブ入れ替え / 拍で自動入れ替えを 1 か 0 で |
| `/dmx/main/rgb f f f [秒]` / `/dmx/sub/rgb f f f [秒]` | メイン色 / サブ色を RGB で。値は 0〜1 で、0〜255 でもよい。受信した色へ**なめらかに**近づく。4つ目の数値で近づく速さを秒で指定する |
| `/dmx/main/rgb/at f f f 秒` / `/dmx/sub/rgb/at …` | 先読みした色。その秒数後に**ちょうどその色**になるよう、まっすぐ近づく。遅延を打ち消すために使う。下の Beat Link Trigger の例を参照 |
| `/dmx/rgb/smooth f` / `/dmx/rgb/off` | RGB で受信した色へ近づく標準の速さを秒で。0 なら即座に変化する。画面でも変更可能 / RGB の色を停止してパレットの色に戻す |
| `/dmx/level f [秒]` / `/dmx/level/at f 秒` | 波形の大きさなどの明るさ。0〜1 で、全体光量に掛ける。色と同様に、なめらかに、または先読みで近づく |
| `/dmx/level/depth f` / `/dmx/level/off` | 明るさの効き具合。0 なら効かず、1 なら明るさ0 のとき真っ暗。標準は 0.7 で、画面でも変更可能 / 停止 |
| `/dmx/phrase s` | 曲のフレーズ。`intro` / `verse` / `chorus` / `bridge` / `up` / `down` / `outro`。下の [Beat Link Trigger との連携](#beat-link-trigger-との連携) を参照 |
| `/dmx/effect s f` / `/dmx/chase s f` | エフェクト / チェイスの ON/OFF。名前と 1 か 0 を送信する |
| `/dmx/page i` | パッドのページ。1〜8 |
| `/dmx/fixture/<ID>/<機能> f` | 機材1台のチャンネル。0〜1。例: `/dmx/fixture/1/dimmer` |
| `/dmx/all/<機能> f` | 全機材のチャンネル。0〜1。例: `/dmx/all/pan` |

- `#bundle` のバンドルにも対応しています。OSC を連続で受信していても、画面の更新は0.25秒に1回までに抑えています
- RGB で受信した色は保存しません。パッドや画面で手動で色を選択すると、RGB の受信は停止した扱いになり、手動で選択した色が有効になります。NDI の色を利用している系統では、NDI の色が優先です

## Beat Link Trigger との連携

Beat Link Trigger (BLT) を利用すると、CDJ や XDJ などの DJ 機器から、テンポ・波形の色・曲の構成を受信して照明に利用できます。曲の構成は、イントロやサビなどのフレーズのことです。BLT は [GitHub](https://github.com/Deep-Symmetry/beat-link-trigger) から入手できます。

- **テンポ**: BLT の Ableton Link の機能で、マスタープレイヤーのテンポと拍を Link に流します。このアプリでは、テンポの元を「Ableton Link」にします。詳しくは [テンポの同期](#テンポの同期)
- **波形の色をメイン色に**: BLT から、**少し先の位置**の波形の色を `/dmx/main/rgb/at r g b 秒` で送信します。このアプリは「その秒数後にちょうどその色」になるよう、まっすぐ色を変化させます。BLT の受信間隔やネットワーク・照明の遅延を先読みで打ち消すため、曲と色がずれません。次の色が届くと現在値から連続し、途切れずになめらかに変化します
  - 先読みしない `/dmx/main/rgb` でも利用できます。受信した色へなめらかに近づきますが、その分わずかに遅れます
- **波形の大きさを明るさに**: 同様に `/dmx/level/at 大きさ 秒` で送信すると、曲の大きい箇所で明るく、静かな箇所で暗くなります。全体光量に掛けます。どこまで暗くするかは「MIDI/OSC」タブの「効き具合」で決定します。標準は70%で、100%なら無音のとき真っ暗です
- **フレーズで照明を切り替える**: BLT から `/dmx/phrase` にフレーズの名前を送信します。次のどちらかが動作します
  1. `/dmx/phrase/<名前>` を OSC アドレスに付けた割り当て。たとえば `/dmx/phrase/chorus` をストロボのパッドの動作に付けます。OSCラーンを押して、BLT から1回送信して登録しても構いません
  2. 割り当てが無ければ、オートのプリセットの見た目と、同一の名前のシーンが動作します。オートについては [オート](auto.md#オート) を確認してください。名前は英語の `chorus` でも、日本語の `サビ`・`イントロ`・`バース`・`ブリッジ`・`アップ`・`ダウン`・`アウトロ` でも構いません。`verse2` のような番号付きは `verse` や `バース` として扱います
- 「MIDI/OSC」タブに、受信中の RGB の色と、最後に受信したフレーズが表示されます

BLT 側の設定例です。BLT の式は Clojure で記述します。関数の名前などは BLT のバージョンで変わることがあるため、BLT のドキュメントの「Expressions」「Shows」も確認してください。

```clojure
;; トリガーの Global Setup: OSCの送信先。このアプリのPCの IPアドレスとポート
(swap! globals assoc :dmx (osc/osc-client "127.0.0.1" 9000))

;; トリガーの Tracked Update: 再生中、状態が届くたびに動く。Watch は Master Player にする
;; 遅延を打ち消すため、LEAD 秒先の波形の色を読み、「LEAD 秒後にこの色」として送信する
(let [lead   0.3     ; 先読みする秒数。状態が届く間隔の約0.2秒より長くする
      light  0.05    ; DMXの送信や機材の反応で、照明が遅れて点灯するぶんの秒数
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

フレーズは、BLT の **Show** にサビ用・イントロ用などの **Phrase Trigger** を作成し、その「Started」の式で送信します。

```clojure
(osc/osc-send (:dmx @globals) "/dmx/phrase" "chorus")
```

- 波形の色は、rekordbox で解析した曲の RGB カラー波形から取得します。3バンド波形の曲では色の意味が変わります
- 波形の色には暗い箇所や白っぽい箇所もあるため、鮮やかな色にしたいときは BLT の式で彩度・明るさを上げてから送信してください

## TechnoTUT ネットワークの BLT を使う

TechnoTUT ネットワークには、Beat Link Trigger が既に動作している環境があります。これを利用すれば、dmxctl の PC に BLT を導入する必要はありません。環境の利用方法は [Beat Link Trigger の案内](https://network.technotut.net/service/beatlinktrigger/) にあります。

```
CDJ / XDJ ── Pro DJ Link ── BLT: TechnoTUT ネットワークのコンテナ、Debian のデスクトップ
                              ├ テンポ・拍 → Ableton Link。Carabiner で設定済み
                              └ 波形の色・大きさ・フレーズ → OSC → dmxctl のPC、ポート 9000
```

**1. BLT を起動する**

案内のページの手順です。

1. TechnoTUT ネットワークに接続する
2. 次のいずれかで、BLT の環境に入る
   - ブラウザの noVNC: `http://web.blt.svc.technotut.net/vnc.html`
   - RDP: `rdp.blt.svc.technotut.net`
   - VNC: `blt.svc.technotut.net:5901`
3. デスクトップの `blt-start.sh` を実行する。停止したときは、ウィンドウを閉じてもう一度実行する

**2. テンポを Ableton Link で一致させる**

この BLT は、DJ 機器と接続すると Carabiner で Link に接続し、マスタープレイヤーのテンポに合わせて**小節の頭まで一致させる**設定になっています。Came Online の式で設定してあります。dmxctl 側は、テンポの元を **Ableton Link** にするだけです。`dmxctl headless` なら自動で Link になります。

- 上部の BPM の横が `Link 1` 以上なら、接続されています。`Link 0` のままならマルチキャスト通信が dmxctl の PC へ届いていないため、下の Beat の式で拍を OSC で送信します

**3. BLT にトリガーを1つ追加する**

色・明るさ・フレーズを dmxctl へ送信するトリガーです。BLT の Triggers のウィンドウでトリガーを1つ作成し、**Watch** を **Master Player** にして、次の式を入力します。`<dmxctlのIP>` は、dmxctl を動作させている PC の TechnoTUT ネットワークでの IPアドレスです。OSC のポートは画面で設定したもので、既定は 9000 です。

Setup の式です。トリガーを作成したときと、BLT を起動したときに動作します。

```clojure
(swap! locals assoc :dmx (osc/osc-client "<dmxctlのIP>" 9000))
```

Tracked Update の式は、再生中に状態が届くたびに動作します。[Beat Link Trigger との連携](#beat-link-trigger-との連携) の、先読みで波形の色・大きさを送信する式を使います。ただし、2か所ある `(:dmx @globals)` を `(:dmx @locals)` に置き換えます。

Beat の式は拍ごとに動作します。**`Link 0` のときだけ** 入力してください。Link が届いているなら不要です。

```clojure
;; Link の代わりに、テンポを送信し、小節の頭で頭出しする
(osc/osc-send (:dmx @locals) "/dmx/bpm" (float effective-tempo))
(when (= beat-within-bar 1)
  (osc/osc-send (:dmx @locals) "/dmx/resync"))
```

Shutdown の式です。トリガーを削除したときと、BLT を終了するときに動作します。

```clojure
(osc/osc-close (:dmx @locals))
```

フレーズは、BLT の **Show** に **Phrase Trigger** を作成し、その「Started」の式で送信します。送信先は `(:dmx @locals)` ではなく、Show の Global Setup で作成したものを使います。

```clojure
(osc/osc-send (:dmx @globals) "/dmx/phrase" "chorus")
```

**4. dmxctl を動作させる**

- 画面のある PC では、「MIDI/OSC」タブで **OSCを受信** を ON にし、**オート** タブで **オート** を ON にします
- モニターの無い Linux では、`./scripts/linux/install-headless.sh --no-blt` で dmxctl だけを常駐させます。詳しくは [モニターの無い Linux でオート](auto.md#モニターの無いlinuxでオート)

BLT の停止中や DJ 機器の非再生時は薄い水色で待機し、BLT を再起動すると自動で復帰します。

**注意**

- 案内のページが参照している BLT のユーザーガイドは 0.6.3 版です。BLT が古い版だと、波形の色を取得する `segmentColor` や、BLT 7 からの Show の Phrase Trigger が利用できないことがあります。BLT の版は、BLT のメニューの About で確認してください。利用できないときは、Link のテンポと、波形の大きさ・拍だけで動作させます。オートは、フレーズが届かない曲では波形の大きさで見た目を選択します
- dmxctl の PC のファイアウォールで、OSC の UDP 9000 と Link の UDP 20808 を受信できるようにしてください

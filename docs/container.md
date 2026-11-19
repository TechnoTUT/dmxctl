# コンテナで動かす

`dmxctl serve` を、コンテナと Kubernetes で動作させる手順です。Podman でも Docker でも作成できます。画面は使わず、自動操作・Art-Net の出力・操作パネルだけを動作させます。

## できることと、できないこと

できること
- Art-Net で照明機材へ値を送信する
- ブラウザの操作パネルを開く。Service では TCP の 80 番で受信する
- OSC を受信する。UDP の 9000 番で受信する
- `ctl` コマンドで様子を確認する・操作する

できないこと・注意点
- **NDI は利用できません。** NDI のライブラリは配布条件のため、イメージに含めていません。
- **USB-DMX は利用しません。** このイメージは Art-Net の出力だけを想定しています。
- **MIDI と音の入力は利用しません。** 入力は OSC と操作パネルだけです。
- **Ableton Link と PRO DJ LINK は届きません。** これらは同じ L2 ネットワークが必要です。Pod の標準ネットワークでは届きません。照明の VLAN lbar へ直接出力する設定は、Infra 側で VLAN 30 のブリッジを作成してから行います。

## ファイル

| ファイル | 役割 |
|---|---|
| `Containerfile` | ビルドと実行のイメージ。画面を持たない `dmxctl-server` をビルドし、イメージの中では `dmxctl` の名前で入れる。実行するのは `dmxctl serve` だけ |
| `.dockerignore` | ビルドに送信しないもの。`target/` などは数 GB あるため除外する |
| `deploy/kubernetes/` | Kubernetes のマニフェスト。`kubectl apply -k` で配置できる |
| `deploy/kubernetes/argocd-application.example.yaml` | Infra の Argo CD に登録する Application の例 |

## イメージを作る

ビルドには、サブモジュール `moip-engine/` が必要です。先に取得してから作成します。

```
git submodule update --init --recursive
podman build -t dmxctl:0.1.0 -f Containerfile .
```

Docker の場合も `docker build -t dmxctl:0.1.0 -f Containerfile .` で同様に作成できます。初回は、Rust のビルドに数十分かかることがあります。2回目以降は、キャッシュで速くなります。

## 手元で試す

本番の設定には触れず、コピーした設定で試します。

```
mkdir -p data && cp examples/cli/project.json data/
podman run --rm --name dmxctl -p 8080:8080 -v "$PWD/data:/data:Z" dmxctl:0.1.0
```

別の端末から様子を確認します。

```
podman exec dmxctl dmxctl ctl status
```

イメージの既定は、実機に出力せず、自動保存もしない安全な設定です。実機へ出力するときは、次の「実機に出す」を確認してください。

## Kubernetes に配置する

前提は、[Infra](https://github.com/TechnoTUT/Infra) の構成です。設定の保存に `nfs-client`、イメージの置き場に `registry.svc.technotut.net`、固定 IP に LoadBalancer の範囲と `lbipam.technotut.net/static` のラベルを使います。

### 1. イメージを置く

```
podman tag dmxctl:0.1.0 registry.svc.technotut.net/dmxctl:0.1.0
podman push registry.svc.technotut.net/dmxctl:0.1.0
```

### 2. マニフェストを置く

namespace を作成し、一式を適用します。

```
kubectl create namespace dmxctl
kubectl apply -k deploy/kubernetes
```

Argo CD で管理するときは、`deploy/kubernetes/` を Infra の `k8s/manifest/dmxctl/` にコピーし、`argocd-application.example.yaml` を `k8s/manifest/apps/dmxctl.yaml` として配置します。

### 3. 設定を入れる

設定の `project.json` は、Secret の `dmxctl-seed` に入れます。Pod の起動時に、PVC の `/data` に設定が無いときだけ、initContainer が Secret からコピーします。PVC に既に設定があれば何もしないため、GUI で変更した内容は消えません。

Secret は、手元の設定から作成します。設定には合言葉などが含まれるため、Git には含めません。形の見本は `deploy/kubernetes/example/project.json` にあります。Kustomize で、ConfigMap の `dmxctl-example` になります。

```
kubectl -n dmxctl create secret generic dmxctl-seed --from-file=project.json=$HOME/.config/dmxctl/project.json
```

Secret を作成したあとで Pod を起動します。`kubectl -n dmxctl rollout restart deploy/dmxctl` で、すぐ再起動できます。

注意点は次のとおりです。

- 作成するのは、初回だけで十分です。Secret を作り直しても、PVC に設定があれば入れ替わりません。
- 設定を入れ直したいときは、PVC の `project.json` を削除してから Pod を再起動します。削除する前に、必ずバックアップを取得してください。

### 4. 様子を確認する

```
kubectl -n dmxctl logs deploy/dmxctl
kubectl -n dmxctl exec deploy/dmxctl -- dmxctl ctl status
```

ログに `"ready":true` が出力されれば、起動できています。

## 実機に出す

マニフェストの `args` で、既定は `--output none` です。Art-Net を実機へ送信するときは、設定の出力先、つまり `project.json` の `outputs` を確認してから、次のように書き換えて適用します。

```yaml
        - --output
        - live
```

`--output live` にすると、設定の出力先へ Art-Net を送信します。最初は `none` のまま、`ctl` で最終の DMX 値を確認することをおすすめします。

## 別の PC から操作する

`dmxctl connect` のクライアントから操作するときは、listen を追加します。listen には認証がなく、接続した人は設定の読み込み・保存やレーザーの ARM までできます。そのため既定では開放していません。照明用の LAN の中だけで利用してください。

`deployment.yaml` の `args` に次を追加し、`ports` に `containerPort: 9300` を、`service.yaml` の `ports` に 9300 を追加します。

```yaml
        - --remote-listen
        - 0.0.0.0:9300
```

`--remote-allow-safety-edit` を追加すると、クライアントからレーザーなどの安全の設定、つまり安全な値と ARM が必要かどうかも変更できます。安全の設定を変更するときだけ一時的に追加し、終わったら外してください。

## 安全のための決まり

- **ARM は起動のたびに解除されます。** 保存もされず、CLI からは ARM にできません。
- **停止するとき、安全な値を送信してから停止します。** Kubernetes は SIGTERM を送信します。`terminationGracePeriodSeconds` を 10 秒にしているのは、この安全な値を送信する時間を待つためです。
- **Pod は 1 つだけ動作させます。** 2つ動作させると、同じ Art-Net の値が交互に届きます。
- **操作パネルを有効にするときは PIN を付けます。** 操作パネルは LAN に開きます。`project.json` の `web_pin` が空のままだと、LAN の誰でも操作できます。
- **制御の窓口は、コンテナの中だけに開きます。** 外部からは届きません。`ctl` は `kubectl exec` で利用します。

## ポート

| 番号 | 種類 | 用途 |
|---|---|---|
| Service の 80 → Pod の 8080 | TCP | 操作パネル。番号は `project.json` の `web_port` |
| 9000 | UDP | OSC の受信。番号は `project.json` の `osc_port` |
| 9190 | TCP | 制御の窓口。コンテナの中だけ。`ctl` の既定の接続先 |
| 9300 | TCP | `dmxctl connect` のクライアントからの接続待ち受け。既定では開放しない。開放するときの追加方法は「別の PC から操作する」 |
| 6454 | UDP | Art-Net の送信先。照明機材や Art-Net ノードへ向けて出力する |

## 更新する

新しい版を出すときは、イメージを新しいタグで作成して置き、`deploy/kubernetes/deployment.yaml` の `image:` を書き換えます。Argo CD の場合は、Infra に変更を入れます。Recreate で入れ替わるため、入れ替えの間は一時的に出力が停止します。

## まだ作っていないこと

- 現在の Kubernetes のマニフェストは、Infra のレジストリのイメージを指しています。GHCR に置いたイメージを利用するには、`deployment.yaml` の `image:` を `ghcr.io/technotut/dmxctl` に書き換えます。
- プルリクエストでは、イメージを作成して `dmxctl help` が動作するかだけを確認します。送信はしません。
- `main` へ push すると、GHCR に `ghcr.io/technotut/dmxctl:main` と、`sha-` の後に commit の短い番号を付けたタグで送信します。
- タグ `v*` を push すると、`0.2.0` のようなタグで GHCR に送信します。
- 照明の VLAN lbar へ直接出力する設定。必要なときは、Infra で VLAN 30 のブリッジと NAD を作成してから、`deployment.yaml` の注釈を外します。
- 実機の照明への出力は、まだ確認していません。Infra の qlcplus も通常の Pod のネットワークで Art-Net を送信しているため、同じ方法で届く見込みです。

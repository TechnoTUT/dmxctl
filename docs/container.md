# コンテナで動かす

`dmxctl serve` を、コンテナと Kubernetes で動かす手順です。Podman でも Docker でも作れます。画面は使わず、自動操作・Art-Net の出力・操作パネルだけを動かします。

## できることと、できないこと

できること
- Art-Net で照明機材へ値を送る
- ブラウザの操作パネルを開く。Service では TCP の 80 番で受ける
- OSC を受ける。UDP の 9000 番で受ける
- `ctl` コマンドで様子を見る・操作する

できないこと・注意すること
- **NDI は使えません。** NDI のライブラリは配布条件のため、イメージに入れていません。
- **USB-DMX は使いません。** このイメージは Art-Net の出力だけを想定しています。
- **MIDI と音の入力は使いません。** 入力は OSC と操作パネルだけです。
- **Ableton Link と PRO DJ LINK は届きません。** これらは同じ L2 のネットワークが要ります。Pod の標準のネットワークでは届きません。照明の VLAN lbar へ直接出す設定は、Infra 側で VLAN 30 のブリッジを作ってから行います。

## ファイル

| ファイル | 役割 |
|---|---|
| `Containerfile` | ビルドと実行のイメージ。実行するのは `dmxctl serve` だけ |
| `.dockerignore` | ビルドに送らないもの。`target/` などは数 GB あるため除く |
| `deploy/kubernetes/` | Kubernetes のマニフェスト。`kubectl apply -k` で置ける |
| `deploy/kubernetes/argocd-application.example.yaml` | Infra の Argo CD に登録する Application の例 |

## イメージを作る

ビルドには、サブモジュール `moip-engine/` が要ります。先に取ってから作ります。

```
git submodule update --init --recursive
podman build -t dmxctl:0.1.0 -f Containerfile .
```

Docker なら `docker build -t dmxctl:0.1.0 -f Containerfile .` で同じです。初回は、Rust のビルドに数十分かかることがあります。2回目以降は、キャッシュで速くなります。

## 手元で試す

本番の設定には触れず、コピーした設定で試します。

```
mkdir -p data && cp examples/cli/project.json data/
podman run --rm --name dmxctl -p 8080:8080 -v "$PWD/data:/data:Z" dmxctl:0.1.0
```

別の端末から様子を見ます。

```
podman exec dmxctl dmxctl ctl status
```

イメージの既定は、実機に出さず、自動保存もしない安全な設定です。実機へ出すときは、次の「実機に出す」を見てください。

## Kubernetes に置く

前提は、[Infra](https://github.com/TechnoTUT/Infra) の構成です。設定の保存に `nfs-client`、イメージの置き場に `registry.svc.technotut.net`、固定 IP に LoadBalancer の範囲と `lbipam.technotut.net/static` のラベルを使います。

### 1. イメージを置く

```
podman tag dmxctl:0.1.0 registry.svc.technotut.net/dmxctl:0.1.0
podman push registry.svc.technotut.net/dmxctl:0.1.0
```

### 2. マニフェストを置く

名前空間を作り、一式を適用します。

```
kubectl create namespace dmxctl
kubectl apply -k deploy/kubernetes
```

Argo CD で管理するときは、`deploy/kubernetes/` を Infra の `k8s/manifest/dmxctl/` にコピーし、`argocd-application.example.yaml` を `k8s/manifest/apps/dmxctl.yaml` として置きます。

### 3. 設定を入れる

設定の `project.json` は、Secret の `dmxctl-seed` に入れます。Pod の起動時に、PVC の `/data` に設定が無いときだけ、initContainer が Secret からコピーします。PVC にすでに設定があれば何もしないので、GUI で変えた内容は消えません。

Secret は、手元の設定から作ります。設定には合言葉などが入るため、Git には入れません。

```
kubectl -n dmxctl create secret generic dmxctl-seed --from-file=project.json=$HOME/.config/dmxctl/project.json
```

Secret を作ったあとで Pod を起動します。`kubectl -n dmxctl rollout restart deploy/dmxctl` で、すぐ起動し直せます。

注意点は次のとおりです。

- 作るのは、初回だけで十分です。Secret を作り直しても、PVC に設定があれば入れ替わりません。
- 設定を入れ直したいときは、PVC の `project.json` を消してから Pod を起動し直します。消す前に、必ずバックアップを取ります。

### 4. 様子を見る

```
kubectl -n dmxctl logs deploy/dmxctl
kubectl -n dmxctl exec deploy/dmxctl -- dmxctl ctl status
```

ログに `"ready":true` が出れば、起動できています。

## 実機に出す

マニフェストの `args` で、既定は `--output none` です。Art-Net を実機へ送るときは、設定の出力先、つまり `project.json` の `outputs` を確かめてから、次のように書き換えて適用します。

```yaml
        - --output
        - live
```

`--output live` にすると、設定の出力先へ Art-Net を送ります。最初は `none` のまま、`ctl` で最終の DMX 値を確かめることをおすすめします。

## 安全のための決まり

- **ARM は起動のたびに解除されます。** 保存もされず、CLI からは ARM にできません。
- **止めるとき、安全な値を送ってから止まります。** Kubernetes は SIGTERM を送ります。`terminationGracePeriodSeconds` を 10 秒にしているのは、この安全な値を送る時間を待つためです。
- **Pod は 1 つだけ動かします。** 2つ動くと、同じ Art-Net の値が交互に届きます。
- **操作パネルを有効にするときは PIN を付けます。** 操作パネルは LAN に開きます。`project.json` の `web_pin` が空のままだと、LAN のだれでも操作できます。
- **制御の窓口は、コンテナの中だけに開きます。** 外からは届きません。`ctl` は `kubectl exec` で使います。

## ポート

| 番号 | 種類 | 用途 |
|---|---|---|
| Service の 80 → Pod の 8080 | TCP | 操作パネル。番号は `project.json` の `web_port` |
| 9000 | UDP | OSC の受信。番号は `project.json` の `osc_port` |
| 9190 | TCP | 制御の窓口。コンテナの中だけ。`ctl` の既定の接続先 |
| 9300 | TCP | `dmxctl connect` のクライアントの待ち受け。認証は無いので、LAN の外には出さない |
| 6454 | UDP | Art-Net の送信先。照明機材や Art-Net ノードへ向けて出す |

## 更新する

新しい版を出すときは、イメージを新しいタグで作って置き、`deploy/kubernetes/deployment.yaml` の `image:` を書き換えます。Argo CD の場合は、Infra に変更を入れます。Recreate で入れ替わるので、入れ替えの間は一時的に出力が止まります。

## まだ作っていないこと

- イメージを GitHub Actions で作って GHCR に置く仕組み。今は手元で作って、Infra のレジストリに置きます。
- 照明の VLAN lbar へ直接出す設定。必要なときは、Infra で VLAN 30 のブリッジと NAD を作ってから、`deployment.yaml` の注釈を外します。
- 実機の照明への出力は、まだ確かめていません。Infra の qlcplus も通常の Pod のネットワークで Art-Net を送っているので、同じ方法で届く見込みです。

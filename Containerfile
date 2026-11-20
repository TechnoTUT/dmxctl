# dmxctl のコンテナイメージ。画面を使わず、`dmxctl serve` だけを動かす。自動操作・Art-Net 出力・操作パネルが動く。
# 作り方。リポジトリの直下で次を実行する:  podman build -t dmxctl:0.1.0 -f Containerfile .
# 動かし方と Kubernetes の例は docs/container.md と deploy/kubernetes/ を見る。
#
# NDI のライブラリは配布条件のため入れない。利用者が `dmxctl-fetch-ndi` で NDI の公式サイトから取得すると、NDI の映像・音の取り込みが使える。
# 取得の手順は docs/container.md の「NDI を使う」を見る。

# ---- 1. ビルド ----
FROM docker.io/library/rust:1-trixie AS build

# 画面を持たない dmxctl-server をビルドするのに要るもの。音と MIDI の ALSA と、シリアルの udev の開発用ファイル、操作パネルをビルドする Node.js と npm
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential pkg-config \
      libasound2-dev libudev-dev \
      nodejs npm \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
# moip-engine は NDI のクレートを入れたサブモジュール。ビルドの前に `git submodule update --init --recursive` で取っておく
COPY . .

# キャッシュを使い、2回目以降のビルドを速くする。取り出すのは release の dmxctl-server 1つだけ。
# 呼び出しは `dmxctl serve` のまま使えるよう、イメージの中では dmxctl の名前で入れる
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    DMXCTL_REQUIRE_WEB=1 cargo build --release --locked -p dmxctl-server \
 && cp target/release/dmxctl-server /dmxctl

# ---- 2. 実行 ----
FROM docker.io/library/debian:trixie-slim

# 実行に要る共有ライブラリ。ALSA は音と MIDI の土台で、udev とともに起動時に読み込まれるので入れておく。
# Avahi は後から取得する NDI のライブラリが必須とするもの、curl は `dmxctl-fetch-ndi` がダウンロードに利用するもの
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libasound2t64 libudev1 libstdc++6 libcap2 tzdata \
      libavahi-client3 libavahi-common3 curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# 権限を落として動かす。UID 10001 は Infra の plmux と同じ数字にしている
RUN groupadd --gid 10001 dmxctl \
 && useradd --uid 10001 --gid dmxctl --home-dir /data --no-create-home --shell /usr/sbin/nologin dmxctl \
 && mkdir /data \
 && chown dmxctl:dmxctl /data

COPY --from=build /dmxctl /usr/local/bin/dmxctl
COPY deploy/container/fetch-ndi.sh /usr/local/bin/dmxctl-fetch-ndi

# NDI のライブラリは /data/ndi から読み込む。無ければ NDI を使わないだけで、起動は妨げない
ENV TZ=Asia/Tokyo \
    HOME=/data \
    DMXCTL_NDI_DIR=/data/ndi \
    DMXCTL_NDI_LIB=/data/ndi

USER 10001:10001
WORKDIR /data

# 操作パネルは TCP、OSC の受信は UDP。実際の番号は project.json の web_port と osc_port で決まる
EXPOSE 8080/tcp 9000/udp

# 既定は安全側: 実機に出さず、自動保存も入力もしない。
# 本番では Kubernetes の args で、--output live・--inputs configured・--autosave を明示して上書きする。
# 制御の窓口は、このコンテナの中の loopback だけで開く。外からは、操作パネルと OSC だけが届く。操作パネルを使うときは project.json の web_pin に PIN を付ける
ENTRYPOINT ["/usr/local/bin/dmxctl"]
CMD ["serve", "--project", "/data/project.json", "--listen", "127.0.0.1:9190"]

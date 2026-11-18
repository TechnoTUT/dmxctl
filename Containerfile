# dmxctl のコンテナイメージ。画面を使わず、`dmxctl serve` だけを動かす。自動操作・Art-Net 出力・操作パネルが動く。
# 作り方。リポジトリの直下で次を実行する:  podman build -t dmxctl:0.1.0 -f Containerfile .
# 動かし方と Kubernetes の例は docs/container.md と deploy/kubernetes/ を見る。
#
# NDI のライブラリは配布条件のため入れない。NDI の映像・音の取り込みは、このイメージでは使えない。

# ---- 1. ビルド ----
FROM docker.io/library/rust:1-trixie AS build

# SDL2 と Ableton Link に要るもの。SDL2 は内蔵のソースからビルドし、Link は C++ で書かれている。
# Linux で dmxctl をビルドするときに要るものと同じ一覧
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential pkg-config cmake \
      libasound2-dev libudev-dev libdrm-dev libgbm-dev libegl-dev libgles-dev libxext-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
# moip-engine は NDI のクレートを入れたサブモジュール。ビルドの前に `git submodule update --init --recursive` で取っておく
COPY . .

# キャッシュを使い、2回目以降のビルドを速くする。取り出すのは release の dmxctl 1つだけ
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin dmxctl \
 && cp target/release/dmxctl /dmxctl

# ---- 2. 実行 ----
FROM docker.io/library/debian:trixie-slim

# 実行に要る共有ライブラリ。ALSA は音と MIDI の土台で、udev とともに起動時に読み込まれるので入れておく
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libasound2t64 libudev1 libstdc++6 libcap2 tzdata \
 && rm -rf /var/lib/apt/lists/*

# 権限を落として動かす。UID 10001 は Infra の plmux と同じ数字にしている
RUN groupadd --gid 10001 dmxctl \
 && useradd --uid 10001 --gid dmxctl --home-dir /data --no-create-home --shell /usr/sbin/nologin dmxctl \
 && mkdir /data \
 && chown dmxctl:dmxctl /data

COPY --from=build /dmxctl /usr/local/bin/dmxctl

ENV TZ=Asia/Tokyo \
    HOME=/data

USER 10001:10001
WORKDIR /data

# 操作パネルは TCP、OSC の受信は UDP。実際の番号は project.json の web_port と osc_port で決まる
EXPOSE 8080/tcp 9000/udp

# 既定は安全側: 実機に出さず、自動保存も入力もしない。
# 本番では Kubernetes の args で、--output live・--inputs configured・--autosave を明示して上書きする。
# 制御の窓口は、このコンテナの中の loopback だけで開く。外からは、操作パネルと OSC だけが届く。操作パネルを使うときは project.json の web_pin に PIN を付ける
ENTRYPOINT ["/usr/local/bin/dmxctl"]
CMD ["serve", "--project", "/data/project.json", "--listen", "127.0.0.1:9190"]

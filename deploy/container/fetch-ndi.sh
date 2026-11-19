#!/bin/sh
# Download the NDI runtime (libndi.so.6) for dmxctl from NDI's official site.
# The library is not bundled in the image because of its license.
# You must read and accept the NDI SDK License Agreement yourself, then pass --accept-license
# or set NDI_ACCEPT_LICENSE=yes. This script only automates what the official installer does.
#
# usage: dmxctl-fetch-ndi [--accept-license] [--dir DIR]
#   DIR defaults to $DMXCTL_NDI_DIR, then /data/ndi. dmxctl looks there at start-up.
set -eu

URL="${NDI_SDK_URL:-https://downloads.ndi.tv/SDK/NDI_SDK_Linux/Install_NDI_SDK_v6_Linux.tar.gz}"
DIR="${DMXCTL_NDI_DIR:-/data/ndi}"
ACCEPT="${NDI_ACCEPT_LICENSE:-no}"

while [ $# -gt 0 ]; do
  case "$1" in
    --accept-license) ACCEPT=yes ;;
    --dir) shift; DIR="${1:?--dir needs a value}" ;;
    -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

if [ -e "$DIR/libndi.so.6" ]; then
  echo "NDI runtime already present: $DIR/libndi.so.6"
  exit 0
fi

if [ "$ACCEPT" != yes ]; then
  cat >&2 <<MSG
The NDI runtime is downloaded from NDI and is covered by the NDI SDK License Agreement.
Read it first: $URL (the installer shows it), or https://ndi.video/for-developers/ndi-sdk/
If you agree, run again with --accept-license (or NDI_ACCEPT_LICENSE=yes).
MSG
  exit 1
fi

case "$(uname -m)" in
  x86_64)        ARCH=x86_64-linux-gnu ;;
  aarch64|arm64) ARCH=aarch64-rpi4-linux-gnueabi ;;
  *) echo "unsupported CPU: $(uname -m)" >&2; exit 1 ;;
esac

mkdir -p "$DIR"
WORK="$(mktemp -d "$DIR/.fetch.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

echo "downloading $URL"
curl -fsSL --retry 3 -o "$WORK/sdk.tar.gz" "$URL"
tar -xzf "$WORK/sdk.tar.gz" -C "$WORK" Install_NDI_SDK_v6_Linux.sh

# The installer is a shell header followed by a tar.gz payload. Extract only the library, without running it.
START="$(awk '/^__NDI_ARCHIVE_BEGIN__/ { print NR+1; exit 0 }' "$WORK/Install_NDI_SDK_v6_Linux.sh")"
[ -n "$START" ] || { echo "unexpected installer format" >&2; exit 1; }
tail -n "+$START" "$WORK/Install_NDI_SDK_v6_Linux.sh" \
  | tar -xz -C "$WORK" --wildcards "NDI SDK for Linux/lib/$ARCH/libndi.so.*" "NDI SDK for Linux/licenses/*"

LIB="$(find "$WORK/NDI SDK for Linux/lib/$ARCH" -name 'libndi.so.*.*.*' | head -n 1)"
[ -n "$LIB" ] || { echo "libndi not found in the SDK" >&2; exit 1; }
cp "$LIB" "$DIR/$(basename "$LIB")"
ln -sf "$(basename "$LIB")" "$DIR/libndi.so.6"
rm -rf "$DIR/licenses"
cp -r "$WORK/NDI SDK for Linux/licenses" "$DIR/licenses"
echo "installed: $DIR/libndi.so.6 ($(basename "$LIB"))"

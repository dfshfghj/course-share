#!/usr/bin/env bash
set -euo pipefail

# Build a minimal FFmpeg binary and place it in
# vendor/ffmpeg-minimal/<target-triple>/bin/ffmpeg. build.rs copies it next to
# the course-share executable, so `cargo run` does not need a system ffmpeg.
#
# Usage:
#   scripts/build-ffmpeg-minimal.sh [--force]
#
# Environment:
#   FFMPEG_VERSION        release to build (default: 8.0)
#   FFMPEG_MIRROR         download base URL (default: https://ffmpeg.org/releases)
#   FFMPEG_BUILD_DIR      scratch directory (default: /tmp/course/ffmpeg-build)
#   FFMPEG_TARGET         Rust target triple (default: rustc host)
#   FFMPEG_STATIC         auto/1/0 (default: auto; static on Linux/Windows, off on macOS)
#   FFMPEG_CONFIGURE_ARGS extra configure arguments, e.g. cross-compilation flags

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${FFMPEG_VERSION:-8.0}"
MIRROR="${FFMPEG_MIRROR:-https://ffmpeg.org/releases}"
BUILD_DIR="${FFMPEG_BUILD_DIR:-/tmp/course/ffmpeg-build}"
STATIC="${FFMPEG_STATIC:-auto}"
FORCE=0
[ "${1:-}" = "--force" ] && FORCE=1

if [ "$STATIC" = "auto" ]; then
  case "$(uname -s)" in
    Darwin) STATIC=0 ;;
    *) STATIC=1 ;;
  esac
fi

TARGET="${FFMPEG_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}"
EXE="ffmpeg"
case "$TARGET" in
  *windows*) EXE="ffmpeg.exe" ;;
esac
DEST="$ROOT/vendor/ffmpeg-minimal/$TARGET"
BIN="$DEST/bin/$EXE"

if [ -x "$BIN" ] && [ "$FORCE" -ne 1 ]; then
  echo "minimal ffmpeg already present: $BIN"
  case "$TARGET" in
    *windows*) ;;
    *) "$BIN" -version | sed -n "1,2p" ;;
  esac
  exit 0
fi

mkdir -p "$BUILD_DIR"
SRC_DIR="$BUILD_DIR/ffmpeg-$VERSION"
TARBALL="$BUILD_DIR/ffmpeg-$VERSION.tar.xz"

if [ ! -d "$SRC_DIR" ]; then
  if [ ! -f "$TARBALL" ]; then
    echo "downloading FFmpeg $VERSION from $MIRROR"
    curl -fL --retry 3 -o "$TARBALL" "$MIRROR/ffmpeg-$VERSION.tar.xz"
  fi
  tar -xJf "$TARBALL" -C "$BUILD_DIR"
fi

rm -rf "$DEST"
mkdir -p "$DEST"

EXTRA=()
if [ "$STATIC" = "1" ]; then
  EXTRA+=(--extra-ldflags=-static)
fi
if [ -n "${FFMPEG_EXTRA_LDFLAGS:-}" ]; then
  EXTRA+=(--extra-ldflags="$FFMPEG_EXTRA_LDFLAGS")
fi
if [ -n "${FFMPEG_CONFIGURE_ARGS:-}" ]; then
  read -r -a CONFIGURE_ARGS <<< "$FFMPEG_CONFIGURE_ARGS"
  EXTRA+=("${CONFIGURE_ARGS[@]}")
fi

JOBS="$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 2)"

echo "configuring minimal FFmpeg $VERSION for $TARGET"
(
  cd "$SRC_DIR"
  bash "$ROOT/scripts/configure-ffmpeg-minimal.sh" --prefix="$DEST" ${EXTRA[@]+"${EXTRA[@]}"}
)

echo "building minimal FFmpeg (jobs: $JOBS)"
make -C "$SRC_DIR" -j"$JOBS"
make -C "$SRC_DIR" install

for license in "$SRC_DIR"/COPYING*; do
  [ -f "$license" ] && cp "$license" "$DEST/"
done

echo
echo "built: $BIN"
ls -lh "$BIN"
case "$TARGET" in
  *windows*) ;;
  *) "$BIN" -version | sed -n "1,3p" ;;
esac

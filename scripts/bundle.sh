#!/usr/bin/env bash
# Builds dist/DSLocal.app from the cargo binary plus assets/icon.icns.
# Usage: scripts/bundle.sh [--debug]
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

APP_NAME="DSLocal"
BIN_NAME="dslocal"
PROFILE="release"
PROFILE_FLAG="--release"

DIST_DIR="$ROOT_DIR/dist"
APP_DIR="$DIST_DIR/$APP_NAME.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
PLIST_SRC="$ROOT_DIR/scripts/Info.plist"
ICON_SRC="$ROOT_DIR/assets/icon.icns"
# The toolbar's switch-icon button loads these at runtime.
ICON_SET_DIR="$ROOT_DIR/assets/icon-candidates"

usage() {
    echo "usage: $0 [--debug|--release]" >&2
}

for arg in "$@"; do
    case "$arg" in
        --debug)
            PROFILE="debug"
            PROFILE_FLAG=""
            ;;
        --release)
            PROFILE="release"
            PROFILE_FLAG="--release"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "[bundle] unknown argument: $arg" >&2
            usage
            exit 2
            ;;
    esac
done

if [ ! -f "$PLIST_SRC" ]; then
    echo "[bundle] missing $PLIST_SRC" >&2
    exit 1
fi
if [ ! -f "$ICON_SRC" ]; then
    echo "[bundle] missing $ICON_SRC" >&2
    exit 1
fi

echo "[bundle] profile: $PROFILE"
if [ -n "$PROFILE_FLAG" ]; then
    cargo build "$PROFILE_FLAG"
else
    cargo build
fi

BIN_SRC="$ROOT_DIR/target/$PROFILE/$BIN_NAME"
if [ ! -x "$BIN_SRC" ]; then
    echo "[bundle] missing binary: $BIN_SRC" >&2
    exit 1
fi

rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp "$BIN_SRC" "$MACOS_DIR/$BIN_NAME"
chmod 755 "$MACOS_DIR/$BIN_NAME"
cp "$ICON_SRC" "$RESOURCES_DIR/icon.icns"
mkdir -p "$RESOURCES_DIR/icons"
cp "$ICON_SET_DIR"/*.png "$RESOURCES_DIR/icons/"
ls "$RESOURCES_DIR/icons"/*.png >/dev/null
cp "$PLIST_SRC" "$CONTENTS_DIR/Info.plist"
printf 'APPL????' > "$CONTENTS_DIR/PkgInfo"

for f in "$MACOS_DIR/$BIN_NAME" "$RESOURCES_DIR/icon.icns" "$CONTENTS_DIR/Info.plist" "$CONTENTS_DIR/PkgInfo"; do
    if [ ! -f "$f" ]; then
        echo "[bundle] missing bundle file: $f" >&2
        exit 1
    fi
done

if command -v codesign >/dev/null 2>&1; then
    if codesign --force --deep --sign - "$APP_DIR" 2>/dev/null; then
        echo "[bundle] codesign: adhoc ok"
    else
        echo "[bundle] WARNING: codesign failed, app left unsigned (not fatal)" >&2
    fi
else
    echo "[bundle] WARNING: codesign not found, app left unsigned" >&2
fi

echo "[bundle] artifact: $APP_DIR"
du -sh "$APP_DIR"
ls -lh "$MACOS_DIR/$BIN_NAME" "$RESOURCES_DIR/icon.icns" "$CONTENTS_DIR/Info.plist" "$CONTENTS_DIR/PkgInfo" | awk '{printf "  %-7s %s\n", $5, $9}'

echo "[bundle] plutil -lint:"
if plutil -lint "$CONTENTS_DIR/Info.plist"; then
    echo "[bundle] Info.plist OK"
else
    echo "[bundle] WARNING: Info.plist failed plutil -lint" >&2
fi

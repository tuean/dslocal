#!/usr/bin/env bash
# Packages the macOS app as a .dmg (drag to /Applications) and a .zip.
#
# Usage:
#   scripts/package-macos.sh              # universal (arm64 + x86_64) build
#   scripts/package-macos.sh --universal  # same
#   scripts/package-macos.sh --arch       # host arch only (fast, local smoke test)
#
# Output: dist/DSLocal-<version>-macos-<arch>.dmg and .zip
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

APP_NAME="DSLocal"
DIST_DIR="$ROOT_DIR/dist"
APP_DIR="$DIST_DIR/$APP_NAME.app"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
MODE="universal"

for arg in "$@"; do
    case "$arg" in
        --universal) MODE="universal" ;;
        --arch|--host) MODE="host" ;;
        -h|--help)
            sed -n '2,10p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "[package] unknown argument: $arg" >&2
            exit 2
            ;;
    esac
done

if [ -z "$VERSION" ]; then
    echo "[package] could not read version from Cargo.toml" >&2
    exit 1
fi

mkdir -p "$DIST_DIR"

if [ "$MODE" = "universal" ]; then
    for triple in aarch64-apple-darwin x86_64-apple-darwin; do
        if ! rustup target list --installed | grep -qx "$triple"; then
            echo "[package] installing rust target $triple"
            rustup target add "$triple"
        fi
    done
    echo "[package] building arm64 + x86_64, then lipo"
    DSLOCAL_TARGET=aarch64-apple-darwin \
        DSLOCAL_APP_DIR="$DIST_DIR/_arm64/$APP_NAME.app" ./scripts/bundle.sh --release
    DSLOCAL_TARGET=x86_64-apple-darwin \
        DSLOCAL_APP_DIR="$DIST_DIR/_x64/$APP_NAME.app" ./scripts/bundle.sh --release

    rm -rf "$APP_DIR"
    cp -R "$DIST_DIR/_arm64/$APP_NAME.app" "$APP_DIR"
    lipo -create \
        "$DIST_DIR/_arm64/$APP_NAME.app/Contents/MacOS/dslocal" \
        "$DIST_DIR/_x64/$APP_NAME.app/Contents/MacOS/dslocal" \
        -output "$APP_DIR/Contents/MacOS/dslocal"
    chmod 755 "$APP_DIR/Contents/MacOS/dslocal"
    rm -rf "$DIST_DIR/_arm64" "$DIST_DIR/_x64"
    codesign --force --deep --sign - "$APP_DIR"
    ARCH_LABEL="universal"
else
    ./scripts/bundle.sh --release
    ARCH_LABEL="$(uname -m)"
    if [ "$ARCH_LABEL" = "x86_64" ]; then ARCH_LABEL="x64"; fi
    if [ "$ARCH_LABEL" = "aarch64" ]; then ARCH_LABEL="arm64"; fi
fi

ARCHS="$(lipo -archs "$APP_DIR/Contents/MacOS/dslocal")"
echo "[package] $APP_NAME.app architectures: $ARCHS"

BASENAME="$APP_NAME-$VERSION-macos-$ARCH_LABEL"
DMG="$DIST_DIR/$BASENAME.dmg"
ZIP="$DIST_DIR/$BASENAME.zip"

# --- dmg: app + /Applications symlink, compressed ---------------------------
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP_DIR" "$STAGE/$APP_NAME.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO -quiet "$DMG"
rm -rf "$STAGE"

# --- zip: same bundle, for people who prefer a plain archive ----------------
rm -f "$ZIP"
ditto -c -k --keepParent "$APP_DIR" "$ZIP"

echo "[package] artifacts:"
ls -lh "$DMG" "$ZIP" | awk '{printf "  %-7s %s\n", $5, $9}'
shasum -a 256 "$DMG" "$ZIP" | sed 's/^/  /'

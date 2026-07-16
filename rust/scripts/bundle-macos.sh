#!/usr/bin/env bash
# Assemble a distributable macOS .app bundle for the StormForge native app.
#
# Steps: release build -> .app skeleton -> Info.plist -> .icns from the Electron
# build icon -> ad-hoc codesign -> zip into rust/dist/.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(dirname "$SCRIPT_DIR")"
REPO_DIR="$(dirname "$RUST_DIR")"

APP_NAME="StormForge Native"
BUNDLE_ID="com.haribote1110.stormforge-native"
BINARY_NAME="stormforge-native"
ICON_SOURCE="$REPO_DIR/build/icon.png"

# Version is read from the native crate's manifest so it has a single source of truth.
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$RUST_DIR/apps/native/Cargo.toml" | head -1)"

DIST_DIR="$RUST_DIR/dist"
APP_DIR="$DIST_DIR/$APP_NAME.app"

echo "==> Building release binary (v$VERSION)"
cargo build --release -p stormforge-native --manifest-path "$RUST_DIR/Cargo.toml"

echo "==> Assembling $APP_NAME.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$RUST_DIR/target/release/$BINARY_NAME" "$APP_DIR/Contents/MacOS/$BINARY_NAME"

cat > "$APP_DIR/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleExecutable</key>
    <string>$BINARY_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
</dict>
</plist>
PLIST

if [[ -f "$ICON_SOURCE" ]]; then
    echo "==> Converting icon"
    ICONSET_DIR="$(mktemp -d)/AppIcon.iconset"
    mkdir -p "$ICONSET_DIR"
    for size in 16 32 128 256 512; do
        sips -z "$size" "$size" "$ICON_SOURCE" --out "$ICONSET_DIR/icon_${size}x${size}.png" >/dev/null
        double=$((size * 2))
        sips -z "$double" "$double" "$ICON_SOURCE" --out "$ICONSET_DIR/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "$ICONSET_DIR" -o "$APP_DIR/Contents/Resources/AppIcon.icns"
    rm -rf "$(dirname "$ICONSET_DIR")"
else
    echo "==> WARNING: icon source not found at $ICON_SOURCE — bundling without an icon"
fi

echo "==> Ad-hoc signing"
codesign --force --deep -s - "$APP_DIR"

ZIP_PATH="$DIST_DIR/$APP_NAME-$VERSION-macos.zip"
echo "==> Zipping to $ZIP_PATH"
rm -f "$ZIP_PATH"
(cd "$DIST_DIR" && ditto -c -k --keepParent "$APP_NAME.app" "$(basename "$ZIP_PATH")")

echo "==> Done"
ls -lh "$ZIP_PATH"

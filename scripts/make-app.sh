#!/usr/bin/env bash
# Build a double-clickable vizz.app bundle (macOS only) into ./dist,
# with Syphon.framework embedded so users install nothing else.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "$(uname)" != "Darwin" ]; then
    echo "error: app bundles can only be built on macOS" >&2
    exit 1
fi

version=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
echo "Building vizz $version (release)..."
cargo build --release

app="dist/vizz.app"
rm -rf dist
mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources"

cp target/release/vizz "$app/Contents/MacOS/vizz"

if [ ! -d vendor/Syphon.framework ]; then
    ./scripts/fetch-syphon.sh vendor
fi
cp -R vendor/Syphon.framework "$app/Contents/Frameworks/Syphon.framework"

# The icon. Derived here from the committed 1024 master rather than
# committing ten PNGs: sips and iconutil are both part of macOS, and an
# .icns is the one format the Dock will read.
#
# Non-fatal on purpose. A bundle without an icon is ugly; a release that
# did not build because of one is worse.
icon_src="assets/icon-1024.png"
if [ -f "$icon_src" ] && command -v iconutil >/dev/null; then
    iconset="$(mktemp -d)/vizz.iconset"
    mkdir -p "$iconset"
    # Each size twice, at 1x and at 2x of the size below it — that pairing
    # is what iconutil expects, and a missing member fails the whole set.
    for size in 16 32 128 256 512; do
        sips -s format png -z "$size" "$size" "$icon_src" \
            --out "$iconset/icon_${size}x${size}.png" >/dev/null
        sips -s format png -z "$((size * 2))" "$((size * 2))" "$icon_src" \
            --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
    done
    if iconutil -c icns "$iconset" -o "$app/Contents/Resources/vizz.icns"; then
        echo "Built vizz.icns from $icon_src"
    else
        echo "warning: iconutil failed — bundling without an icon" >&2
    fi
    rm -rf "$(dirname "$iconset")"
else
    echo "warning: no $icon_src or no iconutil — bundling without an icon" >&2
fi

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>vizz</string>
    <key>CFBundleDisplayName</key><string>vizz</string>
    <key>CFBundleIdentifier</key><string>com.colmhewson.vizz</string>
    <key>CFBundleExecutable</key><string>vizz</string>
    <!-- Names Resources/vizz.icns. Harmless if the icon step above was
         skipped: the Dock falls back to the generic app icon rather than
         refusing to launch. -->
    <key>CFBundleIconFile</key><string>vizz</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
    <!-- Required. A bundled app that reaches the microphone without a
         usage description is terminated by TCC, not prompted — so without
         this key vizz.app dies at launch as soon as audio input opens.
         A CLI run does not show it, because it inherits Terminal's grant. -->
    <key>NSMicrophoneUsageDescription</key><string>vizz analyses audio input to drive visuals from the music.</string>
</dict>
</plist>
EOF

# Sign with a Developer ID when one is available, ad-hoc otherwise, so a
# local build still works for anyone without the certificate. Frameworks
# first: codesign seals what it finds, so signing the app before its
# embedded frameworks produces a bundle that fails verification.
entitlements="$(dirname "$0")/vizz.entitlements"
if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "Signing with $APPLE_SIGNING_IDENTITY (hardened runtime)"
  # --options runtime is what notarization requires; --timestamp is what
  # keeps the signature valid after the certificate expires.
  codesign --force --timestamp --options runtime \
    --sign "$APPLE_SIGNING_IDENTITY" \
    "$app/Contents/Frameworks/Syphon.framework"
  codesign --force --timestamp --options runtime \
    --entitlements "$entitlements" \
    --sign "$APPLE_SIGNING_IDENTITY" "$app"
  codesign --verify --deep --strict --verbose=2 "$app"
else
  echo "No APPLE_SIGNING_IDENTITY — ad-hoc signing (first launch needs right-click → Open)"
  # Ad-hoc is still required: arm64 binaries will not run unsigned at all.
  codesign --force -s - "$app/Contents/Frameworks/Syphon.framework"
  codesign --force -s - "$app"
fi

# ditto preserves the framework's symlink structure, unlike plain zip -r.
ditto -c -k --keepParent "$app" dist/vizz.app.zip
echo "Built $app and dist/vizz.app.zip"

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
# Missing tooling is a warning — someone building on a stripped-down
# machine should still get a working app. A conversion that *fails* is an
# error, because a step that shrugs and carries on is a step that goes
# wrong silently and ships an iconless bundle through a green CI run.
icon_src="assets/icon-1024.png"
if [ -f "$icon_src" ] && command -v iconutil >/dev/null && command -v sips >/dev/null; then
    iconset_dir="$(mktemp -d)"
    iconset="$iconset_dir/vizz.iconset"
    mkdir -p "$iconset"
    # Each size twice, at 1x and at 2x — that pairing is what iconutil
    # expects, and a missing member fails the whole set.
    for size in 16 32 128 256 512; do
        sips -s format png -z "$size" "$size" "$icon_src" \
            --out "$iconset/icon_${size}x${size}.png" >/dev/null
        sips -s format png -z "$((size * 2))" "$((size * 2))" "$icon_src" \
            --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "$iconset" -o "$app/Contents/Resources/vizz.icns"
    rm -rf "$iconset_dir"
    # iconutil can exit 0 having written nothing useful, so check the
    # artifact rather than the status.
    if [ ! -s "$app/Contents/Resources/vizz.icns" ]; then
        echo "error: iconutil produced no vizz.icns" >&2
        exit 1
    fi
    echo "Built vizz.icns from $icon_src"
else
    echo "warning: no $icon_src, sips or iconutil — bundling without an icon" >&2
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
    <!-- Same rule as the microphone, and it bit exactly the same way: a
         bundled app that opens a capture device without a usage
         description gets no frames from TCC. The session starts, the
         camera light may even come on, and every frame is withheld — so
         it reads as "no frames arriving" rather than as a refusal, which
         sends you looking at the camera instead of at this file. A CLI
         run does not show it, because it inherits Terminal's grant. -->
    <key>NSCameraUsageDescription</key><string>vizz can turn a camera or capture card into live visuals.</string>
    <!-- Receiving from a phone, tablet or another Mac on the same wifi
         is local network access, which macOS 15 gates the same way it
         gates the camera. The failure looks like nothing at all: the
         listener binds, the sender reports it is sending, and no bytes
         cross. Same lesson as NSCameraUsageDescription above, on a
         different permission. -->
    <key>NSLocalNetworkUsageDescription</key><string>vizz receives live point clouds and video from apps on your local network.</string>
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

# The zip carries the version; the bundle inside does not. A Downloads
# folder of "vizz.app.zip", "vizz.app-1.zip", "vizz.app-2.zip" cannot be
# told apart without unzipping each one and reading Get Info — while the
# .app itself must keep its plain name, because that is the application's
# identity in /Applications, in the Dock and as the Syphon server.
#
# ditto preserves the framework's symlink structure, unlike plain zip -r.
zip="dist/vizz-$version.app.zip"
ditto -c -k --keepParent "$app" "$zip"
echo "Built $app and $zip"

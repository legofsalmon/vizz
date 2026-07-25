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

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>vizz</string>
    <key>CFBundleDisplayName</key><string>vizz</string>
    <key>CFBundleIdentifier</key><string>com.colmhewson.vizz</string>
    <key>CFBundleExecutable</key><string>vizz</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
</dict>
</plist>
EOF

# Ad-hoc signature: required for arm64 binaries to run at all. A paid
# Developer ID + notarization would remove the right-click-to-open step;
# until then this is the standard unsigned-open-source story.
codesign --force -s - "$app/Contents/Frameworks/Syphon.framework"
codesign --force -s - "$app"

# ditto preserves the framework's symlink structure, unlike plain zip -r.
ditto -c -k --keepParent "$app" dist/vizz.app.zip
echo "Built $app and dist/vizz.app.zip"

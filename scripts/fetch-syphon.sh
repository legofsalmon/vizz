#!/usr/bin/env bash
# Provide Syphon.framework in ./vendor (or $1) for vizz's runtime loader.
#
# The official prebuilt releases (Syphon SDK 5) are x86_64-only — they
# predate Apple Silicon — so on arm64 we must build the framework from
# source with Xcode. The source fully supports arm64 + Metal; we build a
# universal (arm64 + x86_64) binary. Intel Macs without Xcode fall back
# to the prebuilt SDK zip.
#
# End users installing the vizz.app bundle never run this: the framework
# is already embedded by CI.
set -euo pipefail

dest="${1:-vendor}"
mkdir -p "$dest"

have_compatible() {
    local bin="$dest/Syphon.framework/Versions/A/Syphon"
    [ -f "$bin" ] || return 1
    if [ "$(uname -m)" = "arm64" ]; then
        lipo -info "$bin" 2>/dev/null | grep -q "arm64" || return 1
    fi
    return 0
}

if have_compatible; then
    echo "$dest/Syphon.framework already present and compatible"
    exit 0
fi

build_from_source() {
    echo "Building Syphon.framework from source (universal)..."
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    git clone --depth 1 https://github.com/Syphon/Syphon-Framework "$tmp/src"
    xcodebuild -project "$tmp/src/Syphon.xcodeproj" \
        -target Syphon -configuration Release \
        ONLY_ACTIVE_ARCH=NO ARCHS="arm64 x86_64" \
        MACOSX_DEPLOYMENT_TARGET=11.0 \
        CODE_SIGN_IDENTITY="" CODE_SIGNING_REQUIRED=NO CODE_SIGNING_ALLOWED=NO \
        SYMROOT="$tmp/build" build
    rm -rf "$dest/Syphon.framework"
    cp -R "$tmp/build/Release/Syphon.framework" "$dest/Syphon.framework"
    echo "Installed $dest/Syphon.framework (built from source)"
    lipo -info "$dest/Syphon.framework/Versions/A/Syphon" || true
}

download_prebuilt() {
    echo "Downloading prebuilt Syphon SDK (x86_64 only)..."
    local api="https://api.github.com/repos/Syphon/Syphon-Framework/releases/latest"
    # CI runners share anonymous API quota and get 403s; use the token there.
    local json
    if [ -n "${GITHUB_TOKEN:-}" ]; then
        json=$(curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" "$api")
    else
        json=$(curl -fsSL "$api")
    fi
    local url
    url=$(printf '%s' "$json" \
        | grep -o '"browser_download_url": *"[^"]*\.zip"' \
        | grep -o 'https://[^"]*' | head -1)
    [ -n "$url" ] || { echo "error: no .zip asset in latest release" >&2; exit 1; }
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    curl -fsSL "$url" -o "$tmp/syphon.zip"
    unzip -q "$tmp/syphon.zip" -d "$tmp/unpacked"
    local framework
    framework=$(find "$tmp/unpacked" -maxdepth 3 -name "Syphon.framework" -type d | head -1)
    [ -n "$framework" ] || { echo "error: Syphon.framework not in zip" >&2; exit 1; }
    rm -rf "$dest/Syphon.framework"
    cp -R "$framework" "$dest/Syphon.framework"
    echo "Installed $dest/Syphon.framework (prebuilt, x86_64)"
}

if command -v xcodebuild >/dev/null 2>&1 && xcodebuild -version >/dev/null 2>&1; then
    build_from_source
elif [ "$(uname -m)" = "x86_64" ]; then
    download_prebuilt
else
    echo "error: this is an Apple Silicon Mac but the official prebuilt" >&2
    echo "Syphon.framework is Intel-only, and Xcode (for building it from" >&2
    echo "source) was not found. Either install Xcode and re-run, or copy" >&2
    echo "a universal Syphon.framework into $dest/." >&2
    exit 1
fi

#!/usr/bin/env bash
# Fetch the latest Syphon.framework release into ./vendor (or $1).
# vizz loads the framework from vendor/ automatically at startup.
set -euo pipefail

dest="${1:-vendor}"
api="https://api.github.com/repos/Syphon/Syphon-Framework/releases/latest"

echo "Looking up latest Syphon-Framework release..."
json=$(curl -fsSL "$api")
url=$(printf '%s' "$json" \
    | grep -o '"browser_download_url": *"[^"]*\.zip"' \
    | grep -o 'https://[^"]*' \
    | head -1)

if [ -z "$url" ]; then
    echo "error: could not find a .zip asset in the latest release" >&2
    echo "Download Syphon.framework manually from:" >&2
    echo "  https://github.com/Syphon/Syphon-Framework/releases" >&2
    exit 1
fi

echo "Downloading $url"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$url" -o "$tmp/syphon.zip"
unzip -q "$tmp/syphon.zip" -d "$tmp/unpacked"

framework=$(find "$tmp/unpacked" -maxdepth 3 -name "Syphon.framework" -type d | head -1)
if [ -z "$framework" ]; then
    echo "error: Syphon.framework not found inside the release zip" >&2
    exit 1
fi

mkdir -p "$dest"
rm -rf "$dest/Syphon.framework"
cp -R "$framework" "$dest/Syphon.framework"
echo "Installed $dest/Syphon.framework"

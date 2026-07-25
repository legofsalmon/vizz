#!/usr/bin/env bash
# End-user installer: fetch the latest vizz release and put vizz.app in
# /Applications (or ~/Applications). No developer tools needed.
#
#   curl -fsSL https://raw.githubusercontent.com/legofsalmon/vizz/main/scripts/install.sh | bash
set -euo pipefail

repo="legofsalmon/vizz"

if [ "$(uname)" != "Darwin" ]; then
    echo "error: this installer is for macOS" >&2
    exit 1
fi

echo "Looking up the latest vizz release..."
url=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" 2>/dev/null \
    | grep -o '"browser_download_url": *"[^"]*vizz\.app\.zip"' \
    | grep -o 'https://[^"]*' | head -1 || true)

if [ -z "$url" ]; then
    echo "No release published yet." >&2
    echo "Grab the 'vizz.app' artifact from the latest CI run instead:" >&2
    echo "  https://github.com/$repo/actions" >&2
    exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "Downloading $url"
curl -fsSL "$url" -o "$tmp/vizz.app.zip"
ditto -x -k "$tmp/vizz.app.zip" "$tmp"

dest="/Applications"
if [ ! -w "$dest" ]; then
    dest="$HOME/Applications"
    mkdir -p "$dest"
fi
rm -rf "$dest/vizz.app"
mv "$tmp/vizz.app" "$dest/vizz.app"
# Clear the quarantine flag so Gatekeeper allows the unsigned app.
xattr -cr "$dest/vizz.app" 2>/dev/null || true

echo
echo "Installed $dest/vizz.app"
echo "First launch: right-click the app and choose Open (it is not"
echo "notarized with Apple). It appears in Resolume as Syphon source 'vizz'."

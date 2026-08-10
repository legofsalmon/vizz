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
# Matches both naming schemes: "vizz-0.14.0.app.zip" as published from
# v0.14.0 onwards, and the plain "vizz.app.zip" of every release before
# it. This script is fetched fresh from main, but it is also the script
# someone may run against an older release, and an installer that only
# understands the newest convention silently stops working on the ones
# it was written for.
url=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" 2>/dev/null \
    | grep -o '"browser_download_url": *"[^"]*vizz[^"/]*\.app\.zip"' \
    | grep -o 'https://[^"]*' | head -1 || true)

if [ -z "$url" ]; then
    # Distinguish "no release" from "release exists but has no app attached",
    # which is what a failed release workflow looks like from out here.
    if curl -fsSL "https://api.github.com/repos/$repo/releases/latest" >/dev/null 2>&1; then
        echo "The latest release has no app bundle attached yet." >&2
    else
        echo "No release published yet." >&2
    fi
    echo "Grab the 'vizz.app' artifact from the latest CI run instead:" >&2
    echo "  https://github.com/$repo/actions" >&2
    exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "Downloading $url"
# A fixed local name: the download's name carries the version for the
# benefit of a Downloads folder, and this file is deleted on exit.
curl -fsSL "$url" -o "$tmp/vizz.zip"
ditto -x -k "$tmp/vizz.zip" "$tmp"

dest="/Applications"
if [ ! -w "$dest" ]; then
    dest="$HOME/Applications"
    mkdir -p "$dest"
fi
rm -rf "$dest/vizz.app"
mv "$tmp/vizz.app" "$dest/vizz.app"
echo
echo "Installed $dest/vizz.app"
echo "Releases are signed and notarized, so it opens with a normal"
echo "double-click. It appears in Resolume as Syphon source 'vizz'."

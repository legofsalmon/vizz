#!/usr/bin/env bash
# Checks that must pass before a release is cut.
#
# A published release is effectively irreversible: you can delete it from
# GitHub, but not from the machines that already downloaded it, and a tag
# that has been fetched stays fetched. So everything machine-checkable is
# checked here, loudly, before anything is tagged.
#
#     ./scripts/preflight-release.sh v0.2.0
#
# Exits non-zero with a specific reason on any failure. Nothing here
# mutates the repository.

set -euo pipefail

TAG="${1:-}"
if [[ -z "$TAG" ]]; then
  echo "usage: $0 vX.Y.Z" >&2
  exit 2
fi

fail() { echo "preflight: FAIL — $*" >&2; exit 1; }
ok()   { echo "preflight: ok   — $*"; }

# --- tag shape -------------------------------------------------------------
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] \
  || fail "tag '$TAG' is not vX.Y.Z (optionally -prerelease)"
WANT_VERSION="${TAG#v}"
WANT_VERSION="${WANT_VERSION%%-*}"
ok "tag shape"

# --- the version the binaries will report ----------------------------------
# This is the check that keeps the update banner honest: it compares a
# published tag against the running build's own version, so a tag ahead of
# Cargo.toml makes every user nag about an update they already have.
CARGO_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[[ "$CARGO_VERSION" == "$WANT_VERSION" ]] \
  || fail "workspace version is $CARGO_VERSION but the tag says $WANT_VERSION — bump Cargo.toml first"
ok "workspace version $CARGO_VERSION matches $TAG"

# --- the landing page's download button ------------------------------------
# The published download is version-stamped, so the site cannot use
# GitHub's /releases/latest/download/ permalink (it resolves a constant
# filename) and instead pins the release. A pinned link is exactly what
# gets forgotten in a release and found by a user downloading the wrong
# version. The vizz-update test checks this too, on every CI run; it is
# repeated here because this is the last gate before a tag exists.
grep -q "/releases/download/$TAG/" site/index.html \
  || fail "site/index.html's download button does not point at $TAG"
ok "site download button points at $TAG"

# --- repository state ------------------------------------------------------
[[ -z "$(git status --porcelain)" ]] || fail "working tree is dirty"
ok "working tree clean"

git fetch --quiet origin main
LOCAL=$(git rev-parse HEAD)
REMOTE=$(git rev-parse origin/main)
[[ "$LOCAL" == "$REMOTE" ]] \
  || fail "HEAD ($(git rev-parse --short HEAD)) is not origin/main ($(git rev-parse --short origin/main)) — release from main"
ok "on origin/main at ${LOCAL:0:8}"

if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1 \
   || git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
  # Re-running the workflow to repair a release with a missing asset is a
  # legitimate thing to want, so this is a warning rather than a failure —
  # but it must be a deliberate choice, not a surprise.
  echo "preflight: WARN — tag $TAG already exists; the workflow will build from it and re-upload assets"
else
  ok "tag $TAG is new"
fi

# --- the build actually works ---------------------------------------------
# Compiling is not evidence that it runs. A headless frame exercises the
# whole path a live set uses minus the swapchain, and is the cheapest thing
# that would catch a bundle that builds and then dies on startup.
echo "preflight: building release binary…"
# --locked: build with the committed Cargo.lock exactly as it stands, and
# fail rather than rewrite it. Without this the build quietly regenerated
# the lock — so a version bump left Cargo.toml and Cargo.lock disagreeing,
# the clean-tree check above passed (it ran before the build dirtied
# anything), and the release shipped with a lockfile naming the previous
# version. It also means the release is built from the dependency versions
# that were reviewed, not whatever resolves on the day.
cargo build --release --quiet --locked
ok "release build, against the committed lockfile"

REPORT=$(mktemp)
trap 'rm -f "$REPORT"' EXIT
if ! ./target/release/vizz --headless --frames 30 --width 320 --height 180 \
      --no-audio --no-update-check --report "$REPORT" >/dev/null 2>&1; then
  fail "the built binary could not render 30 headless frames"
fi
python3 - "$REPORT" <<'PY' || fail "headless run produced an unusable report"
import json, sys
d = json.load(open(sys.argv[1]))
fps = d.get("health", {}).get("fps", 0)
assert d.get("frames_requested") == 30, d.get("frames_requested")
assert fps > 0, f"fps was {fps}"
print(f"preflight: ok   — headless render, {fps:.1f} fps on the build machine")
PY

REPORTED=$(./target/release/vizz --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
[[ "$REPORTED" == "$WANT_VERSION" ]] \
  || fail "binary self-reports $REPORTED, expected $WANT_VERSION"
ok "binary self-reports $REPORTED"

echo
echo "preflight passed for $TAG"

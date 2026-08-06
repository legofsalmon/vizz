#!/usr/bin/env bash
# Re-render the landing page's preset stills from the presets themselves.
#
# The page claims every still is a real frame, and that claim only survives
# if regenerating them is one command. Any change to the renderer that
# alters a look — a hash, a palette, a tone-map — makes the shots stale;
# run this after one and commit what changes.
#
#     ./scripts/render-site-shots.sh
#
# Needs an already-built release binary and python3 with Pillow (for the
# WebP encode). Each preset is recalled over OSC into a headless run, which
# is the same frame path a live set renders.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=target/release/vizz
[ -x "$BIN" ] || { echo "build first: cargo build --release" >&2; exit 1; }

# Slot numbers follow /preset/recall: built-ins in order, from 1.
PRESETS=(
  "1 slow-bloom"
  "2 butterfly"
  "3 tunnel"
  "4 stage"
  "5 confetti"
  "6 ribbon"
)

# An out-of-the-way port, so a live vizz on 7000 is not disturbed.
PORT=7411
W=1024
H=576
# Enough frames for the recall's ~300ms glide to settle and the field to
# animate well away from its cold start — the icon uses 90 at defaults.
FRAMES=300

for entry in "${PRESETS[@]}"; do
  read -r slot name <<<"$entry"
  echo "== $name (slot $slot)"
  "$BIN" --headless --frames "$FRAMES" --width "$W" --height "$H" \
         --osc-port "$PORT" --no-update-check --no-syphon \
         --dump "site/img/$name.png" &
  pid=$!
  sleep 1.5
  # One OSC float, hand-packed: address, ",f", then the slot big-endian.
  python3 - "$PORT" "$slot" <<'PY'
import socket, struct, sys
port, slot = int(sys.argv[1]), float(sys.argv[2])
def pad(b): return b + b"\0" * (4 - len(b) % 4)
msg = pad(b"/preset/recall") + pad(b",f") + struct.pack(">f", slot)
socket.socket(socket.AF_INET, socket.SOCK_DGRAM).sendto(msg, ("127.0.0.1", port))
PY
  wait "$pid"
  # Same encode as the shots the page already carries: WebP at quality 82.
  python3 - "site/img/$name" <<'PY'
import sys
from PIL import Image
base = sys.argv[1]
Image.open(base + ".png").convert("RGB").save(base + ".webp", quality=82, method=6)
PY
  rm "site/img/$name.png"
done
echo "done — review site/img/*.webp by eye before committing"

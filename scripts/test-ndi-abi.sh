#!/usr/bin/env bash
# Verify the NDI FFI layer against a stub implementing the same C ABI.
#
# The real NDI SDK is registration-walled and cannot be installed on a CI
# runner, but the part of it we can actually get wrong is our own FFI:
# struct layout, field values, stride handling, teardown order. The stub
# declares those structs independently from the C headers, so a mismatch on
# either side shows up as garbage rather than silent agreement.
set -euo pipefail
cd "$(dirname "$0")/.."

stub_src="crates/vizz-io/tests/fixtures/ndi_stub.c"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

case "$(uname)" in
    Darwin) stub="$work/libndi_stub.dylib" ;;
    *)      stub="$work/libndi_stub.so" ;;
esac
log="$work/ndi.log"

echo "Building NDI ABI stub..."
cc -shared -fPIC -O1 -o "$stub" "$stub_src"

# 330px wide so the row stride needs padding to the 256-byte copy
# alignment — the case where a stride bug would corrupt the image.
width=330
height=100
frames=20
expected_stride=$(( ((width * 4 + 255) / 256) * 256 ))

echo "Running vizz headless against the stub (${width}x${height})..."
VIZZ_NDI_RUNTIME="$stub" NDI_STUB_LOG="$log" \
    cargo run --quiet -- --headless --frames "$frames" --ndi \
        --ndi-name vizz-abi-test --width "$width" --height "$height" --fps 60 \
        >/dev/null 2>"$work/run.log" || { cat "$work/run.log"; exit 1; }

fail() { echo "FAIL: $1"; echo "--- stub log ---"; cat "$log"; exit 1; }

grep -q '^initialize$' "$log" || fail "NDIlib_initialize was never called"
grep -q 'create name=vizz-abi-test .*clock_video=0 clock_audio=0' "$log" \
    || fail "send_create got the wrong name or clocking flags"

frame_count=$(grep -c '^frame ' "$log" || true)
[ "$frame_count" -ge 5 ] || fail "only $frame_count frames reached the sender (expected >= 5)"

grep -q 'frame inst=WRONG' "$log" && fail "send instance pointer did not round-trip"

# One representative frame line carries every field we set.
line=$(grep -m1 '^frame ' "$log")
echo "$line" | grep -q "${width}x${height}" || fail "wrong dimensions: $line"
echo "$line" | grep -q 'fourcc=0x41524742' || fail "wrong FourCC (expected 'BGRA'): $line"
echo "$line" | grep -q "stride=$expected_stride" \
    || fail "wrong stride (expected $expected_stride): $line"
echo "$line" | grep -q 'fps=60/1' || fail "wrong frame rate: $line"
echo "$line" | grep -q 'fmt=1' || fail "frame_format_type should be progressive: $line"
echo "$line" | grep -q 'timecode=9223372036854775807' \
    || fail "timecode should be NDIlib_send_timecode_synthesize: $line"

# Pixels must be real rendered content read at the advertised stride:
# opaque, and not uniformly black (which is what a stride or pointer
# mistake typically yields).
echo "$line" | grep -qE 'bgra=[0-9A-F]{6}FF' || fail "alpha channel is not opaque: $line"
grep '^frame ' "$log" | grep -qvE 'bgra=000000' \
    || fail "every frame was black — pixels are not reaching the sender"

echo "NDI ABI verified: $frame_count frames, ${width}x${height}, BGRA, stride $expected_stride."

# vizz

Realtime generative visuals for VJing. Native Rust + wgpu (Metal on macOS,
Vulkan/DX12 on Windows), built to feed Resolume / TouchDesigner / MadMapper
over Syphon, Spout, and NDI, and to be played live over OSC and MIDI.

**Status: phase 2 — Syphon output.** Renders a procedural particle field
into a fixed-resolution master texture, publishes it over **Syphon** on
macOS (zero-copy, works windowed *and* headless), previews it aspect-fitted
in the window, and takes OSC control live — with built-in health monitoring
and a headless benchmark mode. Spout and NDI are next.

## Install (macOS, no developer tools)

```sh
curl -fsSL https://raw.githubusercontent.com/legofsalmon/vizz/main/scripts/install.sh | bash
```

This puts `vizz.app` (with Syphon embedded — nothing else to install) in
your Applications folder. **First launch: right-click → Open** — the app
is not notarized with Apple, so a plain double-click is blocked the first
time. Until the first tagged release exists, download the `vizz.app`
artifact from the latest [Actions run](https://github.com/legofsalmon/vizz/actions)
instead, unzip, and drag to Applications.

Double-clicking runs 1280×720 with OSC on udp/7000 and Syphon on (the
window title shows the live settings). Flags below need a terminal run.
To build your own bundle from source: `./scripts/make-app.sh` → `dist/vizz.app`.

## Running from source

```sh
cargo run --release                 # windowed, OSC on udp/7000
cargo run --release -- --osc-port 9000 --width 1920 --height 1080
```

Escape or closing the window quits. Logs (including the 2-second health
line) go to stderr; tune with `RUST_LOG=debug`.

The scene renders at the fixed `--width`×`--height` output resolution into
a master texture; the window is only an aspect-fitted preview. Resizing
the window never changes what receivers see.

### Syphon output (macOS)

The master texture is published as a Syphon server named `vizz` (change
with `--syphon-name`). It appears automatically in Resolume, VDMX,
MadMapper, Syphon Simple Client, etc. Flags: `--no-syphon` disables it;
`--syphon-flip` marks frames vertically flipped if your receiver shows
the image upside down.

Syphon.framework is loaded at runtime — no link-time dependency — so the
binary runs without it (the output just reports unavailable). Easiest
setup:

```sh
./scripts/fetch-syphon.sh    # provides Syphon.framework in ./vendor
cargo run --release
```

Note: the official prebuilt Syphon SDK is Intel-only, so on Apple
Silicon the script builds the framework from source, which needs Xcode
installed (one-time, ~30s). The vizz.app bundle ships with a universal
framework already embedded, so app users never need Xcode.

Or download it yourself from
<https://github.com/Syphon/Syphon-Framework/releases> and put it in any
of, in search order:

1. `$VIZZ_SYPHON_FRAMEWORK` (path to `Syphon.framework`)
2. `<binary dir>/../Frameworks/` (app-bundle layout) or next to the binary
3. `./vendor/Syphon.framework` or `./Syphon.framework` (working dir)
4. `~/Library/Frameworks/` or `/Library/Frameworks/`

Publishing is ordered on wgpu's own Metal command queue, directly after
each frame's submit — no cross-queue synchronization, no added latency,
and `--headless` publishes too, so vizz can run as a windowless Syphon
source.

### Headless / benchmark mode

```sh
cargo run --release -- --headless --frames 600 \
    --report bench.json --dump frame.png
```

Renders offscreen at a fixed 60 fps timestep, then writes a JSON health
report (fps, frame-time percentiles, over-budget frames, RSS, CPU) and
optionally the final frame as a PNG. This is the regression benchmark:
run it before and after a change and diff the reports. It exercises the
exact frame path of a live set minus the swapchain.

## OSC control

Send standard OSC messages (float, int, double, or bool args) to the UDP
port. Unknown addresses and malformed packets are logged and ignored —
control input can never crash the renderer.

| Address                 | Range        | Default | Meaning                        |
|-------------------------|--------------|---------|--------------------------------|
| `/particles/count`      | 0 – 500000   | 60000   | live particle count            |
| `/particles/size`       | 0.001 – 0.2  | 0.015   | sprite size                    |
| `/particles/speed`      | 0 – 4        | 0.6     | motion rate (phase-continuous) |
| `/particles/spread`     | 0.05 – 3     | 1.2     | field radius                   |
| `/particles/hue`        | 0 – 1        | 0.58    | base hue                       |
| `/particles/saturation` | 0 – 1        | 0.8     | color saturation               |
| `/particles/brightness` | 0 – 2        | 1.0     | value multiplier               |
| `/master/dim`           | 0 – 1        | 1.0     | master fader                   |

Every parameter has a per-parameter smoothing time constant, so stepped
controller input becomes a glide on screen.

## Architecture

```
crates/
  vizz-params   lock-free parameter store: control threads write atomic
                targets; the render thread pulls a smoothed snapshot each
                frame. No locks anywhere on the render path.
  vizz-osc      UDP OSC listener thread → writes into the param store.
  vizz-health   frame-time percentiles, over-budget counts, RSS/CPU.
                Serializable snapshots = the benchmark artifact, and later
                the GUI HUD's data source.
  vizz-render   wgpu GPU context + scenes. First scene: a fully procedural
                particle field (all per-particle state derived in the
                vertex shader — per frame the CPU uploads 32 bytes and
                issues one draw call, regardless of count).
  vizz-io       FrameSender / FrameReceiver traits + the Syphon backend
                (runtime-loaded framework, objc2 bindings, publish ordered
                on wgpu's Metal queue). Spout and NDI follow the same trait.
  vizz-app      the `vizz` binary: winit event loop (windowed) and the
                fixed-timestep headless runner.
```

Threading model: the render thread owns the GPU and never blocks — control
I/O (OSC now; MIDI, UI later) lives on its own threads and communicates
only through the atomic parameter store. Everything external (OSC packets,
and later MIDI devices and NDI sources) is treated as hot-pluggable and
fallible; failures degrade, log, and recover — they never stop the output.

## Platform notes

Primary targets: macOS (Apple Silicon, Metal) and Windows (NVIDIA,
DX12/Vulkan). Linux (Vulkan) builds and runs, including on Mesa's
software rasterizer (`llvmpipe`) for CI.

Planned output/input transports:

| Transport | Platform | Mechanism | Cost |
|-----------|----------|-----------|------|
| Syphon    | macOS    | IOSurface-backed Metal texture | zero-copy |
| Spout     | Windows  | DXGI shared handle | zero-copy |
| NDI       | all      | network (SpeedHQ) | CPU encode + async readback ring |

## CI

Every push runs `.github/workflows/ci.yml`:

- **Linux**: build + full test suite.
- **macOS (Apple Silicon)**: build + tests, then fetches Syphon.framework
  and does a 120-frame headless run on real Metal, failing if the Syphon
  server doesn't start or a publish errors. The health report
  (`bench-macos.json`), last frame (`frame-macos.png`), and run log are
  uploaded as artifacts — a per-commit performance record on real
  Apple-silicon hardware. It also builds and uploads the `vizz.app`
  bundle on every push.

Releases are cut by `.github/workflows/release.yml`, either way round:

- **Push a `v*` tag** — builds the bundle and publishes the release.
- **Actions → Release → Run workflow**, entering a tag (e.g. `v0.1.1`)
  and optionally a branch/commit to tag. This creates the tag if it does
  not exist, then builds and attaches the bundle — one action, no
  ordering pitfalls. Dispatching an *existing* tag rebuilds and
  re-attaches the bundle, which repairs a release that is missing its
  download.

The job fails if `vizz.app.zip` does not end up attached, so a release
can never silently ship without the app.

CI itself can also be re-run on demand via **Actions → CI → Run
workflow** (useful to regenerate the `vizz.app` artifact without an
empty commit).

## Roadmap

1. ~~Skeleton: render loop, param store, OSC, health monitoring, headless benchmark~~
2. Outputs: ~~Syphon send~~ ← here; Spout send, NDI send (async staging-buffer ring)
3. Control depth: MIDI + MIDI-learn, beat clock / Ableton Link, audio FFT
   input, LFO/envelope modulation on any parameter
4. Content: point-cloud & 3D-model generators, effect chains, external
   NDI/Syphon/Spout inputs as scene sources, scene crossfading, ISF support
5. GUI: control surface + health HUD (egui), preset save/recall
6. e2e performance suite: scripted OSC playback against headless runs,
   report diffing across commits

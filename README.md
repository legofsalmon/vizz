# vizz

Realtime generative visuals for VJing. Native Rust + wgpu (Metal on macOS,
Vulkan/DX12 on Windows), built to feed Resolume / TouchDesigner / MadMapper
over Syphon, Spout, and NDI, and to be played live over OSC and MIDI.

**Status: phase 5 — effects.** Renders a procedural particle
field into a fixed-resolution master texture, publishes it over **Syphon**
on macOS (zero-copy) and **NDI** on the network (async readback, never
stalls the renderer), previews it aspect-fitted in the window, and takes
OSC and MIDI control live, LFOs and a beat clock driving parameters on
their own, five morphing geometry modes and a feedback/mirror/glow effect
chain — with an on-screen control
panel, built-in health monitoring and a headless benchmark mode. Both
outputs work windowed *and* headless. Spout is next.

## Install (macOS, no developer tools)

```sh
curl -fsSL https://raw.githubusercontent.com/legofsalmon/vizz/main/scripts/install.sh | bash
```

This puts `vizz.app` (with Syphon embedded — nothing else to install) in
your Applications folder. **First launch: right-click → Open** — the app
is not notarized with Apple, so a plain double-click is blocked the first
time. You can also grab the bundle directly from the
[latest release](https://github.com/legofsalmon/vizz/releases/latest).

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

### NDI output (all platforms)

```sh
cargo run --release -- --ndi --ndi-name vizz --width 1920 --height 1080
```

Publishes the master texture as an NDI source on the network, so other
machines running Resolume/TouchDesigner/OBS can pick it up. Requires the
[NDI Tools/Runtime](https://ndi.video/tools/) redistributable — like
Syphon it is loaded at runtime, so the binary builds and runs without it
and the output simply reports itself unavailable. Set `VIZZ_NDI_RUNTIME`
to point at the library explicitly if it lives somewhere unusual.

Unlike Syphon, NDI cannot be zero-copy: it needs pixels in main memory.
The render thread still never waits for them —
`crates/vizz-io/src/readback.rs` keeps a ring of staging buffers, encodes
each frame's GPU→CPU copy and returns immediately, and a dedicated send
thread transmits whatever has finished. Rows keep wgpu's 256-byte padded
stride all the way to the wire (NDI accepts a line stride), so pixels are
never repacked. If the GPU or the network falls behind, frames are
**dropped for that output** and counted — never awaited, because losing an
NDI frame is survivable and missing vsync is not.

## Updates

On startup vizz asks GitHub once whether a newer release exists and, if
so, shows a banner in the panel with a link. It **never downloads or
replaces itself** — an update landing mid-set is precisely the failure
live software cannot afford, so a human picks the moment. Updating is
the same drag-and-drop as installing.

The check runs on a background thread with a short timeout and fails
silently: no network, an offline venue, a rate-limited API or a changed
response all end with vizz simply not mentioning it. `--no-update-check`
disables the request entirely.

## Geometry

`/shape/mode` sweeps through five forms — **sphere, torus, trefoil knot,
grid plane, hollow shell** — and fractional values sit *between* two of
them. Particles keep their identity across the blend (every form is
sampled from the same per-particle hashes), so the field flows from one
shape into the next rather than being re-scattered. A swept knob is
playable; a stepped one is not.

`/shape/twist` adds shear plus a height-dependent twist, and pairs well
with a slow LFO.

## Effects

The scene renders into an HDR buffer and passes through two full-screen
stages before becoming the master output.

**Feedback** (`/fx/trail`) mixes the previous frame back in, and because
the history is sampled through a per-frame zoom and rotation
(`/fx/zoom`, `/fx/spin`) a sustained setting builds a tunnel out of
whatever is on screen. This is the single effect that most changes how
the output reads. 0.7–0.85 is the useful range; the parameter stops at
0.98 because nothing would ever decay at 1.0.

Feedback *blends* rather than accumulates. Adding the history outright
makes a geometric series with gain `1/(1-trail)` — at 0.96 that is 25×
the scene, which saturates to flat white within a second regardless of
tone-mapping. A lerp keeps the steady state at the scene's own level
while still holding bright cores for a long time.

**Mirror** (`/fx/mirror`) folds UV space: horizontal, quad, or a six-wedge
kaleidoscope. Stepped rather than swept — half a mirror is not a look.

**Glow** (`/fx/glow`) adds a cheap wide-tap bloom, which is what makes
additive particles read as luminous.

Buffers are `Rgba16Float`: trails accumulate past 1.0, and 8-bit would
band and clip before the tone-map could roll it off.

## Control panel

The preview window carries an egui panel (press **Tab** to toggle, or
start with `--no-gui`) showing:

- **Health at a glance** — fps and average frame time, coloured by whether
  the 60 fps budget is being held, over a frame-time sparkline with the
  budget drawn as a reference line, plus p95/p99, worst, over-budget
  count, RSS and CPU.
- **Output status** — which of Syphon/NDI actually came up.
- **Modulation** — beat clock with a downbeat indicator, per-LFO shape
  and rate with a live output dot, and the route list with depth.
- **A slider for every parameter**, generated from the registry's own
  metadata, each with a MIDI **learn** and a **mod** button. Registering a parameter in `params.rs` gives it a control
  automatically, so the panel can never drift from the OSC surface, and
  the labels double as live documentation of the OSC addresses.
  Right-click a slider to restore its default.

The panel is a control-thread citizen exactly like OSC: it writes targets
into the same lock-free store and gets no privileged access to the
renderer. Its draw is one extra render pass inside the frame's existing
command encoder — no added synchronisation point.

To review the layout without a display (or in CI):

```sh
cargo run -p vizz-ui --example render_panel -- panel.png
```

## MIDI control

Any connected controller can drive any parameter. Devices are
hot-pluggable — ports are rescanned every couple of seconds, so plugging
a controller in mid-set connects it, and unplugging one disturbs nothing
else.

**To map a control**: click **learn** next to a parameter in the panel,
then move the knob or fader. The panel echoes whatever it is hearing
while learning, so a silent controller is immediately distinguishable
from a mapping problem. Click the binding label to clear it.

Supported sources:

| Source | Behaviour |
|--------|-----------|
| Control change | 0–127, or **14-bit** when the device sends the LSB pair (CC *n* + CC *n*+32) |
| Note | momentary — velocity while held, 0 on release |
| Pitch bend | full 14-bit range |

Mappings are saved as JSON to `~/.config/vizz/midi.json` (override with
`--midi-map`) the moment they change, so a crash mid-set cannot cost the
mapping you just set up. MIDI failing to start is a degraded mode, not a
failure: the visuals and OSC keep running.

Building on Linux needs ALSA headers (`libasound2-dev`); macOS and
Windows use CoreMIDI/WinMM and need nothing extra.

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
| `/shape/mode`           | 0 – 5        | 0.0     | geometry; fractional values morph |
| `/shape/morph`          | 0 – 1        | 0.0     | extra blend into the next form |
| `/shape/twist`          | 0 – 2        | 0.0     | shear and vertical twist       |
| `/fx/trail`             | 0 – 0.98     | 0.0     | feedback: how much of last frame survives |
| `/fx/zoom`              | 0.9 – 1.1    | 1.0     | per-frame zoom of the feedback (tunnels) |
| `/fx/spin`              | -0.1 – 0.1   | 0.0     | per-frame rotation of the feedback |
| `/fx/mirror`            | 0 – 3        | 0.0     | 0 off · 1 horizontal · 2 quad · 3 kaleidoscope |
| `/fx/glow`              | 0 – 1        | 0.25    | bloom lift                     |
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
  vizz-io       FrameSender / FrameReceiver traits and the output backends:
                Syphon (runtime-loaded framework, objc2 bindings, publish
                ordered on wgpu's Metal queue) and NDI (runtime-loaded
                library, async readback ring, dedicated send thread).
                Spout will follow the same trait.
  vizz-mod      modulation: LFOs and a beat clock producing normalised
                per-parameter offsets applied on top of the base value.
  vizz-midi     MIDI input: wire-format parsing, bindings with 14-bit CC
                pairing, MIDI-learn, and JSON persistence. Hot-plugs
                devices; writes into the param store exactly like OSC.
  vizz-ui       egui control panel + a wgpu 30 paint backend for egui
                (the published egui-wgpu still targets wgpu 29, and two
                wgpu versions cannot share a device).
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

- **Linux**: build, full test suite, and the NDI ABI check
  (`scripts/test-ndi-abi.sh`) — this runs vizz against a stub library
  implementing the NDI C ABI, verifying struct layout, FourCC, stride
  handling, and pixel delivery without needing the proprietary SDK.
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
2. Outputs: ~~Syphon send~~, ~~NDI send (async staging-buffer ring)~~ ← here; Spout send
3. Control depth: MIDI + MIDI-learn, beat clock / Ableton Link, audio FFT
   input, LFO/envelope modulation on any parameter
4. Content: point-cloud & 3D-model generators, effect chains, external
   NDI/Syphon/Spout inputs as scene sources, scene crossfading, ISF support
5. GUI: control surface + health HUD (egui), preset save/recall
6. e2e performance suite: scripted OSC playback against headless runs,
   report diffing across commits

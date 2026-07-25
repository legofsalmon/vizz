# vizz

Realtime generative visuals for VJing. Native Rust + wgpu (Metal on macOS,
Vulkan/DX12 on Windows), built to feed Resolume / TouchDesigner / MadMapper
over Syphon, Spout, and NDI, and to be played live over OSC and MIDI.

**Status: phase 1 skeleton.** Renders a procedural particle field at vsync,
every visual parameter live-controllable over OSC, with built-in health
monitoring and a headless benchmark mode. Output/input backends (Syphon,
Spout, NDI) are specified as traits in `vizz-io` and land in phase 2.

## Running

```sh
cargo run --release                 # windowed, OSC on udp/7000
cargo run --release -- --osc-port 9000 --width 1920 --height 1080
```

Escape or closing the window quits. Logs (including the 2-second health
line) go to stderr; tune with `RUST_LOG=debug`.

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
  vizz-io       FrameSender / FrameReceiver traits for Syphon, Spout, and
                NDI backends, with the non-blocking rules they must obey.
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

## Roadmap

1. ~~Skeleton: render loop, param store, OSC, health monitoring, headless benchmark~~ ← here
2. Outputs: Syphon send, Spout send, NDI send (async staging-buffer ring)
3. Control depth: MIDI + MIDI-learn, beat clock / Ableton Link, audio FFT
   input, LFO/envelope modulation on any parameter
4. Content: point-cloud & 3D-model generators, effect chains, external
   NDI/Syphon/Spout inputs as scene sources, scene crossfading, ISF support
5. GUI: control surface + health HUD (egui), preset save/recall
6. e2e performance suite: scripted OSC playback against headless runs,
   report diffing across commits

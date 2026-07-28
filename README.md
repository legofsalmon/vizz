# vizz

Realtime generative visuals for VJing. Native Rust + wgpu (Metal on macOS,
Vulkan/DX12 on Windows), built to feed Resolume / TouchDesigner / MadMapper
over Syphon, Spout, and NDI, and to be played live over OSC and MIDI.

**Status: phase 7 — camera and space.** A procedural particle field
rendered into a fixed-resolution master texture and published over
**Syphon** on macOS (zero-copy) and **NDI** on the network (async
readback, never stalls the renderer).

Geometry is eight morphing modes including two strange attractors and a
point-cloud pair, with **PLY/XYZ import** and colour. Colour runs through
cosine palettes driven by index, radius, depth or height. A
feedback/mirror/glow/shift chain sits on the output, and a real camera
with orbit, field of view and depth of field looks into an optional
wireframe **room** sized to the frame for forced perspective and parallax.

Control is OSC, MIDI, and a **node graph** — sources, operators and
parameter sinks on a pannable canvas, with saved patches. Modulation
sources include LFOs, a beat clock, and **audio** analysis: four
configurable spectral bands plus tempo detection. There is a control
panel, a stripped-back **performance layout**, health monitoring and a
headless benchmark mode. Both outputs work windowed *and* headless.

Spout is the notable gap.

## Install (macOS, no developer tools)

```sh
curl -fsSL https://raw.githubusercontent.com/legofsalmon/vizz/main/scripts/install.sh | bash
```

This puts `vizz.app` (with Syphon embedded — nothing else to install) in
your Applications folder. You can also grab the bundle directly from the
[latest release](https://github.com/legofsalmon/vizz/releases/latest).

**Ad-hoc signed releases need right-click → Open on first launch.** Once
the Developer ID secrets are configured the workflow signs and notarizes
instead, and those builds double-click normally — verified in CI with
`spctl` and `stapler validate` before publishing, so it is asserted
rather than assumed. Each release's notes say which it is.

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

## Releasing

```sh
./scripts/preflight-release.sh v0.2.0    # refuses if anything is off
# then dispatch the Release workflow with that tag
```

A published release is effectively irreversible — you can delete it from
GitHub but not from machines that already downloaded it — so everything
machine-checkable is checked before anything is tagged.

Preflight refuses on: a malformed tag, a workspace version that does not
match it, a dirty tree, a HEAD that is not `origin/main`, a release build
that fails, a binary that cannot render 30 headless frames, or a binary
that self-reports the wrong version. An existing tag is a warning rather
than a failure, because re-running to repair a release with a missing
asset is a legitimate thing to want.

The version check is not bureaucracy. The update banner compares a
published tag against the running build's own version, so a tag ahead of
`Cargo.toml` makes every user nag about a release they already have.

The Release workflow then repeats the smoke test **on the bundled binary**
— launching the executable inside `vizz.app` and making it render. Building
is not evidence that it runs, and without this a bundle that dies on
startup would publish clean and pass the asset-presence check, with the
first person to find out being someone downloading it.

### Signing and notarization

When `APPLE_CERT_P12` and the App Store Connect API-key secrets are
present, the workflow signs with the Developer ID under the hardened
runtime, notarizes, staples, and then verifies with `stapler validate` and
`spctl -a -t install`. That last check is the important one: `codesign
--verify` only says the signature is well formed, while `spctl` asks the
question Gatekeeper will ask on a user's machine — so the "does it
double-click" property is asserted in CI rather than discovered on
download.

Stapling matters separately: without it, first launch needs a network
round-trip to Apple, which at a venue with no wifi is exactly the failure
this is meant to remove.

Without those secrets the workflow still publishes, ad-hoc signed, with a
warning in the log. Nothing is a flag day.

**The hardened runtime enforces library validation**, which would block
both the NDI runtime and any Syphon framework the user supplies
themselves — neither is signed by us. `scripts/vizz.entitlements`
therefore sets `com.apple.security.cs.disable-library-validation`. That is
a deliberate loosening, permitting third-party code we have not signed to
load; the alternative is dropping NDI from signed builds.

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

## Audio input

```sh
vizz --list-audio                        # device names
vizz --audio-device "Scarlett"           # substring match
vizz --no-audio                          # off entirely
```

Four bands, each with its own frequency range, gain and envelope timing,
available as modulation sources alongside the LFOs. Defaults are kick/sub,
bass, mids and highs; the edges are draggable in the panel because what
counts as "the kick" depends on the material.

Band levels read as the **RMS of the signal inside them**, the same units
as the broadband level, which is what makes the gain control predictable
across tracks. The panel meters both: the top bar is what modulation
receives, the bottom is what is arriving at the input, and the gap between
them is the gain. Levels are deliberately not auto-normalised — no
automatic gain survives a track that opens on silence.

Envelopes are asymmetric one-poles, fast up and slow down, so a kick lands
on the frame it happened but the value stays usable as a modulator rather
than a strobe. Audio sources are unipolar: they push a parameter up from
where you set it, rather than swinging it either side like an LFO.

### Tempo

BPM can be typed, tapped, or detected. Detection autocorrelates the onset
signal — spectral flux, positive differences only, so it follows attacks
rather than loudness — and peak-picks over the lags corresponding to
60–200 BPM.

The characteristic failure of that method is the octave error: reporting
150 for a 75 BPM track, because a signal that correlates at one period
also correlates at its multiples. Two guards, both in `beat.rs`: a
log-normal prior around 120 BPM that breaks ties without overriding a
genuinely unusual tempo, and an explicit check of whether half the
candidate also explains the signal.

Detected tempo only drives the clock when **auto** is ticked *and*
confidence clears a threshold. Ambient material with no pulse still
produces a peak, and letting that retune the clock mid-set is worse than a
stale tempo. Tapping switches auto off — an explicit manual override
should not be overwritten a frame later.

Capture never blocks: the device callback pushes into a lock-free ring, an
analysis thread drains it, and results are published through atomics. If
analysis falls behind, samples are dropped and counted rather than
awaited. No device at all is a normal condition — the engine reports
itself unavailable and everything else runs.

## Geometry

`/shape/mode` sweeps through seven forms — **sphere, torus, trefoil knot,
grid plane, hollow shell, Lorenz, Aizawa** — and fractional values sit
*between* two of them. Particles keep their identity across the blend
(every form is sampled from the same per-particle hashes), so the field
flows from one shape into the next rather than being re-scattered. A
swept knob is playable; a stepped one is not. The range wraps, so the top
morphs the Aizawa attractor back into the sphere.

`/shape/twist` adds shear plus a height-dependent twist, and pairs well
with a slow LFO.

### Attractors

The last two modes are strange attractors, integrated **once on the CPU at
startup** into a 65k-point lookup texture rather than iterated in the
shader. Iterating is the obvious approach and it is the wrong one here: an
attractor only appears after its transient decays — a few hundred Euler
steps for Lorenz — and the vertex shader would pay that six times per
particle (two triangles), twice over while morphing, every frame. It is
also unnecessary, because the shape never changes.

Storing the trajectory in time order pays for itself twice. Consecutive
texels are consecutive points in time, so advancing every particle's index
by the same amount sweeps the cloud *along* the attractor — the flow you
want, and it costs one addition. `/particles/speed` drives the rate.

Attractors rotate rigidly while the other modes keep their per-particle
spin. That differential spin is what shears the blobs into ribbons, but
the Lorenz butterfly is not a body of revolution: giving each particle its
own rate smears the two lobes into an anonymous cone within seconds.

## Modulation

Modulation is a directed graph. Sources, operators and parameter sinks are
all nodes; every node has one output and zero or more inputs. Press **G**
for the canvas.

Node kinds: LFO, audio band, level, phasor and constant (sources); curve,
math, scale, smooth, quantise and sample & hold (operators); parameter
(sink). Drag from an output port to an input to wire; drag an input away
to unplug; right-click for the add menu; Delete removes the selected node.
Scroll zooms about the cursor. Node positions save with the patch.

Three behaviours worth knowing:

**Cycles degrade, they do not explode.** Wiring an output back into its
own chain is a two-second mistake to make live. Nodes in a cycle are
excluded from evaluation and drawn red, and unrelated chains keep running.
A connection that *would* cycle is refused at the drop rather than
accepted and then disabled — a wire that appears and goes dead is worse
than one that never lands.

**One edge per input.** Summing several wires into a port invisibly is how
patches become unreadable; combine with an explicit Math node instead.

**Bypass passes through rather than mutes**, so it auditions a chain
without an operator instead of silencing it.

Parameters are edited in the inspector strip below the canvas rather than
inside the node boxes: inline widgets would have to be hit-tested through
the zoom transform and become unusable when zoomed out, which is exactly
when a patch is big enough to need editing.

### Patches and the palette

The canvas has a palette down the left listing every node kind, grouped
into sources, operators and outputs. It reads the same catalogue the
right-click menu does, so a new kind appears in both — and it makes the
operators discoverable rather than hidden behind a right-click nobody
thinks to try.

Patches save to `~/.config/vizz/patches/*.json`, including node positions:
a patch that reloads with its layout scrambled has to be re-read from
scratch. Writes go to a temporary file and are renamed over the target, so
a crash or a full disk mid-save cannot destroy the patch that was already
there.

Patch names are user-typed and become filenames, so they are reduced to a
conservative allowlist rather than trusted — `../../../.ssh/config` is a
name someone can type, and it has to land in the patch directory as a
mangled filename. The sanitised name is shown back after saving, because
silently renaming a patch makes it unfindable later.

The flat route list still works and its offsets sum with the graph's.

## Performance layout

Press **P**. Eight assigned faders, a master, and the two or three facts
that matter when something goes wrong — is the output live, is it dropping
frames, is audio still arriving.

Deliberately not a smaller control panel. The panel is for building a
look: every parameter, dense, read at a desk. This is for playing one. A
control you did not decide to reach for in advance is a control you will
not find in a dark room, and having it on screen only makes the ones you
do want harder to hit.

The faders are drawn by hand rather than with `egui::Slider`, because the
built-in vertical slider is a thin rail with a small handle whatever size
it is given — the premise of this screen fails with it. Here the **whole
column** is the drag target, and grabbing anywhere in it jumps to that
value rather than dragging relatively, which is what you want when
reaching quickly.

Click a fader's name to reassign it. Assignments live in
`~/.config/vizz/macros.json`, separately from patches: which parameters
you want under your fingers is a property of how you play, not of the
modulation graph, and loading someone else's patch should not rearrange
your faders. A slot pointing at a parameter this build no longer has draws
as an empty placeholder rather than vanishing, so the layout cannot reflow
mid-set.

Eight is a deliberate limit — enough for the things worth reaching for,
few enough that each stays large and unambiguous under stage lighting.

## Camera and room

```
/camera/distance /camera/orbit /camera/elevation
/camera/fov /camera/focus /camera/defocus
/room/brightness /room/depth /room/fade
/room/converge /room/vanish_x /room/vanish_y
/room/anchor /room/embed
```

The camera used to be four hardcoded lines in the vertex shader. It is now
a real view/projection, because parallax, forced perspective and depth of
field all depend on *where you are* and none of them can be expressed
without one.

Distance and field of view are both "zoom" and are deliberately separate:
moving closer changes the perspective, narrowing the lens does not.

**Depth of field** resizes the sprite rather than blurring the frame. A
defocused point light *is* a larger, dimmer disc, so this is closer to the
real thing than a post-process blur and costs nothing — brightness falls
as the square of the disc, or defocusing would brighten the image instead
of softening it.

### The room

A wireframe box drawn with the same projection, so camera movement
parallaxes it against the cloud. That parallax is the point: a static
backdrop reads as wallpaper, one that shifts against the foreground reads
as space. Off by default — it is a strong look, not a neutral one.

Its opening is sized from the camera frustum rather than by eye, so **at
the design viewpoint the frame edge is the room edge** and the screen
reads as a window. Guessing the numbers instead leaves a sliver of
background along one edge, which reads as a floating box and gives the
illusion away. The opening tracks the output aspect automatically, so it
is correct for whatever canvas you configure.

There is a real trade here, and it is the interesting one. The room is
fixed in world space, so the illusion is exact only at
`elevation 0, orbit 0`. Moving off that viewpoint reveals the room's
edges — but that is also what produces the parallax. Orienting the room to
the camera instead would keep the illusion everywhere and eliminate the
parallax entirely, which would defeat the purpose.

The opening actually reaches 1% past the frame. Landing it exactly on the
edge puts its outline at clip ±1, where whether a pixel draws is a coin
flip, and the glow pass smears the resulting ragged line into a soft bar
down the border. The walls should run *off* the edge of the frame; nobody
needs to see the opening's outline.

### Steering the perspective

`/room/converge` is the room's own angle of view, and it is separate from
the lens on purpose. `/camera/fov` decides what the frame contains;
converge decides how deep it feels. At `1.0` the walls are physically
parallel and you get only the perspective the projection gives you; lower
values pull the far end in, which is the forced-perspective exaggeration a
stage set does with physical scenery. At `0` the back wall collapses to a
point.

`/room/vanish_x` and `/room/vanish_y` slide the far end around, in units
of the opening's half-size — `±1` puts the vanishing point on the frame
edge. **The opening never moves**, whatever these do. That is the property
worth protecting: the far end is free to swing, the frame edge stays the
room edge, and there is no setting that unsticks the two. Vertically it is
the difference between looking along a floor and looking along a ceiling.

Both are ordinary parameters, so an LFO on `vanish_x` swings the whole
space while the frame stays locked.

### Putting the cloud in the room

Drawing the room and the cloud with the same camera gives you two objects
in one frame, not an object in a space — the walls converge, the cloud
does not, and the eye reads it as a sprite pasted over a backdrop.

`/room/embed` fixes that by handing the room's volume to the particle
shader. At `1` the cloud takes the same compression as the walls, read at
each point's own depth, so its near side stays larger than its far side
and it belongs to the set. Sprites scale with it: leaving the grain at a
fixed size while the shape around it shrinks is what gives a miniature
away.

`/room/anchor` is where along the room's depth it sits — `0` at the
opening, `1` against the back wall. Sweeping it walks the cloud into the
distance, and it shrinks and drifts toward the vanishing point as it goes.

`embed` defaults to `0`, so turning the room on never moves the cloud.
Reaching for a control mid-set must not teleport the thing the audience is
looking at.

## Point clouds

```sh
vizz --cloud scan.ply --cloud other.xyz
```

Reads **PLY** (ASCII and binary little-endian) and plain **XYZ/CSV/PTS**,
with per-point colour where the file has it. Files load into two slots
alongside the two built-in attractors, giving four in total.

`/shape/mode 7` shows the **cloud pair**: `/cloud/a` and `/cloud/b` choose
slots, `/cloud/morph` blends between them. That is separate from the shape
sweep because the sweep only reaches *adjacent* modes — morphing an
imported scan into Lorenz needs its own control. Slot choice is stepped
(half a slot is not a cloud); the morph is swept and modulatable, so it
can be driven from an LFO, the beat clock or an audio band like anything
else.

Particles keep their index across the blend, so the same point travels
from one cloud to the other rather than the field being re-scattered. Be
aware that a linear blend between two unrelated clouds passes through a
shapeless middle — that is inherent to index-based morphing, not a bug.
Clouds with related structure morph far better than arbitrary pairs.

Imported colour multiplies the palette rather than replacing it, so the
palette still works as a tint and an uncoloured cloud is unaffected.
Colour is packed into the position texture's unused `w` channel, eight
bits per channel, so carrying it costs nothing over positions alone.

Clouds are resampled to fill their slot: fewer points than the slot means
each is repeated with a small deterministic jitter, because a dense clump
at the origin is far more visually wrong than slight duplication. Position
is centred and uniformly scaled to the same box the procedural shapes use,
so `/particles/spread` means one thing everywhere — uniformly, since
fitting each axis independently would stretch a scan into something that
is no longer the thing scanned.

A file that will not parse is a warning, never a startup failure.
Arriving at a venue to find the app refuses to open because a scan has a
malformed header is precisely the wrong trade. A truncated body keeps
whatever was read.

## Colour

`/color/palette` starts at **0 = the original HSV behaviour** and
crossfades up through four cosine gradients — spectrum, amber/magenta,
teal/gold, and a graphic red/blue two-tone. These are Inigo Quilez
gradients (`a + b·cos(2π(c·t + d))`): a whole smooth ramp from four
coefficients, one cosine to evaluate, and it loops seamlessly, which
matters because the drive value wraps.

`/color/drive` picks what maps to palette position — **particle index,
radius, depth, or height**. Index gives per-particle confetti; the other
three tie colour to geometry, which is what makes a shape read as a solid
object rather than a cloud of unrelated dots. It is stepped, not swept:
these are four different ideas, not a continuum.

`/color/spread` sets how much of the palette the field spans, and
`/particles/hue` still offsets into it.

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

**Shift** (`/fx/shift`) splits the red and blue channels along the vector
from the centre of the frame. Offsetting radially rather than by a fixed
amount is what makes it read as a lens: the middle stays sharp and the
fringing grows towards the edges. Green is left alone, so the image
fringes without shifting hue overall.

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
| `/shape/mode`           | 0 – 7        | 0.0     | geometry; fractional values morph |
| `/shape/morph`          | 0 – 1        | 0.0     | extra blend into the next form |
| `/shape/twist`          | 0 – 2        | 0.0     | shear and vertical twist       |
| `/fx/trail`             | 0 – 0.98     | 0.0     | feedback: how much of last frame survives |
| `/fx/zoom`              | 0.9 – 1.1    | 1.0     | per-frame zoom of the feedback (tunnels) |
| `/fx/spin`              | -0.1 – 0.1   | 0.0     | per-frame rotation of the feedback |
| `/fx/mirror`            | 0 – 3        | 0.0     | 0 off · 1 horizontal · 2 quad · 3 kaleidoscope |
| `/fx/glow`              | 0 – 1        | 0.25    | bloom lift                     |
| `/fx/shift`             | 0 – 1        | 0.0     | radial RGB split (chromatic aberration) |
| `/color/palette`        | 0 – 4        | 0.0     | 0 HSV · 1 spectrum · 2 amber · 3 teal · 4 red/blue |
| `/color/spread`         | 0 – 1        | 0.12    | how much of the palette the field spans |
| `/color/drive`          | 0 – 3        | 0.0     | 0 index · 1 radius · 2 depth · 3 height |
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

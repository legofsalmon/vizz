# vizz

Realtime generative visuals for VJing. Native Rust + wgpu (Metal on macOS,
Vulkan/DX12 on Windows), built to feed Resolume / TouchDesigner / MadMapper
over Syphon, Spout, and NDI, and to be played live over OSC and MIDI.

**Status: phase 9 — inputs.** A procedural particle field
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
configurable spectral bands plus tempo detection.

**Presets** capture every knob: six ship with the app, your own save
alongside them, and any of the first ten fires from a number key, from
OSC, or from a MIDI button. The control panel groups and filters its
parameters and marks the ones being modulated; the stripped-back
**performance layout** puts presets and eight assignable faders on one
screen. There is health monitoring and a headless benchmark mode, and both
outputs work windowed *and* headless.

It also **takes video and geometry in**, not just out: NDI video from
another machine or app, and **live point clouds** streamed as PLY over TCP
or watched from a file, which morph against loaded scans and attractors
like any other cloud.

Spout is the notable gap. Windows is built and tested in CI.

## Install (macOS, no developer tools)

```sh
curl -fsSL https://raw.githubusercontent.com/legofsalmon/vizz/main/scripts/install.sh | bash
```

This puts `vizz.app` (with Syphon embedded — nothing else to install) in
your Applications folder. You can also grab the bundle directly from the
[latest release](https://github.com/legofsalmon/vizz/releases/latest).

**Releases are signed and notarized, so they double-click normally** —
no right-click → Open. Every release is checked with `spctl -a -t install`
and `stapler validate` before publishing, so it is asserted rather than
assumed; v0.4.0 was the first to clear it. A build made without the
Developer ID secrets (a fork, or a local `make-app.sh`) is ad-hoc signed
and does need right-click → Open on first launch. Each release's notes
say which it is.

Double-clicking runs 1280×720 with OSC on udp/7000 and Syphon on (the
window title shows the live settings). Flags below need a terminal run.
To build your own bundle from source: `./scripts/make-app.sh` → `dist/vizz.app`.

## Running from source

```sh
cargo run --release                 # windowed, OSC on udp/7000
cargo run --release -- --osc-port 9000 --width 1920 --height 1080
```

Escape (pressed twice) or closing the window quits. Logs (including the
2-second health line) go to stderr; tune with `RUST_LOG=debug`.

OSC listens on every interface by default so a tablet across the stage
can drive it — which also means anyone on the venue's wifi can. On a
network you don't control, restrict it: `--osc-bind 127.0.0.1` accepts
only this machine.

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

### Live point clouds

```sh
vizz --live-cloud tcp://192.168.1.9:9000    # connect to a streaming app
vizz --live-cloud listen://0.0.0.0:9000     # wait for one to connect here
vizz --live-cloud /tmp/live.ply             # re-read a file as it is rewritten
```

Frames land in the last cloud slot. Select it with `/cloud/a` or
`/cloud/b` and set `/shape/mode 7`, and it morphs against a loaded scan or
an attractor like any other cloud.

**The framing needs no wrapper protocol.** An app that streams PLY just
concatenates whole files onto a socket, and a PLY header already declares
`element vertex N` and the properties each vertex carries — which is
exactly enough to compute the body's size. The header is its own frame
length, so anything that writes ordinary PLY files back to back works.

Both ASCII and binary little-endian are handled. Handing the socket
straight to the file parser would *not* have worked: its ASCII path reads
to end of stream, so the first frame would have swallowed every frame
after it. Binary would have framed correctly by accident, which is worse —
it would have passed testing and failed on whichever app sends ASCII.

The vertex count comes off the wire and sizes an allocation, so it is
bounded, and a stream with no `end_header` in the first 64 KB is refused
rather than read forever — pointing vizz at the wrong port should fail
quickly, not hang.

Same rules as every other input here: one slot rather than a queue,
because the newest cloud is the only one worth drawing; `try_lock` on the
render side, so a stalled sender costs a frame of staleness rather than
vsync; and reconnection as the normal path, so restarting the streaming
app does not mean restarting vizz.

Uploads happen only when the frame actually changed, and the cloud is
normalised on the way in exactly as a file is — so a stream in metres and
a scan in millimetres land at the same size on screen.

### NDI input

```sh
vizz --list-ndi          # what is visible on the network
```

Everything else in this project sends. This is the first path that takes
something *in*, which is what makes vizz mixable rather than only a
source: another app's output, a camera over NDI, or a second vizz
instance can come in and go through the same effects chain.

Same two rules as the output side. **The render thread never waits** — a
receive thread owns the NDI instance and writes finished frames into a
single slot, and the renderer takes whatever is there with `try_lock`, so
a stalled network drops a frame rather than missing vsync. One slot rather
than a queue, because for a visual input the newest frame is the only one
worth having. **Fail soft** — no runtime, no source, or a source that
disappears mid-set all log and leave the input reporting itself
unavailable, and reconnection is the normal case rather than an error.

Frames are requested as BGRA so the conversion happens inside NDI's own
optimised path, and the padded row stride is passed through to the texture
upload rather than repacked.

The FFI is hand-declared like the sender's, so the struct layouts are
asserted field by field against the C headers in tests. That is the only
thing between a header change and silent memory corruption.

**Not wired to the renderer yet.** Discovery, connection and frame capture
work and are tested; drawing the received frame is the next step.

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

**Gain is in decibels**, because that is the unit a sensitivity control is
read in everywhere else and because it is the unit that makes the four
rows comparable: the top band is not "ten" against the kick band's "six",
it is fifteen decibels hotter, which is a statement about how mixes are
built rather than an arbitrary multiplier.

The shipped gains are much larger than they look — a band's RMS is a few
percent of full scale once the spectrum is split four ways, and the top
band is the quietest of all — and they err high on purpose. Too much gain
shows up as a clipped meter you turn down; too little shows up as visuals
that barely move, which reads as the audio input not working.

**`fit` sets every band from what is actually arriving.** Play something,
press it, and each gain is scaled so that band peaks just short of the
clamp. This is the honest answer to "what should the default be": it
depends on the interface, the track and how hard it is driven, so no
shipped number is right for two rigs. The measurement is a peak held over
roughly the last four seconds, so a kick from a moment ago still counts. A
band with nothing in it is left exactly as you set it rather than having
its noise floor driven up to full scale.

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
to unplug; right-click for the add menu; Delete removes the selected node. **fit**
frames every node — an infinite canvas otherwise has a state you cannot
get out of, where you have panned far enough that nothing is on screen and
nothing points home. It is also the fastest way to read a patch you have
just loaded, whose layout came from someone else's screen.
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

The preset row sits above the faders, numbered to match the number keys,
so it doubles as the legend for them. Presets were the largest thing
missing from this layout: without them, changing look meant leaving it,
which is the one thing the layout exists to avoid.

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

## The control panel

Everything the panel shows is generated from the parameter table, so
registering a parameter gives it a control automatically and the GUI can
never drift out of sync with the OSC surface.

**One status line stays visible**: frame rate, an indicator per output,
audio input, tempo, and tap. Everything behind it — health detail, output
detail, MIDI devices, the band editors, the LFO editors — folds away,
because it is setup rather than performance. Before that split the status
blocks filled the panel and left the parameter list three rows tall.

**Parameters are grouped by address prefix** — `particles`, `shape`, `fx`,
`color`, `cloud`, `camera`, `room`, `preset`, `master` — with the count in
each header, and a `~` count of how many inside are being modulated, so a
collapsed group still says whether something in it is moving on its own.
A flat list of thirty-seven meant scrolling past everything to reach one
control.

**A modulated parameter is marked**, and hovering the mark shows the
current offset. A slider that will not stay where you put it is otherwise
indistinguishable from a broken one — the value is still yours, modulation
rides on top as an offset.

**A stepped parameter reads as its position's name.** `/shape/mode` at
`5.000` is legible and tells you nothing; `Lorenz` tells you what is on
screen. Same for `/fx/mirror`, `/color/drive` and `/color/palette`. Only
genuinely discrete controls get names — a swept one has none to give.

**Press `/` to filter.** Typing flattens the groups, because when you have
typed a name you already know what you want.

### Keyboard

```
1 – 9, 0   fire preset slot 1–10
Space      flash — white out while held
Tab        show or hide the control panel
G          modulation canvas
P          performance layout
/          filter the parameter list
?          the shortcut list, on screen
Esc        quit
```

`?` exists because a shortcut that lives only in a README is a shortcut
nobody uses. The number keys write `/preset/recall` exactly as OSC or MIDI
would, so there is one recall path rather than a second that can drift.

## Presets

```
/preset/recall     0 = none, 1..N = the list below
```

A preset is **where every knob is sitting**. A patch is the modulation
*graph* — what moves what. They are stored separately on purpose: recalling
a look should not rewire your LFOs, and loading someone else's patch should
not jump your visuals to their settings.

Six ship with the app: **Slow bloom** (wide breathing sphere, a neutral
opener), **Butterfly** (the Lorenz attractor), **Tunnel** (grid plane driven
into feedback, the high-energy one), **Stage** (cloud sitting inside the
room with forced perspective on), **Confetti** (dense, fast, per-particle
colour) and **Ribbon** (torus sheared by twist, mirrored).

They are compiled into the binary rather than written to disk on first run,
so they are always present, cannot be half-installed, and cannot be lost by
clearing the config directory. They are also read-only — "put it back how
it shipped" has to stay available.

Your own go to `~/.config/vizz/presets/*.json`, beside patches and the MIDI
map. Type a name, press save. Names are sanitised the same way patch names
are, because they become filenames.

**Recall does not snap.** Values go in as parameter *targets*, and the
registry's per-parameter smoothing carries them from wherever they are, so
a preset arrives as a glide of a few hundred milliseconds rather than a
cut. That is what makes them usable during a set instead of only between
tracks.

**Two parameters are never captured and never written.** `/master/dim` is
the panic fader — a preset that restored it could black out the show, or
silently undo a blackout somebody reached for. `/preset/recall` is excluded
because a preset containing it would fire another preset on load.

Each preset sets only the parameters that matter to its look, so recalling
one changes the thing you asked for and leaves everything else where you
left it.

### Firing them from a controller

`/preset/recall` is an ordinary parameter, which is what gets it OSC and
MIDI learn for free. **Slot 0 means nothing selected and presets start at
1** — that is what makes startup safe by construction, since the control
rests at 0 and the first frame has nothing to recall. Numbering from 0
instead would also make the first preset unreachable from a fresh start,
because the control is already sitting on it.

Recall is edge-triggered: it fires when the slot *changes*. A button parked
on a slot would otherwise re-apply its preset every frame, pinning every
parameter it names so you could not adjust one by hand afterwards. It is
also unsmoothed — a smoothed value glides through every slot between where
it was and where it is going, firing each preset on the way.

Built-ins come first and keep their order, so a slot learned onto a MIDI
button keeps meaning the same preset after you save your own.

## The scene grid

```
/scene/fire        0 = none, 1..16 = the pads
/scene/time        transition length in seconds; 0 is a cut
/scene/curve       linear · smooth · ease in · ease out · cut
/scene/auto        autopilot off/on
/scene/bars        bars between autopilot steps
```

Sixteen pads, laid out the way a sequencer is. A preset recalled by number
is a cut; firing a scene is the other thing you want during a set, where
moving from one look to the next takes musical time. Store the current
look into a pad with `store`, then press pads to travel between them.

**The blend is in the data, not in the picture.** The obvious way to cross
between two looks is to render both and dissolve the textures, and it is
the wrong way: two pictures of a particle field at half opacity is a double
image that reads as a mistake. Interpolating the *parameters* gives one
field whose settings are somewhere between the two — still one of
everything, still particles, and it looks like the material moving rather
than like a mixer. Point clouds go through the pair-morph above, so every
particle travels from where it sat in the outgoing cloud to where it sits
in the incoming one.

Two things do not interpolate. **Switches jump at the half-way point** —
`/fx/mirror` has an off, an x and a quad and nothing sensible between them,
and sweeping one would spend the transition showing states neither scene
asked for. **Cloud slots are never blended**, because the shader truncates
them to an index: half way between slot 0 and slot 2 is slot 1, a different
cloud entirely, which would flash on screen part-way through every move.
`/shape/mode` *does* sweep, because it is declared as a sweep and the
shader blends adjacent forms.

Firing during a transition re-aims from wherever the blend has reached, so
you are never locked out until the last move finishes.

**Autopilot** walks the filled pads in time with the beat clock, every
`/scene/bars` bars. It fires on the boundary and never on the frame you
switch it on — switching it on mid-bar and having the scene change
instantly is what would make it unusable in time.

The transition settings are ordinary parameters, which is what gets them
OSC and MIDI learn. All of them are excluded from presets, and so is
`/scene/fire`: a scene cell *is* a captured preset, so a cell holding the
fire control would fire itself the moment it arrived, forever.

The grid is drawn four by four in the control panel and sixteen across on
the performance layout, which has the width for the shape it wants. It is
saved to `~/.config/vizz/grid.json` beside the presets and the MIDI map.

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

| Address | Range | Default | Meaning |
|---------|-------|---------|---------|
| `/particles/count` | 0 – 500000 | 60000 | live particle count |
| `/particles/size` | 0.001 – 0.2 | 0.015 | sprite size |
| `/particles/speed` | 0 – 4 | 0.6 | motion rate (phase-continuous) |
| `/particles/spread` | 0.05 – 3 | 1.2 | field radius |
| `/particles/hue` | 0 – 1 | 0.58 | base hue |
| `/particles/saturation` | 0 – 1 | 0.8 | color saturation |
| `/particles/brightness` | 0 – 2 | 1 | value multiplier |
| `/shape/mode` | 0 – 8 | 0 | geometry; fractional values morph: sphere · torus · knot · grid · shell · Lorenz · Aizawa · cloud pair · sphere again |
| `/shape/morph` | 0 – 1 | 0 | extra blend into the next form |
| `/shape/twist` | 0 – 2 | 0 | shear and vertical twist |
| `/fx/trail` | 0 – 0.98 | 0 | feedback: how much of last frame survives |
| `/fx/zoom` | 0.9 – 1.1 | 1 | per-frame zoom of the feedback (tunnels) |
| `/fx/spin` | -0.1 – 0.1 | 0 | per-frame rotation of the feedback |
| `/fx/mirror` | 0 – 3 | 0 | 0 off · 1 mirror · 2 quad · 3 kaleido |
| `/fx/glow` | 0 – 1 | 0.25 | bloom lift |
| `/fx/shift` | 0 – 1 | 0 | radial RGB split (chromatic aberration) |
| `/punch/flash` | 0 – 1 | 0 | white-out while held — Space, a punch button, or a learned MIDI note |
| `/punch/black` | 0 – 1 | 0 | blackout while held; rgb only, coverage stays |
| `/punch/invert` | 0 – 1 | 0 | invert the finished picture while held |
| `/punch/freeze` | 0 – 1 | 0 | hold the picture; the set keeps moving underneath |
| `/punch/strobe` | 0 – 1 | 0 | beat-synced strobe while held |
| `/punch/strobe_div` | 0.25 – 4 | 0.5 | beats per strobe cycle |
| `/color/palette` | 0 – 15 | 0 | palette row: 0 hsv · 1 warm · 2 ember · 3 ice · 4 neon · 5+ loaded palettes |
| `/color/spread` | 0 – 1 | 0.12 | how much of the palette the field spans |
| `/color/drive` | 0 – 3 | 0 | what picks the colour: 0 index · 1 radius · 2 depth · 3 height |
| `/cloud/a` | 0 – 3 | 0 | first slot of the cloud morph pair |
| `/cloud/b` | 0 – 3 | 1 | second slot of the cloud morph pair |
| `/cloud/morph` | 0 – 1 | 0 | blend position between the pair |
| `/camera/distance` | 0.4 – 12 | 3.5 | orbit distance from the field |
| `/camera/orbit` | -3.15 – 3.15 | 0 | orbit angle around the field |
| `/camera/elevation` | -1.4 – 1.4 | 0.34 | height angle of the orbit |
| `/camera/fov` | 0.2 – 2 | 0.9 | field of view, radians |
| `/camera/focus` | 0 – 12 | 3.5 | focus distance |
| `/camera/defocus` | 0 – 1 | 0 | depth-of-field blur amount |
| `/camera/pan_x` | -4 – 4 | 0 | sideways pan of the view |
| `/camera/pan_y` | -4 – 4 | 0 | vertical pan of the view |
| `/room/brightness` | 0 – 1 | 0 | wireframe room visibility |
| `/room/depth` | 1 – 20 | 7 | how deep the room extends |
| `/room/fade` | 0 – 1 | 0.75 | distance fade of the room lines |
| `/room/converge` | 0 – 1 | 0.35 | perspective convergence of the grid |
| `/room/vanish_x` | -1 – 1 | 0 | vanishing point, sideways |
| `/room/vanish_y` | -1 – 1 | 0 | vanishing point, vertical |
| `/room/anchor` | 0 – 1 | 0.35 | where the cloud sits between front and back |
| `/room/embed` | 0 – 1 | 0 | how much the room's perspective bends the cloud |
| `/gravity/amount` | 0 – 1 | 0 | master depth of the gravity layer |
| `/gravity/N/x` (N = 0–3) | -3 – 3 | 0 | well N position, X |
| `/gravity/N/y` (N = 0–3) | -3 – 3 | 0 | well N position, Y |
| `/gravity/N/z` (N = 0–3) | -3 – 3 | 0 | well N position, Z |
| `/gravity/N/strength` (N = 0–3) | -2 – 2 | 0 | pull (positive) or push (negative) |
| `/gravity/N/radius` (N = 0–3) | 0.05 – 4 | 1 | well reach |
| `/gravity/fire` | 0 – 16 | 0 | fire gravity scene 1–16 on change; 0 = none |
| `/gravity/time` | 0 – 60 | 2 | gravity blend time, seconds |
| `/gravity/curve` | 0 – 4 | 1 | 0 linear · 1 smooth · 2 ease in · 3 ease out · 4 cut |
| `/gravity/auto` | 0 – 1 | 0 | gravity autopilot on/off |
| `/gravity/bars` | 0.25 – 16 | 4 | bars between gravity autopilot steps |
| `/bg/red` | 0 – 1 | 0.004 | background red |
| `/bg/green` | 0 – 1 | 0.004 | background green |
| `/bg/blue` | 0 – 1 | 0.008 | background blue |
| `/bg/alpha` | 0 – 1 | 1 | background opacity; 0 delivers the field on nothing |
| `/preset/recall` | 0 – 64 | 0 | recall preset N on change; 0 = none |
| `/scene/fire` | 0 – 16 | 0 | fire scene 1–16 on change; 0 = none |
| `/scene/time` | 0 – 60 | 2 | scene blend time, seconds |
| `/scene/curve` | 0 – 4 | 1 | 0 linear · 1 smooth · 2 ease in · 3 ease out · 4 cut |
| `/scene/auto` | 0 – 1 | 0 | scene autopilot on/off |
| `/scene/bars` | 0.25 – 16 | 4 | bars between scene autopilot steps |
| `/master/dim` | 0 – 1 | 1 | master fader |

The table is checked against the parameter registry by a test
(`the_readme_osc_reference_matches_the_registry`), so it cannot silently
go stale again.

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

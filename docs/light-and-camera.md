# Light and camera moves — design note

Two things were missing that a scan of a real place makes obvious. There
was no lighting at all — a point's colour was its palette entry times the
scan's own RGB times a distance fade, and nothing in the pipeline knew
where a light was. And the camera could orbit a point but not travel
through a space, because the only positional control was a *pan*, which
is defined as the move that keeps the subject centred.

## What shipped

- `crates/vizz-render/src/cameramove.rs`: ten canned paths as pure
  functions of a phase, and `Camera::at`, a world-space aim.
- Lighting in `particles.wgsl`: two lamps with distance falloff, an
  ambient level, and a directional sun shaded by `N · L`.
- `pointcloud::estimate_normals`: a neighbourhood plane fit, plus
  `nx`/`ny`/`nz` in the PLY reader.
- A second GPU texture for normals, half-float, zero meaning "unknown".
- Offline WGSL validation in `cargo test`, which is not part of either
  feature and should have existed a year ago.

## Decisions, and the alternatives they beat

**A move is an offset, not a parameter write.** The same rule modulation
follows, for the same reason: a fader you set stays where you set it. So
switching a move off returns you exactly to your framing, and a move
running is never a reason you cannot still steer. Writing into
`/camera/orbit` instead would have been fewer lines and would fight every
hand on the desk.

**Phase starts when you engage it, not from absolute bar count.** Bar-locked
phase is the tidier idea and it whips: engage an orbit halfway through its
cycle and the offset it wants is half a turn, which the fade then sweeps
through in half a second. Starting from where you pressed it is smooth
from any moment, and lands phrase-locked anyway if you press it on the
one — which is how you would press any other button here.

**Changing move fades out before it fades in.** Crossfading the two paths
against each other was the alternative and is worse: the midpoint of two
unrelated camera moves is a third position belonging to neither, and it
reads as a glitch rather than as a move.

**Ambient defaults to 1, every lamp to 0.** So the lighting term is
exactly `vec3(1.0)` and the picture is bit-for-bit what this renderer drew
before any of this existed. Every preset ever saved and the whole shipped
set depend on that being *exactly* true rather than nearly true, so it is
a named constant — `Uniforms::UNLIT` — that the parameter defaults are
tested against, and a GPU test renders both ends of it.

**Lamps use the gravity well's falloff.** `r² / (d² + r²)`: full at the
centre, half at the radius, asymptotically nothing beyond, no singularity
to divide by and no hard edge to show up as a visible shell. A lamp and a
well set to the same reach should reach the same distance.

**Hue *and* tint, not hue alone.** Hue has no value meaning "no colour".
Every hue is some colour and hue 0 is red, so a lamp described by hue
alone turns the scan red the instant somebody raises its level to see
what it does.

**Normals are estimated, never invented.** Most exports leave the field
empty, so reading them from the file only would have made relighting a
lottery decided by whoever pressed export. But a cloud with no surface —
a procedural shape, a video frame — reports the zero vector and every
directional term stands down, because a made-up normal lights a wall the
wrong way round and that is worse than not lighting it.

**Shading is two-sided.** A plane fit cannot tell `n` from `-n`. Resolving
it properly is a minimum spanning tree over a graph of a million nodes,
and it *still* comes out backwards for a scan of a room, where the
surfaces you want lit face inwards. Flipping the normal towards the eye
makes every surface face the viewer; there are no shadows here for that to
contradict, and the alternative is half of every scan rendering black for
a reason nobody could diagnose from the front of a stage.

**A second texture rather than more channels.** The position bank already
uses all four — xyz, with the colour bit-packed into `w`. Half-float is
plenty for a direction, so the normal bank costs half what the position
bank does.

**Estimation runs inline, on what the slot actually holds.** The plan was
a background thread. Then it was measured: about a tenth of a second for a
full slot in a *debug* build, next to a file read and a parse that already
cost more. A thread would have been machinery bought against a number
nobody had.

## The scale bug, and how it was found

The paths were written in units that suited a scanned room — three units
of travel, a camera two and a half units back. Every cloud is run through
`pointcloud::normalize`, which scales it to about a unit across.

So the walkthrough walked three widths away from a one-width object and
rendered a **black frame**. Every unit test passed: the move closed on its
cycle, it was linear in size, it travelled somewhere. None of them knew
how big the world was.

What found it was rendering one. What now guards it is
`no_move_leaves_the_subject_behind`, which walks every move through its
whole cycle and fails if the aim leaves a bounded box or the camera ends
up behind its own lens — and a named `WORLD` constant, so the paths are
written in multiples of "the thing you are looking at" rather than in
numbers that happened to work once.

Two more came out of the same session, both invisible to the tests that
existed:

- **The walkthrough travelled along the heading it had already turned
  into**, so the square was not a square and the loop teleported once a
  cycle. Walking and looking are two headings — that is the whole of
  turning a corner — and now they are two variables.
- **The square started at the origin instead of being centred on it**, so
  the walk was a tour of the empty space beside the subject, with the
  field pinned to the edge of frame.

## The bug that made the sun do nothing

The shader looked a normal up by **shape mode** where the mapping wanted a
**cloud slot**. Mode 7 means "the cloud pair" and the slots come from
`/cloud/a` and `/cloud/b`; reading 7 as a slot index asks for slot 7,
which is empty. So every normal came back zero, every directional term
stood down, and the sun did nothing at all.

Nothing errors. Nothing looks broken. It renders a perfectly good picture
with the whole of surface shading silently absent, and no unit test can
see it because the mapping only exists inside the shader.

`the_sun_lights_a_surface_by_the_way_it_faces` renders a sheet through the
real pipeline with the sun in front of it and behind it, and fails if the
two agree. Probed against the old code, it reports exactly the symptom:
*facing it 5.57, backing it 5.57*.

## The regression this shipped with

`Attractors::load_slot` is the single path a cloud takes to the GPU, and
adding normal estimation to it added it for *every* caller — including
`set_cloud_streaming`, which runs once per arriving frame of a live PLY
stream.

Fitting normals to a full slot measures **80 ms in release**. On a stream
that is 80 ms of CPU per frame before anything is drawn, capping the whole
thing at around twelve frames a second. Nothing failed, nothing logged,
and the picture was correct — it was just slow, in the way that looks like
a network problem and gets diagnosed as one.

`Normals::Estimate` versus `Normals::AsGiven` now splits it at the call
site, because the two callers genuinely want different answers: 80 ms is
nothing beside the file read it accompanies and unaffordable sixty times a
second. And a normal fitted to a streamed frame is stale by the next one,
so the expensive answer was not even the right one.

`streaming_a_frame_does_not_fit_normals` pins it with a number, which is
unusual in this codebase and earned here: the gap between "uploads a
texture" and "fits sixty-five thousand planes" is two orders of magnitude,
so the threshold can be loose and still catch it. Probed by putting the
estimator back: *a stream frame took 85.9ms*.

The general lesson is about the shape rather than the numbers. A single
chokepoint every caller shares is exactly what you want for correctness
and exactly how a cost meant for one caller reaches all of them, silently,
because the expensive path and the cheap path are the same line of code.

## A pre-existing bug found on the way

`Attractors::load_slot` took `points[i % points.len()]` for each of the
65,536 texels in a slot. For any cloud *larger* than the slot that
expression is just `points[i]` — **the first 65,536 points and nothing
else**. Scanners write in scan order, so on a five-million-point room that
was one corner of it, and the rest of the scan had never been on screen.

Now it strides across the whole cloud. This changes what an existing look
made from a large scan renders: it shows the whole thing rather than a
slice of it. That is a visual change to existing work, done deliberately,
because the alternative is leaving most of every big scan unreachable.

## Sharp edges worth knowing

- **The sun needs normals; the lamps do not.** A lamp works on anything
  with a position, which is everything. If the sun appears to do nothing,
  the cloud has no surface to speak of — a procedural shape or a video
  frame — and that is the honest answer rather than a fault.
- Elevation is re-clamped after a move is added. `Camera::basis` crosses
  the view direction with world up, and a crane on top of an already
  raised camera walks past the ±1.4 that keeps that cross product from
  degenerating. The frame goes to NaN, all of it, in one step.
- Distance is clamped above zero for the same class of reason: a push
  subtracts, and a negative distance puts the eye on the far side looking
  back — a silent 180° flip mid-move.
- Drift's harmonics are whole numbers of the cycle. The readable version
  with three incommensurate periods snaps the camera once per cycle,
  which on a slow ambient drift is the one visible thing it does.

## Deferred, deliberately

- **Shadows.** Nothing occludes anything here, and adding it would make
  two-sided normals inconsistent — you would have to solve orientation
  properly first.
- **Specular.** On a field of soft round sprites there is no highlight to
  put anywhere.
- **More than two lamps.** A lamp is seven faders once you count position,
  reach, level and colour. The limit is not what the shader can afford but
  what a person will map, and two lamps plus a sun is already three-point
  lighting.
- **A move you can draw.** Ten named paths cover the asks; a keyframe
  editor is a different program.

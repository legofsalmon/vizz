# Interaction review — the whole instrument against Nielsen's heuristics

Everything shipped this cycle grew wave by wave, each wave reviewed in
isolation. This review walked the assembled instrument instead: every
control on every screen, what gestures it answers to, what feedback it
gives, and what guards it carries — then judged the inventory against
Nielsen's ten usability heuristics.

**Fix policy for the wave that closed this review:** feedback, help
text, colour and guard-parity gaps were fixed immediately — none of
them move a control. Anything that adds or relocates a control was
filed with its evidence (#48, #49, #50). A few behaviours that look
like findings are deliberate; they are recorded under "Accepted" with
the reasoning, so the next review does not re-litigate them.

Severity uses Nielsen's scale: **1** cosmetic · **2** minor (low
priority) · **3** major (usability problem, high priority) · **4**
catastrophe (blocks or destroys work).

## The state colour language

The root consistency finding deserves its own section because it fed a
third of the others. Each screen had grown its own copies of the state
colours: the performance layout's "learn" amber was a different hue
from the panel's and the grid's, its live green and warning orange were
near-miss copies, and the armed red doubled as the broken-pad colour.
One meaning now has one colour, in `crates/vizz-ui/src/theme.rs`:

| Colour | Meaning |
| --- | --- |
| `LEARN` amber | a MIDI learn is armed and waiting |
| `LIVE` green | an output, input or clock is alive |
| `WARN` orange | needs attention; nothing is armed |
| `ARMED` red | the next press is destructive |
| `CURRENT` blue | the current item — recalled preset, playing pad |

Layout inks (text ramps, track fills, backgrounds) stay per-module:
those are typography, and the two screens legitimately set type
differently. Only *state* lives in the theme.

## H1 — Visibility of system status

| Finding | Evidence | Sev | Disposition |
| --- | --- | --- | --- |
| A latched punch button was pixel-identical to a held one. Mistaking the two is a strobe you cannot stop. | `performance.rs` `punch_button` | 3 | **Fixed.** Latched buttons wear a small ARMED corner pip and the hover names the state ("LATCHED — click to release" vs "shift-click latches"). Painted-shape test both ways. |
| The panel's preset list never marked the recalled preset, though the stage row strokes it blue. The two lists disagreed about whether the app remembers where the look came from. | `panel.rs` `presets_section` vs `performance.rs:281` | 2 | **Fixed.** Same `CURRENT` stroke on the panel row; stroke-colour test both ways. |
| "Over budget" had two conditions and two colours inside one panel: the strip watched the recent window, the health headline compared the running average, in different oranges. | `panel.rs` status strip vs health section | 2 | **Fixed.** One `WARN`, one phrase; a comment records that the two deliberately measure different windows. |
| A configured video input had no persistent indicator anywhere. The only sign a feed had died was the point cloud freezing. | `windowed.rs` video handling | 3 | **Fixed** for status: a video dot with the source's name sits on the panel strip beside audio's, drawn only when a source was configured — most rigs have no video and a permanent "no video" dot would alarm about an absence nobody chose. The picker/panel section is filed as **#48**. |
| Modulation state is well told elsewhere: the `~` markers, the group headers counting driven parameters, the live-value marks on faders. | — | — | Strength; no change. |

## H2 — Match between system and the real world

| Finding | Evidence | Sev | Disposition |
| --- | --- | --- | --- |
| The pad menu said "play preset..." but only assigns — it does not fire the pad. | `grid_view.rs` context menu | 2 | **Fixed.** "assign preset…", and the empty-pad hover says "assign" too. |
| Learn verbs disagreed: the banner and echo line said "move a control", but pads, punches and preset slots are *pressed*. | `panel.rs` learn echo, `lib.rs` banner | 1 | **Fixed.** "move or press", in both. |
| A preset load replaces the live look and said nothing about it. | `panel.rs` preset list hover | 2 | **Fixed.** Hover says "replaces the current look". |

## H3 — User control and freedom · H5 — Error prevention

The destructive-click audit. The app's established guard idiom is the
armed click: first press relabels red, second press within the window
acts, anything else disarms. Preset delete and the grid's store/clear
modes already used it; three destructive actions did not.

| Finding | Evidence | Sev | Disposition |
| --- | --- | --- | --- |
| Canvas "new" cleared the whole modulation graph — the most destructive unguarded click in the app, with no undo. | `graph_view.rs` toolbar | 4 | **Fixed.** Armed: red "clear?" for 3 s, then acts; expires back to safe. Tested with real pointer events — first click must not clear, second must, an expired arm must not. |
| The pad context menu's "clear" emptied the pad instantly while the mode-button route armed first. One action, two doors, one guard. | `grid_view.rs:460` vs `:557` | 3 | **Fixed.** The menu now arms the pad (ARMED outline, hover names the pending clear, "cancel clear" in the menu); the next press on that pad clears, a press anywhere else disarms. |
| Audio "reset" — adjacent to "fit" — threw away a gain setup that took real material to dial in, on one click. | `panel.rs` audio section | 3 | **Fixed.** Armed, same idiom. |
| Right-click-reset was promised for "any slider" but missing on the performance faders and MASTER. | `performance.rs` `vertical_fader`, `master` | 2 | **Fixed.** Implemented on both; the overlay's promise is now true. A dimmed master is exactly the mess the gesture exists to get out of. |
| Applying an output size, scale or precision change silently tears down a running recording — discovered after the show as a short file. | `panel.rs` `output_setup_section` | 3 | **Fixed** for feedback: size controls say "applying stops any recording" on hover, and while a recording runs the section shows a live warning line. |
| Canvas mutations have no undo: loading a patch over an unsaved graph, deleting a wire. | `graph_view.rs` | 3 | **Filed #50.** An undo story is structural, not a text fix. |
| Existing strengths: Esc quits only on a second press; save vs replace is said on the button before it happens; saving under a built-in's name steps aside rather than shadowing; a MIDI learn is cancellable from its banner, its row, and its menu. | — | — | No change. |

## H4 — Consistency and standards

| Finding | Evidence | Sev | Disposition |
| --- | --- | --- | --- |
| Two LEARN ambers, two LIVE greens, two WARN oranges across screens. | `performance.rs:48-51` vs panel/grid constants | 2 | **Fixed.** `theme.rs`; every screen and the learn banner point at it. |
| The ARMED red doubled as "broken pad". A pad whose preset no longer exists read as "the next press is destructive" — wrong twice, since firing it does nothing at all. | `grid_view.rs:334,381` | 3 | **Fixed.** Broken is `WARN`; armed keeps `ARMED`. Tested both ways. |
| The stage preset row offers MIDI learn on right-click; the panel's preset list — same slots, same `/preset/recall` — offered nothing. | `performance.rs:301-324` | 2 | **Fixed.** The panel rows carry the same menu (learn / cancel / unmap) and show the binding in the hover. |
| The drop hints disagreed with the router: `.jpeg` was accepted but never mentioned; the palette hint omitted `.hex`/`.txt`. | `panel.rs` hints vs `windowed.rs::load_dropped` | 2 | **Fixed.** All three strings list exactly the accepted set, with a comment tying them to the router. |

## H6 — Recognition rather than recall

| Finding | Evidence | Sev | Disposition |
| --- | --- | --- | --- |
| The modulation canvas was reachable only via `G` and advertised nowhere — a whole surface you had to already know about. | `panel.rs` modulation section | 3 | **Fixed.** "open the canvas (G)" button in the modulation section, the no-routes hint names the canvas, and the canvas carries one caption line naming its gestures (right-click adds · drag ports to wire · drag an input off to unplug · Delete removes · scroll zooms). |
| The shortcuts overlay listed only keys. Shift-click latch, double-click rename, Delete on the canvas, scroll-zoom and the right-click menus appeared nowhere. | `lib.rs` overlay | 2 | **Fixed.** A mouse block in the same list. The blend chip's hover also gained its right-click. |
| The panel footer carried a second, divergent shortcut digest, drifting from the overlay. | `panel.rs` list footer | 2 | **Fixed.** Replaced with "? shows every shortcut" — one source of truth. |
| The LAYERS strip appears only once a layer is on, so it cannot teach *turning* one on; `/pal` is twelve raw sliders; the vector panel groups carry no prose; `/vec/place` and `/shape/mode` have no cycle affordance outside the strip. | `performance.rs` strip, `params.rs` | 2–3 | **Filed #49.** All add controls or surfaces. |

## H7 — Flexibility and efficiency of use

Strengths, no findings fixed this wave: every parameter is reachable
four ways (panel, OSC, MIDI learn, modulation) without configuration;
number keys fire presets; the punch row is hold-to-engage with the
shift-latch for the long haul; the filter box jumps the parameter list.
The latch gesture existed but was undiscoverable — that finding lives
under H6 and is fixed.

## H8 — Aesthetic and minimalist design

The gravity grid and the LAYERS strip draw only once their layer is in
use; transport parameters are hidden from the panel groups where their
company misled (they remain searchable — hiding a name that was just
typed would be worse). Both deliberate, both keep the default screens
quiet. **Accepted** as designed; the teaching gap this creates for
vector layers is the filed part of #49.

## H9 — Help users recognise, diagnose and recover from errors

| Finding | Evidence | Sev | Disposition |
| --- | --- | --- | --- |
| A broken pad (deleted preset behind it) warns in `WARN` and its hover names the recovery: "this preset no longer exists; right-click to pick another". | `grid_view.rs` | — | Was already in place; the wrong colour was the finding (fixed under H4). |
| Canvas load/save failures hold longer and read red where "saved" reads green — errors used to share the success colour and went unnoticed. | `graph_view.rs` status | — | Fixed in an earlier wave; verified still present. |

## H10 — Help and documentation

The README and docs-site OSC tables are enforcement-tested against the
registry, so the reference documentation cannot drift. In-app help is
hovers plus the shortcuts overlay; the overlay is now complete (H6) and
the footer points at it instead of paraphrasing it. No open findings.

## Accepted behaviours (do not re-file)

- **The window close button quits immediately.** Esc is two-step
  because a stray keystroke mid-set is survivable; the close button is
  the OS's own affordance and overriding it breaks the platform
  contract. Documented-deliberate.
- **Sweeping `/lN/blend`, `/lN/kind` or `/shape/mode` from a gliding
  controller steps through modes on the way.** Pack-time rounding means
  no frame is ever between modes; same accepted behaviour since
  `/shape/mode` shipped (see `docs/vector.md`).
- **The vector stack paints opaque**; `/bg/alpha` routing applies only
  while every layer is off. Documented in `docs/vector.md`.
- **Firing an empty pad is a no-op** rather than an error: the grid is
  played blind, and a click on nothing must cost nothing.

## Dead code removed by the review

- `grid_view::Shape::Panel` — caller-less since the panel lost its
  grid; the enum went with it, and the row always draws stage-shaped.
- `App::focus_filter` in `windowed.rs` — never set; the Gui owns the
  `/` shortcut and overwrote the app's value every frame.

## Verification

- Every fix that changes what is painted has a painted-shape test that
  reads glyphs, fills or stroke colours from the emitted shapes — not
  the strings the widgets were given. New: the latch pip (present
  latched, absent held), the armed-clear outline and hover, the
  broken-pad hue (WARN present, ARMED absent), the panel
  current-preset stroke (present recalled, absent otherwise), the
  video dot (present configured, absent otherwise), and the canvas
  "new" guard driven with real pointer events (first click must not
  clear; second must; an expired arm must not).
- Each of those tests was verified to **fail** with its fix reverted,
  one revert at a time, before this document was written.
- `docs-panel.webp` and `docs-stage.webp` re-rendered from the
  offscreen examples after the changes.

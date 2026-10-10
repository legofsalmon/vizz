# Working on vizz

## Design

This app follows the shared design system in
[legofsalmon/design-system](https://github.com/legofsalmon/design-system),
checked out beside this repository as `../design-system`. Before changing
anything a person sees or touches, read its `docs/designing.md` and answer its
questions, then the pages it points to: `principles.md` (who uses these tools
and where, the colour grammar, type, space and the behaviour rules),
`behavioural.md` (how crew behave while getting a job done), `navigation.md`,
`states.md`, `components.md` and `motion.md`.

- **Write the reasoning down.** A PR that changes what a person sees or
  touches has a short **Design** section: the one job of the view, the
  behaviour it designs for with the effect named (Default Effect, Loss
  Aversion, Time Scarcity…), and any departure from the system and why. A
  bug fix that changes no design says so.
- **Use the system's parts first.** Its roles rather than literal colours,
  and a shared component's states, keys and wording before drawing a new one.
- **Send back what you learn.** A gap in the system, a departure that was
  right, something crew did that the rules did not predict, or a meaning of
  this app's that another app now needs: add it to the design system's
  `docs/learnings.md` in a PR there, or list it under **Learnings** in this
  PR's Design section so it can be carried across.
- **What stays this app's own:** storage and wire names, layout and flow,
  content, and meanings only this app has.

vizz takes the colours, the state model and motion: `crates/vizz-design`
reads every value from the vendored `tokens.rs`
(`python3 scripts/design-system.py --sync` to update it, `--check` runs in
CI), `look::apply` draws egui's widgets in the shared states, and
`docs/design.md` says which names are vizz's own. Dark only. Fonts and the
density sizes for egui's controls come later. Its own meanings (violet
`DRIVEN` for modulation and the autopilot, the recording red, the slot and
engaged fills) stay in `vizz-design`.

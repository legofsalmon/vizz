# The parameter list — a review

The panel's left-hand list is where a look gets built. This is a review of
it against what a VJ is actually doing there, with the changes it led to
and the ones it did not.

## What was there

Measured rather than eyeballed — `print_the_parameter_list_shape` prints
it:

```
176 visible parameters in 20 groups (15 transport, hidden)
  l1  16   l2  16   l3  16   l4  16      the four vector layers
  gravity 21   light 17   pal 12          four wells, two lamps, four inks
  camera 14   room 8   particles 7 …
```

Two numbers do most of the work here:

- **64 of 176 rows — 36% — are four identical copies of the same
  sixteen parameter names.**
- **114 of 176 — 65% — are repeated instances**: layers, gravity wells,
  lamps, palette inks.

And everything opened by default, so the list you met was all of it. To
reach `camera/orbit` you scrolled past four identical sixteen-row blocks.

## Against the heuristics

The list already did several things well, and the review should say so:
the name filter with `/` to focus it, the group captions, the `~` marker
and LFO chips on a modulated row, the scroll bound derived from the
display, transport parameters deliberately kept out of company they would
mislead in. Those are heuristics 1, 6 and 8 already answered.

Where it fell down:

**Recognition rather than recall (6).** Sixteen identically-named rows,
four times over, with only `l1`/`l2`/`l3`/`l4` to tell them apart — and
the prefix is not on the row, only in the header you have scrolled past.
Same for `pal 0/r` and `gravity 2/strength`. You had to hold which
instance you were in.

**Aesthetic and minimalist design (8).** Not "make it prettier" —
Nielsen's point is that every extra unit of information competes with the
relevant units. Two thirds of this list was structure repeated at you.

**Consistency and standards (4).** `bg` is a colour swatch with a name in
the *background* section near the top and four raw r/g/b/alpha sliders in
the list. One thing, two representations, on one screen.

**Flexibility and efficiency of use (7).** The filter answers "where is
the parameter called X", which you can only ask if you know X. The
question someone building a look actually has — *what have I changed?* —
had no answer at all.

**Match between system and the real world (2).** `pal` sat under LOOK, one
section away from the only thing that prints in it, and immediately beside
the point field's *palette*, which is a different system with a similar
name.

## What changed

**Repeated structure became instances.** A group may now claim several
address prefixes, so the four vector layers are one group with four
instances rather than four unrelated groups. Numbered families —
`gravity/2/x`, `light/1/level`, `pal/3/r` — split the same way.

**An instance opens if anything in it is off its default.** No per-group
knowledge, and it answers the question the person opening the list has:
everything you have touched is open, everything you have not costs one
row. egui remembers the state once toggled, so it settles rather than
reopening things underneath you.

Measured, by `print_how_many_rows_open_by_default`:

> **66 rows open by default, of 176 in the list — 38% shown.**

Nothing was removed to get there.

**A `changed` toggle** beside the filter, showing only what is away from
default. It is the other half of the filter: one asks by name, this one
asks by state. Also the fastest way to find a value you moved by accident
and cannot see.

**`pal` moved from LOOK to PRINT and is called `inks`.** With the layers
that print in it, named for what it is, and no longer adjacent to the
other palette.

**Instances are numbered from one.** Addresses count from zero; nobody
reading a panel does. `gravity/0/x` is "1" on screen.

## What was considered and not done

**Colour swatches for `bg` and `pal`.** Twelve ink rows and four
background rows are four colours and one, and a swatch would say so in
five rows instead of sixteen. Not done here because those parameters must
stay individually MIDI-mappable and OSC-addressable — a swatch that
replaced them would quietly remove that, and a swatch that expands to
them is a third representation of a thing that already has two. It is the
right next change and it wants doing properly.

**Collapsing the instances into one editor with a 1/2/3/4 selector.**
Shorter still, and wrong for these: moiré is made by setting two layers to
*near* frequencies, so comparing two layers is the job. Nesting keeps them
side by side when opened; a selector would not.

**Dropping `punch` from the list.** It is momentary performance gesture
with proper controls one screen away, which is the same argument that
hides transport parameters. Left in: `black` is the one people look for
under pressure, and finding it in two places is better than not.

**Section chips that scroll to a section.** Worth having at 66 rows,
worth more at 200. Deferred rather than rejected.

## Sharp edges

- The instance heading is lit when something in it is off default and dim
  otherwise, so a closed row still says which of the four is doing
  something. That is the only status the closed state carries; it does not
  summarise values.
- `default_open` applies on first sight only. Recall a preset that lights
  layer 3 and the layer 3 row does *not* spring open if you have already
  toggled it. Deliberate — a list that rearranged itself as you fired pads
  would be worse than one that is occasionally shut.
- A group claiming several prefixes takes its `id_salt` from the first
  one, so reordering `["l1", "l2", "l3", "l4"]` would lose everybody's
  open/closed state. Not worth guarding; worth knowing.

# Decks — design note

Sixteen scenes and sixteen gravity slots is a generous evening if the set
is one continuous thing, and nowhere near enough if it is twelve songs
that each want their own looks. The choice a fixed grid forces is between
preparing four songs properly and preparing twelve badly, and it is not a
choice anyone should be making at soundcheck.

A deck is a page of both grids together, one per song, switched live —
Resolume's model, so a Resolume user should find nothing surprising here.
The same shape shipped in the sister lighting app first, and the two are
deliberately alike: one performer, two programs, one mental model.

## What shipped

- `crates/vizz-mod/src/deck.rs`: `Deck` (a name, both grids' cells, a
  Resolume column origin) and `Book` (the pages and which is live), with
  `decks.json` beside `grid.json` and `gravity-grid.json`.
- `/deck/select` — transport, 0 = none, 1..16 = the pages. An ordinary
  parameter, so a chip on screen, a MIDI button and an OSC message are
  one gesture, and bindings name the deck number the way pad bindings
  name the slot.
- `/column/fire` — transport, 0 = none, 1..16. Fires the scene pad and
  the gravity pad of that number together, which is what a column means
  in Arena.
- `crates/vizz-osc`: `ColumnSync` (three atomics) and `follow_column`,
  translating `/composition/columns/N/connect` into `/column/fire`.
- `Grid::adopt_cells`, and `Transition::to_slot` as an `Option`.
- A chip row above both grids on the performance layout: switch, rename,
  duplicate, delete, per-deck column origin, and the follow toggle.

## Decisions, and the alternatives they beat

**A deck holds references, not copies.** The alternative — snapshot the
parameters into the page — was rejected for the reason a scene cell was
converted away from it a release earlier: it makes preparing a look and
using it two independent things, so refining a preset leaves every page
still playing the version you had when you filled the pad, with nothing
on screen to say so. References cost a broken pad when a preset is
deleted, which is surfaced rather than hidden.

**The live page stays mirrored in the two grid files.** `decks.json`
could have been the single source of truth. Mirroring means three files
to keep in step, and buys three things: a build that predates decks still
opens the show, a corrupt or lost `decks.json` costs the songs you are
not playing tonight rather than the one you are, and the migration is
free — an existing grid becomes deck 1 with everything where it was.

**A page turn changes nothing on screen.** Every page turn happens in
front of an audience. Firing the new page's first pad on arrival was
considered and is wrong for the same reason autopilot does not fire on
the frame you switch it on: the performer decides when the picture
changes.

**Both fire parameters go back to rest on a page turn.** Two wrong
versions were built first. Clearing the edge latch alone left
`/scene/fire` holding the slot fired on the old page, so the next frame
read that as a change and turning a page played a scene. Leaving both
alone instead made the pad you are most likely to reach for — the same
number you just pressed — dead. Passing through zero is what makes
either work, and is the trick a MIDI trigger already uses on release.

**A blend in flight survives the swap.** It holds captured value maps
rather than cell references, so abandoning it would freeze the picture
half way between two looks for no reason. What it loses is its pad:
after a swap no pad on screen produced what is showing, so `to_slot`
becomes `None` and `current` clears. A lit pad claiming otherwise is
worse than none.

**The listener is dumb and lock-free.** `follow_column` reads two
atomics, writes one parameter and bumps a counter. The alternative —
resolving which deck owns a column on the OSC thread — would need the
whole book behind a lock shared with the render thread, which is exactly
how a flood of OSC traffic stalls a frame.

**Relaunching a column fires again; pressing the same pad twice does
not.** Firing is edge-triggered on the slot number, which is right for a
pad and wrong for a column: a relaunch in Arena is a deliberate
re-trigger. The listener's monotonic counter carries it, released after
the value and read with `Acquire` so a frame that sees the bump sees the
column it belongs to.

**Per-deck column origin rather than deck auto-switch.** A Resolume
column outside the live page's stretch does nothing. Having it switch
pages and then fire was considered and left out: with every page at the
default origin every page owns the same columns, so "which page does
column 3 belong to" has no answer that would not surprise someone
mid-set.

## Sharp edges worth knowing

- **Following is off by default and should stay that way unless asked
  for.** The OSC listener binds every interface, so following hands
  anyone on the venue's wifi the scene transport on both grids at once.
- `/deck/select` cannot be read to answer "which deck am I on". A MIDI
  trigger drives its parameter back to rest on release, so it reads 0
  while deck 3 is live. The book is the authority.
- A page turned by a controller or an OSC message with the panel hidden
  is saved outside the UI action gate. That gate covers everything else
  in the app, and a mapped deck button is exactly the case it would have
  missed.
- The chip row is drawn at every window size. It was guarded by the
  `cramped` flag at first; removing the guard and re-running the window
  sweep changed nothing, and it takes around three hundred points of
  added height before the fader block is laid out below the window at
  all.
- "Deck" already meant the mixing-desk surface throughout the UI's prose.
  Those now say "desk", so the word has one meaning.

## Deferred, deliberately

- **Tempo follow.** Arena sends `/composition/tempocontroller/tempo` as
  its slider normalised over 20–500 BPM, and the sister app converts it
  back. vizz has no BPM *parameter* — the clock is a plain field written
  from three places — so this needs a registry address and a third
  `ClockSource` variant to arbitrate against MIDI clock, auto-BPM and
  tap. Not expensive, not free, and not what was asked for.
- **Deck next/prev.** Direct select covers the workflow: you press the
  button for the song you are about to play. Next/prev would need
  momentary semantics that the value-binding mechanism does not have.
- **Clip-level follow.** `/composition/layers/N/clips/M/connect` is the
  finer unit; columns are the one that maps onto a grid of pads.
- **Undo.** Deleting a page is armed and asks once, which is the same
  guard the rest of the app's destructive actions use, and no more.

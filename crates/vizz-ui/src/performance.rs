//! The performance layout: what you look at during a set.
//!
//! Deliberately not a smaller version of the control panel. The panel is
//! for building a look — every parameter, dense, read at a desk. This is
//! for playing one: assigned faders large enough to hit without aiming, and
//! the facts that matter when something goes wrong.
//!
//! Three rules the layout is built on, all learned by rendering it and
//! looking at it rather than by reasoning about it.
//!
//! **Contrast is a feature, not a finish.** This screen is read in a dark
//! room, at a glance, often at an angle, by someone who is also doing
//! something else. Grey-on-grey is legible at a desk and invisible on
//! stage. Every label here sits at a deliberate step on a four-stop ramp,
//! and the dimmest stop is reserved for things that are genuinely inactive.
//!
//! **A control must show what is happening to it, not just what it is set
//! to.** A fader whose parameter is being modulated is at a different value
//! than its handle: the handle is what you set, the ghost mark is where the
//! LFO or the audio has actually pushed it. Without the second mark the
//! fader is lying whenever modulation is doing its job.
//!
//! **Fill the window.** The layout is anchored top-left and sized from the
//! real screen rect, not from what the widgets happened to need.

use egui::{Color32, Sense, Vec2, vec2};
use vizz_mod::perform::Macros;
use vizz_params::ParamRegistry;

use crate::panel::{AudioView, MidiView, OutputStatus};

// Local aliases into the design system, kept because this module says
// `INK` several dozen times and the short names read better in layout
// code. The values live in `vizz-design` — one place, every screen.
const INK: Color32 = vizz_design::ink::PRIMARY;
const INK_2: Color32 = vizz_design::ink::SECONDARY;
const INK_3: Color32 = vizz_design::ink::TERTIARY;
const INK_4: Color32 = vizz_design::ink::FAINT;

const PANEL_BG: Color32 = vizz_design::surface::BASE;
const TRACK: Color32 = vizz_design::surface::RAISED;
const FILL: Color32 = vizz_design::accent::FILL;
const FILL_BRIGHT: Color32 = vizz_design::accent::FILL_BRIGHT;
const HANDLE: Color32 = vizz_design::surface::HANDLE;
const MOD: Color32 = vizz_design::accent::MOD;
const MASTER_FILL: Color32 = vizz_design::accent::MASTER;
const MASTER_BRIGHT: Color32 = vizz_design::accent::MASTER_BRIGHT;
const LIVE: Color32 = crate::theme::LIVE;
/// An output that is not sending: off, in the ink ramp's word for it.
const DEAD: Color32 = vizz_design::ink::FAINT;
const WARN: Color32 = crate::theme::WARN;
const LEARN: Color32 = crate::theme::LEARN;

/// Faders per row. Sixteen slots as two rows of eight keeps each one wide
/// enough to hit and mirrors the grid's sixteen above it.
const COL_GAP: f32 = 6.0;

// Narrow enough that a full set of twenty-four plus the master fits on
// one axis at any window worth performing on. It was 62, which forced
// two banks at sixteen.
const FADER_MIN_W: f32 = 36.0;
// Wide enough that a fullscreen 1080p rig gets thumb-sized targets —
// at 104 the block spanned barely half of a 1920 window, left-anchored,
// against the module's own "fill the window" rule. The track only paints
// the middle three-quarters of the column, so even the widest fader
// stays a fader rather than a slab.
const FADER_MAX_W: f32 = 200.0;
/// The height floor, below which a fader is decorative. Above it the
/// layout *shrinks* rather than reflowing to one row: positions are what
/// hands find in the dark, and shorter beats moved.
const FADER_ABS_MIN: f32 = 56.0;
/// Value, name and binding under each track, plus their spacing.
const LABEL_H: f32 = 17.0;
const LABEL_GAP: f32 = 2.0;
const FADER_CHROME: f32 = LABEL_H * 3.0 + LABEL_GAP * 3.0;
const PAD: f32 = 14.0;

pub struct PerformanceState<'a> {
    /// A recording in progress: the strip wears a red chip, because
    /// forgetting a recording is how disks fill mid-set.
    pub recording: Option<crate::RecordingView>,
    pub outputs: &'a [OutputStatus],
    pub audio: &'a AudioView,
    pub fps: f32,
    pub over_budget: bool,
    pub bpm: f32,
    pub bar_phase: f32,
    /// The preset library in slot order, so the row can be numbered to
    /// match `/preset/recall` and the number keys.
    ///
    /// The whole entry rather than the name: the row groups and colours
    /// by what each look was built on, and a list of names cannot say.
    pub presets: &'a [crate::PresetEntry],
    /// Bumped whenever the app writes a preset picture, so a look
    /// re-photographed mid-set redraws rather than keeping the handle
    /// egui already had. See [`crate::thumbs`].
    pub thumb_revision: u64,
    /// The recalled slot (1-based): the one button that should look
    /// different from the other nine.
    pub preset_current: Option<usize>,
    /// The scene grid, laid out across the full width here.
    pub grid: &'a crate::grid_view::GridView,
    /// The gravity grid, when there is anything in it. Hidden entirely
    /// when empty: an unused second row of sixteen pads is a lot of
    /// screen spent on a layer you are not using, and this screen's whole
    /// argument is that what is on it is what you decided to reach for.
    pub gravity: Option<&'a crate::grid_view::GridView>,
    /// MIDI, so a fader can show its binding and start a learn without
    /// leaving the layout.
    pub midi: &'a MidiView,
    /// Live smoothed values including modulation, indexed by parameter
    /// position. A slice rather than a borrowed `ParamSnapshot` so
    /// [`crate::PanelState`] stays free of lifetimes. `None` in contexts
    /// with nothing to report, in which case faders draw only what the
    /// user set.
    pub values: Option<&'a [f32]>,
    /// The master output, when the app has handed it over.
    pub output_texture: Option<egui::TextureId>,
    /// Its aspect, so the picture is letterboxed rather than stretched.
    pub output_aspect: f32,
    /// The modulation graph, read-only, so a fader can say what is
    /// driving it and offer to change it. `None` in contexts with no
    /// modulation at all, where the faders simply omit the control.
    pub graph: Option<&'a vizz_mod::graph::NodeGraph>,
    /// The pages of pads, in order, and which one is live. Empty in
    /// contexts that have no deck book, where the row is not drawn at all.
    /// The open show's name, for the chip at the start of the strip.
    pub project: &'a str,
    pub decks: &'a [DeckChip],
    pub active_deck: usize,
    /// Whether Resolume's column launches are being followed, or `None`
    /// where the toggle is not on offer.
    pub follow_columns: Option<bool>,
}

/// One page, as the chip row needs to see it.
///
/// A name and its binding rather than the deck itself, for the reason the
/// grid view takes names: this crate does not depend on the deck module's
/// internals, and a chip row that did could not be drawn in a test without
/// building a book.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeckChip {
    pub name: String,
    /// The controller button that selects this deck, if one is mapped.
    pub midi: Option<String>,
    /// This chip is the one waiting for a button.
    pub learning: bool,
    /// Which Resolume column this deck's column 1 follows.
    pub origin: u32,
}

#[derive(Debug, Default)]
pub struct PerformanceActions {
    /// New show, open, save as, rename, delete — see
    /// [`crate::project_bar`].
    pub project: crate::project_bar::ProjectActions,
    /// The user tapped tempo.
    pub tapped: bool,
    /// Macro assignments changed and should be persisted.
    pub macros_changed: bool,
    /// The fader count changed this frame: `true` grew, `false` shrank.
    /// Carried separately from `macros_changed` so the app can say what
    /// happened — removing a fader that held something is worth a word.
    pub fader_count_changed: Option<bool>,
    /// Leave the performance layout.
    pub exit: bool,
    /// The controls are standing aside so the output can be seen. Read
    /// by the app so the toggle's state is inspectable rather than
    /// buried in egui memory.
    pub peeking: bool,
    /// Fire this preset slot (1-based, matching `/preset/recall`).
    pub preset_slot: Option<u32>,
    /// What the scene grid asks for this frame.
    pub grid: crate::grid_view::GridActions,
    /// What the gravity grid asks for this frame.
    pub gravity: crate::grid_view::GridActions,
    /// Begin MIDI-learn, or cancel with `None`.
    pub set_learn_target: Option<Option<vizz_midi::LearnTarget>>,
    /// Remove the MIDI binding for this parameter.
    pub clear_binding: Option<String>,
    /// Remove the MIDI trigger that recalls this preset slot. Separate
    /// from `clear_binding` because a slot parameter carries many
    /// bindings, and clearing the parameter would unmap every preset to
    /// unmap one.
    pub clear_slot_binding: Option<(String, f32)>,
    /// Put a ready-made modulator on this parameter, or take it off with
    /// `None`. Indexes [`vizz_mod::shapes::SHAPES`].
    pub set_mod_shape: Option<(String, Option<usize>)>,
    /// What the deck row asks for this frame.
    pub decks: DeckActions,
    /// Take a fresh picture of this preset from what is on screen now.
    pub preset_rephoto: Option<String>,
}

/// What the deck row asks the app to do.
///
/// Its own struct rather than fields on [`PerformanceActions`], and
/// deliberately not on [`crate::grid_view::GridActions`]: a page spans
/// both grids, and grid actions are applied once per grid — a deck field
/// there would be acted on twice, or handled by a third special case
/// beside `resync` and `set_lock` that reads only the scene grid's copy
/// and silently drops gravity's.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DeckActions {
    /// Turn to this page (0-based).
    pub select: Option<usize>,
    /// Add an empty page and go to it.
    pub add: bool,
    /// Copy this page and go to the copy.
    pub duplicate: Option<usize>,
    pub rename: Option<(usize, String)>,
    pub delete: Option<usize>,
    /// Point this page at a stretch of Resolume's columns.
    pub origin: Option<(usize, u32)>,
    /// Start or stop following Resolume's column launches.
    pub follow: Option<bool>,
    /// Replace every page with the built-in set.
    pub install_set: bool,
}


/// The pane's hairline. Dim on purpose: it marks where the picture is,
/// and a bright rule would be a bright rectangle in a dark room.
const PANE_EDGE: egui::Color32 = egui::Color32::from_rgb(0x2C, 0x33, 0x42);

/// Below this window width the desk closes and the layout is one column.
/// A picture too small to judge beside a grid too tight to hit is worse
/// than either done properly.
const DESK_MIN_W: f32 = 1180.0;
/// The narrowest the control column may be squeezed: eight gravity pads
/// plus their gaps at a size a hand can still hit. Sixteen used to set
/// this, before the scene grid moved to the desk.
const COL_MIN_W: f32 = 470.0;
/// The smallest picture worth calling a preview.
const PANE_MIN_W: f32 = 420.0;


/// How wide the control column is.
///
/// Named, rather than computed at its one call site, because the widgets
/// inside the column have to be *told* it: the column's `Ui` is set to the
/// full inner width — the status strip spans that — so anything laying
/// itself out from `available_width` wraps across the output pane instead
/// of inside its column. The layer strip has always been handed this. The
/// preset row was not, which nobody could see while a library was a
/// handful and everybody could see the moment one was a hundred and sixty.
fn column_width(inner_w: f32, desk: bool) -> f32 {
    if !desk {
        return inner_w;
    }
    // Narrower now that the scene grid has gone to the desk. What is left
    // has to carry the gravity grid at eight pads wide, which is what sets
    // the floor; everything else in the column is a single row.
    (inner_w * 0.42).clamp(COL_MIN_W, inner_w - PANE_MIN_W - PAD)
}

/// The largest rect of `aspect` that fits inside `outer`, centred.
///
/// A picture stretched to its container is a picture you cannot judge:
/// the whole point of looking is to see the framing you are about to
/// send, and a 16:9 master squeezed into a tall pane is a different
/// composition from the one going out.
fn letterbox(outer: egui::Rect, aspect: f32) -> egui::Rect {
    if aspect <= 0.0 || outer.width() <= 0.0 || outer.height() <= 0.0 {
        return outer;
    }
    let (w, h) = if outer.width() / outer.height() > aspect {
        (outer.height() * aspect, outer.height())
    } else {
        (outer.width(), outer.width() / aspect)
    };
    egui::Rect::from_center_size(outer.center(), vec2(w, h))
}

/// Paint the scrim around `pane`, or over everything when there is none.
///
/// Four rects rather than one with a hole, because the point is that
/// every label keeps an opaque ground while the picture keeps none: a
/// single translucent sheet would dim the output and still leave text
/// sitting on whatever a strobe does next.
fn paint_scrim(
    ui: &egui::Ui,
    slots: &[egui::layers::ShapeIdx],
    full: Vec2,
    pane: Option<egui::Rect>,
) {
    let all = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), full);
    let ink = egui::Color32::from_rgba_unmultiplied(PANEL_BG.r(), PANEL_BG.g(), PANEL_BG.b(), 242);
    let fill = |rect: egui::Rect| egui::Shape::rect_filled(rect, 0.0, ink);
    let none = egui::Shape::Noop;
    match pane {
        None => {
            ui.painter().set(slots[0], fill(all));
            for slot in &slots[1..] {
                ui.painter().set(*slot, none.clone());
            }
        }
        Some(pane) => {
            let pane = pane.intersect(all);
            // Above, below, left, right.
            ui.painter().set(
                slots[0],
                fill(egui::Rect::from_min_max(all.left_top(), egui::pos2(all.right(), pane.top()))),
            );
            ui.painter().set(
                slots[1],
                fill(egui::Rect::from_min_max(
                    egui::pos2(all.left(), pane.bottom()),
                    all.right_bottom(),
                )),
            );
            ui.painter().set(
                slots[2],
                fill(egui::Rect::from_min_max(
                    egui::pos2(all.left(), pane.top()),
                    egui::pos2(pane.left(), pane.bottom()),
                )),
            );
            ui.painter().set(
                slots[3],
                fill(egui::Rect::from_min_max(
                    egui::pos2(pane.right(), pane.top()),
                    egui::pos2(all.right(), pane.bottom()),
                )),
            );
        }
    }
}

/// The CONTROLS caption and its hint line, which sit between the pads
/// and the faders and so belong to the desk's height.
const CAPTION_H: f32 = 34.0;

/// Where last frame's measured scene-block height lives.
fn scene_h_id() -> egui::Id {
    egui::Id::new("performance-scene-h")
}

/// Below this window height the sections between the status strip and
/// the desk stay stood down once they have been stood down.
///
/// Hysteresis, not a second opinion. Standing the sections down frees
/// exactly the room that would say they can come back, so a single
/// measurement makes the flag oscillate every frame — a screen that
/// flickers between two layouts. This is the "and it is still a short
/// window" half of the test.
const CRAMPED_UNDER: f32 = 780.0;

/// Where the cramped flag lives between frames. See the layout body.
fn cramped_id() -> egui::Id {
    egui::Id::new("performance-cramped")
}

/// Where the peek toggle's state lives between frames.
fn peek_id() -> egui::Id {
    egui::Id::new("performance-peek")
}

pub fn draw(
    ctx: &egui::Context,
    registry: &ParamRegistry,
    state: &PerformanceState<'_>,
    macros: &mut Macros,
) -> PerformanceActions {
    let mut actions = PerformanceActions::default();

    egui::Area::new(egui::Id::new("performance"))
        .fixed_pos([0.0, 0.0])
        .show(ctx, |ui| {
            // Fill the window: an Area is content-sized, so without this
            // the layout collapses to whatever the widgets need and leaves
            // a third of the screen empty.
            let full = ui
                .ctx()
                .input(|i| i.raw.screen_rect)
                .map(|r| r.size())
                .unwrap_or_else(|| vec2(1280.0, 800.0));
            ui.set_min_size(full);
            // A near-opaque scrim between the layout and the live preview
            // it composites over. Without it every label sat directly on
            // the output, and the output is the one thing guaranteed to
            // go bright white at the exact moments this screen matters —
            // a strobe, a bloom — taking the BPM readout, the fader
            // values and the binding chips with it. A few percent still
            // bleeds through, deliberately: the show stays ambiently
            // present without ever competing with the text.
            // Peek: stand the controls aside and let the show through.
            //
            // The output is already rendered underneath this layout — the
            // scrim is the only thing hiding it — so showing it costs
            // nothing but restraint. Which matters, because the one thing
            // a performance screen cannot show you is the performance:
            // every section is full-width and stacked, so the picture you
            // are mixing is behind an opaque sheet of its own controls.
            //
            // Held state rather than momentary. Momentary reads better in
            // a demo and fails in a room: checking your output is not a
            // thing you do for half a second with a finger held down, it
            // is a thing you do while deciding what to do next.
            let peeking = ui.ctx().data_mut(|d| *d.get_temp_mut_or(peek_id(), false));
            actions.peeking = peeking;
            // Whether the window is too short to carry the optional
            // sections *and* the faders.
            //
            // The faders used to take whatever was left over, which made
            // them the first thing to go — and they are the last thing
            // that should. At 1024x640 and 900x700 the entire block
            // including the master fader was laid out below the window
            // and culled: the CONTROLS caption drew with nothing under
            // it, and the dim fader that recovers a black output was
            // simply absent.
            //
            // So the block is reserved and the sections above stand
            // down instead, exactly as they already do while peeking.
            // Measured on the previous frame rather than predicted:
            // predicting it means duplicating the height arithmetic of
            // every section here, which is the kind of second copy that
            // drifts. One frame late is invisible on a resize and the
            // layout settles immediately.
            let cramped = ui.ctx().data_mut(|d| *d.get_temp_mut_or(cramped_id(), false));
            // The scrim is painted *around* the output pane, not over it,
            // so the picture comes through at full strength while every
            // label keeps its opaque ground. Four rects — above, below,
            // left, right — rather than one.
            //
            // Reserved now and filled in at the end: the pane's rect is
            // not known until the fader block has been measured, and a
            // shape added later would paint over the widgets.
            let scrim: Vec<egui::layers::ShapeIdx> = (0..4)
                .map(|_| ui.painter().add(egui::Shape::Noop))
                .collect();
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);

            let inner_w = full.x - PAD * 2.0;
            ui.add_space(PAD * 0.5);
            // How wide the control column is, and so how much is left for
            // the picture.
            //
            // A share rather than a fixed width: the sections have to
            // stay usable, and sixteen pads at their minimum readable
            // size is what sets the floor. Below the width where both
            // can hold, the column takes everything and the pane closes
            // — a two-inch preview beside a cramped grid helps nobody, so
            // a small window goes back to being the single column it was.
            let desk = full.x >= DESK_MIN_W && !peeking;
            let col_w = column_width(inner_w, desk);
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                ui.vertical(|ui| {
                    ui.set_width(inner_w);
                    status_strip(ui, registry, state, &mut actions, inner_w);
                    ui.add_space(10.0);
                    // Where the picture starts, measured rather than
                    // assumed: the strip wraps at narrow widths.
                    let pane_top = ui.cursor().top();
                    // An explicitly allocated region, not a scope with a
                    // width set on it.
                    //
                    // `set_width` is a minimum and `set_max_width` did
                    // not hold either: the layer strip and the gravity
                    // grid both size themselves from `available_width`,
                    // kept reading the parent's, and drew straight across
                    // the output pane. Allocating the rect is the only
                    // form that actually bounds what the children can
                    // see. Zero height means "as tall as the content".
                    ui.allocate_ui_with_layout(
                    egui::vec2(col_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {

                    // Everything between the status strip and the faders
                    // stands down while peeking. Dimming it instead was
                    // the first attempt and it does not work: a dim
                    // sixteen-pad grid is still a sixteen-pad grid over
                    // the picture. The two rows that stay are the two a
                    // hand is already on.
                    if !peeking && !cramped {
                        section(ui, "PUNCH");
                        punch_row(ui, registry, state, &mut actions);
                        ui.add_space(10.0);
                    }

                    if !peeking && !cramped && layer_strip(ui, registry, col_w) {
                        ui.add_space(10.0);
                    }

                    if !state.presets.is_empty() && !peeking && !cramped {
                        section(ui, "PRESETS");
                        preset_row(ui, state, &mut actions, col_w, full.y);
                        ui.add_space(10.0);
                    }

                    });
                    // The bottom of the desk: the pads and the faders, both
                    // full width, under both columns.
                    //
                    // These are the two things played rather than set,
                    // and they belong together — a controller puts its
                    // pads above its faders for the same reason. In a
                    // column the grid also had to wrap to two rows of
                    // eight to keep its names; across the full width
                    // sixteen pads are 80 points each and every name
                    // fits on one line.
                    //
                    // It separates the two grids as well, which colour
                    // alone was only papering over: scenes are on the
                    // desk you play, gravity is in the column you edit.
                    ui.set_width(inner_w);
                    // Whatever vertical space is left goes to the faders,
                    // which are the thing you actually play. Measured from
                    // the cursor rather than guessed, so adding a row above
                    // shortens the faders instead of pushing them off.
                    // Measured before the CONTROLS caption is drawn, so
                    // the caption's height has to be taken off the budget
                    // here — the faders start that much lower than this
                    // number implies.
                    //
                    // It was not, and the block overran the window bottom
                    // by exactly the caption's height: the wells drew,
                    // and all three label lines fell off the bottom edge.
                    // Which looked like "the layout lost the faders" and
                    // was really "the faders are 34 points too low".
                    let used = ui.cursor().top() + if peeking { 0.0 } else { CAPTION_H };
                    // The floor is the real column height — track plus its
                    // three label lines — not a guess. The old 46-point
                    // allowance was 11 short of the labels it was
                    // reserving for, which is exactly the bottom row of
                    // text it pushed off the window.
                    let mut left = (full.y - used - PAD).max(FADER_ABS_MIN + FADER_CHROME + 6.0);
                    if desk {
                        // Same reasoning as peeking, for the same reason:
                        // faders that swell to fill whatever the sections
                        // did not use would eat the picture from below,
                        // and how tall they are would depend on how many
                        // scenes happen to be stored. Fixed share, pinned
                        // to the bottom edge.
                        // How tall the scene block was last frame.
                        //
                        // Its height depends on what is in it — whether
                        // autopilot is armed, whether a pad is waiting —
                        // so it cannot be computed before drawing it, and
                        // the desk has to be positioned before. Last
                        // frame's measurement is right in every frame but
                        // the one where it changes, and wrong by a row
                        // for a sixtieth of a second in that one.
                        let scene_h = ui
                            .ctx()
                            .data_mut(|d| *d.get_temp_mut_or(scene_h_id(), 150.0f32));
                        // What is left once the pads have taken theirs.
                        // Sizing the faders before subtracting the pads
                        // ran them off the bottom edge by exactly the
                        // height of the block that had been added above
                        // them.
                        left = (full.y - PAD - used - scene_h)
                            .max(FADER_ABS_MIN + FADER_CHROME + 6.0)
                            .min(full.y * 0.26);
                        let gap = full.y - PAD - used - left - scene_h;
                        if gap > 0.0 {
                            ui.add_space(gap);
                        }
                    }
                    // The pads, at the top of the desk.
                    let desk_top = ui.cursor().top();
                    if !peeking {
                        let top = desk_top;
                    // The set list, above both grids, and inside the block
                    // `measured` covers — so the gap above the pads shrinks
                    // by the row's height next frame and the faders keep
                    // the room they had.
                    //
                    // Drawn at every size, including the ones where PUNCH
                    // and LAYERS stand down. It was guarded by `cramped`
                    // at first, on the theory that the single-column
                    // layout has no gap to give up and the row would come
                    // straight off the fader budget — but the budget
                    // absorbs it: removing the guard and re-running the
                    // window sweep changed nothing, and it takes about
                    // three hundred points of added height before the
                    // faders are laid out below the window at all. Hiding
                    // the set list on a laptop screen, which is where a
                    // set list is most needed, was a real cost paid
                    // against a theoretical one.
                    deck_row(ui, state, &mut actions);
                    if let Some(gravity) = state.gravity.filter(|_| !peeking) {
                        section(ui, "GRAVITY");
                        // Sixteen empty pads for a layer nobody has touched
                        // is a lot of screen spent saying nothing — but
                        // hiding the row entirely hid its *store* button
                        // too, which was the only way to fill it. That is
                        // not a hidden feature, it is a locked door with
                        // the key inside. Empty draws one teaching row
                        // whose button captures the first pad; the full
                        // grid takes over from there.
                        if gravity.names.iter().all(|n| n.is_none()) {
                            gravity_ghost(ui, &mut actions);
                        } else {
                            let mut bounded = gravity.clone();
                            bounded.width = Some(inner_w);
                            actions.gravity =
                                crate::grid_view::draw_with_id(ui, &bounded, "gravity-grid");
                        }
                        ui.add_space(10.0);
                    }



                        section(ui, "SCENES");
                        let mut scenes = state.grid.clone();
                        scenes.width = Some(inner_w);
                        actions.grid = crate::grid_view::draw(ui, &scenes);
                        ui.add_space(10.0);
                        // Measured to include the caption below, which is
                        // part of the block the gap has to account for.
                        let measured = ui.cursor().top() - top + CAPTION_H;
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(scene_h_id(), measured));
                    }
                    if peeking {
                        // The point of standing the other rows down is to
                        // see the output, and faders that grow to fill the
                        // gap put the controls straight back over it. So
                        // they keep roughly the height they had, and the
                        // space that was freed stays free.
                        //
                        // A third of the window: enough for two rows of
                        // eight at a readable size, which is what the
                        // full layout gives them anyway.
                        left = left.min(full.y * 0.34);
                        // Held at the bottom, so the freed space is one
                        // block in the middle of the screen rather than a
                        // gap under the status strip with the faders
                        // floating in the centre.
                        let gap = full.y - PAD - used - left;
                        if gap > 0.0 {
                            ui.add_space(gap);
                        }
                    }
                    if !peeking {
                        // The caption carries the count controls, because
                        // that is where you are already looking when you
                        // decide there are too few or too many.
                        ui.horizontal(|ui| {
                            section(ui, "CONTROLS");
                            ui.add_space(6.0);
                            let count = macros.count();
                            let minus = ui
                                .add_enabled(
                                    count > vizz_mod::perform::MACRO_MIN,
                                    egui::Button::new(
                                        egui::RichText::new("−").size(12.0).color(INK_3),
                                    )
                                    .small(),
                                )
                                .on_hover_text("one fewer fader — the one on the end goes");
                            if minus.clicked() {
                                actions.fader_count_changed = Some(false);
                            }
                            // The number, so pressing the buttons is not
                            // the only way to know where you are between
                            // the limits.
                            ui.label(
                                egui::RichText::new(format!("{count}"))
                                    .size(11.0)
                                    .monospace()
                                    .color(INK_3),
                            );
                            let plus = ui
                                .add_enabled(
                                    count < vizz_mod::perform::MACRO_MAX,
                                    egui::Button::new(
                                        egui::RichText::new("+").size(12.0).color(INK_3),
                                    )
                                    .small(),
                                )
                                .on_hover_text(format!(
                                    "one more fader — up to {}",
                                    vizz_mod::perform::MACRO_MAX
                                ));
                            if plus.clicked() {
                                actions.fader_count_changed = Some(true);
                            }
                        });
                        // The faders are user-chosen, and the gesture that
                        // chooses them is clicking text that does not look
                        // clickable. One line naming it is the cheapest fix;
                        // the hovers on the label and the "assign"
                        // placeholder say the same thing up close.
                        //
                        // Stands down while peeking, along with the pads:
                        // a line teaching reassignment is the wrong thing
                        // to spend the output's screen on.
                        ui.label(
                            egui::RichText::new(
                                "click a fader's name to reassign it · click its value for a modulator · right-click a fader to reset it",
                            )
                            .size(11.0)
                            .color(INK_4),
                        );
                    }
                    // The budget, measured where the faders actually
                    // begin rather than predicted from further up.
                    //
                    // Every prediction has been wrong by whatever was
                    // added below the measurement afterwards: first the
                    // CONTROLS caption, then the line teaching
                    // reassignment, and then — only in the single-column
                    // layout, because the desk branch subtracts it and
                    // this one did not — the entire scene deck. At
                    // 1024x640 that handed the block 369 points starting
                    // at y=393 of a 640-point window, so all sixteen
                    // faders and the master were laid out under the
                    // window and culled: the caption drew over nothing,
                    // and the dim fader that recovers a black output was
                    // simply absent.
                    //
                    // `left` above still positions the desk block in desk
                    // mode, where the gap is sized so the two agree.
                    // This is the number the faders are actually given.
                    let fader_top = ui.cursor().top();
                    let floor = FADER_ABS_MIN + FADER_CHROME + 6.0;
                    let room = full.y - fader_top - PAD;
                    // Next frame's `cramped`: the sections above stand
                    // down when the room left cannot hold the block.
                    // Hysteresis on the way back, because standing them
                    // down frees exactly the room that would say they
                    // can return — a single test makes it oscillate.
                    ui.ctx().data_mut(|d| {
                        let was: bool = *d.get_temp_mut_or(cramped_id(), false);
                        let starved = room < floor;
                        d.insert_temp(cramped_id(), if was {
                            starved || full.y < CRAMPED_UNDER
                        } else {
                            starved
                        });
                    });
                    faders(ui, registry, macros, state, &mut actions, inner_w, room.max(floor));

                    // The hole the picture comes through.
                    //
                    // In desk mode it is the right column between the
                    // status strip and the faders; while peeking it is
                    // the whole band the stood-down sections vacated.
                    // Otherwise there is no hole and the scrim is one
                    // sheet, exactly as before.
                    let pane = if desk {
                        Some(egui::Rect::from_min_max(
                            egui::pos2(PAD + col_w + PAD, pane_top),
                            egui::pos2(full.x - PAD, desk_top - 6.0),
                        ))
                    } else if peeking {
                        Some(egui::Rect::from_min_max(
                            egui::pos2(PAD, pane_top),
                            egui::pos2(full.x - PAD, full.y - PAD - left - 6.0),
                        ))
                    } else {
                        None
                    };
                    // The picture, drawn into the pane.
                    //
                    // Not a hole in the scrim: a hole shows whatever part
                    // of the full-window render happens to fall behind
                    // it, which is a crop of the output rather than a
                    // view of it. Barely noticeable when the opening is
                    // most of the window — which is why peek got away
                    // with it — and plainly wrong the moment the picture
                    // shares the screen, where it showed the right-hand
                    // half of the frame and nothing else.
                    //
                    // With a texture the pane is scrimmed like everything
                    // else and the image is painted on top, letterboxed
                    // to the output's aspect so a 16:9 master in a wide
                    // pane is a 16:9 picture rather than a stretched one.
                    let drawn = match (pane, state.output_texture) {
                        (Some(pane), Some(id)) => {
                            let fitted = letterbox(pane, state.output_aspect);
                            ui.painter().image(
                                id,
                                fitted,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                            Some(fitted)
                        }
                        _ => None,
                    };
                    // With an image there is nothing to see through, so
                    // the scrim covers everything as it always did.
                    paint_scrim(ui, &scrim, full, pane.filter(|_| drawn.is_none()));
                    if let Some(pane) = drawn.or(pane) {
                        // A hairline, so the pane reads as the output
                        // rather than as space nobody got round to
                        // filling. Faint enough that a dark frame does
                        // not draw a bright box around itself.
                        ui.painter().rect_stroke(
                            pane,
                            2.0,
                            egui::Stroke::new(1.0, PANE_EDGE),
                            egui::StrokeKind::Inside,
                        );
                    }
                });
            });
        });

    actions
}

/// A section rule: a small caps label with a hairline running to the right
/// edge. Cheap, and it turns a flat stack of rows into three named regions
/// you can find without reading them.
/// The gravity row before anything has been put in it.
///
/// One line: what the layer is, and a button that captures the current
/// gravity settings into the first pad. After that press the real
/// sixteen-pad row appears and behaves exactly like the scene row —
/// this exists only to get past the empty state, which previously had
/// no exit at all from any screen.
fn gravity_ghost(ui: &mut egui::Ui, actions: &mut PerformanceActions) {
    ui.horizontal(|ui| {
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new("capture gravity into pad 1")
                        .size(13.0)
                        .color(INK),
                )
                .min_size(vec2(0.0, 28.0))
                .fill(Color32::from_rgb(36, 40, 48))
                .stroke(egui::Stroke::new(1.0, vizz_design::surface::EDGE)),
            )
            .on_hover_text(
                "store the attract/repel settings you have now as gravity 1 — \
                 the full sixteen-pad row appears once a pad is filled",
            )
            .clicked()
        {
            actions.gravity.store = Some(0);
        }
        ui.label(
            egui::RichText::new(
                "the attract / repel layer — shape it in the panel's gravity group, then capture it here",
            )
            .size(12.0)
            .color(INK_3),
        );
    });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .size(10.5)
                .color(INK_3)
                .strong()
                .monospace(),
        );
        let rect = ui.available_rect_before_wrap();
        let y = rect.center().y;
        ui.painter().line_segment(
            [
                egui::pos2(rect.left() + 4.0, y),
                egui::pos2(rect.right(), y),
            ],
            egui::Stroke::new(1.0, vizz_design::surface::HAIRLINE),
        );
    });
    ui.add_space(4.0);
}

/// The address a deck chip binds to. Bindings name a parameter by address
/// rather than by id, since they outlive the process.
const DECK_SELECT: &str = "/deck/select";

/// A preset tile, 16:9 because the picture on it is.
///
/// Wide enough that a name is readable across a table and small enough
/// that four fit a desk column — which is what makes the block a *grid*
/// you scan rather than a list you read.
const TILE_W: f32 = 92.0;
const TILE_H: f32 = 52.0;
/// The gap between tiles, and between rows of them.
const TILE_GAP: f32 = 4.0;
/// How many rows of tiles the block shows before it scrolls, on a window
/// with room for them. Two, and not by taste — see [`block_cap`].
const PRESET_ROWS_SHOWN: f32 = 2.0;
/// And on one without: a row and a quarter, so the cut row says there is
/// more without costing a second one. See [`block_cap`] for where the
/// number comes from — it is measured, not chosen.
const PRESET_ROWS_CRAMPED: f32 = 1.25;

/// Longest deck name a chip will show before it starts shrinking. Wide
/// enough for a song title, narrow enough that eight of them fit a row.
const CHIP_NAME_W: f32 = 96.0;

/// The pages, as a row of chips above the pads.
///
/// Above rather than below, and above *both* grids rather than beside one
/// of them, because it says what every pad underneath means. A row that
/// changes sixteen scene pads and sixteen gravity pads at once, drawn
/// under one of the two, would read as belonging to that one.
///
/// The chips are what a set list looks like: one per song, in the order
/// you play them, with the live one lit. Everything else about a deck —
/// rename, duplicate, delete, which Resolume columns it follows — is on
/// the chip's own menu, because those are things you do at soundcheck and
/// the row has to stay hittable at a metre in the dark.
fn deck_row(ui: &mut egui::Ui, state: &PerformanceState<'_>, actions: &mut PerformanceActions) {
    if state.decks.is_empty() {
        return;
    }
    let editing_id = ui.make_persistent_id("deck-rename");
    let editing: Option<(usize, String)> = ui.data(|d| d.get_temp(editing_id));

    ui.horizontal_wrapped(|ui| {
        for (i, deck) in state.decks.iter().enumerate() {
            let live = i == state.active_deck;
            let (text, size) = fit_label(ui, &deck.name, CHIP_NAME_W);
            let button = egui::Button::new(egui::RichText::new(text).size(size).color(INK))
                .min_size(vec2(0.0, 26.0))
                .fill(if deck.learning {
                    LEARN
                } else if live {
                    vizz_design::surface::RAISED
                } else {
                    // Recessed rather than raised: a row where every chip
                    // stands proud says nothing about which page is
                    // playing, and that is the only thing the row is for.
                    vizz_design::surface::WELL
                })
                // The live chip carries the same blue edge the current
                // scene pad and the recalled preset wear. One colour for
                // "this is the thing you are on", everywhere.
                .stroke(if live {
                    egui::Stroke::new(1.5, crate::theme::CURRENT)
                } else {
                    egui::Stroke::new(1.0, vizz_design::surface::EDGE)
                });
            let response = ui.add(button);
            if response.clicked() && !live {
                actions.decks.select = Some(i);
            }
            let hint = match (&deck.midi, deck.learning) {
                (_, true) => "press a button on your controller".to_string(),
                (Some(s), _) => format!("{}  ·  {s}", deck.name),
                (None, _) => {
                    if state.follow_columns == Some(true) {
                        format!("{}  ·  follows Resolume columns {}–{}", deck.name, deck.origin,
                            deck.origin + crate::grid_view::SLOTS as u32 - 1)
                    } else {
                        deck.name.clone()
                    }
                }
            };
            let response = response.on_hover_text(hint);
            armed_menu(&response, |ui| deck_menu(ui, state, i, editing_id, actions));
        }

        if state.decks.len() < vizz_mod::deck::MAX_DECKS {
            let plus = ui
                .add(egui::Button::new("+").min_size(vec2(26.0, 26.0)))
                .on_hover_text("a new empty page of pads  ·  right-click for the built-in set");
            if plus.clicked() {
                actions.decks.add = true;
            }
            // The way back to the shipped show. It installs itself on a
            // machine that has never had one, so this is for the case that
            // is otherwise a dead end: somebody who cleared their pages and
            // wants it again. Armed, because it replaces every page.
            armed_menu(&plus, |ui| {
                if vizz_design::widgets::armed_button(
                    ui,
                    egui::Id::new("deck-set-armed"),
                    0,
                    vizz_design::widgets::Armed {
                        idle_label: "load the built-in set",
                        armed_label: "replace every page",
                        idle_hover: "twenty songs, eight sections each (asks once)",
                        armed_hover: "every page you have now goes; the looks they name stay",
                        small: false,
                    },
                ) {
                    actions.decks.install_set = true;
                    ui.close();
                }
            });
        }

        // The follow switch sits with the decks rather than with the
        // sequencer controls: what it does is hand the deck's columns to
        // another program, which is a fact about the pages, not about how
        // a blend is shaped.
        if let Some(following) = state.follow_columns {
            ui.add_space(10.0);
            if ui
                .selectable_label(following, egui::RichText::new("resolume").size(13.0))
                .on_hover_text(
                    "follow Arena's column launches: launching a column there fires \
                     the scene and gravity pads of the same number here. Turn on \
                     OSC Output in Arena's preferences and point it at this app.",
                )
                .clicked()
            {
                actions.decks.follow = Some(!following);
            }
        }
    });

    if let Some((slot, text)) = editing {
        deck_rename_row(ui, slot, text, editing_id, actions);
    }
    ui.add_space(6.0);
}

/// A context menu that survives being clicked.
///
/// egui closes a menu on any click inside it, which is right for ordinary
/// items and fatal for an armed one: the first press arms the button and
/// takes the menu away with it, so the second press — the one that
/// actually does the thing — has nothing to land on. The state persists,
/// so re-opening the menu inside the arm window shows a confirm nobody
/// would think to look for; after it lapses, the item is simply inert
/// forever.
///
/// That is not a subtle failure. It made "load the built-in set"
/// impossible to reach by any route, on the one screen it is offered
/// from, and deleting a page equally so.
fn armed_menu(on: &egui::Response, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Popup::context_menu(on)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(contents);
}

/// What a chip offers on a right click. Soundcheck work, deliberately one
/// gesture away from the row you play.
fn deck_menu(
    ui: &mut egui::Ui,
    state: &PerformanceState<'_>,
    index: usize,
    editing_id: egui::Id,
    actions: &mut PerformanceActions,
) {
    let deck = &state.decks[index];
    if ui.button("rename…").clicked() {
        ui.data_mut(|d| d.insert_temp(editing_id, (index, deck.name.clone())));
        ui.close();
    }
    if state.decks.len() < vizz_mod::deck::MAX_DECKS && ui.button("duplicate").clicked() {
        actions.decks.duplicate = Some(index);
        ui.close();
    }
    if state.follow_columns.is_some() {
        ui.separator();
        ui.label(
            egui::RichText::new(format!("resolume columns from {}", deck.origin))
                .size(12.0)
                .color(INK_2),
        );
        // Held in egui's own memory while the drag is live, and handed
        // over when it settles. Emitting on `changed()` wrote the set
        // list to disk on every frame of a drag — a file per frame, on
        // the render thread, for one adjustment.
        let pending = ui.make_persistent_id(("deck-origin", index));
        let mut origin: f32 = ui
            .data(|d| d.get_temp(pending))
            .unwrap_or(deck.origin as f32);
        // A whole number of columns: Resolume has no column 4.5, and a
        // slider that offers one would produce a deck that follows
        // nothing. The top is a full book laid out on the ladder the
        // hover text describes — sixteen pages, sixteen columns apart.
        let top = ((vizz_mod::deck::MAX_DECKS - 1) * crate::grid_view::SLOTS + 1) as f32;
        let slider = ui
            .add(
                egui::Slider::new(&mut origin, 1.0..=top)
                    .step_by(1.0)
                    .clamping(egui::SliderClamping::Always),
            )
            .on_hover_text(
                "which Arena column this page's column 1 follows. Leave every page \
                 at 1 when Arena has one grid; set 1, 17, 33 … when it has one long \
                 one and a song every sixteen columns.",
            );
        if slider.changed() {
            ui.data_mut(|d| d.insert_temp(pending, origin));
        }
        if slider.drag_stopped() || slider.lost_focus() {
            actions.decks.origin = Some((index, origin.round().max(1.0) as u32));
            ui.data_mut(|d| d.remove_temp::<f32>(pending));
        }
    }
    // A deck maps exactly as a scene pad and a preset slot do: the binding
    // names the deck number and the button only says when. A plain binding
    // on `/deck/select` would spread one control across the whole book and
    // land on the last page every time.
    if state.midi.available {
        ui.separator();
        let value = index as f32 + 1.0;
        match (&deck.midi, deck.learning) {
            (Some(s), _) => {
                if ui.button(format!("unmap {s}")).clicked() {
                    actions.clear_slot_binding = Some((DECK_SELECT.to_string(), value));
                    ui.close();
                }
            }
            (None, true) => {
                if ui.button("cancel MIDI learn").clicked() {
                    actions.set_learn_target = Some(None);
                    ui.close();
                }
            }
            (None, false) => {
                if ui.button("MIDI learn").clicked() {
                    actions.set_learn_target = Some(Some(vizz_midi::LearnTarget::value(
                        DECK_SELECT,
                        value,
                        format!("deck {}", index + 1),
                    )));
                    ui.close();
                }
            }
        }
    }
    // The last page cannot go: a show with no decks has nowhere to put a
    // pad, and there would be no chip left to click to make one.
    if state.decks.len() > 1 {
        ui.separator();
        if vizz_design::widgets::armed_button(
            ui,
            egui::Id::new("deck-delete-armed"),
            index as u64,
            vizz_design::widgets::Armed {
                idle_label: "delete page",
                armed_label: "delete for good",
                idle_hover: "throw this page of pads away (asks once)",
                armed_hover: "the pads on this page go; the looks they name stay",
                small: false,
            },
        ) {
            actions.decks.delete = Some(index);
            // Delete is the only gesture that renumbers the pages, so a
            // rename still open is holding a position that will name a
            // different song by the time it is committed.
            ui.data_mut(|d| d.remove_temp::<(usize, String)>(editing_id));
            ui.close();
        }
    }
}

/// Typing a deck's name. A row rather than a popup, for the reason the
/// grid's rename is one: a text field floating over the pads covers the
/// thing you are naming.
fn deck_rename_row(
    ui: &mut egui::Ui,
    index: usize,
    mut text: String,
    editing_id: egui::Id,
    actions: &mut PerformanceActions,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("name").size(12.0).color(INK_2));
        let field = ui.add(
            egui::TextEdit::singleline(&mut text)
                .desired_width(180.0)
                .hint_text("song"),
        );
        // Once, on the frame the row appears. Asking every frame re-grabs
        // focus after egui has released it, so `lost_focus` never became
        // true and Enter did not commit — the only way out was the button.
        let focused = editing_id.with("focused");
        let first = ui.data_mut(|d| {
            let first = !d.get_temp::<bool>(focused).unwrap_or(false);
            d.insert_temp(focused, true);
            first
        });
        if first {
            field.request_focus();
        }
        let commit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if commit || ui.button("ok").clicked() {
            actions.decks.rename = Some((index, text.clone()));
            ui.data_mut(|d| {
                d.remove_temp::<(usize, String)>(editing_id);
                d.remove_temp::<bool>(focused);
            });
        } else if cancel || ui.button("cancel").clicked() {
            ui.data_mut(|d| {
                d.remove_temp::<(usize, String)>(editing_id);
                d.remove_temp::<bool>(focused);
            });
        } else {
            ui.data_mut(|d| d.insert_temp(editing_id, (index, text.clone())));
        }
    });
}

/// Preset tiles: a picture of each look, grouped by what it was built on.
///
/// `width` is the column's, and has to be passed in — see
/// [`column_width`]. Bounded in height as well, because a library is no
/// longer necessarily a handful: installing a set puts a hundred and
/// sixty looks in it, and a wrapped grid of that many is taller than the
/// screen. Scrolling keeps every one of them reachable while the block
/// stays the size the layout budgeted for it.
///
/// **Why pictures.** A preset is a list of numbers under a name somebody
/// typed at 2am, and by the next gig the name is a guess. Recognition
/// beats recall, and the only thing that reliably says what a look is,
/// is the look. Each tile carries what was on the master when the look
/// was saved or last fired — see [`vizz_mod::thumb`].
///
/// **Why groups.** A library sorted by name puts a scan between two
/// attractors. Grouped by family — clouds, shapes, attractors, built-ins
/// — it matches how a set is actually built, and unlike a per-file
/// grouping it survives a night of loading files without turning into
/// nine groups of one. The heading is dropped entirely when there is
/// only one family, because a heading over the whole list says nothing.
///
/// The slot number stays on every tile: it is what `/preset/recall` and
/// therefore a MIDI button addresses. Grouping changes where a look sits
/// on screen, never what fires it.
fn preset_row(
    ui: &mut egui::Ui,
    state: &PerformanceState<'_>,
    actions: &mut PerformanceActions,
    width: f32,
    full_h: f32,
) {
    let present: Vec<vizz_mod::preset::Family> = vizz_mod::preset::Family::ALL
        .iter()
        .copied()
        .filter(|f| state.presets.iter().any(|p| entry_family(p) == *f))
        .collect();
    // One family is not a grouping, it is a caption on the whole list. A
    // fresh install has only built-ins.
    let headed = present.len() > 1;
    // Last frame's content height, because this frame's is not knowable
    // before laying it out and the block has to be *given* a height.
    //
    // A `ScrollArea` sizes itself from the space its parent has left, and
    // this column is an explicitly allocated zero-height region — the
    // only form that stops the layer strip and the gravity grid reading
    // the parent's width. Zero height means the scroll area asked for
    // room and was told there was none, so `max_height` never applied and
    // the block was whatever egui's floor happened to be: measured at 64
    // points, a row and a half however many rows were asked for.
    //
    // Measured rather than modelled, because the content is a wrapped
    // flow of tiles and headings, and any formula for its height would be
    // a second implementation of egui's wrapping — wrong in exactly the
    // cases that matter. One frame late is the same trade `cramped` and
    // the scene block already make further down.
    let id = ui.make_persistent_id("preset-block-h");
    let measured: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    let want = measured.max(TILE_H + TILE_GAP).min(block_cap(full_h));
    ui.allocate_ui_with_layout(
        vec2(width, want),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_max_width(width);
            let out = egui::ScrollArea::vertical()
                .max_height(want)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_max_width(width);
                    ui.spacing_mut().item_spacing = vec2(TILE_GAP, TILE_GAP);
                    // One wrapped flow, headings and all.
                    //
                    // A heading on its own line is easier to scan and
                    // costs a row of the desk each — four families would
                    // spend sixty points on four words, in a block the
                    // faders can only spare about a hundred. Measured:
                    // stacked headings starved them, and the desk answers
                    // that by standing punch, layers and presets down
                    // together. A heading that flows with the tiles it
                    // labels costs nothing and still puts the word where
                    // the group starts.
                    ui.horizontal_wrapped(|ui| {
                        for family in present {
                            if headed {
                                ui.label(
                                    egui::RichText::new(family.label())
                                        .size(9.0)
                                        .monospace()
                                        .color(family_tint(family)),
                                );
                            }
                            for (i, entry) in state.presets.iter().enumerate() {
                                if entry_family(entry) != family {
                                    continue;
                                }
                                preset_tile(ui, state, actions, i, entry, family);
                            }
                        }
                    });
                });
            ui.data_mut(|d| d.insert_temp(id, out.content_size.y));
        },
    );
}

/// The most room the tile block may take, by window height.
///
/// Not a taste decision. The desk stands its entire upper half down —
/// punch, layers and presets together — the moment what is left cannot
/// hold the faders, so a block that asked for more would take punch and
/// layers off the screen with it rather than simply scrolling.
///
/// Measured on a 1440x900 window carrying a deck row and both grids: the
/// faders have about 140 points of slack over the row of text buttons
/// this replaced, and two rows of tiles spend 112 of it. On 1280x720
/// there is room for one row and a glimpse of the next. Fewer looks on
/// screen than the text row showed, and more of them *recognised* —
/// which is the trade the tile is for, and why the block scrolls.
fn block_cap(full_h: f32) -> f32 {
    let rows = if full_h >= CRAMPED_UNDER { PRESET_ROWS_SHOWN } else { PRESET_ROWS_CRAMPED };
    rows * (TILE_H + TILE_GAP)
}

/// What a listed look was built on. A built-in says so itself, whatever
/// the source cache has to say about a user file of the same name.
fn entry_family(entry: &crate::PresetEntry) -> vizz_mod::preset::Family {
    if entry.builtin {
        return vizz_mod::preset::Family::Builtin;
    }
    vizz_mod::preset::family(entry.source.as_deref())
}

/// The colour that stands for a family.
///
/// Chosen apart from the state colours rather than from them: a tile can
/// be recalled (blue), learning (amber) or armed at the same time as it
/// is a cloud, and a family that borrowed one of those would be read as
/// a state. These are hues none of the five states use.
fn family_tint(family: vizz_mod::preset::Family) -> egui::Color32 {
    use vizz_mod::preset::Family;
    match family {
        Family::Cloud => egui::Color32::from_rgb(96, 186, 158),
        Family::Shape => egui::Color32::from_rgb(150, 134, 214),
        Family::Attractor => egui::Color32::from_rgb(206, 114, 168),
        Family::Builtin => egui::Color32::from_rgb(126, 134, 150),
        Family::Unknown => egui::Color32::from_rgb(86, 92, 106),
    }
}

/// One preset, as a tile.
fn preset_tile(
    ui: &mut egui::Ui,
    state: &PerformanceState<'_>,
    actions: &mut PerformanceActions,
    index: usize,
    entry: &crate::PresetEntry,
    family: vizz_mod::preset::Family,
) {
    let slot = index as u32 + 1;
    let name = &entry.name;
    // A preset addresses a slot exactly as a scene pad does, so it maps
    // the same way: the binding names the slot, and the tile only says
    // when. A plain binding on `/preset/recall` would spread a button
    // across a 64-slot range and recall the last one every time.
    let bound = state.midi.map.source_for_value(RECALL, slot as f32);
    let waiting = state.midi.learning_value(RECALL, slot as f32);
    // The recalled slot is the answer to "where did the look on screen
    // come from" — the one tile that should not look like the others.
    let current = state.preset_current == Some(slot as usize);

    let (rect, response) = ui.allocate_exact_size(vec2(TILE_W, TILE_H), egui::Sense::click());
    // Asked before the painter is borrowed: reading a picture wants the
    // context's data map mutably, and a `Ui` cannot lend both at once.
    // Skipped entirely when the tile is scrolled out of sight, which is
    // what keeps a library of a hundred and sixty off the disk.
    let picture = if ui.is_rect_visible(rect) {
        crate::thumbs::texture(ui, name, state.thumb_revision)
    } else {
        None
    };

    if ui.is_rect_visible(rect) {
        let tint = family_tint(family);
        let p = ui.painter();
        // The bed. A look with no picture yet is drawn in its family's
        // colour, deeply dimmed: enough to sort it by eye, not enough to
        // compete with the tiles that do have one.
        let bed = if waiting {
            LEARN
        } else {
            tint.gamma_multiply(0.34).blend(vizz_design::surface::WELL)
        };
        p.rect_filled(rect, 3.0, bed);
        if let Some(tex) = &picture {
            let [w, h] = tex.size();
            let fitted = letterbox(rect, w as f32 / (h.max(1)) as f32);
            p.image(
                tex.id(),
                fitted,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        // The family, as a bar down the leading edge. Over the picture
        // rather than behind it, because a picture fills the tile and a
        // code you can only see on the looks that have no picture is a
        // code for the wrong half of the library.
        p.rect_filled(
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + 3.0, rect.bottom())),
            egui::CornerRadius { nw: 3, ne: 0, sw: 3, se: 0 },
            tint,
        );
        // The name, over the picture rather than under it: under it would
        // cost a fifth of the column for something the picture mostly
        // already says.
        //
        // Two lines when one will not do, and the strip grows to fit. A
        // set list names its looks "01 Song Title Here - Intro", where
        // every distinguishing word is at the *end* — clipped to one
        // line, sixteen tiles in a row read "01 Song Title…" and the row
        // is worse than useless. A fixed two-line strip would instead
        // spend half of every tile on the short names.
        let job = {
            let mut job = egui::text::LayoutJob::simple(
                name.clone(),
                egui::FontId::proportional(10.0),
                INK,
                rect.width() - 12.0,
            );
            job.wrap.max_rows = 2;
            job.wrap.overflow_character = Some('…');
            job
        };
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let p = ui.painter();
        let caption = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - galley.size().y - 4.0),
            rect.max,
        );
        // A scrim under the name, so it stays readable over a white-out
        // frame as well as a dark one.
        p.rect_filled(
            caption,
            egui::CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 },
            egui::Color32::from_black_alpha(190),
        );
        p.galley(
            egui::pos2(caption.left() + 6.0, caption.center().y - galley.size().y * 0.5),
            galley,
            INK,
        );
        // The slot. A badge for the ten that have a key on the keyboard,
        // a plain dim number for the rest — showing the eleventh the same
        // way would promise a shortcut that does not exist, which is the
        // reason the old row showed no number past ten at all.
        let keyed = slot <= 10;
        let corner = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 5.0, rect.top() + 3.0),
            vec2(14.0, 12.0),
        );
        if keyed {
            p.rect_filled(corner, 2.0, egui::Color32::from_black_alpha(190));
        }
        p.text(
            corner.center(),
            egui::Align2::CENTER_CENTER,
            if keyed { (slot % 10).to_string() } else { slot.to_string() },
            egui::FontId::monospace(9.0),
            if keyed { INK } else { vizz_design::ink::FAINT },
        );
        if response.hovered() {
            p.rect_filled(rect, 3.0, egui::Color32::from_white_alpha(16));
        }
        p.rect_stroke(
            rect,
            3.0,
            if current {
                egui::Stroke::new(1.5, crate::theme::CURRENT)
            } else {
                egui::Stroke::new(1.0, vizz_design::surface::EDGE)
            },
            egui::StrokeKind::Inside,
        );
    }

    if response.clicked() {
        actions.preset_slot = Some(slot);
    }
    // The source by name as well as by family: the family says "a cloud",
    // and which cloud is the thing you actually wanted to know.
    let response = if waiting {
        response.on_hover_text("press a button on your controller")
    } else {
        let mut hover = format!("recall {name}");
        if let Some(source) = entry.source.as_deref().filter(|s| *s != "built in") {
            hover = format!("{hover}  ·  on {source}");
        }
        if let Some(s) = &bound {
            hover = format!("{hover}  ·  {}", s.label());
        }
        response.on_hover_text(hover)
    };
    response.context_menu(|ui| {
        // Always offered, MIDI or no MIDI: a picture is the tile's whole
        // point, and a look saved before pictures existed has no other
        // way to get one.
        if ui
            .button("update picture")
            .on_hover_text("photograph what is on the output now")
            .clicked()
        {
            actions.preset_rephoto = Some(name.clone());
            ui.close();
        }
        if !state.midi.available {
            return;
        }
        match (&bound, waiting) {
            (Some(s), _) => {
                if ui.button(format!("unmap {}", s.label())).clicked() {
                    actions.clear_slot_binding = Some((RECALL.to_string(), slot as f32));
                    ui.close();
                }
            }
            (None, true) => {
                if ui.button("cancel MIDI learn").clicked() {
                    actions.set_learn_target = Some(None);
                    ui.close();
                }
            }
            (None, false) => {
                if ui.button("MIDI learn").clicked() {
                    actions.set_learn_target = Some(Some(vizz_midi::LearnTarget::value(
                        RECALL,
                        slot as f32,
                        format!("preset {slot}"),
                    )));
                    ui.close();
                }
            }
        }
    });
}

/// The preset recall address. Bindings name a parameter by address rather
/// than by id, since they outlive the process.
pub(crate) const RECALL: &str = "/preset/recall";

/// The punch row: gestures held, not set. A punch is engaged while the
/// button (or its MIDI note, or Space for flash) is down and gone when it
/// is released — the one family of controls on this screen that does
/// nothing when you let go. Shift-click latches for the moments both
/// hands are elsewhere; a plain click on a latched button releases it.
fn punch_row(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    state: &PerformanceState<'_>,
    actions: &mut PerformanceActions,
) {
    const PUNCH: &[(&str, &str, &str)] = &[
        ("/punch/flash", "FLASH", "white out while held · Space does the same"),
        ("/punch/strobe", "STROBE", "beat-synced strobe while held"),
        ("/punch/black", "BLACK", "black out while held"),
        ("/punch/freeze", "FREEZE", "hold the picture while held"),
        ("/punch/invert", "INVERT", "invert the picture while held"),
    ];
    ui.horizontal(|ui| {
        for (addr, label, hint) in PUNCH {
            punch_button(ui, registry, state, actions, addr, label, hint);
        }
        // The strobe's division lives beside the buttons it shapes. It is
        // transport — the panel's parameter list hides it — so this is
        // its home.
        if let Some(id) = registry.id("/punch/strobe_div") {
            let def = &registry.defs()[id.index()];
            let mut div = registry.target(id);
            if ui
                .add(
                    egui::DragValue::new(&mut div)
                        .speed(0.05)
                        .range(def.min..=def.max)
                        .fixed_decimals(2)
                        .suffix(" beats"),
                )
                .on_hover_text("beats per strobe cycle")
                .changed()
            {
                registry.set(id, div);
            }
        }
    });
}

/// The vector layer strip: one compact row per active stack.
///
/// Follows the gravity grid's rule — drawn only when there is something
/// in it (any layer's kind not "off"), so the default layout spends no
/// height on a feature not in use, and the fader block below keeps its
/// budget. All controls write the registry directly, the way faders do;
/// the labels come from the parameter definitions, so the strip cannot
/// drift from what the shader actually has.
fn layer_strip(ui: &mut egui::Ui, registry: &ParamRegistry, width: f32) -> bool {
    let layer_ids: Vec<_> = (1..=8)
        .map_while(|i| {
            Some((
                registry.id(&format!("/l{i}/kind"))?,
                registry.id(&format!("/l{i}/blend"))?,
                registry.id(&format!("/l{i}/opacity"))?,
                registry.id(&format!("/l{i}/freq"))?,
                registry.id(&format!("/l{i}/color"))?,
            ))
        })
        .collect();
    if layer_ids.is_empty() {
        return false;
    }
    let any_on = layer_ids
        .iter()
        .any(|(kind, ..)| registry.target(*kind).round() >= 0.5);

    // With nothing on, one line rather than nothing at all.
    //
    // The strip used to return early here, so the vector layers were
    // invisible on this screen until one was already running — and the
    // only way to start one was the parameter list on the *other*
    // screen. A feature you cannot reach from the layout you play on,
    // and cannot find out exists from it either.
    //
    // A single line is the whole cost: the layers are worth knowing
    // about and are not worth a full strip of eight off ones. The
    // button starts layer 1 on the first real generator rather than
    // opening something, because one press producing a picture is what
    // teaches that the row means anything.
    if !any_on {
        let (kind, ..) = layer_ids[0];
        let def = &registry.defs()[kind.index()];
        // The first position past "off", whatever it happens to be
        // called — read from the definition rather than hardcoded, so
        // adding a generator at the front cannot silently repoint this.
        let first = def.min + 1.0;
        if first > def.max {
            return false;
        }
        section(ui, "LAYERS");
        ui.horizontal(|ui| {
            let name = def.label_for(first).unwrap_or("a layer");
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new(format!("+ {name}")).size(12.0).color(INK_2),
                    )
                    .min_size(vec2(96.0, 22.0)),
                )
                .on_hover_text(
                    "vector layers draw over the field — start one,                      then right-click its name to choose the generator",
                )
                .clicked()
            {
                registry.set(kind, first);
            }
            ui.label(
                egui::RichText::new("flat shapes over the point field")
                    .size(11.0)
                    .color(INK_4),
            );
        });
        return true;
    }

    section(ui, "LAYERS");
    // How many layers fit on a line at this width.
    //
    // A layer is a swatch, a kind, a blend mode and two numbers — about
    // 250 points — so four of them need a thousand, which is more than
    // the control column has. Left as one row it ran past the column and
    // drew over the output pane beside it; `horizontal_wrapped` does not
    // help, because these widgets size themselves from the width the Ui
    // reports and inside a bounded column that is still the parent's.
    let per_row = ((width / LAYER_W).floor() as usize).max(1);
    for (row, chunk) in layer_ids.chunks(per_row).enumerate() {
    ui.horizontal(|ui| {
        for (col, (kind, blend, opacity, freq, color)) in chunk.iter().enumerate() {
            // Index within the whole strip, not within this row: the
            // number is the layer's identity and wrapping must not
            // renumber it.
            let i = row * per_row + col;
            let kind_def = &registry.defs()[kind.index()];
            let cur = registry.target(*kind).round();
            let on = cur >= 0.5;

            // The ink swatch: the layer's palette slot, read live, so
            // the chip is the colour the layer prints with.
            let slot = registry.target(*color).round().max(0.0) as usize;
            let ink = ["r", "g", "b"]
                .map(|ch| registry.id(&format!("/pal/{slot}/{ch}")).map(|id| registry.target(id)));
            let swatch = match ink {
                [Some(r), Some(g), Some(b)] => Color32::from_rgb(
                    (r * 255.0) as u8,
                    (g * 255.0) as u8,
                    (b * 255.0) as u8,
                ),
                _ => TRACK,
            };
            let (rect, _) = ui.allocate_exact_size(vec2(10.0, 22.0), Sense::hover());
            ui.painter().rect_filled(rect, 2.0, if on { swatch } else { TRACK });

            // Kind: click cycles forward through the generators, right
            // click back — off is on the wheel, so a layer is silenced
            // by cycling to it. The label is the definition's own.
            let kind_label = kind_def.label_for(cur).unwrap_or("?");
            let resp = ui.add(
                egui::Button::new(
                    egui::RichText::new(kind_label)
                        .size(12.0)
                        .color(if on { INK } else { INK_4 }),
                )
                .min_size(vec2(62.0, 22.0)),
            );
            let steps = kind_def.max - kind_def.min + 1.0;
            if resp.clicked() {
                registry.set(*kind, (cur + 1.0) % steps);
            }
            // Right-click lists the generators rather than cycling
            // backwards. Backwards was the cheaper thing to build and
            // the worse thing to have: it is still a wheel, so it still
            // never says what is on the wheel, and a menu subsumes it —
            // anything one click back is one click away here too.
            let resp = resp.on_hover_text(format!(
                "layer {} generator — click to cycle, right-click to choose",
                i + 1
            ));
            resp.context_menu(|ui| {
                stepped_menu(ui, registry, *kind, &format!("layer {}", i + 1))
            });

            if on {
                // Blend, same wheel.
                let blend_def = &registry.defs()[blend.index()];
                let bcur = registry.target(*blend).round();
                let bresp = ui.add(
                    egui::Button::new(
                        egui::RichText::new(blend_def.label_for(bcur).unwrap_or("?"))
                            .size(12.0)
                            .color(INK_2),
                    )
                    .min_size(vec2(66.0, 22.0)),
                );
                let bsteps = blend_def.max - blend_def.min + 1.0;
                if bresp.clicked() {
                    registry.set(*blend, (bcur + 1.0) % bsteps);
                }
                let bresp =
                    bresp.on_hover_text("blend mode — click to cycle, right-click to choose");
                bresp.context_menu(|ui| stepped_menu(ui, registry, *blend, "blend mode"));

                // Opacity and frequency as drags: the two you ride.
                let mut op = registry.target(*opacity);
                if ui
                    .add(egui::DragValue::new(&mut op).speed(0.01).range(0.0..=1.0).fixed_decimals(2))
                    .on_hover_text("opacity")
                    .changed()
                {
                    registry.set(*opacity, op);
                }
                let freq_def = &registry.defs()[freq.index()];
                let mut fr = registry.target(*freq);
                if ui
                    .add(
                        egui::DragValue::new(&mut fr)
                            .speed(0.1)
                            .range(freq_def.min..=freq_def.max)
                            .fixed_decimals(1),
                    )
                    .on_hover_text("frequency")
                    .changed()
                {
                    registry.set(*freq, fr);
                }
            }
            ui.add_space(10.0);
        }
    });
    }
    true
}

/// Every position of a stepped parameter, as a menu.
///
/// The wheel these sit on is fast once you know it and teaches nothing:
/// the label says where you are and never that there is anywhere else to
/// be, and reaching a given generator can take seven clicks through
/// eight positions. The menu is the discoverable half — it names every
/// position at once and marks the current one — and the click-to-cycle
/// stays for the case where you know exactly what you want next.
fn stepped_menu(ui: &mut egui::Ui, registry: &ParamRegistry, id: vizz_params::ParamId, title: &str) {
    let def = &registry.defs()[id.index()];
    let cur = registry.target(id).round();
    ui.label(egui::RichText::new(title).color(INK_3).size(11.0));
    ui.separator();
    let steps = (def.max - def.min).round().max(0.0) as i32;
    for step in 0..=steps {
        let value = def.min + step as f32;
        let Some(label) = def.label_for(value) else { continue };
        if ui
            .selectable_label((cur - value).abs() < 0.5, label)
            .clicked()
        {
            registry.set(id, value);
            ui.close();
        }
    }
}

/// Roughly what one layer's controls need on a line: swatch, kind,
/// blend mode and two numbers.
const LAYER_W: f32 = 250.0;

#[allow(clippy::too_many_arguments)]
fn punch_button(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    state: &PerformanceState<'_>,
    actions: &mut PerformanceActions,
    addr: &str,
    label: &str,
    hint: &str,
) {
    let Some(id) = registry.id(addr) else { return };
    let def = &registry.defs()[id.index()];
    let lit = registry.target(id) > def.max * 0.5;
    let bound = state.midi.map.source_for_value(addr, def.max);
    let waiting = state.midi.learning_value(addr, def.max);

    let latch_id = egui::Id::new(("punch-latch", addr));
    let is_latched: bool =
        lit && ui.ctx().data(|d| d.get_temp(latch_id).unwrap_or(false));
    let (fill, ink) = if waiting {
        (LEARN, crate::theme::ON_LEARN)
    } else if lit {
        // Engaged reads loud: this is the row whose whole job is to be
        // unmissable while it is doing something to the output.
        (vizz_design::surface::ENGAGED, vizz_design::surface::ON_ENGAGED)
    } else {
        (vizz_design::surface::RAISED, INK)
    };
    let response = ui
        .add(
            egui::Button::new(egui::RichText::new(label).size(13.0).strong().color(ink))
                .min_size(vec2(86.0, 34.0))
                .fill(fill)
                .stroke(egui::Stroke::new(1.0, vizz_design::surface::EDGE)),
        )
        .on_hover_text({
            // Held and latched must be tellable apart on screen: a lit
            // button under a finger releases when the finger does, a
            // latched one stays until clicked, and confusing the two is
            // a strobe you cannot stop. The hover names the state; the
            // pip below marks it without hovering.
            let state_note = if is_latched {
                "  ·  LATCHED — click to release"
            } else {
                "  ·  shift-click latches"
            };
            match &bound {
                Some(s) => format!("{hint}{state_note}  ·  {}", s.label()),
                None => format!("{hint}{state_note}"),
            }
        });
    if is_latched {
        // The latch pip: a small ARMED corner square, the one visual
        // that says "this stays on when you let go".
        let r = response.rect;
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(r.right() - 8.0, r.top() + 2.0),
                vec2(6.0, 6.0),
            ),
            1.0,
            crate::theme::ARMED,
        );
    }

    // Hold, don't click: pressed engages, released disengages. The
    // latch flag is decided at press time and consulted at release, so
    // shift can only latch a press it started.
    let held = response.is_pointer_button_down_on();
    let held_id = egui::Id::new(("punch-held", addr));
    let was_held: bool = ui.ctx().data(|d| d.get_temp(held_id).unwrap_or(false));
    let mut latched: bool = ui.ctx().data(|d| d.get_temp(latch_id).unwrap_or(false));
    if held && !was_held {
        let shift = ui.input(|i| i.modifiers.shift);
        if shift && lit {
            // Shift on a lit button unlatches and turns it off.
            registry.set(id, def.min);
            latched = false;
        } else {
            registry.set(id, def.max);
            latched = shift;
        }
    }
    if !held && was_held && !latched {
        // A plain press on a latched button lands here too — press wrote
        // max (a no-op) and cleared nothing, release turns it off. That
        // is "click again to release", for free.
        registry.set(id, def.min);
    }
    ui.ctx().data_mut(|d| {
        d.insert_temp(held_id, held);
        d.insert_temp(latch_id, latched && (lit || held));
    });

    if state.midi.available {
        response.context_menu(|ui| match (&bound, waiting) {
            (Some(s), _) => {
                if ui.button(format!("unmap {}", s.label())).clicked() {
                    actions.clear_slot_binding = Some((addr.to_string(), def.max));
                    ui.close();
                }
            }
            (None, true) => {
                if ui.button("cancel MIDI learn").clicked() {
                    actions.set_learn_target = Some(None);
                    ui.close();
                }
            }
            (None, false) => {
                if ui.button("MIDI learn").clicked() {
                    actions.set_learn_target = Some(Some(vizz_midi::LearnTarget::value(
                        addr,
                        def.max,
                        label.to_lowercase(),
                    )));
                    ui.close();
                }
            }
        });
    }
}

fn status_strip(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    state: &PerformanceState<'_>,
    actions: &mut PerformanceActions,
    width: f32,
) {
    ui.horizontal(|ui| {
        // First on the strip, before the output lights: it says which
        // show every pad, look and patch below belongs to, and that has
        // to be readable before any of them are.
        crate::project_bar::chip(ui, state.project, &mut actions.project);
        ui.add_space(12.0);
        for out in state.outputs {
            let (r, _) = ui.allocate_exact_size(vec2(11.0, 11.0), Sense::hover());
            ui.painter()
                .circle_filled(r.center(), 5.0, if out.live { LIVE } else { DEAD });
            // A dead output is the thing you most need to notice, so it
            // gets the *brighter* treatment of the two, not the dimmer:
            // greying out what has failed is exactly backwards.
            ui.label(
                egui::RichText::new(&out.name)
                    .size(13.0)
                    .color(if out.live { INK_2 } else { WARN }),
            );
            ui.add_space(10.0);
        }
        if state.outputs.is_empty() {
            ui.label(egui::RichText::new("no outputs").size(13.0).color(WARN));
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new(
                    egui::RichText::new("edit").size(13.0).color(INK_2),
                ))
                .on_hover_text("back to the control panel  (P)")
                .clicked()
            {
                actions.exit = true;
            }
            ui.add_space(12.0);

            // The way back to the picture. Next to "edit" because they
            // are the same kind of decision — which of the three things
            // this window can show am I looking at — and a toggle whose
            // state you cannot see is a toggle you press twice.
            let peeking = ui.ctx().data_mut(|d| *d.get_temp_mut_or(peek_id(), false));
            let peek = ui.add(egui::Button::new(
                egui::RichText::new("view")
                    .size(13.0)
                    .color(if peeking { LIVE } else { INK_2 }),
            ));
            if peek
                .on_hover_text(
                    "stand the controls aside and watch the output  (V)\n\
                     the punch, scene and preset rows come back when you switch it off",
                )
                .clicked()
                || ui.ctx().input(|i| i.key_pressed(egui::Key::V))
            {
                ui.ctx().data_mut(|d| d.insert_temp(peek_id(), !peeking));
            }
            ui.add_space(12.0);

            // Frame health. Fixed width so the row does not twitch as the
            // number changes width, and coloured because at a glance you
            // need "is it fine", not a percentile.
            let (r, _) = ui.allocate_exact_size(vec2(64.0, 18.0), Sense::hover());
            ui.painter().text(
                r.right_center(),
                egui::Align2::RIGHT_CENTER,
                format!("{:.0} fps", state.fps),
                egui::FontId::proportional(15.0),
                if state.over_budget { WARN } else { LIVE },
            );
            ui.add_space(16.0);

            if ui
                .add(egui::Button::new(
                    egui::RichText::new("tap").size(13.0).color(INK),
                ))
                .on_hover_text("tap the beat — three taps set the tempo and switch auto off")
                .clicked()
            {
                actions.tapped = true;
            }
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!("{:.1} bpm", state.bpm))
                    .size(15.0)
                    .color(INK),
            )
            // Where the tempo comes from, and where to change that. The
            // MIDI-clock switch lives in the panel's audio section, and
            // naming it here is the difference between a feature that
            // exists and one anybody finds.
            .on_hover_text(if state.audio.clock_midi {
                "following MIDI clock — panel ▸ audio ▸ midi clock to go back to the internal one"
            } else {
                "internal clock — tap to set it, or panel ▸ audio ▸ midi clock to follow your mixer"
            });
            // Recording, both ways round. This chip used to appear only
            // once a take was already running, which made it a stop
            // button wearing a record button's name: the only way to
            // *start* was to leave this screen, open the panel and
            // expand a setup section that is collapsed by default. A
            // take you cannot begin from the screen you play on is a
            // take that does not get begun.
            if let Some(id) = registry.id("/record/active") {
                let (text, fill, ink, hover) = match &state.recording {
                    Some(rec) => (
                        format!(
                            "REC {}:{:02} · {}f{}",
                            rec.secs / 60,
                            rec.secs % 60,
                            rec.frames,
                            if rec.dropped > 0 {
                                format!(" · {} dropped", rec.dropped)
                            } else {
                                String::new()
                            }
                        ),
                        vizz_design::accent::REC,
                        Color32::WHITE,
                        "recording the master — click to stop",
                    ),
                    // Idle sits dark with the word in a dimmed red, so it
                    // reads as armed-and-waiting rather than as another
                    // status light, and cannot be mistaken at a glance
                    // for a take in progress.
                    None => (
                        "REC".to_string(),
                        vizz_design::accent::REC_BED,
                        vizz_design::accent::REC_INK,
                        "record the master as a PNG sequence — click to start",
                    ),
                };
                let chip = ui.add(
                    egui::Button::new(egui::RichText::new(text).size(13.0).strong().color(ink))
                        .fill(fill),
                );
                if chip.on_hover_text(hover).clicked() {
                    registry.set(id, if state.recording.is_some() { 0.0 } else { 1.0 });
                }
            }
            if state.audio.clock_midi {
                // Following the wire — or supposed to be. Green while
                // ticks arrive, warning-amber while the wire is silent
                // and the clock is running free on its last tempo.
                let (word, colour) = if state.audio.clock_ticking {
                    ("MIDI", LIVE)
                } else {
                    ("MIDI?", WARN)
                };
                ui.label(egui::RichText::new(word).size(13.0).strong().color(colour));
            }
            // Beat indicator: brightest on the downbeat, so tempo is
            // visible without reading a number.
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            let glow = (1.0 - state.bar_phase * 4.0).clamp(0.0, 1.0);
            ui.painter().circle_filled(
                r.center(),
                7.5,
                Color32::from_rgb(
                    (64.0 + 191.0 * glow) as u8,
                    (64.0 + 156.0 * glow) as u8,
                    (78.0 + 30.0 * glow) as u8,
                ),
            );
        });
    });

    audio_strip(ui, state.audio, width);
}

/// Audio across the full width, labelled.
///
/// This used to be four unlabelled 46-point stubs in the corner. When the
/// visuals stop reacting the first question is whether audio is still
/// arriving and at what level — which the stubs could not answer, because
/// nothing said which band was which or what "full" looked like.
fn audio_strip(ui: &mut egui::Ui, audio: &AudioView, width: f32) {
    const BANDS: [&str; 4] = ["low", "lo-mid", "hi-mid", "high"];
    ui.horizontal(|ui| {
        if !audio.connected {
            ui.label(
                egui::RichText::new("audio: not connected")
                    .size(12.0)
                    .color(WARN),
            );
            return;
        }
        let name = audio.device.as_deref().unwrap_or("input");
        ui.label(egui::RichText::new(name).size(12.0).color(INK_3));
        ui.add_space(8.0);

        // Meters share out the width rather than taking a fixed size, so
        // the strip stays proportionate on a wide display.
        // Wide enough to read a level off, not so wide the meters become
        // the most prominent thing on a screen where they are diagnostic.
        let each = ((width - 160.0) / 4.0).clamp(60.0, 130.0);
        for (i, label) in BANDS.iter().enumerate() {
            let v = audio.bands[i].clamp(0.0, 1.0);
            let (r, _) = ui.allocate_exact_size(vec2(each, 12.0), Sense::hover());
            ui.painter().rect_filled(r, 2.0, TRACK);
            // The track was 15 RGB points off the background — invisible,
            // so a silent band vanished entirely and a fill's length had
            // nothing to be judged against. The hairline is the ruler.
            ui.painter().rect_stroke(
                r,
                2.0,
                (1.0, vizz_design::surface::TICK),
                egui::StrokeKind::Inside,
            );
            ui.painter().rect_filled(
                egui::Rect::from_min_size(r.left_top(), vec2(r.width() * v, r.height())),
                2.0,
                // Warm at the top of the range: a band pinned at 1.0 is
                // clipping its modulation and should not look healthy.
                if v > 0.97 { WARN } else { LIVE },
            );
            ui.painter().text(
                r.left_center() + vec2(5.0, 0.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(10.0),
                if v > 0.35 {
                    Color32::from_rgb(18, 26, 20)
                } else {
                    INK_3
                },
            );
            ui.add_space(4.0);
        }
    });
}

fn faders(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    macros: &mut Macros,
    state: &PerformanceState<'_>,
    actions: &mut PerformanceActions,
    width: f32,
    height: f32,
) {
    // How many rows the height can actually carry.
    //
    // Two rows of eight need `2 * (FADER_MIN_H + chrome)` plus spacing;
    // below that the second row was simply painted outside the window —
    // an `Area` at a fixed position with no scroll area and no clipping,
    // so slots 9-16 were not drawn at all, with no scrollbar and nothing
    // on screen to say eight assigned faders existed. At the app's own
    // default of 1280x720 with the gravity grid shown, that was the
    // normal case.
    //
    // One wide row of sixteen instead. Narrower columns are a real cost,
    // but a fader you can see and hit badly beats one that is not there —
    // and scrolling is not an answer on stage.
    // Two rows survive on shrunken faders down to the absolute floor —
    // reflowing to one row of seventeen moves every fader's position and
    // (before the master was anchored) pushed the rightmost off narrow
    // windows. The single row is the last resort for genuinely short
    // windows, not the response to a busy screen.
    let two_rows = 2.0 * (FADER_ABS_MIN + FADER_CHROME + 6.0) + 8.0;
    // The set's own count, not a constant: how many faders there are is
    // a saved preference now.
    let count = macros.count();
    // How many columns the width can actually carry at the minimum
    // readable fader width — the master's column included, since it is
    // laid out as one of them.
    //
    // Rows used to be decided by height alone. That was survivable while
    // the count was fixed at sixteen and quietly broken the moment it
    // could be raised: at twenty-four on a 1280-point window the height
    // check said one row, twenty-five columns at the 62-point floor
    // needed 1694 points of a 1252-point lane, and six faders were laid
    // out past the right edge of the window. They existed, they answered
    // MIDI, and nobody could see them.
    let fits_across = columns_that_fit(width);
    // One row, always, whenever the width can carry it.
    //
    // Two banks split the one gesture this screen exists for: your eye
    // has to find which bank a fader is in before it can find the fader,
    // and the answer changes as the count does. A single axis is worth
    // more than the width each column gives up for it — which is why
    // FADER_MIN_W is 36 and not 62. Twenty-four faders plus the master
    // is twenty-five columns, so one row needs a lane of 25 × 36 + 24 ×
    // 6 = 1044 points, about a 1076-point window. Narrower than that and
    // it wraps rather than running off the screen, because a fader you
    // cannot see is worse than a fader in the wrong bank.
    // Width alone decides. An earlier version added a height "veto" that
    // forced a second row when the block was short — which is exactly
    // backwards: a short block is the case with the least room for two
    // rows, and it pushed the faders off the bottom of the window.
    let rows = fader_rows_for(count, width);
    let _ = (fits_across, two_rows);
    let per_row = count.div_ceil(rows);
    // The master rides in the last row as one more column, so it is the
    // same size as everything else rather than a full-width slab.
    let cols = per_row + 1;
    let w = ((width - (cols as f32 - 1.0) * 6.0) / cols as f32).clamp(FADER_MIN_W, FADER_MAX_W);
    // When the columns hit their width cap, centre the block instead of
    // leaving everything parked against the left edge with the rest of
    // the screen as dead backdrop.
    let used = cols as f32 * w + (cols as f32 - 1.0) * 6.0;
    let inset = ((width - used) * 0.5).max(0.0);
    let width = used.min(width);
    let chrome = FADER_CHROME;
    // Shrinks below the preferred height rather than overflowing: the old
    // floor of FADER_MIN_H here forced 96-point tracks into rows that had
    // no room for them, which is what clipped the name and binding lines
    // off the bottom of the window.
    let h = ((height / rows as f32) - chrome - 6.0).clamp(FADER_ABS_MIN, f32::MAX);

    for row in 0..rows {
        // Laid out from explicit rects rather than a horizontal flow. A
        // flow child grows to its content, so "reserve the master's
        // column" could not actually hold against an overflowing row —
        // the master still got pushed off the right edge, and the master
        // is the dim fader that recovers a black output, wearing the
        // blackout warning nobody could see. With rects, the master is
        // right-anchored unconditionally and an overflowing row clips its
        // *rightmost macros* against the master's ground instead.
        let origin = ui.cursor().min + vec2(inset, 0.0);
        ui.allocate_rect(
            egui::Rect::from_min_size(ui.cursor().min, vec2(width + inset, h + chrome)),
            Sense::hover(),
        );

        let macro_width = if row == 0 { (width - w - 6.0).max(w) } else { width };
        let macros_rect =
            egui::Rect::from_min_size(origin, vec2(macro_width, h + chrome));
        let mut lane = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(macros_rect)
                .layout(egui::Layout::left_to_right(egui::Align::TOP)),
        );
        // Tight label stacking inside each column. The default 6-point
        // gap is right for a form and far too loose for three lines that
        // belong to one control.
        lane.spacing_mut().item_spacing = vec2(6.0, LABEL_GAP);
        lane.set_clip_rect(lane.clip_rect().intersect(macros_rect));
        for col in 0..per_row {
            let slot = row * per_row + col;
            if slot >= count {
                break;
            }
            lane.allocate_ui_with_layout(
                vec2(w, h + chrome),
                egui::Layout::top_down(egui::Align::Center),
                |ui| fader(ui, registry, macros, slot, state, actions, w, h),
            );
        }
        // Master at the right edge of the first row: same footprint as
        // its neighbours, distinct only by colour and label. It is found
        // by position, which is what actually works in the dark.
        if row == 0 {
            // Inset by the same gap that separates every other column,
            // rather than pinned flush to the block's right edge.
            //
            // The master's labels are laid out from the column's centre
            // line rightwards rather than centred on it — an egui
            // layout quirk this code has not managed to talk it out of —
            // so the caption reaches about half a column past where it
            // looks like it should. Flush against the edge that put
            // "MASTER" four points outside a 1024-point window, where it
            // was clipped away entirely and left the master looking like
            // a nameless fader on the end of the row. The gutter is
            // where the overhang goes.
            let master_rect = egui::Rect::from_min_size(
                egui::pos2(origin.x + width - w - COL_GAP, origin.y),
                vec2(w, h + chrome),
            );
            let mut col = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(master_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            col.spacing_mut().item_spacing = vec2(6.0, LABEL_GAP);
            // The column needs a width for `Align::Center` to centre
            // anything within. Built from `max_rect` alone it had none,
            // so every label in the master column was laid out with its
            // *left edge* on the column's centre line and grew right —
            // half of each one hanging outside its own column at every
            // window size. Nothing showed it until the window was narrow
            // enough that the overhang crossed the screen edge, where
            // egui clipped the caption away and left the master looking
            // like a nameless fader on the end of the row.
            master(&mut col, registry, state, w, h);
        }
        ui.add_space(4.0);
    }
}

#[allow(clippy::too_many_arguments)]
fn fader(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    macros: &mut Macros,
    slot: usize,
    state: &PerformanceState<'_>,
    actions: &mut PerformanceActions,
    w: f32,
    h: f32,
) {
    let assigned = macros.get(slot).map(str::to_owned);
    let id = assigned.as_deref().and_then(|a| registry.id(a));

    match (assigned.as_deref(), id) {
        (Some(addr), Some(param)) => {
            let def = &registry.defs()[param.index()];
            let value = registry.target(param);
            // Where modulation has actually put it. Drawn as a second mark
            // so a moving parameter does not look like a stuck fader.
            let live = state.values.and_then(|v| v.get(param.index()).copied());
            let modulated = live.filter(|l| (l - value).abs() > (def.max - def.min) * 0.01);

            if let Some(v) = vertical_fader(
                ui, value, modulated, def, w, h, slot, FILL, FILL_BRIGHT, state.midi,
            ) {
                registry.set(param, v);
            }
            let value = registry.target(param);

            // A stepped parameter shows its position's name. `2.000` under
            // a fader called `mirror` is legible and still tells you
            // nothing.
            let shown = def
                .label_for(value)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{value:.2}"));
            // The short name identifies the fader; the full address is only
            // needed when reassigning, so it lives in the tooltip — unless
            // another fader ends in the same word. The shipped layout has
            // /color/spread and /particles/spread, and two faders both
            // labelled "spread" cannot be told apart in the dark, which is
            // the one job this screen has.
            let short = addr.rsplit('/').next().unwrap_or(addr);
            let clash = macros
                .slots
                .iter()
                .flatten()
                .any(|a| a != addr && a.rsplit('/').next() == Some(short));
            let shown_name = if clash {
                addr.trim_start_matches('/').replace('/', " ")
            } else {
                short.to_string()
            };
            // Qualified names are long, and a fader column is narrow: at
            // the shipped fourteen across a 1280 window "particles
            // spread" ellipsised to "particles …", which is the group
            // word — the only part that distinguishes it from "color
            // spread" — surviving while the part that says what the
            // fader *does* is thrown away. Shrink to fit instead, taking
            // the largest step that measures inside the column. A label
            // one stop down is legible; a label ending in an ellipsis is
            // a label the room has to guess at.
            let (shown_name, size) = fit_label(ui, &shown_name, w);
            // The name leads, and is the brightest thing in the column.
            //
            // It used to sit second and dimmer, under the number. But the
            // bar has already answered "how much" — that is the entire
            // job of the column of light above it — so the line touching
            // the bar should answer "of what". Identity is the one thing
            // about a fader you cannot read off the picture, and it was
            // the one thing set in the quietest type.
            // A button rather than a bare label. It has always been
            // clickable and never looked it: the one line teaching
            // reassignment lives under the whole block, so on a screen
            // you are meant to read at a glance the gesture was
            // discoverable only by being told. A frame costs nothing
            // and says "press me" without a word.
            let name_btn = ui.add(
                egui::Button::new(egui::RichText::new(shown_name).size(size).color(INK))
                    .frame(true)
                    .min_size(vec2(w, 0.0)),
            );
            let at = name_btn.rect.left_bottom();
            if name_btn
                .on_hover_text(format!("{addr}  —  click to reassign"))
                .clicked()
            {
                open_assign(ui, slot, at);
            }
            // Then the number, which is the confirmation rather than the
            // headline — except when something else is moving it, where
            // amber makes it the thing that catches the eye.
            //
            // And it is also the way in to *what* is moving it. The
            // number is the right handle for that: it is the line that
            // already goes amber when the parameter is being driven,
            // so "click the amber number to change what is driving it"
            // needs no new chrome on a column that is 36 points wide at
            // its narrowest. The track cannot take it — right-click
            // there already restores the default — and a fourth label
            // line would come off the fader's own height.
            let shape = state.graph.and_then(|g| vizz_mod::shapes::attached(g, addr));
            let driven = state
                .graph
                .map(|g| vizz_mod::shapes::driven(g, addr))
                .unwrap_or(false);
            // Amber whenever something is attached, not only while it
            // happens to be off zero. An envelope between hits outputs
            // nothing, and a fader that only admits to being modulated
            // on the frames it is moving cannot be read at all.
            let number = egui::RichText::new(shown)
                .size(11.0)
                .monospace()
                .color(if modulated.is_some() || driven { MOD } else { INK_2 });
            if state.graph.is_some() {
                let hint = match (shape, driven) {
                    (Some(i), _) => format!(
                        "{} — {}\nclick to change it",
                        vizz_mod::shapes::SHAPES[i].name,
                        vizz_mod::shapes::SHAPES[i].about
                    ),
                    // Driven by something this menu did not build: the
                    // canvas can wire anything, and calling that "none"
                    // would be the fader denying what it is visibly doing.
                    (None, true) => {
                        "modulated from the canvas\nclick to replace it with a ready-made one"
                            .to_string()
                    }
                    (None, false) => "no modulator — click to add one".to_string(),
                };
                if ui
                    .add(egui::Label::new(number).sense(Sense::click()))
                    .on_hover_text(hint)
                    .clicked()
                {
                    open_mod(ui, slot);
                }
            } else {
                ui.label(number);
            }
            midi_chip(ui, state, actions, addr);
            mod_popup(ui, addr, slot, shape, actions);
        }
        _ => {
            // Unassigned, or pointing at a parameter this build no longer
            // has: draw an inert placeholder rather than hiding the slot,
            // so the layout does not reflow mid-set.
            //
            // Recessive on purpose. Eight empty slots drawn as solid blocks
            // dominate the lower half of the screen and read as eight
            // broken faders; an outline the same width as a real track
            // reads as room for more, which is what it is.
            let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
            let track = r.shrink2(vec2(r.width() * 0.12, 0.0));
            ui.painter().rect_filled(track, 5.0, PANEL_BG);
            ui.painter().rect_stroke(
                track,
                5.0,
                egui::Stroke::new(1.0, vizz_design::surface::HAIRLINE),
                egui::StrokeKind::Inside,
            );
            ui.label(egui::RichText::new("—").size(13.0).color(INK_4));
            let assign_btn = ui.add(
                egui::Button::new(egui::RichText::new("assign").size(13.0).color(INK_3))
                    .frame(true)
                    .min_size(vec2(w, 0.0)),
            );
            let at = assign_btn.rect.left_bottom();
            if assign_btn
                .on_hover_text("choose what this fader controls")
                .clicked()
            {
                open_assign(ui, slot, at);
            }
            ui.label(egui::RichText::new(" ").size(11.0));
        }
    }

    assign_popup(ui, registry, macros, slot, actions);
}

/// The label a fader wears, and the size that fits it in the column.
///
/// Faders normally carry one word — "glow", "twist" — and it fits at
/// full size. The awkward case is a name qualified by its group, worn
/// when another fader ends in the same word: "particles spread" beside
/// "color spread", where the group word is the whole point and dropping
/// it makes the two indistinguishable.
///
/// Ellipsising is the wrong answer because it cuts from the right, and
/// the right is the word saying what the fader *does*: "particles
/// spread" reaching the screen as "particles …". So the size steps down
/// first, and if the column is narrower than the label at any legible
/// size, the *group* is abbreviated instead — three letters still tells
/// "par." from "col.", and no current group collides in three.
///
/// Falls through to the smallest abbreviated form rather than failing;
/// at that point the column is narrower than any label and this is the
/// variant that loses least.
fn fit_label(ui: &egui::Ui, name: &str, w: f32) -> (String, f32) {
    const STEPS: [f32; 4] = [13.0, 11.5, 10.0, 9.0];
    let fits = |text: &str, size: f32| {
        ui.painter()
            .layout_no_wrap(text.to_string(), egui::FontId::proportional(size), INK_2)
            .rect
            .width()
            <= w
    };
    for size in STEPS {
        if fits(name, size) {
            return (name.to_string(), size);
        }
    }
    // Only a qualified name has a group to give up.
    let Some((group, rest)) = name.split_once(' ') else {
        return (name.to_string(), STEPS[STEPS.len() - 1]);
    };
    let short_group: String = group.chars().take(3).collect();
    let abbreviated = format!("{short_group}. {rest}");
    for size in STEPS.into_iter().skip(1) {
        if fits(&abbreviated, size) {
            return (abbreviated, size);
        }
    }
    (abbreviated, STEPS[STEPS.len() - 1])
}

/// The MIDI binding, or the way to make one.
///
/// Learn belongs here rather than only on the panel's parameter rows: the
/// faders are what a controller is mapped *to*, so binding one should not
/// mean leaving the screen you are binding it for.
fn midi_chip(
    ui: &mut egui::Ui,
    state: &PerformanceState<'_>,
    actions: &mut PerformanceActions,
    addr: &str,
) {
    if !state.midi.available {
        // Keep the row's height so faders with and without MIDI line up.
        ui.label(egui::RichText::new(" ").size(11.0));
        return;
    }
    let learning = state.midi.learning(addr);
    match state.midi.map.source_for(addr) {
        Some(source) => {
            if ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(source.label())
                            .size(11.0)
                            .monospace()
                            .color(INK_3),
                    )
                    .sense(Sense::click()),
                )
                .on_hover_text("click to clear this MIDI binding")
                .clicked()
            {
                actions.clear_binding = Some(addr.to_string());
            }
        }
        None if learning => {
            if ui
                .add(
                    egui::Label::new(egui::RichText::new("waiting").size(11.0).color(LEARN))
                        .sense(Sense::click()),
                )
                .on_hover_text("move a control on your controller, or click to cancel")
                .clicked()
            {
                actions.set_learn_target = Some(None);
            }
        }
        None => {
            if ui
                .add(
                    egui::Label::new(egui::RichText::new("learn").size(11.0).color(INK_4))
                        .sense(Sense::click()),
                )
                .on_hover_text("bind the next control you move to this fader")
                .clicked()
            {
                actions.set_learn_target = Some(Some(vizz_midi::LearnTarget::param(addr)));
            }
        }
    }
}

/// The ready-made modulators, on the fader they would drive.
///
/// A list rather than a submenu tree: there are fourteen, they are read
/// in a dark room, and every one of them is one line. The current pick
/// is selected, so the popup doubles as the answer to "what is on this
/// fader" — which is the question you ask before you change it.
fn mod_popup(
    ui: &mut egui::Ui,
    addr: &str,
    slot: usize,
    current: Option<usize>,
    actions: &mut PerformanceActions,
) {
    if !is_mod_open(ui, slot) {
        return;
    }
    let popup_id = egui::Id::new(("mod", slot));
    let mut chosen: Option<Option<usize>> = None;
    let mut close = false;
    egui::Area::new(popup_id.with("area"))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_height(360.0);
                ui.set_min_width(230.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(addr.rsplit('/').next().unwrap_or(addr)).color(INK),
                    );
                    if ui.small_button("close").clicked() {
                        close = true;
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt(popup_id)
                    .show(ui, |ui| {
                        if ui.selectable_label(current.is_none(), "— none —").clicked() {
                            chosen = Some(None);
                        }
                        for (i, s) in vizz_mod::shapes::SHAPES.iter().enumerate() {
                            // The swing is worth saying out loud: a
                            // bipolar shape moves the fader either side
                            // of where it is set, a unipolar one only
                            // pushes up from it. That is the difference
                            // between a fader you can still park at the
                            // top and one you cannot, and it is not
                            // recoverable from the name.
                            let label = format!(
                                "{}   {}",
                                s.name,
                                if s.bipolar { "±" } else { "+" }
                            );
                            if ui
                                .selectable_label(current == Some(i), label)
                                .on_hover_text(s.about)
                                .clicked()
                            {
                                chosen = Some(Some(i));
                            }
                        }
                    });
            });
        });
    if let Some(pick) = chosen {
        actions.set_mod_shape = Some((addr.to_string(), pick));
        close = true;
    }
    if close {
        close_mod(ui, slot);
    }
}

fn mod_key(slot: usize) -> egui::Id {
    egui::Id::new(("mod-open", slot))
}
fn open_mod(ui: &egui::Ui, slot: usize) {
    ui.memory_mut(|m| m.data.insert_temp(mod_key(slot), true));
}
fn close_mod(ui: &egui::Ui, slot: usize) {
    ui.memory_mut(|m| m.data.insert_temp(mod_key(slot), false));
}
fn is_mod_open(ui: &egui::Ui, slot: usize) -> bool {
    ui.memory(|m| m.data.get_temp::<bool>(mod_key(slot)).unwrap_or(false))
}

fn assign_popup(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    macros: &mut Macros,
    slot: usize,
    actions: &mut PerformanceActions,
) {
    let popup_id = egui::Id::new(("assign", slot));
    if !is_assign_open(ui, slot) {
        return;
    }
    let mut chosen: Option<Option<String>> = None;
    let mut close = false;
    let filter_id = popup_id.with("filter");
    let mut filter: String = ui
        .ctx()
        .data_mut(|d| d.get_temp(filter_id))
        .unwrap_or_default();
    // Anchored under the fader that opened it, rather than wherever egui
    // would park a free Area. A picker for *this* fader that appears
    // somewhere else makes you check which one you are editing, and the
    // whole point of the layout is not having to look twice.
    let anchor = ui
        .ctx()
        .data_mut(|d| d.get_temp::<egui::Pos2>(popup_id.with("at")))
        .unwrap_or_else(|| ui.cursor().min);
    let mut area = egui::Area::new(popup_id.with("area")).order(egui::Order::Foreground);
    // Kept on screen: a fader near the right edge would otherwise open
    // its picker half off the window, and the faders at the edges are
    // the ones a wide set puts there.
    // Same source the layout itself measures from, a few hundred lines
    // up: `raw.screen_rect` rather than a derived one, so the picker and
    // the desk cannot disagree about where the window ends.
    let screen = ui
        .ctx()
        .input(|i| i.raw.screen_rect)
        .unwrap_or_else(|| ui.ctx().globally_used_rect());
    let at = egui::pos2(
        anchor.x.min(screen.right() - POPUP_W - PAD),
        anchor.y.min(screen.bottom() - POPUP_H - PAD),
    );
    area = area.fixed_pos(at);
    area.show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_max_height(POPUP_H);
            ui.set_min_width(POPUP_W);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("fader {}", slot + 1)).color(INK));
                if ui.small_button("close").clicked() {
                    close = true;
                }
            });
            // Typing beats scrolling once the list is longer than a
            // screen, and this one is every parameter in the app. The
            // field takes focus on the frame the popup opens, so the
            // gesture is click-then-type rather than click, aim, click.
            let field = ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .hint_text("type to narrow")
                    .desired_width(f32::INFINITY),
            );
            if ui.ctx().data_mut(|d| {
                let first: bool = !*d.get_temp_mut_or(popup_id.with("focused"), false);
                d.insert_temp(popup_id.with("focused"), true);
                first
            }) {
                field.request_focus();
            }
            // Enter takes the only remaining match, so a unique prefix
            // is a complete gesture without the pointer coming back.
            let go = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.separator();
            let needle = filter.trim().to_ascii_lowercase();
            let matches = |addr: &str| {
                needle.is_empty() || addr.to_ascii_lowercase().contains(&needle)
            };
            let hits: Vec<&str> = registry
                .iter()
                .map(|(_, def)| def.addr.as_str())
                .filter(|a| matches(a))
                .collect();
            if go && hits.len() == 1 {
                chosen = Some(Some(hits[0].to_string()));
            }
            egui::ScrollArea::vertical()
                .id_salt(popup_id)
                .show(ui, |ui| {
                    if needle.is_empty() && ui.selectable_label(false, "— none —").clicked() {
                        chosen = Some(None);
                    }
                    for addr in &hits {
                        if ui.selectable_label(false, *addr).clicked() {
                            chosen = Some(Some((*addr).to_string()));
                        }
                    }
                    if hits.is_empty() {
                        ui.label(
                            egui::RichText::new("nothing matches")
                                .color(vizz_design::ink::FAINT),
                        );
                    }
                });
        });
    });
    ui.ctx().data_mut(|d| d.insert_temp(filter_id, filter));
    if let Some(pick) = chosen {
        macros.set(slot, pick);
        actions.macros_changed = true;
        close = true;
    }
    if close {
        close_assign(ui, slot);
    }
}

/// The picker's footprint, used to keep it on screen.
const POPUP_W: f32 = 240.0;
const POPUP_H: f32 = 320.0;

// egui 0.35 made the popup helpers private, so the open slot is tracked in
// the public temp-data store instead. Keyed per slot so two faders cannot
// both think they own the picker.
fn assign_key(slot: usize) -> egui::Id {
    egui::Id::new(("assign-open", slot))
}
fn open_assign(ui: &egui::Ui, slot: usize, at: egui::Pos2) {
    let id = egui::Id::new(("assign", slot));
    ui.memory_mut(|m| {
        m.data.insert_temp(assign_key(slot), true);
        // Where to draw it, and a fresh start for the filter and the
        // focus latch — reopening a picker that still holds the last
        // search is a picker that looks broken.
        m.data.insert_temp(id.with("at"), at);
        m.data.insert_temp(id.with("filter"), String::new());
        m.data.insert_temp(id.with("focused"), false);
    });
}
fn close_assign(ui: &egui::Ui, slot: usize) {
    ui.memory_mut(|m| m.data.insert_temp(assign_key(slot), false));
}
fn is_assign_open(ui: &egui::Ui, slot: usize) -> bool {
    ui.memory(|m| m.data.get_temp::<bool>(assign_key(slot)).unwrap_or(false))
}


/// How many fader columns a lane of `width` can carry, the master's
/// column included.
fn columns_that_fit(width: f32) -> usize {
    (((width + COL_GAP) / (FADER_MIN_W + COL_GAP)).floor() as usize).max(2)
}

/// How many rows `count` faders need in a lane of `width`.
///
/// One, whenever the width can carry it — two banks split the one
/// gesture this screen exists for. Pure, and tested as such: inferring
/// the answer from what was painted is inferring it from the thing
/// under test.
fn fader_rows_for(count: usize, width: f32) -> usize {
    let across = columns_that_fit(width).saturating_sub(1).max(1);
    count.div_ceil(across).max(1)
}

/// A fader drawn by hand rather than with `egui::Slider`.
///
/// The built-in vertical slider is a thin rail with a small handle whatever
/// size it is given, so the entire premise of this screen — hit it without
/// aiming — fails with it. Here the *whole column* is the drag target,
/// which is the property that actually matters in a dark room.
///
/// `modulated` is where the value has actually been pushed to, when that
/// differs from where the fader is set. Returns the new value when moved.
#[allow(clippy::too_many_arguments)]
fn vertical_fader(
    ui: &mut egui::Ui,
    value: f32,
    modulated: Option<f32>,
    def: &vizz_params::ParamDef,
    w: f32,
    h: f32,
    // Which fader this is, so per-fader memory is keyed by identity
    // rather than by where it happens to sit — a reflow between one and
    // two rows must not make two faders swap animations.
    slot: usize,
    // The colour of the light, and the colour of it under a hand. Passed
    // so the master is the same widget in a different colour rather than
    // a second implementation that can drift.
    fill_colour: egui::Color32,
    fill_bright: egui::Color32,
    // Read for the learn state and the MIDI rail.
    midi: &MidiView,
) -> Option<f32> {
    let (min, max) = (def.min, def.max);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
    // Right-click restores the default, honouring the overlay's promise
    // that this works on any slider — these faders were the exception.
    let grab_id = egui::Id::new(("fader-grab", slot));
    if response.secondary_clicked() {
        ui.ctx().data_mut(|d| d.remove::<Grab>(grab_id));
        return Some(def.default);
    }
    let span = (max - min).max(f32::EPSILON);
    let t = ((value - min) / span).clamp(0.0, 1.0);

    // The well is a hole milled into the desk, not a block raised out of
    // it. That is the whole idea, and it is also the only version that
    // survives the room: when the output behind the scrim flashes white
    // the ground lifts, so a raised block closes on it and all but
    // vanishes, while a hole opens away from it and reads *better*.
    let track = rect.shrink2(vec2(rect.width() * 0.12, 0.0));
    // One inner rect, computed independently of the rim's width, so a
    // state change can thicken the rim without moving any geometry. A
    // control that shifts when you approach it is a control you cannot
    // find in the dark.
    let inner = track.shrink(1.0);
    // Paint and input are exact inverses of one another. The old widget
    // had two sources of truth — the fill grew from an unclamped height
    // while the handle was clamped six points from each end — so at off
    // and at full, the two positions that matter most, the indicator sat
    // still and lied while the value kept moving.
    let y_of = |t: f32| inner.bottom() - inner.height() * t;
    let t_of = |y: f32| ((inner.bottom() - y) / inner.height().max(1.0)).clamp(0.0, 1.0);

    // Motorised recall: a value that arrived from somewhere other than
    // this hand travels to where it now is, rather than teleporting.
    // Under your own hand there is no easing at all — a fader that lags
    // the hand holding it is a fader you fight.
    let now = ui.input(|i| i.time);
    let addr_hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        def.addr.hash(&mut h);
        h.finish()
    };
    let held = response.is_pointer_button_down_on();
    let t_shown = glide(ui, slot, t, addr_hash, now, held);

    let p = ui.painter();
    p.rect_filled(track, vizz_design::radius::CHIP, vizz_design::surface::GROOVE);

    // The column of light.
    let edge_y = y_of(t_shown);
    if inner.bottom() - edge_y >= 0.5 {
        let lit = if held { fill_bright } else { fill_colour };
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(inner.left(), edge_y.max(inner.top())),
                inner.right_bottom(),
            ),
            // Square at the top because it is a cut edge; the bottom
            // follows the milled corner, one point tighter than the well
            // so no paint escapes the rim.
            egui::CornerRadius { nw: 0, ne: 0, sw: 1, se: 1 },
            lit,
        );
    }

    // Two-tone engraving: a mark inside the well takes the well's colour
    // where it crosses light and the tick colour where it crosses dark,
    // so it is legible against either. The old quarter ticks were drawn
    // in TICK over FILL — four points of luminance apart, which is why
    // nobody has ever seen them.
    let engrave = |over_fill: bool| {
        if over_fill {
            vizz_design::surface::GROOVE
        } else {
            vizz_design::surface::TICK
        }
    };

    // The default, as two stubs flush to the walls. Positioned with the
    // same function as the value, so when the parameter is at its default
    // the notch and the waterline coincide exactly rather than nearly.
    let t_default = ((def.default - min) / span).clamp(0.0, 1.0);
    let dy = y_of(t_default);
    let over = dy >= edge_y;
    for (x0, x1) in [
        (inner.left(), inner.left() + 5.0),
        (inner.right() - 5.0, inner.right()),
    ] {
        p.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, dy - 0.75), egui::pos2(x1, dy + 0.75)),
            0.0,
            engrave(over),
        );
    }

    // Where modulation has actually taken it, amber, and inside the well.
    // Amber's three meanings on this screen are within thirty RGB points
    // of one another and no palette discipline separates them at a
    // distance — so they are separated by *where* they are allowed to
    // appear instead. Amber inside the well is modulation; amber on the
    // rim is an armed learn. Location survives peripheral vision.
    if let Some(m) = modulated {
        let my = y_of(((m - min) / span).clamp(0.0, 1.0));
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(inner.left(), my - 1.0),
                egui::pos2(inner.right(), my + 1.0),
            ),
            0.0,
            MOD,
        );
    }

    // The waterline: the surface of the light, and the one object nothing
    // is ever drawn on top of. Full track width, one point proud of the
    // fill on each side, so the value is a line you read rather than a
    // block whose height you estimate.
    let mark_top = (edge_y - 1.0).clamp(inner.top(), inner.bottom() - 2.0);
    p.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(track.left(), mark_top),
            egui::pos2(track.right(), mark_top + 2.0),
        ),
        1.0,
        HANDLE,
    );

    // One rim carries every chrome state, by strict priority, so an
    // outline can never mean two things at once.
    //
    // The resting rim is 1.5 rather than a hairline. It was 1.0, which
    // renders as exactly one pixel of a 48-point luminance step — real
    // in the code, invisible in the room, which a pixel scan of the
    // rendered frame settled rather than an argument about it. The well
    // then had no drawn boundary at all: only GROOVE against the desk,
    // nine points apart.
    let (rim_w, rim) = if midi.learning(&def.addr) {
        // Breathing rather than blinking: insistent, never a strobe.
        let phase = ((now / 1.6).fract() * std::f64::consts::TAU).sin() as f32;
        let a = (198.0 + 57.0 * phase).clamp(140.0, 255.0) as u8;
        (2.0, crate::theme::LEARN.gamma_multiply(a as f32 / 255.0))
    } else if held {
        (2.0, HANDLE)
    } else if response.hovered() {
        (1.5, vizz_design::surface::FOCUS)
    } else {
        (1.5, vizz_design::surface::EDGE)
    };
    p.rect_stroke(
        track,
        vizz_design::radius::CHIP,
        egui::Stroke::new(rim_w, rim),
        egui::StrokeKind::Inside,
    );

    // A control your hardware owns, said as a rail in the margin rather
    // than only as small print below. A word has to be read; a rail at a
    // constant x does not, so a mapped desk and an unmapped desk are
    // different pictures from across a room.
    //
    // Flush against the well rather than floating beside it. With a gap
    // it read as a divider between two columns instead of a property of
    // one — the same mark, three pixels away, meaning something else
    // entirely.
    if midi.map.source_for(&def.addr).is_some() {
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(track.left() - 2.5, track.top()),
                egui::pos2(track.left(), track.bottom()),
            ),
            0.0,
            vizz_design::accent::BINDING,
        );
    }

    // The press rule, in one sentence: a press far from the value in the
    // body of the well jumps to your hand; a press near the value, or
    // with Shift held, moves it from where it already is.
    //
    // Gated on `is_pointer_button_down_on` rather than `dragged()`:
    // egui cannot decide a drag until the pointer has travelled six
    // points, so a `dragged()` gate throws away the press frame and then
    // delivers those six points as one jump.
    if !held {
        ui.ctx().data_mut(|d| d.remove::<Grab>(grab_id));
        return None;
    }
    let fine = ui.input(|i| i.modifiers.shift);
    let here = response.interact_pointer_pos()?;
    let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(here);
    let existing = ui.ctx().data(|d| d.get_temp::<Grab>(grab_id));
    let g = match existing {
        Some(g) => g,
        None => {
            // A catch band around the current value, sized off the track
            // so it is the same *promise* on a tall fader and a short one.
            let band = (inner.height() * 0.10).clamp(8.0, 16.0);
            let near = (press.y - y_of(t)).abs() <= band;
            let g = Grab { origin_y: press.y, base_t: t, relative: near || fine };
            ui.ctx().data_mut(|d| d.insert_temp(grab_id, g));
            g
        }
    };
    // Re-anchored every frame against the modifier, so pressing or
    // releasing Shift mid-gesture changes the gain without teleporting
    // the value.
    let gain = if fine { 0.25 } else { 1.0 };
    let nt = if g.relative || fine {
        let travelled = (g.origin_y - here.y) / inner.height().max(1.0);
        (g.base_t + travelled * gain).clamp(0.0, 1.0)
    } else {
        t_of(here.y)
    };
    Some(min + nt * span)
}

/// A press in progress on one fader.
#[derive(Clone, Copy)]
struct Grab {
    /// Where the press landed, for relative moves.
    origin_y: f32,
    /// The value at the moment of the press.
    base_t: f32,
    /// Move from where it is, rather than jumping to the pointer.
    relative: bool,
}

/// What a fader is *showing*, which is the value except just after it
/// changed under someone else's hand.
///
/// Keyed by slot rather than by position, so a reflow between one and two
/// rows cannot make two faders swap animations, and carries a hash of the
/// assigned address so reassigning a slot re-seeds instead of gliding
/// between two unrelated parameters.
fn glide(
    ui: &egui::Ui,
    slot: usize,
    t: f32,
    addr: u64,
    now: f64,
    held: bool,
) -> f32 {
    #[derive(Clone, Copy)]
    struct Glide {
        from: f32,
        to: f32,
        t0: f64,
        addr: u64,
    }
    let id = egui::Id::new(("fader-glide", slot));
    let settle = vizz_design::motion::SETTLE;
    let eased = |g: &Glide, now: f64| {
        let x = (((now - g.t0) as f32) / settle).clamp(0.0, 1.0);
        // Smoothstep: no overshoot, which a fader must never do — it
        // would be showing a value the engine never held.
        g.from + (g.to - g.from) * (x * x * (3.0 - 2.0 * x))
    };
    let settled = Glide { from: t, to: t, t0: now - settle as f64, addr };
    let mut g = ui
        .ctx()
        .data(|d| d.get_temp::<Glide>(id))
        .filter(|g| g.addr == addr)
        .unwrap_or(settled);
    if held {
        // Under your own hand, exactly where you put it.
        g = settled;
    } else if (t - g.to).abs() > 0.0005 {
        g = Glide { from: eased(&g, now), to: t, t0: now, addr };
    }
    let shown = eased(&g, now);
    ui.ctx().data_mut(|d| d.insert_temp(id, g));
    shown
}

/// The master, as one more column in the fader row.
///
/// It used to be a full-width bar with its own row, which made it the
/// loudest object on a screen where it is not the thing being played. Same
/// size as its neighbours now; it stays findable by being red and by always
/// sitting at the end of the first row.
fn master(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    state: &PerformanceState<'_>,
    w: f32,
    h: f32,
) {
    let Some(id) = registry.id("/master/dim") else {
        return;
    };
    let def = &registry.defs()[id.index()];
    let value = registry.target(id);

    // The same widget as every other fader, in a different colour.
    //
    // It used to be a second hand-rolled copy of the drawing and the
    // input, which is how the two came to disagree: the macros got a
    // catch band, exact paint/input inverses and a waterline, and the
    // master kept the clamped handle that freezes across the top and
    // bottom of its travel — on the one fader whose bottom of travel is
    // a black output.
    //
    // The slot key is `usize::MAX` so the master's per-fader memory can
    // never collide with a macro's, whatever the count is grown to.
    if let Some(v) = vertical_fader(
        ui,
        value,
        None,
        def,
        w,
        h,
        usize::MAX,
        MASTER_FILL,
        MASTER_BRIGHT,
        state.midi,
    ) {
        registry.set(id, v);
    }
    let t = ((registry.target(id) - def.min) / (def.max - def.min).max(f32::EPSILON))
        .clamp(0.0, 1.0);
    // Dimmed-out is a state worth shouting about: a black output with
    // everything else apparently fine is the classic mid-set panic.
    // Drawn over the widget rather than inside it, because it is the one
    // rim state that belongs to the master alone.
    if t < 0.02 {
        let rect = ui.min_rect();
        ui.painter().rect_stroke(
            egui::Rect::from_min_size(rect.left_top(), vec2(w, h)),
            vizz_design::radius::CHIP,
            egui::Stroke::new(1.5, WARN),
            egui::StrokeKind::Inside,
        );
    }
    let _ = state;

    ui.label(
        egui::RichText::new(format!("{value:.2}"))
            .size(13.0)
            .monospace()
            .color(if t < 0.02 { WARN } else { INK }),
    );
    // Shrunk to the column like every other fader name, rather than
    // drawn at a fixed size and allowed to overhang. At a 1024-point
    // window the master's column is 55 points wide and this caption
    // wanted 45 starting at x=983 — four points past the right edge of
    // the window, where egui clipped it. The value above it stayed, so
    // the master read as a nameless fader at the end of the row: the
    // one control whose whole job is to be found without looking.
    // Fitted to the column and *bounded* by it. Every macro fader's name
    // is added with `.truncate()`, which keeps it inside its column; this
    // caption was a plain `ui.label`, which is free to overhang. At a
    // 1024-point window it ran from x=983 to x=1028 — four points past
    // the right edge of the window, where egui clipped it away entirely.
    // The value above it stayed, so the master read as a nameless fader
    // at the end of the row: the one control whose whole job is to be
    // found without looking.
    //
    // Capped at the size it always was, so this only ever shrinks.
    let (master_label, master_size) = fit_label(ui, "MASTER", w);
    ui.label(
        egui::RichText::new(master_label)
            .size(master_size.min(11.0))
            .strong()
            .color(vizz_design::accent::MASTER_INK),
    );
    ui.label(egui::RichText::new(" ").size(11.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use vizz_params::ParamDef;

    fn registry() -> ParamRegistry {
        let mut b = ParamRegistry::builder();
        b.add(ParamDef::new("/particles/size", 0.001, 0.2, 0.015));
        b.add(ParamDef::new("/fx/glow", 0.0, 1.0, 0.25));
        b.add(ParamDef::new("/master/dim", 0.0, 1.0, 1.0));
        b.add(ParamDef::new("/punch/flash", 0.0, 1.0, 0.0).gesture());
        b.add(ParamDef::new("/punch/strobe", 0.0, 1.0, 0.0).gesture());
        b.add(ParamDef::new("/punch/strobe_div", 0.25, 4.0, 0.5).transport());
        b.add(ParamDef::new("/record/active", 0.0, 1.0, 0.0).transport());
        // A vector layer, for the strip tests. Kind 0 = off, the default.
        b.add(
            ParamDef::new("/l1/kind", 0.0, 7.0, 0.0).labels(&[
                "off", "rings", "stripes", "checker", "polygon", "star", "rays", "dots",
            ]),
        );
        b.add(ParamDef::new("/l1/blend", 0.0, 6.0, 0.0).labels(&[
            "normal", "multiply", "screen", "add", "difference", "exclusion", "subtract",
        ]));
        b.add(ParamDef::new("/l1/opacity", 0.0, 1.0, 1.0));
        b.add(ParamDef::new("/l1/freq", 0.5, 64.0, 8.0));
        b.add(ParamDef::new("/l1/color", 0.0, 3.0, 0.0));
        // A clashing pair: both end in "spread", so both faders have to
        // wear their group word to be told apart.
        b.add(ParamDef::new("/color/spread", 0.0, 1.0, 0.12));
        b.add(ParamDef::new("/particles/spread", 0.05, 3.0, 1.2));
        b.build()
    }


    /// The performance screen can get out of the way of the performance.
    ///
    /// Every section is full-width and stacked, so the one thing this
    /// screen cannot show you is the thing it is for: the output sits
    /// behind an opaque sheet of its own controls. Peeking stands the
    /// stacked rows down and leaves the picture visible.
    ///
    /// Asserted on what is drawn rather than on the scrim's alpha: the
    /// complaint is "I cannot see the output", and a transparent scrim
    /// with a sixteen-pad grid still over it fixes nothing.
    #[test]
    fn peeking_gets_the_controls_out_of_the_way() {
        let reg = registry();
        let mut macros = Macros::default();
        for (slot, addr) in ["/particles/size", "/particles/speed"].iter().enumerate() {
            macros.set(slot, Some((*addr).to_string()));
        }

        // Normally the stacked rows are there, and so is the way in.
        let text = render(&mut macros, &reg);
        assert!(text.contains("SCENES"), "the scene grid is missing: {text}");
        assert!(text.contains("PUNCH"), "the punch row is missing: {text}");
        assert!(
            text.contains("view"),
            "no way to see the output from the performance screen: {text}"
        );

        // Peeking: the rows that cover the picture stand down, and the
        // two a hand is on stay.
        let text = render_peeking(&mut macros, &reg);
        assert!(!text.contains("SCENES"), "the scene grid still covers the output: {text}");
        assert!(!text.contains("PUNCH"), "the punch row still covers the output: {text}");
        assert!(!text.contains("PRESETS"), "the preset row still covers the output: {text}");
        // The faders stay — checking the output is something you do
        // while playing, not instead of it.
        assert!(text.contains("size"), "the faders went with everything else: {text}");
        // And the status strip stays, because whether you are recording
        // and whether the output is alive do not stop mattering.
        assert!(text.contains("bpm"), "the status strip went: {text}");
        // The way back is still visible.
        assert!(text.contains("view"), "no way back out of peek: {text}");
    }

    fn render_peeking(macros: &mut Macros, reg: &ParamRegistry) -> String {
        RENDER_PEEK.with(|f| f.set(true));
        let out = render(macros, reg);
        RENDER_PEEK.with(|f| f.set(false));
        out
    }

    thread_local! {
        /// Set by [`render_peeking`] so the harness can put the context
        /// into the peeking state before the layout reads it.
        static RENDER_PEEK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        /// The set list the shared harness draws, so the layout tests can
        /// opt into a deck row without every other test in the file
        /// suddenly finding chip names in its sheet.
        static SHEET_DECKS: std::cell::RefCell<Vec<DeckChip>> =
            const { std::cell::RefCell::new(Vec::new()) };
        /// The preset library the shared harness draws, for the tests that
        /// care what happens when it is large.
        static SHEET_PRESETS: std::cell::RefCell<Vec<crate::PresetEntry>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Run `f` with a large preset library loaded.
    fn with_presets<T>(names: Vec<crate::PresetEntry>, f: impl FnOnce() -> T) -> T {
        SHEET_PRESETS.with(|d| *d.borrow_mut() = names);
        let out = f();
        SHEET_PRESETS.with(|d| d.borrow_mut().clear());
        out
    }

    /// What the built-in set puts in the library: twenty songs of eight.
    fn a_set_of_presets() -> Vec<crate::PresetEntry> {
        (1..=20)
            .flat_map(|n| {
                SECTION_NAMES
                    .iter()
                    .map(move |s| crate::PresetEntry::from(&*format!("{n:02} Song Title Here - {s}")))
            })
            .collect()
    }

    const SECTION_NAMES: [&str; 8] = [
        "Intro", "Build", "Break", "Drop", "Bridge", "Peak", "Outro", "Blackout",
    ];

    /// Run `f` with a set list on the desk.
    fn with_decks<T>(decks: Vec<DeckChip>, f: impl FnOnce() -> T) -> T {
        SHEET_DECKS.with(|d| *d.borrow_mut() = decks);
        let out = f();
        SHEET_DECKS.with(|d| d.borrow_mut().clear());
        out
    }

    fn chip(name: &str) -> DeckChip {
        DeckChip { name: name.into(), midi: None, learning: false, origin: 1 }
    }


    /// The desk: output beside the controls, not behind them.
    ///
    /// Peeking answers "let me see the output" but not "let me see it
    /// while I fire a scene" — the grid is exactly what stands down to
    /// make the room. At a width that can carry both, the sections take
    /// a column and the picture takes the rest.
    #[test]
    fn a_wide_window_puts_the_output_beside_the_controls() {
        let reg = registry();
        let mut macros = Macros::default();
        macros.set(0, Some("/particles/size".to_string()));

        // Wide: everything is still there, because nothing had to stand
        // down to make room for the picture.
        let text = render_at(&mut macros, &reg, &MidiView::default(), None, None, 1440.0);
        for want in ["SCENES", "PUNCH", "CONTROLS", "size"] {
            assert!(text.contains(want), "{want} went missing on a wide window: {text}");
        }

        // Narrow: the desk closes rather than serving a preview too
        // small to judge beside a grid too tight to hit.
        //
        // Rendered tall as well as narrow, deliberately. At 900x800 the
        // faders are still drawn but their three label lines fall off
        // the bottom, because punch, layers, presets and two full grids
        // above them have already spent the window — the long-standing
        // short-window starvation, tracked separately. Asserting at 800
        // would be asserting that bug rather than this one, and the
        // thing under test here is the *width* fallback.
        let text = render_sized(
            &mut macros,
            &reg,
            &MidiView::default(),
            None,
            None,
            vec2(900.0, 1000.0),
        );
        assert!(text.contains("SCENES"), "the narrow layout lost the grid: {text}");
        // The fader block is asserted by its caption, not by a fader's
        // name. At narrow widths the three label lines under each fader
        // are starved by everything stacked above them and fall outside
        // the window — the long-standing short-window starvation, which
        // is its own problem and not the width fallback under test here.
        // Asserting a name would be asserting that bug instead of this
        // one, and would go on failing after this one was fixed.
        assert!(text.contains("CONTROLS"), "the narrow layout lost the desk: {text}");
    }


    /// The fader count is adjustable, within limits.
    ///
    /// A set is personal: some people play four things and some play
    /// twenty. What matters is that the limits are real — the screen
    /// still has to show the picture, and faders you cannot see past
    /// are not more control.
    #[test]
    fn the_fader_count_can_be_changed_from_the_desk() {
        let reg = registry();
        let mut macros = Macros::default();
        macros.set(0, Some("/particles/size".to_string()));

        let text = render(&mut macros, &reg);
        assert!(
            text.contains("CONTROLS"),
            "the desk lost its caption: {text}"
        );
        // The count is shown, not only implied by counting faders.
        assert!(
            text.contains(&format!("{}", vizz_mod::perform::MACRO_COUNT)),
            "the fader count is not shown: {text}"
        );

        // At the ceiling the grow control is disabled rather than absent
        // — a control that vanishes teaches nothing about why.
        let mut full = Macros::default();
        while full.grow() {}
        assert_eq!(full.count(), vizz_mod::perform::MACRO_MAX);
        let text = render(&mut full, &reg);
        assert!(
            text.contains(&format!("{}", vizz_mod::perform::MACRO_MAX)),
            "a full set does not show its count: {text}"
        );
    }

    /// Every fader in the set is drawn, whatever the count.
    ///
    /// The layout used to iterate a constant. If it kept doing that, a
    /// grown set would have faders that exist in the file, respond to
    /// MIDI, and are invisible.
    #[test]
    fn every_fader_in_the_set_is_drawn() {
        let reg = registry();
        // Every slot cleared first, so the only thing that can put the
        // name on screen is the fader under test.
        //
        // Without this the test passes on a coincidence: the default set
        // already assigns /fx/glow to slot 6, so asserting "glow" after
        // assigning it to slot 23 proves nothing about slot 23 at all.
        // It was written that way and it was vacuous.
        let cleared = |m: &mut Macros| {
            for i in 0..m.count() {
                m.set(i, None);
            }
        };

        let mut macros = Macros::default();
        while macros.shrink() {}
        cleared(&mut macros);
        let last = macros.count() - 1;
        macros.set(last, Some("/fx/glow".to_string()));
        let text = render(&mut macros, &reg);
        assert!(text.contains("glow"), "the last fader of a small set is missing: {text}");

        // Grow past the original sixteen and assign the new end.
        let mut big = Macros::default();
        while big.grow() {}
        cleared(&mut big);
        let last = big.count() - 1;
        big.set(last, Some("/fx/glow".to_string()));
        let text = render(&mut big, &reg);
        assert!(
            text.contains("glow"),
            "a fader beyond the old fixed count was never drawn: {text}"
        );
    }


    /// A full set fits inside the window it is drawn in.
    ///
    /// Rows used to be chosen by height alone, which was survivable
    /// while the count was fixed at sixteen and broke the moment it
    /// could be raised: twenty-four faders on a 1280-point window laid
    /// out twenty-five columns needing 1694 points of a 1252-point lane,
    /// and six of them were placed past the right edge. They existed,
    /// they answered MIDI, and nobody could see them.
    ///
    /// Asserted on where things were actually painted, not on the row
    /// arithmetic — the arithmetic is what was wrong.
    #[test]
    fn a_full_set_of_faders_stays_inside_the_window() {
        let reg = registry();
        let mut macros = Macros::default();
        while macros.grow() {}
        assert_eq!(macros.count(), vizz_mod::perform::MACRO_MAX);
        // Distinct names, so a fader drawn off-screen is identifiable
        // rather than hidden behind a duplicate.
        for i in 0..macros.count() {
            macros.set(i, Some("/particles/size".to_string()));
        }

        for (w, h) in [(1280.0, 800.0), (1440.0, 900.0), (1100.0, 720.0)] {
            let size = vec2(w, h);
            let right = painted_right_edge(&mut macros, &reg, size);
            assert!(
                right <= w + 1.0,
                "at {w}x{h} a {} fader set painted out to {right}, past the window",
                macros.count()
            );
        }
    }

    /// The rightmost x any shape reaches, which is where the layout
    /// actually put things rather than where it meant to.
    fn painted_right_edge(macros: &mut Macros, reg: &ParamRegistry, size: Vec2) -> f32 {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let audio = AudioView::default();
        let names = [crate::PresetEntry::from("Slow bloom")];
        let grid = crate::grid_view::GridView::default();
        let midi = MidiView::default();
        let state = PerformanceState {
            project: "Show 1",
            decks: &[],
            active_deck: 0,
            follow_columns: None,
            recording: None,
            preset_current: None,
            outputs: &[],
            audio: &audio,
            fps: 60.0,
            over_budget: false,
            bpm: 128.0,
            bar_phase: 0.1,
            presets: &names,
            thumb_revision: 0,
            grid: &grid,
            gravity: None,
            midi: &midi,
            values: None,
            output_texture: None,
            output_aspect: 16.0 / 9.0,
            graph: None,
        };
        let mut right = 0.0f32;
        for i in 0..4 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(i as f64 * 0.05),
                ..Default::default()
            });
            draw(&ctx, reg, &state, macros);
            let out = ctx.end_pass();
            fn walk(shape: &egui::Shape, right: &mut f32) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, right)),
                    other => {
                        let r = other.visual_bounding_rect();
                        // Infinite rects are egui's "unbounded" marker on
                        // things like clip shapes, not painted extent.
                        if r.is_finite() {
                            *right = right.max(r.right());
                        }
                    }
                }
            }
            for p in &out.shapes {
                walk(&p.shape, &mut right);
            }
        }
        right
    }


    /// A full set stays on one axis, which is the point of the width.
    ///
    /// Two banks split the one gesture this screen exists for: the eye
    /// has to find which bank a fader is in before it can find the
    /// fader, and the answer moves as the count does.
    #[test]
    fn a_full_set_stays_on_one_row_at_a_normal_window() {
        let max = vizz_mod::perform::MACRO_MAX;
        // Twenty-five columns at the minimum width need a lane of about
        // 1044 points. These are the windows that clear it, measured as
        // the lane the layout actually gets rather than the window.
        for lane in [1408.0, 1248.0, 1068.0] {
            assert_eq!(
                fader_rows_for(max, lane),
                1,
                "a full set of {max} took more than one row in a {lane}pt lane"
            );
        }
        // Sixteen, the default, has room to spare.
        assert_eq!(fader_rows_for(16, 1068.0), 1);
        // And below the width where one row fits, it wraps rather than
        // running off the screen — a fader you cannot see is worse than
        // a fader in the wrong bank.
        assert!(
            fader_rows_for(max, 700.0) > 1,
            "a narrow lane kept everything on one unreachable row"
        );
    }


    /// A modulator can be put on a fader without leaving the layout.
    ///
    /// Driven through real clicks on the desk rather than by calling the
    /// popup directly, because the thing worth proving is that the
    /// control is *reachable*: the value line has to take a click, the
    /// popup has to open over a layout that redraws every frame, and the
    /// pick has to survive as an action. Any of those failing leaves a
    /// feature that exists and cannot be used.
    #[test]
    fn a_modulator_can_be_put_on_a_fader_from_the_desk() {
        let reg = registry();
        let mut macros = Macros::default();
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let audio = AudioView::default();
        let names = [crate::PresetEntry::from("Slow bloom")];
        let grid = crate::grid_view::GridView::default();
        let midi = MidiView::default();
        let size = vec2(1440.0, 900.0);
        // A live graph, as the app passes: empty to start with.
        let mut graph = vizz_mod::graph::NodeGraph::default();
        // The clock has to advance, or the faders never finish their
        // settle animation and every label keeps moving under the
        // pointer — which is exactly how the first version of this test
        // clicked half a point below the readout and saw nothing.
        let mut clock = 0.0_f64;

        // Returns the actions with the painted text, rather than writing
        // them to a captured binding: the test has to read what one
        // frame asked for before driving the next.
        let mut frame = |click: Option<egui::Pos2>,
                         graph: &vizz_mod::graph::NodeGraph,
                         macros: &mut Macros|
         -> (PerformanceActions, Vec<(String, egui::Rect)>) {
            let state = PerformanceState {
                project: "Show 1",
            decks: &[],
                active_deck: 0,
                follow_columns: None,
                recording: None,
                preset_current: None,
                outputs: &[],
                audio: &audio,
                fps: 60.0,
                over_budget: false,
                bpm: 128.0,
                bar_phase: 0.1,
                presets: &names,
                thumb_revision: 0,
                grid: &grid,
                gravity: None,
                midi: &midi,
                values: None,
                output_texture: None,
                output_aspect: 16.0 / 9.0,
                graph: Some(graph),
            };
            clock += 0.1;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(clock),
                ..Default::default()
            };
            if let Some(at) = click {
                input.events.push(egui::Event::PointerMoved(at));
                for pressed in [true, false] {
                    input.events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    });
                }
            }
            ctx.begin_pass(input);
            let actions = draw(&ctx, &reg, &state, macros);
            let out = ctx.end_pass();
            let mut found = Vec::new();
            fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    egui::Shape::Text(t) => {
                        out.push((t.galley.job.text.clone(), t.galley.rect.translate(t.pos.to_vec2())))
                    }
                    _ => {}
                }
            }
            for p in &out.shapes {
                walk(&p.shape, &mut found);
            }
            (actions, found)
        };

        /// Where the readout under a named fader is, measured on the
        /// frame it will be clicked on rather than remembered from an
        /// earlier one — the desk animates, and a stale position is how
        /// this test first "passed" a click that landed on nothing.
        fn readout_under(texts: &[(String, egui::Rect)], name: &str) -> egui::Pos2 {
            let label = texts
                .iter()
                .find(|(t, _)| t.trim() == name)
                .map(|(_, r)| *r)
                .unwrap_or_else(|| panic!("no fader labelled '{name}' on the desk"));
            texts
                .iter()
                .filter(|(_, r)| {
                    (r.center().x - label.center().x).abs() < 12.0 && r.top() > label.top()
                })
                .min_by(|a, b| a.1.top().total_cmp(&b.1.top()))
                .map(|(_, r)| r.center())
                .expect("the fader has no readout under its name")
        }

        // Settle, then click the readout under the first fader.
        for _ in 0..6 {
            frame(None, &graph, &mut macros);
        }
        let (_, texts) = frame(None, &graph, &mut macros);
        frame(Some(readout_under(&texts, "size")), &graph, &mut macros);
        // The popup is an Area, so it lands on the frame after the click
        // that opens it — which is also what the eye sees.
        let (_, opened) = frame(None, &graph, &mut macros);
        assert!(
            opened.iter().any(|(t, _)| t.contains("Slow sweep")),
            "clicking the readout did not open the modulator list: {:?}",
            opened.iter().map(|(t, _)| t).collect::<Vec<_>>()
        );

        let kick = opened
            .iter()
            .find(|(t, _)| t.starts_with("Kick"))
            .map(|(_, r)| r.center())
            .expect("the list has no Kick");
        let (actions, _) = frame(Some(kick), &graph, &mut macros);
        let (addr, shape) = actions
            .set_mod_shape
            .expect("picking a modulator produced no action");
        assert_eq!(addr, "/particles/size");
        let shape = shape.expect("picking a modulator asked to remove one");
        assert_eq!(vizz_mod::shapes::SHAPES[shape].name, "Kick");

        // Apply it the way the app does, and the fader now reports it.
        vizz_mod::shapes::attach(&mut graph, shape, &addr);
        assert_eq!(vizz_mod::shapes::attached(&graph, &addr), Some(shape));

        // With one attached, the same click offers to take it off.
        let (_, texts) = frame(None, &graph, &mut macros);
        frame(Some(readout_under(&texts, "size")), &graph, &mut macros);
        let (_, reopened) = frame(None, &graph, &mut macros);
        let none_at = reopened
            .iter()
            .find(|(t, _)| t.contains("none"))
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| {
                panic!(
                    "reopening the list offered no way back to none: {:?}",
                    reopened.iter().map(|(t, _)| t).collect::<Vec<_>>()
                )
            });
        let (actions, _) = frame(Some(none_at), &graph, &mut macros);
        assert_eq!(
            actions.set_mod_shape,
            Some(("/particles/size".to_string(), None)),
            "choosing none did not ask for the modulator to come off"
        );
        vizz_mod::shapes::detach(&mut graph, &addr);
        assert!(!vizz_mod::shapes::driven(&graph, &addr));
    }

    /// Clear works from the performance desk, not only from the widget.
    ///
    /// The grid widget clears correctly when driven on its own — that is
    /// tested in grid_view. Reported behaviour is that it does nothing
    /// on the performance screen, which means the fault is in what this
    /// layout does around it, so this drives the whole layout.
    #[test]
    fn clear_works_from_the_performance_desk() {
        let reg = registry();
        let mut macros = Macros::default();
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let audio = AudioView::default();
        let names = [crate::PresetEntry::from("Slow bloom")];
        let mut grid = crate::grid_view::GridView::default();
        grid.names[0] = Some("intro".into());
        grid.curve_names = vec!["linear".into(), "smooth".into()];
        let midi = MidiView::default();
        let size = vec2(1440.0, 900.0);
        let mut actions = PerformanceActions::default();

        let mut frame = |click: Option<egui::Pos2>,
                         ctx: &egui::Context,
                         macros: &mut Macros|
         -> Vec<(String, egui::Pos2)> {
            let state = PerformanceState {
                project: "Show 1",
            decks: &[],
                active_deck: 0,
                follow_columns: None,
                recording: None,
                preset_current: None,
                outputs: &[],
                audio: &audio,
                fps: 60.0,
                over_budget: false,
                bpm: 128.0,
                bar_phase: 0.1,
                presets: &names,
                thumb_revision: 0,
                grid: &grid,
                gravity: None,
                midi: &midi,
                values: None,
                output_texture: None,
                output_aspect: 16.0 / 9.0,
                graph: None,
            };
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            if let Some(at) = click {
                input.events.push(egui::Event::PointerMoved(at));
                input.events.push(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                });
                input.events.push(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                });
            }
            ctx.begin_pass(input);
            actions = draw(ctx, &reg, &state, macros);
            let out = ctx.end_pass();
            let mut found = Vec::new();
            fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    egui::Shape::Text(t) => out.push((
                        t.galley.job.text.clone(),
                        t.pos + t.galley.rect.center().to_vec2(),
                    )),
                    _ => {}
                }
            }
            for p in &out.shapes {
                walk(&p.shape, &mut found);
            }
            found
        };

        frame(None, &ctx, &mut macros);
        let texts = frame(None, &ctx, &mut macros);
        let clear_at = texts
            .iter()
            .find(|(t, _)| t.trim() == "clear")
            .map(|(_, p)| *p)
            .expect("no clear button on the performance desk");
        // The pad we mean to clear, found by its own name so the test
        // does not depend on the deck's layout arithmetic.
        let pad_at = texts
            .iter()
            .find(|(t, _)| t.trim() == "intro")
            .map(|(_, p)| *p)
            .expect("no intro pad on the performance desk");

        frame(Some(clear_at), &ctx, &mut macros);
        frame(Some(pad_at), &ctx, &mut macros);

        assert_eq!(
            actions.grid.clear,
            Some(0),
            "arming clear and pressing a pad on the desk produced no clear"
        );
    }


    /// A look with a source, for the grouping tests.
    fn look(name: &str, source: &str) -> crate::PresetEntry {
        crate::PresetEntry {
            name: name.to_string(),
            builtin: false,
            about: None,
            source: Some(source.to_string()),
        }
    }

    /// A library spanning three families.
    ///
    /// Three rather than all five: the block is capped at three headings
    /// on a 1440x900 window — see [`block_cap`] — so a fourth group is
    /// correctly below the fold, and a test that asserted it was on
    /// screen would be asserting the cap does not work.
    fn a_mixed_library() -> Vec<crate::PresetEntry> {
        vec![
            look("Scanned torso", "torso-scan.ply"),
            look("Bloom", "sphere"),
            look("Butterfly", "Aizawa"),
        ]
    }

    /// The other two families, small enough to both fit.
    fn a_shipped_library() -> Vec<crate::PresetEntry> {
        vec![
            crate::PresetEntry {
                name: "Slow bloom".into(),
                builtin: true,
                about: Some("opener".into()),
                source: Some("built in".into()),
            },
            crate::PresetEntry::from("Ancient look"),
        ]
    }

    /// A mixed library is grouped by what each look was built on.
    ///
    /// The whole reason a preset records a source: a set built on scans,
    /// generated shapes and attractors is three different kinds of thing,
    /// and a flat alphabetical row makes you read every name to find the
    /// three that go together.
    #[test]
    fn a_mixed_library_is_grouped_by_what_the_looks_were_built_on() {
        let reg = registry();
        let mut macros = Macros::default();
        let sheet = with_presets(a_mixed_library(), || {
            sheet_sized(
                &mut macros,
                &reg,
                &MidiView::default(),
                None,
                None,
                vec2(1440.0, 900.0),
            )
        });
        let text = sheet.text();
        for heading in ["clouds", "shapes", "attractors"] {
            assert!(text.contains(heading), "no '{heading}' heading: {text}");
        }
        // Each heading above the look it labels, or it is labelling the
        // wrong group — which a substring search cannot tell.
        let at = |t: &str| {
            sheet
                .items
                .iter()
                .find(|p| p.text.trim() == t)
                .unwrap_or_else(|| panic!("'{t}' was not painted: {text}"))
                .rect
        };
        // Each heading ahead of the look it labels, and behind the one
        // before it. The headings flow with the tiles rather than sitting
        // over them, so "ahead" is reading order — left to right, then
        // down.
        //
        // Compared on trailing edges, and that is not arbitrary: egui
        // lays a label inside a wrapped row out as a galley anchored at
        // the *row's* origin with the indentation as leading space, so
        // every heading in a row reports the same `left` and only its
        // `right` says where the words actually end. Measured: the three
        // headings all come back starting at x=14.
        //
        // Same-row is a tolerance rather than an exact edge, because a
        // tile's name sits along its bottom and a heading at the middle
        // of the row — the same row, a couple of points apart. A row is
        // fifty-six, so the tolerance separates the two cases easily.
        let before = |a: &str, b: &str| {
            let (a, b) = (at(a), at(b));
            if (a.bottom() - b.bottom()).abs() < TILE_H {
                a.right() < b.right()
            } else {
                a.bottom() < b.bottom()
            }
        };
        assert!(before("clouds", "Scanned torso"), "the clouds heading is not before its look");
        assert!(before("Scanned torso", "shapes"), "a cloud look is not before the shapes");
        assert!(before("shapes", "Bloom"), "the shapes heading is not before its look");
        assert!(before("Bloom", "attractors"), "a shape is not before the attractors");
        assert!(before("attractors", "Butterfly"), "the attractors heading is not before its look");

        // The two families the first library has none of.
        let shipped = with_presets(a_shipped_library(), || {
            sheet_sized(
                &mut macros,
                &reg,
                &MidiView::default(),
                None,
                None,
                vec2(1440.0, 900.0),
            )
        });
        let text = shipped.text();
        for heading in ["built in", "unsorted"] {
            assert!(text.contains(heading), "no '{heading}' heading: {text}");
        }
    }

    /// One family is a caption on the whole list, not a grouping — so it
    /// is not drawn. A fresh install ships built-ins and nothing else,
    /// and a lone "built in" heading over every look is a row of
    /// vertical space spent saying nothing.
    #[test]
    fn one_family_gets_no_heading() {
        let reg = registry();
        let mut macros = Macros::default();
        let library = vec![look("Bloom", "sphere"), look("Knotted", "knot")];
        let sheet = with_presets(library, || {
            sheet_sized(
                &mut macros,
                &reg,
                &MidiView::default(),
                None,
                None,
                vec2(1440.0, 900.0),
            )
        });
        let text = sheet.text();
        assert!(text.contains("Bloom"), "the looks are missing: {text}");
        assert!(
            !sheet.items.iter().any(|p| p.text.trim() == "shapes"),
            "a lone family was still given a heading: {text}"
        );
    }

    /// Grouping must not renumber anything. The slot is what
    /// `/preset/recall`, the number keys and every MIDI binding address;
    /// re-sorting the list to make the groups tidy would silently remap
    /// a controller mid-set.
    #[test]
    fn grouping_does_not_renumber_the_slots() {
        let reg = registry();
        let mut macros = Macros::default();
        // Slot 1 is a cloud, so grouping moves it below the shapes and
        // attractors it would otherwise sit above.
        let sheet = with_presets(a_mixed_library(), || {
            sheet_sized(
                &mut macros,
                &reg,
                &MidiView::default(),
                None,
                None,
                vec2(1440.0, 900.0),
            )
        });
        let text = sheet.text();
        let name = sheet
            .items
            .iter()
            .find(|p| p.text.trim() == "Scanned torso")
            .unwrap_or_else(|| panic!("the first preset is missing: {text}"));
        let tile = egui::Rect::from_min_max(
            egui::pos2(name.rect.left() - 8.0, name.rect.bottom() - TILE_H),
            egui::pos2(name.rect.left() - 8.0 + TILE_W, name.rect.bottom() + 6.0),
        );
        assert!(
            sheet
                .items
                .iter()
                .any(|p| p.text.trim() == "1" && tile.contains_rect(p.rect)),
            "the first slot lost its number when the list was grouped: {text}"
        );
    }

    /// Every tile paints its family's colour, so the coding is there for
    /// the looks that have a picture as well as the ones that do not.
    ///
    /// Painted over the picture rather than behind it: a bar you can only
    /// see on the looks with no photograph is a code for the wrong half
    /// of the library.
    #[test]
    fn every_family_paints_its_own_colour() {
        use vizz_mod::preset::Family;
        let reg = registry();
        let mut macros = Macros::default();
        let mut fills = Vec::new();
        for library in [a_mixed_library(), a_shipped_library()] {
            let sheet = with_presets(library, || {
                sheet_sized(
                    &mut macros,
                    &reg,
                    &MidiView::default(),
                    None,
                    None,
                    vec2(1440.0, 900.0),
                )
            });
            fills.extend(sheet.fills);
        }
        for family in Family::ALL {
            assert!(
                fills.contains(&family_tint(family)),
                "{family:?} has no tile wearing its colour"
            );
        }
        // And no two families share one, or the code says nothing.
        let mut tints: Vec<_> = Family::ALL.iter().map(|f| family_tint(*f)).collect();
        tints.sort_by_key(|c| c.to_array());
        tints.dedup();
        assert_eq!(tints.len(), Family::ALL.len(), "two families share a colour");
    }

    /// A picture, when the look has one.
    ///
    /// The whole point of the tile. Asserted by counting painted images
    /// rather than by reading text: a thumbnail draws nothing a string
    /// search can see, which is exactly how a broken one would go
    /// unnoticed.
    #[test]
    fn a_look_with_a_picture_paints_it() {
        let _guard = crate::project_bar::tests::scoped_config("perf-thumbs");
        let picture = vizz_mod::thumb::Thumb {
            width: 8,
            height: 5,
            rgba: vec![255; 8 * 5 * 4],
        };
        vizz_mod::thumb::save("Bloom", &picture).expect("saving a picture");
        let reg = registry();
        let mut macros = Macros::default();
        let library = vec![look("Bloom", "sphere"), look("Knotted", "knot")];
        let images = |sheet: &Sheet| -> usize { sheet.images };
        let with = with_presets(library.clone(), || {
            sheet_sized(
                &mut macros,
                &reg,
                &MidiView::default(),
                None,
                None,
                vec2(1440.0, 900.0),
            )
        });
        vizz_mod::thumb::remove("Bloom");
        // A fresh context, or the cached handle from the first pass would
        // answer for the second — see [`crate::thumbs`].
        let without = with_presets(library, || {
            sheet_sized(
                &mut macros,
                &reg,
                &MidiView::default(),
                None,
                None,
                vec2(1440.0, 900.0),
            )
        });
        assert_eq!(
            images(&with),
            images(&without) + 1,
            "the look with a picture painted {} images, the one without {}",
            images(&with),
            images(&without)
        );
    }

    /// Every look can be photographed from its own tile, whether or not
    /// there is a controller plugged in. A look saved before pictures
    /// existed has no other way to get one.
    #[test]
    fn a_tile_offers_to_take_a_fresh_picture() {
        let reg = registry();
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let mut macros = Macros::default();
        let library = vec![look("Bloom", "sphere")];
        let audio = AudioView::default();
        let grid = crate::grid_view::GridView::default();
        let size = vec2(1440.0, 900.0);

        let mut frame = |events: Vec<egui::Event>| -> (PerformanceActions, Vec<(String, egui::Pos2)>) {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            });
            let state = PerformanceState {
                project: "Show 1",
                decks: &[],
                active_deck: 0,
                follow_columns: None,
                recording: None,
                preset_current: None,
                outputs: &[],
                audio: &audio,
                fps: 60.0,
                over_budget: false,
                bpm: 128.0,
                bar_phase: 0.1,
                presets: &library,
                thumb_revision: 0,
                grid: &grid,
                gravity: None,
                midi: &MidiView::default(),
                values: None,
                output_texture: None,
                output_aspect: 16.0 / 9.0,
                graph: None,
            };
            let actions = draw(&ctx, &reg, &state, &mut macros);
            let out = ctx.end_pass();
            let mut runs = Vec::new();
            for c in &out.shapes {
                let mut found = Vec::new();
                collect_text(&c.shape, &mut found);
                runs.extend(found.into_iter().map(|p| (p.text, p.rect.center())));
            }
            (actions, runs)
        };

        // A few passes first: a scroll area does not know its content on
        // the frame it is created, and the tile is inside one.
        for _ in 0..4 {
            frame(Vec::new());
        }
        let (_, runs) = frame(Vec::new());
        let tile = runs
            .iter()
            .find(|(t, _)| t.trim() == "Bloom")
            .map(|(_, at)| *at)
            .expect("the preset tile was not painted");
        // Right-click the tile: the name sits on the tile, so its centre
        // is inside it.
        frame(vec![
            egui::Event::PointerMoved(tile),
            egui::Event::PointerButton {
                pos: tile,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos: tile,
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: Default::default(),
            },
        ]);
        // The menu is opened by the click and drawn on the frame after.
        let (_, runs) = frame(vec![egui::Event::PointerMoved(tile)]);
        let item = runs
            .iter()
            .find(|(t, _)| t.trim() == "update picture")
            .map(|(_, at)| *at)
            .unwrap_or_else(|| {
                panic!(
                    "no way to photograph a look: {:?}",
                    runs.iter().map(|(t, _)| t.trim()).collect::<Vec<_>>()
                )
            });
        let (actions, _) = frame(vec![
            egui::Event::PointerMoved(item),
            egui::Event::PointerButton {
                pos: item,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos: item,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ]);
        assert_eq!(
            actions.preset_rephoto.as_deref(),
            Some("Bloom"),
            "the menu item paints but does nothing"
        );
    }

    fn render(macros: &mut Macros, reg: &ParamRegistry) -> String {
        render_with(macros, reg, &MidiView::default(), None)
    }

    fn render_with_recording(
        macros: &mut Macros,
        reg: &ParamRegistry,
        rec: crate::RecordingView,
    ) -> String {
        render_inner(macros, reg, &MidiView::default(), None, Some(rec))
    }

    fn render_with(
        macros: &mut Macros,
        reg: &ParamRegistry,
        midi: &MidiView,
        values: Option<&[f32]>,
    ) -> String {
        render_inner(macros, reg, midi, values, None)
    }

    /// What a galley actually puts on screen, glyph by glyph.
    ///
    /// `Galley::text()` returns the string the galley was *given*, not
    /// the one it drew: a label elided to "particles …" still reports
    /// "particles spread", and a wrapped label reports its whole text
    /// however little of it fits. Every assertion in this module is a
    /// claim about what a performer can read in a dark room, so reading
    /// back the source string makes those claims unfalsifiable — a label
    /// clipped to nothing passes a `contains` check for its full name.
    /// The glyphs are what was rasterised, ellipsis included.
    /// Whether a run of text is inside the window at all.
    ///
    /// Shapes are emitted whether or not they land on screen: at
    /// 1024x640 the whole fader label row sits below the bottom edge and
    /// still arrives in this list. Reading it back unfiltered means a
    /// layout that has pushed its labels off the window — the exact
    /// regression the fader chrome allowance exists to prevent — passes
    /// every assertion here.
    fn on_screen(shape: &egui::epaint::TextShape, screen: egui::Rect) -> bool {
        let rect = egui::Rect::from_min_size(shape.pos, shape.galley.rect.size());
        screen.contains_rect(rect)
    }

    fn painted(galley: &egui::Galley) -> String {
        galley
            .rows
            .iter()
            .flat_map(|row| row.glyphs.iter().map(|g| g.chr))
            .collect()
    }

    fn render_inner(
        macros: &mut Macros,
        reg: &ParamRegistry,
        midi: &MidiView,
        values: Option<&[f32]>,
        recording: Option<crate::RecordingView>,
    ) -> String {
        render_at(macros, reg, midi, values, recording, 1280.0)
    }

    fn render_at_size(macros: &mut Macros, reg: &ParamRegistry, size: Vec2) -> String {
        render_sized(macros, reg, &MidiView::default(), None, None, size)
    }

    fn render_at(
        macros: &mut Macros,
        reg: &ParamRegistry,
        midi: &MidiView,
        values: Option<&[f32]>,
        recording: Option<crate::RecordingView>,
        width: f32,
    ) -> String {
        render_sized(macros, reg, midi, values, recording, vec2(width, 800.0))
    }



    /// A layer can be started from the screen you play on.
    ///
    /// The strip used to return early when every layer was off, so the
    /// vector layers were invisible here until one was already running
    /// — and the only way to start one was the parameter list on the
    /// other screen. A feature unreachable from the layout you play on,
    /// and undiscoverable from it too.
    ///
    /// Driven through a real click, because "the button is painted" and
    /// "the button starts a layer" are different claims and only the
    /// second one matters.
    #[test]
    fn a_layer_can_be_started_from_the_performance_screen() {
        let reg = registry();
        let kind = reg.id("/l1/kind").expect("no layer in the test registry");
        assert_eq!(reg.target(kind), 0.0, "the fixture starts with a layer on");

        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let mut macros = Macros::default();
        let size = vec2(1440.0, 900.0);

        let mut frame = |click: Option<egui::Pos2>| -> Vec<(String, egui::Rect)> {
            let audio = AudioView::default();
            let names = [crate::PresetEntry::from("Slow bloom")];
            let grid = crate::grid_view::GridView::default();
            let midi = MidiView::default();
            let state = PerformanceState {
                project: "Show 1",
            decks: &[],
                active_deck: 0,
                follow_columns: None,
                recording: None,
                preset_current: None,
                outputs: &[],
                audio: &audio,
                fps: 60.0,
                over_budget: false,
                bpm: 128.0,
                bar_phase: 0.1,
                presets: &names,
                thumb_revision: 0,
                grid: &grid,
                gravity: None,
                midi: &midi,
                values: None,
                output_texture: None,
                output_aspect: 16.0 / 9.0,
                graph: None,
            };
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            if let Some(at) = click {
                input.events.push(egui::Event::PointerMoved(at));
                for pressed in [true, false] {
                    input.events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    });
                }
            }
            ctx.begin_pass(input);
            draw(&ctx, &reg, &state, &mut macros);
            let out = ctx.end_pass();
            let mut items = Vec::new();
            for p in &out.shapes {
                collect_text(&p.shape, &mut items);
            }
            items.into_iter().map(|p| (p.text, p.rect)).collect()
        };

        frame(None);
        let painted = frame(None);
        let joined: Vec<&str> = painted.iter().map(|(t, _)| t.trim()).collect();
        assert!(
            joined.contains(&"LAYERS"),
            "the layers section is invisible with nothing on: {joined:?}"
        );
        let start = painted
            .iter()
            .find(|(t, _)| t.starts_with("+ "))
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| panic!("no way to start a layer: {joined:?}"));

        frame(Some(start));
        assert!(
            reg.target(kind) >= 0.5,
            "clicking the start button left every layer off"
        );
        // And it landed on a real generator, not merely off-by-one into
        // something unnamed.
        let def = &reg.defs()[kind.index()];
        assert!(
            def.label_for(reg.target(kind)).is_some_and(|l| l != "off"),
            "the layer started on {:?}",
            def.label_for(reg.target(kind))
        );
    }

    /// The generator menu names every position and marks the current one.
    ///
    /// The wheel alone says where you are and never that there is
    /// anywhere else to be; eight positions also means seven clicks to
    /// cross. This asserts against the parameter's own labels rather
    /// than a copy of them, so adding a generator cannot leave the menu
    /// quietly one short.
    #[test]
    fn the_generator_menu_offers_every_kind() {
        let reg = registry();
        let kind = reg.id("/l1/kind").unwrap();
        reg.set(kind, 2.0);
        let def = &reg.defs()[kind.index()];

        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        // Two passes: an Area is placed on the first and painted on the
        // second, so reading the first gives an empty sheet and says
        // nothing about the menu.
        let mut items = Vec::new();
        for _ in 0..2 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(400.0, 400.0),
                )),
                ..Default::default()
            });
            egui::Area::new(egui::Id::new("menu-probe")).show(&ctx, |ui| {
                stepped_menu(ui, &reg, kind, "layer 1");
            });
            let out = ctx.end_pass();
            items.clear();
            for p in &out.shapes {
                collect_text(&p.shape, &mut items);
            }
        }
        let painted: Vec<String> = items.iter().map(|p| p.text.trim().to_string()).collect();

        let steps = (def.max - def.min).round() as i32;
        for step in 0..=steps {
            let want = def.label_for(def.min + step as f32).unwrap();
            assert!(
                painted.iter().any(|t| t == want),
                "the menu is missing {want:?}: {painted:?}"
            );
        }
    }

    /// The faders are on screen at every window size worth playing on.
    ///
    /// Presence, not position — and that distinction is the whole point
    /// of this test. egui culls shapes outside the clip rect, so a label
    /// laid out below the window bottom does not arrive as an
    /// off-screen shape: it does not arrive at all. Checking for
    /// stragglers past the edge therefore cannot catch this, and the
    /// first version of this test did exactly that and passed against a
    /// build with no faders in it.
    ///
    /// What it caught once the assertion was turned round: at 1024x640
    /// and 900x700 the entire block — every fader and the master with
    /// it — was laid out under the window and culled. The CONTROLS
    /// caption drew over nothing, and the dim fader that recovers a
    /// black output was gone.
    ///
    /// The master matters most and is asserted separately. Everything
    /// else on this screen is a thing you reach for; that one is the
    /// thing you reach for when the output is already black.
    #[test]
    fn the_faders_survive_every_window_worth_playing_on() {
        let reg = registry();
        for size in [
            vec2(1280.0, 720.0),
            vec2(1280.0, 680.0),
            vec2(1024.0, 640.0),
            vec2(1440.0, 900.0),
            vec2(1920.0, 1080.0),
            vec2(900.0, 700.0),
            vec2(1100.0, 620.0),
        ] {
            let mut macros = Macros::default();
            let sheet = sheet_sized(&mut macros, &reg, &MidiView::default(), None, None, size);
            let text = sheet.text();
            assert!(
                text.contains("MASTER"),
                "at {}x{} the master fader is not on screen: {text}",
                size.x,
                size.y
            );
            // And a named macro fader, so "the master survived alone"
            // cannot pass for the block being there.
            assert!(
                text.contains("size"),
                "at {}x{} the macro faders are not on screen: {text}",
                size.x,
                size.y
            );
            // Nothing legible may sit outside the window either. This
            // cannot catch a culled label, but it does catch one that
            // hangs over an edge rather than being dropped past it.
            let off = sheet.offscreen();
            assert!(
                off.is_empty(),
                "at {}x{}, {:?} was painted outside the window",
                size.x,
                size.y,
                off.iter().map(|p| p.text.trim()).collect::<Vec<_>>()
            );
        }
    }

    /// The set list is on the screen you play on, named, and the live
    /// page is not the one you have to guess at.
    ///
    /// The row exists to answer "which song's pads am I looking at"
    /// across a dark room. A row that draws but does not name the pages,
    /// or names them all identically, answers nothing.
    #[test]
    fn the_deck_row_names_every_page() {
        let reg = registry();
        let mut macros = Macros::default();
        let sheet = with_decks(
            vec![chip("opener"), chip("second song"), chip("encore")],
            || sheet_sized(&mut macros, &reg, &MidiView::default(), None, None, vec2(1440.0, 900.0)),
        );
        let text = sheet.text();
        for name in ["opener", "second song", "encore"] {
            assert!(text.contains(name), "'{name}' is not on the desk: {text}");
        }
        // And the way to make another one, which is the only route to a
        // second page there is.
        assert!(text.contains('+'), "no way to add a page: {text}");
    }

    /// The Resolume switch is offered before it is on.
    ///
    /// Following is off by default, so a toggle that only appeared once
    /// following was already running would be a switch nobody could ever
    /// find to turn on — the locked-door shape this codebase has shipped
    /// once already with the gravity sequencer.
    #[test]
    fn the_resolume_switch_is_visible_while_it_is_off() {
        let reg = registry();
        let mut macros = Macros::default();
        let sheet = with_decks(vec![chip("opener")], || {
            sheet_sized(&mut macros, &reg, &MidiView::default(), None, None, vec2(1440.0, 900.0))
        });
        assert!(
            sheet.text().contains("resolume"),
            "the column-follow switch is not on the desk: {}",
            sheet.text()
        );
    }

    /// Without a book there is no row at all.
    ///
    /// Every context that is not the live app — the mockups, the panel
    /// preview — hands an empty set list, and a row of nothing would take
    /// height off the picture to say that there are no pages.
    #[test]
    fn no_book_means_no_row() {
        let reg = registry();
        let mut macros = Macros::default();
        let sheet = sheet_sized(&mut macros, &reg, &MidiView::default(), None, None, vec2(1440.0, 900.0));
        assert!(
            !sheet.text().contains("resolume"),
            "the deck row drew itself with no pages to show: {}",
            sheet.text()
        );
    }

    /// The faders survive every window worth playing on *with* a full set
    /// list on the desk.
    ///
    /// The row costs height off the pads block, and the pads block is what
    /// the fader budget subtracts — in the single-column layout there is
    /// no gap to absorb it, so it comes straight off `room`, the number
    /// that decides whether the fader block is laid out below the window
    /// and culled entirely. `the_faders_survive_every_window_worth_playing_on`
    /// cannot catch that: it runs with no decks and no row.
    ///
    /// Sixteen pages with long names, which is the worst the row can be —
    /// a full book, every chip at its widest, wrapping to as many lines as
    /// it takes. Probed by adding height at the row's position: the
    /// assertions here start failing at around three hundred points, so
    /// they are measuring the budget and not merely the strings.
    #[test]
    fn the_faders_survive_every_window_with_a_full_set_list_on_the_desk() {
        let reg = registry();
        let decks: Vec<DeckChip> = (1..=vizz_mod::deck::MAX_DECKS)
            .map(|n| chip(&format!("song {n} with a long title")))
            .collect();
        for size in [
            vec2(1280.0, 720.0),
            vec2(1280.0, 680.0),
            vec2(1024.0, 640.0),
            vec2(1440.0, 900.0),
            vec2(1920.0, 1080.0),
            vec2(900.0, 700.0),
            vec2(1100.0, 620.0),
        ] {
            let mut macros = Macros::default();
            let sheet = with_decks(decks.clone(), || {
                sheet_sized(&mut macros, &reg, &MidiView::default(), None, None, size)
            });
            let text = sheet.text();
            assert!(
                text.contains("MASTER"),
                "at {}x{} the master fader is not on screen with a set list up: {text}",
                size.x,
                size.y
            );
            assert!(
                text.contains("size"),
                "at {}x{} the macro faders are not on screen with a set list up: {text}",
                size.x,
                size.y
            );
            let off = sheet.offscreen();
            assert!(
                off.is_empty(),
                "at {}x{}, {:?} was painted outside the window",
                size.x,
                size.y,
                off.iter().map(|p| p.text.trim()).collect::<Vec<_>>()
            );
        }
    }

    /// A song title long enough to be a real one does not push the row
    /// past the right edge.
    ///
    /// Chips are laid out wrapped, so a name that refuses to shrink grows
    /// the row instead — which in the single column comes off the faders.
    /// `fit_label` is what stops it, and nothing else would.
    #[test]
    fn a_long_song_title_stays_inside_the_window() {
        let reg = registry();
        let mut macros = Macros::default();
        let decks = vec![
            chip("the one with the very long name that nobody would type"),
            chip("also quite a long name for a song"),
        ];
        let sheet = with_decks(decks, || {
            sheet_sized(&mut macros, &reg, &MidiView::default(), None, None, vec2(1024.0, 640.0))
        });
        let off = sheet.offscreen();
        assert!(
            off.is_empty(),
            "{:?} was painted outside the window",
            off.iter().map(|p| p.text.trim()).collect::<Vec<_>>()
        );
    }

    /// The open show is named, and named first.
    ///
    /// A presence test, not an absence one: egui culls anything laid out
    /// past the clip rect, so a chip that failed to fit would simply not
    /// arrive and every "nothing spilled" assertion would still pass.
    #[test]
    fn the_open_show_is_named_at_the_start_of_the_strip() {
        let reg = registry();
        let mut macros = Macros::default();
        let midi = MidiView::default();
        let sheet = sheet_sized(
            &mut macros,
            &reg,
            &midi,
            None,
            None,
            vec2(1440.0, 900.0),
        );
        let show = sheet
            .items
            .iter()
            .find(|p| p.text.trim() == "Show 1")
            .unwrap_or_else(|| {
                panic!(
                    "the open show is not named anywhere on the strip: {:?}",
                    sheet
                        .items
                        .iter()
                        .map(|p| p.text.trim())
                        .filter(|t| !t.is_empty())
                        .collect::<Vec<_>>()
                )
            });
        // Ahead of the output lights, which are the next thing along.
        let output = sheet
            .items
            .iter()
            .find(|p| p.text.contains("syphon"))
            .expect("no output name on the strip");
        assert!(
            show.rect.left() < output.rect.left(),
            "the show name is not first: it sits at {} and the output name at {}",
            show.rect.left(),
            output.rect.left()
        );
    }

    /// Making a new show works from the chip, click by click.
    ///
    /// The lesson of the built-in set, applied before it can be repeated:
    /// a menu item that paints and cannot be reached is the feature not
    /// existing. This one carries a text field as well, which egui's
    /// default close-on-click would take away on the first press.
    #[test]
    fn a_new_show_can_be_made_from_the_chip() {
        let _guard = crate::project_bar::tests::scoped_config("perf-newshow");
        let reg = registry();
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let mut macros = Macros::default();
        let size = vec2(1440.0, 900.0);
        let decks = vec![chip("deck 1")];

        let mut frame = |events: Vec<egui::Event>| -> (PerformanceActions, Vec<(String, egui::Pos2)>) {
            let audio = AudioView::default();
            let names = [crate::PresetEntry::from("Slow bloom")];
            let grid = crate::grid_view::GridView::default();
            let midi = MidiView::default();
            let state = PerformanceState {
                project: "Show 1",
                decks: &decks,
                active_deck: 0,
                follow_columns: Some(false),
                recording: None,
                preset_current: None,
                outputs: &[],
                audio: &audio,
                fps: 60.0,
                over_budget: false,
                bpm: 128.0,
                bar_phase: 0.1,
                presets: &names,
                thumb_revision: 0,
                grid: &grid,
                gravity: None,
                midi: &midi,
                values: None,
                output_texture: None,
                output_aspect: 16.0 / 9.0,
                graph: None,
            };
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            });
            let actions = draw(&ctx, &reg, &state, &mut macros);
            let out = ctx.end_pass();
            let mut found = Vec::new();
            fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    egui::Shape::Text(t) => out.push((
                        painted(&t.galley),
                        egui::Rect::from_min_size(t.pos, t.galley.rect.size()).center(),
                    )),
                    _ => {}
                }
            }
            for p in &out.shapes {
                walk(&p.shape, &mut found);
            }
            (actions, found)
        };

        let press = |at: egui::Pos2| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]
        };

        let mut painted_now = Vec::new();
        for _ in 0..8 {
            painted_now = frame(Vec::new()).1;
        }
        let chip_at = painted_now
            .iter()
            .find(|(t, _)| t.trim() == "Show 1")
            .map(|(_, p)| *p)
            .expect("no show chip on the strip");

        // Left-click opens it: this is a menu, not a context menu.
        frame(press(chip_at));
        let mut with_menu = Vec::new();
        for _ in 0..3 {
            with_menu = frame(Vec::new()).1;
        }
        let item = with_menu
            .iter()
            .find(|(t, _)| t.contains("new show"))
            .map(|(_, p)| *p)
            .unwrap_or_else(|| {
                panic!(
                    "no way to make a show from the chip: {:?}",
                    with_menu.iter().map(|(t, _)| t.trim()).collect::<Vec<_>>()
                )
            });

        frame(press(item));
        let mut prompt = Vec::new();
        for _ in 0..3 {
            prompt = frame(Vec::new()).1;
        }
        let ok = prompt
            .iter()
            .find(|(t, _)| t.trim() == "ok")
            .map(|(_, p)| *p)
            .unwrap_or_else(|| {
                panic!(
                    "the name field did not survive the click that opened it: {:?}",
                    prompt.iter().map(|(t, _)| t.trim()).collect::<Vec<_>>()
                )
            });
        let (actions, _) = frame(press(ok));
        assert_eq!(
            actions.project.create.as_deref(),
            Some("Show 2"),
            "the confirming click did not ask for a new show"
        );
    }

    /// The built-in set can actually be loaded from the desk.
    ///
    /// Reported behaviour is that it cannot. The set installs itself only
    /// on a machine that has never had a show, so for anyone who has ever
    /// filled a pad the right-click on `+` is the *only* route — and an
    /// only route that does not work is the feature not existing.
    ///
    /// Driven through real pointer events, because "the menu item is
    /// painted" and "the menu item loads the set" are different claims and
    /// only the second one matters. The arming click and the confirming
    /// click are separate frames, exactly as a hand would send them.
    #[test]
    fn the_built_in_set_can_be_loaded_from_the_deck_row() {
        let reg = registry();
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let mut macros = Macros::default();
        let size = vec2(1440.0, 900.0);
        let decks = vec![chip("deck 1")];

        // (label, centre) for every text run, so the menu items can be hit
        // where they actually landed rather than where they were meant to.
        let mut frame = |events: Vec<egui::Event>| -> (PerformanceActions, Vec<(String, egui::Pos2)>) {
            let audio = AudioView::default();
            let names = [crate::PresetEntry::from("Slow bloom")];
            let grid = crate::grid_view::GridView::default();
            let midi = MidiView::default();
            let state = PerformanceState {
                project: "Show 1",
                decks: &decks,
                active_deck: 0,
                follow_columns: Some(false),
                recording: None,
                preset_current: None,
                outputs: &[],
                audio: &audio,
                fps: 60.0,
                over_budget: false,
                bpm: 128.0,
                bar_phase: 0.1,
                presets: &names,
                thumb_revision: 0,
                grid: &grid,
                gravity: None,
                midi: &midi,
                values: None,
                output_texture: None,
                output_aspect: 16.0 / 9.0,
                graph: None,
            };
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            });
            let actions = draw(&ctx, &reg, &state, &mut macros);
            let out = ctx.end_pass();
            let mut found = Vec::new();
            fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    egui::Shape::Text(t) => out.push((
                        painted(&t.galley),
                        egui::Rect::from_min_size(t.pos, t.galley.rect.size()).center(),
                    )),
                    _ => {}
                }
            }
            for p in &out.shapes {
                walk(&p.shape, &mut found);
            }
            (actions, found)
        };

        let press = |at: egui::Pos2, button: egui::PointerButton| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]
        };

        // Settle. The layout animates and areas need a second pass, so a
        // single frame is a picture of the desk mid-assembly.
        let mut painted_now = Vec::new();
        for _ in 0..8 {
            painted_now = frame(Vec::new()).1;
        }
        let plus = painted_now
            .iter()
            .find(|(t, _)| t.trim() == "+")
            .map(|(_, p)| *p)
            .expect("no + button beside the deck chips");

        // Right-click it. The menu opens on the next frame.
        frame(press(plus, egui::PointerButton::Secondary));
        let mut with_menu = Vec::new();
        for _ in 0..3 {
            with_menu = frame(Vec::new()).1;
        }
        let item = with_menu
            .iter()
            .find(|(t, _)| t.contains("load the built-in set"))
            .map(|(_, p)| *p)
            .unwrap_or_else(|| {
                panic!(
                    "no way to load the set on the + menu: {:?}",
                    with_menu.iter().map(|(t, _)| t.trim()).collect::<Vec<_>>()
                )
            });

        // Arm it, then confirm. The item relabels between the two.
        frame(press(item, egui::PointerButton::Primary));
        let mut armed = Vec::new();
        for _ in 0..3 {
            armed = frame(Vec::new()).1;
        }
        let confirm = armed
            .iter()
            .find(|(t, _)| t.contains("replace every page"))
            .map(|(_, p)| *p)
            .unwrap_or_else(|| {
                panic!(
                    "the menu item did not arm — it is unreachable in two clicks: {:?}",
                    armed.iter().map(|(t, _)| t.trim()).collect::<Vec<_>>()
                )
            });
        let (actions, _) = frame(press(confirm, egui::PointerButton::Primary));
        assert!(
            actions.decks.install_set,
            "the confirming click did not ask for the set"
        );
    }

    /// Deleting a page works too, by the same route and for the same
    /// reason: it is the other armed button living in a menu, and it was
    /// broken in exactly the same way.
    #[test]
    fn a_page_can_be_deleted_from_its_own_chip() {
        let reg = registry();
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let mut macros = Macros::default();
        let size = vec2(1440.0, 900.0);
        // Two pages, because the last one cannot be deleted.
        let decks = vec![chip("opener"), chip("encore")];

        let mut frame = |events: Vec<egui::Event>| -> (PerformanceActions, Vec<(String, egui::Pos2)>) {
            let audio = AudioView::default();
            let names = [crate::PresetEntry::from("Slow bloom")];
            let grid = crate::grid_view::GridView::default();
            let midi = MidiView::default();
            let state = PerformanceState {
                project: "Show 1",
                decks: &decks,
                active_deck: 0,
                follow_columns: Some(false),
                recording: None,
                preset_current: None,
                outputs: &[],
                audio: &audio,
                fps: 60.0,
                over_budget: false,
                bpm: 128.0,
                bar_phase: 0.1,
                presets: &names,
                thumb_revision: 0,
                grid: &grid,
                gravity: None,
                midi: &midi,
                values: None,
                output_texture: None,
                output_aspect: 16.0 / 9.0,
                graph: None,
            };
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            });
            let actions = draw(&ctx, &reg, &state, &mut macros);
            let out = ctx.end_pass();
            let mut found = Vec::new();
            fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    egui::Shape::Text(t) => out.push((
                        painted(&t.galley),
                        egui::Rect::from_min_size(t.pos, t.galley.rect.size()).center(),
                    )),
                    _ => {}
                }
            }
            for p in &out.shapes {
                walk(&p.shape, &mut found);
            }
            (actions, found)
        };
        let press = |at: egui::Pos2, button: egui::PointerButton| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button, pressed: true, modifiers: Default::default() },
                egui::Event::PointerButton { pos: at, button, pressed: false, modifiers: Default::default() },
            ]
        };
        let find = |painted: &[(String, egui::Pos2)], want: &str| -> egui::Pos2 {
            painted
                .iter()
                .find(|(t, _)| t.contains(want))
                .map(|(_, p)| *p)
                .unwrap_or_else(|| {
                    panic!(
                        "no {want:?}: {:?}",
                        painted.iter().map(|(t, _)| t.trim()).collect::<Vec<_>>()
                    )
                })
        };

        let mut painted_now = Vec::new();
        for _ in 0..8 {
            painted_now = frame(Vec::new()).1;
        }
        let chip_at = find(&painted_now, "encore");

        frame(press(chip_at, egui::PointerButton::Secondary));
        let mut menu = Vec::new();
        for _ in 0..3 {
            menu = frame(Vec::new()).1;
        }
        let item = find(&menu, "delete page");

        frame(press(item, egui::PointerButton::Primary));
        let mut armed = Vec::new();
        for _ in 0..3 {
            armed = frame(Vec::new()).1;
        }
        let confirm = find(&armed, "delete for good");
        let (actions, _) = frame(press(confirm, egui::PointerButton::Primary));
        assert_eq!(
            actions.decks.delete,
            Some(1),
            "the confirming click did not delete the page it was opened on"
        );
    }

    /// A full preset library does not push the desk apart.
    ///
    /// The row draws one button per preset and was written when a library
    /// was a handful. Installing the built-in set makes it a hundred and
    /// sixty, wrapped — a block taller than the column it sits in, which
    /// takes the faders and the master off the bottom of the window with
    /// it. Reported as "presets now overflow the container", and it is the
    /// set that made an old assumption load-bearing.
    #[test]
    fn a_large_preset_library_does_not_overflow_the_desk() {
        let reg = registry();
        let library = a_set_of_presets();
        assert_eq!(library.len(), 160);
        for size in [
            vec2(1440.0, 900.0),
            vec2(1920.0, 1080.0),
            vec2(1280.0, 720.0),
            vec2(1280.0, 900.0),
        ] {
            let mut macros = Macros::default();
            // A full library *and* a full set list: the two arrive
            // together, since installing the set is what produces both.
            let chips: Vec<DeckChip> = (1..=20).map(|n| chip(&format!("{n:02} Song Title"))).collect();
            let sheet = with_presets(library.clone(), || {
                with_decks(chips.clone(), || {
                    sheet_sized(&mut macros, &reg, &MidiView::default(), None, None, size)
                })
            });
            let text = sheet.text();
            // Presence, not absence. egui culls anything laid out past
            // the clip rect, so an unbounded row of a hundred and sixty
            // buttons does not paint *outside* the column — it does not
            // paint at all, while still taking the space. Measured: the
            // original draws zero preset runs here. Asserting that
            // nothing spilled would therefore have passed against the
            // bug, and did, until this was turned round — the same trap
            // that once passed a fader test against a build with no
            // faders in it.
            let shown = sheet
                .items
                .iter()
                .filter(|p| library.iter().any(|n| p.text.contains(n.name.as_str())))
                .count();
            assert!(
                shown > 0,
                "at {}x{} the preset row painted nothing — a full library is laid out past its clip",
                size.x,
                size.y
            );
            // The chips are the only way to change song. A library that
            // pushes them off the desk takes the set with it.
            assert!(
                text.contains("01 Song Title"),
                "at {}x{} a full library pushed the deck chips off the desk",
                size.x,
                size.y
            );
            assert!(
                text.contains("MASTER"),
                "at {}x{} a full library pushed the master off the desk",
                size.x,
                size.y
            );
            assert!(
                text.contains("size"),
                "at {}x{} a full library pushed the faders off the desk",
                size.x,
                size.y
            );
            let off = sheet.offscreen();
            assert!(
                off.is_empty(),
                "at {}x{}, {:?} was painted outside the window",
                size.x,
                size.y,
                off.iter().map(|p| p.text.trim()).take(8).collect::<Vec<_>>()
            );

            // And inside its own column, which is the failure that was
            // actually reported. `offscreen` cannot see this: the row
            // stayed within the window the whole time, it just wrapped at
            // the window's width instead of the column's and painted
            // itself across the output pane.
            let desk = size.x >= DESK_MIN_W;
            let right = PAD + column_width(size.x - PAD * 2.0, desk);
            // And what it does paint stays inside its column.
            let spilled: Vec<&str> = sheet
                .items
                .iter()
                .filter(|p| library.iter().any(|n| p.text.contains(n.name.as_str())))
                .filter(|p| p.rect.right() > right + 1.0)
                .map(|p| p.text.trim())
                .take(6)
                .collect();
            assert!(
                spilled.is_empty(),
                "at {}x{} the preset row spilled past the column at x={right}: {spilled:?}",
                size.x,
                size.y
            );
        }
    }

    /// One run of text as it was actually painted, and where it landed.
    #[derive(Clone, Debug)]
    struct Painted {
        text: String,
        rect: egui::Rect,
    }

    /// Everything the layout painted at one size, with positions.
    ///
    /// The tests in this module could previously see *that* a string was
    /// drawn and nothing else. That is enough to catch a missing label
    /// and blind to every way a label can be present and useless: off
    /// the bottom of the window, under another label, or clipped to
    /// nothing. Several layout bugs this module exists to prevent were
    /// found by looking at a screenshot instead, which does not scale
    /// and does not run in CI.
    struct Sheet {
        items: Vec<Painted>,
        screen: egui::Rect,
        /// How many images the pass painted. A thumbnail draws nothing a
        /// string search can see, so the count is the only way a test can
        /// tell a tile with a picture from one without.
        images: usize,
        /// Every rectangle fill the pass painted. The layout's colour
        /// claims — a family's own tint, a state colour — are invisible
        /// to a string search, which is exactly how a code that stopped
        /// being painted would go unnoticed.
        fills: Vec<egui::Color32>,
    }

    impl Sheet {
        /// The on-screen text, joined — what the string-based tests read.
        fn text(&self) -> String {
            self.items
                .iter()
                .filter(|p| self.screen.contains_rect(p.rect))
                .map(|p| p.text.clone())
                .collect::<Vec<_>>()
                .join(" ")
        }

        /// Runs that were painted outside the window.
        ///
        /// Blank runs are ignored: the layout paints a space to hold a
        /// row's height where a chip would go, and a space nobody can
        /// see falling off the edge is not a bug.
        fn offscreen(&self) -> Vec<&Painted> {
            self.items
                .iter()
                .filter(|p| !p.text.trim().is_empty())
                .filter(|p| !self.screen.contains_rect(p.rect))
                .collect()
        }
    }

    /// Collect every rectangle fill, nested groups included.
    fn collect_fills(shape: &egui::Shape, out: &mut Vec<egui::Color32>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_fills(s, out)),
            egui::Shape::Rect(r) => out.push(r.fill),
            _ => {}
        }
    }

    /// Count painted images, nested groups included.
    ///
    /// An image reaches the paint list as a mesh carrying a texture, so
    /// the test is the texture rather than the shape: everything egui
    /// draws itself — text, rectangles, strokes — is a mesh on the font
    /// atlas, which is managed texture zero.
    fn count_images(shape: &egui::Shape, out: &mut usize) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| count_images(s, out)),
            egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::Managed(0) => *out += 1,
            _ => {}
        }
    }

    /// Collect every text run, including those nested inside groups.
    ///
    /// Recursive on purpose. The flat version missed anything egui
    /// emitted inside a `Shape::Vec` — which is most of what a widget
    /// draws — so "this text is not painted" was a claim the harness
    /// could not actually make.
    fn collect_text(shape: &egui::Shape, out: &mut Vec<Painted>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_text(s, out)),
            egui::Shape::Text(t) => out.push(Painted {
                text: painted(&t.galley),
                rect: egui::Rect::from_min_size(t.pos, t.galley.rect.size()),
            }),
            _ => {}
        }
    }

    fn render_sized(
        macros: &mut Macros,
        reg: &ParamRegistry,
        midi: &MidiView,
        values: Option<&[f32]>,
        recording: Option<crate::RecordingView>,
        size: Vec2,
    ) -> String {
        sheet_sized(macros, reg, midi, values, recording, size).text()
    }

    fn sheet_sized(
        macros: &mut Macros,
        reg: &ParamRegistry,
        midi: &MidiView,
        values: Option<&[f32]>,
        recording: Option<crate::RecordingView>,
        size: Vec2,
    ) -> Sheet {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        if RENDER_PEEK.with(|f| f.get()) {
            ctx.data_mut(|d| d.insert_temp(peek_id(), true));
        }
        let audio = AudioView::default();
        let names = SHEET_PRESETS.with(|f| {
            let extra = f.borrow().clone();
            if extra.is_empty() {
                vec![crate::PresetEntry::from("Slow bloom"), crate::PresetEntry::from("Butterfly")]
            } else {
                extra
            }
        });
        let grid = crate::grid_view::GridView::default();
        let decks = SHEET_DECKS.with(|f| f.borrow().clone());
        let state = PerformanceState {
            project: "Show 1",
            decks: &decks,
            active_deck: 1,
            follow_columns: (!decks.is_empty()).then_some(false),
            recording,
            preset_current: None,
            outputs: &[OutputStatus {
                name: "syphon:vizz".into(),
                live: true,
            }],
            audio: &audio,
            fps: 60.0,
            over_budget: false,
            bpm: 128.0,
            bar_phase: 0.1,
            presets: &names,
            thumb_revision: 0,
            grid: &grid,
            gravity: None,
            midi,
            values,
            output_texture: None,
            output_aspect: 16.0 / 9.0,
            graph: None,
        };
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let mut items = Vec::new();
        let mut images = 0usize;
        let mut fills = Vec::new();
        // Eight passes so the layout's own animations settle; only the
        // last is read, for the same reason a photograph of a fader
        // mid-glide tells you nothing about where it came to rest.
        for i in 0..8 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(screen),
                time: Some(i as f64 * 0.05),
                ..Default::default()
            });
            draw(&ctx, reg, &state, macros);
            let out = ctx.end_pass();
            items.clear();
            images = 0;
            fills.clear();
            for p in &out.shapes {
                collect_text(&p.shape, &mut items);
                count_images(&p.shape, &mut images);
                collect_fills(&p.shape, &mut fills);
            }
        }
        Sheet { items, screen, images, fills }
    }

    /// Every painted run with the colour it was painted in.
    ///
    /// The layout's central claim is that colour carries meaning — the
    /// ink ramp, and the readout that turns warm when what you are
    /// reading is not what the parameter is currently at. The string
    /// view throws all of that away, so a readout that lost its colour
    /// coding entirely would pass every other test in this module.
    fn render_coloured(
        macros: &mut Macros,
        reg: &ParamRegistry,
        values: Option<&[f32]>,
    ) -> Vec<(String, Color32)> {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let audio = AudioView::default();
        let names = [crate::PresetEntry::from("Slow bloom"), crate::PresetEntry::from("Butterfly")];
        let grid = crate::grid_view::GridView::default();
        let midi = MidiView::default();
        let state = PerformanceState {
            project: "Show 1",
            decks: &[],
            active_deck: 0,
            follow_columns: None,
            recording: None,
            preset_current: None,
            outputs: &[OutputStatus { name: "syphon:vizz".into(), live: true }],
            audio: &audio,
            fps: 60.0,
            over_budget: false,
            bpm: 128.0,
            bar_phase: 0.1,
            presets: &names,
            thumb_revision: 0,
            grid: &grid,
            gravity: None,
            midi: &midi,
            values,
            output_texture: None,
            output_aspect: 16.0 / 9.0,
            graph: None,
        };
        let mut runs = Vec::new();
        for i in 0..8 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(1280.0, 800.0),
                )),
                time: Some(i as f64 * 0.05),
                ..Default::default()
            });
            draw(&ctx, reg, &state, macros);
            let out = ctx.end_pass();
            runs = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => {
                        // The colour a RichText carries lives on the job
                        // section; an override wins when one is set.
                        let colour = t.override_text_color.unwrap_or_else(|| {
                            t.galley
                                .job
                                .sections
                                .first()
                                .map(|sec| sec.format.color)
                                .unwrap_or(Color32::PLACEHOLDER)
                        });
                        Some((painted(&t.galley), colour))
                    }
                    _ => None,
                })
                .collect();
        }
        runs
    }

    /// Count the small ARMED-coloured fills — the latch pips. Small,
    /// because the ARMED colour legitimately fills whole buttons
    /// elsewhere (the grid's armed store/clear); the pip is a 6-point
    /// corner square and nothing else that size wears that colour.
    fn armed_pips(reg: &ParamRegistry, latched: bool) -> usize {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        if latched {
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::new(("punch-latch", "/punch/flash")), true);
            });
        }
        let audio = AudioView::default();
        let names = [crate::PresetEntry::from("Slow bloom")];
        let grid = crate::grid_view::GridView::default();
        let midi = MidiView::default();
        let state = PerformanceState {
            project: "Show 1",
            decks: &[],
            active_deck: 0,
            follow_columns: None,
            recording: None,
            preset_current: None,
            outputs: &[],
            audio: &audio,
            fps: 60.0,
            over_budget: false,
            bpm: 128.0,
            bar_phase: 0.1,
            presets: &names,
            thumb_revision: 0,
            grid: &grid,
            gravity: None,
            midi: &midi,
            values: None,
            output_texture: None,
            output_aspect: 16.0 / 9.0,
            graph: None,
        };
        let mut macros = Macros::default();
        let mut count = 0;
        // The clock advances so the window's fade-in finishes — mid-fade
        // every colour is alpha-scaled and matches nothing.
        for i in 0..6 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(1280.0, 800.0),
                )),
                time: Some(i as f64 * 0.2),
                ..Default::default()
            });
            draw(&ctx, reg, &state, &mut macros);
            let out = ctx.end_pass();
            count = out
                .shapes
                .iter()
                .filter(|s| match &s.shape {
                    egui::Shape::Rect(r) => {
                        r.fill == crate::theme::ARMED && r.rect.width() <= 8.0
                    }
                    _ => false,
                })
                .count();
        }
        count
    }

    /// Draw the layout with a gravity grid in a given state, and return
    /// what was painted.
    fn render_with_gravity(reg: &ParamRegistry, gravity: &crate::grid_view::GridView) -> String {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let audio = AudioView::default();
        let names = [crate::PresetEntry::from("Slow bloom")];
        let grid = crate::grid_view::GridView::default();
        let midi = MidiView::default();
        let state = PerformanceState {
            project: "Show 1",
            decks: &[],
            active_deck: 0,
            follow_columns: None,
            recording: None,
            preset_current: None,
            outputs: &[],
            audio: &audio,
            fps: 60.0,
            over_budget: false,
            bpm: 128.0,
            bar_phase: 0.1,
            presets: &names,
            thumb_revision: 0,
            grid: &grid,
            gravity: Some(gravity),
            midi: &midi,
            values: None,
            output_texture: None,
            output_aspect: 16.0 / 9.0,
            graph: None,
        };
        let mut macros = Macros::default();
        let size = vec2(1280.0, 900.0);
        let mut text = String::new();
        for i in 0..8 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(i as f64 * 0.05),
                ..Default::default()
            });
            draw(&ctx, reg, &state, &mut macros);
            let out = ctx.end_pass();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            text = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) if on_screen(t, screen) => Some(painted(&t.galley)),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
        }
        text
    }

    /// An empty gravity grid must still offer the way into itself.
    ///
    /// The row used to be hidden entirely until a pad was filled — but
    /// the store button lives *in* the row, and no other screen draws a
    /// grid, so a fresh install had no way to fill it from anywhere.
    /// Hidden-when-empty is fine; hidden-when-empty with the only key
    /// inside is a locked door.
    #[test]
    fn an_empty_gravity_row_still_offers_a_way_to_fill_it() {
        let reg = registry();
        let empty = crate::grid_view::GridView::default();
        let text = render_with_gravity(&reg, &empty);
        assert!(text.contains("GRAVITY"), "the gravity section vanished: {text}");
        assert!(
            text.contains("capture"),
            "an empty gravity row offers no way to fill it: {text}"
        );

        // And once a pad holds something, the real row takes over: the
        // teaching line goes and the pad numbers arrive.
        let mut filled = crate::grid_view::GridView::default();
        filled.names[0] = Some("pull in".into());
        let text = render_with_gravity(&reg, &filled);
        assert!(text.contains("pull in"), "the filled pad is not drawn: {text}");
        assert!(
            text.contains("store"),
            "the full row's own store button is missing: {text}"
        );
    }

    /// A latched punch must not look identical to a held one: one keeps
    /// strobing when the finger leaves, and mistaking the two is a strobe
    /// you cannot stop. The corner pip is the tell — present exactly when
    /// the latch is on, absent on a plain lit hold.
    #[test]
    fn a_latched_punch_wears_a_pip_and_a_held_one_does_not() {
        let reg = registry();
        reg.set_by_addr("/punch/flash", 1.0); // lit either way
        assert_eq!(armed_pips(&reg, false), 0, "a plain hold grew a latch pip");
        assert!(armed_pips(&reg, true) >= 1, "the latched button has no pip");
    }

    /// Two faders whose parameters end in the same word are qualified
    /// with their group, and that label is long enough to overrun a
    /// narrow column. It used to ellipsise, and it ellipsised from the
    /// right, so "particles spread" reached the screen as "particles …"
    /// — the word saying what the fader did was the part thrown away.
    ///
    /// Tested through the helper rather than the rendered text because
    /// this is about what fits, and the sweep below is what checks that
    /// nothing on the real layout ends up cut.
    #[test]
    fn a_qualified_fader_label_shrinks_then_abbreviates_its_group() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(1280.0, 800.0),
            )),
            ..Default::default()
        });
        egui::Area::new(egui::Id::new("fit")).show(&ctx, |ui| {
            let long = "particles spread";
            let measure = |text: &str, size: f32| {
                ui.painter()
                    .layout_no_wrap(text.to_string(), egui::FontId::proportional(size), INK_2)
                    .rect
                    .width()
            };

            // Room for the whole thing: nothing is given up.
            let roomy = measure(long, 13.0) + 10.0;
            assert_eq!(fit_label(ui, long, roomy), (long.to_string(), 13.0));

            // Slightly tight: the size steps down, the words stay whole.
            let tight = measure(long, 13.0) - 12.0;
            let (text, size) = fit_label(ui, long, tight);
            assert_eq!(text, long, "gave up the full name too early");
            assert!(size < 13.0 && measure(&text, size) <= tight);

            // Too narrow for the full name at any legible size: the
            // group abbreviates, and the word that says what the fader
            // does survives intact.
            let narrow = measure(long, 9.0) - 6.0;
            let (text, size) = fit_label(ui, long, narrow);
            assert_eq!(text, "par. spread", "abbreviated the wrong end");
            assert!(
                measure(&text, size) <= narrow,
                "abbreviated to {text:?} at {size}pt and still overran {narrow}pt"
            );

            // Groups stay distinguishable once abbreviated — the whole
            // reason the label is qualified in the first place.
            let (other, _) = fit_label(ui, "color spread", narrow);
            assert_ne!(other, text, "two groups abbreviated to the same label");

            // A one-word label is never touched.
            assert_eq!(fit_label(ui, "glow", 62.0), ("glow".to_string(), 13.0));
        });
        let _ = ctx.end_pass();
    }

    /// Nothing on this screen may reach the eye ellipsised.
    ///
    /// Only assertable because the harness reads painted glyphs: a
    /// clipped label reports its full text through `Galley::text()`, so
    /// for as long as the tests here read that, every `contains` check
    /// in this module passed whether or not the words survived to the
    /// screen. Swept across the window sizes a rig actually presents —
    /// a laptop, a 720p projector, a 1080p one — because the layout
    /// shrinks rather than reflowing, and shrinking is what pushes a
    /// label past its column.
    #[test]
    fn no_label_is_ellipsised_at_any_realistic_window_size() {
        let reg = registry();
        for size in [
            vec2(1280.0, 800.0),
            vec2(1440.0, 900.0),
            vec2(1920.0, 1080.0),
            vec2(1024.0, 640.0),
            vec2(900.0, 600.0),
        ] {
            let mut macros = Macros::default();
            // The pair that has to wear its group word, in the slots a
            // default layout puts them in.
            macros.set(0, Some("/color/spread".to_string()));
            macros.set(1, Some("/particles/spread".to_string()));
            let text = render_at_size(&mut macros, &reg, size);
            assert!(
                !text.contains('…'),
                "a label was cut short at {size:?}: {text}"
            );
        }
    }

    /// The layer strip earns its space in both states, differently.
    ///
    /// It used to follow the gravity grid's rule — absent until a layer
    /// is on — and this test asserted exactly that. The rule was wrong
    /// here and right there: an empty gravity grid is a grid you know
    /// about and are not using, while an absent layer strip was the
    /// only sign the vector layers exist, on the one screen you play
    /// from. Nothing on it could start a layer because nothing was on
    /// it, and the only way in was the parameter list on the other
    /// screen.
    ///
    /// So: one teaching line when nothing is on, the full per-layer
    /// controls when something is. Checked both ways, because a strip
    /// that always drew the same thing would pass either half alone.
    #[test]
    fn the_layer_strip_teaches_when_idle_and_controls_when_live() {
        let reg = registry();
        let mut macros = Macros::default();
        let idle = render(&mut macros, &reg);
        assert!(
            idle.contains("LAYERS"),
            "the strip is invisible with every layer off: {idle}"
        );
        // The teaching state, not the control state: no blend mode, no
        // per-layer numbers, one line.
        assert!(
            !idle.contains("normal"),
            "the idle strip drew the full controls: {idle}"
        );

        reg.set_by_addr("/l1/kind", 1.0);
        let on = render(&mut macros, &reg);
        assert!(on.contains("LAYERS"), "no strip with a layer on: {on}");
        assert!(
            on.contains("rings"),
            "the strip does not name the layer's generator: {on}"
        );
        assert!(
            on.contains("normal"),
            "the strip does not name the blend mode: {on}"
        );
    }

    /// The fader labels must be on the window, not merely drawn.
    ///
    /// The layout reserves room for three label lines under each track
    /// and shrinks the tracks to fit — but the shrink has a floor, and
    /// below it the labels go over the bottom edge instead. They are
    /// still emitted as shapes when that happens, which is why this
    /// reads back only what lands inside the screen rect: the value, the
    /// name and the binding under a fader are the whole reason the
    /// screen exists, and a build that pushed them off would otherwise
    /// pass every assertion in this module.
    #[test]
    fn fader_labels_stay_on_the_window_at_realistic_sizes() {
        let reg = registry();
        for size in [
            vec2(1280.0, 800.0),
            vec2(1440.0, 900.0),
            vec2(1920.0, 1080.0),
            vec2(1366.0, 768.0),
            vec2(1280.0, 720.0),
        ] {
            let mut macros = Macros::default();
            macros.set(0, Some("/fx/glow".to_string()));
            let text = render_at_size(&mut macros, &reg, size);
            assert!(
                text.contains("MASTER"),
                "the master fader's label is off the window at {size:?}: {text}"
            );
            assert!(
                text.contains("glow"),
                "a fader's name is off the window at {size:?}: {text}"
            );
        }
    }

    /// The performance view must show what is assigned and the master
    /// fader, and must not fall over on a slot pointing at a parameter
    /// this build does not have — a patch from another version is the
    /// normal way that happens.
    #[test]
    fn draws_assigned_slots_and_survives_stale_ones() {
        let reg = registry();
        let mut macros = Macros {
            slots: vec![None; vizz_mod::perform::MACRO_COUNT],
        };
        macros.set(0, Some("/particles/size".into()));
        macros.set(1, Some("/fx/glow".into()));
        // Deliberately stale: this parameter does not exist here.
        macros.set(2, Some("/gone/missing".into()));

        let text = render(&mut macros, &reg);
        assert!(text.contains("size"), "assigned slot missing: {text}");
        assert!(text.contains("glow"), "assigned slot missing: {text}");
        assert!(text.contains("MASTER"), "master fader missing: {text}");
        assert!(
            text.contains("syphon:vizz"),
            "output status missing: {text}"
        );
        assert!(text.contains("128.0 bpm"), "tempo missing: {text}");
        // The punch row draws for whichever gestures the registry has —
        // and only those, so a build without a punch param shows no
        // phantom button.
        assert!(text.contains("PUNCH"), "punch section missing: {text}");
        assert!(text.contains("FLASH"), "flash button missing: {text}");
        assert!(text.contains("STROBE"), "strobe button missing: {text}");
        assert!(!text.contains("BLACK"), "a button drew without its parameter: {text}");
        // The stale slot renders as an empty placeholder rather than
        // vanishing, so the fader layout does not reflow mid-set.
        assert!(
            text.contains("assign"),
            "stale slot did not fall back: {text}"
        );
    }

    #[test]
    fn empty_macros_still_draw_the_master() {
        let reg = registry();
        let mut macros = Macros {
            slots: vec![None; vizz_mod::perform::MACRO_COUNT],
        };
        let text = render(&mut macros, &reg);
        assert!(text.contains("MASTER"), "got: {text}");
    }

    /// Presets must be on the performance surface and numbered to match
    /// the keyboard. Without them, changing look means leaving the layout
    /// — the one thing the layout exists to avoid.
    ///
    /// The number is a badge in the corner of the tile rather than a word
    /// in front of the name, so the claim is spatial: this asserts the
    /// "1" is painted *on the first preset's own tile*, which a substring
    /// search cannot tell from the sixteen scene pads also numbered 1.
    #[test]
    fn the_performance_layout_offers_presets_by_number() {
        let reg = registry();
        let mut macros = Macros::default();
        let sheet = sheet_sized(
            &mut macros,
            &reg,
            &MidiView::default(),
            None,
            None,
            vec2(1440.0, 900.0),
        );
        let text = sheet.text();
        assert!(text.contains("Slow bloom"), "preset missing: {text}");
        assert!(text.contains("Butterfly"), "preset missing: {text}");
        let name = sheet
            .items
            .iter()
            .find(|p| p.text.trim() == "Slow bloom")
            .unwrap_or_else(|| panic!("the first preset's name was not painted: {text}"));
        // The caption sits along the bottom of the tile, so the tile is
        // the box of TILE_W x TILE_H that ends just under the name.
        let tile = egui::Rect::from_min_max(
            egui::pos2(name.rect.left() - 8.0, name.rect.bottom() - TILE_H),
            egui::pos2(name.rect.left() - 8.0 + TILE_W, name.rect.bottom() + 6.0),
        );
        assert!(
            sheet
                .items
                .iter()
                .any(|p| p.text.trim() == "1" && tile.contains_rect(p.rect)),
            "slot 1 is not numbered on its own tile: {text}"
        );
    }

    /// Every fader must be bindable from this screen. Learn used to live
    /// only on the panel's parameter rows, which meant mapping a
    /// controller to the faders you play required leaving the layout the
    /// faders are on.
    #[test]
    fn faders_offer_midi_learn_and_show_their_bindings() {
        let reg = registry();
        let mut macros = Macros {
            slots: vec![None; vizz_mod::perform::MACRO_COUNT],
        };
        macros.set(0, Some("/particles/size".into()));
        macros.set(1, Some("/fx/glow".into()));
        // A third, assigned but unbound — the only state that offers a
        // learn. A bound fader shows its binding and a learning one shows
        // "waiting", so without this the assertion below tests nothing.
        macros.set(2, Some("/master/dim".into()));

        let mut midi = MidiView {
            available: true,
            ..Default::default()
        };
        midi.map.bind(
            vizz_midi::Source::ControlChange {
                channel: 0,
                controller: 21,
            },
            "/particles/size",
        );
        midi.learn_target = Some(vizz_midi::LearnTarget::param("/fx/glow"));

        let text = render_with(&mut macros, &reg, &midi, None);
        assert!(
            text.contains("ch1 cc21"),
            "binding not shown on the fader: {text}"
        );
        assert!(text.contains("waiting"), "learn state not shown: {text}");
        assert!(text.contains("learn"), "no way to start a learn: {text}");
    }

    /// The chip used to be drawn only while a take was running, which
    /// left the performance screen with no way to *start* one — arming
    /// meant leaving for the panel and expanding a collapsed section.
    /// Both states are checked, because a chip that only appears once
    /// recording has begun passes any test that starts it recording.
    #[test]
    fn recording_can_be_started_and_stopped_from_the_performance_screen() {
        let reg = registry();
        let mut macros = Macros::default();
        let idle = render(&mut macros, &reg);
        assert!(
            idle.contains("REC"),
            "no way to arm a recording from the performance screen: {idle}"
        );

        let rolling = render_with_recording(
            &mut macros,
            &reg,
            crate::RecordingView { secs: 62, frames: 3720, dropped: 4 },
        );
        assert!(rolling.contains("REC 1:02"), "no elapsed time: {rolling}");
        assert!(rolling.contains("3720f"), "no frame count: {rolling}");
        assert!(rolling.contains("4 dropped"), "drops not surfaced: {rolling}");
    }

    /// The number under a fader must stay put while modulation moves the
    /// parameter.
    ///
    /// The obvious design — show the live value — is wrong, and this test
    /// exists to stop it being reintroduced. An LFO at any musical rate
    /// turns a live readout into an unreadable blur, and the digit that
    /// blurs is the one that tells you where your own hand left the
    /// control. So the number is what you set, the ghost mark on the track
    /// is where modulation has taken it, and the number's *colour* says
    /// which of the two you are looking at.
    #[test]
    fn the_readout_stays_stable_while_modulation_moves_the_parameter() {
        let reg = registry();
        let mut macros = Macros {
            slots: vec![None; vizz_mod::perform::MACRO_COUNT],
        };
        macros.set(0, Some("/fx/glow".into()));
        let glow = reg.id("/fx/glow").unwrap();
        reg.set(glow, 0.25);

        // Push it well away from where the fader is set.
        let mut snap = vizz_params::ParamSnapshot::new(&reg);
        let mut offsets = vec![0.0; reg.len()];
        offsets[glow.index()] = 0.5;
        snap.advance_modulated(&reg, 1.0, &offsets);
        let values: Vec<f32> = reg.iter().map(|(id, _)| snap.get(id)).collect();

        let text = render_with(&mut macros, &reg, &MidiView::default(), Some(&values));
        assert!(
            text.contains("0.25"),
            "the readout must show what the user set, not the modulated value: {text}"
        );
        assert!(
            !text.contains("0.75"),
            "the readout followed modulation, which makes it a blur under any LFO: {text}"
        );

        // And the second half of the claim: the colour is what says the
        // number is no longer where the parameter is. Asserted because
        // the doc comment above promises it and nothing else here could
        // tell — the string view has no colour in it at all, so a
        // readout that went back to plain ink would pass every other
        // assertion in this module.
        let runs = render_coloured(&mut macros, &reg, Some(&values));
        let readout = runs
            .iter()
            .find(|(t, _)| t.trim() == "0.25")
            .unwrap_or_else(|| panic!("no 0.25 readout among {runs:?}"));
        assert_eq!(
            readout.1, MOD,
            "a modulated readout must be warm, or nothing says the number              is not where the parameter is"
        );

        // Unmodulated, the same readout is *not* warm — otherwise "warm"
        // means nothing, because everything is warm.
        //
        // Asserted as "not MOD" rather than as one exact ink. The claim
        // here is about the amber, and pinning the readout's exact
        // colour also pinned its rank in the label stack — which is a
        // separate decision, and one that has since changed: the name
        // leads now and the number is the quieter confirmation under it.
        let plain = render_coloured(&mut macros, &reg, None);
        let plain_readout = plain
            .iter()
            .find(|(t, _)| t.trim() == "0.25")
            .unwrap_or_else(|| panic!("no 0.25 readout among {plain:?}"));
        assert_ne!(
            plain_readout.1, MOD,
            "an unmodulated readout wears the colour that is supposed to mean modulated"
        );
    }
}


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
use vizz_mod::perform::{MACRO_COUNT, Macros};
use vizz_params::ParamRegistry;

use crate::panel::{AudioView, MidiView, OutputStatus};

/// The text ramp. Four stops, used consistently: anything that matters is
/// at `INK` or `INK_2`, and `INK_4` means "this is off".
const INK: Color32 = Color32::from_rgb(236, 240, 246);
const INK_2: Color32 = Color32::from_rgb(178, 187, 200);
const INK_3: Color32 = Color32::from_rgb(132, 141, 156);
const INK_4: Color32 = Color32::from_rgb(94, 101, 114);

const PANEL_BG: Color32 = Color32::from_rgb(23, 25, 30);
const TRACK: Color32 = Color32::from_rgb(38, 41, 48);
const FILL: Color32 = Color32::from_rgb(74, 128, 178);
const FILL_TOP: Color32 = Color32::from_rgb(96, 158, 214);
const HANDLE: Color32 = Color32::from_rgb(226, 233, 242);
/// Modulation, warm against the blues so "something else is moving this"
/// reads instantly. Matches the panel's own modulation colour.
const MOD: Color32 = Color32::from_rgb(255, 190, 90);
const MASTER_FILL: Color32 = Color32::from_rgb(178, 78, 78);
const LIVE: Color32 = Color32::from_rgb(104, 208, 132);
const DEAD: Color32 = Color32::from_rgb(96, 102, 112);
const WARN: Color32 = Color32::from_rgb(242, 156, 92);
const LEARN: Color32 = Color32::from_rgb(232, 132, 108);

/// Faders per row. Sixteen slots as two rows of eight keeps each one wide
/// enough to hit and mirrors the grid's sixteen above it.
const PER_ROW: usize = 8;
const FADER_MIN_W: f32 = 62.0;
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
    pub outputs: &'a [OutputStatus],
    pub audio: &'a AudioView,
    pub fps: f32,
    pub over_budget: bool,
    pub bpm: f32,
    pub bar_phase: f32,
    /// Preset names in slot order, so the row can be numbered to match
    /// `/preset/recall` and the number keys.
    pub presets: &'a [String],
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
}

#[derive(Debug, Default)]
pub struct PerformanceActions {
    /// The user tapped tempo.
    pub tapped: bool,
    /// Macro assignments changed and should be persisted.
    pub macros_changed: bool,
    /// Leave the performance layout.
    pub exit: bool,
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
    pub clear_slot_binding: Option<f32>,
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
            ui.painter().rect_filled(
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), full),
                0.0,
                egui::Color32::from_rgba_unmultiplied(PANEL_BG.r(), PANEL_BG.g(), PANEL_BG.b(), 242),
            );
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);

            let inner_w = full.x - PAD * 2.0;
            ui.add_space(PAD * 0.5);
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                ui.vertical(|ui| {
                    ui.set_width(inner_w);
                    status_strip(ui, state, &mut actions, inner_w);
                    ui.add_space(10.0);

                    section(ui, "SCENES");
                    actions.grid =
                        crate::grid_view::draw(ui, state.grid, crate::grid_view::Shape::Stage);
                    ui.add_space(10.0);

                    if let Some(gravity) = state.gravity {
                        section(ui, "GRAVITY");
                        actions.gravity = crate::grid_view::draw_with_id(
                            ui,
                            gravity,
                            crate::grid_view::Shape::Stage,
                            "gravity-grid",
                        );
                        ui.add_space(10.0);
                    }

                    if !state.presets.is_empty() {
                        section(ui, "PRESETS");
                        preset_row(ui, state, &mut actions);
                        ui.add_space(10.0);
                    }

                    section(ui, "CONTROLS");
                    // Whatever vertical space is left goes to the faders,
                    // which are the thing you actually play. Measured from
                    // the cursor rather than guessed, so adding a row above
                    // shortens the faders instead of pushing them off.
                    let used = ui.cursor().top();
                    // The floor is the real column height — track plus its
                    // three label lines — not a guess. The old 46-point
                    // allowance was 11 short of the labels it was
                    // reserving for, which is exactly the bottom row of
                    // text it pushed off the window.
                    let left = (full.y - used - PAD).max(FADER_ABS_MIN + FADER_CHROME + 6.0);
                    faders(ui, registry, macros, state, &mut actions, inner_w, left);
                });
            });
        });

    actions
}

/// A section rule: a small caps label with a hairline running to the right
/// edge. Cheap, and it turns a flat stack of rows into three named regions
/// you can find without reading them.
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
            egui::Stroke::new(1.0, Color32::from_rgb(44, 48, 56)),
        );
    });
    ui.add_space(4.0);
}

/// Presets as a row of buttons, numbered to match the keyboard.
fn preset_row(ui: &mut egui::Ui, state: &PerformanceState<'_>, actions: &mut PerformanceActions) {
    ui.horizontal_wrapped(|ui| {
        for (i, name) in state.presets.iter().enumerate() {
            let slot = i as u32 + 1;
            // Only the first ten have a key; showing a number beside the
            // eleventh would promise a shortcut that does not exist.
            let label = if slot <= 10 {
                egui::RichText::new(format!("{}  {name}", slot % 10))
                    .size(14.0)
                    .color(INK)
            } else {
                egui::RichText::new(name).size(14.0).color(INK)
            };
            // A preset addresses a slot exactly as a scene pad does, so it
            // maps the same way: the binding names the slot, and the
            // button only says when. A plain binding on `/preset/recall`
            // would spread a button across a 64-slot range and recall the
            // last one every time.
            let bound = state.midi.map.source_for_value(RECALL, slot as f32);
            let waiting = state.midi.learning_value(RECALL, slot as f32);
            // The recalled slot is the answer to "where did the look on
            // screen come from" — the one button that should not look
            // like the other nine. Blue, matching the grid's CURRENT.
            let current = state.preset_current == Some(slot as usize);
            let button = egui::Button::new(label)
                .min_size(vec2(0.0, 30.0))
                .fill(if waiting {
                    LEARN
                } else {
                    Color32::from_rgb(36, 40, 48)
                })
                // An edge, so the row reads as buttons rather than as a
                // line of caption text — which is what it was mistaken
                // for when the fills sat 13 points off the background.
                .stroke(if current {
                    egui::Stroke::new(1.5, Color32::from_rgb(110, 180, 255))
                } else {
                    egui::Stroke::new(1.0, Color32::from_rgb(62, 68, 82))
                });
            let response = ui.add(button);
            if response.clicked() {
                actions.preset_slot = Some(slot);
            }
            let response = match (&bound, waiting) {
                (_, true) => response.on_hover_text("press a button on your controller"),
                (Some(s), _) => response.on_hover_text(format!("recall {name}  ·  {}", s.label())),
                (None, _) => response.on_hover_text(format!("recall {name}")),
            };
            if state.midi.available {
                response.context_menu(|ui| match (&bound, waiting) {
                    (Some(s), _) => {
                        if ui.button(format!("unmap {}", s.label())).clicked() {
                            actions.clear_slot_binding = Some(slot as f32);
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
                });
            }
        }
    });
}

/// The preset recall address. Bindings name a parameter by address rather
/// than by id, since they outlive the process.
pub(crate) const RECALL: &str = "/preset/recall";

fn status_strip(
    ui: &mut egui::Ui,
    state: &PerformanceState<'_>,
    actions: &mut PerformanceActions,
    width: f32,
) {
    ui.horizontal(|ui| {
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
            );
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
                (1.0, Color32::from_rgb(64, 70, 84)),
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
    let rows = if height >= two_rows {
        MACRO_COUNT.div_ceil(PER_ROW)
    } else {
        1
    };
    let per_row = MACRO_COUNT.div_ceil(rows);
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
            if slot >= MACRO_COUNT {
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
            let master_rect = egui::Rect::from_min_size(
                egui::pos2(origin.x + width - w, origin.y),
                vec2(w, h + chrome),
            );
            let mut col = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(master_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            col.spacing_mut().item_spacing = vec2(6.0, LABEL_GAP);
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

            if let Some(v) = vertical_fader(ui, value, modulated, def.min, def.max, w, h) {
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
            ui.label(
                egui::RichText::new(shown)
                    .size(13.0)
                    .monospace()
                    .color(if modulated.is_some() { MOD } else { INK }),
            );
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
            if ui
                .add(
                    egui::Label::new(egui::RichText::new(shown_name).size(13.0).color(INK_2))
                        .sense(Sense::click())
                        .truncate(),
                )
                .on_hover_text(format!("{addr}  —  click to reassign"))
                .clicked()
            {
                open_assign(ui, slot);
            }
            midi_chip(ui, state, actions, addr);
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
                egui::Stroke::new(1.0, Color32::from_rgb(46, 50, 58)),
                egui::StrokeKind::Inside,
            );
            ui.label(egui::RichText::new("—").size(13.0).color(INK_4));
            if ui
                .add(
                    egui::Label::new(egui::RichText::new("assign").size(13.0).color(INK_3))
                        .sense(Sense::click()),
                )
                .clicked()
            {
                open_assign(ui, slot);
            }
            ui.label(egui::RichText::new(" ").size(11.0));
        }
    }

    assign_popup(ui, registry, macros, slot, actions);
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
    egui::Area::new(popup_id.with("area"))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_height(320.0);
                ui.set_min_width(200.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("fader {}", slot + 1)).color(INK));
                    if ui.small_button("close").clicked() {
                        close = true;
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt(popup_id)
                    .show(ui, |ui| {
                        if ui.selectable_label(false, "— none —").clicked() {
                            chosen = Some(None);
                        }
                        for (_, def) in registry.iter() {
                            if ui.selectable_label(false, &def.addr).clicked() {
                                chosen = Some(Some(def.addr.clone()));
                            }
                        }
                    });
            });
        });
    if let Some(pick) = chosen {
        macros.set(slot, pick);
        actions.macros_changed = true;
        close = true;
    }
    if close {
        close_assign(ui, slot);
    }
}

// egui 0.35 made the popup helpers private, so the open slot is tracked in
// the public temp-data store instead. Keyed per slot so two faders cannot
// both think they own the picker.
fn assign_key(slot: usize) -> egui::Id {
    egui::Id::new(("assign-open", slot))
}
fn open_assign(ui: &egui::Ui, slot: usize) {
    ui.memory_mut(|m| m.data.insert_temp(assign_key(slot), true));
}
fn close_assign(ui: &egui::Ui, slot: usize) {
    ui.memory_mut(|m| m.data.insert_temp(assign_key(slot), false));
}
fn is_assign_open(ui: &egui::Ui, slot: usize) -> bool {
    ui.memory(|m| m.data.get_temp::<bool>(assign_key(slot)).unwrap_or(false))
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
fn vertical_fader(
    ui: &mut egui::Ui,
    value: f32,
    modulated: Option<f32>,
    min: f32,
    max: f32,
    w: f32,
    h: f32,
) -> Option<f32> {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
    let span = (max - min).max(f32::EPSILON);
    let t = ((value - min) / span).clamp(0.0, 1.0);

    let p = ui.painter();
    let track = rect.shrink2(vec2(rect.width() * 0.12, 0.0));
    p.rect_filled(track, 5.0, TRACK);

    // Fill from the bottom: a fader reads as "how much", and a bar growing
    // upward says that without needing the number.
    let fill_h = track.height() * t;
    let fill_rect = egui::Rect::from_min_size(
        egui::pos2(track.left(), track.bottom() - fill_h),
        vec2(track.width(), fill_h),
    );
    p.rect_filled(fill_rect, 5.0, FILL);
    // A brighter cap on the fill. The eye finds an edge far faster than it
    // judges the height of a flat block, which is the whole task here.
    if fill_h > 3.0 {
        p.rect_filled(
            egui::Rect::from_min_size(fill_rect.left_top(), vec2(track.width(), 2.5)),
            2.0,
            FILL_TOP,
        );
    }

    // Quarter ticks, so a position can be read as a fraction rather than
    // estimated. Drawn over the fill, subtle enough not to compete.
    for q in 1..4 {
        let y = track.bottom() - track.height() * (q as f32 / 4.0);
        p.line_segment(
            [
                egui::pos2(track.left() + 2.0, y),
                egui::pos2(track.left() + 7.0, y),
            ],
            egui::Stroke::new(1.0, Color32::from_rgb(70, 76, 88)),
        );
    }

    // Where modulation has actually taken it. A separate mark rather than
    // moving the handle: the handle is the promise the user made, and this
    // is what the renderer is doing with it.
    if let Some(m) = modulated {
        let mt = ((m - min) / span).clamp(0.0, 1.0);
        let my = track.bottom() - track.height() * mt;
        p.line_segment(
            [egui::pos2(track.left(), my), egui::pos2(track.right(), my)],
            egui::Stroke::new(2.0, MOD),
        );
    }

    // Chunky handle, full width so it reads as grabbable and stays visible
    // against the fill.
    let hy = track.bottom() - fill_h;
    let handle = egui::Rect::from_center_size(
        egui::pos2(
            rect.center().x,
            hy.clamp(rect.top() + 6.0, rect.bottom() - 6.0),
        ),
        vec2(rect.width(), 11.0),
    );
    p.rect_filled(handle, 3.0, HANDLE);
    // Hover feedback: without it there is no way to tell a live fader from
    // a picture of one until you have already moved it.
    if response.hovered() {
        p.rect_stroke(
            rect,
            5.0,
            egui::Stroke::new(1.5, Color32::from_rgb(120, 150, 185)),
            egui::StrokeKind::Inside,
        );
    }

    // Absolute positioning rather than relative dragging: grabbing anywhere
    // in the column jumps to that value, which is what you want when
    // reaching quickly.
    let pos = if response.dragged() || response.clicked() {
        response.interact_pointer_pos()
    } else {
        None
    }?;
    let nt = (1.0 - (pos.y - track.top()) / track.height()).clamp(0.0, 1.0);
    Some(min + nt * span)
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

    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
    let span = (def.max - def.min).max(f32::EPSILON);
    let t = ((value - def.min) / span).clamp(0.0, 1.0);
    let p = ui.painter();
    let track = rect.shrink2(vec2(rect.width() * 0.12, 0.0));
    p.rect_filled(track, 5.0, TRACK);
    let fill_h = track.height() * t;
    p.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(track.left(), track.bottom() - fill_h),
            vec2(track.width(), fill_h),
        ),
        5.0,
        MASTER_FILL,
    );
    let hy = track.bottom() - fill_h;
    p.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(
                rect.center().x,
                hy.clamp(rect.top() + 6.0, rect.bottom() - 6.0),
            ),
            vec2(rect.width(), 11.0),
        ),
        3.0,
        HANDLE,
    );
    // Dimmed-out is a state worth shouting about: a black output with
    // everything else apparently fine is the classic mid-set panic.
    if t < 0.02 {
        p.rect_stroke(
            rect,
            5.0,
            egui::Stroke::new(2.0, WARN),
            egui::StrokeKind::Inside,
        );
    }
    if (response.dragged() || response.clicked())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let nt = (1.0 - (pos.y - track.top()) / track.height()).clamp(0.0, 1.0);
        registry.set(id, def.min + nt * span);
    }
    let _ = state;

    ui.label(
        egui::RichText::new(format!("{value:.2}"))
            .size(13.0)
            .monospace()
            .color(if t < 0.02 { WARN } else { INK }),
    );
    ui.label(
        egui::RichText::new("MASTER")
            .size(12.0)
            .strong()
            .color(Color32::from_rgb(226, 150, 150)),
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
        b.build()
    }

    fn render(macros: &mut Macros, reg: &ParamRegistry) -> String {
        render_with(macros, reg, &MidiView::default(), None)
    }

    fn render_with(
        macros: &mut Macros,
        reg: &ParamRegistry,
        midi: &MidiView,
        values: Option<&[f32]>,
    ) -> String {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let audio = AudioView::default();
        let names = ["Slow bloom".to_string(), "Butterfly".to_string()];
        let grid = crate::grid_view::GridView::default();
        let state = PerformanceState {
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
            grid: &grid,
            gravity: None,
            midi,
            values,
        };
        let mut text = String::new();
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
            text = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
        }
        text
    }

    /// The performance view must show what is assigned and the master
    /// fader, and must not fall over on a slot pointing at a parameter
    /// this build does not have — a patch from another version is the
    /// normal way that happens.
    #[test]
    fn draws_assigned_slots_and_survives_stale_ones() {
        let reg = registry();
        let mut macros = Macros {
            slots: vec![None; MACRO_COUNT],
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
            slots: vec![None; MACRO_COUNT],
        };
        let text = render(&mut macros, &reg);
        assert!(text.contains("MASTER"), "got: {text}");
    }

    /// Presets must be on the performance surface and numbered to match
    /// the keyboard. Without them, changing look means leaving the layout
    /// — the one thing the layout exists to avoid.
    #[test]
    fn the_performance_layout_offers_presets_by_number() {
        let reg = registry();
        let mut macros = Macros::default();
        let text = render(&mut macros, &reg);
        assert!(text.contains("Slow bloom"), "preset missing: {text}");
        assert!(text.contains("Butterfly"), "preset missing: {text}");
        // Numbered, because the number keys fire the same slots and this
        // row is the only place that says so.
        assert!(
            text.contains("1  Slow bloom"),
            "slot numbers missing: {text}"
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
            slots: vec![None; MACRO_COUNT],
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
            slots: vec![None; MACRO_COUNT],
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
    }
}

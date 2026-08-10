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
const LIVE: Color32 = crate::theme::LIVE;
const DEAD: Color32 = Color32::from_rgb(96, 102, 112);
const WARN: Color32 = crate::theme::WARN;
const LEARN: Color32 = crate::theme::LEARN;

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
    /// A recording in progress: the strip wears a red chip, because
    /// forgetting a recording is how disks fill mid-set.
    pub recording: Option<crate::RecordingView>,
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
    pub clear_slot_binding: Option<(String, f32)>,
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
                    status_strip(ui, registry, state, &mut actions, inner_w);
                    ui.add_space(10.0);

                    section(ui, "PUNCH");
                    punch_row(ui, registry, state, &mut actions);
                    ui.add_space(10.0);

                    if layer_strip(ui, registry) {
                        ui.add_space(10.0);
                    }

                    section(ui, "SCENES");
                    actions.grid = crate::grid_view::draw(ui, state.grid);
                    ui.add_space(10.0);

                    if let Some(gravity) = state.gravity {
                        section(ui, "GRAVITY");
                        actions.gravity =
                            crate::grid_view::draw_with_id(ui, gravity, "gravity-grid");
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
                    egui::Stroke::new(1.5, crate::theme::CURRENT)
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
                });
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
fn layer_strip(ui: &mut egui::Ui, registry: &ParamRegistry) -> bool {
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
    if !any_on {
        return false;
    }

    section(ui, "LAYERS");
    ui.horizontal(|ui| {
        for (i, (kind, blend, opacity, freq, color)) in layer_ids.iter().enumerate() {
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
            } else if resp.secondary_clicked() {
                registry.set(*kind, (cur - 1.0).rem_euclid(steps));
            }
            resp.on_hover_text(format!(
                "layer {} generator — click to cycle, right-click back",
                i + 1
            ));

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
                } else if bresp.secondary_clicked() {
                    registry.set(*blend, (bcur - 1.0).rem_euclid(bsteps));
                }
                bresp.on_hover_text("blend mode — click to cycle, right-click back");

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
    true
}

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
        (LEARN, Color32::from_rgb(46, 32, 12))
    } else if lit {
        // Engaged reads loud: this is the row whose whole job is to be
        // unmissable while it is doing something to the output.
        (Color32::from_rgb(232, 236, 242), Color32::from_rgb(24, 26, 32))
    } else {
        (Color32::from_rgb(36, 40, 48), INK)
    };
    let response = ui
        .add(
            egui::Button::new(egui::RichText::new(label).size(13.0).strong().color(ink))
                .min_size(vec2(86.0, 34.0))
                .fill(fill)
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(62, 68, 82))),
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
                        Color32::from_rgb(150, 40, 36),
                        Color32::WHITE,
                        "recording the master — click to stop",
                    ),
                    // Idle sits dark with the word in a dimmed red, so it
                    // reads as armed-and-waiting rather than as another
                    // status light, and cannot be mistaken at a glance
                    // for a take in progress.
                    None => (
                        "REC".to_string(),
                        Color32::from_rgb(38, 26, 28),
                        Color32::from_rgb(196, 106, 100),
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
                    ("MIDI", Color32::from_rgb(90, 200, 120))
                } else {
                    ("MIDI?", Color32::from_rgb(240, 150, 90))
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

            if let Some(v) =
                vertical_fader(ui, value, modulated, def.min, def.max, def.default, w, h)
            {
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
            if ui
                .add(
                    egui::Label::new(egui::RichText::new(shown_name).size(size).color(INK_2))
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
    default: f32,
    w: f32,
    h: f32,
) -> Option<f32> {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
    // Right-click restores the default, honouring the overlay's promise
    // that this works on any slider — these faders were the exception.
    if response.secondary_clicked() {
        return Some(default);
    }
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
    // The same right-click reset as every other slider — a dimmed master
    // is exactly the mess the gesture exists to get out of.
    if response.secondary_clicked() {
        registry.set(id, def.default);
    } else if (response.dragged() || response.clicked())
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

    fn render_sized(
        macros: &mut Macros,
        reg: &ParamRegistry,
        midi: &MidiView,
        values: Option<&[f32]>,
        recording: Option<crate::RecordingView>,
        size: Vec2,
    ) -> String {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let audio = AudioView::default();
        let names = ["Slow bloom".to_string(), "Butterfly".to_string()];
        let grid = crate::grid_view::GridView::default();
        let state = PerformanceState {
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
                    size,
                )),
                time: Some(i as f64 * 0.05),
                ..Default::default()
            });
            draw(&ctx, reg, &state, macros);
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
        let names = ["Slow bloom".to_string(), "Butterfly".to_string()];
        let grid = crate::grid_view::GridView::default();
        let midi = MidiView::default();
        let state = PerformanceState {
            recording: None,
            preset_current: None,
            outputs: &[OutputStatus { name: "syphon:vizz".into(), live: true }],
            audio: &audio,
            fps: 60.0,
            over_budget: false,
            bpm: 128.0,
            bar_phase: 0.1,
            presets: &names,
            grid: &grid,
            gravity: None,
            midi: &midi,
            values,
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
        let names = ["Slow bloom".to_string()];
        let grid = crate::grid_view::GridView::default();
        let midi = MidiView::default();
        let state = PerformanceState {
            recording: None,
            preset_current: None,
            outputs: &[],
            audio: &audio,
            fps: 60.0,
            over_budget: false,
            bpm: 128.0,
            bar_phase: 0.1,
            presets: &names,
            grid: &grid,
            gravity: None,
            midi: &midi,
            values: None,
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

    /// The layer strip follows the gravity grid's rule: absent until a
    /// layer is on, so the default layout spends nothing on it — and
    /// present the moment one is, or the print side of the app has no
    /// home on the screen you play from. Checked both ways, because a
    /// strip that always draws would pass any single-state test.
    #[test]
    fn the_layer_strip_appears_only_when_a_layer_is_on() {
        let reg = registry();
        let mut macros = Macros::default();
        let idle = render(&mut macros, &reg);
        assert!(
            !idle.contains("LAYERS"),
            "the strip drew with every layer off: {idle}"
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

        // Unmodulated, the same readout is plain ink — otherwise "warm"
        // means nothing, because everything is warm.
        let plain = render_coloured(&mut macros, &reg, None);
        let plain_readout = plain
            .iter()
            .find(|(t, _)| t.trim() == "0.25")
            .unwrap_or_else(|| panic!("no 0.25 readout among {plain:?}"));
        assert_eq!(
            plain_readout.1, INK,
            "an unmodulated readout must be plain ink"
        );
    }
}


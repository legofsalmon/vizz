//! The scene grid: sixteen pads, and the blend between them.
//!
//! Laid out as a sequencer row rather than as a list, because that is what
//! it is played like — sixteen positions in a fixed place, so pad 11 is
//! always in the same spot whether or not pads 1 to 10 are filled. A list
//! that reflows when you fill a slot is a list you have to read; a row you
//! can hit without looking.
//!
//! The pad shows how far a transition to it has got, filling left to
//! right. That is the one piece of feedback the grid genuinely needs: with
//! a four-bar blend running, "which scene am I on" and "how much of the
//! way there am I" are the same question, and the answer has to be legible
//! from across a room.

use egui::{Color32, Sense, vec2};

/// Slots drawn. Mirrors `vizz_mod::scene::SLOTS`; asserted equal in tests
/// so the row cannot silently stop showing the last pads.
pub const SLOTS: usize = 16;

/// Pad size for the width actually on offer.
///
/// One row of sixteen is the shape this wants — it is the shape of a
/// sequencer, and it is what makes pad 11 findable without counting.
/// Sixteen across only works if sixteen fit: at a fixed width the row
/// runs past the right edge of a 900-point window, and everything laid
/// out from that edge — the frame rate, the tempo, the tap button —
/// goes with it. So the row divides up what it is given instead, with a
/// floor below which the pads stop being hittable and the row is better
/// off overflowing visibly than shrinking to nothing.
fn pad_size(available: f32) -> egui::Vec2 {
    let cols = SLOTS as f32;
    let each = (available - (cols - 1.0) * GAP) / cols;
    vec2(each.clamp(34.0, 96.0), 46.0)
}

const GAP: f32 = 4.0;

const FILLED: Color32 = Color32::from_rgb(58, 66, 84);
const EMPTY: Color32 = Color32::from_rgb(34, 36, 42);
const CURRENT: Color32 = crate::theme::CURRENT;
const ARRIVING: Color32 = Color32::from_rgb(255, 175, 80);
const ARMED: Color32 = crate::theme::ARMED;
/// A pad whose preset no longer exists. This used to borrow `ARMED`, so a
/// broken pad read as "the next press is destructive" — wrong twice over,
/// since firing it does nothing at all. Broken is a warning, not an arm.
const WARN: Color32 = crate::theme::WARN;
/// The autopilot's own colour. Green rather than the blue of `CURRENT` or
/// the amber of `ARRIVING`: those two say where the grid *is*, and this
/// says something is driving it. Sharing a colour with either would make
/// the sweep read as another transition.
const AUTO_ON: Color32 = Color32::from_rgb(72, 160, 104);
const AUTO_BED: Color32 = Color32::from_rgb(30, 46, 36);
/// A pad waiting for a MIDI control, and the chip on one that has found
/// it. Amber, matching the learn colour the panel and the performance
/// faders already use, so the state means the same thing everywhere.
const LEARN: Color32 = crate::theme::LEARN;
/// A pad's binding at rest — quiet enough that a fully mapped grid does
/// not read as sixteen alarms, bright enough to survive the transition
/// fill passing underneath it. The pad being blended to is exactly the pad
/// you are most likely to be looking at, so the one place the chip sits on
/// amber rather than on the pad colour is not a corner case.
const MIDI_INK: Color32 = Color32::from_rgb(158, 180, 206);
/// Secondary text. The egui default is dimmer than anything on a stage
/// should be, so labels here are set explicitly rather than inherited.
const LABEL: Color32 = Color32::from_rgb(178, 187, 200);

/// What the grid row needs to draw itself. Names rather than the `Grid`
/// itself so this crate does not depend on the scene module's internals.
pub struct GridView {
    /// Cell names in slot order; `None` for an empty pad.
    pub names: Vec<Option<String>>,
    /// Per slot: the pad is filled but its preset no longer exists.
    /// A scene names a look rather than owning one, so a deleted or
    /// renamed preset leaves a pad that would silently do nothing — the
    /// kind of fault that gets blamed on the controller mid-set.
    pub missing: Vec<bool>,
    /// Presets available to put on a pad, for the assign menu.
    pub presets: Vec<String>,
    /// The cell arrived at.
    pub current: Option<usize>,
    /// The cell being moved to, and how far along, 0..1.
    pub in_flight: Option<(usize, f32)>,
    pub duration: f32,
    /// Index into the curve list.
    pub curve: usize,
    pub curve_names: Vec<String>,
    pub autopilot: bool,
    pub bars: f32,
    /// How far through the current autopilot step the clock is, 0..1.
    /// `None` when the autopilot is off.
    pub auto_phase: Option<f32>,
    /// The slot the autopilot will move to next, so the control can say
    /// what is coming rather than only that something is.
    pub upcoming: Option<usize>,
    /// Per slot: the label of the MIDI control that fires it, if any.
    ///
    /// On the pad rather than beside the fire parameter because sixteen
    /// pads share one parameter — the useful question is "which of these
    /// is mapped", and a single row in the panel cannot answer it.
    pub midi: Vec<Option<String>>,
    /// The slot whose MIDI binding is being learned, waiting for a
    /// control to arrive.
    pub learning: Option<usize>,
    /// Whether MIDI is available at all. With no controller the arm would
    /// be a button that starts a wait nothing can ever end.
    pub midi_available: bool,
    /// What one pad is called, singular: "scene", "gravity". The row is
    /// drawn for both layers, and a gravity pad offering to "capture the
    /// current look into scene 3" describes the wrong layer entirely —
    /// which is worse than saying nothing, because it is the layer the
    /// other row is about.
    pub noun: &'static str,
}

impl Default for GridView {
    fn default() -> Self {
        Self {
            names: vec![None; SLOTS],
            missing: vec![false; SLOTS],
            presets: Vec::new(),
            current: None,
            in_flight: None,
            duration: 2.0,
            curve: 1,
            curve_names: Vec::new(),
            autopilot: false,
            bars: 4.0,
            auto_phase: None,
            upcoming: None,
            midi: vec![None; SLOTS],
            learning: None,
            midi_available: false,
            noun: "scene",
        }
    }
}

/// Persisted between frames: which mode the next pad press means.
///
/// A modal store is the one concession to having a single mouse button
/// that has to both fire and record. It is visibly armed, in red, and any
/// press disarms it — so the failure mode is one unintended store, not a
/// grid you cannot fire.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PadMode {
    #[default]
    Fire,
    Store,
    Clear,
    /// The next pad pressed will wait for a MIDI control to bind to it.
    ///
    /// Learning is per pad rather than per parameter because the fire
    /// parameter addresses a slot: one binding for `/scene/fire` would be
    /// one button for all sixteen scenes, which is the bug this replaces.
    Learn,
}

#[derive(Debug, Default)]
pub struct GridActions {
    /// Fire this slot (0-based). The app turns it into a `/scene/fire`
    /// write so a click and a MIDI pad take the same path.
    pub fire: Option<usize>,
    /// Capture the live parameters into this slot, as a new preset.
    pub store: Option<usize>,
    /// Put an existing preset on this pad.
    pub assign: Option<(usize, String)>,
    pub clear: Option<usize>,
    pub rename: Option<(usize, String)>,
    /// Wait for a MIDI control and bind it to firing this slot. `None`
    /// inside the `Some` cancels a wait already running.
    pub learn: Option<Option<usize>>,
    /// Drop the MIDI binding that fires this slot.
    pub unlearn: Option<usize>,
    /// Transition settings the user moved. `Option` rather than a value so
    /// an untouched control does not fight OSC or a MIDI knob writing the
    /// same parameter — the UI only speaks when it is spoken to.
    pub set_duration: Option<f32>,
    pub set_curve: Option<usize>,
    pub set_autopilot: Option<bool>,
    pub set_bars: Option<f32>,
    /// The grid itself changed and should be written to disk.
    pub changed: bool,
}

#[derive(Debug, Default, Clone)]
pub struct GridState {
    pub mode: PadMode,
    /// The slot whose name is being edited, and the text so far.
    editing: Option<(usize, String)>,
    /// The slot the context menu armed to clear: the next press on it
    /// clears it, a press anywhere else disarms. The menu's "clear" used
    /// to empty the pad on the spot while the mode-button route armed
    /// first — one action, two doors, and only one of them had a guard.
    clear_armed: Option<usize>,
}

/// Draw the row, keeping its arm state in egui's own memory.
///
/// The alternative is threading a `&mut GridState` through every caller —
/// the panel, the performance layout, the preview example and every test.
/// This is exactly what egui's temporary storage is for, and it keeps the
/// grid a drop-in widget rather than something with a lifetime.
pub fn draw(ui: &mut egui::Ui, view: &GridView) -> GridActions {
    draw_with_id(ui, view, "scene-grid")
}

/// As [`draw`], under a distinct identity.
///
/// Two grids on one screen must not share their arm state: arming "store"
/// on the scene row and then pressing a gravity pad would capture the
/// wrong layer into the wrong slot, which is a data-loss bug rather than
/// a cosmetic one.
pub fn draw_with_id(ui: &mut egui::Ui, view: &GridView, salt: &str) -> GridActions {
    let id = ui.make_persistent_id(salt);
    let mut state: GridState = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let actions = draw_with(ui, view, &mut state);
    ui.data_mut(|d| d.insert_temp(id, state));
    actions
}

/// As [`draw`], with the state passed in. For tests, which need to drive
/// the arm state directly rather than through clicks.
pub fn draw_with(ui: &mut egui::Ui, view: &GridView, state: &mut GridState) -> GridActions {
    let mut actions = GridActions::default();
    pads(ui, view, state, &mut actions);
    modes(ui, view, state, &mut actions);
    ui.add_space(4.0);
    controls(ui, view, &mut actions);
    if let Some((slot, text)) = state.editing.clone() {
        rename_row(ui, slot, text, state, &mut actions);
    }
    actions
}

fn pads(ui: &mut egui::Ui, view: &GridView, state: &mut GridState, actions: &mut GridActions) {
    let cols = SLOTS;
    // Measured once, before the first row narrows it.
    let size = pad_size(ui.available_width());
    for row in 0..SLOTS.div_ceil(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for col in 0..cols {
                let slot = row * cols + col;
                if slot >= SLOTS {
                    break;
                }
                pad(ui, slot, view, state, size, actions);
            }
        });
    }
}

fn pad(
    ui: &mut egui::Ui,
    slot: usize,
    view: &GridView,
    state: &mut GridState,
    size: egui::Vec2,
    actions: &mut GridActions,
) {
    let name = view.names.get(slot).and_then(|n| n.as_deref());
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let base = if name.is_some() { FILLED } else { EMPTY };
    p.rect_filled(rect, 3.0, base);

    // The blend, filling left to right. Drawn under the label so a long
    // name stays readable while it fills.
    if let Some((to, t)) = view.in_flight
        && to == slot
    {
        let mut fill = rect;
        fill.set_width(rect.width() * t.clamp(0.0, 1.0));
        p.rect_filled(fill, 3.0, ARRIVING.gamma_multiply(0.5));
    }

    // An armed clear outranks everything — the next press here destroys —
    // then a waiting learn, then a dangling reference: a pad that will
    // not fire is more urgent than which pad you are on.
    let broken = view.missing.get(slot).copied().unwrap_or(false);
    let waiting = view.learning == Some(slot);
    let armed_clear = state.clear_armed == Some(slot);
    let outline = if armed_clear {
        Some(ARMED)
    } else if waiting {
        Some(LEARN)
    } else if broken {
        Some(WARN)
    } else if view.in_flight.map(|(to, _)| to) == Some(slot) {
        Some(ARRIVING)
    } else if view.current == Some(slot) {
        Some(CURRENT)
    } else {
        None
    };
    if let Some(color) = outline {
        p.rect_stroke(rect, 3.0, (1.5, color), egui::StrokeKind::Inside);
    }

    // Numbered from 1 to match `/scene/fire` and a pad controller.
    p.text(
        rect.left_center() + vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{}", slot + 1),
        egui::FontId::monospace(9.0),
        Color32::from_rgb(150, 155, 170),
    );
    // The binding, along the bottom edge, where it cannot collide with the
    // name on the centre line. Only on a pad tall enough to have a bottom
    // edge to spare — the panel's four-by-four is 24 points high and the
    // chip would sit on top of the name.
    let bound = view.midi.get(slot).and_then(|m| m.as_deref());
    let roomy = rect.height() >= 40.0;
    if roomy && (waiting || bound.is_some()) {
        p.with_clip_rect(rect.shrink(2.0)).text(
            rect.right_bottom() + vec2(-4.0, -3.0),
            egui::Align2::RIGHT_BOTTOM,
            if waiting { "waiting" } else { bound.unwrap_or_default() },
            egui::FontId::monospace(8.0),
            if waiting { LEARN } else { MIDI_INK },
        );
    }

    if let Some(name) = name {
        // Clipped to the pad: a long name must not spill over its
        // neighbour and make the row unreadable. The clip stops short of
        // the bottom when a binding is drawn there, so a name long enough
        // to be truncated cannot bleed into it.
        let mut clip = rect.shrink(3.0);
        if roomy && (waiting || bound.is_some()) {
            clip.max.y -= 10.0;
        }
        p.with_clip_rect(clip).text(
            rect.left_center() + vec2(19.0, 0.0),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(11.0),
            if broken {
                WARN
            } else {
                Color32::from_rgb(225, 228, 235)
            },
        );
    }

    let response = response.on_hover_text(tooltip(
        state.mode,
        slot,
        Pad { name, noun: view.noun, broken, waiting, bound, armed_clear },
    ));
    // Double-click to rename, which is where a name gets edited in every
    // other program. The context menu keeps its entry: this is a second
    // door to the same room, not a replacement.
    //
    // The second click of the pair also reports as a click, so without the
    // guard below a rename fires the pad twice. The *first* click still
    // fires it, and deliberately: suppressing that would mean holding
    // every pad press for the length of the double-click window before
    // acting on it, and a third of a second of latency on the one gesture
    // the whole screen exists for is a far worse fault than firing the pad
    // you just clicked on.
    if response.double_clicked() && name.is_some() {
        state.editing = Some((slot, name.unwrap_or_default().to_string()));
    }
    if response.clicked() && !response.double_clicked() {
        if armed_clear {
            state.clear_armed = None;
            actions.clear = Some(slot);
            return;
        }
        // A press on any other pad disarms — the same rule as the mode
        // buttons, so the worst case is one extra click, never a clear
        // landing on a pad the menu was not opened over.
        state.clear_armed = None;
        match state.mode {
            // Firing an empty pad is a no-op in the grid itself, so this
            // does not need a guard — but offering a rename on it is the
            // useful thing to do with a click on nothing.
            PadMode::Fire if name.is_some() => actions.fire = Some(slot),
            PadMode::Fire => {}
            PadMode::Store => {
                actions.store = Some(slot);
                state.mode = PadMode::Fire;
            }
            PadMode::Clear => {
                actions.clear = Some(slot);
                state.mode = PadMode::Fire;
            }
            PadMode::Learn => {
                actions.learn = Some(Some(slot));
                state.mode = PadMode::Fire;
            }
        }
    }
    response.context_menu(|ui| {
        // Assignment first: a scene names a preset, so choosing which
        // preset is the primary thing you do to a pad. Capture is below
        // it, as the shortcut it now is.
        ui.menu_button("assign preset…", |ui| {
            if view.presets.is_empty() {
                ui.label("no presets saved yet");
                return;
            }
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for name in &view.presets {
                    if ui.button(name).clicked() {
                        actions.assign = Some((slot, name.clone()));
                        ui.close();
                    }
                }
            });
        });
        if ui
            .button("capture current look")
            .on_hover_text("save what is on screen as a preset and play it here")
            .clicked()
        {
            actions.store = Some(slot);
            ui.close();
        }
        if name.is_some() {
            if ui.button("rename").clicked() {
                state.editing = Some((slot, name.unwrap_or_default().to_string()));
                ui.close();
            }
            // Arms rather than clears: the mode-button route asks for a
            // second press, and one action must not be guarded through one
            // door and instant through the other.
            if armed_clear {
                if ui.button("cancel clear").clicked() {
                    state.clear_armed = None;
                    ui.close();
                }
            } else if ui
                .button("clear")
                .on_hover_text("arms the pad — press it to empty it")
                .clicked()
            {
                state.clear_armed = Some(slot);
                ui.close();
            }
        }
        if view.midi_available {
            ui.separator();
            match bound {
                Some(m) => {
                    if ui.button(format!("unmap {m}")).clicked() {
                        actions.unlearn = Some(slot);
                        ui.close();
                    }
                }
                None if waiting => {
                    if ui.button("cancel MIDI learn").clicked() {
                        actions.learn = Some(None);
                        ui.close();
                    }
                }
                None => {
                    if ui
                        .button("MIDI learn")
                        .on_hover_text("bind a button on your controller to firing this pad")
                        .clicked()
                    {
                        actions.learn = Some(Some(slot));
                        ui.close();
                    }
                }
            }
        }
    });
}

/// One pad's state, for the hover text.
struct Pad<'a> {
    name: Option<&'a str>,
    noun: &'a str,
    broken: bool,
    waiting: bool,
    bound: Option<&'a str>,
    armed_clear: bool,
}

/// What a pad says on hover.
///
/// Split out because it is the widget's only prose, it changes with five
/// inputs, and the row is drawn for two different layers — a tooltip that
/// hardcoded "scene" described the wrong layer on every gravity pad, which
/// is worse than describing nothing.
fn tooltip(mode: PadMode, slot: usize, pad: Pad<'_>) -> String {
    let Pad { name, noun, broken, waiting, bound, armed_clear } = pad;
    let n = slot + 1;
    if armed_clear {
        return format!("armed — press to empty {noun} {n}, press anything else to keep it");
    }
    match mode {
        PadMode::Fire if broken => format!(
            "{} — this preset no longer exists; right-click to pick another",
            name.unwrap_or(noun)
        ),
        PadMode::Fire if waiting => format!("waiting for a control to fire {noun} {n}"),
        PadMode::Fire => name.map_or_else(
            || format!("{noun} {n} — empty; right-click to assign a preset"),
            // The rename is advertised here because it was previously only
            // on the right-click menu, where nobody found it. A hover that
            // names the gesture is the cheapest possible fix, and the
            // double-click is the gesture people try first.
            |name| match bound {
                Some(m) => format!("fire {name}  ·  {m}  ·  double-click to rename"),
                None => format!("fire {name}  ·  double-click to rename"),
            },
        ),
        PadMode::Store => format!("capture the current look into {noun} {n}"),
        PadMode::Clear => format!("empty {noun} {n}"),
        PadMode::Learn => format!("bind the next control you press to firing {noun} {n}"),
    }
}

fn modes(ui: &mut egui::Ui, view: &GridView, state: &mut GridState, actions: &mut GridActions) {
    ui.horizontal(|ui| {
        let armed = state.mode == PadMode::Store;
        let store = egui::Button::new(if armed {
            egui::RichText::new("store").color(Color32::from_rgb(50, 18, 12))
        } else {
            egui::RichText::new("store")
        })
        .fill(if armed {
            ARMED
        } else {
            ui.visuals().widgets.inactive.bg_fill
        });
        if ui
            .add(store)
            .on_hover_text("arm, then press a pad to capture the current look into it")
            .clicked()
        {
            state.mode = if armed { PadMode::Fire } else { PadMode::Store };
        }
        let arming = state.mode == PadMode::Clear;
        let clear = egui::Button::new(if arming {
            egui::RichText::new("clear").color(Color32::from_rgb(50, 18, 12))
        } else {
            egui::RichText::new("clear")
        })
        .fill(if arming {
            ARMED
        } else {
            ui.visuals().widgets.inactive.bg_fill
        });
        if ui
            .add(clear)
            .on_hover_text("arm, then press a pad to empty it")
            .clicked()
        {
            state.mode = if arming {
                PadMode::Fire
            } else {
                PadMode::Clear
            };
        }

        // Mapping is per pad because the fire parameter names a slot, so
        // the arm belongs on the row rather than beside a parameter.
        if !view.midi_available {
            return;
        }
        // A wait already running is cancelled from here too: the pad shows
        // "waiting" but the eye goes to the button that started it.
        if let Some(slot) = view.learning {
            // Dark text on the amber fill: the default light grey was
            // near-unreadable on it, on the one button that says the grid
            // is waiting for your controller.
            let cancel = egui::Button::new(
                egui::RichText::new(format!("waiting for {}", slot + 1))
                    .color(Color32::from_rgb(46, 32, 12)),
            )
            .fill(LEARN);
            if ui
                .add(cancel)
                .on_hover_text("press a button on your controller, or click to cancel")
                .clicked()
            {
                actions.learn = Some(None);
            }
            return;
        }
        let learning = state.mode == PadMode::Learn;
        let midi = egui::Button::new(if learning {
            egui::RichText::new("MIDI").color(Color32::from_rgb(46, 32, 12))
        } else {
            egui::RichText::new("MIDI")
        })
        .fill(if learning {
            LEARN
        } else {
            ui.visuals().widgets.inactive.bg_fill
        });
        if ui
            .add(midi)
            .on_hover_text("arm, then press a pad to bind a controller button to it")
            .clicked()
        {
            state.mode = if learning { PadMode::Fire } else { PadMode::Learn };
        }
    });
}

/// The transition settings.
///
/// Every one of these is a parameter underneath, so it also has OSC, MIDI
/// learn and a row in the panel. These are here because reaching for the
/// parameter list to change a blend time is the wrong distance away from
/// the pads you are pressing.
fn controls(ui: &mut egui::Ui, view: &GridView, actions: &mut GridActions) {
    // Blend, curve and autopilot on one line. They are one thought — "how
    // does a scene change happen" — and stacking them over three rows both
    // wasted the height the faders need and made each one look like an
    // unrelated setting.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("blend").size(13.0).color(LABEL));
        let mut duration = view.duration;
        let slider = egui::Slider::new(&mut duration, 0.0..=30.0)
            .suffix(" s")
            .clamping(egui::SliderClamping::Always);
        if ui
            .add_sized([170.0, 20.0], slider)
            .on_hover_text(format!("how long a {} change takes. 0 is a cut", view.noun))
            .changed()
        {
            actions.set_duration = Some(duration);
        }
        ui.add_space(14.0);
        // The curve as a row of names rather than a number: "ease out"
        // means something, 3.0 does not.
        for (i, name) in view.curve_names.iter().enumerate() {
            let text = egui::RichText::new(name)
                .size(13.0)
                .color(if view.curve == i {
                    Color32::from_rgb(240, 244, 250)
                } else {
                    LABEL
                });
            if ui.selectable_label(view.curve == i, text).clicked() {
                actions.set_curve = Some(i);
            }
        }
    });
    ui.horizontal(|ui| {
        autopilot_toggle(ui, view, actions);
        let mut bars = view.bars;
        let slider = egui::Slider::new(&mut bars, 0.25..=16.0)
            .suffix(" bars")
            .clamping(egui::SliderClamping::Always);
        if ui
            .add_sized([150.0, 20.0], slider)
            .on_hover_text("how often the autopilot steps")
            .changed()
        {
            actions.set_bars = Some(bars);
        }
    });
}

/// The autopilot switch, drawn as a countdown rather than a checkbox.
///
/// A checkbox was the wrong widget for this. Autopilot is the one control
/// that changes the output when nobody touched anything, so "is it on" has
/// to be answerable from across a room — and a 13-point tick box beside a
/// grey label is not. Worse, a tick box can only say *on*, and on with a
/// sixteen-bar rate looks exactly like off for the best part of a minute.
///
/// So the switch shows the thing that distinguishes them: it fills towards
/// the next fire. Sweeping means running, flat means off, and the two are
/// never confusable no matter how slow the rate. It also names the pad it
/// is about to move to, because during a set the question is not only
/// whether something is coming but what.
fn autopilot_toggle(ui: &mut egui::Ui, view: &GridView, actions: &mut GridActions) {
    let label = if view.autopilot { "AUTO" } else { "auto" };
    let name = view
        .upcoming
        .and_then(|s| view.names.get(s).cloned().flatten());
    // ASCII only. egui's default font has no arrow glyph, so "→" renders
    // as a tofu box — which on the one control that says whether the show
    // is running itself reads as a bug.
    let text = match (view.autopilot, name) {
        (true, Some(n)) => format!("{label}  >  {n}"),
        (true, None) => format!("{label}  >  (empty grid)"),
        (false, _) => label.to_string(),
    };

    let size = vec2(ui.available_width().clamp(120.0, 210.0), 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, if view.autopilot { AUTO_BED } else { EMPTY });

    // The sweep. Drawn under the label so the text stays readable as the
    // fill passes behind it.
    if let Some(phase) = view.auto_phase {
        let w = rect.width() * phase.clamp(0.0, 1.0);
        p.rect_filled(
            egui::Rect::from_min_size(rect.left_top(), vec2(w, rect.height())),
            4.0,
            AUTO_ON,
        );
    }
    // A lit border as well as a fill: at phase 0 the sweep is zero pixels
    // wide, and without this the control would blink to looking off once
    // per cycle.
    if view.autopilot {
        p.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.5, AUTO_ON),
            egui::StrokeKind::Inside,
        );
    }
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(13.0),
        if view.autopilot {
            Color32::from_rgb(238, 250, 240)
        } else {
            Color32::from_rgb(150, 154, 162)
        },
    );

    if response
        .on_hover_text("walk the filled pads in time with the clock")
        .clicked()
    {
        actions.set_autopilot = Some(!view.autopilot);
    }
}

fn rename_row(
    ui: &mut egui::Ui,
    slot: usize,
    mut text: String,
    state: &mut GridState,
    actions: &mut GridActions,
) {
    ui.horizontal(|ui| {
        ui.label(format!("name {}", slot + 1));
        let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(120.0));
        let done = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("ok").clicked() || done {
            actions.rename = Some((slot, text.clone()));
            state.editing = None;
        } else if ui.button("cancel").clicked() {
            state.editing = None;
        } else {
            state.editing = Some((slot, text));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> GridView {
        let mut names = vec![None; SLOTS];
        names[0] = Some("opener".into());
        names[5] = Some("drop".into());
        GridView {
            names,
            current: Some(0),
            in_flight: Some((5, 0.4)),
            curve_names: vec!["linear".into(), "smooth".into()],
            ..Default::default()
        }
    }

    /// Draw the row and return every string it emitted.
    ///
    /// Without a `screen_rect` egui clips the whole window away and draws
    /// nothing, so a test that omits it passes by seeing an empty string
    /// no matter what the code does.
    fn run(view: &GridView, state: &mut GridState) -> String {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(500.0, 700.0),
            )),
            ..Default::default()
        };
        let mut text = String::new();
        for _ in 0..2 {
            ctx.begin_pass(input.clone());
            egui::Area::new(egui::Id::new("grid-test")).show(&ctx, |ui| {
                draw_with(ui, view, state);
            });
            text = collect_text(&ctx.end_pass().shapes);
        }
        text
    }

    fn collect_text(shapes: &[egui::epaint::ClippedShape]) -> String {
        fn walk(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(t) => {
                    // The painted glyphs, not the string the galley was
                    // given: an elided label reports its full text
                    // through Galley::text(), so a `contains` check
                    // passes whether or not the words reached the eye.
                    out.extend(t.galley.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| g.chr)));
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
            out.push(' ');
        }
        let mut out = String::new();
        for s in shapes {
            walk(&s.shape, &mut out);
        }
        out
    }

    /// The pads have to be numbered, and the numbers have to match what
    /// `/scene/fire` and a MIDI pad address. A row you cannot map to a
    /// controller is a row you cannot play.
    #[test]
    fn every_pad_is_drawn_and_numbered_from_one() {
        let v = view();
        let mut state = GridState::default();
        let text = run(&v, &mut state);
        for n in 1..=SLOTS {
            assert!(text.contains(&n.to_string()), "pad {n} missing: {text}");
        }
        assert!(text.contains("opener"), "cell name missing: {text}");
        assert!(text.contains("drop"), "cell name missing: {text}");
    }

    /// The UI's slot count and the model's must agree, or the last pads
    /// exist and cannot be pressed.
    #[test]
    fn the_row_shows_every_slot_the_grid_has() {
        assert_eq!(SLOTS, vizz_mod::scene::SLOTS);
    }

    /// With sixteen pads sharing one fire parameter, the only place the
    /// map can be read is on the pads themselves. If a bound pad looks
    /// exactly like an unbound one there is no way to tell which of a
    /// controller's buttons does what short of pressing them all — during
    /// a set.
    #[test]
    fn a_mapped_pad_shows_what_fires_it() {
        let mut v = view();
        v.midi_available = true;
        v.midi[0] = Some("ch1 note36".into());
        let text = run(&v, &mut GridState::default());
        assert!(text.contains("ch1 note36"), "binding not shown on the pad: {text}");
    }

    /// A learn that gives no sign it is waiting is indistinguishable from
    /// a click that did nothing.
    #[test]
    fn a_pad_waiting_for_a_control_says_so() {
        let mut v = view();
        v.midi_available = true;
        v.learning = Some(5);
        let text = run(&v, &mut GridState::default());
        assert!(text.contains("waiting"), "no sign of the pending learn: {text}");
    }

    /// The arm is the only route to a per-pad binding, so it has to exist
    /// — and must not appear when there is no MIDI to bind, where it would
    /// start a wait nothing could ever end.
    #[test]
    fn the_midi_arm_appears_only_when_there_is_midi() {
        let mut v = view();
        v.midi_available = true;
        assert!(
            run(&v, &mut GridState::default()).contains("MIDI"),
            "no way to map a pad"
        );

        v.midi_available = false;
        assert!(
            !run(&v, &mut GridState::default()).contains("MIDI"),
            "offered mapping with no MIDI available"
        );
    }

    /// The row is drawn for both layers. A gravity pad that offers to
    /// capture a look "into scene 3" names the other row entirely — worse
    /// than saying nothing, because it points at real pads that exist and
    /// hold something else.
    #[test]
    fn a_pad_never_describes_the_other_layer() {
        let pad = |noun| Pad {
            name: None,
            noun,
            broken: false,
            waiting: false,
            bound: None,
            armed_clear: false,
        };
        for mode in [PadMode::Fire, PadMode::Store, PadMode::Clear, PadMode::Learn] {
            let text = tooltip(mode, 2, pad("gravity"));
            assert!(text.contains("gravity"), "{mode:?} did not say gravity: {text}");
            assert!(!text.contains("scene"), "{mode:?} named the wrong layer: {text}");
            // And it numbers the pad the way the pad is labelled and the
            // fire parameter addresses it: from one.
            assert!(text.contains('3'), "{mode:?} misnumbered the pad: {text}");
        }
    }

    /// A filled pad's hover is where the binding is named in full — the
    /// chip on the pad is eight points and abbreviated by width.
    #[test]
    fn a_mapped_pads_hover_names_its_control() {
        let text = tooltip(
            PadMode::Fire,
            0,
            Pad {
                name: Some("opener"),
                noun: "scene",
                broken: false,
                waiting: false,
                bound: Some("ch1 note36"),
                armed_clear: false,
            },
        );
        assert!(text.contains("ch1 note36"), "binding not named on hover: {text}");
        assert!(text.contains("rename"), "lost the rename hint: {text}");
    }

    /// Every rect-stroke colour the row painted — the outlines. Fills are
    /// deliberately excluded: the armed mode buttons fill in ARMED, and
    /// counting them would let a pad without its outline pass.
    fn stroke_colours(view: &GridView, state: &mut GridState) -> Vec<Color32> {
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        // The clock must advance across passes: a fresh Area fades in, and
        // while it does every colour is alpha-scaled — an ARMED stroke
        // read mid-fade equals no colour this test knows about.
        for i in 0..6 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(500.0, 700.0),
                )),
                time: Some(i as f64 * 0.2),
                ..Default::default()
            };
            ctx.begin_pass(input);
            egui::Area::new(egui::Id::new("grid-test")).show(&ctx, |ui| {
                draw_with(ui, view, state);
            });
            fn walk(shape: &egui::Shape, out: &mut Vec<Color32>) {
                match shape {
                    egui::Shape::Rect(r) if !r.stroke.is_empty() => out.push(r.stroke.color),
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    _ => {}
                }
            }
            out.clear();
            for s in &ctx.end_pass().shapes {
                walk(&s.shape, &mut out);
            }
        }
        out
    }

    /// The context menu's clear arms instead of firing — the same guard
    /// as the mode-button route. An armed pad has to say so: the ARMED
    /// outline, and a hover naming what the next press will do.
    #[test]
    fn a_pad_armed_to_clear_wears_the_armed_outline_and_says_so() {
        let v = view();
        let mut state = GridState::default();
        assert!(
            !stroke_colours(&v, &mut state).contains(&ARMED),
            "an unarmed grid painted the armed colour"
        );
        state.clear_armed = Some(0);
        assert!(
            stroke_colours(&v, &mut state).contains(&ARMED),
            "the armed pad has no armed outline"
        );
        let text = tooltip(
            PadMode::Fire,
            0,
            Pad {
                name: Some("opener"),
                noun: "scene",
                broken: false,
                waiting: false,
                bound: None,
                armed_clear: true,
            },
        );
        assert!(text.contains("armed"), "the hover does not name the armed state: {text}");
    }

    /// A broken pad is a warning, not an armed action: nothing about it
    /// destroys on the next press. It borrowed the ARMED colour for a
    /// while, which claimed exactly that.
    #[test]
    fn a_broken_pad_warns_rather_than_reading_armed() {
        let mut v = view();
        v.missing[0] = true;
        let mut state = GridState::default();
        let colours = stroke_colours(&v, &mut state);
        assert!(colours.contains(&WARN), "no warning outline on the broken pad");
        assert!(
            !colours.contains(&ARMED),
            "a broken pad still reads as an armed destructive action"
        );
    }

    /// Arming store must be visible. An invisible mode that changes what
    /// a click does is the worst kind of state.
    #[test]
    fn the_store_and_clear_arms_are_offered() {
        let v = view();
        let mut state = GridState::default();
        let text = run(&v, &mut state);
        assert!(text.contains("store"), "no way to store: {text}");
        assert!(text.contains("clear"), "no way to clear: {text}");
        assert!(text.contains("blend"), "no blend time: {text}");
    }
}

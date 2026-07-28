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

/// Where the row is being drawn, which decides how wide it may be and
/// whether the transition settings come with it.
///
/// One row of sixteen is the shape this wants — it is the shape of a
/// sequencer, and it is what makes pad 11 findable without counting. But
/// sixteen pads wide enough to carry a name is wider than the control
/// panel has any business being, so the panel folds it to four by four and
/// the performance layout, which owns the whole window, lays it out
/// straight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// In the control panel: four by four, pads only. The transition
    /// settings are parameters like everything else and already have rows
    /// in the list below — drawing them twice would give the panel two
    /// controls for one value.
    Panel,
    /// On the performance layout: sixteen across, with the settings, since
    /// changing a blend time mid-set must not mean leaving the layout.
    Stage,
}

impl Shape {
    fn cols(self) -> usize {
        match self {
            Shape::Panel => 4,
            Shape::Stage => SLOTS,
        }
    }

    /// Pad size for the width actually on offer.
    ///
    /// Sixteen across only works if sixteen fit: at a fixed width the row
    /// runs past the right edge of a 900-point window, and everything laid
    /// out from that edge — the frame rate, the tempo, the tap button —
    /// goes with it. So the stage row divides up what it is given instead,
    /// with a floor below which the pads stop being hittable and the row
    /// is better off overflowing visibly than shrinking to nothing.
    fn pad(self, available: f32) -> egui::Vec2 {
        match self {
            Shape::Panel => vec2(66.0, 24.0),
            Shape::Stage => {
                let cols = self.cols() as f32;
                let each = (available - (cols - 1.0) * GAP) / cols;
                vec2(each.clamp(34.0, 96.0), 46.0)
            }
        }
    }

    fn settings(self) -> bool {
        self == Shape::Stage
    }
}

const GAP: f32 = 4.0;

const FILLED: Color32 = Color32::from_rgb(58, 66, 84);
const EMPTY: Color32 = Color32::from_rgb(34, 36, 42);
const CURRENT: Color32 = Color32::from_rgb(110, 180, 255);
const ARRIVING: Color32 = Color32::from_rgb(255, 175, 80);
const ARMED: Color32 = Color32::from_rgb(255, 120, 90);
/// The autopilot's own colour. Green rather than the blue of `CURRENT` or
/// the amber of `ARRIVING`: those two say where the grid *is*, and this
/// says something is driving it. Sharing a colour with either would make
/// the sweep read as another transition.
const AUTO_ON: Color32 = Color32::from_rgb(72, 160, 104);
const AUTO_BED: Color32 = Color32::from_rgb(30, 46, 36);
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
}

/// Draw the row, keeping its arm state in egui's own memory.
///
/// The alternative is threading a `&mut GridState` through every caller —
/// the panel, the performance layout, the preview example and every test.
/// This is exactly what egui's temporary storage is for, and it keeps the
/// grid a drop-in widget rather than something with a lifetime.
pub fn draw(ui: &mut egui::Ui, view: &GridView, shape: Shape) -> GridActions {
    draw_with_id(ui, view, shape, "scene-grid")
}

/// As [`draw`], under a distinct identity.
///
/// Two grids on one screen must not share their arm state: arming "store"
/// on the scene row and then pressing a gravity pad would capture the
/// wrong layer into the wrong slot, which is a data-loss bug rather than
/// a cosmetic one.
pub fn draw_with_id(
    ui: &mut egui::Ui,
    view: &GridView,
    shape: Shape,
    salt: &str,
) -> GridActions {
    let id = ui.make_persistent_id(salt);
    let mut state: GridState = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let actions = draw_with(ui, view, &mut state, shape);
    ui.data_mut(|d| d.insert_temp(id, state));
    actions
}

/// As [`draw`], with the state passed in. For tests, which need to drive
/// the arm state directly rather than through clicks.
pub fn draw_with(
    ui: &mut egui::Ui,
    view: &GridView,
    state: &mut GridState,
    shape: Shape,
) -> GridActions {
    let mut actions = GridActions::default();
    pads(ui, view, state, shape, &mut actions);
    modes(ui, state, &mut actions);
    if shape.settings() {
        ui.add_space(4.0);
        controls(ui, view, &mut actions);
    }
    if let Some((slot, text)) = state.editing.clone() {
        rename_row(ui, slot, text, state, &mut actions);
    }
    actions
}

fn pads(
    ui: &mut egui::Ui,
    view: &GridView,
    state: &mut GridState,
    shape: Shape,
    actions: &mut GridActions,
) {
    let cols = shape.cols();
    // Measured once, before the first row narrows it.
    let size = shape.pad(ui.available_width());
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

    // A dangling reference outranks every other outline: a pad that will
    // not fire is more urgent than which pad you are on.
    let broken = view.missing.get(slot).copied().unwrap_or(false);
    let outline = if broken {
        Some(ARMED)
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
    if let Some(name) = name {
        // Clipped to the pad: a long name must not spill over its
        // neighbour and make the row unreadable.
        p.with_clip_rect(rect.shrink(3.0)).text(
            rect.left_center() + vec2(19.0, 0.0),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(11.0),
            if broken {
                ARMED
            } else {
                Color32::from_rgb(225, 228, 235)
            },
        );
    }

    let response = response.on_hover_text(match state.mode {
        PadMode::Fire if broken => format!(
            "{} — this preset no longer exists; right-click to pick another",
            name.unwrap_or("scene")
        ),
        PadMode::Fire => name.map_or_else(
            || format!("scene {} — empty; right-click to play a preset here", slot + 1),
            // The rename is advertised here because it was previously only
            // on the right-click menu, where nobody found it. A hover that
            // names the gesture is the cheapest possible fix, and the
            // double-click below is the gesture people try first.
            |n| format!("fire {n}  ·  double-click to rename"),
        ),
        PadMode::Store => format!("capture the current look into scene {}", slot + 1),
        PadMode::Clear => format!("empty scene {}", slot + 1),
    });
    // Double-click to rename, which is where a name gets edited in every
    // other program. The context menu keeps its entry: this is a second
    // door to the same room, not a replacement.
    if response.double_clicked() && name.is_some() {
        state.editing = Some((slot, name.unwrap_or_default().to_string()));
    }
    if response.clicked() {
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
        }
    }
    response.context_menu(|ui| {
        // Assignment first: a scene names a preset, so choosing which
        // preset is the primary thing you do to a pad. Capture is below
        // it, as the shortcut it now is.
        ui.menu_button("play preset...", |ui| {
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
            if ui.button("clear").clicked() {
                actions.clear = Some(slot);
                ui.close();
            }
        }
    });
}

fn modes(ui: &mut egui::Ui, state: &mut GridState, _actions: &mut GridActions) {
    ui.horizontal(|ui| {
        let armed = state.mode == PadMode::Store;
        let store = egui::Button::new("store").fill(if armed {
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
        let clear = egui::Button::new("clear").fill(if arming {
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
            .on_hover_text("how long a scene change takes. 0 is a cut")
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

    let size = vec2(ui.available_width().min(210.0).max(120.0), 26.0);
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
                draw_with(ui, view, state, Shape::Stage);
            });
            text = collect_text(&ctx.end_pass().shapes);
        }
        text
    }

    fn collect_text(shapes: &[egui::epaint::ClippedShape]) -> String {
        fn walk(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(t) => out.push_str(t.galley.text()),
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

//! The performance layout: what you look at during a set.
//!
//! Deliberately not a smaller version of the control panel. The panel is
//! for building a look — every parameter, dense, read at a desk. This is
//! for playing one: a handful of assigned faders, large enough to hit
//! without aiming, and the two or three facts that matter when something
//! goes wrong (is the output live, is it dropping frames).
//!
//! Everything else is deliberately absent. A control you did not decide to
//! reach for in advance is a control you will not find in a dark room, and
//! having it on screen only makes the ones you do want harder to hit.

use egui::{Color32, Sense, Vec2, vec2};
use vizz_mod::perform::{MACRO_COUNT, Macros};
use vizz_params::ParamRegistry;

use crate::panel::{AudioView, OutputStatus};

/// Big enough to hit without aiming, which is the whole design constraint.
const FADER_W: f32 = 74.0;
const FADER_H: f32 = 230.0;

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
    /// The scene grid, laid out across the full width here.
    pub grid: &'a crate::grid_view::GridView,
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
            // Fill the window: an Area is content-sized, so without
            // this the layout collapses to whatever the widgets need.
            let full = ui.ctx().input(|i| i.raw.screen_rect).map(|r| r.size())
                .unwrap_or_else(|| vec2(900.0, 520.0));
            ui.set_min_size(full);
            status_strip(ui, state, &mut actions);
            ui.add_space(6.0);
            // The grid above the presets: this is the thing you play, and
            // sixteen across is what it is for. The preset row stays,
            // because a preset is still the fastest way to a known look
            // when a transition is the wrong answer.
            actions.grid = crate::grid_view::draw(ui, state.grid, crate::grid_view::Shape::Stage);
            ui.add_space(8.0);
            preset_row(ui, state, &mut actions);
            ui.add_space(10.0);
            faders(ui, registry, macros, &mut actions);
            ui.add_space(14.0);
            master(ui, registry, full.x - 28.0);
        });

    actions
}

/// Presets as a row of buttons, numbered to match the keyboard.
///
/// The whole point of the performance layout is that everything you play
/// with is on one screen. Presets were the largest thing missing: without
/// them, changing look meant leaving the layout, which is the one thing it
/// exists to avoid. Numbered because the number keys fire the same slots,
/// so the row doubles as the legend for the keyboard.
fn preset_row(ui: &mut egui::Ui, state: &PerformanceState<'_>, actions: &mut PerformanceActions) {
    if state.presets.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (i, name) in state.presets.iter().enumerate() {
            let slot = i as u32 + 1;
            // Only the first ten have a key; showing a number beside the
            // eleventh would promise a shortcut that does not exist.
            let label = if slot <= 10 {
                format!("{}  {name}", slot % 10)
            } else {
                name.clone()
            };
            let button = egui::Button::new(egui::RichText::new(label).size(15.0))
                .min_size(vec2(0.0, 30.0));
            if ui.add(button).clicked() {
                actions.preset_slot = Some(slot);
            }
        }
    });
}

fn status_strip(ui: &mut egui::Ui, state: &PerformanceState<'_>, actions: &mut PerformanceActions) {
    ui.horizontal(|ui| {
        for out in state.outputs {
            let (r, _) = ui.allocate_exact_size(vec2(11.0, 11.0), Sense::hover());
            ui.painter().circle_filled(
                r.center(),
                5.0,
                if out.live {
                    Color32::from_rgb(90, 205, 120)
                } else {
                    Color32::from_rgb(120, 120, 120)
                },
            );
            ui.label(egui::RichText::new(&out.name).size(13.0));
            ui.add_space(8.0);
        }
        if state.outputs.is_empty() {
            ui.label(egui::RichText::new("no outputs").size(13.0).weak());
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("edit").clicked() {
                actions.exit = true;
            }
            ui.add_space(10.0);

            // Frame health, coloured rather than numeric-only: at a glance
            // you need "is it fine", not a percentile.
            ui.label(
                egui::RichText::new(format!("{:.0} fps", state.fps))
                    .size(15.0)
                    .color(if state.over_budget {
                        Color32::from_rgb(240, 150, 90)
                    } else {
                        Color32::from_rgb(150, 210, 160)
                    }),
            );
            ui.add_space(14.0);

            if ui.button("tap").clicked() {
                actions.tapped = true;
            }
            ui.label(egui::RichText::new(format!("{:.1} bpm", state.bpm)).size(15.0));
            // Beat indicator: brightest on the downbeat, so tempo is
            // visible without reading a number.
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            let glow = (1.0 - state.bar_phase * 4.0).clamp(0.0, 1.0);
            ui.painter().circle_filled(
                r.center(),
                7.0,
                Color32::from_rgb(
                    (70.0 + 185.0 * glow) as u8,
                    (70.0 + 150.0 * glow) as u8,
                    95,
                ),
            );
        });
    });

    // Audio bands, if there is any. Small, but present: when the visuals
    // stop reacting the first question is whether audio is still arriving.
    if state.audio.connected {
        ui.horizontal(|ui| {
            for b in state.audio.bands {
                let (r, _) = ui.allocate_exact_size(vec2(46.0, 6.0), Sense::hover());
                ui.painter().rect_filled(r, 2.0, Color32::from_black_alpha(150));
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        r.left_top(),
                        vec2(r.width() * b.clamp(0.0, 1.0), r.height()),
                    ),
                    2.0,
                    Color32::from_rgb(110, 200, 150),
                );
            }
        });
    }
}

fn faders(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    macros: &mut Macros,
    actions: &mut PerformanceActions,
) {
    ui.horizontal(|ui| {
        for i in 0..MACRO_COUNT {
            ui.allocate_ui_with_layout(
                vec2(FADER_W, FADER_H + 46.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    fader(ui, registry, macros, i, actions);
                },
            );
            ui.add_space(6.0);
        }
    });
}

fn fader(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    macros: &mut Macros,
    slot: usize,
    actions: &mut PerformanceActions,
) {
    let assigned = macros.get(slot).map(str::to_owned);
    let id = assigned.as_deref().and_then(|a| registry.id(a));

    match (assigned.as_deref(), id) {
        (Some(addr), Some(param)) => {
            let def = &registry.defs()[param.index()];
            let value = registry.target(param);
            if let Some(v) = vertical_fader(ui, value, def.min, def.max) {
                registry.set(param, v);
            }
            let value = registry.target(param);
            // Value under the fader rather than inside it: a number drawn
            // over a moving bar is unreadable at a glance.
            //
            // A stepped parameter shows its position's name. `5.000` under
            // a fader called `mode` is unreadable in a different sense —
            // it is legible and still tells you nothing.
            let shown = def
                .label_for(value)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{value:.2}"));
            ui.label(egui::RichText::new(shown).size(12.0).monospace());
            // The short name is what identifies the fader; the full address
            // is only needed when reassigning.
            let short = addr.rsplit('/').next().unwrap_or(addr);
            if ui
                .add(egui::Label::new(egui::RichText::new(short).size(13.0)).sense(Sense::click()))
                .on_hover_text(format!("{addr} — click to reassign"))
                .clicked()
            {
                open_assign(ui, slot);
            }
        }
        _ => {
            // Unassigned, or assigned to a parameter this build no longer
            // has: draw an inert placeholder rather than hiding the slot,
            // so the layout does not reflow mid-set.
            let (r, _) = ui.allocate_exact_size(vec2(FADER_W, FADER_H), Sense::hover());
            ui.painter().rect_filled(r, 4.0, Color32::from_rgb(26, 28, 33));
            ui.label(egui::RichText::new("—").size(12.0).weak());
            if ui
                .add(egui::Label::new(egui::RichText::new("assign").size(13.0).weak()).sense(Sense::click()))
                .clicked()
            {
                open_assign(ui, slot);
            }
        }
    }

    assign_popup(ui, registry, macros, slot, actions);
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
    egui::Area::new(popup_id.with("area"))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_height(280.0);
                egui::ScrollArea::vertical().id_salt(popup_id).show(ui, |ui| {
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
/// aiming — fails with it. Here the *whole column* is the drag target, not
/// just the handle, which is the property that actually matters in a dark
/// room.
///
/// Returns the new value when the user moved it.
fn vertical_fader(ui: &mut egui::Ui, value: f32, min: f32, max: f32) -> Option<f32> {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(FADER_W, FADER_H), Sense::click_and_drag());
    let span = (max - min).max(f32::EPSILON);
    let t = ((value - min) / span).clamp(0.0, 1.0);

    let p = ui.painter();
    let track = rect.shrink2(vec2(rect.width() * 0.18, 0.0));
    p.rect_filled(track, 5.0, Color32::from_rgb(28, 30, 35));
    // Fill from the bottom: a fader reads as "how much", and a bar growing
    // upward says that without needing the number.
    let fill_h = track.height() * t;
    p.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(track.left(), track.bottom() - fill_h),
            vec2(track.width(), fill_h),
        ),
        5.0,
        Color32::from_rgb(58, 104, 148),
    );
    // Chunky handle, wider than the track so it reads as grabbable and
    // stays visible against the fill.
    let hy = track.bottom() - fill_h;
    let handle = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, hy.clamp(rect.top() + 6.0, rect.bottom() - 6.0)),
        vec2(rect.width(), 13.0),
    );
    p.rect_filled(handle, 3.0, Color32::from_rgb(208, 216, 226));

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

/// The one control that must always be reachable, given its own row and the
/// full width. If everything else goes wrong this is what gets pulled.
fn master(ui: &mut egui::Ui, registry: &ParamRegistry, width: f32) {
    let Some(id) = registry.id("/master/dim") else { return };
    let def = &registry.defs()[id.index()];
    let value = registry.target(id);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("MASTER").size(16.0).strong());
        // Width comes from the caller, not `available_width`: inside a
        // content-sized Area that reports whatever the widgets so far
        // needed, and the master ends up a stub.
        let w = (width - 150.0).max(140.0);
        let (rect, response) = ui.allocate_exact_size(vec2(w, 44.0), Sense::click_and_drag());
        let span = (def.max - def.min).max(f32::EPSILON);
        let t = ((value - def.min) / span).clamp(0.0, 1.0);
        let p = ui.painter();
        p.rect_filled(rect, 5.0, Color32::from_rgb(28, 30, 35));
        p.rect_filled(
            egui::Rect::from_min_size(rect.left_top(), vec2(rect.width() * t, rect.height())),
            5.0,
            Color32::from_rgb(150, 70, 70),
        );
        let hx = rect.left() + rect.width() * t;
        p.rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(hx.clamp(rect.left() + 5.0, rect.right() - 5.0), rect.center().y),
                vec2(10.0, rect.height()),
            ),
            3.0,
            Color32::from_rgb(232, 238, 245),
        );
        if (response.dragged() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let nt = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            registry.set(id, def.min + nt * span);
        }
        ui.label(egui::RichText::new(format!("{value:.2}")).size(16.0).monospace());
    });
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
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let audio = AudioView::default();
        let names = ["Slow bloom".to_string(), "Butterfly".to_string()];
        let grid = crate::grid_view::GridView::default();
        let state = PerformanceState {
            outputs: &[OutputStatus { name: "syphon:vizz".into(), live: true }],
            audio: &audio,
            fps: 60.0,
            over_budget: false,
            bpm: 128.0,
            bar_phase: 0.1,
            presets: &names,
            grid: &grid,
        };
        let mut text = String::new();
        for i in 0..8 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(900.0, 520.0),
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
        let mut macros = Macros { slots: vec![None; MACRO_COUNT] };
        macros.set(0, Some("/particles/size".into()));
        macros.set(1, Some("/fx/glow".into()));
        // Deliberately stale: this parameter does not exist here.
        macros.set(2, Some("/gone/missing".into()));

        let text = render(&mut macros, &reg);
        assert!(text.contains("size"), "assigned slot missing: {text}");
        assert!(text.contains("glow"), "assigned slot missing: {text}");
        assert!(text.contains("MASTER"), "master fader missing: {text}");
        assert!(text.contains("syphon:vizz"), "output status missing: {text}");
        assert!(text.contains("128.0 bpm"), "tempo missing: {text}");
        // The stale slot renders as an empty placeholder rather than
        // vanishing, so the fader layout does not reflow mid-set.
        assert!(text.contains("assign"), "stale slot did not fall back: {text}");
    }

    #[test]
    fn empty_macros_still_draw_the_master() {
        let reg = registry();
        let mut macros = Macros { slots: vec![None; MACRO_COUNT] };
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
        assert!(text.contains("1  Slow bloom"), "slot numbers missing: {text}");
    }
}

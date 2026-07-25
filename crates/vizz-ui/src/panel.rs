//! The control panel content.
//!
//! Every slider is generated from [`ParamRegistry`]'s own metadata rather
//! than hand-written, so adding a parameter to the app's table gives it a
//! control automatically and the GUI can never drift out of sync with the
//! OSC surface. The panel writes targets exactly like the OSC listener
//! does — it gets no privileged access to the renderer.

use vizz_health::HealthSnapshot;
use vizz_midi::{MidiMap, Source};
use vizz_mod::{ModEngine, Rate, Shape};
use vizz_params::ParamRegistry;

/// Status of one video output, as shown in the panel.
pub struct OutputStatus {
    pub name: String,
    pub live: bool,
}

/// A read-only view of MIDI, refreshed without ever blocking the render
/// thread — if the MIDI lock is busy the previous snapshot is reused.
#[derive(Default, Clone)]
pub struct MidiView {
    pub available: bool,
    pub connected: Vec<String>,
    pub map: MidiMap,
    pub learn_target: Option<String>,
    pub last_source: Option<Source>,
}

/// What the panel asks the app to do. Returned rather than applied
/// directly so the panel keeps no privileged access of its own.
#[derive(Default)]
pub struct PanelActions {
    /// Begin MIDI-learn for this parameter (or cancel, with None).
    pub set_learn_target: Option<Option<String>>,
    /// Remove the MIDI binding for this parameter.
    pub clear_binding: Option<String>,
}

/// Everything the panel displays that it cannot read from the registry.
pub struct PanelState {
    /// Newer version string, if the background check found one.
    pub update_available: Option<String>,
    pub health: Option<HealthSnapshot>,
    pub outputs: Vec<OutputStatus>,
    /// Recent frame times in ms, oldest first, for the sparkline.
    pub frame_times_ms: Vec<f32>,
    pub frame_budget_ms: f32,
    pub midi: MidiView,
}

pub fn draw(
    ctx: &egui::Context,
    registry: &ParamRegistry,
    state: &PanelState,
    modulation: &mut ModEngine,
) -> PanelActions {
    let mut actions = PanelActions::default();
    egui::Window::new("vizz")
        .default_pos([12.0, 12.0])
        .default_width(360.0)
        .resizable(true)
        .show(ctx, |ui| {
            update_banner(ui, state);
            health_section(ui, state);
            ui.separator();
            outputs_section(ui, state);
            ui.separator();
            midi_section(ui, state);
            ui.separator();
            modulation_section(ui, registry, modulation);
            ui.separator();
            params_section(ui, registry, state, modulation, &mut actions);
            ui.separator();
            ui.small("Tab toggles this panel · Esc quits");
        });
    actions
}

/// Notify, never install: the link opens the release page and the user
/// picks the moment. Nothing about a running show changes.
fn update_banner(ui: &mut egui::Ui, state: &PanelState) {
    let Some(version) = &state.update_available else { return };
    ui.horizontal(|ui| {
        ui.colored_label(
            egui::Color32::from_rgb(255, 200, 90),
            format!("vizz {version} available"),
        );
        ui.hyperlink_to("download", vizz_update::RELEASES_URL);
    });
    ui.separator();
}

fn midi_section(ui: &mut egui::Ui, state: &PanelState) {
    ui.label(egui::RichText::new("MIDI").strong());
    if !state.midi.available {
        ui.small("unavailable");
        return;
    }
    if state.midi.connected.is_empty() {
        ui.small("no devices — plug one in, it connects automatically");
    } else {
        for name in &state.midi.connected {
            ui.small(format!("· {name}"));
        }
    }
    // While learning, echo whatever is arriving: the usual failure is a
    // controller that is not sending at all, and this distinguishes that
    // from a mapping problem immediately.
    if let Some(target) = &state.midi.learn_target {
        let seen = state
            .midi
            .last_source
            .map(|s| s.label())
            .unwrap_or_else(|| "nothing yet".into());
        ui.colored_label(
            egui::Color32::from_rgb(255, 200, 90),
            format!("learning {target} — move a control (seen: {seen})"),
        );
    }
}

/// Modulation is owned by the render thread, and the panel draws on that
/// same thread, so it edits the engine directly — no lock, no snapshot,
/// no action plumbing.
fn modulation_section(ui: &mut egui::Ui, registry: &ParamRegistry, m: &mut ModEngine) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Modulation").strong());
        // Beat indicator: brightest on the downbeat, so tempo is visible
        // at a glance rather than inferred from a number.
        let phase = m.clock.bar_phase(4.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        let glow = (1.0 - phase * 4.0).clamp(0.0, 1.0);
        let color = egui::Color32::from_rgb(
            (60.0 + 195.0 * glow) as u8,
            (60.0 + 160.0 * glow) as u8,
            90,
        );
        ui.painter().circle_filled(rect.center(), 5.0, color);
        ui.add(
            egui::DragValue::new(&mut m.clock.bpm)
                .speed(0.5)
                .range(20.0..=300.0)
                .suffix(" bpm"),
        );
        ui.checkbox(&mut m.clock.running, "run");
        if ui.small_button("reset").on_hover_text("restart on the downbeat").clicked() {
            m.clock.reset();
        }
    });

    for (i, lfo) in m.lfos.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("lfo{}", i + 1));
            egui::ComboBox::from_id_salt(format!("shape{i}"))
                .width(64.0)
                .selected_text(lfo.shape.label())
                .show_ui(ui, |ui| {
                    for shape in Shape::ALL {
                        ui.selectable_value(&mut lfo.shape, shape, shape.label());
                    }
                });
            // Beat-synced or free-running, switchable in place: the same
            // LFO is useful both locked to the track and drifting against it.
            let mut synced = matches!(lfo.rate, Rate::Beats(_));
            if ui.checkbox(&mut synced, "sync").changed() {
                lfo.rate = if synced { Rate::Beats(4.0) } else { Rate::Hz(1.0) };
            }
            match &mut lfo.rate {
                Rate::Beats(beats) => {
                    ui.add(
                        egui::DragValue::new(beats)
                            .speed(0.05)
                            .range(0.0625..=32.0)
                            .suffix(" beats"),
                    );
                }
                Rate::Hz(hz) => {
                    ui.add(egui::DragValue::new(hz).speed(0.01).range(0.0..=20.0).suffix(" Hz"));
                }
            }
            // Live output, so it is obvious which LFO is doing what.
            let v = lfo.value();
            let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, egui::Color32::from_black_alpha(120));
            let x = rect.left() + (v * 0.5 + 0.5) * rect.width();
            ui.painter().circle_filled(
                egui::pos2(x, rect.center().y),
                3.0,
                egui::Color32::from_rgb(130, 190, 255),
            );
        });
    }

    let mut remove = None;
    for (i, route) in m.routes.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.checkbox(&mut route.enabled, "");
            ui.label(format!("lfo{} →", route.lfo + 1));
            ui.label(route.param.trim_start_matches('/'));
            ui.add(
                egui::DragValue::new(&mut route.depth)
                    .speed(0.01)
                    .range(-1.0..=1.0)
                    .prefix("×"),
            );
            if ui.small_button("x").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        m.routes.remove(i);
    }
    if m.routes.is_empty() {
        ui.small("no routes — use ‘mod' next to a parameter");
    }
    let _ = registry;
}

fn health_section(ui: &mut egui::Ui, state: &PanelState) {
    let Some(h) = &state.health else {
        ui.label("Collecting health data…");
        return;
    };

    // Colour the headline by whether we are actually holding the budget —
    // this is the number that matters mid-set, readable at a glance.
    let over = h.frame_avg_ms > state.frame_budget_ms;
    let color = if over {
        egui::Color32::from_rgb(255, 120, 90)
    } else {
        egui::Color32::from_rgb(120, 220, 150)
    };
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new(format!("{:.0} fps", h.fps)).color(color));
        ui.label(
            egui::RichText::new(format!("{:.2} ms avg", h.frame_avg_ms))
                .color(color)
                .small(),
        );
    });

    sparkline(ui, &state.frame_times_ms, state.frame_budget_ms);

    egui::Grid::new("health-grid").num_columns(2).show(ui, |ui| {
        ui.label("p95 / p99");
        ui.label(format!("{:.2} / {:.2} ms", h.frame_p95_ms, h.frame_p99_ms));
        ui.end_row();
        ui.label("worst");
        ui.label(format!("{:.2} ms", h.frame_worst_ms));
        ui.end_row();
        ui.label("over budget");
        ui.label(format!("{:.1}% ({} total)", h.over_budget_window_pct, h.over_budget_total));
        ui.end_row();
        ui.label("memory / cpu");
        ui.label(format!(
            "{} · {}",
            h.rss_mib.map(|m| format!("{m:.0} MiB")).unwrap_or_else(|| "n/a".into()),
            h.cpu_pct.map(|c| format!("{c:.0}%")).unwrap_or_else(|| "n/a".into()),
        ));
        ui.end_row();
        ui.label("frames");
        ui.label(format!("{}", h.frames_total));
        ui.end_row();
    });
}

/// Frame-time history with the budget drawn as a reference line, so a
/// spike is visible as shape rather than as a number that already passed.
fn sparkline(ui: &mut egui::Ui, samples: &[f32], budget_ms: f32) {
    let height = 40.0;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(120));

    if samples.is_empty() {
        return;
    }
    // Scale to the budget or the worst sample, whichever is larger, so the
    // budget line stays on screen and spikes are not clipped.
    let peak = samples.iter().copied().fold(budget_ms, f32::max).max(0.001);

    let budget_y = rect.bottom() - (budget_ms / peak) * rect.height();
    painter.line_segment(
        [egui::pos2(rect.left(), budget_y), egui::pos2(rect.right(), budget_y)],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 90, 110)),
    );

    let step = rect.width() / samples.len().max(1) as f32;
    let points: Vec<egui::Pos2> = samples
        .iter()
        .enumerate()
        .map(|(i, &ms)| {
            egui::pos2(
                rect.left() + i as f32 * step,
                rect.bottom() - (ms / peak).clamp(0.0, 1.0) * rect.height(),
            )
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(130, 190, 255)),
    ));
}

fn outputs_section(ui: &mut egui::Ui, state: &PanelState) {
    ui.label(egui::RichText::new("Outputs").strong());
    if state.outputs.is_empty() {
        ui.small("none active — preview only");
        return;
    }
    for out in &state.outputs {
        ui.horizontal(|ui| {
            // Painted rather than a glyph: egui's default font has no
            // U+25CF, so a text bullet renders as a missing-glyph box.
            let color = if out.live {
                egui::Color32::from_rgb(120, 220, 150)
            } else {
                egui::Color32::from_gray(120)
            };
            let size = egui::vec2(10.0, 10.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let center = rect.center();
            if out.live {
                ui.painter().circle_filled(center, 4.0, color);
            } else {
                ui.painter().circle_stroke(center, 4.0, egui::Stroke::new(1.0, color));
            }
            ui.label(&out.name);
        });
    }
}

fn params_section(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    state: &PanelState,
    modulation: &mut ModEngine,
    actions: &mut PanelActions,
) {
    ui.label(egui::RichText::new("Parameters").strong());
    for (id, def) in registry.iter() {
        let mut value = registry.target(id);
        ui.horizontal(|ui| {
            // Strip the leading '/' and show the address as the label, so
            // the panel doubles as live documentation of the OSC surface.
            let label = def.addr.trim_start_matches('/');
            let response = ui.add(
                egui::Slider::new(&mut value, def.min..=def.max)
                    .text(label)
                    .clamping(egui::SliderClamping::Always),
            );
            if response.changed() {
                registry.set(id, value);
            }
            // Right-click restores the default: the fastest way out of a
            // mess mid-set, and it costs nothing to support.
            if response.secondary_clicked() {
                registry.set(id, def.default);
            }

            // Route the first LFO to this parameter as a starting point;
            // which LFO and how deep are then adjustable above.
            if ui.small_button("mod").on_hover_text("route lfo1 to this parameter").clicked() {
                modulation.add_route(0, def.addr.clone(), 0.25);
            }

            if !state.midi.available {
                return;
            }
            let learning = state.midi.learn_target.as_deref() == Some(def.addr.as_str());
            match state.midi.map.source_for(&def.addr) {
                Some(source) => {
                    if ui
                        .small_button(source.label())
                        .on_hover_text("click to clear this MIDI binding")
                        .clicked()
                    {
                        actions.clear_binding = Some(def.addr.clone());
                    }
                }
                None if learning => {
                    if ui.small_button("cancel").clicked() {
                        actions.set_learn_target = Some(None);
                    }
                }
                None => {
                    if ui.small_button("learn").clicked() {
                        actions.set_learn_target = Some(Some(def.addr.clone()));
                    }
                }
            }
        });
    }
    ui.small("right-click a slider to reset it · learn binds the next control you move");
}

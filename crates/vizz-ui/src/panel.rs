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
    /// Audio settings the user changed this frame.
    pub audio: AudioEdits,
    /// Recall this preset by name.
    pub preset_load: Option<String>,
    /// Capture the current parameters under this name.
    pub preset_save: Option<String>,
    /// Delete this user preset.
    pub preset_delete: Option<String>,
}

/// One entry in the preset list.
#[derive(Debug, Clone)]
pub struct PresetEntry {
    pub name: String,
    /// Built-ins are read-only, so they get no delete button.
    pub builtin: bool,
    /// One-line description, built-ins only.
    pub about: Option<String>,
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
    pub audio: AudioView,
    /// Current analysis settings, mirrored here so the widgets have
    /// something to edit without locking the analysis thread while drawing.
    pub audio_bands: [vizz_audio::Band; 4],
    pub audio_auto_bpm: bool,
    /// Beat clock, mirrored for the performance layout (which does not get
    /// a mutable ModEngine).
    pub bpm: f32,
    pub bar_phase: f32,
    /// Built-ins first, then user presets, matching `/preset/recall` slots.
    pub presets: Vec<PresetEntry>,
    /// The `/` shortcut was pressed this frame; focus the parameter filter.
    pub focus_filter: bool,
    /// Draw every collapsible section open.
    ///
    /// For offscreen rendering — tests and the preview example — where
    /// there is nobody to click a header, and asserting on content that
    /// is one click away is still asserting on content that exists.
    pub expand_sections: bool,
}

/// What the panel needs to know about audio this frame. A snapshot rather
/// than a live handle, so drawing never touches the analysis thread.
#[derive(Debug, Clone, Default)]
pub struct AudioView {
    pub connected: bool,
    pub device: Option<String>,
    /// Post-gain envelopes, 0..1 — what modulation actually sees.
    pub bands: [f32; 4],
    /// Pre-gain levels, for setting the gain against real material.
    pub raw: [f32; 4],
    pub level: f32,
    pub detected_bpm: f32,
    pub confidence: f32,
    pub dropped: usize,
}

/// Edits the panel wants applied to the audio settings, collected here
/// rather than written directly because the settings live behind a mutex
/// shared with the analysis thread.
#[derive(Debug, Clone, Default)]
pub struct AudioEdits {
    pub bands: Option<[vizz_audio::Band; 4]>,
    pub auto_bpm: Option<bool>,
    /// The user tapped tempo; the caller resolves it to a BPM.
    pub tapped: bool,
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
            // One line of everything you need to glance at mid-set: is it
            // keeping up, is it going out, is audio arriving, what tempo.
            // The detail behind each is setup, not performance, so it
            // folds away — before this the status blocks filled the panel
            // and left the parameter list three rows tall.
            status_strip(ui, state, &mut actions);
            ui.separator();
            egui::CollapsingHeader::new("health")
                .id_salt("health")
                .default_open(state.expand_sections)
                .show(ui, |ui| health_section(ui, state));
            egui::CollapsingHeader::new("outputs")
                .id_salt("outputs")
                .default_open(state.expand_sections)
                .show(ui, |ui| outputs_section(ui, state));
            egui::CollapsingHeader::new("midi")
                .id_salt("midi")
                .default_open(state.expand_sections)
                .show(ui, |ui| midi_section(ui, state));
            egui::CollapsingHeader::new("audio")
                .id_salt("audio")
                .default_open(state.expand_sections)
                .show(ui, |ui| audio_section(ui, state, &mut actions));
            egui::CollapsingHeader::new("modulation")
                .id_salt("modulation")
                .default_open(state.expand_sections)
                .show(ui, |ui| modulation_section(ui, registry, modulation));
            ui.separator();
            presets_section(ui, state, &mut actions);
            ui.separator();
            params_section(ui, registry, state, modulation, &mut actions);
            ui.separator();
            ui.small("Tab panel · G modulation · P performance · Esc quits");
        });
    actions
}

/// The always-visible line: health, outputs, audio, tempo.
///
/// Everything here is something you would want to see without opening
/// anything, mid-set, without looking away from the output for long. Tap
/// tempo lives here rather than in the audio settings for the same reason
/// — it is a thing you do while playing, not while setting up.
fn status_strip(ui: &mut egui::Ui, state: &PanelState, actions: &mut PanelActions) {
    ui.horizontal_wrapped(|ui| {
        if let Some(h) = &state.health {
            let over = h.over_budget_window_pct > 1.0;
            ui.colored_label(
                if over { WARN } else { GOOD },
                egui::RichText::new(format!("{:.0} fps", h.fps)).strong(),
            )
            .on_hover_text(format!(
                "frame avg {:.2} ms · p95 {:.2} ms · over budget {:.1}%",
                h.frame_avg_ms, h.frame_p95_ms, h.over_budget_window_pct
            ));
        }
        for out in &state.outputs {
            dot(ui, out.live, if out.live { GOOD } else { WARN }).on_hover_text(&out.name);
            ui.small(&out.name);
        }
        let audio = &state.audio;
        dot(ui, audio.connected, if audio.connected { GOOD } else { WARN })
            .on_hover_text(if audio.connected { "audio input" } else { "no audio input" });
        ui.small(audio.device.as_deref().unwrap_or("no audio"));
        ui.small(format!("{:.1} bpm", state.bpm));
        if ui.small_button("tap").on_hover_text("tap the beat to set the tempo").clicked() {
            actions.audio.tapped = true;
        }
    });
}

/// A status dot, painted rather than written.
///
/// egui's default font has no U+25CF, so a text bullet renders as a
/// missing-glyph box — which is exactly what happened the first time the
/// status strip was written, despite the comment in `outputs_section`
/// saying so. Filled means live, hollow means not.
fn dot(ui: &mut egui::Ui, live: bool, color: egui::Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    if live {
        ui.painter().circle_filled(rect.center(), 4.0, color);
    } else {
        ui.painter()
            .circle_stroke(rect.center(), 4.0, egui::Stroke::new(1.0, color));
    }
    response
}

const GOOD: egui::Color32 = egui::Color32::from_rgb(120, 220, 160);
const WARN: egui::Color32 = egui::Color32::from_rgb(255, 190, 90);

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
/// Two stacked bars: what modulation receives on top, what is arriving at
/// the input underneath. Stacked rather than overlaid because with any
/// gain above 1 the envelope covers the raw signal completely, and the
/// whole point of the meter is comparing the two to set the gain.
fn meter(ui: &mut egui::Ui, raw: f32, env: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(80.0, 12.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(140));
    let h = rect.height() * 0.5;
    p.rect_filled(
        egui::Rect::from_min_size(
            rect.left_top(),
            egui::vec2(rect.width() * env.clamp(0.0, 1.0), h),
        ),
        1.0,
        egui::Color32::from_rgb(120, 200, 255),
    );
    p.rect_filled(
        egui::Rect::from_min_size(
            rect.left_top() + egui::vec2(0.0, h),
            egui::vec2(rect.width() * raw.clamp(0.0, 1.0), h),
        ),
        1.0,
        egui::Color32::from_rgb(70, 95, 125),
    );
    // Clipping marker: at 1.0 the band is pinned and the gain is too high.
    if env >= 0.999 {
        p.rect_filled(
            egui::Rect::from_min_size(
                rect.right_top() - egui::vec2(3.0, 0.0),
                egui::vec2(3.0, rect.height()),
            ),
            0.0,
            egui::Color32::from_rgb(255, 120, 90),
        );
    }
}

fn audio_section(ui: &mut egui::Ui, state: &PanelState, actions: &mut PanelActions) {
    let a = &state.audio;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Audio").strong());
        let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().circle_filled(
            dot.center(),
            4.0,
            if a.connected {
                egui::Color32::from_rgb(90, 200, 120)
            } else {
                egui::Color32::from_rgb(110, 110, 110)
            },
        );
        match &a.device {
            Some(d) => ui.small(d.as_str()),
            None => ui.small("no input"),
        };
    });

    if !a.connected {
        ui.small("Start with --audio-device, or --list-audio to see names.");
        return;
    }

    let mut bands = state.audio_bands;
    for (i, band) in bands.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            meter(ui, a.raw[i], a.bands[i]);
            ui.add(
                egui::DragValue::new(&mut band.lo_hz)
                    .speed(2.0)
                    .range(20.0..=18_000.0)
                    .suffix("Hz"),
            );
            ui.add(
                egui::DragValue::new(&mut band.hi_hz)
                    .speed(2.0)
                    .range(20.0..=20_000.0)
                    .suffix("Hz"),
            );
            ui.add(
                egui::DragValue::new(&mut band.gain)
                    .speed(0.1)
                    .range(0.1..=60.0)
                    .prefix("×"),
            );
        });
    }
    // A band whose high edge is under its low edge would silently read
    // zero; clamp on edit rather than letting a drag produce a dead band.
    for b in &mut bands {
        b.hi_hz = b.hi_hz.max(b.lo_hz + 10.0);
    }
    if bands != state.audio_bands {
        actions.audio.bands = Some(bands);
    }

    ui.horizontal(|ui| {
        ui.small(format!(
            "detected {:.1} bpm ({:.0}% sure)",
            a.detected_bpm,
            a.confidence * 100.0
        ));
        let mut auto = state.audio_auto_bpm;
        if ui
            .checkbox(&mut auto, "auto")
            .on_hover_text("let detected tempo drive the beat clock")
            .changed()
        {
            actions.audio.auto_bpm = Some(auto);
        }
        if ui.small_button("tap").on_hover_text("tap tempo: three taps sets it").clicked() {
            actions.audio.tapped = true;
        }
    });
    if a.dropped > 0 {
        ui.small(format!("{} samples dropped", a.dropped));
    }
}

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
            ui.label(format!("{} ->", route.source.label()));
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
            dot(ui, out.live, if out.live { GOOD } else { egui::Color32::from_gray(120) });
            ui.label(&out.name);
        });
    }
}

/// Presets: recall a whole look, and store your own.
///
/// The list is deliberately above the parameter list rather than buried
/// below it. Recalling a look is something you do mid-set with one hand;
/// scrolling to find it is not.
fn presets_section(ui: &mut egui::Ui, state: &PanelState, actions: &mut PanelActions) {
    ui.label(egui::RichText::new("Presets").strong());
    egui::ScrollArea::vertical()
        .id_salt("presets")
        .max_height(PRESET_LIST_H)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (i, p) in state.presets.iter().enumerate() {
                ui.horizontal(|ui| {
                    // The slot number, because it is what `/preset/recall`
                    // and therefore a MIDI button addresses. Showing it
                    // saves counting rows to work out what to bind.
                    ui.small(format!("{:>2}", i + 1));
                    let mut button = ui.button(&p.name);
                    if let Some(about) = &p.about {
                        button = button.on_hover_text(about);
                    }
                    if button.clicked() {
                        actions.preset_load = Some(p.name.clone());
                    }
                    if p.builtin {
                        return;
                    }
                    if ui
                        .small_button("x")
                        .on_hover_text("delete this preset")
                        .clicked()
                    {
                        actions.preset_delete = Some(p.name.clone());
                    }
                });
            }
        });

    ui.horizontal(|ui| {
        let id = egui::Id::new("preset-save-name");
        let mut name: String = ui.memory_mut(|m| m.data.get_temp(id).unwrap_or_default());
        let editing = ui.add(
            egui::TextEdit::singleline(&mut name)
                .hint_text("name")
                .desired_width(140.0),
        );
        // Enter saves, so the whole thing is type-and-go rather than
        // type-then-aim.
        let entered = editing.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let clicked = ui.button("save").on_hover_text("store the current look").clicked();
        if (entered || clicked) && !name.trim().is_empty() {
            actions.preset_save = Some(name.clone());
            name.clear();
        }
        ui.memory_mut(|m| m.data.insert_temp(id, name));
    });
    ui.small("saving overwrites a preset of the same name; built-ins cannot be replaced");
}

/// Room for a handful of presets before the list scrolls. Smaller than the
/// parameter list: presets are chosen, not scanned.
const PRESET_LIST_H: f32 = 112.0;

fn params_section(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    state: &PanelState,
    modulation: &mut ModEngine,
    actions: &mut PanelActions,
) {
    // The one section that grows without bound — it gained the whole
    // camera and room set in two releases. An egui window sizes to its
    // content, so left unscrolled the list runs off the bottom of the
    // display and everything past roughly /fx/spin becomes unreachable:
    // a control you cannot scroll to is a control you do not have.
    //
    // Take whatever the sections above left on screen, so the panel as a
    // whole fits rather than the list merely being bounded — a window
    // that runs past the bottom edge hides its own footer too.
    let screen_h = ui.ctx().input(|i| i.raw.screen_rect).map_or(720.0, |r| r.height());
    let total = registry.iter().count();
    // Provisional, only to decide whether to say "scroll for more"; the
    // binding measurement happens below, once the header has been laid
    // out and the cursor is where the list will actually start.
    let scrolls = total as f32 * PARAM_ROW_H > screen_h - ui.cursor().top();

    // Filter first, because with nine groups the fastest route to a known
    // parameter is typing its name, not opening headers until you find it.
    let filter_id = egui::Id::new("param-filter");
    let mut filter: String = ui.memory_mut(|m| m.data.get_temp(filter_id).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Parameters").strong());
        let response = ui.add(
            egui::TextEdit::singleline(&mut filter)
                .hint_text("filter")
                .desired_width(96.0),
        );
        // `/` focuses it, the way it does in every other list worth
        // searching. Guarded on the field not already having focus, or
        // typing a slash into it would re-grab and swallow the character.
        if state.focus_filter && !response.has_focus() {
            response.request_focus();
        }
        if !filter.is_empty() && ui.small_button("x").clicked() {
            filter.clear();
        }
        if scrolls && filter.is_empty() {
            ui.small(format!("{total}"));
        }
    });
    let needle = filter.trim().to_ascii_lowercase();
    ui.memory_mut(|m| m.data.insert_temp(filter_id, filter));

    let height = (screen_h - ui.cursor().top() - PARAM_LIST_MARGIN)
        .clamp(PARAM_LIST_MIN, PARAM_LIST_MAX);
    egui::ScrollArea::vertical()
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if !needle.is_empty() {
                // Filtering flattens: groups are for browsing, and when
                // you have typed a name you already know what you want.
                let mut hits = 0;
                for (id, def) in registry.iter() {
                    if def.addr.to_ascii_lowercase().contains(&needle) {
                        hits += 1;
                        param_row(ui, registry, id, def, state, modulation, actions);
                    }
                }
                if hits == 0 {
                    ui.small(format!("nothing matches {needle:?}"));
                }
                return;
            }
            // Grouped by the address's first segment, in registry order —
            // which is the order they were authored in, and reads better
            // than alphabetical. A flat list of thirty-seven means
            // scrolling past everything to reach one control.
            // All open by default. Collapsing is the user's call — and
            // closing any group by default hides whatever is inside it,
            // which for `master` would mean hiding the panic fader.
            for group in groups(registry) {
                egui::CollapsingHeader::new(group.label(modulation, registry))
                    .id_salt(group.name)
                    .default_open(true)
                    .show(ui, |ui| {
                        for (id, def) in group.params {
                            param_row(ui, registry, id, def, state, modulation, actions);
                        }
                    });
            }
        });
    ui.small("right-click a slider to reset it · / filters · learn binds the next control you move");
}

/// One address-prefix group, e.g. everything under `/room/`.
struct Group<'a> {
    name: &'a str,
    params: Vec<(vizz_params::ParamId, &'a vizz_params::ParamDef)>,
}

impl Group<'_> {
    /// Header text: the group and how many of its parameters are being
    /// modulated, so a collapsed group still says whether something inside
    /// it is moving on its own.
    fn label(&self, modulation: &ModEngine, _reg: &ParamRegistry) -> String {
        let moving = self
            .params
            .iter()
            .filter(|(_, d)| modulation.drives(&d.addr))
            .count();
        if moving > 0 {
            format!("{}  ({} · {moving}~)", self.name, self.params.len())
        } else {
            format!("{}  ({})", self.name, self.params.len())
        }
    }
}

/// Split the registry by the first path segment, preserving registry order
/// both within and between groups.
fn groups(registry: &ParamRegistry) -> Vec<Group<'_>> {
    let mut out: Vec<Group<'_>> = Vec::new();
    for (id, def) in registry.iter() {
        let name = def
            .addr
            .trim_start_matches('/')
            .split('/')
            .next()
            .unwrap_or("other");
        match out.iter_mut().find(|g| g.name == name) {
            Some(g) => g.params.push((id, def)),
            None => out.push(Group { name, params: vec![(id, def)] }),
        }
    }
    out
}

/// Approximate row height, for deciding whether the list will be cut.
/// Only drives a label, so being a pixel or two out costs nothing.
const PARAM_ROW_H: f32 = 21.0;

/// Room left below the list for the hint line and the window's own edge.
const PARAM_LIST_MARGIN: f32 = 46.0;
/// Never shrink below about five rows: past that the list is unusable and
/// it is better to let the panel overflow than to hide everything.
const PARAM_LIST_MIN: f32 = 92.0;
/// Nor grow past this — a long list is easier to scan in a fixed frame
/// than one that changes height with the display.
const PARAM_LIST_MAX: f32 = 320.0;

fn param_row(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    id: vizz_params::ParamId,
    def: &vizz_params::ParamDef,
    state: &PanelState,
    modulation: &mut ModEngine,
    actions: &mut PanelActions,
) {
    let mut value = registry.target(id);
    ui.horizontal(|ui| {
        // Inside a group the prefix is redundant and eats the width the
        // slider needs; filtered results are flat, so they keep it.
        let label = def.addr.trim_start_matches('/');
        let short = label.split_once('/').map_or(label, |(_, rest)| rest);

        let driven = modulation.drives(&def.addr);
        if driven {
            // A slider that will not stay where you put it is otherwise
            // indistinguishable from a broken one. The value is still
            // yours — modulation rides on top as an offset.
            let offset = modulation.offset_for(registry, &def.addr);
            ui.colored_label(MOD_COLOR, "~").on_hover_text(format!(
                "modulated, currently {offset:+.2} of range"
            ));
        }

        // A stepped parameter shows its position's name rather than a
        // number: `mode 5.000` says nothing, `mode Lorenz` says what is
        // on screen.
        let mut slider = egui::Slider::new(&mut value, def.min..=def.max)
            .text(short)
            .clamping(egui::SliderClamping::Always);
        if def.labels.is_some() {
            let labels = def.labels;
            slider = slider.custom_formatter(move |v, _| {
                labels
                    .and_then(|l| l.get(v.round().max(0.0) as usize))
                    .map(|s| (*s).to_string())
                    .unwrap_or_else(|| format!("{v:.0}"))
            });
        }
        let response = ui.add(slider);
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
            modulation.add_route(vizz_mod::Source::Lfo(0), def.addr.clone(), 0.25);
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

/// Marks a modulated parameter. Warm against the panel's blues so it reads
/// as "something else is touching this" at a glance.
const MOD_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 190, 90);

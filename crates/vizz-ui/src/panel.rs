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
    /// Slider working ranges changed and should be persisted.
    pub ranges_changed: bool,
    /// What the scene grid asks for this frame.
    pub grid: crate::grid_view::GridActions,
    /// Output size, render scale and master precision, when changed.
    pub output_setup: Option<OutputSetup>,
    /// What the gravity grid asks for this frame.
    pub gravity: crate::grid_view::GridActions,
}

/// How big the output is and how hard it is worked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputSetup {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub wide: bool,
}

impl Default for OutputSetup {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            scale: 1.0,
            wide: false,
        }
    }
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
    /// Live smoothed values including modulation, indexed by parameter
    /// position. Owned rather than borrowed so this struct stays free of
    /// lifetimes; it is one float per parameter, rebuilt each frame.
    ///
    /// Empty when nothing is reporting, in which case controls fall back
    /// to drawing only what the user set.
    pub modulated: Vec<f32>,
    /// Name of the cloud in each slot, in slot order. Lets the panel say
    /// what `/cloud/a` and `/cloud/b` are actually selecting — the numbers
    /// alone are meaningless once anything has been loaded.
    pub clouds: Vec<String>,
    /// Current output size, render scale and precision.
    pub output: OutputSetup,
    /// Palette name per row, so `/color/palette` can be read as colours
    /// rather than as a number. Empty entries are unused rows.
    pub palettes: Vec<String>,
    /// Beat clock, mirrored for the performance layout (which does not get
    /// a mutable ModEngine).
    pub bpm: f32,
    pub bar_phase: f32,
    /// Built-ins first, then user presets, matching `/preset/recall` slots.
    pub presets: Vec<PresetEntry>,
    /// The scene grid as it stands this frame.
    pub grid: crate::grid_view::GridView,
    /// The gravity grid, when the layer is in use. `None` hides it from
    /// the performance layout entirely.
    pub gravity_grid: Option<crate::grid_view::GridView>,
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
    /// Decaying peak of `raw`, which is what `fit` divides into.
    pub raw_peak: [f32; 4],
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
    /// Switch to this input device. `Some(None)` means the system
    /// default — distinct from `None`, which means "unchanged".
    pub device: Option<Option<String>>,
}

pub fn draw(
    ctx: &egui::Context,
    registry: &ParamRegistry,
    state: &PanelState,
    modulation: &mut ModEngine,
    ranges: &mut vizz_mod::ranges::Ranges,
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
                .show(ui, |ui| {
                    outputs_section(ui, state);
                    ui.separator();
                    output_setup_section(ui, state, &mut actions);
                });
            egui::CollapsingHeader::new("clouds")
                .id_salt("clouds")
                .default_open(state.expand_sections)
                .show(ui, |ui| {
                    clouds_section(ui, state);
                    ui.separator();
                    palettes_section(ui, state);
                });
            egui::CollapsingHeader::new("background")
                .id_salt("background")
                .default_open(state.expand_sections)
                .show(ui, |ui| background_section(ui, registry));
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
            // No scene grid here any more.
            //
            // The two screens have distinct jobs: this one is for building
            // a look, the performance layout is for playing looks in an
            // order. The grid belongs entirely to the second, and a
            // four-by-four copy of it here was both a duplicate control
            // and — because it read as a row of sliders among the visual
            // parameters — a thing people mistook for part of the look.
            // The sixteen-across row on the performance layout is the one.
            presets_section(ui, state, &mut actions);
            ui.separator();
            params_section(ui, registry, state, modulation, ranges, &mut actions);
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
                // Monospace and padded: a proportional font makes "60"
                // narrower than "137", so the whole status line reflowed
                // every time the frame rate crossed 100. Padding alone
                // does not fix it — a space is narrower than a digit.
                egui::RichText::new(format!("{:>3.0} fps", h.fps)).strong().monospace(),
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
        // Same reason: 99.5 and 128.0 are different widths otherwise.
        ui.small(egui::RichText::new(format!("{:>5.1} bpm", state.bpm)).monospace());
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

/// What is in each cloud slot, and how to put something there.
///
/// `/cloud/a` and `/cloud/b` are indices, and an index is unreadable once
/// anything has been loaded — "2" says nothing about whether that is the
/// torso scan or the room. This is the legend for those two sliders.
///
/// The drop hint is here rather than nowhere because a gesture with no
/// visible affordance is a gesture nobody discovers. That was the whole
/// lesson of the rename living only on a right-click menu.
fn clouds_section(ui: &mut egui::Ui, state: &PanelState) {
    for (i, name) in state.clouds.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.small(format!("{i}"));
            ui.label(name);
        });
    }
    if state.clouds.is_empty() {
        ui.small("no cloud slots");
    }
    ui.small("drag a .ply, .xyz, .csv or .pts onto the window to load one");
}

/// The colour ramps, by the index `/color/palette` uses.
///
/// Same reason as the cloud list: the parameter is a number, and past the
/// four shipped names a number says nothing at all about what is in the
/// slot. Unused rows are left out rather than listed as blanks — sixteen
/// entries of which twelve are empty is a worse legend than four.
fn palettes_section(ui: &mut egui::Ui, state: &PanelState) {
    for (i, name) in state.palettes.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        ui.horizontal(|ui| {
            ui.small(format!("{i}"));
            ui.label(name);
        });
    }
    ui.small("drag a .gpl or a list of hex colours onto the window to add one");
}

/// The background colour, and whether there is one at all.
///
/// A swatch rather than three sliders, because nobody picks a colour by
/// typing red, green and blue — those live in the parameter list for OSC
/// and MIDI, and this is how a human chooses.
///
/// Alpha is separate and labelled, because it is not a colour decision. At
/// zero the field is delivered on nothing, which is what makes vizz a
/// layer in Resolume or VDMX rather than a whole picture, and that is
/// worth saying out loud rather than leaving as the left end of a slider.
fn background_section(ui: &mut egui::Ui, registry: &ParamRegistry) {
    let (Some(r), Some(g), Some(b), Some(a)) = (
        registry.id("/bg/red"),
        registry.id("/bg/green"),
        registry.id("/bg/blue"),
        registry.id("/bg/alpha"),
    ) else {
        return;
    };

    ui.horizontal(|ui| {
        // egui's picker works in 0..255 gamma space; the parameters are
        // linear 0..1, which is what the clear colour wants.
        let mut rgb = [registry.target(r), registry.target(g), registry.target(b)];
        if ui.color_edit_button_rgb(&mut rgb).changed() {
            registry.set(r, rgb[0]);
            registry.set(g, rgb[1]);
            registry.set(b, rgb[2]);
        }
        ui.label("colour");
        if ui
            .small_button("black")
            .on_hover_text("a true black background")
            .clicked()
        {
            registry.set(r, 0.0);
            registry.set(g, 0.0);
            registry.set(b, 0.0);
        }
    });

    ui.horizontal(|ui| {
        let mut alpha = registry.target(a);
        if ui
            .add(
                egui::Slider::new(&mut alpha, 0.0..=1.0)
                    .text("opacity")
                    .clamping(egui::SliderClamping::Always),
            )
            .on_hover_text("0 sends the field on a transparent background")
            .changed()
        {
            registry.set(a, alpha);
        }
    });
    // Say which state you are in rather than making it inferred from a
    // slider position — "why is my key not working" is the question this
    // line exists to answer.
    if registry.target(a) <= 0.001 {
        ui.small("transparent — receivers get the field with an alpha channel");
    } else if registry.target(a) < 0.999 {
        ui.small("partly transparent");
    } else {
        ui.small("opaque");
    }
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

/// Choose the input device.
///
/// This used to be a command-line flag only, which meant picking your
/// interface required quitting, remembering the flag and restarting — at a
/// venue, with the wrong input already on screen. The list is read when
/// the menu is opened rather than every frame: enumerating devices talks
/// to CoreAudio/WASAPI and is far too expensive to do at 60 Hz.
fn device_picker(ui: &mut egui::Ui, state: &PanelState, actions: &mut PanelActions) {
    let current = state.audio.device.as_deref().unwrap_or("no input");
    ui.horizontal(|ui| {
        ui.label("input");
        egui::ComboBox::from_id_salt("audio-device")
            .selected_text(current)
            .width(220.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(state.audio.device.is_none(), "system default")
                    .clicked()
                {
                    actions.audio.device = Some(None);
                }
                ui.separator();
                for name in vizz_audio::input_devices() {
                    let selected = state.audio.device.as_deref() == Some(name.as_str());
                    if ui.selectable_label(selected, &name).clicked() {
                        actions.audio.device = Some(Some(name.clone()));
                    }
                }
            });
        // A device that has gone away should not look like a live one.
        if !state.audio.connected {
            ui.label(
                egui::RichText::new("not capturing").color(egui::Color32::from_rgb(240, 150, 90)),
            );
        }
    });
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
    });

    device_picker(ui, state, actions);

    if !a.connected {
        ui.small("pick an input above, or start with --audio-device");
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
            // Decibels, not a multiplier. "×10" is not a quantity anyone
            // can act on — it does not say whether the band is hot or
            // quiet, and it is not comparable with the number in the row
            // above unless you do the arithmetic. Decibels are the unit
            // every other gain control in a studio is read in, and they
            // make the four rows comparable at a glance.
            let mut db = band.gain_db();
            if ui
                .add(
                    egui::DragValue::new(&mut db)
                        .speed(0.5)
                        .range(vizz_audio::MIN_GAIN_DB..=vizz_audio::MAX_GAIN_DB)
                        .fixed_decimals(1)
                        .suffix(" dB"),
                )
                .on_hover_text("sensitivity — how hard this band drives modulation")
                .changed()
            {
                band.set_gain_db(db);
            }
        });
    }
    // One press, and every band is scaled to what is actually arriving.
    //
    // This is the honest answer to "what should the default gain be": it
    // depends on the interface, the track and how hard it is being driven,
    // and no shipped number is right for two rigs. A default can only be a
    // starting point; this is the thing that finishes the job.
    ui.horizontal(|ui| {
        if ui
            .button("fit")
            .on_hover_text("set every band's gain from the last few seconds of audio")
            .clicked()
        {
            let mut fitted = bands;
            for (i, band) in fitted.iter_mut().enumerate() {
                // A silent band keeps whatever it had: dividing into
                // nothing would ask for infinite gain, and a band nobody
                // is feeding is not evidence of anything.
                if let Some(db) = vizz_audio::fit_gain_db(a.raw_peak[i]) {
                    band.set_gain_db(db);
                }
            }
            bands = fitted;
        }
        if ui
            .button("reset")
            .on_hover_text("back to the shipped bands and gains")
            .clicked()
        {
            bands = vizz_audio::default_bands();
        }
        ui.small("play something first — fit reads the last few seconds");
    });
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

/// Output size, render scale, and how many bits the master carries.
///
/// The two sizes are genuinely different things and were conflated into
/// one before: what receivers get, and how hard the renderer works to
/// produce it. Above 1× the extra pixels are thrown away by the downscale
/// into the master, which is exactly the point — that averaging is the
/// only thing that reliably cleans up a field of one-pixel sprites, and it
/// costs fill rate rather than complexity.
fn output_setup_section(ui: &mut egui::Ui, state: &PanelState, actions: &mut PanelActions) {
    let mut next = state.output;

    ui.horizontal(|ui| {
        ui.label("output");
        ui.add(
            egui::DragValue::new(&mut next.width)
                .range(160..=7680)
                .speed(8.0),
        );
        ui.label("x");
        ui.add(
            egui::DragValue::new(&mut next.height)
                .range(160..=7680)
                .speed(8.0),
        );
    });
    // The sizes people actually output at, because typing 3840 by dragging
    // a spinner is nobody's idea of a control.
    ui.horizontal(|ui| {
        for (label, w, h) in [
            ("720p", 1280, 720),
            ("1080p", 1920, 1080),
            ("1440p", 2560, 1440),
            ("4K", 3840, 2160),
        ] {
            if ui.small_button(label).clicked() {
                next.width = w;
                next.height = h;
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("render");
        ui.add(
            egui::Slider::new(&mut next.scale, 0.25..=2.0)
                .suffix("x")
                .clamping(egui::SliderClamping::Always),
        )
        .on_hover_text("above 1 supersamples: draw larger, let the downscale anti-alias");
    });
    // Say the resulting size out loud. A multiplier is easy to set and
    // hard to picture, and the number that matters for whether the machine
    // will hold 60 fps is the pixel count, not the factor.
    let rw = (next.width as f32 * next.scale) as u32;
    let rh = (next.height as f32 * next.scale) as u32;
    ui.small(format!("drawing {rw} x {rh}"));

    ui.checkbox(&mut next.wide, "16-bit float master")
        .on_hover_text("smoother gradients, at double the master's bandwidth");
    if next.wide {
        // Not a warning about something broken — a statement of what it
        // costs. Syphon and NDI are BGRA8 by definition, so this cannot
        // reach them without a conversion, and pretending otherwise would
        // be discovered as a black frame at a venue.
        ui.small("Syphon and NDI still receive 8-bit; a conversion pass is added for them");
    }

    if next != state.output {
        actions.output_setup = Some(next);
    }
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

/// The preset library: where looks are made, kept and thrown away.
///
/// This is not the same control as the preset row on the performance
/// layout, even though both list the same names. That one *fires* a look
/// with one press during a set. This one is the library behind it —
/// saving what is on screen, opening a saved look to keep working on it,
/// and deleting the ones that did not survive the night. Those are design
/// activities, and this is the design screen; firing lives next to the
/// grid that sequences them.
///
/// So the list stays here even though a version of it exists there: it is
/// the only place a preset can be created or removed, and creating them is
/// the entire purpose of this screen.
fn presets_section(ui: &mut egui::Ui, state: &PanelState, actions: &mut PanelActions) {
    ui.label(egui::RichText::new("Presets").strong());
    ui.small("click to open a look and keep editing it");
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

#[allow(clippy::too_many_arguments)]
fn params_section(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    state: &PanelState,
    modulation: &mut ModEngine,
    ranges: &mut vizz_mod::ranges::Ranges,
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
    let total = registry.iter().filter(|(_, d)| !is_transport(&d.addr)).count();
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
                //
                // Transport parameters *are* searchable here, unlike in the
                // grouped list. Hiding them from the groups is about not
                // putting them in misleading company; refusing to show one
                // whose name has just been typed would be hiding it, which
                // is a different and worse thing.
                let mut hits = 0;
                for (id, def) in registry.iter() {
                    if def.addr.to_ascii_lowercase().contains(&needle) {
                        hits += 1;
                        param_row(ui, registry, id, def, state, modulation, ranges, actions);
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
                let name = group.name;
                egui::CollapsingHeader::new(group.label(modulation, registry))
                    .id_salt(group.name)
                    .default_open(true)
                    .show(ui, |ui| {
                        for (id, def) in group.params {
                            param_row(ui, registry, id, def, state, modulation, ranges, actions);
                        }
                        if name == "camera" {
                            camera_buttons(ui, registry);
                        }
                        if name == "room" {
                            room_buttons(ui, registry);
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
/// Parameters that are transport, not look.
///
/// These drive *when* and *whether* something happens rather than what it
/// looks like: which scene to fire, how long a blend takes, which preset
/// to recall. They are real parameters — addressable over OSC, assignable
/// to a fader, learnable — and they have proper controls on the
/// performance layout, next to the grid they belong to.
///
/// They are hidden from this list because being here actively misled: a
/// row called `time` or `bars` sitting among `size`, `morph` and `twist`
/// reads as something that changes the picture, and people reasonably
/// took them for point-cloud settings. A parameter in the wrong company
/// is worse than a parameter you have to go one screen to find.
fn is_transport(addr: &str) -> bool {
    addr.starts_with("/scene/") || addr == "/preset/recall"
}

fn groups(registry: &ParamRegistry) -> Vec<Group<'_>> {
    let mut out: Vec<Group<'_>> = Vec::new();
    for (id, def) in registry.iter() {
        if is_transport(&def.addr) {
            continue;
        }
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

/// Getting back to a known camera.
///
/// Two buttons rather than one, because they answer different questions.
/// After pushing the subject off frame you want the framing back without
/// losing a distance and lens you spent time on — that is `centre`.
/// After an hour of experimenting you want the camera you started with —
/// that is `reset`. Collapsing them into a single button would make the
/// cheap, common recovery destroy work every time it was used.
///
/// Both write parameter targets, so the move is smoothed like any other
/// and rides through OSC and recording rather than teleporting.
fn camera_buttons(ui: &mut egui::Ui, registry: &ParamRegistry) {
    ui.horizontal(|ui| {
        if ui
            .button("centre")
            .on_hover_text("bring the subject back to the middle, keeping distance and lens")
            .clicked()
        {
            for addr in ["/camera/pan_x", "/camera/pan_y"] {
                if let Some(id) = registry.id(addr) {
                    registry.set(id, 0.0);
                }
            }
        }
        if ui
            .button("reset")
            .on_hover_text("every camera control back to its default")
            .clicked()
        {
            for (id, def) in registry.iter() {
                if def.addr.starts_with("/camera/") {
                    registry.set(id, def.default);
                }
            }
        }
    });
}

/// The room's canonical setup: the screen *is* the front face.
///
/// The room's half-extents are already derived from the camera's frustum,
/// so its opening tracks the output aspect automatically — change the
/// output size and the front face still lands exactly on the frame edge,
/// with no numbers to redial.
///
/// What that derivation cannot do is fix where you are standing. The
/// illusion is that the frame edge is the opening, and it only reads that
/// way square-on: orbit or pan away and you are looking at a box from an
/// angle, which is a different and perfectly good look but not this one.
/// So the button neutralises the camera as well as setting the room —
/// setting half of it and leaving the illusion broken would make the
/// button look like it does not work.
fn room_buttons(ui: &mut egui::Ui, registry: &ParamRegistry) {
    if !ui
        .button("screen is the front face")
        .on_hover_text(
            "square the camera up and open the room to exactly the frame — \
             follows the output size on its own",
        )
        .clicked()
    {
        return;
    }
    for (addr, value) in [
        // Square-on: the only orientation where the frame edge and the
        // opening coincide.
        ("/camera/orbit", 0.0),
        ("/camera/elevation", 0.0),
        ("/camera/pan_x", 0.0),
        ("/camera/pan_y", 0.0),
        // Visible, and a box rather than a tunnel.
        ("/room/brightness", 0.7),
        ("/room/converge", 0.35),
        // A vanishing point off-centre is the other way to break the
        // window reading.
        ("/room/vanish_x", 0.0),
        ("/room/vanish_y", 0.0),
        // Cloud inside the room rather than pressed against its face.
        ("/room/anchor", 0.35),
        ("/room/embed", 1.0),
    ] {
        if let Some(id) = registry.id(addr) {
            registry.set(id, value);
        }
    }
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

#[allow(clippy::too_many_arguments)]
fn param_row(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    id: vizz_params::ParamId,
    def: &vizz_params::ParamDef,
    state: &PanelState,
    modulation: &mut ModEngine,
    ranges: &mut vizz_mod::ranges::Ranges,
    actions: &mut PanelActions,
) {
    let mut value = registry.target(id);
    ui.horizontal(|ui| {
        // Inside a group the prefix is redundant and eats the width the
        // slider needs; filtered results are flat, so they keep it.
        let label = def.addr.trim_start_matches('/');
        let short = label.split_once('/').map_or(label, |(_, rest)| rest);

        // Global rather than part of a look. A preset does not capture
        // these and recalling one leaves them alone, which is correct —
        // the master and the blend time belong to the performer and the
        // room, not to the picture — but entirely invisible until now.
        // "Why didn't my preset restore the master" is a bug report about
        // a feature working as designed, and this line is the answer.
        if vizz_mod::preset::EXCLUDED.contains(&def.addr.as_str()) {
            ui.colored_label(GLOBAL_COLOR, "g").on_hover_text(
                "global — presets and scenes leave this alone, so it stays where you put it",
            );
        }

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

        // The slider covers the *working* range, which may be narrower
        // than what the parameter accepts. OSC, MIDI and presets still
        // address the full range — only the mouse is constrained, because
        // the mouse is the control with a fixed number of pixels to spend.
        let (lo, hi) = ranges.span(&def.addr, def.min, def.max);
        // A stepped parameter shows its position's name rather than a
        // number: `mode 5.000` says nothing, `mode Lorenz` says what is
        // on screen.
        let mut slider = egui::Slider::new(&mut value, lo..=hi)
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

        // Zoom the slider around where it is now, or restore the full
        // range. Stepped parameters are left alone: their whole range is
        // a handful of positions and there is nothing to zoom into.
        if def.labels.is_none() {
            let narrowed = ranges.is_narrowed(&def.addr);
            let (label, hint) = if narrowed {
                ("↔", "restore the full range")
            } else {
                ("→←", "narrow the slider around this value for finer control")
            };
            if ui
                .add(egui::Button::new(label).small().selected(narrowed))
                .on_hover_text(hint)
                .clicked()
            {
                if narrowed {
                    ranges.clear(&def.addr);
                } else {
                    ranges.zoom_around(&def.addr, value, def.min, def.max, 0.1);
                }
                actions.ranges_changed = true;
            }
        }

        // A toggle, and drawn as one. Routing the first LFO here is a
        // starting point; which LFO and how deep are adjustable above.
        let lfo1 = vizz_mod::Source::Lfo(0);
        let routed = modulation.has_route(lfo1, &def.addr);
        let hint = if routed {
            "lfo1 is routed here — click to remove"
        } else {
            "route lfo1 to this parameter"
        };
        if ui
            .add(egui::Button::new("mod").small().selected(routed))
            .on_hover_text(hint)
            .clicked()
        {
            modulation.toggle_route(lfo1, &def.addr, 0.25);
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

/// Marks a parameter that presets do not capture. Cool against the
/// modulation marker's warm, since the two appear side by side and mean
/// unrelated things: one is "something else is moving this", the other is
/// "nothing else will touch this".
const GLOBAL_COLOR: egui::Color32 = egui::Color32::from_rgb(120, 170, 220);

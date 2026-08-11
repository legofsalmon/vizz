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
    pub learn_target: Option<vizz_midi::LearnTarget>,
    pub last_source: Option<Source>,
    /// Binding-change counter, so a learn completing on the MIDI thread
    /// is observable from the frame after.
    pub revision: u64,
    /// Tempo heard as MIDI clock on the wire, when ticks are arriving.
    pub clock_bpm: Option<f32>,
    /// A transport Start arrived; consumed by the app when it resets
    /// the downbeat.
    pub clock_started: bool,
}

impl MidiView {
    /// Is a sweep learn pending for this parameter?
    pub fn learning(&self, param: &str) -> bool {
        matches!(&self.learn_target, Some(t) if t.param == param && t.value.is_none())
    }

    /// Is a trigger learn pending for this value of this parameter?
    pub fn learning_value(&self, param: &str, value: f32) -> bool {
        matches!(&self.learn_target, Some(t) if t.param == param && t.value == Some(value))
    }
}

/// What the panel asks the app to do. Returned rather than applied
/// directly so the panel keeps no privileged access of its own.
#[derive(Default)]
pub struct PanelActions {
    /// Begin MIDI-learn (or cancel, with None).
    pub set_learn_target: Option<Option<vizz_midi::LearnTarget>>,
    /// Remove the MIDI binding for this parameter.
    pub clear_binding: Option<String>,
    /// Remove the MIDI trigger for one value of a parameter, leaving the
    /// other values of it mapped.
    pub clear_slot_binding: Option<(String, f32)>,
    /// A word typed in the clouds section, to become a point cloud.
    pub text_cloud: Option<String>,
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
    /// Put this cloud slot on screen. `true` sets it as the far end of
    /// the morph (b) rather than the near end (a).
    pub cloud_show: Option<(usize, bool)>,
    /// Recording settings the user changed this frame.
    pub record_setup: Option<RecordSetup>,
    /// Connect the video input to this spec, or `Some(None)` to stop
    /// the one running. The app owns opening: it holds the GPU and the
    /// runtimes, and the panel only ever asks.
    pub video_open: Option<Option<String>>,
    /// Look for sources again.
    pub video_rescan: bool,
    /// Open the modulation canvas window. The canvas was reachable only
    /// through `G`, which made it a feature you had to already know about.
    pub open_canvas: bool,
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
    /// How the next take is written. Edited here, applied by the app.
    pub record: RecordSetup,
    /// What is available to receive from, refreshed on demand. Empty
    /// until the section is opened, because discovery blocks.
    pub video_sources: VideoSources,
    /// The video input, when one was configured. `None` draws nothing:
    /// most rigs have no video, and a permanent "no video" dot would be
    /// an alarm about an absence nobody chose. Once a source exists its
    /// health belongs on the strip like audio's does — before this, the
    /// only sign a feed had died was the cloud freezing.
    pub video: Option<VideoStatus>,
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
    /// The recalled slot (1-based), so the preset rows can show where the
    /// look on screen came from.
    pub preset_current: Option<usize>,
    /// The scene grid as it stands this frame.
    pub grid: crate::grid_view::GridView,
    /// The gravity grid, when the layer is in use. `None` hides it from
    /// the performance layout entirely.
    pub gravity_grid: Option<crate::grid_view::GridView>,
    /// The `/` shortcut was pressed this frame; focus the parameter filter.
    pub focus_filter: bool,
    /// A recording in progress, if one is.
    pub recording: Option<RecordingView>,
    /// Draw every collapsible section open.
    ///
    /// For offscreen rendering — tests and the preview example — where
    /// there is nobody to click a header, and asserting on content that
    /// is one click away is still asserting on content that exists.
    pub expand_sections: bool,
}

/// How the next take is written, and what it will cost.
///
/// A mirror of `vizz_io::recorder::Settings` plus the two numbers that
/// make it a decision rather than a surprise: the rate it will write at
/// and the space there is to write into. vizz-ui does not depend on
/// vizz-io, so the app converts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordSetup {
    /// Lossless PNG when true, JPEG otherwise.
    pub lossless: bool,
    pub quality: u8,
    pub fps: f32,
    /// Seconds, or `None` to run until stopped.
    pub max_secs: Option<f32>,
    /// Seconds to count down before the first frame.
    pub countdown_secs: u32,
    /// Bytes a second at the current output size and settings.
    pub bytes_per_sec: u64,
    /// Free space on the volume takes are written to, when known.
    pub free_bytes: Option<u64>,
}

impl Default for RecordSetup {
    fn default() -> Self {
        Self {
            lossless: false,
            quality: 92,
            fps: 30.0,
            max_secs: None,
            countdown_secs: 0,
            bytes_per_sec: 0,
            free_bytes: None,
        }
    }
}

/// What the panel can offer to connect to. Plain strings: this crate
/// knows nothing about NDI, Syphon or AVFoundation, and should not —
/// the app discovers, the panel lists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoSources {
    pub ndi: Vec<String>,
    pub syphon: Vec<String>,
    pub cameras: Vec<String>,
    /// Why a kind found nothing, when the reason is not "nothing there".
    pub notes: Vec<String>,
}

/// Video input state, for the status strip.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoStatus {
    /// Frames are actually arriving — the difference between "wired up"
    /// and "watching a source that went away".
    pub connected: bool,
    /// The source's own name: "ndi:Cam 1", "test pattern".
    pub label: String,
}

/// A recording in progress, for the panel and the performance strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordingView {
    pub secs: u64,
    pub frames: u64,
    pub dropped: u64,
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
    /// The beat clock follows MIDI clock rather than running free.
    pub clock_midi: bool,
    /// Ticks are actually arriving right now — the difference between
    /// "following the wire" and "waiting for a wire that is silent".
    pub clock_ticking: bool,
}

/// Edits the panel wants applied to the audio settings, collected here
/// rather than written directly because the settings live behind a mutex
/// shared with the analysis thread.
#[derive(Debug, Clone, Default)]
pub struct AudioEdits {
    pub bands: Option<[vizz_audio::Band; 4]>,
    pub auto_bpm: Option<bool>,
    /// Follow MIDI clock from the controller (true) or run the internal
    /// clock (false).
    pub midi_clock: Option<bool>,
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
                    outputs_section(ui, state, registry);
                    ui.separator();
                    output_setup_section(ui, state, &mut actions);
                });
            egui::CollapsingHeader::new("clouds")
                .id_salt("clouds")
                .default_open(state.expand_sections)
                .show(ui, |ui| clouds_section(ui, state, registry, &mut actions));
            // Their own header, not a stowaway inside "clouds": someone
            // looking for their colours has no reason to open a section
            // named after geometry.
            egui::CollapsingHeader::new("palettes")
                .id_salt("palettes")
                .default_open(state.expand_sections)
                .show(ui, |ui| palettes_section(ui, state, registry));
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
            egui::CollapsingHeader::new("recording")
                .id_salt("recording")
                .default_open(state.expand_sections)
                .show(ui, |ui| recording_section(ui, state, &mut actions));
            egui::CollapsingHeader::new("video in")
                .id_salt("video-in")
                .default_open(state.expand_sections)
                .show(ui, |ui| video_section(ui, state, &mut actions));
            egui::CollapsingHeader::new("modulation")
                .id_salt("modulation")
                .default_open(state.expand_sections)
                .show(ui, |ui| modulation_section(ui, registry, modulation, &mut actions));
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
            // Every key the app answers to, including the one that documents the
    // rest — a shortcut listed only inside the overlay it opens can never
    // be discovered. And Esc is honest about being a two-step.
    ui.small("Tab panel · G canvas · P performance · ? shortcuts · Esc quits, twice");
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
        // Only when a source was configured — see the field's comment.
        if let Some(v) = &state.video {
            dot(ui, v.connected, if v.connected { GOOD } else { WARN }).on_hover_text(
                if v.connected {
                    "video frames arriving"
                } else {
                    "video source configured but not sending"
                },
            );
            ui.small(&v.label);
        }
        // Same reason: 99.5 and 128.0 are different widths otherwise.
        ui.small(egui::RichText::new(format!("{:>5.1} bpm", state.bpm)).monospace());
        if ui.small_button("tap").on_hover_text("tap the beat — three taps set the tempo and switch auto off").clicked() {
            actions.audio.tapped = true;
        }
    });
}

/// A status dot — the design system's, under the short local name.
fn dot(ui: &mut egui::Ui, live: bool, color: egui::Color32) -> egui::Response {
    vizz_design::widgets::status_dot(ui, live, color)
}

const GOOD: egui::Color32 = crate::theme::LIVE;
const WARN: egui::Color32 = crate::theme::WARN;

/// Notify, never install: the link opens the release page and the user
/// picks the moment. Nothing about a running show changes.
fn update_banner(ui: &mut egui::Ui, state: &PanelState) {
    let Some(version) = &state.update_available else { return };
    ui.horizontal(|ui| {
        ui.colored_label(
            WARN,
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
fn clouds_section(
    ui: &mut egui::Ui,
    state: &PanelState,
    registry: &ParamRegistry,
    actions: &mut PanelActions,
) {
    let slot = |addr: &str| {
        registry.id(addr).map(|id| registry.target(id).round().max(0.0) as usize)
    };
    let (a, b) = (slot("/cloud/a"), slot("/cloud/b"));
    // What the pair actually is, said once. "a" and "b" are the two ends
    // of a morph, not a playlist — and they are set with number sliders
    // whose value is a slot index, which is the least guessable control
    // in the app. The buttons below do the setting; this says why there
    // are two.
    ui.small("“a” and “b” are the two ends of the morph — show one, or set both and blend");
    for (i, name) in state.clouds.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.small(format!("{i}"));
            // Click the name to put this cloud on screen: sets the shape
            // to cloud, points a at this slot and takes the morph fully
            // to it. Exactly what dropping a file does — which was the
            // only way to reach it, and only at the moment of the drop.
            if ui
                .add(egui::Label::new(name).sense(egui::Sense::click()))
                .on_hover_text("show this cloud — click “b” to make it the far end of the morph instead")
                .clicked()
            {
                actions.cloud_show = Some((i, false));
            }
            if ui
                .small_button("b")
                .on_hover_text("make this the far end of the morph, and leave “a” where it is")
                .clicked()
            {
                actions.cloud_show = Some((i, true));
            }
            // Say which rows the morph pair is showing right now. The
            // legend explained what the numbers meant but not which of
            // them was on screen — the one question a legend is for.
            let mut live = Vec::new();
            if a == Some(i) {
                live.push("a");
            }
            if b == Some(i) {
                live.push("b");
            }
            if !live.is_empty() {
                ui.label(
                    egui::RichText::new(live.join(" ")).small().color(LIVE_MARK),
                )
                .on_hover_text("on screen — this end of the cloud morph pair");
            }
        });
    }
    if state.clouds.is_empty() {
        ui.small("no cloud slots");
    }
    // This list and the router in `windowed.rs::load_dropped` must agree;
    // a hint that omits an accepted extension teaches people it will not
    // work, which is worse than no hint.
    ui.small("drag a .ply, .xyz, .csv, .pts, .png, .jpg or .jpeg onto the window to load one");
    // Or type one. A word becomes a cloud: the particles form the
    // letters, morphable against any other slot like any shape.
    ui.horizontal(|ui| {
        let id = egui::Id::new("text-cloud-draft");
        let mut draft: String = ui.memory_mut(|m| m.data.get_temp(id).unwrap_or_default());
        let field = ui.add(
            egui::TextEdit::singleline(&mut draft)
                .hint_text("type a word")
                .desired_width(140.0),
        );
        let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.small_button("make cloud").clicked() || submitted) && !draft.trim().is_empty() {
            actions.text_cloud = Some(draft.trim().to_string());
            draft.clear();
        }
        ui.memory_mut(|m| m.data.insert_temp(id, draft));
    });
}

/// The colour ramps, by the index `/color/palette` uses.
///
/// Same reason as the cloud list: the parameter is a number, and past the
/// four shipped names a number says nothing at all about what is in the
/// slot. Unused rows are left out rather than listed as blanks — sixteen
/// entries of which twelve are empty is a worse legend than four.
fn palettes_section(ui: &mut egui::Ui, state: &PanelState, registry: &ParamRegistry) {
    let current = registry
        .id("/color/palette")
        .map(|id| registry.target(id).round().max(0.0) as usize);
    for (i, name) in state.palettes.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        ui.horizontal(|ui| {
            ui.small(format!("{i}"));
            ui.label(name);
            if current == Some(i) {
                ui.label(egui::RichText::new("live").small().color(LIVE_MARK))
                    .on_hover_text("the ramp /color/palette is set to");
            }
        });
    }
    ui.small("drag a .gpl, .hex or .txt list of hex colours onto the window to add one");
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
            crate::theme::LEARN,
            // "move or press": sweeps are moved, triggers are pressed, and
            // this line is shown for both kinds of learn.
            format!("learning {} — move or press a control (seen: {seen})", target.label),
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
        vizz_design::accent::METER,
    );
    p.rect_filled(
        egui::Rect::from_min_size(
            rect.left_top() + egui::vec2(0.0, h),
            egui::vec2(rect.width() * raw.clamp(0.0, 1.0), h),
        ),
        1.0,
        vizz_design::accent::METER_DIM,
    );
    // Clipping marker: at 1.0 the band is pinned and the gain is too high.
    if env >= 0.999 {
        p.rect_filled(
            egui::Rect::from_min_size(
                rect.right_top() - egui::vec2(3.0, 0.0),
                egui::vec2(3.0, rect.height()),
            ),
            0.0,
            WARN,
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
/// How long an enumerated device list is reused. Long enough that holding
/// the menu open is not an audio-API call per frame, short enough that
/// something plugged in while the menu is open still turns up.
const DEVICE_LIST_TTL: std::time::Duration = std::time::Duration::from_secs(1);

/// The input devices, enumerated at most once a second.
///
/// This only runs while the menu is open — a combo box does not draw its
/// body otherwise — but open at sixty frames a second it was sixty device
/// enumerations a second, on the render thread. Asking CoreAudio for the
/// device list is not a cheap call, and the answer does not change sixty
/// times a second.
fn device_list(ui: &egui::Ui) -> Vec<String> {
    let id = egui::Id::new("audio-device-list");
    let cached: Option<(std::time::Instant, Vec<String>)> =
        ui.data(|d| d.get_temp(id));
    if let Some((at, names)) = cached
        && at.elapsed() < DEVICE_LIST_TTL
    {
        return names;
    }
    let names = vizz_audio::input_devices();
    ui.data_mut(|d| d.insert_temp(id, (std::time::Instant::now(), names.clone())));
    names
}

fn device_picker(ui: &mut egui::Ui, state: &PanelState, actions: &mut PanelActions) {
    let current = state.audio.device.as_deref().unwrap_or("no input");
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().circle_filled(
            dot.center(),
            4.0,
            if state.audio.connected {
                GOOD
            } else {
                vizz_design::ink::FAINT
            },
        );
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
                for name in device_list(ui) {
                    let selected = state.audio.device.as_deref() == Some(name.as_str());
                    if ui.selectable_label(selected, &name).clicked() {
                        actions.audio.device = Some(Some(name.clone()));
                    }
                }
            });
        // A device that has gone away should not look like a live one.
        if !state.audio.connected {
            ui.label(
                egui::RichText::new("not capturing").color(WARN),
            );
        }
    });
}

fn audio_section(ui: &mut egui::Ui, state: &PanelState, actions: &mut PanelActions) {
    let a = &state.audio;
    // No bold "Audio" heading — the collapsing header the user just
    // clicked already says so, and half the sections never restated
    // theirs. The status dot lives on the input row instead.
    device_picker(ui, state, actions);

    if !a.connected {
        ui.small("pick an input above, or start with --audio-device");
        return;
    }

    let mut bands = state.audio_bands;
    for (i, band) in bands.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            // Named the way every modulation source list names them, so
            // "Band 2" in a route can be found in this section without
            // counting rows.
            ui.small(format!("band {}", i + 1));
            meter(ui, a.raw[i], a.bands[i]);
            ui.add(
                egui::DragValue::new(&mut band.lo_hz)
                    .speed(2.0)
                    .range(20.0..=18_000.0)
                    .suffix(" Hz"),
            );
            ui.add(
                egui::DragValue::new(&mut band.hi_hz)
                    .speed(2.0)
                    .range(20.0..=20_000.0)
                    .suffix(" Hz"),
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
        // Armed, matching the other destructive clicks: this sits one
        // button away from "fit" and throws away a gain setup that took
        // real material to dial in.
        if vizz_design::widgets::armed_button(
            ui,
            egui::Id::new("audio-reset-armed"),
            0,
            vizz_design::widgets::Armed {
                idle_label: "reset",
                armed_label: "reset?",
                idle_hover: "back to the shipped bands and gains (asks once)",
                armed_hover: "click again for the shipped bands and gains",
                small: false,
            },
        ) {
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
        let mut follow = a.clock_midi;
        if ui
            .checkbox(&mut follow, "midi clock")
            .on_hover_text(
                "follow MIDI clock from the controller — tapping or auto \
                 switches back to the internal clock",
            )
            .changed()
        {
            actions.audio.midi_clock = Some(follow);
        }
        if a.clock_midi && !a.clock_ticking {
            // Selected but silent is the state worth a word: the clock
            // is running free on its last tempo, not following anything.
            ui.small(
                egui::RichText::new("no ticks")
                    .color(WARN),
            );
        }
        // Same words as the status strip's tap: three surfaces telling
        // three different stories about one behaviour reads as three
        // different behaviours.
        if ui.small_button("tap").on_hover_text("tap the beat — three taps set the tempo and switch auto off").clicked() {
            actions.audio.tapped = true;
        }
    });
    if a.dropped > 0 {
        ui.small(format!("{} samples dropped", a.dropped));
    }
}

fn modulation_section(
    ui: &mut egui::Ui,
    registry: &ParamRegistry,
    m: &mut ModEngine,
    actions: &mut PanelActions,
) {
    ui.horizontal(|ui| {
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
            // "LFO 1", capitalised, because that is how the routes list
            // and the canvas both name it — one object, one name.
            ui.label(format!("LFO {}", i + 1));
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
                vizz_design::accent::METER,
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
        ui.small("no routes — use “LFO 1” next to a parameter, or wire nodes on the canvas");
    }
    // The canvas is this section's bigger sibling — envelopes, gates,
    // beat-synced patterns — and `G` was its only door. A section about
    // modulation that never mentions the modulation canvas is how a
    // feature stays unfound.
    if ui
        .button("open the canvas (G)")
        .on_hover_text("the node editor: audio bands, envelopes and LFOs wired to parameters")
        .clicked()
    {
        actions.open_canvas = true;
    }
    let _ = registry;
}

/// Where the picture comes in from.
///
/// Parity with the audio section, and for the same reason: an input you
/// can only select on the command line is an input most people never
/// find. Everything here is discovered by the app on demand — NDI
/// announcements take a moment and enumerating capture devices wakes
/// hardware, so neither happens per frame.
fn video_section(ui: &mut egui::Ui, state: &PanelState, actions: &mut PanelActions) {
    ui.horizontal(|ui| {
        match &state.video {
            Some(v) => {
                dot(ui, v.connected, if v.connected { GOOD } else { WARN });
                ui.label(&v.label);
                if ui
                    .small_button("stop")
                    .on_hover_text("disconnect the video input")
                    .clicked()
                {
                    actions.video_open = Some(None);
                }
            }
            None => {
                dot(ui, false, vizz_design::ink::FAINT);
                ui.small("no video input");
            }
        }
        if ui
            .small_button("rescan")
            .on_hover_text("look for senders, servers and cameras again")
            .clicked()
        {
            actions.video_rescan = true;
        }
    });
    if let Some(v) = &state.video
        && !v.connected
    {
        // Configured but silent is the state worth a word: the source
        // was found once and has stopped, which is a different problem
        // from never having connected.
        ui.small(egui::RichText::new("no frames arriving").color(WARN));
    }

    let src = &state.video_sources;
    // The test pattern first and always: it is the thing to reach for
    // when nothing appears, because it proves the whole path downstream
    // of the source without a network, a runtime or a device.
    ui.horizontal(|ui| {
        if ui
            .button("test pattern")
            .on_hover_text("a moving picture through the identical path — proves everything downstream")
            .clicked()
        {
            actions.video_open = Some(Some("test".into()));
        }
    });

    let mut list = |ui: &mut egui::Ui, title: &str, prefix: &str, names: &[String]| {
        if names.is_empty() {
            return;
        }
        ui.small(title);
        for name in names {
            if ui
                .button(name)
                .on_hover_text(format!("receive from {name}"))
                .clicked()
            {
                actions.video_open = Some(Some(format!("{prefix}{name}")));
            }
        }
    };
    list(ui, "NDI on the network", "ndi:", &src.ndi);
    list(ui, "Syphon on this Mac", "syphon:", &src.syphon);
    list(ui, "cameras and capture cards", "camera:", &src.cameras);

    if src.ndi.is_empty() && src.syphon.is_empty() && src.cameras.is_empty() {
        ui.small("nothing found — press rescan once a sender or camera is running");
    }
    for note in &src.notes {
        // A missing runtime is not the same as an empty network, and
        // the difference is the whole question when a feed will not
        // appear. Said plainly rather than left as an absence.
        ui.small(egui::RichText::new(note).color(WARN));
    }
    ui.small("the picture arrives as a point cloud — select its slot with /cloud/a");
}

/// How the next take is written, and what it will cost.
///
/// Recording had no settings at all: lossless PNG, every rendered frame,
/// full output size, until something stopped it — about 800 MB a second
/// at 1080p60, which is a laptop's free space in well under a minute
/// with nothing said before or during. The headline here is therefore
/// the *rate*, not the format: the number that tells you whether the
/// take you are about to start fits on the disk you have.
fn recording_section(ui: &mut egui::Ui, state: &PanelState, actions: &mut PanelActions) {
    let mut next = state.record;

    // The cost line first, because it is the reason this section exists.
    let per_sec = next.bytes_per_sec as f64 / 1_000_000.0;
    let headline = format!("{per_sec:.0} MB/s at the current size");
    match next.free_bytes {
        Some(free) if next.bytes_per_sec > 0 => {
            let secs = free / next.bytes_per_sec.max(1);
            let free_gb = free as f64 / 1_000_000_000.0;
            // Colour by how long you have, not by how fast it writes: a
            // fast rate onto an empty array is fine, a slow one onto a
            // full disk is not.
            let colour = if secs < 120 { WARN } else { vizz_design::ink::SECONDARY };
            ui.label(
                egui::RichText::new(format!(
                    "{headline} · {free_gb:.0} GB free · about {} left",
                    fmt_duration(secs)
                ))
                .small()
                .color(colour),
            );
        }
        _ => {
            ui.small(egui::RichText::new(headline).color(vizz_design::ink::SECONDARY));
        }
    }

    ui.horizontal(|ui| {
        ui.label("format");
        // Lossless is the exception now, not the default: takes almost
        // always go into an edit, and JPEG is a tenth the size.
        if ui
            .selectable_label(!next.lossless, "jpeg")
            .on_hover_text("about a tenth the size of PNG — the sane default for footage")
            .clicked()
        {
            next.lossless = false;
        }
        if ui
            .selectable_label(next.lossless, "png")
            .on_hover_text("lossless and large — for compositing, not for long takes")
            .clicked()
        {
            next.lossless = true;
        }
        if !next.lossless {
            ui.add(
                egui::DragValue::new(&mut next.quality)
                    .range(40..=100)
                    .speed(1.0)
                    .prefix("q "),
            )
            .on_hover_text("JPEG quality — 92 is visually clean");
        }
    });

    ui.horizontal(|ui| {
        ui.label("record at");
        ui.add(
            egui::DragValue::new(&mut next.fps)
                .range(1.0..=120.0)
                .speed(1.0)
                .suffix(" fps"),
        )
        .on_hover_text(
            "frames captured a second, independent of the render rate — \
             halving this halves the files and the disk rate",
        );
    });

    ui.horizontal(|ui| {
        ui.label("stop after");
        let mut limited = next.max_secs.is_some();
        if ui.checkbox(&mut limited, "").changed() {
            next.max_secs = limited.then_some(30.0);
        }
        match &mut next.max_secs {
            Some(secs) => {
                ui.add(
                    egui::DragValue::new(secs)
                        .range(1.0..=3600.0)
                        .speed(1.0)
                        .suffix(" s"),
                )
                .on_hover_text("the take ends itself and keeps everything written");
            }
            None => {
                ui.small("runs until stopped");
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("countdown");
        ui.add(
            egui::DragValue::new(&mut next.countdown_secs)
                .range(0..=30)
                .speed(1.0)
                .suffix(" s"),
        )
        .on_hover_text("time to get your hands to the controls before the first frame");
    });

    ui.small("takes are stopped automatically if the disk gets close to full");

    if next != state.record {
        actions.record_setup = Some(next);
    }
}

/// "3 min", "2 h" — long enough to act on, short enough to read.
fn fmt_duration(secs: u64) -> String {
    match secs {
        0..=90 => format!("{secs} s"),
        91..=5400 => format!("{} min", secs / 60),
        _ => format!("{} h", secs / 3600),
    }
}

fn health_section(ui: &mut egui::Ui, state: &PanelState) {
    let Some(h) = &state.health else {
        ui.small("collecting health data…");
        return;
    };

    // Colour the headline by whether we are actually holding the budget —
    // this is the number that matters mid-set, readable at a glance.
    // The same WARN as the status strip's "over budget" word: one
    // phrase, one colour, even though the two read different windows
    // (the strip watches the recent percentage, this headline compares
    // the running average against the budget).
    let over = h.frame_avg_ms > state.frame_budget_ms;
    let color = if over { WARN } else { GOOD };
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new(format!("{:.0} fps", h.fps)).color(color));
        ui.label(
            egui::RichText::new(format!("{:.2} ms avg", h.frame_avg_ms))
                .color(color)
                .small(),
        );
        if over {
            // In words as well as colour: red against green is exactly
            // the pair that collapses for red-green colour-blind eyes,
            // and this is the one headline that matters mid-set.
            ui.label(egui::RichText::new("over budget").color(color).small());
        }
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
        egui::Stroke::new(1.0, vizz_design::accent::METER),
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
    // Edits accumulate here, in egui's own memory, and are only released
    // as an action when a gesture *ends*. Applying on every changed()
    // frame — the previous behaviour — rebuilt the master, the whole post
    // chain and every sender on each tick of a spinner drag: dozens of
    // texture allocations per adjustment, and Syphon/NDI receivers
    // watching the source drop and reappear dozens of times.
    let id = egui::Id::new("output-setup-pending");
    let mut next: OutputSetup =
        ui.data(|d| d.get_temp(id)).unwrap_or(state.output);
    let mut commit = false;
    // A settle, not a commit: the drag or the typing finished.
    let settled = |r: &egui::Response| r.drag_stopped() || r.lost_focus();

    ui.horizontal(|ui| {
        ui.label("output");
        // 8192 matches the app's own side limit (wgpu's default texture
        // ceiling), and lets 8K DCI through; the total-pixel budget is
        // enforced where the textures are allocated, not here.
        let w = ui
            .add(
                egui::DragValue::new(&mut next.width)
                    .range(160..=8192)
                    .speed(8.0),
            )
            .on_hover_text("applying stops any recording");
        ui.label("x");
        let h = ui
            .add(
                egui::DragValue::new(&mut next.height)
                    .range(160..=8192)
                    .speed(8.0),
            )
            .on_hover_text("applying stops any recording");
        commit |= settled(&w) || settled(&h);
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
            if ui
                .small_button(label)
                .on_hover_text("applying stops any recording")
                .clicked()
            {
                next.width = w;
                next.height = h;
                commit = true;
            }
        }
    });
    // Said out loud while it matters, not only on hover: rebuilding the
    // output tears down the recorder, and the first sign used to be a
    // shorter file discovered after the show.
    if state.recording.is_some() {
        ui.small(
            egui::RichText::new("recording — changing the output stops it").color(WARN),
        );
    }

    ui.horizontal(|ui| {
        ui.label("render");
        let r = ui
            .add(
                egui::Slider::new(&mut next.scale, 0.25..=2.0)
                    .suffix("x")
                    .clamping(egui::SliderClamping::Always),
            )
            .on_hover_text("above 1 supersamples: draw larger, let the downscale anti-alias");
        commit |= settled(&r);
    });
    // Say the resulting size out loud. A multiplier is easy to set and
    // hard to picture, and the number that matters for whether the machine
    // will hold 60 fps is the pixel count, not the factor.
    let rw = (next.width as f32 * next.scale) as u32;
    let rh = (next.height as f32 * next.scale) as u32;
    ui.small(format!("drawing {rw} x {rh}"));

    let wide = ui
        .checkbox(&mut next.wide, "16-bit float master")
        .on_hover_text("smoother gradients, at double the master's bandwidth");
    commit |= wide.changed();
    if next.wide {
        // Not a warning about something broken — a statement of what it
        // costs. Syphon and NDI are BGRA8 by definition, so this cannot
        // reach them without a conversion, and pretending otherwise would
        // be discovered as a black frame at a venue.
        ui.small("Syphon and NDI still receive 8-bit; a conversion pass is added for them");
    }

    if commit {
        if next != state.output {
            actions.output_setup = Some(next);
        }
        // Re-sync with what the app actually applied — which may differ
        // from what was asked, since the apply path fits the size to the
        // pixel budget. Holding the raw numbers here would show a size
        // the app has already refused.
        ui.data_mut(|d| d.remove_temp::<OutputSetup>(id));
    } else {
        ui.data_mut(|d| d.insert_temp(id, next));
    }
}

fn outputs_section(ui: &mut egui::Ui, state: &PanelState, registry: &ParamRegistry) {
    // Record lives with the outputs: it is one more consumer of the
    // master. The button writes /record/active exactly as OSC or a
    // learned MIDI button would — one path.
    if let Some(id) = registry.id("/record/active") {
        ui.horizontal(|ui| {
            let on = registry.target(id) >= 0.5;
            let label = if on { "stop recording" } else { "record" };
            let button = egui::Button::new(
                egui::RichText::new(label).color(if on {
                    vizz_design::feedback::ERR_TEXT
                } else {
                    vizz_design::ink::SECONDARY
                }),
            );
            if ui
                .add(button)
                .on_hover_text("PNG sequence of the master output — heavy resolutions drop frames rather than stall the show")
                .clicked()
            {
                registry.set(id, if on { 0.0 } else { 1.0 });
            }
            if let Some(rec) = &state.recording {
                ui.small(format!(
                    "{}:{:02} · {} frames{}",
                    rec.secs / 60,
                    rec.secs % 60,
                    rec.frames,
                    if rec.dropped > 0 {
                        format!(" · {} dropped", rec.dropped)
                    } else {
                        String::new()
                    }
                ));
            }
        });
    }
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
                    let slot = (i + 1) as f32;
                    // Marked as on the stage row: the recalled slot is the
                    // answer to "where did the look on screen come from",
                    // and the two preset lists must not disagree about
                    // whether the app remembers.
                    let current = state.preset_current == Some(i + 1);
                    let bound =
                        state.midi.map.source_for_value(crate::performance::RECALL, slot);
                    let waiting =
                        state.midi.learning_value(crate::performance::RECALL, slot);
                    let mut b = egui::Button::new(&p.name);
                    if waiting {
                        b = b.fill(crate::theme::LEARN);
                    }
                    if current {
                        b = b.stroke(egui::Stroke::new(1.5, crate::theme::CURRENT));
                    }
                    let hover = if waiting {
                        "press a button on your controller".to_string()
                    } else {
                        let mut h = match &p.about {
                            Some(about) => format!("{about} — replaces the current look"),
                            None => "replaces the current look".to_string(),
                        };
                        if let Some(s) = &bound {
                            h = format!("{h}  ·  {}", s.label());
                        }
                        h
                    };
                    let button = ui.add(b).on_hover_text(hover);
                    if button.clicked() {
                        actions.preset_load = Some(p.name.clone());
                    }
                    // The same learn menu as the stage row: the two lists
                    // address the same `/preset/recall` slots, so a
                    // binding must be reachable from either.
                    if state.midi.available {
                        button.context_menu(|ui| match (&bound, waiting) {
                            (Some(s), _) => {
                                if ui.button(format!("unmap {}", s.label())).clicked() {
                                    actions.clear_slot_binding = Some((
                                        crate::performance::RECALL.to_string(),
                                        slot,
                                    ));
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
                                    actions.set_learn_target =
                                        Some(Some(vizz_midi::LearnTarget::value(
                                            crate::performance::RECALL,
                                            slot,
                                            format!("preset {}", i + 1),
                                        )));
                                    ui.close();
                                }
                            }
                        });
                    }
                    if p.builtin {
                        return;
                    }
                    // Armed delete, matching the grid's store/clear
                    // idiom. One click on a 14-point "x" permanently
                    // erasing a file — with no undo anywhere — was the
                    // cheapest destruction in the app, sitting a few
                    // pixels from the load button. Keyed by name inside
                    // one group, so arming one row disarms any other.
                    let key = {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::hash::DefaultHasher::new();
                        p.name.hash(&mut h);
                        h.finish()
                    };
                    if vizz_design::widgets::armed_button(
                        ui,
                        egui::Id::new("preset-delete-armed"),
                        key,
                        vizz_design::widgets::Armed {
                            idle_label: "x",
                            armed_label: "delete?",
                            idle_hover: "delete this preset (asks once)",
                            armed_hover: "click again to delete for good — there is no undo",
                            small: true,
                        },
                    ) {
                        actions.preset_delete = Some(p.name.clone());
                    }
                });
            }
        });

    let id = egui::Id::new("preset-save-name");
    let mut name: String = ui.memory_mut(|m| m.data.get_temp(id).unwrap_or_default());
    let clash = name_clash(&name, &state.presets);
    ui.horizontal(|ui| {
        let editing = ui.add(
            egui::TextEdit::singleline(&mut name)
                .hint_text("name")
                .desired_width(140.0),
        );
        // Enter saves, so the whole thing is type-and-go rather than
        // type-then-aim.
        let entered = editing.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        // The button says which of the two things it is about to do. A
        // button labelled "save" that silently replaces a night's work is
        // the failure this is here to prevent, and a confirmation dialog
        // is the wrong shape for it — this screen is used with one hand
        // while something is on the projector.
        let (label, hover) = match clash {
            Some(Clash::Builtin(n)) => (
                "save",
                format!("{n} is a built-in and cannot be replaced — choose another name"),
            ),
            Some(Clash::User(n)) => ("replace", format!("overwrite the saved look {n}")),
            None => ("save", "store the current look".to_string()),
        };
        // Blocked rather than warned when it would be useless: a preset
        // saved under a built-in's name is written to disk successfully
        // and can then never be recalled, because `by_name` prefers the
        // built-in. Succeeding and doing nothing is worse than refusing.
        let blocked = matches!(clash, Some(Clash::Builtin(_)));
        let button = ui.add_enabled(!blocked, egui::Button::new(label)).on_hover_text(hover);
        if (entered || button.clicked()) && !blocked && !name.trim().is_empty() {
            actions.preset_save = Some(name.clone());
            name.clear();
        }
    });
    match clash {
        Some(Clash::Builtin(n)) => ui.colored_label(
            WARN_COLOR,
            format!("{n} is a built-in — saving over it would hide your look, not replace it"),
        ),
        Some(Clash::User(n)) => {
            ui.colored_label(WARN_COLOR, format!("this replaces the saved look {n}"))
        }
        None => ui.small("names are tidied for the filesystem, so \"a/b\" becomes \"a_b\""),
    };
    ui.memory_mut(|m| m.data.insert_temp(id, name));
}

/// What an existing preset of the same name is.
enum Clash<'a> {
    /// A user preset, which saving replaces.
    User(&'a str),
    /// A built-in, which saving cannot replace — `by_name` prefers
    /// built-ins, so the saved file would simply never be found.
    Builtin(&'a str),
}

/// Does this name already belong to something?
///
/// Compared through the same tidying `save` applies on the way to a
/// filename, because that is what decides whether two names are the same
/// file. Comparing the raw strings would miss "my look?" landing on top of
/// "my look_" — which is exactly the collision nobody would predict.
fn name_clash<'a>(name: &str, presets: &'a [PresetEntry]) -> Option<Clash<'a>> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let wanted = vizz_mod::library::sanitize(name);
    let hit = presets
        .iter()
        .find(|p| vizz_mod::library::sanitize(&p.name) == wanted)?;
    Some(if hit.builtin {
        Clash::Builtin(&hit.name)
    } else {
        Clash::User(&hit.name)
    })
}

/// Warnings in the panel. Warm, matching the modulation marker's family
/// rather than shouting red — this is "look at this", not "something
/// broke".
const WARN_COLOR: egui::Color32 = crate::theme::WARN;

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
    let total = registry.iter().filter(|(_, d)| !is_transport(d)).count();
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
            ui.small(format!("{total} params"))
                .on_hover_text("the list scrolls — or type here to jump");
        }
    });
    let needle = filter.trim().to_ascii_lowercase();
    ui.memory_mut(|m| m.data.insert_temp(filter_id, filter));

    // As tall as the display allows, floored so a cramped window still
    // shows a few rows. There used to be a 320px ceiling here, which on
    // a tall display parked the panel's main working surface in a third
    // of the space while the rest sat empty.
    let height = (screen_h - ui.cursor().top() - PARAM_LIST_MARGIN).max(PARAM_LIST_MIN);
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
            for section in sections(registry) {
                // The section headings are the map: five words that say
                // what the whole list is for, in the order a look gets
                // built. Groups keep their own headers underneath.
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(section.title)
                        .size(vizz_design::text::SECTION)
                        .strong()
                        .monospace()
                        .color(vizz_design::ink::TERTIARY),
                );
                // Each section gets its own id namespace. Without it a
                // group's id depends on how many widgets were emitted
                // before it in this Ui — so adding the heading above
                // shifted every following group's identity, and a
                // CollapsingHeader whose id moves loses the open state
                // that `default_open` set, drawing its header and
                // nothing else. Salting per section makes a group's
                // identity depend on where it *is*, not on what came
                // before it.
                ui.push_id(section.title, |ui| {
                for group in section.groups {
                    let name = group.name;
                    let about = group.about;
                    let title = if group.title.is_empty() { name } else { group.title };
                    egui::CollapsingHeader::new(group.label(modulation, registry))
                        .id_salt(name)
                        .default_open(true)
                        .show(ui, |ui| {
                            // What the group is for, in the group. A
                            // title alone is only self-explanatory to
                            // whoever chose it.
                            ui.small(egui::RichText::new(about).color(vizz_design::ink::FAINT));
                            for (id, def) in group.params {
                                param_row(ui, registry, id, def, state, modulation, ranges, actions);
                            }
                            if name == "camera" {
                                camera_buttons(ui, registry);
                            }
                            if name == "room" {
                                room_buttons(ui, registry);
                            }
                        })
                        .header_response
                        .on_hover_text(format!("{title} — {about}"));
                }
                });
            }
        });
    // One pointer, not a second list: this footer used to carry its own
    // three-item shortcut digest, which drifted from the overlay's and
    // then disagreed with it — two sources of truth about the same keys.
    ui.small("? shows every shortcut");
}

/// One group of parameters, named for what it does.
struct Group<'a> {
    /// The address prefix this group collects, e.g. `particles`. Kept
    /// because the camera and room buttons attach by it, and because a
    /// stable id_salt must not change when a title is reworded.
    name: &'a str,
    /// What a person calls it. `pal` is a namespace; "palette" is the
    /// thing on screen.
    title: &'static str,
    /// One line saying what the group is for, shown inside it. A group
    /// called "vector layers" is only self-explanatory to whoever built
    /// it.
    about: &'static str,
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
            format!("{}  ({} · {moving}~)", self.title, self.params.len())
        } else {
            format!("{}  ({})", self.title, self.params.len())
        }
    }
}

/// The panel's shape: sections in the order you build a look, each
/// holding groups named for what they change.
///
/// The list used to be generated from the first segment of every OSC
/// address, which meant the screen showed the *network namespace* —
/// `pal`, `vec`, `bg` and four peers called `l1`…`l4` sitting beside
/// `particles`. That is the right structure for a wire protocol and the
/// wrong one for a person: it groups by who owns the address rather
/// than by what the control does, and it cannot say what a group is
/// for.
///
/// This table is the human layout. It is deliberately a table and not a
/// naming convention on the addresses, because the OSC surface is
/// public and stable — renaming `/pal/0/r` to please a panel would
/// break every show file and every script anyone has written.
///
/// A prefix missing from here still appears, under its own name, in a
/// final "more" section. That is the rule that keeps this honest: a
/// parameter added tomorrow shows up without being registered twice,
/// and shows up somewhere visible enough that someone will come and
/// place it properly.
struct SectionSpec {
    title: &'static str,
    /// Address prefixes, in the order they should read.
    groups: &'static [(&'static str, &'static str, &'static str)],
}

const SECTIONS: &[SectionSpec] = &[
    SectionSpec {
        title: "SHAPE",
        groups: &[
            (
                "particles",
                "particles",
                "how many points there are, how big and how bright",
            ),
            (
                "shape",
                "form",
                "which shape the points take, and the morph between two of them",
            ),
            (
                "cloud",
                "clouds",
                "which of the eight loaded clouds the morph runs between",
            ),
            (
                "gravity",
                "gravity",
                "the attract / repel layer that pulls points off their shape",
            ),
        ],
    },
    SectionSpec {
        title: "LOOK",
        groups: &[
            ("color", "colour", "palette choice, hue spread and saturation"),
            ("pal", "vector palette", "the four inks the vector layers print with"),
            ("bg", "background", "paper colour behind everything, and its alpha"),
            ("fx", "effects", "the feedback chain: trails, zoom, spin, mirror, glow"),
        ],
    },
    SectionSpec {
        title: "PRINT",
        groups: &[
            (
                "l1",
                "vector layer 1",
                "hard-edged pattern: generator, blend mode, frequency and ink",
            ),
            (
                "l2",
                "vector layer 2",
                "a second layer — near frequencies interfere into moiré",
            ),
            (
                "l3",
                "vector layer 3",
                "a third layer — off by default, like the fourth",
            ),
            (
                "l4",
                "vector layer 4",
                "the fourth and last layer of the print stack",
            ),
            (
                "vec",
                "vector placement",
                "whether the stack lives inside the feedback chain or prints clean over it",
            ),
        ],
    },
    SectionSpec {
        title: "STAGE",
        groups: &[
            ("camera", "camera", "where you are standing: orbit, distance, lens and pan"),
            ("room", "room", "the box around the field, and its wireframe"),
            ("video", "live video", "how an incoming picture becomes relief"),
        ],
    },
    SectionSpec {
        title: "OUTPUT",
        groups: &[
            ("master", "master", "the last thing before the output — dim, and the panic fader"),
            ("punch", "punch", "the hold-to-engage gestures, also on the performance row"),
        ],
    },
];

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
fn is_transport(def: &vizz_params::ParamDef) -> bool {
    def.transport
}

/// A section of the parameter list: a title and the groups under it.
struct Section<'a> {
    title: &'static str,
    groups: Vec<Group<'a>>,
}

/// Build the panel's sections from the registry.
///
/// Groups still come from the address prefix — that is what actually
/// ties a set of parameters together — but their order, their titles
/// and which section they sit in come from [`SECTIONS`]. Anything the
/// table does not mention lands in a final "more" section under its own
/// prefix, so a new parameter is never invisible.
fn sections(registry: &ParamRegistry) -> Vec<Section<'_>> {
    let mut by_prefix: Vec<(&str, Vec<(vizz_params::ParamId, &vizz_params::ParamDef)>)> =
        Vec::new();
    for (id, def) in registry.iter() {
        if is_transport(def) {
            continue;
        }
        let name = def
            .addr
            .trim_start_matches('/')
            .split('/')
            .next()
            .unwrap_or("other");
        match by_prefix.iter_mut().find(|(p, _)| *p == name) {
            Some((_, params)) => params.push((id, def)),
            None => by_prefix.push((name, vec![(id, def)])),
        }
    }

    let mut out = Vec::new();
    for spec in SECTIONS {
        let mut groups = Vec::new();
        for (prefix, title, about) in spec.groups {
            // `swap_remove`-by-search rather than a lookup: what is left
            // over at the end is exactly the set nothing claimed, which
            // is how the "more" section stays correct without a second
            // list to keep in step.
            if let Some(i) = by_prefix.iter().position(|(p, _)| p == prefix) {
                let (name, params) = by_prefix.remove(i);
                groups.push(Group { name, title, about, params });
            }
        }
        if !groups.is_empty() {
            out.push(Section { title: spec.title, groups });
        }
    }
    if !by_prefix.is_empty() {
        // Unplaced. Named after their address because that is all this
        // code knows about them — and visible, because a parameter you
        // cannot find is worse than one in the wrong company.
        let groups = by_prefix
            .into_iter()
            .map(|(name, params)| Group {
                name,
                title: "",
                about: "not yet placed in a section — see SECTIONS in panel.rs",
                params,
            })
            .collect();
        out.push(Section { title: "MORE", groups });
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
                ("<>", "restore the full range")
            } else {
                ("><", "narrow the slider around this value for finer control")
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
            "LFO 1 is routed here — click to remove"
        } else {
            "route LFO 1 to this parameter"
        };
    // Modulation cannot reach transport: the engine reads fire, blend time,
    // curve and autopilot from `target()`, which modulation never touches,
    // so a route there is inert. Offering the button and then drawing the
    // "modulated" marker beside it was the app claiming to do something it
    // had no path to do.
    // Labelled with what it actually routes. As "mod" it contradicted
    // the ~ marker: a parameter driven by an audio band showed ~ while
    // the button sat unlit, which read as the panel disagreeing with
    // itself about whether the row was modulated.
    if !is_transport(def)
        && ui
            .add(egui::Button::new("LFO 1").small().selected(routed))
            .on_hover_text(hint)
            .clicked()
    {
        modulation.toggle_route(lfo1, &def.addr, 0.25);
    }

        if !state.midi.available {
            return;
        }
        let learning = state.midi.learning(&def.addr);
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
                    actions.set_learn_target =
                        Some(Some(vizz_midi::LearnTarget::param(def.addr.clone())));
                }
            }
        }
    });
}

/// Marks a modulated parameter. Warm against the panel's blues so it reads
/// as "something else is touching this" at a glance.
const MOD_COLOR: egui::Color32 = vizz_design::accent::MOD;
/// The "this row is what you are seeing" marker in the slot legends.
const LIVE_MARK: egui::Color32 = crate::theme::CURRENT;

/// Marks a parameter that presets do not capture. Cool against the
/// modulation marker's warm, since the two appear side by side and mean
/// unrelated things: one is "something else is moving this", the other is
/// "nothing else will touch this".
const GLOBAL_COLOR: egui::Color32 = vizz_design::accent::GLOBAL;

#[cfg(test)]
mod save_name_tests {
    use super::*;

    fn entries() -> Vec<PresetEntry> {
        vec![
            PresetEntry {
                name: "Butterfly".into(),
                builtin: true,
                about: Some("a built-in".into()),
            },
            PresetEntry { name: "warehouse 2am".into(), builtin: false, about: None },
            // As it appears on disk having been saved from "night/shift":
            // the separator was rewritten on the way to a filename.
            PresetEntry { name: "night_shift".into(), builtin: false, about: None },
        ]
    }

    /// Saving used to overwrite in silence. A night's work is one
    /// mistyped name away from gone, and the only warning was a line of
    /// small print that said it happens in general rather than that it is
    /// about to happen now.
    #[test]
    fn a_name_already_in_use_is_recognised() {
        let e = entries();
        assert!(matches!(name_clash("warehouse 2am", &e), Some(Clash::User(_))));
        assert!(name_clash("warehouse 3am", &e).is_none());
        // Nothing typed is not a collision with anything.
        assert!(name_clash("", &e).is_none());
        assert!(name_clash("   ", &e).is_none());
    }

    /// Two names are the same preset when they land on the same file, and
    /// that is decided after tidying. Comparing the raw strings would miss
    /// the one collision nobody could predict — a punctuation mark that
    /// the filesystem will not take being rewritten onto an existing name.
    #[test]
    fn names_that_tidy_to_the_same_file_are_the_same_preset() {
        let e = entries();
        // Different punctuation, same file: both become "night_shift".
        assert!(matches!(name_clash("night?shift", &e), Some(Clash::User(_))));
        assert!(matches!(name_clash("night/shift", &e), Some(Clash::User(_))));
        // And leading or trailing space is not a different preset.
        assert!(matches!(name_clash("  warehouse 2am  ", &e), Some(Clash::User(_))));
    }

    /// A preset saved under a built-in's name writes successfully and can
    /// then never be recalled: `by_name` prefers the built-in, so the file
    /// is simply never looked at. Reported separately because "you will
    /// replace this" and "this will do nothing" call for different
    /// answers.
    #[test]
    fn a_builtins_name_is_flagged_as_unusable_rather_than_as_a_replacement() {
        let e = entries();
        match name_clash("Butterfly", &e) {
            Some(Clash::Builtin(n)) => assert_eq!(n, "Butterfly"),
            _ => panic!("a built-in's name was not recognised as one"),
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use vizz_params::ParamDef;

    fn registry_with(addrs: &[&str]) -> ParamRegistry {
        let mut b = ParamRegistry::builder();
        for a in addrs {
            b.add(ParamDef::new(*a, 0.0, 1.0, 0.5));
        }
        b.build()
    }

    /// Every prefix the table names must be spelled the way the
    /// registry spells it.
    ///
    /// A typo here is silent and expensive: the group simply never
    /// matches, so its parameters fall through to "more" wearing their
    /// raw address, which looks exactly like a parameter nobody has got
    /// round to placing yet.
    #[test]
    fn every_named_section_group_matches_a_real_prefix() {
        // The app's real address surface, as the panel will see it.
        let addrs = [
            "/particles/count", "/shape/mode", "/cloud/a", "/gravity/mode",
            "/color/palette", "/pal/0/r", "/bg/red", "/fx/glow",
            "/l1/kind", "/l2/kind", "/l3/kind", "/l4/kind", "/vec/place",
            "/camera/orbit", "/room/size", "/video/depth",
            "/master/dim", "/punch/flash",
        ];
        let reg = registry_with(&addrs);
        let built = sections(&reg);
        assert!(
            !built.iter().any(|s| s.title == "MORE"),
            "a known prefix fell through to MORE — check SECTIONS for a typo"
        );
        // And the titles are human, not namespaces.
        let titles: Vec<&str> = built
            .iter()
            .flat_map(|s| s.groups.iter().map(|g| g.title))
            .collect();
        assert!(titles.contains(&"palette") || titles.contains(&"vector palette"));
        assert!(!titles.contains(&"pal"), "a raw namespace reached the screen");
        assert!(!titles.contains(&"l1"), "a raw namespace reached the screen");
    }

    /// A parameter under a prefix nobody placed must still be drawn.
    ///
    /// The failure this prevents is the quiet one: a group added to the
    /// registry, forgotten in the table, and therefore invisible in the
    /// panel — which is how a control ships that only OSC can reach.
    #[test]
    fn an_unplaced_prefix_still_appears() {
        let reg = registry_with(&["/particles/count", "/newthing/size"]);
        let built = sections(&reg);
        let more = built
            .iter()
            .find(|s| s.title == "MORE")
            .expect("an unplaced prefix vanished from the panel");
        assert!(
            more.groups.iter().any(|g| g.name == "newthing"),
            "the unplaced group is not in MORE"
        );
    }

    /// Sections read in the order a look gets built, and every group
    /// carries a line saying what it is for.
    #[test]
    fn sections_are_ordered_and_every_group_explains_itself() {
        let order: Vec<&str> = SECTIONS.iter().map(|s| s.title).collect();
        assert_eq!(order, ["SHAPE", "LOOK", "PRINT", "STAGE", "OUTPUT"]);
        for spec in SECTIONS {
            for (prefix, title, about) in spec.groups {
                assert!(!title.is_empty(), "{prefix} has no human title");
                assert!(
                    about.len() > 20,
                    "{prefix}'s caption is too short to say anything useful"
                );
            }
        }
    }
}

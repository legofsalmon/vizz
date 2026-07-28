//! The vizz control panel: an egui overlay on the preview window.
//!
//! The GUI is a control-thread citizen like OSC — it writes parameter
//! targets into the shared [`ParamRegistry`] and reads health snapshots,
//! and has no privileged access to the renderer. Its draw happens inside
//! the same frame's command encoder, so it costs one extra render pass
//! and never introduces a synchronisation point.

pub mod graph_view;
pub mod panel;
pub mod performance;
mod renderer;

/// Exposed for the offscreen panel-preview example; the app uses [`Gui`].
pub use renderer::EguiRenderer as EguiRendererForPreview;

use anyhow::Result;
use vizz_params::ParamRegistry;
use winit::event::WindowEvent;
use winit::window::Window;

pub use graph_view::GraphView;
pub use performance::{PerformanceActions, PerformanceState};
pub use panel::{AudioEdits, AudioView, MidiView, OutputStatus, PanelActions, PanelState, PresetEntry};

/// Exposed for the offscreen preview, so the overlay is reviewed through
/// the same code the app runs rather than a copy of it.
pub fn draw_shortcuts_for_preview(ctx: &egui::Context, open: &mut bool) {
    shortcuts_overlay(ctx, open);
}

/// Keyboard shortcuts, on screen rather than only in the README.
///
/// A shortcut nobody can discover is a shortcut nobody uses, and the
/// number keys in particular are the difference between presets being
/// playable and being a menu.
fn shortcuts_overlay(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("shortcuts")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            for (key, what) in [
                ("1 – 9, 0", "fire preset slot 1–10"),
                ("Tab", "show or hide the control panel"),
                ("G", "modulation canvas"),
                ("P", "performance layout"),
                ("/", "filter the parameter list"),
                ("?", "this list"),
                ("Esc", "quit"),
            ] {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [72.0, 18.0],
                        egui::Label::new(egui::RichText::new(key).strong().monospace()),
                    );
                    ui.label(what);
                });
            }
            ui.separator();
            ui.small("right-click any slider to reset it to its default");
        });
}

/// How many frame times the sparkline keeps.
const HISTORY: usize = 240;

pub struct Gui {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: renderer::EguiRenderer,
    /// Toggled with Tab. Hidden by default is wrong for a first run — you
    /// would not know the panel exists — so it starts visible.
    pub visible: bool,
    /// The node canvas, in its own window. Off by default: patching is an
    /// editing activity, and a live set should not open onto it.
    pub graph_open: bool,
    /// Performance layout: faders and status, nothing else. Off by
    /// default — you arrive wanting to build a look, not play one.
    pub performance: bool,
    /// Keyboard-shortcut overlay, toggled with `?`. Shortcuts that are
    /// only in the README are shortcuts nobody uses.
    pub shortcuts_open: bool,
    /// `/` was pressed; the panel focuses its filter next frame.
    pub focus_filter: bool,
    /// A number key was pressed: fire this preset slot.
    pub preset_key: Option<u32>,
    graph_view: graph_view::GraphView,
    macros: vizz_mod::perform::Macros,
    history: Vec<f32>,
}

impl Gui {
    pub fn new(window: &Window, device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            // Keep atlases within what the adapter accepts.
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        Self {
            ctx,
            state,
            renderer: renderer::EguiRenderer::new(device, target_format),
            visible: true,
            graph_open: false,
            performance: false,
            shortcuts_open: false,
            focus_filter: false,
            preset_key: None,
            graph_view: graph_view::GraphView::default(),
            macros: vizz_mod::perform::Macros::load(),
            history: Vec::with_capacity(HISTORY),
        }
    }

    /// Feed a window event to egui. Returns `true` if egui consumed it,
    /// in which case the caller should not act on it (so dragging a
    /// slider does not also trigger app shortcuts).
    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        // Tab is ours, always: the panel must be dismissible even when
        // egui wants keyboard focus.
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state.is_pressed()
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab)
        {
            self.visible = !self.visible;
            return true;
        }
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state.is_pressed()
            && !self.ctx.egui_wants_keyboard_input()
        {
            match event.logical_key.as_ref() {
                winit::keyboard::Key::Character("g") => {
                    self.graph_open = !self.graph_open;
                    return true;
                }
                winit::keyboard::Key::Character("p") => {
                    self.performance = !self.performance;
                    return true;
                }
                // `/` jumps to the parameter filter, as it does in every
                // other searchable list. Consumed here so the character
                // does not also land in the field it just focused.
                winit::keyboard::Key::Character("/") => {
                    self.visible = true;
                    self.focus_filter = true;
                    return true;
                }
                winit::keyboard::Key::Character("?") => {
                    self.shortcuts_open = !self.shortcuts_open;
                    return true;
                }
                // Number keys fire preset slots. This is the reason to
                // have presets at all during a set: one keystroke, no
                // pointer, no looking away from the output.
                winit::keyboard::Key::Character(d)
                    if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() =>
                {
                    let n = d.as_bytes()[0] - b'0';
                    // 1..9 are slots 1..9; 0 is slot 10, matching how the
                    // row reads left to right.
                    self.preset_key = Some(if n == 0 { 10 } else { n as u32 });
                    return true;
                }
                _ => {}
            }
        }
        if !self.visible {
            return false;
        }
        self.state.on_window_event(window, event).consumed
    }

    /// Record a frame time for the sparkline.
    pub fn push_frame_time(&mut self, ms: f32) {
        if self.history.len() == HISTORY {
            self.history.remove(0);
        }
        self.history.push(ms);
    }

    /// Draw the panel into `target` within the current frame's encoder.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        registry: &ParamRegistry,
        mut state: PanelState,
        modulation: &mut vizz_mod::ModEngine,
        size_px: [u32; 2],
    ) -> Result<PanelActions> {
        // The shortcut list counts as something to draw. Leaving it out
        // meant `?` did nothing with everything hidden — which is exactly
        // the moment you press it, having forgotten how to get the panel
        // back.
        if !self.visible && !self.graph_open && !self.performance && !self.shortcuts_open {
            return Ok(PanelActions::default());
        }
        state.frame_times_ms = self.history.clone();
        state.focus_filter = std::mem::take(&mut self.focus_filter);

        let input = self.state.take_egui_input(window);
        // begin_pass/end_pass rather than run_ui: the panel builds its own
        // window from the context instead of drawing into a provided Ui.
        self.ctx.begin_pass(input);
        // Before the performance branch, not after: that branch returns
        // early, so an overlay drawn below it never appears in the layout
        // where a shortcut list is most wanted.
        if self.shortcuts_open {
            shortcuts_overlay(&self.ctx, &mut self.shortcuts_open);
        }
        if self.performance {
            return self.render_performance(window, device, queue, encoder, target, registry, state, size_px);
        }
        let actions = if self.visible {
            panel::draw(&self.ctx, registry, &state, modulation)
        } else {
            PanelActions::default()
        };
        if self.graph_open {
            let mut open = true;
            egui::Window::new("modulation")
                .open(&mut open)
                .default_pos([420.0, 60.0])
                .default_size([760.0, 520.0])
                .resizable(true)
                .show(&self.ctx, |ui| {
                    self.graph_view.show(ui, &mut modulation.graph, registry);
                });
            self.graph_open = open;
        }
        let output = self.ctx.end_pass();
        self.state
            .handle_platform_output(window, output.platform_output);

        self.renderer.update_textures(device, queue, &output.textures_delta);
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.renderer.render(
            device,
            queue,
            encoder,
            target,
            &primitives,
            size_px,
            output.pixels_per_point,
        )?;
        Ok(actions)
    }

    /// The performance layout replaces the panel and canvas entirely
    /// rather than sitting alongside them: its whole value is that nothing
    /// else is competing for the same screen.
    #[allow(clippy::too_many_arguments)]
    fn render_performance(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        registry: &ParamRegistry,
        state: PanelState,
        size_px: [u32; 2],
    ) -> Result<PanelActions> {
        let health = state.health.as_ref();
        let preset_names: Vec<String> = state.presets.iter().map(|p| p.name.clone()).collect();
        let perf_state = performance::PerformanceState {
            outputs: &state.outputs,
            audio: &state.audio,
            fps: health.map(|h| h.fps).unwrap_or(0.0),
            over_budget: health.map(|h| h.over_budget_window_pct > 1.0).unwrap_or(false),
            bpm: state.bpm,
            bar_phase: state.bar_phase,
            presets: &preset_names,
        };
        let perf = performance::draw(&self.ctx, registry, &perf_state, &mut self.macros);
        if perf.exit {
            self.performance = false;
        }
        if perf.macros_changed && let Err(e) = self.macros.save() {
            log::warn!("could not save macro assignments: {e}");
        }

        let output = self.ctx.end_pass();
        self.state.handle_platform_output(window, output.platform_output);
        self.renderer.update_textures(device, queue, &output.textures_delta);
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.renderer.render(
            device,
            queue,
            encoder,
            target,
            &primitives,
            size_px,
            output.pixels_per_point,
        )?;

        // Tap tempo is the one action shared with the panel, so it rides
        // the same path the app already handles.
        let mut actions = PanelActions::default();
        actions.audio.tapped = perf.tapped;
        // Routed through the same one-shot the number keys use, so a
        // click and a keystroke take an identical path to the recall
        // parameter — one way to fire a preset, not two that can drift.
        if let Some(slot) = perf.preset_slot {
            self.preset_key = Some(slot);
        }
        Ok(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vizz_params::ParamDef;

    fn registry() -> ParamRegistry {
        let mut b = ParamRegistry::builder();
        b.add(ParamDef::new("/particles/count", 0.0, 100.0, 25.0));
        b.add(ParamDef::new("/master/dim", 0.0, 1.0, 1.0));
        b.build()
    }

    /// A stepped parameter must read as its position's name. `mode 5.000`
    /// is legible and still tells you nothing; `mode Lorenz` tells you
    /// what is on screen, which in a dark room is the whole difference.
    #[test]
    fn stepped_parameters_read_as_names_not_numbers() {
        let mut b = ParamRegistry::builder();
        let mode = b.add(
            ParamDef::new("/shape/mode", 0.0, 7.0, 0.0)
                .labels(&["sphere", "torus", "knot", "grid", "shell", "Lorenz", "Aizawa", "cloud"]),
        );
        let reg = b.build();
        reg.set(mode, 5.0);
        let ctx = egui::Context::default();
        let state = PanelState {
            update_available: None,
            health: None,
            outputs: Vec::new(),
            frame_times_ms: Vec::new(),
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            bpm: 120.0,
            presets: Vec::new(),
            focus_filter: false,
            expand_sections: true,
            bar_phase: 0.0,
        };
        let text = run_panel(&ctx, &reg, &state);
        assert!(text.contains("Lorenz"), "stepped value not named: {text}");
        assert!(!text.contains("5.000"), "raw number still shown: {text}");
    }

    /// The preset list has to render, including the slot numbers — they
    /// are what `/preset/recall` and therefore a MIDI button address, and
    /// without them you would count rows to work out what to bind.
    ///
    /// Built-ins must not offer a delete button: they are the "put it back
    /// how it shipped" path, and a starting point you can destroy is not a
    /// starting point.
    #[test]
    fn the_panel_lists_presets_with_slot_numbers() {
        let reg = registry();
        let ctx = egui::Context::default();
        let state = PanelState {
            update_available: None,
            health: None,
            outputs: Vec::new(),
            frame_times_ms: Vec::new(),
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            bpm: 120.0,
            focus_filter: false,
            expand_sections: true,
        presets: vec![
                PresetEntry { name: "Slow bloom".into(), builtin: true, about: Some("opener".into()) },
                PresetEntry { name: "Warehouse 2".into(), builtin: false, about: None },
            ],
            bar_phase: 0.0,
        };
        let text = run_panel(&ctx, &reg, &state);
        assert!(text.contains("Presets"), "no presets section: {text}");
        assert!(text.contains("Slow bloom"), "built-in missing: {text}");
        assert!(text.contains("Warehouse 2"), "user preset missing: {text}");
        assert!(text.contains("save"), "no way to store a preset: {text}");
        // Slots are 1-based; slot 0 means nothing selected.
        assert!(text.contains(" 1"), "slot numbers missing: {text}");
        assert!(text.contains(" 2"), "slot numbers missing: {text}");
    }

    /// The panel must be buildable from nothing but the registry — this is
    /// what keeps it in sync with the OSC surface automatically.
    #[test]
    fn panel_renders_a_control_for_every_registered_param() {
        let reg = registry();
        let ctx = egui::Context::default();
        let state = PanelState {
            update_available: None,
            health: None,
            outputs: vec![OutputStatus { name: "syphon:vizz".into(), live: true }],
            frame_times_ms: vec![16.0, 17.0, 15.5],
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            bpm: 120.0,
            presets: Vec::new(),
            focus_filter: false,
            expand_sections: true,
            bar_phase: 0.0,
        };

        let text = run_panel(&ctx, &reg, &state);

        // Every parameter must have a control. Rows inside a group show
        // the short name — the prefix is the group header, and repeating
        // it eats the width the slider needs — so check both halves.
        for (_, def) in reg.iter() {
            let path = def.addr.trim_start_matches('/');
            let (group, short) = path.split_once('/').unwrap_or(("", path));
            assert!(text.contains(short), "no control drawn for {path}; got: {text}");
            assert!(text.contains(group), "no group header for {path}; got: {text}");
        }
        assert!(text.contains("syphon:vizz"), "output status missing: {text}");
    }

    #[test]
    fn panel_tolerates_missing_health_and_no_outputs() {
        let reg = registry();
        let ctx = egui::Context::default();
        // First frames have no health snapshot yet and no senders; the
        // panel must still draw rather than panic on unwrapping.
        let state = PanelState {
            update_available: None,
            health: None,
            outputs: vec![],
            frame_times_ms: vec![],
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            bpm: 120.0,
            presets: Vec::new(),
            focus_filter: false,
            expand_sections: true,
            bar_phase: 0.0,
        };
        let text = run_panel(&ctx, &reg, &state);
        assert!(text.contains("Collecting health data"), "got: {text}");
        assert!(text.contains("preview only"), "got: {text}");
    }

    /// The audio section has to show the device, the band edges the user
    /// can retune, and the detected tempo — without those the gain and
    /// filter controls are unusable, because there is nothing to set them
    /// against.
    #[test]
    fn panel_shows_audio_bands_and_detected_tempo() {
        let reg = registry();
        let ctx = egui::Context::default();
        let mut state = PanelState {
            update_available: None,
            health: None,
            outputs: vec![],
            frame_times_ms: vec![],
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView {
                connected: true,
                device: Some("Scarlett 2i2".into()),
                bands: [0.8, 0.4, 0.2, 0.1],
                raw: [0.13, 0.1, 0.05, 0.01],
                level: 0.2,
                detected_bpm: 128.0,
                confidence: 0.7,
                dropped: 0,
            },
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: true,
            bpm: 128.0,
            presets: Vec::new(),
            focus_filter: false,
            expand_sections: true,
            bar_phase: 0.05,
        };
        let text = run_panel(&ctx, &reg, &state);
        assert!(text.contains("Scarlett 2i2"), "device missing: {text}");
        assert!(text.contains("128.0 bpm"), "detected tempo missing: {text}");
        assert!(text.contains("tap"), "tap tempo missing: {text}");
        // Band edges are the filter control; they must be editable numbers.
        assert!(text.contains("30 Hz") && text.contains("110 Hz"), "band edges missing: {text}");

        // Disconnected must say so and explain the fix rather than showing
        // four dead meters.
        state.audio = AudioView::default();
        let text = run_panel(&ctx, &reg, &state);
        assert!(text.contains("no input"), "got: {text}");
        assert!(text.contains("--list-audio"), "no hint about finding a device: {text}");
    }

    /// A learned binding must be visible on its slider, and learn mode
    /// must announce itself — otherwise there is no way to tell whether
    /// the controller is being heard at all.
    #[test]
    fn panel_shows_midi_bindings_and_learn_state() {
        let reg = registry();
        let ctx = egui::Context::default();
        let mut map = vizz_midi::MidiMap::default();
        map.bind(
            vizz_midi::Source::ControlChange { channel: 0, controller: 7 },
            "/master/dim",
        );
        let state = PanelState {
            update_available: None,
            health: None,
            outputs: vec![],
            frame_times_ms: vec![],
            frame_budget_ms: 16.67,
            midi: MidiView {
                available: true,
                connected: vec!["Launch Control XL".into()],
                map,
                learn_target: Some("/particles/count".into()),
                last_source: Some(vizz_midi::Source::Note { channel: 9, note: 36 }),
            },
            audio: AudioView::default(),
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            bpm: 120.0,
            presets: Vec::new(),
            focus_filter: false,
            expand_sections: true,
            bar_phase: 0.0,
        };
        let text = run_panel(&ctx, &reg, &state);
        assert!(text.contains("Launch Control XL"), "device missing: {text}");
        // 1-based channel, matching what controllers display.
        assert!(text.contains("ch1 cc7"), "binding label missing: {text}");
        assert!(text.contains("learning /particles/count"), "learn prompt missing: {text}");
        assert!(text.contains("ch10 note36"), "learn feedback missing: {text}");
    }

    /// The update banner must appear only when there is something to
    /// report, and must link out rather than imply an in-place install.
    #[test]
    fn update_banner_appears_only_when_a_newer_version_exists() {
        let reg = registry();
        let base = |update: Option<String>| PanelState {
            update_available: update,
            health: None,
            outputs: vec![],
            frame_times_ms: vec![],
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            bpm: 120.0,
            presets: Vec::new(),
            focus_filter: false,
            expand_sections: true,
            bar_phase: 0.0,
        };

        let quiet = run_panel(&egui::Context::default(), &reg, &base(None));
        // Match the banner's own words: a bare "available" also matches
        // the MIDI section's "unavailable".
        assert!(!quiet.contains("download"), "banner shown with no update: {quiet}");
        assert!(!quiet.contains("vizz 0.2.0"), "banner shown with no update: {quiet}");

        let loud = run_panel(&egui::Context::default(), &reg, &base(Some("0.2.0".into())));
        assert!(loud.contains("vizz 0.2.0 available"), "banner missing: {loud}");
        assert!(loud.contains("download"), "no link to the release: {loud}");
    }

    /// Drive the panel and return every string it drew.
    ///
    /// Two details this has to get right, both learned the hard way:
    /// without a `screen_rect` egui clips the whole window away and emits
    /// nothing, and the first pass over a freshly-created Window only
    /// measures it — real shapes appear on the second. The live app
    /// renders continuously so it never notices, but a one-shot test does.
    fn run_panel(ctx: &egui::Context, reg: &ParamRegistry, state: &PanelState) -> String {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(900.0, 700.0),
            )),
            ..Default::default()
        };
        let mut text = String::new();
        for _ in 0..2 {
            ctx.begin_pass(input.clone());
            let _ = panel::draw(ctx, reg, state, &mut vizz_mod::ModEngine::with_defaults());
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
}


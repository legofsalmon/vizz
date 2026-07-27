//! The vizz control panel: an egui overlay on the preview window.
//!
//! The GUI is a control-thread citizen like OSC — it writes parameter
//! targets into the shared [`ParamRegistry`] and reads health snapshots,
//! and has no privileged access to the renderer. Its draw happens inside
//! the same frame's command encoder, so it costs one extra render pass
//! and never introduces a synchronisation point.

pub mod graph_view;
pub mod panel;
mod renderer;

/// Exposed for the offscreen panel-preview example; the app uses [`Gui`].
pub use renderer::EguiRenderer as EguiRendererForPreview;

use anyhow::Result;
use vizz_params::ParamRegistry;
use winit::event::WindowEvent;
use winit::window::Window;

pub use graph_view::GraphView;
pub use panel::{AudioEdits, AudioView, MidiView, OutputStatus, PanelActions, PanelState};

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
    graph_view: graph_view::GraphView,
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
            graph_view: graph_view::GraphView::default(),
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
            && event.logical_key == winit::keyboard::Key::Character("g".into())
            && !self.ctx.egui_wants_keyboard_input()
        {
            self.graph_open = !self.graph_open;
            return true;
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
        if !self.visible && !self.graph_open {
            return Ok(PanelActions::default());
        }
        state.frame_times_ms = self.history.clone();

        let input = self.state.take_egui_input(window);
        // begin_pass/end_pass rather than run_ui: the panel builds its own
        // window from the context instead of drawing into a provided Ui.
        self.ctx.begin_pass(input);
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
        };

        let text = run_panel(&ctx, &reg, &state);

        // Every parameter's label must appear in the emitted text.
        for (_, def) in reg.iter() {
            let label = def.addr.trim_start_matches('/');
            assert!(text.contains(label), "no control drawn for {label}; got: {text}");
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


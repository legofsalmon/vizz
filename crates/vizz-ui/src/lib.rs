//! The vizz control panel: an egui overlay on the preview window.
//!
//! The GUI is a control-thread citizen like OSC — it writes parameter
//! targets into the shared [`ParamRegistry`] and reads health snapshots,
//! and has no privileged access to the renderer. Its draw happens inside
//! the same frame's command encoder, so it costs one extra render pass
//! and never introduces a synchronisation point.

pub mod graph_view;
pub mod notices;
pub mod grid_view;
pub mod panel;
pub mod theme;
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
pub use panel::{
    AudioEdits, AudioView, MidiView, OutputSetup, OutputStatus, PanelActions, PanelState,
    PresetEntry, RecordingView, VideoStatus,
};

/// Exposed for the offscreen preview, so the overlay is reviewed through
/// the same code the app runs rather than a copy of it.
pub fn draw_shortcuts_for_preview(ctx: &egui::Context, open: &mut bool) {
    shortcuts_overlay(ctx, open);
}

/// As above, for the quit confirmation — which appears over a running set
/// and so is worth looking at rather than only reasoning about.
pub fn draw_quit_prompt_for_preview(ctx: &egui::Context) {
    quit_prompt(ctx);
}

/// And the armed-learn banner, same reasoning.
pub fn draw_learn_banner_for_preview(ctx: &egui::Context, label: &str) {
    learn_banner(ctx, label);
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
                ("Space", "flash — white out while held"),
                ("Tab", "show or hide the control panel"),
                ("G", "modulation canvas"),
                ("P", "performance layout"),
                ("/", "filter the parameter list"),
                ("?", "this list"),
                ("F11", "fullscreen — Esc leaves it"),
                ("Esc", "quit — twice, to mean it"),
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
            // The mouse carries as many gestures as the keyboard, and they
            // were harder to find: none of these appeared anywhere but the
            // README (or nowhere at all) until this block.
            for (gesture, what) in [
                ("right-click", "reset a slider · menus on pads, presets and the canvas"),
                ("shift-click", "latch a punch button until the next click"),
                ("double-click", "rename a pad"),
                ("scroll", "zoom the modulation canvas"),
                ("Delete", "remove the selected canvas node"),
            ] {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [92.0, 18.0],
                        egui::Label::new(egui::RichText::new(gesture).strong().monospace()),
                    );
                    ui.label(what);
                });
            }
        });
}

/// "Press Escape again to quit."
///
/// Escape used to quit on the first press. On a laptop driving a projector
/// that is one stray keystroke — a hand on the wrong part of the keyboard,
/// a habit from dismissing something — between a running set and a black
/// screen with no way back. Nothing else in the app is destructive on one
/// key, and this was the most destructive thing in it.
///
/// Drawn centre-screen and drawn *whatever else is hidden*, because the
/// press it answers is one that would otherwise have ended the show.
fn quit_prompt(ctx: &egui::Context) {
    egui::Area::new(egui::Id::new("quit-prompt"))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(vizz_design::feedback::DANGER_BED)
                .inner_margin(egui::Margin::symmetric(22, 16))
                .corner_radius(6.0)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("press Esc again to quit")
                            .size(20.0)
                            .color(vizz_design::feedback::ON_DANGER),
                    );
                    ui.label(
                        egui::RichText::new("any other key carries on")
                            .size(13.0)
                            .color(vizz_design::feedback::ON_DANGER_DIM),
                    );
                });
        });
}

/// The armed-learn banner: a mode this global deserves an indicator this
/// global.
///
/// An armed learn binds the next control that moves, whichever screen is
/// up and whichever device sends it — armed from the panel, it survived
/// switching to the performance layout, hiding everything, even the
/// controller being unplugged and replugged, all with no on-screen sign
/// anywhere. The first stray knob touch then bound itself to the waiting
/// parameter. Returns true when clicked, which cancels the learn.
fn learn_banner(ctx: &egui::Context, label: &str) -> bool {
    let mut clicked = false;
    egui::Area::new(egui::Id::new("learn-banner"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -14.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let r = egui::Frame::NONE
                .fill(vizz_design::feedback::LEARN_BED)
                .stroke(egui::Stroke::new(1.0, theme::LEARN))
                .inner_margin(egui::Margin::symmetric(14, 8))
                .corner_radius(6.0)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "MIDI learn armed: the next control you move or press binds to {label} — click to cancel"
                        ))
                        .size(13.0)
                        .color(vizz_design::feedback::ON_LEARN_BED),
                    );
                })
                .response
                .interact(egui::Sense::click());
            clicked = r.clicked();
        });
    clicked
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
    /// Escape has been pressed once and is waiting for a second press.
    /// Owned by the app, which holds the timer; this is just told.
    pub quit_armed: bool,
    /// `/` was pressed; the panel focuses its filter next frame.
    pub focus_filter: bool,
    /// A number key was pressed: fire this preset slot.
    pub preset_key: Option<u32>,
    /// Space went down (`Some(true)`) or up (`Some(false)`) this frame:
    /// the flash gesture, taken by the app like `preset_key` so there is
    /// one path writing the parameter. Held state is tracked here so a
    /// release is only reported for a press this handler saw — a space
    /// typed into a text field must not end as a flash release.
    pub flash_key: Option<bool>,
    space_flashing: bool,
    graph_view: graph_view::GraphView,
    macros: vizz_mod::perform::Macros,
    /// Slider working ranges, loaded once and saved when they change.
    ranges: vizz_mod::ranges::Ranges,
    /// On-screen notices — the channel runtime failures report through.
    notices: notices::Notices,
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
            quit_armed: false,
            focus_filter: false,
            preset_key: None,
            flash_key: None,
            space_flashing: false,
            graph_view: graph_view::GraphView::default(),
            macros: vizz_mod::perform::Macros::load(),
            ranges: vizz_mod::ranges::Ranges::load(),
            notices: notices::Notices::default(),
            history: Vec::with_capacity(HISTORY),
        }
    }

    /// Something worked and is worth confirming on screen.
    /// The modulation canvas view, for persisting across launches. The
    /// canvas forgetting where you were — pan, zoom, the patch's name —
    /// made every launch start with a scavenger hunt.
    pub fn graph_view_memory(&self) -> graph_view::ViewMemory {
        self.graph_view.memory()
    }

    /// Restore a persisted canvas view. Sanitised inside — see
    /// [`GraphView::restore`].
    pub fn restore_graph_view(&mut self, m: graph_view::ViewMemory) {
        self.graph_view.restore(m);
    }

    pub fn notify_info(&mut self, text: impl Into<String>) {
        self.notices.info(text);
    }

    /// Something failed. This is the loud path: it draws whatever else is
    /// hidden, because a save failing with the panel closed is precisely
    /// the case a log line was silently eating.
    pub fn notify_error(&mut self, text: impl Into<String>) {
        self.notices.error(text);
    }

    /// Will `render` put anything on screen this frame?
    ///
    /// Asked *before* building the state to hand it, because gathering
    /// that state is not free — a health snapshot, the preset listing,
    /// both grids resolved, the MIDI map cloned, one float per parameter —
    /// and `render` returning early after all of it had been assembled
    /// meant paying for a panel nobody could see. Which is the state a set
    /// is actually played in.
    pub fn will_draw(&self) -> bool {
        self.visible
            || self.graph_open
            || self.performance
            || self.shortcuts_open
            || self.quit_armed
            || !self.notices.is_empty()
    }

    /// Feed a window event to egui. Returns `true` if egui consumed it,
    /// in which case the caller should not act on it (so dragging a
    /// slider does not also trigger app shortcuts).
    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        // Tab is ours whenever nothing is being typed into: the panel
        // must be dismissible even while egui has mouse focus. While a
        // text field IS focused, Tab goes to egui as the focus-next it
        // means there — it used to hide the entire panel mid-way through
        // typing a preset name.
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state.is_pressed()
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab)
            && !self.ctx.egui_wants_keyboard_input()
        {
            self.visible = !self.visible;
            if !self.visible {
                self.drop_text_focus();
            }
            return true;
        }
        // Space is the flash. Press and release both matter — it is the
        // one held gesture on the keyboard — so it is handled before the
        // pressed-only block below.
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space)
        {
            if event.state.is_pressed()
                && !event.repeat
                && !self.ctx.egui_wants_keyboard_input()
                && !self.space_flashing
            {
                self.space_flashing = true;
                self.flash_key = Some(true);
                return true;
            }
            if !event.state.is_pressed() && self.space_flashing {
                self.space_flashing = false;
                self.flash_key = Some(false);
                return true;
            }
        }
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state.is_pressed()
            && !self.ctx.egui_wants_keyboard_input()
        {
            match event.logical_key.as_ref() {
                // Case-insensitive: the ? overlay advertises these as "G"
                // and "P", and Caps Lock — or a Shift held for something
                // else — used to silently kill both.
                winit::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("g") => {
                    self.graph_open = !self.graph_open;
                    return true;
                }
                winit::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("p") => {
                    self.performance = !self.performance;
                    if !self.performance {
                        self.drop_text_focus();
                    }
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
        // Events flow to egui whenever anything is on screen — not only
        // when the *panel* is. Gating this on `visible` alone meant Tab
        // silently killed all mouse input to the performance layout and
        // the modulation canvas while both kept drawing: every pad,
        // fader and node looked live and responded to nothing.
        if !self.will_draw() {
            return false;
        }
        self.state.on_window_event(window, event).consumed
    }

    /// Take keyboard focus away from whatever text field holds it.
    ///
    /// Called when the screen that field lives on goes away. A field left
    /// focused behind a hidden panel kept `egui_wants_keyboard_input`
    /// true, which silently disabled the number keys and every letter
    /// shortcut — with nothing on screen to say why.
    fn drop_text_focus(&mut self) {
        if let Some(id) = self.ctx.memory(|m| m.focused()) {
            self.ctx.memory_mut(|m| m.surrender_focus(id));
        }
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
        // Callers are expected to check `will_draw` and skip assembling
        // the state entirely; this stays as the backstop that keeps the
        // two from disagreeing.
        if !self.will_draw() {
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
        if self.quit_armed {
            quit_prompt(&self.ctx);
        }
        self.notices.draw(&self.ctx);
        if self.performance {
            return self.render_performance(window, device, queue, encoder, target, registry, state, size_px);
        }
        let mut actions = if self.visible {
            panel::draw(&self.ctx, registry, &state, modulation, &mut self.ranges)
        } else {
            PanelActions::default()
        };
        if let Some(target) = &state.midi.learn_target
            && learn_banner(&self.ctx, &target.label)
        {
            actions.set_learn_target = Some(None);
        }
        if actions.ranges_changed && let Err(e) = self.ranges.save() {
            log::error!("could not save slider ranges: {e:#}");
            self.notices.error(format!("could not save the slider ranges: {e}"));
        }
        // The panel's button and the G key take the same door.
        if actions.open_canvas {
            self.graph_open = true;
        }
        if self.graph_open {
            let mut open = true;
            // "modulation canvas", not "modulation": the panel already has
            // a section by that name for the LFOs and routes, and the two
            // share nothing but the engine underneath. One word, two
            // unrelated surfaces, is how G "opens the wrong thing".
            egui::Window::new("modulation canvas")
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
            recording: state.recording,
            outputs: &state.outputs,
            audio: &state.audio,
            fps: health.map(|h| h.fps).unwrap_or(0.0),
            over_budget: health.map(|h| h.over_budget_window_pct > 1.0).unwrap_or(false),
            bpm: state.bpm,
            bar_phase: state.bar_phase,
            presets: &preset_names,
            preset_current: state.preset_current,
            grid: &state.grid,
            // Only shown when the layer is in use.
            gravity: state.gravity_grid.as_ref(),
            midi: &state.midi,
            values: (!state.modulated.is_empty()).then_some(&state.modulated[..]),
        };
        let mut perf = performance::draw(&self.ctx, registry, &perf_state, &mut self.macros);
        // The armed-learn banner rides both screens; see the panel path.
        if let Some(target) = &state.midi.learn_target
            && learn_banner(&self.ctx, &target.label)
        {
            perf.set_learn_target = Some(None);
        }
        if perf.exit {
            self.performance = false;
        }
        if perf.macros_changed && let Err(e) = self.macros.save() {
            self.notices.error(format!("could not save the fader assignments: {e}"));
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
        // The grid on the performance layout drives the same actions the
        // panel's does, so storing a scene mid-set works from either.
        actions.grid = perf.grid;
        actions.gravity = perf.gravity;
        // Learn and unbind are handled identically to the panel's, so a
        // controller mapped from the performance layout and one mapped
        // from the parameter list end up in the same map by the same path.
        actions.set_learn_target = perf.set_learn_target;
        actions.clear_binding = perf.clear_binding;
        actions.clear_slot_binding = perf.clear_slot_binding;
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
            recording: None,
            preset_current: None,
            update_available: None,
            health: None,
            outputs: Vec::new(),
            frame_times_ms: Vec::new(),
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            video: None,
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            modulated: Vec::new(),
            clouds: Vec::new(),
            palettes: Vec::new(),
            gravity_grid: None,
            output: Default::default(),
            bpm: 120.0,
            presets: Vec::new(),
            focus_filter: false,
            grid: Default::default(),
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
            recording: None,
            preset_current: None,
            update_available: None,
            health: None,
            outputs: Vec::new(),
            frame_times_ms: Vec::new(),
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            video: None,
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            modulated: Vec::new(),
            clouds: Vec::new(),
            palettes: Vec::new(),
            gravity_grid: None,
            output: Default::default(),
            bpm: 120.0,
            focus_filter: false,
            grid: Default::default(),
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
            recording: None,
            preset_current: None,
            update_available: None,
            health: None,
            outputs: vec![OutputStatus { name: "syphon:vizz".into(), live: true }],
            frame_times_ms: vec![16.0, 17.0, 15.5],
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            video: None,
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            modulated: Vec::new(),
            clouds: Vec::new(),
            palettes: Vec::new(),
            gravity_grid: None,
            output: Default::default(),
            bpm: 120.0,
            presets: Vec::new(),
            focus_filter: false,
            grid: Default::default(),
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
            recording: None,
            preset_current: None,
            update_available: None,
            health: None,
            outputs: vec![],
            frame_times_ms: vec![],
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            video: None,
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            modulated: Vec::new(),
            clouds: Vec::new(),
            palettes: Vec::new(),
            gravity_grid: None,
            output: Default::default(),
            bpm: 120.0,
            presets: Vec::new(),
            focus_filter: false,
            grid: Default::default(),
            expand_sections: true,
            bar_phase: 0.0,
        };
        let text = run_panel(&ctx, &reg, &state);
        assert!(text.contains("collecting health data"), "got: {text}");
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
            recording: None,
            preset_current: None,
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
                raw_peak: [0.21, 0.16, 0.08, 0.02],
                level: 0.2,
                detected_bpm: 128.0,
                confidence: 0.7,
                dropped: 0,
                clock_midi: false,
                clock_ticking: false,
            },
            video: None,
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: true,
            modulated: Vec::new(),
            clouds: Vec::new(),
            palettes: Vec::new(),
            gravity_grid: None,
            output: Default::default(),
            bpm: 128.0,
            presets: Vec::new(),
            focus_filter: false,
            grid: Default::default(),
            expand_sections: true,
            bar_phase: 0.05,
        };
        let text = run_panel(&ctx, &reg, &state);
        assert!(text.contains("Scarlett 2i2"), "device missing: {text}");
        assert!(text.contains("128.0 bpm"), "detected tempo missing: {text}");
        assert!(text.contains("tap"), "tap tempo missing: {text}");
        // Band edges are the filter control; they must be editable numbers.
        // The value and its unit are separate glyph runs, so they are
        // matched separately rather than as one string.
        assert!(text.contains("30") && text.contains("110"), "band edges missing: {text}");
        assert!(text.contains("Hz"), "band edge unit missing: {text}");
        // Sensitivity reads in decibels, at the value the band actually
        // carries. "×10" is not a quantity anyone can act on, and it was
        // the thing that made the gain control look like it meant nothing.
        // Asserting the number as well as the unit ties this to the
        // shipped defaults, so raising one without the other is caught.
        //
        // Whitespace is collapsed first: egui emits a spin box's number
        // and its suffix as separate galleys, so the collected text has
        // them a couple of spaces apart rather than as one string.
        let squashed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let first_band_db = format!("{:.1} dB", vizz_audio::default_bands()[0].gain_db());
        assert!(
            squashed.contains(&first_band_db),
            "gain is not shown as {first_band_db}: {text}"
        );
        assert!(text.contains("fit"), "no way to set the gains from the input: {text}");

        // Disconnected must say so and explain the fix rather than showing
        // four dead meters. The fix used to be a command-line flag, which
        // meant quitting and restarting to change soundcard; it is a
        // picker now, so the panel points at that instead.
        state.audio = AudioView::default();
        let text = run_panel(&ctx, &reg, &state);
        assert!(text.contains("no input"), "got: {text}");
        assert!(
            text.contains("pick an input"),
            "no hint about finding a device: {text}"
        );
        assert!(
            text.contains("not capturing"),
            "a dead input must not look live: {text}"
        );
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
            recording: None,
            preset_current: None,
            update_available: None,
            health: None,
            outputs: vec![],
            frame_times_ms: vec![],
            frame_budget_ms: 16.67,
            midi: MidiView {
                revision: 0,
                available: true,
                connected: vec!["Launch Control XL".into()],
                map,
                learn_target: Some(vizz_midi::LearnTarget::param("/particles/count")),
                last_source: Some(vizz_midi::Source::Note { channel: 9, note: 36 }),
                clock_bpm: None,
                clock_started: false,
            },
            audio: AudioView::default(),
            video: None,
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            modulated: Vec::new(),
            clouds: Vec::new(),
            palettes: Vec::new(),
            gravity_grid: None,
            output: Default::default(),
            bpm: 120.0,
            presets: Vec::new(),
            focus_filter: false,
            grid: Default::default(),
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
            recording: None,
            preset_current: None,
            update_available: update,
            health: None,
            outputs: vec![],
            frame_times_ms: vec![],
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            video: None,
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            modulated: Vec::new(),
            clouds: Vec::new(),
            palettes: Vec::new(),
            gravity_grid: None,
            output: Default::default(),
            bpm: 120.0,
            presets: Vec::new(),
            focus_filter: false,
            grid: Default::default(),
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
    /// Three details this has to get right, all learned the hard way.
    /// Without a `screen_rect` egui clips the whole window away and emits
    /// nothing. The first pass over a freshly-created Window only measures
    /// it — real shapes appear on the second; the live app renders
    /// continuously so it never notices, but a one-shot test does. And the
    /// screen has to be tall enough for the whole panel with every section
    /// open at once, which no real window ever is: an egui window sizes to
    /// its content and anything past the bottom edge is not drawn at all,
    /// so a short screen here reports controls as missing that merely sit
    /// below the fold.
    fn run_panel(ctx: &egui::Context, reg: &ParamRegistry, state: &PanelState) -> String {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(900.0, 1000.0),
            )),
            ..Default::default()
        };
        let mut text = String::new();
        for _ in 0..2 {
            ctx.begin_pass(input.clone());
            let _ = panel::draw(ctx, reg, state, &mut vizz_mod::ModEngine::with_defaults(), &mut Default::default());
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

    /// Every rect-stroke colour the panel painted. The clock advances
    /// across passes because a fresh Window fades in, and mid-fade every
    /// colour is alpha-scaled into something no equality check knows.
    fn panel_stroke_colours(reg: &ParamRegistry, state: &PanelState) -> Vec<egui::Color32> {
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        for i in 0..6 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(900.0, 1000.0),
                )),
                time: Some(i as f64 * 0.2),
                ..Default::default()
            });
            let _ = panel::draw(
                &ctx,
                reg,
                state,
                &mut vizz_mod::ModEngine::with_defaults(),
                &mut Default::default(),
            );
            fn walk(shape: &egui::Shape, out: &mut Vec<egui::Color32>) {
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

    /// The panel's preset list marks the recalled slot the same way the
    /// stage row does — same question ("where did the look on screen come
    /// from"), same blue. It used to be marked on one screen and not the
    /// other, which read as the panel forgetting.
    #[test]
    fn the_recalled_preset_is_outlined_in_the_panel_list_too() {
        let reg = registry();
        let state = |current: Option<usize>| PanelState {
            recording: None,
            preset_current: current,
            update_available: None,
            health: None,
            outputs: Vec::new(),
            frame_times_ms: Vec::new(),
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            video: None,
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            modulated: Vec::new(),
            clouds: Vec::new(),
            palettes: Vec::new(),
            gravity_grid: None,
            output: Default::default(),
            bpm: 120.0,
            focus_filter: false,
            grid: Default::default(),
            expand_sections: true,
            presets: vec![
                PresetEntry { name: "Slow bloom".into(), builtin: true, about: None },
                PresetEntry { name: "Warehouse 2".into(), builtin: false, about: None },
            ],
            bar_phase: 0.0,
        };
        assert!(
            !panel_stroke_colours(&reg, &state(None)).contains(&theme::CURRENT),
            "nothing recalled, yet something wears the current-preset stroke"
        );
        assert!(
            panel_stroke_colours(&reg, &state(Some(2))).contains(&theme::CURRENT),
            "the recalled preset is not marked in the panel list"
        );
    }

    /// The status strip only mentions video once a source is configured:
    /// a permanent "no video" dot would alarm about an absence nobody
    /// chose. With one configured, its name and its health belong on the
    /// strip exactly as audio's do.
    #[test]
    fn the_status_strip_shows_video_only_when_a_source_exists() {
        let reg = registry();
        let ctx = egui::Context::default();
        let mut state = PanelState {
            recording: None,
            preset_current: None,
            update_available: None,
            health: None,
            outputs: Vec::new(),
            frame_times_ms: Vec::new(),
            frame_budget_ms: 16.67,
            midi: MidiView::default(),
            audio: AudioView::default(),
            video: None,
            audio_bands: vizz_audio::default_bands(),
            audio_auto_bpm: false,
            modulated: Vec::new(),
            clouds: Vec::new(),
            palettes: Vec::new(),
            gravity_grid: None,
            output: Default::default(),
            bpm: 120.0,
            focus_filter: false,
            grid: Default::default(),
            expand_sections: false,
            presets: Vec::new(),
            bar_phase: 0.0,
        };
        let without = run_panel(&ctx, &reg, &state);
        assert!(
            !without.contains("ndi:cam"),
            "a video label appeared with no source configured: {without}"
        );
        state.video = Some(VideoStatus { connected: true, label: "ndi:cam".into() });
        let ctx = egui::Context::default();
        let with = run_panel(&ctx, &reg, &state);
        assert!(with.contains("ndi:cam"), "the video source is not on the strip: {with}");
    }
}


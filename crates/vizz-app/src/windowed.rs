//! Windowed mode: winit event loop driving the frame engine at vsync.
//!
//! Scenes render into the fixed-resolution master [`OutputTarget`]; the
//! window only shows an aspect-fitted preview of it. Resizing the window
//! never changes what receivers (Syphon/Spout/NDI) see.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use vizz_render::{GpuContext, blit::BlitPass, output::OutputTarget, particles::ParticleScene};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use vizz_midi::{MidiEngine, SharedMidi};
use vizz_update::SharedUpdate;
use vizz_ui::{Gui, MidiView, OutputStatus, PanelState};

use crate::engine::FrameEngine;
use crate::outputs::{self, OutputOpts};
use crate::params::AppParams;

pub struct WindowedOpts {
    pub width: u32,
    pub height: u32,
    pub show_gui: bool,
    /// Check GitHub for a newer release once at startup.
    pub check_updates: bool,
    /// Where MIDI mappings are persisted.
    pub midi_map_path: std::path::PathBuf,
    /// Window title; shows OSC port etc. so double-click users can see
    /// where to point a controller without reading logs.
    pub title: String,
    pub outputs: OutputOpts,
}

struct RenderState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    ctx: GpuContext,
    scene: ParticleScene,
    output: OutputTarget,
    blit: BlitPass,
    blit_bind: wgpu::BindGroup,
    senders: Vec<Box<dyn vizz_io::FrameSender>>,
    gui: Gui,
}

struct App {
    engine: FrameEngine,
    opts: WindowedOpts,
    state: Option<RenderState>,
    /// Shared with OSC; the panel writes targets through it just as the
    /// OSC listener does.
    params: Arc<AppParams>,
    midi: Option<MidiEngine>,
    midi_shared: SharedMidi,
    /// Last MIDI snapshot the panel saw. Refreshed with try_lock so a
    /// busy MIDI thread can never stall the render thread.
    midi_view: MidiView,
    /// Revision last written to disk, so saves happen only on change.
    saved_revision: u64,
    update: SharedUpdate,
}

impl App {
    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<RenderState> {
        let attrs = Window::default_attributes()
            .with_title(self.opts.title.clone())
            .with_inner_size(LogicalSize::new(self.opts.width, self.opts.height));
        let window = Arc::new(event_loop.create_window(attrs)?);

        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(window.clone())?;
        let ctx = pollster::block_on(async {
            // Reuse the instance the surface came from.
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                    apply_limit_buckets: false,
                })
                .await?;
            let info = adapter.get_info();
            log::info!(
                "GPU: {} ({:?}, {:?} backend)",
                info.name,
                info.device_type,
                info.backend
            );
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("vizz-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: wgpu::Trace::Off,
                })
                .await?;
            anyhow::Ok(GpuContext {
                instance,
                adapter,
                device,
                queue,
            })
        })?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&ctx.adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        // Fifo = vsync: never tear on the output projector.
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&ctx.device, &config);

        // Scenes draw into the master target at the fixed output resolution;
        // the swapchain only ever sees the preview blit.
        let output = OutputTarget::new(&ctx.device, self.opts.width, self.opts.height);
        let scene = ParticleScene::new(&ctx, vizz_render::output::OUTPUT_FORMAT);
        let blit = BlitPass::new(&ctx.device, config.format);
        let blit_bind = blit.bind(&ctx.device, &output.view);
        let senders = outputs::build_senders(&ctx.device, &self.opts.outputs);
        let mut gui = Gui::new(&window, &ctx.device, config.format);
        gui.visible = self.opts.show_gui;

        Ok(RenderState {
            window,
            surface,
            config,
            ctx,
            scene,
            output,
            blit,
            blit_bind,
            senders,
            gui,
        })
    }

    fn redraw(&mut self) {
        let Some(state) = &mut self.state else { return };
        let frame_start = Instant::now();

        use wgpu::CurrentSurfaceTexture as Cst;
        let frame = match state.surface.get_current_texture() {
            Cst::Success(frame) => frame,
            Cst::Suboptimal(frame) => {
                // Usable but stale (e.g. mid-resize): draw it, reconfigure after.
                state.surface.configure(&state.ctx.device, &state.config);
                frame
            }
            Cst::Lost | Cst::Outdated => {
                // Resize/display change: reconfigure and try again next frame.
                state.surface.configure(&state.ctx.device, &state.config);
                state.window.request_redraw();
                return;
            }
            Cst::Timeout | Cst::Occluded => {
                // Skip the frame; keep the loop alive so we recover when
                // the window is visible again.
                state.window.request_redraw();
                return;
            }
            Cst::Validation => {
                log::error!("surface validation error — skipping frame");
                state.window.request_redraw();
                return;
            }
        };

        let inputs = self.engine.begin_frame(state.output.aspect(), None);
        let preview = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = state
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        state.scene.render(
            &state.ctx,
            &mut encoder,
            &state.output.view,
            &inputs.uniforms,
            inputs.count,
        );
        state.blit.draw(
            &mut encoder,
            &preview,
            &state.blit_bind,
            state.output.aspect(),
            state.config.width,
            state.config.height,
        );
        // The panel composites over the preview, inside the same encoder,
        // so it costs one extra pass and no synchronisation point.
        let outputs_status: Vec<OutputStatus> = state
            .senders
            .iter()
            .map(|s| OutputStatus { name: s.name().to_owned(), live: true })
            .collect();
        refresh_midi_view(&self.midi, &self.midi_shared, &mut self.midi_view);
        let panel_state = PanelState {
            // try_lock: the update thread holds this for microseconds, but
            // the render thread still never waits on it.
            update_available: self
                .update
                .try_lock()
                .ok()
                .and_then(|u| u.available.map(|v| v.to_string())),
            health: Some(self.engine.health.snapshot()),
            outputs: outputs_status,
            frame_times_ms: Vec::new(),
            frame_budget_ms: 1000.0 / 60.0,
            midi: self.midi_view.clone(),
        };
        let actions = state.gui.render(
            &state.window,
            &state.ctx.device,
            &state.ctx.queue,
            &mut encoder,
            &preview,
            &self.params.registry,
            panel_state,
            &mut self.engine.modulation,
            [state.config.width, state.config.height],
        );
        match actions {
            Ok(actions) => apply_panel_actions(
                actions,
                &self.midi_shared,
                &self.opts.midi_map_path,
                &mut self.saved_revision,
            ),
            // A GUI failure must never take down the output.
            Err(e) => log::error!("GUI draw failed: {e:#}"),
        }

        state.ctx.queue.submit([encoder.finish()]);

        // After submit: senders enqueue work ordered behind this frame.
        outputs::publish_all(
            &mut state.senders,
            &state.ctx.device,
            &state.ctx.queue,
            &state.output.texture,
        );

        state.window.pre_present_notify();
        state.ctx.queue.present(frame);

        let elapsed = frame_start.elapsed();
        state.gui.push_frame_time(elapsed.as_secs_f32() * 1e3);
        if let Some(snap) = self.engine.end_frame(elapsed) {
            log::info!("{}", snap.log_line());
        }
        state.window.request_redraw();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match self.init(event_loop) {
            Ok(state) => {
                state.window.request_redraw();
                self.state = Some(state);
            }
            Err(e) => {
                log::error!("failed to initialize window/GPU: {e:#}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // The panel sees events first; if it used one (dragging a slider,
        // typing in a field) the app must not also act on it.
        if let Some(state) = &mut self.state {
            let window = Arc::clone(&state.window);
            if state.gui.on_window_event(&window, &event) && !matches!(event, WindowEvent::RedrawRequested) {
                window.request_redraw();
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. }
                if event.logical_key == Key::Named(NamedKey::Escape) && event.state.is_pressed() =>
            {
                event_loop.exit()
            }
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state
                    && size.width > 0
                    && size.height > 0
                {
                    state.config.width = size.width;
                    state.config.height = size.height;
                    state.surface.configure(&state.ctx.device, &state.config);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }
}

/// Copy MIDI state for the panel. Non-blocking by design: if the MIDI
/// thread holds the lock we reuse the previous snapshot, which is at most
/// one frame stale — the render thread must never wait on MIDI traffic.
fn refresh_midi_view(midi: &Option<MidiEngine>, shared: &SharedMidi, view: &mut MidiView) {
    if midi.is_none() {
        return;
    }
    let Ok(state) = shared.try_lock() else { return };
    view.available = true;
    view.connected = state.connected.clone();
    view.map = state.map.clone();
    view.learn_target = state.learn_target.clone();
    view.last_source = state.last_source;
}

fn apply_panel_actions(
    actions: vizz_ui::PanelActions,
    shared: &SharedMidi,
    map_path: &std::path::Path,
    saved_revision: &mut u64,
) {
    if actions.set_learn_target.is_none() && actions.clear_binding.is_none() {
        return;
    }
    let Ok(mut state) = shared.lock() else { return };
    if let Some(target) = actions.set_learn_target {
        state.learn_target = target;
    }
    if let Some(param) = actions.clear_binding {
        state.map.unbind_param(&param);
        state.revision += 1;
    }
    // Persist as soon as a mapping changes: a crash mid-set should not
    // cost the mappings that were just set up.
    if state.revision != *saved_revision {
        let (map, revision) = (state.map.clone(), state.revision);
        drop(state);
        match vizz_midi::save_map(map_path, &map) {
            Ok(()) => *saved_revision = revision,
            Err(e) => log::error!("could not save MIDI map: {e:#}"),
        }
    }
}

pub fn run(params: Arc<AppParams>, opts: WindowedOpts) -> Result<()> {
    // MIDI mappings load before the engine starts so a learned setup is
    // live from the first frame.
    let map = match vizz_midi::load_map(&opts.midi_map_path) {
        Ok(map) => map,
        Err(e) => {
            log::error!("could not load MIDI map: {e:#} — starting with none");
            vizz_midi::MidiMap::default()
        }
    };
    let midi_shared: SharedMidi = Arc::new(std::sync::Mutex::new(vizz_midi::MidiState {
        map,
        ..Default::default()
    }));
    let midi = match MidiEngine::spawn(Arc::clone(&params.registry), Arc::clone(&midi_shared)) {
        Ok(engine) => Some(engine),
        // No MIDI is a degraded mode, not a failure: visuals and OSC run.
        Err(e) => {
            log::warn!("MIDI unavailable: {e:#}");
            None
        }
    };

    let update: SharedUpdate = Arc::new(std::sync::Mutex::new(Default::default()));
    if opts.check_updates {
        vizz_update::spawn_check(Arc::clone(&update));
    }

    let event_loop = EventLoop::new()?;
    // Poll: we drive redraws ourselves; vsync provides the pacing.
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        engine: FrameEngine::new(Arc::clone(&params)),
        params,
        opts,
        state: None,
        midi,
        midi_shared,
        midi_view: MidiView::default(),
        saved_revision: 0,
        update,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

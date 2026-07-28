//! Windowed mode: winit event loop driving the frame engine at vsync.
//!
//! Scenes render into the fixed-resolution master [`OutputTarget`]; the
//! window only shows an aspect-fitted preview of it. Resizing the window
//! never changes what receivers (Syphon/Spout/NDI) see.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use vizz_render::{GpuContext, blit::BlitPass, output::OutputTarget, particles::ParticleScene, post::PostChain};
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
    /// Substring match against an input device name; None picks the default.
    pub audio_device: Option<String>,
    /// Point clouds to load into the loadable slots, in order.
    pub clouds: Vec<std::path::PathBuf>,
    /// Live point-cloud stream feeding the last slot, if any.
    pub live_cloud: Option<vizz_render::plystream::Source>,
}

struct RenderState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    ctx: GpuContext,
    scene: ParticleScene,
    room: vizz_render::room::Room,
    output: OutputTarget,
    blit: BlitPass,
    blit_bind: wgpu::BindGroup,
    senders: Vec<Box<dyn vizz_io::FrameSender>>,
    post: PostChain,
    gui: Gui,
    /// Eight-bit copy of the master, present only when the master is
    /// wide. Syphon and NDI are BGRA8 by definition, so a float master has
    /// to be converted before it can leave — and doing that here, once,
    /// keeps every sender unaware that the option exists.
    publish: Option<OutputTarget>,
    /// Blit used for that conversion, with its bind group.
    publish_blit: Option<(BlitPass, wgpu::BindGroup)>,
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
    /// Cached preset listing. The panel and both grids ask what is in the
    /// library every frame; answering from disk made that dozens of file
    /// operations on the render thread per frame.
    library: vizz_mod::preset::Library,
    /// Revision last written to disk, so saves happen only on change.
    saved_revision: u64,
    update: SharedUpdate,
    /// Panel-side mirror of the analysis settings. The panel edits this
    /// copy and the change is pushed to the analysis thread once, rather
    /// than locking its settings every frame just to draw.
    audio_bands: [vizz_audio::Band; 4],
    audio_auto_bpm: bool,
    tap: vizz_audio::TapTempo,
    /// `/` was pressed this frame: focus the panel's parameter filter.
    /// One-shot, cleared after the panel has drawn.
    focus_filter: bool,
    /// Live point-cloud stream, if one was configured.
    live: Option<vizz_render::plystream::LiveCloud>,
    /// Revision last uploaded, so an unchanged stream costs nothing.
    live_revision: u64,
    /// Paths currently in the loadable cloud slots, in slot order.
    /// Mirrors what is on the GPU so a drop can be persisted without
    /// asking the renderer what it is holding.
    clouds: Vec<String>,
    /// Which loadable slot the next drop fills. Round-robin, so dropping
    /// repeatedly cycles through the slots rather than always replacing
    /// the same one — the point of having two is comparing them.
    next_cloud: usize,
    /// Palette files loaded this session, in order, for persistence.
    palettes: Vec<String>,
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

        // Render size and output size are separate things.
        //
        // The output is what receivers get and what the aspect is judged
        // against. The render size is how large the scene and the post
        // chain actually work, and above 1× the downscale into the master
        // is free anti-aliasing — which is the only thing that reliably
        // cleans up a field of one-pixel sprites. Below 1× it buys frame
        // rate on a machine that cannot hold the budget.
        let s = crate::settings::load();
        let [ow, oh] = s.output_or([self.opts.width, self.opts.height]);
        let [rw, rh] = s.render_size([ow, oh]);
        let master_format = if s.wide_output {
            vizz_render::output::WIDE_FORMAT
        } else {
            vizz_render::output::OUTPUT_FORMAT
        };
        log::info!(
            "output {ow}x{oh} ({}), rendering at {rw}x{rh} ({:.2}x)",
            if s.wide_output { "16-bit float" } else { "8-bit" },
            s.scale()
        );
        let output = OutputTarget::with_format(&ctx.device, ow, oh, master_format);
        let post = PostChain::new(&ctx, rw, rh, master_format);
        // The scene draws into the post chain's HDR buffer, not straight
        // to the master: feedback needs somewhere to accumulate.
        let mut scene = ParticleScene::new(&ctx, vizz_render::post::SCENE_FORMAT);
        scene.load_clouds(&ctx, &self.opts.clouds);
        // Palettes come back in the order they were dropped, so the
        // indices a preset saved still point at the same colours.
        for path in &self.palettes {
            if let Err(e) = scene.load_palette(&ctx, std::path::Path::new(path)) {
                log::warn!("could not reload palette {path}: {e:#}");
            }
        }
        // A stream that will not start is a warning, never a startup
        // failure — the same trade as a cloud file that will not parse.
        if let Some(source) = self.opts.live_cloud.clone() {
            match vizz_render::plystream::LiveCloud::start(source) {
                Ok(live) => {
                    log::info!("live cloud: {}", live.label());
                    self.live = Some(live);
                }
                Err(e) => log::warn!("could not start the live cloud: {e:#}"),
            }
        }
        let room = vizz_render::room::Room::new(&ctx, vizz_render::post::SCENE_FORMAT);
        let blit = BlitPass::new(&ctx.device, config.format);
        let blit_bind = blit.bind(&ctx.device, &output.view);
        // Only allocated when it is needed: an eight-bit master is already
        // publishable, and a second full-size texture is not something to
        // carry for a setting that is off.
        let (publish, publish_blit) = if output.publishable() {
            (None, None)
        } else {
            let target = OutputTarget::new(&ctx.device, ow, oh);
            let pass = BlitPass::new(&ctx.device, vizz_render::output::OUTPUT_FORMAT);
            let bind = pass.bind(&ctx.device, &output.view);
            (Some(target), Some((pass, bind)))
        };
        // Senders describe the stream up front (NDI sizes its ring from
        // this), so they must be told the size the master actually is —
        // not the one the command line asked for. When settings override
        // the size, a ring sized from the CLI value is smaller than the
        // texture being copied into it, and `ReadbackRing::capture` copies
        // a fixed extent with no bounds check.
        self.opts.outputs.width = ow;
        self.opts.outputs.height = oh;
        let senders = outputs::build_senders(&ctx.device, &self.opts.outputs);
        // The title is the one place a performer checks what is going out.
        window.set_title(&format!("{} — {ow}x{oh}", self.opts.title));
        let mut gui = Gui::new(&window, &ctx.device, config.format);
        gui.visible = self.opts.show_gui;

        Ok(RenderState {
            window,
            surface,
            config,
            ctx,
            scene,
            room,
            output,
            blit,
            blit_bind,
            senders,
            post,
            gui,
            publish,
            publish_blit,
        })
    }

    /// Rebuild the master, the post chain and the publish path.
    ///
    /// Done between frames, not during one: every texture here is bound
    /// into pipelines that this frame's encoder may already reference, and
    /// swapping one mid-frame is a use-after-free the validation layer
    /// catches and a release build does not.
    ///
    /// Rebuilding rather than requiring a restart because output
    /// resolution is a thing you get wrong once at a venue, and finding
    /// out means relaunching into whatever the app opens with.
    fn apply_output_setup(&mut self, setup: vizz_ui::OutputSetup) {
        let Some(state) = &mut self.state else { return };
        let ow = setup.width.clamp(crate::settings::MIN_DIM, crate::settings::MAX_DIM);
        let oh = setup.height.clamp(crate::settings::MIN_DIM, crate::settings::MAX_DIM);
        let scale = setup
            .scale
            .clamp(crate::settings::MIN_SCALE, crate::settings::MAX_SCALE);
        let rw = ((ow as f32 * scale) as u32).clamp(crate::settings::MIN_DIM, crate::settings::MAX_DIM);
        let rh = ((oh as f32 * scale) as u32).clamp(crate::settings::MIN_DIM, crate::settings::MAX_DIM);
        let format = if setup.wide {
            vizz_render::output::WIDE_FORMAT
        } else {
            vizz_render::output::OUTPUT_FORMAT
        };

        state.output = OutputTarget::with_format(&state.ctx.device, ow, oh, format);
        state.post = PostChain::new(&state.ctx, rw, rh, format);
        state.blit_bind = state.blit.bind(&state.ctx.device, &state.output.view);
        let (publish, publish_blit) = if state.output.publishable() {
            (None, None)
        } else {
            let target = OutputTarget::new(&state.ctx.device, ow, oh);
            let pass = BlitPass::new(&state.ctx.device, vizz_render::output::OUTPUT_FORMAT);
            let bind = pass.bind(&state.ctx.device, &state.output.view);
            (Some(target), Some((pass, bind)))
        };
        state.publish = publish;
        state.publish_blit = publish_blit;

        // Rebuild the senders as well. This was the omission: the master,
        // the post chain and the publish path were all rebuilt and the
        // senders were not, so NDI kept a ring sized for the old
        // resolution — copied into with no bounds check, which is a hard
        // panic when the master grows — and Syphon kept publishing a
        // texture nobody was rendering into any more, which reads as the
        // output having frozen.
        //
        // Dropping the old senders first releases the Syphon server name
        // and the NDI sender before the replacements claim them; receivers
        // see the source drop and reappear, which is the honest signal
        // that the stream's size changed.
        self.opts.outputs.width = ow;
        self.opts.outputs.height = oh;
        state.senders.clear();
        state.senders = outputs::build_senders(&state.ctx.device, &self.opts.outputs);
        state.window.set_title(&format!("{} — {ow}x{oh}", self.opts.title));

        let mut s = crate::settings::load();
        s.output_size = Some([ow, oh]);
        s.render_scale = Some(scale);
        s.wide_output = setup.wide;
        if let Err(e) = crate::settings::save(&s) {
            log::warn!("could not remember the output setup: {e:#}");
        }
        log::info!(
            "output now {ow}x{oh} ({}), rendering at {rw}x{rh} ({scale:.2}x)",
            if setup.wide { "16-bit float" } else { "8-bit" }
        );
    }

    /// Route a dropped file by what it is.
    ///
    /// One gesture for both, dispatched on extension, because "put this
    /// file into vizz" is the same intent whether the file is geometry or
    /// colour, and asking the user to remember two different ways to do it
    /// would be a distinction that serves the implementation.
    fn load_dropped(&mut self, path: std::path::PathBuf) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match ext.as_str() {
            "ply" | "xyz" | "pts" => self.load_dropped_cloud(path),
            "gpl" | "hex" | "txt" => self.load_dropped_palette(path),
            // `.csv` is both a point cloud and a plausible palette export.
            // Geometry wins: it is the one this app has always taken, and
            // a palette that arrives as a cloud is obvious immediately
            // whereas the reverse silently recolours the scene.
            "csv" => self.load_dropped_cloud(path),
            other => log::warn!("nothing to do with a .{other} file"),
        }
    }

    fn load_dropped_palette(&mut self, path: std::path::PathBuf) {
        let Some(state) = &mut self.state else { return };
        match state.scene.load_palette(&state.ctx, &path) {
            Ok((name, row)) => {
                // Select it, for the same reason a dropped cloud is
                // selected: a palette you cannot see has not arrived.
                let p = &*self.params;
                p.registry.set(p.palette, row as f32);
                self.palettes.push(path.display().to_string());
                if let Err(e) = crate::settings::save_palettes(&self.palettes) {
                    log::warn!("could not remember the loaded palettes: {e:#}");
                }
                log::info!("palette {name} is now selected");
            }
            Err(e) => log::warn!("could not load {}: {e:#}", path.display()),
        }
    }

    fn load_dropped_cloud(&mut self, path: std::path::PathBuf) {
        let Some(state) = &mut self.state else { return };
        let Some(slot) = ParticleScene::loadable_slot(self.next_cloud) else {
            return;
        };
        match state.scene.load_cloud(&state.ctx, slot, &path) {
            Ok(name) => {
                // A dropped file that will not parse is a warning, never a
                // crash — the same trade the command-line path makes.
                // Arriving at a venue and having the app die because a
                // scan has a malformed header is the wrong failure.
                let text = path.display().to_string();
                if self.clouds.len() <= self.next_cloud {
                    self.clouds.resize(self.next_cloud + 1, String::new());
                }
                self.clouds[self.next_cloud] = text;
                self.next_cloud = (self.next_cloud + 1) % ParticleScene::LOADABLE;
                if let Err(e) = crate::settings::save_clouds(&self.clouds) {
                    log::warn!("could not remember the loaded clouds: {e:#}");
                }
                log::info!("loaded {name} into cloud slot {slot}");
                // Point the shape at what just arrived, so a drop shows
                // something. Loading a cloud nobody can see is the same as
                // not loading it, and hunting for the slot index
                // afterwards is exactly the fiddling a drop avoids.
                let p = &*self.params;
                p.registry.set(p.cloud_a, slot as f32);
                p.registry.set(p.cloud_morph, 0.0);
            }
            Err(e) => log::warn!("could not load {}: {e:#}", path.display()),
        }
    }

    fn redraw(&mut self) {
        let Some(state) = &mut self.state else { return };
        let frame_start = Instant::now();
        // Upload a new stream frame before drawing, and only when the
        // revision moved: re-uploading an unchanged cloud every frame
        // would cost a texture write for nothing.
        if let Some(live) = &self.live {
            let revision = live.revision();
            if revision != self.live_revision {
                let uploaded = live.with_latest(|points| {
                    state.scene.set_cloud(
                        &state.ctx,
                        ParticleScene::LIVE_SLOT,
                        points,
                        "live",
                    );
                });
                // Only advance when the slot was actually free; otherwise
                // this frame is skipped and the next one retries.
                if uploaded.is_some() {
                    self.live_revision = revision;
                }
            }
        }

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
        // Room first: it clears the scene texture and the particles then
        // add on top. Skipped entirely when dark, since it is off by
        // default and drawing invisible lines is wasted work.
        if inputs.room_visible {
            state
                .room
                .render(&state.ctx, &mut encoder, &state.post.scene_view, &inputs.room, inputs.background);
        }
        state.scene.render(
            &state.ctx,
            &mut encoder,
            &state.post.scene_view,
            &inputs.uniforms,
            inputs.count,
            !inputs.room_visible,
            inputs.background,
        );
        state.post.render(&state.ctx, &mut encoder, &state.output.view, &inputs.post);
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
        // Picks up presets added behind the app's back — dropped into the
        // folder, or synced in. Anything the app writes refreshes this
        // directly, so the interval only has to catch what it did not do
        // itself.
        self.library.tick();
        // Collected before the panel draws: the render call takes
        // `&mut state.gui` and reading `state.scene` inside its argument
        // list would borrow the same struct twice.
        let cloud_names: Vec<String> = state.scene.cloud_names().to_vec();
        let palette_names: Vec<String> = state.scene.palettes.names.clone();
        // What the panel shows as current, so the controls reflect what is
        // actually allocated rather than what was last typed.
        let output_setup = vizz_ui::OutputSetup {
            width: state.output.width,
            height: state.output.height,
            scale: crate::settings::load().scale(),
            wide: !state.output.publishable(),
        };
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
            audio: {
                let st = &self.engine.audio.state;
                vizz_ui::AudioView {
                    connected: st.connected(),
                    device: self.engine.audio.device_name.clone(),
                    bands: std::array::from_fn(|i| st.band(i)),
                    raw: std::array::from_fn(|i| st.raw(i)),
                    raw_peak: std::array::from_fn(|i| st.raw_peak(i)),
                    level: st.level(),
                    detected_bpm: st.bpm(),
                    confidence: st.confidence(),
                    dropped: st.dropped.load(std::sync::atomic::Ordering::Relaxed),
                }
            },
            audio_bands: self.audio_bands,
            audio_auto_bpm: self.audio_auto_bpm,
            clouds: cloud_names,
            palettes: palette_names,
            output: output_setup,
            // What the renderer is actually using this frame, so a fader
            // whose parameter is being modulated can show where the value
            // has really gone rather than only where its handle sits.
            modulated: self
                .params
                .registry
                .iter()
                .map(|(id, _)| self.engine.snapshot.get(id))
                .collect(),
            presets: preset_entries(&self.library),
            grid: grid_view(
                &self.engine.grid,
                self.engine.modulation.clock.beats,
                &self.midi_view,
                &self.library,
                vizz_mod::preset::Kind::Look,
                SCENE_FIRE,
                "scene",
            ),
            // Shown only once the layer is in use. Sixteen empty pads for
            // a layer nobody has touched is a lot of the performance
            // screen spent saying nothing.
            gravity_grid: (!self.engine.gravity_grid.is_empty()).then(|| {
                gravity_grid_view(
                    &self.engine.gravity_grid,
                    self.engine.modulation.clock.beats,
                    &self.midi_view,
                    &self.library,
                )
            }),
            focus_filter: std::mem::take(&mut self.focus_filter),
            expand_sections: false,
            bpm: self.engine.modulation.clock.bpm,
            bar_phase: self.engine.modulation.clock.bar_phase(4.0),
        };
        let preset_key = state.gui.preset_key.take();
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
        let mut pending_output = None;
        match actions {
            Ok(actions) => {
                apply_audio_actions(
                    &actions,
                    &mut self.engine,
                    &mut self.audio_bands,
                    &mut self.audio_auto_bpm,
                    &mut self.tap,
                );
                apply_preset_actions(&actions, &self.params.registry, &mut self.library);
                apply_grid_actions(
                    &actions.grid,
                    &self.params,
                    &mut self.engine.grid,
                    &GridBinding::scenes(&self.params),
                    &self.midi_shared,
                    &mut self.library,
                );
                apply_grid_actions(
                    &actions.gravity,
                    &self.params,
                    &mut self.engine.gravity_grid,
                    &GridBinding::gravity(&self.params),
                    &self.midi_shared,
                    &mut self.library,
                );
                // A number key fires a slot by writing the recall
                // parameter, exactly as OSC or MIDI would — so there is
                // one recall path, not a second one that can drift.
                if let Some(slot) = preset_key {
                    self.params
                        .registry
                        .set(self.params.preset_recall, slot as f32);
                }
                // Deferred to after this frame: rebuilding the master
                // mid-encoder would swap a texture the encoder already
                // references.
                pending_output = actions.output_setup;
                apply_panel_actions(
                    actions,
                    &self.midi_shared,
                    &self.opts.midi_map_path,
                    &mut self.saved_revision,
                )
            }
            // A GUI failure must never take down the output.
            Err(e) => log::error!("GUI draw failed: {e:#}"),
        }

        // Convert the wide master down for the senders, in this frame's
        // encoder so it is ordered behind the render that produced it.
        if let (Some(target), Some((pass, bind))) = (&state.publish, &state.publish_blit) {
            pass.draw(
                &mut encoder,
                &target.view,
                bind,
                state.output.aspect(),
                target.width,
                target.height,
            );
        }

        state.ctx.queue.submit([encoder.finish()]);

        // After submit: senders enqueue work ordered behind this frame.
        //
        // A wide master cannot be published directly — Syphon hands out an
        // IOSurface and NDI's fourcc is literally BGRA — so it is
        // converted into an eight-bit copy first. The conversion is the
        // same blit the preview uses, which is why it costs one pass and
        // no new shader.
        let publish = match &state.publish {
            Some(p) => &p.texture,
            None => &state.output.texture,
        };
        outputs::publish_all(
            &mut state.senders,
            &state.ctx.device,
            &state.ctx.queue,
            publish,
        );

        state.window.pre_present_notify();
        state.ctx.queue.present(frame);

        let elapsed = frame_start.elapsed();
        state.gui.push_frame_time(elapsed.as_secs_f32() * 1e3);
        if let Some(snap) = self.engine.end_frame(elapsed) {
            log::info!("{}", snap.log_line());
        }
        state.window.request_redraw();
        // Now that the frame is presented and its encoder is retired,
        // it is safe to swap the textures out from under the next one.
        if let Some(setup) = pending_output {
            self.apply_output_setup(setup);
        }
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
            // Drag a cloud onto the window and it loads.
            //
            // A drop rather than a file dialog, for two reasons. It is the
            // gesture people already use for this — you have the scan in a
            // folder and you want it in the visualiser — and a dialog
            // would mean a new dependency that pulls GTK in on Linux, for
            // a modal window that is strictly more work to operate.
            WindowEvent::DroppedFile(path) => self.load_dropped(path),
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

/// Push panel edits through to the analysis thread. A free function taking
/// disjoint fields rather than a method, because the render state is
/// already mutably borrowed for the duration of the frame.
///
/// The settings mutex is held only long enough for a couple of field
/// writes, and is never contended with the audio callback — that side only
/// touches the lock-free ring.
fn apply_audio_actions(
    actions: &vizz_ui::PanelActions,
    engine: &mut FrameEngine,
    bands: &mut [vizz_audio::Band; 4],
    auto_bpm: &mut bool,
    tap: &mut vizz_audio::TapTempo,
) {
    let a = &actions.audio;
    if a.bands.is_none() && a.auto_bpm.is_none() && !a.tapped && a.device.is_none() {
        return;
    }
    if let Some(want) = &a.device {
        // Reopen rather than rebuild: the band gains live in the settings
        // the analysis thread shares, and rebuilding would reset the one
        // thing the user tuned to their interface.
        engine.audio.reopen(want.as_deref());
        // Remember it, so plugging the same interface in tomorrow does not
        // mean finding this menu again.
        if let Err(e) = crate::settings::save_audio_device(want.as_deref()) {
            log::warn!("could not remember the audio device: {e:#}");
        }
    }
    if let Some(b) = a.bands {
        *bands = b;
    }
    if let Some(auto) = a.auto_bpm {
        *auto_bpm = auto;
    }
    if a.tapped && let Some(bpm) = tap.tap() {
        engine.modulation.clock.bpm = bpm;
        // Tapping is an explicit manual override; leaving auto on would
        // have the detector overwrite it on the next frame.
        *auto_bpm = false;
    }
    if let Ok(mut s) = engine.audio.settings.lock() {
        s.bands = *bands;
        s.auto_bpm = *auto_bpm;
    }
}

/// The preset list the panel shows: built-ins first, then whatever is on
/// disk, in the same order `/preset/recall` numbers them.
fn preset_entries(library: &vizz_mod::preset::Library) -> Vec<vizz_ui::PresetEntry> {
    use vizz_mod::preset;
    preset::BUILTINS
        .iter()
        .map(|b| vizz_ui::PresetEntry {
            name: b.name.to_string(),
            builtin: true,
            about: Some(b.about.to_string()),
        })
        .chain(
            library
                .user(preset::Kind::Look)
                .iter()
                .map(|name| vizz_ui::PresetEntry {
                    name: name.clone(),
                    builtin: false,
                    about: None,
                }),
        )
        .collect()
}

/// The gravity grid, resolved against the gravity preset library.
fn gravity_grid_view(
    grid: &vizz_mod::scene::Grid,
    beats: f64,
    midi: &MidiView,
    library: &vizz_mod::preset::Library,
) -> vizz_ui::grid_view::GridView {
    grid_view(
        grid,
        beats,
        midi,
        library,
        vizz_mod::preset::Kind::Gravity,
        GRAVITY_FIRE,
        "gravity",
    )
}

/// The addresses a pad press writes. Named here because the grid view, the
/// learn target and [`GridBinding`] all have to agree on them, and a typo
/// in one of the three is a pad that maps to nothing.
const SCENE_FIRE: &str = "/scene/fire";
const GRAVITY_FIRE: &str = "/gravity/fire";

/// The slot number a pad addresses. Pads are numbered from 1 because 0 is
/// "nothing selected" — see `Engine::tick_grid`.
fn fire_value(slot: usize) -> f32 {
    slot as f32 + 1.0
}

/// The scene grid as the panel needs to see it.
///
/// `beats` is the musical clock, so the autopilot switch can show how far
/// through its step it is rather than only that it is on.
fn grid_view(
    grid: &vizz_mod::scene::Grid,
    beats: f64,
    midi: &MidiView,
    library: &vizz_mod::preset::Library,
    kind: vizz_mod::preset::Kind,
    fire: &str,
    noun: &'static str,
) -> vizz_ui::grid_view::GridView {
    use vizz_mod::scene::Curve;
    vizz_ui::grid_view::GridView {
        // Which pads a controller can fire, and which one is waiting for a
        // button. Per pad rather than per parameter: sixteen pads share
        // one fire address, so a single binding shown beside it would say
        // nothing about which of them is mapped.
        midi: (0..vizz_ui::grid_view::SLOTS)
            .map(|slot| {
                midi.map
                    .source_for_value(fire, fire_value(slot))
                    .map(|s| s.label())
            })
            .collect(),
        learning: (0..vizz_ui::grid_view::SLOTS)
            .find(|&slot| midi.learning_value(fire, fire_value(slot))),
        midi_available: midi.available,
        noun,
        names: grid
            .cells()
            .iter()
            .map(|c| c.as_ref().map(|c| c.display().to_string()))
            .collect(),
        // A pad whose preset has been deleted or renamed must say so
        // rather than looking filled and doing nothing when pressed.
        //
        // Asked of the cached listing rather than by loading each one:
        // `by_name` parsed the whole preset to answer a question about
        // whether a file exists, sixteen times a frame per layer.
        missing: grid
            .cells()
            .iter()
            .map(|c| c.as_ref().is_some_and(|c| !library.has(kind, &c.preset)))
            .collect(),
        presets: library.all(kind),
        current: grid.current(),
        in_flight: grid.in_flight(),
        duration: grid.duration,
        curve: Curve::ALL.iter().position(|c| *c == grid.curve).unwrap_or(1),
        curve_names: Curve::ALL.iter().map(|c| c.name().to_string()).collect(),
        autopilot: grid.autopilot.enabled,
        bars: grid.autopilot.bars,
        auto_phase: grid.autopilot_phase(beats),
        upcoming: grid.upcoming(),
    }
}

/// Grid actions from the panel.
///
/// Firing goes through `/scene/fire` rather than calling `Grid::fire`
/// directly, so a pad click, an OSC message and a MIDI note take the same
/// path — the rule preset recall already follows, and for the same reason:
/// two paths to the same thing drift apart, and then only one of them gets
/// the bug fix.
///
/// The transition settings are parameters too, so the UI writes those
/// rather than the grid's fields; the engine reads them back next frame.
/// Which layer a grid belongs to, and the parameters that drive it.
///
/// The two grids are the same machine pointed at different libraries and
/// different transport parameters. Passing that in rather than branching
/// inside means there is one implementation of "what a pad press does",
/// and the gravity grid cannot quietly drift away from the scene grid's
/// behaviour as either changes.
struct GridBinding {
    kind: vizz_mod::preset::Kind,
    fire: vizz_params::ParamId,
    time: vizz_params::ParamId,
    curve: vizz_params::ParamId,
    auto: vizz_params::ParamId,
    bars: vizz_params::ParamId,
    /// The fire parameter's address, for MIDI bindings — those name a
    /// parameter by address rather than by id, since they outlive the
    /// process.
    addr: &'static str,
    /// What a captured pad is called when the slot was empty.
    noun: &'static str,
}

impl GridBinding {
    fn scenes(p: &crate::params::AppParams) -> Self {
        Self {
            kind: vizz_mod::preset::Kind::Look,
            fire: p.scene_fire,
            time: p.scene_time,
            curve: p.scene_curve,
            auto: p.scene_auto,
            bars: p.scene_bars,
            addr: SCENE_FIRE,
            noun: "scene",
        }
    }

    fn gravity(p: &crate::params::AppParams) -> Self {
        Self {
            kind: vizz_mod::preset::Kind::Gravity,
            fire: p.gravity_fire,
            time: p.gravity_time,
            curve: p.gravity_curve,
            auto: p.gravity_auto,
            bars: p.gravity_bars,
            addr: GRAVITY_FIRE,
            noun: "gravity",
        }
    }
}

fn apply_grid_actions(
    actions: &vizz_ui::grid_view::GridActions,
    params: &crate::params::AppParams,
    grid: &mut vizz_mod::scene::Grid,
    b: &GridBinding,
    midi: &SharedMidi,
    library: &mut vizz_mod::preset::Library,
) {
    let reg = &params.registry;
    let mut dirty = false;
    if let Some(slot) = actions.fire {
        reg.set(b.fire, fire_value(slot));
    }
    // Learning a pad rather than the parameter. A binding on `/scene/fire`
    // alone would be one button for all sixteen pads, which is what this
    // replaces — see `Binding::value`.
    //
    // `try_lock` for the same reason as everywhere else the render thread
    // touches this: a missed frame is a click that did not take, a blocked
    // one is a dropped frame on stage. The revision bump is picked up by
    // the flush in `apply_panel_actions`, which runs later in the frame.
    if actions.learn.is_some() || actions.unlearn.is_some() {
        if let Ok(mut state) = midi.try_lock() {
            if let Some(target) = actions.learn {
                state.learn_target = target.map(|slot| {
                    vizz_midi::LearnTarget::value(
                        b.addr,
                        fire_value(slot),
                        format!("{} {}", b.noun, slot + 1),
                    )
                });
            }
            if let Some(slot) = actions.unlearn {
                state.map.unbind_value(b.addr, fire_value(slot));
                state.revision += 1;
            }
        } else {
            log::debug!("MIDI busy this frame; the pad mapping click did not take");
        }
    }
    // Put an existing preset on a pad. The core gesture now that a scene
    // names a look rather than owning a copy of one.
    if let Some((slot, name)) = &actions.assign {
        grid.assign(*slot, name.clone());
        dirty = true;
    }
    if let Some(slot) = actions.store {
        // Capture still exists, because during a set the useful gesture is
        // "keep what is on screen" and stopping to name it is exactly the
        // wrong moment. But what it captures is now a *preset* — saved to
        // the library under the pad's name and then referenced — rather
        // than a copy hidden inside the grid. One gesture, and the result
        // is a look you can also recall, edit and put on another pad.
        let name = grid
            .cell(slot)
            .map(|c| c.preset.clone())
            .unwrap_or_else(|| format!("{} {}", b.noun, slot + 1));
        let captured = vizz_mod::preset::Preset::capture_kind(reg, b.kind);
        match vizz_mod::preset::save_kind(b.kind, &name, &captured) {
            Ok(saved) => {
                grid.assign(slot, saved);
                // The pad now names a preset that did not exist a moment
                // ago; without this the cache says it is missing and the
                // pad you just filled draws as broken.
                library.refresh();
                dirty = true;
            }
            // A failed save must not leave the pad pointing at a preset
            // that was never written — that would be a pad which looks
            // filled and does nothing.
            Err(e) => log::warn!("could not save {} {} as a preset: {e:#}", b.noun, slot + 1),
        }
    }
    if let Some(slot) = actions.clear {
        grid.clear(slot);
        dirty = true;
    }
    if let Some((slot, name)) = &actions.rename {
        // Renames the pad, not the preset. The same look is the drop in
        // one set and the outro in another.
        grid.relabel(*slot, name.clone());
        dirty = true;
    }
    if let Some(v) = actions.set_duration {
        reg.set(b.time, v);
        dirty = true;
    }
    if let Some(i) = actions.set_curve {
        reg.set(b.curve, i as f32);
        dirty = true;
    }
    if let Some(on) = actions.set_autopilot {
        reg.set(b.auto, if on { 1.0 } else { 0.0 });
        dirty = true;
    }
    if let Some(bars) = actions.set_bars {
        reg.set(b.bars, bars);
        dirty = true;
    }
    if dirty || actions.changed {
        // What the grid persists is read back from the parameters, so
        // mirror them in before writing or the file keeps the values it
        // was loaded with.
        grid.duration = reg.target(b.time);
        grid.autopilot.bars = reg.target(b.bars);
        grid.autopilot.enabled = reg.target(b.auto) >= 0.5;
        // The curve was missing from this list, so the file kept whatever
        // `tick_grid` had synced *before* the click was processed — set
        // "cut" for a set that needs hard changes, restart, and you were
        // back on "smooth" with nothing on screen to say so, because live
        // behaviour was correct all along.
        let curve = reg.target(b.curve).round().max(0.0) as usize;
        grid.curve = vizz_mod::scene::Curve::ALL
            .get(curve)
            .copied()
            .unwrap_or_default();
        if let Err(e) = vizz_mod::scene::save_kind(b.kind, grid) {
            log::error!("could not save the {} grid: {e:#}", b.noun);
        }
    }
}

/// Preset actions from the panel. Disk work happens here rather than in
/// the panel so drawing stays free of side effects, and every failure is
/// logged rather than propagated — losing a preset must not take the show
/// with it.
fn apply_preset_actions(
    actions: &vizz_ui::PanelActions,
    registry: &vizz_params::ParamRegistry,
    library: &mut vizz_mod::preset::Library,
) {
    use vizz_mod::preset;
    if let Some(name) = &actions.preset_load {
        match preset::by_name(name) {
            Some(p) => log::info!("recalled preset {name} ({} parameters)", p.apply(registry)),
            None => log::error!("preset {name} could not be read"),
        }
    }
    // Both of these change what is on disk, so the cached listing is
    // refreshed rather than left to the interval — a preset you just saved
    // has to be on the assign menu now, not in up to two seconds.
    if let Some(name) = &actions.preset_save {
        let snapshot = preset::Preset::capture(registry);
        match preset::save(name, &snapshot) {
            Ok(saved) => {
                log::info!("saved preset {saved} ({} parameters)", snapshot.values.len());
                library.refresh();
            }
            Err(e) => log::error!("could not save preset {name}: {e:#}"),
        }
    }
    if let Some(name) = &actions.preset_delete {
        match preset::delete(name) {
            Ok(()) => {
                log::info!("deleted preset {name}");
                library.refresh();
            }
            Err(e) => log::error!("could not delete preset {name}: {e:#}"),
        }
    }
}

fn apply_panel_actions(
    actions: vizz_ui::PanelActions,
    shared: &SharedMidi,
    map_path: &std::path::Path,
    saved_revision: &mut u64,
) {
    // No early return on "the UI did nothing this frame".
    //
    // A learn *completes* on the MIDI callback thread, not in response to
    // a click, so the frame that creates a binding has neither of these
    // actions set. Gating the flush on them meant the newest binding was
    // only written the next time the user armed learn or cleared one —
    // and there is no save on exit, so the last binding of every session
    // was silently lost. The chip still rendered as bound in-session, so
    // nothing warned.
    //
    // `try_lock`, because this now runs every frame and the render thread
    // must never wait on the MIDI thread. A missed flush is picked up on
    // the next frame.
    let Ok(mut state) = shared.try_lock() else { return };
    if let Some(target) = actions.set_learn_target {
        state.learn_target = target;
    }
    if let Some(param) = actions.clear_binding {
        state.map.unbind_param(&param);
        state.revision += 1;
    }
    if let Some((param, value)) = actions.clear_slot_binding {
        state.map.unbind_value(&param, value);
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

pub fn run(params: Arc<AppParams>, mut opts: WindowedOpts) -> Result<()> {
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
    // A file that has since been moved or deleted is dropped from the list
    // rather than being retried and warned about on every start.
    let cloud_paths: Vec<String> = if opts.clouds.is_empty() {
        crate::settings::load()
            .clouds
            .into_iter()
            .filter(|p| std::path::Path::new(p).exists())
            .collect()
    } else {
        opts.clouds.iter().map(|p| p.display().to_string()).collect()
    };
    opts.clouds = cloud_paths.iter().map(std::path::PathBuf::from).collect();
    // Same treatment for palettes: a file that has since moved is dropped
    // rather than warned about on every launch.
    let palette_paths: Vec<String> = crate::settings::load()
        .palettes
        .into_iter()
        .filter(|p| std::path::Path::new(p).exists())
        .collect();
    let mut engine = FrameEngine::new(
        Arc::clone(&params),
        vizz_audio::AudioEngine::start(opts.audio_device.as_deref()),
    );
    // The grid is user state like the MIDI map and the macros, so it comes
    // back with the app. A missing or unreadable file gives an empty grid
    // rather than a startup failure — see `scene::load`.
    engine.adopt_grid(vizz_mod::scene::load());
    engine.adopt_gravity_grid(vizz_mod::scene::load_kind(vizz_mod::preset::Kind::Gravity));
    let mut app = App {
        engine,
        params,
        opts,
        state: None,
        audio_bands: vizz_audio::default_bands(),
        audio_auto_bpm: false,
        tap: vizz_audio::TapTempo::new(),
        focus_filter: false,
        live: None,
        live_revision: 0,
        // Clouds named on the command line win; otherwise restore whatever
        // was last dropped, so a set survives a restart.
        clouds: cloud_paths,
        next_cloud: 0,
        palettes: palette_paths,
        midi,
        midi_shared,
        midi_view: MidiView::default(),
        library: vizz_mod::preset::Library::new(),
        saved_revision: 0,
        update,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

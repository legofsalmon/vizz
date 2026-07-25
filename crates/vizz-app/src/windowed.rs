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

use crate::engine::FrameEngine;
use crate::outputs::{self, OutputOpts};
use crate::params::AppParams;

pub struct WindowedOpts {
    pub width: u32,
    pub height: u32,
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
}

struct App {
    engine: FrameEngine,
    opts: WindowedOpts,
    state: Option<RenderState>,
}

impl App {
    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<RenderState> {
        let attrs = Window::default_attributes()
            .with_title("vizz")
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

        if let Some(snap) = self.engine.end_frame(frame_start.elapsed()) {
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

pub fn run(params: Arc<AppParams>, opts: WindowedOpts) -> Result<()> {
    let event_loop = EventLoop::new()?;
    // Poll: we drive redraws ourselves; vsync provides the pacing.
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        engine: FrameEngine::new(params),
        opts,
        state: None,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

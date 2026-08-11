//! Output sender construction and the fail-soft publish loop.

use vizz_io::FrameSender;

// Some fields/args are only touched on the platforms whose backend uses them.
#[derive(Debug, Clone)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct OutputOpts {
    pub syphon: bool,
    pub syphon_name: String,
    pub syphon_flip: bool,
    pub ndi: bool,
    pub ndi_name: String,
    /// Master output size and rate, needed by senders that describe the
    /// stream up front (NDI).
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// How a slot rebuilds its sender, both at startup and every time it dies.
type Builder =
    Box<dyn Fn(&wgpu::Device, &OutputOpts) -> anyhow::Result<Box<dyn FrameSender>> + Send>;

/// How long a dead output waits before trying to come back. Long enough
/// that a genuinely absent receiver costs one construction attempt every
/// few seconds rather than one per frame; short enough that plugging the
/// network lead back in is not followed by a wait anyone notices.
const RETRY: std::time::Duration = std::time::Duration::from_secs(3);

/// One requested output, alive or not.
///
/// The requested outputs are the roster, permanently. The old shape — a
/// `Vec` of live senders that dropped a member on its first error — meant
/// a dead output was indistinguishable from one that was never asked for:
/// the panel's dead-output rendering was unreachable code, the promised
/// background retry did not exist, and the only record that the projector
/// feed died was one log line.
struct Slot {
    name: String,
    build: Builder,
    sender: Option<Box<dyn FrameSender>>,
    /// When a dead slot may next attempt to come back.
    next_try: std::time::Instant,
}

/// Every output the user asked for, with liveness and self-repair.
pub struct Outputs {
    slots: Vec<Slot>,
    opts: OutputOpts,
    retry: std::time::Duration,
}

impl Outputs {
    /// Build every requested sender. One failing to start is a warning,
    /// never a startup failure — the show runs without it, and the slot
    /// keeps retrying in the background.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables, unused_mut))]
    pub fn new(device: &wgpu::Device, opts: &OutputOpts) -> Self {
        let mut slots: Vec<Slot> = Vec::new();

        #[cfg(target_os = "macos")]
        if opts.syphon {
            slots.push(Slot::start(
                device,
                opts,
                format!("syphon:{}", opts.syphon_name),
                Box::new(|device, opts| {
                    vizz_io::syphon::SyphonSender::new(device, &opts.syphon_name, opts.syphon_flip)
                        .map(|s| Box::new(s) as Box<dyn FrameSender>)
                }),
            ));
        }
        #[cfg(not(target_os = "macos"))]
        if opts.syphon {
            log::debug!("Syphon is macOS-only; no sender started");
        }

        if opts.ndi {
            slots.push(Slot::start(
                device,
                opts,
                format!("ndi:{}", opts.ndi_name),
                Box::new(|device, opts| {
                    vizz_io::ndi::NdiSender::new(
                        device,
                        &opts.ndi_name,
                        opts.width,
                        opts.height,
                        opts.fps,
                        1,
                    )
                    .map(|s| Box::new(s) as Box<dyn FrameSender>)
                }),
            ));
        }

        if slots.is_empty() {
            log::info!("no video outputs requested (preview/headless only)");
        }
        Self { slots, opts: opts.clone(), retry: RETRY }
    }

    /// Publish the master to every live output, and give one dead output
    /// per call its chance to come back.
    ///
    /// A sender that errors is killed on the spot — output loss must never
    /// take down the render loop — but its slot stays, reports itself dead
    /// to the panel, and retries on the cadence above. One comeback
    /// attempt per call, so a frame never pays for more than one sender
    /// construction.
    pub fn publish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) {
        let now = std::time::Instant::now();
        for slot in &mut self.slots {
            if let Some(sender) = &mut slot.sender
                && let Err(e) = sender.publish(device, queue, texture)
            {
                log::error!(
                    "output '{}' failed: {e:#} — retrying in the background",
                    slot.name
                );
                slot.sender = None;
                slot.next_try = now + self.retry;
            }
        }
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|s| s.sender.is_none() && s.next_try <= now)
        {
            match (slot.build)(device, &self.opts) {
                Ok(sender) => {
                    log::info!("output '{}' is back", slot.name);
                    slot.sender = Some(sender);
                }
                Err(e) => {
                    // Debug, not error: while the receiver is genuinely
                    // absent this fires every few seconds all night.
                    log::debug!("output '{}' still unavailable: {e:#}", slot.name);
                    slot.next_try = now + self.retry;
                }
            }
        }
    }

    /// The roster as the panel shows it: every requested output, and
    /// whether it is actually carrying frames right now. This is what
    /// makes the dead-output warning in both UIs reachable at all.
    pub fn status(&self) -> Vec<vizz_ui::OutputStatus> {
        self.slots
            .iter()
            .map(|s| vizz_ui::OutputStatus {
                name: s.name.clone(),
                live: s.sender.is_some(),
            })
            .collect()
    }
}

impl Slot {
    /// Try to bring the sender up now; a failure leaves a dead slot that
    /// the publish loop will keep retrying.
    fn start(device: &wgpu::Device, opts: &OutputOpts, name: String, build: Builder) -> Self {
        let sender = match build(device, opts) {
            Ok(sender) => {
                log::info!("output '{name}' is live");
                Some(sender)
            }
            Err(e) => {
                log::warn!("output '{name}' unavailable: {e:#} — retrying in the background");
                None
            }
        };
        Slot { name, build, sender, next_try: std::time::Instant::now() + RETRY }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A sender that works for `ok_for` publishes and then errors.
    struct Flaky {
        ok_for: usize,
        published: Arc<AtomicUsize>,
    }

    impl FrameSender for Flaky {
        fn name(&self) -> &str {
            "flaky"
        }
        fn publish(
            &mut self,
            _device: &wgpu::Device,
            _queue: &wgpu::Queue,
            _texture: &wgpu::Texture,
        ) -> anyhow::Result<()> {
            let n = self.published.fetch_add(1, Ordering::Relaxed);
            if n < self.ok_for {
                Ok(())
            } else {
                anyhow::bail!("receiver went away")
            }
        }
    }

    fn gpu() -> (wgpu::Device, wgpu::Queue, wgpu::Texture) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("no GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("outputs-test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .expect("no device");
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("master-stub"),
            size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        (device, queue, texture)
    }


    /// Syphon output goes out the right way up.
    ///
    /// Metal's origin is the top left and Syphon's convention is
    /// OpenGL's, origin at the bottom, so a published Metal texture must
    /// carry the flip flag to arrive upright. It defaulted to off and
    /// shipped that way: every receiver showed vizz upside down, and the
    /// only remedy was a command-line flag — unreachable to anyone
    /// double-clicking the app, which is how it is meant to be used.
    ///
    /// Asserted on the parsed command line rather than on a rendered
    /// frame because there is no Syphon to receive from in CI: this is
    /// the value that reaches the server, and it is the value that was
    /// wrong.
    #[test]
    fn syphon_publishes_the_right_way_up_by_default() {
        use clap::Parser;
        let args = crate::Args::parse_from(["vizz"]);
        assert!(
            args.syphon_flip,
            "Syphon output defaults to upside down in every receiver"
        );
        // And it can still be turned off, for a receiver that corrects
        // for the flip itself.
        let off = crate::Args::parse_from(["vizz", "--syphon-flip", "false"]);
        assert!(!off.syphon_flip, "the flip cannot be turned off");
    }

    fn opts() -> OutputOpts {
        OutputOpts {
            syphon: false,
            syphon_name: "t".into(),
            syphon_flip: false,
            ndi: false,
            ndi_name: "t".into(),
            width: 4,
            height: 4,
            fps: 60,
        }
    }

    /// A dying output must stay on the roster as dead — that is what makes
    /// the panel's warning reachable — and must come back by itself when
    /// its builder succeeds again. The `Vec` shape this replaced dropped
    /// the sender on first error: dead was indistinguishable from
    /// never-requested, and the documented background retry did not exist.
    #[test]
    fn a_dead_output_reports_dead_and_comes_back_on_its_own() {
        let (device, queue, texture) = gpu();
        let publishes = Arc::new(AtomicUsize::new(0));
        let rebuilds = Arc::new(AtomicUsize::new(0));

        let p = publishes.clone();
        let r = rebuilds.clone();
        let slot = Slot {
            name: "test-output".into(),
            sender: Some(Box::new(Flaky { ok_for: 1, published: publishes.clone() })),
            build: Box::new(move |_, _| {
                // First comeback attempt fails — the receiver is still
                // gone — the second succeeds.
                if r.fetch_add(1, Ordering::Relaxed) == 0 {
                    anyhow::bail!("still gone")
                }
                Ok(Box::new(Flaky { ok_for: usize::MAX, published: p.clone() })
                    as Box<dyn FrameSender>)
            }),
            next_try: std::time::Instant::now(),
        };
        let mut outputs = Outputs {
            slots: vec![slot],
            opts: opts(),
            // Zero, so the test drives the clock with publish calls alone.
            retry: std::time::Duration::ZERO,
        };

        assert!(outputs.status()[0].live, "starts live");

        outputs.publish(&device, &queue, &texture); // ok
        assert!(outputs.status()[0].live);

        // This publish kills the sender, and — with the retry interval at
        // zero — the same call's comeback pass immediately makes rebuild
        // attempt 1, which fails. In production the interval is seconds,
        // so death and first retry land on different frames.
        outputs.publish(&device, &queue, &texture);
        assert!(!outputs.status()[0].live, "a failed output must report dead, not vanish");
        assert_eq!(outputs.status()[0].name, "test-output", "and must stay on the roster");
        assert_eq!(rebuilds.load(Ordering::Relaxed), 1);

        outputs.publish(&device, &queue, &texture); // retry #2 succeeds
        assert!(outputs.status()[0].live, "a recovered output must report live again");
        assert_eq!(rebuilds.load(Ordering::Relaxed), 2);

        // And the recovered sender is actually the one publishing.
        let before = publishes.load(Ordering::Relaxed);
        outputs.publish(&device, &queue, &texture);
        assert!(publishes.load(Ordering::Relaxed) > before);
    }
}

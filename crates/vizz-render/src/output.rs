//! The master output target: scenes render here at a fixed output
//! resolution, independent of the preview window. This texture is what
//! gets published to Syphon/Spout/NDI and blitted to the window.

/// BGRA + sRGB: the native interchange format on macOS (what CAMetalLayer
/// and Syphon receivers expect) and universally supported as a render
/// target on Vulkan/DX12.
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

/// The wider master, for when eight bits per channel is the thing you can
/// see. Sixteen-bit float keeps the headroom the post chain already works
/// in all the way to the output instead of quantising at the last step,
/// which is where banding in a slow gradient comes from.
///
/// Not the default, and not free: it doubles the master's bandwidth, and
/// neither Syphon nor NDI will take it — both are BGRA8 by definition, so
/// publishing needs a conversion. See [`OutputTarget::publishable`].
pub const WIDE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub struct OutputTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
}

impl OutputTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self::with_format(device, width, height, OUTPUT_FORMAT)
    }

    /// Whether this target can be handed to a sender as-is.
    ///
    /// Syphon publishes an IOSurface and NDI's fourcc is literally BGRA;
    /// neither has a path for a float texture. A wide master therefore
    /// has to be converted before it leaves, and the caller needs to know
    /// that rather than discovering it as a black frame at a venue.
    pub fn publishable(&self) -> bool {
        self.format == OUTPUT_FORMAT
    }

    pub fn with_format(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("master-output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            // Rendered to by scenes, sampled by the preview blit, copied
            // out by NDI readback (and Syphon's internal blit reads it).
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
            format,
        }
    }

    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }
}

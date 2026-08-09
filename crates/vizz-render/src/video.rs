//! A live video frame on the GPU.
//!
//! One texture, replaced in place while the size holds and recreated when
//! it changes. Everything that wants video — the particle field sampling
//! it as a cloud, the background pass drawing it behind the field — binds
//! this one view, so a frame is uploaded once however many places read it.
//!
//! **Always present, even with nothing connected.** A bind group entry
//! cannot be empty, and making every consumer branch on whether video
//! exists would put an `Option` through half the renderer. Instead the
//! texture starts as a single black pixel and `present` says whether
//! anything has arrived, which is one uniform rather than one code path.

use crate::GpuContext;

/// BGRA is what NDI is asked for and what Syphon hands over, so the
/// texture takes it directly rather than the renderer swizzling every
/// pixel of every frame to avoid naming the format once.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

pub struct Video {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    width: u32,
    height: u32,
    /// Whether a real frame has ever landed. The placeholder is black, so
    /// without this a disconnected input is indistinguishable from a feed
    /// that happens to be showing black — and those want opposite
    /// treatment, one being a fault and the other a picture.
    pub present: bool,
    /// Frames uploaded, for the panel to show that something is arriving
    /// even when the picture itself is dark.
    pub frames: u64,
}

impl Video {
    /// A one-pixel black texture, so the binding is valid from the first
    /// frame drawn.
    pub fn new(ctx: &GpuContext) -> Self {
        let mut v = Self::allocate(ctx, 1, 1);
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &v.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0, 0, 0, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        v.present = false;
        v
    }

    fn allocate(ctx: &GpuContext, width: u32, height: u32) -> Self {
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
            present: false,
            frames: 0,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Aspect ratio, or 1 before anything has arrived. Used to fit the
    /// picture into the field's box without stretching it.
    pub fn aspect(&self) -> f32 {
        if self.height == 0 {
            1.0
        } else {
            self.width as f32 / self.height as f32
        }
    }

    /// Upload a frame. Returns `true` when the texture was reallocated,
    /// which invalidates any bind group holding the old view.
    ///
    /// `stride` is bytes per row as the source delivered them and may
    /// exceed `width * 4`; it is passed to the copy rather than the rows
    /// being repacked, because repacking is a full copy of every frame to
    /// save a number the API already accepts.
    pub fn upload(
        &mut self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        stride: u32,
        bgra: &[u8],
    ) -> bool {
        if width == 0 || height == 0 {
            return false;
        }
        // A short buffer is a source disagreeing with itself. Uploading it
        // would read past the end, so the frame is dropped and the last
        // good one stays on screen — a stale frame beats a crash mid-set.
        let needed = stride as usize * height as usize;
        if bgra.len() < needed {
            log::warn!(
                "video frame is {} bytes, short of the {needed} its {width}x{height} \
                 at stride {stride} claims — dropping it",
                bgra.len()
            );
            return false;
        }
        let resized = width != self.width || height != self.height;
        if resized {
            let fresh = Self::allocate(ctx, width, height);
            self.texture = fresh.texture;
            self.view = fresh.view;
            self.width = width;
            self.height = height;
        }
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bgra[..needed],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.present = true;
        self.frames += 1;
        resized
    }
}

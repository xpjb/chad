use crate::{Config, RenderContext, wgpu};

const HEADLESS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Native offscreen wgpu context for screenshots, golden images, and render
/// smoke checks without creating a window, event loop, or surface.
pub struct HeadlessCtx {
    /// Raw wgpu device selected from `Config`.
    pub device: wgpu::Device,
    /// Raw wgpu queue paired with [`Self::device`].
    pub queue: wgpu::Queue,
    /// Format of the owned offscreen target.
    pub surface_format: wgpu::TextureFormat,
    /// Caller-controlled simulation step in seconds.
    pub dt: f32,
    /// Caller-controlled elapsed time in seconds.
    pub elapsed: f32,
    /// Caller-controlled frame number.
    pub frame_index: u64,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
    alpha: f32,
}

impl HeadlessCtx {
    /// Request a GPU using `config`'s size, power preference, features, and
    /// limits, then create one RGBA8 sRGB render target. Other `Config` fields
    /// do not affect this windowless, caller-driven context.
    pub fn new(config: &Config) -> Result<Self, String> {
        pollster::block_on(Self::request(config))
    }

    async fn request(config: &Config) -> Result<Self, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: config.power_preference,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| format!("request headless adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("chad-headless-device"),
                required_features: config.device_features,
                required_limits: config.device_limits.clone(),
                memory_hints: wgpu::MemoryHints::default(),
                ..Default::default()
            })
            .await
            .map_err(|error| format!("request headless device: {error}"))?;
        let size = clamped_size(config.size);
        let (texture, view) = create_target(&device, size);
        Ok(Self {
            device,
            queue,
            surface_format: HEADLESS_FORMAT,
            texture,
            view,
            size,
            dt: 0.0,
            elapsed: 0.0,
            frame_index: 0,
            alpha: 1.0,
        })
    }

    /// Current offscreen target size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Interpolation factor used by deterministic fixed-step rendering.
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// Set the interpolation factor exposed through [`RenderContext`].
    pub fn set_alpha(&mut self, alpha: f32) {
        self.alpha = alpha.clamp(0.0, 1.0);
    }

    /// The context's owned RGBA8 sRGB render target.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Replace the owned render target. Zero dimensions are clamped to one.
    pub fn resize(&mut self, size: (u32, u32)) {
        let size = clamped_size(size);
        if self.size == size {
            return;
        }
        let (texture, view) = create_target(&self.device, size);
        self.texture = texture;
        self.view = view;
        self.size = size;
    }

    /// Read the owned target into tightly packed, top-to-bottom RGBA8 pixels.
    ///
    /// Call this only after submitting the commands that render the target.
    pub fn read_rgba8(&self) -> Result<Vec<u8>, String> {
        let unpadded = self.size.0 * 4;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chad-headless-readback"),
            size: (padded * self.size.1) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("chad-headless-copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(self.size.1),
                },
            },
            wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("poll headless readback: {error}"))?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| format!("map headless readback: {error}"))?;
        let mut pixels = Vec::with_capacity((unpadded * self.size.1) as usize);
        for row in 0..self.size.1 {
            let start = (row * padded) as usize;
            pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
        }
        Ok(pixels)
    }
}

impl RenderContext for HeadlessCtx {
    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    fn size(&self) -> (u32, u32) {
        self.size()
    }

    fn dt(&self) -> f32 {
        self.dt
    }

    fn elapsed(&self) -> f32 {
        self.elapsed
    }

    fn frame_index(&self) -> u64 {
        self.frame_index
    }

    fn alpha(&self) -> f32 {
        self.alpha()
    }
}

fn clamped_size(size: (u32, u32)) -> (u32, u32) {
    (size.0.max(1), size.1.max(1))
}

fn create_target(device: &wgpu::Device, size: (u32, u32)) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("chad-headless-target"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HEADLESS_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

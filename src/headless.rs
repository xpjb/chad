use crate::wgpu;

/// Native offscreen wgpu context for screenshots, golden images, and render
/// smoke checks without creating a window or surface.
pub struct Headless {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
}

/// RGBA8 sRGB render target owned by a [`Headless`] context.
pub struct HeadlessTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
}

impl Headless {
    pub fn new() -> Result<Self, String> {
        pollster::block_on(Self::request())
    }

    async fn request() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| format!("request headless adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("chad-headless"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|error| format!("request headless device: {error}"))?;
        Ok(Self {
            device,
            queue,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
        })
    }

    pub fn target(&self, size: (u32, u32)) -> HeadlessTarget {
        let size = (size.0.max(1), size.1.max(1));
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("chad-headless-target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        HeadlessTarget {
            texture,
            view,
            size,
        }
    }

    /// Read a target into tightly packed, top-to-bottom RGBA8 pixels.
    /// Call this only after submitting the commands that render the target.
    pub fn read_rgba8(&self, target: &HeadlessTarget) -> Result<Vec<u8>, String> {
        let unpadded = target.size.0 * 4;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chad-headless-readback"),
            size: (padded * target.size.1) as u64,
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
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(target.size.1),
                },
            },
            wgpu::Extent3d {
                width: target.size.0,
                height: target.size.1,
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
        let mut pixels = Vec::with_capacity((unpadded * target.size.1) as usize);
        for row in 0..target.size.1 {
            let start = (row * padded) as usize;
            pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
        }
        Ok(pixels)
    }
}

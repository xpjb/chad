//! A first real 2D pipeline: generated texture, bind group, resize-aware
//! screen uniform, dynamic instance buffer, and alpha-blended sprite batch.
//! `cargo run --example sprite_batch`

use bytemuck::{Pod, Zeroable};
use chad::winit::event::WindowEvent;
use chad::{wgpu, ChadApp, Config, Ctx};

const SPRITE_COUNT: usize = 24;
const TEXTURE_SIZE: u32 = 32;
const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.025,
    g: 0.030,
    b: 0.045,
    a: 1.0,
};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Sprite {
    center: [f32; 2],
    half_size: [f32; 2],
    rotation: f32,
    tint: [f32; 4],
}

struct SpriteBatch {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    screen: wgpu::Buffer,
    instances: wgpu::Buffer,
}

impl ChadApp for SpriteBatch {
    fn init(ctx: &mut Ctx) -> Result<Self, String> {
        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("sprite-batch-shader"),
                source: wgpu::ShaderSource::Wgsl(SPRITE_WGSL.into()),
            });
        let screen = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite-batch-screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instances = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite-batch-instances"),
            size: (SPRITE_COUNT * std::mem::size_of::<Sprite>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sprite-batch-texture"),
            size: wgpu::Extent3d {
                width: TEXTURE_SIZE,
                height: TEXTURE_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &sprite_pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TEXTURE_SIZE * 4),
                rows_per_image: Some(TEXTURE_SIZE),
            },
            wgpu::Extent3d {
                width: TEXTURE_SIZE,
                height: TEXTURE_SIZE,
                depth_or_array_layers: 1,
            },
        );
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprite-batch-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("sprite-batch-bind-group-layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite-batch-bind-group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: screen.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("sprite-batch-pipeline-layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });
        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sprite-batch-pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Sprite>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x2,
                            1 => Float32x2,
                            2 => Float32,
                            3 => Float32x4
                        ],
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.surface_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        Ok(Self {
            pipeline,
            bind_group,
            screen,
            instances,
        })
    }

    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent) {
        if let WindowEvent::CloseRequested = event {
            ctx.exit();
        }
    }

    fn update(&mut self, _ctx: &mut Ctx) {}

    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView) {
        let (width, height) = ctx.size();
        let screen = [width as f32, height as f32, 0.0, 0.0];
        let sprites = make_sprites(ctx.elapsed, width as f32, height as f32);
        ctx.queue
            .write_buffer(&self.screen, 0, bytemuck::cast_slice(&screen));
        ctx.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(&sprites));

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sprite-batch-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sprite-batch-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            pass.draw(0..6, 0..SPRITE_COUNT as u32);
        }
        ctx.queue.submit(std::iter::once(encoder.finish()));
    }
}

fn make_sprites(time: f32, width: f32, height: f32) -> [Sprite; SPRITE_COUNT] {
    let mut sprites = [Sprite::default(); SPRITE_COUNT];
    let radius = width.min(height) * 0.30;
    for (index, sprite) in sprites.iter_mut().enumerate() {
        let phase = index as f32 / SPRITE_COUNT as f32 * std::f32::consts::TAU;
        let orbit = phase + time * (0.25 + index as f32 * 0.006);
        let pulse = 30.0 + 10.0 * (time * 1.7 + phase * 3.0).sin();
        let hue = index as f32 / SPRITE_COUNT as f32;
        *sprite = Sprite {
            center: [
                width * 0.5 + orbit.cos() * radius,
                height * 0.5 + orbit.sin() * radius * 0.62,
            ],
            half_size: [pulse, pulse],
            rotation: -orbit + time * 0.4,
            tint: palette(hue),
        };
    }
    sprites
}

fn palette(t: f32) -> [f32; 4] {
    let tau = std::f32::consts::TAU;
    [
        0.55 + 0.45 * (tau * (t + 0.00)).cos(),
        0.55 + 0.45 * (tau * (t + 0.67)).cos(),
        0.55 + 0.45 * (tau * (t + 0.33)).cos(),
        0.82,
    ]
}

fn sprite_pixels() -> Vec<u8> {
    let mut pixels = vec![0; (TEXTURE_SIZE * TEXTURE_SIZE * 4) as usize];
    for y in 0..TEXTURE_SIZE {
        for x in 0..TEXTURE_SIZE {
            let px = (x as f32 + 0.5) / TEXTURE_SIZE as f32 * 2.0 - 1.0;
            let py = (y as f32 + 0.5) / TEXTURE_SIZE as f32 * 2.0 - 1.0;
            let distance = (px * px + py * py).sqrt();
            let alpha = ((1.0 - distance) * 8.0).clamp(0.0, 1.0);
            let offset = ((y * TEXTURE_SIZE + x) * 4) as usize;
            pixels[offset..offset + 4].copy_from_slice(&[255, 255, 255, (alpha * 255.0) as u8]);
        }
    }
    pixels
}

fn main() {
    let config = Config {
        title: "chad sprite batch".into(),
        ..Default::default()
    };
    if let Err(error) = chad::run::<SpriteBatch>(config) {
        eprintln!("{error}");
    }
}

const SPRITE_WGSL: &str = r#"
struct Screen {
    size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;
@group(0) @binding(1) var sprite_texture: texture_2d<f32>;
@group(0) @binding(2) var sprite_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) rotation: f32,
    @location(3) tint: vec4<f32>,
) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2( 1.0, -1.0), vec2(-1.0,  1.0),
        vec2(-1.0,  1.0), vec2( 1.0, -1.0), vec2( 1.0,  1.0),
    );
    var uvs = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let local = corners[vertex] * half_size;
    let sine = sin(rotation);
    let cosine = cos(rotation);
    let rotated = vec2(
        local.x * cosine - local.y * sine,
        local.x * sine + local.y * cosine,
    );
    let pixel = center + rotated;
    let clip = vec2(pixel.x / screen.size.x * 2.0 - 1.0, 1.0 - pixel.y / screen.size.y * 2.0);

    var out: VsOut;
    out.position = vec4(clip, 0.0, 1.0);
    out.uv = uvs[vertex];
    out.tint = tint;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(sprite_texture, sprite_sampler, in.uv) * in.tint;
}
"#;

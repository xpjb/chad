//! The smallest complete chad app: draw a triangle and exit on close.
//! `cargo run --example triangle`

use chad::winit::event::WindowEvent;
use chad::{ChadApp, Config, Ctx, RenderContext, wgpu};

#[cfg(not(target_arch = "wasm32"))]
mod common;

const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.030,
    g: 0.022,
    b: 0.055,
    a: 1.0,
};

struct Triangle {
    pipeline: wgpu::RenderPipeline,
}

impl Triangle {
    fn new(ctx: &impl RenderContext) -> Self {
        let shader = ctx
            .device()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("triangle-shader"),
                source: wgpu::ShaderSource::Wgsl(TRIANGLE_WGSL.into()),
            });
        let layout = ctx
            .device()
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("triangle-layout"),
                bind_group_layouts: &[],
                immediate_size: 0,
            });
        let pipeline = ctx
            .device()
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("triangle-pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.format(),
                        blend: Some(wgpu::BlendState::REPLACE),
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
        Self { pipeline }
    }

    fn render(&self, ctx: &impl RenderContext, view: &wgpu::TextureView) {
        let mut encoder = ctx
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main-pass"),
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
            pass.draw(0..3, 0..1);
        }
        ctx.queue().submit(std::iter::once(encoder.finish()));
    }
}

impl ChadApp for Triangle {
    fn init(ctx: &mut Ctx) -> Result<Self, String> {
        Ok(Self::new(ctx))
    }

    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent) {
        if let WindowEvent::CloseRequested = event {
            ctx.exit();
        }
    }

    fn update(&mut self, _ctx: &mut Ctx) {}

    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView) {
        self.render(ctx, view);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn screenshot(path: &std::path::Path) -> Result<(), String> {
    const SIZE: (u32, u32) = (854, 480);
    let config = Config {
        size: SIZE,
        init_logging: false,
        ..Default::default()
    };
    let ctx = chad::HeadlessCtx::new(&config)?;
    let triangle = Triangle::new(&ctx);
    triangle.render(&ctx, ctx.view());
    let rgba = ctx.read_rgba8()?;
    common::write_png(path, SIZE.0, SIZE.1, &rgba)
}

fn main() {
    let config = Config {
        title: "chad triangle".into(),
        ..Default::default()
    };
    #[cfg(not(target_arch = "wasm32"))]
    if common::run_screenshot(screenshot) {
        return;
    }
    if let Err(e) = chad::run::<Triangle>(config) {
        eprintln!("{e}");
    }
}

const TRIANGLE_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>( 0.0,  0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
    );
    var colors = array<vec3<f32>, 3>(
        vec3<f32>(1.0, 0.25, 0.35),
        vec3<f32>(0.25, 1.0, 0.45),
        vec3<f32>(0.30, 0.45, 1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(positions[vi], 0.0, 1.0);
    out.color = colors[vi];
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
"#;

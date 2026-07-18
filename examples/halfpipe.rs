//! Fix-your-timestep showcase: a ball rolling in a half-pipe with physics at
//! a deliberately chunky 20 Hz, rendered at display rate.
//!
//! SPACE toggles interpolation. Off, you see the raw 20 Hz steps; on, the
//! renderer blends the last two physics states with `ctx.alpha()` and 20 Hz
//! looks smooth.
//!
//! A ball rolling without slipping in a circular pipe is a pendulum with the
//! solid-sphere factor: theta'' = -(5/7)(g/R) sin(theta). Semi-implicit Euler
//! is symplectic, so the oscillation's energy stays bounded forever — no
//! damping fudge needed. `update_while_minimized` keeps the sim ticking while
//! minimized; minimize it for a while and the ball is where it should be.
//!
//! `cargo run --example halfpipe`

use chad::winit::event::WindowEvent;
use chad::winit::keyboard::{KeyCode, PhysicalKey};
use chad::{wgpu, ChadApp, Config, Ctx, Timestep};

const G_OVER_R: f32 = 9.81; // g/R with R = 1 m
const PIPE_CENTER_Y: f32 = 0.45;
const PIPE_R: f32 = 0.75; // drawn radius (screen units)
const BALL_R: f32 = 0.06;

struct Halfpipe {
    pipeline: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    theta: f32,
    omega: f32,
    prev_theta: f32,
    interp: bool,
}

impl ChadApp for Halfpipe {
    fn init(ctx: &mut Ctx) -> Result<Self, String> {
        let (pipeline, ubuf, bind) = fullscreen_pipeline(ctx, WGSL, 32);
        Ok(Self {
            pipeline,
            ubuf,
            bind,
            theta: 1.15,
            omega: 0.0,
            prev_theta: 1.15,
            interp: true,
        })
    }

    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent) {
        match event {
            WindowEvent::CloseRequested => ctx.exit(),
            WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() && !event.repeat => {
                if event.physical_key == PhysicalKey::Code(KeyCode::Space) {
                    self.interp = !self.interp;
                    ctx.window.set_title(&title(self.interp));
                }
            }
            _ => {}
        }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        self.prev_theta = self.theta;
        self.omega += -(5.0 / 7.0) * G_OVER_R * self.theta.sin() * ctx.dt;
        self.theta += self.omega * ctx.dt;
    }

    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView) {
        let a = if self.interp { ctx.alpha() } else { 1.0 };
        let theta = self.prev_theta + (self.theta - self.prev_theta) * a;
        // Ball center rolls on the pipe's inner surface.
        let r = PIPE_R - 0.02 - BALL_R;
        let (s, c) = theta.sin_cos();
        let (w, h) = ctx.size();
        write_uniforms(ctx, &self.ubuf, &[
            w as f32, h as f32, 0.0, 0.0,
            r * s, PIPE_CENTER_Y - r * c, BALL_R, 0.0,
        ]);
        draw_fullscreen(ctx, view, &self.pipeline, &self.bind);
    }
}

fn title(interp: bool) -> String {
    format!("halfpipe — 20 Hz physics — interp {} (SPACE)", if interp { "ON" } else { "OFF" })
}

fn main() {
    let config = Config {
        title: title(true),
        timestep: Timestep::Fixed { hz: 20, max_updates_per_frame: 5 },
        update_while_minimized: true,
        ..Default::default()
    };
    if let Err(e) = chad::run::<Halfpipe>(config) {
        eprintln!("{e}");
    }
}

// --- boilerplate shared by nothing (examples are self-contained) ---

fn fullscreen_pipeline(
    ctx: &Ctx,
    wgsl: &str,
    uniform_size: u64,
) -> (wgpu::RenderPipeline, wgpu::Buffer, wgpu::BindGroup) {
    let device = &ctx.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("uniforms"),
        size: uniform_size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
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
            targets: &[Some(ctx.surface_format.into())],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, ubuf, bind)
}

fn write_uniforms(ctx: &Ctx, buf: &wgpu::Buffer, data: &[f32]) {
    let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_ne_bytes()).collect();
    ctx.queue.write_buffer(buf, 0, &bytes);
}

fn draw_fullscreen(ctx: &Ctx, view: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, bind: &wgpu::BindGroup) {
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
    ctx.queue.submit(std::iter::once(encoder.finish()));
}

const WGSL: &str = r#"
struct U {
    res: vec4<f32>,   // width, height, _, _
    ball: vec4<f32>,  // x, y, radius, _
};
@group(0) @binding(0) var<uniform> u: U;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(3.0, 1.0), vec2(-1.0, 1.0));
    return vec4<f32>(p[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) fc: vec4<f32>) -> @location(0) vec4<f32> {
    let res = u.res.xy;
    let p = (fc.xy * 2.0 - res) / res.y * vec2(1.0, -1.0);

    let center = vec2(0.0, 0.45);
    let d_circle = length(p - center) - 0.75;
    let d_band = abs(d_circle) - 0.02;
    let d_pipe = select(1e9, d_band, p.y < center.y); // lower half only
    let d_ball = length(p - u.ball.xy) - u.ball.z;

    var col = vec3(0.030, 0.022, 0.055);
    col = mix(vec3(0.55, 0.58, 0.68), col, smoothstep(0.0, 0.005, d_pipe));
    col = mix(vec3(1.00, 0.42, 0.30), col, smoothstep(0.0, 0.005, d_ball));
    return vec4<f32>(col, 1.0);
}
"#;

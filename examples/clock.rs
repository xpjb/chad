//! The "applicationy" side of chad: `RedrawMode::OnDemand` + `Waker`.
//!
//! A stopwatch face that renders exactly once per second. Between renders the
//! event loop sleeps in the OS — near-zero CPU and GPU. A background thread
//! pings `Waker::wake()` once a second; the loop wakes, ticks, presents one
//! frame, and goes back to sleep. Resizes and expose events still redraw
//! immediately, because those flow through the loop as normal events.
//!
//! Also shows a procedural window icon (`Config.icon`) — check the taskbar.
//!
//! `cargo run --example clock`

use chad::winit::event::WindowEvent;
use chad::{wgpu, AppIcon, ChadApp, Config, Ctx, RedrawMode};

struct Clock {
    pipeline: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

impl ChadApp for Clock {
    fn init(ctx: &mut Ctx) -> Result<Self, String> {
        let (pipeline, ubuf, bind) = fullscreen_pipeline(ctx, WGSL, 32);
        // The waker is Send + Clone; hand it to any thread. Payloadless by
        // design: it only promises the loop will come around soon.
        let waker = ctx.waker();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            waker.wake();
        });
        Ok(Self { pipeline, ubuf, bind })
    }

    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent) {
        if let WindowEvent::CloseRequested = event {
            ctx.exit();
        }
    }

    fn update(&mut self, _ctx: &mut Ctx) {}

    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView) {
        const TAU: f32 = std::f32::consts::TAU;
        let e = ctx.elapsed;
        let sec_angle = (e % 60.0) / 60.0 * TAU;
        let min_angle = ((e / 60.0) % 60.0) / 60.0 * TAU;
        let (w, h) = ctx.size();
        write_uniforms(ctx, &self.ubuf, &[
            w as f32, h as f32, 0.0, 0.0,
            sec_angle, min_angle, 0.0, 0.0,
        ]);
        draw_fullscreen(ctx, view, &self.pipeline, &self.bind);
    }
}

/// 32x32 RGBA disc, no image crate needed.
fn icon() -> AppIcon {
    let (w, h) = (32u32, 32u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let d = (dx * dx + dy * dy).sqrt();
            if d < 14.5 {
                rgba.extend([235, 105, (70.0 + d * 9.0) as u8, 255]);
            } else {
                rgba.extend([0, 0, 0, 0]);
            }
        }
    }
    AppIcon { rgba, width: w, height: h }
}

fn main() {
    let config = Config {
        title: "chad clock — redraws once per second".into(),
        size: (480, 480),
        redraw: RedrawMode::OnDemand,
        icon: Some(icon()),
        ..Default::default()
    };
    if let Err(e) = chad::run::<Clock>(config) {
        eprintln!("{e}");
    }
}

// --- boilerplate (examples are self-contained) ---

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
    res: vec4<f32>,    // width, height, _, _
    hands: vec4<f32>,  // second angle, minute angle, _, _
};
@group(0) @binding(0) var<uniform> u: U;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(3.0, 1.0), vec2(-1.0, 1.0));
    return vec4<f32>(p[vi], 0.0, 1.0);
}

fn seg(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

@fragment
fn fs_main(@builtin(position) fc: vec4<f32>) -> @location(0) vec4<f32> {
    let res = u.res.xy;
    let p = (fc.xy * 2.0 - res) / res.y * vec2(1.0, -1.0);

    let d_ring = abs(length(p) - 0.82) - 0.008;
    var d_tick = 1e9;
    for (var i = 0; i < 12; i++) {
        let a = f32(i) / 12.0 * 6.2831853;
        let dir = vec2(sin(a), cos(a));
        d_tick = min(d_tick, seg(p, dir * 0.70, dir * 0.76) - 0.006);
    }
    // Angles measured clockwise from 12 o'clock.
    let sd = vec2(sin(u.hands.x), cos(u.hands.x));
    let md = vec2(sin(u.hands.y), cos(u.hands.y));
    let d_sec = seg(p, vec2(0.0), sd * 0.64) - 0.006;
    let d_min = seg(p, vec2(0.0), md * 0.46) - 0.014;
    let d_hub = length(p) - 0.03;

    var col = vec3(0.045, 0.04, 0.07);
    col = mix(vec3(0.60, 0.63, 0.72), col, smoothstep(0.0, 0.004, min(d_ring, d_tick)));
    col = mix(vec3(0.80, 0.82, 0.90), col, smoothstep(0.0, 0.004, d_min));
    col = mix(vec3(0.95, 0.38, 0.33), col, smoothstep(0.0, 0.004, min(d_sec, d_hub)));
    return vec4<f32>(col, 1.0);
}
"#;

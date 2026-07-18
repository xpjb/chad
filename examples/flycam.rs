//! First-person crawl through the endless interior of a repeated Mandelbox.
//!
//! Space is filled by tiling a scale -1.75 Mandelbox with a period (8) smaller
//! than the structure itself, so the copies weld into one continuous alien
//! hulk. You spawn inside the shell, where the veiny corridors live. There is
//! no sun and no sky light — you carry a headlamp, and the structure's
//! orbit-trap veins glow on their own.
//!
//! chad machinery on show:
//! - raymarch at a capped internal resolution (<= 640 wide) into an offscreen
//!   texture, then upscale-blit to the surface — fractal cost stops scaling
//!   with window size
//! - CPU-mirrored SDF for real collision: you cannot clip through the walls
//! - click captures the mouse for mouselook; Esc releases it (close the
//!   window with the X); raw deltas via `device_event`
//! - 1-pole filtered movement, Shift to sprint, variable `ctx.dt`
//! - V toggles vsync at runtime, F fullscreen, `max_fps` caps vsync-off
//!
//! `cargo run --example flycam`

use std::collections::HashSet;

use chad::winit::event::{DeviceEvent, MouseButton, WindowEvent};
use chad::winit::keyboard::{KeyCode, PhysicalKey};
use chad::winit::window::CursorGrabMode;
use chad::{wgpu, ChadApp, Config, Ctx};

/// World scale: the fractal is evaluated in its own unit space and blown up
/// by this factor, so the player is ant-sized inside the corridor detail.
const WORLD_SCALE: f32 = 400.0;
const PLAYER_RADIUS: f32 = 0.1;
const BASE_SPEED: f32 = 15.0;
const SPRINT_MULT: f32 = 4.0;
const MAX_INTERNAL_WIDTH: u32 = 640;

struct Flycam {
    pipeline: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    low_view: wgpu::TextureView,
    low_size: (u32, u32),
    blit_bind: wgpu::BindGroup,
    pos: [f32; 3],
    vel: [f32; 3],
    yaw: f32,
    pitch: f32,
    keys: HashSet<KeyCode>,
    grabbed: bool,
    vsync: bool,
    fps_time: f32,
    fps_frames: u32,
}

impl Flycam {
    fn set_grab(&mut self, ctx: &Ctx, on: bool) {
        if on {
            let _ = ctx
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| ctx.window.set_cursor_grab(CursorGrabMode::Confined));
        } else {
            let _ = ctx.window.set_cursor_grab(CursorGrabMode::None);
        }
        ctx.window.set_cursor_visible(!on);
        self.grabbed = on;
    }

    fn remake_offscreen(&mut self, ctx: &Ctx) {
        let (view, size) = make_offscreen(ctx);
        self.blit_bind = make_blit_bind(ctx, &self.blit_bgl, &self.sampler, &view);
        self.low_view = view;
        self.low_size = size;
    }
}

impl ChadApp for Flycam {
    fn init(ctx: &mut Ctx) -> Result<Self, String> {
        let (pipeline, ubuf, bind) = fullscreen_pipeline(ctx, WGSL, 48);
        let (blit_pipeline, blit_bgl, sampler) = make_blit_pipeline(ctx);
        let (low_view, low_size) = make_offscreen(ctx);
        let blit_bind = make_blit_bind(ctx, &blit_bgl, &sampler, &low_view);
        Ok(Self {
            pipeline,
            ubuf,
            bind,
            blit_pipeline,
            blit_bgl,
            sampler,
            low_view,
            low_size,
            blit_bind,
            // A pocket in the flush-tiled field (found by probing the SDF at
            // (-2.0, -1.6, -1.44) in fractal units), scaled to world.
            pos: [-2.0 * WORLD_SCALE, -1.6 * WORLD_SCALE, -1.44 * WORLD_SCALE],
            vel: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            keys: HashSet::new(),
            grabbed: false,
            vsync: true,
            fps_time: 0.0,
            fps_frames: 0,
        })
    }

    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent) {
        match event {
            WindowEvent::CloseRequested => ctx.exit(),
            WindowEvent::Resized(_) => self.remake_offscreen(ctx),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. }
                if state.is_pressed() && !self.grabbed =>
            {
                self.set_grab(ctx, true);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                if event.state.is_pressed() {
                    self.keys.insert(code);
                    if !event.repeat {
                        match code {
                            KeyCode::Escape => self.set_grab(ctx, false),
                            KeyCode::KeyV => {
                                self.vsync = !self.vsync;
                                ctx.set_vsync(self.vsync);
                            }
                            KeyCode::KeyF => ctx.toggle_fullscreen(),
                            _ => {}
                        }
                    }
                } else {
                    self.keys.remove(&code);
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _ctx: &mut Ctx, event: &DeviceEvent) {
        if !self.grabbed {
            return;
        }
        if let DeviceEvent::MouseMotion { delta } = event {
            self.yaw += delta.0 as f32 * 0.002;
            self.pitch = (self.pitch - delta.1 as f32 * 0.002).clamp(-1.5, 1.5);
        }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        let (sy, cy) = self.yaw.sin_cos();
        let fwd = [sy, 0.0, -cy];
        let right = [cy, 0.0, sy];
        let mut wish = [0.0f32; 3];
        let mut add = |dir: [f32; 3], sign: f32| {
            wish[0] += dir[0] * sign;
            wish[1] += dir[1] * sign;
            wish[2] += dir[2] * sign;
        };
        if self.keys.contains(&KeyCode::KeyW) { add(fwd, 1.0) }
        if self.keys.contains(&KeyCode::KeyS) { add(fwd, -1.0) }
        if self.keys.contains(&KeyCode::KeyD) { add(right, 1.0) }
        if self.keys.contains(&KeyCode::KeyA) { add(right, -1.0) }
        if self.keys.contains(&KeyCode::Space) { add([0.0, 1.0, 0.0], 1.0) }
        if self.keys.contains(&KeyCode::ControlLeft) { add([0.0, 1.0, 0.0], -1.0) }
        let speed = BASE_SPEED
            * if self.keys.contains(&KeyCode::ShiftLeft) { SPRINT_MULT } else { 1.0 };

        // 1-pole low-pass on velocity: exponential approach to the wish
        // velocity, framerate-independent.
        let k = 1.0 - (-10.0 * ctx.dt).exp();
        for i in 0..3 {
            self.vel[i] += (wish[i] * speed - self.vel[i]) * k;
            self.pos[i] += self.vel[i] * ctx.dt;
        }
        resolve_collision(&mut self.pos);

        self.fps_frames += 1;
        self.fps_time += ctx.dt;
        if self.fps_time >= 0.5 {
            let fps = self.fps_frames as f32 / self.fps_time;
            ctx.window.set_title(&format!(
                "flycam — {fps:.0} fps — click to look, Esc frees mouse — WASD+Shift — V vsync {} — F fullscreen",
                if self.vsync { "ON" } else { "OFF" }
            ));
            self.fps_time = 0.0;
            self.fps_frames = 0;
        }
    }

    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView) {
        write_uniforms(ctx, &self.ubuf, &[
            self.low_size.0 as f32, self.low_size.1 as f32, ctx.elapsed, 0.0,
            self.pos[0], self.pos[1], self.pos[2], 0.0,
            self.yaw, self.pitch, 0.0, 0.0,
        ]);
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        // Pass 1: raymarch at capped internal resolution.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("raymarch-lowres"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.low_view,
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        // Pass 2: upscale to the window.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
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
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &self.blit_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        ctx.queue.submit(std::iter::once(encoder.finish()));
    }
}

fn main() {
    let config = Config {
        title: "flycam — click to look, Esc frees mouse".into(),
        max_fps: Some(240),
        ..Default::default()
    };
    if let Err(e) = chad::run::<Flycam>(config) {
        eprintln!("{e}");
    }
}

// --- CPU mirror of the shader's distance field, for collision ---
// (Keep in sync with WGSL `map` below: same folds, same 0.7 safety factor.)

// Tile period 4.2 = the measured solid extent of the mandelbox (+/-2.1), so
// the copies repeat flush: faces meet exactly, one continuous megastructure.
fn rep(c: f32) -> f32 {
    (c + 2.1).rem_euclid(4.2) - 2.1
}

fn map_cpu(p: [f32; 3]) -> f32 {
    let s = [p[0] / WORLD_SCALE, p[1] / WORLD_SCALE, p[2] / WORLD_SCALE];
    let q = [rep(s[0]), rep(s[1]), rep(s[2])];
    let (mut x, mut y, mut z) = (q[0], q[1], q[2]);
    let mut dr = 1.0f32;
    for _ in 0..12 {
        x = x.clamp(-1.0, 1.0) * 2.0 - x;
        y = y.clamp(-1.0, 1.0) * 2.0 - y;
        z = z.clamp(-1.0, 1.0) * 2.0 - z;
        let r2 = x * x + y * y + z * z;
        if r2 < 0.25 {
            x *= 4.0; y *= 4.0; z *= 4.0; dr *= 4.0;
        } else if r2 < 1.0 {
            x /= r2; y /= r2; z /= r2; dr /= r2;
        }
        x = x * -1.75 + q[0];
        y = y * -1.75 + q[1];
        z = z * -1.75 + q[2];
        dr = dr * 1.75 + 1.0;
    }
    // 0.8: conservative step factor (the field is continuous across tiles —
    // the mandelbox is even-symmetric — but the DE is an estimate).
    (x * x + y * y + z * z).sqrt() / dr.abs() * 0.8 * WORLD_SCALE
}

fn normal_cpu(p: [f32; 3]) -> [f32; 3] {
    let e = 0.05;
    let n = [
        map_cpu([p[0] + e, p[1], p[2]]) - map_cpu([p[0] - e, p[1], p[2]]),
        map_cpu([p[0], p[1] + e, p[2]]) - map_cpu([p[0], p[1] - e, p[2]]),
        map_cpu([p[0], p[1], p[2] + e]) - map_cpu([p[0], p[1], p[2] - e]),
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
    [n[0] / len, n[1] / len, n[2] / len]
}

/// Sphere-vs-SDF: if the player sphere penetrates a wall, push it out along
/// the field gradient. A few iterations settle corners.
fn resolve_collision(pos: &mut [f32; 3]) {
    for _ in 0..3 {
        let d = map_cpu(*pos);
        if d >= PLAYER_RADIUS {
            break;
        }
        let n = normal_cpu(*pos);
        let push = (PLAYER_RADIUS - d) + 0.01;
        for i in 0..3 {
            pos[i] += n[i] * push;
        }
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

fn make_offscreen(ctx: &Ctx) -> (wgpu::TextureView, (u32, u32)) {
    let (w, h) = ctx.size();
    let lw = MAX_INTERNAL_WIDTH.min(w.max(1));
    let lh = ((lw as f32 * h.max(1) as f32 / w.max(1) as f32).round() as u32).max(1);
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lowres"),
        size: wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: ctx.surface_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    (tex.create_view(&wgpu::TextureViewDescriptor::default()), (lw, lh))
}

fn make_blit_pipeline(ctx: &Ctx) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
    let device = &ctx.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
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
    (pipeline, bgl, sampler)
}

fn make_blit_bind(
    ctx: &Ctx,
    bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

fn write_uniforms(ctx: &Ctx, buf: &wgpu::Buffer, data: &[f32]) {
    let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_ne_bytes()).collect();
    ctx.queue.write_buffer(buf, 0, &bytes);
}

const WGSL: &str = r#"
struct U {
    res: vec4<f32>,  // internal width, internal height, time, _
    pos: vec4<f32>,  // camera xyz, _
    cam: vec4<f32>,  // yaw, pitch, _, _
};
@group(0) @binding(0) var<uniform> u: U;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(3.0, 1.0), vec2(-1.0, 1.0));
    return vec4<f32>(p[vi], 0.0, 1.0);
}

// Orbit trap from the most recent map() call (per-invocation private).
var<private> g_trap: f32 = 0.0;

const W: f32 = 400.0; // world scale; keep in sync with WORLD_SCALE in Rust

// Repeated Mandelbox (scale -1.75, 12 folds), tiled at period 4.2 — the
// measured solid extent of the structure — so copies repeat flush and space
// is one continuous megastructure, blown up by W so the player is ant-sized
// in its seams and chambers. Keep in sync with map_cpu() in the Rust half —
// collision uses the same field.
fn map(p_world: vec3<f32>) -> f32 {
    let p = p_world / W;
    let q = p - 4.2 * floor((p + vec3(2.1)) / 4.2);
    var z = q;
    var dr = 1.0;
    var trap = 1e9;
    for (var i = 0; i < 12; i++) {
        z = clamp(z, vec3(-1.0), vec3(1.0)) * 2.0 - z; // box fold
        let r2 = dot(z, z);
        if (r2 < 0.25) {
            z *= 4.0;
            dr *= 4.0;
        } else if (r2 < 1.0) {
            z /= r2;
            dr /= r2;
        }
        z = z * -1.75 + q;
        dr = dr * 1.75 + 1.0;
        trap = min(trap, length(z));
    }
    g_trap = trap;
    // 0.8: conservative step factor (the field is continuous across tiles —
    // the mandelbox is even-symmetric — but the DE is an estimate).
    return length(z) / abs(dr) * 0.8 * W;
}

// Normal sampled at the pixel footprint for the hit distance: distant
// surfaces get smooth normals instead of subpixel shading shimmer.
fn normal(p: vec3<f32>, eps: f32) -> vec3<f32> {
    let e = vec2(eps, 0.0);
    return normalize(vec3(
        map(p + e.xyy) - map(p - e.xyy),
        map(p + e.yxy) - map(p - e.yxy),
        map(p + e.yyx) - map(p - e.yyx),
    ));
}

// SDF ambient occlusion: sample the field a few steps along the normal;
// the shortfall vs. an unoccluded field is how boxed-in the point is.
fn ao(p: vec3<f32>, n: vec3<f32>) -> f32 {
    var occ = 0.0;
    var sca = 1.0;
    let hs = W * 0.02; // AO radius ~ a few meters at player scale
    for (var i = 0; i < 5; i++) {
        let h = (0.005 + 0.06 * f32(i) / 4.0) * hs;
        occ += (h - map(p + n * h)) * sca;
        sca *= 0.9;
    }
    return clamp(1.0 - (20.0 / hs) * occ, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) fc: vec4<f32>) -> @location(0) vec4<f32> {
    let res = u.res.xy;
    let uv = (fc.xy * 2.0 - res) / res.y * vec2(1.0, -1.0);

    let cy = cos(u.cam.x); let sy = sin(u.cam.x);
    let cp = cos(u.cam.y); let sp = sin(u.cam.y);
    let fwd = vec3(sy * cp, sp, -cy * cp);
    let right = vec3(cy, 0.0, sy);
    let up = cross(right, fwd);
    let ro = u.pos.xyz;
    let rd = normalize(fwd + uv.x * right + uv.y * up);

    // Angular size of one internal pixel: resolving the surface any finer
    // than this is invisible and shimmers, so the hit epsilon grows with t —
    // geometric LOD matched to the render resolution.
    let px = 2.0 / res.y;
    var t = 0.0;
    var hit = false;
    // The mandelbox DE is an estimate, not exact — it overshoots near the
    // surface, so step conservatively (0.6) with more iterations.
    for (var i = 0; i < 220; i++) {
        let d = map(ro + rd * t);
        if (d < max(t * px * 0.5, 0.02)) { hit = true; break; }
        t += d * 0.6;
        if (t > 2500.0) { break; }
    }

    let haze = vec3(0.004, 0.005, 0.012);
    var col = haze;
    if (hit) {
        let p = ro + rd * t;
        let trap = g_trap; // save before normal/ao overwrite it
        // Floor well above the micro-porosity: sub-player-scale detail shades
        // as rounded rock instead of spidery normal noise.
        let n = normal(p, max(0.25, t * px));
        let occ = ao(p, n);
        // Headlamp: point light carried by the camera. The only light source,
        // so nothing ever shines through walls.
        let lvec = ro - p;
        let ldist = max(length(lvec), 1e-4);
        let ldir = lvec / ldist;
        let atten = 2.0 / (1.0 + pow(ldist / 35.0, 2.0));
        let dif = clamp(dot(n, ldir), 0.0, 1.0);
        // Orbit-trap palette: position in the fractal picks the hue.
        let base = 0.45 + 0.35 * cos(6.2831853 * (trap * 0.30 + vec3(0.00, 0.10, 0.22)) + 0.8);
        var lin = vec3(0.0);
        lin += dif * atten * vec3(1.00, 0.93, 0.82);   // headlamp
        lin += occ * vec3(0.045, 0.060, 0.095);        // faint cool ambient
        let rim = pow(clamp(1.0 + dot(rd, n), 0.0, 1.0), 3.0);
        lin += 0.15 * rim * occ * vec3(0.20, 0.28, 0.45);
        let hal = normalize(ldir - rd);
        let spe = pow(clamp(dot(n, hal), 0.0, 1.0), 64.0) * dif * atten;
        col = base * lin + 0.35 * spe * vec3(1.0, 0.97, 0.90);
        // Glowing veins where the orbit passed near the origin: the structure
        // lights its own guts.
        col += vec3(1.0, 0.45, 0.15) * 0.8 * exp(-1.6 * trap) * occ;
        col = mix(col, haze, 1.0 - exp(-0.003 * t));
    }
    // ACES-ish tone map; the sRGB surface handles gamma encoding.
    col = clamp((col * (2.51 * col + 0.03)) / (col * (2.43 * col + 0.59) + 0.14), vec3(0.0), vec3(1.0));
    return vec4<f32>(col, 1.0);
}
"#;

const BLIT_WGSL: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(3.0, 1.0), vec2(-1.0, 1.0));
    var out: VsOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    out.uv = vec2(p[vi].x, -p[vi].y) * 0.5 + 0.5;
    return out;
}

// FXAA on the low-res image before it stretches: directional blur along
// detected edges, luma-clamped so it can't smear detail.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let inv = 1.0 / vec2<f32>(textureDimensions(tex));
    let uv = in.uv;
    let rgb_nw = textureSampleLevel(tex, smp, uv + vec2(-1.0, -1.0) * inv, 0.0).rgb;
    let rgb_ne = textureSampleLevel(tex, smp, uv + vec2(1.0, -1.0) * inv, 0.0).rgb;
    let rgb_sw = textureSampleLevel(tex, smp, uv + vec2(-1.0, 1.0) * inv, 0.0).rgb;
    let rgb_se = textureSampleLevel(tex, smp, uv + vec2(1.0, 1.0) * inv, 0.0).rgb;
    let rgb_m = textureSampleLevel(tex, smp, uv, 0.0).rgb;
    let luma = vec3(0.299, 0.587, 0.114);
    let l_nw = dot(rgb_nw, luma);
    let l_ne = dot(rgb_ne, luma);
    let l_sw = dot(rgb_sw, luma);
    let l_se = dot(rgb_se, luma);
    let l_m = dot(rgb_m, luma);
    let l_min = min(l_m, min(min(l_nw, l_ne), min(l_sw, l_se)));
    let l_max = max(l_m, max(max(l_nw, l_ne), max(l_sw, l_se)));
    // Early out on low-contrast areas: only real edges get blurred, flat
    // texture stays crisp.
    if (l_max - l_min < 0.08) {
        return vec4<f32>(rgb_m, 1.0);
    }

    var dir = vec2(-((l_nw + l_ne) - (l_sw + l_se)), (l_nw + l_sw) - (l_ne + l_se));
    let dir_reduce = max((l_nw + l_ne + l_sw + l_se) * 0.25 * 0.125, 1.0 / 128.0);
    let rcp = 1.0 / (min(abs(dir.x), abs(dir.y)) + dir_reduce);
    dir = clamp(dir * rcp, vec2(-8.0), vec2(8.0)) * inv;

    let rgb_a = 0.5
        * (textureSampleLevel(tex, smp, uv + dir * (1.0 / 3.0 - 0.5), 0.0).rgb
            + textureSampleLevel(tex, smp, uv + dir * (2.0 / 3.0 - 0.5), 0.0).rgb);
    let rgb_b = rgb_a * 0.5
        + 0.25
            * (textureSampleLevel(tex, smp, uv + dir * -0.5, 0.0).rgb
                + textureSampleLevel(tex, smp, uv + dir * 0.5, 0.0).rgb);
    let l_b = dot(rgb_b, luma);
    var col = rgb_b;
    if (l_b < l_min || l_b > l_max) {
        col = rgb_a;
    }
    return vec4<f32>(col, 1.0);
}
"#;

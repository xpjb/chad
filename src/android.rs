//! NativeActivity/Vulkan runner, separate from the desktop/web runner.
//!
//! Call [`run`] from `android_main`. The game and GPU survive surface suspension;
//! the drawing surface is released before the suspend callback returns. Process
//! death still ends the game: durable saves and game-time policy belong to the app.

use crate::{RenderContext, wgpu};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::android::EventLoopBuilderExtAndroid;
use winit::window::Window;

pub use winit::platform::android::activity::AndroidApp;

/// GPU requirements and presentation policy. Android owns the window size.
pub struct Config {
    pub device_features: wgpu::Features,
    pub device_limits: wgpu::Limits,
    pub present_mode: wgpu::PresentMode,
    pub desired_maximum_frame_latency: u32,
    /// Continuous for games; OnDemand redraws on input or window.request_redraw().
    pub redraw: crate::RedrawMode,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device_features: wgpu::Features::empty(),
            device_limits: wgpu::Limits::downlevel_defaults(),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 1,
            redraw: crate::RedrawMode::Continuous,
        }
    }
}

/// GPU and input context. The runner owns the separate, temporary surface.
pub struct Ctx {
    pub app: AndroidApp,
    /// The native surface behind this window is unavailable while suspended.
    pub window: Arc<Window>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_format: wgpu::TextureFormat,
    /// Real seconds since the previous update, reset on resume and focus changes.
    pub dt: f32,
    /// Wall-clock seconds since GPU initialization; includes suspended time.
    pub elapsed: f32,
    pub frame_index: u64,
    pub focused: bool,
    size: (u32, u32),
    exit: bool,
}

impl Ctx {
    /// Last configured, nonzero surface size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Return from the Android runner. NativeActivity then finishes the activity.
    pub fn exit(&mut self) {
        self.exit = true;
    }
}

impl RenderContext for Ctx {
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
        self.size
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
        1.0
    }
}

/// Callbacks run on the NativeActivity `android_main` thread, not Java's UI thread.
pub trait App: Sized {
    /// Called once for this runner, after the first surface and GPU are ready.
    fn init(ctx: &mut Ctx) -> Result<Self, String>;
    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent);
    fn device_event(&mut self, _ctx: &mut Ctx, _event: &DeviceEvent) {}
    /// Called after init and whenever a suspended surface has been recreated.
    fn resumed(&mut self, _ctx: &mut Ctx) {}
    /// The surface has already been released. Use this to cancel input or save.
    fn suspended(&mut self, _ctx: &mut Ctx) {}
    /// One variable-step update per redraw, only while a surface exists.
    fn update(&mut self, ctx: &mut Ctx);
    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView);
}

/// Run a single NativeActivity. Android's glue forwards stdout/stderr to logcat.
pub fn run<G: App + 'static>(app: AndroidApp, config: Config) -> Result<(), String> {
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"),
    )
    .try_init();
    let event_loop = EventLoop::builder()
        .with_android_app(app.clone())
        .build()
        .map_err(|error| format!("Android event loop: {error}"))?;
    let mut runner = Runner::<G> {
        app,
        config,
        state: None,
        error: None,
    };
    event_loop
        .run_app(&mut runner)
        .map_err(|error| format!("Android event loop: {error}"))?;
    match runner.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct Runner<G> {
    app: AndroidApp,
    config: Config,
    state: Option<State<G>>,
    error: Option<String>,
}

struct State<G> {
    game: G,
    ctx: Ctx,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    surface: Option<wgpu::Surface<'static>>,
    surface_config: wgpu::SurfaceConfiguration,
    start: Instant,
    last: Instant,
}

impl<G> State<G> {
    fn attach_surface(&mut self) -> Result<(), String> {
        let surface = self
            .instance
            .create_surface(self.ctx.window.clone())
            .map_err(|error| format!("recreate Android surface: {error}"))?;
        let caps = surface.get_capabilities(&self.adapter);
        if !caps.formats.contains(&self.surface_config.format) {
            return Err("Android surface format changed; restart the app".into());
        }
        let size = self.ctx.window.inner_size();
        self.surface_config.width = size.width.max(1);
        self.surface_config.height = size.height.max(1);
        surface.configure(&self.ctx.device, &self.surface_config);
        self.ctx.size = (self.surface_config.width, self.surface_config.height);
        self.surface = Some(surface);
        self.last = Instant::now();
        self.ctx.dt = 0.0;
        self.ctx.elapsed = (self.last - self.start).as_secs_f32();
        self.ctx.focused = self.ctx.window.has_focus();
        log::info!(
            "Android surface ready: {}x{}",
            self.ctx.size.0,
            self.ctx.size.1
        );
        Ok(())
    }
}

impl<G> Runner<G> {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        log::error!("{error}");
        self.error = Some(error);
        event_loop.exit();
    }
}

impl<G: App> ApplicationHandler for Runner<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(state) = &mut self.state {
            if state.surface.is_some() {
                return;
            }
            if let Err(error) = state.attach_surface() {
                self.fail(event_loop, error);
                return;
            }
            state.game.resumed(&mut state.ctx);
            if state.ctx.exit {
                event_loop.exit();
            } else {
                state.ctx.window.request_redraw();
            }
            return;
        }

        let result = pollster::block_on(async {
            let window = Arc::new(
                event_loop
                    .create_window(Window::default_attributes())
                    .map_err(|error| format!("Android window: {error}"))?,
            );
            let mut descriptor = wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(window.clone()),
            );
            descriptor.backends = wgpu::Backends::VULKAN;
            let instance = wgpu::Instance::new(descriptor);
            let surface = instance
                .create_surface(window.clone())
                .map_err(|error| format!("Android surface: {error}"))?;
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                    apply_limit_buckets: false,
                })
                .await
                .map_err(|error| format!("Android Vulkan adapter: {error}"))?;
            let info = adapter.get_info();
            log::info!("Android GPU: {} ({:?})", info.name, info.backend);
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("chad-android-device"),
                    required_features: self.config.device_features,
                    required_limits: self.config.device_limits.clone(),
                    ..Default::default()
                })
                .await
                .map_err(|error| format!("Android GPU requirements: {error}"))?;
            let caps = surface.get_capabilities(&adapter);
            let format = caps
                .formats
                .iter()
                .copied()
                .find(|format| format.is_srgb())
                .or_else(|| caps.formats.first().copied())
                .ok_or("Android surface has no formats")?;
            let surface_format = format.add_srgb_suffix();
            let size = window.inner_size();
            let surface_config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: self.config.present_mode,
                alpha_mode: caps.alpha_modes[0],
                view_formats: if surface_format == format {
                    vec![]
                } else {
                    vec![surface_format]
                },
                desired_maximum_frame_latency: self.config.desired_maximum_frame_latency,
                color_space: wgpu::SurfaceColorSpace::Auto,
            };
            surface.configure(&device, &surface_config);
            let mut ctx = Ctx {
                app: self.app.clone(),
                focused: window.has_focus(),
                window,
                device,
                queue,
                surface_format,
                dt: 0.0,
                elapsed: 0.0,
                frame_index: 0,
                size: (surface_config.width, surface_config.height),
                exit: false,
            };
            let game = G::init(&mut ctx).map_err(|error| format!("Android game init: {error}"))?;
            let now = Instant::now();
            Ok::<_, String>(State {
                game,
                ctx,
                instance,
                adapter,
                surface: Some(surface),
                surface_config,
                start: now,
                last: now,
            })
        });
        match result {
            Ok(mut state) => {
                state.game.resumed(&mut state.ctx);
                if state.ctx.exit {
                    event_loop.exit();
                } else {
                    state.ctx.window.request_redraw();
                }
                self.state = Some(state);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(state) = &mut self.state {
            let Some(surface) = state.surface.take() else {
                return;
            };
            drop(surface);
            state.ctx.elapsed = state.start.elapsed().as_secs_f32();
            state.ctx.dt = 0.0;
            state.ctx.focused = false;
            state.game.suspended(&mut state.ctx);
            log::info!("Android surface suspended");
            if state.ctx.exit {
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else { return };
        state.ctx.elapsed = state.start.elapsed().as_secs_f32();
        match &event {
            WindowEvent::Focused(focused) => {
                state.ctx.focused = *focused;
                state.last = Instant::now();
                state.ctx.dt = 0.0;
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                if let Some(surface) = &state.surface {
                    state.surface_config.width = size.width;
                    state.surface_config.height = size.height;
                    surface.configure(&state.ctx.device, &state.surface_config);
                    state.ctx.size = (size.width, size.height);
                    state.ctx.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let Some(surface) = &state.surface else {
                    return;
                };
                let size = state.ctx.window.inner_size();
                if size.width == 0 || size.height == 0 {
                    return;
                }
                let now = Instant::now();
                state.ctx.dt = (now - state.last).as_secs_f32();
                state.ctx.elapsed = (now - state.start).as_secs_f32();
                state.last = now;
                state.game.update(&mut state.ctx);
                if state.ctx.exit {
                    event_loop.exit();
                    return;
                }
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame) => Some(frame),
                    wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        state.surface_config.width = size.width;
                        state.surface_config.height = size.height;
                        surface.configure(&state.ctx.device, &state.surface_config);
                        state.ctx.size = (size.width, size.height);
                        None
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        state.surface = None;
                        if let Err(error) = state.attach_surface() {
                            self.fail(event_loop, error);
                            return;
                        }
                        None
                    }
                    wgpu::CurrentSurfaceTexture::Occluded
                    | wgpu::CurrentSurfaceTexture::Timeout => None,
                    wgpu::CurrentSurfaceTexture::Validation => {
                        self.fail(event_loop, "Android surface validation failed".into());
                        return;
                    }
                };
                let frame_retry = frame.is_none();
                if let Some(frame) = frame {
                    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                        format: Some(state.ctx.surface_format),
                        ..Default::default()
                    });
                    state.game.frame(&mut state.ctx, &view);
                    state.ctx.window.pre_present_notify();
                    state.ctx.queue.present(frame);
                    state.ctx.frame_index += 1;
                }
                if state.ctx.exit {
                    event_loop.exit();
                } else if matches!(self.config.redraw, crate::RedrawMode::Continuous) || frame_retry
                {
                    state.ctx.window.request_redraw();
                }
                return;
            }
            _ => {}
        }
        state.game.event(&mut state.ctx, &event);
        if state.ctx.exit {
            event_loop.exit();
        } else if state.surface.is_some() {
            state.ctx.window.request_redraw();
        }
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let Some(state) = &mut self.state {
            state.game.device_event(&mut state.ctx, &event);
            if state.ctx.exit {
                event_loop.exit();
            } else if state.surface.is_some() {
                state.ctx.window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

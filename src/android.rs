//! Experimental Android runner driven by a regular Java `Activity` UI Looper.
//!
//! Use [`android_activity_entry!`](crate::android_activity_entry) instead of exporting
//! `android_main`. The application model, WGPU rendering, pointer dispatch, Back handling, and
//! synchronous text client all run serially on Android's UI thread.

use std::sync::Arc;
use std::time::Instant;

use crate::{
    MobileEvent, MobileViewport, PhysicalRect, RenderContext, TextInputOptions, TextInputState,
    wgpu, winit,
};
use winit::application::{ApplicationHandler, BackResponse};
use winit::event::{DeviceEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::platform::android::java_activity::WindowExtJavaActivity;
use winit::text_input::{
    TextInputRequest, TextInputResponse, TextInputSessionId,
};
use winit::window::{
    ImeCapabilities, ImeEnableRequest, ImeHint, ImePurpose, ImeRequest, ImeRequestData, Window,
};

/// GPU requirements and presentation policy. Android owns the window size.
pub struct Config {
    pub device_features: wgpu::Features,
    pub device_limits: wgpu::Limits,
    pub present_mode: wgpu::PresentMode,
    pub desired_maximum_frame_latency: u32,
    /// Continuous for games; OnDemand redraws on input or `window.request_redraw()`.
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct ActiveTextInput {
    id: u64,
    options: TextInputOptions,
    field_rect: PhysicalRect,
    caret_rect: PhysicalRect,
    enabled: bool,
}

/// GPU and input context. The Android Activity owns the event loop and surface lifecycle.
pub struct Ctx {
    pub window: Arc<dyn Window>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_format: wgpu::TextureFormat,
    /// Real seconds since the previous update, reset on surface recreation and focus changes.
    pub dt: f32,
    /// Wall-clock seconds since GPU initialization; includes suspended time.
    pub elapsed: f32,
    pub frame_index: u64,
    pub focused: bool,
    mobile_viewport: MobileViewport,
    text_input: Option<ActiveTextInput>,
    size: (u32, u32),
    exit: bool,
}

impl Ctx {
    /// Last configured, nonzero surface size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn mobile_viewport(&self) -> MobileViewport {
        self.mobile_viewport
    }

    pub fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// Configures the focused text session. The supplied text window is no longer mirrored into
    /// Java; synchronous [`App::text_input_request`] callbacks are the document source of truth.
    /// `generation`, `document_start`, `text`, and `selection` remain in this transitional
    /// signature only so applications can migrate without changing focus plumbing first.
    pub fn set_text_input(&mut self, state: TextInputState<'_>) -> Result<(), String> {
        if state.id == 0 {
            return Err("text input ID must be nonzero".into());
        }
        let next = ActiveTextInput {
            id: state.id,
            options: state.options,
            field_rect: state.field_rect,
            caret_rect: state.caret_rect,
            enabled: true,
        };
        let previous = self.text_input;
        if previous.map(|input| input.id) != Some(state.id) {
            if previous.is_some_and(|input| input.enabled) {
                self.window
                    .request_ime_update(ImeRequest::Disable)
                    .map_err(|error| format!("disable previous Android text input: {error}"))?;
            }
            self.window
                .as_ref()
                .set_text_input_session(TextInputSessionId::from_raw(state.id));
            self.enable_text_input(next)?;
        } else if previous != Some(next) {
            self.update_text_input(next)?;
        }
        self.text_input = Some(next);
        Ok(())
    }

    pub fn clear_text_input(&mut self) -> Result<(), String> {
        if self.text_input.is_some_and(|input| input.enabled) {
            self.window
                .request_ime_update(ImeRequest::Disable)
                .map_err(|error| format!("disable Android text input: {error}"))?;
        }
        self.text_input = None;
        Ok(())
    }

    pub fn show_keyboard(&mut self) -> Result<(), String> {
        let Some(mut input) = self.text_input else {
            return Err("no Android text input session is configured".into());
        };
        if !input.enabled {
            input.enabled = true;
            self.window
                .as_ref()
                .set_text_input_session(TextInputSessionId::from_raw(input.id));
            self.enable_text_input(input)?;
            self.text_input = Some(input);
        }
        Ok(())
    }

    pub fn hide_keyboard(&mut self) -> Result<(), String> {
        let Some(mut input) = self.text_input else {
            return Ok(());
        };
        if input.enabled {
            self.window
                .request_ime_update(ImeRequest::Disable)
                .map_err(|error| format!("hide Android keyboard: {error}"))?;
            input.enabled = false;
            self.text_input = Some(input);
        }
        Ok(())
    }

    pub fn move_task_to_back(&mut self) -> Result<(), String> {
        Err("move_task_to_back is not implemented by the Java-loop experiment".into())
    }

    pub fn show_text_menu(&self) -> Result<(), String> {
        Err("native Android text menus are not implemented by the Java-loop experiment".into())
    }

    pub fn selection_haptic(&self) -> Result<(), String> {
        Err("Android selection haptics are not implemented by the Java-loop experiment".into())
    }

    /// Finish the Activity after the current callback returns.
    pub fn exit(&mut self) {
        self.exit = true;
    }

    fn enable_text_input(&self, input: ActiveTextInput) -> Result<(), String> {
        let capabilities = ImeCapabilities::new()
            .with_hint_and_purpose()
            .with_cursor_area();
        let data = ime_data(input);
        let request = ImeEnableRequest::new(capabilities, data)
            .ok_or("invalid Android IME enable request")?;
        self.window
            .request_ime_update(ImeRequest::Enable(request))
            .map_err(|error| format!("enable Android text input: {error}"))
    }

    fn update_text_input(&self, input: ActiveTextInput) -> Result<(), String> {
        if !input.enabled {
            return Ok(());
        }
        self.window
            .request_ime_update(ImeRequest::Update(ime_data(input)))
            .map_err(|error| format!("update Android text input: {error}"))
    }
}

fn ime_data(input: ActiveTextInput) -> ImeRequestData {
    let mut hint = ImeHint::COMPLETION | ImeHint::SPELLCHECK;
    let purpose = if input.options.secure {
        hint |= ImeHint::HIDDEN_TEXT | ImeHint::SENSITIVE_DATA;
        ImePurpose::Password
    } else {
        ImePurpose::Normal
    };
    if input.options.multiline {
        hint |= ImeHint::MULTILINE;
    }
    ImeRequestData::default()
        .with_hint_and_purpose(hint, purpose)
        .with_cursor_area(
            winit::dpi::PhysicalPosition::new(input.caret_rect.x, input.caret_rect.y).into(),
            winit::dpi::PhysicalSize::new(
                input.caret_rect.width.max(1.0),
                input.caret_rect.height.max(1.0),
            )
            .into(),
        )
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

/// Application callbacks are serialized on Android's Java UI thread.
pub trait App: Sized {
    fn init(ctx: &mut Ctx) -> Result<Self, String>;
    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent);
    fn mobile_event(&mut self, _ctx: &mut Ctx, _event: MobileEvent) {}
    fn device_event(&mut self, _ctx: &mut Ctx, _event: &DeviceEvent) {}
    /// Called after init and whenever a destroyed Android surface has been recreated.
    fn resumed(&mut self, _ctx: &mut Ctx) {}
    /// Called after the WGPU surface has been dropped and before Android releases its Surface.
    fn suspended(&mut self, _ctx: &mut Ctx) {}
    /// Synchronously services the focused Android `InputConnection`.
    fn text_input_request(
        &mut self,
        _ctx: &mut Ctx,
        _session_id: TextInputSessionId,
        _request: TextInputRequest,
    ) -> TextInputResponse {
        TextInputResponse::Unsupported
    }
    /// Return `true` to consume Android Back.
    fn back_requested(&mut self, _ctx: &mut Ctx) -> bool {
        false
    }
    fn update(&mut self, ctx: &mut Ctx);
    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView);
}

/// Winit application handler used by [`android_activity_entry!`](crate::android_activity_entry).
pub struct Runner<G> {
    config: Config,
    state: Option<State<G>>,
    error: Option<String>,
}

impl<G> Runner<G> {
    pub fn new(config: Config) -> Self {
        let _ = env_logger::Builder::from_env(
            env_logger::Env::default()
                .default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"),
        )
        .try_init();
        Self { config, state: None, error: None }
    }

    fn fail(&mut self, event_loop: &dyn ActiveEventLoop, error: String) {
        log::error!("{error}");
        self.error = Some(error);
        event_loop.exit();
    }
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

impl<G: App> State<G> {
    fn attach_surface(&mut self) -> Result<(), String> {
        let surface = self
            .instance
            .create_surface(self.ctx.window.clone())
            .map_err(|error| format!("recreate Android surface: {error}"))?;
        let caps = surface.get_capabilities(&self.adapter);
        if !caps.formats.contains(&self.surface_config.format) {
            return Err("Android surface format changed; restart the app".into());
        }
        let size = self.ctx.window.surface_size();
        self.surface_config.width = size.width.max(1);
        self.surface_config.height = size.height.max(1);
        surface.configure(&self.ctx.device, &self.surface_config);
        self.ctx.size = (self.surface_config.width, self.surface_config.height);
        self.surface = Some(surface);
        self.last = Instant::now();
        self.ctx.dt = 0.0;
        self.ctx.elapsed = (self.last - self.start).as_secs_f32();
        self.ctx.focused = self.ctx.window.has_focus();
        self.refresh_mobile_viewport();
        log::info!("Android surface ready: {}x{}", self.ctx.size.0, self.ctx.size.1);
        Ok(())
    }

    fn refresh_mobile_viewport(&mut self) {
        let insets = self.ctx.window.safe_area();
        let viewport = MobileViewport {
            visible: PhysicalRect::new(
                insets.left as f32,
                insets.top as f32,
                self.ctx
                    .size
                    .0
                    .saturating_sub(insets.left.saturating_add(insets.right)) as f32,
                self.ctx
                    .size
                    .1
                    .saturating_sub(insets.top.saturating_add(insets.bottom)) as f32,
            ),
            keyboard_visible: false,
        };
        if viewport != self.ctx.mobile_viewport {
            self.ctx.mobile_viewport = viewport;
            self.game.mobile_event(&mut self.ctx, MobileEvent::ViewportChanged(viewport));
        }
    }
}

impl<G: App + 'static> ApplicationHandler for Runner<G> {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
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
            let window: Arc<dyn Window> = Arc::from(
                event_loop
                    .create_window(winit::window::WindowAttributes::default())
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
            let size = window.surface_size();
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
                focused: window.has_focus(),
                window,
                device,
                queue,
                surface_format,
                dt: 0.0,
                elapsed: 0.0,
                frame_index: 0,
                mobile_viewport: MobileViewport {
                    visible: PhysicalRect::new(
                        0.0,
                        0.0,
                        surface_config.width as f32,
                        surface_config.height as f32,
                    ),
                    keyboard_visible: false,
                },
                text_input: None,
                size: (surface_config.width, surface_config.height),
                exit: false,
            };
            let game = G::init(&mut ctx).map_err(|error| format!("Android app init: {error}"))?;
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
                state.refresh_mobile_viewport();
                state.game.resumed(&mut state.ctx);
                if state.ctx.exit {
                    event_loop.exit();
                } else {
                    state.ctx.window.request_redraw();
                }
                self.state = Some(state);
            },
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn destroy_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
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
        event_loop: &dyn ActiveEventLoop,
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
            },
            WindowEvent::SurfaceResized(size) if size.width > 0 && size.height > 0 => {
                if let Some(surface) = &state.surface {
                    state.surface_config.width = size.width;
                    state.surface_config.height = size.height;
                    surface.configure(&state.ctx.device, &state.surface_config);
                    state.ctx.size = (size.width, size.height);
                    state.refresh_mobile_viewport();
                    state.ctx.window.request_redraw();
                }
            },
            WindowEvent::RedrawRequested => {
                if state.surface.is_none() {
                    return;
                }
                let size = state.ctx.window.surface_size();
                if size.width == 0 || size.height == 0 {
                    return;
                }
                let now = Instant::now();
                state.ctx.dt = (now - state.last).as_secs_f32();
                state.ctx.elapsed = (now - state.start).as_secs_f32();
                state.last = now;
                state.refresh_mobile_viewport();
                state.game.update(&mut state.ctx);
                if state.ctx.exit {
                    event_loop.exit();
                    return;
                }
                let surface = state.surface.as_ref().unwrap();
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame) => Some(frame),
                    wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        state.surface_config.width = size.width;
                        state.surface_config.height = size.height;
                        surface.configure(&state.ctx.device, &state.surface_config);
                        state.ctx.size = (size.width, size.height);
                        None
                    },
                    wgpu::CurrentSurfaceTexture::Lost => {
                        state.surface = None;
                        if let Err(error) = state.attach_surface() {
                            self.fail(event_loop, error);
                            return;
                        }
                        None
                    },
                    wgpu::CurrentSurfaceTexture::Occluded
                    | wgpu::CurrentSurfaceTexture::Timeout => None,
                    wgpu::CurrentSurfaceTexture::Validation => {
                        self.fail(event_loop, "Android surface validation failed".into());
                        return;
                    },
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
            },
            _ => {},
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
        event_loop: &dyn ActiveEventLoop,
        _id: Option<winit::event::DeviceId>,
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

    fn text_input_request(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        _window_id: winit::window::WindowId,
        session_id: TextInputSessionId,
        request: TextInputRequest,
    ) -> TextInputResponse {
        let Some(state) = &mut self.state else {
            return TextInputResponse::Unsupported;
        };
        state.game.text_input_request(&mut state.ctx, session_id, request)
    }

    fn back_requested(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        _window_id: winit::window::WindowId,
    ) -> BackResponse {
        let Some(state) = &mut self.state else {
            return BackResponse::SystemDefault;
        };
        if state.game.back_requested(&mut state.ctx) {
            BackResponse::Handled
        } else {
            BackResponse::SystemDefault
        }
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

/// Export the JNI bridge consumed by `org.rustwindowing.winit.WinitActivity`.
#[macro_export]
macro_rules! android_activity_entry {
    ($app:ty, $config:expr) => {
        $crate::winit::platform::android::java_activity_entry!(
            $crate::android::Runner<$app>,
            $crate::android::Runner::<$app>::new($config)
        );
    };
}

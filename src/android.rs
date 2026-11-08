//! NativeActivity/Vulkan runner, separate from the desktop/web runner.
//!
//! Call [`run`] from `android_main`. The game and GPU survive surface suspension;
//! the drawing surface is released before the suspend callback returns. Process
//! death still ends the game: durable saves and game-time policy belong to the app.

use crate::{
    MobileEvent, MobileViewport, NativeTextSnapshot, PhysicalRect, RenderContext, TextInputState,
    TextMirror, apply_native_snapshot, byte_to_utf16, wgpu,
};
use jni::{
    EnvUnowned, JavaVM,
    errors::ThrowRuntimeExAndDefault,
    objects::{JClass, JObject, JString, JValue},
    sys::{jboolean, jint, jlong},
};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::android::EventLoopBuilderExtAndroid;
use winit::window::Window;

pub use winit::platform::android::activity::AndroidApp;

enum ActivityMethod {
    ClearTextInput,
    ShowKeyboard,
    HideKeyboard,
}

enum NativeEvent {
    Mobile(MobileEvent),
    Viewport {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        keyboard_visible: bool,
    },
}

struct NativeRuntime {
    window: Weak<Window>,
    events: Vec<NativeEvent>,
    mirror: Option<TextMirror>,
}

static NATIVE_RUNTIME: Mutex<Option<NativeRuntime>> = Mutex::new(None);
static PENDING_VIEWPORT: Mutex<Option<(i32, i32, i32, i32, bool)>> = Mutex::new(None);

fn wake_native(runtime: &NativeRuntime) {
    if let Some(window) = runtime.window.upgrade() {
        window.request_redraw();
    }
}

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
    mobile_viewport: MobileViewport,
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

    fn java(&self, method: ActivityMethod) -> Result<(), String> {
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr().cast()) };
        vm.attach_current_thread(|env| {
            let activity = unsafe { JObject::from_raw(env, self.app.activity_as_ptr().cast()) };
            match method {
                ActivityMethod::ClearTextInput => {
                    env.call_method(
                        &activity,
                        jni::jni_str!("chadClearTextInput"),
                        jni::jni_sig!("()V"),
                        &[],
                    )?;
                }
                ActivityMethod::ShowKeyboard => {
                    env.call_method(
                        &activity,
                        jni::jni_str!("chadShowKeyboard"),
                        jni::jni_sig!("()V"),
                        &[],
                    )?;
                }
                ActivityMethod::HideKeyboard => {
                    env.call_method(
                        &activity,
                        jni::jni_str!("chadHideKeyboard"),
                        jni::jni_sig!("()V"),
                        &[],
                    )?;
                }
            }
            Ok::<_, jni::errors::Error>(())
        })
        .map_err(|error| format!("Android activity method: {error}"))
    }

    pub fn set_text_input(&mut self, state: TextInputState<'_>) -> Result<(), String> {
        if state.id == 0 {
            return Err("text input ID must be nonzero".into());
        }
        let end = state
            .document_start
            .checked_add(state.text.len())
            .ok_or("text input document range overflow")?;
        if state.selection.anchor < state.document_start
            || state.selection.anchor > end
            || state.selection.focus < state.document_start
            || state.selection.focus > end
        {
            return Err("text input selection is outside its surrounding text".into());
        }
        let local_anchor = state.selection.anchor - state.document_start;
        let local_focus = state.selection.focus - state.document_start;
        let anchor = byte_to_utf16(state.text, local_anchor)
            .ok_or("text input anchor is not a UTF-8 boundary")?;
        let focus = byte_to_utf16(state.text, local_focus)
            .ok_or("text input focus is not a UTF-8 boundary")?;
        let id = i64::try_from(state.id).map_err(|_| "text input ID exceeds Android range")?;
        let generation = i64::try_from(state.generation)
            .map_err(|_| "text input generation exceeds Android range")?;
        let document_start = i64::try_from(state.document_start)
            .map_err(|_| "text input document offset exceeds Android range")?;
        let mut reset = false;
        if let Ok(mut guard) = NATIVE_RUNTIME.lock() {
            let runtime = guard.as_mut().ok_or("Android text input is not initialized")?;
            match runtime.mirror.as_mut() {
                Some(mirror)
                    if mirror.id == state.id
                        && mirror.generation == state.generation
                        && mirror.document_start == state.document_start =>
                {
                    if mirror.text != state.text {
                        mirror.text.clear();
                        mirror.text.push_str(state.text);
                        mirror.composition = None;
                        reset = true;
                    }
                    mirror.selection = state.selection;
                }
                _ => {
                    runtime.mirror = Some(TextMirror {
                        id: state.id,
                        generation: state.generation,
                        document_start: state.document_start,
                        text: state.text.to_string(),
                        selection: state.selection,
                        composition: None,
                    });
                    reset = true;
                }
            }
        } else {
            return Err("Android text input state is unavailable".into());
        }

        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr().cast()) };
        vm.attach_current_thread(|env| {
            let text = env.new_string(state.text)?;
            let arguments = [
                JValue::Long(id),
                JValue::Long(generation),
                JValue::Long(document_start),
                JValue::Object(&text),
                JValue::Int(anchor),
                JValue::Int(focus),
                JValue::Float(state.field_rect.x),
                JValue::Float(state.field_rect.y),
                JValue::Float(state.field_rect.width),
                JValue::Float(state.field_rect.height),
                JValue::Float(state.caret_rect.x),
                JValue::Float(state.caret_rect.y),
                JValue::Float(state.caret_rect.width),
                JValue::Float(state.caret_rect.height),
                JValue::Bool(state.options.multiline),
                JValue::Bool(state.options.secure),
                JValue::Bool(reset),
            ];
            let activity = unsafe { JObject::from_raw(env, self.app.activity_as_ptr().cast()) };
            env.call_method(
                &activity,
                jni::jni_str!("chadSetTextInput"),
                jni::jni_sig!("(JJJLjava/lang/String;IIFFFFFFFFZZZ)V"),
                &arguments,
            )?;
            Ok::<_, jni::errors::Error>(())
        })
        .map_err(|error| format!("Android text input: {error}"))

    }

    pub fn clear_text_input(&mut self) -> Result<(), String> {
        if let Ok(mut guard) = NATIVE_RUNTIME.lock()
            && let Some(runtime) = guard.as_mut()
        {
            runtime.mirror = None;
        }
        self.java(ActivityMethod::ClearTextInput)
    }

    pub fn show_keyboard(&self) -> Result<(), String> {
        self.java(ActivityMethod::ShowKeyboard)
    }

    pub fn hide_keyboard(&self) -> Result<(), String> {
        self.java(ActivityMethod::HideKeyboard)
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
    fn mobile_event(&mut self, _ctx: &mut Ctx, _event: MobileEvent) {}
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
    let result = event_loop
        .run_app(&mut runner)
        .map_err(|error| format!("Android event loop: {error}"));
    if let Ok(mut runtime) = NATIVE_RUNTIME.lock() {
        *runtime = None;
    }
    result?;
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

impl<G: App> State<G> {
    fn drain_native_events(&mut self) {
        let events = NATIVE_RUNTIME
            .lock()
            .ok()
            .and_then(|mut guard| {
                guard
                    .as_mut()
                    .map(|runtime| std::mem::take(&mut runtime.events))
            })
            .unwrap_or_default();
        for event in events {
            let event = match event {
                NativeEvent::Mobile(event) => event,
                NativeEvent::Viewport {
                    left,
                    top,
                    right,
                    bottom,
                    keyboard_visible,
                } => {
                    let (width, height) = self.ctx.size;
                    let content = self.ctx.app.content_rect();
                    let mut visible = PhysicalRect::new(0.0, 0.0, width as f32, height as f32);
                    for (left, top, right, bottom) in [
                        (content.left, content.top, content.right, content.bottom),
                        (left, top, right, bottom),
                    ] {
                        if right > left && bottom > top {
                            let x = visible.x.max(left.max(0) as f32);
                            let y = visible.y.max(top.max(0) as f32);
                            let right = (visible.x + visible.width).min(right.max(0) as f32);
                            let bottom = (visible.y + visible.height).min(bottom.max(0) as f32);
                            visible = PhysicalRect::new(
                                x,
                                y,
                                (right - x).max(0.0),
                                (bottom - y).max(0.0),
                            );
                        }
                    }
                    let viewport = MobileViewport {
                        visible,
                        keyboard_visible,
                    };
                    if viewport == self.ctx.mobile_viewport {
                        continue;
                    }
                    self.ctx.mobile_viewport = viewport;
                    MobileEvent::ViewportChanged(viewport)
                }
            };
            self.game.mobile_event(&mut self.ctx, event);
        }
    }

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
                mobile_viewport: MobileViewport {
                    visible: PhysicalRect::new(
                        0.0,
                        0.0,
                        surface_config.width as f32,
                        surface_config.height as f32,
                    ),
                    keyboard_visible: false,
                },
                size: (surface_config.width, surface_config.height),
                exit: false,
            };
            if let Ok(mut runtime) = NATIVE_RUNTIME.lock() {
                *runtime = Some(NativeRuntime {
                    window: Arc::downgrade(&ctx.window),
                    events: Vec::new(),
                    mirror: None,
                });
                if let Ok(mut viewport) = PENDING_VIEWPORT.lock()
                    && let Some((left, top, right, bottom, keyboard_visible)) = viewport.take()
                    && let Some(runtime) = runtime.as_mut()
                {
                    runtime.events.push(NativeEvent::Viewport {
                        left,
                        top,
                        right,
                        bottom,
                        keyboard_visible,
                    });
                }
            }
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
            state.drain_native_events();
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
                if state.surface.is_none() {
                    return;
                }
                let size = state.ctx.window.inner_size();
                if size.width == 0 || size.height == 0 {
                    return;
                }
                let now = Instant::now();
                state.ctx.dt = (now - state.last).as_secs_f32();
                state.ctx.elapsed = (now - state.start).as_secs_f32();
                state.last = now;
                state.drain_native_events();
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

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_xpjb_chad_ChadActivity_nativeTextInput(
    mut env: EnvUnowned,
    _: JClass,
    id: jlong,
    generation: jlong,
    document_start: jlong,
    text: JString,
    selection_start: jint,
    selection_end: jint,
    composition_start: jint,
    composition_end: jint,
) {
    env.with_env(|env| {
        if id <= 0 || generation < 0 || document_start < 0 {
            return Ok(());
        }
        let text = text.try_to_string(env)?;
        if let Ok(mut guard) = NATIVE_RUNTIME.lock()
            && let Some(runtime) = guard.as_mut()
        {
            let event = runtime.mirror.as_mut().and_then(|mirror| {
                apply_native_snapshot(
                    mirror,
                    NativeTextSnapshot {
                        id: id as u64,
                        generation: generation as u64,
                        document_start: document_start as usize,
                        text,
                        selection_start_utf16: selection_start,
                        selection_end_utf16: selection_end,
                        composition_start_utf16: composition_start,
                        composition_end_utf16: composition_end,
                    },
                )
            });
            if let Some(event) = event {
                runtime
                    .events
                    .push(NativeEvent::Mobile(MobileEvent::TextInput(event)));
                wake_native(runtime);
            }
        }
        Ok::<_, jni::errors::Error>(())
    })
    .resolve::<ThrowRuntimeExAndDefault>();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_xpjb_chad_ChadActivity_nativeViewport(
    mut env: EnvUnowned,
    _: JClass,
    left: jint,
    top: jint,
    right: jint,
    bottom: jint,
    keyboard_visible: jboolean,
) {
    env.with_env(|_| {
        let viewport = (left, top, right, bottom, keyboard_visible);
        if let Ok(mut pending) = PENDING_VIEWPORT.lock() {
            *pending = Some(viewport);
        }
        if let Ok(mut guard) = NATIVE_RUNTIME.lock()
            && let Some(runtime) = guard.as_mut()
        {
            if matches!(runtime.events.last(), Some(NativeEvent::Viewport { .. })) {
                runtime.events.pop();
            }
            runtime.events.push(NativeEvent::Viewport {
                left,
                top,
                right,
                bottom,
                keyboard_visible,
            });
            wake_native(runtime);
        }
        Ok::<_, jni::errors::Error>(())
    })
    .resolve::<ThrowRuntimeExAndDefault>();
}

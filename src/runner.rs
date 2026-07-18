use crate::*;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::Window;

#[cfg(target_arch = "wasm32")]
use std::{cell::Cell, rc::Rc};

/// Run the app. Owns the event loop, window, GPU init, surface lifecycle, and
/// frame loop; returns when the game calls `ctx.exit()` (or init fails).
///
/// On wasm this hands control to the browser and returns immediately; GPU init
/// runs as a spawned future (the browser forbids blocking the main thread) and
/// the game's `init` fires when it completes.
pub fn run<G: ChadApp + 'static>(config: Config) -> Result<(), String> {
    init_logging_and_panic_hook(&config);
    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;
    let proxy = event_loop.create_proxy();
    let runner = Runner::<G> {
        config,
        proxy,
        state: None,
        init_error: None,
        #[cfg(target_arch = "wasm32")]
        pending_ctx: Rc::new(Cell::new(None)),
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut runner = runner;
        event_loop
            .run_app(&mut runner)
            .map_err(|e| format!("event loop: {e}"))?;
        match runner.init_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(runner);
        Ok(())
    }
}

fn init_logging_and_panic_hook(config: &Config) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if config.init_logging {
            // Quiet wgpu internals by default (the Vulkan loader alone is
            // hundreds of lines at info); RUST_LOG overrides.
            let _ = env_logger::Builder::from_env(
                env_logger::Env::default()
                    .default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"),
            )
            .try_init();
        }
        if let Some(path) = config.crash_log.clone() {
            let prev = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let backtrace = std::backtrace::Backtrace::force_capture();
                let _ = std::fs::write(&path, format!("{info}\n\nbacktrace:\n{backtrace}\n"));
                prev(info);
            }));
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        if config.init_logging {
            let _ = console_log::init_with_level(log::Level::Info);
        }
        console_error_panic_hook::set_once();
    }
}

/// The owned slice of Config that GPU init needs — owned so the wasm path can
/// move it into a spawned future.
struct GpuOptions {
    power_preference: wgpu::PowerPreference,
    features: wgpu::Features,
    limits: wgpu::Limits,
    present_mode: wgpu::PresentMode,
}

fn gpu_options(cfg: &Config) -> GpuOptions {
    GpuOptions {
        power_preference: cfg.power_preference,
        features: cfg.device_features,
        limits: cfg.device_limits.clone(),
        present_mode: cfg.initial_present_mode(),
    }
}

fn create_window(cfg: &Config, event_loop: &ActiveEventLoop) -> Result<Arc<Window>, String> {
    #[allow(unused_mut)]
    let mut attrs = Window::default_attributes()
        .with_title(&cfg.title)
        .with_inner_size(winit::dpi::LogicalSize::new(cfg.size.0, cfg.size.1))
        .with_resizable(cfg.resizable);
    // Native: created invisible, shown after the first frame presents (no
    // white flash). Web: no white-flash problem and set_visible is unreliable
    // on the canvas, so it starts visible.
    #[cfg(not(target_arch = "wasm32"))]
    {
        attrs = attrs.with_visible(false);
        if let Some(icon) = &cfg.icon {
            match winit::window::Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height) {
                Ok(icon) => attrs = attrs.with_window_icon(Some(icon)),
                Err(e) => log::warn!("window icon rejected: {e}"),
            }
        }
    }
    let window = Arc::new(
        event_loop
            .create_window(attrs)
            .map_err(|e| format!("create_window: {e}"))?,
    );
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::WindowExtWebSys;
        let canvas = window.canvas().ok_or("window has no canvas")?;
        // Fill the parent; winit's ResizeObserver picks up the CSS size and
        // emits Resized (config.size is only the initial backing store on web).
        let _ = canvas.style().set_property("width", "100%");
        let _ = canvas.style().set_property("height", "100%");
        web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.body())
            .ok_or("no document body")?
            .append_child(&canvas)
            .map_err(|_| "failed to append canvas to body")?;
    }
    Ok(window)
}

/// Async GPU init: adapter, device, configured surface, assembled Ctx.
/// Blocked on via pollster on native; spawned as a browser future on wasm.
async fn build_ctx(
    opts: GpuOptions,
    window: Arc<Window>,
    proxy: EventLoopProxy<()>,
) -> Result<Ctx, String> {
    // _from_env: WGPU_BACKEND etc. can override without a rebuild. The display
    // handle is needed for presentability on GLES/Wayland; harmless elsewhere.
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
        Box::new(window.clone()),
    ));
    let surface = instance
        .create_surface(window.clone())
        .map_err(|e| format!("create_surface: {e}"))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: opts.power_preference,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|e| format!("request_adapter: {e}"))?;
    let info = adapter.get_info();
    log::info!("GPU: {} ({:?}, {:?})", info.name, info.device_type, info.backend);

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("chad-device"),
            required_features: opts.features,
            required_limits: opts.limits,
            memory_hints: wgpu::MemoryHints::default(),
            ..Default::default()
        })
        .await
        .map_err(|e| format!("request_device: {e}"))?;

    let caps = surface.get_capabilities(&adapter);
    let base_format = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(caps.formats[0]);
    // WebGPU surfaces only list non-sRGB formats; render through an sRGB view
    // of the same texture so colors match native. No-op where base is sRGB.
    let surface_format = base_format.add_srgb_suffix();
    let view_formats = if surface_format != base_format {
        vec![surface_format]
    } else {
        vec![]
    };
    let size = window.inner_size();
    let surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: base_format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: opts.present_mode,
        alpha_mode: caps.alpha_modes[0],
        view_formats,
        desired_maximum_frame_latency: 2,
        // Auto = wgpu's historical sRGB behavior; HDR output is opt-in and
        // out of scope for the runner (games can reconfigure if they care).
        color_space: wgpu::SurfaceColorSpace::Auto,
    };
    surface.configure(&device, &surface_config);
    log::info!(
        "surface ready ({}x{}, {:?} via {:?}, {:?})",
        surface_config.width,
        surface_config.height,
        surface_format,
        base_format,
        surface_config.present_mode
    );

    Ok(Ctx {
        window,
        device,
        queue,
        surface_format,
        dt: 0.0,
        elapsed: 0.0,
        frame_index: 0,
        surface,
        surface_config,
        proxy,
        exit: false,
        surface_dirty: false,
        alpha: 1.0,
    })
}

struct Runner<G: ChadApp> {
    config: Config,
    proxy: EventLoopProxy<()>,
    state: Option<State<G>>,
    init_error: Option<String>,
    /// wasm: the spawned init future parks the finished Ctx here, then pings
    /// the loop via the proxy; `user_event` completes the handoff. Cell (not
    /// RefCell): the value is only ever moved in and taken out, never borrowed.
    #[cfg(target_arch = "wasm32")]
    pending_ctx: Rc<Cell<Option<Result<Ctx, String>>>>,
}

struct State<G> {
    game: G,
    ctx: Ctx,
    start: Instant,
    last: Instant,
    accumulator: f32,
    shown: bool,
    minimized: bool,
}

impl<G: ChadApp> Runner<G> {
    fn finish_init(&mut self, mut ctx: Ctx, event_loop: &ActiveEventLoop) {
        // Re-sync size at handoff: on web the canvas gets its real (CSS-driven)
        // size asynchronously, and any Resized emitted during async GPU init
        // was dropped (no state yet). Resized events from here on flow normally.
        let size = ctx.window.inner_size();
        if size.width > 0
            && size.height > 0
            && (size.width, size.height) != (ctx.surface_config.width, ctx.surface_config.height)
        {
            ctx.surface_config.width = size.width;
            ctx.surface_config.height = size.height;
            ctx.surface_dirty = true;
        }
        match G::init(&mut ctx) {
            Ok(game) => {
                let now = Instant::now();
                self.state = Some(State {
                    game,
                    ctx,
                    start: now,
                    last: now,
                    accumulator: 0.0,
                    shown: cfg!(target_arch = "wasm32"),
                    minimized: false,
                });
                // Drive the first frame directly: hidden windows don't
                // reliably receive RedrawRequested (Windows sends no paint
                // messages to invisible windows), and the window is only shown
                // after the first present — waiting would deadlock invisibly.
                let state = self.state.as_mut().unwrap();
                Self::tick(state, &self.config);
                if !state.shown {
                    // First present failed (e.g. stale surface): show anyway
                    // rather than never; the next tick repairs the surface.
                    state.ctx.window.set_visible(true);
                    state.shown = true;
                    state.ctx.window.request_redraw();
                }
                if state.ctx.exit {
                    event_loop.exit();
                }
            }
            Err(e) => self.fail(format!("game init: {e}"), event_loop),
        }
    }

    fn fail(&mut self, e: String, event_loop: &ActiveEventLoop) {
        log::error!("init failed: {e}");
        self.init_error = Some(e);
        event_loop.exit();
    }

    /// Advance game time: measure dt and run `update` per the timestep policy.
    fn advance(state: &mut State<G>, cfg: &Config) {
        let now = Instant::now();
        let real_dt = (now - state.last).as_secs_f32().min(cfg.max_dt);
        state.last = now;
        state.ctx.elapsed = (now - state.start).as_secs_f32();
        match cfg.timestep {
            Timestep::Variable => {
                state.ctx.dt = real_dt;
                state.ctx.alpha = 1.0;
                state.game.update(&mut state.ctx);
            }
            Timestep::Fixed { hz, max_updates_per_frame } => {
                let step = 1.0 / hz.max(1) as f32;
                state.accumulator =
                    (state.accumulator + real_dt).min(step * max_updates_per_frame.max(1) as f32);
                state.ctx.dt = step;
                while state.accumulator >= step && !state.ctx.exit {
                    state.game.update(&mut state.ctx);
                    state.accumulator -= step;
                }
                state.ctx.alpha = state.accumulator / step;
            }
        }
    }

    /// One full tick: update(s), then render, then pacing / next-frame scheduling.
    fn tick(state: &mut State<G>, cfg: &Config) {
        #[cfg(not(target_arch = "wasm32"))]
        let tick_start = Instant::now();
        Self::advance(state, cfg);
        if state.ctx.exit {
            return;
        }

        if !state.minimized {
            if state.ctx.surface_dirty {
                state.ctx.surface.configure(&state.ctx.device, &state.ctx.surface_config);
                state.ctx.surface_dirty = false;
            }
            let acquired = match state.ctx.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) => Some(frame),
                // Usable but stale-ish: render this frame, reconfigure next tick.
                wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    state.ctx.surface_dirty = true;
                    Some(frame)
                }
                // Stale surface (resize race, display change): reconfigure on
                // the next tick and skip this frame.
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    state.ctx.surface_dirty = true;
                    None
                }
                // Hidden/covered or slow compositor: skip quietly, try again.
                wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout => None,
                wgpu::CurrentSurfaceTexture::Validation => {
                    log::warn!("dropped frame: surface validation error");
                    None
                }
            };
            if let Some(frame) = acquired {
                // View in surface_format (the sRGB variant where the raw
                // surface is non-sRGB, e.g. WebGPU).
                let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(state.ctx.surface_format),
                    ..Default::default()
                });
                state.game.frame(&mut state.ctx, &view);
                state.ctx.queue.present(frame);
                state.ctx.frame_index += 1;
                if !state.shown {
                    state.ctx.window.set_visible(true);
                    state.shown = true;
                }
            }
        }

        // Sleep-based limiter; browser (rAF) paces the web build instead.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(cap) = cfg.max_fps {
            let target = Duration::from_secs_f32(1.0 / cap.max(1) as f32);
            let spent = tick_start.elapsed();
            if spent < target {
                std::thread::sleep(target - spent);
            }
        }
        if matches!(cfg.redraw, RedrawMode::Continuous) && !state.minimized {
            state.ctx.window.request_redraw();
        }
    }
}

impl<G: ChadApp + 'static> ApplicationHandler for Runner<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return; // desktop/web assumption: resumed fires once
        }
        let window = match create_window(&self.config, event_loop) {
            Ok(w) => w,
            Err(e) => return self.fail(e, event_loop),
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            match pollster::block_on(build_ctx(
                gpu_options(&self.config),
                window,
                self.proxy.clone(),
            )) {
                Ok(ctx) => self.finish_init(ctx, event_loop),
                Err(e) => self.fail(e, event_loop),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            // Can't block the browser's main thread: init runs as a future,
            // parks the Ctx in pending_ctx, and wakes the loop. Events that
            // arrive in the gap are dropped (state is still None).
            let slot = self.pending_ctx.clone();
            let opts = gpu_options(&self.config);
            let proxy = self.proxy.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = build_ctx(opts, window, proxy.clone()).await;
                slot.set(Some(result));
                let _ = proxy.send_event(());
            });
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else { return };
        match &event {
            WindowEvent::Resized(size) => {
                state.minimized = size.width == 0 || size.height == 0;
                if !state.minimized {
                    state.ctx.surface_config.width = size.width;
                    state.ctx.surface_config.height = size.height;
                    state.ctx.surface_dirty = true;
                    state.ctx.window.request_redraw();
                }
                // falls through: forwarded to the game after bookkeeping
            }
            WindowEvent::RedrawRequested => {
                Self::tick(state, &self.config);
                if state.ctx.exit {
                    event_loop.exit();
                }
                return; // frame() is the redraw; not forwarded as an event
            }
            _ => {}
        }
        state.game.event(&mut state.ctx, &event);
        if state.ctx.exit {
            event_loop.exit();
        }
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        let Some(state) = &mut self.state else { return };
        state.game.device_event(&mut state.ctx, &event);
        if state.ctx.exit {
            event_loop.exit();
        }
    }

    #[allow(unused_variables)]
    fn user_event(&mut self, event_loop: &ActiveEventLoop, _event: ()) {
        // wasm: a ping may mean async init just finished — complete the handoff.
        #[cfg(target_arch = "wasm32")]
        if self.state.is_none() {
            if let Some(result) = self.pending_ctx.take() {
                match result {
                    Ok(ctx) => self.finish_init(ctx, event_loop),
                    Err(e) => self.fail(e, event_loop),
                }
            }
            return;
        }
        // Waker ping from another thread: schedule a tick so the game can
        // drain whatever channels it owns.
        if let Some(state) = &self.state {
            state.ctx.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // While minimized, RedrawRequested stops arriving and the redraw chain
        // stalls; keep updates ticking on a timer if configured. (Native only:
        // browsers don't minimize the canvas, and std Instant doesn't exist
        // on wasm.)
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(state) = &mut self.state else { return };
            if state.minimized && self.config.update_while_minimized {
                Self::advance(state, &self.config);
                if state.ctx.exit {
                    event_loop.exit();
                    return;
                }
                let step = match self.config.timestep {
                    Timestep::Fixed { hz, .. } => 1.0 / hz.max(1) as f32,
                    Timestep::Variable => 1.0 / 60.0,
                };
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    std::time::Instant::now() + Duration::from_secs_f32(step),
                ));
                return;
            }
        }
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

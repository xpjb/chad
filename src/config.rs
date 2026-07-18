use std::path::PathBuf;

#[derive(Clone, Copy)]
pub enum Timestep {
    /// `update` once per frame, `ctx.dt` = real frame time (clamped to `max_dt`).
    Variable,
    /// Fix-your-timestep accumulator: `update` 0..`max_updates_per_frame` times
    /// per frame at a constant `1/hz` step; `ctx.alpha()` for render interpolation.
    /// `max_updates_per_frame` clamps the accumulator (death-spiral guard).
    Fixed { hz: u32, max_updates_per_frame: u32 },
}

#[derive(Clone, Copy)]
pub enum RedrawMode {
    /// Redraw continuously (games). vsync/`max_fps` set the pace.
    Continuous,
    /// Redraw only on events or `ctx.request_redraw()` (tools).
    OnDemand,
}

/// Window icon as raw RGBA8 pixels (no image-decoding dependency here; decode
/// however you like, or embed raw bytes).
pub struct AppIcon {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct Config {
    pub title: String,
    /// Initial inner size, logical pixels.
    pub size: (u32, u32),
    pub resizable: bool,
    pub icon: Option<AppIcon>,

    /// Maps to `PresentMode::AutoVsync` / `AutoNoVsync`. Toggle at runtime via
    /// `ctx.set_vsync`. Ignored if `present_mode` is set.
    pub vsync: bool,
    /// Exact present mode override for those who know what they want.
    pub present_mode: Option<wgpu::PresentMode>,
    /// Sleep-based frame limiter. Dumb by design; mostly for vsync-off,
    /// menus, and battery.
    pub max_fps: Option<u32>,
    pub timestep: Timestep,
    pub redraw: RedrawMode,
    /// Largest dt the game will ever observe (debugger pauses, lid closes,
    /// first frame). Seconds.
    pub max_dt: f32,

    pub power_preference: wgpu::PowerPreference,
    /// Configurable so a game needing e.g. push constants doesn't fork the runner.
    pub device_features: wgpu::Features,
    pub device_limits: wgpu::Limits,

    /// Where the panic hook writes panic + backtrace. `None` = no hook.
    pub crash_log: Option<PathBuf>,
    /// Install an env_logger backend (default filter "info", `RUST_LOG` overrides).
    pub init_logging: bool,
    /// Keep `update` ticking on a timer while minimized (multiplayer clients
    /// want true, single-player pause wants false).
    pub update_while_minimized: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            title: "chad".into(),
            size: (1280, 720),
            resizable: true,
            icon: None,
            vsync: true,
            present_mode: None,
            max_fps: None,
            timestep: Timestep::Variable,
            redraw: RedrawMode::Continuous,
            max_dt: 0.1,
            power_preference: wgpu::PowerPreference::HighPerformance,
            device_features: wgpu::Features::empty(),
            device_limits: wgpu::Limits::default(),
            crash_log: Some("crash.log".into()),
            init_logging: true,
            update_while_minimized: false,
        }
    }
}

impl Config {
    pub(crate) fn initial_present_mode(&self) -> wgpu::PresentMode {
        self.present_mode.unwrap_or(if self.vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        })
    }
}

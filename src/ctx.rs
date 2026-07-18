use std::sync::Arc;
use winit::event_loop::EventLoopProxy;
use winit::window::{Fullscreen, Window};

/// Cheap, clonable, `Send` handle that wakes the event loop from another
/// thread. Payloadless by design: data travels through channels the game owns;
/// this only guarantees the loop comes around soon so `update` can drain them.
/// Continuous-mode games never need it.
#[derive(Clone)]
pub struct Waker(pub(crate) EventLoopProxy<()>);

impl Waker {
    pub fn wake(&self) {
        let _ = self.0.send_event(());
    }
}

/// The one library-defined type the game touches. Everything in it is either
/// raw (window, device, queue) or state only the runner can know (timing,
/// surface config, loop control). No wrappers over winit or wgpu.
pub struct Ctx {
    pub window: Arc<Window>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_format: wgpu::TextureFormat,

    /// Seconds per `update` tick: the fixed step under `Timestep::Fixed`,
    /// real clamped frame time under `Variable`.
    pub dt: f32,
    /// Seconds since init.
    pub elapsed: f32,
    /// Frames presented so far.
    pub frame_index: u64,

    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) surface_config: wgpu::SurfaceConfiguration,
    pub(crate) proxy: EventLoopProxy<()>,
    pub(crate) exit: bool,
    pub(crate) surface_dirty: bool,
    pub(crate) alpha: f32,
}

impl Ctx {
    /// Current surface size in physical pixels (never zero).
    pub fn size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }

    /// Interpolation factor in [0, 1) between the last two fixed updates.
    /// Always 1.0 under `Timestep::Variable`.
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// The only way the app exits. The runner checks this after every callback.
    pub fn exit(&mut self) {
        self.exit = true;
    }

    /// Schedule a frame (meaningful under `RedrawMode::OnDemand`).
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn set_vsync(&mut self, on: bool) {
        self.set_present_mode(if on {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        });
    }

    pub fn set_present_mode(&mut self, mode: wgpu::PresentMode) {
        if self.surface_config.present_mode != mode {
            self.surface_config.present_mode = mode;
            self.surface_dirty = true;
        }
    }

    pub fn toggle_fullscreen(&self) {
        let next = match self.window.fullscreen() {
            Some(_) => None,
            None => Some(Fullscreen::Borderless(None)),
        };
        self.window.set_fullscreen(next);
    }

    pub fn waker(&self) -> Waker {
        Waker(self.proxy.clone())
    }
}

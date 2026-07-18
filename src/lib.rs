//! Thin platform layer over winit + wgpu. The runner owns the event loop,
//! window, GPU init, surface lifecycle, and frame timing; the game implements
//! [`ChadApp`] and sees only [`Ctx`] plus raw winit events and wgpu types.
//!
//! Scope rule: something belongs in this crate only if implementing it
//! correctly requires touching the event loop, window, or surface lifecycle —
//! or if it is identical in every game. Everything else is userland.

pub use wgpu;
pub use winit;

mod config;
pub use config::*;
mod ctx;
pub use ctx::*;
mod runner;
pub use runner::*;

use winit::event::{DeviceEvent, WindowEvent};

pub trait ChadApp: Sized {
    /// Called once, on the main thread, after the window and GPU are ready.
    fn init(ctx: &mut Ctx) -> Result<Self, String>;

    /// Raw window events, unfiltered. The runner has already handled surface
    /// bookkeeping for `Resized` before forwarding it. `CloseRequested` is
    /// forwarded, not acted on — call `ctx.exit()` yourself (or intercept it
    /// for a save prompt). `RedrawRequested` is not forwarded; [`Self::frame`]
    /// is the redraw.
    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent);

    /// Raw device events (e.g. `DeviceEvent::MouseMotion` for unaccelerated
    /// mouse deltas — what a mouselook camera wants, unlike `CursorMoved`).
    fn device_event(&mut self, _ctx: &mut Ctx, _event: &DeviceEvent) {}

    /// Simulation tick. Under `Timestep::Variable`, called once per frame with
    /// `ctx.dt` = real (clamped) frame time. Under `Timestep::Fixed`, called
    /// 0..max_updates times per frame with `ctx.dt` = the constant step.
    fn update(&mut self, ctx: &mut Ctx);

    /// Render the frame into `view`. Acquire/present are the runner's job;
    /// record and submit whatever passes you like. Skipped while minimized.
    /// Under `Timestep::Fixed`, `ctx.alpha()` is the interpolation factor.
    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView);
}

# chad

A thin platform layer for games on **winit + wgpu**. Not an engine.

> **Pins winit `0.30` + wgpu `30`**, re-exported as `chad::winit` / `chad::wgpu` — write against those, don't add your own. See [Versioning](#versioning).

chad owns the part of every winit+wgpu project that is ugly, subtle, and
identical across projects — the event loop, window creation, GPU init
(including the async dance the browser forces), surface lifecycle, and frame
timing — and hands you raw `winit` events and raw `wgpu` types. No wrappers,
no ECS, no scenes, no assets. You implement one trait; everything else is
your code.

```rust
use chad::{wgpu, ChadApp, Config, Ctx};
use chad::winit::event::WindowEvent;

struct Game;

impl ChadApp for Game {
    fn init(ctx: &mut Ctx) -> Result<Self, String> {
        // ctx.device / ctx.queue / ctx.surface_format: build your pipelines here
        Ok(Game)
    }
    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent) {
        if let WindowEvent::CloseRequested = event {
            ctx.exit(); // chad never exits on its own
        }
    }
    fn update(&mut self, ctx: &mut Ctx) {
        // simulation tick; ctx.dt per the configured timestep
    }
    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView) {
        // record and submit whatever passes you like into `view`
    }
}

fn main() {
    chad::run::<Game>(Config::default()).unwrap();
}
```

## Examples

`cargo run --example <name>` — each one is a single self-contained file:

| example | shows |
|---|---|
| `triangle` | the smallest complete app |
| `sprite_batch` | the next step after `triangle`: generated texture, bind group, resize-aware screen uniform, dynamic instance buffer, and alpha-blended sprites in one draw |
| `halfpipe` | `Timestep::Fixed` at 20 Hz, interpolation via `ctx.alpha()` (SPACE toggles it — see what it buys you), symplectic integration, `update_while_minimized` |
| `flycam` | first-person crawl inside an endless repeated Mandelbox: capped internal resolution + upscale blit, CPU-mirrored SDF collision (no clipping), click-to-capture mouselook via `device_event`, 1-pole smoothed movement + Shift sprint, headlamp + orbit-trap glow, vsync toggle (V), fullscreen (F), `max_fps` |
| `clock` | `RedrawMode::OnDemand` + `Waker`: renders once per second at ~zero idle CPU, procedural window icon |

## Headless rendering

On native targets, `Headless` creates a wgpu device without a window or
surface. Use it for screenshot harnesses and render smoke checks:

```rust
let gpu = chad::Headless::new()?;
let target = gpu.target((1280, 720));

// Build pipelines with gpu.device, upload through gpu.queue, and render into
// target.view with a normal wgpu render pass.

let rgba = gpu.read_rgba8(&target)?;
```

The readback is tightly packed, top-to-bottom RGBA8. Image encoding remains
consumer-owned.

## What you get

- Window + full wgpu init, blocking on native, async on wasm (browsers forbid
  blocking the main thread; chad runs init as a future and calls your `init`
  when the GPU is ready)
- Surface lifecycle: resize, surface-lost recovery, minimize handling, sRGB
  view formats where the surface is non-sRGB (WebGPU), show-after-first-frame
  (no white flash)
- Native headless wgpu initialization, RGBA8 render targets, and readback for screenshot harnesses
- Frame timing: variable dt or a fix-your-timestep accumulator
  (`Timestep::Fixed`) with interpolation alpha and a death-spiral clamp; dt is
  clamped so debugger pauses don't launch your player through a wall
- Vsync as a `Config` bool with a runtime toggle (`ctx.set_vsync`), or an
  exact `wgpu::PresentMode` if you know what you want
- `DeviceEvent` forwarding (raw mouse deltas — what a mouselook camera needs)
- Continuous or on-demand redraw, optional sleep-based frame cap
- A payloadless `Waker` to nudge the loop from other threads (drain your own
  channels in `update`)
- Crash-log panic hook, logging init, window icon, fullscreen toggle
- `Config` exposes `wgpu` device features and limits, so needing push
  constants doesn't mean forking the runner

## What you don't get (on purpose)

The scope rule: something belongs in chad only if implementing it correctly
requires touching the event loop, window, or surface lifecycle — or is
literally identical in every game. Everything else is your code: input
mapping, audio, assets, networking, ECS, scenes, UI. If chad ever needs to
know what a "game object" is, that's a bug.

## Web

`winit` and `wgpu` are re-exported (`chad::winit`, `chad::wgpu`) — depend
only on chad and versions can never drift. Build for the web with:

```sh
cargo build --lib --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir pkg target/wasm32-unknown-unknown/debug/your_crate.wasm
```

Your crate needs `crate-type = ["cdylib", "rlib"]` and a
`#[wasm_bindgen(start)]` entry that calls the same `chad::run` as native.
The canvas is appended to `<body>` and fills its parent; `Config.size` is
only the initial backing store on web. WebGPU only (no WebGL fallback).

## Versioning

Because `winit` and `wgpu` are re-exported, their major versions are part of
chad's public API: a release that bumps either is a breaking release of chad.
Currently winit 0.30, wgpu 30 (for winit, `0.30` *is* the major under pre-1.0
semver). Reach them through `chad::winit` / `chad::wgpu`, never your own dep.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

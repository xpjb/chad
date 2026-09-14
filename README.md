# chad

A thin platform layer for games and apps on **winit + wgpu**. Not an engine.

**Desktop · WebGPU · Android (NativeActivity + Vulkan)**

> **Re-exports winit `0.30` + wgpu `30`** as `chad::winit` / `chad::wgpu` — write against those, don't add your own. See [Versioning](#versioning).

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

Run an example with `cargo run --example <name>`. Click a preview to run its Web build:

| # | preview | example / focus |
|---:|---|---|
| 1 | [![Triangle](https://raw.githubusercontent.com/xpjb/chad/master/gallery/triangle.png)](https://xpjb.github.io/chad/triangle/) | [`triangle`](examples/triangle.rs) · [Web](https://xpjb.github.io/chad/triangle/)<br>Smallest complete app |
| 2 | [![Halfpipe](https://raw.githubusercontent.com/xpjb/chad/master/gallery/halfpipe.png)](https://xpjb.github.io/chad/halfpipe/) | [`halfpipe`](examples/halfpipe.rs) · [Web](https://xpjb.github.io/chad/halfpipe/)<br>Fixed 20 Hz updates and render interpolation; Space toggles smoothing |
| 3 | [![Clock](https://raw.githubusercontent.com/xpjb/chad/master/gallery/clock.png)](https://xpjb.github.io/chad/clock/) | [`clock`](examples/clock.rs) · [Web](https://xpjb.github.io/chad/clock/)<br>On-demand redraw with `Waker`, plus a procedural window icon |
| 4 | [![Sprite batch](https://raw.githubusercontent.com/xpjb/chad/master/gallery/sprite_batch.png)](https://xpjb.github.io/chad/sprite_batch/) | [`sprite_batch`](examples/sprite_batch.rs) · [Web](https://xpjb.github.io/chad/sprite_batch/)<br>Generated character texture, dynamic instances, alpha blending, one draw |
| 5 | [![Fractal flight](https://raw.githubusercontent.com/xpjb/chad/master/gallery/fractal_flight.png)](https://xpjb.github.io/chad/fractal_flight/) | [`fractal_flight`](examples/fractal_flight.rs) · [Web](https://xpjb.github.io/chad/fractal_flight/)<br>Advanced Mandelbox raymarching showcase with flight controls and collision |

## Headless rendering

On native targets, `HeadlessCtx` owns a wgpu device and one RGBA8 sRGB
offscreen target. Both `Ctx` and `HeadlessCtx` implement `RenderContext`, so
window-independent setup and drawing can use the same renderer:

```rust
fn draw(ctx: &impl chad::RenderContext, view: &chad::wgpu::TextureView) {
    // Build/encode/submit raw wgpu work through ctx.device() and ctx.queue().
}

let config = chad::Config {
    size: (1280, 720),
    ..Default::default()
};
let ctx = chad::HeadlessCtx::new(&config)?;
draw(&ctx, ctx.view());
let rgba = ctx.read_rgba8()?;
```

The readback is tightly packed, top-to-bottom RGBA8; image encoding remains
consumer-owned. The examples add a local `--screenshot <path>` path using this
same rendering code. Regenerate every committed example preview with
`scripts/update-gallery.ps1` on Windows or `scripts/update-gallery.sh` on Unix.

## Android

Android uses the separate, Android-only `chad::android` module. Its `run`, `Ctx`, `Config`, and `App` types own NativeActivity startup, Vulkan presentation, and suspend/resume. Desktop/web `run`, `Ctx`, and `ChadApp` keep their existing contract. Rendering code can share `RenderContext` across all three execution paths.

Implement `android::App` and call `android::run` from an exported `android_main(AndroidApp)` in an Android `cdylib`. Chad selects winit's `android-native-activity` feature only on Android. APK packaging stays with the application; no Java/Kotlin code is required for the NativeActivity path.

- `init` runs once after the first GPU and surface are ready. `resumed` follows init and each surface resume.
- `suspended` runs after the surface has been dropped. The game, device, and GPU resources stay alive within that runner.
- Updates and frames run only while a drawing surface exists. The callbacks run on the `android_main` thread, not Java's UI thread.
- `elapsed` is wall time, including suspension. `dt` is the real update interval, reset on resume and focus changes. The application chooses its game-clock, input cancellation, and save policy.
- Presentation defaults to `AutoVsync` with a one-frame latency hint. This runner has no desktop sleep/spin frame limiter. `Config.redraw` defaults to `RedrawMode::Continuous`; tools can use `OnDemand` to redraw only on window/device events or `ctx.window.request_redraw()` (also callable from another thread). Surface acquisition failures are retried.
- Process death still needs application-owned durable saves. GPU device loss and a changed surface format require restarting this first implementation.
- **Activity teardown is not process teardown.** Android can destroy and recreate an Activity without ending its process. See [upstream winit issues we're tracking](#upstream-winit-issues-were-tracking) before choosing an exit policy.

The Android runner is now on **master**, after use in Android app/game builds and user-reported phone testing. It is no longer necessary to depend on the `android-runner` branch. Driver compatibility still depends on the device; the current runner requires Vulkan.

Chad is a library, not an installable app. For packaging examples, see the [flow / 100 Android test app](https://github.com/xpjb/flow100/tree/android-app/android) and its build instructions (ARM64, Android 10+, development-signed APK). Building or sideloading a demo is entirely optional; desktop and Web consumers need no Android tooling.

### Upstream winit issues we're tracking

Last checked **2026-09-14** against **0.30.13**, **0.31.0-beta.3**, and upstream
master ([`475f5e2`](https://github.com/rust-windowing/winit/tree/475f5e236366bd2ea5697eea25d3f6c4fc16ad28)).
The Back mapping remains unchanged and both lifecycle problems remain unfixed;
upgrading to that beta/master is not a solution. These are not fixed by Chad's
Android merge.

1. **Android Back naming/documentation mismatch.** The 0.30 documentation labels
   `NamedKey::GoBack` as Android `KEYCODE_BACK`, but the backend actually emits
   `NamedKey::BrowserBack`. Following the documentation alone can therefore leave
   an app's Back handler doing nothing. Related upstream issue:
   [#2304 — Support Back button/KeyCode on Android](https://github.com/rust-windowing/winit/issues/2304).
   That older, broader issue also discusses returning unhandled Back events to
   Android; it is not a dedicated report of this exact naming mismatch.
   **For now:** apps should recognize `BrowserBack | GoBack` (and Escape if
   appropriate). Chad preserves raw events; the app decides what Back does.
   **Waiting for:** clarification of the intended mapping and matching docs,
   rather than silently changing an established mapping and breaking consumers.

2. **Activity Destroy does not exit the event loop.** winit's Android backend
   ignores `MainEvent::Destroy` instead of ending the loop. `android_main()` can
   consequently fail to return, leaving native Activity teardown stuck. Ordinary
   surface suspension/resumption is a different path and does not fix this.
   Upstream: [#4303 — Destroy does not exit the event loop](https://github.com/rust-windowing/winit/issues/4303).
   **Needed:** backend handling that ends the loop and permits cleanup/return
   when the Activity is actually destroyed, not on every background/suspend.

3. **A replacement Activity cannot create a new event loop in the same process.**
   After an earlier loop exits, Android may call `android_main()` again with a
   new Activity/`AndroidApp`, but winit's process-wide guard rejects the new loop
   with `RecreationAttempt` ("EventLoop can't be recreated"). Fixing Destroy alone
   exposes this second problem. Upstream:
   [#3325 — RecreationAttempt on Activity reopen](https://github.com/rust-windowing/winit/issues/3325).
   **Needed:** safe sequential loop recreation after the previous loop has fully
   torn down. This is not a request to allow arbitrary concurrent loops/Activities.

There is an [unmerged candidate patch for both lifecycle issues](https://github.com/rib/winit/commit/c28e425214e82bdb86dcdf89c9488554a18e24b2),
linked from #4303. **Chad does not currently include that patch.** Any fix carried
through Chad's Android dependency integration needs regression checks for
background/resume, actual Activity destruction, and subsequent creation in the
same process; it should not become bespoke teardown code in every app.

For ordinary root-screen Back, an app can background its task (for example,
`Activity.moveTaskToBack(true)`) and keep the existing loop resumable. This avoids
that exit path; **it does not solve actual Activity destruction/recreation**.
Process termination is not Chad's generic lifecycle policy. In particular, an
app-owned process-exit workaround after `android::run` returns cannot fix an
ignored Destroy event that prevents the runner from returning in the first place.

## What you get

- Window + full wgpu init, blocking on native, async on wasm (browsers forbid
  blocking the main thread; chad runs init as a future and calls your `init`
  when the GPU is ready)
- Surface lifecycle: resize, surface-lost recovery, minimize handling, sRGB
  view formats where the surface is non-sRGB (WebGPU), show-after-first-frame
  (no white flash)
- A native `HeadlessCtx` implementing the same `RenderContext` as `Ctx`, with an owned offscreen target and RGBA8 readback
- Frame timing: variable dt or a fix-your-timestep accumulator
  (`Timestep::Fixed`) with interpolation alpha and a death-spiral clamp; dt is
  clamped so debugger pauses don't launch your player through a wall
- Vsync as a `Config` bool with a runtime toggle (`ctx.set_vsync`), or an
  exact `wgpu::PresentMode` if you know what you want
- `DeviceEvent` forwarding (raw mouse deltas — what a mouselook camera needs)
- Continuous or on-demand redraw, optional hybrid sleep/spin frame cap
- A payloadless `Waker` to nudge the loop from other threads (drain your own
  channels in `update`)
- Logging and panic reporting are installed by default: native panics also
  write `crash.log`; Web logs and panics go to the browser console
- `Config` exposes `wgpu` device features and limits, so needing push
  constants doesn't mean forking the runner

## What you don't get (on purpose)

The scope rule: something belongs in chad only if implementing it correctly
requires touching the event loop, window, or surface lifecycle — or is
literally identical in every game. Everything else is your code: input
mapping, audio, assets, networking, ECS, scenes, UI. If chad ever needs to
know what a "game object" is, that's a bug.

## Web

Desktop and Web use the same `ChadApp`. Keep it in `src/lib.rs`, expose one
shared `run`, and add the small browser entry point below.

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib", "rlib"]

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "0.2"
```

```rust
pub fn run() -> Result<(), String> {
    chad::run::<Game>(Config::default())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn wasm_start() {
    run().unwrap();
}
```

The native `src/main.rs` can call the same function:

```rust
fn main() -> Result<(), String> {
    your_crate::run()
}
```

Install the target and `wasm-bindgen` CLI, then build the library and generate
the browser module. The CLI version must match the `wasm-bindgen` version in
`Cargo.lock`.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
cargo build --lib --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web/pkg target/wasm32-unknown-unknown/debug/your_crate.wasm
```

Use a minimal `web/index.html`:

```html
<!doctype html>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>My game</title>
<style>
  html, body { margin: 0; width: 100%; height: 100%; overflow: hidden; background: #000; color: #fff; }
  canvas { display: block; }
</style>
<body>
  <script type="module">
    import init from "./pkg/your_crate.js";
    if (!navigator.gpu) {
      document.body.textContent = "This game requires WebGPU.";
    } else {
      init().catch((error) => console.error(error));
    }
  </script>
</body>
```

Serve it over HTTP—for example,
`python -m http.server 8080 --directory web`—and open
`http://localhost:8080`. chad appends its canvas to `<body>` and keeps it at
the body's size; `Config.size` is only the initial backing size. WebGPU/game
initialization continues asynchronously after the module loads. After the
first frame is presented, chad dispatches a `chad-ready` event on `window`;
loading screens can wait for it. Fatal startup errors and panics are reported
to the browser console. WebGPU only; there is no WebGL fallback.

## Versioning

Because `winit` and `wgpu` are re-exported, their major versions are part of
chad's public API: a release that bumps either is a breaking release of chad.
Currently winit 0.30, wgpu 30 (for winit, `0.30` *is* the major under pre-1.0
semver). Reach them through `chad::winit` / `chad::wgpu`, never your own dep.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

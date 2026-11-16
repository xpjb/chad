#[cfg(target_os = "android")]
use chad::android::{App, Config, Ctx};
#[cfg(target_os = "android")]
use chad::{MobileEvent, wgpu, winit};

#[cfg(target_os = "android")]
struct AndroidExample;

#[cfg(target_os = "android")]
impl App for AndroidExample {
    fn init(_ctx: &mut Ctx) -> Result<Self, String> {
        Ok(Self)
    }

    fn event(&mut self, _ctx: &mut Ctx, _event: &winit::event::WindowEvent) {}

    fn mobile_event(&mut self, _ctx: &mut Ctx, _event: MobileEvent) {}

    fn update(&mut self, _ctx: &mut Ctx) {}

    fn frame(&mut self, _ctx: &mut Ctx, _view: &wgpu::TextureView) {}
}

#[cfg(target_os = "android")]
chad::android_activity_entry!(AndroidExample, Config::default());

fn main() {}

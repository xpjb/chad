/// GPU and frame state shared by windowed and offscreen renderers.
///
/// Rendering code should depend on this trait when it does not need window or
/// event-loop operations. The returned device and queue are raw wgpu handles.
pub trait RenderContext {
    /// Raw wgpu device selected for this context.
    fn device(&self) -> &wgpu::Device;
    /// Raw wgpu queue paired with [`Self::device`].
    fn queue(&self) -> &wgpu::Queue;
    /// Texture format expected by render pipelines targeting this context.
    fn format(&self) -> wgpu::TextureFormat;
    /// Current render-target size in physical pixels.
    fn size(&self) -> (u32, u32);
    /// Seconds in the current simulation step.
    fn dt(&self) -> f32;
    /// Seconds since application initialization.
    fn elapsed(&self) -> f32;
    /// Number of frames presented or explicitly assigned by a headless caller.
    fn frame_index(&self) -> u64;
    /// Fixed-step interpolation factor.
    fn alpha(&self) -> f32;
}

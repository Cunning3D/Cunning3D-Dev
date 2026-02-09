#![allow(unsafe_code)]

pub use wgpu;

pub mod sdf;

use std::sync::Arc;

/// You can use this for storage when implementing [`CallbackTrait`].
pub type CallbackResources = type_map::concurrent::TypeMap;

/// Callback-shared target color format (matches the active render pass).
#[derive(Clone, Copy, Debug)]
pub struct TargetFormat(pub wgpu::TextureFormat);

/// Information about the screen used for rendering.
#[derive(Clone, Copy, Debug)]
pub struct ScreenDescriptor {
    /// Size of the window in physical pixels.
    pub size_in_pixels: [u32; 2],
    /// HiDPI scale factor (pixels per point).
    pub pixels_per_point: f32,
}

/// A callback trait that can be used to compose an [`epaint::PaintCallback`] via [`Callback`].
pub trait CallbackTrait: Send + Sync {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> { Vec::new() }

    fn finish_prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> { Vec::new() }

    fn paint<'a>(
        &'a self,
        info: epaint::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'a>,
        resources: &'a CallbackResources,
    );
}

pub struct Callback(pub Box<dyn CallbackTrait>);

impl Callback {
    pub fn new_paint_callback(rect: egui::Rect, callback: impl CallbackTrait + 'static) -> epaint::PaintCallback {
        epaint::PaintCallback { rect, callback: Arc::new(Self(Box::new(callback))) }
    }
}



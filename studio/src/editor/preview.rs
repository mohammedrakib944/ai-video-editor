//! The preview: draws the evaluated scene with the compositor inside iced's
//! own render pass, letterboxed to the canvas aspect ratio.

use std::sync::Arc;

use iced::mouse;
use iced::wgpu;
use iced::widget::shader::{self, Viewport};
use iced::Rectangle;

use crate::render::compositor::Compositor;
use crate::render::scene::Scene;

pub struct Preview {
    pub scene: Arc<Scene>,
}

impl<Message> shader::Program<Message> for Preview {
    type State = ();
    type Primitive = PreviewPrimitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> PreviewPrimitive {
        PreviewPrimitive { scene: self.scene.clone() }
    }
}

#[derive(Debug)]
pub struct PreviewPrimitive {
    scene: Arc<Scene>,
}

pub struct PreviewPipeline {
    compositor: Compositor,
    /// Canvas rectangle in physical pixels.
    rect: [f32; 4],
}

impl shader::Pipeline for PreviewPipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        PreviewPipeline { compositor: Compositor::new(device, queue, format), rect: [0.0; 4] }
    }
}

impl shader::Primitive for PreviewPrimitive {
    type Pipeline = PreviewPipeline;

    fn prepare(&self, pipeline: &mut PreviewPipeline, device: &wgpu::Device, queue: &wgpu::Queue, bounds: &Rectangle, viewport: &Viewport) {
        let scale = viewport.scale_factor() as f32;
        let (cw, ch) = (self.scene.width as f32, self.scene.height as f32);
        let fit = (bounds.width / cw).min(bounds.height / ch);
        let (w, h) = (cw * fit, ch * fit);
        let (x, y) = (bounds.x + (bounds.width - w) / 2.0, bounds.y + (bounds.height - h) / 2.0);
        pipeline.rect = [(x * scale).round(), (y * scale).round(), (w * scale).round().max(1.0), (h * scale).round().max(1.0)];
        let aa = cw / pipeline.rect[2];
        pipeline.compositor.prepare(device, queue, &self.scene, aa);
    }

    fn draw(&self, pipeline: &PreviewPipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        let [x, y, w, h] = pipeline.rect;
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_scissor_rect(x.max(0.0) as u32, y.max(0.0) as u32, w as u32, h as u32);
        pipeline.compositor.draw(pass);
        true
    }
}

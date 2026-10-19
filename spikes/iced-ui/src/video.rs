//! FFmpeg decodes to raw YUV 4:2:0 on a background thread; the GPU converts to RGB.
//! Uploading YUV instead of RGBA moves 2.7x less data per frame.

use std::fmt;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, sync_channel};

use iced::Rectangle;
use iced::mouse;
use iced::wgpu;
use iced::widget::shader::{self, Viewport};

pub struct Frame {
    pub index: u64,
    pub width: u32,
    pub height: u32,
    /// Y plane, then U, then V (planar 4:2:0).
    pub planes: Vec<u8>,
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Frame #{} {}x{}", self.index, self.width, self.height)
    }
}

pub struct Decoder {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    rx: Receiver<Frame>,
    child: Child,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Decoder {
    /// Starts decoding `path` in a loop. The channel is small so the decoder runs just ahead of playback.
    pub fn open(path: &Path) -> Result<Decoder, String> {
        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,r_frame_rate", "-of", "csv=p=0"])
            .arg(path)
            .output()
            .map_err(|e| format!("ffprobe: {e}"))?;
        let line = String::from_utf8_lossy(&probe.stdout);
        let parts: Vec<&str> = line.trim().split(',').collect();
        let [w, h, rate] = parts[..] else { return Err(format!("could not probe {}", path.display())) };
        let (width, height): (u32, u32) = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
        let fps = match rate.split_once('/') {
            Some((n, d)) => n.parse::<f64>().unwrap_or(30.0) / d.parse::<f64>().unwrap_or(1.0),
            None => rate.parse().unwrap_or(30.0),
        };

        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-stream_loop", "-1", "-i"])
            .arg(path)
            .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "pipe:1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let mut stdout = child.stdout.take().unwrap();
        let (tx, rx) = sync_channel::<Frame>(4);
        let frame_len = (width * height + 2 * (width / 2) * (height / 2)) as usize;
        std::thread::Builder::new()
            .name("decoder".into())
            .spawn(move || {
                for index in 0.. {
                    let mut planes = vec![0u8; frame_len];
                    if stdout.read_exact(&mut planes).is_err() || tx.send(Frame { index, width, height, planes }).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Decoder { width, height, fps, rx, child })
    }

    pub fn try_next(&self) -> Option<Frame> {
        self.rx.try_recv().ok()
    }
}

/// The shader widget program: draws whatever frame it is given.
pub struct VideoView {
    pub frame: Option<Arc<Frame>>,
}

impl<Message> shader::Program<Message> for VideoView {
    type State = ();
    type Primitive = VideoPrimitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> VideoPrimitive {
        VideoPrimitive { frame: self.frame.clone() }
    }
}

#[derive(Debug)]
pub struct VideoPrimitive {
    frame: Option<Arc<Frame>>,
}

impl shader::Primitive for VideoPrimitive {
    type Pipeline = VideoPipeline;

    fn prepare(&self, pipeline: &mut VideoPipeline, device: &wgpu::Device, queue: &wgpu::Queue, bounds: &Rectangle, _viewport: &Viewport) {
        let Some(frame) = &self.frame else { return };
        pipeline.upload(device, queue, frame);
        let (bw, bh) = (bounds.width.max(1.0), bounds.height.max(1.0));
        let (box_aspect, frame_aspect) = (bw / bh, frame.width as f32 / frame.height as f32);
        let scale = if frame_aspect > box_aspect { [1.0, box_aspect / frame_aspect] } else { [frame_aspect / box_aspect, 1.0] };
        let uniforms = [scale[0], scale[1], if pipeline.srgb_target { 1.0 } else { 0.0 }, 0.0];
        queue.write_buffer(&pipeline.uniforms, 0, bytemuck::cast_slice(&uniforms));
    }

    fn draw(&self, pipeline: &VideoPipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        if self.frame.is_none() {
            return true;
        }
        if let Some(textures) = &pipeline.textures {
            render_pass.set_pipeline(&pipeline.pipeline);
            render_pass.set_bind_group(0, &textures.bind_group, &[]);
            render_pass.draw(0..4, 0..1);
        }
        true
    }
}

struct Textures {
    size: (u32, u32),
    y: wgpu::Texture,
    u: wgpu::Texture,
    v: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

pub struct VideoPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    srgb_target: bool,
    textures: Option<Textures>,
    uploaded: Option<u64>,
}

impl shader::Pipeline for VideoPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yuv"),
            source: wgpu::ShaderSource::Wgsl(include_str!("yuv.wgsl").into()),
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yuv"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yuv"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yuv"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        VideoPipeline { pipeline, layout, sampler, uniforms, srgb_target: format.is_srgb(), textures: None, uploaded: None }
    }
}

impl VideoPipeline {
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &Frame) {
        if self.uploaded == Some(frame.index) && self.textures.as_ref().is_some_and(|t| t.size == (frame.width, frame.height)) {
            return;
        }
        let (w, h) = (frame.width, frame.height);
        if self.textures.as_ref().is_none_or(|t| t.size != (w, h)) {
            let plane = |label, w, h| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            };
            let (y, u, v) = (plane("y", w, h), plane("u", w / 2, h / 2), plane("v", w / 2, h / 2));
            let views = [&y, &u, &v].map(|t| t.create_view(&Default::default()));
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("yuv"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.uniforms.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[0]) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[1]) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views[2]) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            self.textures = Some(Textures { size: (w, h), y, u, v, bind_group });
        }
        let t = self.textures.as_ref().unwrap();
        let luma = (w * h) as usize;
        let chroma = ((w / 2) * (h / 2)) as usize;
        let write = |texture: &wgpu::Texture, data: &[u8], w: u32, h: u32| {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        };
        write(&t.y, &frame.planes[..luma], w, h);
        write(&t.u, &frame.planes[luma..luma + chroma], w / 2, h / 2);
        write(&t.v, &frame.planes[luma + chroma..luma + 2 * chroma], w / 2, h / 2);
        self.uploaded = Some(frame.index);
    }
}

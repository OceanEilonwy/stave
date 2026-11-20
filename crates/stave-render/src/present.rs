//! Drawing on a surface: a page's canvas (the web viewer) or a window's.
//!
//! The frame is drawn as for any other picture, into the live target, and
//! then drawn once more onto the surface's texture by a full-screen
//! triangle that samples it. The surface may have any format with an sRGB
//! form (a canvas usually offers `bgra8unorm`): it is drawn through that
//! sRGB view, so the colours come out as they do offscreen.

use crate::{RenderError, Resolved, SurfaceTrouble, convert};

/// The blit: a triangle covering the screen, sampling the frame's corner
/// of the target.
const BLIT_SHADER: &str = r"
struct Share {
    // xy: the frame's share of the target, across and down.
    size: vec4<f32>,
}

@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;
@group(0) @binding(2) var<uniform> share: Share;

struct Blitted {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn blit_vertex(@builtin(vertex_index) index: u32) -> Blitted {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Blitted;
    out.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y) * share.size.xy;
    return out;
}

@fragment
fn blit_fragment(in: Blitted) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(picture, picture_sampler, in.uv).rgb, 1.0);
}
";

/// A surface and what draws on it.
pub(crate) struct Presenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The sRGB view of the surface's format, which the blit draws in.
    view_format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The frame's share of the target, for the shader.
    share: wgpu::Buffer,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Presenter {
    /// Configures `surface` for `device`, `width` by `height` pixels, in
    /// the first of its formats with an sRGB form.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        adapter: &wgpu::Adapter,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Presenter, RenderError> {
        let capabilities = surface.get_capabilities(adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| format.add_srgb_suffix().is_srgb())
            .ok_or(RenderError::UnsuitableSurface)?;
        let view_format = format.add_srgb_suffix();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            desired_maximum_frame_latency: 2,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: capabilities
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: if view_format == format {
                Vec::new()
            } else {
                vec![view_format]
            },
        };
        surface.configure(device, &config);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("blit_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("blit_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: view_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let share = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blit share"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Presenter {
            surface,
            config,
            view_format,
            pipeline,
            layout,
            sampler,
            share,
            device: device.clone(),
            queue: queue.clone(),
        })
    }

    /// Configures the surface again at `width` by `height` pixels.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if (self.config.width, self.config.height) != (width, height) {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(device, &self.config);
        }
    }

    /// The surface's next texture to draw in, configuring it again once
    /// when it is out of date.
    pub fn next_texture(&self, device: &wgpu::Device) -> Result<wgpu::SurfaceTexture, RenderError> {
        let mut configured = false;
        loop {
            let trouble = match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(texture)
                | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => return Ok(texture),
                wgpu::CurrentSurfaceTexture::Outdated if !configured => {
                    self.surface.configure(device, &self.config);
                    configured = true;
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Outdated => SurfaceTrouble::Outdated,
                wgpu::CurrentSurfaceTexture::Timeout => SurfaceTrouble::Timeout,
                wgpu::CurrentSurfaceTexture::Occluded => SurfaceTrouble::Occluded,
                wgpu::CurrentSurfaceTexture::Lost => SurfaceTrouble::Lost,
                wgpu::CurrentSurfaceTexture::Validation => SurfaceTrouble::Validation,
            };
            return Err(RenderError::Surface(trouble));
        }
    }

    /// Draws `resolved`'s picture over the whole of `output`.
    pub fn blit(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        resolved: &Resolved<'_>,
        output: &wgpu::Texture,
    ) {
        let size = resolved.texture.size();
        let share = |frame: u32, target: u32| {
            convert::f32_of_u32(frame) / convert::f32_of_u32(target.max(1))
        };
        let shares = [
            share(resolved.width, size.width),
            share(resolved.height, size.height),
            0.0_f32,
            0.0,
        ];
        self.queue
            .write_buffer(&self.share, 0, bytemuck::cast_slice(&shares));
        let picture = resolved
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&picture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.share.as_entire_binding(),
                },
            ],
        });
        let view = output.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

//! Offscreen rendering with wgpu: the scene is drawn into a multisampled
//! texture, resolved, and read back as RGBA bytes. The window shows those
//! bytes as a GDK texture; `--screenshot` writes them to a PNG.
//!
//! Reading back costs a copy per frame, but it works on every adapter and
//! every GDK backend without sharing GPU memory between wgpu and GTK, and
//! frames are only drawn when something changes.

use std::num::NonZeroU64;

use anyhow::{Context as _, Result, anyhow, bail};
use glam::{Mat4, Vec3};
use stave_scene::{Material, NodeId, Orbit, Posed, SceneGraph};
use wgpu::util::DeviceExt as _;

const COLOUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
const SHADOW_SIZE: u32 = 2048;

/// The light grey stage behind the design (#e7e3dd), linear.
const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.7991,
    g: 0.7681,
    b: 0.7230,
    a: 1.0,
};

/// The selection accent (#2f6fd6), linear.
const ACCENT: [f32; 3] = [0.0284, 0.1590, 0.6724];

/// Toward the key light: front, left and above, like a window.
const KEY_TOWARD: Vec3 = Vec3::new(-0.42, 0.78, 0.47);
/// Toward the fill light: front right, lower.
const FILL_TOWARD: Vec3 = Vec3::new(0.85, 0.42, 0.30);

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_projection: [[f32; 4]; 4],
    light_view_projection: [[f32; 4]; 4],
    eye: [f32; 4],
    key_direction: [f32; 4],
    fill_direction: [f32; 4],
    floor: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct NodeData {
    model: [[f32; 4]; 4],
    colour: [f32; 4],
    params: [f32; 4],
    tint: [f32; 4],
}

/// A finished frame: tightly packed sRGB RGBA, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// What to draw.
pub struct Frame<'a> {
    /// The scene.
    pub scene: &'a SceneGraph,
    /// Where everything is.
    pub posed: &'a Posed,
    /// The camera.
    pub orbit: Orbit,
    /// The selected node, drawn tinted.
    pub selection: Option<NodeId>,
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    edges: Option<(wgpu::Buffer, u32)>,
    /// Hardware: drawn over wood it is flush with or sunk into (a hinge
    /// cup or leaf, until cuts are drawn).
    metal: bool,
}

struct Target {
    width: u32,
    height: u32,
    padded_row: u32,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolved: wgpu::Texture,
    resolved_view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

/// The wgpu device, pipelines and the uploaded scene.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    globals: wgpu::Buffer,
    nodes: wgpu::Buffer,
    node_capacity: u64,
    scene_layout: wgpu::BindGroupLayout,
    scene_group: wgpu::BindGroup,
    shadow_group: wgpu::BindGroup,
    shadow_view: wgpu::TextureView,
    mesh_pipeline: wgpu::RenderPipeline,
    /// Like `mesh_pipeline` without the depth bias that pushes wood back, so
    /// hardware wins where it lies in the same plane as wood.
    metal_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    floor_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    meshes: Vec<GpuMesh>,
    target: Option<Target>,
}

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<stave_geom::Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

fn edge_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

fn node_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("nodes"),
        size: capacity.max(1) * std::mem::size_of::<NodeData>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn scene_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    nodes: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: nodes.as_entire_binding(),
            },
        ],
    })
}

impl Renderer {
    /// Opens a GPU without a window. Fails when no adapter is available.
    pub fn new() -> Result<Renderer> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .context("no graphics adapter found (wgpu)")?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("stave"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .context("the graphics adapter refused a device")?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<Globals>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let node_capacity = 64;
        let nodes = node_buffer(&device, node_capacity);
        let scene_group = scene_group(&device, &scene_layout, &globals, &nodes);

        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow"),
            layout: &shadow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });

        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("main"),
            bind_group_layouts: &[Some(&scene_layout), Some(&shadow_layout)],
            immediate_size: 0,
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow"),
                bind_group_layouts: &[Some(&scene_layout)],
                immediate_size: 0,
            });
        let depth = |write: bool, bias: wgpu::DepthBiasState| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias,
        };
        let multisample = wgpu::MultisampleState {
            count: SAMPLES,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        let blended = [Some(wgpu::ColorTargetState {
            format: COLOUR_FORMAT,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let opaque = [Some(wgpu::ColorTargetState {
            format: COLOUR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let mesh_buffers = [Some(vertex_layout())];
        let edge_buffers = [Some(edge_layout())];
        let mesh_pipeline_with = |label: &'static str, bias: wgpu::DepthBiasState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&main_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("mesh_vertex"),
                    compilation_options: Default::default(),
                    buffers: &mesh_buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(depth(true, bias)),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("mesh_fragment"),
                    compilation_options: Default::default(),
                    targets: &opaque,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let metal_pipeline = mesh_pipeline_with(
            "hardware",
            wgpu::DepthBiasState {
                constant: -2,
                slope_scale: -1.0,
                clamp: 0.0,
            },
        );
        let mesh_pipeline = mesh_pipeline_with(
            "meshes",
            wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 1.0,
                clamp: 0.0,
            },
        );
        let edge_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("outlines"),
            layout: Some(&main_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("edge_vertex"),
                compilation_options: Default::default(),
                buffers: &edge_buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(depth(false, wgpu::DepthBiasState::default())),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("edge_fragment"),
                compilation_options: Default::default(),
                targets: &blended,
            }),
            multiview_mask: None,
            cache: None,
        });
        let floor_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("floor"),
            layout: Some(&main_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("floor_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth(false, wgpu::DepthBiasState::default())),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("floor_fragment"),
                compilation_options: Default::default(),
                targets: &blended,
            }),
            multiview_mask: None,
            cache: None,
        });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("shadow_vertex"),
                compilation_options: Default::default(),
                buffers: &mesh_buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth(
                true,
                wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            )),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        Ok(Renderer {
            device,
            queue,
            adapter_name,
            globals,
            nodes,
            node_capacity,
            scene_layout,
            scene_group,
            shadow_group,
            shadow_view,
            mesh_pipeline,
            metal_pipeline,
            edge_pipeline,
            floor_pipeline,
            shadow_pipeline,
            meshes: Vec::new(),
            target: None,
        })
    }

    /// The adapter's name, for logs.
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Uploads a scene's meshes, replacing the previous scene's.
    pub fn upload(&mut self, scene: &SceneGraph) {
        self.meshes = scene
            .nodes
            .iter()
            .map(|node| {
                let mesh = &node.geometry.mesh;
                let vertices = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("vertices"),
                        contents: bytemuck::cast_slice(&mesh.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let indices = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("indices"),
                        contents: bytemuck::cast_slice(&mesh.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                let segments = &node.geometry.edges.segments;
                let edges = (!segments.is_empty()).then(|| {
                    (
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("edges"),
                                contents: bytemuck::cast_slice(segments),
                                usage: wgpu::BufferUsages::VERTEX,
                            }),
                        u32::try_from(segments.len() * 2).unwrap_or(u32::MAX),
                    )
                });
                GpuMesh {
                    vertices,
                    indices,
                    index_count: u32::try_from(mesh.indices.len()).unwrap_or(u32::MAX),
                    edges,
                    metal: matches!(node.material, Material::Metal),
                }
            })
            .collect();
        let needed = u64::try_from(scene.nodes.len()).unwrap_or(u64::MAX);
        if needed > self.node_capacity {
            self.node_capacity = needed.next_power_of_two();
            self.nodes = node_buffer(&self.device, self.node_capacity);
            self.scene_group =
                scene_group(&self.device, &self.scene_layout, &self.globals, &self.nodes);
        }
    }

    fn ensure_target(&mut self, width: u32, height: u32) {
        let matches = self
            .target
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height);
        if !matches {
            let size = wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };
            let texture = |label, samples, format, usage| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let msaa = texture(
                "colour",
                SAMPLES,
                COLOUR_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
            .create_view(&wgpu::TextureViewDescriptor::default());
            let depth = texture(
                "depth",
                SAMPLES,
                DEPTH_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
            .create_view(&wgpu::TextureViewDescriptor::default());
            let resolved = texture(
                "resolved",
                1,
                COLOUR_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let resolved_view = resolved.create_view(&wgpu::TextureViewDescriptor::default());
            let padded_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: u64::from(padded_row) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.target = Some(Target {
                width,
                height,
                padded_row,
                msaa,
                depth,
                resolved,
                resolved_view,
                readback,
            });
        }
    }

    /// Draws a frame and reads it back.
    pub fn render(&mut self, frame: &Frame<'_>) -> Result<Image> {
        let (width, height) = (frame.width.max(1), frame.height.max(1));
        let limit = self.device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            bail!("a {width}x{height} view is larger than the GPU allows ({limit})");
        }
        if self.meshes.len() != frame.scene.nodes.len() {
            bail!("the scene changed without being uploaded");
        }
        self.write_uniforms(frame, width, height);
        self.ensure_target(width, height);
        let target = self.target.as_ref().context("no render target")?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let visible: Vec<(u32, &GpuMesh)> = frame
            .scene
            .node_ids()
            .zip(&self.meshes)
            .filter(|(id, _)| frame.posed.shown(*id).is_some())
            .filter_map(|(id, mesh)| Some((u32::try_from(id.index()).ok()?, mesh)))
            .collect();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.scene_group, &[]);
            for (index, mesh) in &visible {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, *index..*index + 1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    depth_slice: None,
                    resolve_target: Some(&target.resolved_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BACKGROUND),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.scene_group, &[]);
            pass.set_bind_group(1, &self.shadow_group, &[]);
            for (pipeline, metal) in [(&self.mesh_pipeline, false), (&self.metal_pipeline, true)] {
                pass.set_pipeline(pipeline);
                for (index, mesh) in visible.iter().filter(|(_, mesh)| mesh.metal == metal) {
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, *index..*index + 1);
                }
            }
            pass.set_pipeline(&self.floor_pipeline);
            pass.draw(0..6, 0..1);
            pass.set_pipeline(&self.edge_pipeline);
            for (index, mesh) in &visible {
                if let Some((buffer, count)) = &mesh.edges {
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw(0..*count, *index..*index + 1);
                }
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.resolved,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        self.read_back(width, height)
    }

    fn read_back(&self, width: u32, height: u32) -> Result<Image> {
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| anyhow!("no render target"))?;
        let slice = target.readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // The receiver waits below; if it is gone there is no one to tell.
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .context("waiting for the GPU")?;
        receiver
            .recv()
            .context("the GPU didn't finish the frame")?
            .context("couldn't read the frame back")?;
        let row = usize::try_from(width)? * 4;
        let padded = usize::try_from(target.padded_row)?;
        let mut rgba = Vec::with_capacity(row * usize::try_from(height)?);
        {
            let mapped = slice.get_mapped_range().context("mapping the frame")?;
            for line in mapped.chunks(padded) {
                rgba.extend_from_slice(line.get(..row).unwrap_or(line));
            }
        }
        target.readback.unmap();
        Ok(Image {
            width,
            height,
            rgba,
        })
    }

    fn write_uniforms(&self, frame: &Frame<'_>, width: u32, height: u32) {
        let aspect = width as f32 / height as f32;
        let scene = frame.scene;
        let rest = scene.rest();
        let bounds = scene.bounds(frame.posed).or_else(|| scene.bounds(&rest));
        let (centre, radius, floor_y) = bounds.map_or((Vec3::ZERO, 1000.0, 0.0), |b| {
            (b.centre(), (b.size().length() * 0.5).max(10.0), b.min.y)
        });
        // The floor is where the lowest part stands at rest.
        let floor_y = scene.bounds(&rest).map_or(floor_y, |b| b.min.y);
        let key = KEY_TOWARD.normalize();
        let fill = FILL_TOWARD.normalize();
        // The shadow camera covers the design and the floor around it.
        let reach = radius * 2.6;
        let light_view = Mat4::look_at_rh(centre + key * reach * 2.0, centre, Vec3::Y);
        let light_projection =
            Mat4::orthographic_rh(-reach, reach, -reach, reach, 0.0, reach * 4.0);
        let globals = Globals {
            view_projection: frame.orbit.view_projection(aspect).to_cols_array_2d(),
            light_view_projection: (light_projection * light_view).to_cols_array_2d(),
            eye: frame.orbit.eye().extend(1.0).to_array(),
            key_direction: key.extend(0.0).to_array(),
            fill_direction: fill.extend(0.0).to_array(),
            floor: [centre.x, floor_y, centre.z, radius * 2.4],
        };
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        // Every node of the selected item is tinted (a hinge is two).
        let selected = frame
            .selection
            .and_then(|id| scene.node(id))
            .map(|node| node.item);
        let nodes: Vec<NodeData> = scene
            .node_ids()
            .filter_map(|id| {
                let node = scene.node(id)?;
                let model = frame
                    .posed
                    .transforms
                    .get(id.index())
                    .copied()
                    .unwrap_or(Mat4::IDENTITY);
                let seed = (id.index() % 97) as f32 * 1.618;
                let (colour, params) = match node.material {
                    Material::Wood(wood) => (
                        [wood.colour[0], wood.colour[1], wood.colour[2], wood.figure],
                        [wood.pith[0], wood.pith[1], seed, 0.0],
                    ),
                    Material::Metal => {
                        let [r, g, b] = node.material.base_colour();
                        ([r, g, b, 0.0], [0.0, 0.0, seed, 1.0])
                    }
                };
                let tint = if selected == Some(node.item) {
                    [ACCENT[0], ACCENT[1], ACCENT[2], 0.6]
                } else {
                    [0.0; 4]
                };
                Some(NodeData {
                    model: model.to_cols_array_2d(),
                    colour,
                    params,
                    tint,
                })
            })
            .collect();
        self.queue
            .write_buffer(&self.nodes, 0, bytemuck::cast_slice(&nodes));
    }
}

/// Writes an image as a PNG.
pub fn write_png(image: &Image, path: &std::path::Path) -> Result<()> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("can't create '{}'", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.rgba)?;
    writer.finish()?;
    Ok(())
}

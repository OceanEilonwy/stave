//! Offscreen rendering with wgpu: the scene is drawn into a multisampled
//! texture, resolved, and read back as RGBA bytes. The window shows those
//! bytes as a GDK texture; `--screenshot` writes them to a PNG.
//!
//! Reading back works on every adapter and every GDK backend without
//! sharing GPU memory between wgpu and GTK, and frames are only drawn when
//! something changes. The live view doesn't wait for the GPU: it submits a
//! frame ([`Renderer::submit`]) and a later tick takes it
//! ([`Renderer::take_ready`]) and hands GTK the mapped buffer as it is (see
//! [`readback`]). Screenshots wait ([`Renderer::render`]).

pub mod profile;
mod readback;

pub use readback::{LiveFrame, Ticket};

use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use glam::{Mat4, Vec3};
use stave_scene::{
    Aabb, Highlight, LineKind, Look, Material, NodeId, Orbit, Overlay, Posed, SceneGraph, Section,
};
use wgpu::util::DeviceExt as _;

const COLOUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The selection mask: red where the selection is seen, green wherever it
/// covers.
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;
const SAMPLES: u32 = 4;
const SHADOW_SIZE: u32 = 2048;

/// The stage behind the design, light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backdrop {
    /// A light warm grey (#e8e5df), for the light style.
    #[default]
    Light,
    /// A dark grey (#242428), for the dark style.
    Dark,
}

impl Backdrop {
    /// The ground shadow on it: a warm dark brown on the light stage,
    /// black (and a little stronger) on the dark one, so it darkens both.
    fn shadow(self) -> [f32; 4] {
        match self {
            Backdrop::Light => [0.10, 0.07, 0.04, 0.30],
            Backdrop::Dark => [0.0, 0.0, 0.0, 0.45],
        }
    }

    /// The selection outline on it, linear, and how strongly it shows
    /// where the selection is hidden: the accent, a little deeper on the
    /// light stage, so it stands out against the stage and the wood.
    fn outline(self) -> [f32; 4] {
        match self {
            // #1f5fcc
            Backdrop::Light => [0.0137, 0.1144, 0.6038, 0.3],
            // #2f6fd6
            Backdrop::Dark => [0.0284, 0.1590, 0.6724, 0.35],
        }
    }

    /// Its colour, linear.
    fn colour(self) -> wgpu::Color {
        match self {
            Backdrop::Light => wgpu::Color {
                r: 0.8069,
                g: 0.7835,
                b: 0.7379,
                a: 1.0,
            },
            Backdrop::Dark => wgpu::Color {
                r: 0.0176,
                g: 0.0176,
                b: 0.0212,
                a: 1.0,
            },
        }
    }
}

/// What a selected part's base colour is washed with: a light accent
/// (#6f9ff0), linear, so the part reads as selected without going dark.
const SELECTED_WASH: [f32; 3] = [0.159, 0.3467, 0.8714];

/// The problem red (#d6302f), linear, for parts in a problem.
const PROBLEM: [f32; 3] = [0.6724, 0.0296, 0.0284];

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
    /// xyz: the section's unit normal (zero: no section); w: its offset.
    section: [f32; 4],
    /// rgb: the ground shadow's colour, linear; a: its strength.
    shadow: [f32; 4],
    /// rgb: the selection outline's colour, linear; a: its strength where
    /// the selection is hidden.
    outline: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct NodeData {
    model: [[f32; 4]; 4],
    colour: [f32; 4],
    params: [f32; 4],
    tint: [f32; 4],
    /// x: opacity; y: how far faded toward a pale grey (a ghost); z: how
    /// much darker faces cut by a joint are; w: outline strength.
    style: [f32; 4],
    /// rgb: the selection wash's colour; a: 1 when selected.
    selected: [f32; 4],
}

/// A vertex of the overlay: a point on screen (normalised device
/// coordinates) and a colour.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OverlayVertex {
    position: [f32; 2],
    colour: [f32; 4],
}

/// A finished frame: tightly packed sRGB RGBA, rows top to bottom, alpha
/// 1 everywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// A problem as a frame draws it, borrowed: its nodes tinted red and its
/// shared volumes drawn as red boxes on top of everything.
#[derive(Debug, Clone, Copy)]
pub struct Highlighted<'a> {
    /// The nodes involved.
    pub nodes: &'a [NodeId],
    /// The shared volumes, posed; empty to draw only the nodes.
    pub volumes: &'a [Aabb],
}

impl<'a> From<&'a Highlight> for Highlighted<'a> {
    fn from(highlight: &'a Highlight) -> Self {
        Self {
            nodes: &highlight.nodes,
            volumes: &highlight.volumes,
        }
    }
}

/// What to draw.
pub struct Frame<'a> {
    /// The scene.
    pub scene: &'a SceneGraph,
    /// Where everything is.
    pub posed: &'a Posed,
    /// The camera.
    pub orbit: Orbit,
    /// The selected nodes, drawn tinted: one item's, or every item of a
    /// selected assembly.
    pub selection: &'a [NodeId],
    /// A problem to show: its nodes tinted red and its shared volumes drawn
    /// as red boxes on top of everything.
    pub highlight: Option<Highlighted<'a>>,
    /// A plane cutting the view: what lies beyond it isn't drawn, and the
    /// wood it cuts through is drawn flat.
    pub section: Option<Section>,
    /// How each node looks (focus mode); `None` draws every node as usual.
    pub looks: Option<&'a [Look]>,
    /// Lines and numbered labels drawn on top of everything.
    pub overlay: Option<&'a Overlay>,
    /// Draw the faces cuts made darker, so the joinery reads.
    pub tint_cut_faces: bool,
    /// Spaces lit for the editor (a region under the pointer, or the one
    /// an edit is for): drawn as accent boxes on top of everything.
    pub lit: Option<&'a [Aabb]>,
    /// A tint per node, by node index (rgb linear, a: how much): an edit's
    /// ghosts in the colour of their verdict. A zero `a` leaves the node's
    /// own tint.
    pub tints: Option<&'a [[f32; 4]]>,
    /// The stage behind the design.
    pub backdrop: Backdrop,
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
}

struct Target {
    width: u32,
    height: u32,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// The selection mask, multisampled; its resolved copy is bound for
    /// the outline pass by `mask_group`.
    mask: wgpu::TextureView,
    mask_resolved: wgpu::TextureView,
    mask_group: wgpu::BindGroup,
    resolved: wgpu::Texture,
    resolved_view: wgpu::TextureView,
}

/// What the GPU reported outside any call's result: wgpu's callbacks
/// write it, the next frame reads it.
#[derive(Debug, Default)]
struct Trouble {
    /// Why the device was lost (a driver reset, suspend, an external GPU
    /// unplugged): it must be opened again.
    lost: Option<String>,
    /// The first error since the last frame began. Without a handler wgpu
    /// panics on one, taking the window and unsaved edits with it.
    error: Option<String>,
}

/// What the light sees: the shadow map drawn for one key stays right for
/// every frame with the same key, however the camera moves.
#[derive(Debug, PartialEq)]
struct ShadowKey {
    /// Which upload of meshes was drawn.
    meshes: u64,
    /// The light's view and projection.
    light: [[f32; 4]; 4],
    /// Each node casting a shadow, by index, and where it is; in order of
    /// index.
    casters: Vec<(u32, [[f32; 4]; 4])>,
}

/// The wgpu device, pipelines and the uploaded scene.
pub struct Renderer {
    device: wgpu::Device,
    trouble: Arc<Mutex<Trouble>>,
    queue: wgpu::Queue,
    adapter_name: String,
    globals: wgpu::Buffer,
    nodes: wgpu::Buffer,
    node_capacity: u64,
    scene_layout: wgpu::BindGroupLayout,
    scene_group: wgpu::BindGroup,
    shadow_group: wgpu::BindGroup,
    shadow_view: wgpu::TextureView,
    /// Wood and hardware alike. Parts are drawn with their cuts made, so
    /// pieces that fit meet face to face without overlapping (a tenon in
    /// its mortise, a hinge cup in its hole, a knob's base on its door: the
    /// coincident faces face opposite ways and one is culled), and nothing
    /// needs pulling forward or pushing back against anything else. The
    /// small depth bias only lets outlines, drawn on the surfaces they
    /// outline, win against those surfaces.
    mesh_pipeline: wgpu::RenderPipeline,
    /// In a section, back faces only: seen through the section's cut they
    /// are the inside of the wood, drawn flat as the cut face. With the
    /// meshes' bias, like `mesh_pipeline`.
    section_cut_pipeline: wgpu::RenderPipeline,
    /// In a section, front faces only, pushed back a little further than
    /// the cut faces. Where the section opens a part whose face meets
    /// another's (a hinge cup on the floor of its hole), the opened part's
    /// back face lies exactly on the other's front face, and both are
    /// drawn: the back face must win, because a line of sight leaves the
    /// opened part there before it enters the other. Without the extra
    /// bias the two fight and the cut face comes out torn.
    section_faces_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    floor_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    /// A problem's shared volumes: translucent faces and outlines, drawn
    /// last without a depth test so wood never hides them.
    volume_pipeline: wgpu::RenderPipeline,
    volume_edge_pipeline: wgpu::RenderPipeline,
    /// A lit space (the editor's): accent faces and outlines, on top.
    lit_pipeline: wgpu::RenderPipeline,
    lit_edge_pipeline: wgpu::RenderPipeline,
    /// Ghosts and see-through parts: blended over what is drawn, without
    /// writing depth, pushed back a little so a surface they share with a
    /// solid part (a socket's wall around its rung) never shows through.
    translucent_pipeline: wgpu::RenderPipeline,
    /// The same with back faces, for a section: a see-through part's cut
    /// face shows, faint.
    translucent_section_pipeline: wgpu::RenderPipeline,
    /// The overlay: triangles on screen, on top of everything.
    overlay_pipeline: wgpu::RenderPipeline,
    /// The selection mask: where the selection covers (no depth test),
    /// and where it is seen (tested against the scene's depth, with the
    /// meshes' bias so its own faces pass).
    mask_covered_pipeline: wgpu::RenderPipeline,
    mask_seen_pipeline: wgpu::RenderPipeline,
    mask_layout: wgpu::BindGroupLayout,
    /// The selection outline: a full-screen pass over the mask.
    outline_pipeline: wgpu::RenderPipeline,
    meshes: Vec<GpuMesh>,
    /// Counts uploads, so a shadow map of older meshes is never kept.
    meshes_generation: u64,
    /// What the shadow map holds; `None` before the first frame.
    shadow_key: Option<ShadowKey>,
    target: Option<Target>,
    /// The readback buffers and the frames in them.
    ring: readback::Ring,
}

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x3,
        3 => Float32
    ];
    wgpu::VertexBufferLayout {
        array_stride: crate::convert::bytes(std::mem::size_of::<stave_geom::Vertex>()),
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
        size: capacity.max(1) * crate::convert::bytes(std::mem::size_of::<NodeData>()),
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
    ///
    /// It asks for the low-power adapter (the integrated one on a laptop
    /// with two), which draws a few hundred parts with ease and leaves the
    /// discrete GPU asleep; `WGPU_POWER_PREF=high` asks for the other.
    pub fn new() -> Result<Renderer> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:
                wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::LowPower),
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
        let trouble = Arc::new(Mutex::new(Trouble::default()));
        {
            let trouble = Arc::clone(&trouble);
            device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                eprintln!("stave: the GPU reported an error: {error}");
                if let Ok(mut trouble) = trouble.lock() {
                    trouble.error.get_or_insert_with(|| error.to_string());
                }
            }));
        }
        {
            let trouble = Arc::clone(&trouble);
            device.set_device_lost_callback(move |reason, message| {
                if let Ok(mut trouble) = trouble.lock() {
                    trouble.lost = Some(format!("{reason:?}: {message}"));
                }
            });
        }

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
                        min_binding_size: NonZeroU64::new(crate::convert::bytes(
                            std::mem::size_of::<Globals>(),
                        )),
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
            size: crate::convert::bytes(std::mem::size_of::<Globals>()),
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
        // Nothing writes the frame's alpha: it stays at the backdrop's 1,
        // so every frame is opaque and its bytes are the same straight or
        // premultiplied, and GTK takes them without converting.
        let blended = [Some(wgpu::ColorTargetState {
            format: COLOUR_FORMAT,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::COLOR,
        })];
        let opaque = [Some(wgpu::ColorTargetState {
            format: COLOUR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::COLOR,
        })];
        let mesh_buffers = [Some(vertex_layout())];
        let edge_buffers = [Some(edge_layout())];
        let mesh_pipeline_with =
            |label: &'static str, bias: wgpu::DepthBiasState, cull_mode: Option<wgpu::Face>| {
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
                        cull_mode,
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
        let mesh_bias = wgpu::DepthBiasState {
            constant: 2,
            slope_scale: 1.0,
            clamp: 0.0,
        };
        let mesh_pipeline = mesh_pipeline_with("meshes", mesh_bias, Some(wgpu::Face::Back));
        let section_cut_pipeline =
            mesh_pipeline_with("cut faces in a section", mesh_bias, Some(wgpu::Face::Front));
        let section_faces_pipeline = mesh_pipeline_with(
            "faces in a section",
            wgpu::DepthBiasState {
                constant: mesh_bias.constant + 6,
                slope_scale: mesh_bias.slope_scale + 1.0,
                clamp: 0.0,
            },
            Some(wgpu::Face::Back),
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
        let overlay =
            |label: &'static str, topology: wgpu::PrimitiveTopology, fragment: &'static str| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&main_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("volume_vertex"),
                        compilation_options: Default::default(),
                        buffers: &edge_buffers,
                    },
                    primitive: wgpu::PrimitiveState {
                        topology,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::Always),
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    }),
                    multisample,
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fragment),
                        compilation_options: Default::default(),
                        targets: &blended,
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let volume_pipeline = overlay(
            "volumes",
            wgpu::PrimitiveTopology::TriangleList,
            "volume_fragment",
        );
        let volume_edge_pipeline = overlay(
            "volume outlines",
            wgpu::PrimitiveTopology::LineList,
            "volume_edge_fragment",
        );
        let lit_pipeline = overlay(
            "lit spaces",
            wgpu::PrimitiveTopology::TriangleList,
            "lit_fragment",
        );
        let lit_edge_pipeline = overlay(
            "lit space outlines",
            wgpu::PrimitiveTopology::LineList,
            "lit_edge_fragment",
        );
        let translucent_with = |label: &'static str, cull_mode: Option<wgpu::Face>| {
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
                    cull_mode,
                    ..Default::default()
                },
                depth_stencil: Some(depth(
                    false,
                    wgpu::DepthBiasState {
                        constant: 24,
                        slope_scale: 3.0,
                        clamp: 0.0,
                    },
                )),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("mesh_fragment"),
                    compilation_options: Default::default(),
                    targets: &blended,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let translucent_pipeline = translucent_with("translucent meshes", Some(wgpu::Face::Back));
        let translucent_section_pipeline =
            translucent_with("translucent meshes in a section", None);
        const OVERLAY_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
        let overlay_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: crate::convert::bytes(std::mem::size_of::<OverlayVertex>()),
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &OVERLAY_ATTRIBUTES,
        })];
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay"),
            layout: Some(&main_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("overlay_vertex"),
                compilation_options: Default::default(),
                buffers: &overlay_buffers,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("overlay_fragment"),
                compilation_options: Default::default(),
                targets: &blended,
            }),
            multiview_mask: None,
            cache: None,
        });
        let mask_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("selection mask"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let outline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("outline"),
            bind_group_layouts: &[Some(&scene_layout), Some(&mask_layout)],
            immediate_size: 0,
        });
        let mask_pipeline =
            |label: &'static str, compare: wgpu::CompareFunction, channel: wgpu::ColorWrites| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&main_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("mesh_vertex"),
                        compilation_options: Default::default(),
                        buffers: &mesh_buffers,
                    },
                    // Both faces: a mirrored part or a section's cut face
                    // counts as much as any other.
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(compare),
                        stencil: wgpu::StencilState::default(),
                        bias: mesh_bias,
                    }),
                    multisample,
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("mask_fragment"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: MASK_FORMAT,
                            blend: None,
                            write_mask: channel,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let mask_covered_pipeline = mask_pipeline(
            "selection mask, covered",
            wgpu::CompareFunction::Always,
            wgpu::ColorWrites::GREEN,
        );
        let mask_seen_pipeline = mask_pipeline(
            "selection mask, seen",
            wgpu::CompareFunction::LessEqual,
            wgpu::ColorWrites::RED,
        );
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("selection outline"),
            layout: Some(&outline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("outline_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("outline_fragment"),
                compilation_options: Default::default(),
                targets: &blended,
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Renderer {
            mask_covered_pipeline,
            mask_seen_pipeline,
            mask_layout,
            outline_pipeline,
            translucent_pipeline,
            translucent_section_pipeline,
            overlay_pipeline,
            device,
            trouble,
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
            section_cut_pipeline,
            section_faces_pipeline,
            edge_pipeline,
            floor_pipeline,
            shadow_pipeline,
            volume_pipeline,
            volume_edge_pipeline,
            lit_pipeline,
            lit_edge_pipeline,
            meshes: Vec::new(),
            meshes_generation: 0,
            shadow_key: None,
            target: None,
            ring: readback::Ring::default(),
        })
    }

    /// The adapter's name, for logs.
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Uploads a scene's meshes, replacing the previous scene's.
    pub fn upload(&mut self, scene: &SceneGraph) {
        let started = profile::start();
        self.upload_meshes(scene);
        profile::finish(&format!("upload of {} meshes", scene.nodes.len()), started);
    }

    fn upload_meshes(&mut self, scene: &SceneGraph) {
        self.meshes_generation = self.meshes_generation.wrapping_add(1);
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
            let mask = texture(
                "selection mask",
                SAMPLES,
                MASK_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
            .create_view(&wgpu::TextureViewDescriptor::default());
            let mask_resolved = texture(
                "selection mask, resolved",
                1,
                MASK_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            )
            .create_view(&wgpu::TextureViewDescriptor::default());
            let mask_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("selection mask"),
                layout: &self.mask_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&mask_resolved),
                }],
            });
            self.target = Some(Target {
                width,
                height,
                msaa,
                depth,
                mask,
                mask_resolved,
                mask_group,
                resolved,
                resolved_view,
            });
        }
    }

    /// Draws a frame and waits for it (at most [`readback::PATIENCE`]):
    /// for screenshots and still pictures. The live view uses
    /// [`Renderer::submit`] instead.
    ///
    /// When the device has been lost it is opened again, the frame's scene
    /// uploaded to it, and the frame drawn there; an error the GPU reports
    /// while drawing fails the frame instead of the application.
    pub fn render(&mut self, frame: &Frame<'_>) -> Result<Image> {
        let drawn = self.draw_checked(frame)?;
        self.read_back(drawn)
    }

    /// Draws a frame and returns at once, without waiting for the GPU
    /// (unless it is already [`readback`]'s limit of frames behind).
    /// [`Renderer::take_ready`] takes it once it has arrived.
    ///
    /// A lost device and errors are dealt with as by [`Renderer::render`].
    pub fn submit(&mut self, frame: &Frame<'_>) -> Result<Ticket> {
        if let Err(error) = self.ring.make_room(&self.device)
            && !self.is_lost()
        {
            return Err(error);
        }
        let drawn = self.draw_checked(frame)?;
        Ok(self.ring.push(drawn))
    }

    /// The newest submitted frame that has arrived, if one has since the
    /// last call; older ones are passed over. Never waits. An error says
    /// why the newest frame to finish failed (an error mapping it, or the
    /// GPU taking longer than [`readback::PATIENCE`]).
    pub fn take_ready(&mut self) -> Result<Option<LiveFrame>> {
        self.ring.take(&self.device)
    }

    /// Whether nothing more will come of `ticket`'s frame: it was taken,
    /// passed over for a newer one, or failed.
    pub fn settled(&self, ticket: Ticket) -> bool {
        self.ring.settled(ticket)
    }

    /// Draws a frame into a readback buffer, opening the device again
    /// when it has been lost and turning an error the GPU reports into a
    /// failed frame.
    fn draw_checked(&mut self, frame: &Frame<'_>) -> Result<readback::Drawn> {
        self.recover_if_lost(frame.scene)?;
        let drawn = self.draw(frame);
        // A device lost while drawing, or before (it may only say so when
        // polled): once more, on a new one. Frames still on the old one
        // count as done.
        let _ = self.device.poll(wgpu::PollType::Poll);
        let drawn = match drawn {
            _ if self.is_lost() => {
                self.recover_if_lost(frame.scene)?;
                self.draw(frame)
            }
            drawn => drawn,
        };
        let error = self
            .trouble
            .lock()
            .ok()
            .and_then(|mut trouble| trouble.error.take());
        match error {
            Some(error) => Err(anyhow!("the GPU couldn't draw the frame: {error}")),
            None => drawn,
        }
    }

    /// Whether wgpu has said the device is lost.
    fn is_lost(&self) -> bool {
        self.trouble
            .lock()
            .is_ok_and(|trouble| trouble.lost.is_some())
    }

    /// Opens the GPU again and uploads `scene` to it, when the device has
    /// been lost.
    fn recover_if_lost(&mut self, scene: &SceneGraph) -> Result<()> {
        let lost = self
            .trouble
            .lock()
            .ok()
            .and_then(|mut trouble| trouble.lost.take());
        if let Some(reason) = lost {
            eprintln!("stave: the graphics device was lost ({reason}); opening it again");
            let ring = readback::Ring::after(&self.ring);
            *self = Renderer::new()
                .context("the graphics device was lost and couldn't be opened again")?;
            self.ring = ring;
            self.upload(scene);
        }
        Ok(())
    }

    fn draw(&mut self, frame: &Frame<'_>) -> Result<readback::Drawn> {
        let (width, height) = (frame.width.max(1), frame.height.max(1));
        let limit = self.device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            bail!("a {width}x{height} view is larger than the GPU allows ({limit})");
        }
        if self.meshes.len() != frame.scene.nodes.len() {
            bail!("the scene changed without being uploaded");
        }
        let started = profile::start();
        let light = self.write_uniforms(frame, width, height);
        self.ensure_target(width, height);
        let layout = readback::Layout::new(width, height);
        let buffer = self.ring.buffer(&self.device, layout);
        let target = self.target.as_ref().context("no render target")?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let volumes = frame
            .highlight
            .filter(|h| !h.volumes.is_empty())
            .map(|h| self.volume_buffers(h.volumes));
        let lit = frame
            .lit
            .filter(|boxes| !boxes.is_empty())
            .map(|boxes| self.volume_buffers(boxes));
        let look = |id: NodeId| {
            frame
                .looks
                .and_then(|looks| looks.get(id.index()).copied())
                .unwrap_or(Look::Normal)
        };
        let visible: Vec<(u32, &GpuMesh)> = frame
            .scene
            .node_ids()
            .zip(&self.meshes)
            .filter(|(id, _)| frame.posed.shown(*id).is_some() && look(*id) == Look::Normal)
            .filter_map(|(id, mesh)| Some((u32::try_from(id.index()).ok()?, mesh)))
            .collect();
        // Ghosts and see-through parts, farthest first so they blend in
        // order.
        let eye = frame.orbit.eye();
        let mut translucent: Vec<(f32, Look, u32, &GpuMesh)> = frame
            .scene
            .node_ids()
            .zip(&self.meshes)
            .filter_map(|(id, mesh)| {
                let transform = frame.posed.shown(id)?;
                let look = look(id);
                if look == Look::Normal {
                    return None;
                }
                let centre = frame
                    .scene
                    .node(id)?
                    .geometry
                    .bounds
                    .transformed(transform)
                    .centre();
                Some((
                    centre.distance(eye),
                    look,
                    u32::try_from(id.index()).ok()?,
                    mesh,
                ))
            })
            .collect();
        translucent.sort_by(|a, b| b.0.total_cmp(&a.0));
        // Every shown node of a selected item is outlined (a hinge is
        // two), solid or see-through.
        let selected_items: Vec<stave_scene::Item> = frame
            .selection
            .iter()
            .filter_map(|id| frame.scene.node(*id))
            .map(|node| node.item)
            .collect();
        let outlined: Vec<(u32, &GpuMesh)> = frame
            .scene
            .node_ids()
            .zip(&self.meshes)
            .filter(|(id, _)| {
                frame.posed.shown(*id).is_some()
                    && frame
                        .scene
                        .node(*id)
                        .is_some_and(|node| selected_items.contains(&node.item))
            })
            .filter_map(|(id, mesh)| Some((u32::try_from(id.index()).ok()?, mesh)))
            .collect();
        let overlay = frame
            .overlay
            .map(|overlay| overlay_triangles(overlay, frame, width, height))
            .filter(|vertices| !vertices.is_empty())
            .map(|vertices| {
                (
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("overlay"),
                            contents: bytemuck::cast_slice(&vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                    u32::try_from(vertices.len()).unwrap_or(u32::MAX),
                )
            });
        // Ghosts cast no shadow; see-through parts do.
        let casting = visible.iter().copied().chain(
            translucent
                .iter()
                .filter(|(_, look, _, _)| *look == Look::XRay)
                .map(|(_, _, index, mesh)| (*index, *mesh)),
        );
        let mut casters: Vec<(u32, [[f32; 4]; 4])> = casting
            .clone()
            .map(|(index, _)| {
                let model = usize::try_from(index)
                    .ok()
                    .and_then(|index| frame.posed.transforms.get(index))
                    .copied()
                    .unwrap_or(Mat4::IDENTITY);
                (index, model.to_cols_array_2d())
            })
            .collect();
        casters.sort_by_key(|(index, _)| *index);
        let shadow_key = ShadowKey {
            meshes: self.meshes_generation,
            light: light.to_cols_array_2d(),
            casters,
        };
        // Only the camera moved (an orbit, a pan, a zoom): the shadow map
        // drawn before is still right.
        let shadow_drawn = self.shadow_key.as_ref() != Some(&shadow_key);
        if shadow_drawn {
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
            for (index, mesh) in casting {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, index..index + 1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(frame.backdrop.colour()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
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
            pass.set_bind_group(0, &self.scene_group, &[]);
            pass.set_bind_group(1, &self.shadow_group, &[]);
            // In a section, every mesh twice: its cut faces, then its
            // faces, which lose to a cut face lying on them.
            let pipelines: &[&wgpu::RenderPipeline] = if frame.section.is_some() {
                &[&self.section_cut_pipeline, &self.section_faces_pipeline]
            } else {
                &[&self.mesh_pipeline]
            };
            for pipeline in pipelines {
                pass.set_pipeline(pipeline);
                for (index, mesh) in &visible {
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
            if !translucent.is_empty() {
                pass.set_pipeline(if frame.section.is_some() {
                    &self.translucent_section_pipeline
                } else {
                    &self.translucent_pipeline
                });
                for (_, _, index, mesh) in &translucent {
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, *index..*index + 1);
                }
                pass.set_pipeline(&self.edge_pipeline);
                for (_, _, index, mesh) in &translucent {
                    if let Some((buffer, count)) = &mesh.edges {
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw(0..*count, *index..*index + 1);
                    }
                }
            }
        }
        if !outlined.is_empty() {
            // The selection mask, tested against the depth just drawn.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("selection mask"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.mask,
                    depth_slice: None,
                    resolve_target: Some(&target.mask_resolved),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
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
            for pipeline in [&self.mask_covered_pipeline, &self.mask_seen_pipeline] {
                pass.set_pipeline(pipeline);
                for (index, mesh) in &outlined {
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, *index..*index + 1);
                }
            }
        }
        {
            // On top: the selection's outline, a problem's shared wood and
            // the overlay.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("on top"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    depth_slice: None,
                    resolve_target: Some(&target.resolved_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
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
            if !outlined.is_empty() {
                pass.set_pipeline(&self.outline_pipeline);
                pass.set_bind_group(1, &target.mask_group, &[]);
                pass.draw(0..3, 0..1);
            }
            pass.set_bind_group(1, &self.shadow_group, &[]);
            if let Some((faces, face_count, lines, line_count)) = &lit {
                pass.set_pipeline(&self.lit_pipeline);
                pass.set_vertex_buffer(0, faces.slice(..));
                pass.draw(0..*face_count, 0..1);
                pass.set_pipeline(&self.lit_edge_pipeline);
                pass.set_vertex_buffer(0, lines.slice(..));
                pass.draw(0..*line_count, 0..1);
            }
            if let Some((faces, face_count, lines, line_count)) = &volumes {
                pass.set_pipeline(&self.volume_pipeline);
                pass.set_vertex_buffer(0, faces.slice(..));
                pass.draw(0..*face_count, 0..1);
                pass.set_pipeline(&self.volume_edge_pipeline);
                pass.set_vertex_buffer(0, lines.slice(..));
                pass.draw(0..*line_count, 0..1);
            }
            if let Some((buffer, count)) = &overlay {
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..*count, 0..1);
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
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(layout.padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let encoded = started.map(|_| Instant::now());
        let submission = self.queue.submit([encoder.finish()]);
        self.shadow_key = Some(shadow_key);
        let submitted = Instant::now();
        let timings = match (started, encoded) {
            (Some(started), Some(encoded)) => Some(profile::FrameTimings {
                width,
                height,
                encode: encoded.duration_since(started),
                submit: submitted.duration_since(encoded),
                gpu_wait: Duration::ZERO,
                readback: Duration::ZERO,
                shadow_drawn,
            }),
            _ => None,
        };
        Ok(readback::Drawn {
            buffer,
            layout,
            submission,
            submitted,
            timings,
        })
    }

    /// Waits for a drawn frame, at most [`readback::PATIENCE`], and copies
    /// it out without the rows' padding.
    fn read_back(&mut self, drawn: readback::Drawn) -> Result<Image> {
        let readback::Drawn {
            buffer,
            layout,
            submission,
            submitted,
            timings,
        } = drawn;
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // The receiver waits below; if it is gone there is no one to tell.
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(readback::PATIENCE),
            })
            .context("waiting for the GPU")?;
        receiver
            .try_recv()
            .context("the GPU didn't finish the frame")?
            .context("couldn't read the frame back")?;
        let mapped_at = Instant::now();
        let row = usize::try_from(layout.width)? * 4;
        let padded = usize::try_from(layout.padded_row)?;
        let mut rgba = Vec::with_capacity(row * usize::try_from(layout.height)?);
        {
            let mapped = slice.get_mapped_range().context("mapping the frame")?;
            for line in mapped.chunks(padded) {
                rgba.extend_from_slice(line.get(..row).unwrap_or(line));
            }
        }
        buffer.unmap();
        self.ring.give_back(buffer);
        if let Some(mut timings) = timings {
            timings.gpu_wait = mapped_at.duration_since(submitted);
            timings.readback = mapped_at.elapsed();
            timings.print();
        }
        Ok(Image {
            width: layout.width,
            height: layout.height,
            rgba,
        })
    }

    /// Vertex buffers of boxes: their faces (triangles) and edges (lines).
    fn volume_buffers(&self, boxes: &[Aabb]) -> (wgpu::Buffer, u32, wgpu::Buffer, u32) {
        let mut faces: Vec<[f32; 3]> = Vec::new();
        let mut lines: Vec<[f32; 3]> = Vec::new();
        for b in boxes {
            let c = b.corners().map(|v| v.to_array());
            // Corner index bits: x = 1, y = 2, z = 4.
            for quad in [
                [0, 2, 3, 1],
                [4, 5, 7, 6],
                [0, 1, 5, 4],
                [2, 6, 7, 3],
                [0, 4, 6, 2],
                [1, 3, 7, 5],
            ] {
                let [a, b2, c2, d] = quad.map(|i| c[i]);
                faces.extend([a, b2, c2, a, c2, d]);
            }
            for (from, to) in [
                (0, 1),
                (2, 3),
                (4, 5),
                (6, 7),
                (0, 2),
                (1, 3),
                (4, 6),
                (5, 7),
                (0, 4),
                (1, 5),
                (2, 6),
                (3, 7),
            ] {
                lines.extend([c[from], c[to]]);
            }
        }
        let buffer = |label, data: &[[f32; 3]]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(data),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        };
        let count = |data: &[[f32; 3]]| u32::try_from(data.len()).unwrap_or(u32::MAX);
        (
            buffer("volume faces", &faces),
            count(&faces),
            buffer("volume outlines", &lines),
            count(&lines),
        )
    }

    /// Writes the globals and every node's data; returns the light's view
    /// and projection, which decides the shadow map.
    fn write_uniforms(&self, frame: &Frame<'_>, width: u32, height: u32) -> Mat4 {
        let aspect = crate::convert::f32_of_u32(width) / crate::convert::f32_of_u32(height);
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
        let light_view_projection = light_projection * light_view;
        let globals = Globals {
            view_projection: frame.orbit.view_projection(aspect).to_cols_array_2d(),
            light_view_projection: light_view_projection.to_cols_array_2d(),
            eye: frame.orbit.eye().extend(1.0).to_array(),
            key_direction: key.extend(0.0).to_array(),
            fill_direction: fill.extend(0.0).to_array(),
            floor: [centre.x, floor_y, centre.z, radius * 2.4],
            section: frame.section.map_or([0.0; 4], |section| {
                section.normal.extend(section.offset).to_array()
            }),
            shadow: frame.backdrop.shadow(),
            outline: frame.backdrop.outline(),
        };
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        // Every node of a selected item is tinted (a hinge is two).
        let selected: Vec<stave_scene::Item> = frame
            .selection
            .iter()
            .filter_map(|id| scene.node(*id))
            .map(|node| node.item)
            .collect();
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
                let seed = crate::convert::f32_of_index(id.index() % 97) * 1.618;
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
                let cut_tint = if frame.tint_cut_faces { 1.0 } else { 0.0 };
                let style = match frame
                    .looks
                    .and_then(|looks| looks.get(id.index()).copied())
                    .unwrap_or(Look::Normal)
                {
                    Look::Normal => [1.0, 0.0, cut_tint, 1.0],
                    Look::Ghost => [0.16, 0.85, 0.0, 0.18],
                    Look::Dim => [0.55, 0.5, cut_tint * 0.5, 0.6],
                    Look::XRay => [0.38, 0.1, cut_tint * 1.4, 0.9],
                };
                let in_problem = frame.highlight.is_some_and(|h| h.nodes.contains(&id));
                let is_selected = selected.contains(&node.item);
                // Selected wins over a problem's red: the outline and the
                // flat wash say which part is meant.
                let tint = if in_problem && !is_selected {
                    [PROBLEM[0], PROBLEM[1], PROBLEM[2], 0.18]
                } else {
                    [0.0; 4]
                };
                // An edit's ghost takes its verdict's colour.
                let tint = frame
                    .tints
                    .and_then(|tints| tints.get(id.index()))
                    .filter(|tint| tint[3] > 0.0)
                    .copied()
                    .unwrap_or(tint);
                let selected = if is_selected {
                    [SELECTED_WASH[0], SELECTED_WASH[1], SELECTED_WASH[2], 1.0]
                } else {
                    [0.0; 4]
                };
                Some(NodeData {
                    model: model.to_cols_array_2d(),
                    colour,
                    params,
                    tint,
                    style,
                    selected,
                })
            })
            .collect();
        self.queue
            .write_buffer(&self.nodes, 0, bytemuck::cast_slice(&nodes));
        light_view_projection
    }
}

/// The overlay's colours, linear: the path in the selection blue, the
/// home outline dark, the labels white on blue.
/// The path of a door's edge: a deep teal (#00756f), apart from the
/// selection and badge blue and the gap's orange, and over 4.5:1 against
/// the light stage.
const PATH_COLOUR: [f32; 4] = [0.0, 0.178, 0.155, 1.0];
const HOME_COLOUR: [f32; 4] = [0.05, 0.03, 0.02, 0.85];
const LABEL_FILL: [f32; 4] = [0.0284, 0.1590, 0.6724, 1.0];
const LABEL_INK: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// The closest approach, in a warm orange, its note dark on pale.
const GAP_COLOUR: [f32; 4] = [0.82, 0.25, 0.02, 1.0];
const NOTE_FILL: [f32; 4] = [0.98, 0.95, 0.90, 0.92];
const NOTE_INK: [f32; 4] = [0.45, 0.10, 0.0, 1.0];

/// Polylines on a glyph's grid.
type Strokes = Vec<Vec<(f32, f32)>>;

/// A character of a note: its width (a digit is 1 wide, 2 high) and its
/// strokes. Characters without a glyph are a space.
fn glyph(character: char) -> (f32, Strokes) {
    let owned = |strokes: &[&[(f32, f32)]]| strokes.iter().map(|s| s.to_vec()).collect();
    match character {
        '0'..='9' => {
            let digit = u8::try_from(u32::from(character) - u32::from('0')).unwrap_or(0);
            (1.0, owned(digit_strokes(digit)))
        }
        '.' => (0.4, owned(&[&[(0.15, 0.0), (0.15, 0.25)]])),
        'm' => (
            1.3,
            owned(&[
                &[(0.0, 0.0), (0.0, 1.3)],
                &[(0.0, 1.1), (0.3, 1.3), (0.65, 1.1), (0.65, 0.0)],
                &[(0.65, 1.1), (0.95, 1.3), (1.3, 1.1), (1.3, 0.0)],
            ]),
        ),
        'c' => (
            1.0,
            owned(&[&[(1.0, 1.3), (0.0, 1.3), (0.0, 0.0), (1.0, 0.0)]]),
        ),
        'l' => (0.6, owned(&[&[(0.3, 2.0), (0.3, 0.0)]])),
        'o' => (
            1.0,
            owned(&[&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.3), (0.0, 1.3), (0.0, 0.0)]]),
        ),
        's' => (
            1.0,
            owned(&[&[
                (1.0, 1.3),
                (0.0, 1.3),
                (0.0, 0.65),
                (1.0, 0.65),
                (1.0, 0.0),
                (0.0, 0.0),
            ]]),
        ),
        'e' => (
            1.0,
            owned(&[&[
                (0.0, 0.65),
                (1.0, 0.65),
                (1.0, 1.3),
                (0.0, 1.3),
                (0.0, 0.0),
                (1.0, 0.0),
            ]]),
        ),
        't' => (
            0.9,
            owned(&[
                &[(0.35, 2.0), (0.35, 0.0), (0.9, 0.0)],
                &[(0.0, 1.3), (0.9, 1.3)],
            ]),
        ),
        'a' => (
            1.0,
            owned(&[&[
                (0.0, 1.3),
                (1.0, 1.3),
                (1.0, 0.0),
                (0.0, 0.0),
                (0.0, 0.65),
                (1.0, 0.65),
            ]]),
        ),
        ':' => (
            0.4,
            owned(&[&[(0.15, 0.1), (0.15, 0.3)], &[(0.15, 1.0), (0.15, 1.2)]]),
        ),
        '°' => (
            0.6,
            owned(&[&[(0.0, 1.6), (0.3, 2.0), (0.6, 1.6), (0.3, 1.2), (0.0, 1.6)]]),
        ),
        _ => (0.6, Vec::new()),
    }
}

/// Strokes of a digit in a box 1 wide and 2 high, from the bottom left.
fn digit_strokes(digit: u8) -> &'static [&'static [(f32, f32)]] {
    match digit {
        0 => &[&[(0.0, 0.0), (1.0, 0.0), (1.0, 2.0), (0.0, 2.0), (0.0, 0.0)]],
        1 => &[&[(0.2, 1.6), (0.55, 2.0), (0.55, 0.0)]],
        2 => &[&[
            (0.0, 2.0),
            (1.0, 2.0),
            (1.0, 1.0),
            (0.0, 1.0),
            (0.0, 0.0),
            (1.0, 0.0),
        ]],
        3 => &[
            &[(0.0, 2.0), (1.0, 2.0), (1.0, 0.0), (0.0, 0.0)],
            &[(0.1, 1.0), (1.0, 1.0)],
        ],
        4 => &[
            &[(0.0, 2.0), (0.0, 1.0), (1.0, 1.0)],
            &[(1.0, 2.0), (1.0, 0.0)],
        ],
        5 => &[&[
            (1.0, 2.0),
            (0.0, 2.0),
            (0.0, 1.0),
            (1.0, 1.0),
            (1.0, 0.0),
            (0.0, 0.0),
        ]],
        6 => &[&[
            (1.0, 2.0),
            (0.0, 2.0),
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0, 1.0),
            (0.0, 1.0),
        ]],
        7 => &[&[(0.0, 2.0), (1.0, 2.0), (0.35, 0.0)]],
        8 => &[
            &[(0.0, 0.0), (1.0, 0.0), (1.0, 2.0), (0.0, 2.0), (0.0, 0.0)],
            &[(0.0, 1.0), (1.0, 1.0)],
        ],
        _ => &[&[
            (1.0, 1.0),
            (0.0, 1.0),
            (0.0, 2.0),
            (1.0, 2.0),
            (1.0, 0.0),
            (0.0, 0.0),
        ]],
    }
}

/// The overlay as triangles on screen: lines a constant width in pixels,
/// labels as filled discs with their numbers in white.
fn overlay_triangles(
    overlay: &Overlay,
    frame: &Frame<'_>,
    width: u32,
    height: u32,
) -> Vec<OverlayVertex> {
    let aspect = aspect(width, height);
    let view_projection = frame.orbit.view_projection(aspect);
    let (w, h) = (
        crate::convert::f32_of_u32(width),
        crate::convert::f32_of_u32(height),
    );
    // Pixels to normalised device coordinates.
    let per_pixel = glam::Vec2::new(2.0 / w.max(1.0), 2.0 / h.max(1.0));
    let project = |point: Vec3| {
        let clip = view_projection * point.extend(1.0);
        (clip.w > 1e-6).then(|| glam::Vec2::new(clip.x / clip.w, clip.y / clip.w))
    };
    let mut out = Vec::new();
    let segment = |out: &mut Vec<OverlayVertex>,
                   a: glam::Vec2,
                   b: glam::Vec2,
                   pixels: f32,
                   colour: [f32; 4]| {
        let along = ((b - a) / per_pixel).normalize_or_zero();
        let across = glam::Vec2::new(-along.y, along.x) * pixels * 0.5 * per_pixel;
        let corners = [a - across, a + across, b + across, b - across];
        for index in [0, 1, 2, 0, 2, 3] {
            if let Some(corner) = corners.get(index) {
                out.push(OverlayVertex {
                    position: corner.to_array(),
                    colour,
                });
            }
        }
    };
    for line in &overlay.lines {
        let (Some(a), Some(b)) = (project(line.from), project(line.to)) else {
            continue;
        };
        let (pixels, colour) = match line.kind {
            LineKind::Path => (2.5, PATH_COLOUR),
            LineKind::PathTravelled => (3.0, PATH_COLOUR),
            LineKind::Home => (2.0, HOME_COLOUR),
            LineKind::Gap => (2.5, GAP_COLOUR),
        };
        segment(&mut out, a, b, pixels, colour);
    }
    let radius = 13.0;
    let mut placed: Vec<glam::Vec2> = Vec::new();
    for label in &overlay.labels {
        let Some(anchor) = project(label.at) else {
            continue;
        };
        // A label that would cover another moves up, with a leader line
        // back to its point.
        let mut centre = anchor;
        let clear = (2.0 * radius + 3.0) * per_pixel.y;
        let mut tries = 0;
        while tries < 8
            && placed
                .iter()
                .any(|p| ((*p - centre) / per_pixel).length() < 2.0 * radius + 3.0)
        {
            centre.y += clear;
            tries += 1;
        }
        if tries > 0 {
            segment(&mut out, anchor, centre, 1.5, LABEL_FILL);
        }
        placed.push(centre);
        let steps = 24_u16;
        let point = |step: u16| {
            let angle = std::f32::consts::TAU * f32::from(step) / f32::from(steps);
            centre + glam::Vec2::new(angle.cos(), angle.sin()) * radius * per_pixel
        };
        for step in 0..steps {
            for corner in [centre, point(step), point(step + 1)] {
                out.push(OverlayVertex {
                    position: corner.to_array(),
                    colour: LABEL_FILL,
                });
            }
        }
        let digits: Vec<u8> = label
            .number
            .to_string()
            .bytes()
            .map(|b| b.saturating_sub(b'0'))
            .collect();
        let glyph = glam::Vec2::new(6.0, 6.0);
        let gap = 4.0;
        let count = crate::convert::f32_of_index(digits.len());
        let total = count * glyph.x + (count - 1.0).max(0.0) * gap;
        for (index, digit) in digits.iter().enumerate() {
            let left = -total * 0.5 + crate::convert::f32_of_index(index) * (glyph.x + gap);
            let place = |(x, y): (f32, f32)| {
                centre + glam::Vec2::new(left + x * glyph.x, (y - 1.0) * glyph.y) * per_pixel
            };
            for stroke in digit_strokes(*digit) {
                for pair in stroke.windows(2) {
                    if let [a, b] = pair {
                        segment(&mut out, place(*a), place(*b), 2.2, LABEL_INK);
                    }
                }
            }
        }
    }
    // Text in strokes from an origin (its bottom left on screen), with a
    // pale pill behind it when asked.
    let text = |out: &mut Vec<OverlayVertex>,
                origin: glam::Vec2,
                words: &str,
                size: f32,
                ink: [f32; 4],
                pill: bool| {
        let gap = 0.45;
        let glyphs: Vec<(f32, Strokes)> = words.chars().map(glyph).collect();
        let width: f32 = glyphs.iter().map(|(w, _)| w + gap).sum::<f32>() - gap;
        if pill {
            let pad = glam::Vec2::new(5.0, 4.0);
            let low = origin - pad * per_pixel;
            let high = origin + (glam::Vec2::new(width, 2.0) * size + pad) * per_pixel;
            for corner in [
                low,
                glam::Vec2::new(high.x, low.y),
                high,
                low,
                high,
                glam::Vec2::new(low.x, high.y),
            ] {
                out.push(OverlayVertex {
                    position: corner.to_array(),
                    colour: NOTE_FILL,
                });
            }
        }
        let mut left = 0.0;
        for (advance, strokes) in &glyphs {
            for stroke in strokes {
                for pair in stroke.windows(2) {
                    if let [a, b] = pair {
                        let place = |(x, y): (f32, f32)| {
                            origin + glam::Vec2::new(left + x, y) * size * per_pixel
                        };
                        segment(out, place(*a), place(*b), size * 0.3, ink);
                    }
                }
            }
            left += advance + gap;
        }
    };
    for note in &overlay.notes {
        let Some(at) = project(note.at) else {
            continue;
        };
        // Above the point, ending at it, so it covers neither the moving
        // part nor the path beside it.
        let size = 6.5;
        let width = note.text.chars().map(|c| glyph(c).0 + 0.45).sum::<f32>() * size;
        let origin = at + glam::Vec2::new(-width - 6.0, 10.0) * per_pixel;
        text(&mut out, origin, &note.text, size, NOTE_INK, true);
    }
    let marker_points: Vec<Option<glam::Vec2>> =
        overlay.markers.iter().map(|m| project(m.at)).collect();
    for (index, marker) in overlay.markers.iter().enumerate() {
        let Some(Some(at)) = marker_points.get(index).copied() else {
            continue;
        };
        // The caption sits beyond the dot, away from the path's other end.
        let others: Vec<glam::Vec2> = marker_points
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .filter_map(|(_, p)| *p)
            .collect();
        let away = others.first().map_or(glam::Vec2::new(-1.0, -1.0), |other| {
            ((at - *other) / per_pixel).normalize_or(glam::Vec2::new(-1.0, -1.0))
        });
        let steps = 12_u16;
        let point = |step: u16| {
            let angle = std::f32::consts::TAU * f32::from(step) / f32::from(steps);
            at + glam::Vec2::new(angle.cos(), angle.sin()) * 4.0 * per_pixel
        };
        for step in 0..steps {
            for corner in [at, point(step), point(step + 1)] {
                out.push(OverlayVertex {
                    position: corner.to_array(),
                    colour: PATH_COLOUR,
                });
            }
        }
        let size = 5.5;
        let width = marker.text.chars().map(|c| glyph(c).0 + 0.45).sum::<f32>() * size;
        let height = 2.0 * size;
        let centre = at + away * 22.0 * per_pixel;
        let origin = centre - glam::Vec2::new(width, height) * 0.5 * per_pixel;
        text(&mut out, origin, &marker.text, size, PATH_COLOUR, false);
    }
    out
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

/// A view's width over its height, for projections. Sizes beyond 65535
/// pixels (never a view's) count as 65535.
pub fn aspect(width: u32, height: u32) -> f32 {
    let pixels = |n: u32| f32::from(u16::try_from(n.max(1)).unwrap_or(u16::MAX));
    pixels(width) / pixels(height)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// A renderer, or `None` (and a note) on a machine without a GPU
    /// adapter wgpu can use.
    fn renderer() -> Option<Renderer> {
        match Renderer::new() {
            Ok(renderer) => Some(renderer),
            Err(error) => {
                eprintln!("skipped: {error:#}");
                None
            }
        }
    }

    /// The stool's scene, at rest.
    fn stool() -> (SceneGraph, Posed) {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stool");
        let opened = stave_app::open(&folder).unwrap();
        let scene = stave_app::view::scene(opened.design.as_ref().unwrap());
        let posed = scene.rest();
        (scene, posed)
    }

    fn frame<'a>(scene: &'a SceneGraph, posed: &'a Posed, yaw: f32) -> Frame<'a> {
        let bounds = scene.bounds(posed).unwrap();
        let orbit = Orbit::framing(bounds, aspect(96, 64));
        Frame {
            scene,
            posed,
            orbit: Orbit {
                yaw: orbit.yaw + yaw,
                ..orbit
            },
            selection: &[],
            highlight: None,
            section: None,
            looks: None,
            overlay: None,
            tint_cut_faces: true,
            lit: None,
            tints: None,
            backdrop: Backdrop::Light,
            width: 96,
            height: 64,
        }
    }

    /// Waits for the GPU, then takes the newest frame that has arrived.
    fn wait_and_take(renderer: &mut Renderer) -> Option<LiveFrame> {
        renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        renderer.take_ready().unwrap()
    }

    /// A live frame's rows without their padding.
    fn unpadded(frame: &LiveFrame) -> Vec<u8> {
        let row = usize::try_from(frame.width()).unwrap() * 4;
        let stride = usize::try_from(frame.stride()).unwrap();
        frame
            .as_ref()
            .chunks(stride)
            .flat_map(|line| &line[..row])
            .copied()
            .collect()
    }

    #[test]
    fn the_live_view_shows_the_frame_a_screenshot_does_and_it_is_opaque() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        let still = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        assert!(still.rgba.chunks(4).all(|pixel| pixel[3] == 255));
        let ticket = renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        let live = wait_and_take(&mut renderer).unwrap();
        assert!(renderer.settled(ticket));
        assert_eq!((live.width(), live.height()), (96, 64));
        // 96 * 4 bytes, padded to wgpu's 256-byte alignment: GTK is given
        // the padded rows, not a copy without them.
        assert_eq!(live.stride(), 512);
        assert_eq!(unpadded(&live), still.rgba);
    }

    #[test]
    fn a_newer_frame_is_shown_instead_of_an_older_one_not_yet_taken() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        let first = renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        let second = renderer.submit(&frame(&scene, &posed, 0.7)).unwrap();
        assert!(first < second);
        let shown = wait_and_take(&mut renderer).unwrap();
        assert!(renderer.settled(first) && renderer.settled(second));
        let turned = renderer.render(&frame(&scene, &posed, 0.7)).unwrap();
        assert_eq!(unpadded(&shown), turned.rgba);
        // Nothing more to take.
        assert!(renderer.take_ready().unwrap().is_none());
    }

    #[test]
    fn a_frame_s_buffer_is_used_again_once_gtk_lets_it_go() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        for _ in 0..2 {
            renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
            let shown = wait_and_take(&mut renderer).unwrap();
            drop(shown);
        }
        renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        assert_eq!(renderer.ring.made(), 1);
        // Held, the frame's buffer isn't reused: another is made.
        renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        let held = wait_and_take(&mut renderer).unwrap();
        renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        assert_eq!(renderer.ring.made(), 2);
        drop(held);
    }

    #[test]
    fn frames_in_flight_when_the_device_is_lost_count_as_done() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        let before = renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        renderer.device.destroy();
        // wgpu may say the device is lost at once or only after a frame on
        // it fails; either way a frame soon arrives from a new one, and
        // nothing is left waiting on the old.
        let mut shown = None;
        for _ in 0..3 {
            let Ok(after) = renderer.submit(&frame(&scene, &posed, 0.0)) else {
                continue;
            };
            assert!(before < after);
            let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
            if let Ok(Some(frame)) = renderer.take_ready() {
                assert!(renderer.settled(after));
                shown = Some(frame);
                break;
            }
        }
        assert!(shown.is_some());
        assert!(renderer.settled(before));
        assert!(!renderer.is_lost());
    }

    #[test]
    fn a_lost_device_is_opened_again_and_the_frame_drawn() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        let before = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        renderer.device.destroy();
        let after = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        assert_eq!(before.rgba.len(), after.rgba.len());
        assert!(!renderer.is_lost());
    }

    #[test]
    fn the_shadow_map_is_kept_while_only_the_camera_moves() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        let first = renderer.shadow_key.as_ref().map(|key| key.casters.len());
        assert!(first.is_some_and(|casters| casters > 0));
        // Turned: the same map, kept, and the picture as with a map drawn
        // afresh.
        let turned = frame(&scene, &posed, 0.7);
        let generation = renderer.meshes_generation;
        let kept = renderer.render(&turned).unwrap();
        assert_eq!(renderer.meshes_generation, generation);
        renderer.shadow_key = None;
        let fresh = renderer.render(&turned).unwrap();
        assert_eq!(kept, fresh);
        // A part moved: the key differs, so the map is drawn again.
        let caster = renderer
            .shadow_key
            .as_ref()
            .and_then(|key| key.casters.first())
            .map(|(index, _)| usize::try_from(*index).unwrap())
            .unwrap();
        let mut moved = posed.clone();
        if let Some(transform) = moved.transforms.get_mut(caster) {
            *transform = Mat4::from_translation(Vec3::new(0.0, 50.0, 0.0)) * *transform;
        }
        let before = renderer.shadow_key.take();
        renderer.render(&frame(&scene, &moved, 0.7)).unwrap();
        assert_ne!(renderer.shadow_key, before);
        // A new upload draws it again too, even with nothing moved.
        renderer.upload(&scene);
        assert_ne!(
            renderer.shadow_key.as_ref().map(|key| key.meshes),
            Some(renderer.meshes_generation)
        );
    }
}

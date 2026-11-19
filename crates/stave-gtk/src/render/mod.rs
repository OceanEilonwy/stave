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
//!
//! Where the GPU and the display can share memory (Vulkan with dmabuf
//! export, and a GTK that imports the layout), the live view skips the
//! readback altogether: its frames are copied on the GPU into dmabufs GTK
//! reads itself ([`dmabuf`], enabled by [`Renderer::use_dmabufs`]). Any
//! failure there puts the live view back on the readback path.

// The one module allowed `unsafe` code; see its own documentation.
#[allow(unsafe_code)]
mod dmabuf;
mod meshes;
pub mod profile;
mod readback;

#[cfg(test)]
use dmabuf::Frame as DmabufFrame;
pub use dmabuf::display_modifiers;
use meshes::MeshRange;
pub use readback::{LiveFrame, MappedFrame, Ticket};

use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use glam::{Mat4, Vec3};
use stave_scene::{
    Aabb, Highlight, LineKind, Look, Material, NodeId, Orbit, Overlay, Posed, SceneGraph, Section,
};

const COLOUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The selection mask: red where the selection is seen, green wherever it
/// covers.
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;
const SAMPLES: u32 = 4;
const SHADOW_SIZE: u32 = 2048;
/// Render targets are made in steps of this many pixels each way, and a
/// frame is drawn in their top-left corner, so a window resized a pixel at
/// a time doesn't make new ones at every step.
const TARGET_STEP: u32 = 256;

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
    /// xy: the frame's size in pixels, the corner of the target drawn in.
    screen: [f32; 4],
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

/// The textures a frame is drawn in. They may be larger than the frame,
/// which is drawn in their top-left corner.
struct Target {
    /// Pixels across it has room for.
    width: u32,
    /// Pixels down it has room for.
    height: u32,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// Made the first time something is selected.
    mask: Option<Mask>,
    resolved: wgpu::Texture,
    resolved_view: wgpu::TextureView,
}

/// The selection mask, multisampled; its resolved copy is bound for the
/// outline pass by `group`.
struct Mask {
    msaa: wgpu::TextureView,
    resolved: wgpu::TextureView,
    group: wgpu::BindGroup,
}

impl Mask {
    /// A mask for a target `size` large.
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: wgpu::Extent3d) -> Mask {
        let texture = |label, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: MASK_FORMAT,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let msaa = texture(
            "selection mask",
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let resolved = texture(
            "selection mask, resolved",
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("selection mask"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&resolved),
            }],
        });
        Mask {
            msaa,
            resolved,
            group,
        }
    }
}

/// Which target a frame is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// The live view's.
    Live,
    /// Still pictures'.
    Still,
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
    /// Every node's mesh, in shared buffers.
    meshes: meshes::MeshStore,
    /// Counts uploads that changed a mesh, so a shadow map of older meshes
    /// is never kept.
    meshes_generation: u64,
    /// What the shadow map holds; `None` before the first frame.
    shadow_key: Option<ShadowKey>,
    /// Where the live view is drawn ([`Renderer::submit`]).
    live_target: Option<Target>,
    /// Where still pictures are drawn ([`Renderer::render`]): the small
    /// pictures beside the view and screenshots, so they never make the
    /// live view's targets anew.
    still_target: Option<Target>,
    /// How many targets were made, for tests.
    targets_made: u64,
    /// The readback buffers and the frames in them.
    ring: readback::Ring,
    /// The DRM format modifiers GTK imports dmabufs of [`dmabuf::FOURCC`]
    /// with, as [`Renderer::use_dmabufs`] was given them (kept to make the
    /// exporter again on a new device).
    dmabuf_accepted: Vec<u64>,
    /// Makes the dmabufs the live view's frames go to GTK in; `None` when
    /// they go by readback.
    dmabuf: Option<dmabuf::Exporter>,
    /// The box around every node at rest (where the floor is), from the
    /// last upload.
    rest_bounds: Option<Aabb>,
    /// What the node buffer holds, so a frame that would write the same
    /// again (only the camera moved) doesn't.
    nodes_written: Vec<NodeData>,
    /// Lists for working out a frame.
    scratch: Scratch,
    /// The vertices drawn on top: a problem's shared volumes, the lit
    /// spaces, and the overlay.
    volume_faces: Vertices,
    volume_lines: Vertices,
    lit_faces: Vertices,
    lit_lines: Vertices,
    overlay_vertices: Vertices,
    /// How many times the node buffer was written, for tests.
    node_writes: u64,
    /// For tests: draw every pass as if all of it were needed (the
    /// multisampled scene kept, the "on top" pass run, the selection
    /// outline over the whole frame), to compare with the shortcuts.
    #[cfg(test)]
    full_passes: bool,
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
        // Asked for when there, for the live view's dmabufs; nothing needs
        // it otherwise.
        let required_features = if dmabuf::wanted() {
            adapter.features() & wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("stave"),
            required_features,
            // The meshes share a few buffers, which a large design can
            // make bigger than the downlevel 256 MB: ask for what the
            // adapter has.
            required_limits: wgpu::Limits {
                max_buffer_size: adapter.limits().max_buffer_size,
                ..wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits())
            },
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
        let meshes = meshes::MeshStore::new(&device);
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
            meshes,
            meshes_generation: 0,
            shadow_key: None,
            live_target: None,
            still_target: None,
            targets_made: 0,
            ring: readback::Ring::default(),
            dmabuf_accepted: Vec::new(),
            dmabuf: None,
            rest_bounds: None,
            nodes_written: Vec::new(),
            scratch: Scratch::default(),
            volume_faces: Vertices::new("volume faces"),
            volume_lines: Vertices::new("volume outlines"),
            lit_faces: Vertices::new("lit space faces"),
            lit_lines: Vertices::new("lit space outlines"),
            overlay_vertices: Vertices::new("overlay"),
            node_writes: 0,
            #[cfg(test)]
            full_passes: false,
        })
    }

    /// The adapter's name, for logs.
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Sends the live view's frames to GTK as dmabufs from now on, laid
    /// out with one of `accepted` (the DRM format modifiers the display
    /// imports [`dmabuf::FOURCC`] with). An error says why they keep going
    /// by readback: the GPU or its driver can't export them, none of those
    /// layouts suits it, or `STAVE_DMABUF=0` turned them off.
    pub fn use_dmabufs(&mut self, accepted: Vec<u64>) -> Result<()> {
        self.dmabuf_accepted = accepted;
        self.dmabuf = None;
        if !dmabuf::wanted() {
            bail!("turned off by STAVE_DMABUF=0");
        }
        self.dmabuf = Some(dmabuf::Exporter::new(&self.device, &self.dmabuf_accepted)?);
        Ok(())
    }

    /// Whether the live view's frames go to GTK as dmabufs.
    #[cfg(test)]
    pub fn uses_dmabufs(&self) -> bool {
        self.dmabuf.as_ref().is_some_and(dmabuf::Exporter::usable)
    }

    /// Uploads a scene's meshes, replacing the previous scene's. Only
    /// meshes the GPU doesn't hold already are written, and nothing is
    /// when every node's mesh is one it holds.
    pub fn upload(&mut self, scene: &SceneGraph) {
        let started = profile::start();
        self.upload_meshes(scene);
        profile::finish(&format!("upload of {} meshes", scene.nodes.len()), started);
    }

    fn upload_meshes(&mut self, scene: &SceneGraph) {
        if self.meshes.upload(&self.device, &self.queue, &scene.nodes) {
            self.meshes_generation = self.meshes_generation.wrapping_add(1);
        }
        let needed = u64::try_from(scene.nodes.len()).unwrap_or(u64::MAX);
        if needed > self.node_capacity {
            self.node_capacity = needed.next_power_of_two();
            self.nodes = node_buffer(&self.device, self.node_capacity);
            self.nodes_written.clear();
            self.scene_group =
                scene_group(&self.device, &self.scene_layout, &self.globals, &self.nodes);
        }
        self.rest_bounds = scene
            .nodes
            .iter()
            .map(|node| node.geometry.bounds)
            .reduce(Aabb::union);
    }

    /// Makes sure `slot`'s target has room for a `width` by `height`
    /// frame: one is made only when the frame doesn't fit, or is much
    /// smaller than the target, and then in steps of [`TARGET_STEP`].
    fn ensure_target(&mut self, slot: Slot, width: u32, height: u32) {
        let limit = self.device.limits().max_texture_dimension_2d;
        let step = |pixels: u32| {
            pixels
                .checked_next_multiple_of(TARGET_STEP)
                .unwrap_or(limit)
                .min(limit)
                .max(pixels)
        };
        let (width, height) = (step(width), step(height));
        let area = |w: u32, h: u32| u64::from(w) * u64::from(h);
        let current = match slot {
            Slot::Live => &self.live_target,
            Slot::Still => &self.still_target,
        };
        let fits = current.as_ref().is_some_and(|t| {
            t.width >= width
                && t.height >= height
                && area(width, height).saturating_mul(4) >= area(t.width, t.height)
        });
        if !fits {
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
            let target = Some(Target {
                width,
                height,
                msaa,
                depth,
                mask: None,
                resolved,
                resolved_view,
            });
            match slot {
                Slot::Live => self.live_target = target,
                Slot::Still => self.still_target = target,
            }
            self.targets_made += 1;
        }
    }

    /// Makes sure `slot`'s target has a selection mask.
    fn ensure_mask(&mut self, slot: Slot) {
        let target = match slot {
            Slot::Live => self.live_target.as_mut(),
            Slot::Still => self.still_target.as_mut(),
        };
        if let Some(target) = target
            && target.mask.is_none()
        {
            let size = wgpu::Extent3d {
                width: target.width,
                height: target.height,
                depth_or_array_layers: 1,
            };
            target.mask = Some(Mask::new(&self.device, &self.mask_layout, size));
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
        let drawn = self.draw_checked(frame, Slot::Still)?;
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
        let drawn = self.draw_checked(frame, Slot::Live)?;
        Ok(self.ring.push(&self.queue, drawn))
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
    fn draw_checked(&mut self, frame: &Frame<'_>, slot: Slot) -> Result<readback::Drawn> {
        self.recover_if_lost(frame.scene)?;
        let drawn = self.draw(frame, slot);
        // A device lost while drawing, or before (it may only say so when
        // polled): once more, on a new one. Frames still on the old one
        // count as done.
        let _ = self.device.poll(wgpu::PollType::Poll);
        let drawn = match drawn {
            _ if self.is_lost() => {
                self.recover_if_lost(frame.scene)?;
                self.draw(frame, slot)
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
            let accepted = std::mem::take(&mut self.dmabuf_accepted);
            let had_dmabufs = self.dmabuf.is_some();
            *self = Renderer::new()
                .context("the graphics device was lost and couldn't be opened again")?;
            self.ring = ring;
            if had_dmabufs && let Err(error) = self.use_dmabufs(accepted) {
                eprintln!("stave: the 3D view's frames go by readback now: {error:#}");
            }
            self.upload(scene);
        }
        Ok(())
    }

    fn draw(&mut self, frame: &Frame<'_>, slot: Slot) -> Result<readback::Drawn> {
        // The lists are taken out while the frame is worked out, and put
        // back for the next, whatever happens.
        let mut scratch = std::mem::take(&mut self.scratch);
        let drawn = self.draw_with(frame, slot, &mut scratch);
        self.scratch = scratch;
        drawn
    }

    fn draw_with(
        &mut self,
        frame: &Frame<'_>,
        slot: Slot,
        scratch: &mut Scratch,
    ) -> Result<readback::Drawn> {
        let (width, height) = (frame.width.max(1), frame.height.max(1));
        let limit = self.device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            bail!("a {width}x{height} view is larger than the GPU allows ({limit})");
        }
        if let Some(failed) = self.meshes.failed() {
            bail!("couldn't upload the design's meshes: {failed}");
        }
        if self.meshes.ranges().len() != frame.scene.nodes.len() {
            bail!("the scene changed without being uploaded");
        }
        let started = profile::start();
        scratch.mark(frame);
        let (light, view_projection) = self.write_uniforms(frame, width, height, scratch);
        self.ensure_target(slot, width, height);
        scratch.sort(frame, self.meshes.ranges());
        // Where the selection's outline can fall: around the selection's
        // box on screen. `None`: nothing selected is on screen.
        let outline_rect = scratch
            .outlined
            .iter()
            .filter_map(|(index, _)| {
                let index = usize::try_from(*index).ok()?;
                let node = frame.scene.nodes.get(index)?;
                let transform = frame.posed.transforms.get(index)?;
                Some(node.geometry.bounds.transformed(*transform))
            })
            .reduce(Aabb::union)
            .and_then(|bounds| screen_rect(view_projection, bounds, width, height));
        #[cfg(test)]
        let outline_rect = outline_rect.map(|rect| {
            if self.full_passes {
                (0, 0, width, height)
            } else {
                rect
            }
        });
        if outline_rect.is_some() {
            self.ensure_mask(slot);
        }
        let layout = readback::Layout::new(width, height);
        let output = self.output(slot, layout);
        let volumes = match frame.highlight.filter(|h| !h.volumes.is_empty()) {
            Some(highlight) => {
                box_vertices(highlight.volumes, &mut scratch.faces, &mut scratch.lines);
                self.volume_faces
                    .write(&self.device, &self.queue, &scratch.faces)
                    .zip(
                        self.volume_lines
                            .write(&self.device, &self.queue, &scratch.lines),
                    )
            }
            None => None,
        };
        let lit = match frame.lit.filter(|boxes| !boxes.is_empty()) {
            Some(boxes) => {
                box_vertices(boxes, &mut scratch.faces, &mut scratch.lines);
                self.lit_faces
                    .write(&self.device, &self.queue, &scratch.faces)
                    .zip(
                        self.lit_lines
                            .write(&self.device, &self.queue, &scratch.lines),
                    )
            }
            None => None,
        };
        let overlay = match frame.overlay {
            Some(overlay) => {
                overlay_triangles(overlay, frame, width, height, &mut scratch.overlay);
                self.overlay_vertices
                    .write(&self.device, &self.queue, &scratch.overlay)
            }
            None => None,
        };
        // Ghosts cast no shadow; see-through parts do.
        let casting = scratch
            .solid
            .iter()
            .map(|(_, index, mesh)| (*index, *mesh))
            .chain(
                scratch
                    .translucent
                    .iter()
                    .filter(|(_, look, _, _)| *look == Look::XRay)
                    .map(|(_, _, index, mesh)| (*index, *mesh)),
            );
        scratch.casters.clear();
        scratch.casters.extend(casting.clone().map(|(index, _)| {
            let model = usize::try_from(index)
                .ok()
                .and_then(|index| frame.posed.transforms.get(index))
                .copied()
                .unwrap_or(Mat4::IDENTITY);
            (index, model.to_cols_array_2d())
        }));
        scratch.casters.sort_unstable_by_key(|(index, _)| *index);
        let light = light.to_cols_array_2d();
        // Only the camera moved (an orbit, a pan, a zoom): the shadow map
        // drawn before is still right.
        let shadow_drawn = !self.shadow_key.as_ref().is_some_and(|key| {
            key.meshes == self.meshes_generation
                && key.light == light
                && key.casters == scratch.casters
        });
        let target = match slot {
            Slot::Live => &self.live_target,
            Slot::Still => &self.still_target,
        }
        .as_ref()
        .context("no render target")?;
        // Every pass draws in the target's top-left corner.
        let corner = |pass: &mut wgpu::RenderPass<'_>| {
            pass.set_viewport(
                0.0,
                0.0,
                crate::convert::f32_of_u32(width),
                crate::convert::f32_of_u32(height),
                0.0,
                1.0,
            );
            pass.set_scissor_rect(0, 0, width, height);
        };
        let mask = outline_rect.and_then(|rect| Some((rect, target.mask.as_ref()?)));
        // Anything drawn over the resolved scene. Without it, the view pass
        // resolves straight into the frame and nothing multisampled is kept.
        let on_top = mask.is_some() || lit.is_some() || volumes.is_some() || overlay.is_some();
        #[cfg(test)]
        let on_top = on_top || self.full_passes;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
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
            self.bind_meshes(&mut pass);
            for (index, mesh) in casting {
                draw_mesh(&mut pass, index, mesh);
            }
        }
        {
            let keep = |kept: bool| {
                if kept {
                    wgpu::StoreOp::Store
                } else {
                    wgpu::StoreOp::Discard
                }
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    depth_slice: None,
                    resolve_target: (!on_top).then_some(&target.resolved_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(frame.backdrop.colour()),
                        store: keep(on_top),
                    },
                })],
                // Only the selection mask tests against the scene's depth.
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: keep(mask.is_some()),
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            corner(&mut pass);
            pass.set_bind_group(0, &self.scene_group, &[]);
            pass.set_bind_group(1, &self.shadow_group, &[]);
            // In a section, every mesh twice: its cut faces, then its
            // faces, which lose to a cut face lying on them.
            let pipelines: &[&wgpu::RenderPipeline] = if frame.section.is_some() {
                &[&self.section_cut_pipeline, &self.section_faces_pipeline]
            } else {
                &[&self.mesh_pipeline]
            };
            self.bind_meshes(&mut pass);
            for pipeline in pipelines {
                pass.set_pipeline(pipeline);
                for (_, index, mesh) in &scratch.solid {
                    draw_mesh(&mut pass, *index, *mesh);
                }
            }
            pass.set_pipeline(&self.floor_pipeline);
            pass.draw(0..6, 0..1);
            pass.set_pipeline(&self.edge_pipeline);
            pass.set_vertex_buffer(0, self.meshes.edge_buffer().slice(..));
            for (_, index, mesh) in &scratch.solid {
                draw_edges(&mut pass, *index, *mesh);
            }
            if !scratch.translucent.is_empty() {
                pass.set_pipeline(if frame.section.is_some() {
                    &self.translucent_section_pipeline
                } else {
                    &self.translucent_pipeline
                });
                self.bind_meshes(&mut pass);
                for (_, _, index, mesh) in &scratch.translucent {
                    draw_mesh(&mut pass, *index, *mesh);
                }
                pass.set_pipeline(&self.edge_pipeline);
                pass.set_vertex_buffer(0, self.meshes.edge_buffer().slice(..));
                for (_, _, index, mesh) in &scratch.translucent {
                    draw_edges(&mut pass, *index, *mesh);
                }
            }
        }
        if let Some((_, mask)) = mask {
            // The selection mask, tested against the depth just drawn.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("selection mask"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &mask.msaa,
                    depth_slice: None,
                    resolve_target: Some(&mask.resolved),
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
            corner(&mut pass);
            pass.set_bind_group(0, &self.scene_group, &[]);
            pass.set_bind_group(1, &self.shadow_group, &[]);
            self.bind_meshes(&mut pass);
            for pipeline in [&self.mask_covered_pipeline, &self.mask_seen_pipeline] {
                pass.set_pipeline(pipeline);
                for (index, mesh) in &scratch.outlined {
                    draw_mesh(&mut pass, *index, *mesh);
                }
            }
        }
        if on_top {
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
            corner(&mut pass);
            pass.set_bind_group(0, &self.scene_group, &[]);
            if let Some(((x, y, rect_width, rect_height), mask)) = mask {
                // Only around the selection: everywhere else the band is
                // empty, and every pixel of it reads the mask 29 times.
                pass.set_scissor_rect(x, y, rect_width, rect_height);
                pass.set_pipeline(&self.outline_pipeline);
                pass.set_bind_group(1, &mask.group, &[]);
                pass.draw(0..3, 0..1);
                pass.set_scissor_rect(0, 0, width, height);
            }
            pass.set_bind_group(1, &self.shadow_group, &[]);
            if let Some((faces, lines)) = &lit {
                pass.set_pipeline(&self.lit_pipeline);
                faces.draw(&mut pass);
                pass.set_pipeline(&self.lit_edge_pipeline);
                lines.draw(&mut pass);
            }
            if let Some((faces, lines)) = &volumes {
                pass.set_pipeline(&self.volume_pipeline);
                faces.draw(&mut pass);
                pass.set_pipeline(&self.volume_edge_pipeline);
                lines.draw(&mut pass);
            }
            if let Some(overlay) = &overlay {
                pass.set_pipeline(&self.overlay_pipeline);
                overlay.draw(&mut pass);
            }
        }
        let resolved = wgpu::TexelCopyTextureInfo {
            texture: &target.resolved,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        };
        let copied = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        match &output {
            readback::Output::Buffer(buffer) => encoder.copy_texture_to_buffer(
                resolved,
                wgpu::TexelCopyBufferInfo {
                    buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(layout.padded_row),
                        rows_per_image: Some(height),
                    },
                },
                copied,
            ),
            // The same bytes: the dmabuf's format is the target's without
            // the sRGB view, and copies don't convert.
            readback::Output::Dmabuf(frame) => {
                if let Some(texture) = frame.texture() {
                    encoder.copy_texture_to_texture(resolved, texture.as_image_copy(), copied);
                }
            }
        }
        let encoded = started.map(|_| Instant::now());
        let submission = self.queue.submit([encoder.finish()]);
        if shadow_drawn {
            let key = ShadowKey {
                meshes: self.meshes_generation,
                light,
                casters: std::mem::take(&mut scratch.casters),
            };
            // The old key's list is used again for the next frame's.
            if let Some(old) = self.shadow_key.replace(key) {
                scratch.casters = old.casters;
            }
        }
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
                dmabuf: matches!(output, readback::Output::Dmabuf(_)),
            }),
            _ => None,
        };
        Ok(readback::Drawn {
            output,
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
            output,
            layout,
            submission,
            submitted,
            timings,
        } = drawn;
        let buffer = match output {
            readback::Output::Buffer(buffer) => buffer,
            // Still pictures are never drawn into dmabufs (see `output`).
            readback::Output::Dmabuf(_) => bail!("a still picture went to a dmabuf"),
        };
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

    /// Where a frame of `layout` drawn into `slot` is copied for the
    /// window: a dmabuf for the live view when GTK takes them, otherwise
    /// a readback buffer. A dmabuf that can't be made puts the live view
    /// on readback for good.
    fn output(&mut self, slot: Slot, layout: readback::Layout) -> readback::Output {
        if let (Slot::Live, Some(exporter)) = (slot, &mut self.dmabuf) {
            if exporter.usable() {
                match exporter.frame(&self.device, layout.width, layout.height) {
                    Ok(frame) => return readback::Output::Dmabuf(frame),
                    Err(error) => {
                        eprintln!("stave: the 3D view's frames go by readback now: {error:#}");
                        self.dmabuf = None;
                    }
                }
            } else {
                self.dmabuf = None;
            }
        }
        readback::Output::Buffer(self.ring.buffer(&self.device, layout))
    }

    /// Binds the shared vertex and index buffers, for [`draw_mesh`].
    fn bind_meshes(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.meshes.vertex_buffer().slice(..));
        pass.set_index_buffer(
            self.meshes.index_buffer().slice(..),
            wgpu::IndexFormat::Uint32,
        );
    }

    /// Writes the globals, and every node's data when it differs from what
    /// was written last (a frame where only the camera moved writes only
    /// the globals); returns the light's view and projection, which decides
    /// the shadow map, and the camera's.
    fn write_uniforms(
        &mut self,
        frame: &Frame<'_>,
        width: u32,
        height: u32,
        scratch: &mut Scratch,
    ) -> (Mat4, Mat4) {
        let aspect = crate::convert::f32_of_u32(width) / crate::convert::f32_of_u32(height);
        let scene = frame.scene;
        let bounds = scene.bounds(frame.posed).or(self.rest_bounds);
        let (centre, radius, floor_y) = bounds.map_or((Vec3::ZERO, 1000.0, 0.0), |b| {
            (b.centre(), (b.size().length() * 0.5).max(10.0), b.min.y)
        });
        // The floor is where the lowest part stands at rest.
        let floor_y = self.rest_bounds.map_or(floor_y, |b| b.min.y);
        let key = KEY_TOWARD.normalize();
        let fill = FILL_TOWARD.normalize();
        // The shadow camera covers the design and the floor around it.
        let reach = radius * 2.6;
        let light_view = Mat4::look_at_rh(centre + key * reach * 2.0, centre, Vec3::Y);
        let light_projection =
            Mat4::orthographic_rh(-reach, reach, -reach, reach, 0.0, reach * 4.0);
        let light_view_projection = light_projection * light_view;
        let view_projection = frame.orbit.view_projection(aspect);
        let globals = Globals {
            view_projection: view_projection.to_cols_array_2d(),
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
            screen: [
                crate::convert::f32_of_u32(width),
                crate::convert::f32_of_u32(height),
                0.0,
                0.0,
            ],
        };
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let cut_tint = if frame.tint_cut_faces { 1.0 } else { 0.0 };
        scratch.nodes.clear();
        scratch
            .nodes
            .extend(scene.nodes.iter().enumerate().map(|(index, node)| {
                let model = frame
                    .posed
                    .transforms
                    .get(index)
                    .copied()
                    .unwrap_or(Mat4::IDENTITY);
                let seed = crate::convert::f32_of_index(index % 97) * 1.618;
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
                let style = match frame
                    .looks
                    .and_then(|looks| looks.get(index).copied())
                    .unwrap_or(Look::Normal)
                {
                    Look::Normal => [1.0, 0.0, cut_tint, 1.0],
                    Look::Ghost => [0.16, 0.85, 0.0, 0.18],
                    Look::Dim => [0.55, 0.5, cut_tint * 0.5, 0.6],
                    Look::XRay => [0.38, 0.1, cut_tint * 1.4, 0.9],
                };
                let in_problem = scratch.in_problem.get(index).copied().unwrap_or(false);
                let is_selected = scratch.selected.get(index).copied().unwrap_or(false);
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
                    .and_then(|tints| tints.get(index))
                    .filter(|tint| tint[3] > 0.0)
                    .copied()
                    .unwrap_or(tint);
                let selected = if is_selected {
                    [SELECTED_WASH[0], SELECTED_WASH[1], SELECTED_WASH[2], 1.0]
                } else {
                    [0.0; 4]
                };
                NodeData {
                    model: model.to_cols_array_2d(),
                    colour,
                    params,
                    tint,
                    style,
                    selected,
                }
            }));
        let written: &[u8] = bytemuck::cast_slice(&self.nodes_written);
        if written != bytemuck::cast_slice::<NodeData, u8>(&scratch.nodes) {
            self.queue
                .write_buffer(&self.nodes, 0, bytemuck::cast_slice(&scratch.nodes));
            self.node_writes += 1;
            // What was written is kept; the old list is used again next
            // frame.
            std::mem::swap(&mut self.nodes_written, &mut scratch.nodes);
        }
        (light_view_projection, view_projection)
    }
}

/// Lists a frame is worked out in, kept from frame to frame so that once
/// they have grown, drawing a frame allocates nothing on the CPU.
#[derive(Default)]
struct Scratch {
    /// Solid parts nearest first: distance from the eye, node index, mesh.
    solid: Vec<(f32, u32, MeshRange)>,
    /// Ghosts and see-through parts, farthest first: distance from the
    /// eye, look, node index, mesh.
    translucent: Vec<(f32, Look, u32, MeshRange)>,
    /// Shown nodes of a selected item: node index, mesh.
    outlined: Vec<(u32, MeshRange)>,
    /// The selected items, sorted, without repeats.
    selected_items: Vec<stave_scene::Item>,
    /// By node index: whether the node's item is selected.
    selected: Vec<bool>,
    /// By node index: whether the node is in the problem shown.
    in_problem: Vec<bool>,
    /// By node index: what the shaders are given.
    nodes: Vec<NodeData>,
    /// The shadow casters: node index and where it is, in index order.
    casters: Vec<(u32, [[f32; 4]; 4])>,
    /// The overlay's triangles.
    overlay: Vec<OverlayVertex>,
    /// Boxes' faces (triangles) and edges (lines).
    faces: Vec<[f32; 3]>,
    lines: Vec<[f32; 3]>,
}

impl Scratch {
    /// Marks each node selected (every node of a selected item: a hinge
    /// is two) and in the problem shown, by index.
    fn mark(&mut self, frame: &Frame<'_>) {
        let scene = frame.scene;
        self.selected_items.clear();
        self.selected_items.extend(
            frame
                .selection
                .iter()
                .filter_map(|id| scene.node(*id))
                .map(|node| node.item),
        );
        self.selected_items.sort_unstable();
        self.selected_items.dedup();
        self.selected.clear();
        self.selected.extend(
            scene
                .nodes
                .iter()
                .map(|node| self.selected_items.binary_search(&node.item).is_ok()),
        );
        self.in_problem.clear();
        self.in_problem.resize(scene.nodes.len(), false);
        if let Some(highlight) = frame.highlight {
            for id in highlight.nodes {
                if let Some(slot) = self.in_problem.get_mut(id.index()) {
                    *slot = true;
                }
            }
        }
    }

    /// Sorts the shown nodes into solid, see-through and outlined, in the
    /// order they are drawn.
    fn sort(&mut self, frame: &Frame<'_>, ranges: &[MeshRange]) {
        self.solid.clear();
        self.translucent.clear();
        self.outlined.clear();
        let eye = frame.orbit.eye();
        for ((id, mesh), node) in frame
            .scene
            .node_ids()
            .zip(ranges.iter().copied())
            .zip(&frame.scene.nodes)
        {
            let (Some(transform), Ok(index)) = (frame.posed.shown(id), u32::try_from(id.index()))
            else {
                continue;
            };
            // How far the node's centre is from the eye, where it is now.
            let distance = node
                .geometry
                .bounds
                .transformed(transform)
                .centre()
                .distance(eye);
            let look = frame
                .looks
                .and_then(|looks| looks.get(id.index()).copied())
                .unwrap_or(Look::Normal);
            match look {
                Look::Normal => self.solid.push((distance, index, mesh)),
                Look::Ghost | Look::Dim | Look::XRay => {
                    self.translucent.push((distance, look, index, mesh));
                }
            }
            // Every shown node of a selected item is outlined, solid or
            // see-through.
            if self.selected.get(id.index()).copied().unwrap_or(false) {
                self.outlined.push((index, mesh));
            }
        }
        // Solid parts, nearest first: the wood in front is drawn before
        // what it hides, so the depth test spares the hidden fragments
        // their shading. Ties go by index, so every frame is drawn alike.
        self.solid
            .sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        // Ghosts and see-through parts, farthest first so they blend in
        // order.
        self.translucent
            .sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.2.cmp(&b.2)));
    }
}

/// A vertex buffer written afresh for each frame that needs it, and kept
/// while it is large enough, so frames don't make buffers.
struct Vertices {
    label: &'static str,
    buffer: Option<wgpu::Buffer>,
}

/// Vertices written for a frame.
struct Written {
    buffer: wgpu::Buffer,
    bytes: u64,
    count: u32,
}

impl Written {
    /// Draws them with the pipeline set, as vertex buffer 0.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.buffer.slice(..self.bytes));
        pass.draw(0..self.count, 0..1);
    }
}

impl Vertices {
    fn new(label: &'static str) -> Vertices {
        Vertices {
            label,
            buffer: None,
        }
    }

    /// Writes `vertices`, growing the buffer (by doubling) when they don't
    /// fit; `None` when there are none.
    fn write<T: bytemuck::Pod>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[T],
    ) -> Option<Written> {
        if vertices.is_empty() {
            return None;
        }
        let data: &[u8] = bytemuck::cast_slice(vertices);
        let bytes = crate::convert::bytes(data.len());
        let fits = self.buffer.as_ref().is_some_and(|b| b.size() >= bytes);
        if !fits {
            self.buffer = Some(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(self.label),
                    size: bytes
                        .max(4096)
                        .checked_next_power_of_two()
                        .unwrap_or(bytes)
                        .next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            );
        }
        let buffer = self.buffer.clone()?;
        queue.write_buffer(&buffer, 0, data);
        Some(Written {
            buffer,
            bytes,
            count: u32::try_from(vertices.len()).unwrap_or(u32::MAX),
        })
    }
}

/// The pixels of a `width` by `height` frame that a box covers seen
/// through `view_projection`, widened by four pixels (the outline's band
/// and a pixel to spare): left, top, width, height. The whole frame when
/// some of the box is behind the eye; `None` when none of it is on screen.
fn screen_rect(
    view_projection: Mat4,
    bounds: Aabb,
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    const MARGIN: f32 = 4.0;
    let (w, h) = (
        crate::convert::f32_of_u32(width),
        crate::convert::f32_of_u32(height),
    );
    let mut low = glam::Vec2::splat(f32::MAX);
    let mut high = glam::Vec2::splat(f32::MIN);
    for corner in bounds.corners() {
        let clip = view_projection * corner.extend(1.0);
        if clip.w <= 1e-6 {
            return Some((0, 0, width, height));
        }
        // Normalised device coordinates to pixels, from the top left.
        let pixel = glam::Vec2::new(
            (clip.x / clip.w + 1.0) * 0.5 * w,
            (1.0 - clip.y / clip.w) * 0.5 * h,
        );
        low = low.min(pixel);
        high = high.max(pixel);
    }
    let whole = |value: f32, limit: u32| {
        stave_geom::render::integer_from_f64(f64::from(value.clamp(0.0, f32::MAX)))
            .and_then(|n| u32::try_from(n).ok())
            .map_or(limit, |n| n.min(limit))
    };
    let (left, top) = (
        whole((low.x - MARGIN).floor(), width),
        whole((low.y - MARGIN).floor(), height),
    );
    let (right, bottom) = (
        whole((high.x + MARGIN).ceil(), width),
        whole((high.y + MARGIN).ceil(), height),
    );
    (right > left && bottom > top).then(|| (left, top, right - left, bottom - top))
}

/// Boxes' faces (triangles) and edges (lines), written into `faces` and
/// `lines` in place of what they held.
fn box_vertices(boxes: &[Aabb], faces: &mut Vec<[f32; 3]>, lines: &mut Vec<[f32; 3]>) {
    faces.clear();
    lines.clear();
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
}

/// Draws node `index`'s mesh from the shared buffers bound by
/// [`Renderer::bind_meshes`].
fn draw_mesh(pass: &mut wgpu::RenderPass<'_>, index: u32, mesh: MeshRange) {
    let indices = mesh.indices();
    if !indices.is_empty() {
        pass.draw_indexed(indices, 0, index..index + 1);
    }
}

/// Draws node `index`'s outline from the shared outline buffer, bound as
/// vertex buffer 0.
fn draw_edges(pass: &mut wgpu::RenderPass<'_>, index: u32, mesh: MeshRange) {
    let edges = mesh.edges();
    if !edges.is_empty() {
        pass.draw(edges, index..index + 1);
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
type Strokes = &'static [&'static [(f32, f32)]];

/// A character of a note: its width (a digit is 1 wide, 2 high) and its
/// strokes. Characters without a glyph are a space.
fn glyph(character: char) -> (f32, Strokes) {
    match character {
        '0'..='9' => {
            let digit = u8::try_from(u32::from(character) - u32::from('0')).unwrap_or(0);
            (1.0, digit_strokes(digit))
        }
        '.' => (0.4, (&[&[(0.15, 0.0), (0.15, 0.25)]])),
        'm' => (
            1.3,
            (&[
                &[(0.0, 0.0), (0.0, 1.3)],
                &[(0.0, 1.1), (0.3, 1.3), (0.65, 1.1), (0.65, 0.0)],
                &[(0.65, 1.1), (0.95, 1.3), (1.3, 1.1), (1.3, 0.0)],
            ]),
        ),
        'c' => (1.0, (&[&[(1.0, 1.3), (0.0, 1.3), (0.0, 0.0), (1.0, 0.0)]])),
        'l' => (0.6, (&[&[(0.3, 2.0), (0.3, 0.0)]])),
        'o' => (
            1.0,
            (&[&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.3), (0.0, 1.3), (0.0, 0.0)]]),
        ),
        's' => (
            1.0,
            (&[&[
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
            (&[&[
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
            (&[
                &[(0.35, 2.0), (0.35, 0.0), (0.9, 0.0)],
                &[(0.0, 1.3), (0.9, 1.3)],
            ]),
        ),
        'a' => (
            1.0,
            (&[&[
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
            (&[&[(0.15, 0.1), (0.15, 0.3)], &[(0.15, 1.0), (0.15, 1.2)]]),
        ),
        '°' => (
            0.6,
            (&[&[(0.0, 1.6), (0.3, 2.0), (0.6, 1.6), (0.3, 1.2), (0.0, 1.6)]]),
        ),
        _ => (0.6, &[]),
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

/// The overlay as triangles on screen, written into `out` in place of what
/// it held: lines a constant width in pixels, labels as filled discs with
/// their numbers in white.
fn overlay_triangles(
    overlay: &Overlay,
    frame: &Frame<'_>,
    width: u32,
    height: u32,
    out: &mut Vec<OverlayVertex>,
) {
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
    out.clear();
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
        segment(out, a, b, pixels, colour);
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
            segment(out, anchor, centre, 1.5, LABEL_FILL);
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
                        segment(out, place(*a), place(*b), 2.2, LABEL_INK);
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
        let width: f32 = words.chars().map(|c| glyph(c).0 + gap).sum::<f32>() - gap;
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
        for (advance, strokes) in words.chars().map(glyph) {
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
        text(out, origin, &note.text, size, NOTE_INK, true);
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
        text(out, origin, &marker.text, size, PATH_COLOUR, false);
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
    fn wait_and_take_live(renderer: &mut Renderer) -> Option<LiveFrame> {
        renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        renderer.take_ready().unwrap()
    }

    /// As [`wait_and_take_live`], for a renderer whose frames go by
    /// readback.
    fn wait_and_take(renderer: &mut Renderer) -> Option<MappedFrame> {
        wait_and_take_live(renderer).map(|frame| match frame {
            LiveFrame::Mapped(frame) => frame,
            LiveFrame::Dmabuf(frame) => panic!("a dmabuf without asking: {frame:?}"),
        })
    }

    /// A live frame's rows without their padding.
    fn unpadded(frame: &MappedFrame) -> Vec<u8> {
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

    /// A renderer whose live frames go to dmabufs laid out linearly (the
    /// one layout every driver that exports dmabufs offers, standing in for
    /// GTK's list); `None`, saying why, when this GPU can't.
    fn dmabuf_renderer() -> Option<Renderer> {
        let mut renderer = renderer()?;
        const LINEAR: u64 = 0;
        match renderer.use_dmabufs(vec![LINEAR]) {
            Ok(()) => Some(renderer),
            Err(error) => {
                eprintln!("skipped: no dmabufs here: {error:#}");
                None
            }
        }
    }

    /// Waits for the newest live frame and says it went to a dmabuf.
    fn wait_and_take_dmabuf(renderer: &mut Renderer) -> DmabufFrame {
        match wait_and_take_live(renderer) {
            Some(LiveFrame::Dmabuf(frame)) => frame,
            Some(LiveFrame::Mapped(_)) => panic!("the frame went by readback"),
            None => panic!("no frame arrived"),
        }
    }

    #[test]
    fn a_live_frame_in_a_dmabuf_holds_what_a_screenshot_does() {
        let Some(mut renderer) = dmabuf_renderer() else {
            return;
        };
        assert!(renderer.uses_dmabufs());
        let (scene, posed) = stool();
        renderer.upload(&scene);
        let still = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        let ticket = renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        let live = wait_and_take_dmabuf(&mut renderer);
        assert!(renderer.settled(ticket));
        // Read through a second texture imported from the dmabuf's
        // descriptor, with the layout it reports, as GTK imports it.
        let seen = dmabuf::read_through_import(&renderer.device, &renderer.queue, &live).unwrap();
        assert_eq!(seen, still.rgba);
    }

    #[test]
    fn still_pictures_go_by_readback_while_the_live_view_uses_dmabufs() {
        let Some(mut renderer) = dmabuf_renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        assert_eq!(renderer.ring.made(), 1);
        assert_eq!(
            renderer.dmabuf.as_ref().map(dmabuf::Exporter::made),
            Some(0)
        );
    }

    #[test]
    fn a_frame_s_dmabuf_is_used_again_once_gtk_lets_it_go() {
        let Some(mut renderer) = dmabuf_renderer() else {
            return;
        };
        let made = |renderer: &Renderer| renderer.dmabuf.as_ref().map(dmabuf::Exporter::made);
        let (scene, posed) = stool();
        renderer.upload(&scene);
        for _ in 0..2 {
            renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
            drop(wait_and_take_dmabuf(&mut renderer));
        }
        assert_eq!(made(&renderer), Some(1));
        // Held, the frame's dmabuf isn't drawn into again: another is made.
        renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        let held = wait_and_take_dmabuf(&mut renderer);
        renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        assert_eq!(made(&renderer), Some(2));
        drop(held);
        // No readback buffer was ever needed.
        assert_eq!(renderer.ring.made(), 0);
    }

    #[test]
    fn a_resized_view_gets_dmabufs_of_its_new_size() {
        let Some(mut renderer) = dmabuf_renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        drop(wait_and_take_dmabuf(&mut renderer));
        let mut wider = frame(&scene, &posed, 0.0);
        wider.width = 128;
        let still = renderer.render(&wider).unwrap();
        renderer.submit(&wider).unwrap();
        let live = wait_and_take_dmabuf(&mut renderer);
        let seen = dmabuf::read_through_import(&renderer.device, &renderer.queue, &live).unwrap();
        assert_eq!(seen, still.rgba);
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
        // An upload of the same meshes keeps it; one with a mesh changed
        // draws it again, even with nothing moved.
        let generation = renderer.meshes_generation;
        renderer.upload(&scene);
        assert_eq!(renderer.meshes_generation, generation);
        let mut changed = scene.clone();
        raise(&mut changed.nodes[caster], 5.0);
        renderer.upload(&changed);
        assert_ne!(
            renderer.shadow_key.as_ref().map(|key| key.meshes),
            Some(renderer.meshes_generation)
        );
    }

    /// The stool's frame at another size.
    fn sized<'a>(scene: &'a SceneGraph, posed: &'a Posed, width: u32, height: u32) -> Frame<'a> {
        Frame {
            width,
            height,
            ..frame(scene, posed, 0.0)
        }
    }

    #[test]
    fn targets_have_room_to_grow_and_still_pictures_keep_their_own() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        // A window dragged larger a few pixels at a time: one target.
        for (width, height) in [(1030, 600), (1031, 600), (1100, 700), (1280, 768)] {
            renderer
                .submit(&sized(&scene, &posed, width, height))
                .unwrap();
        }
        assert_eq!(renderer.targets_made, 1);
        // Small pictures between frames get their own, and leave the
        // view's alone.
        for _ in 0..3 {
            renderer.render(&sized(&scene, &posed, 160, 90)).unwrap();
            renderer.submit(&sized(&scene, &posed, 1280, 768)).unwrap();
        }
        assert_eq!(renderer.targets_made, 2);
        // Much smaller: a smaller target is made.
        renderer.submit(&sized(&scene, &posed, 64, 48)).unwrap();
        assert_eq!(renderer.targets_made, 3);
        assert_eq!(
            renderer.live_target.as_ref().map(|t| (t.width, t.height)),
            Some((256, 256))
        );
        let _ = wait_and_take(&mut renderer);
    }

    #[test]
    fn a_frame_drawn_in_a_larger_target_is_the_same_frame() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        let selection = [scene.node_ids().next().unwrap()];
        let selected = |width, height| Frame {
            selection: &selection,
            ..sized(&scene, &posed, width, height)
        };
        renderer.upload(&scene);
        renderer.render(&selected(500, 300)).unwrap();
        let small = renderer.render(&selected(200, 120)).unwrap();
        assert_eq!(
            renderer.still_target.as_ref().map(|t| (t.width, t.height)),
            Some((512, 512))
        );
        let mut fresh = Renderer::new().unwrap();
        fresh.upload(&scene);
        fresh.render(&selected(200, 120)).unwrap();
        assert_eq!(
            fresh.still_target.as_ref().map(|t| (t.width, t.height)),
            Some((256, 256))
        );
        assert_eq!(small, fresh.render(&selected(200, 120)).unwrap());
    }

    /// Moves a node's mesh and outline up by `millimetres`, as an edit
    /// to one part would change it.
    fn raise(node: &mut stave_scene::Node, millimetres: f32) {
        for vertex in &mut node.geometry.mesh.vertices {
            vertex.position[1] += millimetres;
        }
        for segment in &mut node.geometry.edges.segments {
            for point in segment {
                point[1] += millimetres;
            }
        }
    }

    #[test]
    fn a_box_on_screen_covers_its_pixels_and_a_margin() {
        let scene_box = Aabb {
            min: Vec3::new(-1.0, -1.0, -1.0),
            max: Vec3::new(1.0, 1.0, 1.0),
        };
        // Flat, looking down -z, the box filling the middle half.
        let flat = Mat4::orthographic_rh(-2.0, 2.0, -2.0, 2.0, -10.0, 10.0);
        assert_eq!(
            screen_rect(flat, scene_box, 200, 100),
            Some((46, 21, 108, 58))
        );
        // Off the edge: clamped to the frame.
        let moved = flat * Mat4::from_translation(Vec3::new(1.5, 0.0, 0.0));
        assert_eq!(
            screen_rect(moved, scene_box, 200, 100),
            Some((121, 21, 79, 58))
        );
        // Wholly off screen: nothing.
        let away = flat * Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(screen_rect(away, scene_box, 200, 100), None);
        // Partly behind the eye: the whole frame.
        let eye = Mat4::perspective_rh(1.0, 2.0, 0.1, 100.0);
        assert_eq!(
            screen_rect(eye, scene_box, 200, 100),
            Some((0, 0, 200, 100))
        );
    }

    #[test]
    fn the_shortcuts_draw_the_frame_the_full_passes_do() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        let selection: Vec<NodeId> = scene.node_ids().take(2).collect();
        let selected = |yaw| Frame {
            selection: &selection,
            ..frame(&scene, &posed, yaw)
        };
        // Nothing selected: no mask is made, and nothing multisampled kept.
        let plain = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        assert!(renderer.still_target.as_ref().unwrap().mask.is_none());
        let pictures: Vec<Image> = [0.0, 0.7, 2.4]
            .into_iter()
            .map(|yaw| renderer.render(&selected(yaw)).unwrap())
            .collect();
        assert!(renderer.still_target.as_ref().unwrap().mask.is_some());
        assert_ne!(plain, pictures[0]);
        renderer.full_passes = true;
        assert_eq!(renderer.render(&frame(&scene, &posed, 0.0)).unwrap(), plain);
        for (yaw, picture) in [0.0, 0.7, 2.4].into_iter().zip(&pictures) {
            assert_eq!(&renderer.render(&selected(yaw)).unwrap(), picture);
        }
    }

    #[test]
    fn the_nodes_are_written_only_when_they_change() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        let selection = [scene.node_ids().next().unwrap()];
        renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        let writes = renderer.node_writes;
        // The camera turned: the nodes are as they were.
        let turned = renderer.render(&frame(&scene, &posed, 0.7)).unwrap();
        assert_eq!(renderer.node_writes, writes);
        // Selected, then not: written each time, and the picture is the
        // one drawn before.
        renderer
            .render(&Frame {
                selection: &selection,
                ..frame(&scene, &posed, 0.7)
            })
            .unwrap();
        assert_eq!(
            renderer.render(&frame(&scene, &posed, 0.7)).unwrap(),
            turned
        );
        assert_eq!(renderer.node_writes, writes + 2);
    }

    #[test]
    fn an_upload_writes_only_the_meshes_the_gpu_does_not_hold() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        let held = renderer.meshes.held();
        assert!(held > 0);
        let first = renderer.meshes.ranges().to_vec();
        let picture = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        // The same scene again (a renamed part, a comment): nothing new.
        let generation = renderer.meshes_generation;
        renderer.upload(&scene);
        assert_eq!(renderer.meshes_generation, generation);
        assert_eq!(renderer.meshes.held(), held);
        // One part changed: only its mesh is written; the rest keep their
        // place.
        let mut changed = scene.clone();
        raise(&mut changed.nodes[0], 5.0);
        renderer.upload(&changed);
        assert_ne!(renderer.meshes_generation, generation);
        assert_eq!(renderer.meshes.held(), held + 1);
        let second = renderer.meshes.ranges().to_vec();
        assert_ne!(first[0], second[0]);
        assert_eq!(first[1..], second[1..]);
        // Drawn from the shared buffers, it looks as a renderer with
        // nothing else uploaded draws it.
        let reused = renderer.render(&frame(&changed, &posed, 0.0)).unwrap();
        let mut fresh = Renderer::new().unwrap();
        fresh.upload(&changed);
        assert_eq!(reused, fresh.render(&frame(&changed, &posed, 0.0)).unwrap());
        assert_ne!(reused, picture);
        // Back to the first: every mesh is still held.
        renderer.upload(&scene);
        assert_eq!(renderer.meshes.ranges(), first.as_slice());
        assert_eq!(
            renderer.render(&frame(&scene, &posed, 0.0)).unwrap(),
            picture
        );
    }

    #[test]
    fn the_shared_buffers_grow_and_start_afresh_when_meshes_no_longer_fit() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        let picture = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        // Far more meshes than the buffers were made for, none of them the
        // stool's: the buffers grow and are written from the start.
        let many: Vec<stave_scene::Node> = (1..=60)
            .flat_map(|step| {
                scene.nodes.iter().map(move |node| {
                    let mut node = node.clone();
                    raise(&mut node, f32::from(u8::try_from(step).unwrap()));
                    node
                })
            })
            .collect();
        assert!(
            renderer
                .meshes
                .upload(&renderer.device, &renderer.queue, &many)
        );
        assert!(renderer.meshes.failed().is_none());
        assert_eq!(renderer.meshes.held(), many.len());
        // The stool again, written into the grown buffers, looks the same.
        renderer.upload(&scene);
        assert_eq!(
            renderer.render(&frame(&scene, &posed, 0.0)).unwrap(),
            picture
        );
    }
}

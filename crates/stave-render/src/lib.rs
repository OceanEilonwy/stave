//! `stave-render`: the Stave renderer. It draws a scene (`stave-scene`)
//! with wgpu, on the desktop and on the web alike.
//!
//! A frame is drawn into a multisampled texture and resolved; then it is
//! either read back as RGBA bytes ([`Renderer::render`], for screenshots
//! and pictures), drawn onto a surface ([`Renderer::present`], a page's
//! canvas), or copied wherever the caller wants before it is submitted
//! ([`Renderer::draw`]: the window's live view copies it into its own
//! readback buffers or dmabufs).
//!
//! Setting up the device is `async` ([`Renderer::headless`],
//! [`Renderer::for_surface`]): the desktop blocks on it, the web awaits it.
//! A lost device is reported ([`RenderError::DeviceLost`]), not mended:
//! the caller opens a new renderer and uploads its scene again.

#![forbid(unsafe_code)]

mod convert;
mod meshes;
mod present;
pub mod profile;

use meshes::MeshRange;

use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use glam::{Mat4, Vec3};
use stave_scene::{
    Aabb, Highlight, LineKind, Look, Material, NodeId, Orbit, Overlay, Posed, SceneGraph, Section,
};
use web_time::Instant;

/// Why a frame couldn't be drawn, or a renderer opened.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// No graphics adapter wgpu can use.
    #[error("no graphics adapter found (wgpu): {0}")]
    NoAdapter(#[source] wgpu::RequestAdapterError),
    /// The adapter wouldn't give a device.
    #[error("the graphics adapter refused a device: {0}")]
    NoDevice(#[source] wgpu::RequestDeviceError),
    /// The device was lost (a driver reset, suspend, an external GPU
    /// unplugged): a new renderer must be opened.
    #[error("the graphics device was lost ({0})")]
    DeviceLost(String),
    /// The GPU reported an error while drawing.
    #[error("the GPU couldn't draw the frame: {0}")]
    Gpu(String),
    /// The frame is larger than the GPU's textures can be.
    #[error("a {width}x{height} view is larger than the GPU allows ({limit})")]
    TooLarge {
        /// Pixels across.
        width: u32,
        /// Pixels down.
        height: u32,
        /// The largest texture side the GPU allows.
        limit: u32,
    },
    /// The scene's meshes couldn't be put on the GPU.
    #[error("couldn't upload the design's meshes: {0}")]
    Meshes(String),
    /// The frame's scene isn't the one last uploaded.
    #[error("the scene changed without being uploaded")]
    NotUploaded,
    /// The frame's textures weren't there to draw in.
    #[error("no render target")]
    NoTarget,
    /// Waiting for the GPU failed.
    #[error("waiting for the GPU: {0}")]
    Wait(#[source] wgpu::PollError),
    /// The GPU didn't finish the frame in time.
    #[error("the GPU didn't finish the frame")]
    Unfinished,
    /// The finished frame couldn't be mapped for reading.
    #[error("couldn't read the frame back: {0}")]
    ReadBack(#[source] wgpu::BufferAsyncError),
    /// The mapped frame couldn't be reached.
    #[error("mapping the frame: {0}")]
    Mapping(#[source] wgpu::MapRangeError),
    /// [`Renderer::present`] on a renderer opened without a surface.
    #[error("this renderer draws offscreen, not to a surface")]
    NoSurface,
    /// The surface has no format the renderer can draw in.
    #[error("the surface can't show the pictures (none of its formats has an sRGB form)")]
    UnsuitableSurface,
    /// The surface gave no texture to draw the frame in.
    #[error("the surface has no frame to draw in: {0}")]
    Surface(SurfaceTrouble),
    /// The picture couldn't be encoded as a PNG.
    #[error("couldn't encode the picture as a PNG: {0}")]
    Png(#[from] png::EncodingError),
}

/// Why a surface gave no texture to draw in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceTrouble {
    /// It timed out; try the next frame.
    Timeout,
    /// The window or page is hidden; try once it shows again.
    Occluded,
    /// It is outdated even after being configured again.
    Outdated,
    /// It was lost and must be made again.
    Lost,
    /// wgpu reported a validation error getting it.
    Validation,
}

impl std::fmt::Display for SurfaceTrouble {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            SurfaceTrouble::Timeout => "it timed out",
            SurfaceTrouble::Occluded => "it is hidden",
            SurfaceTrouble::Outdated => "it is out of date",
            SurfaceTrouble::Lost => "it was lost",
            SurfaceTrouble::Validation => "wgpu reported an error",
        })
    }
}

/// How long the GPU may take over a still picture before it counts as
/// failed, so a hung device shows an error instead of freezing.
const PATIENCE: Duration = Duration::from_secs(10);

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
    /// Its colour as 8-bit sRGB, for drawing the stage outside the
    /// renderer: the colour a frame is cleared to, as it lands on screen.
    #[must_use]
    pub fn srgb(self) -> [u8; 3] {
        match self {
            Backdrop::Light => [0xe8, 0xe5, 0xdf],
            Backdrop::Dark => [0x24, 0x24, 0x28],
        }
    }

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

/// Which of the renderer's targets a frame is drawn in. Each keeps its
/// own textures, so small still pictures drawn between live frames never
/// make the live view's anew.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The live view's (and a surface's).
    Live,
    /// Still pictures'.
    Still,
}

/// A frame's finished picture, resolved, before it is submitted: what
/// [`Renderer::draw`] hands its caller to copy out.
#[derive(Debug, Clone, Copy)]
pub struct Resolved<'a> {
    /// The texture holding it in its top-left corner, `Rgba8UnormSrgb`;
    /// it may be larger than the frame.
    pub texture: &'a wgpu::Texture,
    /// The frame's pixels across.
    pub width: u32,
    /// The frame's pixels down.
    pub height: u32,
}

impl Resolved<'_> {
    /// Where to copy the picture from.
    pub fn copy_source(&self) -> wgpu::TexelCopyTextureInfo<'_> {
        wgpu::TexelCopyTextureInfo {
            texture: self.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        }
    }

    /// How much to copy: the frame, not the whole texture.
    pub fn extent(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        }
    }
}

/// A frame drawn and submitted.
#[derive(Debug)]
pub struct Submitted {
    /// The submission that draws and copies it.
    pub submission: wgpu::SubmissionIndex,
    /// When it was submitted.
    pub submitted: Instant,
    /// The CPU side of drawing it, when timings are on
    /// ([`profile::enabled`]).
    pub timings: Option<profile::FrameTimings>,
}

/// How a frame's bytes lie in a readback buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
    width: u32,
    height: u32,
    /// Bytes from one row to the next: `width * 4`, rounded up to wgpu's
    /// copy alignment.
    padded_row: u32,
}

impl Layout {
    fn new(width: u32, height: u32) -> Layout {
        Layout {
            width,
            height,
            padded_row: width
                .saturating_mul(4)
                .next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
        }
    }

    fn bytes(self) -> u64 {
        u64::from(self.padded_row) * u64::from(self.height)
    }
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
    /// The surface frames are presented on; `None` offscreen.
    presenter: Option<present::Presenter>,
    /// A readback buffer kept from the last still picture, for the next.
    still_readback: Option<wgpu::Buffer>,
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
    /// Where the live view is drawn, and a surface's frames.
    live_target: Option<Target>,
    /// Where still pictures are drawn ([`Renderer::render`]): the small
    /// pictures beside the view and screenshots, so they never make the
    /// live view's targets anew.
    still_target: Option<Target>,
    /// How many targets were made, for tests.
    targets_made: u64,
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
        array_stride: convert::bytes(std::mem::size_of::<stave_geom::Vertex>()),
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
        size: capacity.max(1) * convert::bytes(std::mem::size_of::<NodeData>()),
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
    /// Opens a GPU without a window, to draw offscreen. Fails when no
    /// adapter is available.
    ///
    /// It asks for the low-power adapter (the integrated one on a laptop
    /// with two), which draws a few hundred parts with ease and leaves the
    /// discrete GPU asleep; `WGPU_POWER_PREF=high` asks for the other.
    pub async fn headless() -> Result<Renderer, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::from_env()
                    .unwrap_or(wgpu::PowerPreference::LowPower),
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
            .map_err(RenderError::NoAdapter)?;
        Renderer::on_adapter(&adapter).await
    }

    /// Opens a GPU that draws on `surface` (a window's, or a page's
    /// canvas), made from `instance`, `width` by `height` pixels.
    /// [`Renderer::present`] draws on it; [`Renderer::resize`] follows the
    /// surface's size.
    pub async fn for_surface(
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Renderer, RenderError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::from_env()
                    .unwrap_or(wgpu::PowerPreference::LowPower),
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await
            .map_err(RenderError::NoAdapter)?;
        let mut renderer = Renderer::on_adapter(&adapter).await?;
        renderer.presenter = Some(present::Presenter::new(
            &renderer.device,
            &renderer.queue,
            &adapter,
            surface,
            width,
            height,
        )?);
        Ok(renderer)
    }

    /// The device features asked for: with the `dmabuf` feature, the
    /// export of dmabufs where the adapter has it and `STAVE_DMABUF` isn't
    /// `0`, for the window's live view; nothing otherwise.
    fn wanted_features(adapter: &wgpu::Adapter) -> wgpu::Features {
        if cfg!(feature = "dmabuf")
            && std::env::var_os("STAVE_DMABUF").is_none_or(|value| value != "0")
        {
            adapter.features() & wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF
        } else {
            wgpu::Features::empty()
        }
    }

    /// Opens a device on `adapter` and makes the pipelines.
    async fn on_adapter(adapter: &wgpu::Adapter) -> Result<Renderer, RenderError> {
        let adapter_name = adapter.get_info().name;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("stave"),
                required_features: Renderer::wanted_features(adapter),
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
            })
            .await
            .map_err(RenderError::NoDevice)?;
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
                        min_binding_size: NonZeroU64::new(convert::bytes(std::mem::size_of::<
                            Globals,
                        >(
                        ))),
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
            size: convert::bytes(std::mem::size_of::<Globals>()),
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
            array_stride: convert::bytes(std::mem::size_of::<OverlayVertex>()),
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
            presenter: None,
            still_readback: None,
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

    /// The device, for copying frames out ([`Renderer::draw`]).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The queue, for copying frames out ([`Renderer::draw`]).
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Why the device was lost, once wgpu has said it is: then every
    /// frame fails with [`RenderError::DeviceLost`], and a new renderer
    /// must be opened.
    pub fn lost(&self) -> Option<String> {
        self.trouble
            .lock()
            .ok()
            .and_then(|trouble| trouble.lost.clone())
    }

    /// Whether wgpu has said the device is lost.
    pub fn is_lost(&self) -> bool {
        self.trouble
            .lock()
            .is_ok_and(|trouble| trouble.lost.is_some())
    }

    /// Follows the surface's new size, `width` by `height` pixels;
    /// nothing for a renderer drawing offscreen.
    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(presenter) = &mut self.presenter {
            presenter.resize(&self.device, width, height);
        }
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
            .map(stave_scene::Node::bounds)
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
                // Copied out, or sampled onto a surface.
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
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

    /// Draws a frame offscreen and waits for it (at most ten seconds):
    /// for screenshots and still pictures.
    ///
    /// When the device has been lost, or is lost while drawing, it fails
    /// with [`RenderError::DeviceLost`]; an error the GPU reports while
    /// drawing fails the frame instead of the application.
    pub fn render(&mut self, frame: &Frame<'_>) -> Result<Image, RenderError> {
        let layout = Layout::new(frame.width.max(1), frame.height.max(1));
        let buffer = match self.still_readback.take() {
            Some(buffer) if buffer.size() == layout.bytes() => buffer,
            Some(_) | None => self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: layout.bytes(),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        let submitted = self.draw(frame, Slot::Still, |encoder, resolved| {
            encoder.copy_texture_to_buffer(
                resolved.copy_source(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(layout.padded_row),
                        rows_per_image: Some(layout.height),
                    },
                },
                resolved.extent(),
            );
        })?;
        self.read_back(buffer, layout, submitted)
    }

    /// Draws a frame on the surface the renderer was opened for
    /// ([`Renderer::for_surface`]), scaled to fill it, and presents it.
    /// Fails with [`RenderError::NoSurface`] on a renderer drawing
    /// offscreen.
    pub fn present(&mut self, frame: &Frame<'_>) -> Result<(), RenderError> {
        let mut presenter = self.presenter.take().ok_or(RenderError::NoSurface)?;
        let presented = self.present_on(&mut presenter, frame);
        self.presenter = Some(presenter);
        presented
    }

    fn present_on(
        &mut self,
        presenter: &mut present::Presenter,
        frame: &Frame<'_>,
    ) -> Result<(), RenderError> {
        let output = presenter.next_texture(&self.device)?;
        self.draw(frame, Slot::Live, |encoder, resolved| {
            presenter.blit(encoder, resolved, &output.texture);
        })?;
        self.queue.present(output);
        Ok(())
    }

    /// Draws a frame into `slot`'s target and calls `copy` with its
    /// resolved picture, to copy it out on the same encoder, then submits
    /// it without waiting. Every way of getting a frame out goes through
    /// here: [`Renderer::render`], [`Renderer::present`], and the window's
    /// own readback buffers and dmabufs.
    ///
    /// When the device has been lost, or is lost while drawing, it fails
    /// with [`RenderError::DeviceLost`]; an error the GPU reports while
    /// drawing fails the frame instead of the application.
    pub fn draw(
        &mut self,
        frame: &Frame<'_>,
        slot: Slot,
        copy: impl FnOnce(&mut wgpu::CommandEncoder, &Resolved<'_>),
    ) -> Result<Submitted, RenderError> {
        if let Some(reason) = self.lost() {
            return Err(RenderError::DeviceLost(reason));
        }
        // The lists are taken out while the frame is worked out, and put
        // back for the next, whatever happens.
        let mut scratch = std::mem::take(&mut self.scratch);
        let drawn = self.draw_with(frame, slot, &mut scratch, copy);
        self.scratch = scratch;
        // A device lost while drawing, or before (it may only say so when
        // polled).
        let _ = self.device.poll(wgpu::PollType::Poll);
        if let Some(reason) = self.lost() {
            return Err(RenderError::DeviceLost(reason));
        }
        let error = self
            .trouble
            .lock()
            .ok()
            .and_then(|mut trouble| trouble.error.take());
        match error {
            Some(error) => Err(RenderError::Gpu(error)),
            None => drawn,
        }
    }

    fn draw_with(
        &mut self,
        frame: &Frame<'_>,
        slot: Slot,
        scratch: &mut Scratch,
        copy: impl FnOnce(&mut wgpu::CommandEncoder, &Resolved<'_>),
    ) -> Result<Submitted, RenderError> {
        let (width, height) = (frame.width.max(1), frame.height.max(1));
        let limit = self.device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            return Err(RenderError::TooLarge {
                width,
                height,
                limit,
            });
        }
        if let Some(failed) = self.meshes.failed() {
            return Err(RenderError::Meshes(failed.to_owned()));
        }
        if self.meshes.ranges().len() != frame.scene.nodes.len() {
            return Err(RenderError::NotUploaded);
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
                let transform = frame.posed.transforms().get(index)?;
                Some(node.bounds().transformed(*transform))
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
            // Where the mesh is drawn: a part that moved casts its shadow
            // elsewhere though its mesh (in its own frame) is the same.
            let model = usize::try_from(index)
                .ok()
                .and_then(|index| {
                    let node = frame.scene.nodes.get(index)?;
                    Some(node.placed(*frame.posed.transforms().get(index)?))
                })
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
        .ok_or(RenderError::NoTarget)?;
        // Every pass draws in the target's top-left corner.
        let corner = |pass: &mut wgpu::RenderPass<'_>| {
            pass.set_viewport(
                0.0,
                0.0,
                convert::f32_of_u32(width),
                convert::f32_of_u32(height),
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
        copy(
            &mut encoder,
            &Resolved {
                texture: &target.resolved,
                width,
                height,
            },
        );
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
                dmabuf: false,
            }),
            _ => None,
        };
        Ok(Submitted {
            submission,
            submitted,
            timings,
        })
    }

    /// Waits for a still picture drawn into `buffer`, at most
    /// [`PATIENCE`], and copies it out without the rows' padding; the
    /// buffer is kept for the next.
    fn read_back(
        &mut self,
        buffer: wgpu::Buffer,
        layout: Layout,
        submitted: Submitted,
    ) -> Result<Image, RenderError> {
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // The receiver waits below; if it is gone there is no one to tell.
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted.submission),
                timeout: Some(PATIENCE),
            })
            .map_err(RenderError::Wait)?;
        receiver
            .try_recv()
            .map_err(|_| RenderError::Unfinished)?
            .map_err(RenderError::ReadBack)?;
        let mapped_at = Instant::now();
        let row = usize::try_from(layout.width)
            .unwrap_or(usize::MAX)
            .saturating_mul(4);
        let padded = usize::try_from(layout.padded_row).unwrap_or(usize::MAX);
        let mut rgba =
            Vec::with_capacity(row.saturating_mul(usize::try_from(layout.height).unwrap_or(0)));
        {
            let mapped = slice.get_mapped_range().map_err(RenderError::Mapping)?;
            for line in mapped.chunks(padded) {
                rgba.extend_from_slice(line.get(..row).unwrap_or(line));
            }
        }
        buffer.unmap();
        self.still_readback = Some(buffer);
        if let Some(mut timings) = submitted.timings {
            timings.gpu_wait = mapped_at.duration_since(submitted.submitted);
            timings.readback = mapped_at.elapsed();
            timings.print();
        }
        Ok(Image {
            width: layout.width,
            height: layout.height,
            rgba,
        })
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
        let aspect = convert::f32_of_u32(width) / convert::f32_of_u32(height);
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
                convert::f32_of_u32(width),
                convert::f32_of_u32(height),
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
                // The mesh is in the node's own frame: its rest place,
                // then the pose.
                let model = node.placed(
                    frame
                        .posed
                        .transforms()
                        .get(index)
                        .copied()
                        .unwrap_or(Mat4::IDENTITY),
                );
                let seed = convert::f32_of_index(index % 97) * 1.618;
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
            let distance = node.bounds().transformed(transform).centre().distance(eye);
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
        let bytes = convert::bytes(data.len());
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
    let (w, h) = (convert::f32_of_u32(width), convert::f32_of_u32(height));
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
    let (w, h) = (convert::f32_of_u32(width), convert::f32_of_u32(height));
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
        let count = convert::f32_of_index(digits.len());
        let total = count * glyph.x + (count - 1.0).max(0.0) * gap;
        for (index, digit) in digits.iter().enumerate() {
            let left = -total * 0.5 + convert::f32_of_index(index) * (glyph.x + gap);
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
impl Renderer {
    /// A picture as PNG bytes, marked sRGB.
    pub fn encode_png(image: &Image) -> Result<Vec<u8>, RenderError> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, image.width, image.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&image.rgba)?;
            writer.finish()?;
        }
        Ok(bytes)
    }
}

/// A view's width over its height, for projections. Sizes beyond 65535
/// pixels (never a view's) count as 65535.
pub fn aspect(width: u32, height: u32) -> f32 {
    let pixels = |n: u32| f32::from(u16::try_from(n.max(1)).unwrap_or(u16::MAX));
    pixels(width) / pixels(height)
}

#[cfg(test)]
mod tests;

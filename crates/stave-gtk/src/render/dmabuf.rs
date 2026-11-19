//! Frames handed to GTK as dmabufs: the live view is drawn as usual, then
//! copied on the GPU into an image whose memory is exported as a Linux
//! dmabuf, and GTK is given its file descriptor. Nothing crosses to the
//! CPU: no readback, no upload of the frame by GTK.
//!
//! This is the one module of Stave allowed `unsafe` code (see the Rust
//! standards in `CLAUDE.md`): wgpu can't export memory, so the image is
//! made with Vulkan directly (through `ash`, on wgpu's own device) and
//! given to wgpu as a texture, and GTK's dmabuf textures take a raw file
//! descriptor. Every `unsafe` block says what it relies on.
//!
//! It is used only when wgpu runs on Vulkan with the dmabuf extensions,
//! and only with a layout (DRM format modifier) GTK says it can import and
//! the GPU can copy into without a second plane (no compression metadata
//! to keep in step). Anything else, or a failure at run time, leaves the
//! live view on the readback path ([`super::readback`]).
//!
//! Synchronisation: a frame is given to GTK only once the GPU has finished
//! the submission that copies it (the ring waits for that as it waits for a
//! readback buffer to map), so GTK never reads it half written. An image
//! comes back for reuse when GTK releases its texture, and released images
//! are reused oldest first, so the GPU never draws into the one GTK has
//! just let go of while its last draw of it may still be running.
//!
//! wgpu can't hand the image over to the foreign queue family that Vulkan
//! asks for before another API reads it. With a single-plane layout, which
//! has no compression metadata, that handover changes nothing in memory on
//! the drivers in use, which is one more reason only such layouts are
//! chosen. `STAVE_DMABUF=0` turns the whole path off.

use std::collections::VecDeque;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow, bail};
use ash::vk;
use gtk::gdk;
use wgpu::hal::api::Vulkan;

/// The DRM format of every frame: `AB24`, bytes red, green, blue, alpha in
/// memory, the same bytes as the readback path's `R8g8b8a8Premultiplied`.
pub const FOURCC: u32 = u32::from_le_bytes(*b"AB24");

/// The Vulkan format with that byte order. The frame's own format is the
/// sRGB view of the same bytes, and wgpu copies between the two as they
/// are.
const VULKAN_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const WGPU_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// What the images are used for: copied into (the frame), and out of (the
/// tests' check of what GTK would read).
const VULKAN_USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::from_raw(
    vk::ImageUsageFlags::TRANSFER_DST.as_raw() | vk::ImageUsageFlags::TRANSFER_SRC.as_raw(),
);

/// The most released images kept for reuse.
const SPARE: usize = 3;

/// The DRM format modifiers `display` can import dmabufs of [`FOURCC`]
/// with, for [`super::Renderer::use_dmabufs`].
pub fn display_modifiers(display: &gdk::Display) -> Vec<u64> {
    use gtk::prelude::DisplayExt as _;
    let formats = display.dmabuf_formats();
    (0..formats.n_formats())
        .map(|index| formats.format(index))
        .filter(|(fourcc, _)| *fourcc == FOURCC)
        .map(|(_, modifier)| modifier)
        .collect()
}

/// Whether dmabufs may be used at all: not when the environment variable
/// `STAVE_DMABUF` is `0`, which keeps the live view on readback (to
/// compare the two, or to get round a driver that draws them wrong). Read
/// once.
pub(super) fn wanted() -> bool {
    static WANTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WANTED.get_or_init(|| std::env::var_os("STAVE_DMABUF").is_none_or(|value| value != "0"))
}

/// An image whose memory is a dmabuf, as a wgpu texture.
struct Image {
    /// Owns the Vulkan image and its memory (wgpu frees both when the last
    /// reference goes).
    texture: wgpu::Texture,
    /// The dmabuf. It holds its own reference to the memory, so GTK can
    /// read it for as long as the descriptor is open, even after wgpu has
    /// freed its side.
    fd: OwnedFd,
    width: u32,
    height: u32,
    /// The layout the driver chose, from those GTK accepts.
    modifier: u64,
    /// Where the pixels start in the dmabuf, and bytes from one row to the
    /// next.
    offset: u32,
    stride: u32,
}

/// Released images, oldest first.
type Pool = Arc<Mutex<VecDeque<Image>>>;

/// Makes and reuses the images frames are copied into.
pub(super) struct Exporter {
    /// The layouts GTK accepts that the GPU can export and copy into.
    modifiers: Vec<u64>,
    pool: Pool,
    /// Set when GTK refused a frame: frames go back to the readback path.
    refused: Arc<AtomicBool>,
    /// How many images were made, for tests of reuse.
    made: u64,
}

impl Exporter {
    /// An exporter for `device`, for the layouts in `accepted` (the DRM
    /// format modifiers GTK can import for [`FOURCC`]); an error, saying
    /// why, when the device can't export dmabufs or none of those
    /// layouts suits it.
    pub fn new(device: &wgpu::Device, accepted: &[u64]) -> Result<Exporter> {
        if !device
            .features()
            .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
        {
            bail!("the GPU's driver can't export dmabufs (or wgpu isn't on Vulkan)");
        }
        if accepted.is_empty() {
            bail!("GTK can't import dmabufs in this format on this display");
        }
        let modifiers = exportable_modifiers(device, accepted)?;
        if modifiers.is_empty() {
            bail!("the GPU can't export any layout GTK can import");
        }
        Ok(Exporter {
            modifiers,
            pool: Pool::default(),
            refused: Arc::default(),
            made: 0,
        })
    }

    /// Whether GTK still takes the frames.
    pub fn usable(&self) -> bool {
        !self.refused.load(Ordering::Relaxed)
    }

    /// An image for a `width` by `height` frame: the oldest released one
    /// of that size, or a new one. Released images of other sizes, from
    /// before the view was resized, are dropped.
    pub fn frame(&mut self, device: &wgpu::Device, width: u32, height: u32) -> Result<Frame> {
        let reused = self.pool.lock().ok().and_then(|mut pool| {
            pool.retain(|image| (image.width, image.height) == (width, height));
            pool.pop_front()
        });
        let image = match reused {
            Some(image) => image,
            None => {
                self.made += 1;
                make_image(device, &self.modifiers, width, height)?
            }
        };
        Ok(Frame {
            width: image.width,
            height: image.height,
            image: Some(image),
            pool: Arc::clone(&self.pool),
            refused: Arc::clone(&self.refused),
        })
    }

    /// How many images have been made.
    #[cfg(test)]
    pub fn made(&self) -> u64 {
        self.made
    }
}

/// A frame of the live view in a dmabuf. Dropping it (or GTK releasing the
/// texture made of it) gives its image back for reuse.
pub struct Frame {
    /// Pixels across.
    width: u32,
    /// Pixels down.
    height: u32,
    /// `None` only while dropping.
    image: Option<Image>,
    pool: Pool,
    refused: Arc<AtomicBool>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Frame")
            .field("size", &self.image.as_ref().map(|i| (i.width, i.height)))
            .field("modifier", &self.image.as_ref().map(|i| i.modifier))
            .finish()
    }
}

impl Frame {
    /// Its size in pixels, across and down.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The texture to copy the frame into.
    pub(super) fn texture(&self) -> Option<&wgpu::Texture> {
        self.image.as_ref().map(|image| &image.texture)
    }

    /// Wraps the frame as a GDK texture, which holds it until GTK lets it
    /// go. An error means GTK refused it; frames after it are copied back
    /// as before.
    pub fn into_texture(mut self) -> Result<gdk::Texture> {
        let image = self.image.take().context("the frame was already used")?;
        let builder = gdk::DmabufTextureBuilder::new()
            .set_width(image.width)
            .set_height(image.height)
            .set_fourcc(FOURCC)
            .set_modifier(image.modifier)
            // Alpha is 1 everywhere: straight and premultiplied alike.
            .set_premultiplied(true)
            .set_n_planes(1)
            .set_offset(0, image.offset)
            .set_stride(0, image.stride);
        let builder = match gdk::Display::default() {
            Some(display) => builder.set_display(&display),
            None => builder,
        };
        // SAFETY: GTK requires the descriptor to stay open for as long as
        // the texture lives. It is owned by `image`, which moves into the
        // release function below, so it is closed only when GTK calls that
        // function (texture released), never before.
        let builder = unsafe { builder.set_fd(0, image.fd.as_raw_fd()) };
        let pool = Arc::clone(&self.pool);
        let release = move || give_back(&pool, image);
        // SAFETY: the plane's descriptor, offset, stride, modifier and
        // size are those Vulkan reported for this image's memory (see
        // `make_image`), the format is the image's, and the GPU has
        // finished writing it (the ring hands over a frame only once its
        // submission is done). The release function owns everything GTK
        // reads. If GTK refuses the frame it doesn't call the function,
        // and the image (one, once: frames stop coming here) is leaked
        // rather than freed while GTK might hold it.
        let built = unsafe { builder.build_with_release_func(release) };
        built.map_err(|error| {
            self.refused.store(true, Ordering::Relaxed);
            anyhow!("GTK refused the frame as a dmabuf: {error}")
        })
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        if let Some(image) = self.image.take() {
            give_back(&self.pool, image);
        }
    }
}

/// Keeps a released image for reuse, newest last, unless enough are kept.
fn give_back(pool: &Pool, image: Image) {
    if let Ok(mut pool) = pool.lock()
        && pool.len() < SPARE
    {
        pool.push_back(image);
    }
}

/// Of `accepted`, the modifiers the GPU can make an exportable,
/// single-plane image with, for copying into and out of.
fn exportable_modifiers(device: &wgpu::Device, accepted: &[u64]) -> Result<Vec<u64>> {
    // SAFETY: the hal device is only read from (instance, physical device)
    // while `device`, which owns it, is borrowed for this call; nothing is
    // destroyed through it.
    let hal = unsafe { device.as_hal::<Vulkan>() }.context("wgpu isn't running on Vulkan")?;
    let shared = hal.shared_instance();
    if shared.instance_api_version() < vk::API_VERSION_1_1 {
        bail!("the Vulkan instance is older than 1.1");
    }
    let instance = shared.raw_instance();
    let physical = hal.raw_physical_device();

    // Every modifier the driver has for the format, with its features.
    let mut count_list = vk::DrmFormatModifierPropertiesListEXT::default();
    {
        let mut properties = vk::FormatProperties2::default().push_next(&mut count_list);
        // SAFETY: `physical` is the device's own physical device, the
        // instance is 1.1 (checked above, so the entry point is loaded),
        // and the chained list asks only for the count (null array).
        unsafe {
            instance.get_physical_device_format_properties2(
                physical,
                VULKAN_FORMAT,
                &mut properties,
            );
        }
    }
    let count = usize::try_from(count_list.drm_format_modifier_count)?;
    let mut entries = vec![vk::DrmFormatModifierPropertiesEXT::default(); count];
    {
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
            .drm_format_modifier_properties(&mut entries);
        let mut properties = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: as above; the array is `count` long and its length is
        // passed with it, so the driver writes at most that many.
        unsafe {
            instance.get_physical_device_format_properties2(
                physical,
                VULKAN_FORMAT,
                &mut properties,
            );
        }
    }
    let needed = vk::FormatFeatureFlags::TRANSFER_DST | vk::FormatFeatureFlags::TRANSFER_SRC;
    let modifiers = entries
        .iter()
        .filter(|entry| {
            entry.drm_format_modifier_plane_count == 1
                && entry.drm_format_modifier_tiling_features.contains(needed)
                && accepted.contains(&entry.drm_format_modifier)
        })
        .map(|entry| entry.drm_format_modifier)
        .filter(|modifier| {
            let mut modifier_info = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
                .drm_format_modifier(*modifier)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let mut external_info = vk::PhysicalDeviceExternalImageFormatInfo::default()
                .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let info = vk::PhysicalDeviceImageFormatInfo2::default()
                .format(VULKAN_FORMAT)
                .ty(vk::ImageType::TYPE_2D)
                .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                .usage(VULKAN_USAGE)
                .push_next(&mut external_info)
                .push_next(&mut modifier_info);
            let mut external = vk::ExternalImageFormatProperties::default();
            let supported = {
                let mut properties = vk::ImageFormatProperties2::default().push_next(&mut external);
                // SAFETY: `physical` is the device's own; the input and
                // output chains are well formed and live for the call.
                unsafe {
                    instance.get_physical_device_image_format_properties2(
                        physical,
                        &info,
                        &mut properties,
                    )
                }
                .is_ok()
            };
            supported
                && external
                    .external_memory_properties
                    .external_memory_features
                    .contains(vk::ExternalMemoryFeatureFlags::EXPORTABLE)
        })
        .collect();
    Ok(modifiers)
}

/// A Vulkan image and its memory not yet handed to wgpu: freed if making
/// the frame's image fails part way.
struct Unwrapped<'a> {
    device: &'a ash::Device,
    image: vk::Image,
    memory: Option<vk::DeviceMemory>,
}

impl Drop for Unwrapped<'_> {
    fn drop(&mut self) {
        // SAFETY: the image and memory were made on `device` by
        // `make_image`, nothing else refers to them (they were never given
        // to wgpu or submitted), and they are destroyed once, here.
        unsafe {
            self.device.destroy_image(self.image, None);
            if let Some(memory) = self.memory {
                self.device.free_memory(memory, None);
            }
        }
    }
}

/// Makes an image whose memory is an exported dmabuf, laid out with one of
/// `modifiers`, as a wgpu texture on `device`.
fn make_image(device: &wgpu::Device, modifiers: &[u64], width: u32, height: u32) -> Result<Image> {
    let (hal_texture, fd, modifier, offset, stride) = {
        // SAFETY: the hal device is used only during this block, while
        // `device` (which owns it and outlives the guard) is borrowed. What
        // is made through it is either destroyed here on failure or handed
        // to wgpu, which then owns it.
        let hal = unsafe { device.as_hal::<Vulkan>() }.context("wgpu isn't running on Vulkan")?;
        let raw = hal.raw_device();
        let instance = hal.shared_instance().raw_instance();
        let physical = hal.raw_physical_device();
        let memory_fd = ash::khr::external_memory_fd::Device::new(instance, raw);
        let modifier_fns = ash::ext::image_drm_format_modifier::Device::new(instance, raw);

        let mut modifier_list =
            vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(modifiers);
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(VULKAN_FORMAT)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(VULKAN_USAGE)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut external)
            .push_next(&mut modifier_list);
        // SAFETY: a valid create info for wgpu's own device, whose
        // dmabuf and modifier extensions are enabled (the device has
        // VULKAN_EXTERNAL_MEMORY_DMA_BUF); every modifier listed was
        // checked as supported for this format, usage and export.
        let image = unsafe { raw.create_image(&info, None) }.context("making the image")?;
        let mut unwrapped = Unwrapped {
            device: raw,
            image,
            memory: None,
        };

        // SAFETY: `image` is a live image of `raw`.
        let requirements = unsafe { raw.get_image_memory_requirements(image) };
        // SAFETY: `physical` is the device's own physical device.
        let types = unsafe { instance.get_physical_device_memory_properties(physical) };
        let allowed = |index: usize| {
            u32::try_from(index)
                .ok()
                .and_then(|index| 1_u32.checked_shl(index))
                .is_some_and(|bit| requirements.memory_type_bits & bit != 0)
        };
        let local = types
            .memory_types_as_slice()
            .iter()
            .enumerate()
            .position(|(index, kind)| {
                allowed(index)
                    && kind
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            });
        let memory_type = local
            .or_else(|| (0..types.memory_types_as_slice().len()).find(|index| allowed(*index)))
            .context("no memory type for the image")?;
        let mut export = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(u32::try_from(memory_type)?)
            .push_next(&mut export)
            .push_next(&mut dedicated);
        // SAFETY: the size and type come from the image's requirements;
        // the memory is dedicated to `image` and exportable as a dmabuf,
        // as the image was made.
        let memory = unsafe { raw.allocate_memory(&allocation, None) }
            .context("allocating the image's memory")?;
        unwrapped.memory = Some(memory);
        // SAFETY: the memory was allocated for this image, dedicated, so
        // it is bound whole at offset 0, once.
        unsafe { raw.bind_image_memory(image, memory, 0) }.context("binding the memory")?;

        let get_fd = vk::MemoryGetFdInfoKHR::default()
            .memory(memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        // SAFETY: the memory was allocated exportable as a dmabuf.
        let raw_fd = unsafe { memory_fd.get_memory_fd(&get_fd) }.context("exporting the memory")?;
        // SAFETY: vkGetMemoryFdKHR makes a new descriptor holding its own
        // reference to the memory and gives the caller ownership of it;
        // nothing else closes it, so `OwnedFd` may.
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        let mut chosen = vk::ImageDrmFormatModifierPropertiesEXT::default();
        // SAFETY: `image` was made with DRM-modifier tiling on `raw`.
        unsafe { modifier_fns.get_image_drm_format_modifier_properties(image, &mut chosen) }
            .context("asking which layout the image has")?;
        let plane = vk::ImageSubresource {
            aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
            mip_level: 0,
            array_layer: 0,
        };
        // SAFETY: a DRM-modifier image's memory planes are queried with
        // MEMORY_PLANE_i aspects; its modifier has one plane (checked when
        // the modifiers were chosen).
        let layout = unsafe { raw.get_image_subresource_layout(image, plane) };
        let offset = u32::try_from(layout.offset).context("the image's offset")?;
        let stride = u32::try_from(layout.row_pitch).context("the image's stride")?;

        let descriptor = wgpu::hal::TextureDescriptor {
            label: Some("dmabuf frame"),
            size: extent(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WGPU_FORMAT,
            usage: wgpu::wgt::TextureUses::COPY_DST | wgpu::wgt::TextureUses::COPY_SRC,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };
        // The image goes to wgpu, which frees it and its memory.
        std::mem::forget(unwrapped);
        // SAFETY: `image` was made on this device with this descriptor's
        // size, format, usage (TRANSFER_DST and TRANSFER_SRC) and one
        // level, without MUTABLE_FORMAT and with no view formats; `memory`
        // is its dedicated memory, bound. With no drop callback and
        // `Dedicated` memory, wgpu-hal takes ownership of both.
        let hal_texture = unsafe {
            hal.texture_from_raw(
                image,
                &descriptor,
                None,
                wgpu::hal::vulkan::TextureMemory::Dedicated(memory),
            )
        };
        (hal_texture, fd, chosen.drm_format_modifier, offset, stride)
    };
    // SAFETY: the texture was made from this device's own handle, as the
    // descriptor below says (the same size, format and usage), and it is
    // in the UNDEFINED layout wgpu calls uninitialised; every frame copies
    // over all of it before it is read.
    let texture = unsafe {
        device.create_texture_from_hal::<Vulkan>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("dmabuf frame"),
                size: extent(width, height),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WGPU_FORMAT,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            wgpu::wgt::TextureUses::UNINITIALIZED,
        )
    };
    Ok(Image {
        texture,
        fd,
        width,
        height,
        modifier,
        offset,
        stride,
    })
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

/// For tests: what a frame's dmabuf holds, read back through a second
/// texture imported from its descriptor with the layout it reports, so
/// the export (descriptor, modifier, offset, stride) is checked as GTK
/// would use it. Tightly packed RGBA rows.
#[cfg(test)]
pub(super) fn read_through_import(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &Frame,
) -> Result<Vec<u8>> {
    let image = frame.image.as_ref().context("no image")?;
    let fd = image.fd.try_clone()?;
    let descriptor = wgpu::hal::TextureDescriptor {
        label: Some("imported dmabuf"),
        size: extent(image.width, image.height),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: WGPU_FORMAT,
        usage: wgpu::wgt::TextureUses::COPY_SRC,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    let hal_texture = {
        // SAFETY: used only in this block, while `device` is borrowed.
        let hal = unsafe { device.as_hal::<Vulkan>() }.context("not Vulkan")?;
        // SAFETY: `fd` is a dmabuf of this format and size, laid out with
        // the modifier, stride and offset Vulkan reported for it.
        unsafe {
            hal.texture_from_dmabuf_fd(
                fd,
                &descriptor,
                image.modifier,
                u64::from(image.stride),
                u64::from(image.offset),
            )
        }
        .map_err(|error| anyhow!("importing the dmabuf: {error}"))?
    };
    // SAFETY: made from this device's handle with this descriptor; its
    // contents are the frame's, already written.
    let texture = unsafe {
        device.create_texture_from_hal::<Vulkan>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("imported dmabuf"),
                size: extent(image.width, image.height),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WGPU_FORMAT,
                usage: wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            wgpu::wgt::TextureUses::UNINITIALIZED,
        )
    };
    let padded = (image.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dmabuf check"),
        size: u64::from(padded) * u64::from(image.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(image.height),
            },
        },
        extent(image.width, image.height),
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely())?;
    let mapped = buffer.slice(..).get_mapped_range()?;
    let row = usize::try_from(image.width)? * 4;
    Ok(mapped
        .chunks(usize::try_from(padded)?)
        .flat_map(|line| &line[..row])
        .copied()
        .collect())
}

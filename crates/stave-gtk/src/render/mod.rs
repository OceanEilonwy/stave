//! The 3D view's way to the screen. The drawing itself is `stave-render`'s;
//! this is how its frames reach GTK.
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
//!
//! A lost device is opened again here (`stave-render` only reports it):
//! the frame's scene is uploaded to the new one and the frame drawn there.

// The one module allowed `unsafe` code; see its own documentation.
#[allow(unsafe_code)]
mod dmabuf;
mod readback;

#[cfg(test)]
use dmabuf::Frame as DmabufFrame;
pub use dmabuf::display_modifiers;
pub use readback::{LiveFrame, MappedFrame, Ticket};
pub use stave_render::{Backdrop, Frame, Highlighted, Image, aspect, profile};

use anyhow::{Context as _, Result, bail};
use stave_render::Slot;
use stave_scene::SceneGraph;

/// `stave-render`'s renderer, with the live view's readback buffers and
/// dmabufs.
pub struct Renderer {
    /// Draws the frames.
    inner: stave_render::Renderer,
    /// The readback buffers and the frames in them.
    ring: readback::Ring,
    /// The DRM format modifiers GTK imports dmabufs of [`dmabuf::FOURCC`]
    /// with, as [`Renderer::use_dmabufs`] was given them (kept to make the
    /// exporter again on a new device).
    dmabuf_accepted: Vec<u64>,
    /// Makes the dmabufs the live view's frames go to GTK in; `None` when
    /// they go by readback.
    dmabuf: Option<dmabuf::Exporter>,
}

impl Renderer {
    /// Opens a GPU without a window. Fails when no adapter is available.
    ///
    /// It asks for the low-power adapter (the integrated one on a laptop
    /// with two); `WGPU_POWER_PREF=high` asks for the other.
    pub fn new() -> Result<Renderer> {
        Ok(Renderer {
            inner: pollster::block_on(stave_render::Renderer::headless())?,
            ring: readback::Ring::default(),
            dmabuf_accepted: Vec::new(),
            dmabuf: None,
        })
    }

    /// The adapter's name, for logs.
    pub fn adapter_name(&self) -> &str {
        self.inner.adapter_name()
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
        self.dmabuf = Some(dmabuf::Exporter::new(
            self.inner.device(),
            &self.dmabuf_accepted,
        )?);
        Ok(())
    }

    /// GTK refused a frame as a dmabuf (the window heard
    /// `Shown::Refused` from the 3D view): the live view's frames go
    /// by readback from now on.
    pub fn refuse_dmabufs(&mut self) {
        if let Some(exporter) = &mut self.dmabuf {
            exporter.refuse();
        }
    }

    /// Whether the live view's frames go to GTK as dmabufs.
    #[cfg(test)]
    pub fn uses_dmabufs(&self) -> bool {
        self.dmabuf.as_ref().is_some_and(dmabuf::Exporter::usable)
    }

    /// Uploads a scene's meshes, replacing the previous scene's. Only
    /// meshes the GPU doesn't hold already are written.
    pub fn upload(&mut self, scene: &SceneGraph) {
        self.inner.upload(scene);
    }

    /// Draws a frame and waits for it: for screenshots and still
    /// pictures. The live view uses [`Renderer::submit`] instead.
    ///
    /// When the device has been lost it is opened again, the frame's scene
    /// uploaded to it, and the frame drawn there; an error the GPU reports
    /// while drawing fails the frame instead of the application.
    pub fn render(&mut self, frame: &Frame<'_>) -> Result<Image> {
        self.recover_if_lost(frame.scene)?;
        match self.inner.render(frame) {
            // Lost while drawing: once more, on a new one.
            Err(_) if self.inner.is_lost() => {
                self.recover_if_lost(frame.scene)?;
                Ok(self.inner.render(frame)?)
            }
            image => Ok(image?),
        }
    }

    /// Draws a frame and returns at once, without waiting for the GPU
    /// (unless it is already [`readback`]'s limit of frames behind).
    /// [`Renderer::take_ready`] takes it once it has arrived.
    ///
    /// A lost device and errors are dealt with as by [`Renderer::render`].
    pub fn submit(&mut self, frame: &Frame<'_>) -> Result<Ticket> {
        if let Err(error) = self.ring.make_room(self.inner.device())
            && !self.inner.is_lost()
        {
            return Err(error);
        }
        self.recover_if_lost(frame.scene)?;
        let drawn = match self.draw_live(frame) {
            // Lost while drawing: once more, on a new one. Frames still on
            // the old one count as done.
            Err(_) if self.inner.is_lost() => {
                self.recover_if_lost(frame.scene)?;
                self.draw_live(frame)?
            }
            drawn => drawn?,
        };
        Ok(self.ring.push(self.inner.queue(), drawn))
    }

    /// The newest submitted frame that has arrived, if one has since the
    /// last call; older ones are passed over. Never waits. An error says
    /// why the newest frame to finish failed (an error mapping it, or the
    /// GPU taking longer than [`readback`]'s patience).
    pub fn take_ready(&mut self) -> Result<Option<LiveFrame>> {
        self.ring.take(self.inner.device())
    }

    /// Whether nothing more will come of `ticket`'s frame: it was taken,
    /// passed over for a newer one, or failed.
    pub fn settled(&self, ticket: Ticket) -> bool {
        self.ring.settled(ticket)
    }

    /// Draws a live frame and copies it into a readback buffer or a
    /// dmabuf.
    fn draw_live(&mut self, frame: &Frame<'_>) -> Result<readback::Drawn> {
        let layout = readback::Layout::new(frame.width.max(1), frame.height.max(1));
        let output = self.output(layout);
        let submitted = self
            .inner
            .draw(frame, Slot::Live, |encoder, resolved| match &output {
                readback::Output::Buffer(buffer) => encoder.copy_texture_to_buffer(
                    resolved.copy_source(),
                    wgpu::TexelCopyBufferInfo {
                        buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(layout.padded_row),
                            rows_per_image: Some(layout.height),
                        },
                    },
                    resolved.extent(),
                ),
                // The same bytes: the dmabuf's format is the target's
                // without the sRGB view, and copies don't convert.
                readback::Output::Dmabuf(frame) => {
                    if let Some(texture) = frame.texture() {
                        encoder.copy_texture_to_texture(
                            resolved.copy_source(),
                            texture.as_image_copy(),
                            resolved.extent(),
                        );
                    }
                }
            })?;
        let dmabuf = matches!(output, readback::Output::Dmabuf(_));
        Ok(readback::Drawn {
            output,
            layout,
            submission: submitted.submission,
            submitted: submitted.submitted,
            timings: submitted
                .timings
                .map(|timings| profile::FrameTimings { dmabuf, ..timings }),
        })
    }

    /// Opens the GPU again and uploads `scene` to it, when the device has
    /// been lost.
    fn recover_if_lost(&mut self, scene: &SceneGraph) -> Result<()> {
        if let Some(reason) = self.inner.lost() {
            eprintln!("stave: the graphics device was lost ({reason}); opening it again");
            self.inner = pollster::block_on(stave_render::Renderer::headless())
                .context("the graphics device was lost and couldn't be opened again")?;
            self.ring = readback::Ring::after(&self.ring);
            let accepted = std::mem::take(&mut self.dmabuf_accepted);
            if self.dmabuf.is_some()
                && let Err(error) = self.use_dmabufs(accepted)
            {
                eprintln!("stave: the 3D view's frames go by readback now: {error:#}");
            }
            self.inner.upload(scene);
        }
        Ok(())
    }

    /// Where a live frame of `layout` is copied for the window: a dmabuf
    /// when GTK takes them, otherwise a readback buffer. A dmabuf that
    /// can't be made puts the live view on readback for good.
    fn output(&mut self, layout: readback::Layout) -> readback::Output {
        if let Some(exporter) = &mut self.dmabuf {
            if exporter.usable() {
                match exporter.frame(self.inner.device(), layout.width, layout.height) {
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
        readback::Output::Buffer(self.ring.buffer(self.inner.device(), layout))
    }
}

/// Writes a picture to `path` as a PNG.
pub fn write_png(image: &Image, path: &std::path::Path) -> Result<()> {
    let bytes = stave_render::Renderer::encode_png(image)?;
    std::fs::write(path, bytes).with_context(|| format!("can't create '{}'", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use stave_scene::{Orbit, Posed};

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
            .inner
            .device()
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
        let seen =
            dmabuf::read_through_import(renderer.inner.device(), renderer.inner.queue(), &live)
                .unwrap();
        assert_eq!(seen, still.rgba);
    }

    #[test]
    fn still_pictures_go_by_readback_while_the_live_view_uses_dmabufs() {
        let Some(mut renderer) = dmabuf_renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        // Read back by `stave-render`, not through the live view's ring.
        let still = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        assert_eq!((still.width, still.height), (96, 64));
        assert_eq!(renderer.ring.made(), 0);
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
    fn frames_go_by_readback_once_gtk_refuses_a_dmabuf() {
        let Some(mut renderer) = dmabuf_renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        drop(wait_and_take_dmabuf(&mut renderer));
        // The window heard `Shown::Refused` and says so.
        renderer.refuse_dmabufs();
        assert!(!renderer.uses_dmabufs());
        renderer.submit(&frame(&scene, &posed, 0.0)).unwrap();
        assert!(
            matches!(
                wait_and_take_live(&mut renderer),
                Some(LiveFrame::Mapped(_))
            ),
            "the next frame goes by readback"
        );
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
        let seen =
            dmabuf::read_through_import(renderer.inner.device(), renderer.inner.queue(), &live)
                .unwrap();
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
        renderer.inner.device().destroy();
        // wgpu may say the device is lost at once or only after a frame on
        // it fails; either way a frame soon arrives from a new one, and
        // nothing is left waiting on the old.
        let mut shown = None;
        for _ in 0..3 {
            let Ok(after) = renderer.submit(&frame(&scene, &posed, 0.0)) else {
                continue;
            };
            assert!(before < after);
            let _ = renderer
                .inner
                .device()
                .poll(wgpu::PollType::wait_indefinitely());
            if let Ok(Some(frame)) = renderer.take_ready() {
                assert!(renderer.settled(after));
                shown = Some(frame);
                break;
            }
        }
        assert!(shown.is_some());
        assert!(renderer.settled(before));
        assert!(!renderer.inner.is_lost());
    }

    #[test]
    fn a_lost_device_is_opened_again_and_the_frame_drawn() {
        let Some(mut renderer) = renderer() else {
            return;
        };
        let (scene, posed) = stool();
        renderer.upload(&scene);
        let before = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        renderer.inner.device().destroy();
        let after = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
        assert_eq!(before.rgba.len(), after.rgba.len());
        assert!(!renderer.inner.is_lost());
    }
}

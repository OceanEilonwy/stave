//! Getting frames back from the GPU without stopping the main loop.
//!
//! Each frame is copied into a readback buffer, with each row padded to
//! wgpu's alignment, and the buffer is mapped without waiting. A later tick
//! polls the device and takes the newest frame that has arrived. The live
//! view hands GTK the mapped memory as it is: padding included, so it is
//! never copied again on the CPU. The buffer is unmapped and reused once
//! GTK lets go of the texture.
//!
//! At most [`IN_FLIGHT`] frames wait on the GPU at once. With the one GTK
//! shows and a spare or two, the buffers form a small ring that only grows
//! while GTK holds on to old frames.
//!
//! When frames can go to GTK as dmabufs ([`super::dmabuf`]), a frame is
//! copied into one instead of a buffer, and the ring waits for the GPU to
//! finish its submission instead of for a mapping; nothing is read back.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};

use super::{dmabuf, profile};

/// The most frames waiting on the GPU at once. Submitting another first
/// waits for the oldest, so a GPU that falls behind slows the view down
/// rather than piling up frames.
const IN_FLIGHT: usize = 2;

/// The most unused buffers kept for reuse.
const SPARE: usize = 3;

/// How long the GPU may take over a frame before it counts as failed, so a
/// hung device shows an error instead of freezing the window.
pub(super) const PATIENCE: Duration = Duration::from_secs(10);

/// A frame handed to the GPU, numbered in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ticket(u64);

/// How a frame's bytes lie in a readback buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Layout {
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
    /// Bytes from one row to the next: `width * 4`, rounded up to wgpu's
    /// copy alignment.
    pub padded_row: u32,
}

impl Layout {
    /// The layout of a `width` by `height` frame.
    pub fn new(width: u32, height: u32) -> Layout {
        Layout {
            width,
            height,
            padded_row: width
                .saturating_mul(4)
                .next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
        }
    }

    /// The buffer size the frame needs.
    pub fn bytes(self) -> u64 {
        u64::from(self.padded_row) * u64::from(self.height)
    }
}

/// Unused readback buffers, all unmapped. Shared with the frames GTK holds,
/// so a frame's buffer comes back when GTK drops its texture.
type Spares = Arc<Mutex<Vec<wgpu::Buffer>>>;

/// Keeps an unmapped buffer for reuse, unless enough are kept already.
fn give_back(spares: &Spares, buffer: wgpu::Buffer) {
    if let Ok(mut spares) = spares.lock()
        && spares.len() < SPARE
    {
        spares.push(buffer);
    }
}

/// Where a frame is copied for the window.
pub(super) enum Output {
    /// A readback buffer, mapped once the GPU is done.
    Buffer(wgpu::Buffer),
    /// A dmabuf GTK reads itself.
    Dmabuf(dmabuf::Frame),
}

/// A frame copied out and submitted.
pub(super) struct Drawn {
    /// Where the frame is being copied.
    pub output: Output,
    /// How it lies there.
    pub layout: Layout,
    /// The submission that copies it.
    pub submission: wgpu::SubmissionIndex,
    /// When it was submitted.
    pub submitted: Instant,
    /// The CPU side of drawing it, when timings are on.
    pub timings: Option<profile::FrameTimings>,
}

/// A frame whose buffer is being mapped.
struct Pending {
    ticket: Ticket,
    drawn: Drawn,
    mapped: Receiver<Result<(), wgpu::BufferAsyncError>>,
}

/// The readback buffers and the frames in them.
#[derive(Default)]
pub(super) struct Ring {
    spares: Spares,
    /// Submitted frames not yet mapped, oldest first.
    pending: VecDeque<Pending>,
    /// The newest mapped frame not yet taken.
    ready: Option<(Ticket, Drawn)>,
    /// Why the newest frame to finish failed, until it is reported.
    failed: Option<String>,
    /// The next ticket to hand out.
    next: u64,
    /// Every ticket below this has been shown, passed over for a newer
    /// frame, or failed.
    settled: u64,
    /// How many buffers were made, for tests of reuse.
    made: u64,
}

impl Ring {
    /// A ring whose tickets carry on from `previous`'s, after the device
    /// was opened again: `previous`'s frames are abandoned and count as
    /// settled.
    pub fn after(previous: &Ring) -> Ring {
        Ring {
            next: previous.next,
            settled: previous.next,
            ..Ring::default()
        }
    }

    /// A buffer for a frame of `layout`: a spare of the right size, or a
    /// new one. Spares of other sizes, from before the view was resized,
    /// are dropped.
    pub fn buffer(&mut self, device: &wgpu::Device, layout: Layout) -> wgpu::Buffer {
        let wanted = layout.bytes();
        if let Ok(mut spares) = self.spares.lock() {
            spares.retain(|buffer| buffer.size() == wanted);
            if let Some(buffer) = spares.pop() {
                return buffer;
            }
        }
        self.made += 1;
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: wanted,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }

    /// Keeps a buffer that was used and is unmapped.
    pub fn give_back(&self, buffer: wgpu::Buffer) {
        give_back(&self.spares, buffer);
    }

    /// Waits, at most [`PATIENCE`] each, until fewer than [`IN_FLIGHT`]
    /// frames are waiting on the GPU.
    pub fn make_room(&mut self, device: &wgpu::Device) -> Result<()> {
        while self.pending.len() >= IN_FLIGHT {
            let Some(oldest) = self.pending.front() else {
                break;
            };
            let waited = device.poll(wgpu::PollType::Wait {
                submission_index: Some(oldest.drawn.submission.clone()),
                timeout: Some(PATIENCE),
            });
            if let Err(error) = waited {
                // Give up on the oldest, so the ring can't stay full.
                self.fail_oldest(format!("waiting for the GPU: {error}"));
                return Err(anyhow!("the GPU didn't finish a frame: {error}"));
            }
            self.collect();
        }
        Ok(())
    }

    /// Starts mapping a drawn frame (or, for a dmabuf, waiting for its
    /// submission to finish), and numbers it.
    pub fn push(&mut self, queue: &wgpu::Queue, drawn: Drawn) -> Ticket {
        let ticket = Ticket(self.next);
        self.next += 1;
        let (sender, mapped) = std::sync::mpsc::channel();
        // The ring may be gone (the device was opened again); then nobody
        // waits for this frame.
        match &drawn.output {
            Output::Buffer(buffer) => {
                buffer
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        let _ = sender.send(result);
                    });
            }
            Output::Dmabuf(_) => {
                // Everything submitted so far, this frame last.
                queue.on_submitted_work_done(move || {
                    let _ = sender.send(Ok(()));
                });
            }
        }
        self.pending.push_back(Pending {
            ticket,
            drawn,
            mapped,
        });
        ticket
    }

    /// Whether `ticket`'s frame has been shown, passed over for a newer
    /// one, or failed: nothing more will come of it.
    pub fn settled(&self, ticket: Ticket) -> bool {
        ticket.0 < self.settled
    }

    /// Takes the newest frame that has arrived (polling the device without
    /// waiting), or says why the newest to finish failed. Older frames that
    /// arrived are passed over.
    pub fn take(&mut self, device: &wgpu::Device) -> Result<Option<LiveFrame>> {
        device
            .poll(wgpu::PollType::Poll)
            .context("checking on the GPU")?;
        self.collect();
        if let Some(failed) = self.failed.take() {
            return Err(anyhow!("the GPU couldn't draw the frame: {failed}"));
        }
        let Some((_, drawn)) = self.ready.take() else {
            return Ok(None);
        };
        let frame = match drawn.output {
            Output::Buffer(buffer) => {
                let view = match buffer.slice(..).get_mapped_range() {
                    Ok(view) => view,
                    Err(error) => {
                        buffer.unmap();
                        self.give_back(buffer);
                        return Err(anyhow!("couldn't read the frame back: {error}"));
                    }
                };
                LiveFrame::Mapped(MappedFrame {
                    layout: drawn.layout,
                    view: Some(view),
                    buffer: Some(buffer),
                    spares: self.spares.clone(),
                })
            }
            Output::Dmabuf(frame) => LiveFrame::Dmabuf(frame),
        };
        if let Some(mut timings) = drawn.timings {
            timings.gpu_wait = drawn.submitted.elapsed();
            timings.print();
        }
        Ok(Some(frame))
    }

    /// Lets go of a finished frame not shown: a mapped buffer is unmapped
    /// and kept, a dmabuf goes back to its pool.
    fn release(&self, output: Output) {
        match output {
            Output::Buffer(buffer) => {
                buffer.unmap();
                self.give_back(buffer);
            }
            Output::Dmabuf(frame) => drop(frame),
        }
    }

    /// Moves the frames that have finished, oldest first, out of
    /// `pending`: the newest mapped one becomes `ready`, older ones go back
    /// to the spares, and failures (an error, or longer than [`PATIENCE`])
    /// are noted.
    fn collect(&mut self) {
        while let Some(oldest) = self.pending.front() {
            let failure = match oldest.mapped.try_recv() {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(format!("couldn't read the frame back: {error}")),
                Err(TryRecvError::Disconnected) => Some("the frame was dropped".to_owned()),
                Err(TryRecvError::Empty) if oldest.drawn.submitted.elapsed() > PATIENCE => Some(
                    format!("it took longer than {} seconds", PATIENCE.as_secs()),
                ),
                Err(TryRecvError::Empty) => break,
            };
            match failure {
                Some(failure) => self.fail_oldest(failure),
                None => {
                    if let Some(Pending { ticket, drawn, .. }) = self.pending.pop_front() {
                        self.settled = ticket.0 + 1;
                        self.failed = None;
                        if let Some((_, older)) = self.ready.replace((ticket, drawn)) {
                            self.release(older.output);
                        }
                    }
                }
            }
        }
    }

    /// Drops the oldest pending frame as failed, with any older frame
    /// still waiting to be shown.
    fn fail_oldest(&mut self, failure: String) {
        if let Some(Pending { ticket, .. }) = self.pending.pop_front() {
            self.settled = ticket.0 + 1;
            self.failed = Some(failure);
            if let Some((_, older)) = self.ready.take() {
                self.release(older.output);
            }
        }
    }

    /// How many buffers have been made.
    #[cfg(test)]
    pub fn made(&self) -> u64 {
        self.made
    }
}

/// A finished frame of the live view.
#[derive(Debug)]
pub enum LiveFrame {
    /// In the GPU's mapped readback buffer.
    Mapped(MappedFrame),
    /// In a dmabuf, for GTK to read itself.
    Dmabuf(dmabuf::Frame),
}

/// A finished frame of the live view, still in the GPU's mapped readback
/// buffer: sRGB RGBA, rows top to bottom, each [`MappedFrame::stride`]
/// bytes apart, with alpha 1 everywhere (so straight and premultiplied
/// alike).
///
/// It is meant to be wrapped as it is in a texture (it is `AsRef<[u8]>`
/// and `Send`, as `glib::Bytes::from_owned` asks). Dropping it unmaps the
/// buffer and gives it back to the ring.
#[derive(Debug)]
pub struct MappedFrame {
    layout: Layout,
    /// `None` only while dropping.
    view: Option<wgpu::BufferView>,
    /// `None` only while dropping.
    buffer: Option<wgpu::Buffer>,
    spares: Spares,
}

impl MappedFrame {
    /// Pixels across.
    pub fn width(&self) -> u32 {
        self.layout.width
    }

    /// Pixels down.
    pub fn height(&self) -> u32 {
        self.layout.height
    }

    /// Bytes from the start of one row to the next.
    pub fn stride(&self) -> u32 {
        self.layout.padded_row
    }
}

impl AsRef<[u8]> for MappedFrame {
    fn as_ref(&self) -> &[u8] {
        self.view.as_deref().unwrap_or(&[])
    }
}

impl Drop for MappedFrame {
    fn drop(&mut self) {
        // The view must go before the buffer can be unmapped.
        drop(self.view.take());
        if let Some(buffer) = self.buffer.take() {
            buffer.unmap();
            give_back(&self.spares, buffer);
        }
    }
}

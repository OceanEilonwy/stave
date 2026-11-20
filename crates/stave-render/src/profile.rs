//! Timings of the 3D view, printed to standard error when the environment
//! variable `STAVE_PROFILE` is set (to anything but `0` or nothing).
//!
//! They stay in the build so the costs of a frame and of an edit can be
//! measured on any machine without a special build: each frame prints its
//! split (encoding the commands, submitting them, waiting for the GPU,
//! copying the pixels out, and making the GTK texture) and whether it went
//! to GTK as a dmabuf or by readback, and each check
//! prints how long it took from the edit until its first frame was shown.
//! With the variable unset nothing is timed and nothing is printed.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use web_time::Instant;

/// Whether `STAVE_PROFILE` asks for timings. Read once.
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("STAVE_PROFILE").is_some_and(|value| !value.is_empty() && value != "0")
    })
}

/// Now, when timings are on.
pub fn start() -> Option<Instant> {
    enabled().then(Instant::now)
}

/// Milliseconds, for a log line.
fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Prints how long something took since `started` (from [`start`]).
pub fn finish(what: &str, started: Option<Instant>) {
    if let Some(started) = started {
        eprintln!(
            "stave-profile: {what} {:.2} ms",
            milliseconds(started.elapsed())
        );
    }
}

/// Times `work` and prints it under `what` when timings are on.
pub fn time<T>(what: &str, work: impl FnOnce() -> T) -> T {
    let started = start();
    let value = work();
    finish(what, started);
    value
}

/// Where a frame's time went, on the CPU side of each step.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameTimings {
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
    /// Writing the uniforms and recording the passes.
    pub encode: Duration,
    /// Handing the commands to the GPU.
    pub submit: Duration,
    /// From handing the commands over until the frame was mapped (for
    /// the live view, until a tick found it mapped; the main loop runs
    /// meanwhile).
    pub gpu_wait: Duration,
    /// Copying the rows out of the mapped buffer without their padding;
    /// zero for the live view, which hands GTK the mapped buffer.
    pub readback: Duration,
    /// Whether the shadow map was drawn again (false: kept from before).
    pub shadow_drawn: bool,
    /// Whether the frame went to GTK as a dmabuf (false: by readback).
    pub dmabuf: bool,
}

impl FrameTimings {
    /// Prints the split on one line.
    pub fn print(&self) {
        eprintln!(
            "stave-profile: frame {}x{} encode {:.2} ms, submit {:.2} ms, gpu wait {:.2} ms, \
             readback {:.2} ms, shadow {}, via {}",
            self.width,
            self.height,
            milliseconds(self.encode),
            milliseconds(self.submit),
            milliseconds(self.gpu_wait),
            milliseconds(self.readback),
            if self.shadow_drawn { "drawn" } else { "kept" },
            if self.dmabuf { "dmabuf" } else { "readback" },
        );
    }
}

/// Where the edit whose result is awaited has got to.
#[derive(Debug, Clone, Copy)]
enum Edit {
    /// Its check is running; started then.
    Checking(Instant),
    /// Its result is in; the next frame shown is its first.
    Checked(Instant),
}

/// The edit being followed: set when a check starts, taken by the first
/// frame shown after its result.
static EDIT: Mutex<Option<Edit>> = Mutex::new(None);

/// Notes that a check has started for an edit (when timings are on). A
/// newer edit replaces an older one still waiting.
pub fn edit_started() {
    if enabled()
        && let Ok(mut edit) = EDIT.lock()
    {
        *edit = Some(Edit::Checking(Instant::now()));
    }
}

/// Notes that a check's result is in: prints how long the check took, and
/// lets the next frame shown count as the edit's.
pub fn edit_checked() {
    if !enabled() {
        return;
    }
    if let Ok(mut edit) = EDIT.lock()
        && let Some(Edit::Checking(started)) = *edit
    {
        eprintln!(
            "stave-profile: check {:.2} ms",
            milliseconds(started.elapsed())
        );
        *edit = Some(Edit::Checked(started));
    }
}

/// Notes that a frame is on screen: once an edit's result is in, prints
/// the latency from the edit to this frame and forgets the edit. Frames
/// drawn while the check still runs (an orbit) don't count.
pub fn frame_shown() {
    if !enabled() {
        return;
    }
    let started = EDIT.lock().ok().and_then(|mut edit| match *edit {
        Some(Edit::Checked(started)) => {
            *edit = None;
            Some(started)
        }
        Some(Edit::Checking(_)) | None => None,
    });
    if let Some(started) = started {
        eprintln!(
            "stave-profile: edit to view {:.2} ms",
            milliseconds(started.elapsed())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_returns_the_work_s_value() {
        assert_eq!(time("nothing", || 7), 7);
    }

    #[test]
    fn a_frame_s_split_prints_without_panicking() {
        FrameTimings {
            width: 4,
            height: 3,
            shadow_drawn: true,
            ..FrameTimings::default()
        }
        .print();
    }
}

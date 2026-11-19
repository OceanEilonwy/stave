//! One thread for every test that builds widgets: GTK lives on the
//! thread that started it, and tests run on threads of their own. Each
//! test hands its body to that thread and waits; a failed assertion
//! there fails the test. Without a display (no desktop session, no
//! Broadway) the bodies are skipped, saying so.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send>;
type Done = Sender<std::thread::Result<()>>;
/// Where jobs are handed to the GTK thread.
type Jobs = Mutex<Sender<(Job, Done)>>;

static GTK: OnceLock<Option<Jobs>> = OnceLock::new();

/// Runs `body` on the GTK thread and waits for it. `false` when GTK can't
/// start here, and the body didn't run.
pub(crate) fn on_gtk_thread(body: impl FnOnce() + Send + 'static) -> bool {
    let jobs = GTK.get_or_init(|| {
        let (ready, started) = channel();
        let (jobs, received) = channel::<(Job, Done)>();
        std::thread::spawn(move || {
            let up = gtk::init().is_ok() && adw::init().is_ok();
            let _ = ready.send(up);
            if !up {
                return;
            }
            // The simple input method, not the desktop's: on Wayland,
            // GTK's text-input listener looks up the focused entry's
            // display, and an event still queued for an entry whose window
            // an earlier test destroyed reaches a freed widget in the next
            // test's main loop (SIGSEGV in `gtk_widget_get_display`). No
            // test exercises an input method, so none is listened to.
            if let Some(settings) = gtk::Settings::default() {
                settings.set_gtk_im_module(Some("gtk-im-context-simple"));
            }
            for (job, done) in received {
                let _ = done.send(catch_unwind(AssertUnwindSafe(job)));
            }
        });
        started
            .recv()
            .ok()
            .filter(|up| *up)
            .map(|_| Mutex::new(jobs))
    });
    let Some(jobs) = jobs else {
        eprintln!("skipped: GTK can't start here (no display)");
        return false;
    };
    let (done, finished) = channel();
    jobs.lock()
        .expect("the GTK thread's queue")
        .send((Box::new(body), done))
        .expect("the GTK thread is running");
    match finished.recv().expect("the GTK thread answers") {
        Ok(()) => true,
        Err(panic) => resume_unwind(panic),
    }
}

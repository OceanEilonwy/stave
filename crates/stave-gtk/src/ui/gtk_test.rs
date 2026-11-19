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

/// The buttons under `root` (itself included) that show no words and have
/// no accessible name, by their tooltip: a screen reader would call each
/// just "button". A menu button counts as one button, not its inner one.
pub(crate) fn unnamed_buttons(root: &impl gtk::prelude::IsA<gtk::Widget>) -> Vec<String> {
    use gtk::prelude::*;

    fn shows_words(widget: &gtk::Widget) -> bool {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            return !label.label().is_empty();
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            if shows_words(&next) {
                return true;
            }
            child = next.next_sibling();
        }
        false
    }

    fn walk(widget: &gtk::Widget, unnamed: &mut Vec<String>) {
        let menu = widget.is::<gtk::MenuButton>();
        if (menu || widget.is::<gtk::Button>())
            && !shows_words(widget)
            && !gtk::test_accessible_has_property(widget, gtk::AccessibleProperty::Label)
        {
            unnamed.push(
                widget
                    .tooltip_text()
                    .map_or_else(|| widget.type_().name().to_owned(), String::from),
            );
        }
        if menu {
            return;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            walk(&next, unnamed);
            child = next.next_sibling();
        }
    }

    let mut unnamed = Vec::new();
    walk(root.as_ref(), &mut unnamed);
    unnamed
}

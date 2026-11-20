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
                // The font and resolution `tools/window-shot` lays the
                // window out with (GNOME's default), so tests that measure
                // text read the same on a desktop and on a private
                // Broadway display, which has no desktop settings.
                settings.set_gtk_font_name(Some("Adwaita Sans 11"));
                settings.set_gtk_xft_dpi(96 * 1024);
            }
            // What the application puts on the display at startup that
            // the windows tests build rely on: the grips' sizes.
            if let Some(display) = gtk::gdk::Display::default() {
                crate::ui::edit_view::grip_panel::install_style(&display);
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

/// Runs the GTK thread's main loop until nothing is waiting. Call it on
/// the GTK thread, from a test body: relm4 components handle their queued
/// messages, pass on their outputs and shut down after their controller
/// is dropped only when the main loop runs. Stops after a bound, so a
/// timer that is always due can't keep a test here forever.
pub(crate) fn settle() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..10_000 {
        if !context.iteration(false) {
            break;
        }
    }
}

/// Settles until `done` holds, or `limit` has passed: `true` when it
/// holds. For what finishes on a later frame or timer, not when the main
/// loop is next idle: [`settle`] returns as soon as nothing is due, and
/// an animation (libadwaita's dialogs fade out even when force-closed,
/// holding the dialog until the last frame) runs on the frame clock's
/// time, so it may still be running then, more so on a busy machine.
pub(crate) fn settle_until(limit: std::time::Duration, done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + limit;
    loop {
        settle();
        if done() {
            return true;
        }
        if std::time::Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
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

/// A sender for a component's outputs (forwarded at launch, as the
/// window forwards them), and what reached it: each message is kept, in
/// order, once the main loop has run (`settle()`).
pub(crate) fn recorder<T: 'static>() -> (relm4::Sender<T>, std::rc::Rc<std::cell::RefCell<Vec<T>>>)
{
    let (sender, receiver) = relm4::channel::<T>();
    let heard = std::rc::Rc::<std::cell::RefCell<Vec<T>>>::default();
    let log = std::rc::Rc::clone(&heard);
    relm4::spawn_local(async move {
        while let Some(message) = receiver.recv().await {
            log.borrow_mut().push(message);
        }
    });
    (sender, heard)
}

/// Handles each message `receiver` gets with `handle`, in order, as the
/// owner of a component would in its update: from the main loop, once
/// the message's turn comes (`settle()`).
pub(crate) fn handle_each<T: 'static>(
    receiver: relm4::Receiver<T>,
    mut handle: impl FnMut(T) + 'static,
) {
    relm4::spawn_local(async move {
        while let Some(message) = receiver.recv().await {
            handle(message);
        }
    });
}

/// A sender whose messages go to `on_message`, one at a time, when the
/// main loop runs (`settle()`): what a test gives a component to forward
/// its outputs to, as an owner gives its own input. `on_message` may do
/// what an owner's handler would (send setters, fire signals); it is
/// freed once every clone of the sender has gone.
pub(crate) fn listener<T: 'static>(on_message: impl FnMut(T) + 'static) -> relm4::Sender<T> {
    let (sender, receiver) = relm4::channel::<T>();
    handle_each(receiver, on_message);
    sender
}

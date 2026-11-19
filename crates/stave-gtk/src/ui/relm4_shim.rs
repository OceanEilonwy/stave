//! The bridge between a relm4 component and the window's older
//! callback-style code, while the window moves to relm4 one part at a
//! time (docs/relm4-migration.md, "The shim pattern").
//!
//! A [`Bridge`] owns its component's [`Controller`]: dropping the bridge
//! shuts the component down the next time the main loop runs, which
//! frees its model and widgets. The old setters become input messages
//! with [`Bridge::send`]; the component's outputs go to the callback
//! connected last with [`Bridge::connect`].
//!
//! Outputs reach the callback from a future of their own on the main
//! loop, never from inside the component's `update`, so a callback may
//! call back into the bridge (send, read the model, connect again)
//! without re-entering the component. Nothing here holds a borrow while
//! it calls GTK or a callback: the callback sits in a [`Cell`] and is
//! cloned out before it is called.

use std::cell::{Cell, Ref};
use std::rc::Rc;

use relm4::{Component, ComponentController, Controller};

/// A callback for a component's outputs.
type OutputCallback<Output> = Rc<dyn Fn(Output)>;

/// Where a component's outputs go: the callback connected last, if any.
struct OutputSlot<Output> {
    callback: Cell<Option<OutputCallback<Output>>>,
}

impl<Output> OutputSlot<Output> {
    /// Calls the callback with `output`, or drops it when none is
    /// connected (as the old callback slots did).
    fn deliver(&self, output: Output) {
        // Cloned out, so the callback is free to connect another.
        let callback = self.callback.take();
        self.callback.set(callback.clone());
        if let Some(callback) = callback {
            callback(output);
        }
    }
}

/// A launched relm4 component behind an old-style API: the one owner of
/// its [`Controller`].
pub(crate) struct Bridge<C: Component> {
    controller: Controller<C>,
    outputs: Rc<OutputSlot<C::Output>>,
}

impl<C: Component> Bridge<C> {
    /// Launches the component with `init`. Its outputs are dropped until
    /// [`Bridge::connect`] gives them somewhere to go.
    pub(crate) fn launch(init: C::Init) -> Bridge<C> {
        let outputs = Rc::new(OutputSlot {
            callback: Cell::new(None),
        });
        // The forwarding future ends when the component's output sender
        // goes: after it shuts down and its widget's handlers are freed.
        let forward = Rc::clone(&outputs);
        let controller = C::builder()
            .launch(init)
            .connect_receiver(move |_, output| forward.deliver(output));
        Bridge {
            controller,
            outputs,
        }
    }

    /// The component's root widget.
    pub(crate) fn widget(&self) -> &C::Root {
        self.controller.widget()
    }

    /// Queues `input` for the component; it is handled when the main loop
    /// next runs, after the caller has returned.
    pub(crate) fn send(&self, input: C::Input) {
        // Sending fails only once the component has shut down, when there
        // is nothing left to tell.
        let _ = self.controller.sender().send(input);
    }

    /// Calls `callback` with each output from now on, instead of the one
    /// connected before.
    pub(crate) fn connect(&self, callback: impl Fn(C::Output) + 'static) {
        self.outputs.callback.set(Some(Rc::new(callback)));
    }

    /// The component's model, as of the last message it handled.
    ///
    /// relm4 borrows the model mutably while `update` runs, so this must
    /// not be called from a handler of the component's own widgets (a
    /// signal GTK emits synchronously during `update`). From an output
    /// callback, a timeout or another component's handler it is safe.
    pub(crate) fn model(&self) -> Ref<'_, C> {
        self.controller.model()
    }

    /// The component's input sender, to watch it close in tests.
    #[cfg(test)]
    pub(crate) fn sender(&self) -> &relm4::Sender<C::Input> {
        self.controller.sender()
    }
}

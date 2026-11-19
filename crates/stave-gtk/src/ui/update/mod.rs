//! The project window's behaviour, in groups. Each group has its own
//! input messages (wrapped by the window's [`WindowInput`]), its own
//! handlers (`impl Window` blocks in its file, and for editing the
//! `edit_view` files), and a `wire` function, called from the window's
//! `init`, that turns its leaves' outputs, its actions and its widgets'
//! signals into those messages.
//!
//! - [`selection`]: what is selected and focused, focus mode, posed
//!   problems, the motion sliders and the finder.
//! - [`project`]: the folder, checks, the editor's files, saving, closing
//!   and the command line's wishes.
//! - [`editing`]: editing in the 3D view (the menu, the card, grips,
//!   changes, holds, repeats and the finder's verbs).
//! - [`rendering`]: the 3D view's camera, drawing, pins, pulling apart and
//!   exporting.

use std::fmt;

use adw::prelude::*;
use gtk::gio;

use super::WindowInput;

pub(super) mod editing;
pub(super) mod project;
pub(super) mod rendering;
pub(super) mod selection;

/// A message's payload that has no `Debug` of its own: printed as its
/// type's name.
pub(in crate::ui) struct Unprinted<T>(pub(in crate::ui) T);

impl<T> fmt::Debug for Unprinted<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Unprinted<{}>", std::any::type_name::<T>())
    }
}

/// Adds a window action that sends `message` when activated.
pub(in crate::ui) fn add_action<Message: Into<WindowInput>>(
    window: &adw::ApplicationWindow,
    name: &str,
    input: &relm4::Sender<WindowInput>,
    message: impl Fn() -> Message + 'static,
) {
    let action = gio::SimpleAction::new(name, None);
    let input = input.clone();
    action.connect_activate(move |_, _| input.emit(message().into()));
    window.add_action(&action);
}

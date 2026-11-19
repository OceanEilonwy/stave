//! What a link in the Inspector or the Edit panel does (`docs/gui.md`,
//! "Details"): a click on a part, a piece of hardware or an assembly
//! selects it; a click on a space lights it. While the pointer is on a
//! link, what it names is lit in the 3D view: a part or a piece of
//! hardware is outlined as a selection is, a space is lit as the pointer
//! in it lights it.

use stave_app::inspect::LinkTarget;
use stave_app::session::{Command, Selection};

use super::super::Window;
use super::super::sections::LinkHandlers;
use crate::ui::update::editing::EditingInput;

/// What a link the pointer is on lights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Hovered {
    /// An item, outlined.
    Item(Selection),
    /// A space, lit (it is put in the edit view's hovered space).
    Space,
}

impl Window {
    /// Connects a panel's links: a click selects, the pointer lights.
    pub(in crate::ui) fn connect_links(&self, links: &LinkHandlers) {
        let input = self.input();
        links.connect_click(move |target| input.emit(EditingInput::LinkClicked(target).into()));
        let input = self.input();
        links.connect_hover(move |target| input.emit(EditingInput::LinkHovered(target).into()));
    }

    /// A link was clicked: an item is selected, a space lit.
    pub(in crate::ui) fn link_clicked(&mut self, target: LinkTarget) {
        match target {
            LinkTarget::Select(selection) => {
                self.editing.state.link_hover = None;
                self.dispatch(Command::Select(Some(selection)));
            }
            LinkTarget::Space(region) => self.hover_region(region),
        }
    }

    /// The pointer is on a link (or left one): what it names is lit.
    pub(in crate::ui) fn link_hovered(&mut self, target: Option<LinkTarget>) {
        {
            let state = &mut self.editing.state;
            if state.link_hover == Some(Hovered::Space) {
                state.hover = None;
            }
            state.link_hover = match target {
                Some(LinkTarget::Select(selection)) => Some(Hovered::Item(selection)),
                Some(LinkTarget::Space(region)) => {
                    state.hover = Some(region);
                    Some(Hovered::Space)
                }
                None => None,
            };
        }
        self.request_render();
    }
}

impl super::EditView {
    /// What the link under the pointer lights, if anything: the frame
    /// lists outline the item it names with the selection.
    pub(in crate::ui) fn link_hover(&self) -> Option<&Hovered> {
        self.state.link_hover.as_ref()
    }
}

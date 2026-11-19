//! What a link in the Inspector or the Edit panel does (`docs/gui.md`,
//! "Details"): a click on a part, a piece of hardware or an assembly
//! selects it; a click on a space lights it. While the pointer is on a
//! link, what it names is lit in the 3D view: a part or a piece of
//! hardware is outlined as a selection is, a space is lit as the pointer
//! in it lights it.

use std::rc::Rc;

use stave_app::inspect::LinkTarget;
use stave_app::session::{Command, Selection};
use stave_scene::NodeId;

use super::super::Window;
use super::super::sections::LinkHandlers;

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
    pub(in crate::ui) fn connect_links(self: &Rc<Self>, links: &LinkHandlers) {
        let window = Rc::downgrade(self);
        links.connect_click(move |target| {
            let Some(window) = window.upgrade() else {
                return;
            };
            match target {
                LinkTarget::Select(selection) => {
                    window.editing.state.borrow_mut().link_hover = None;
                    window.dispatch(Command::Select(Some(selection)));
                }
                LinkTarget::Space(region) => window.hover_region(region),
            }
        });
        let window = Rc::downgrade(self);
        links.connect_hover(move |target| {
            let Some(window) = window.upgrade() else {
                return;
            };
            {
                let mut state = window.editing.state.borrow_mut();
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
            window.request_render();
        });
    }

    /// The nodes the link under the pointer names, to outline with the
    /// selection.
    pub(in crate::ui) fn link_hover_nodes(&self) -> Vec<NodeId> {
        let Some(Hovered::Item(selection)) = self.editing.state.borrow().link_hover.clone() else {
            return Vec::new();
        };
        let session = self.session.borrow();
        let Some((_, graph)) = session.drawable() else {
            return Vec::new();
        };
        selection
            .item()
            .into_iter()
            .flat_map(|item| graph.nodes_of(item).into_iter().take(1))
            .collect()
    }
}

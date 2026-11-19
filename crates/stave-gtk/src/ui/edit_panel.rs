//! The Edit panel of the docked window (`docs/gui.md`, "Editing in the 3D
//! view"): the card for a chosen candidate, the card for a change of the
//! design, or the hold card, whichever is in progress, docked as any other
//! panel is (a tab, dragged to any area, resized by the splitters, its
//! place remembered with the rest of the layout). While nothing is in
//! progress it stays where it is and says how to start.

use adw::prelude::*;

/// The panel's content.
pub struct EditPanel {
    /// What the workbench docks.
    pub root: gtk::Stack,
    cards: gtk::Box,
}

impl EditPanel {
    /// The panel, saying how to start an edit, with these cards in it
    /// (each shown by its owner while it is in progress, one at a time).
    pub fn new(cards: &[&gtk::Widget]) -> EditPanel {
        let empty = adw::StatusPage::builder()
            .icon_name("document-edit-symbolic")
            .title("Nothing being added")
            .description(
                "Click inside a space, or in the gap between two parts, to add something there; or select a part and choose “Add something onto its face” in Details. Select a part or an assembly to change it in Details.",
            )
            .css_classes(["compact", "stave-edit-empty"])
            .build();
        let holder = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .vexpand(true)
            .hexpand(true)
            .build();
        for card in cards {
            card.set_vexpand(true);
            holder.append(*card);
        }
        let root = gtk::Stack::builder().hexpand(true).vexpand(true).build();
        root.add_named(&empty, Some("empty"));
        root.add_named(&holder, Some("card"));
        root.set_visible_child_name("empty");
        EditPanel {
            root,
            cards: holder,
        }
    }

    /// Shows the card in progress, or how to start when none is. `true`
    /// when a card shows.
    pub fn update(&self) -> bool {
        let mut child = self.cards.first_child();
        let mut any = false;
        while let Some(card) = child {
            any |= card.is_visible();
            child = card.next_sibling();
        }
        self.root
            .set_visible_child_name(if any { "card" } else { "empty" });
        any
    }

    /// `true` while it says how to start (for tests).
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.root.visible_child_name().as_deref() == Some("empty")
    }
}

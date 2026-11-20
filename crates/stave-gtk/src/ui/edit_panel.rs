//! The Edit panel of the docked window (`docs/gui.md`, "Editing in the 3D
//! view"): the Sizes card of the selection, over the card for a chosen
//! candidate, the card for a change of the design (with a size's
//! section), or the hold card, whichever is in progress, each drawn as
//! the Inspector is, with Cancel (Esc) and Apply (Enter) at its foot,
//! docked as any other
//! panel is (a tab, dragged to any area, resized by the splitters, its
//! place remembered with the rest of the layout). While nothing is in
//! progress it stays where it is and says how to start.

use adw::prelude::*;
use stave_app::i18n::tr;

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
            .title(tr("Nothing being added"))
            .description(tr(
                "Click inside a space, or in the gap between two parts, to add something there. To add onto a part, or change it, select it and use Details.",
            ))
            .css_classes(["compact", "stave-edit-empty"])
            .build();
        // The panel's content is put into the dock after it is docked, so
        // the workbench doesn't find this page to fit.
        super::empty_state::fit(&empty);
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

    /// Puts the Sizes card at the top of the panel, above the cards in
    /// progress, as tall as its rows.
    pub fn set_sizes(&self, sizes: &gtk::Widget) {
        sizes.set_vexpand(false);
        self.cards.prepend(sizes);
    }

    /// Shows the cards that show (the Sizes card, the card in progress),
    /// or how to start when none does. `true` when a card shows.
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

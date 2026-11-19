//! What the card of a selected item offers (`docs/editing.md`, "Changes
//! of the design"): which item of the project the selection changes, how
//! many pieces it is, whether a repeat can fill its space, and what "And
//! the other side" does for it. The window shows these answers and sends
//! the command they name; it decides none of them.
//!
//! ```text
//! selection ──selected_item──▶ "door"
//! "door" ──item_offer──▶ ItemOffer { pieces: 1, repeatable: true,
//!                                    mirror: Some(Split(Across)) }
//! ```

use std::path::Path;

use stave_lang::placement::Axis;
use stave_model::solved::SolvedDesign;
use stave_modules::FileSource;

use crate::Opened;
use crate::candidates::EditScope;
use crate::change::{MirrorOffer, mirror_offer};
use crate::editing::EditCommand;
use crate::session::Selection;

/// What "And the other side" does for an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorGesture {
    /// The design has the item's mirror image ready (it is placed
    /// against parts that are mirrored): adds it
    /// ([`EditCommand::Mirror`]).
    Image(MirrorOffer),
    /// The item is one handed piece filling a whole space (a door hinged
    /// on the left side): splits the space along the axis into a mirrored
    /// pair ([`EditCommand::MirrorSplit`]).
    Split(Axis),
}

impl MirrorGesture {
    /// The command that carries the gesture out for `item`.
    pub fn command(&self, item: stave_lang::ast::Ident) -> EditCommand {
        match self {
            MirrorGesture::Image(_) => EditCommand::Mirror(item),
            MirrorGesture::Split(axis) => EditCommand::MirrorSplit { item, axis: *axis },
        }
    }
}

/// What a selected item's card offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemOffer {
    /// How many pieces the item is: the elements of its array, or one.
    pub pieces: usize,
    /// `true` when the repeat controls apply: it is already pieces of a
    /// repeat, or one piece filling a space.
    pub repeatable: bool,
    /// What "And the other side" does; `None` when it offers nothing: no
    /// mirror image and not a handed piece filling a space, or it already
    /// has its mirror image (or is one).
    pub mirror: Option<MirrorGesture>,
}

/// How the repeat controls show on a card (`docs/editing.md`, "Repeat and
/// mirror"): one rule for every component and every item, never a
/// particular one's. A repeat is valid wherever something fills a space
/// (two cases side by side in the envelope), but rarely wanted, so the
/// controls take no room until asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatFold {
    /// No repeat applies: it fills no space and is no repeat.
    Hidden,
    /// Folded behind a small "Repeat…" control.
    Folded,
    /// The count and the direction show.
    Open,
}

/// The repeat rule: hidden when no repeat applies; open when it is already
/// more than one piece, or the user asked (pressed "Repeat…"); folded
/// otherwise.
pub fn repeat_fold(applies: bool, pieces: usize, asked: bool) -> RepeatFold {
    match (applies, pieces > 1 || asked) {
        (false, _) => RepeatFold::Hidden,
        (true, true) => RepeatFold::Open,
        (true, false) => RepeatFold::Folded,
    }
}

/// `true` when a chosen candidate can be repeated: a component whose
/// `Region` place param is bound, so its copies split that space.
pub fn candidate_repeatable(candidate: &crate::edit::Candidate) -> bool {
    crate::editing::is_component(candidate)
        && candidate
            .place
            .iter()
            .any(|param| matches!(param.slot, crate::edit::PlaceSlot::Region(Some(_))))
}

/// The repeat rule for a chosen candidate, with the repeat it holds
/// (`None` for one piece) and whether the user asked for the controls.
pub fn candidate_repeat_fold(
    candidate: &crate::edit::Candidate,
    repeat: Option<crate::candidates::Repeat>,
    asked: bool,
) -> RepeatFold {
    repeat_fold(
        candidate_repeatable(candidate),
        repeat.map_or(1, |repeat| repeat.count.get()),
        asked,
    )
}

impl ItemOffer {
    /// The repeat rule for a selected item: its pieces and whether the
    /// user asked for the controls.
    pub fn repeat_fold(&self, asked: bool) -> RepeatFold {
        repeat_fold(self.repeatable, self.pieces, asked)
    }
}

/// The name of the item of the project a name in the design belongs to:
/// the outermost assembly or the part itself, without its index
/// (`drawer[top].front` → `drawer`, `case.side[left]` → `case`,
/// `foot[2]` → `foot`).
pub fn item_base(name: &str) -> &str {
    let end = name.find(['.', '[', ' ', '\'']).unwrap_or(name.len());
    name.get(..end).unwrap_or(name)
}

/// The item of the project a selection belongs to, by name: what is
/// renamed, repeated, mirrored or removed when the selection is. `None`
/// for a joint, a member, a location or a problem.
pub fn selected_item(design: &SolvedDesign, selection: &Selection) -> Option<String> {
    let outermost = |mut id| {
        while let Some(parent) = design
            .assemblies()
            .iter()
            .find(|assembly| assembly.id == id)
            .and_then(|assembly| assembly.parent)
        {
            id = parent;
        }
        design
            .assemblies()
            .iter()
            .find(|assembly| assembly.id == id)
    };
    let name = match selection {
        Selection::Assembly(id) => outermost(*id)?.name.clone(),
        Selection::Part(part) => match design
            .assemblies()
            .iter()
            .find(|assembly| assembly.parent.is_none() && assembly.parts.contains(part))
        {
            Some(assembly) => assembly.name.clone(),
            None => design.part(*part)?.name.clone(),
        },
        Selection::Hardware(hardware) => match design
            .assemblies()
            .iter()
            .find(|assembly| assembly.parent.is_none() && assembly.hardware.contains(hardware))
        {
            Some(assembly) => assembly.name.clone(),
            None => design
                .hardware()
                .iter()
                .find(|item| item.id == *hardware)?
                .name
                .clone(),
        },
        Selection::Joint { .. }
        | Selection::Member(_)
        | Selection::Location(_)
        | Selection::Problem(_) => return None,
    };
    Some(item_base(&name).to_owned())
}

/// How many pieces an item of the project is: the elements of its array,
/// or one.
pub fn piece_count(design: &SolvedDesign, item: &str) -> usize {
    let mut names: Vec<&str> = design
        .assemblies()
        .iter()
        .filter(|assembly| assembly.parent.is_none())
        .map(|assembly| assembly.name.as_str())
        .chain(design.parts().iter().map(|part| part.name.as_str()))
        .filter(|name| {
            item_base(name) == item
                && name
                    .get(item.len()..)
                    .is_some_and(|rest| !rest.contains('.'))
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    names.len().max(1)
}

/// What the card of `item` offers in the project `opened` (read from
/// `folder` through `files`). `None` when the project has no design.
pub fn item_offer(
    folder: &Path,
    files: &dyn FileSource,
    opened: &Opened,
    item: &str,
) -> Option<ItemOffer> {
    let design = opened.design.as_ref()?;
    let model = opened.model.as_ref()?;
    let pieces = piece_count(design, item);
    let fills = !crate::spaces::filled_spaces(model, item).is_empty();
    let mirror = match mirror_offer(folder, files, opened, item) {
        Some(offer) => Some(MirrorGesture::Image(offer)),
        None => {
            let axis = Axis::Across;
            let scope = EditScope::new(opened).ok()?;
            let split = pieces < 2
                && fills
                && crate::repeat::mirror_image_of(&scope, item).is_none()
                && crate::repeat::handed(&scope, design, item, axis);
            split.then_some(MirrorGesture::Split(axis))
        }
    };
    Some(ItemOffer {
        pieces,
        repeatable: pieces > 1 || fills,
        mirror,
    })
}

#[cfg(test)]
mod tests {
    use super::{ItemOffer, RepeatFold, item_base, repeat_fold};

    #[test]
    fn the_repeat_controls_are_folded_unless_asked_or_already_repeated() {
        assert_eq!(repeat_fold(false, 1, true), RepeatFold::Hidden);
        assert_eq!(repeat_fold(true, 1, false), RepeatFold::Folded);
        assert_eq!(repeat_fold(true, 1, true), RepeatFold::Open);
        assert_eq!(repeat_fold(true, 3, false), RepeatFold::Open);
        let offer = ItemOffer {
            pieces: 2,
            repeatable: true,
            mirror: None,
        };
        assert_eq!(offer.repeat_fold(false), RepeatFold::Open);
        let one = ItemOffer { pieces: 1, ..offer };
        assert_eq!(one.repeat_fold(false), RepeatFold::Folded);
    }

    #[test]
    fn an_item_is_named_by_its_outermost_name_without_an_index() {
        assert_eq!(item_base("drawer[top].front"), "drawer");
        assert_eq!(item_base("case.side[left]"), "case");
        assert_eq!(item_base("foot[2]"), "foot");
        assert_eq!(item_base("door_l"), "door_l");
        assert_eq!(item_base("top_dowels's pin between a and b"), "top_dowels");
    }
}

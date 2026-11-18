//! What the pointer picks for editing, beyond the scene's own
//! [`SceneGraph::pick_target`]: a click that binds a place param asks for
//! the kind of thing the param holds, and a design with spaces but no
//! parts yet (a new project's envelope) is still drawn and framed, so its
//! first space can be clicked.

use stave_app::edit::{Picked, PlaceSlot};
use stave_app::editing::{Edit, picked_from_facing, picked_from_part_face};
use stave_app::empty::{EmptyState, Reason};
use stave_app::session::Session;
use stave_scene::{Aabb, Posed, Ray, SceneGraph, Target};

/// What the place params still to point at can hold.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct Wanted {
    /// A `Region`: a click in a space.
    pub(in crate::ui) region: bool,
    /// An `Anchor` or a `Part`: a click on a part's face.
    pub(in crate::ui) face: bool,
    /// `Parts`: a click between two facing parts (or on one part).
    pub(in crate::ui) parts: bool,
}

impl Wanted {
    /// What the chosen candidate of an edit still asks to be pointed at;
    /// nothing when no candidate is chosen or every place is bound.
    pub(in crate::ui) fn of(edit: Option<&Edit>) -> Wanted {
        let mut wanted = Wanted::default();
        let Some(chosen) = edit.and_then(Edit::chosen) else {
            return wanted;
        };
        for param in chosen.candidate().unbound_place() {
            match param.slot {
                PlaceSlot::Region(_) => wanted.region = true,
                PlaceSlot::Anchor(_) | PlaceSlot::Part(_) => wanted.face = true,
                PlaceSlot::Parts(_) => wanted.parts = true,
            }
        }
        wanted
    }

    /// `true` when something is still to be pointed at.
    pub(in crate::ui) fn any(self) -> bool {
        self.region || self.face || self.parts
    }
}

/// What a click binds while the card asks for a place. A space wins over
/// its own walls only when a `Region` is wanted; otherwise the click is
/// on the part the ray meets, even through a space (the hinge side is a
/// wall of the bay, and its inside face can only be reached through the
/// bay), or, for `Parts` alone, on the two parts facing across the gap.
pub(in crate::ui) fn binding_pick(
    graph: &SceneGraph,
    posed: &Posed,
    ray: Ray,
    wanted: Wanted,
) -> Option<Picked> {
    let face = || {
        graph
            .pick(posed, ray)
            .and_then(|hit| hit.part_face())
            .map(picked_from_part_face)
    };
    let between = || {
        graph
            .facing(posed, ray)
            .as_ref()
            .and_then(picked_from_facing)
    };
    if wanted.region
        && let Some(Target::Region(hit)) = graph.pick_target(posed, ray)
    {
        return Some(Picked::Region(hit.region));
    }
    if wanted.face {
        return face().or_else(|| wanted.parts.then(between).flatten());
    }
    if wanted.parts {
        return between().or_else(face);
    }
    None
}

/// The box a camera frames: every node drawn, or, with no nodes yet,
/// every space (a new project's envelope).
pub(in crate::ui) fn drawn_bounds(graph: &SceneGraph, posed: &Posed) -> Option<Aabb> {
    graph.bounds(posed).or_else(|| {
        graph
            .regions()
            .iter()
            .filter_map(|region| graph.region_volume(posed, region.id))
            .reduce(Aabb::union)
    })
}

/// Why the 3D view has nothing to show, if it hasn't: a design with no
/// parts but a space to click (a new project's envelope) is drawn, so
/// the first piece can be added by clicking in it.
pub(in crate::ui) fn empty_state(session: &Session) -> Option<EmptyState> {
    let empty = stave_app::empty::of(session)?;
    let has_space = matches!(empty.reason, Reason::NoParts)
        && session
            .drawable()
            .is_some_and(|(_, graph)| !graph.regions().is_empty());
    (!has_space).then_some(empty)
}

/// `true` for a design of spaces with no parts yet: a new project, its
/// envelope the first space to click.
pub(in crate::ui) fn only_spaces(graph: &SceneGraph) -> bool {
    graph.node_ids().next().is_none() && !graph.regions().is_empty()
}

/// The spaces a design with no parts shows while nothing is edited, so
/// there is a box to click in; none once there are parts or an edit.
pub(in crate::ui) fn spaces_waiting(graph: &SceneGraph, posed: &Posed, editing: bool) -> Vec<Aabb> {
    if editing || !only_spaces(graph) {
        return Vec::new();
    }
    graph
        .regions()
        .iter()
        .filter_map(|region| graph.region_volume(posed, region.id))
        .collect()
}

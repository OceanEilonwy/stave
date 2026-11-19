//! What each frame of the 3D view draws selected and how each node looks,
//! kept between frames: worked out again only when the selection, the
//! hover, the result, X-ray, focus mode or the posed problem changes, so
//! an orbit or a slider drag builds no lists.

use std::rc::Rc;

use stave_app::check::Checked;
use stave_app::session::Selection;
use stave_scene::{Item, Look, NodeId};

use stave_app::inspect;

use super::Window;
use super::edit_view::Hovered;

/// The lists a frame draws with, and what they were worked out for.
#[derive(Default)]
pub(super) struct FrameLists {
    made_for: Option<ListsKey>,
    /// The nodes drawn outlined: the selection's, then what a partner link
    /// in the Parts panel or a name link in the Inspector or the Edit
    /// panel points at.
    pub(super) selection: Vec<NodeId>,
    /// How each node looks outside focus mode (X-ray, a posed problem);
    /// `None` draws every node as usual.
    pub(super) looks: Option<Vec<Look>>,
    /// A selected joint member shows through the parts it sits in
    /// (`Session::looks`).
    pub(super) outside: Option<Vec<Look>>,
    /// Room for `looks` with an edit's covering parts made see-through,
    /// filled on the frames that have some.
    pub(super) covered: Vec<Look>,
}

/// Everything the lists depend on. The checks are held by `Rc`, so one
/// still held is never mistaken for a new one at the same address.
#[derive(Default)]
struct ListsKey {
    shown: Option<Rc<Checked>>,
    latest: Option<Rc<Checked>>,
    selection: Option<Selection>,
    hovered: Vec<Item>,
    link_hover: Option<Hovered>,
    xray: bool,
    focused: bool,
    problem_nodes: Option<Vec<NodeId>>,
}

/// The same inputs as a [`ListsKey`], borrowed from the window as it is
/// this frame: compared without copying anything.
struct ListsNow<'a> {
    shown: Option<&'a Rc<Checked>>,
    latest: Option<&'a Rc<Checked>>,
    selection: Option<&'a Selection>,
    hovered: &'a [Item],
    link_hover: Option<&'a Hovered>,
    xray: bool,
    focused: bool,
    problem_nodes: Option<&'a [NodeId]>,
}

fn same_check(kept: Option<&Rc<Checked>>, now: Option<&Rc<Checked>>) -> bool {
    match (kept, now) {
        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

impl ListsKey {
    fn same(&self, now: &ListsNow<'_>) -> bool {
        same_check(self.shown.as_ref(), now.shown)
            && same_check(self.latest.as_ref(), now.latest)
            && self.selection.as_ref() == now.selection
            && self.hovered == now.hovered
            && self.link_hover.as_ref() == now.link_hover
            && self.xray == now.xray
            && self.focused == now.focused
            && self.problem_nodes.as_deref() == now.problem_nodes
    }

    fn of(now: &ListsNow<'_>) -> Self {
        Self {
            shown: now.shown.cloned(),
            latest: now.latest.cloned(),
            selection: now.selection.cloned(),
            hovered: now.hovered.to_vec(),
            link_hover: now.link_hover.cloned(),
            xray: now.xray,
            focused: now.focused,
            problem_nodes: now.problem_nodes.map(<[NodeId]>::to_vec),
        }
    }
}

/// Every node `rest`, but those in `kept`, which look as usual; `out` is
/// filled in place, its room kept.
fn fill_looks(out: &mut Vec<Look>, count: usize, kept: &[NodeId], rest: Look) {
    out.clear();
    out.resize(count, rest);
    for node in kept {
        if let Some(look) = out.get_mut(node.index()) {
            *look = Look::Normal;
        }
    }
}

impl FrameLists {
    /// What a frame draws with: the outlined nodes, the outside looks and
    /// `looks` with each of `covering` see-through (in `covered`'s room;
    /// `looks` itself when nothing covers).
    pub(super) fn for_frame(
        &mut self,
        covering: &[NodeId],
        count: usize,
    ) -> (&[NodeId], Option<&[Look]>, Option<&[Look]>) {
        if covering.is_empty() {
            return (
                &self.selection,
                self.outside.as_deref(),
                self.looks.as_deref(),
            );
        }
        self.covered.clear();
        match &self.looks {
            Some(looks) => self.covered.extend_from_slice(looks),
            None => self.covered.resize(count, Look::Normal),
        }
        for node in covering {
            if let Some(look) = self.covered.get_mut(node.index()) {
                *look = Look::XRay;
            }
        }
        (
            &self.selection,
            self.outside.as_deref(),
            Some(&self.covered),
        )
    }
}

impl Window {
    /// Brings the kept frame lists up to date, working them out again only
    /// when something they depend on changed since the last frame.
    pub(super) fn refresh_frame_lists(&mut self) {
        let session = &self.session;
        let state = &self.state;
        let now = ListsNow {
            shown: session.shown(),
            latest: session.latest(),
            selection: session.selection(),
            hovered: &state.hovered,
            link_hover: self.editing.link_hover(),
            xray: state.xray,
            focused: state.focus.is_some(),
            problem_nodes: state
                .problem
                .as_ref()
                .map(|problem| problem.highlight.nodes.as_slice()),
        };
        let lists = &mut self.frame_lists;
        if lists.made_for.as_ref().is_some_and(|key| key.same(&now)) {
            return;
        }
        let FrameLists {
            selection, looks, ..
        } = &mut *lists;
        selection.clear();
        let mut kept_looks = looks.take().unwrap_or_default();
        let mut outside = None;
        if let Some((design, graph)) = session.drawable() {
            let first = |item: Item| graph.nodes_of(item).into_iter().take(1);
            if let Some(chosen) = now.selection {
                selection.extend(
                    inspect::items_in(design, chosen)
                        .into_iter()
                        .flat_map(first),
                );
            }
            selection.extend(now.hovered.iter().copied().flat_map(first));
            if let Some(Hovered::Item(linked)) = now.link_hover {
                selection.extend(linked.item().into_iter().flat_map(first));
            }
            if !now.focused {
                let kept: Option<&[NodeId]> = match now.problem_nodes {
                    Some(nodes) => Some(nodes),
                    None if now.xray => Some(selection),
                    None => None,
                };
                if let Some(kept) = kept {
                    let rest = if now.xray { Look::XRay } else { Look::Dim };
                    fill_looks(&mut kept_looks, graph.node_ids().count(), kept, rest);
                    *looks = Some(kept_looks);
                }
                outside = session.looks();
            }
        }
        lists.outside = outside;
        lists.made_for = Some(ListsKey::of(&now));
    }
}

#[cfg(test)]
mod tests {
    use stave_app::session::ProblemIndex;

    use super::*;

    fn now<'a>(selection: Option<&'a Selection>) -> ListsNow<'a> {
        ListsNow {
            shown: None,
            latest: None,
            selection,
            hovered: &[],
            link_hover: None,
            xray: false,
            focused: false,
            problem_nodes: None,
        }
    }

    #[test]
    fn the_lists_are_kept_while_nothing_they_depend_on_changes() {
        let chosen = Selection::Problem(ProblemIndex(1));
        let key = ListsKey::of(&now(Some(&chosen)));
        assert!(key.same(&now(Some(&chosen))));
    }

    #[test]
    fn the_lists_are_made_again_when_the_selection_or_the_look_changes() {
        let chosen = Selection::Problem(ProblemIndex(1));
        let other = Selection::Problem(ProblemIndex(2));
        let key = ListsKey::of(&now(Some(&chosen)));
        assert!(!key.same(&now(Some(&other))));
        assert!(!key.same(&now(None)));
        assert!(!key.same(&ListsNow {
            xray: true,
            ..now(Some(&chosen))
        }));
        assert!(!key.same(&ListsNow {
            focused: true,
            ..now(Some(&chosen))
        }));
        assert!(!key.same(&ListsNow {
            problem_nodes: Some(&[]),
            ..now(Some(&chosen))
        }));
    }

    #[test]
    fn looks_are_filled_in_place() {
        let mut looks = vec![Look::Normal; 8];
        fill_looks(&mut looks, 3, &[], Look::Dim);
        assert_eq!(looks, vec![Look::Dim; 3]);
        let mut lists = FrameLists {
            looks: Some(looks),
            ..FrameLists::default()
        };
        assert_eq!(lists.for_frame(&[], 3).2, Some(&[Look::Dim; 3][..]));
    }
}

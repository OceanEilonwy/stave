//! The 3D view's camera: framing everything or the selection, the start
//! camera from the command line, looking from a side, and the keys and
//! drags that turn and zoom it.

use std::rc::Rc;

use stave_app::check::Checked;
use stave_app::inspect;
use stave_app::session::Selection;
use stave_scene::{NodeId, Orbit};

use super::{FRAME_ROOM, State, Window, edit_view};

impl Window {
    pub(super) fn aspect(state: &State) -> f32 {
        crate::render::aspect(state.size.0, state.size.1)
    }

    /// A camera that shows everything shown.
    pub(super) fn frame_all(state: &State, shown: &Checked) -> Option<Orbit> {
        let (_, graph) = shown.drawable()?;
        let bounds = edit_view::drawn_bounds(graph, state.posed.as_ref()?)?;
        let aspect = Self::aspect(state);
        Some(match state.orbit {
            Some(orbit) => orbit.framed(bounds, aspect),
            None => Orbit::framing(bounds, aspect),
        })
    }

    /// The node drawn as selected.
    pub(super) fn selected_node(&self) -> Option<NodeId> {
        let session = self.session.borrow();
        let item = session.selection()?.item()?;
        let (_, graph) = session.drawable()?;
        graph.nodes_of(item).into_iter().next()
    }

    /// Every node to draw selected: the selected item's, or those of every
    /// part and piece of hardware inside a selected assembly.
    pub(super) fn selected_nodes(&self) -> Vec<NodeId> {
        let session = self.session.borrow();
        let Some((design, graph)) = session.drawable() else {
            return Vec::new();
        };
        let items = session
            .selection()
            .map_or_else(Vec::new, |selection| inspect::items_in(design, selection));
        items
            .into_iter()
            .flat_map(|item| graph.nodes_of(item).into_iter().take(1))
            .collect()
    }

    /// F: frames the selection (every part of a selected assembly), or
    /// everything.
    pub(super) fn frame(self: &Rc<Self>) {
        let nodes = self.selected_nodes();
        {
            let session = self.session.borrow();
            let Some(shown) = session.shown() else {
                return;
            };
            let mut state = self.state.borrow_mut();
            let aspect = Self::aspect(&state);
            let selected = (!nodes.is_empty())
                .then(|| {
                    let (_, graph) = shown.drawable()?;
                    let bounds = graph.bounds_of(state.posed.as_ref()?, &nodes)?;
                    let framed = state.orbit?.framed(bounds, aspect);
                    Some(Orbit {
                        distance: framed.distance * FRAME_ROOM,
                        ..framed
                    })
                })
                .flatten();
            state.orbit = selected.or_else(|| Self::frame_all(&state, shown));
        }
        self.request_render();
    }

    /// Choosing something frames it at once: a part, a piece of hardware
    /// or an assembly, or everything again when nothing is chosen. A
    /// problem frames itself where it is posed; focus mode frames its
    /// joint; a place in the text leaves the camera alone.
    pub(super) fn frame_selection(self: &Rc<Self>) {
        let reframes = {
            let session = self.session.borrow();
            session.focus().is_none()
                && matches!(
                    session.selection(),
                    None | Some(
                        Selection::Part(_) | Selection::Hardware(_) | Selection::Assembly(_)
                    )
                )
        };
        if reframes {
            self.frame();
        }
    }

    /// Moves the camera where the command line asked, once the view has a
    /// size and the asked-for selection is made.
    pub(super) fn apply_start_camera(self: &Rc<Self>) {
        let ready = {
            let state = self.state.borrow();
            state.size.0 > 1 && state.wanted.is_none() && state.start_camera.is_some()
        };
        if !ready {
            return;
        }
        let node = self.selected_node();
        {
            let session = self.session.borrow();
            let mut state = self.state.borrow_mut();
            let Some(camera) = state.start_camera.take() else {
                return;
            };
            let aspect = Self::aspect(&state);
            let Some(orbit) = state.orbit else {
                return;
            };
            let turned = Orbit {
                yaw: (orbit.yaw + camera.turn.to_radians()).rem_euclid(std::f32::consts::TAU),
                pitch: camera.pitch.map_or(orbit.pitch, f32::to_radians),
                ..orbit
            };
            let posed = state.posed.as_ref();
            let drawable = session.drawable();
            let bounds = drawable.and_then(|(_, graph)| {
                let posed = posed?;
                match node.filter(|_| camera.frame_selection) {
                    Some(id) => graph.bounds_of(posed, &[id]),
                    None => edit_view::drawn_bounds(graph, posed),
                }
            });
            let framed = bounds.map_or(turned, |bounds| turned.framed(bounds, aspect));
            let zoomed = Orbit {
                distance: framed.distance / camera.zoom.unwrap_or(1.0),
                ..framed
            };
            state.orbit = Some(zoomed);
        }
        self.request_render();
    }

    /// Turns the camera to look from a direction, framing everything.
    pub(super) fn look_from(self: &Rc<Self>, yaw: f32, pitch: f32) {
        {
            let session = self.session.borrow();
            let mut state = self.state.borrow_mut();
            let aspect = Self::aspect(&state);
            let bounds = session
                .drawable()
                .and_then(|(_, graph)| edit_view::drawn_bounds(graph, state.posed.as_ref()?));
            state.orbit = state.orbit.map(|orbit| {
                let turned = Orbit {
                    yaw: yaw.rem_euclid(std::f32::consts::TAU),
                    pitch,
                    ..orbit
                };
                bounds.map_or(turned, |bounds| turned.framed(bounds, aspect))
            });
        }
        self.request_render();
    }

    pub(super) fn update_orbit(self: &Rc<Self>, change: impl FnOnce(Orbit) -> Orbit) {
        {
            let mut state = self.state.borrow_mut();
            state.orbit = state.orbit.map(change);
        }
        self.request_render();
    }

    /// The part of a drag not applied yet.
    pub(super) fn drag_step(&self, x: f64, y: f64) -> (f32, f32) {
        let mut state = self.state.borrow_mut();
        let (done_x, done_y) = state.drag_applied;
        state.drag_applied = (x, y);
        (
            crate::convert::f32_of(x - done_x),
            crate::convert::f32_of(y - done_y),
        )
    }
}

/// How far one arrow key turns the 3D view, in the pixels of a mouse drag
/// (about 15°).
pub(super) const ORBIT_KEY_STEP: f32 = 32.0;

/// What a camera key does to the 3D view's camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum CameraKey {
    /// Turns it as a drag of (dx, dy) pixels would: right turns the design
    /// right, down looks from higher up.
    Turn(f32, f32),
    /// Zooms in (`> 0`) or out by this many scroll steps.
    Zoom(f32),
}

impl CameraKey {
    /// The camera after the key.
    pub(super) fn apply(self, orbit: Orbit) -> Orbit {
        match self {
            CameraKey::Turn(dx, dy) => orbit.orbited(dx, dy),
            CameraKey::Zoom(steps) => orbit.zoomed(steps),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::app;
    use super::*;

    #[test]
    fn the_arrow_keys_turn_the_view_and_plus_and_minus_zoom() {
        let start = Orbit {
            target: glam::Vec3::ZERO,
            yaw: 1.0,
            pitch: 0.3,
            distance: 1000.0,
            projection: stave_scene::Projection::default(),
        };
        let left = CameraKey::Turn(-ORBIT_KEY_STEP, 0.0).apply(start);
        let right = CameraKey::Turn(ORBIT_KEY_STEP, 0.0).apply(start);
        assert!(left.yaw > start.yaw && right.yaw < start.yaw);
        let higher = CameraKey::Turn(0.0, ORBIT_KEY_STEP).apply(start);
        assert!(higher.pitch > start.pitch);
        assert!(CameraKey::Zoom(1.0).apply(start).distance < start.distance);
        assert!(CameraKey::Zoom(-1.0).apply(start).distance > start.distance);
        // Every camera key is an action in the table the shortcuts dialog
        // lists, bound by the view.
        for name in [
            "turn-left",
            "turn-right",
            "look-higher",
            "look-lower",
            "zoom-in",
            "zoom-out",
        ] {
            let action = format!("view.{name}");
            assert!(
                app::ACCELERATORS
                    .iter()
                    .any(|a| a.action == action && a.scope == app::KeyScope::View),
                "{action} has no keys"
            );
        }
    }
}

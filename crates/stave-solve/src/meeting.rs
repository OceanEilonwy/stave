//! How two parts meet (grammar, "Built-in functions"): one classification,
//! of which `edge_to_edge(a, b)` is the first question asked.
//!
//! The classification is made on the parts' drawn boxes and orientations,
//! exactly, the same boxes `contact(a, b)` measures: before joints lengthen
//! the parts, so a tongue reaching into its groove doesn't make two boards
//! cross. A part's surfaces are named along its grain: its **edges** run
//! along the grain beside its faces (square to its width, or to its length
//! when `grain: width`), its **ends** are across the grain, and its
//! **faces** are square to its thickness. So a part's boards, which are
//! split across the grain, always meet edge to edge.
//!
//! Only `edge_to_edge` is asked now; end to end, an end against a face and
//! face to face are further questions of the same [`Meeting`].

use stave_lang::placement::PartAxis;
use stave_model::ids::PartId;
use stave_model::value::Value;
use stave_schema::functions::Builtin;
use stave_schema::words::Grain;

use crate::eval::Env;
use crate::{Solved, Solver};

/// One of a part's surfaces, named along its grain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
    /// Across the grain: end grain.
    End,
    /// Along the grain, beside the faces.
    Edge,
    /// Square to the part's thickness.
    Face,
}

impl Surface {
    /// The surface square to one of a part's own axes, for its grain.
    pub(crate) fn of(axis: PartAxis, grain: Grain) -> Surface {
        match (axis, grain) {
            (PartAxis::Thickness, Grain::Length | Grain::Width) => Surface::Face,
            (PartAxis::Width, Grain::Length) | (PartAxis::Length, Grain::Width) => Surface::Edge,
            (PartAxis::Length, Grain::Length) | (PartAxis::Width, Grain::Width) => Surface::End,
        }
    }

    /// The word for it in a message.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Surface::End => "end",
            Surface::Edge => "edge",
            Surface::Face => "face",
        }
    }
}

/// How the faces of two parts lie to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Faces {
    /// Their thicknesses run along the same world axis: they lie in one
    /// plane, or in parallel planes.
    Parallel,
    /// Their thicknesses run along different world axes: they meet at
    /// right angles, as at a corner.
    Square,
}

/// How two parts meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Meeting {
    /// They don't touch over an area (or are the same part).
    Apart,
    /// Their drawn boxes share wood: they cross, as in a half lap.
    Cross,
    /// They touch, a surface of each against the other.
    Butt {
        /// The surface of the first part that touches.
        first: Surface,
        /// The surface of the second part that touches.
        second: Surface,
        /// How their faces lie to each other.
        faces: Faces,
    },
}

impl Meeting {
    /// `edge_to_edge(a, b)`: side by side in one plane, an edge of each
    /// against the other. Boards of different thickness still count.
    pub(crate) fn edge_to_edge(self) -> bool {
        match self {
            Meeting::Butt {
                first: Surface::Edge,
                second: Surface::Edge,
                faces: Faces::Parallel,
            } => true,
            Meeting::Butt { .. } | Meeting::Apart | Meeting::Cross => false,
        }
    }

    /// The answer to a question about how two parts meet, or `None` for a
    /// built-in that asks none.
    pub(crate) fn answer(self, question: Builtin) -> Option<bool> {
        match question {
            Builtin::EdgeToEdge => Some(self.edge_to_edge()),
            Builtin::Between
            | Builtin::Extent
            | Builtin::Mid
            | Builtin::Opening
            | Builtin::Split
            | Builtin::Cover
            | Builtin::Overlay
            | Builtin::Inset
            | Builtin::Turned
            | Builtin::Joined
            | Builtin::Spliced
            | Builtin::Box
            | Builtin::RegionBox
            | Builtin::Cylinder
            | Builtin::Model
            | Builtin::Ends
            | Builtin::Pitch
            | Builtin::Gap
            | Builtin::Contact
            | Builtin::Wall => None,
        }
    }

    /// How they meet, in words, for the note on a failed rule that asks:
    /// `side's end meets base's face at right angles`.
    pub(crate) fn describe(self, first: &str, second: &str) -> String {
        match self {
            Meeting::Apart => format!("{first} and {second} don't touch"),
            Meeting::Cross => format!("{first} and {second} cross each other"),
            Meeting::Butt {
                first: a,
                second: b,
                faces,
            } => {
                let angle = match faces {
                    Faces::Parallel => "",
                    Faces::Square => " at right angles",
                };
                format!(
                    "{first}'s {} meets {second}'s {}{angle}",
                    a.word(),
                    b.word()
                )
            }
        }
    }
}

impl Solver<'_> {
    /// How two parts meet, from their exact drawn boxes and orientations.
    pub(crate) fn meeting(&mut self, a: PartId, b: PartId) -> Solved<Meeting> {
        if a == b {
            return Ok(Meeting::Apart);
        }
        let (box_a, box_b) = (self.drawn_box(a)?, self.drawn_box(b)?);
        if !box_a.meets(&box_b) {
            return Ok(Meeting::Apart);
        }
        let Some(normal) = box_a.touching_axis(&box_b) else {
            return Ok(Meeting::Cross);
        };
        let (frame_a, frame_b) = (self.part_orientation(a)?, self.part_orientation(b)?);
        let first = Surface::of(frame_a.part_axis(normal), self.part_grain(a)?);
        let second = Surface::of(frame_b.part_axis(normal), self.part_grain(b)?);
        let faces = if frame_a.direction(PartAxis::Thickness).axis()
            == frame_b.direction(PartAxis::Thickness).axis()
        {
            Faces::Parallel
        } else {
            Faces::Square
        };
        Ok(Meeting::Butt {
            first,
            second,
            faces,
        })
    }

    /// Which way a part's grain runs. A board or a piece of a spliced part
    /// has its part's grain, a mirror image its source's; a part a joint
    /// makes runs along its length.
    fn part_grain(&mut self, part: PartId) -> Solved<Grain> {
        let part = match self.boards.get(&part) {
            Some(board) => board.part,
            None => part,
        };
        if part.index() >= self.model.parts.len() {
            return Ok(Grain::Length);
        }
        let source = self.body_source(part)?;
        Ok(self
            .part_body(source)
            .map_or(Grain::Length, |body| body.grain))
    }

    /// The first question about how two parts meet in a condition
    /// (`edge_to_edge(a, b)`), worked out for this pair: the two parts and
    /// how they meet. `None` when the condition asks none.
    pub(crate) fn meeting_asked(
        &mut self,
        condition: &stave_model::defs::RExpr,
        env: &Env<'_>,
    ) -> Option<(PartId, PartId, Meeting)> {
        let (a, b) = meeting_question(condition)?;
        let a = self.eval(a, env).ok()?.parts()?;
        let b = self.eval(b, env).ok()?.parts()?;
        let (&[a], &[b]) = (a.as_slice(), b.as_slice()) else {
            return None;
        };
        let meeting = self.meeting(a, b).ok()?;
        Some((a, b, meeting))
    }

    /// The note on a failed rule that asks how the parts meet: how they do
    /// (`case.side[left]'s end meets case.base's face at right angles`).
    pub(crate) fn meeting_note(
        &mut self,
        condition: &stave_model::defs::RExpr,
        env: &Env<'_>,
    ) -> Option<String> {
        let (a, b, meeting) = self.meeting_asked(condition, env)?;
        let (a_name, b_name) = (self.part_name(a), self.part_name(b));
        Some(meeting.describe(&a_name, &b_name))
    }

    /// `edge_to_edge(a, b)` and the questions like it, evaluated.
    pub(crate) fn eval_meeting(
        &mut self,
        question: Builtin,
        args: &[stave_model::defs::Setting],
        env: &Env<'_>,
    ) -> Solved<Value> {
        let find = |name: &str| args.iter().find(|arg| arg.name == name).map(|a| &a.value);
        let (Some(a), Some(b)) = (find("a"), find("b")) else {
            return Err(crate::Reported);
        };
        let a = self.one_part(question, a, env)?;
        let b = self.one_part(question, b, env)?;
        let meeting = self.meeting(a, b)?;
        meeting
            .answer(question)
            .map(Value::Bool)
            .ok_or(crate::Reported)
    }
}

/// `true` if a condition asks how two parts meet anywhere in it.
pub(crate) fn asks_how_parts_meet(condition: &stave_model::defs::RExpr) -> bool {
    meeting_question(condition).is_some()
}

/// The two part expressions of the first question about how two parts
/// meet in an expression.
fn meeting_question(
    expr: &stave_model::defs::RExpr,
) -> Option<(&stave_model::defs::RExpr, &stave_model::defs::RExpr)> {
    use stave_model::defs::RExprKind;
    match &expr.kind {
        RExprKind::Builtin { function, args } if function.asks_how_parts_meet() => {
            let a = args.iter().find(|arg| arg.name == "a")?;
            let b = args.iter().find(|arg| arg.name == "b")?;
            Some((&a.value, &b.value))
        }
        RExprKind::Builtin { args, .. } | RExprKind::KindValue { settings: args, .. } => {
            args.iter().find_map(|arg| meeting_question(&arg.value))
        }
        RExprKind::Member { base, .. } | RExprKind::Index { base, .. } => meeting_question(base),
        RExprKind::Unary { operand, .. } => meeting_question(operand),
        RExprKind::Binary { lhs, rhs, .. } => {
            meeting_question(lhs).or_else(|| meeting_question(rhs))
        }
        RExprKind::List(items) => items.iter().find_map(meeting_question),
        RExprKind::Length(_)
        | RExprKind::Angle(_)
        | RExprKind::Number(_)
        | RExprKind::Bool(_)
        | RExprKind::Text(_)
        | RExprKind::Fit
        | RExprKind::Word(_)
        | RExprKind::Symbol(_)
        | RExprKind::SelfElement(_)
        | RExprKind::Wall(_)
        | RExprKind::Error => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_s_surfaces_are_named_along_its_grain() {
        assert_eq!(Surface::of(PartAxis::Width, Grain::Length), Surface::Edge);
        assert_eq!(Surface::of(PartAxis::Length, Grain::Length), Surface::End);
        assert_eq!(Surface::of(PartAxis::Length, Grain::Width), Surface::Edge);
        assert_eq!(Surface::of(PartAxis::Width, Grain::Width), Surface::End);
        assert_eq!(
            Surface::of(PartAxis::Thickness, Grain::Width),
            Surface::Face
        );
    }

    #[test]
    fn only_two_edges_in_parallel_planes_are_edge_to_edge() {
        let butt = |first, second, faces| Meeting::Butt {
            first,
            second,
            faces,
        };
        assert!(butt(Surface::Edge, Surface::Edge, Faces::Parallel).edge_to_edge());
        assert!(!butt(Surface::Edge, Surface::Edge, Faces::Square).edge_to_edge());
        assert!(!butt(Surface::Edge, Surface::End, Faces::Parallel).edge_to_edge());
        assert!(!butt(Surface::End, Surface::Face, Faces::Square).edge_to_edge());
        assert!(!Meeting::Apart.edge_to_edge());
        assert!(!Meeting::Cross.edge_to_edge());
        assert_eq!(Meeting::Apart.answer(Builtin::Contact), None);
    }

    #[test]
    fn a_meeting_is_described_in_woodworking_words() {
        let corner = Meeting::Butt {
            first: Surface::End,
            second: Surface::Face,
            faces: Faces::Square,
        };
        assert_eq!(
            corner.describe("side", "base"),
            "side's end meets base's face at right angles"
        );
        assert_eq!(Meeting::Apart.describe("a", "b"), "a and b don't touch");
    }
}

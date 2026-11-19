//! Joint members (grammar, "Joint members"): a joint kind's `member`s,
//! laid out `on:` the patch where two of its parts touch, `count:` copies
//! `spaced:` along it, and the cuts they make.
//!
//! Each copy has its own frame, the same three words a hardware kind's
//! frame has: `along` the patch's length, `across` its width, `depth`
//! square to it from the first part of `contact(a, b)` into the second.
//! Its origin is on the contact plane, in the middle of the patch's width,
//! where the spacing puts it along the length. Everything is exact; the
//! shapes are rounded once, when the design is written. Because a
//! member's cuts are made where its shapes meet each part, the holes a pin
//! makes in both parts share their centres by construction.

use stave_lang::Code;
use stave_lang::ast::Span;
use stave_lang::units::ExactLength;
use stave_model::defs::{BodyId, Carrier, FrameAxis, KindDef, LayoutDef, Word};
use stave_model::value::{KindValue, Patch, Spacing, Value};

use crate::eval::{Env, midpoint};
use crate::hardware::{Frame, FrameBox, Outline, PlacedShape, positive_direction};
use crate::place::{add, divide, scale, sub};
use crate::{Fault, Solved, Solver};

/// One copy of a joint's member: its frame on the patch, and its shapes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MemberCopy {
    pub frame: Frame,
    /// One per shape, in the kind's order.
    pub shapes: Vec<PlacedShape>,
}

/// A joint's member laid out for one pair.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LaidOut {
    pub body: BodyId,
    pub name: String,
    /// Its copies, in order along the patch.
    pub copies: Vec<MemberCopy>,
    /// What it is, when the kind says: counted once for every copy.
    pub item: Option<KindValue>,
}

/// A cut a joint's member makes in one part, in the world: kept so a
/// mirrored assembly can make it again, reflected.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorldMemberCut {
    pub pair: usize,
    pub part: stave_model::ids::PartId,
    pub body: BodyId,
    pub name: Option<stave_model::defs::CutName>,
    /// The world face of the part it is cut from.
    pub face: stave_lang::placement::Face,
    /// The shape's world box.
    pub shape: stave_model::value::ExactBox,
    pub kind: stave_model::solved::CutKind,
}

/// Where each of `count` copies `size` long starts along a span `length`
/// long, from its start (grammar, "Spacing values", the same as for
/// parts): `None` when the last copy would end past the span with `pitch`
/// or `gap`.
pub(crate) fn offsets(
    spacing: Spacing,
    count: usize,
    size: ExactLength,
    length: ExactLength,
) -> Solved<(Vec<ExactLength>, Option<ExactLength>)> {
    let total = scale(size, count)?;
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let before = scale(size, index)?;
        let low = match spacing {
            Spacing::Evenly => {
                let gap = divide(sub(length, total)?, count + 1)?;
                add(scale(gap, index + 1)?, before)?
            }
            Spacing::Ends(inset) => {
                if count <= 1 {
                    inset
                } else {
                    let free = sub(sub(length, scale(inset, 2)?)?, total)?;
                    let gap = divide(free, count - 1)?;
                    add(add(inset, scale(gap, index)?)?, before)?
                }
            }
            Spacing::Pitch(pitch) => scale(pitch, index)?,
            Spacing::Gap(gap) => add(scale(gap, index)?, before)?,
        };
        out.push(low);
    }
    // How far the last copy ends past the span, for a fixed spacing.
    let past = match (spacing, out.last()) {
        (Spacing::Pitch(_) | Spacing::Gap(_), Some(last)) => {
            let end = add(*last, size)?;
            (end > length).then(|| sub(end, length)).transpose()?
        }
        (Spacing::Pitch(_) | Spacing::Gap(_), None) | (Spacing::Evenly | Spacing::Ends(_), _) => {
            None
        }
    };
    Ok((out, past))
}

impl Solver<'_> {
    /// Every member of a joint kind, laid out for one pair. `owner` names
    /// the joint in messages; `place` is its kind call.
    pub(crate) fn lay_out_members(
        &mut self,
        kind: &KindDef,
        env: &Env<'_>,
        owner: &str,
        place: (stave_lang::FileId, Span),
    ) -> Solved<Vec<LaidOut>> {
        let mut out = Vec::new();
        for (index, body) in kind.bodies.iter().enumerate() {
            let Carrier::Contact(layout) = &body.with else {
                continue;
            };
            let id = BodyId::new(index).ok_or(Fault::Internal("a member's number"))?;
            let patch = self.member_patch(layout, env)?;
            let count = match &layout.count {
                Some(count) => {
                    usize::try_from(self.eval_count(count, env)?).map_err(|_| Fault::Overflow)?
                }
                None => 1,
            };
            let spacing = self.member_spacing(layout, env)?;
            let shapes = body
                .shapes
                .iter()
                .map(|shape| self.shape_box(&shape.shape, env))
                .collect::<Solved<Vec<_>>>()?;
            // The copies are as long as their shapes together are along.
            let (low, high) = shapes
                .iter()
                .map(|(frame_box, _, _)| frame_box.along)
                .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
                .ok_or(Fault::Internal("a member's shapes"))?;
            let size = sub(high, low)?;
            let length_axis = patch.axis(stave_lang::placement::PartAxis::Length);
            let width_axis = patch.axis(stave_lang::placement::PartAxis::Width);
            let area = patch.area();
            let start = area.min(length_axis);
            let length = sub(area.max(length_axis), start)?;
            let (lows, past) = offsets(spacing, count, size, length)?;
            if let Some(past) = past {
                let (file, span) = place;
                let what = self.describe(&Value::Length(past));
                return Err(self
                    .error(
                        Code::Overlap,
                        format!(
                            "{owner}: its {}s run {what} past the end of where its parts touch",
                            body.name
                        ),
                        file,
                        span,
                        "use fewer of them, or space them closer together",
                    )
                    .into());
            }
            let across =
                midpoint(area.min(width_axis), area.max(width_axis)).ok_or(Fault::Overflow)?;
            let plane = midpoint(area.min(patch.normal()), area.max(patch.normal()))
                .ok_or(Fault::Overflow)?;
            let mut copies = Vec::with_capacity(count);
            for low_end in lows {
                // The shapes' low end along sits where the spacing puts it.
                let along = sub(add(start, low_end)?, low)?;
                let mut origin = [ExactLength::ZERO; 3];
                for (axis, value) in [
                    (length_axis, along),
                    (width_axis, across),
                    (patch.normal(), plane),
                ] {
                    if let Some(slot) = origin.get_mut(crate::hardware::world_index(axis)) {
                        *slot = value;
                    }
                }
                let frame = Frame {
                    origin,
                    along: positive_direction(length_axis),
                    across: positive_direction(width_axis),
                    depth: patch.into_second(),
                    across_edge: None,
                };
                let placed = shapes
                    .iter()
                    .map(|(frame_box, cylinder, outline)| {
                        placed_shape(&frame, *frame_box, *cylinder, outline.clone())
                    })
                    .collect::<Solved<Vec<_>>>()?;
                copies.push(MemberCopy {
                    frame,
                    shapes: placed,
                });
            }
            let item = match &body.item {
                Some(item) => match self.eval(item, env)? {
                    Value::Kind(kind_value) => Some(kind_value),
                    _ => return Err(Fault::Earlier.into()),
                },
                None => None,
            };
            out.push(LaidOut {
                body: id,
                name: body.name.as_str().to_owned(),
                copies,
                item,
            });
        }
        Ok(out)
    }

    /// `on:`: the patch, `contact(a, b)`.
    fn member_patch(&mut self, layout: &LayoutDef, env: &Env<'_>) -> Solved<Patch> {
        match self.eval(&layout.on, env)? {
            Value::Patch(patch) => Ok(patch),
            other => Err(self
                .error(
                    Code::TypeMismatch,
                    format!("'on' needs a contact patch, found {}", other.description()),
                    env.file,
                    layout.on.span,
                    "write on: contact(piece, into), the patch where the parts touch",
                )
                .into()),
        }
    }

    /// `spaced:`, `evenly` when left out.
    fn member_spacing(&mut self, layout: &LayoutDef, env: &Env<'_>) -> Solved<Spacing> {
        let Some(spaced) = &layout.spaced else {
            return Ok(Spacing::Evenly);
        };
        match self.eval(spaced, env)? {
            Value::Spacing(spacing) => Ok(spacing),
            Value::Word(Word::Evenly) => Ok(Spacing::Evenly),
            Value::Word(Word::Ends) => Ok(Spacing::Ends(ExactLength::ZERO)),
            other => Err(self
                .error(
                    Code::TypeMismatch,
                    format!("'spaced' needs a spacing, found {}", other.description()),
                    env.file,
                    spaced.span,
                    "write evenly, ends, ends(inset: d), pitch(d) or gap(d)",
                )
                .into()),
        }
    }
}

/// A shape in a copy's frame, placed in the world.
fn placed_shape(
    frame: &Frame,
    frame_box: FrameBox,
    cylinder: Option<FrameAxis>,
    outline: Option<Outline>,
) -> Solved<PlacedShape> {
    Ok(PlacedShape {
        world: frame.world_box(&frame_box)?,
        frame: frame_box,
        cylinder,
        outline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::placement::Axis;
    use stave_lang::units::Length;

    /// The world axis of a frame axis.
    fn axis_of(frame: &Frame, axis: FrameAxis) -> Axis {
        frame.direction(axis).axis()
    }

    fn mm(value: i64) -> ExactLength {
        Length::from_mm(value).to_exact()
    }

    #[test]
    fn evenly_leaves_equal_gaps_before_between_and_after() {
        // Four 8mm pins on 560mm: gaps of (560 - 32) / 5 = 105.6mm.
        let (lows, past) = offsets(Spacing::Evenly, 4, mm(8), mm(560)).unwrap();
        let gap = mm(528).checked_div(5i64.into()).unwrap();
        assert_eq!(lows.first().copied(), Some(gap));
        let second = gap.checked_add(gap).unwrap().checked_add(mm(8)).unwrap();
        assert_eq!(lows.get(1).copied(), Some(second));
        assert_eq!(past, None);
    }

    #[test]
    fn a_fixed_pitch_says_how_far_it_runs_past_the_end() {
        let (lows, past) = offsets(Spacing::Pitch(mm(30)), 3, mm(8), mm(50)).unwrap();
        assert_eq!(lows, vec![mm(0), mm(30), mm(60)]);
        assert_eq!(past, Some(mm(18)));
    }

    #[test]
    fn a_frame_on_a_patch_uses_its_axes() {
        let frame = Frame {
            origin: [mm(0); 3],
            along: positive_direction(Axis::Deep),
            across: positive_direction(Axis::Across),
            depth: stave_lang::placement::Direction::Up,
            across_edge: None,
        };
        assert_eq!(axis_of(&frame, FrameAxis::Along), Axis::Deep);
        assert_eq!(axis_of(&frame, FrameAxis::Depth), Axis::Up);
    }
}

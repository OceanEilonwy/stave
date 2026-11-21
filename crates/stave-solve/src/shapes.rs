//! Shaped parts (grammar, "Shaped parts"): a square part tapered toward
//! one end or arched along one face, made as `outline` cuts of the part
//! itself, sawn straight through.

use stave_lang::Code;
use stave_lang::ast::Span;
use stave_lang::placement::{Axis, Face, PartAxis};
use stave_lang::units::{ExactLength, Length};
use stave_model::defs::{BuiltinCall, CutName, RExpr, RExprKind};
use stave_model::ids::PartId;
use stave_model::solved::{Cut, CutKind, CutPlace, CutSource, Depth, End, LocalFace, Mark};
use stave_model::value::{SquareShape, format_length};
use stave_schema::words::StockForm;

use crate::edges::{local_face, measure_word};
use crate::joints::{GeomTable, PartGeom, round_length};
use crate::{Fault, Solved, Solver};

/// How many straight cuts an arch is drawn as. Their corners lie on its
/// circle, so they leave a little more wood than the curve, never less.
pub(crate) const ARCH_STEPS: i64 = 24;

/// Where the arguments of a part's `shape:` are written: each argument's
/// own span when the shape is written as the call itself, the whole
/// `shape:` value otherwise (a `let` or a param).
struct Spans {
    /// `to:` or `rise:`.
    size: Span,
    /// `toward:`.
    toward: Span,
    /// `on:`, and each face in it when it is a list.
    on: Span,
    /// The faces of `on:`, in order, when it is written as a list.
    faces: Vec<Span>,
    /// `square:` or `ends:`.
    keep: Span,
}

impl Spans {
    fn of(shape: &RExpr) -> Spans {
        let whole = shape.span;
        let mut spans = Spans {
            size: whole,
            toward: whole,
            on: whole,
            faces: Vec::new(),
            keep: whole,
        };
        let RExprKind::Builtin(call) = &shape.kind else {
            return spans;
        };
        let on = match &**call {
            BuiltinCall::Tapered {
                to,
                toward,
                on,
                square,
            } => {
                spans.size = to.span;
                spans.toward = toward.span;
                spans.keep = square.as_ref().map_or(whole, |square| square.span);
                on
            }
            BuiltinCall::Arched { rise, on, ends } => {
                spans.size = rise.span;
                spans.keep = ends.as_ref().map_or(whole, |ends| ends.span);
                on
            }
            BuiltinCall::Between { .. }
            | BuiltinCall::Extent { .. }
            | BuiltinCall::Mid { .. }
            | BuiltinCall::Opening(_)
            | BuiltinCall::Split { .. }
            | BuiltinCall::Cover { .. }
            | BuiltinCall::Overlay { .. }
            | BuiltinCall::Inset { .. }
            | BuiltinCall::Turned { .. }
            | BuiltinCall::Joined { .. }
            | BuiltinCall::Spliced { .. }
            | BuiltinCall::Box { .. }
            | BuiltinCall::RegionBox { .. }
            | BuiltinCall::Cylinder { .. }
            | BuiltinCall::Model { .. }
            | BuiltinCall::Node { .. }
            | BuiltinCall::Ends { .. }
            | BuiltinCall::Pitch { .. }
            | BuiltinCall::Gap { .. }
            | BuiltinCall::Contact { .. }
            | BuiltinCall::EdgeToEdge { .. } => return spans,
        };
        spans.on = on.span;
        if let RExprKind::List(items) = &on.kind {
            spans.faces = items.iter().map(|item| item.span).collect();
        }
        spans
    }

    /// Where the `index`th face of `on:` is written.
    fn face(&self, index: usize) -> Span {
        self.faces.get(index).copied().unwrap_or(self.on)
    }
}

/// The faces on a world axis, in the order the grammar lists them.
fn faces_on(axis: Axis) -> Vec<&'static str> {
    Face::ALL
        .into_iter()
        .filter(|face| face.axis() == axis)
        .map(Face::word)
        .collect()
}

/// How a part whose length runs along `axis` is said to run.
fn runs(axis: Axis) -> &'static str {
    match axis {
        Axis::Up => "up and down",
        Axis::Across => "from left to right",
        Axis::Deep => "from front to back",
    }
}

/// `a, b, c or d`.
fn or_list(words: &[&str]) -> String {
    match words {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// The third of a part's axes, besides two different ones.
fn third_axis(a: PartAxis, b: PartAxis) -> PartAxis {
    PartAxis::ALL
        .into_iter()
        .find(|axis| *axis != a && *axis != b)
        .unwrap_or(PartAxis::Thickness)
}

/// The drawn box of a part along one of its axes, rounded once: where it
/// starts in the part's cut box, and how long it is.
fn drawn_span(geom: &PartGeom, axis: PartAxis) -> Solved<(Length, Length)> {
    Ok((
        round_length(geom.start[axis])?,
        round_length(geom.drawn[axis])?,
    ))
}

fn add(a: Length, b: Length) -> Solved<Length> {
    a.checked_add(b).ok_or(Fault::Overflow.into())
}

fn sub(a: Length, b: Length) -> Solved<Length> {
    a.checked_sub(b).ok_or(Fault::Overflow.into())
}

/// A face of the drawn box across `axis`: where it lies, and which way
/// is into the part (+1 or -1).
fn face_line(geom: &PartGeom, face: LocalFace) -> Solved<(Length, i64)> {
    let (start, size) = drawn_span(geom, face.axis)?;
    Ok(match face.end {
        End::Start => (start, 1),
        End::End => (add(start, size)?, -1),
    })
}

/// `value` moved `by` into the part from a face whose inward sign is
/// `sign`.
fn inward(value: Length, by: Length, sign: i64) -> Solved<Length> {
    if sign > 0 {
        add(value, by)
    } else {
        sub(value, by)
    }
}

/// An outline cut of the part itself, sawn straight through square to
/// the plane of the length and `across`.
fn outline(part: PartId, name: &str, across: PartAxis, marks: Vec<Mark>) -> Cut {
    Cut {
        source: CutSource::Shape { part },
        name: CutName::new(name),
        face: LocalFace {
            axis: third_axis(PartAxis::Length, across),
            end: End::Start,
        },
        place: CutPlace::Side {
            from: part,
            along_from: End::Start,
            across_axis: across,
            across_from: End::Start,
        },
        kind: CutKind::Outline {
            marks,
            depth: Depth::Through,
            from_depth: Length::ZERO,
        },
        wall: None,
    }
}

/// How deep below the chord an arch of half-chord `half` and rise `rise`
/// is at `from_middle` along the chord from its middle, in nanometres,
/// rounded toward the chord: on or inside the circle, never past it.
fn arch_depth(half: i128, rise: i128, from_middle: i128) -> Option<i128> {
    // The circle through both ends of the chord and the top of the arch.
    let radius = half
        .checked_mul(half)?
        .checked_add(rise.checked_mul(rise)?)?
        .checked_div(rise.checked_mul(2)?)?;
    let square = radius
        .checked_mul(radius)?
        .checked_sub(from_middle.checked_mul(from_middle)?)?
        .max(0);
    let root = i128::try_from(u128::try_from(square).ok()?.isqrt()).ok()?;
    Some(root.checked_sub(radius.checked_sub(rise)?)?.clamp(0, rise))
}

/// A source's shape cut moved onto a mirror image of it: the same marks in
/// the image's own frame, measured from its own drawn box.
fn rebase(cut: &mut Cut, image: PartId, from: &PartGeom, to: &PartGeom) -> Solved<()> {
    let CutPlace::Side {
        from: owner,
        across_axis,
        ..
    } = &mut cut.place
    else {
        return Err(Fault::Internal("a shape's cut").into());
    };
    *owner = image;
    let across = *across_axis;
    let shift = |axis: PartAxis, value: Length| -> Solved<Length> {
        let (old, _) = drawn_span(from, axis)?;
        let (new, _) = drawn_span(to, axis)?;
        add(sub(value, old)?, new)
    };
    if let CutKind::Outline { marks, .. } = &mut cut.kind {
        for mark in marks {
            mark.along = shift(PartAxis::Length, mark.along)?;
            mark.across = shift(across, mark.across)?;
        }
    }
    Ok(())
}

impl Solver<'_> {
    /// Every part's taper or arch, as cuts of the part. A part in a
    /// mirrored assembly, and a mirror image of a part, get their
    /// source's with the rest of its cuts, on the faces of their own frame
    /// that the source's are on; the source reports what is wrong.
    pub(crate) fn make_shapes(&mut self, geoms: &GeomTable, cuts: &mut [Vec<Cut>]) {
        for index in 0..self.model.parts.len() {
            let part = PartId::from_usize(index);
            // A part in a mirrored assembly is given its source's cuts.
            if self
                .model
                .parts
                .get(index)
                .is_none_or(|inst| inst.mirror_of.is_some())
            {
                continue;
            }
            let (file, span) = self.part_place(part);
            let made = self.part_shape_cuts(part, geoms);
            if let Some(made) = self.settled(made, file, span)
                && let Some(list) = cuts.get_mut(index)
            {
                list.extend(made);
            }
        }
    }

    fn part_shape_cuts(&mut self, part: PartId, geoms: &GeomTable) -> Solved<Vec<Cut>> {
        let Some(shape) = self.part_square_shape(part)? else {
            return Ok(Vec::new());
        };
        // A mirror image of a part has its source's shape, on the faces
        // of its own frame that the source's are on; the source reports.
        let source = self.body_source(part)?;
        if source != part {
            let mut made = match self.part_shape_cuts(source, geoms) {
                Ok(made) => made,
                Err(_) => return Err(Fault::Earlier.into()),
            };
            let (Some(from), Some(to)) = (geoms.get(source), geoms.get(part)) else {
                return Err(Fault::Earlier.into());
            };
            for cut in &mut made {
                rebase(cut, part, from, to)?;
            }
            return Ok(made);
        }
        let body = self.sound_part_body(part)?;
        let Some(written) = &body.shape else {
            return Err(Fault::Internal("a part's shape").into());
        };
        let spans = Spans::of(written);
        let env = self
            .part_env(part)
            .ok_or(Fault::Internal("where a part is declared"))?;
        let file = env.file;
        let geom = geoms.get(part).ok_or(Fault::Earlier)?;
        let name = self.part_name(part);
        let verb = shape.verb();

        // Round stock has no flat faces.
        let stock = self.part_stock(part)?;
        let form = self
            .sound(self.model.defs.sound_stock(stock), "a stock's declaration")?
            .form;
        if form == StockForm::Dowel {
            return Err(self
                .error(
                    Code::TypeMismatch,
                    format!("{name} is round, so it can't be {verb}"),
                    file,
                    written.span,
                    "cut it from square stock, or turn it to a profile with shape: turned(profile)",
                )
                .into());
        }

        // Every face shaped runs along the length, and is named once.
        let length_axis = geom.orientation.direction(PartAxis::Length).axis();
        let along: Vec<&str> = Face::ALL
            .into_iter()
            .filter(|face| face.axis() != length_axis)
            .map(Face::word)
            .collect();
        let faces = shape.faces();
        let mut locals: Vec<(Face, LocalFace)> = Vec::with_capacity(faces.len());
        for (index, face) in faces.iter().enumerate() {
            let at = spans.face(index);
            if face.axis() == length_axis {
                return Err(self
                    .error(
                        Code::TypeMismatch,
                        format!("{name}'s {} is an end, so it can't be {verb}", face.word()),
                        file,
                        at,
                        format!(
                            "name {} along {name}'s length: {}",
                            match shape {
                                SquareShape::Tapered { .. } => "faces",
                                SquareShape::Arched { .. } => "a face",
                            },
                            or_list(&along)
                        ),
                    )
                    .into());
            }
            if locals.iter().any(|(seen, _)| seen == face) {
                return Err(self
                    .error(
                        Code::DuplicateName,
                        format!("{} is {verb} twice", face.word()),
                        file,
                        at,
                        "name each face once",
                    )
                    .into());
            }
            locals.push((*face, local_face(&geom.orientation, *face)));
        }

        // An edge profile can't follow an edge of a shaped face.
        for profile in &body.edges {
            for edge in &profile.edges {
                if let Some(face) = edge.faces.iter().find(|face| faces.contains(face)) {
                    return Err(self
                        .error(
                            Code::TypeMismatch,
                            format!(
                                "{name}'s {} face is {verb}, so the {} can't follow its edges",
                                face.word(),
                                profile.operation.word()
                            ),
                            env.file,
                            profile.span,
                            format!(
                                "take the profile off {}'s edges, or the face out of the shape",
                                face.word()
                            ),
                        )
                        .into());
                }
            }
        }

        let (length_start, length) = drawn_span(geom, PartAxis::Length)?;
        match shape {
            SquareShape::Tapered {
                to, toward, square, ..
            } => {
                if toward.axis() != length_axis {
                    return Err(self
                        .error(
                            Code::TypeMismatch,
                            format!(
                                "{name} runs {}, so its taper runs toward its {}, not {}",
                                runs(length_axis),
                                faces_on(length_axis).join(" or its "),
                                toward.word()
                            ),
                            file,
                            spans.toward,
                            format!(
                                "name the end the taper runs to, as toward: {}",
                                or_list(&faces_on(length_axis))
                            ),
                        )
                        .into());
                }
                let narrow = local_face(&geom.orientation, toward);
                if to <= ExactLength::ZERO {
                    return Err(self
                        .error(
                            Code::OutOfRange,
                            format!("{name} can't be tapered to {}", format_length(to)),
                            file,
                            spans.size,
                            "taper it to more than 0mm",
                        )
                        .into());
                }
                for (_, local) in &locals {
                    let across = geom.drawn[local.axis];
                    if to >= across {
                        return Err(self
                            .error(
                                Code::OutOfRange,
                                format!(
                                    "{name} can't be tapered to {}: it is only {} {}",
                                    format_length(to),
                                    format_length(across),
                                    measure_word(local.axis)
                                ),
                                file,
                                spans.size,
                                format!("taper it to less than {}", format_length(across)),
                            )
                            .into());
                    }
                }
                let drawn_length = geom.drawn[PartAxis::Length];
                if square < ExactLength::ZERO {
                    return Err(self
                        .error(
                            Code::OutOfRange,
                            format!("'square' must be 0mm or more, found {}", format_length(square)),
                            file,
                            spans.keep,
                            format!(
                                "say how far {name} stays square from its wide end, as in square: 120mm"
                            ),
                        )
                        .into());
                }
                if square >= drawn_length {
                    return Err(self
                        .error(
                            Code::OutOfRange,
                            format!(
                                "{name} stays square for {}, which is all of its {} length, so nothing is tapered",
                                format_length(square),
                                format_length(drawn_length)
                            ),
                            file,
                            spans.keep,
                            format!("make square: less than {}", format_length(drawn_length)),
                        )
                        .into());
                }
                let to = round_length(to)?;
                let square = round_length(square)?;
                // Distance from the wide end, as a coordinate along the
                // length of the cut box.
                let at = |distance: Length| -> Solved<Length> {
                    match narrow.end {
                        End::End => add(length_start, distance),
                        End::Start => sub(add(length_start, length)?, distance),
                    }
                };
                let mut out = Vec::with_capacity(locals.len());
                for (_, local) in &locals {
                    let (_, across) = drawn_span(geom, local.axis)?;
                    let mut removed = sub(across, to)?;
                    let opposite = LocalFace {
                        axis: local.axis,
                        end: match local.end {
                            End::Start => End::End,
                            End::End => End::Start,
                        },
                    };
                    if locals.iter().any(|(_, other)| *other == opposite) {
                        removed = Length::from_nm(removed.nm() / 2);
                    }
                    let (line, sign) = face_line(geom, *local)?;
                    let marks = vec![
                        Mark {
                            along: at(square)?,
                            across: line,
                        },
                        Mark {
                            along: at(length)?,
                            across: line,
                        },
                        Mark {
                            along: at(length)?,
                            across: inward(line, removed, sign)?,
                        },
                    ];
                    out.push(outline(part, "taper", local.axis, marks));
                }
                Ok(out)
            }
            SquareShape::Arched { rise, ends, .. } => {
                let Some((_, local)) = locals.first().copied() else {
                    return Err(Fault::Internal("the face an arch is sawn into").into());
                };
                let across = geom.drawn[local.axis];
                if rise <= ExactLength::ZERO {
                    return Err(self
                        .error(
                            Code::OutOfRange,
                            format!(
                                "an arch must rise more than 0mm, found {}",
                                format_length(rise)
                            ),
                            file,
                            spans.size,
                            "give the arch a rise of more than 0mm, as in arched(rise: 30mm, ...)",
                        )
                        .into());
                }
                if rise >= across {
                    return Err(self
                        .error(
                            Code::OutOfRange,
                            format!(
                                "a {} arch is too big for {name}: it is only {} {}",
                                format_length(rise),
                                format_length(across),
                                measure_word(local.axis)
                            ),
                            file,
                            spans.size,
                            format!(
                                "make the rise less than {}, so some of {name} is left over the arch",
                                format_length(across)
                            ),
                        )
                        .into());
                }
                if ends < ExactLength::ZERO {
                    return Err(self
                        .error(
                            Code::OutOfRange,
                            format!("'ends' must be 0mm or more, found {}", format_length(ends)),
                            file,
                            spans.keep,
                            format!(
                                "say how much of {name} stays straight at each end, as in ends: 60mm"
                            ),
                        )
                        .into());
                }
                let drawn_length = geom.drawn[PartAxis::Length];
                let both = ends.checked_add(ends).ok_or(Fault::Overflow)?;
                if both >= drawn_length {
                    return Err(self
                        .error(
                            Code::OutOfRange,
                            format!(
                                "{name}'s straight ends take {} of its {} length, so nothing is arched",
                                format_length(both),
                                format_length(drawn_length)
                            ),
                            file,
                            spans.keep,
                            format!(
                                "make ends: less than {}",
                                format_length(Length::from_nm(length.nm() / 2).to_exact())
                            ),
                        )
                        .into());
                }
                let rise = round_length(rise)?;
                let ends = round_length(ends)?;
                let first = add(length_start, ends)?;
                let last = sub(add(length_start, length)?, ends)?;
                let chord = i128::from(sub(last, first)?.nm());
                let (line, sign) = face_line(geom, local)?;
                let mut marks = vec![Mark {
                    along: first,
                    across: line,
                }];
                let steps = i128::from(ARCH_STEPS);
                for step in 1..steps {
                    let from_first = chord.checked_mul(step).ok_or(Fault::Overflow)? / steps;
                    let half = chord / 2;
                    let depth = arch_depth(half, i128::from(rise.nm()), from_first - half)
                        .ok_or(Fault::Overflow)?;
                    let along = add(
                        first,
                        Length::from_nm(i64::try_from(from_first).map_err(|_| Fault::Overflow)?),
                    )?;
                    let depth = Length::from_nm(i64::try_from(depth).map_err(|_| Fault::Overflow)?);
                    marks.push(Mark {
                        along,
                        across: inward(line, depth, sign)?,
                    });
                }
                marks.push(Mark {
                    along: last,
                    across: line,
                });
                Ok(vec![outline(part, "arch", local.axis, marks)])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arch_is_on_its_circle_and_rises_to_its_middle() {
        let mm = 1_000_000i128;
        // A 680mm chord rising 30mm.
        assert_eq!(arch_depth(340 * mm, 30 * mm, 0), Some(30 * mm));
        assert_eq!(arch_depth(340 * mm, 30 * mm, 340 * mm), Some(0));
        let quarter = arch_depth(340 * mm, 30 * mm, 170 * mm).unwrap();
        assert!(quarter > 20 * mm && quarter < 30 * mm, "{quarter}");
    }

    #[test]
    fn or_lists_read_as_english() {
        assert_eq!(or_list(&["top", "bottom"]), "top or bottom");
        assert_eq!(or_list(&["a", "b", "c"]), "a, b or c");
    }
}

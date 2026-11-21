//! Edge profiles (grammar, "Edge profiles"): the chamfers and roundovers a
//! part's `edges:` puts on the edges where two of its faces meet, made as
//! cuts of the part itself.

use stave_lang::Code;
use stave_lang::placement::{Face, PartAxis};
use stave_lang::units::ExactLength;
use stave_model::defs::EdgeProfileDef;
use stave_model::ids::PartId;
use stave_model::solved::{Cut, CutKind, CutPlace, CutSource, End, LocalFace, Orientation};
use stave_model::value::format_length;
use stave_schema::CutOperation;

use crate::joints::{GeomTable, round_length};
use crate::{Fault, Solved, Solver};

/// The face of the part's own frame a world face is, once placed.
pub(crate) fn local_face(orientation: &Orientation, face: Face) -> LocalFace {
    let axis = orientation.part_axis(face.axis());
    LocalFace {
        axis,
        end: if orientation.direction(axis) == face.direction() {
            End::End
        } else {
            End::Start
        },
    }
}

/// How a size along a part's own axis is said.
pub(crate) fn measure_word(axis: PartAxis) -> &'static str {
    match axis {
        PartAxis::Length => "long",
        PartAxis::Width => "wide",
        PartAxis::Thickness => "thick",
    }
}

impl Solver<'_> {
    /// Every part's edge profiles, as cuts of the part. A part in a
    /// mirrored assembly gets its source's with the rest of its cuts; a
    /// mirror image of a part takes its source's `edges:`, on the faces of
    /// its own frame that the source's are on (so a chamfer on the left
    /// edge of a part is on the right edge of its mirror image about
    /// `across`), and its source reports what is wrong with them.
    pub(crate) fn make_edge_profiles(&mut self, geoms: &GeomTable, cuts: &mut [Vec<Cut>]) {
        for index in 0..self.model.parts.len() {
            let part = PartId::from_usize(index);
            if self
                .model
                .parts
                .get(index)
                .is_none_or(|inst| inst.mirror_of.is_some())
            {
                continue;
            }
            let (file, span) = self.part_place(part);
            let made = self.part_edge_profiles(part, geoms);
            if let Some(made) = self.settled(made, file, span)
                && let Some(list) = cuts.get_mut(index)
            {
                list.extend(made);
            }
        }
    }

    fn part_edge_profiles(&mut self, part: PartId, geoms: &GeomTable) -> Solved<Vec<Cut>> {
        let source = self.body_source(part)?;
        let Some(body) = self.part_body(source) else {
            return Ok(Vec::new());
        };
        if body.edges.is_empty() {
            return Ok(Vec::new());
        }
        // A mirror image's source reports.
        let report = source == part;
        let env = self
            .part_env(source)
            .ok_or(Fault::Internal("where a part is declared"))?;
        let geom = geoms.get(source).ok_or(Fault::Earlier)?;
        let orientation = geom.orientation;
        let drawn = geom.drawn;
        let name = self.part_name(source);
        let mut out = Vec::new();
        for profile in &body.edges {
            if profile.edges.is_empty() {
                continue;
            }
            if self.is_round(source)? {
                if !report {
                    return Err(Fault::Earlier.into());
                }
                return Err(self
                    .error(
                        Code::TypeMismatch,
                        format!(
                            "{name} is round, so it has no edges to {}",
                            profile_verb(profile)
                        ),
                        env.file,
                        profile.span,
                        "take edges: off a round or turned part; shape its ends with its profile instead",
                    )
                    .into());
            }
            let size = self.eval_length(&profile.size, &env)?;
            let what = profile.operation.word();
            if size <= ExactLength::ZERO {
                if !report {
                    return Err(Fault::Earlier.into());
                }
                return Err(self
                    .error(
                        Code::OutOfRange,
                        format!(
                            "a {what} must be more than 0mm, found {}",
                            format_length(size)
                        ),
                        env.file,
                        profile.size.span,
                        format!("give the {what} a size of more than 0mm, as in {what}(3mm, ...)"),
                    )
                    .into());
            }
            let rounded = round_length(size)?;
            let kind = match profile.operation {
                CutOperation::Chamfer => CutKind::Chamfer { size: rounded },
                CutOperation::Roundover => CutKind::Roundover { radius: rounded },
                CutOperation::Notch
                | CutOperation::Slot
                | CutOperation::Pocket
                | CutOperation::Tenon
                | CutOperation::Spigot
                | CutOperation::Round
                | CutOperation::Drill
                | CutOperation::DovetailSocket
                | CutOperation::DovetailTail
                | CutOperation::Outline
                | CutOperation::Diagonal => {
                    return Err(Fault::Internal("an edge profile's operation").into());
                }
            };
            for edge in &profile.edges {
                let faces = edge.faces.map(|face| local_face(&orientation, face));
                // The profile must leave some of each face: it is smaller
                // than the part across each face it runs back along.
                for (eaten, local) in [edge.faces[1], edge.faces[0]].into_iter().zip(faces) {
                    let across = drawn[local.axis];
                    if size >= across {
                        if !report {
                            return Err(Fault::Earlier.into());
                        }
                        return Err(self
                            .error(
                                Code::OutOfRange,
                                format!(
                                    "a {} {what} is too big for {name}: it is only {} {} where {} meets {}",
                                    format_length(size),
                                    format_length(across),
                                    measure_word(local.axis),
                                    edge.faces[0].word(),
                                    edge.faces[1].word()
                                ),
                                env.file,
                                profile.size.span,
                                format!(
                                    "make the {what} smaller than {}, so some of {name}'s {} face is left",
                                    format_length(across),
                                    eaten.word()
                                ),
                            )
                            .into());
                    }
                }
                let [face, other] = faces;
                out.push(Cut {
                    source: CutSource::Edges { part: source },
                    name: None,
                    face,
                    place: CutPlace::Edge { other },
                    kind: kind.clone(),
                    wall: None,
                });
            }
        }
        Ok(out)
    }
}

/// What a profile does to an edge, for messages: `chamfer`, `round over`.
fn profile_verb(profile: &EdgeProfileDef) -> &'static str {
    match profile.operation {
        CutOperation::Roundover => "round over",
        CutOperation::Chamfer
        | CutOperation::Notch
        | CutOperation::Slot
        | CutOperation::Pocket
        | CutOperation::Tenon
        | CutOperation::Spigot
        | CutOperation::Round
        | CutOperation::Drill
        | CutOperation::DovetailSocket
        | CutOperation::DovetailTail
        | CutOperation::Outline
        | CutOperation::Diagonal => "chamfer",
    }
}

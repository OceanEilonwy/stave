//! The wood a cut leaves behind (grammar, "Walls" and "Cuts stay in their
//! part"): each cut's own `wall:`, and the backstops no kind should be
//! able to get wrong.
//!
//! 1. **Cuts only remove wood** (`error[cut-size]`): a `tenon` or `spigot`
//!    bigger than the part it is cut on.
//! 2. **Stopped cuts leave wood behind** (`error[cut-size]`): a depth given
//!    as a length must be less than the part there; only `through` goes
//!    through. Checked before the overlap check, which would only report
//!    the same mistake again as the parts that no longer fit; a cut that
//!    says its own `wall:` waits for that to be measured, which says more.
//! 3. **Each cut's `wall:`** (`error[wall]`), measured now that every cut
//!    is made: the least wood between the cut and every face it isn't open
//!    to and every other cut of its part, reported at the joint's or
//!    hardware's line, once for a line.
//! 4. **Closed cuts keep a wall** (`error[wall]`): every `drill` and
//!    `pocket` leaves wood between it and every face it isn't aimed at and
//!    every closed cut of another joint pair or piece of hardware. A hole that runs
//!    into another joint's empty hole is caught here; one that runs into a
//!    pin or tenon is an overlap, already reported.
//! 5. **A part stays one piece** (`error[wall]`, in [`crate::whole`]):
//!    cuts that meet across a part, or one that goes right across it,
//!    leave it in pieces.
//!
//! One mistake, one error: a pair or piece of hardware whose cut's `wall:`
//! wasn't met, or whose cuts are too big or too deep, gets no backstop error
//! for the same cuts, and nothing here is reported for a part or member
//! that already overlaps something (the overlap names what collides).

use stave_geom::wall::{Toward, Wall, Walls};
use stave_lang::placement::PartAxis;
use stave_lang::units::Length;
use stave_lang::{Code, Diagnostic, Label, Severity};
use stave_model::defs::CutName;
use stave_model::solved::{BodyOwner, Cut, CutSource, End, LocalFace, Part, PartId};

use crate::names::{and_list, length_text};
use crate::{Checker, Contact, Item};

/// What messages call a cut: its name, or its operation's word.
pub(crate) fn cut_noun(cut: &Cut) -> String {
    cut.name.as_ref().map_or_else(
        || cut.kind.operation().unnamed_noun().to_owned(),
        CutName::words,
    )
}

/// A face of a part as a message says it: `the front edge of side_l`.
fn face_phrase(part: &Part, face: LocalFace) -> String {
    let direction = part.pose.orientation.direction(face.axis);
    let side = match face.end {
        End::End => direction,
        End::Start => direction.opposite(),
    };
    let word = match face.axis {
        PartAxis::Length => "end",
        PartAxis::Width => "edge",
        PartAxis::Thickness => "face",
    };
    format!("the {} {word} of {}", side.face().word(), part.name)
}

/// How a size along a part's axis is said: `thick`, `wide`, `long`.
fn measure_word(axis: PartAxis) -> &'static str {
    match axis {
        PartAxis::Length => "long",
        PartAxis::Width => "wide",
        PartAxis::Thickness => "thick",
    }
}

/// The member owner of a cut's source, for the overlap check's items.
fn body_owner(source: CutSource) -> Option<BodyOwner> {
    match source {
        CutSource::Joint { joint, pair } => Some(BodyOwner::Joint { joint, pair }),
        CutSource::Hardware { hardware } => Some(BodyOwner::Hardware { hardware }),
        CutSource::Boards { .. } | CutSource::Edges { .. } | CutSource::Shape { .. } => None,
    }
}

/// A cut too big or too deep, waiting to be reported.
#[derive(Debug, Clone)]
pub(crate) struct SizeProblem {
    part: PartId,
    source: CutSource,
    /// The cut says its own `wall:`, which is measured first.
    walled: bool,
    message: String,
    help: String,
}

impl Checker<'_> {
    /// The joint, hardware or part's boards a cut is for.
    pub(crate) fn source_name(&self, source: CutSource) -> String {
        match source {
            CutSource::Joint { joint, .. } => self.joint_name(joint),
            CutSource::Hardware { hardware } => self.hardware_name(hardware),
            CutSource::Boards { part, .. } => format!("{}'s boards", self.part_name(part)),
            CutSource::Edges { part } => format!("{}'s edges", self.part_name(part)),
            CutSource::Shape { part } => format!("{}'s shape", self.part_name(part)),
        }
    }

    /// Where a cut's joint or hardware is written.
    pub(crate) fn source_place(
        &self,
        source: CutSource,
    ) -> Option<(stave_lang::FileId, stave_lang::ast::Span)> {
        match source {
            CutSource::Joint { joint, .. } => self.joint_place(joint),
            CutSource::Hardware { hardware } => self.hardware_place(hardware),
            CutSource::Boards { part, .. }
            | CutSource::Edges { part }
            | CutSource::Shape { part } => self.part_place(part),
        }
    }

    /// `side_l and rod in rod_sockets`, or a piece of hardware's name.
    fn source_pair(&self, source: CutSource) -> String {
        match source {
            CutSource::Joint { joint, pair } => {
                let parts: Vec<String> = self
                    .design
                    .joints()
                    .get(joint.index())
                    .and_then(|j| j.pair(pair))
                    .map(|p| p.roles.iter().map(|r| self.part_name(r.part)).collect())
                    .unwrap_or_default();
                format!("{} in {}", and_list(&parts), self.joint_name(joint))
            }
            CutSource::Hardware { hardware } => self.hardware_name(hardware),
            CutSource::Boards { part, .. } => format!("{}'s boards", self.part_name(part)),
            CutSource::Edges { part } => format!("{}'s edges", self.part_name(part)),
            CutSource::Shape { part } => format!("{}'s shape", self.part_name(part)),
        }
    }

    /// `true` when a part, or a member of a cut's joint pair or piece of
    /// hardware, already has an error at rest.
    pub(crate) fn already_failed(&self, part: PartId, source: CutSource) -> bool {
        self.failed.contains(&Item::Part(part)) || self.owner_overlaps(source)
    }

    /// `true` when a member of a cut's joint pair or piece of hardware
    /// already has an error at rest.
    fn owner_overlaps(&self, source: CutSource) -> bool {
        let Some(owner) = body_owner(source) else {
            return false;
        };
        self.failed.iter().any(|item| match item {
            Item::Member(member) => member.owner == owner,
            Item::Part(_) => false,
        })
    }

    /// Where the least wood round a cut is, as a message says it.
    fn wall_place(&self, part: &Part, wall: &Wall) -> String {
        match wall.toward {
            Toward::Face(face) => face_phrase(part, face),
            Toward::Surface => format!("the side of {}", part.name),
            Toward::Cut(index) => part.cuts.get(index).map_or_else(String::new, |other| {
                format!(
                    "the {} that {} cuts in {}",
                    cut_noun(other),
                    self.source_name(other.source),
                    part.name
                )
            }),
        }
    }

    // -----------------------------------------------------------------------
    // 1 and 2. Cuts only remove wood; stopped cuts leave wood behind
    // -----------------------------------------------------------------------

    /// Cuts too big or too deep. A cut that says its own `wall:` is held
    /// back until that is measured ([`Checker::held_back_cut_sizes`]),
    /// which says more.
    pub(crate) fn cut_sizes(&mut self) {
        let mut found: Vec<SizeProblem> = Vec::new();
        for (part, solid) in self.design.parts().iter().zip(&self.solids) {
            let walls = Walls::of(part, self.design, solid);
            for (index, cut) in part.cuts.iter().enumerate() {
                let noun = cut_noun(cut);
                let owner = self.source_name(cut.source);
                if let Some(oversize) = walls.oversize(index) {
                    let (size, part_size) =
                        (length_text(oversize.size), length_text(oversize.part));
                    let (message, help) = match oversize.axis {
                        Some(axis) => {
                            let word = measure_word(axis);
                            (
                                format!(
                                    "{owner}: its {noun} on {} is {size} {word}, but {} is only {part_size} {word}",
                                    part.name, part.name
                                ),
                                format!(
                                    "a {noun} is the wood left when the wood round it is cut away, so it can't be {word}er than the part; make it less than {part_size} {word}"
                                ),
                            )
                        }
                        None => (
                            format!(
                                "{owner}: its {noun} on {} is {size} across, but {} is only {part_size} across at that end",
                                part.name, part.name
                            ),
                            format!(
                                "a {noun} is the wood left when the wood round it is cut away, so it must be smaller than the end it is cut on; make it less than {part_size} across"
                            ),
                        ),
                    };
                    found.push(SizeProblem {
                        part: part.id,
                        source: cut.source,
                        walled: cut.wall.is_some(),
                        message,
                        help,
                    });
                    continue;
                }
                if let Some(deep) = walls.too_deep(index) {
                    let word = if deep.round {
                        "deep"
                    } else {
                        match deep.axis {
                            PartAxis::Length => "long",
                            PartAxis::Width => "wide",
                            PartAxis::Thickness => "thick",
                        }
                    };
                    let (depth, there) = (length_text(deep.depth), length_text(deep.there));
                    found.push(SizeProblem {
                        part: part.id,
                        source: cut.source,
                        walled: cut.wall.is_some(),
                        message: format!(
                            "{owner}: its {noun} in {} is {depth} deep, but {} is only {there} {word} there",
                            part.name, part.name
                        ),
                        help: format!(
                            "a cut with a depth stops inside the part and leaves wood behind it; make the {noun} less than {there} deep, or write depth: through if it should go right through"
                        ),
                    });
                }
            }
        }
        for problem in found {
            if problem.walled {
                self.held_back.push(problem);
            } else {
                self.report_size(problem);
            }
        }
    }

    /// The cuts too big or too deep that say their own `wall:`, once those
    /// are measured: reported only when the wall didn't already say so.
    pub(crate) fn held_back_cut_sizes(&mut self) {
        for problem in std::mem::take(&mut self.held_back) {
            if self.wall_failed.contains(&problem.source)
                || self.already_failed(problem.part, problem.source)
            {
                continue;
            }
            self.report_size(problem);
        }
    }

    fn report_size(&mut self, problem: SizeProblem) {
        let SizeProblem {
            part,
            source,
            message,
            help,
            ..
        } = problem;
        self.reported.insert(part);
        self.failed.insert(Item::Part(part));
        if !self.wall_failed.contains(&source) {
            self.wall_failed.push(source);
        }
        self.next_contact = Some(Contact::of(self.design, &[Item::Part(part)], &[]));
        self.report(
            Severity::Error,
            Code::CutSize,
            message,
            self.source_place(source),
            Vec::new(),
            Some(help),
        );
    }

    // -----------------------------------------------------------------------
    // 3 and 4. Each cut's wall, and closed cuts keep a wall
    // -----------------------------------------------------------------------

    pub(crate) fn walls(&mut self) {
        // The solids are lent out while the walls borrow them.
        let solids = std::mem::take(&mut self.solids);
        let design = self.design;
        {
            let walls: Vec<Walls<'_>> = design
                .parts()
                .iter()
                .zip(&solids)
                .map(|(part, solid)| Walls::of(part, design, solid))
                .collect();
            self.cut_walls(&walls);
            self.held_back_cut_sizes();
            self.closed_cuts(&walls);
        }
        self.solids = solids;
        self.whole_parts();
    }

    /// Every cut that says its own `wall:` leaves at least that much wood
    /// round it (grammar, "Walls"): to every face it isn't open to and to
    /// every other cut of its part. Reported once for a pair or piece of
    /// hardware, at its line, the other pairs of the same line as notes.
    fn cut_walls(&mut self, walls: &[Walls<'_>]) {
        struct Thin {
            part: PartId,
            index: usize,
            wall: Wall,
            needs: Length,
        }
        let mut thin: Vec<(CutSource, Thin)> = Vec::new();
        let mut unmeasured: Vec<(CutSource, String)> = Vec::new();
        for (part, part_walls) in self.design.parts().iter().zip(walls) {
            for (index, cut) in part.cuts.iter().enumerate() {
                let Some(needs) = cut.wall else {
                    continue;
                };
                if self.already_failed(part.id, cut.source)
                    || thin.iter().any(|(source, _)| *source == cut.source)
                {
                    continue;
                }
                // A cut whose joint or hardware already overlaps something
                // is left to that overlap, which names what collides. The
                // other cuts of the same pair or piece of hardware are made
                // with it (a counterbore and its shank hole) and are no
                // walls to it; its own copies are (a row of dowel holes).
                let skip = |other: usize| {
                    part.cuts.get(other).is_some_and(|other| {
                        self.owner_overlaps(other.source)
                            || (other.source == cut.source
                                && (other.name != cut.name
                                    || other.kind.operation() != cut.kind.operation()))
                    })
                };
                let Some(wall) = part_walls.wall(index, skip) else {
                    if !unmeasured.iter().any(|(source, _)| *source == cut.source) {
                        unmeasured.push((cut.source, cut_noun(cut)));
                    }
                    continue;
                };
                if wall.least >= needs {
                    continue;
                }
                thin.push((
                    cut.source,
                    Thin {
                        part: part.id,
                        index,
                        wall,
                        needs,
                    },
                ));
            }
        }
        for (source, noun) in unmeasured {
            let Some((file, span)) = self.source_place(source) else {
                continue;
            };
            let mut diagnostic = Diagnostic::warning(
                Code::NotSupported,
                format!("measuring the wall round {noun} is not supported yet"),
                Label::new(file, span),
            )
            .with_help("Stave doesn't measure the wall round dovetails, outlines, angled slots or the cuts between boards yet; this cut's wall: is not checked");
            diagnostic.severity = Severity::Warning;
            self.diagnostics.push((diagnostic, None));
        }
        for (source, found) in thin {
            if !self.wall_failed.contains(&source) {
                self.wall_failed.push(source);
            }
            let Some(part) = self.design.part(found.part) else {
                continue;
            };
            let Some((file, span)) = self.source_place(source) else {
                continue;
            };
            let pair = self.source_pair(source);
            // One mistake, one error: another pair of the same line with a
            // wall too thin is a note on the first.
            if let Some((earlier, _)) = self.diagnostics.iter_mut().find(|(d, _)| {
                d.code == Code::Wall
                    && d.primary()
                        .is_some_and(|label| label.file == file && label.span == span)
            }) {
                let note = format!("the same for {pair}");
                if !earlier.notes.contains(&note) {
                    earlier.notes.push(note);
                }
                continue;
            }
            let owner = self.source_name(source);
            let noun = part.cuts.get(found.index).map(cut_noun).unwrap_or_default();
            let place = self.wall_place(part, &found.wall);
            let needs = length_text(found.needs);
            let message = match found.wall.past {
                Some(past) => format!(
                    "{owner}: its {noun} in {} runs {} past {place}; it needs {needs} of wood there",
                    part.name,
                    length_text(past)
                ),
                None => format!(
                    "{owner}: its {noun} in {} leaves {} of wood at {place}; it needs {needs}",
                    part.name,
                    length_text(found.wall.least)
                ),
            };
            let diagnostic = Diagnostic::error(Code::Wall, message, Label::new(file, span))
                .with_note(format!("for {pair}"))
                .with_help(format!(
                    "move the parts so the {noun} has {needs} of wood round it, or change the joint's settings"
                ));
            let contact = Contact::of(self.design, &[Item::Part(found.part)], &[]);
            self.diagnostics.push((diagnostic, Some(contact)));
        }
    }

    /// Every `drill` and `pocket` keeps wood between it and every face it
    /// isn't aimed at, and every closed cut of another joint pair or piece
    /// of hardware.
    fn closed_cuts(&mut self, walls: &[Walls<'_>]) {
        let mut found: Vec<(PartId, CutSource, String, String)> = Vec::new();
        // One mistake, one error: two cuts that run into each other are
        // reported once, for the first; the second leaves the first alone.
        let mut flagged: Vec<CutSource> = Vec::new();
        for (part, part_walls) in self.design.parts().iter().zip(walls) {
            for (index, cut) in part.cuts.iter().enumerate() {
                if !part_walls.is_closed(index)
                    || self.wall_failed.contains(&cut.source)
                    || self.already_failed(part.id, cut.source)
                {
                    continue;
                }
                // Only closed cuts are walls to each other here: how near a hole may
                // come to a groove or a dado is a kind's judgement (`wall()`).
                let skip = |other_index: usize| {
                    !part_walls.is_closed(other_index)
                        || part.cuts.get(other_index).is_some_and(|other| {
                            other.source == cut.source
                                || self.wall_failed.contains(&other.source)
                                || flagged.contains(&other.source)
                                || self.owner_overlaps(other.source)
                        })
                };
                let Some(wall) = part_walls.wall(index, skip) else {
                    continue;
                };
                if wall.least > stave_lang::units::Length::ZERO {
                    continue;
                }
                let owner = self.source_name(cut.source);
                let noun = cut_noun(cut);
                let (message, help) = match wall.toward {
                    Toward::Face(_) | Toward::Surface => {
                        let place = self.wall_place(part, &wall);
                        let how = match wall.past {
                            Some(past) => format!("runs {} past {place}", length_text(past)),
                            None => format!("leaves no wood at {place}"),
                        };
                        (
                            format!("{owner}: its {noun} in {} breaks out of {place}", part.name),
                            format!(
                                "the {noun} {how}; move the parts so the {noun} sits inside {} with wood all round it, or make the {noun} smaller",
                                part.name
                            ),
                        )
                    }
                    Toward::Cut(other_index) => {
                        let Some(other) = part.cuts.get(other_index) else {
                            continue;
                        };
                        let other_noun = cut_noun(other);
                        let other_owner = self.source_name(other.source);
                        let verb = if wall.past.is_some() {
                            "runs into"
                        } else {
                            "meets"
                        };
                        let how = match wall.past {
                            Some(past) => format!(
                                "the {noun} runs {} into the {other_noun} in {}",
                                length_text(past),
                                part.name
                            ),
                            None => format!(
                                "the {noun} and the {other_noun} leave no wood between them in {}",
                                part.name
                            ),
                        };
                        (
                            format!(
                                "{owner}: its {noun} in {} {verb} the {other_noun} of {other_owner}",
                                part.name
                            ),
                            format!(
                                "{how}; move one of the joints so there is wood between the two cuts, or make one of them smaller"
                            ),
                        )
                    }
                };
                if !flagged.contains(&cut.source) {
                    flagged.push(cut.source);
                }
                found.push((part.id, cut.source, message, help));
            }
        }
        for (part, source, message, help) in found {
            if !self.wall_failed.contains(&source) {
                self.wall_failed.push(source);
            }
            self.next_contact = Some(Contact::of(self.design, &[Item::Part(part)], &[]));
            self.report(
                Severity::Error,
                Code::Wall,
                message,
                self.source_place(source),
                Vec::new(),
                Some(help),
            );
        }
    }
}

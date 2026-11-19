//! A part stays one piece (grammar, "Cuts stay in their part"): once
//! every cut is made, a part's wood must be one connected piece. Two cuts
//! from opposite faces that meet, or one `through` cut right across the
//! part, leave it in pieces that the cut list would still call one part
//! (`error[wall]`, at the joint or hardware whose cut parts it).
//!
//! One mistake, one error: a part that already has an error, or whose
//! cuts already broke a wall rule or a size, is left to that error.

use stave_geom::whole::{Severed, severed};
use stave_lang::Code;
use stave_lang::Severity;
use stave_model::solved::{Cut, CutSource, Part, PartId};

use crate::names::and_list;
use crate::walls::cut_noun;
use crate::{Checker, Contact, Item};

/// `in two`, `into three pieces`.
fn pieces_phrase(pieces: usize) -> String {
    match pieces {
        0..=2 => "in two".to_owned(),
        3 => "into three pieces".to_owned(),
        4 => "into four pieces".to_owned(),
        many => format!("into {many} pieces"),
    }
}

impl Checker<'_> {
    /// The other parts of a cut's joint pair, as a list: `shelf_l`.
    fn cut_for(&self, cut: &Cut, part: PartId) -> String {
        let CutSource::Joint { joint, pair } = cut.source else {
            return String::new();
        };
        let parts: Vec<String> = self
            .design
            .joints
            .get(joint.index())
            .and_then(|j| j.pairs.get(pair))
            .map(|p| {
                p.roles
                    .iter()
                    .filter(|role| role.part != part)
                    .map(|role| self.part_name(role.part))
                    .collect()
            })
            .unwrap_or_default();
        and_list(&parts)
    }

    /// Every part still one piece of wood once its cuts are made.
    pub(crate) fn whole_parts(&mut self) {
        let mut found = Vec::new();
        for (part, solid) in self.design.parts.iter().zip(&self.solids) {
            if self.failed.contains(&Item::Part(part.id)) {
                continue;
            }
            if let Some(cut) = severed(part, self.design, solid) {
                found.push((part.id, cut));
            }
        }
        for (part_id, cut) in found {
            let Some(part) = self.design.part(part_id) else {
                continue;
            };
            self.report_severed(part, cut);
        }
    }

    fn report_severed(&mut self, part: &Part, severed: Severed) {
        let pieces = pieces_phrase(severed.pieces);
        let Some(cut) = part.cuts.get(severed.cut) else {
            return;
        };
        let meets = severed.meets.and_then(|index| part.cuts.get(index));
        let sources: Vec<CutSource> = std::iter::once(cut)
            .chain(meets)
            .map(|c| c.source)
            .collect();
        if sources.iter().any(|source| {
            self.wall_failed.contains(source) || self.already_failed(part.id, *source)
        }) {
            return;
        }
        let owner = self.source_name(cut.source);
        let noun = cut_noun(cut);
        let own = match self.cut_for(cut, part.id) {
            others if others.is_empty() => format!("in {}", part.name),
            others => format!("for {others}"),
        };
        let (message, help) = match meets {
            Some(other) => {
                let other_noun = cut_noun(other);
                let theirs = match (other.source, cut.source) {
                    (CutSource::Joint { joint: a, .. }, CutSource::Joint { joint: b, .. })
                        if a == b =>
                    {
                        format!("for {}", self.cut_for(other, part.id))
                    }
                    _ => format!("of {}", self.source_name(other.source)),
                };
                (
                    format!(
                        "{owner}: its {noun} {own} meets the {other_noun} {theirs} and cuts {} {pieces}",
                        part.name
                    ),
                    format!(
                        "together the two cuts go right through {} and leave no wood between them; make one of them shallower, or move one of the joints so the cuts don't line up",
                        part.name
                    ),
                )
            }
            None => (
                format!(
                    "{owner}: its {noun} {own} goes right across {} and cuts it {pieces}",
                    part.name
                ),
                format!(
                    "a cut that goes through must stop short of an edge, or {} falls apart; make the {noun} shallower, or move the parts so it stops inside {}",
                    part.name, part.name
                ),
            ),
        };
        let place = self.source_place(cut.source);
        for source in sources {
            if !self.wall_failed.contains(&source) {
                self.wall_failed.push(source);
            }
        }
        self.failed.insert(Item::Part(part.id));
        self.next_contact = Some(Contact::of(self.design, &[Item::Part(part.id)], &[]));
        self.report(
            Severity::Error,
            Code::Wall,
            message,
            place,
            Vec::new(),
            Some(help),
        );
    }
}

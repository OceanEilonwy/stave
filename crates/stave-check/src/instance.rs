//! Problems between a component instance and something outside it.
//!
//! When parts of an assembly (a door, a drawer) run into something outside
//! the assembly, the mistake is almost never in the component (often a
//! library), but where the instance is placed: the `assembly` line in the
//! user's file, and in particular the `Mount` it is given (`mount:
//! overlay(top: 10mm)`). Every such overlap between one instance (or its
//! mirror image) and one other item is a single error there, naming the
//! assembly; the component's parts involved are labelled, and the help
//! says what to change at that line.

use std::collections::BTreeMap;

use stave_geom::collide::Overlap;
use stave_lang::ast::Span;
use stave_lang::placement::Direction;
use stave_lang::{Code, Diagnostic, FileId, Label, Severity};
use stave_model::defs::AssemblyKind;
use stave_model::solved::{AssemblyId, WorldBox};
use stave_model::types::Ty;

use crate::names::{length_text, list};
use crate::{Checker, Contact, Item, deepest};

/// Two items whose wood overlaps, and where.
pub(crate) type Clash = (Item, Item, Vec<Overlap>);

/// Where an instance is placed in the user's file.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InstancePlace {
    pub(crate) file: FileId,
    pub(crate) span: Span,
    /// The place is the instance's `Mount` argument.
    pub(crate) mount: bool,
}

/// The overlaps of one instance with one other item.
pub(crate) struct Group {
    /// The instance (the source, for a mirrored one).
    root: AssemblyId,
    /// The other item.
    other: Item,
    /// The instance's items involved, with their outermost assembly.
    members: Vec<(Item, AssemblyId)>,
    /// Every overlap.
    found: Vec<Overlap>,
}

/// The overlap edge of a front a `Mount` field controls, for the direction
/// the front must move to clear: moving down shrinks how far it reaches
/// past the top edge, and so on.
fn mount_field(direction: Direction, toward_front: bool) -> Option<&'static str> {
    match direction {
        Direction::Down => Some("top"),
        Direction::Up => Some("bottom"),
        Direction::Right if toward_front => Some("meeting"),
        Direction::Left if toward_front => Some("meeting"),
        Direction::Right => Some("left"),
        Direction::Left => Some("right"),
        Direction::Front | Direction::Back => None,
    }
}

impl Checker<'_> {
    /// The outermost assembly an item moves with.
    pub(crate) fn outer(&self, item: Item) -> Option<AssemblyId> {
        self.bodies
            .iter()
            .find(|b| b.item == item)
            .and_then(|b| b.chain.first().copied())
    }

    /// The assembly a mirrored one is the image of (itself otherwise).
    pub(crate) fn mirror_root(&self, mut assembly: AssemblyId) -> AssemblyId {
        for _ in 0..self.design.assemblies.len() {
            match self
                .design
                .assemblies
                .get(assembly.index())
                .and_then(|a| a.mirror_of)
            {
                Some(source) => assembly = source,
                None => break,
            }
        }
        assembly
    }

    /// Where an instance is placed: its `Mount` argument, or its name.
    pub(crate) fn instance_place(&self, assembly: AssemblyId) -> Option<InstancePlace> {
        let inst = self.model.assemblies.get(assembly.index())?;
        let def = self.model.defs.assembly(inst.def)?;
        if let AssemblyKind::Instance { args, .. } = &def.kind
            && let Some((_, value)) = args.iter().find(|(_, value)| value.ty == Ty::Mount)
        {
            return Some(InstancePlace {
                file: def.decl.file,
                span: value.span,
                mount: true,
            });
        }
        Some(InstancePlace {
            file: def.decl.file,
            span: def.decl.name_span,
            mount: false,
        })
    }

    /// The instance to blame for an overlap between `a` and `b`, and the
    /// other item: the one of them inside an assembly the other isn't in.
    pub(crate) fn blame(&self, a: Item, b: Item) -> Option<(AssemblyId, Item, Item)> {
        match (self.outer(a), self.outer(b)) {
            (Some(x), Some(y)) if x == y => None,
            (Some(x), _) => Some((x, a, b)),
            (None, Some(y)) => Some((y, b, a)),
            (None, None) => None,
        }
    }

    /// Sorts clashes into groups per instance and other item; returns the
    /// groups and the clashes that aren't about an instance.
    pub(crate) fn group(&self, clashes: Vec<Clash>) -> (Vec<Group>, Vec<Clash>) {
        let mut groups: BTreeMap<(AssemblyId, Item), Group> = BTreeMap::new();
        let mut rest = Vec::new();
        for (a, b, found) in clashes {
            match self.blame(a, b) {
                Some((outer, member, other)) => {
                    let root = self.mirror_root(outer);
                    let group = groups.entry((root, other)).or_insert_with(|| Group {
                        root,
                        other,
                        members: Vec::new(),
                        found: Vec::new(),
                    });
                    group.members.push((member, outer));
                    group.found.extend(found);
                }
                None => rest.push((a, b, found)),
            }
        }
        (groups.into_values().collect(), rest)
    }

    /// The box around an assembly's items.
    fn assembly_bounds(&self, assembly: AssemblyId) -> Option<WorldBox> {
        self.bodies
            .iter()
            .filter(|b| b.chain.first() == Some(&assembly))
            .map(|b| b.bounds)
            .reduce(|x, y| stave_geom::collide::union(&x, &y))
    }

    pub(crate) fn report_group(&mut self, group: Group) {
        let Some(deepest) = deepest(&group.found).copied() else {
            return;
        };
        let name = self.assembly_name(group.root);
        let other = self.item_name(group.other);
        let depth = length_text(deepest.depth);
        let place = self.instance_place(group.root);
        // The direction the instance moves to clear the other item.
        let direction = self.assembly_bounds(group.root).and_then(|bounds| {
            let axis = deepest.axis;
            let middle = |b: &WorldBox| {
                i128::from(b.min().get(axis).nm()) + i128::from(b.max().get(axis).nm())
            };
            let positive = middle(&bounds) > middle(&deepest.region);
            Direction::ALL
                .into_iter()
                .find(|d| d.axis() == axis && d.is_positive() == positive)
        });
        let other_is_front = self.outer(group.other).is_some();
        let field = place
            .filter(|p| p.mount)
            .and(direction)
            .and_then(|d| mount_field(d, other_is_front));
        let help = match field {
            Some(field) => format!(
                "{name} reaches {depth} into {other}; make its {field} overlay {depth} smaller"
            ),
            None => format!(
                "{name} reaches {depth} into {other}; change how it is placed here so it clears {other} by {depth}"
            ),
        };
        let mut members: Vec<String> = Vec::new();
        let mut mirrors: Vec<String> = Vec::new();
        for (item, outer) in &group.members {
            if *outer == group.root {
                let item_name = self.item_name(*item);
                if !members.contains(&item_name) {
                    members.push(item_name);
                }
            } else {
                let mirror = self.assembly_name(*outer);
                if !mirrors.contains(&mirror) {
                    mirrors.push(mirror);
                }
            }
        }
        let mut notes = vec![format!(
            "{} {} into {other}, {} deep at most",
            list(&members),
            if members.len() == 1 { "goes" } else { "go" },
            depth
        )];
        if !mirrors.is_empty() {
            notes.push(format!(
                "the same happens to {}, the mirror image of {name}",
                list(&mirrors)
            ));
        }
        let Some(place) = place.or_else(|| {
            self.project_place().map(|(file, span)| InstancePlace {
                file,
                span,
                mount: false,
            })
        }) else {
            return;
        };
        let mut diagnostic = Diagnostic::error(
            Code::Overlap,
            format!("{name} and {other} overlap"),
            Label::new(place.file, place.span),
        );
        diagnostic.severity = Severity::Error;
        diagnostic.notes = notes;
        diagnostic.help = Some(help);
        // Point at the component's parts involved.
        let mut labelled = 0;
        for (item, outer) in &group.members {
            if *outer != group.root || labelled >= 3 {
                continue;
            }
            if let Item::Part(part) = item
                && let Some((file, span)) = self.part_place(*part)
                && !diagnostic
                    .labels
                    .iter()
                    .any(|l| l.span == span && l.file == file)
            {
                diagnostic = diagnostic.with_label(Label::with_message(
                    file,
                    span,
                    format!("{} is part of {name}", self.part_name(*part)),
                ));
                labelled += 1;
            }
        }
        let mut contact = Contact::default();
        contact.add_item(group.other);
        for (item, _) in &group.members {
            contact.add_item(*item);
        }
        contact.regions = group.found.iter().map(|o| o.region).collect();
        self.diagnostics.push((diagnostic, Some(contact)));
    }
}

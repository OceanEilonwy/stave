//! What a viewer shows, worked out without any UI: the scene graph of a
//! design, a description of the selected part or hardware, and the
//! project's problems as plain rows.

use std::path::Path;

use stave_lang::units::Length;
use stave_lang::{Diagnostic, Severity, SourceMap};
use stave_model::solved::{Envelope, HardwareBase, HardwareData, Part, SolvedDesign};
pub use stave_scene::{
    BuildOptions, Extent, Highlight, Item, MotionRange, Posed, SceneEntry, SceneGraph,
};

use crate::{Contact, ContactItem};

use crate::Opened;

/// The scene graph a viewer draws.
///
/// Every part is drawn as cut, with its cuts made: tenons sit inside
/// their mortises, shelves in their dados, and the joinery shows wherever
/// it is visible. Parts that fit meet face to face without overlapping, so
/// nothing flickers.
pub fn scene(design: &SolvedDesign) -> SceneGraph {
    SceneGraph::build(
        design,
        BuildOptions {
            extent: Extent::Cut,
        },
    )
}

/// One labelled value of a description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// What it is: "Stock".
    pub label: &'static str,
    /// Its value: "oak_19 (white oak, solid)".
    pub value: String,
}

/// What a viewer shows about the selected item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description {
    /// Its name as written: `door_l.stile[hinge]`.
    pub name: String,
    /// "Part", "Made part", "Mirror image", "Hinge", "Slide", "Pull".
    pub kind: &'static str,
    /// Labelled values, in display order.
    pub rows: Vec<Row>,
}

/// A length in millimetres without the unit: `1149.5`.
fn millimetres(length: Length) -> String {
    let text = length.to_string();
    text.strip_suffix("mm").map_or(text.clone(), str::to_owned)
}

fn size_text(length: Length, width: Length, thickness: Length) -> String {
    format!(
        "{} × {} × {} mm",
        millimetres(length),
        millimetres(width),
        millimetres(thickness)
    )
}

fn assembly_path(
    design: &SolvedDesign,
    mut assembly: Option<stave_model::solved::AssemblyId>,
) -> String {
    let mut names = Vec::new();
    while let Some(id) = assembly {
        let Some(found) = design.assemblies.get(id.index()) else {
            break;
        };
        if names.len() > design.assemblies.len() {
            break;
        }
        names.push(format!("{} ({})", found.name, found.component));
        assembly = found.parent;
    }
    if names.is_empty() {
        "the project itself".to_owned()
    } else {
        names.reverse();
        names.join(" › ")
    }
}

fn describe_part(design: &SolvedDesign, part: &Part) -> Description {
    use stave_model::solved::{PartOrigin, Shape};
    let kind = match part.origin {
        PartOrigin::Declared => "Part",
        PartOrigin::Mirror { .. } | PartOrigin::MirroredAssembly { .. } => "Mirror image",
        PartOrigin::Made { .. } | PartOrigin::Boards { .. } => "Made part",
    };
    let mut rows = vec![
        Row {
            label: "Stock",
            value: format!("{} ({})", part.stock.name, part.stock.form.word()),
        },
        Row {
            label: "Species",
            value: part.stock.species.title().to_owned(),
        },
        Row {
            label: "Cut size",
            value: size_text(part.cut.length, part.cut.width, part.cut.thickness),
        },
    ];
    if part.cut != part.drawn {
        rows.push(Row {
            label: "Drawn size",
            value: size_text(part.drawn.length, part.drawn.width, part.drawn.thickness),
        });
    }
    match &part.shape {
        Shape::Board => {}
        Shape::Round { diameter } => rows.push(Row {
            label: "Shape",
            value: format!("round, {} mm across", millimetres(*diameter)),
        }),
        Shape::Turned { profile, .. } => rows.push(Row {
            label: "Shape",
            value: format!("turned to {profile}"),
        }),
    }
    rows.push(Row {
        label: "Grain",
        value: format!("along the {}", part.grain.word()),
    });
    if let PartOrigin::Mirror { source } | PartOrigin::MirroredAssembly { source } = part.origin
        && let Some(source) = design.part(source)
    {
        rows.push(Row {
            label: "Mirror of",
            value: source.name.clone(),
        });
    }
    rows.push(Row {
        label: "Assembly",
        value: assembly_path(design, part.assembly),
    });
    if let Some(note) = &part.note {
        rows.push(Row {
            label: "Note",
            value: note.clone(),
        });
    }
    Description {
        name: part.name.clone(),
        kind,
        rows,
    }
}

/// Describes a part or piece of hardware for the selection panel.
pub fn describe(design: &SolvedDesign, item: Item) -> Option<Description> {
    match item {
        Item::Part(id) => design.part(id).map(|part| describe_part(design, part)),
        Item::Hardware(id) => {
            let hardware = design.hardware.get(id.index())?;
            let kind = match hardware.base {
                HardwareBase::Slide => "Slide",
                HardwareBase::Hinge => "Hinge",
                HardwareBase::Pull => "Pull",
            };
            let shape = |envelope: &Envelope| match *envelope {
                Envelope::Box {
                    length,
                    width,
                    thickness,
                } => format!("box {}", size_text(length, width, thickness)),
                // A hinge's cup is sunk into the door; everything else
                // stands off its face.
                Envelope::Cylinder { diameter, length } => format!(
                    "cylinder {} mm across, {} mm {}",
                    millimetres(diameter),
                    millimetres(length),
                    if hardware.base == HardwareBase::Hinge {
                        "deep"
                    } else {
                        "long"
                    }
                ),
            };
            let (moving_label, fixed_label) = match hardware.base {
                HardwareBase::Slide => ("Drawer member", "Cabinet member"),
                HardwareBase::Hinge if hardware.is_butt_hinge() => ("Leaf", "Fixed leaf"),
                HardwareBase::Hinge => ("Cup", "Plate"),
                HardwareBase::Pull => ("Envelope", "Fixed member"),
            };
            let piece = design
                .part(hardware.piece)
                .map_or_else(|| "?".to_owned(), |p| p.name.clone());
            Some(Description {
                name: hardware.name.clone(),
                kind,
                rows: [
                    Row {
                        label: "Kind",
                        value: hardware.kind.clone(),
                    },
                    Row {
                        label: "Mounted on",
                        value: format!("{piece}, {} face", hardware.mount_face.word()),
                    },
                    Row {
                        label: moving_label,
                        value: shape(&hardware.envelope),
                    },
                ]
                .into_iter()
                .chain(hardware.fixed.as_ref().map(|member| Row {
                    label: fixed_label,
                    value: if hardware.is_butt_hinge() {
                        format!(
                            "{}, housed in {}",
                            shape(&member.envelope),
                            match hardware.data {
                                HardwareData::ButtHinge { on, .. } => design
                                    .part(on)
                                    .map_or_else(|| "?".to_owned(), |p| p.name.clone()),
                                _ => "?".to_owned(),
                            }
                        )
                    } else {
                        format!("{}, fixed to the carcass", shape(&member.envelope))
                    },
                }))
                .chain(hardware.mechanism.as_ref().map(|mechanism| Row {
                    label: "Mechanism",
                    value: format!(
                        "a four-bar linkage: an arm on the plate, two links ({} mm and {} mm, pin to pin) and the boss in the cup; opens {}",
                        millimetres(link_length(&mechanism.linkage, 0)),
                        millimetres(link_length(&mechanism.linkage, 1)),
                        mechanism.linkage.opening_angle()
                    ),
                }))
                .chain([Row {
                    label: "Assembly",
                    value: assembly_path(design, hardware.assembly),
                }])
                .collect(),
            })
        }
    }
}

/// The length of one of a linkage's links, pin to pin, to the nearest
/// nanometre (the integer square root of its squared length).
fn link_length(linkage: &stave_model::solved::Linkage, index: usize) -> Length {
    let Some(link) = linkage.links().get(index) else {
        return Length::ZERO;
    };
    let dx = i128::from(link.coupler.x.nm()).saturating_sub(i128::from(link.ground.x.nm()));
    let dy = i128::from(link.coupler.y.nm()).saturating_sub(i128::from(link.ground.y.nm()));
    let square = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
    // Newton's method from above.
    let mut root = square.max(1);
    loop {
        let next = root.saturating_add(square / root) / 2;
        if next >= root {
            break;
        }
        root = next;
    }
    Length::from_nm(i64::try_from(root).unwrap_or(i64::MAX))
}

/// A diagnostic as rows for a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Error, warning or note.
    pub severity: Severity,
    /// The code: `axis-not-set`.
    pub code: String,
    /// The message.
    pub message: String,
    /// `cabinet.stave:12:5`, when it points into a file.
    pub location: Option<String>,
    /// What to do about it.
    pub help: Option<String>,
    /// What it is about in the design, for a viewer to pose and show.
    pub contact: Option<Contact>,
}

fn problem(sources: &SourceMap, folder: &Path, diagnostic: &Diagnostic) -> Problem {
    let location = diagnostic.primary().and_then(|label| {
        let file = sources.get(label.file)?;
        let at = file.line_column(label.span.start());
        let path = Path::new(file.path());
        let shown = path.strip_prefix(folder).unwrap_or(path);
        Some(format!("{}:{}:{}", shown.display(), at.line, at.column))
    });
    Problem {
        severity: diagnostic.severity,
        code: diagnostic.code.as_str().to_owned(),
        message: diagnostic.message.clone(),
        location,
        help: diagnostic.help.clone(),
        contact: None,
    }
}

/// The project's diagnostics as rows, in their sorted order. Files inside
/// `folder` are named relative to it.
pub fn problems(opened: &Opened, folder: &Path) -> Vec<Problem> {
    opened
        .diagnostics
        .iter()
        .map(|diagnostic| Problem {
            contact: opened.contact(diagnostic).cloned(),
            ..problem(&opened.sources, folder, diagnostic)
        })
        .collect()
}

/// The scene a problem happens in: its scene, or `rest`.
pub fn contact_scene<'g>(graph: &'g SceneGraph, contact: &Contact) -> Option<&'g SceneEntry> {
    match contact.scene {
        Some(id) => graph.scenes.iter().find(|scene| scene.id == id),
        None => graph.scenes.first(),
    }
}

/// The design posed where a problem happens: its scene's poses, and a
/// motion stopped a little past where it first hits (where the shared
/// volume is measured). `amount` overrides that motion (a slider), in
/// radians or millimetres.
pub fn pose_at_contact(
    graph: &SceneGraph,
    contact: &Contact,
    amount: Option<f32>,
) -> Option<Posed> {
    let scene = contact_scene(graph, contact)?;
    let overrides: Vec<_> = contact
        .motion
        .iter()
        .filter_map(|motion| {
            let value = amount.or_else(|| graph.amount(motion.assembly, motion.shown))?;
            Some((motion.assembly, value))
        })
        .collect();
    Some(graph.pose_with(scene, &overrides, 0.0))
}

/// The range a problem's motion can be scrubbed over, when it has one.
pub fn contact_motion_range(graph: &SceneGraph, contact: &Contact) -> Option<MotionRange> {
    graph.motion_range(contact.motion?.assembly)
}

/// What to tint and draw in red for a problem: every node of the items it
/// is about, and the shared wood (measured in the pose of
/// [`pose_at_contact`]).
pub fn highlight(graph: &SceneGraph, contact: &Contact) -> Highlight {
    let nodes = graph
        .node_ids()
        .filter(|id| {
            graph.node(*id).is_some_and(|node| {
                contact.items.iter().any(|item| match (item, node.item) {
                    (ContactItem::Part(a), Item::Part(b)) => *a == b,
                    (ContactItem::Hardware(a), Item::Hardware(b)) => *a == b,
                    (ContactItem::Part(_), Item::Hardware(_))
                    | (ContactItem::Hardware(_), Item::Part(_)) => false,
                })
            })
        })
        .collect();
    let volumes = contact
        .regions
        .iter()
        .filter_map(|region| {
            stave_geom::Aabb::around([
                stave_geom::render::point(region.min()),
                stave_geom::render::point(region.max()),
            ])
        })
        .collect();
    Highlight { nodes, volumes }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_show_in_millimetres() {
        assert_eq!(millimetres(Length::from_nm(1_149_500_000)), "1149.5");
        assert_eq!(
            size_text(
                Length::from_mm(900),
                Length::from_mm(560),
                Length::from_mm(19)
            ),
            "900 × 560 × 19 mm"
        );
    }
}

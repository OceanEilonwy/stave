//! What a viewer shows, worked out without any UI: the scene graph of a
//! design, a description of the selected part or hardware, and the
//! project's problems as plain rows.

use crate::i18n::{fill_in, ntr, tr};
use std::path::Path;

use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_lang::{Diagnostic, Severity, SourceMap};
use stave_model::mechanism::{DriveValue, Mechanism, MechanismEntry};
use stave_model::solved::{
    Body, BodyCarrier, Hardware, HardwareBase, MemberItem, MemberShape, Part, SolvedDesign,
    WorldBox,
};
pub use stave_scene::{
    BuildOptions, Extent, Highlight, Item, Look, MeshCache, MotionRange, NodeId, Posed, SceneEntry,
    SceneGraph,
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
    scene_with(design, &MeshCache::new())
}

/// [`scene`], taking the part meshes whose cuts haven't changed from
/// `meshes` (the last build that used it) and leaving this build's in it.
pub fn scene_with(design: &SolvedDesign, meshes: &MeshCache) -> SceneGraph {
    SceneGraph::build_with(
        design,
        BuildOptions {
            extent: Extent::Cut,
        },
        meshes,
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
    match text.strip_suffix("mm") {
        Some(number) => number.to_owned(),
        None => text,
    }
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
        let Some(found) = design.assembly(id) else {
            break;
        };
        if names.len() > design.assemblies().len() {
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
        _ if part.pieces.is_some() => "Piece of a spliced part",
        PartOrigin::Piece { .. } => "Piece of a spliced part",
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

/// Describes a part, piece of hardware or joint's member for the
/// selection panel.
pub fn describe(design: &SolvedDesign, item: Item) -> Option<Description> {
    match item {
        Item::Part(id) => design.part(id).map(|part| describe_part(design, part)),
        Item::Member(member) => {
            let body = design.pair_member(member)?;
            let joint = design.joint(member.joint)?;
            let mut rows = vec![Row {
                label: "Placed by",
                value: format!("{} ({})", joint.name, joint.kind),
            }];
            rows.extend(body.item.as_ref().map(|item| Row {
                label: "Fastener",
                value: item_text(item),
            }));
            rows.push(Row {
                label: "Copies",
                value: body.copies().to_string(),
            });
            rows.push(Row {
                label: "Each",
                value: copy_text(body),
            });
            Some(Description {
                name: design
                    .pair_member_name(member)
                    .unwrap_or_else(|| body.name.clone()),
                kind: "Joint member",
                rows,
            })
        }
        Item::Hardware(id) => {
            let hardware = design.hardware_piece(id)?;
            let kind = match hardware.base {
                HardwareBase::Slide => "Slide",
                HardwareBase::Hinge => "Hinge",
                HardwareBase::Pull => "Pull",
                HardwareBase::Support => "Support",
                HardwareBase::Fitting => "Fitting",
            };
            let piece = design.part(hardware.piece).map_or("?", |p| p.name.as_str());
            let members = hardware.members.iter().map(|member| Row {
                label: "Member",
                value: member_text(design, hardware, member),
            });
            let mechanism = hardware.own_mechanism.as_ref().map(|mechanism| Row {
                label: "Mechanism",
                value: mechanism_text(mechanism),
            });
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
                ]
                .into_iter()
                .chain(members)
                .chain(mechanism)
                .chain([Row {
                    label: "Assembly",
                    value: assembly_path(design, hardware.assembly),
                }])
                .collect(),
            })
        }
    }
}

/// What a joint's member is in words: "dowel_pin, beech", "wood_screw,
/// steel".
pub fn item_text(item: &MemberItem) -> String {
    let made_of = item
        .wood
        .map_or_else(|| tr("steel"), |species| species.title().to_lowercase());
    format!("{}, {made_of}", item.kind)
}

/// One copy of a joint's member in words, from its shapes: "cylinder 8 mm
/// across, 30 mm long".
pub fn copy_text(body: &Body) -> String {
    let shapes = body.copy_shapes().next().unwrap_or(body.shapes());
    match shapes {
        [
            MemberShape::Cylinder {
                diameter,
                start,
                end,
                ..
            },
        ] => fill_in(
            &tr("cylinder {diameter} mm across, {length} mm long"),
            &[
                ("diameter", &millimetres(*diameter)),
                (
                    "length",
                    &millimetres(Length::from_nm(end.nm().saturating_sub(start.nm()))),
                ),
            ],
        ),
        [
            MemberShape::Slanted {
                diameter,
                length,
                angle,
                ..
            },
        ] => fill_in(
            &tr("cylinder {diameter} mm across, {length} mm long, slanted {angle}"),
            &[
                ("diameter", &millimetres(*diameter)),
                ("length", &millimetres(*length)),
                ("angle", &angle.to_string()),
            ],
        ),
        [MemberShape::Bar { thickness, .. }] => fill_in(
            &tr("bar {thickness} mm thick"),
            &[("thickness", &millimetres(*thickness))],
        ),
        [MemberShape::Prism { .. }] => tr("outline"),
        [MemberShape::Box(region)] => {
            let mut sizes = Axis::ALL.map(|axis| region.size(axis));
            sizes.sort_unstable_by(|a, b| b.cmp(a));
            let [length, width, thickness] = sizes;
            fill_in(
                &tr("box {size}"),
                &[("size", &size_text(length, width, thickness))],
            )
        }
        [] | [_, _, ..] => fill_in(
            &ntr("{count} shape", "{count} shapes", shapes.len()),
            &[("count", &shapes.len())],
        ),
    }
}

/// The scene item a problem's item is.
pub fn contact_item(item: ContactItem) -> Item {
    match item {
        ContactItem::Part(id) => Item::Part(id),
        ContactItem::Hardware(id) => Item::Hardware(id),
        ContactItem::Member(member) => Item::Member(member),
    }
}

/// How each node is drawn outside focus mode, when a joint's member is
/// among `shown` (what is selected, or what a problem is about): the parts
/// of its pair see-through, so the pins inside them show; everything else
/// as usual. `None` when no member is shown.
pub fn see_into(design: &SolvedDesign, graph: &SceneGraph, shown: &[Item]) -> Option<Vec<Look>> {
    let parts: Vec<stave_model::solved::PartId> = shown
        .iter()
        .filter_map(|item| match item {
            Item::Member(member) => design.joint(member.joint)?.pair(member.pair),
            Item::Part(_) | Item::Hardware(_) => None,
        })
        .flat_map(|pair| pair.roles.iter().map(|role| role.part))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let xray: Vec<NodeId> = graph
        .node_ids()
        .filter(|id| {
            graph.node(*id).is_some_and(|node| match node.item {
                Item::Part(part) => parts.contains(&part),
                Item::Hardware(_) | Item::Member(_) => false,
            })
        })
        .collect();
    Some(graph.looks(&[], &xray, false))
}

/// `true` when `inner` lies inside `outer`.
fn within(inner: &WorldBox, outer: &WorldBox) -> bool {
    Axis::ALL.into_iter().all(|axis| {
        outer.min().get(axis) <= inner.min().get(axis)
            && inner.max().get(axis) <= outer.max().get(axis)
    })
}

/// One member of a piece of hardware in words: its name, its shape, and
/// what holds it: "fixed leaf, box 40 × 12 × 1.5 mm, housed in back".
fn member_text(design: &SolvedDesign, hardware: &Hardware, member: &Body) -> String {
    let bounds = member.bounds();
    // The part that carries it, and whether it sits inside that part's
    // wood (a cup in its bore, a leaf in its mortise).
    let carrier = match member.carrier {
        BodyCarrier::Part(part) => design.part(part),
        BodyCarrier::Region(_) | BodyCarrier::Link(_) => None,
    };
    let sunk = carrier.is_some_and(|part| within(&bounds, &part.cut_box()));
    let shape = match member.shapes() {
        [
            MemberShape::Cylinder {
                diameter,
                start,
                end,
                ..
            },
        ] => format!(
            "cylinder {} mm across, {} mm {}",
            millimetres(*diameter),
            millimetres(Length::from_nm(end.nm().saturating_sub(start.nm()))),
            if sunk { "deep" } else { "long" }
        ),
        [
            MemberShape::Slanted {
                diameter,
                length,
                angle,
                ..
            },
        ] => format!(
            "cylinder {} mm across, {} mm {}, slanted {angle}",
            millimetres(*diameter),
            millimetres(*length),
            if sunk { "deep" } else { "long" }
        ),
        [MemberShape::Bar { thickness, .. }] => {
            format!("bar {} mm thick", millimetres(*thickness))
        }
        [MemberShape::Prism { .. }] => "outline".to_owned(),
        [MemberShape::Box(_)] | [] => {
            let mut sizes = Axis::ALL.map(|axis| bounds.size(axis));
            sizes.sort_unstable_by(|a, b| b.cmp(a));
            let [length, width, thickness] = sizes;
            format!("box {}", size_text(length, width, thickness))
        }
        [_, _, ..] => {
            let mut sizes = Axis::ALL.map(|axis| bounds.size(axis));
            sizes.sort_unstable_by(|a, b| b.cmp(a));
            let [length, width, thickness] = sizes;
            format!(
                "{} shapes within {}",
                member.shapes().len(),
                size_text(length, width, thickness)
            )
        }
    };
    let held = match member.carrier {
        BodyCarrier::Part(part) if part == hardware.piece => String::new(),
        BodyCarrier::Part(part) => {
            let name = design.part(part).map_or("?", |p| p.name.as_str());
            if sunk {
                format!(", housed in {name}")
            } else {
                format!(", fixed to {name}")
            }
        }
        BodyCarrier::Region(region) => {
            let name = design
                .regions()
                .get(region.index())
                .map_or("?", |r| r.name.as_str());
            format!(", fixed in {name}")
        }
        BodyCarrier::Link(_) => ", moved by the mechanism".to_owned(),
    };
    format!("{}, {shape}{held}", member.name.replace('_', " "))
}

/// A piece of hardware's own mechanism in words: what it is made of and
/// how far it moves: "4 pivots; turns up to 110deg".
fn mechanism_text(mechanism: &Mechanism) -> String {
    let count = |test: fn(&MechanismEntry) -> bool, one: &str, many: &str| {
        let found = mechanism
            .entries()
            .iter()
            .filter(|entry| test(entry))
            .count();
        match found {
            0 => None,
            1 => Some(format!("1 {one}")),
            found => Some(format!("{found} {many}")),
        }
    };
    let parts: Vec<String> = [
        count(
            |e| matches!(e, MechanismEntry::Pivot { .. }),
            "pivot",
            "pivots",
        ),
        count(
            |e| matches!(e, MechanismEntry::Slot { .. }),
            "slot",
            "slots",
        ),
        count(
            |e| matches!(e, MechanismEntry::Rail { .. }),
            "rail",
            "rails",
        ),
        count(
            |e| matches!(e, MechanismEntry::Couple { .. }),
            "couple",
            "couples",
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    let reach = match mechanism.drive().extent() {
        DriveValue::Turn(angle) => format!("turns up to {angle}"),
        DriveValue::Travel(length) => format!("travels up to {} mm", millimetres(length)),
    };
    format!("{}; {reach}", parts.join(", "))
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
                contact
                    .items
                    .iter()
                    .any(|item| contact_item(*item) == node.item)
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

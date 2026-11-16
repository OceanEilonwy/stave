//! What a viewer shows, worked out without any UI: the scene graph of a
//! design, a description of the selected part or hardware, and the
//! project's problems as plain rows.

use std::path::Path;

use stave_lang::units::Length;
use stave_lang::{Diagnostic, Severity, SourceMap};
use stave_model::solved::{Envelope, HardwareBase, HardwareData, Part, SolvedDesign};
pub use stave_scene::{BuildOptions, Extent, Item, SceneGraph};

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
        PartOrigin::Made { .. } => "Made part",
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
                .chain([Row {
                    label: "Assembly",
                    value: assembly_path(design, hardware.assembly),
                }])
                .collect(),
            })
        }
    }
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
    }
}

/// The project's diagnostics as rows, in their sorted order. Files inside
/// `folder` are named relative to it.
pub fn problems(opened: &Opened, folder: &Path) -> Vec<Problem> {
    opened
        .diagnostics
        .iter()
        .map(|diagnostic| problem(&opened.sources, folder, diagnostic))
        .collect()
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

//! The text and the 3D view share one selection (FilesMode.dc.html): the
//! cursor on a part, an assembly or a piece of hardware selects it in the
//! 3D view ([`selection_for`]), and selecting in the 3D view or the parts
//! tree moves the editor to the line that places it ([`placed_at`]),
//! which the view's chip names ([`chip`]: "Linked to the text · door_l,
//! line 9").

use stave_ide::{Analysis, SymbolId, SymbolKind};
use stave_model::solved::{AssemblyId, SolvedDesign};

use crate::check::Checked;
use crate::session::{Selection, SourceLocation};

/// `true` if `name` is `base` or an element of it: `foot` matches
/// `foot[front_left]`.
fn names(name: &str, base: &str) -> bool {
    name.strip_prefix(base)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('['))
}

/// The assemblies around a selection, innermost first.
fn assemblies_around(design: &SolvedDesign, selection: Option<&Selection>) -> Vec<AssemblyId> {
    let mut assembly = match selection {
        Some(Selection::Part(id)) => design.part(*id).and_then(|p| p.assembly),
        Some(Selection::Hardware(id)) => design.hardware.get(id.index()).and_then(|h| h.assembly),
        Some(Selection::Assembly(id)) => Some(*id),
        Some(Selection::Joint { .. } | Selection::Location(_) | Selection::Problem(_)) | None => {
            None
        }
    };
    let mut out = Vec::new();
    while let Some(id) = assembly {
        if out.contains(&id) || out.len() > design.assemblies.len() {
            break;
        }
        out.push(id);
        assembly = design.assemblies.get(id.index()).and_then(|a| a.parent);
    }
    out
}

/// What the 3D view selects for the symbol under the editor's cursor: the
/// part, assembly or piece of hardware it declares or names. A label
/// (`foot[front_left]`) selects that element; an item of a component
/// selects it in the assembly around `current` when that assembly is an
/// instance of the component, otherwise in the first instance.
pub fn selection_for(
    design: &SolvedDesign,
    analysis: &Analysis,
    symbol: SymbolId,
    current: Option<&Selection>,
) -> Option<Selection> {
    let mut found = analysis.symbol(symbol)?;
    let mut label = None;
    if found.kind == SymbolKind::Label {
        label = Some(found.name.as_str());
        found = analysis.symbol(found.container?)?;
    }
    if !found.kind.is_shown_in_3d() {
        return None;
    }
    let component = found
        .container
        .and_then(|c| analysis.symbol(c))
        .filter(|c| c.kind == SymbolKind::Component);
    let prefixes: Vec<String> = match component {
        None if analysis.is_in_project(found.id) => vec![String::new()],
        None => return None,
        Some(component) => {
            let of_component = |id: &AssemblyId| {
                design.assemblies.get(id.index()).is_some_and(|a| {
                    a.component.rsplit('.').next() == Some(component.name.as_str())
                })
            };
            let mut order: Vec<AssemblyId> = assemblies_around(design, current)
                .into_iter()
                .filter(of_component)
                .collect();
            let mut rest: Vec<&stave_model::solved::Assembly> = design
                .assemblies
                .iter()
                .filter(|a| of_component(&a.id) && !order.contains(&a.id))
                .collect();
            // Instances before mirror images.
            rest.sort_by_key(|a| a.mirror_of.is_some());
            order.extend(rest.into_iter().map(|a| a.id));
            order
                .into_iter()
                .filter_map(|id| design.assemblies.get(id.index()))
                .map(|a| format!("{}.", a.name))
                .collect()
        }
    };
    for prefix in prefixes {
        let base = format!("{prefix}{}", found.name);
        let wanted = |name: &str| match label {
            Some(label) => name == format!("{base}[{label}]"),
            None => names(name, &base),
        };
        let selection = match found.kind {
            SymbolKind::Part => design
                .parts
                .iter()
                .find(|p| wanted(&p.name))
                .map(|p| Selection::Part(p.id)),
            SymbolKind::Hardware => design
                .hardware
                .iter()
                .find(|h| wanted(&h.name))
                .map(|h| Selection::Hardware(h.id)),
            SymbolKind::Assembly => design
                .assemblies
                .iter()
                .find(|a| wanted(&a.name))
                .map(|a| Selection::Assembly(a.id)),
            _ => None,
        };
        if selection.is_some() {
            return selection;
        }
    }
    None
}

/// The line in the project's own files that places a selection: a part's
/// or a piece of hardware's declaration, or, for one inside a component
/// from a library, the declaration of its assembly (the inspector's
/// "Defined in the text").
pub fn placed_at(checked: &Checked, selection: &Selection) -> Option<SourceLocation> {
    match selection {
        Selection::Location(location) => Some(location.clone()),
        Selection::Part(_)
        | Selection::Hardware(_)
        | Selection::Assembly(_)
        | Selection::Joint { .. } => crate::inspect::inspect(checked, selection)?
            .source
            .map(|snippet| snippet.location),
        Selection::Problem(_) => None,
    }
}

/// The chip on the 3D view in the Files view: "Linked to the text ·
/// door_l, line 9".
pub fn chip(
    design: &SolvedDesign,
    selection: &Selection,
    location: &SourceLocation,
) -> Option<String> {
    let name = match selection {
        Selection::Part(id) => design.part(*id).map(|p| p.name.as_str()),
        Selection::Hardware(id) => design.hardware.get(id.index()).map(|h| h.name.as_str()),
        Selection::Assembly(id) => design.assemblies.get(id.index()).map(|a| a.name.as_str()),
        Selection::Joint { joint, .. } => design.joints.get(joint.index()).map(|j| j.name.as_str()),
        Selection::Location(_) | Selection::Problem(_) => None,
    }?;
    Some(format!(
        "Linked to the text · {name}, line {}",
        location.line
    ))
}

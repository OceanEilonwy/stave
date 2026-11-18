//! Hold this (`docs/editing.md`, "Hold this"): a distance the user
//! measured between two things, pinned with a minimum or a maximum, as the
//! patch that adds a `require`.
//!
//! ```text
//! require slat_to_apron_front_min : between(slat[0], apron_front) >= 12mm "room for a finger"
//! ```
//!
//! The two things are parts, regions or faces of either. The distance is
//! `between(a, b)`, or, when the two things are opposite faces of one part
//! or region, its own size along that axis: `inside.deep` for a region,
//! `extent(base, up)` for a part. The rule is named from the two things
//! measured plus `_min` or `_max` ([`crate::naming::held_rule_name`]), and
//! the reason the user types is its message. Like any edit the patch is
//! tried before it is written, so a rule the design already breaks is
//! refused by the check, and once written the rule refuses any later edit
//! that would break it.

use stave_lang::ast::{DesignItem, Ident, ItemKind};
use stave_lang::placement::Face;
use stave_lang::units::Length;
use stave_model::defs::{AssemblyKind, DefId};

use crate::candidates::EditScope;
use crate::candidates::print::quoted;
use crate::naming::{Bound, NamingError, TakenNames, held_rule_name};
use crate::repeat::{GeneratedPatch, PatchBuildError};

/// One of the two things a distance is measured between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Measured {
    /// A part or a region by its path: `slat[0]`, `drawer[top].front`,
    /// `door_bay[left]`.
    Item(String),
    /// A face of a part or a region: `apron_front.back`.
    Face {
        /// The part or region, by its path.
        item: String,
        /// The face.
        face: Face,
    },
}

impl Measured {
    /// The thing as written in an expression: `slat[0]`, `apron_front.back`.
    pub fn path(&self) -> String {
        match self {
            Measured::Item(item) => item.clone(),
            Measured::Face { item, face } => format!("{item}.{}", face.word()),
        }
    }

    fn item(&self) -> &str {
        match self {
            Measured::Item(item) | Measured::Face { item, .. } => item,
        }
    }
}

/// What the user pinned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldRequest {
    /// The first thing measured.
    pub first: Measured,
    /// The second.
    pub second: Measured,
    /// A minimum or a maximum.
    pub bound: Bound,
    /// The value pinned: the distance as it is now, or what the user typed.
    pub value: Length,
    /// Why, as the user typed it: the rule's message.
    pub reason: String,
}

/// The patch for a held rule, with the name and condition it writes.
#[derive(Debug, Clone)]
pub struct HeldPatch {
    /// The rule's name: `slat_to_apron_front_min`.
    pub name: Ident,
    /// Its condition as written: `between(slat[0], apron_front) >= 12mm`.
    pub condition: String,
    /// The patch adding it.
    pub patch: GeneratedPatch,
}

/// Why a held rule can't be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HoldError {
    /// The reason is empty; it is the message the rule shows when broken.
    #[error("say why this must hold: it is the message shown when a change breaks it")]
    NoReason,
    /// Both things are the same.
    #[error("'{0}' was picked twice; a distance needs two different things")]
    SameThing(String),
    /// A thing's first name is not an item of the project.
    #[error("there is no item named '{0}' in this project")]
    UnknownItem(String),
    /// A generated name couldn't be made.
    #[error(transparent)]
    Naming(#[from] NamingError),
    /// The written patch didn't parse: a bug.
    #[error(transparent)]
    Build(#[from] PatchBuildError),
}

/// The patch that adds the held rule (see the module's notes). It is added
/// without `in`, so it goes to the file of the first item it names.
///
/// # Errors
///
/// An empty reason, the same thing twice, a thing that is not in the
/// project, or a name that can't be made; see [`HoldError`].
pub fn hold_patch(scope: &EditScope<'_>, request: &HoldRequest) -> Result<HeldPatch, HoldError> {
    let reason = request.reason.trim();
    if reason.is_empty() {
        return Err(HoldError::NoReason);
    }
    let (first, second) = (request.first.path(), request.second.path());
    if first == second {
        return Err(HoldError::SameThing(first));
    }
    for thing in [&request.first, &request.second] {
        let root = root_of(thing.item());
        if !TakenNames::of_project(scope.model()).contains(root) {
            return Err(HoldError::UnknownItem(root.to_owned()));
        }
    }
    let measure = match (&request.first, &request.second) {
        (Measured::Face { item: a, face: fa }, Measured::Face { item: b, face: fb })
            if a == b && fa.opposite() == *fb =>
        {
            let axis = fa.axis().word();
            if is_region(scope, a) {
                format!("{a}.{axis}")
            } else {
                format!("extent({a}, {axis})")
            }
        }
        _ => format!("between({first}, {second})"),
    };
    let condition = format!("{measure} {} {}", request.bound.operator(), request.value);
    let taken = TakenNames::of_project(scope.model());
    let name = held_rule_name(&first, &second, request.bound, &taken)?;
    let ops = vec![format!(
        "add require {} : {condition} {}",
        name.as_str(),
        quoted(reason)
    )];
    let summary = format!(
        "Hold {measure} {} {}",
        request.bound.operator(),
        request.value
    );
    let patch = GeneratedPatch::new(&summary, &ops)?;
    Ok(HeldPatch {
        name,
        condition,
        patch,
    })
}

/// The first name of a path: `drawer` in `drawer[top].front`.
fn root_of(path: &str) -> &str {
    path.split(['[', '.']).next().unwrap_or(path).trim()
}

/// `true` when a path names a region: a region of the project (or an
/// element of one), or a region declared in the component an assembly of
/// the project is an instance of (`divider.below`).
fn is_region(scope: &EditScope<'_>, path: &str) -> bool {
    let root = root_of(path);
    let rest = path.get(root.len()..).unwrap_or("");
    // Past the root and any index, what member is named.
    let after_index = match rest.strip_prefix('[') {
        Some(inner) => inner.split_once(']').map_or("", |(_, after)| after),
        None => rest,
    };
    let member = after_index.strip_prefix('.');
    let in_files = scope.files().iter().find_map(|file| {
        file.parse
            .ast
            .as_ref()?
            .items
            .iter()
            .find_map(|item| (item.kind.name()?.as_str() == root).then_some(&item.kind))
    });
    match (in_files, member) {
        (Some(ItemKind::Design(DesignItem::Region(_))), None) => true,
        (Some(ItemKind::Design(DesignItem::Assembly(_))), Some(member))
            if !member.contains(['.', '[']) =>
        {
            component_region(scope, root, member)
        }
        _ => false,
    }
}

/// `true` when the component the project's assembly `assembly` is an
/// instance of declares a region named `member`.
fn component_region(scope: &EditScope<'_>, assembly: &str, member: &str) -> bool {
    let model = scope.model();
    let defs = &model.defs;
    let Some(module) = defs.modules.get(model.root.index()) else {
        return false;
    };
    module.items.iter().any(|item| {
        let DefId::Assembly(id) = item else {
            return false;
        };
        let Some(def) = defs.assembly(*id) else {
            return false;
        };
        let AssemblyKind::Instance { component, .. } = &def.kind else {
            return false;
        };
        def.decl.name == assembly
            && defs.component(*component).is_some_and(|component| {
                component.items.iter().any(|inner| {
                    matches!(inner, DefId::Region(_))
                        && defs.decl(*inner).is_some_and(|decl| decl.name == member)
                })
            })
    })
}

#[cfg(test)]
mod tests {
    use stave_lang::placement::Face;
    use stave_lang::units::Length;

    use super::{HoldError, HoldRequest, Measured, hold_patch};
    use crate::candidates::EditScope;
    use crate::naming::Bound;
    use crate::repeat::GeneratedPatch;
    use crate::repeat::test_support::{
        ASSEMBLIES, assemblies, clean_base, commit, fits, small_project, try_on,
    };

    fn face(item: &str, face: Face) -> Measured {
        Measured::Face {
            item: item.to_owned(),
            face,
        }
    }

    #[test]
    fn a_held_depth_refuses_a_later_patch_that_breaks_it() {
        let (folder, mut files) = small_project(&[]);
        let shallower =
            GeneratedPatch::new("Make it shallower", &["set depth = 250mm".to_owned()]).unwrap();
        // Before the hold, a shallower case fits.
        fits(&folder, &files, &shallower);

        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let held = hold_patch(
            &scope,
            &HoldRequest {
                first: face("inside", Face::Front),
                second: face("inside", Face::Back),
                bound: Bound::Min,
                value: Length::from_mm(280),
                reason: "room for files".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(held.name.as_str(), "inside_front_to_inside_back_min");
        assert_eq!(held.condition, "inside.deep >= 280mm");
        assert!(held.patch.text.contains(
            "add require inside_front_to_inside_back_min : inside.deep >= 280mm \"room for files\""
        ));
        let tried = fits(&folder, &files, &held.patch);
        drop(scope);
        drop(base);
        commit(&mut files, &tried);

        // Now the same change is refused, by the rule.
        let tried = try_on(&folder, &files, &shallower);
        assert!(!tried.fits());
        let shown = tried.shown();
        assert!(
            shown.iter().any(|d| d.message.contains("room for files")),
            "{shown:?}"
        );
    }

    #[test]
    fn a_rule_the_design_already_breaks_is_refused() {
        let (folder, files) = small_project(&[]);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let held = hold_patch(
            &scope,
            &HoldRequest {
                first: face("inside", Face::Front),
                second: face("inside", Face::Back),
                bound: Bound::Min,
                value: Length::from_mm(400),
                reason: "room for files".to_owned(),
            },
        )
        .unwrap();
        assert!(!try_on(&folder, &files, &held.patch).fits());
    }

    #[test]
    fn distances_between_parts_and_a_part_size() {
        let (folder, files) = small_project(&[]);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let between = hold_patch(
            &scope,
            &HoldRequest {
                first: Measured::Item("side_l".to_owned()),
                second: Measured::Item("side_r".to_owned()),
                bound: Bound::Max,
                value: Length::from_mm(370),
                reason: "fits the alcove".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(between.name.as_str(), "side_l_to_side_r_max");
        assert_eq!(between.condition, "between(side_l, side_r) <= 370mm");
        fits(&folder, &files, &between.patch);

        let size = hold_patch(
            &scope,
            &HoldRequest {
                first: face("base", Face::Top),
                second: face("base", Face::Bottom),
                bound: Bound::Min,
                value: Length::from_mm(19),
                reason: "strong enough".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(size.condition, "extent(base, up) >= 19mm");
        fits(&folder, &files, &size.patch);

        assert!(matches!(
            hold_patch(
                &scope,
                &HoldRequest {
                    first: face("base", Face::Top),
                    second: face("base", Face::Top),
                    bound: Bound::Min,
                    value: Length::from_mm(1),
                    reason: "x".to_owned(),
                }
            ),
            Err(HoldError::SameThing(_))
        ));
        assert!(matches!(
            hold_patch(
                &scope,
                &HoldRequest {
                    first: face("base", Face::Top),
                    second: face("base", Face::Bottom),
                    bound: Bound::Min,
                    value: Length::from_mm(1),
                    reason: "  ".to_owned(),
                }
            ),
            Err(HoldError::NoReason)
        ));
        assert!(matches!(
            hold_patch(
                &scope,
                &HoldRequest {
                    first: Measured::Item("nothing".to_owned()),
                    second: Measured::Item("base".to_owned()),
                    bound: Bound::Min,
                    value: Length::from_mm(1),
                    reason: "x".to_owned(),
                }
            ),
            Err(HoldError::UnknownItem(_))
        ));
    }

    #[test]
    fn a_held_drawer_space_in_the_cabinet() {
        let files = assemblies();
        let base = clean_base(ASSEMBLIES, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        // The space under the divider is a region of the divider's
        // component: its own height.
        let held = hold_patch(
            &scope,
            &HoldRequest {
                first: face("divider.below", Face::Bottom),
                second: face("divider.below", Face::Top),
                bound: Bound::Min,
                value: Length::from_mm(300),
                reason: "two drawers fit".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(held.condition, "divider.below.up >= 300mm");
        assert_eq!(
            held.name.as_str(),
            "divider_below_bottom_to_divider_below_top_min"
        );
        let tried = fits(ASSEMBLIES, &files, &held.patch);
        assert!(
            tried
                .changed()
                .iter()
                .any(|file| file.path.ends_with("carcass.stave")),
            "goes to the file of the divider"
        );
    }
}

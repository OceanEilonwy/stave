//! Remove (`docs/editing.md`, "Remove"): what depends on an item, and the
//! patch that removes it, alone or with everything that depends on it.
//!
//! What depends on an item is every item of the project whose text names
//! it (one that places against it, joins into it, fills a region it
//! bounds, mirrors it, poses it in a scene, checks it in a `require`), and
//! what depends on those, as the navigation layer reads the names
//! ([`stave_ide::Analysis::dependants`]). Removing an item that something
//! still names would leave an error, which strict commit never writes, so
//! [`removal_patch`] refuses to remove an item alone while anything depends
//! on it; with [`Dependants::Included`] it removes the item and its
//! dependants, those furthest from it first, so every operation applies to
//! text that still holds what it removes.

use crate::i18n::{fill_in, ntr, tr};
use std::path::Path;

use stave_ide::{Analysis, Document, Symbol, SymbolId};
use stave_lang::ast::ItemTag;

use crate::repeat::{GeneratedPatch, PatchBuildError};

/// An item of the project, as a removal lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedItem {
    /// Its name.
    pub name: String,
    /// Its keyword, which the patch's `remove` writes.
    pub tag: ItemTag,
    /// The file of the project's folder it is in.
    pub file: String,
}

/// An item and what depends on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// The item.
    pub item: RemovedItem,
    /// What depends on it, nearest first: the items that name it, then the
    /// items that name those.
    pub dependants: Vec<RemovedItem>,
}

/// Whether a removal takes the item's dependants with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dependants {
    /// Remove the item alone; refused while anything depends on it.
    Refuse,
    /// Remove the item and everything that depends on it.
    Included,
}

/// Why an item can't be removed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoveError {
    /// The project declares no item by that name.
    #[error("{}", self.text())]
    UnknownItem(String),
    /// The item is of a sort a patch can't remove.
    #[error("{}", self.text())]
    NotRemovable(String),
    /// Something still names the item, and the removal was asked for
    /// alone.
    #[error("{}", self.text())]
    StillUsed {
        /// The item.
        name: String,
        /// What names it, nearest first.
        dependants: Vec<String>,
    },
    /// The patch text didn't parse: a bug.
    #[error(transparent)]
    Build(#[from] PatchBuildError),
}

impl RemoveError {
    /// The error in the window's words (translated).
    fn text(&self) -> String {
        match self {
            RemoveError::UnknownItem(name) => fill_in(
                &tr("there is no item named '{name}' in this project"),
                &[("name", name)],
            ),
            RemoveError::NotRemovable(name) => fill_in(
                &tr("'{name}' can't be removed by a patch"),
                &[("name", name)],
            ),
            RemoveError::StillUsed { name, dependants } => fill_in(
                &tr("'{name}' is still used by {users}; remove those too, or change them first"),
                &[("name", name), ("users", &dependants.join(", "))],
            ),
            RemoveError::Build(error) => error.to_string(),
        }
    }
}

/// The item named `name` and everything that depends on it.
///
/// # Errors
///
/// [`RemoveError::UnknownItem`] when the project has no top-level item by
/// that name; [`RemoveError::NotRemovable`] for an item a patch can't
/// remove.
pub fn removal(analysis: &Analysis, name: &str) -> Result<Removal, RemoveError> {
    let symbol = analysis
        .project_symbol(name)
        .ok_or_else(|| RemoveError::UnknownItem(name.to_owned()))?;
    let item =
        removed_item(analysis, symbol).ok_or_else(|| RemoveError::NotRemovable(name.to_owned()))?;
    let dependants = analysis
        .dependants(symbol.id)
        .into_iter()
        .map(|id| dependant(analysis, id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Removal { item, dependants })
}

fn dependant(analysis: &Analysis, id: SymbolId) -> Result<RemovedItem, RemoveError> {
    let symbol = analysis
        .symbol(id)
        .ok_or_else(|| RemoveError::UnknownItem(String::new()))?;
    removed_item(analysis, symbol).ok_or_else(|| RemoveError::NotRemovable(symbol.name.clone()))
}

fn removed_item(analysis: &Analysis, symbol: &Symbol) -> Option<RemovedItem> {
    let tag = ItemTag::from_keyword(symbol.kind.word())?;
    let file = match analysis.document(symbol.file)? {
        Document::File(path) => Path::new(&path).file_name()?.to_string_lossy().into_owned(),
        Document::Std { .. } => return None,
    };
    Some(RemovedItem {
        name: symbol.name.clone(),
        tag,
        file,
    })
}

/// The patch that removes the item named `name`: alone, or with its
/// dependants.
///
/// # Errors
///
/// An unknown or unremovable item, or [`RemoveError::StillUsed`] for an
/// item something depends on when `dependants` is
/// [`Dependants::Refuse`].
pub fn removal_patch(
    analysis: &Analysis,
    name: &str,
    dependants: Dependants,
) -> Result<GeneratedPatch, RemoveError> {
    let removal = removal(analysis, name)?;
    match dependants {
        Dependants::Refuse if !removal.dependants.is_empty() => {
            return Err(RemoveError::StillUsed {
                name: removal.item.name.clone(),
                dependants: removal
                    .dependants
                    .iter()
                    .map(|item| item.name.clone())
                    .collect(),
            });
        }
        Dependants::Refuse | Dependants::Included => {}
    }
    Ok(patch_for(&removal)?)
}

/// The patch for a removal: its dependants furthest first, then the item.
///
/// # Errors
///
/// [`PatchBuildError`] when the written text doesn't parse (a bug).
pub fn patch_for(removal: &Removal) -> Result<GeneratedPatch, PatchBuildError> {
    let ops: Vec<String> = removal
        .dependants
        .iter()
        .rev()
        .chain(std::iter::once(&removal.item))
        .map(|item| format!("remove {} {}", item.tag.keyword(), item.name))
        .collect();
    let count = removal.dependants.len();
    let summary = if count == 0 {
        fill_in(&tr("Remove {item}"), &[("item", &removal.item.name)])
    } else {
        fill_in(
            &ntr(
                "Remove {item} and the item that uses it",
                "Remove {item} and the {count} items that use it",
                count,
            ),
            &[("item", &removal.item.name), ("count", &count)],
        )
    };
    GeneratedPatch::new(&summary, &ops)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use stave_ide::Analysis;
    use stave_lang::ast::ItemTag;

    use super::{Dependants, RemoveError, removal, removal_patch};
    use crate::repeat::test_support::{ASSEMBLIES, assemblies, fits, small_project};

    #[test]
    fn removing_the_divider_lists_what_stands_on_it() {
        let files = assemblies();
        let analysis = Analysis::load(Path::new(ASSEMBLIES), &files).unwrap();
        let found = removal(&analysis, "divider").unwrap();
        assert_eq!(found.item.tag, ItemTag::Assembly);
        assert_eq!(found.item.file, "carcass.stave");
        let names: Vec<&str> = found.dependants.iter().map(|d| d.name.as_str()).collect();
        for expected in ["shelf", "drawer_bay", "drawer", "door_bay", "hanging_rail"] {
            assert!(names.contains(&expected), "{expected} in {names:?}");
        }
        // The shelf stands on the divider directly; the drawers through
        // the bays the divider bounds.
        let at = |name| names.iter().position(|n| *n == name).unwrap();
        assert!(at("shelf") < at("hanging_rail"));
        assert!(at("drawer_bay") < at("drawer"));
        assert!(!names.contains(&"divider"));
        assert!(!names.contains(&"base"));

        // Alone, it is refused, naming what uses it.
        let refused = removal_patch(&analysis, "divider", Dependants::Refuse);
        assert!(
            matches!(&refused, Err(RemoveError::StillUsed { dependants, .. }) if dependants.contains(&"shelf".to_owned())),
            "{refused:?}"
        );
        // With its dependants, the patch applies and the design checks.
        let patch = removal_patch(&analysis, "divider", Dependants::Included).unwrap();
        assert!(
            patch.text.ends_with("  remove assembly divider\n}\n"),
            "{}",
            patch.text
        );
        let tried = fits(ASSEMBLIES, &files, &patch);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        assert!(design.part_named("base").is_some());
        assert!(design.part_named("shelf.board").is_none());
        assert!(!design.parts.iter().any(|p| p.name.starts_with("drawer")));
    }

    #[test]
    fn an_item_nothing_uses_is_removed_alone() {
        let shelf = r#"part shelf : oak_19 {
  length: width - 50mm
  width:  depth - 20mm
  place {
    against(base.top, gap: 250mm)
    center(base, across)
    center(base, deep)
  }
}"#;
        let (folder, files) = small_project(&[shelf]);
        let analysis = Analysis::load(Path::new(&folder), &files).unwrap();
        let found = removal(&analysis, "shelf").unwrap();
        assert!(found.dependants.is_empty());
        let patch = removal_patch(&analysis, "shelf", Dependants::Refuse).unwrap();
        assert!(patch.text.contains("  remove part shelf\n"));
        let tried = fits(&folder, &files, &patch);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        assert!(design.part_named("shelf").is_none());
        assert!(matches!(
            removal(&analysis, "nothing"),
            Err(RemoveError::UnknownItem(_))
        ));
    }
}

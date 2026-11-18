//! Signature matching (`docs/editing.md`, "Signature matching"): what can
//! go where the user points, worked out from the declarations in scope and
//! never from a list in the app.
//!
//! ```text
//! Opened ──EditScope::new──▶ EditScope ──candidates(Picked)──▶ [Candidate]
//!                                     └─write_patch(Candidate, Repeat?)──▶ WrittenPatch
//! ```
//!
//! [`EditScope::candidates`] lists every `component` and `kind` in scope
//! whose place params the selection can fill, each with its menu entry, its
//! place params bound or still to point at, and its setting params with
//! where each value comes from: the param's default, a copy of what the
//! project's most recent assembly of that component wrote, or nothing yet.
//! Whether a candidate applies (building it in a scratch copy and checking
//! it) is not worked out here: every candidate comes back
//! [`Applicability::NotChecked`].
//!
//! [`EditScope::write_patch`] writes the patch for a chosen candidate: an
//! ordinary patch of the `.stave` grammar that adds the assembly (or the
//! joint or piece of hardware), with the region and array of a repeat and
//! any import the file it goes to still needs ([`write`]).
//! [`new_project::new_project_text`] writes a new project started from a
//! box.

pub mod new_project;
pub mod print;
#[cfg(test)]
mod project_tests;
pub mod write;

use std::cmp::Ordering;

use stave_lang::ast::{self, Ident, Name, Span, TypeName};
use stave_lang::{FileId, Parse, SourceMap};
use stave_model::Model;
use stave_model::defs::{AssemblyKind, DefId, Defs, ParamDef, Scope};
use stave_model::ids::{AssemblyId, ComponentId, KindId};
use stave_model::nonempty::NonEmpty;
use stave_model::types::Ty;
use stave_modules::ModuleId;
use stave_schema::kinds::RoleType;

use crate::Opened;
use crate::edit::{
    Applicability, Candidate, Declaration, MenuEntry, MenuGroup, Picked, PlaceParam, PlaceSlot,
    SettingParam, SettingValue,
};

pub use write::{Repeat, WriteError, WrittenPatch};

/// Why an opened project can't be edited by pointing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScopeError {
    /// Names and types didn't resolve far enough to have a model.
    #[error("the project has no resolved model; fix its name and type errors first")]
    NoModel,
}

/// One file of the project's own module, parsed again for the editor: the
/// text a patch is applied to and copied values are read from.
#[derive(Debug, Clone)]
pub struct ProjectFile {
    /// The file in the opened project's [`SourceMap`].
    pub id: FileId,
    /// Its name within the project's folder: `carcass.stave`.
    pub name: String,
    /// The file parsed.
    pub parse: Parse<ast::File>,
}

/// An opened project seen from the editor: its model, and its own files
/// parsed, in the order the folder lists them.
#[derive(Debug)]
pub struct EditScope<'a> {
    model: &'a Model,
    files: Vec<ProjectFile>,
}

impl<'a> EditScope<'a> {
    /// The editor's view of an opened project.
    ///
    /// # Errors
    ///
    /// [`ScopeError::NoModel`] when the project has no resolved model.
    pub fn new(opened: &'a Opened) -> Result<Self, ScopeError> {
        let model = opened.model.as_ref().ok_or(ScopeError::NoModel)?;
        Ok(Self::from_parts(model, &opened.sources))
    }

    /// The editor's view of a model and the sources it was read from.
    pub fn from_parts(model: &'a Model, sources: &SourceMap) -> Self {
        let files = model
            .defs
            .files
            .iter()
            .filter(|(_, scope)| scope.module == Some(model.root))
            .filter_map(|(id, _)| {
                let source = sources.get(*id)?;
                let name = std::path::Path::new(source.path())
                    .file_name()?
                    .to_string_lossy()
                    .into_owned();
                Some(ProjectFile {
                    id: *id,
                    name,
                    parse: stave_lang::parse_file(*id, source.text()),
                })
            })
            .collect();
        Self { model, files }
    }

    /// The resolved model.
    pub fn model(&self) -> &'a Model {
        self.model
    }

    /// The project's own files, in the order the folder lists them.
    pub fn files(&self) -> &[ProjectFile] {
        &self.files
    }

    /// What a patch is applied to: the project's own files.
    pub fn patch_target(&self) -> stave_lang::PatchTarget<'_> {
        stave_lang::PatchTarget::new(
            self.files
                .iter()
                .map(|file| stave_lang::ModuleFile {
                    name: &file.name,
                    parse: &file.parse,
                })
                .collect(),
        )
    }

    fn file(&self, id: FileId) -> Option<&ProjectFile> {
        self.files.iter().find(|file| file.id == id)
    }

    /// The menu for a selection: every component and kind in scope whose
    /// place params the selection can fill, in menu order (see
    /// [`sort_menu`]). Each is [`Applicability::NotChecked`].
    pub fn candidates(&self, picked: &Picked) -> Vec<Candidate> {
        let defs = &self.model.defs;
        let mut out = Vec::new();
        for (module, group) in self.modules_in_scope() {
            let own = module == self.model.root;
            let Some(module_defs) = defs.modules.get(module.index()) else {
                continue;
            };
            for item in &module_defs.items {
                let Some(decl) = defs.decl(*item) else {
                    continue;
                };
                if decl.scope != Scope::Module(module)
                    || (decl.private && !own)
                    || defs.is_poisoned(*item)
                {
                    continue;
                }
                let candidate = match *item {
                    DefId::Component(id) => self.component_candidate(id, picked, &group),
                    DefId::Kind(id) => self.kind_candidate(id, picked, &group),
                    _ => None,
                };
                out.extend(candidate);
            }
        }
        sort_menu(&mut out);
        out
    }

    /// The modules whose declarations are in scope, each once with its
    /// menu group: the project's own ([`MenuGroup::Project`]), then every
    /// module a file of the project imports, a standard one such as
    /// `std:casework` included ([`MenuGroup::Module`], by the name it is
    /// imported under, the first file's name when several import it), then
    /// the prelude's standard modules ([`MenuGroup::Standard`]), which are
    /// in scope without an import. This is the grouping
    /// [`crate::edit::clothing_cabinet_fixture`] shows.
    fn modules_in_scope(&self) -> Vec<(ModuleId, MenuGroup)> {
        let defs = &self.model.defs;
        let mut out = vec![(self.model.root, MenuGroup::Project)];
        let seen =
            |out: &[(ModuleId, MenuGroup)], module: ModuleId| out.iter().any(|(m, _)| *m == module);
        for file in &self.files {
            let Some(scope) = defs.files.get(&file.id) else {
                continue;
            };
            for (alias, module) in &scope.imports {
                // A prelude module is in scope everywhere: it is listed
                // under std below, even when a file also imports it.
                if seen(&out, *module) || defs.prelude.contains(module) {
                    continue;
                }
                let Ok(alias) = Ident::new(alias.as_str(), Span::default()) else {
                    continue;
                };
                out.push((*module, MenuGroup::Module(alias)));
            }
        }
        for module in &defs.prelude {
            if seen(&out, *module) {
                continue;
            }
            let Some(name) = defs
                .modules
                .get(module.index())
                .and_then(|m| Name::new(m.name.as_str(), Span::default()).ok())
            else {
                continue;
            };
            out.push((*module, MenuGroup::Standard(name)));
        }
        out
    }

    fn component_candidate(
        &self,
        id: ComponentId,
        picked: &Picked,
        group: &MenuGroup,
    ) -> Option<Candidate> {
        let defs = &self.model.defs;
        let component = defs.component(id)?;
        let mut place = Vec::new();
        let mut settings = Vec::new();
        for param_id in &component.params {
            let param = defs.param(*param_id)?;
            match place_slot(&param.ty) {
                Some(slot) => place.push(PlaceParam {
                    name: Name::new(param.decl.name.as_str(), Span::default()).ok()?,
                    slot,
                }),
                None => {
                    let name = Ident::new(param.decl.name.as_str(), Span::default()).ok()?;
                    let Some(ty) = type_name(&param.ty) else {
                        // A param whose type no type name says (a list
                        // default) can't be shown in the card; with a
                        // default it needs nothing, so it is left out.
                        continue;
                    };
                    let value = if param.default.is_some() {
                        SettingValue::Default
                    } else {
                        self.copied(id, *param_id, &param.decl.name)
                            .unwrap_or(SettingValue::Unset)
                    };
                    settings.push(SettingParam { name, ty, value });
                }
            }
        }
        if !bind_component(&mut place, picked) {
            return None;
        }
        Some(Candidate {
            declaration: Declaration::Component(id),
            entry: menu_entry(&component.decl.name, &component.decl.docs, group)?,
            place,
            settings,
            applicability: Applicability::NotChecked,
        })
    }

    fn kind_candidate(&self, id: KindId, picked: &Picked, group: &MenuGroup) -> Option<Candidate> {
        let kind = self.model.defs.kind(id)?;
        let mut place = Vec::new();
        for role in &kind.roles {
            let name = Name::new(role.name.as_str(), Span::default()).ok()?;
            let slot = match role.ty {
                RoleType::Part => PlaceSlot::Part(None),
                RoleType::Parts => PlaceSlot::Parts(None),
                RoleType::Region => PlaceSlot::Region(None),
            };
            place.push(PlaceParam { name, slot });
        }
        if !bind_kind(&mut place, picked) {
            return None;
        }
        let settings = kind.params.iter().filter_map(kind_setting).collect();
        Some(Candidate {
            declaration: Declaration::Kind(id),
            entry: menu_entry(&kind.decl.name, &kind.decl.docs, group)?,
            place,
            settings,
            applicability: Applicability::NotChecked,
        })
    }

    /// The value the project's most recent assembly of `component` gives
    /// `param`, read from the text: the last such assembly in file order of
    /// the project's files.
    fn copied(
        &self,
        component: ComponentId,
        param: stave_model::ids::ParamDefId,
        name: &str,
    ) -> Option<SettingValue> {
        let defs = &self.model.defs;
        let module = defs.modules.get(self.model.root.index())?;
        module.items.iter().rev().find_map(|item| {
            let DefId::Assembly(assembly_def) = *item else {
                return None;
            };
            let def = defs.assembly(assembly_def)?;
            let AssemblyKind::Instance {
                component: found,
                args,
            } = &def.kind
            else {
                return None;
            };
            if *found != component || !args.iter().any(|(p, _)| *p == param) {
                return None;
            }
            let value = self.written_argument(def.decl.file, &def.decl.name, name)?;
            let from = self.last_instance(assembly_def)?;
            Some(SettingValue::Copied { from, value })
        })
    }

    /// The expression an assembly's body gives a param, as written.
    fn written_argument(&self, file: FileId, assembly: &str, param: &str) -> Option<ast::Expr> {
        let parsed = self.file(file)?.parse.ast.as_ref()?;
        parsed.items.iter().find_map(|item| {
            let ast::ItemKind::Design(ast::DesignItem::Assembly(found)) = &item.kind else {
                return None;
            };
            if found.name.as_str() != assembly {
                return None;
            }
            let ast::AssemblyDef::Instance { body, .. } = &found.def else {
                return None;
            };
            body.field(param).map(|field| field.value.clone())
        })
    }

    /// The last concrete assembly of a declaration (an array's last
    /// element).
    fn last_instance(&self, def: stave_model::ids::AssemblyDefId) -> Option<AssemblyId> {
        self.model
            .assemblies
            .iter()
            .rev()
            .find(|assembly| assembly.def == def)
            .map(|assembly| assembly.id)
    }
}

/// The slot of a place param's type, or `None` for a setting.
fn place_slot(ty: &Ty) -> Option<PlaceSlot> {
    match ty {
        Ty::Region => Some(PlaceSlot::Region(None)),
        Ty::Anchor => Some(PlaceSlot::Anchor(None)),
        Ty::Part => Some(PlaceSlot::Part(None)),
        Ty::Parts => Some(PlaceSlot::Parts(None)),
        _ => None,
    }
}

/// The type name that declares a setting of this type, if one does.
fn type_name(ty: &Ty) -> Option<TypeName> {
    let wanted = match ty {
        Ty::Kind(kind) => Ty::Kind(stave_model::types::KindTy {
            base: kind.base,
            kind: None,
        }),
        other => other.clone(),
    };
    TypeName::ALL
        .into_iter()
        .find(|name| Ty::from_type_name(*name) == wanted)
}

/// A kind's param as a setting: its default, or nothing yet. Settings are
/// copied only from assemblies, so a kind's are never copied.
fn kind_setting(param: &ParamDef) -> Option<SettingParam> {
    let ty = type_name(&param.ty)?;
    Some(SettingParam {
        name: Ident::new(param.decl.name.as_str(), Span::default()).ok()?,
        ty,
        value: if param.default.is_some() {
            SettingValue::Default
        } else {
            SettingValue::Unset
        },
    })
}

fn menu_entry(name: &str, docs: &[String], group: &MenuGroup) -> Option<MenuEntry> {
    Some(MenuEntry {
        label: Ident::new(name, Span::default()).ok()?,
        hint: first_sentence(docs),
        group: group.clone(),
    })
}

/// The first sentence of a doc comment: up to and including the first `.`,
/// `!` or `?` followed by a space or the end, its lines joined by spaces.
/// `None` when there is no doc comment.
pub fn first_sentence(docs: &[String]) -> Option<String> {
    let joined = docs
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        return None;
    }
    let mut chars = joined.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let ends = matches!(c, '.' | '!' | '?')
            && chars.peek().is_none_or(|(_, next)| next.is_whitespace());
        if ends {
            return joined.get(..index + c.len_utf8()).map(str::to_owned);
        }
    }
    Some(joined)
}

/// Binds the selection to a component's place params (`docs/editing.md`,
/// "Signature matching", step 2): every typed value of the selection fills
/// the first unbound place param of its own type, and no value may be left
/// over. A `Region` fills a `Region` param; a face gives two values, its
/// `Anchor` and its `Part`, and both must find a param; `Parts` (two facing
/// parts, or a group) fills a `Parts` param. `false` when the selection
/// doesn't fit.
fn bind_component(place: &mut [PlaceParam], picked: &Picked) -> bool {
    match picked {
        Picked::Region(region) => fill_first(place, |slot| match slot {
            PlaceSlot::Region(slot @ None) => {
                *slot = Some(*region);
                true
            }
            _ => false,
        }),
        Picked::Face(face) => {
            let anchor = fill_first(place, |slot| match slot {
                PlaceSlot::Anchor(slot @ None) => {
                    *slot = Some(*face);
                    true
                }
                _ => false,
            });
            let part = fill_first(place, |slot| match slot {
                PlaceSlot::Part(slot @ None) => {
                    *slot = Some(face.part);
                    true
                }
                _ => false,
            });
            anchor && part
        }
        Picked::Between(pair) => NonEmpty::new(vec![pair.first(), pair.second()])
            .is_some_and(|parts| fill_parts(place, parts)),
        Picked::Group(parts) => fill_parts(place, parts.clone()),
    }
}

/// Fills the first place param `fill` takes. `true` when one took it.
fn fill_first(place: &mut [PlaceParam], mut fill: impl FnMut(&mut PlaceSlot) -> bool) -> bool {
    place.iter_mut().any(|param| fill(&mut param.slot))
}

fn fill_parts(place: &mut [PlaceParam], parts: NonEmpty<stave_model::ids::PartId>) -> bool {
    fill_first(place, |slot| match slot {
        PlaceSlot::Parts(slot @ None) => {
            *slot = Some(parts.clone());
            true
        }
        _ => false,
    })
}

/// Binds the selection to a kind's roles. A `Region` fills the first
/// `Region` role. The parts of the selection (a face's part, the two facing
/// parts, the group) go one to each `Part` or `Parts` role in order, the
/// part pointed at first to the first role; parts left over join the last
/// `Parts` role that took one. A face's `Anchor` is a typed value of the
/// selection too, and a kind has no `Anchor` role to take it, so a kind is
/// never a candidate on a face (step 2: every typed value must fill a place
/// param). `false` when a value of the selection is left over.
fn bind_kind(place: &mut [PlaceParam], picked: &Picked) -> bool {
    let parts = match picked {
        Picked::Region(region) => {
            return place.iter_mut().any(|param| match &mut param.slot {
                PlaceSlot::Region(slot @ None) => {
                    *slot = Some(*region);
                    true
                }
                _ => false,
            });
        }
        Picked::Face(_) => return false,
        Picked::Between(pair) => vec![pair.first(), pair.second()],
        Picked::Group(parts) => parts.iter().copied().collect(),
    };
    let mut parts = parts.into_iter();
    let mut last_parts_role = None;
    for (index, param) in place.iter_mut().enumerate() {
        match &mut param.slot {
            PlaceSlot::Part(slot @ None) => match parts.next() {
                Some(part) => *slot = Some(part),
                None => break,
            },
            PlaceSlot::Parts(slot @ None) => match parts.next() {
                Some(part) => {
                    *slot = NonEmpty::new(vec![part]);
                    last_parts_role = Some(index);
                }
                None => break,
            },
            PlaceSlot::Part(Some(_))
            | PlaceSlot::Parts(Some(_))
            | PlaceSlot::Region(_)
            | PlaceSlot::Anchor(_) => {}
        }
    }
    let rest: Vec<_> = parts.collect();
    if rest.is_empty() {
        return true;
    }
    let Some(PlaceParam {
        slot: PlaceSlot::Parts(Some(taken)),
        ..
    }) = last_parts_role.and_then(|index| place.get_mut(index))
    else {
        return false;
    };
    let mut all: Vec<_> = taken.iter().copied().collect();
    all.extend(rest);
    match NonEmpty::new(all) {
        Some(all) => {
            *taken = all;
            true
        }
        None => false,
    }
}

/// Sorts a menu: by group (the project's own first, then imported modules
/// by name, then the standard modules by name), within a group the entries
/// that apply (or aren't checked yet) before the greyed ones, each in name
/// order.
pub fn sort_menu(candidates: &mut [Candidate]) {
    candidates.sort_by(|a, b| {
        group_key(&a.entry.group)
            .cmp(&group_key(&b.entry.group))
            .then_with(|| greyed(a).cmp(&greyed(b)))
            .then_with(|| a.entry.label.as_str().cmp(b.entry.label.as_str()))
            .then_with(|| declaration_key(a).cmp(&declaration_key(b)))
    });
}

fn group_key(group: &MenuGroup) -> (u8, &str) {
    match group {
        MenuGroup::Project => (0, ""),
        MenuGroup::Module(name) => (1, name.as_str()),
        MenuGroup::Standard(name) => (2, name.as_str()),
    }
}

fn greyed(candidate: &Candidate) -> bool {
    match candidate.applicability {
        Applicability::Greyed(_) => true,
        Applicability::NotChecked | Applicability::Applies => false,
    }
}

fn declaration_key(candidate: &Candidate) -> (u8, usize) {
    match candidate.declaration {
        Declaration::Component(id) => (0, id.index()),
        Declaration::Kind(id) => (1, id.index()),
    }
}

/// Compares two menu groups in menu order.
pub fn compare_groups(a: &MenuGroup, b: &MenuGroup) -> Ordering {
    group_key(a).cmp(&group_key(b))
}

/// The name a declaration is written by in a file of the project, given
/// the module it is in: bare for the project's own module and the prelude,
/// `alias.Name` for an imported module the file imports. `None` when the
/// file doesn't import the module.
pub(crate) fn imported_alias(defs: &Defs, file: FileId, module: ModuleId) -> Option<&str> {
    defs.files.get(&file).and_then(|scope| {
        scope
            .imports
            .iter()
            .find(|(_, m)| **m == module)
            .map(|(alias, _)| alias.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hint_is_the_first_sentence() {
        let docs = |lines: &[&str]| lines.iter().map(|l| (*l).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            first_sentence(&docs(&[
                " A drawer on side-mount slides: an applied front on a box with a grooved-in bottom.",
                " The box follows the opening.",
            ])),
            Some(
                "A drawer on side-mount slides: an applied front on a box with a grooved-in bottom."
                    .to_owned()
            )
        );
        assert_eq!(
            first_sentence(&docs(&[" A shelf that sits", " on pins. More."])),
            Some("A shelf that sits on pins.".to_owned())
        );
        assert_eq!(
            first_sentence(&docs(&[" Holds 1.5mm of play"])),
            Some("Holds 1.5mm of play".to_owned())
        );
        assert_eq!(first_sentence(&[]), None);
    }

    #[test]
    fn menu_groups_are_ordered_project_modules_standard() {
        let name = |n: &str| Ident::new(n, Span::default()).unwrap();
        let word = |n: &str| Name::new(n, Span::default()).unwrap();
        let mut groups = [
            MenuGroup::Standard(word("joinery")),
            MenuGroup::Module(name("hw")),
            MenuGroup::Project,
            MenuGroup::Standard(word("hardware")),
            MenuGroup::Module(name("cat")),
        ];
        groups.sort_by(compare_groups);
        assert_eq!(
            groups,
            [
                MenuGroup::Project,
                MenuGroup::Module(name("cat")),
                MenuGroup::Module(name("hw")),
                MenuGroup::Standard(word("hardware")),
                MenuGroup::Standard(word("joinery")),
            ]
        );
    }
}

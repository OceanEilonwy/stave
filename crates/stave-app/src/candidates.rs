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
//! Whether a candidate applies is not worked out by the listing: every
//! candidate comes back [`Applicability::NotChecked`], and
//! [`applicability`] builds each in a scratch copy and checks it.
//!
//! [`EditScope::write_patch`] writes the patch for a chosen candidate: an
//! ordinary patch of the `.stave` grammar that adds the assembly (or the
//! joint or piece of hardware), with the region and array of a repeat and
//! any import the file it goes to still needs ([`write`]).
//! [`new_project::new_project_text`] writes a new project started from a
//! box.

pub mod applicability;
pub mod describe;
pub mod new_project;
pub mod print;
#[cfg(test)]
mod project_tests;
pub mod write;

use crate::i18n::tr;
use std::cmp::Ordering;

use stave_lang::ast::{self, Ident, Name, Span, TypeName};
use stave_lang::{FileId, Parse, SourceMap};
use stave_model::Model;
use stave_model::defs::{AssemblyKind, DefId, Defs, ParamDef, Scope};
use stave_model::ids::{AssemblyId, ComponentId, KindId};
use stave_model::nonempty::NonEmpty;
use stave_model::solved::SolvedDesign;
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
    #[error("{}", self.text())]
    NoModel,
}

impl ScopeError {
    /// The error in the window's words (translated).
    fn text(&self) -> String {
        match self {
            ScopeError::NoModel => {
                tr("the project has no resolved model; fix its name and type errors first")
            }
        }
    }
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
    /// The solved design, when the scope was made from an opened project
    /// that solved: where the parts are, for the rules that need it.
    design: Option<&'a SolvedDesign>,
}

impl<'a> EditScope<'a> {
    /// The editor's view of an opened project.
    ///
    /// # Errors
    ///
    /// [`ScopeError::NoModel`] when the project has no resolved model.
    pub fn new(opened: &'a Opened) -> Result<Self, ScopeError> {
        let model = opened.model.as_ref().ok_or(ScopeError::NoModel)?;
        let mut scope = Self::from_parts(model, &opened.sources);
        scope.design = opened.design.as_ref();
        Ok(scope)
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
        Self {
            model,
            files,
            design: None,
        }
    }

    /// The solved design, when the scope was made from a project that
    /// solved.
    pub fn design(&self) -> Option<&'a SolvedDesign> {
        self.design
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
    pub(crate) fn modules_in_scope(&self) -> Vec<(ModuleId, MenuGroup)> {
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
                    prompt: String::new(),
                    text: None,
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
                    settings.push(SettingParam::new(name, ty, value));
                }
            }
        }
        if !bind_component(&mut place, picked) {
            return None;
        }
        if self.would_nest_in_itself(id, picked) && !may_nest(&component.decl.docs) {
            return None;
        }
        Some(Candidate {
            declaration: Declaration::Component(id),
            entry: menu_entry(&component.decl.name, &component.decl.docs, group)?,
            place,
            settings,
            applicability: Applicability::NotChecked,
            generated_name: None,
            mounted_on: None,
        })
    }

    fn kind_candidate(&self, id: KindId, picked: &Picked, group: &MenuGroup) -> Option<Candidate> {
        let kind = self.model.defs.kind(id)?;
        // A fastener or a tool is used by a joint or a tool list, never
        // placed by itself, so it is not a menu entry.
        match kind.base {
            ast::BaseKind::Joint
            | ast::BaseKind::Slide
            | ast::BaseKind::Hinge
            | ast::BaseKind::Pull => {}
            ast::BaseKind::Fastener | ast::BaseKind::Tool => return None,
        }
        let mut place = Vec::new();
        for role in &kind.roles {
            let name = Name::new(role.name.as_str(), Span::default()).ok()?;
            let slot = match role.ty {
                RoleType::Part => PlaceSlot::Part(None),
                RoleType::Parts => PlaceSlot::Parts(None),
                RoleType::Region => PlaceSlot::Region(None),
            };
            place.push(PlaceParam {
                name,
                slot,
                prompt: String::new(),
                text: None,
            });
        }
        if !bind_kind(&mut place, picked) {
            return None;
        }
        // A joint joins parts that meet: offered on parts that don't, it
        // could never apply, so it is not listed.
        if kind.base == ast::BaseKind::Joint && !self.joined_parts_meet(&place) {
            return None;
        }
        let settings = kind.params.iter().filter_map(kind_setting).collect();
        // Hardware offered on a face is mounted on that face.
        let mounted_on = match (picked, kind.base) {
            (
                Picked::Face(face),
                ast::BaseKind::Slide | ast::BaseKind::Hinge | ast::BaseKind::Pull,
            ) => Some(*face),
            _ => None,
        };
        Some(Candidate {
            declaration: Declaration::Kind(id),
            entry: menu_entry(&kind.decl.name, &kind.decl.docs, group)?,
            place,
            settings,
            applicability: Applicability::NotChecked,
            generated_name: None,
            mounted_on,
        })
    }

    /// Whether filling the picked space with `component` would put it
    /// straight inside an instance of itself: the space is a region one
    /// of its own assemblies declares (a case's `inside`), with nothing
    /// between.
    fn would_nest_in_itself(&self, component: ComponentId, picked: &Picked) -> bool {
        let Picked::Region(region) = picked else {
            return false;
        };
        let model = self.model;
        model
            .regions
            .get(region.index())
            .and_then(|region| model.instance(region.ctx.instance))
            .is_some_and(|instance| {
                matches!(
                    instance.kind,
                    stave_model::instance::InstanceKind::Component { component: owner, .. }
                        if owner == component
                )
            })
    }

    /// Whether the parts a joint's roles were bound to can meet: the part
    /// of its first role touches or overlaps at least one part of the
    /// others. `true` while a role is still to point at, or without a
    /// solved design to tell.
    fn joined_parts_meet(&self, place: &[PlaceParam]) -> bool {
        let Some(design) = self.design else {
            return true;
        };
        let mut roles = place.iter().map(|param| match &param.slot {
            PlaceSlot::Part(Some(part)) => Some(vec![*part]),
            PlaceSlot::Parts(Some(parts)) => Some(parts.iter().copied().collect()),
            PlaceSlot::Part(None)
            | PlaceSlot::Parts(None)
            | PlaceSlot::Region(_)
            | PlaceSlot::Anchor(_) => None,
        });
        let Some(Some(first)) = roles.next() else {
            return true;
        };
        let mut others = Vec::new();
        for role in roles {
            match role {
                Some(parts) => others.extend(parts),
                None => return true,
            }
        }
        let bounds = |part: stave_model::ids::PartId| design.part(part).map(|p| p.drawn_box());
        first.iter().any(|a| {
            others.iter().any(|b| match (bounds(*a), bounds(*b)) {
                (Some(a), Some(b)) => stave_lang::placement::Axis::ALL.into_iter().all(|axis| {
                    a.min().get(axis) <= b.max().get(axis) && b.min().get(axis) <= a.max().get(axis)
                }),
                _ => true,
            })
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

    /// The values the project's own assemblies already give settings like
    /// `setting`, as written, the most recent first (the last assembly in
    /// file order first, as [`EditScope::copied`] reads), each once, at
    /// most [`crate::choices::RECENT_LIMIT`]. "Like" is the same type for a
    /// type with a list of options (a stock, a mount, a kind, a `Bool`);
    /// for a quantity (a length, an angle, a count), whose type every
    /// size shares, it is also the same param name.
    pub(crate) fn recent_values(&self, setting: &SettingParam) -> Vec<String> {
        let defs = &self.model.defs;
        let Some(module) = defs.modules.get(self.model.root.index()) else {
            return Vec::new();
        };
        let by_name = crate::choices::matched_by_name(setting.ty);
        let mut out: Vec<String> = Vec::new();
        for item in module.items.iter().rev() {
            let DefId::Assembly(assembly_def) = *item else {
                continue;
            };
            let Some(def) = defs.assembly(assembly_def) else {
                continue;
            };
            let AssemblyKind::Instance { args, .. } = &def.kind else {
                continue;
            };
            for (param, _) in args {
                let Some(param_def) = defs.param(*param) else {
                    continue;
                };
                if type_name(&param_def.ty) != Some(setting.ty)
                    || (by_name && param_def.decl.name != setting.name.as_str())
                {
                    continue;
                }
                let Some(value) =
                    self.written_argument(def.decl.file, &def.decl.name, &param_def.decl.name)
                else {
                    continue;
                };
                let text = print::expr_text(&value);
                if !out.contains(&text) {
                    out.push(text);
                    if out.len() >= crate::choices::RECENT_LIMIT {
                        return out;
                    }
                }
            }
        }
        out
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
    Some(SettingParam::new(
        Ident::new(param.decl.name.as_str(), Span::default()).ok()?,
        ty,
        if param.default.is_some() {
            SettingValue::Default
        } else {
            SettingValue::Unset
        },
    ))
}

/// Whether a component's `///` doc comment says it may be put inside a
/// space one of its own assemblies exposes: a sentence that says it "may
/// nest inside itself" (in any case).
pub fn may_nest(docs: &[String]) -> bool {
    docs.iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
        .contains("may nest inside itself")
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
/// the first unbound place param of its own type. A `Region` fills a
/// `Region` param; `Parts` (two facing parts, or a group) fills a `Parts`
/// param, and must find one. A face gives two values, its `Anchor` and its
/// `Part`, and the component is a candidate when either finds a param
/// (the face rule): a component that sits on a face without naming its
/// part, or hangs from a part without naming a face, is offered on a
/// face too. `false` when the selection doesn't fit.
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
            anchor || part
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
/// `Parts` role that took one. A kind has no `Anchor` role, so of a face's
/// two typed values only its `Part` is used (the face rule): a kind whose
/// first part role is a `Part` (a knob's `piece`) is offered on a face.
/// `false` when a value of the selection is left over, or nothing was
/// bound.
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
        Picked::Face(face) => vec![face.part],
        Picked::Between(pair) => vec![pair.first(), pair.second()],
        Picked::Group(parts) => parts.iter().copied().collect(),
    };
    if !place
        .iter()
        .any(|param| matches!(param.slot, PlaceSlot::Part(None) | PlaceSlot::Parts(None)))
    {
        return false;
    }
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

/// Sorts a menu: the entries that apply, need a choice or aren't checked
/// yet first, then the refused (greyed) ones, so the refused ones can be
/// folded away together ([`refused_from`]); each part by group (the
/// project's own first, then imported modules by name, then the standard
/// modules by name), within a group in name order.
pub fn sort_menu(candidates: &mut [Candidate]) {
    candidates.sort_by(|a, b| {
        greyed(a)
            .cmp(&greyed(b))
            .then_with(|| group_key(&a.entry.group).cmp(&group_key(&b.entry.group)))
            .then_with(|| a.entry.label.as_str().cmp(b.entry.label.as_str()))
            .then_with(|| declaration_key(a).cmp(&declaration_key(b)))
    });
}

/// Where the refused entries of a sorted menu begin: the index of the
/// first greyed entry, or the menu's length when none is greyed. The
/// window folds `menu[refused_from(menu)..]` under one heading.
pub fn refused_from(candidates: &[Candidate]) -> usize {
    candidates
        .iter()
        .position(greyed)
        .unwrap_or(candidates.len())
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
        Applicability::NotChecked | Applicability::Applies | Applicability::NeedsChoice(_) => false,
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

//! The patch a menu pick writes (`docs/editing.md`, "Generated text").
//!
//! A component writes `assembly <name> : <Component> { ... }` with its
//! place params bound to what was pointed at and every setting that isn't
//! left to its default; a kind writes `joint <name> : <kind>(...)` or
//! `hardware <name> : <kind>(...)`. A repeat adds the region the pieces
//! fill (`region drawer_bay[bottom, top] = split(bay, up)`) and makes the
//! assembly an array bound with `[same]`.
//!
//! The item goes to the project's only file, or, once the project is split
//! into several, to the file of the first item of the project it refers
//! to (as a patch's `add` chooses), or else to the file holding the
//! `project` item. The patch names that file with `in "..."`, and adds the
//! imports that file still needs for the names the item uses through a
//! module (`casework.Drawer`, `hw.bar_pull_160`).

use std::collections::BTreeMap;

use stave_lang::ast::{self, Expr, ExprKind, Ident, Patch, PatchFile, StringPart};
use stave_lang::placement::Axis;
use stave_lang::{Diagnostic, FileId, Parse, SourceMap};
use stave_model::defs::{DefId, Scope};
use stave_modules::ModuleId;

use super::print::{expr_text, quoted};
use super::{EditScope, imported_alias};
use crate::edit::{Candidate, Declaration, PlaceSlot, SettingValue};
use crate::naming::{
    NamingError, RepeatCount, TakenNames, assembly_name, bay_name, kind_item_name, repeat_array,
    snake_case,
};

/// A repeat of the picked component: so many pieces, filling the picked
/// region split along an axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Repeat {
    /// How many pieces.
    pub count: RepeatCount,
    /// The axis the region is split along.
    pub axis: Axis,
}

/// The patch for a chosen candidate, as text and parsed.
#[derive(Debug, Clone)]
pub struct WrittenPatch {
    /// The patch file's text: `stave 1` and one patch.
    pub text: String,
    /// The text parsed; it has no errors.
    pub parse: Parse<PatchFile>,
    /// The patch, as the parse holds it; its spans point into
    /// [`WrittenPatch::text`].
    pub patch: Patch,
    /// The file of the project's folder the item goes to.
    pub file: String,
    /// The name the new assembly, joint or piece of hardware takes.
    pub name: Ident,
    /// For a repeat, the name of the region its pieces fill.
    pub bay: Option<Ident>,
}

/// Where a piece of hardware sits, as its patch writes it.
struct HardwareMount {
    /// The call's `face:`, when the hardware goes on another face than
    /// the kind's own.
    face_argument: Option<stave_lang::placement::Face>,
    /// The `place { }` terms for the axes the kind leaves open.
    place: Option<Vec<String>>,
}

/// Why a patch can't be written for a candidate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// A place param still to point at or a setting still to choose.
    #[error("still to fill in: {}", .0.join(", "))]
    Incomplete(Vec<String>),
    /// The candidate names a declaration the model doesn't have.
    #[error("the candidate's declaration is not in this project")]
    UnknownDeclaration,
    /// A place param is bound to an item the model doesn't have.
    #[error("'{0}' is bound to something that is not in this design")]
    UnknownItem(String),
    /// A kind that is neither a joint nor mounted hardware (a fastener or
    /// a tool) is not placed by itself.
    #[error("{0} is a {1} kind, which is used by a joint or a tool list, not placed by itself")]
    NotPlaceable(String, &'static str),
    /// A repeat needs a component with a `Region` place param bound.
    #[error("only a component that fills a space can be repeated")]
    NotRepeatable,
    /// The file the item goes to imports a module the item needs under
    /// another name, or uses the name for another module.
    #[error(
        "'{file}' already uses the name '{alias}' differently; import the module there by hand first"
    )]
    ImportClash {
        /// The file.
        file: String,
        /// The module name.
        alias: String,
    },
    /// The name typed for the new item is already used in the project, or
    /// is a word of the language.
    #[error(
        "'{0}' is already used in this project or is a word of the language; choose another name"
    )]
    NameTaken(String),
    /// The project has no file to add to.
    #[error("the project has no file to add to")]
    NoFile,
    /// A generated name couldn't be made.
    #[error(transparent)]
    Naming(#[from] NamingError),
    /// The written text didn't parse: a bug in the writer.
    #[error("the written patch doesn't parse; please report this as a bug")]
    Unparsable(Vec<Diagnostic>),
}

/// One field or argument the item writes.
struct Written {
    /// The param's or role's name.
    key: String,
    /// The value as text.
    text: String,
    /// The names it refers to, in the order written: the first segment of
    /// every name.
    roots: Vec<String>,
    /// Where names used through a module are looked up: the file a copied
    /// value was read from; `None` for a value of the editor's own.
    names_from: Option<FileId>,
}

/// An import the target file needs.
struct NeededImport {
    alias: String,
    /// The import's source, as written in the file it was found in.
    source: String,
    /// `true` when that file wrote `as alias`.
    explicit: bool,
}

impl EditScope<'_> {
    /// Writes the patch for a chosen candidate, with `repeat` for an array
    /// of pieces filling the candidate's region.
    ///
    /// # Errors
    ///
    /// A candidate that isn't complete, or that names something the model
    /// doesn't have, or a repeat of something that fills no region, or an
    /// import the target file can't take; see [`WriteError`].
    pub fn write_patch(
        &self,
        candidate: &Candidate,
        repeat: Option<Repeat>,
    ) -> Result<WrittenPatch, WriteError> {
        self.write_patch_named(candidate, repeat, None)
    }

    /// [`EditScope::write_patch`], with the new item named `name` (a name
    /// the user typed on the card) instead of the generated one. `None`
    /// generates it.
    ///
    /// # Errors
    ///
    /// As for [`EditScope::write_patch`], and [`WriteError::NameTaken`]
    /// when `name` is already used in the project or is a word of the
    /// language.
    pub fn write_patch_named(
        &self,
        candidate: &Candidate,
        repeat: Option<Repeat>,
        name: Option<&Ident>,
    ) -> Result<WrittenPatch, WriteError> {
        let missing: Vec<String> = candidate
            .unbound_place()
            .map(|param| param.name.as_str().to_owned())
            .chain(
                candidate
                    .unset_settings()
                    .map(|param| param.name.as_str().to_owned()),
            )
            .collect();
        if !missing.is_empty() {
            return Err(WriteError::Incomplete(missing));
        }
        let defs = &self.model.defs;
        let (def, keyword) = match candidate.declaration {
            Declaration::Component(id) => (DefId::Component(id), "assembly"),
            Declaration::Kind(id) => {
                let kind = defs.kind(id).ok_or(WriteError::UnknownDeclaration)?;
                let keyword = match kind.base {
                    ast::BaseKind::Joint => "joint",
                    ast::BaseKind::Slide | ast::BaseKind::Hinge | ast::BaseKind::Pull => "hardware",
                    base @ (ast::BaseKind::Fastener | ast::BaseKind::Tool) => {
                        return Err(WriteError::NotPlaceable(
                            kind.decl.name.clone(),
                            base.word(),
                        ));
                    }
                };
                if repeat.is_some() {
                    return Err(WriteError::NotRepeatable);
                }
                (DefId::Kind(id), keyword)
            }
        };
        let decl = defs.decl(def).ok_or(WriteError::UnknownDeclaration)?;
        let Scope::Module(module) = decl.scope else {
            return Err(WriteError::UnknownDeclaration);
        };

        let mut taken = TakenNames::of_project(self.model);
        let name = match (name, candidate.declaration) {
            (Some(name), _) if taken.is_unavailable(name.as_str()) => {
                return Err(WriteError::NameTaken(name.as_str().to_owned()));
            }
            (Some(name), _) => name.clone(),
            (None, Declaration::Component(_)) => assembly_name(&decl.name, &taken)?,
            (None, Declaration::Kind(_)) => kind_item_name(&decl.name, &taken)?,
        };
        taken.insert(name.as_str());
        let bay = repeat.map(|_| bay_name(&decl.name, &taken)).transpose()?;

        let (mut fields, repeated) = self.fields(candidate, bay.as_ref())?;
        let region = match (repeat, repeated, &bay) {
            (Some(repeat), Some(region), Some(bay)) => Some((repeat, region, bay)),
            (None, _, _) => None,
            (Some(_), _, _) => return Err(WriteError::NotRepeatable),
        };

        // The file: the first project item referred to, in the order the
        // patch writes them.
        let own_component = (module == self.model.root).then_some(decl.file);
        let roots = region
            .iter()
            .flat_map(|(_, written, _)| written.roots.iter())
            .chain(fields.iter().flat_map(|field| field.roots.iter()));
        let file_id = own_component
            .or_else(|| {
                roots
                    .into_iter()
                    .find_map(|root| self.project_item_file(root))
            })
            .or_else(|| {
                defs.modules
                    .get(self.model.root.index())
                    .and_then(|m| m.project.as_ref())
                    .map(|project| project.file)
            })
            .or_else(|| self.files.first().map(|file| file.id))
            .ok_or(WriteError::NoFile)?;
        let file = self
            .file(file_id)
            .map(|file| file.name.clone())
            .ok_or(WriteError::NoFile)?;

        // The imports it needs, and the name the declaration is written by.
        let mut imports: BTreeMap<String, NeededImport> = BTreeMap::new();
        let qualifier = self.qualifier(module, file_id, &file, &mut imports)?;
        for field in &fields {
            self.field_imports(field, file_id, &file, &mut imports)?;
        }
        if let Some((_, written, _)) = &region {
            self.field_imports(written, file_id, &file, &mut imports)?;
        }

        let array = repeat.map(|repeat| repeat_array(repeat.axis, repeat.count));
        let array_text = array
            .as_ref()
            .map(|array| array.declaration())
            .unwrap_or_default();
        let item = match candidate.declaration {
            Declaration::Component(_) => {
                let mut text = format!(
                    "assembly {}{array_text} : {qualifier}{} {{\n",
                    name.as_str(),
                    decl.name
                );
                let width = fields.iter().map(|f| f.key.len()).max().unwrap_or(0);
                for field in &fields {
                    let pad = " ".repeat(width - field.key.len());
                    text.push_str(&format!("  {}: {pad}{}\n", field.key, field.text));
                }
                text.push('}');
                text
            }
            Declaration::Kind(_) => {
                let mut args = Vec::new();
                for (index, field) in fields.iter_mut().enumerate() {
                    let text = std::mem::take(&mut field.text);
                    args.push(if index == 0 {
                        text
                    } else {
                        format!("{}: {text}", field.key)
                    });
                }
                let mut placed = None;
                if keyword == "hardware"
                    && let Declaration::Kind(id) = candidate.declaration
                {
                    let mount = self.hardware_mount(id, candidate);
                    if let Some(face) = mount.face_argument {
                        args.push(format!("face: {}", face.word()));
                    }
                    placed = mount.place;
                }
                let mut text = format!(
                    "{keyword} {} : {qualifier}{}({})",
                    name.as_str(),
                    decl.name,
                    args.join(", ")
                );
                if let Some(terms) = placed {
                    text.push_str(" {\n  place {\n");
                    for term in terms {
                        text.push_str(&format!("    {term}\n"));
                    }
                    text.push_str("  }\n}");
                }
                text
            }
        };

        let words = snake_case(&decl.name).replace('_', " ");
        let summary = match repeat {
            Some(repeat) => format!("Add {} {words}s", repeat.count.get()),
            None => format!("Add {} {words}", article(&words)),
        };
        let mut text = format!("stave 1\n\npatch {} {{\n", quoted(&summary));
        let in_file = format!(" in {}", quoted(&file));
        for import in imports.values() {
            let alias = if import.explicit {
                format!(" as {}", import.alias)
            } else {
                String::new()
            };
            text.push_str(&format!(
                "  add import {}{alias}{in_file}\n",
                quoted(&import.source)
            ));
        }
        if let (Some((repeat, written, bay)), Some(array)) = (&region, &array) {
            text.push_str(&format!(
                "  add region {}{} = split({}, {}){in_file}\n",
                bay.as_str(),
                array.declaration(),
                written.text,
                repeat.axis.word()
            ));
        }
        let mut lines = item.lines().peekable();
        let mut first = true;
        while let Some(line) = lines.next() {
            text.push_str("  ");
            if first {
                text.push_str("add ");
                first = false;
            }
            text.push_str(line);
            if lines.peek().is_none() {
                text.push_str(&in_file);
            }
            text.push('\n');
        }
        text.push_str("}\n");

        let mut sources = SourceMap::new();
        let patch_file = sources
            .add("patch.stave", text.as_str())
            .map_err(|_| WriteError::Unparsable(Vec::new()))?;
        let parse = stave_lang::parse_patch_file(patch_file, &text);
        if parse.has_errors() {
            return Err(WriteError::Unparsable(parse.diagnostics));
        }
        let patch = parse
            .ast
            .as_ref()
            .and_then(|file| file.patches.first())
            .cloned()
            .ok_or_else(|| WriteError::Unparsable(Vec::new()))?;
        Ok(WrittenPatch {
            text,
            parse,
            patch,
            file,
            name,
            bay,
        })
    }

    /// The fields the item writes, in declaration order (a component's
    /// params, or a kind's roles then its params), leaving out settings
    /// left to their default; and, when `bay` is given, the region param
    /// the repeat fills, whose field becomes `bay[same]`.
    fn fields(
        &self,
        candidate: &Candidate,
        bay: Option<&Ident>,
    ) -> Result<(Vec<Written>, Option<Written>), WriteError> {
        let defs = &self.model.defs;
        let order: Vec<String> = match candidate.declaration {
            Declaration::Component(id) => {
                let component = defs.component(id).ok_or(WriteError::UnknownDeclaration)?;
                component
                    .params
                    .iter()
                    .filter_map(|param| defs.param(*param).map(|p| p.decl.name.clone()))
                    .collect()
            }
            Declaration::Kind(_) => candidate
                .place
                .iter()
                .map(|param| param.name.as_str().to_owned())
                .chain(
                    candidate
                        .settings
                        .iter()
                        .map(|param| param.name.as_str().to_owned()),
                )
                .collect(),
        };
        let mut fields = Vec::new();
        let mut repeated = None;
        for key in order {
            if let Some(param) = candidate.place.iter().find(|p| p.name.as_str() == key) {
                let text = self
                    .binding(&param.slot)
                    .ok_or_else(|| WriteError::UnknownItem(key.clone()))?;
                let written = Written {
                    roots: vec![root_of(&text)],
                    key: key.clone(),
                    text,
                    names_from: None,
                };
                match (bay, &param.slot, &repeated) {
                    (Some(bay), PlaceSlot::Region(_), None) => {
                        fields.push(Written {
                            key,
                            text: format!("{}[same]", bay.as_str()),
                            roots: Vec::new(),
                            names_from: None,
                        });
                        repeated = Some(written);
                    }
                    _ => fields.push(written),
                }
            } else if let Some(param) = candidate.settings.iter().find(|p| p.name.as_str() == key) {
                let (value, names_from) = match &param.value {
                    SettingValue::Default => continue,
                    SettingValue::Unset => return Err(WriteError::Incomplete(vec![key])),
                    SettingValue::Copied { from, value } => {
                        let file = self
                            .model
                            .assemblies
                            .get(from.index())
                            .and_then(|assembly| defs.assembly(assembly.def))
                            .map(|def| def.decl.file);
                        (value, file)
                    }
                    SettingValue::Chosen(value) => (value, None),
                };
                let mut roots = Vec::new();
                expr_roots(value, &mut roots);
                fields.push(Written {
                    key,
                    text: expr_text(value),
                    roots,
                    names_from,
                });
            }
        }
        Ok((fields, repeated))
    }

    /// Where a piece of hardware of kind `id` sits on its piece (its first
    /// role): the `face:` its call writes, when it is offered on a face
    /// the kind doesn't mount it on, and the `place { }` terms that fix
    /// every axis the kind's `mount { }` leaves open, each centred on the
    /// piece (`center(stile, up)`). The open axes are found as the solver
    /// finds them: `along` is the piece's length (its width when the mount
    /// face is an end), `across` the other axis of the mount face. Without
    /// a solved design they are known only when both are open.
    fn hardware_mount(&self, id: stave_model::ids::KindId, candidate: &Candidate) -> HardwareMount {
        use stave_lang::placement::PartAxis;
        use stave_model::defs::{AxisRule, FaceRule};
        let mut out = HardwareMount {
            face_argument: None,
            place: None,
        };
        let Some(hardware) = self
            .model
            .defs
            .kind(id)
            .and_then(|kind| kind.hardware.as_ref())
        else {
            return out;
        };
        let piece = candidate.place.first().and_then(|param| match &param.slot {
            PlaceSlot::Part(Some(part)) => Some(*part),
            _ => None,
        });
        let Some(piece) = piece else {
            return out;
        };
        let clicked = candidate
            .mounted_on
            .filter(|face| face.part == piece)
            .map(|face| face.face);
        if let Some(face) = clicked
            && hardware.mount.face != FaceRule::Word(face)
        {
            out.face_argument = Some(face);
        }
        let solved = self.design.and_then(|design| design.part(piece));
        let face_axis = match (clicked, hardware.mount.face) {
            (Some(face), _) | (None, FaceRule::Word(face)) => Some(face.axis()),
            (None, FaceRule::Nearer { .. }) => {
                solved.map(|part| part.pose.orientation.direction(PartAxis::Thickness).axis())
            }
        };
        let Some(face_axis) = face_axis else {
            return out;
        };
        let open_across = hardware.mount.across == AxisRule::Open;
        let open_along = hardware.mount.along == AxisRule::Open;
        let axes: Vec<Axis> = match solved {
            Some(part) => {
                let orientation = part.pose.orientation;
                let length = orientation.direction(PartAxis::Length).axis();
                let along = if length == face_axis {
                    orientation.direction(PartAxis::Width).axis()
                } else {
                    length
                };
                let across = along.third(face_axis);
                let mut axes = Vec::new();
                if open_across && let Some(across) = across {
                    axes.push(across);
                }
                if open_along {
                    axes.push(along);
                }
                axes
            }
            None if open_across && open_along => face_axis.others().to_vec(),
            None => Vec::new(),
        };
        if axes.is_empty() {
            return out;
        }
        let Some(name) = self.model.parts.get(piece.index()).map(|p| p.name.clone()) else {
            return out;
        };
        let mut axes = axes;
        let order = [Axis::Across, Axis::Up, Axis::Deep];
        axes.sort_by_key(|axis| order.iter().position(|a| a == axis));
        out.place = Some(
            axes.into_iter()
                .map(|axis| format!("center({name}, {})", axis.word()))
                .collect(),
        );
        out
    }

    /// A place param's value as text: the region or part by its name, a
    /// face as `part.face`, several parts as a list.
    fn binding(&self, slot: &PlaceSlot) -> Option<String> {
        let model = self.model;
        let part = |id: stave_model::ids::PartId| model.parts.get(id.index()).map(|p| &p.name);
        match slot {
            PlaceSlot::Region(Some(id)) => model.regions.get(id.index()).map(|r| r.name.clone()),
            PlaceSlot::Anchor(Some(face)) => {
                part(face.part).map(|name| format!("{name}.{}", face.face.word()))
            }
            PlaceSlot::Part(Some(id)) => part(*id).cloned(),
            PlaceSlot::Parts(Some(parts)) => {
                let names = parts
                    .iter()
                    .map(|id| part(*id).cloned())
                    .collect::<Option<Vec<_>>>()?;
                match names.as_slice() {
                    [one] => Some(one.clone()),
                    _ => Some(format!("[{}]", names.join(", "))),
                }
            }
            PlaceSlot::Region(None)
            | PlaceSlot::Anchor(None)
            | PlaceSlot::Part(None)
            | PlaceSlot::Parts(None) => None,
        }
    }

    /// The file a top-level item of the project's module is declared in.
    fn project_item_file(&self, name: &str) -> Option<FileId> {
        let defs = &self.model.defs;
        let module = defs.modules.get(self.model.root.index())?;
        module.items.iter().find_map(|item| {
            let decl = defs.decl(*item)?;
            (decl.scope == Scope::Module(self.model.root) && decl.name == name).then_some(decl.file)
        })
    }

    /// What a declaration of `module` is prefixed with in `file`: nothing
    /// for the project's own module and the prelude, else the name the
    /// file imports the module under, adding the import from another file
    /// of the project when `file` lacks it.
    fn qualifier(
        &self,
        module: ModuleId,
        file: FileId,
        file_name: &str,
        imports: &mut BTreeMap<String, NeededImport>,
    ) -> Result<String, WriteError> {
        let defs = &self.model.defs;
        if module == self.model.root || defs.prelude.contains(&module) {
            return Ok(String::new());
        }
        if let Some(alias) = imported_alias(defs, file, module) {
            return Ok(format!("{alias}."));
        }
        let found = self.files.iter().find_map(|other| {
            imported_alias(defs, other.id, module).map(|alias| (other.id, alias.to_owned()))
        });
        let Some((from, alias)) = found else {
            return Err(WriteError::UnknownDeclaration);
        };
        self.need_import(from, &alias, module, file, file_name, imports)?;
        Ok(format!("{alias}."))
    }

    /// Adds the imports a field's names need in `file`.
    fn field_imports(
        &self,
        field: &Written,
        file: FileId,
        file_name: &str,
        imports: &mut BTreeMap<String, NeededImport>,
    ) -> Result<(), WriteError> {
        let defs = &self.model.defs;
        for root in &field.roots {
            if self.project_item_file(root).is_some() {
                continue;
            }
            let source = match field.names_from {
                Some(from) => defs
                    .files
                    .get(&from)
                    .and_then(|scope| scope.imports.get(root))
                    .map(|module| (from, *module)),
                None => self.files.iter().find_map(|other| {
                    defs.files
                        .get(&other.id)
                        .and_then(|scope| scope.imports.get(root))
                        .map(|module| (other.id, *module))
                }),
            };
            if let Some((from, module)) = source {
                self.need_import(from, root, module, file, file_name, imports)?;
            }
        }
        Ok(())
    }

    /// Makes sure `file` imports `module` as `alias`, taking the import as
    /// written in `from`.
    fn need_import(
        &self,
        from: FileId,
        alias: &str,
        module: ModuleId,
        file: FileId,
        file_name: &str,
        imports: &mut BTreeMap<String, NeededImport>,
    ) -> Result<(), WriteError> {
        let defs = &self.model.defs;
        let clash = || WriteError::ImportClash {
            file: file_name.to_owned(),
            alias: alias.to_owned(),
        };
        let target = defs.files.get(&file);
        match target.and_then(|scope| scope.imports.get(alias)) {
            Some(found) if *found == module => return Ok(()),
            Some(_) => return Err(clash()),
            None => {}
        }
        if imported_alias(defs, file, module).is_some() {
            return Err(clash());
        }
        if imports.contains_key(alias) {
            return Ok(());
        }
        let import = self
            .file(from)
            .and_then(|file| file.parse.ast.as_ref())
            .and_then(|parsed| {
                parsed
                    .imports
                    .iter()
                    .find(|import| import_name(import).as_deref() == Some(alias))
            })
            .ok_or_else(clash)?;
        let source = plain(&import.source).ok_or_else(clash)?;
        imports.insert(
            alias.to_owned(),
            NeededImport {
                alias: alias.to_owned(),
                source,
                explicit: import.alias.is_some(),
            },
        );
        Ok(())
    }
}

/// The name an import makes a module known by: its `as` name, or the last
/// segment of its source (`std:casework` gives `casework`, `./hardware`
/// gives `hardware`, a remote source drops its `@version` or `#hash`).
fn import_name(import: &ast::Import) -> Option<String> {
    if let Some(alias) = &import.alias {
        return Some(alias.as_str().to_owned());
    }
    let source = plain(&import.source)?;
    let source = source.split(['#', '@']).next().unwrap_or("");
    let last = match source.strip_prefix("std:") {
        Some(name) => name,
        None => source.rsplit(['/', ':']).find(|s| !s.is_empty())?,
    };
    Some(last.to_owned())
}

fn plain(literal: &ast::StringLit) -> Option<String> {
    let mut text = String::new();
    for part in &literal.parts {
        match part {
            StringPart::Text { text: piece, .. } => text.push_str(piece),
            StringPart::Interp { .. } => return None,
        }
    }
    Some(text)
}

/// The first segment of a written path: `door_bay` of `door_bay[left]`.
fn root_of(path: &str) -> String {
    path.trim_start_matches('[')
        .split(['[', '.', ',', ']'])
        .next()
        .unwrap_or("")
        .trim()
        .to_owned()
}

/// The names an expression refers to, in the order written: every bare
/// name, and the first segment of every member chain. Labels inside `[ ]`
/// are not names of items and are left out.
fn expr_roots(expr: &Expr, out: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::Name(name) => out.push(name.as_str().to_owned()),
        ExprKind::Member { base, .. } => expr_roots(base, out),
        ExprKind::Index { base, .. } => expr_roots(base, out),
        ExprKind::Call { callee, args } => {
            expr_roots(callee, out);
            for arg in args {
                expr_roots(&arg.value, out);
            }
        }
        ExprKind::Unary { operand, .. } => expr_roots(operand, out),
        ExprKind::Binary { lhs, rhs, .. } => {
            expr_roots(lhs, out);
            expr_roots(rhs, out);
        }
        ExprKind::Range { lo, hi } => {
            expr_roots(lo, out);
            expr_roots(hi, out);
        }
        ExprKind::List(items) => {
            for item in items {
                expr_roots(item, out);
            }
        }
        ExprKind::Paren(inner) => expr_roots(inner, out),
        ExprKind::Quantity(_)
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Bool(_)
        | ExprKind::Fit => {}
    }
}

/// `a` or `an` before a word.
fn article(words: &str) -> &'static str {
    match words.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_and_import_names() {
        assert_eq!(root_of("door_bay[left]"), "door_bay");
        assert_eq!(root_of("drawer[top].front"), "drawer");
        assert_eq!(root_of("[side_l, side_r]"), "side_l");
        assert_eq!(root_of("side_l.right"), "side_l");
        let import = |text: &str| {
            let source = format!("stave 1\n\n{text}\n");
            let mut sources = SourceMap::new();
            let file = sources.add("t.stave", source.as_str()).unwrap();
            let parse = stave_lang::parse_file(file, &source);
            import_name(&parse.ast.unwrap().imports[0])
        };
        assert_eq!(
            import(r#"import "std:casework""#).as_deref(),
            Some("casework")
        );
        assert_eq!(
            import(r#"import "./hardware" as hw"#).as_deref(),
            Some("hw")
        );
        assert_eq!(import(r#"import "../shop/jigs""#).as_deref(), Some("jigs"));
        assert_eq!(article("drawer"), "a");
        assert_eq!(article("apron"), "an");
    }
}

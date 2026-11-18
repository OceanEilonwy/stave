//! What the card shows for a candidate beyond what signature matching
//! needs (`docs/editing.md`, "The card"): the name the new item takes,
//! the text of every value, what a default stands for, what each setting
//! can be set to, a prompt for each place param still to point at, and
//! names for the ids its slots hold.
//!
//! [`EditScope::describe`] fills these fields of a [`Candidate`] from the
//! declarations in scope and the text they were read from. It is run when
//! the menu is listed and again after every change to the chosen
//! candidate, so the texts always say what the candidate now holds.

use stave_lang::ast::{self, BaseKind, Expr, ExprKind, Span, TypeName};
use stave_lang::{FileId, SourceMap};
use stave_model::defs::{DefId, Domain, ParamDef, Scope};

use super::print::expr_text;
use super::{EditScope, first_sentence};
use crate::edit::{
    Candidate, Declaration, MenuGroup, Names, SettingOption, SettingParam, SettingValue, ValueRange,
};
use crate::naming::{TakenNames, assembly_name, kind_item_name};

/// The mounts always offered for a `Mount` setting, after those the
/// project's assemblies already write: a front laid over its opening
/// (every edge `0mm` until set), and a front set inside it with a small
/// even gap all round.
const MOUNTS: [&str; 2] = ["overlay()", "inset(reveal: 2mm)"];

impl EditScope<'_> {
    /// [`EditScope::candidates`], each [`EditScope::describe`]d.
    pub fn described_candidates(
        &self,
        sources: &SourceMap,
        picked: &crate::edit::Picked,
    ) -> Vec<Candidate> {
        let mut candidates = self.candidates(picked);
        for candidate in &mut candidates {
            self.describe(sources, candidate);
        }
        candidates
    }

    /// Fills the card's fields of `candidate` ([`Candidate::generated_name`],
    /// each [`crate::edit::PlaceParam`]'s prompt and text, each
    /// [`SettingParam`]'s text, default, copied-from name, options and
    /// range) from the declarations in scope; `sources` are the opened
    /// project's sources, the standard library's included, which the
    /// defaults and domains are read from as written.
    pub fn describe(&self, sources: &SourceMap, candidate: &mut Candidate) {
        let model = self.model();
        let defs = &model.defs;
        let names = Names::new(model);
        let taken = TakenNames::of_project(model);
        let (params, generated): (Vec<&ParamDef>, _) = match candidate.declaration {
            Declaration::Component(id) => match defs.component(id) {
                Some(component) => (
                    component
                        .params
                        .iter()
                        .filter_map(|param| defs.param(*param))
                        .collect(),
                    assembly_name(&component.decl.name, &taken).ok(),
                ),
                None => (Vec::new(), None),
            },
            Declaration::Kind(id) => match defs.kind(id) {
                Some(kind) => (
                    kind.params.iter().collect(),
                    kind_item_name(&kind.decl.name, &taken).ok(),
                ),
                None => (Vec::new(), None),
            },
        };
        candidate.generated_name = generated;
        let param = |name: &str| params.iter().copied().find(|p| p.decl.name == name);

        for place in &mut candidate.place {
            place.text = names.slot(&place.slot);
            let docs = param(place.name.as_str()).map(|p| p.decl.docs.as_slice());
            place.prompt = docs.and_then(first_sentence).unwrap_or_else(|| {
                format!("Point at the {}.", place.name.as_str().replace('_', " "))
            });
        }
        for setting in &mut candidate.settings {
            let def = param(setting.name.as_str());
            setting.default_text = def.and_then(|def| {
                let default = def.default.as_ref()?;
                slice(sources, def.decl.file, default.span)
            });
            setting.text = match &setting.value {
                SettingValue::Default => setting.default_text.clone(),
                SettingValue::Copied { value, .. } | SettingValue::Chosen(value) => {
                    Some(expr_text(value))
                }
                SettingValue::Unset => None,
            };
            setting.copied_from = match &setting.value {
                SettingValue::Copied { from, .. } => names.assembly(*from).map(str::to_owned),
                SettingValue::Default | SettingValue::Chosen(_) | SettingValue::Unset => None,
            };
            setting.options = self.options(sources, setting, def);
            setting.range = def.and_then(|def| match def.domain.as_ref()? {
                Domain::Range { low, high, .. } => Some(ValueRange {
                    low: slice(sources, def.decl.file, low.span)?,
                    high: slice(sources, def.decl.file, high.span)?,
                }),
                Domain::OneOf { .. } => None,
            });
        }
    }

    /// What a setting can be set to, by its type (see
    /// [`SettingParam::options`]).
    fn options(
        &self,
        sources: &SourceMap,
        setting: &SettingParam,
        def: Option<&ParamDef>,
    ) -> Vec<SettingOption> {
        let bases: &[BaseKind] = match setting.ty {
            TypeName::Stock => return self.in_scope(|item| matches!(item, DefId::Stock(_))),
            TypeName::Mount => return self.mounts(),
            TypeName::JointKind => &[BaseKind::Joint],
            TypeName::HardwareKind => &[BaseKind::Slide, BaseKind::Hinge, BaseKind::Pull],
            TypeName::SlideKind => &[BaseKind::Slide],
            TypeName::HingeKind => &[BaseKind::Hinge],
            TypeName::PullKind => &[BaseKind::Pull],
            TypeName::FastenerKind => &[BaseKind::Fastener],
            TypeName::Tool => &[BaseKind::Tool],
            TypeName::Bool => {
                return ["true", "false"]
                    .into_iter()
                    .map(|text| SettingOption {
                        text: text.to_owned(),
                        hint: None,
                    })
                    .collect();
            }
            TypeName::Length
            | TypeName::Angle
            | TypeName::Ratio
            | TypeName::Count
            | TypeName::Text
            | TypeName::Part
            | TypeName::Parts
            | TypeName::Region
            | TypeName::Profile
            | TypeName::Anchor
            | TypeName::Axis
            | TypeName::Direction
            | TypeName::Shape
            | TypeName::Boards
            | TypeName::Splices
            | TypeName::Envelope
            | TypeName::Model
            | TypeName::Spacing
            | TypeName::Patch
            | TypeName::Recipe => {
                return def
                    .and_then(|def| match def.domain.as_ref()? {
                        Domain::OneOf { values, .. } => Some(
                            values
                                .iter()
                                .filter_map(|value| slice(sources, def.decl.file, value.span))
                                .map(|text| SettingOption { text, hint: None })
                                .collect(),
                        ),
                        Domain::Range { .. } => None,
                    })
                    .unwrap_or_default();
            }
        };
        let defs = &self.model().defs;
        self.in_scope(|item| match item {
            DefId::Kind(id) => defs
                .kind(*id)
                .is_some_and(|kind| bases.contains(&kind.base)),
            _ => false,
        })
    }

    /// The declarations in scope that `wanted` accepts, in menu-group
    /// order, each as the project would write it: bare for its own and
    /// the prelude's, through the import's name for an imported module's
    /// (`casework.oak_19`).
    fn in_scope(&self, wanted: impl Fn(&DefId) -> bool) -> Vec<SettingOption> {
        let model = self.model();
        let defs = &model.defs;
        let mut out = Vec::new();
        for (module, group) in self.modules_in_scope() {
            let own = module == model.root;
            let prefix = match &group {
                MenuGroup::Project | MenuGroup::Standard(_) => String::new(),
                MenuGroup::Module(alias) => format!("{}.", alias.as_str()),
            };
            let Some(module_defs) = defs.modules.get(module.index()) else {
                continue;
            };
            for item in &module_defs.items {
                if !wanted(item) || defs.is_poisoned(*item) {
                    continue;
                }
                let Some(decl) = defs.decl(*item) else {
                    continue;
                };
                if decl.scope != Scope::Module(module) || (decl.private && !own) {
                    continue;
                }
                let text = format!("{prefix}{}", decl.name);
                if out.iter().any(|option: &SettingOption| option.text == text) {
                    continue;
                }
                out.push(SettingOption {
                    text,
                    hint: first_sentence(&decl.docs),
                });
            }
        }
        out
    }

    /// The mounts the project's assemblies write, in file order and each
    /// once, then [`MOUNTS`].
    fn mounts(&self) -> Vec<SettingOption> {
        let mut texts: Vec<String> = Vec::new();
        for file in self.files() {
            let Some(parsed) = file.parse.ast.as_ref() else {
                continue;
            };
            for item in &parsed.items {
                let ast::ItemKind::Design(ast::DesignItem::Assembly(assembly)) = &item.kind else {
                    continue;
                };
                let ast::AssemblyDef::Instance { body, .. } = &assembly.def else {
                    continue;
                };
                for field in &body.fields {
                    if is_mount(&field.value) {
                        let text = expr_text(&field.value);
                        if !texts.contains(&text) {
                            texts.push(text);
                        }
                    }
                }
            }
        }
        for mount in MOUNTS {
            if !texts.iter().any(|text| text == mount) {
                texts.push(mount.to_owned());
            }
        }
        texts
            .into_iter()
            .map(|text| SettingOption { text, hint: None })
            .collect()
    }
}

/// `true` for a call of one of the language's mount functions.
fn is_mount(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Call { callee, .. } => {
            matches!(&callee.kind, ExprKind::Name(name) if matches!(name.as_str(), "overlay" | "inset"))
        }
        ExprKind::Paren(inner) => is_mount(inner),
        _ => false,
    }
}

/// The text a span covers in a file, as written.
fn slice(sources: &SourceMap, file: FileId, span: Span) -> Option<String> {
    let text = sources.get(file)?.text();
    let start = usize::try_from(span.start()).ok()?;
    let end = usize::try_from(span.end()).ok()?;
    text.get(start..end).map(str::to_owned)
}

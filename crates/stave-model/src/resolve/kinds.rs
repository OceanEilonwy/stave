//! Lowering kinds (joint, hardware, fastener and tool kinds) and recipes.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use stave_lang::ast::BaseKind;
use stave_lang::ast::{self, ExprKind, Span};
use stave_lang::placement::PartAxis;
use stave_lang::{Code, Diagnostic, Label};
use stave_schema::fields::{self, Requirement};
use stave_schema::kinds::{
    BaseField, JOINT_DEFAULT_ROLES, RESERVED_JOINT_SETTINGS, RoleType, base,
};
use stave_schema::operations::{
    ArgumentDefault, Feature, MARK, Measurement, Selector, StepTarget, cut_operation, operation,
    retired_argument,
};
use stave_schema::placement::{AssemblyStepKind, EXTEND, MadePartTerm};
use stave_schema::recipes::{
    COMMON_STEP_ARGUMENTS, CUTTING_STEP_ARGUMENTS, CuttingStep, MarkingStep, NeedKind,
};
use stave_schema::words::WordSet;
use stave_schema::{CutOperation, Operation};

use super::expr::Expect;
use super::items::kind_ids;
use super::{Body, Cx, Origin, Resolver};
use crate::defs::{
    BaseEntry, CutDef, CutName, CutStepDef, DefId, ExtendDef, HardwareKindDef, JointKindDef,
    JointMember, KindBody, KindDef, KindFields, MadePartDef, MadeTerm, MadeTermKind, MarkStepDef,
    MountedBase, MountedKindDef, Need, OperationDef, ParamDef, RExpr, RExprKind, RecipeDef,
    RequireDef, RoleDef, RoleIndex, SelectorDef, Setting, StepDef, StepMotionDef, Symbol, UseDef,
    Word,
};
use crate::ids::KindId;
use crate::types::Ty;

/// The roles of a kind: name, type and where declared.
pub(crate) fn role_specs(syntax: &ast::Kind) -> Vec<(String, RoleType, Span)> {
    let declared = syntax.body.entries.iter().find_map(|entry| match entry {
        ast::KindEntry::Roles(roles) => Some(roles),
        ast::KindEntry::Param(_)
        | ast::KindEntry::Require(_)
        | ast::KindEntry::Makes(_)
        | ast::KindEntry::Member(_)
        | ast::KindEntry::Cut(_)
        | ast::KindEntry::Mount(_)
        | ast::KindEntry::Mechanism(_)
        | ast::KindEntry::Adjust(_)
        | ast::KindEntry::Field(_) => None,
    });
    match (syntax.base.value, declared) {
        (
            BaseKind::Joint
            | BaseKind::Slide
            | BaseKind::Hinge
            | BaseKind::Pull
            | BaseKind::Support,
            Some(roles),
        ) => roles
            .roles
            .iter()
            .map(|role| {
                let ty = ast::TypeName::from_word(role.ty.as_str())
                    .and_then(RoleType::from_type)
                    .unwrap_or(RoleType::Parts);
                (role.name.as_str().to_owned(), ty, role.span)
            })
            .collect(),
        (BaseKind::Joint, None) => JOINT_DEFAULT_ROLES
            .iter()
            .map(|role| (role.name.to_owned(), role.ty, syntax.name.span()))
            .collect(),
        (
            BaseKind::Slide
            | BaseKind::Hinge
            | BaseKind::Pull
            | BaseKind::Support
            | BaseKind::Fastener
            | BaseKind::Tool,
            _,
        ) => Vec::new(),
    }
}

/// The params of a kind, in order.
pub(crate) fn kind_params(syntax: &ast::Kind) -> Vec<&ast::Param> {
    syntax
        .body
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ast::KindEntry::Param(param) => Some(&param.node),
            ast::KindEntry::Require(_)
            | ast::KindEntry::Makes(_)
            | ast::KindEntry::Member(_)
            | ast::KindEntry::Roles(_)
            | ast::KindEntry::Cut(_)
            | ast::KindEntry::Mount(_)
            | ast::KindEntry::Mechanism(_)
            | ast::KindEntry::Adjust(_)
            | ast::KindEntry::Field(_) => None,
        })
        .collect()
}

/// The parts a kind makes, in order.
pub(crate) fn made_parts(syntax: &ast::Kind) -> Vec<&ast::MadePart> {
    syntax
        .body
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ast::KindEntry::Makes(made) => Some(&made.node),
            ast::KindEntry::Param(_)
            | ast::KindEntry::Require(_)
            | ast::KindEntry::Member(_)
            | ast::KindEntry::Roles(_)
            | ast::KindEntry::Cut(_)
            | ast::KindEntry::Mount(_)
            | ast::KindEntry::Mechanism(_)
            | ast::KindEntry::Adjust(_)
            | ast::KindEntry::Field(_) => None,
        })
        .collect()
}

/// `true` for `cut crossing(role, member) { ... }` where `member` is one
/// of the kind's members with a body.
fn is_member_cut(cut: &ast::Cut, members: &[&str]) -> bool {
    cut.selector.head.as_str() == Selector::Crossing.word()
        && cut
            .selector
            .args
            .iter()
            .filter(|arg| arg.name.is_none())
            .nth(1)
            .is_some_and(|arg| {
                matches!(&arg.value.kind, ExprKind::Name(name) if members.contains(&name.as_str()))
            })
}

/// A list expression's elements, or the expression itself.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
)]
fn elements(expr: &ast::Expr) -> Vec<&ast::Expr> {
    match &expr.kind {
        ExprKind::List(items) => items.iter().collect(),
        _ => vec![expr],
    }
}

impl Resolver<'_> {
    pub(super) fn lower_kind(
        &mut self,
        id: KindId,
        syntax: &ast::Kind,
        origin: Origin<'_>,
    ) -> KindDef {
        self.ensure_kind_names(id, syntax);
        let cx = Cx {
            module: origin.module,
            file: origin.file,
            body: Body::Kind(id),
            array: None,
        };
        let base_kind = syntax.base.value;
        let what = format!("kind '{}'", syntax.name.as_str());
        self.check_kind_names(&cx, syntax);

        let roles: Vec<RoleDef> = role_specs(syntax)
            .into_iter()
            .map(|(name, ty, span)| RoleDef { name, span, ty })
            .collect();
        let mut params = Vec::new();
        for (index, param) in kind_params(syntax).into_iter().enumerate() {
            self.kind_param_ty(id, index);
            let (ty, default) = self
                .early_kind_params
                .remove(&(id, index))
                .unwrap_or((Ty::Unknown, None));
            self.check_param_kind(origin.file, param, &ty, default.is_some(), false);
            let domain = param
                .within
                .as_ref()
                .map(|d| self.lower_domain(&cx, d, &ty));
            params.push(ParamDef {
                decl: crate::defs::Decl {
                    name: param.name.as_str().to_owned(),
                    name_span: param.name.span(),
                    file: origin.file,
                    span: param.name.span(),
                    scope: origin.scope,
                    docs: Vec::new(),
                    private: false,
                },
                ty,
                default,
                domain,
                kind: param.kind(),
            });
        }
        let mut requires = Vec::new();
        let mut kind_fields: Vec<Setting> = Vec::new();
        let mut joint = (base_kind == BaseKind::Joint).then(JointKindDef::default);
        let mut seen_fields: BTreeMap<String, Span> = BTreeMap::new();
        let mut written_members: Vec<&ast::Documented<ast::KindMember>> = Vec::new();
        let mut mechanisms: Vec<&ast::MechanismBlock> = Vec::new();
        let mounted = base_kind.is_mounted_hardware();
        let mut mounts: Vec<&ast::MountBlock> = Vec::new();
        let mut hardware_cuts: Vec<&ast::Cut> = Vec::new();
        let mut adjusts: Vec<&ast::Adjust> = Vec::new();
        let mut hardware_uses = JointKindDef::default();
        let mut joint_cuts: Vec<&ast::Cut> = Vec::new();
        self.check_cut_names(&cx, &syntax.body.entries);
        let cut_names: Vec<String> = syntax
            .body
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ast::KindEntry::Cut(cut) => cut.name.as_ref().map(|n| n.as_str().to_owned()),
                ast::KindEntry::Param(_)
                | ast::KindEntry::Require(_)
                | ast::KindEntry::Makes(_)
                | ast::KindEntry::Member(_)
                | ast::KindEntry::Roles(_)
                | ast::KindEntry::Mount(_)
                | ast::KindEntry::Mechanism(_)
                | ast::KindEntry::Adjust(_)
                | ast::KindEntry::Field(_) => None,
            })
            .collect();
        self.kind_cut_names.insert(id, cut_names);
        for entry in &syntax.body.entries {
            match entry {
                ast::KindEntry::Param(_) => {}
                ast::KindEntry::Mount(block) => {
                    if mounted {
                        mounts.push(block);
                    } else {
                        self.error_help(
                            Code::UnknownField,
                            format!("a {} kind has no field 'mount'", base_kind.word()),
                            cx.file,
                            block.span,
                            "only slide, hinge and pull kinds are mounted on a part",
                        );
                    }
                }
                ast::KindEntry::Member(member) => written_members.push(member),
                ast::KindEntry::Mechanism(block) => mechanisms.push(block),
                ast::KindEntry::Adjust(adjust) => {
                    if mounted {
                        adjusts.push(adjust);
                    } else {
                        self.error_help(
                            Code::UnknownField,
                            format!("a {} kind has no field 'adjust'", base_kind.word()),
                            cx.file,
                            adjust.span,
                            "only slide, hinge and pull kinds have members that are set on each other",
                        );
                    }
                }
                ast::KindEntry::Require(require) => {
                    self.in_kind_require = true;
                    let condition = self.value(
                        &cx,
                        &require.node.condition,
                        &Expect::ty(Ty::Bool).named(require.node.name.as_str()),
                    );
                    self.in_kind_require = false;
                    self.check_wall_comparison(&cx, &condition);
                    let message = self.text(&cx, &require.node.message);
                    requires.push(RequireDef {
                        decl: crate::defs::Decl {
                            name: require.node.name.as_str().to_owned(),
                            name_span: require.node.name.span(),
                            file: origin.file,
                            span: require.span,
                            scope: origin.scope,
                            docs: require.docs.clone(),
                            private: false,
                        },
                        condition,
                        message,
                    });
                }
                ast::KindEntry::Roles(roles) => {
                    if base_kind != BaseKind::Joint && !mounted {
                        self.error_help(
                            Code::UnknownField,
                            format!("a {} kind has no field 'roles'", base_kind.word()),
                            cx.file,
                            roles.span,
                            "only joint, slide, hinge and pull kinds declare their roles",
                        );
                    }
                }
                ast::KindEntry::Cut(cut) => match &mut joint {
                    // Lowered once the members are known: a cut on
                    // crossing(role, member) is the member's.
                    Some(_) => joint_cuts.push(cut),
                    None if mounted => hardware_cuts.push(cut),
                    None => self.error_help(
                        Code::UnknownField,
                        format!("a {} kind has no field 'cut'", base_kind.word()),
                        cx.file,
                        cut.span,
                        "only joint, slide, hinge and pull kinds cut parts",
                    ),
                },
                ast::KindEntry::Makes(made) => match &mut joint {
                    Some(joint) => {
                        let lowered = self.lower_made_part(&cx, id, &made.node, made.span);
                        joint.makes.push(lowered);
                    }
                    None => self.error_help(
                        Code::UnknownField,
                        format!("a {} kind has no field 'makes'", base_kind.word()),
                        cx.file,
                        made.span,
                        "only joint kinds make parts",
                    ),
                },
                ast::KindEntry::Field(field) => {
                    let key = field.key.as_str();
                    if let Some(first) = seen_fields.get(key) {
                        self.report(
                            Diagnostic::error(
                                Code::DuplicateName,
                                format!("'{key}' is declared twice"),
                                Label::new(cx.file, field.key.span()),
                            )
                            .with_label(Label::with_message(cx.file, *first, "first written here"))
                            .with_help(format!("write '{key}' once; remove one of them")),
                        );
                        continue;
                    }
                    seen_fields.insert(key.to_owned(), field.key.span());
                    match &mut joint {
                        Some(joint_def) => {
                            self.lower_joint_field(&cx, id, field, joint_def, &what);
                        }
                        None if mounted && key == "uses" => {
                            self.lower_joint_field(&cx, id, field, &mut hardware_uses, &what);
                        }
                        None if mounted && fields::find(fields::JOINT_KIND, key).is_some() => {
                            self.error_help(
                                Code::UnknownField,
                                format!("a {} kind has no field '{key}'", base_kind.word()),
                                cx.file,
                                field.key.span(),
                                "insert, cut_first, adds and note are a joint kind's entries",
                            );
                        }
                        None if mounted && base(base_kind).entry(key).is_none() => {
                            // Catalogue data: any field the kind likes.
                            if kind_params(syntax).iter().any(|p| p.name.as_str() == key) {
                                self.error(
                                    Code::DuplicateName,
                                    format!("'{key}' is declared twice"),
                                    cx.file,
                                    field.key.span(),
                                );
                                continue;
                            }
                            let value = self.value(&cx, &field.value, &Expect::any().named(key));
                            kind_fields.push(Setting {
                                name: key.to_owned(),
                                name_span: field.key.span(),
                                value,
                            });
                        }
                        None => {
                            let Some(spec) = base(base_kind).entry(key) else {
                                let spec = base(base_kind);
                                let names: Vec<&str> = spec
                                    .required
                                    .iter()
                                    .chain(spec.optional)
                                    .map(|s| s.name)
                                    .collect();
                                self.unknown(
                                    Code::UnknownField,
                                    format!("a {} kind has no field '{key}'", base_kind.word()),
                                    cx.file,
                                    field.key.span(),
                                    key,
                                    names.iter().copied(),
                                    format!("its entries are: {}", names.join(", ")),
                                );
                                continue;
                            };
                            if kind_params(syntax).iter().any(|p| p.name.as_str() == key) {
                                self.error(
                                    Code::DuplicateName,
                                    format!("'{key}' is declared twice"),
                                    cx.file,
                                    field.key.span(),
                                );
                                continue;
                            }
                            let value = self.value(
                                &cx,
                                &field.value,
                                &Expect::value_type(spec.ty).named(key),
                            );
                            kind_fields.push(Setting {
                                name: key.to_owned(),
                                name_span: field.key.span(),
                                value,
                            });
                        }
                    }
                }
            }
        }
        if base_kind != BaseKind::Joint {
            for spec in base(base_kind).required {
                let misspelt = syntax.body.entries.iter().any(|entry| match entry {
                    ast::KindEntry::Field(field) => {
                        base(base_kind).entry(field.key.as_str()).is_none()
                            && !crate::suggest::closest(field.key.as_str(), [spec.name]).is_empty()
                    }
                    ast::KindEntry::Param(_)
                    | ast::KindEntry::Require(_)
                    | ast::KindEntry::Makes(_)
                    | ast::KindEntry::Member(_)
                    | ast::KindEntry::Roles(_)
                    | ast::KindEntry::Cut(_)
                    | ast::KindEntry::Mount(_)
                    | ast::KindEntry::Mechanism(_)
                    | ast::KindEntry::Adjust(_) => false,
                });
                let written = kind_fields.iter().any(|f| f.name == spec.name)
                    || params.iter().any(|p| p.decl.name == spec.name)
                    || misspelt;
                if spec.requirement == Requirement::Required && !written {
                    self.error_help(
                        Code::MissingField,
                        format!("{what} is missing '{}'", spec.name),
                        cx.file,
                        syntax.body.span,
                        format!("write {}: ... or declare param {}", spec.name, spec.name),
                    );
                }
            }
            for param in &params {
                if let Some(spec) = base(base_kind).entry(&param.decl.name)
                    && let Some(wanted) = Ty::from_value_type(spec.ty)
                {
                    {
                        if !param.ty.fits(&wanted) {
                            self.error(
                                Code::TypeMismatch,
                                format!("'{}' needs {wanted}, found {}", param.decl.name, param.ty),
                                cx.file,
                                param.decl.name_span,
                            );
                        }
                    }
                }
            }
        }
        self.check_members(&cx, base_kind, &written_members);
        let bodies = if base_kind.is_mounted_hardware() || base_kind == BaseKind::Joint {
            self.lower_bodies(&cx, base_kind, &roles, &written_members)
        } else {
            // Already reported: this kind has no members at all.
            None
        };
        if let Some(joint) = &mut joint {
            let body_names: Vec<&str> = written_members
                .iter()
                .filter(|member| member.node.body.is_some())
                .map(|member| member.node.name.as_str())
                .collect();
            for cut in joint_cuts {
                if is_member_cut(cut, &body_names) {
                    // A member that didn't lower has been reported.
                    if let Some(bodies) = bodies.as_deref()
                        && let Some(lowered) = self.lower_hardware_cut(&cx, &roles, bodies, cut)
                    {
                        joint.member_cuts.push(lowered);
                    }
                } else if let Some(lowered) = self.lower_cut(&cx, id, cut) {
                    joint.cuts.push(lowered);
                }
            }
        }
        let mechanism = self.kind_mechanism(&cx, syntax, base_kind, bodies.as_deref(), &mechanisms);
        let hardware = if mounted {
            self.hardware_kind(
                &cx,
                syntax,
                &roles,
                bodies.as_deref(),
                &mounts,
                &hardware_cuts,
                &adjusts,
                hardware_uses.uses,
            )
        } else {
            None
        };
        // A param of the entry's name is read before a field of it.
        let spec = base(base_kind);
        let base_entries = BaseField::ALL
            .into_iter()
            .filter(|field| spec.entry(field.word()).is_some())
            .filter_map(|field| {
                let word = field.word();
                let entry = match params.iter().position(|p| p.decl.name == word) {
                    Some(index) => BaseEntry::Param(index),
                    None => BaseEntry::Field(kind_fields.iter().position(|f| f.name == word)?),
                };
                Some((field, entry))
            })
            .collect();
        let fields = KindFields {
            settings: kind_fields,
            base_entries,
        };
        let body = match (joint, MountedBase::new(base_kind)) {
            (Some(joint), _) => KindBody::Joint(joint),
            (None, Some(base)) => KindBody::Mounted(Box::new(MountedKindDef {
                base,
                fields,
                mechanism,
                hardware,
            })),
            (None, None) if base_kind == BaseKind::Tool => KindBody::Tool(fields),
            (None, None) => KindBody::Fastener(fields),
        };
        KindDef {
            decl: self.decl(DefId::Kind(id), &syntax.name, origin),
            roles,
            params,
            requires,
            bodies: bodies.unwrap_or_default(),
            body,
        }
    }

    /// A slide, hinge or pull kind's roles, `mount { }`, cuts, `adjust`
    /// entries and `uses:`.
    /// `None` (reported) when one of them is missing or doesn't lower.
    #[allow(clippy::too_many_arguments)]
    fn hardware_kind(
        &mut self,
        cx: &Cx,
        syntax: &ast::Kind,
        roles: &[RoleDef],
        bodies: Option<&[crate::defs::BodyDef]>,
        mounts: &[&ast::MountBlock],
        cuts: &[&ast::Cut],
        adjusts: &[&ast::Adjust],
        uses: Vec<UseDef>,
    ) -> Option<HardwareKindDef> {
        let what = format!("kind '{}'", syntax.name.as_str());
        let piece_is_part = roles.first().map(|role| role.ty);
        if piece_is_part != Some(RoleType::Part) {
            let help = if roles.is_empty() {
                "declare the parts it joins: roles { piece: Part, on: Part }; the first is the part it is mounted on"
            } else {
                "make the first role the part it is mounted on: piece: Part"
            };
            self.error_help(
                Code::MissingField,
                format!("{what} is missing 'roles'"),
                cx.file,
                syntax.name.span(),
                help,
            );
        }
        for role in roles {
            if role.ty == RoleType::Parts {
                self.error_help(
                    Code::TypeMismatch,
                    format!("'{}' needs Part or Region, found 'Parts'", role.name),
                    cx.file,
                    role.span,
                    "a piece of hardware joins single parts: write Part (or Region for an opening)",
                );
            }
        }
        let (first, rest) = match mounts.split_first() {
            Some(split) => split,
            None => {
                self.error_help(
                    Code::MissingField,
                    format!("{what} is missing 'mount'"),
                    cx.file,
                    syntax.name.span(),
                    "say where it sits on its piece: mount { face: ..., across: ..., along: ... }",
                );
                return None;
            }
        };
        for again in rest {
            self.duplicate("mount", again.span, first.span, cx);
        }
        let mount = self.lower_mount(cx, roles, first);
        let bodies = bodies.unwrap_or_default();
        let mut lowered = Vec::new();
        for cut in cuts {
            if let Some(cut) = self.lower_hardware_cut(cx, roles, bodies, cut) {
                lowered.push(cut);
            }
        }
        let adjusts = self.lower_adjusts(cx, syntax.name.as_str(), bodies, adjusts);
        Some(HardwareKindDef {
            mount: mount?,
            cuts: lowered,
            uses,
            adjusts,
        })
    }

    /// A kind's one `mechanism`, where its base may have one. Version 1
    /// solves only a cup hinge's (grammar, "Mechanisms").
    fn kind_mechanism(
        &mut self,
        cx: &Cx,
        syntax: &ast::Kind,
        base_kind: BaseKind,
        bodies: Option<&[crate::defs::BodyDef]>,
        mechanisms: &[&ast::MechanismBlock],
    ) -> Option<crate::defs::MechanismDef> {
        let (first, rest) = mechanisms.split_first()?;
        for again in rest {
            self.report(
                Diagnostic::error(
                    Code::DuplicateName,
                    "'mechanism' is declared twice",
                    Label::new(cx.file, again.span),
                )
                .with_label(Label::with_message(
                    cx.file,
                    first.span,
                    "first written here",
                ))
                .with_help("write one mechanism block; put every entry in it"),
            );
        }
        if !base_kind.is_mounted_hardware() {
            self.error_help(
                Code::UnknownField,
                format!("a {} kind has no field 'mechanism'", base_kind.word()),
                cx.file,
                first.span,
                "only hinge, slide and pull kinds have members that move",
            );
            return None;
        }
        self.lower_mechanism(cx, syntax.name.as_str(), bodies?, first)
    }

    /// A kind's `member` entries: each named once; only a slide, hinge or
    /// pull kind has members, and every member has a body (its shapes are
    /// lowered with the bodies).
    fn check_members(
        &mut self,
        cx: &Cx,
        base_kind: BaseKind,
        written: &[&ast::Documented<ast::KindMember>],
    ) {
        let mut seen: BTreeMap<&str, Span> = BTreeMap::new();
        for member in written {
            let word = member.node.name.as_str();
            let span = member.node.name.span();
            if let Some(first) = seen.get(word) {
                self.report(
                    Diagnostic::error(
                        Code::DuplicateName,
                        format!("'{word}' is declared twice"),
                        Label::new(cx.file, span),
                    )
                    .with_label(Label::with_message(cx.file, *first, "first written here"))
                    .with_help(format!("write 'member {word}' once; remove one of them")),
                );
                continue;
            }
            seen.insert(word, span);
            if !base_kind.is_mounted_hardware() && base_kind != BaseKind::Joint {
                self.error_help(
                    Code::UnknownField,
                    format!("a {} kind has no member '{word}'", base_kind.word()),
                    cx.file,
                    span,
                    "only joint, hinge, slide and pull kinds name their members",
                );
            } else if member.node.body.is_none() {
                let help = if base_kind == BaseKind::Joint {
                    format!(
                        "a joint's member is placed and checked: member {word} : ...(...) {{ on: contact(piece, into), shape: cylinder(...) }}"
                    )
                } else {
                    format!(
                        "give it what it is made of: member {word} {{ shape: box(along: ..., across: ..., depth: ...) }}"
                    )
                };
                self.error_help(
                    Code::MissingField,
                    format!("member '{word}' is missing 'shape'"),
                    cx.file,
                    span,
                    help,
                );
            }
        }
    }

    /// Duplicate role, param and made-part names; reserved setting names;
    /// role types.
    fn check_kind_names(&mut self, cx: &Cx, syntax: &ast::Kind) {
        let mut seen: BTreeMap<String, Span> = BTreeMap::new();
        let roles = role_specs(syntax);
        let declared_roles = syntax
            .body
            .entries
            .iter()
            .any(|e| matches!(e, ast::KindEntry::Roles(_)));
        let mut names: Vec<(String, Span)> = Vec::new();
        if declared_roles {
            for entry in &syntax.body.entries {
                if let ast::KindEntry::Roles(block) = entry {
                    for role in &block.roles {
                        names.push((role.name.as_str().to_owned(), role.name.span()));
                        let joint = syntax.base.value == BaseKind::Joint;
                        let valid = ast::TypeName::from_word(role.ty.as_str())
                            .and_then(RoleType::from_type)
                            .filter(|ty| !joint || *ty != RoleType::Region);
                        if valid.is_none() {
                            let (wanted, help) = if joint {
                                (
                                    "Part or Parts",
                                    "a role is Part (exactly one part) or Parts (one or more)",
                                )
                            } else {
                                (
                                    "Part or Region",
                                    "a hardware role is Part (exactly one part) or Region (an opening)",
                                )
                            };
                            self.error_help(
                                Code::TypeMismatch,
                                format!(
                                    "'{}' needs {wanted}, found '{}'",
                                    role.name.as_str(),
                                    role.ty.as_str()
                                ),
                                cx.file,
                                role.ty.span(),
                                help,
                            );
                        }
                    }
                }
            }
            let has_parts = roles.iter().any(|(_, ty, _)| *ty == RoleType::Parts);
            if has_parts && roles.len() > 2 {
                self.error_help(
                    Code::TypeMismatch,
                    format!(
                        "kind '{}' has a Parts role, so it has one role or two",
                        syntax.name.as_str()
                    ),
                    cx.file,
                    syntax.name.span(),
                    "a joint between three or more parts uses Part roles only",
                );
            }
        }
        names.extend(
            kind_params(syntax)
                .iter()
                .map(|p| (p.name.as_str().to_owned(), p.name.span())),
        );
        names.extend(
            made_parts(syntax)
                .iter()
                .map(|m| (m.name.as_str().to_owned(), m.name.span())),
        );
        for (name, span) in names {
            if syntax.base.value == BaseKind::Joint
                && RESERVED_JOINT_SETTINGS.contains(&name.as_str())
            {
                self.error_help(
                    Code::DuplicateName,
                    format!("'{name}' is declared twice"),
                    cx.file,
                    span,
                    format!("every joint call already accepts '{name}'; choose another name"),
                );
                continue;
            }
            if let Some(first) = seen.get(&name) {
                self.report(
                    Diagnostic::error(
                        Code::DuplicateName,
                        format!("'{name}' is declared twice"),
                        Label::new(cx.file, span),
                    )
                    .with_label(Label::with_message(cx.file, *first, "first declared here"))
                    .with_help("roles, params and made parts of a kind share one namespace"),
                );
                continue;
            }
            if !declared_roles && roles.iter().any(|(role, _, _)| *role == name) {
                self.error_help(
                    Code::DuplicateName,
                    format!("'{name}' is declared twice"),
                    cx.file,
                    span,
                    format!("'{name}' is a role of this kind"),
                );
                continue;
            }
            seen.insert(name, span);
        }
    }

    fn joint_member(&mut self, cx: &Cx, expr: &ast::Expr, what: &str) -> Option<JointMember> {
        let ExprKind::Name(ident) = &expr.kind else {
            self.error(
                Code::TypeMismatch,
                format!("'{what}' needs a role, found an expression"),
                cx.file,
                expr.span,
            );
            return None;
        };
        match self.lookup(cx, ident.as_str()) {
            Some(Symbol::Role(_, index)) => Some(JointMember::Role(index)),
            Some(Symbol::MadePart(_, index)) => Some(JointMember::Made(index)),
            Some(_) => {
                self.error_help(
                    Code::TypeMismatch,
                    format!("'{what}' needs a role, found '{}'", ident.as_str()),
                    cx.file,
                    expr.span,
                    "name one of the kind's roles or the parts it makes",
                );
                None
            }
            None => {
                let candidates = self.names_in_scope(cx);
                self.unknown(
                    Code::UnknownName,
                    format!("unknown name '{}'", ident.as_str()),
                    cx.file,
                    expr.span,
                    ident.as_str(),
                    candidates,
                    "name one of the kind's roles or the parts it makes",
                );
                None
            }
        }
    }

    fn role_index(&mut self, cx: &Cx, expr: &ast::Expr, what: &str) -> Option<RoleIndex> {
        match self.joint_member(cx, expr, what)? {
            JointMember::Role(index) => Some(index),
            JointMember::Made(_) => {
                self.error(
                    Code::TypeMismatch,
                    format!("'{what}' needs a role, found a made part"),
                    cx.file,
                    expr.span,
                );
                None
            }
        }
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "picks out only the variants it names: any other, a new one included, is not one of them"
    )]
    fn part_axis(&mut self, cx: &Cx, expr: &ast::Expr, what: &str) -> Option<PartAxis> {
        let value = self.value(
            cx,
            expr,
            &Expect::ty(Ty::Word(WordSet::PartAxis)).named(what),
        );
        match value.kind {
            RExprKind::Word(Word::PartAxis(axis)) => Some(axis),
            _ => None,
        }
    }

    fn bool_literal(&mut self, cx: &Cx, expr: &ast::Expr, what: &str) -> bool {
        let value = self.value(cx, expr, &Expect::ty(Ty::Bool).named(what));
        matches!(value.kind, RExprKind::Bool(true))
    }

    fn lower_joint_field(
        &mut self,
        cx: &Cx,
        kind: KindId,
        field: &ast::Field,
        joint: &mut JointKindDef,
        what: &str,
    ) {
        let key = field.key.as_str();
        match key {
            "insert" => {
                for step in elements(&field.value) {
                    if let Some(lowered) = self.lower_step(cx, step) {
                        joint.insert.push(lowered);
                    }
                }
            }
            "cut_first" => {
                for role in elements(&field.value) {
                    if let Some(index) = self.role_index(cx, role, "cut_first") {
                        joint.cut_first.push(index);
                    }
                }
            }
            "adds" => {
                for extend in elements(&field.value) {
                    let ExprKind::Call { callee, args } = &extend.kind else {
                        self.error_help(
                            Code::TypeMismatch,
                            "'adds' needs extend(role, by: d)",
                            cx.file,
                            extend.span,
                            "write adds: extend(piece, by: depth)",
                        );
                        continue;
                    };
                    let is_extend =
                        matches!(&callee.kind, ExprKind::Name(n) if n.as_str() == EXTEND);
                    if !is_extend {
                        self.unknown(
                            Code::UnknownName,
                            format!("unknown name '{}'", callee_text(callee)),
                            cx.file,
                            callee.span,
                            &callee_text(callee),
                            [EXTEND],
                            "'adds' takes extend(role, by: d)",
                        );
                        continue;
                    }
                    let args = self.split_args(cx, EXTEND, args, &["by"], 1);
                    let (Some(role), Some(by)) = (args.positional.first(), args.get("by")) else {
                        self.missing_argument(cx, EXTEND, "a role and by:", extend.span);
                        continue;
                    };
                    let Some(role) = self.role_index(cx, role, "extend") else {
                        continue;
                    };
                    let by = self.value(cx, by, &Expect::ty(Ty::Length).named("by"));
                    joint.adds.push(ExtendDef {
                        role,
                        by,
                        span: extend.span,
                    });
                }
            }
            "uses" => {
                for fastener in elements(&field.value) {
                    #[expect(
                        clippy::wildcard_enum_match_arm,
                        reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
                    )]
                    let (callee, args) = match &fastener.kind {
                        ExprKind::Call { callee, args } => (&**callee, args.as_slice()),
                        _ => (fastener, &[][..]),
                    };
                    let kind_expr = self.value(
                        cx,
                        callee,
                        &Expect::ty(Ty::from_type_name(ast::TypeName::FastenerKind)).named("uses"),
                    );
                    let Ty::Kind(kind_ty) = kind_expr.ty.clone() else {
                        continue;
                    };
                    let (count_args, setting_args): (Vec<&ast::Arg>, Vec<&ast::Arg>) =
                        args.iter().partition(|a| {
                            a.name
                                .as_ref()
                                .is_some_and(|n| n.as_str() == fields::USES_COUNT)
                        });
                    let setting_args: Vec<ast::Arg> = setting_args.into_iter().cloned().collect();
                    let settings = self.kind_settings(cx, kind_ty, &setting_args, &[]);
                    let count = count_args
                        .first()
                        .map(|a| self.value(cx, &a.value, &Expect::ty(Ty::Count).named("count")));
                    let span = fastener.span;
                    joint.uses.push(UseDef {
                        fastener: RExpr {
                            kind: RExprKind::KindValue {
                                kind: Box::new(kind_expr),
                                settings,
                            },
                            ty: Ty::Kind(kind_ty),
                            span,
                        },
                        count,
                        span,
                    });
                }
            }
            "note" => {
                if let ExprKind::String(string) = &field.value.kind {
                    joint.note = Some(self.text(cx, string));
                } else {
                    self.plain_text(cx, &field.value, "note");
                }
            }
            _ => {
                let names: Vec<&str> = fields::JOINT_KIND.iter().map(|s| s.name).collect();
                self.unknown(
                    Code::UnknownField,
                    format!("a joint kind has no field '{key}'"),
                    cx.file,
                    field.key.span(),
                    key,
                    names.iter().copied(),
                    format!("its fields are: {}, plus roles, param, require, cut and makes part ({what})", names.join(", ")),
                );
            }
        }
        let _ = kind;
    }

    fn lower_step(&mut self, cx: &Cx, step: &ast::Expr) -> Option<StepDef> {
        let ExprKind::Call { callee, args } = &step.kind else {
            self.error_help(
                Code::TypeMismatch,
                "'insert' needs assembly steps",
                cx.file,
                step.span,
                "write insert: [push(piece, along: length)]",
            );
            return None;
        };
        let head = callee_text(callee);
        let Some(kind) = AssemblyStepKind::from_word(&head) else {
            self.unknown(
                Code::UnknownName,
                format!("unknown name '{head}'"),
                cx.file,
                callee.span,
                &head,
                AssemblyStepKind::ALL.map(AssemblyStepKind::word),
                "an assembly step is push, rotate or drive",
            );
            return None;
        };
        let named: &[&str] = match kind {
            AssemblyStepKind::Push => &["along", "by", "angle"],
            AssemblyStepKind::Rotate => &["by", "about"],
            AssemblyStepKind::Drive => &["along"],
        };
        let parsed = self.split_args(cx, &head, args, named, 1);
        let part = self.joint_member(cx, parsed.positional.first()?, &head)?;
        let axis_name = if kind == AssemblyStepKind::Rotate {
            "about"
        } else {
            "along"
        };
        let Some(axis_expr) = parsed.get(axis_name) else {
            self.missing_argument(cx, &head, &format!("{axis_name}:"), step.span);
            return None;
        };
        let axis = self.part_axis(cx, axis_expr, axis_name)?;
        let motion = match kind {
            AssemblyStepKind::Push => StepMotionDef::Push {
                by: parsed
                    .get("by")
                    .map(|e| self.value(cx, e, &Expect::ty(Ty::Length).named("by"))),
                angle: parsed
                    .get("angle")
                    .map(|e| self.value(cx, e, &Expect::ty(Ty::Angle).named("angle"))),
            },
            AssemblyStepKind::Rotate => {
                let Some(by) = parsed.get("by") else {
                    self.missing_argument(cx, &head, "by:", step.span);
                    return None;
                };
                StepMotionDef::Rotate {
                    by: self.value(cx, by, &Expect::ty(Ty::Angle).named("by")),
                }
            }
            AssemblyStepKind::Drive => StepMotionDef::Drive,
        };
        Some(StepDef {
            part,
            axis,
            motion,
            span: step.span,
        })
    }

    fn lower_selector(
        &mut self,
        cx: &Cx,
        head: &ast::Ident,
        args: &[ast::Arg],
        span: Span,
    ) -> Option<SelectorDef> {
        let Some(selector) = Selector::from_word(head.as_str()) else {
            self.unknown(
                Code::UnknownName,
                format!("unknown name '{}'", head.as_str()),
                cx.file,
                head.span(),
                head.as_str(),
                Selector::ALL.map(Selector::word),
                "a face selector is end, side or crossing",
            );
            return None;
        };
        let names: Vec<&str> = selector.arguments().iter().map(|a| a.name).collect();
        let parsed = self.split_args(cx, head.as_str(), args, &names, selector.positional_parts());
        let Some(first) = parsed.positional.first().copied() else {
            self.missing_argument(cx, head.as_str(), "a part", span);
            return None;
        };
        let part = self.joint_member(cx, first, head.as_str())?;
        // `facing:` and `from:` may be left out: the selector then names the
        // part against itself (grammar, "Faces to cut").
        let other = match selector {
            Selector::End => parsed.get("facing"),
            Selector::Side => parsed.get("from"),
            Selector::Crossing => parsed.positional.get(1).copied(),
        };
        let other = match (selector, other) {
            (_, Some(other_expr)) => self.joint_member(cx, other_expr, head.as_str())?,
            (Selector::End | Selector::Side, None) => part,
            (Selector::Crossing, None) => {
                self.missing_argument(cx, head.as_str(), "a second part", span);
                return None;
            }
        };
        let normal = match parsed.get("normal") {
            Some(e) => match self.part_axis(cx, e, "normal") {
                Some(PartAxis::Length) => {
                    self.error_help(
                        Code::TypeMismatch,
                        "'normal' needs width or thickness, found length".to_owned(),
                        cx.file,
                        e.span,
                        "name the face across the part's width (normal: width) or its thickness (normal: thickness); the ends are cut with end(...)",
                    );
                    return None;
                }
                axis => axis,
            },
            None => None,
        };
        if selector == Selector::Side && normal.is_none() && parsed.get("normal").is_none() {
            self.missing_argument(cx, head.as_str(), "normal:", span);
        }
        let opposite = parsed
            .get("opposite")
            .is_some_and(|e| self.bool_literal(cx, e, "opposite"));
        let flip = parsed
            .get("flip")
            .is_some_and(|e| self.bool_literal(cx, e, "flip"));
        Some(SelectorDef {
            selector,
            part,
            other,
            normal,
            opposite,
            flip,
            span,
        })
    }

    fn lower_cut(&mut self, cx: &Cx, _kind: KindId, cut: &ast::Cut) -> Option<CutDef> {
        if cut.selector.head.as_str() == super::hardware::ROW {
            self.error_help(
                Code::TypeMismatch,
                "a joint kind can't cut a row of holes with row(...)",
                cx.file,
                cut.selector.head.span(),
                "give the joint a member for what goes in the holes, with count: and spaced:, and drill crossing(role, member): one hole for every copy",
            );
            return None;
        }
        let selector = self.lower_selector(
            cx,
            &cut.selector.head,
            &cut.selector.args,
            cut.selector.span,
        )?;
        let operations = self.lower_operations(cx, selector.selector, &cut.operations);
        let name = match &cut.name {
            Some(name) => Some(CutName::new(name.as_str())?),
            None => {
                self.warn_unnamed_cut(cx, cut);
                None
            }
        };
        Some(CutDef {
            name,
            selector,
            operations,
            span: cut.span,
        })
    }

    /// `error[duplicate-name]` for two cuts of one kind with the same name
    /// (grammar, "Named cuts"): a cut's name is how messages and patches
    /// tell its cuts apart, so each is written once per kind.
    fn check_cut_names(&mut self, cx: &Cx, entries: &[ast::KindEntry]) {
        let mut seen: BTreeMap<&str, Span> = BTreeMap::new();
        for entry in entries {
            let ast::KindEntry::Cut(cut) = entry else {
                continue;
            };
            let Some(name) = &cut.name else {
                continue;
            };
            match seen.get(name.as_str()) {
                Some(first) => {
                    let word = name.as_str();
                    self.report(
                        Diagnostic::error(
                            Code::DuplicateName,
                            format!("'{word}' is declared twice"),
                            Label::new(cx.file, name.span()),
                        )
                        .with_label(Label::with_message(cx.file, *first, "first written here"))
                        .with_help(format!(
                            "give each cut of a kind its own name, as in {word} and mating_{word}"
                        )),
                    );
                }
                None => {
                    seen.insert(name.as_str(), name.span());
                }
            }
        }
    }

    /// `warning[unnamed-cut]` for a joint or hardware kind's `cut` with no
    /// name (grammar, "Named cuts"). Kinds are lowered once each, used or
    /// not, so every unnamed cut is reported once, at its kind.
    pub(super) fn warn_unnamed_cut(&mut self, cx: &Cx, cut: &ast::Cut) {
        let mut nouns = cut
            .operations
            .iter()
            .filter_map(|call| CutOperation::from_word(call.head.as_str()))
            .map(CutOperation::unnamed_noun);
        let first = nouns.next().unwrap_or("cut");
        let noun = if nouns.all(|noun| noun == first) {
            first
        } else {
            "cut"
        };
        let head = cut.selector.head.as_str();
        self.report(
            Diagnostic::warning(
                Code::UnnamedCut,
                format!("this cut has no name, so messages will call it \"a {noun}\""),
                Label::new(cx.file, cut.selector.span),
            )
            .with_help(format!(
                "name it after what it is, as in cut trench: {head}(...) {{ ... }}"
            )),
        );
    }

    /// The operations of a `cut` block, each checked against the selector
    /// it is written under.
    pub(super) fn lower_operations(
        &mut self,
        cx: &Cx,
        selector: stave_schema::operations::Selector,
        calls: &[ast::Call],
    ) -> Vec<OperationDef> {
        let mut operations = Vec::new();
        for call in calls {
            let head = call.head.as_str();
            let Some(op) = CutOperation::from_word(head) else {
                self.unknown(Code::UnknownName, format!("unknown name '{head}'"), cx.file, call.head.span(), head, CutOperation::ALL.map(CutOperation::word), "a cut operation is one of notch, slot, pocket, tenon, spigot, round, drill, dovetail_socket, dovetail_tail, outline, diagonal");
                continue;
            };
            let spec = cut_operation(op);
            if op.is_edge_profile() {
                self.error_help(
                    Code::TypeMismatch,
                    format!("'{head}' shapes a part's edge, so it can't be cut on {}", selector.word()),
                    cx.file,
                    call.span,
                    format!("write it in the part's edges:, as in edges: {head}(3mm, on: edge(top, front))"),
                );
                continue;
            }
            if !spec.selectors.contains(&selector) {
                let allowed: Vec<&str> = spec.selectors.iter().map(|s| s.word()).collect();
                self.error_help(
                    Code::TypeMismatch,
                    format!("'{head}' can't be cut on {}", selector.word()),
                    cx.file,
                    call.span,
                    format!("'{head}' is cut on {}", allowed.join(" or ")),
                );
                continue;
            }
            let names: Vec<&str> = spec.arguments.iter().map(|a| a.name).collect();
            // An argument the operation no longer takes says what to write
            // instead.
            let mut kept = Vec::with_capacity(call.args.len());
            for arg in &call.args {
                match arg
                    .name
                    .as_ref()
                    .and_then(|name| retired_argument(op, name.as_str()).zip(Some(name)))
                {
                    Some((retired, name)) => self.error_help(
                        Code::UnknownField,
                        format!("'{head}' has no field '{}'", retired.name),
                        cx.file,
                        name.span(),
                        retired.help,
                    ),
                    None => kept.push(arg),
                }
            }
            let parsed = self.split_args(cx, head, kept, &names, 0);
            let mut args = Vec::new();
            let mut marks = Vec::new();
            for argument in spec.arguments {
                let Some(value) = parsed.get(argument.name) else {
                    if argument.default == ArgumentDefault::Required {
                        self.missing_argument(cx, head, &format!("{}:", argument.name), call.span);
                    }
                    continue;
                };
                if argument.name == "marks" {
                    marks = self.lower_marks(cx, value);
                    continue;
                }
                let lowered = self.value(
                    cx,
                    value,
                    &Expect::value_type(argument.ty).named(argument.name),
                );
                args.push(Setting {
                    name: argument.name.to_owned(),
                    name_span: value.span,
                    value: lowered,
                });
            }
            operations.push(OperationDef {
                operation: op,
                args,
                marks,
                span: call.span,
            });
        }
        operations
    }

    fn lower_marks(&mut self, cx: &Cx, value: &ast::Expr) -> Vec<(RExpr, RExpr)> {
        let mut marks = Vec::new();
        let ExprKind::List(items) = &value.kind else {
            self.error_help(
                Code::TypeMismatch,
                "'marks' needs a list of mark(along:, across:)",
                cx.file,
                value.span,
                "write marks: [mark(along: 0mm, across: 0mm), ...]",
            );
            return marks;
        };
        for item in items {
            let ExprKind::Call { callee, args } = &item.kind else {
                self.error(
                    Code::TypeMismatch,
                    "'marks' needs mark(along:, across:)",
                    cx.file,
                    item.span,
                );
                continue;
            };
            if callee_text(callee) != MARK {
                self.unknown(
                    Code::UnknownName,
                    format!("unknown name '{}'", callee_text(callee)),
                    cx.file,
                    callee.span,
                    &callee_text(callee),
                    [MARK],
                    "an outline's marks are mark(along:, across:)",
                );
                continue;
            }
            let parsed = self.split_args(cx, MARK, args, &["along", "across"], 0);
            let (Some(along), Some(across)) = (parsed.get("along"), parsed.get("across")) else {
                self.missing_argument(cx, MARK, "along: and across:", item.span);
                continue;
            };
            let along = self.value(cx, along, &Expect::ty(Ty::Length).named("along"));
            let across = self.value(cx, across, &Expect::ty(Ty::Length).named("across"));
            marks.push((along, across));
        }
        marks
    }

    fn lower_made_part(
        &mut self,
        cx: &Cx,
        kind: KindId,
        made: &ast::MadePart,
        span: Span,
    ) -> MadePartDef {
        let what = format!("made part '{}'", made.name.as_str());
        let (body, place) = self.lower_part_body(cx, &made.stock, &made.body, &what);
        let mut terms = Vec::new();
        match place {
            None => self.error_help(
                Code::MissingField,
                format!("{what} is missing 'place'"),
                cx.file,
                made.body.span,
                "a made part is placed relative to the joint's parts",
            ),
            Some(place) => {
                for call in &place.terms {
                    if let Some(term) = self.lower_made_term(cx, kind, call) {
                        terms.push(term);
                    }
                }
                if !terms
                    .iter()
                    .any(|t| matches!(t.kind, MadeTermKind::Orient { .. }))
                {
                    self.error_help(
                        Code::MissingField,
                        format!("{what} is missing 'orient'"),
                        cx.file,
                        place.span,
                        "write orient(length: <axis>, width: <axis>, of: <role>)",
                    );
                }
            }
        }
        MadePartDef {
            name: made.name.as_str().to_owned(),
            name_span: made.name.span(),
            body: Box::new(body),
            place: terms,
            span,
        }
    }

    fn lower_made_term(&mut self, cx: &Cx, _kind: KindId, call: &ast::Call) -> Option<MadeTerm> {
        let head = call.head.as_str();
        let Some(term) = MadePartTerm::from_word(head) else {
            self.unknown(
                Code::UnknownName,
                format!("unknown name '{head}'"),
                cx.file,
                call.head.span(),
                head,
                MadePartTerm::ALL.map(MadePartTerm::word),
                "a made part is placed with orient, center, against or flush",
            );
            return None;
        };
        let selector_of = |this: &mut Self, expr: &ast::Expr| -> Option<SelectorDef> {
            let ExprKind::Call { callee, args } = &expr.kind else {
                this.error(
                    Code::TypeMismatch,
                    format!("'{head}' needs a face selector"),
                    cx.file,
                    expr.span,
                );
                return None;
            };
            let ExprKind::Name(ident) = &callee.kind else {
                this.error(
                    Code::TypeMismatch,
                    format!("'{head}' needs a face selector"),
                    cx.file,
                    expr.span,
                );
                return None;
            };
            this.lower_selector(cx, ident, args, expr.span)
        };
        let kind = match term {
            MadePartTerm::Orient => {
                let parsed = self.split_args(cx, head, &call.args, &["length", "width", "of"], 0);
                let (Some(length), Some(width), Some(of)) =
                    (parsed.get("length"), parsed.get("width"), parsed.get("of"))
                else {
                    self.missing_argument(cx, head, "length:, width: and of:", call.span);
                    return None;
                };
                MadeTermKind::Orient {
                    length: self.part_axis(cx, length, "length")?,
                    width: self.part_axis(cx, width, "width")?,
                    of: self.role_index(cx, of, "of")?,
                }
            }
            MadePartTerm::Center => {
                let parsed = self.split_args(cx, head, &call.args, &[], 2);
                let selector = selector_of(self, parsed.positional.first()?)?;
                let axis = match parsed.positional.get(1) {
                    Some(axis) => Some(self.part_axis(cx, axis, "axis")?),
                    None => None,
                };
                MadeTermKind::Center { selector, axis }
            }
            MadePartTerm::Against | MadePartTerm::Flush => {
                let extra = if term == MadePartTerm::Against {
                    "gap"
                } else {
                    "inset"
                };
                let parsed = self.split_args(cx, head, &call.args, &[extra], 1);
                let selector = selector_of(self, parsed.positional.first()?)?;
                let offset = parsed
                    .get(extra)
                    .map(|e| self.value(cx, e, &Expect::ty(Ty::Length).named(extra)));
                if term == MadePartTerm::Against {
                    MadeTermKind::Against {
                        selector,
                        gap: offset,
                    }
                } else {
                    MadeTermKind::Flush {
                        selector,
                        inset: offset,
                    }
                }
            }
        };
        Some(MadeTerm {
            kind,
            span: call.span,
        })
    }

    // -----------------------------------------------------------------------
    // Recipes
    // -----------------------------------------------------------------------

    pub(super) fn lower_recipe(
        &mut self,
        cx: &Cx,
        def: DefId,
        syntax: &ast::Recipe,
        origin: Origin<'_>,
    ) -> RecipeDef {
        let what = format!("recipe '{}'", syntax.name.as_str());
        let op = match Operation::from_word(syntax.operation.as_str()) {
            Some(op) => op,
            None => {
                self.unknown(Code::UnknownName, format!("unknown name '{}'", syntax.operation.as_str()), cx.file, syntax.operation.span(), syntax.operation.as_str(), Operation::all().map(Operation::word), "a recipe is for one of notch, slot, pocket, tenon, spigot, round, drill, dovetail_socket, dovetail_tail, outline, diagonal, chamfer, roundover, turning");
                Operation::Cut(CutOperation::Notch)
            }
        };
        let spec = operation(op);
        let field_list: Vec<&ast::Field> = syntax
            .body
            .entries
            .iter()
            .filter_map(|e| match e {
                ast::RecipeEntry::Field(f) => Some(f),
                ast::RecipeEntry::Mark(_) | ast::RecipeEntry::Cut(_) => None,
            })
            .collect();
        let found = self.field_table(
            cx,
            field_list,
            fields::RECIPE_FIELDS,
            &what,
            syntax.body.span,
        );
        // Without a `tools:` list (already reported missing or misspelt),
        // a step's `with:` isn't checked against it.
        let tools_given = found.contains_key("tools");
        let tools = found
            .get("tools")
            .map(|f| kind_ids(&self.field_value(cx, f, fields::RECIPE_FIELDS)))
            .unwrap_or_default();
        let rank = match found
            .get("rank")
            .map(|f| self.field_value(cx, f, fields::RECIPE_FIELDS))
        {
            Some(RExpr {
                kind: RExprKind::Number(n),
                ..
            }) if n.is_integer() => {
                u32::try_from(n.numerator()).unwrap_or(stave_schema::DEFAULT_RECIPE_RANK)
            }
            Some(other) if other.kind != RExprKind::Error => {
                self.error(
                    Code::TypeMismatch,
                    "'rank' needs a whole number",
                    cx.file,
                    other.span,
                );
                stave_schema::DEFAULT_RECIPE_RANK
            }
            _ => stave_schema::DEFAULT_RECIPE_RANK,
        };
        let mut needs = Vec::new();
        if let Some(field) = found.get("needs") {
            for need in elements(&field.value) {
                if let Some(lowered) = self.lower_need(cx, need) {
                    needs.push(lowered);
                }
            }
        }
        let check = found
            .get("check")
            .and_then(|f| self.plain_text(cx, &f.value, "check"));
        let mut mark = Vec::new();
        let mut cut = Vec::new();
        let mut has_cut = false;
        for entry in &syntax.body.entries {
            match entry {
                ast::RecipeEntry::Mark(block) => {
                    for step in &block.steps {
                        if let Some(lowered) = self.lower_mark_step(
                            cx,
                            step,
                            spec,
                            tools_given.then_some(tools.as_slice()),
                        ) {
                            mark.push(lowered);
                        }
                    }
                }
                ast::RecipeEntry::Cut(block) => {
                    has_cut = true;
                    for step in &block.steps {
                        if let Some(lowered) = self.lower_cut_step(
                            cx,
                            step,
                            spec,
                            tools_given.then_some(tools.as_slice()),
                        ) {
                            cut.push(lowered);
                        }
                    }
                }
                ast::RecipeEntry::Field(_) => {}
            }
        }
        if !has_cut {
            self.error_help(
                Code::MissingField,
                format!("{what} is missing 'cut'"),
                cx.file,
                syntax.body.span,
                "add cut { ... } with the cutting steps",
            );
        }
        RecipeDef {
            decl: self.decl(def, &syntax.name, origin),
            operation: op,
            tools,
            rank,
            needs,
            mark,
            cut,
            check,
        }
    }

    fn lower_need(&mut self, cx: &Cx, need: &ast::Expr) -> Option<Need> {
        let (head, args, span) = match &need.kind {
            ExprKind::Name(ident) => (ident.as_str().to_owned(), &[][..], ident.span()),
            ExprKind::Call { callee, args } => (callee_text(callee), args.as_slice(), callee.span),
            ExprKind::Quantity(_)
            | ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::Bool(_)
            | ExprKind::Fit
            | ExprKind::Member { .. }
            | ExprKind::Index { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::List(_)
            | ExprKind::Paren(_)
            | ExprKind::Range { .. } => {
                self.error(
                    Code::TypeMismatch,
                    "'needs' needs flat_face, square_stock, before(...) or after(...)",
                    cx.file,
                    need.span,
                );
                return None;
            }
        };
        let Some(kind) = NeedKind::from_word(&head) else {
            self.unknown(
                Code::UnknownName,
                format!("unknown name '{head}'"),
                cx.file,
                span,
                &head,
                NeedKind::ALL.map(NeedKind::word),
                "a need is flat_face, square_stock, before(operation) or after(operation)",
            );
            return None;
        };
        if !kind.takes_operation() {
            return Some(Need {
                kind,
                operation: None,
            });
        }
        let Some(ast::Arg {
            value:
                ast::Expr {
                    kind: ExprKind::Name(op_name),
                    span: op_span,
                },
            ..
        }) = args.first()
        else {
            self.missing_argument(cx, &head, "an operation", need.span);
            return None;
        };
        match Operation::from_word(op_name.as_str()) {
            Some(op) => Some(Need {
                kind,
                operation: Some(op),
            }),
            None => {
                self.unknown(
                    Code::UnknownName,
                    format!("unknown name '{}'", op_name.as_str()),
                    cx.file,
                    *op_span,
                    op_name.as_str(),
                    Operation::all().map(Operation::word),
                    "name an operation, such as before(drill)",
                );
                None
            }
        }
    }

    fn step_tool(&mut self, cx: &Cx, expr: &ast::Expr, tools: Option<&[KindId]>) -> Option<KindId> {
        let value = self.value(cx, expr, &Expect::ty(Ty::Tool).named("with"));
        let RExprKind::Symbol(Symbol::Def(DefId::Kind(id))) = value.kind else {
            return None;
        };
        if tools.is_some_and(|tools| !tools.contains(&id)) {
            self.error_help(
                Code::TypeMismatch,
                "'with' needs one of the recipe's tools",
                cx.file,
                expr.span,
                "add the tool to the recipe's tools: list",
            );
        }
        Some(id)
    }

    /// A step argument that is one word from `allowed`.
    fn word_arg<T: Copy>(
        &mut self,
        cx: &Cx,
        expr: &ast::Expr,
        allowed: &[T],
        word: fn(T) -> &'static str,
        what: &str,
    ) -> Option<T> {
        let ExprKind::Name(ident) = &expr.kind else {
            self.error(
                Code::TypeMismatch,
                format!("'{what}' needs a word, found an expression"),
                cx.file,
                expr.span,
            );
            return None;
        };
        if let Some(found) = allowed
            .iter()
            .copied()
            .find(|item| word(*item) == ident.as_str())
        {
            return Some(found);
        }
        let words: Vec<&str> = allowed.iter().copied().map(word).collect();
        self.unknown(
            Code::UnknownName,
            format!("unknown name '{}'", ident.as_str()),
            cx.file,
            ident.span(),
            ident.as_str(),
            words.iter().copied(),
            format!("here '{what}' is one of: {}", words.join(", ")),
        );
        None
    }

    /// `passes:`: a whole number of at least one, written out.
    fn passes_arg(&mut self, cx: &Cx, expr: &ast::Expr) -> Option<NonZeroU32> {
        let value = self.value(cx, expr, &Expect::ty(Ty::Count).named("passes"));
        // A negative literal is lowered as `-` applied to a number.
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "takes only the values it names: any other, a new variant included, is a type error"
        )]
        let (found, passes) = match &value.kind {
            RExprKind::Number(number) => (
                number.to_string(),
                number
                    .is_integer()
                    .then(|| u32::try_from(number.numerator()).ok())
                    .flatten()
                    .and_then(NonZeroU32::new),
            ),
            RExprKind::Unary {
                op: ast::UnaryOp::Neg,
                operand,
            } if matches!(operand.kind, RExprKind::Number(_)) => {
                #[expect(
                    clippy::wildcard_enum_match_arm,
                    reason = "picks out only the variants it names: any other, a new one included, is not one of them"
                )]
                let found = match &operand.kind {
                    RExprKind::Number(number) => format!("-{number}"),
                    _ => String::new(),
                };
                (found, None)
            }
            // Reported where it was lowered.
            RExprKind::Error => return None,
            _ => {
                self.error_help(
                    Code::TypeMismatch,
                    "'passes' needs a number written out",
                    cx.file,
                    expr.span,
                    "write how many passes to take, such as passes: 2",
                );
                return None;
            }
        };
        if passes.is_none() {
            self.error_help(
                Code::OutOfRange,
                format!("'passes' must be a whole number of at least 1, found {found}"),
                cx.file,
                expr.span,
                "write how many passes to take, such as passes: 2",
            );
        }
        passes
    }

    fn note_arg(&mut self, cx: &Cx, expr: Option<&ast::Expr>) -> Option<String> {
        expr.and_then(|e| self.plain_text(cx, e, "note"))
    }

    fn lower_mark_step(
        &mut self,
        cx: &Cx,
        call: &ast::Call,
        spec: &stave_schema::operations::OperationSpec,
        tools: Option<&[KindId]>,
    ) -> Option<MarkStepDef> {
        let head = call.head.as_str();
        let Some(step) = MarkingStep::from_word(head) else {
            self.unknown(Code::UnknownName, format!("unknown name '{head}'"), cx.file, call.head.span(), head, MarkingStep::ALL.map(MarkingStep::word), "a marking step is knife, pencil, line, gauge, square_around, centre_mark, transfer, hatch or template");
            return None;
        };
        let named: Vec<&str> = step
            .arguments()
            .iter()
            .map(|argument| argument.name)
            .chain(COMMON_STEP_ARGUMENTS)
            .collect();
        let parsed = self.split_args(cx, head, &call.args, &named, 1);
        let Some(target) = parsed.positional.first() else {
            self.missing_argument(cx, head, "a feature", call.span);
            return None;
        };
        let target = if step.takes_measurement() {
            StepTarget::Measurement(self.word_arg(
                cx,
                target,
                spec.measurements,
                Measurement::word,
                head,
            )?)
        } else {
            StepTarget::Feature(self.word_arg(cx, target, spec.features, Feature::word, head)?)
        };
        let on = match parsed.get("on") {
            Some(e) => self.word_arg(cx, e, spec.features, Feature::word, "on"),
            None => {
                if step.takes_measurement() {
                    self.missing_argument(cx, head, "on:", call.span);
                }
                None
            }
        };
        let with = parsed
            .get("with")
            .and_then(|e| self.step_tool(cx, e, tools));
        let note = self.note_arg(cx, parsed.get("note"));
        Some(MarkStepDef {
            step,
            target,
            on,
            with,
            note,
            span: call.span,
        })
    }

    fn lower_cut_step(
        &mut self,
        cx: &Cx,
        call: &ast::Call,
        spec: &stave_schema::operations::OperationSpec,
        tools: Option<&[KindId]>,
    ) -> Option<CutStepDef> {
        let head = call.head.as_str();
        let Some(step) = CuttingStep::from_word(head) else {
            self.unknown(
                Code::UnknownName,
                format!("unknown name '{head}'"),
                cx.file,
                call.head.span(),
                head,
                CuttingStep::ALL.map(CuttingStep::word),
                "a cutting step is saw, chop, pare, plane, rout, bore, turn or shave",
            );
            return None;
        };
        let named: Vec<&str> = CUTTING_STEP_ARGUMENTS
            .into_iter()
            .filter(|name| step.takes_diameter() || *name != "diameter")
            .chain(COMMON_STEP_ARGUMENTS)
            .collect();
        let parsed = self.split_args(cx, head, &call.args, &named, 1);
        let Some(feature) = parsed.positional.first() else {
            self.missing_argument(cx, head, "a feature", call.span);
            return None;
        };
        let feature = self.word_arg(cx, feature, spec.features, Feature::word, head)?;
        let both: Vec<StepTarget> = spec
            .measurements
            .iter()
            .copied()
            .map(StepTarget::Measurement)
            .chain(spec.features.iter().copied().map(StepTarget::Feature))
            .collect();
        let to = parsed
            .get("to")
            .and_then(|e| self.word_arg(cx, e, &both, StepTarget::word, "to"));
        let guide = parsed
            .get("guide")
            .and_then(|e| self.word_arg(cx, e, spec.features, Feature::word, "guide"));
        let diameter = parsed
            .get("diameter")
            .and_then(|e| self.word_arg(cx, e, spec.measurements, Measurement::word, "diameter"));
        let passes = parsed.get("passes").and_then(|e| self.passes_arg(cx, e));
        let with = parsed
            .get("with")
            .and_then(|e| self.step_tool(cx, e, tools));
        let note = self.note_arg(cx, parsed.get("note"));
        Some(CutStepDef {
            step,
            feature,
            to,
            passes,
            guide,
            diameter,
            with,
            note,
            span: call.span,
        })
    }
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
)]
fn callee_text(callee: &ast::Expr) -> String {
    match &callee.kind {
        ExprKind::Name(ident) => ident.as_str().to_owned(),
        _ => "this".into(),
    }
}

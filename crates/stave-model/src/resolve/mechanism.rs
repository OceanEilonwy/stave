//! Lowering a hardware kind's members with bodies, their shapes, and its
//! `mechanism` (grammar, "Shapes" and "Mechanisms").
//!
//! Everything here is written in the kind's own frame (grammar, "The kind's
//! frame"): measurements within the hardware, never positions in a design.
//! Values stay expressions over the kind's params; the solver evaluates
//! them for each piece of hardware.

use std::collections::BTreeMap;

use stave_lang::ast::{self, BaseKind, ExprKind, Span};
use stave_lang::units::ExactLength;
use stave_lang::{Code, Diagnostic, Label};
use stave_schema::functions::{POINT, POINT_ARGUMENTS, ShapeFunction};
use stave_schema::words::WordSet;

use super::expr::Expect;
use super::{Cx, Resolver};
use crate::defs::{
    AdjustDef, BodyDef, BodyId, BodyName, Carrier, ClampDef, ConstraintDef, CouplingDef,
    CylinderTilt, DriveDef, EntryPair, EntrySide, FrameAxis, KindPoint, LayoutDef, MechanismDef,
    NamedShape, RExpr, RExprKind, RangeDef, RoleDef, RoleIndex, ShapeDef, StopDef,
};
use crate::nonempty::NonEmpty;
use crate::types::Ty;

/// The mechanism field that names the direction its pins run.
const PINS: &str = "pins";

/// The fields of a joint's member that say where it goes.
const LAYOUT_FIELDS: [&str; 3] = ["on", "count", "spaced"];

/// A joint member's `on:`, `count:` and `spaced:` while they are read.
#[derive(Default)]
struct Layout {
    on: Option<RExpr>,
    count: Option<RExpr>,
    spaced: Option<RExpr>,
}

/// The named arguments of a call, by name.
type Arguments<'a> = BTreeMap<&'a str, &'a ast::Arg>;

/// `turn` and `travel`: the two drives.
const DRIVES: [&str; 2] = ["turn", "travel"];

/// A `with:` before the list of bodies is final.
#[derive(Clone, Copy)]
enum Pending<'a> {
    Role(RoleIndex),
    Body(&'a str),
    Mechanism,
}

impl Resolver<'_> {
    /// A hardware kind's members with a body, in source order; `None`
    /// (reported) when one of them doesn't lower.
    pub(super) fn lower_bodies(
        &mut self,
        cx: &Cx,
        base_kind: BaseKind,
        roles: &[RoleDef],
        written: &[&ast::Documented<ast::KindMember>],
    ) -> Option<Vec<BodyDef>> {
        let joint = base_kind == BaseKind::Joint;
        // A member written twice is reported where the members are listed;
        // only its first is a body.
        let mut unique: Vec<&str> = Vec::new();
        let first_written: Vec<&ast::Documented<ast::KindMember>> = written
            .iter()
            .copied()
            .filter(|member| {
                let word = member.node.name.as_str();
                let first = !unique.contains(&word);
                unique.push(word);
                first
            })
            .collect();
        let with_body: Vec<(&ast::Documented<ast::KindMember>, &ast::MemberBody)> = first_written
            .iter()
            .filter_map(|member| member.node.body.as_ref().map(|body| (*member, body)))
            .collect();
        let names: Vec<&str> = with_body
            .iter()
            .map(|(member, _)| member.node.name.as_str())
            .collect();
        let mut bodies: Vec<(BodyDef, Option<&str>)> = Vec::new();
        for (member, body) in &with_body {
            let member_name = member.node.name.as_str();
            let Some(name) = BodyName::new(member_name) else {
                continue;
            };
            let mut with = Pending::Mechanism;
            let mut with_span: Option<Span> = None;
            let mut shapes: Vec<NamedShape> = Vec::new();
            let mut shape_names: BTreeMap<Option<String>, Span> = BTreeMap::new();
            let mut mount = Vec::new();
            let mut mount_span: Option<Span> = None;
            let mut layout = Layout::default();
            let item = self.member_item(cx, joint, member);
            for entry in &body.entries {
                match entry {
                    ast::MemberEntry::Mount(block) if joint => {
                        self.error_help(
                            Code::UnknownField,
                            format!("member '{member_name}' has no field 'mount'"),
                            cx.file,
                            block.span,
                            "a joint's member is placed on: the patch where two of its parts touch, with count: and spaced:",
                        );
                    }
                    ast::MemberEntry::Field(field)
                        if joint && LAYOUT_FIELDS.contains(&field.key.as_str()) =>
                    {
                        self.layout_field(cx, &mut layout, field);
                    }
                    ast::MemberEntry::Field(field)
                        if !joint && LAYOUT_FIELDS.contains(&field.key.as_str()) =>
                    {
                        self.error_help(
                            Code::UnknownField,
                            format!(
                                "member '{member_name}' has no field '{}'",
                                field.key.as_str()
                            ),
                            cx.file,
                            field.key.span(),
                            "only a joint's members are laid out on a contact patch; a hardware member is carried with: a role or another member",
                        );
                    }
                    ast::MemberEntry::Field(field) if joint && field.key.as_str() == "with" => {
                        self.error_help(
                            Code::UnknownField,
                            format!("member '{member_name}' has no field 'with'"),
                            cx.file,
                            field.key.span(),
                            "a joint's member sits where its parts touch: write on: contact(piece, into)",
                        );
                    }
                    ast::MemberEntry::Mount(block) => {
                        if let Some(first) = mount_span {
                            self.duplicate("mount", block.span, first, cx);
                            continue;
                        }
                        mount_span = Some(block.span);
                        mount = self.member_mount(cx, roles, block);
                    }
                    ast::MemberEntry::Field(field) if field.key.as_str() == "with" => {
                        if let Some(first) = with_span {
                            self.duplicate("with", field.key.span(), first, cx);
                            continue;
                        }
                        with_span = Some(field.key.span());
                        if let Some(carrier) = self.carrier(cx, roles, &names, member_name, field) {
                            with = carrier;
                        }
                    }
                    ast::MemberEntry::Field(field) => {
                        let key = field.key.as_str();
                        if joint {
                            self.unknown(
                                Code::UnknownField,
                                format!("member '{member_name}' has no field '{key}'"),
                                cx.file,
                                field.key.span(),
                                key,
                                ["on", "count", "spaced", "shape"],
                                "a joint's member holds where it goes (on:, count:, spaced:) and its shapes (shape: cylinder(...))",
                            );
                        } else {
                            self.unknown(
                                Code::UnknownField,
                                format!("member '{member_name}' has no field '{key}'"),
                                cx.file,
                                field.key.span(),
                                key,
                                ["with", "shape", "mount"],
                                "a member's body holds its shapes (shape: box(...)), what carries it (with: ...) and its own mount { }",
                            );
                        }
                    }
                    ast::MemberEntry::Shape(shape) => {
                        let shape_name = shape.name.as_ref().map(|name| name.as_str().to_owned());
                        let at = shape.name.as_ref().map_or(shape.span, ast::Name::span);
                        if let Some(first) = shape_names.get(&shape_name) {
                            let word = shape_name.as_deref().unwrap_or("shape");
                            self.duplicate(word, at, *first, cx);
                            continue;
                        }
                        shape_names.insert(shape_name.clone(), at);
                        if let Some(solid) = self.shape(cx, &shape.value) {
                            shapes.push(NamedShape {
                                name: shape_name,
                                span: shape.span,
                                shape: solid,
                            });
                        }
                    }
                }
            }
            let has_shape = body
                .entries
                .iter()
                .any(|entry| matches!(entry, ast::MemberEntry::Shape(_)));
            if !has_shape {
                self.error_help(
                    Code::MissingField,
                    format!("member '{member_name}' is missing 'shape'"),
                    cx.file,
                    member.node.name.span(),
                    "give it what it is made of: shape: box(along: ..., across: ..., depth: ...)",
                );
            }
            // A shape that failed to resolve was reported; the member is
            // left out so nothing downstream sees half of it.
            let Some(shapes) = NonEmpty::new(shapes) else {
                continue;
            };
            let (carrier, rides_on) = if joint {
                let Some(on) = layout.on else {
                    self.error_help(
                        Code::MissingField,
                        format!("member '{member_name}' is missing 'on'"),
                        cx.file,
                        member.node.name.span(),
                        "say where it goes: on: contact(piece, into), the patch where the parts touch",
                    );
                    continue;
                };
                let carrier = Carrier::Contact(Box::new(LayoutDef {
                    span: on.span,
                    on,
                    count: layout.count,
                    spaced: layout.spaced,
                }));
                (carrier, None)
            } else {
                match with {
                    Pending::Role(role) => (Carrier::Role(role), None),
                    Pending::Body(other) => (Carrier::Mechanism, Some(other)),
                    Pending::Mechanism => (Carrier::Mechanism, None),
                }
            };
            bodies.push((
                BodyDef {
                    name,
                    span: member.node.name.span(),
                    docs: member.docs.clone(),
                    with: carrier,
                    item,
                    shapes,
                    mount,
                },
                rides_on,
            ));
        }
        // `with: other` is looked up in the final list. A member that was
        // left out has been reported, and whatever rides on it is left out
        // too, so no id points at the wrong member.
        let names: Vec<String> = bodies
            .iter()
            .map(|(body, _)| body.name.as_str().to_owned())
            .collect();
        let mut lowered = Vec::new();
        for (mut body, rides_on) in bodies {
            if let Some(other) = rides_on {
                match names
                    .iter()
                    .position(|name| name == other)
                    .and_then(BodyId::new)
                {
                    Some(id) => body.with = Carrier::Body(id),
                    None => continue,
                }
            }
            lowered.push(body);
        }
        // Ids would shift past a member left out; its errors are reported.
        (lowered.len() == with_body.len()).then_some(lowered)
    }

    /// What a member is, after `:`: a fastener kind value with its
    /// settings, in a joint kind only.
    fn member_item(
        &mut self,
        cx: &Cx,
        joint: bool,
        member: &ast::Documented<ast::KindMember>,
    ) -> Option<RExpr> {
        let item = member.node.item.as_ref()?;
        if !joint {
            self.error_help(
                Code::UnknownField,
                format!(
                    "member '{}' of a hardware kind can't name a fastener",
                    member.node.name.as_str()
                ),
                cx.file,
                item.span,
                "only a joint's members say what they are (member name : kind(...)); list a hardware kind's fasteners with uses:",
            );
            return None;
        }
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
        )]
        let (callee, args) = match &item.kind {
            ExprKind::Call { callee, args } => (&**callee, args.as_slice()),
            _ => (item, &[][..]),
        };
        let kind_expr = self.value(
            cx,
            callee,
            &Expect::ty(Ty::from_type_name(ast::TypeName::FastenerKind)).named("member"),
        );
        let Ty::Kind(kind_ty) = kind_expr.ty.clone() else {
            return None;
        };
        let settings = self.kind_settings(cx, kind_ty, args, &[]);
        Some(RExpr {
            kind: RExprKind::KindValue {
                kind: Box::new(kind_expr),
                settings,
            },
            ty: Ty::Kind(kind_ty),
            span: item.span,
        })
    }

    /// `on:`, `count:` or `spaced:` of a joint's member, each at most once.
    fn layout_field(&mut self, cx: &Cx, layout: &mut Layout, field: &ast::Field) {
        let key = field.key.as_str();
        let (written, expect) = match key {
            "on" => (&layout.on, Expect::ty(Ty::Patch).named("on")),
            "count" => (&layout.count, Expect::ty(Ty::Count).named("count")),
            _ => (
                &layout.spaced,
                Expect::ty(Ty::Spacing)
                    .with_words(&[WordSet::Spacing])
                    .named("spaced"),
            ),
        };
        if let Some(first) = written.as_ref().map(|value| value.span) {
            self.duplicate(key, field.key.span(), first, cx);
            return;
        }
        let value = Some(self.value(cx, &field.value, &expect));
        match key {
            "on" => layout.on = value,
            "count" => layout.count = value,
            _ => layout.spaced = value,
        }
    }

    /// `with: piece` or `with: plate`.
    fn carrier<'a>(
        &mut self,
        cx: &Cx,
        roles: &[RoleDef],
        names: &[&'a str],
        member: &str,
        field: &ast::Field,
    ) -> Option<Pending<'a>> {
        let ExprKind::Name(ident) = &field.value.kind else {
            self.error_help(
                Code::TypeMismatch,
                format!("'with' of member '{member}' needs a role or a member"),
                cx.file,
                field.value.span,
                "name the role whose part it moves with (with: piece) or another member it rides on",
            );
            return None;
        };
        let word = ident.as_str();
        if let Some(index) = roles.iter().position(|role| role.name == word) {
            return RoleIndex::new(index).map(Pending::Role);
        }
        if word == member {
            self.error_help(
                Code::TypeMismatch,
                format!("member '{member}' can't ride on itself"),
                cx.file,
                field.value.span,
                "name a role (with: piece) or another member",
            );
            return None;
        }
        if let Some(name) = names.iter().find(|name| **name == word) {
            return Some(Pending::Body(name));
        }
        let candidates: Vec<&str> = roles
            .iter()
            .map(|role| role.name.as_str())
            .chain(names.iter().copied())
            .collect();
        self.unknown(
            Code::UnknownName,
            format!("unknown name '{word}'"),
            cx.file,
            ident.span(),
            word,
            candidates.iter().copied(),
            "name one of the kind's roles or a member with a body",
        );
        None
    }

    pub(super) fn duplicate(&mut self, word: &str, at: Span, first: Span, cx: &Cx) {
        self.report(
            Diagnostic::error(
                Code::DuplicateName,
                format!("'{word}' is declared twice"),
                Label::new(cx.file, at),
            )
            .with_label(Label::with_message(cx.file, first, "first written here"))
            .with_help(format!("write '{word}' once; remove one of them")),
        );
    }

    /// The named arguments of a call to `function`, each one it takes and
    /// at most once. `None` (reported) when one isn't.
    fn arguments<'a>(
        &mut self,
        cx: &Cx,
        function: &str,
        args: &'a [ast::Arg],
        allowed: &[&str],
    ) -> Option<Arguments<'a>> {
        let mut found: Arguments<'a> = BTreeMap::new();
        let mut ok = true;
        for arg in args {
            let Some(name) = &arg.name else {
                self.error_help(
                    Code::Syntax,
                    format!("{function}(...) takes only named arguments"),
                    cx.file,
                    arg.span,
                    format!("name each argument: {}", argument_list(allowed)),
                );
                ok = false;
                continue;
            };
            let word = name.as_str();
            if !allowed.contains(&word) {
                self.unknown(
                    Code::UnknownField,
                    format!("{function}(...) has no argument '{word}'"),
                    cx.file,
                    name.span(),
                    word,
                    allowed.iter().copied(),
                    format!("its arguments are {}", argument_list(allowed)),
                );
                ok = false;
                continue;
            }
            if let Some(first) = found.get(word) {
                self.duplicate(word, name.span(), first.span, cx);
                ok = false;
                continue;
            }
            found.insert(word, arg);
        }
        ok.then_some(found)
    }

    /// The argument `name` of `function`, or `missing-field`.
    fn required<'a>(
        &mut self,
        cx: &Cx,
        function: &str,
        found: &Arguments<'a>,
        name: &str,
        span: Span,
    ) -> Option<&'a ast::Expr> {
        match found.get(name) {
            Some(arg) => Some(&arg.value),
            None => {
                self.error_help(
                    Code::MissingField,
                    format!("{function}(...) is missing '{name}'"),
                    cx.file,
                    span,
                    format!("write {name}: ... inside the parentheses"),
                );
                None
            }
        }
    }

    /// A length in the kind's frame: a range is an error here.
    fn frame_length(&mut self, cx: &Cx, expr: &ast::Expr, what: &str) -> RExpr {
        self.value(cx, expr, &Expect::ty(Ty::Length).named(what))
    }

    /// A shape's range argument: `lo..hi` or one length.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
    )]
    fn range(&mut self, cx: &Cx, expr: &ast::Expr, what: &str) -> RangeDef {
        match &expr.kind {
            ExprKind::Range { lo, hi } => RangeDef::Between {
                low: self.frame_length(cx, lo, what),
                high: self.frame_length(cx, hi, what),
            },
            _ => RangeDef::Centred(self.frame_length(cx, expr, what)),
        }
    }

    /// `point(along:, across:, depth:)`.
    fn point(&mut self, cx: &Cx, expr: &ast::Expr, what: &str) -> Option<KindPoint> {
        let ExprKind::Call { callee, args } = &expr.kind else {
            return self.not_a_point(cx, expr, what);
        };
        let ExprKind::Name(head) = &callee.kind else {
            return self.not_a_point(cx, expr, what);
        };
        if head.as_str() != POINT {
            return self.not_a_point(cx, expr, what);
        }
        let found = self.arguments(cx, POINT, args, &POINT_ARGUMENTS)?;
        let coordinate = |resolver: &mut Self, name: &str| match found.get(name) {
            Some(arg) => resolver.frame_length(cx, &arg.value, name),
            None => zero(expr.span),
        };
        Some(KindPoint {
            along: coordinate(self, "along"),
            across: coordinate(self, "across"),
            depth: coordinate(self, "depth"),
        })
    }

    fn not_a_point<T>(&mut self, cx: &Cx, expr: &ast::Expr, what: &str) -> Option<T> {
        self.error_help(
            Code::TypeMismatch,
            format!("'{what}' needs a point"),
            cx.file,
            expr.span,
            "write point(across: ..., depth: ...) in the kind's frame",
        );
        None
    }

    /// A cylinder's `tilt:` and `toward:` (grammar, "Shapes"): both or
    /// neither, and `toward` never the direction it runs along. `Some(None)`
    /// for a straight cylinder, `None` after an error.
    fn cylinder_tilt(
        &mut self,
        cx: &Cx,
        found: &Arguments<'_>,
        axis: FrameAxis,
        span: Span,
    ) -> Option<Option<CylinderTilt>> {
        let (tilt, toward) = match (found.get("tilt"), found.get("toward")) {
            (None, None) => return Some(None),
            (Some(tilt), Some(toward)) => (tilt, toward),
            (Some(_), None) | (None, Some(_)) => {
                self.error_help(
                    Code::MissingField,
                    "a slanted cylinder needs both 'tilt' and 'toward'",
                    cx.file,
                    span,
                    "write tilt: 15deg, toward: across, or leave both out for a straight cylinder",
                );
                return None;
            }
        };
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
        )]
        let parsed = match &toward.value.kind {
            ExprKind::Name(ident) => FrameAxis::from_word(ident.as_str()),
            _ => None,
        };
        let others: Vec<&str> = FrameAxis::ALL
            .into_iter()
            .filter(|other| *other != axis)
            .map(FrameAxis::word)
            .collect();
        let help = format!("lean it toward {}", others.join(" or "));
        let Some(toward_axis) = parsed else {
            self.error_help(
                Code::TypeMismatch,
                "'toward' needs along, across or depth",
                cx.file,
                toward.value.span,
                help,
            );
            return None;
        };
        if toward_axis == axis {
            self.error_help(
                Code::TypeMismatch,
                format!(
                    "a cylinder along {} can't lean toward {}",
                    axis.word(),
                    toward_axis.word()
                ),
                cx.file,
                toward.value.span,
                help,
            );
            return None;
        }
        let angle = self.value(cx, &tilt.value, &Expect::ty(Ty::Angle).named("tilt"));
        Some(Some(CylinderTilt {
            angle,
            toward: toward_axis,
        }))
    }

    /// `box(...)`, `cylinder(...)`, `prism(...)` or `bar(...)`.
    fn shape(&mut self, cx: &Cx, expr: &ast::Expr) -> Option<ShapeDef> {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
        )]
        let shape = match &expr.kind {
            ExprKind::Call { callee, args } => match &callee.kind {
                ExprKind::Name(head) => ShapeFunction::from_word(head.as_str()).map(|s| (s, args)),
                _ => None,
            },
            _ => None,
        };
        let Some((function, args)) = shape else {
            self.error_help(
                Code::TypeMismatch,
                "'shape' needs a shape",
                cx.file,
                expr.span,
                "write box(...), cylinder(...), prism(...) or bar(...)",
            );
            return None;
        };
        let word = function.word();
        let found = self.arguments(cx, word, args, function.arguments())?;
        let span = expr.span;
        match function {
            ShapeFunction::Box => {
                let along = self.required(cx, word, &found, "along", span);
                let across = self.required(cx, word, &found, "across", span);
                let depth = self.required(cx, word, &found, "depth", span);
                let (along, across, depth) = (along?, across?, depth?);
                Some(ShapeDef::Box {
                    along: self.range(cx, along, "along"),
                    across: self.range(cx, across, "across"),
                    depth: self.range(cx, depth, "depth"),
                })
            }
            ShapeFunction::Cylinder => {
                let axis = match found.get("axis") {
                    None => FrameAxis::Depth,
                    Some(arg) => {
                        #[expect(
                            clippy::wildcard_enum_match_arm,
                            reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
                        )]
                        let parsed = match &arg.value.kind {
                            ExprKind::Name(ident) => FrameAxis::from_word(ident.as_str()),
                            _ => None,
                        };
                        let Some(axis) = parsed else {
                            self.error_help(
                                Code::TypeMismatch,
                                "'axis' needs along, across or depth",
                                cx.file,
                                arg.value.span,
                                "write axis: along, axis: across, or leave it out for depth",
                            );
                            return None;
                        };
                        axis
                    }
                };
                for other in FrameAxis::ALL.into_iter().filter(|other| *other != axis) {
                    if let Some(arg) = found.get(other.word()) {
                        self.error_help(
                            Code::TypeMismatch,
                            format!(
                                "a cylinder along '{}' has no '{}' range",
                                axis.word(),
                                other.word()
                            ),
                            cx.file,
                            arg.span,
                            format!(
                                "its diameter sets its size across; write its range as {}: ...",
                                axis.word()
                            ),
                        );
                        return None;
                    }
                }
                let diameter = self.required(cx, word, &found, "diameter", span);
                let extent = self.required(cx, word, &found, axis.word(), span);
                let (diameter, extent) = (diameter?, extent?);
                let at = match found.get("at") {
                    Some(arg) => self.point(cx, &arg.value, "at")?,
                    None => origin(span),
                };
                let tilt = self.cylinder_tilt(cx, &found, axis, span)?;
                Some(ShapeDef::Cylinder {
                    diameter: self.frame_length(cx, diameter, "diameter"),
                    extent: self.range(cx, extent, axis.word()),
                    at: Box::new(at),
                    axis,
                    tilt,
                })
            }
            ShapeFunction::Prism => {
                let points = self.required(cx, word, &found, "points", span);
                let along = self.required(cx, word, &found, "along", span);
                let (points, along) = (points?, along?);
                let ExprKind::List(items) = &points.kind else {
                    self.error_help(
                        Code::TypeMismatch,
                        "'points' needs a list of points",
                        cx.file,
                        points.span,
                        "write points: [point(...), point(...), point(...)], in order around the outline",
                    );
                    return None;
                };
                if items.len() < 3 {
                    self.error_help(
                        Code::TypeMismatch,
                        format!(
                            "a prism's outline needs at least 3 points, found {}",
                            items.len()
                        ),
                        cx.file,
                        points.span,
                        "list the corners of the outline in order around it",
                    );
                    return None;
                }
                let mut corners = Vec::new();
                for item in items {
                    corners.push(self.point(cx, item, "points"));
                }
                let corners: Option<Vec<KindPoint>> = corners.into_iter().collect();
                Some(ShapeDef::Prism {
                    points: NonEmpty::new(corners?)?,
                    along: self.range(cx, along, "along"),
                })
            }
            ShapeFunction::Bar => {
                let from = self.required(cx, word, &found, "from", span);
                let to = self.required(cx, word, &found, "to", span);
                let thickness = self.required(cx, word, &found, "thickness", span);
                let along = self.required(cx, word, &found, "along", span);
                let (from, to, thickness, along) = (from?, to?, thickness?, along?);
                let from = self.point(cx, from, "from");
                let to = self.point(cx, to, "to");
                Some(ShapeDef::Bar {
                    from: Box::new(from?),
                    to: Box::new(to?),
                    thickness: self.frame_length(cx, thickness, "thickness"),
                    along: self.range(cx, along, "along"),
                })
            }
        }
    }

    /// A kind's `mechanism { }`, over its roles and bodies.
    pub(super) fn lower_mechanism(
        &mut self,
        cx: &Cx,
        kind: &str,
        roles: &[RoleDef],
        bodies: &[BodyDef],
        block: &ast::MechanismBlock,
    ) -> Option<MechanismDef> {
        let mut constraints = Vec::new();
        let mut couplings = Vec::new();
        let mut drive: Option<DriveDef> = None;
        let mut stop: Option<(&ast::Expr, &ast::StringLit, Span)> = None;
        let mut pin_diameter: Option<RExpr> = None;
        let mut seen: BTreeMap<&str, Span> = BTreeMap::new();
        let mut ok = true;
        // The direction the pins run (grammar, "Mechanisms", the plane):
        // the first `pins:` written, or the default, which a rail along
        // `along` turns to `across`.
        let written = block.entries.iter().find_map(|entry| match entry {
            ast::MechanismEntry::Field(field) if field.key.as_str() == PINS => Some(field),
            ast::MechanismEntry::Field(_)
            | ast::MechanismEntry::Pivot { .. }
            | ast::MechanismEntry::Slot { .. }
            | ast::MechanismEntry::Rail { .. }
            | ast::MechanismEntry::Couple { .. }
            | ast::MechanismEntry::Drive { .. }
            | ast::MechanismEntry::Stop { .. } => None,
        });
        let pins_written = match written {
            Some(field) => match self.pins(cx, field) {
                Some(axis) => Some(axis),
                None => {
                    ok = false;
                    None
                }
            },
            None => None,
        };
        let rail_along = block.entries.iter().any(|entry| {
            matches!(entry, ast::MechanismEntry::Rail { along, .. }
                if FrameAxis::from_word(along.as_str()) == Some(FrameAxis::Along))
        });
        let pins = pins_written.unwrap_or(if rail_along {
            FrameAxis::Across
        } else {
            FrameAxis::Along
        });
        for entry in &block.entries {
            match entry {
                ast::MechanismEntry::Pivot { members, at, span } => {
                    let pair = self.entry_pair(cx, kind, roles, bodies, members, "pivot");
                    let at = self.mechanism_point(cx, at, "at", pins);
                    match (pair, at) {
                        (Some(bodies), Some(at)) => constraints.push(ConstraintDef::Pivot {
                            bodies,
                            at: Box::new(at),
                            span: *span,
                        }),
                        _ => ok = false,
                    }
                }
                ast::MechanismEntry::Slot {
                    members,
                    from,
                    to,
                    span,
                } => {
                    let pair = self.entry_pair(cx, kind, roles, bodies, members, "slot");
                    let from = self.mechanism_point(cx, from, "from", pins);
                    let to = self.mechanism_point(cx, to, "to", pins);
                    match (pair, from, to) {
                        (Some(bodies), Some(from), Some(to)) => {
                            constraints.push(ConstraintDef::Slot {
                                bodies,
                                from: Box::new(from),
                                to: Box::new(to),
                                span: *span,
                            });
                        }
                        _ => ok = false,
                    }
                }
                ast::MechanismEntry::Rail {
                    members,
                    along,
                    span,
                } => {
                    let pair = self.entry_pair(cx, kind, roles, bodies, members, "rail");
                    let mut axis = FrameAxis::from_word(along.as_str());
                    if axis.is_none() {
                        self.error_help(
                            Code::TypeMismatch,
                            format!("a rail runs along along, across or depth, found '{along}'"),
                            cx.file,
                            along.span(),
                            "write rail a, b along along (or across, or depth)",
                        );
                    }
                    // Left out, rails along `along` and `across` are
                    // reported where the plane is chosen.
                    if let Some(written) = pins_written
                        && axis == Some(written)
                    {
                        let word = written.word();
                        self.error_help(
                            Code::Mechanism,
                            format!(
                                "this mechanism's pins run along '{word}', so a rail can't run along '{word}'"
                            ),
                            cx.file,
                            along.span(),
                            "run the rail along one of the two directions the members move in, or change 'pins'",
                        );
                        axis = None;
                    }
                    match (pair, axis) {
                        (Some(bodies), Some(along)) => constraints.push(ConstraintDef::Rail {
                            bodies,
                            along,
                            span: *span,
                        }),
                        _ => ok = false,
                    }
                }
                ast::MechanismEntry::Couple {
                    member,
                    to,
                    ratio,
                    span,
                } => {
                    let pair = self.member_pair(cx, kind, bodies, [member, to], "couple");
                    let ratio = self.value(cx, ratio, &Expect::ty(Ty::Ratio).named("couple"));
                    match pair {
                        Some([body, to]) => couplings.push(CouplingDef {
                            body,
                            to,
                            ratio,
                            span: *span,
                        }),
                        None => ok = false,
                    }
                }
                ast::MechanismEntry::Drive { key, value, span } => {
                    if let Some(first) = seen.insert("drive", key.span()) {
                        self.duplicate("drive", key.span(), first, cx);
                        continue;
                    }
                    match self.drive(cx, kind, bodies, value, *span) {
                        Some(lowered) => drive = Some(lowered),
                        None => ok = false,
                    }
                }
                ast::MechanismEntry::Stop {
                    key,
                    value,
                    message,
                    span,
                } => {
                    if let Some(first) = seen.insert("stop", key.span()) {
                        self.duplicate("stop", key.span(), first, cx);
                        continue;
                    }
                    stop = Some((value, message, *span));
                }
                ast::MechanismEntry::Field(field) => {
                    let key = field.key.as_str();
                    if key == PINS {
                        // Read before the entries; a second is reported.
                        if let Some(first) = seen.insert(PINS, field.key.span()) {
                            self.duplicate(key, field.key.span(), first, cx);
                        }
                        continue;
                    }
                    if key != "pin_diameter" {
                        self.unknown(
                            Code::UnknownField,
                            format!("a mechanism has no field '{key}'"),
                            cx.file,
                            field.key.span(),
                            key,
                            ["drive", "stop", "pin_diameter", PINS],
                            "a mechanism holds pivot, slot, rail and couple entries, drive:, stop:, pin_diameter: and pins:",
                        );
                        ok = false;
                        continue;
                    }
                    if let Some(first) = seen.insert("pin_diameter", field.key.span()) {
                        self.duplicate(key, field.key.span(), first, cx);
                        continue;
                    }
                    pin_diameter = Some(self.frame_length(cx, &field.value, key));
                }
            }
        }
        if drive.is_none() && seen.contains_key("drive") {
            // Written, but it didn't lower (reported).
            return None;
        }
        // A mechanism without a drive follows the hardware that drives its
        // assembly (a lid stay); its stop is an angle or a length, as that
        // drive's values are.
        let stop = stop.map(|(value, message, span)| {
            let expect = match drive {
                Some(DriveDef::Turn { .. }) => Expect::ty(Ty::Angle),
                Some(DriveDef::Travel { .. }) => Expect::ty(Ty::Length),
                None => Expect::any(),
            };
            StopDef {
                value: self.value(cx, value, &expect.named("stop")),
                message: self.text(cx, message),
                span,
            }
        });
        ok.then_some(MechanismDef {
            span: block.span,
            constraints,
            couplings,
            drive,
            stop,
            pin_diameter,
            pins: pins_written,
        })
    }

    /// The direction a mechanism's `pins:` names: one of the three frame
    /// words. `None` (reported) for anything else.
    fn pins(&mut self, cx: &Cx, field: &ast::Field) -> Option<FrameAxis> {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "a syntax-shape check: only a bare word means anything here, and every other expression is the error case"
        )]
        let axis = match &field.value.kind {
            ExprKind::Name(ident) => FrameAxis::from_word(ident.as_str()),
            _ => None,
        };
        if axis.is_none() {
            self.error_help(
                Code::TypeMismatch,
                "'pins' needs along, across or depth",
                cx.file,
                field.value.span,
                "write pins: depth for a pin that runs into the mount face, or leave it out for along",
            );
        }
        axis
    }

    /// A `point(...)` of a pivot or a slot: written in the two directions
    /// of the plane, never on the pins' own (`error[mechanism]`).
    fn mechanism_point(
        &mut self,
        cx: &Cx,
        expr: &ast::Expr,
        what: &str,
        pins: FrameAxis,
    ) -> Option<KindPoint> {
        if let ExprKind::Call { args, .. } = &expr.kind
            && let Some(arg) = args
                .iter()
                .find(|arg| arg.name.as_ref().is_some_and(|n| n.as_str() == pins.word()))
        {
            let [first, second] = pins.plane();
            self.error_help(
                Code::Mechanism,
                format!(
                    "this mechanism's pins run along '{}', so a point is given by '{}' and '{}'",
                    pins.word(),
                    first.word(),
                    second.word()
                ),
                cx.file,
                arg.span,
                format!(
                    "remove '{}:' from the point; every pin is parallel to '{}', so the point is anywhere along it",
                    pins.word(),
                    pins.word()
                ),
            );
            return None;
        }
        self.point(cx, expr, what)
    }

    /// The body a mechanism entry names.
    fn body(
        &mut self,
        cx: &Cx,
        kind: &str,
        bodies: &[BodyDef],
        ident: &ast::Ident,
    ) -> Option<BodyId> {
        let word = ident.as_str();
        if let Some(index) = bodies.iter().position(|body| body.name.as_str() == word) {
            return BodyId::new(index);
        }
        self.unknown(
            Code::UnknownName,
            format!("unknown name '{word}'"),
            cx.file,
            ident.span(),
            word,
            bodies.iter().map(|body| body.name.as_str()),
            format!("a mechanism names members with a body declared in kind '{kind}'"),
        );
        None
    }

    /// A hardware kind's one `clamp` (grammar, "Clamps"): two different
    /// members with a body, an axis of the frame, and the second member
    /// moving with a role that is a part. Any `clamp` after the first is
    /// `error[duplicate-name]`; one that doesn't lower is reported and left
    /// out.
    pub(super) fn lower_clamps(
        &mut self,
        cx: &Cx,
        kind: &str,
        roles: &[RoleDef],
        bodies: &[BodyDef],
        clamps: &[&ast::Clamp],
    ) -> Option<ClampDef> {
        let (first, rest) = clamps.split_first()?;
        for again in rest {
            self.report(
                Diagnostic::error(
                    Code::DuplicateName,
                    "'clamp' is declared twice",
                    Label::new(cx.file, again.span),
                )
                .with_label(Label::with_message(cx.file, first.span, "first written here"))
                .with_help(
                    "a kind draws parts together in one place; hardware that draws in two places is two pieces of hardware, each with its own call",
                ),
            );
        }
        let clamp = first;
        let named = |this: &mut Self, ident: &ast::Ident| -> Option<BodyId> {
            let word = ident.as_str();
            if let Some(index) = bodies.iter().position(|body| body.name.as_str() == word) {
                return BodyId::new(index);
            }
            this.unknown(
                Code::UnknownName,
                format!("unknown name '{word}'"),
                cx.file,
                ident.span(),
                word,
                bodies.iter().map(|body| body.name.as_str()),
                format!("a clamp names members with a body declared in kind '{kind}'"),
            );
            None
        };
        let [bearing_name, drawn_name] = &clamp.members;
        let bearing = named(self, bearing_name);
        let drawn = named(self, drawn_name);
        let axis = FrameAxis::from_word(clamp.axis.as_str());
        if axis.is_none() {
            self.error_help(
                Code::TypeMismatch,
                format!(
                    "a clamp draws along along, across or depth, found '{}'",
                    clamp.axis
                ),
                cx.file,
                clamp.axis.span(),
                "write clamp a, b along depth (or along, or across)",
            );
        }
        let (Some(bearing), Some(drawn), Some(axis)) = (bearing, drawn, axis) else {
            return None;
        };
        if bearing == drawn {
            self.error_help(
                Code::TypeMismatch,
                format!("'{drawn_name}' is named twice in one clamp"),
                cx.file,
                drawn_name.span(),
                "a clamp draws one member toward another: name two different members",
            );
            return None;
        }
        if !moves_with_a_part(roles, bodies, drawn) {
            self.error_help(
                Code::TypeMismatch,
                format!(
                    "clamp {bearing_name}, {drawn_name}: '{drawn_name}' moves with no part, so there is nothing for it to draw"
                ),
                cx.file,
                drawn_name.span(),
                format!(
                    "give '{drawn_name}' with: a role that is a Part, or a member that moves with one"
                ),
            );
            return None;
        }
        Some(ClampDef {
            bearing,
            drawn,
            axis,
            span: clamp.span,
        })
    }

    /// A hardware kind's `adjust` entries (grammar, "Adjustment"): each
    /// names two members with a body and an axis of the frame on which
    /// exactly one of them has its own mount; each member is adjusted on an
    /// axis at most once. Entries that don't lower are reported and left
    /// out.
    pub(super) fn lower_adjusts(
        &mut self,
        cx: &Cx,
        kind: &str,
        bodies: &[BodyDef],
        adjusts: &[&ast::Adjust],
    ) -> Vec<AdjustDef> {
        let mut lowered = Vec::new();
        let mut seen: BTreeMap<(BodyId, FrameAxis), Span> = BTreeMap::new();
        for adjust in adjusts {
            let within = self.frame_length(cx, &adjust.within, "within");
            let named = |this: &mut Self, ident: &ast::Ident| -> Option<BodyId> {
                let word = ident.as_str();
                if let Some(index) = bodies.iter().position(|body| body.name.as_str() == word) {
                    return BodyId::new(index);
                }
                this.unknown(
                    Code::UnknownName,
                    format!("unknown name '{word}'"),
                    cx.file,
                    ident.span(),
                    word,
                    bodies.iter().map(|body| body.name.as_str()),
                    format!("an adjust names members with a body declared in kind '{kind}'"),
                );
                None
            };
            let body = named(self, &adjust.member);
            let on = named(self, &adjust.on);
            let axis = FrameAxis::from_word(adjust.axis.as_str());
            if axis.is_none() {
                self.error_help(
                    Code::TypeMismatch,
                    format!(
                        "an adjust is along along, across or depth, found '{}'",
                        adjust.axis
                    ),
                    cx.file,
                    adjust.axis.span(),
                    "write adjust a on b: across within 2mm (or along, or depth)",
                );
            }
            let (Some(body), Some(on), Some(axis)) = (body, on, axis) else {
                continue;
            };
            if body == on {
                self.error_help(
                    Code::DuplicateName,
                    format!("'{}' is named twice in one adjust", adjust.on),
                    cx.file,
                    adjust.on.span(),
                    "an adjust sets one member on another",
                );
                continue;
            }
            let mounted = |id: BodyId| {
                bodies
                    .get(id.index())
                    .is_some_and(|b| b.mount.iter().any(|placement| placement.axis == axis))
            };
            match (mounted(body), mounted(on)) {
                (true, false) | (false, true) => {}
                (false, false) => {
                    self.error_help(
                        Code::MissingField,
                        format!(
                            "adjust {} on {}: neither has a mount on '{}', so nothing would ever be set",
                            adjust.member,
                            adjust.on,
                            axis.word()
                        ),
                        cx.file,
                        adjust.span,
                        format!(
                            "give one of them a mount {{ {}: from(...) }}, or remove this adjust",
                            axis.word()
                        ),
                    );
                    continue;
                }
                (true, true) => {
                    self.error_help(
                        Code::TypeMismatch,
                        format!(
                            "adjust {} on {}: both have a mount on '{}', so the setting is fixed by the design twice",
                            adjust.member,
                            adjust.on,
                            axis.word()
                        ),
                        cx.file,
                        adjust.span,
                        format!(
                            "mount only one of them on '{}'; the other is drawn where the kind puts it",
                            axis.word()
                        ),
                    );
                    continue;
                }
            }
            let mut again = None;
            for id in [body, on] {
                if let Some(first) = seen.get(&(id, axis)) {
                    again = Some((id, *first));
                    break;
                }
            }
            if let Some((id, first)) = again {
                let name = bodies
                    .get(id.index())
                    .map(|b| b.name.as_str().to_owned())
                    .unwrap_or_default();
                self.report(
                    Diagnostic::error(
                        Code::DuplicateName,
                        format!("'{name}' is adjusted twice {}", axis.word()),
                        Label::new(cx.file, adjust.span),
                    )
                    .with_label(Label::with_message(cx.file, first, "first written here"))
                    .with_help("write one adjust for a member on each axis"),
                );
                continue;
            }
            seen.insert((body, axis), adjust.span);
            seen.insert((on, axis), adjust.span);
            lowered.push(AdjustDef {
                body,
                on,
                axis,
                within,
                span: adjust.span,
            });
        }
        lowered
    }

    /// The two different bodies of a mechanism entry.
    /// The two names of a pivot, slot or rail: a member, then a member or
    /// a role of the kind that is a `Part` (grammar, "Mechanisms", a role
    /// in an entry).
    fn entry_pair(
        &mut self,
        cx: &Cx,
        kind: &str,
        roles: &[RoleDef],
        bodies: &[BodyDef],
        members: &[ast::Ident; 2],
        entry: &str,
    ) -> Option<EntryPair> {
        let [a, b] = members;
        let first = self.body(cx, kind, bodies, a);
        let word = b.as_str();
        let second = match bodies.iter().position(|body| body.name.as_str() == word) {
            Some(index) => BodyId::new(index).map(EntrySide::Member),
            None => match roles.iter().position(|role| role.name == word) {
                Some(index) => self.entry_role(cx, roles, index, b),
                None => {
                    self.unknown(
                        Code::UnknownName,
                        format!("unknown name '{word}'"),
                        cx.file,
                        b.span(),
                        word,
                        bodies
                            .iter()
                            .map(|body| body.name.as_str())
                            .chain(roles.iter().map(|role| role.name.as_str())),
                        format!(
                            "a {entry} names a member with a body declared in kind '{kind}', then a member or one of its roles"
                        ),
                    );
                    None
                }
            },
        };
        let (member, other) = (first?, second?);
        if other == EntrySide::Member(member) {
            self.error_help(
                Code::DuplicateName,
                format!("'{a}' is named twice in one {entry}"),
                cx.file,
                b.span(),
                format!("a {entry} joins two different members"),
            );
            return None;
        }
        Some(EntryPair { member, other })
    }

    /// A role named as the second name of an entry: one part.
    fn entry_role(
        &mut self,
        cx: &Cx,
        roles: &[RoleDef],
        index: usize,
        ident: &ast::Ident,
    ) -> Option<EntrySide> {
        let role = roles.get(index)?;
        let what = match role.ty {
            stave_schema::kinds::RoleType::Part => {
                return RoleIndex::new(index).map(EntrySide::Role);
            }
            stave_schema::kinds::RoleType::Region => "a region",
            stave_schema::kinds::RoleType::Parts => "a list of parts",
        };
        self.error_help(
            Code::Mechanism,
            format!("a pin turns in one part; '{ident}' is {what}"),
            cx.file,
            ident.span(),
            "name a role that is a Part, or a member of the kind",
        );
        None
    }

    /// The two members of a couple.
    fn member_pair(
        &mut self,
        cx: &Cx,
        kind: &str,
        bodies: &[BodyDef],
        members: [&ast::Ident; 2],
        entry: &str,
    ) -> Option<[BodyId; 2]> {
        let [a, b] = members;
        let first = self.body(cx, kind, bodies, a);
        let second = self.body(cx, kind, bodies, b);
        let (first, second) = (first?, second?);
        if first == second {
            self.error_help(
                Code::DuplicateName,
                format!("'{a}' is named twice in one {entry}"),
                cx.file,
                b.span(),
                format!("a {entry} joins two different members"),
            );
            return None;
        }
        Some([first, second])
    }

    /// `turn(m, from:, to:)` or `travel(m, from:, to:)`.
    fn drive(
        &mut self,
        cx: &Cx,
        kind: &str,
        bodies: &[BodyDef],
        value: &ast::Expr,
        span: Span,
    ) -> Option<DriveDef> {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
        )]
        let call = match &value.kind {
            ExprKind::Call { callee, args } => match &callee.kind {
                ExprKind::Name(head) if DRIVES.contains(&head.as_str()) => Some((head, args)),
                _ => None,
            },
            _ => None,
        };
        let Some((head, args)) = call else {
            self.error_help(
                Code::TypeMismatch,
                "'drive' needs turn(...) or travel(...)",
                cx.file,
                value.span,
                "write drive: turn(member, from: 0deg, to: ...) or drive: travel(member, from: 0mm, to: ...)",
            );
            return None;
        };
        let function = head.as_str();
        let (positional, named): (Vec<&ast::Arg>, Vec<&ast::Arg>) =
            args.iter().partition(|arg| arg.name.is_none());
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "a syntax-shape check: only the forms it names mean anything here, and every other expression is the error case"
        )]
        let member = match positional.as_slice() {
            [only] => match &only.value.kind {
                ExprKind::Name(ident) => self.body(cx, kind, bodies, ident),
                _ => {
                    self.error_help(
                        Code::TypeMismatch,
                        format!("{function}(...) needs the member it moves first"),
                        cx.file,
                        only.value.span,
                        format!("write {function}(member, from: ..., to: ...)"),
                    );
                    None
                }
            },
            _ => {
                self.error_help(
                    Code::MissingField,
                    format!("{function}(...) needs the member it moves first"),
                    cx.file,
                    value.span,
                    format!("write {function}(member, from: ..., to: ...)"),
                );
                None
            }
        };
        let named: Vec<ast::Arg> = named.into_iter().cloned().collect();
        let found = self.arguments(cx, function, &named, &["from", "to"])?;
        let from = self.required(cx, function, &found, "from", value.span);
        let to = self.required(cx, function, &found, "to", value.span);
        let (body, from, to) = (member?, from?, to?);
        let ty = if function == "turn" {
            Ty::Angle
        } else {
            Ty::Length
        };
        let from = self.value(cx, from, &Expect::ty(ty.clone()).named("from"));
        let to = self.value(cx, to, &Expect::ty(ty).named("to"));
        Some(if function == "turn" {
            DriveDef::Turn {
                body,
                from,
                to,
                span,
            }
        } else {
            DriveDef::Travel {
                body,
                from,
                to,
                span,
            }
        })
    }
}

/// `0mm`, written nowhere: an unwritten coordinate.
fn zero(span: Span) -> RExpr {
    RExpr {
        kind: RExprKind::Length(ExactLength::ZERO),
        ty: Ty::Length,
        span,
    }
}

/// The frame's origin.
fn origin(span: Span) -> KindPoint {
    KindPoint {
        along: zero(span),
        across: zero(span),
        depth: zero(span),
    }
}

/// `along:, across:, depth:` for a help line.
fn argument_list(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("{name}:"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether a member moves with a role that is a part: its `with:` names
/// one, or a member that does, followed member by member. A loop of
/// members riding on each other (reported where members are lowered)
/// moves with nothing.
fn moves_with_a_part(roles: &[RoleDef], bodies: &[BodyDef], body: BodyId) -> bool {
    let mut current = body;
    for _ in 0..=bodies.len() {
        match bodies.get(current.index()).map(|body| &body.with) {
            Some(Carrier::Role(role)) => {
                return roles
                    .get(role.index())
                    .is_some_and(|role| role.ty == stave_schema::kinds::RoleType::Part);
            }
            Some(Carrier::Body(next)) => current = *next,
            Some(Carrier::Mechanism | Carrier::Contact(_)) | None => return false,
        }
    }
    false
}

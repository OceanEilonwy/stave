//! Lowering what a slide, hinge or pull kind says about where it sits and
//! what it cuts (grammar, "Hardware placement" and "Faces to cut"): its
//! `mount { }`, a member's own `mount { }`, and its `cut` entries.
//!
//! Nothing here knows a base: every rule is the kind's own.

use stave_lang::Code;
use stave_lang::ast::{self, ExprKind, Span};
use stave_lang::placement::{Direction, Face};
use stave_schema::kinds::RoleType;
use stave_schema::operations::Selector;

use super::expr::Expect;
use super::{Cx, Resolver};
use crate::defs::{
    AxisRule, BodyDef, BodyId, CutName, FaceRule, FrameAxis, FromTerm, HardwareCutDef,
    MemberPlacement, MountDef, MountSelector, MountTerm, RoleDef, RoleIndex,
};
use crate::types::Ty;

/// The anchors a `from(...)` takes.
const ANCHORS: [&str; 3] = ["edge", "end", "face"];

/// Which anchors an entry may use.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// A kind's `across:` or `along:`: an edge or an end of the mount face.
    Kind,
    /// A member's `mount { }`: any anchor.
    Member,
}

impl Resolver<'_> {
    /// A hardware kind's `mount { face:, across:, along: }`.
    pub(super) fn lower_mount(
        &mut self,
        cx: &Cx,
        roles: &[RoleDef],
        block: &ast::MountBlock,
    ) -> Option<MountDef> {
        let mut face = None;
        let mut across = None;
        let mut along = None;
        let mut seen: Vec<(&str, Span)> = Vec::new();
        for field in &block.fields {
            let key = field.key.as_str();
            if let Some((_, first)) = seen.iter().find(|(word, _)| *word == key) {
                self.duplicate(key, field.key.span(), *first, cx);
                continue;
            }
            seen.push((key, field.key.span()));
            match key {
                "face" => face = self.face_rule(cx, roles, &field.value),
                "across" => across = self.axis_rule(cx, roles, &field.value),
                "along" => along = self.axis_rule(cx, roles, &field.value),
                _ => self.unknown(
                    Code::UnknownField,
                    format!("a mount has no field '{key}'"),
                    cx.file,
                    field.key.span(),
                    key,
                    ["face", "across", "along"],
                    "a kind's mount says its face, and where it sits across and along it",
                ),
            }
        }
        for (word, written) in [
            ("face", seen.iter().any(|(w, _)| *w == "face")),
            ("across", seen.iter().any(|(w, _)| *w == "across")),
            ("along", seen.iter().any(|(w, _)| *w == "along")),
        ] {
            if !written {
                self.error_help(
                    Code::MissingField,
                    format!("the mount is missing '{word}'"),
                    cx.file,
                    block.span,
                    format!(
                        "write {word}: ...; use open to leave an axis to the design's place {{ }}"
                    ),
                );
            }
        }
        Some(MountDef {
            face: face?,
            across: across?,
            along: along?,
            span: block.span,
        })
    }

    /// A member's own `mount { }`: `across:`, `along:` or `depth:`, each a
    /// `from(anchor, inset: d)`.
    pub(super) fn member_mount(
        &mut self,
        cx: &Cx,
        roles: &[RoleDef],
        block: &ast::MountBlock,
    ) -> Vec<MemberPlacement> {
        let mut placements: Vec<MemberPlacement> = Vec::new();
        let mut seen: Vec<(FrameAxis, Span)> = Vec::new();
        for field in &block.fields {
            let key = field.key.as_str();
            let Some(axis) = FrameAxis::from_word(key) else {
                self.unknown(
                    Code::UnknownField,
                    format!("a member's mount has no field '{key}'"),
                    cx.file,
                    field.key.span(),
                    key,
                    ["across", "along", "depth"],
                    "a member's mount places it on the kind's frame axes: across, along and depth",
                );
                continue;
            };
            if let Some((_, first)) = seen.iter().find(|(a, _)| *a == axis) {
                self.duplicate(key, field.key.span(), *first, cx);
                continue;
            }
            seen.push((axis, field.key.span()));
            match self.mount_term(cx, roles, &field.value, Place::Member) {
                Some(MountTerm::From(from)) => placements.push(MemberPlacement { axis, from }),
                Some(MountTerm::Centre { .. }) => self.error_help(
                    Code::TypeMismatch,
                    format!("'{key}' of a member's mount needs from(...)"),
                    cx.file,
                    field.value.span,
                    "write from(face(on, toward: piece), inset: ...)",
                ),
                None => {}
            }
        }
        placements
    }

    /// `cut [name:] crossing(role, member) { operations }` in a hardware
    /// kind.
    pub(super) fn lower_hardware_cut(
        &mut self,
        cx: &Cx,
        roles: &[RoleDef],
        bodies: &[BodyDef],
        cut: &ast::Cut,
    ) -> Option<HardwareCutDef> {
        let head = cut.selector.head.as_str();
        if head != Selector::Crossing.word() {
            self.error_help(
                Code::TypeMismatch,
                format!("a hardware kind cuts on crossing, found '{head}'"),
                cx.file,
                cut.selector.head.span(),
                "write cut crossing(piece, member) { ... }: the face of the part the member touches or enters",
            );
            return None;
        }
        let args = self.split_args(cx, head, &cut.selector.args, &[], 2);
        let [part, member] = args.positional.as_slice() else {
            self.missing_argument(cx, head, "a role and a member", cut.selector.span);
            return None;
        };
        let part = self.role_of(cx, roles, part, "crossing")?;
        let body = self.body_of(cx, bodies, member)?;
        let name = match &cut.name {
            Some(name) => Some(CutName::new(name.as_str())?),
            None => {
                self.warn_unnamed_cut(cx, cut);
                None
            }
        };
        let operations = self.lower_operations(cx, Selector::Crossing, &cut.operations);
        Some(HardwareCutDef {
            name,
            part,
            body,
            operations,
            span: cut.span,
        })
    }

    /// `face: back` or `face: face(piece, nearer: in)`.
    fn face_rule(&mut self, cx: &Cx, roles: &[RoleDef], expr: &ast::Expr) -> Option<FaceRule> {
        match &expr.kind {
            ExprKind::Name(word) => match Face::from_word(word.as_str()) {
                Some(face) => Some(FaceRule::Word(face)),
                None => {
                    self.unknown(
                        Code::UnknownName,
                        format!("unknown name '{}'", word.as_str()),
                        cx.file,
                        expr.span,
                        word.as_str(),
                        Face::ALL.map(Face::word),
                        "write a face word (back, front, left, right, top, bottom) or face(piece, nearer: region)",
                    );
                    None
                }
            },
            ExprKind::Call { callee, args } if callee_word(callee) == Some("face") => {
                let parsed = self.split_args(cx, "face", args, &["nearer"], 1);
                let (Some(part), Some(region)) = (parsed.positional.first(), parsed.get("nearer"))
                else {
                    self.missing_argument(cx, "face", "a role and nearer:", expr.span);
                    return None;
                };
                let part = self.role_of(cx, roles, part, "face")?;
                let region_index = self.role_of(cx, roles, region, "nearer")?;
                let is_region = roles
                    .get(region_index.index())
                    .is_some_and(|role| role.ty == RoleType::Region);
                if !is_region {
                    self.error_help(
                        Code::TypeMismatch,
                        "'nearer' of a mount's face needs a region role",
                        cx.file,
                        region.span,
                        "name the kind's Region role: face(piece, nearer: in)",
                    );
                    return None;
                }
                Some(FaceRule::Nearer {
                    part,
                    region: region_index,
                })
            }
            _ => {
                self.error_help(
                    Code::TypeMismatch,
                    "'face' of a mount needs a face word or face(piece, nearer: region)",
                    cx.file,
                    expr.span,
                    "write face: back, or face: face(piece, nearer: in)",
                );
                None
            }
        }
    }

    /// `open`, or a mount term.
    fn axis_rule(&mut self, cx: &Cx, roles: &[RoleDef], expr: &ast::Expr) -> Option<AxisRule> {
        if let ExprKind::Name(word) = &expr.kind
            && word.as_str() == "open"
        {
            return Some(AxisRule::Open);
        }
        self.mount_term(cx, roles, expr, Place::Kind)
            .map(AxisRule::Term)
    }

    /// `from(anchor, inset: d)` or `center(p)`.
    fn mount_term(
        &mut self,
        cx: &Cx,
        roles: &[RoleDef],
        expr: &ast::Expr,
        place: Place,
    ) -> Option<MountTerm> {
        let ExprKind::Call { callee, args } = &expr.kind else {
            self.error_help(
                Code::TypeMismatch,
                "a mount entry needs from(...), center(...) or open",
                cx.file,
                expr.span,
                "write from(edge(piece, nearer: on), inset: ...), center(piece) or open",
            );
            return None;
        };
        match callee_word(callee) {
            Some("from") => {
                let parsed = self.split_args(cx, "from", args, &["inset"], 1);
                let Some(anchor) = parsed.positional.first() else {
                    self.missing_argument(cx, "from", "an anchor", expr.span);
                    return None;
                };
                let inset = parsed
                    .get("inset")
                    .map(|inset| self.value(cx, inset, &Expect::ty(Ty::Length).named("inset")));
                let anchor = self.anchor(cx, roles, anchor, place)?;
                Some(MountTerm::From(FromTerm {
                    anchor,
                    inset,
                    span: expr.span,
                }))
            }
            Some("center") => {
                let parsed = self.split_args(cx, "center", args, &[], 1);
                let Some(part) = parsed.positional.first() else {
                    self.missing_argument(cx, "center", "a role", expr.span);
                    return None;
                };
                let part = self.role_of(cx, roles, part, "center")?;
                Some(MountTerm::Centre { part })
            }
            _ => {
                let word = callee_word(callee).unwrap_or("");
                self.unknown(
                    Code::UnknownName,
                    format!("unknown name '{word}'"),
                    cx.file,
                    callee.span,
                    word,
                    ["from", "center"],
                    "a mount entry is from(anchor, inset: d), center(piece) or open",
                );
                None
            }
        }
    }

    /// An anchor: `edge(...)`, `end(...)`, `face(p, toward: q)` or
    /// `crossing(p, q)`.
    fn anchor(
        &mut self,
        cx: &Cx,
        roles: &[RoleDef],
        expr: &ast::Expr,
        place: Place,
    ) -> Option<MountSelector> {
        let ExprKind::Call { callee, args } = &expr.kind else {
            self.error_help(
                Code::TypeMismatch,
                "'from' needs an anchor",
                cx.file,
                expr.span,
                "write from(edge(piece, nearer: on)) or from(face(on, toward: piece))",
            );
            return None;
        };
        let word = callee_word(callee).unwrap_or("");
        let selector = match word {
            "edge" => {
                let parsed = self.split_args(cx, "edge", args, &["nearer", "flush_with"], 1);
                let Some(part) = parsed.positional.first() else {
                    self.missing_argument(cx, "edge", "a role", expr.span);
                    return None;
                };
                let part = self.role_of(cx, roles, part, "edge")?;
                match (parsed.get("nearer"), parsed.get("flush_with")) {
                    (Some(other), None) => MountSelector::EdgeNearer {
                        part,
                        other: self.role_of(cx, roles, other, "nearer")?,
                    },
                    (None, Some(other)) => MountSelector::EdgeFlushWith {
                        part,
                        other: self.role_of(cx, roles, other, "flush_with")?,
                    },
                    _ => {
                        self.error_help(
                            Code::TypeMismatch,
                            "'edge' needs one of nearer: or flush_with:",
                            cx.file,
                            expr.span,
                            "write edge(piece, nearer: on) or edge(piece, flush_with: on)",
                        );
                        return None;
                    }
                }
            }
            "end" => {
                let parsed = self.split_args(cx, "end", args, &["toward"], 1);
                let (Some(part), Some(toward)) = (parsed.positional.first(), parsed.get("toward"))
                else {
                    self.missing_argument(cx, "end", "a role and toward:", expr.span);
                    return None;
                };
                let part = self.role_of(cx, roles, part, "end")?;
                let direction = match &toward.kind {
                    ExprKind::Name(word) => Direction::from_word(word.as_str()),
                    _ => None,
                };
                let Some(toward) = direction else {
                    self.error_help(
                        Code::TypeMismatch,
                        "'toward' needs a direction",
                        cx.file,
                        toward.span,
                        "write a direction: front, back, left, right, up or down",
                    );
                    return None;
                };
                MountSelector::EndToward { part, toward }
            }
            "face" if place == Place::Member => {
                let parsed = self.split_args(cx, "face", args, &["toward"], 1);
                let (Some(part), Some(other)) = (parsed.positional.first(), parsed.get("toward"))
                else {
                    self.missing_argument(cx, "face", "a role and toward:", expr.span);
                    return None;
                };
                MountSelector::FaceToward {
                    part: self.role_of(cx, roles, part, "face")?,
                    other: self.role_of(cx, roles, other, "toward")?,
                }
            }
            "crossing" if place == Place::Member => {
                let parsed = self.split_args(cx, "crossing", args, &[], 2);
                let [part, other] = parsed.positional.as_slice() else {
                    self.missing_argument(cx, "crossing", "two roles", expr.span);
                    return None;
                };
                MountSelector::Crossing {
                    part: self.role_of(cx, roles, part, "crossing")?,
                    other: self.role_of(cx, roles, other, "crossing")?,
                }
            }
            "face" | "crossing" => {
                self.error_help(
                    Code::TypeMismatch,
                    format!("a kind's mount measures from an edge or an end, found '{word}'"),
                    cx.file,
                    expr.span,
                    "use edge(...) or end(...) here; a face is an anchor of a member's own mount",
                );
                return None;
            }
            _ => {
                self.unknown(
                    Code::UnknownName,
                    format!("unknown name '{word}'"),
                    cx.file,
                    callee.span,
                    word,
                    ANCHORS.iter().copied().chain(["crossing"]),
                    "an anchor is edge(...), end(...), face(p, toward: q) or crossing(p, q)",
                );
                return None;
            }
        };
        Some(selector)
    }

    /// A role of the kind, by name.
    fn role_of(
        &mut self,
        cx: &Cx,
        roles: &[RoleDef],
        expr: &ast::Expr,
        what: &str,
    ) -> Option<RoleIndex> {
        if let ExprKind::Name(ident) = &expr.kind
            && let Some(index) = roles.iter().position(|role| role.name == ident.as_str())
        {
            return RoleIndex::new(index);
        }
        let word = match &expr.kind {
            ExprKind::Name(ident) => ident.as_str().to_owned(),
            _ => String::new(),
        };
        self.unknown(
            Code::UnknownName,
            format!("'{what}' needs a role of the kind, found '{word}'"),
            cx.file,
            expr.span,
            &word,
            roles.iter().map(|role| role.name.as_str()),
            "name one of the kind's roles",
        );
        None
    }

    /// A member with a body, by name.
    fn body_of(&mut self, cx: &Cx, bodies: &[BodyDef], expr: &ast::Expr) -> Option<BodyId> {
        if let ExprKind::Name(ident) = &expr.kind
            && let Some(index) = bodies
                .iter()
                .position(|body| body.name.as_str() == ident.as_str())
        {
            return BodyId::new(index);
        }
        let word = match &expr.kind {
            ExprKind::Name(ident) => ident.as_str().to_owned(),
            _ => String::new(),
        };
        self.unknown(
            Code::UnknownName,
            format!("'crossing' needs a member of the kind, found '{word}'"),
            cx.file,
            expr.span,
            &word,
            bodies.iter().map(|body| body.name.as_str()),
            "name one of the kind's members with a shape: cut crossing(piece, member) { ... }",
        );
        None
    }
}

/// The word a call's callee is, when it is a bare name.
fn callee_word(callee: &ast::Expr) -> Option<&str> {
    match &callee.kind {
        ExprKind::Name(name) => Some(name.as_str()),
        _ => None,
    }
}

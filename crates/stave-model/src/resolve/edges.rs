//! Lowering a part's `edges:` (grammar, "Edge profiles"): chamfers and
//! roundovers on the edges where two of its faces meet.

use std::collections::BTreeMap;

use stave_lang::ast::{self, ExprKind, Span};
use stave_lang::placement::Face;
use stave_lang::{Code, Diagnostic, Label};
use stave_schema::CutOperation;
use stave_schema::operations::{AROUND, EDGE, cut_operation};

use super::expr::Expect;
use super::{Cx, Resolver};
use crate::defs::{EdgeDef, EdgeProfileDef, RExpr};
use crate::types::Ty;

/// The help every malformed `edges:` gets.
const EDGES_HELP: &str =
    "write edges: chamfer(3mm, on: edge(top, front)), or a list of chamfer(...) and roundover(...)";

/// The help for an edge that isn't two faces that meet.
const EDGE_HELP: &str = "name two faces of the part that meet, as edge(top, front), or every edge of one face with around(top)";

/// The word an expression is, when it is a bare name.
fn bare_name(expr: &ast::Expr) -> Option<&str> {
    if let ExprKind::Name(ident) = &expr.kind {
        Some(ident.as_str())
    } else {
        None
    }
}

/// The items of a value that is one thing or a list of them.
fn one_or_list(expr: &ast::Expr) -> &[ast::Expr] {
    if let ExprKind::List(items) = &expr.kind {
        items
    } else {
        std::slice::from_ref(expr)
    }
}

impl Resolver<'_> {
    /// A part's `edges:`: one profile or a list of them, each edge shaped
    /// once.
    pub(super) fn lower_edges(&mut self, cx: &Cx, value: &ast::Expr) -> Vec<EdgeProfileDef> {
        let mut profiles = Vec::new();
        // Each edge, as its two faces in order, with where it was named.
        let mut seen: BTreeMap<[Face; 2], Span> = BTreeMap::new();
        for item in one_or_list(value) {
            let Some(mut profile) = self.lower_edge_profile(cx, item) else {
                continue;
            };
            let mut kept = Vec::with_capacity(profile.edges.len());
            for edge in profile.edges {
                let mut key = edge.faces;
                key.sort();
                if let Some(first) = seen.get(&key) {
                    self.report(
                        Diagnostic::error(
                            Code::DuplicateName,
                            format!(
                                "the edge where {} meets {} is shaped twice",
                                edge.faces[0].word(),
                                edge.faces[1].word()
                            ),
                            Label::new(cx.file, edge.span),
                        )
                        .with_label(Label::with_message(cx.file, *first, "first shaped here"))
                        .with_help("give each edge one profile: take it out of one of the lists"),
                    );
                    continue;
                }
                seen.insert(key, edge.span);
                kept.push(edge);
            }
            profile.edges = kept;
            profiles.push(profile);
        }
        profiles
    }

    /// One `chamfer(size, on: ...)` or `roundover(radius, on: ...)`.
    fn lower_edge_profile(&mut self, cx: &Cx, item: &ast::Expr) -> Option<EdgeProfileDef> {
        let ExprKind::Call { callee, args } = &item.kind else {
            self.error_help(
                Code::TypeMismatch,
                "'edges' needs chamfer(...) or roundover(...), or a list of them",
                cx.file,
                item.span,
                EDGES_HELP,
            );
            return None;
        };
        let head = bare_name(callee).unwrap_or("this");
        let Some(operation) = CutOperation::from_word(head).filter(|op| op.is_edge_profile())
        else {
            self.unknown(
                Code::UnknownName,
                format!("unknown name '{head}'"),
                cx.file,
                callee.span,
                head,
                [CutOperation::Chamfer.word(), CutOperation::Roundover.word()],
                "an edge profile is chamfer(size, on: ...) or roundover(radius, on: ...)",
            );
            return None;
        };
        let size_name = cut_operation(operation)
            .arguments
            .first()
            .map_or("size", |argument| argument.name);
        let parsed = self.split_args(cx, head, args, &[size_name, "on"], 1);
        let size = match (parsed.positional.first().copied(), parsed.get(size_name)) {
            (Some(_), Some(named)) => {
                self.error_help(
                    Code::DuplicateName,
                    format!("'{size_name}' is declared twice"),
                    cx.file,
                    named.span,
                    format!("give '{size_name}' once; remove one of them"),
                );
                None
            }
            (Some(expr), None) | (None, Some(expr)) => Some(expr),
            (None, None) => None,
        };
        let Some(size) = size else {
            if parsed.get(size_name).is_none() {
                self.missing_argument(cx, head, &format!("{size_name}:"), item.span);
            }
            return None;
        };
        let size: RExpr = self.value(cx, size, &Expect::ty(Ty::Length).named(size_name));
        let Some(on) = parsed.get("on") else {
            self.missing_argument(cx, head, "on:", item.span);
            return None;
        };
        let mut edges = Vec::new();
        for edge in one_or_list(on) {
            self.lower_edge(cx, edge, &mut edges);
        }
        Some(EdgeProfileDef {
            operation,
            size,
            edges,
            span: item.span,
        })
    }

    /// One `edge(a, b)` or `around(face)`, added to `edges`.
    fn lower_edge(&mut self, cx: &Cx, expr: &ast::Expr, edges: &mut Vec<EdgeDef>) {
        let call = if let ExprKind::Call { callee, args } = &expr.kind {
            bare_name(callee).map(|head| (head, args))
        } else {
            None
        };
        let Some((head, args)) = call.filter(|(head, _)| *head == EDGE || *head == AROUND) else {
            let word = call.map_or("this", |(head, _)| head);
            let fallback = format!("'on' needs edge(face, face) or around(face); {EDGE_HELP}");
            match call {
                Some(_) => self.unknown(
                    Code::UnknownName,
                    format!("unknown name '{word}'"),
                    cx.file,
                    expr.span,
                    word,
                    [EDGE, AROUND],
                    fallback,
                ),
                None => self.error_help(
                    Code::TypeMismatch,
                    "'on' needs edge(face, face) or around(face), or a list of them",
                    cx.file,
                    expr.span,
                    EDGE_HELP,
                ),
            }
            return;
        };
        let wanted = if head == EDGE { 2 } else { 1 };
        let parsed = self.split_args(cx, head, args, &[], wanted);
        let mut faces = Vec::with_capacity(wanted);
        for value in &parsed.positional {
            match bare_name(value).and_then(Face::from_word) {
                Some(face) => faces.push(face),
                None => {
                    self.error_help(
                        Code::TypeMismatch,
                        format!("'{head}' needs faces of the part: top, bottom, left, right, front or back"),
                        cx.file,
                        value.span,
                        EDGE_HELP,
                    );
                    return;
                }
            }
        }
        match faces.as_slice() {
            [a, b] => {
                if a.axis() == b.axis() {
                    let message = if a == b {
                        format!("edge({}, {}) names {} twice", a.word(), b.word(), a.word())
                    } else {
                        format!(
                            "{} and {} are opposite faces, so they don't meet at an edge",
                            a.word(),
                            b.word()
                        )
                    };
                    self.error_help(Code::TypeMismatch, message, cx.file, expr.span, EDGE_HELP);
                    return;
                }
                edges.push(EdgeDef {
                    faces: [*a, *b],
                    span: expr.span,
                });
            }
            [face] => {
                for other in Face::ALL {
                    if other.axis() != face.axis() {
                        edges.push(EdgeDef {
                            faces: [*face, other],
                            span: expr.span,
                        });
                    }
                }
            }
            _ => {
                let what = if wanted == 2 { "two faces" } else { "one face" };
                self.missing_argument(cx, head, what, expr.span);
            }
        }
    }
}

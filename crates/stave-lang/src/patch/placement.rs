//! Which axes a placement term fixes, read from its text.
//!
//! A patch addresses a term by axis (`set side_l.place.deep = ...`). The
//! axis of `against(base.top)` and `flush(base.left)` is the axis of the
//! face; `center(base, across)` names it; `center(from: a, to: b)` and
//! `spaced(..., from: a, to: b)` take it from their anchors. An anchor whose
//! face can't be read from the text (an `Anchor` param) fixes no axis the
//! patcher can see.

use crate::placement::{Axis, Face};
use crate::syntax::{SyntaxKind, SyntaxNode};

use super::tree::{arg_name, arg_value, word};

/// What a placement term fixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fixes {
    /// One or more world axes.
    Axes(&'static [Axis]),
    /// The part's rotation: `orient`.
    Orientation,
    /// Nothing the patcher can read from the text.
    Unknown,
}

impl Fixes {
    fn axis(axis: Axis) -> Self {
        match axis {
            Axis::Up => Fixes::Axes(&[Axis::Up]),
            Axis::Across => Fixes::Axes(&[Axis::Across]),
            Axis::Deep => Fixes::Axes(&[Axis::Deep]),
        }
    }
}

/// What a patch path's last word (`up`, `across`, `deep`, `orient`)
/// addresses in a `place` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TermAddress {
    /// The term that fixes this axis.
    Axis(Axis),
    /// The `orient` term.
    Orientation,
}

impl TermAddress {
    /// The address for a path's word.
    pub fn from_word(word: &str) -> Option<Self> {
        if word == "orient" {
            return Some(TermAddress::Orientation);
        }
        Axis::from_word(word).map(TermAddress::Axis)
    }

    /// `true` if a term that fixes `fixes` is the one addressed.
    pub fn matches(self, fixes: Fixes) -> bool {
        match (self, fixes) {
            (TermAddress::Axis(axis), Fixes::Axes(axes)) => axes.contains(&axis),
            (TermAddress::Orientation, Fixes::Orientation) => true,
            (TermAddress::Axis(_), Fixes::Orientation | Fixes::Unknown)
            | (TermAddress::Orientation, Fixes::Axes(_) | Fixes::Unknown) => false,
        }
    }
}

/// What a `CALL` in a `place` block fixes.
pub(super) fn fixes(term: &SyntaxNode) -> Fixes {
    let Some(head) = term
        .children()
        .find(|child| child.kind() == SyntaxKind::NAME)
        .and_then(|name| word(&name))
    else {
        return Fixes::Unknown;
    };
    let args: Vec<SyntaxNode> = term
        .children()
        .find(|child| child.kind() == SyntaxKind::ARG_LIST)
        .map(|list| {
            list.children()
                .filter(|child| child.kind() == SyntaxKind::ARG)
                .collect()
        })
        .unwrap_or_default();
    let positional = |index: usize| {
        args.iter()
            .filter(|arg| arg_name(arg).is_none())
            .nth(index)
            .and_then(arg_value)
    };
    let named = |name: &str| {
        args.iter()
            .find(|arg| arg_name(arg).as_deref() == Some(name))
            .and_then(arg_value)
    };
    let from_anchor = |anchor: Option<SyntaxNode>| {
        anchor
            .as_ref()
            .and_then(anchor_axis)
            .map_or(Fixes::Unknown, Fixes::axis)
    };
    match head.as_str() {
        "ground" => Fixes::Axes(&[Axis::Up, Axis::Across, Axis::Deep]),
        "corners" => Fixes::Axes(&[Axis::Across, Axis::Deep]),
        "orient" => Fixes::Orientation,
        "against" | "flush" => from_anchor(positional(0)),
        "center" => match positional(1).as_ref().and_then(axis_word) {
            Some(axis) => Fixes::axis(axis),
            None => from_anchor(named("from").or_else(|| named("to"))),
        },
        "spaced" => from_anchor(named("from").or_else(|| named("to"))),
        _ => Fixes::Unknown,
    }
}

/// The axis of an anchor expression: a face (`base.top`,
/// `drawer[top].front`) or a mid-plane (`mid(base, across)`).
fn anchor_axis(anchor: &SyntaxNode) -> Option<Axis> {
    match anchor.kind() {
        SyntaxKind::MEMBER_EXPR => {
            let member = anchor
                .children()
                .filter(|child| child.kind() == SyntaxKind::NAME)
                .last()?;
            Face::from_word(&word(&member)?).map(Face::axis)
        }
        SyntaxKind::CALL_EXPR => {
            let callee = anchor.children().next()?;
            if callee.kind() != SyntaxKind::NAME_REF || word(&callee)?.as_str() != "mid" {
                return None;
            }
            let list = anchor
                .children()
                .find(|child| child.kind() == SyntaxKind::ARG_LIST)?;
            let axis = list
                .children()
                .filter(|child| child.kind() == SyntaxKind::ARG && arg_name(child).is_none())
                .nth(1)?;
            axis_word(&arg_value(&axis)?)
        }
        SyntaxKind::PAREN_EXPR => anchor_axis(&anchor.children().next()?),
        _ => None,
    }
}

/// An axis written as a bare word.
fn axis_word(expr: &SyntaxNode) -> Option<Axis> {
    if expr.kind() != SyntaxKind::NAME_REF {
        return None;
    }
    Axis::from_word(&word(expr)?)
}

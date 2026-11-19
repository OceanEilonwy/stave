//! Fixes for a hinged front that hits something as it opens (grammar,
//! "Fixes"): a patch that sets the overlay on the edge that hits to what
//! clears, and one that opens it no further than where it hits, when its
//! hardware's kind allows that.
//!
//! A fix edits the project's own text, so each is built from the text as
//! written: the `overlay(...)` that places the front is copied with only
//! the one edge changed, to an exact length.

use stave_lang::Fix;
use stave_lang::ast::{Span, UnaryOp};
use stave_lang::units::{Angle, ExactAngle, ExactLength, Length, Rational};
use stave_model::defs::{AssemblyKind, BuiltinCall, DefId, DriveDef, RExpr, RExprKind, Symbol};
use stave_model::solved::AssemblyId;
use stave_model::types::Ty;
use stave_schema::functions::Builtin;

use crate::Checker;

/// How the overlay of a hinged front changes to clear what it hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverlayChange {
    /// The overlay on this edge (`left`, `right`, `top`, `bottom`) is this
    /// much smaller.
    Smaller {
        /// The edge, as `overlay(...)` names it.
        edge: &'static str,
        /// How much smaller.
        by: Length,
    },
    /// The `meeting:` overlay of two fronts that meet becomes this.
    Meeting(Length),
}

/// Where a front's `overlay(...)` is written, and how a patch reaches it.
struct OverlaySite<'m> {
    /// The path a `set` names: `door.mount`, `Door.face`.
    path: String,
    /// The whole value the `set` replaces.
    value: &'m RExpr,
    /// The `overlay(...)` call inside it (the value itself, for a mount).
    overlay: &'m RExpr,
    /// The file it is written in.
    file: stave_lang::FileId,
}

/// A length as source text, exactly: `16.8mm`, `-0.25mm`.
fn length_source(length: ExactLength) -> Option<String> {
    length.to_length().map(|length| length.to_string())
}

/// The value of a length written as a literal (`16mm`, `-1mm`).
fn literal_length(expr: &RExpr) -> Option<ExactLength> {
    match &expr.kind {
        RExprKind::Length(length) => Some(*length),
        RExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => literal_length(operand)?.checked_neg(),
        _ => None,
    }
}

/// The literal value of an angle (`110deg`).
fn literal_angle(expr: &RExpr) -> Option<ExactAngle> {
    match &expr.kind {
        RExprKind::Angle(angle) => Some(*angle),
        _ => None,
    }
}

/// The call, when `expr` is a call to `function`.
fn builtin_call(expr: &RExpr, function: Builtin) -> Option<&BuiltinCall> {
    match &expr.kind {
        RExprKind::Builtin(call) if call.function() == function => Some(call),
        _ => None,
    }
}

/// The overlay written on one edge of `overlay(...)`, by the edge's name
/// as [`OverlayChange`] carries it, and whether any edge is written.
fn overlay_edge<'e>(call: &'e BuiltinCall, edge: &str) -> (Option<&'e RExpr>, bool) {
    let BuiltinCall::Overlay {
        left,
        right,
        top,
        bottom,
        meeting,
    } = call
    else {
        return (None, false);
    };
    let found = match edge {
        "left" => left,
        "right" => right,
        "top" => top,
        "bottom" => bottom,
        "meeting" => meeting,
        _ => &None,
    };
    (found.as_ref(), call.arguments().next().is_some())
}

/// `text` (written at `span`) with the text at `inner` replaced, or, when
/// `inner` is empty, `insert` placed there.
fn splice(text: &str, span: Span, inner: Span, insert: &str) -> Option<String> {
    let start = usize::try_from(inner.start().checked_sub(span.start())?).ok()?;
    let end = usize::try_from(inner.end().checked_sub(span.start())?).ok()?;
    Some(format!(
        "{}{insert}{}",
        text.get(..start)?,
        text.get(end..)?
    ))
}

/// An expression written over several lines, on one: its lines joined by
/// spaces, with no space just inside a bracket and no comma before a
/// closing one.
fn one_line(text: &str) -> String {
    let joined = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    joined
        .replace("( ", "(")
        .replace(", )", ")")
        .replace(",)", ")")
}

impl Checker<'_> {
    /// Where the overlay of an assembly's front is written: its `Mount`
    /// argument, or else the one `cover(..., overlay(...))` region of its
    /// component. `None` when neither is in the project's own text.
    fn overlay_site(&self, assembly: AssemblyId) -> Option<OverlaySite<'_>> {
        let defs = &self.model.defs;
        let root = self.model.root;
        let inst = self.model.assemblies.get(assembly.index())?;
        let def = defs.assembly(inst.def)?;
        let AssemblyKind::Instance { component, args } = &def.kind else {
            return None;
        };
        if let Some((param, value)) = args.iter().find(|(_, value)| value.ty == Ty::Mount) {
            builtin_call(value, Builtin::Overlay)?;
            let name = &defs.param(*param)?.decl.name;
            let path = defs.patch_path(DefId::Assembly(inst.def), root)?;
            return Some(OverlaySite {
                path: format!("{path}.{name}"),
                value,
                overlay: value,
                file: def.decl.file,
            });
        }
        let mut found = defs
            .component(*component)?
            .items
            .iter()
            .filter_map(|item| match item {
                DefId::Region(id) => {
                    let region = defs.region(*id)?;
                    let overlay = builtin_call(&region.value, Builtin::Cover)?
                        .arguments()
                        .find(|value| builtin_call(value, Builtin::Overlay).is_some())?;
                    Some((*item, region, overlay))
                }
                _ => None,
            });
        let (item, region, overlay) = found.next()?;
        if found.next().is_some() {
            return None;
        }
        Some(OverlaySite {
            path: defs.patch_path(item, root)?,
            value: &region.value,
            overlay,
            file: region.decl.file,
        })
    }

    /// The fix that changes the overlay of `assembly`'s front as `change`
    /// says, as an exact length on that one edge.
    pub(crate) fn overlay_fix(&self, assembly: AssemblyId, change: OverlayChange) -> Option<Fix> {
        let site = self.overlay_site(assembly)?;
        let defs = &self.model.defs;
        let text = defs.source_text(site.file, site.value.span)?;
        let call = builtin_call(site.overlay, Builtin::Overlay)?;
        let (edge, new) = match change {
            OverlayChange::Smaller { edge, by } => {
                let old = match overlay_edge(call, edge).0 {
                    Some(value) => literal_length(value)?,
                    None => ExactLength::ZERO,
                };
                (edge, old.checked_sub(by.to_exact())?)
            }
            OverlayChange::Meeting(value) => ("meeting", value.to_exact()),
        };
        let new_text = length_source(new)?;
        let (written, any_written) = overlay_edge(call, edge);
        let edited = match written {
            Some(value) => splice(text, site.value.span, value.span, &new_text)?,
            None => {
                // Before the call's closing bracket.
                let close = site.overlay.span.end().checked_sub(1)?;
                let at = Span::new(close, close);
                let separator = if any_written { ", " } else { "" };
                splice(
                    text,
                    site.value.span,
                    at,
                    &format!("{separator}{edge}: {new_text}"),
                )?
            }
        };
        Fix::new(
            format!("Use {edge}: {new_text}"),
            &format!("set {} = {}", site.path, one_line(&edited)),
        )
        .ok()
    }

    /// The fix that opens `assembly` no further than just short of
    /// `at`, where it first hits: its hardware's kind's param that the
    /// drive turns to, set on each piece of hardware that moves it.
    /// `None` when that param's `within` doesn't allow such a value, or
    /// the hardware isn't written in the project's own text.
    pub(crate) fn opening_fix(&self, assembly: AssemblyId, at: Angle) -> Option<Fix> {
        let defs = &self.model.defs;
        let mechanism = self
            .design
            .assemblies()
            .get(assembly.index())?
            .mechanism
            .as_ref()?;
        // Rounded down to a tenth of a degree, and short of the hit.
        let tenth = 100_000;
        let mut limit = at.microdeg().div_euclid(tenth) * tenth;
        if limit == at.microdeg() {
            limit = limit.checked_sub(tenth)?;
        }
        let limit = Angle::from_microdeg(limit);
        let mut operations: Vec<String> = Vec::new();
        let mut value = None;
        for hardware in mechanism.hardware() {
            let found = self.design.hardware_piece(*hardware)?;
            let kind = defs.kind(found.kind_id)?;
            let to = match kind.mechanism()?.drive.as_ref() {
                Some(DriveDef::Turn { to, .. }) => to,
                // Hardware that follows the one that drives the assembly.
                None => continue,
                Some(DriveDef::Travel { .. }) => return None,
            };
            let RExprKind::Symbol(Symbol::KindParam(_, index)) = &to.kind else {
                return None;
            };
            let param = kind.params.get(*index)?;
            let allowed = allowed_below(param.domain.as_ref()?, limit)?;
            if value.is_some_and(|value| value != allowed) {
                return None;
            }
            value = Some(allowed);
            let inst = self.model.hardware.get(hardware.index())?;
            let def = defs.hardware_def(inst.def)?;
            // A setting a kind value gives elsewhere can't be given
            // again in the call.
            let written = def.call.named.iter().any(|s| s.name == param.decl.name);
            let plain = matches!(
                def.call.kind.kind,
                RExprKind::Symbol(Symbol::Def(DefId::Kind(_)))
            );
            if !written && !plain {
                return None;
            }
            let path = defs.patch_path(DefId::Hardware(inst.def), self.model.root)?;
            let text = format!("{path}.{}", param.decl.name);
            if !operations.contains(&text) {
                operations.push(text);
            }
        }
        let value = value?.to_angle()?;
        let operations: Vec<String> = operations
            .into_iter()
            .map(|path| format!("set {path} = {value}"))
            .collect();
        if operations.is_empty() {
            return None;
        }
        Fix::new(
            format!("Open no further than {value}"),
            &operations.join(", "),
        )
        .ok()
    }
}

/// The largest value a param's `within` allows that is at most `limit`:
/// `limit` itself for a range that holds it, or the largest value of a
/// list. Only a domain written with literal angles is read.
fn allowed_below(domain: &stave_model::defs::Domain, limit: Angle) -> Option<ExactAngle> {
    let limit = ExactAngle::new(Rational::from(limit.microdeg()));
    match domain {
        stave_model::defs::Domain::Range { low, high, .. } => {
            let (low, high) = (literal_angle(low)?, literal_angle(high)?);
            (low <= limit && limit <= high).then_some(limit)
        }
        stave_model::defs::Domain::OneOf { values, .. } => values
            .iter()
            .filter_map(literal_angle)
            .filter(|value| *value <= limit)
            .max(),
    }
}

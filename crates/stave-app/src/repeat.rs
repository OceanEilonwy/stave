//! Repeat and mirror (`docs/editing.md`, "Repeat and mirror"), as pure
//! functions from a project and a request to a patch in the `.stave`
//! grammar, which the scratch check ([`crate::scratch`]) then judges.
//!
//! - [`repeat_assembly`] turns an assembly that fills a region into an
//!   array of them: it splits the region into a `<component>_bay` array
//!   (equal, or with `shares:`) and binds each element with `[same]`. The
//!   array is labelled from the direction of the split for two or three
//!   pieces and counted for four or more ([`crate::naming::repeat_array`]).
//!   On an assembly that is already such an array, it changes the count of
//!   the same two declarations instead of adding a second set.
//! - [`repeat_part`] turns a part into an array spaced along a span with
//!   `spaced(evenly, from:, to:)`, or changes the count of one.
//! - [`move_shares`] is the grip between two neighbouring pieces of a
//!   split: it sets their two shares so the boundary between them lands
//!   where it was dragged (snapped to whole millimetres), keeping the sum
//!   of the two, so no other piece changes size.
//! - [`mirror_assembly`] finds the mirror image of an assembly about the
//!   mid-plane of what it is placed against or in, renames the assembly to
//!   its side (`door_left`) and adds its image (`door_right =
//!   mirror(door_left, about: ...)`).
//!
//! A patch here is text and its parse ([`GeneratedPatch`]). The patcher
//! doesn't apply `rename` (it needs every reference, which only the
//! navigation layer knows), so a mirror comes as the rename's text edits
//! ([`stave_ide::Analysis::rename`]) and a patch that applies after them.

use crate::i18n::{fill_in, tr, tr_noop};
use std::num::NonZeroU32;

use stave_ide::{Analysis, RenameEdit, RenameError};
use stave_lang::ast::{
    self, Arg, AssemblyDef, DesignItem, Expr, ExprKind, Ident, ItemKind, PartDef,
};
use stave_lang::placement::{Axis, Face};
use stave_lang::units::{Length, Rational, number_to_rational};
use stave_lang::{Diagnostic, Parse, SourceMap};
use stave_model::defs::{AssemblyKind, DefId};
use stave_model::solved::{Part, SolvedDesign, WorldBox};
use stave_model::types::Ty;

use crate::candidates::print::quoted;
use crate::candidates::{EditScope, ProjectFile};
use crate::naming::{NamingError, RepeatCount, TakenNames, bay_name, mirrored_pair, repeat_array};

// ---------------------------------------------------------------------------
// A generated patch
// ---------------------------------------------------------------------------

/// A patch the editor wrote: one `patch` in a patch file, as text and
/// parsed.
#[derive(Debug, Clone)]
pub struct GeneratedPatch {
    /// What the patch does, as the card shows it: "Make drawer 3".
    pub summary: String,
    /// The patch file's text: `stave 1` and one patch.
    pub text: String,
    /// The text parsed; it has no errors.
    pub parse: Parse<ast::PatchFile>,
}

/// What a [`PatchBuildError`] says.
const PATCH_BUG: &str = tr_noop("the written patch doesn't parse; please report this as a bug");

/// A generated patch didn't parse: a bug in the writer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", tr(PATCH_BUG))]
pub struct PatchBuildError(pub Vec<Diagnostic>);

impl GeneratedPatch {
    /// A patch with this summary and these operations, one per entry (an
    /// operation may span lines).
    ///
    /// # Errors
    ///
    /// [`PatchBuildError`] when the text doesn't parse.
    pub fn new(summary: &str, ops: &[String]) -> Result<Self, PatchBuildError> {
        let mut text = format!("stave 1\n\npatch {} {{\n", quoted(summary));
        for op in ops {
            for line in op.lines() {
                text.push_str("  ");
                text.push_str(line);
                text.push('\n');
            }
        }
        text.push_str("}\n");
        let mut sources = SourceMap::new();
        let file = sources
            .add("patch.stave", text.as_str())
            .map_err(|_| PatchBuildError(Vec::new()))?;
        let parse = stave_lang::parse_patch_file(file, &text);
        if parse.has_errors() {
            return Err(PatchBuildError(parse.diagnostics));
        }
        Ok(Self {
            summary: summary.to_owned(),
            text,
            parse,
        })
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a repeat, a shares grip or a mirror can't be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepeatError {
    /// The project has no top-level item by that name.
    #[error("{}", self.text())]
    UnknownItem(String),
    /// The item isn't of the sort the request needs.
    #[error("{}", self.text())]
    NotRepeatable {
        /// The item.
        name: String,
        /// Why, in words.
        reason: String,
    },
    /// The repeat already has that many pieces.
    #[error("{}", self.text())]
    SameCount {
        /// The item.
        name: String,
        /// Its count.
        count: usize,
    },
    /// A first repeat needs the axis to split along.
    #[error("{}", self.text())]
    NoAxis(String),
    /// `shares:` with the wrong number of entries.
    #[error("{}", self.text())]
    SharesCount {
        /// How many pieces.
        pieces: usize,
        /// How many shares were given.
        found: usize,
    },
    /// The split's shares are not plain numbers, so a grip can't move them.
    #[error("{}", self.text())]
    SharesNotNumbers(String),
    /// The grip names pieces the split doesn't have.
    #[error("{}", self.text())]
    NoSuchPieces {
        /// The region.
        region: String,
        /// The first piece's index.
        lower: usize,
        /// The second's.
        upper: usize,
    },
    /// The boundary is dragged onto or past the far side of a piece.
    #[error("{}", self.text())]
    BoundaryOutside {
        /// The two pieces' sizes together.
        pair: Length,
    },
    /// The design has no solved region by that name (it didn't solve).
    #[error("{}", self.text())]
    NotSolved(String),
    /// No mirror image was found for the assembly.
    #[error("{}", self.text())]
    NoMirror(String),
    /// The rename a mirror makes was refused.
    #[error(transparent)]
    Rename(#[from] RenameError),
    /// A generated name couldn't be made.
    #[error(transparent)]
    Naming(#[from] NamingError),
    /// Arithmetic that doesn't fit the exact numbers (a bug, or sizes of
    /// kilometres).
    #[error("{}", self.text())]
    Overflow,
    /// The written patch didn't parse: a bug.
    #[error(transparent)]
    Build(#[from] PatchBuildError),
}

impl RepeatError {
    /// The error in the window's words (translated).
    fn text(&self) -> String {
        match self {
            RepeatError::UnknownItem(v0) => fill_in(
                &tr("there is no item named '{value}' in this project"),
                &[("value", v0)],
            ),
            RepeatError::NotRepeatable { name, reason } => fill_in(
                &tr("'{name}' can't be repeated this way: {reason}"),
                &[("name", name), ("reason", reason)],
            ),
            RepeatError::SameCount { name, count } => fill_in(
                &tr("'{name}' already has {count} pieces"),
                &[("name", name), ("count", count)],
            ),
            RepeatError::NoAxis(v0) => fill_in(
                &tr("say which way to split the space for '{value}': across, up or deep"),
                &[("value", v0)],
            ),
            RepeatError::SharesCount { pieces, found } => fill_in(
                &tr("{pieces} pieces need {pieces} shares, found {found}"),
                &[("pieces", pieces), ("found", found)],
            ),
            RepeatError::SharesNotNumbers(v0) => fill_in(
                &tr(
                    "the shares of '{value}' are not plain numbers, so they can't be moved by a grip",
                ),
                &[("value", v0)],
            ),
            RepeatError::NoSuchPieces {
                region,
                lower,
                upper,
            } => fill_in(
                &tr("'{region}' has no pieces {lower} and {upper}"),
                &[("region", region), ("lower", lower), ("upper", upper)],
            ),
            RepeatError::BoundaryOutside { pair } => fill_in(
                &tr(
                    "the boundary must leave both pieces more than 0mm; it can move between 0mm and {pair}",
                ),
                &[("pair", pair)],
            ),
            RepeatError::NotSolved(v0) => fill_in(
                &tr(
                    "the design has no solved space named '{value}'; it must solve before a grip can move it",
                ),
                &[("value", v0)],
            ),
            RepeatError::NoMirror(v0) => fill_in(
                &tr(
                    "'{value}' isn't placed against a part that has a mirror image, or in one of two matching spaces, so it has no mirror image to add",
                ),
                &[("value", v0)],
            ),
            RepeatError::Rename(error) => error.to_string(),
            RepeatError::Naming(error) => error.to_string(),
            RepeatError::Overflow => tr("the sizes are too large to work with exactly"),
            RepeatError::Build(error) => error.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Reading the project's text
// ---------------------------------------------------------------------------

/// A top-level item of the project, with the text of its file.
pub(crate) struct Found<'s> {
    pub(crate) file: &'s ProjectFile,
    pub(crate) text: String,
    pub(crate) item: &'s ast::Item,
}

impl Found<'_> {
    pub(crate) fn slice(&self, span: ast::Span) -> Option<&str> {
        slice(&self.text, span)
    }
}

pub(crate) fn slice(text: &str, span: ast::Span) -> Option<&str> {
    let start = usize::try_from(span.start()).ok()?;
    let end = usize::try_from(span.end()).ok()?;
    text.get(start..end)
}

/// The project's top-level item named `name`.
pub(crate) fn find<'s>(scope: &'s EditScope<'_>, name: &str) -> Option<Found<'s>> {
    scope.files().iter().find_map(|file| {
        let item = file
            .parse
            .ast
            .as_ref()?
            .items
            .iter()
            .find(|item| item.kind.name().is_some_and(|n| n.as_str() == name))?;
        Some(Found {
            file,
            text: file.parse.syntax().to_string(),
            item,
        })
    })
}

/// A `split(region, axis, ...)` call, read from the text.
struct Split<'e> {
    source: &'e Expr,
    axis: Axis,
    shares: Option<&'e Expr>,
    at: bool,
}

fn split_of(expr: &Expr) -> Option<Split<'_>> {
    let ExprKind::Call { callee, args } = &expr.kind else {
        return None;
    };
    let ExprKind::Name(head) = &callee.kind else {
        return None;
    };
    if head.as_str() != "split" {
        return None;
    }
    let mut positional = args.iter().filter(|arg| arg.name.is_none());
    let source = &positional.next()?.value;
    let axis = match &positional.next()?.value.kind {
        ExprKind::Name(word) => Axis::from_word(word.as_str())?,
        _ => return None,
    };
    let named = |key: &str| {
        args.iter()
            .find(|arg| arg.name.as_ref().is_some_and(|n| n.as_str() == key))
            .map(|arg: &Arg| &arg.value)
    };
    Some(Split {
        source,
        axis,
        shares: named("shares"),
        at: named("at").is_some(),
    })
}

/// The shares of a split as exact numbers, when each is a plain number.
fn share_values(shares: &Expr) -> Option<Vec<Rational>> {
    let ExprKind::List(items) = &shares.kind else {
        return None;
    };
    items
        .iter()
        .map(|item| match &item.kind {
            ExprKind::Number(number) => number_to_rational(*number).ok(),
            _ => None,
        })
        .collect()
}

/// How many elements an array declaration has, when it is written as
/// labels or a whole number.
fn array_count(spec: &ast::ArraySpec) -> Option<usize> {
    match spec {
        ast::ArraySpec::Labels(labels) => Some(labels.as_slice().len()),
        ast::ArraySpec::Count(expr) => match &expr.kind {
            ExprKind::Number(ast::Number::Integer(n)) => usize::try_from(*n).ok(),
            _ => None,
        },
    }
}

/// The element names of an array declaration, in order: its labels, or
/// its numbers from 0.
fn array_indexes(spec: &ast::ArraySpec) -> Option<Vec<String>> {
    match spec {
        ast::ArraySpec::Labels(labels) => Some(
            labels
                .as_slice()
                .iter()
                .map(|label| label.as_str().to_owned())
                .collect(),
        ),
        ast::ArraySpec::Count(_) => {
            array_count(spec).map(|count| (0..count).map(|n| n.to_string()).collect())
        }
    }
}

/// The name of the first `Region` param of the component an assembly of
/// the project's module is an instance of.
fn region_param(scope: &EditScope<'_>, assembly: &str) -> Option<String> {
    let model = scope.model();
    let defs = &model.defs;
    let module = defs.modules.get(model.root.index())?;
    let component = module.items.iter().find_map(|item| match item {
        DefId::Assembly(id) => {
            let def = defs.assembly(*id)?;
            if def.decl.name != assembly {
                return None;
            }
            match &def.kind {
                AssemblyKind::Instance { component, .. } => Some(*component),
                AssemblyKind::Mirror(_) | AssemblyKind::Invalid => None,
            }
        }
        _ => None,
    })?;
    defs.component(component)?
        .params
        .iter()
        .filter_map(|param| defs.param(*param))
        .find(|param| param.ty == Ty::Region)
        .map(|param| param.decl.name.clone())
}

/// `, shares: [a, b, c]`, or nothing.
fn shares_text(shares: Option<&[NonZeroU32]>) -> String {
    shares.map_or_else(String::new, |shares| {
        let list: Vec<String> = shares.iter().map(ToString::to_string).collect();
        format!(", shares: [{}]", list.join(", "))
    })
}

// ---------------------------------------------------------------------------
// Repeat
// ---------------------------------------------------------------------------

/// A repeat of an assembly that fills a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepeatRequest<'a> {
    /// The assembly: a single one, or the array of an earlier repeat.
    pub assembly: &'a str,
    /// How many pieces it becomes.
    pub count: RepeatCount,
    /// The axis to split the region along. A first repeat needs it; on an
    /// existing repeat the split's own axis is kept and this is ignored.
    pub axis: Option<Axis>,
    /// Relative sizes of the pieces, one per piece in label order; `None`
    /// for equal pieces.
    pub shares: Option<&'a [NonZeroU32]>,
}

/// The patch that turns an assembly into `count` of them filling its
/// region, or changes the count of an existing repeat (see the module's
/// notes). Labels are regenerated for the new count by the naming rule;
/// an existing repeat's `shares:` are dropped unless new ones are given,
/// since they were for a different number of pieces.
///
/// # Errors
///
/// An unknown item, an item that isn't an assembly filling a region, a
/// repeat to the count it already has, a first repeat with no axis, or
/// shares of the wrong number; see [`RepeatError`].
pub fn repeat_assembly(
    scope: &EditScope<'_>,
    request: RepeatRequest<'_>,
) -> Result<GeneratedPatch, RepeatError> {
    let name = request.assembly;
    let not = |reason: &str| RepeatError::NotRepeatable {
        name: name.to_owned(),
        reason: reason.to_owned(),
    };
    let found = find(scope, name).ok_or_else(|| RepeatError::UnknownItem(name.to_owned()))?;
    let ItemKind::Design(DesignItem::Assembly(assembly)) = &found.item.kind else {
        return Err(not("only an assembly of a component fills a space"));
    };
    let AssemblyDef::Instance { component, body } = &assembly.def else {
        return Err(not("it is a mirror image; repeat the assembly it mirrors"));
    };
    if let Some(shares) = request.shares
        && shares.len() != request.count.get()
    {
        return Err(RepeatError::SharesCount {
            pieces: request.count.get(),
            found: shares.len(),
        });
    }
    let param = region_param(scope, name)
        .ok_or_else(|| not("its component has no space (Region) param to fill"))?;
    let field = body
        .field(&param)
        .ok_or_else(|| not("its space param is not written"))?;
    let component_text = found.slice(component.span).ok_or(RepeatError::Overflow)?;
    let body_text = found.slice(body.span).ok_or(RepeatError::Overflow)?;

    match &assembly.array {
        None => {
            let axis = request
                .axis
                .ok_or_else(|| RepeatError::NoAxis(name.to_owned()))?;
            let array = repeat_array(axis, request.count);
            let source = found.slice(field.value.span).ok_or(RepeatError::Overflow)?;
            let mut taken = TakenNames::of_project(scope.model());
            taken.insert(name);
            let component_name = component
                .rest
                .last()
                .map_or(component.root.as_str(), ast::Name::as_str);
            let bay = bay_name(component_name, &taken)?;
            let declaration = array.declaration();
            let ops = vec![
                format!(
                    "add region {}{declaration} = split({source}, {}{})",
                    bay.as_str(),
                    axis.word(),
                    shares_text(request.shares)
                ),
                format!("replace assembly {name}{declaration} : {component_text} {body_text}"),
                format!("set {name}.{param} = {}[same]", bay.as_str()),
            ];
            let summary = format!("Make {} of {name}", request.count.get());
            Ok(GeneratedPatch::new(&summary, &ops)?)
        }
        Some(spec) => {
            // An earlier repeat: the region it fills is `bay[same]`.
            let ExprKind::Index {
                base,
                index: ast::Index::Same(_),
            } = &field.value.kind
            else {
                return Err(not(
                    "its space is not an element of a split, bound with [same]",
                ));
            };
            let ExprKind::Name(bay) = &base.kind else {
                return Err(not("its space is not a split of this project"));
            };
            let region = find(scope, bay.as_str())
                .ok_or_else(|| not("its space is not a split of this project"))?;
            let ItemKind::Design(DesignItem::Region(region_item)) = &region.item.kind else {
                return Err(not("its space is not a region"));
            };
            let split = split_of(&region_item.value)
                .filter(|split| !split.at)
                .ok_or_else(|| not("its space is not an even split or a split with shares"))?;
            let current =
                array_count(spec).ok_or_else(|| not("its count is not a fixed number"))?;
            if current == request.count.get() && request.shares.is_none() {
                return Err(RepeatError::SameCount {
                    name: name.to_owned(),
                    count: current,
                });
            }
            let array = repeat_array(split.axis, request.count);
            let declaration = array.declaration();
            let source = region
                .slice(split.source.span)
                .ok_or(RepeatError::Overflow)?;
            let ops = vec![
                format!(
                    "replace region {}{declaration} = split({source}, {}{})",
                    bay.as_str(),
                    split.axis.word(),
                    shares_text(request.shares)
                ),
                format!("replace assembly {name}{declaration} : {component_text} {body_text}"),
            ];
            let summary = format!("Make {} of {name}", request.count.get());
            Ok(GeneratedPatch::new(&summary, &ops)?)
        }
    }
}

/// A repeat of a part spaced along a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpacedRequest<'a> {
    /// The part: a single one, or an array.
    pub part: &'a str,
    /// How many it becomes.
    pub count: RepeatCount,
    /// The axis the pieces are spaced along, which names the array's
    /// labels for two or three.
    pub axis: Axis,
    /// The anchor the span starts at: `apron_l.right`.
    pub from: &'a str,
    /// The anchor it ends at: `apron_r.left`.
    pub to: &'a str,
}

/// The patch that turns a part into `count` of them spaced evenly from one
/// anchor to another: the part becomes an array (same stock, same body)
/// and its placement term on the axis becomes `spaced(evenly, from:,
/// to:)`. On a part that is already an array it changes the count of the
/// same declaration.
///
/// # Errors
///
/// An unknown item, or one that isn't a part cut from stock; see
/// [`RepeatError`].
pub fn repeat_part(
    scope: &EditScope<'_>,
    request: SpacedRequest<'_>,
) -> Result<GeneratedPatch, RepeatError> {
    let name = request.part;
    let not = |reason: &str| RepeatError::NotRepeatable {
        name: name.to_owned(),
        reason: reason.to_owned(),
    };
    let found = find(scope, name).ok_or_else(|| RepeatError::UnknownItem(name.to_owned()))?;
    let ItemKind::Design(DesignItem::Part(part)) = &found.item.kind else {
        return Err(not("only a part can be spaced along a span"));
    };
    let PartDef::FromStock { stock, body } = &part.def else {
        return Err(not("it is a mirror image; repeat the part it mirrors"));
    };
    if let Some(spec) = &part.array
        && array_count(spec) == Some(request.count.get())
    {
        return Err(RepeatError::SameCount {
            name: name.to_owned(),
            count: request.count.get(),
        });
    }
    let declaration = repeat_array(request.axis, request.count).declaration();
    let stock_text = found.slice(stock.span).ok_or(RepeatError::Overflow)?;
    let body_text = found.slice(body.span).ok_or(RepeatError::Overflow)?;
    let ops = vec![
        format!("replace part {name}{declaration} : {stock_text} {body_text}"),
        format!(
            "set {name}.place.{} = spaced(evenly, from: {}, to: {})",
            request.axis.word(),
            request.from,
            request.to
        ),
    ];
    let summary = format!("Make {} of {name}", request.count.get());
    Ok(GeneratedPatch::new(&summary, &ops)?)
}

// ---------------------------------------------------------------------------
// The shares grip
// ---------------------------------------------------------------------------

/// The grip between two neighbouring pieces of a split, let go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharesMove<'a> {
    /// The region array the split makes: `drawer_bay`.
    pub region: &'a str,
    /// The index of the first of the two pieces, in label order; the
    /// other is the next one.
    pub lower: usize,
    /// Where the boundary between them goes: the new size of the first
    /// piece along the split's axis, from its start. Snapped to the
    /// project's resolution.
    pub boundary: Length,
}

/// The patch that moves the boundary between two neighbouring pieces of
/// a split: the region's declaration again with only those two shares
/// changed and their sum kept, so the pieces around them keep their
/// sizes. The boundary is snapped to the project's `resolution:`; every
/// piece's size (from the solved design, the two moved ones as moved) is
/// counted in steps of the resolution, and the counts, divided by what
/// they have in common, are the shares: small whole numbers (`[6, 9,
/// 5]` for pieces of 150mm, 225mm and 125mm at 0.5mm). A piece whose
/// size is not a whole number of steps is rounded to the nearest step.
///
/// # Errors
///
/// An unknown region, one that is not a split array, shares that are not
/// plain numbers, pieces it doesn't have, a region the design didn't
/// solve, or a boundary that leaves a piece 0mm or less; see
/// [`RepeatError`].
pub fn move_shares(
    scope: &EditScope<'_>,
    design: &SolvedDesign,
    grip: SharesMove<'_>,
) -> Result<GeneratedPatch, RepeatError> {
    let name = grip.region;
    let not = |reason: &str| RepeatError::NotRepeatable {
        name: name.to_owned(),
        reason: reason.to_owned(),
    };
    let found = find(scope, name).ok_or_else(|| RepeatError::UnknownItem(name.to_owned()))?;
    let ItemKind::Design(DesignItem::Region(region)) = &found.item.kind else {
        return Err(not("only a region split into pieces has shares"));
    };
    let spec = region
        .array
        .as_ref()
        .ok_or_else(|| not("it is one space, not split into pieces"))?;
    let split = split_of(&region.value)
        .filter(|split| !split.at)
        .ok_or_else(|| not("it is not an even split or a split with shares"))?;
    let indexes = array_indexes(spec).ok_or_else(|| not("its count is not a fixed number"))?;
    // The shares must be plain numbers for a grip to read them back.
    if let Some(expr) = split.shares
        && share_values(expr).is_none()
    {
        return Err(RepeatError::SharesNotNumbers(name.to_owned()));
    }
    let upper = grip.lower.checked_add(1).ok_or(RepeatError::Overflow)?;
    let no_pieces = || RepeatError::NoSuchPieces {
        region: name.to_owned(),
        lower: grip.lower,
        upper,
    };
    let (Some(lower_label), Some(upper_label)) = (indexes.get(grip.lower), indexes.get(upper))
    else {
        return Err(no_pieces());
    };
    let size = |label: &str| {
        let piece = format!("{name}[{label}]");
        design
            .regions
            .iter()
            .find(|region| region.name == piece)
            .map(|region| region.bounds.size(split.axis))
            .ok_or(RepeatError::NotSolved(piece))
    };
    let mut sizes = indexes
        .iter()
        .map(|label| size(label))
        .collect::<Result<Vec<Length>, RepeatError>>()?;
    let (Some(a), Some(b)) = (sizes.get(grip.lower), sizes.get(upper)) else {
        return Err(no_pieces());
    };
    let pair = a.checked_add(*b).ok_or(RepeatError::Overflow)?;
    // Sizes are counted in steps of the project's resolution.
    let unit = design.project.resolution.nm().max(1);
    let units = |nm: i64| -> Option<i128> {
        let nm = i128::from(nm);
        let unit = i128::from(unit);
        nm.checked_add(unit / 2)?.checked_div(unit)
    };
    let snapped = units(grip.boundary.nm())
        .and_then(|n| n.checked_mul(i128::from(unit)))
        .and_then(|nm| i64::try_from(nm).ok())
        .ok_or(RepeatError::Overflow)?;
    if snapped <= 0 || snapped >= pair.nm() {
        return Err(RepeatError::BoundaryOutside { pair });
    }
    let new_upper = pair
        .nm()
        .checked_sub(snapped)
        .ok_or(RepeatError::Overflow)?;
    if let Some(slot) = sizes.get_mut(grip.lower) {
        *slot = Length::from_nm(snapped);
    }
    if let Some(slot) = sizes.get_mut(upper) {
        *slot = Length::from_nm(new_upper);
    }
    let counted = sizes
        .iter()
        .map(|size| units(size.nm()).filter(|n| *n > 0))
        .collect::<Option<Vec<i128>>>()
        .ok_or(RepeatError::BoundaryOutside { pair })?;
    let list: Vec<String> = reduced(&counted)
        .ok_or(RepeatError::Overflow)?
        .iter()
        .map(i128::to_string)
        .collect();
    // The whole declaration is replaced: the patcher doesn't reach into a
    // region's call by `set <region>.shares` yet.
    let declaration = match spec {
        ast::ArraySpec::Labels(_) => format!("[{}]", indexes.join(", ")),
        ast::ArraySpec::Count(_) => format!("[{}]", indexes.len()),
    };
    let source = found
        .slice(split.source.span)
        .ok_or(RepeatError::Overflow)?;
    let ops = vec![format!(
        "replace region {name}{declaration} = split({source}, {}, shares: [{}])",
        split.axis.word(),
        list.join(", ")
    )];
    let summary =
        format!("Move the boundary between {name}[{lower_label}] and {name}[{upper_label}]");
    Ok(GeneratedPatch::new(&summary, &ops)?)
}

/// Whole numbers divided by what they all have in common: `[300, 450,
/// 250]` gives `[6, 9, 5]`. `None` when they are all zero.
fn reduced(numbers: &[i128]) -> Option<Vec<i128>> {
    fn gcd(mut a: i128, mut b: i128) -> i128 {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a.abs()
    }
    let common = numbers.iter().fold(0, |acc, n| gcd(acc, *n));
    if common == 0 {
        return None;
    }
    Some(numbers.iter().map(|n| n / common).collect())
}

// ---------------------------------------------------------------------------
// Mirror
// ---------------------------------------------------------------------------

/// A mirror of an assembly: the rename of the original to its side, and
/// the patch that adds its image, which applies to the files as the
/// rename leaves them.
#[derive(Debug, Clone)]
pub struct MirrorPlan {
    /// The assembly's name before.
    pub source: String,
    /// The name it takes: `door_left`.
    pub renamed: Ident,
    /// The image's name: `door_right`.
    pub image: Ident,
    /// The axis it is mirrored across.
    pub axis: Axis,
    /// The plane it is mirrored about, as written: `mid(base, across)`.
    pub about: String,
    /// The rename's edits, per file.
    pub rename: Vec<RenameEdit>,
    /// The patch adding the image; apply it after [`MirrorPlan::rename`].
    pub patch: GeneratedPatch,
}

/// The mirror image of an assembly, when it has one: it is placed against
/// a part that has a mirror image (`side_r = mirror(side_l, about: M)`, so
/// about `M`), or fills a piece of a split whose matching piece lies at
/// the same place from the other end (`door_bay[left]` of an even split
/// across, about the split region's mid-plane). Parts are looked for in
/// every place param before regions are. Failing both, it is placed
/// against parts that each have a counterpart of the same size reflected
/// about one plane, such as an apron between the two front legs of a
/// `Legs` (whose back legs are the front ones reflected), about the
/// mid-plane of a part it names that the plane halves (`mid(top.board,
/// deep)`); see [`reflected_mirror`]. The original is renamed with its
/// side and the image takes the other (`door_left`, `door_right`).
///
/// # Errors
///
/// An unknown item, one that isn't an assembly of a component, one with
/// no mirror image, or a rename that is refused; see [`RepeatError`].
pub fn mirror_assembly(
    scope: &EditScope<'_>,
    analysis: &Analysis,
    design: &SolvedDesign,
    assembly: &str,
) -> Result<MirrorPlan, RepeatError> {
    let not = |reason: &str| RepeatError::NotRepeatable {
        name: assembly.to_owned(),
        reason: reason.to_owned(),
    };
    let found =
        find(scope, assembly).ok_or_else(|| RepeatError::UnknownItem(assembly.to_owned()))?;
    let ItemKind::Design(DesignItem::Assembly(item)) = &found.item.kind else {
        return Err(not("only an assembly is mirrored this way"));
    };
    if item.array.is_some() {
        return Err(not("it is an array; mirror a single assembly"));
    }
    let AssemblyDef::Instance { body, .. } = &item.def else {
        return Err(not("it is already a mirror image"));
    };
    if let Some(image) = mirror_image_of(scope, assembly) {
        return Err(not(&format!("it already has a mirror image, {image}")));
    }
    let (axis, about, first_side) = body
        .fields
        .iter()
        .find_map(|field| part_mirror(scope, design, &field.value))
        .or_else(|| {
            body.fields
                .iter()
                .find_map(|field| split_mirror(scope, &field.value))
        })
        .or_else(|| {
            let values: Vec<&Expr> = body.fields.iter().map(|field| &field.value).collect();
            reflected_mirror(&found, design, assembly, &values)
        })
        .ok_or_else(|| RepeatError::NoMirror(assembly.to_owned()))?;

    let mut taken = TakenNames::of_project(scope.model());
    // Its own name doesn't stand in the way: `door_left` stays `door_left`.
    taken.remove(assembly);
    let (first, last) = mirrored_pair(assembly, axis, &taken)?;
    let (renamed, image) = if first_side {
        (first, last)
    } else {
        (last, first)
    };
    let symbol = analysis
        .project_symbol(assembly)
        .ok_or_else(|| RepeatError::UnknownItem(assembly.to_owned()))?;
    let rename = if renamed.as_str() == assembly {
        Vec::new()
    } else {
        analysis.rename(symbol.id, renamed.as_str())?
    };
    let ops = vec![format!(
        "add assembly {} = mirror({}, about: {about}) in {}",
        image.as_str(),
        renamed.as_str(),
        quoted(&found.file.name)
    )];
    let summary = format!("Add the mirror image of {assembly}");
    let patch = GeneratedPatch::new(&summary, &ops)?;
    Ok(MirrorPlan {
        source: assembly.to_owned(),
        renamed,
        image,
        axis,
        about,
        rename,
        patch,
    })
}

/// A mirrored pair made from an assembly that fills a whole space: the
/// space is split in two along `axis` into `<component>_bay` (as a repeat
/// of two splits it), the assembly is renamed with the first side and
/// fills the first piece, and its mirror image, about the space's
/// mid-plane, fills the other. This is "repeat across as a mirrored
/// pair": a door hinged on the left side becomes `door_left`, and
/// `door_right` hangs on the right, where a plain repeat would hang both
/// on the left.
///
/// ```text
/// region door_bay[left, right] = split(divider.above, across)
/// assembly door_left : FramePanelDoor { opening: door_bay[left] ... }
/// assembly door_right = mirror(door_left, about: mid(divider.above, across))
/// ```
///
/// # Errors
///
/// An unknown item, one that isn't a single assembly of a component, one
/// whose component has no space (`Region`) param, one that already fills
/// one of two matching pieces of a split (mirror it without an axis), or
/// a rename that is refused; see [`RepeatError`].
pub fn mirror_split_assembly(
    scope: &EditScope<'_>,
    analysis: &Analysis,
    assembly: &str,
    axis: Axis,
) -> Result<MirrorPlan, RepeatError> {
    let not = |reason: &str| RepeatError::NotRepeatable {
        name: assembly.to_owned(),
        reason: reason.to_owned(),
    };
    let found =
        find(scope, assembly).ok_or_else(|| RepeatError::UnknownItem(assembly.to_owned()))?;
    let ItemKind::Design(DesignItem::Assembly(item)) = &found.item.kind else {
        return Err(not("only an assembly is mirrored this way"));
    };
    if item.array.is_some() {
        return Err(not("it is an array; mirror a single assembly"));
    }
    let AssemblyDef::Instance { component, body } = &item.def else {
        return Err(not("it is already a mirror image"));
    };
    if let Some(image) = mirror_image_of(scope, assembly) {
        return Err(not(&format!("it already has a mirror image, {image}")));
    }
    let param = region_param(scope, assembly)
        .ok_or_else(|| not("its component has no space (Region) param to fill"))?;
    let field = body
        .field(&param)
        .ok_or_else(|| not("its space param is not written"))?;
    if split_mirror(scope, &field.value).is_some() {
        return Err(not(
            "it already fills one of two matching spaces; mirror it without saying which way",
        ));
    }
    let source = found.slice(field.value.span).ok_or(RepeatError::Overflow)?;

    let mut taken = TakenNames::of_project(scope.model());
    let component_name = component
        .rest
        .last()
        .map_or(component.root.as_str(), ast::Name::as_str);
    let bay = bay_name(component_name, &taken)?;
    taken.insert(bay.as_str());
    taken.remove(assembly);
    let (renamed, image) = mirrored_pair(assembly, axis, &taken)?;
    let pieces = RepeatCount::new(2).ok_or(RepeatError::Overflow)?;
    let array = repeat_array(axis, pieces);
    let first = array
        .indexes()
        .into_iter()
        .next()
        .ok_or(RepeatError::Overflow)?;
    let symbol = analysis
        .project_symbol(assembly)
        .ok_or_else(|| RepeatError::UnknownItem(assembly.to_owned()))?;
    let rename = if renamed.as_str() == assembly {
        Vec::new()
    } else {
        analysis.rename(symbol.id, renamed.as_str())?
    };
    let about = format!("mid({source}, {})", axis.word());
    let ops = vec![
        format!(
            "add region {}{} = split({source}, {})",
            bay.as_str(),
            array.declaration(),
            axis.word()
        ),
        format!(
            "set {}.{param} = {}[{first}]",
            renamed.as_str(),
            bay.as_str()
        ),
        format!(
            "add assembly {} = mirror({}, about: {about}) in {}",
            image.as_str(),
            renamed.as_str(),
            quoted(&found.file.name)
        ),
    ];
    let summary = format!("Make {assembly} a mirrored pair");
    let patch = GeneratedPatch::new(&summary, &ops)?;
    Ok(MirrorPlan {
        source: assembly.to_owned(),
        renamed,
        image,
        axis,
        about,
        rename,
        patch,
    })
}

/// The name of the item of the project that is the mirror image of
/// `name` (`door_right = mirror(door_left, ...)`), if there is one.
pub(crate) fn mirror_image_of(scope: &EditScope<'_>, name: &str) -> Option<String> {
    scope.files().iter().find_map(|file| {
        file.parse.ast.as_ref()?.items.iter().find_map(|item| {
            let source = match &item.kind {
                ItemKind::Design(DesignItem::Assembly(assembly)) => match &assembly.def {
                    AssemblyDef::Mirror(mirror) => &mirror.source,
                    AssemblyDef::Instance { .. } => return None,
                },
                ItemKind::Design(DesignItem::Part(part)) => match &part.def {
                    PartDef::Mirror(mirror) => &mirror.source,
                    PartDef::FromStock { .. } => return None,
                },
                _ => return None,
            };
            let path = source.as_path()?;
            if path.root.as_str() != name || !path.segments.is_empty() {
                return None;
            }
            item.kind.name().map(|image| image.as_str().to_owned())
        })
    })
}

/// Whether an assembly that fills a space is handed along `axis`: one of
/// its place params that takes a single part or face (a door's
/// `hinge_side`) is bound to one off the middle of that space. Only then
/// does a mirrored pair across the space differ from a plain repeat: a
/// door hinged on the left side gets a partner hinged on the right. A
/// top, a case, a shelf or a divider, whose places are the space itself
/// or both sides at once, is the same either way round.
pub(crate) fn handed(
    scope: &EditScope<'_>,
    design: &SolvedDesign,
    assembly: &str,
    axis: Axis,
) -> bool {
    let model = scope.model();
    let Some(space) = crate::spaces::filled_spaces(model, assembly)
        .first()
        .and_then(|region| design.regions.iter().find(|found| found.id == *region))
    else {
        return false;
    };
    let Some((middle, _)) = doubled_centre(&space.bounds, axis) else {
        return false;
    };
    let Some(found) = find(scope, assembly) else {
        return false;
    };
    let ItemKind::Design(DesignItem::Assembly(item)) = &found.item.kind else {
        return false;
    };
    let AssemblyDef::Instance { body, .. } = &item.def else {
        return false;
    };
    let single = single_place_params(scope, assembly);
    body.fields
        .iter()
        .filter(|field| single.iter().any(|name| name == field.key.as_str()))
        .flat_map(|field| named_parts(&found, design, &field.value))
        .any(|part| doubled_centre(&drawn_box(part), axis).is_some_and(|(c, _)| c != middle))
}

/// The names of an assembly's component params that take a single part
/// or face (`Part`, `Anchor`).
fn single_place_params(scope: &EditScope<'_>, assembly: &str) -> Vec<String> {
    let model = scope.model();
    let defs = &model.defs;
    let Some(module) = defs.modules.get(model.root.index()) else {
        return Vec::new();
    };
    let component = module.items.iter().find_map(|item| match item {
        DefId::Assembly(id) => {
            let def = defs.assembly(*id)?;
            if def.decl.name != assembly {
                return None;
            }
            match &def.kind {
                AssemblyKind::Instance { component, .. } => Some(*component),
                AssemblyKind::Mirror(_) | AssemblyKind::Invalid => None,
            }
        }
        _ => None,
    });
    component
        .and_then(|component| defs.component(component))
        .map(|component| {
            component
                .params
                .iter()
                .filter_map(|param| defs.param(*param))
                .filter(|param| matches!(param.ty, Ty::Part | Ty::Anchor))
                .map(|param| param.decl.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// A place param bound to a part (or a face of one) that has a mirror
/// image: the axis, the plane as written, and whether the part is on the
/// side nearer the start of the axis.
fn part_mirror(
    scope: &EditScope<'_>,
    design: &SolvedDesign,
    value: &Expr,
) -> Option<(Axis, String, bool)> {
    let path = value.as_path()?;
    let name = path.root.as_str();
    // Only a part by its own name, or a face of one.
    match path.segments.as_slice() {
        [] => {}
        [ast::PathSegment::Member(member)] if Face::from_word(member.as_str()).is_some() => {}
        _ => return None,
    }
    let found = find(scope, name)?;
    let ItemKind::Design(DesignItem::Part(part)) = &found.item.kind else {
        return None;
    };
    // `name = mirror(other, about: M)`, or `other = mirror(name, about: M)`.
    let (other, about) = match &part.def {
        PartDef::Mirror(mirror) => {
            let source = mirror.source.as_path()?;
            (
                source.root.as_str().to_owned(),
                found.slice(mirror.about.span)?.to_owned(),
            )
        }
        PartDef::FromStock { .. } => scope.files().iter().find_map(|file| {
            let text = file.parse.syntax().to_string();
            file.parse.ast.as_ref()?.items.iter().find_map(|item| {
                let ItemKind::Design(DesignItem::Part(other)) = &item.kind else {
                    return None;
                };
                let PartDef::Mirror(mirror) = &other.def else {
                    return None;
                };
                let source = mirror.source.as_path()?;
                (source.root.as_str() == name && source.segments.is_empty()).then(|| {
                    Some((
                        other.name.as_str().to_owned(),
                        slice(&text, mirror.about.span)?.to_owned(),
                    ))
                })?
            })
        })?,
    };
    let axis = plane_axis(&about)?;
    let centre = |part: &str| {
        let part = design.part_named(part)?;
        let bounds = part.pose.world_box(part.drawn);
        bounds
            .min()
            .get(axis)
            .nm()
            .checked_add(bounds.max().get(axis).nm())
    };
    Some((axis, about, centre(name)? < centre(&other)?))
}

/// A place param bound to a piece of an even split that has a matching
/// piece at the other end: the axis, the plane, and whether the piece is
/// on the first side.
fn split_mirror(scope: &EditScope<'_>, value: &Expr) -> Option<(Axis, String, bool)> {
    let ExprKind::Index {
        base,
        index: ast::Index::At(label),
    } = &value.kind
    else {
        return None;
    };
    let ExprKind::Name(region_name) = &base.kind else {
        return None;
    };
    let ExprKind::Name(label) = &label.kind else {
        return None;
    };
    let found = find(scope, region_name.as_str())?;
    let ItemKind::Design(DesignItem::Region(region)) = &found.item.kind else {
        return None;
    };
    let split = split_of(&region.value).filter(|split| !split.at)?;
    let indexes = array_indexes(region.array.as_ref()?)?;
    let position = indexes.iter().position(|index| index == label.as_str())?;
    let last = indexes.len().checked_sub(1)?;
    let partner = last.checked_sub(position)?;
    if partner == position {
        return None;
    }
    if let Some(shares) = split.shares {
        let values = share_values(shares)?;
        if values.iter().ne(values.iter().rev()) {
            return None;
        }
    }
    let source = found.slice(split.source.span)?;
    Some((
        split.axis,
        format!("mid({source}, {})", split.axis.word()),
        position < partner,
    ))
}

/// The parts an assembly's fields name, as the design names them: a part
/// (`top.board`, `legs.leg[front_left]`), the part of a face
/// (`legs.leg[front_left].front`), or each of a list.
fn named_parts<'d>(found: &Found<'_>, design: &'d SolvedDesign, value: &Expr) -> Vec<&'d Part> {
    if let ExprKind::List(items) = &value.kind {
        return items
            .iter()
            .flat_map(|item| named_parts(found, design, item))
            .collect();
    }
    let Some(text) = found.slice(value.span) else {
        return Vec::new();
    };
    let text = text.trim();
    let part = design.part_named(text).or_else(|| {
        let (part, word) = text.rsplit_once('.')?;
        Face::from_word(word)?;
        design.part_named(part)
    });
    part.into_iter().collect()
}

/// Where a part's drawn box lies in the world (the box placement uses,
/// inside its cut box).
fn drawn_box(part: &Part) -> WorldBox {
    part.pose.world_box_at(part.drawn_offset, part.drawn)
}

/// Twice the centre of a box on an axis, in nanometres (exact: no
/// halving), and its size on that axis.
fn doubled_centre(bounds: &WorldBox, axis: Axis) -> Option<(i64, i64)> {
    let (low, high) = (bounds.min().get(axis).nm(), bounds.max().get(axis).nm());
    Some((low.checked_add(high)?, high.checked_sub(low)?))
}

/// Whether `other` is `part` reflected about the plane across `axis` at a
/// quarter of `plane` (both in doubled-centre nanometres, so a plane is
/// the sum of the two doubled centres): the same size on every axis, the
/// same centre on the other two, and the two centres summing to `plane`.
fn reflects(part: &WorldBox, other: &WorldBox, axis: Axis, plane: i64) -> bool {
    Axis::ALL.iter().all(
        |&each| match (doubled_centre(part, each), doubled_centre(other, each)) {
            (Some((a, size_a)), Some((b, size_b))) if size_a == size_b => {
                if each == axis {
                    a.checked_add(b) == Some(plane)
                } else {
                    a == b
                }
            }
            _ => false,
        },
    )
}

/// The mirror image of an assembly placed against parts that each have a
/// counterpart reflected about one plane: every part its fields name has
/// a part of the same size at the same place from the other side of the
/// plane, and the assembly's own parts don't already lie astride it (an
/// apron between the front legs is mirrored front to back, not left to
/// right). The plane is written as the mid-plane of a part or region that
/// it halves, the assembly's named parts first (`mid(top.board, deep)`).
/// Axes are tried across, up, deep, so the answer is the same every run.
fn reflected_mirror(
    found: &Found<'_>,
    design: &SolvedDesign,
    assembly: &str,
    values: &[&Expr],
) -> Option<(Axis, String, bool)> {
    let named: Vec<&Part> = values
        .iter()
        .flat_map(|value| named_parts(found, design, value))
        .collect();
    let first = named.first()?;
    let prefix = format!("{assembly}.");
    let own: Vec<WorldBox> = design
        .parts
        .iter()
        .filter(|part| part.name.starts_with(&prefix))
        .map(drawn_box)
        .collect();
    let world = drawn_box;
    for axis in [Axis::Across, Axis::Up, Axis::Deep] {
        // The assembly's own extent along the axis, doubled-centre.
        let low = own.iter().map(|b| b.min().get(axis).nm()).min()?;
        let high = own.iter().map(|b| b.max().get(axis).nm()).max()?;
        let centre = low.checked_add(high)?;
        let first_box = world(first);
        let (first_centre, _) = doubled_centre(&first_box, axis)?;
        for other in &design.parts {
            if other.id == first.id {
                continue;
            }
            let (other_centre, _) = doubled_centre(&world(other), axis)?;
            let Some(plane) = first_centre.checked_add(other_centre) else {
                continue;
            };
            if !reflects(&first_box, &world(other), axis, plane) {
                continue;
            }
            // The assembly lies astride the plane: its image is itself.
            if centre.checked_mul(2) == Some(plane) {
                continue;
            }
            let all = named.iter().all(|part| {
                let bounds = world(part);
                design
                    .parts
                    .iter()
                    .any(|candidate| reflects(&bounds, &world(candidate), axis, plane))
            });
            if !all {
                continue;
            }
            let halves = |bounds: &WorldBox| {
                doubled_centre(bounds, axis)
                    .and_then(|(c, _)| c.checked_mul(2))
                    .is_some_and(|c| c == plane)
            };
            let Some(base) = named
                .iter()
                .find(|part| halves(&world(part)))
                .map(|part| part.name.clone())
                .or_else(|| {
                    design
                        .regions
                        .iter()
                        .find(|region| !region.name.starts_with(&prefix) && halves(&region.bounds))
                        .map(|region| region.name.clone())
                })
            else {
                continue;
            };
            return Some((
                axis,
                format!("mid({base}, {})", axis.word()),
                centre.checked_mul(2)? < plane,
            ));
        }
    }
    None
}

/// The axis of a mid-plane as written: `mid(base, across)`.
fn plane_axis(about: &str) -> Option<Axis> {
    let inner = about.trim().strip_prefix("mid(")?.strip_suffix(')')?;
    let word = inner.rsplit(',').next()?.trim();
    Axis::from_word(word)
}

// ---------------------------------------------------------------------------
// Test support: projects in memory, shared with `hold` and `remove`
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;

    use stave_lang::render_diagnostics;
    use stave_modules::MemoryFiles;

    use super::GeneratedPatch;
    use crate::scratch::{EditBase, Tried};

    /// Where the copy of `examples/clothing_cabinet_assemblies` lives in
    /// memory.
    pub(crate) const ASSEMBLIES: &str = "d4/clothing_cabinet_assemblies";

    /// A copy of `examples/clothing_cabinet_assemblies` (its `hardware`
    /// module included), in memory under [`ASSEMBLIES`].
    pub(crate) fn assemblies() -> MemoryFiles {
        let example = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/clothing_cabinet_assemblies");
        let mut files = MemoryFiles::new();
        let mut folders = vec![(example, PathBuf::from(ASSEMBLIES))];
        while let Some((from, to)) = folders.pop() {
            for entry in std::fs::read_dir(&from).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_owned();
                if path.is_dir() {
                    folders.push((path, to.join(name)));
                } else if path.extension().is_some_and(|e| e == "stave") {
                    files.insert(to.join(name), std::fs::read_to_string(&path).unwrap());
                }
            }
        }
        files
    }

    /// The text of a file of a project in memory.
    pub(crate) fn text(files: &MemoryFiles, folder: &str, name: &str) -> String {
        use stave_modules::FileSource;
        files.read(&Path::new(folder).join(name)).unwrap()
    }

    /// A small case built in the test: a base, two sides (one the mirror
    /// of the other), a top, the space inside, and a tray of the project's
    /// own `Tray` component filling it. `extra` is added to the end of
    /// `case.stave`.
    pub(crate) fn small_project(extra: &[&str]) -> (String, MemoryFiles) {
        let folder = "d4/small_case".to_owned();
        let mut main = SMALL.to_owned();
        for text in extra {
            main.push('\n');
            main.push_str(text);
            main.push('\n');
        }
        let mut files = MemoryFiles::new();
        files.insert(Path::new(&folder).join("case.stave"), main);
        (folder, files)
    }

    const SMALL: &str = r#"stave 1

project "Small case" {
  units:      mm
  resolution: 0.5mm
}

param width  = 400mm
param height = 500mm
param depth  = 300mm

stock oak_19 { species: white_oak, form: solid, thickness: 19mm, max_width: 400mm, max_length: 2700mm }

part base : oak_19 {
  length: width
  width:  depth
  place {
    ground
  }
}

part side_l : oak_19 {
  length: height
  width:  depth
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.back)
  }
}

part side_r = mirror(side_l, about: mid(base, across))

part top : oak_19 {
  length: width
  width:  depth
  place {
    against(side_l.top)
    center(base, across)
    flush(base.back)
  }
}

joint base_dowels : dowel([side_l, side_r], into: base, count: 4)
joint top_dowels  : dowel([side_l, side_r], into: top, count: 4)

region inside = opening(
  left:   side_l.right,
  right:  side_r.left,
  bottom: base.top,
  top:    top.bottom,
  front:  side_l.front,
  back:   side_l.back,
)

/// A loose board lying on the bottom of a space.
component Tray {
  param opening     : Region
  param board_stock : Stock

  part board : board_stock {
    length: opening.across - 10mm
    width:  opening.deep - 10mm
    place {
      flush(opening.bottom)
      center(opening, across)
      center(opening, deep)
    }
  }
}

assembly tray : Tray {
  opening:     inside
  board_stock: oak_19
}
"#;

    /// Tries a generated patch on the project in `folder` and asserts it
    /// fits and the result has no error; returns what was tried.
    pub(crate) fn fits(folder: &str, files: &MemoryFiles, patch: &GeneratedPatch) -> Tried {
        let tried = try_on(folder, files, patch);
        let rendered = tried
            .opened()
            .map(|opened| render_diagnostics(&opened.sources, &opened.diagnostics))
            .unwrap_or_default();
        assert!(
            tried.fits(),
            "{}\n{}\n{rendered}",
            patch.text,
            render_diagnostics(tried.sources(), &tried.shown())
        );
        let opened = tried.opened().unwrap();
        assert_eq!(opened.error_count(), 0, "{}\n{rendered}", patch.text);
        tried
    }

    /// Tries a generated patch on the project in `folder`.
    pub(crate) fn try_on(folder: &str, files: &MemoryFiles, patch: &GeneratedPatch) -> Tried {
        let base = EditBase::new(Path::new(folder), files).unwrap();
        base.try_patch("generated.stave", &patch.parse, &AtomicBool::new(false))
            .unwrap()
    }

    /// Writes a tried patch's files into `files`, as a commit would.
    pub(crate) fn commit(files: &mut MemoryFiles, tried: &Tried) {
        assert!(tried.fits());
        for file in tried.changed() {
            files.insert(&file.path, file.text.clone());
        }
    }

    /// A base for the project in `folder`, asserting it opens with no
    /// error.
    pub(crate) fn clean_base<'f>(folder: &str, files: &'f MemoryFiles) -> EditBase<'f> {
        let base = EditBase::new(Path::new(folder), files).unwrap();
        let opened = base.opened();
        assert_eq!(
            opened.error_count(),
            0,
            "{}",
            render_diagnostics(&opened.sources, &opened.diagnostics)
        );
        base
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::path::Path;

    use stave_ide::{Analysis, Document};
    use stave_lang::placement::Axis;
    use stave_lang::units::Length;
    use stave_modules::MemoryFiles;

    use super::test_support::{
        ASSEMBLIES, assemblies, clean_base, commit, fits, small_project, text,
    };
    use super::*;
    use crate::candidates::EditScope;
    use crate::naming::RepeatCount;

    fn count(n: usize) -> RepeatCount {
        RepeatCount::new(n).unwrap()
    }

    #[test]
    fn the_small_project_and_the_cabinet_check_clean() {
        let (folder, files) = small_project(&[]);
        clean_base(&folder, &files);
        let files = assemblies();
        clean_base(ASSEMBLIES, &files);
    }

    #[test]
    fn two_drawers_become_three() {
        let mut files = assemblies();
        let base = clean_base(ASSEMBLIES, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let patch = repeat_assembly(
            &scope,
            RepeatRequest {
                assembly: "drawer",
                count: count(3),
                axis: None,
                shares: None,
            },
        )
        .unwrap();
        // The same two declarations change: no second set.
        assert!(
            patch.text.contains(
                "replace region drawer_bay[bottom, middle, top] = split(divider.below, up)"
            )
        );
        assert!(
            patch
                .text
                .contains("replace assembly drawer[bottom, middle, top] : casework.Drawer {")
        );
        assert!(!patch.text.contains("add "));
        let tried = fits(ASSEMBLIES, &files, &patch);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        let fronts: Vec<&str> = design
            .parts
            .iter()
            .map(|p| p.name.as_str())
            .filter(|n| n.starts_with("drawer[") && n.ends_with("].front"))
            .collect();
        assert_eq!(
            fronts,
            [
                "drawer[bottom].front",
                "drawer[middle].front",
                "drawer[top].front"
            ]
        );
        drop(scope);
        drop(base);
        commit(&mut files, &tried);
        let drawers = text(&files, ASSEMBLIES, "drawers.stave");
        assert!(drawers.contains("assembly drawer[bottom, middle, top] : casework.Drawer {"));
        // The comment inside the body is kept.
        assert!(drawers.contains("// drawer[bottom] gets drawer_bay[bottom], and so on"));

        // The same count again is not a patch.
        let base = clean_base(ASSEMBLIES, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        assert!(matches!(
            repeat_assembly(
                &scope,
                RepeatRequest {
                    assembly: "drawer",
                    count: count(3),
                    axis: None,
                    shares: None
                }
            ),
            Err(RepeatError::SameCount { count: 3, .. })
        ));
    }

    #[test]
    fn a_single_assembly_becomes_a_split_array() {
        let (folder, files) = small_project(&[]);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let shares = [NonZeroU32::new(1).unwrap(), NonZeroU32::new(2).unwrap()];
        let patch = repeat_assembly(
            &scope,
            RepeatRequest {
                assembly: "tray",
                count: count(2),
                axis: Some(Axis::Up),
                shares: Some(&shares),
            },
        )
        .unwrap();
        assert!(
            patch
                .text
                .contains("add region tray_bay[bottom, top] = split(inside, up, shares: [1, 2])"),
            "{}",
            patch.text
        );
        assert!(patch.text.contains("set tray.opening = tray_bay[same]"));
        let tried = fits(&folder, &files, &patch);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        assert!(design.part_named("tray[bottom].board").is_some());
        assert!(design.part_named("tray[top].board").is_some());

        // Four or more are counted; the wrong number of shares is refused.
        let four = repeat_assembly(
            &scope,
            RepeatRequest {
                assembly: "tray",
                count: count(4),
                axis: Some(Axis::Up),
                shares: None,
            },
        )
        .unwrap();
        assert!(
            four.text
                .contains("add region tray_bay[4] = split(inside, up)")
        );
        fits(&folder, &files, &four);
        assert!(matches!(
            repeat_assembly(
                &scope,
                RepeatRequest {
                    assembly: "tray",
                    count: count(3),
                    axis: Some(Axis::Up),
                    shares: Some(&shares)
                }
            ),
            Err(RepeatError::SharesCount {
                pieces: 3,
                found: 2
            })
        ));
        assert!(matches!(
            repeat_assembly(
                &scope,
                RepeatRequest {
                    assembly: "tray",
                    count: count(2),
                    axis: None,
                    shares: None
                }
            ),
            Err(RepeatError::NoAxis(_))
        ));
        assert!(matches!(
            repeat_assembly(
                &scope,
                RepeatRequest {
                    assembly: "base",
                    count: count(2),
                    axis: Some(Axis::Up),
                    shares: None
                }
            ),
            Err(RepeatError::NotRepeatable { .. })
        ));
    }

    #[test]
    fn a_part_is_spaced_along_a_span() {
        let slat = r#"part slat : oak_19 {
  length: depth - 20mm
  width:  40mm
  place {
    orient(length: back, width: right)
    against(top.top)
    center(top, across)
    center(top, deep)
  }
}"#;
        let (folder, mut files) = small_project(&[slat]);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let patch = repeat_part(
            &scope,
            SpacedRequest {
                part: "slat",
                count: count(5),
                axis: Axis::Across,
                from: "top.left",
                to: "top.right",
            },
        )
        .unwrap();
        assert!(patch.text.contains("replace part slat[5] : oak_19 {"));
        assert!(
            patch
                .text
                .contains("set slat.place.across = spaced(evenly, from: top.left, to: top.right)")
        );
        let tried = fits(&folder, &files, &patch);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        let slats = design
            .parts
            .iter()
            .filter(|p| p.name.starts_with("slat["))
            .count();
        assert_eq!(slats, 5);
        drop(scope);
        drop(base);
        commit(&mut files, &tried);
        // Changing the count is a patch on the same part.
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let three = repeat_part(
            &scope,
            SpacedRequest {
                part: "slat",
                count: count(3),
                axis: Axis::Across,
                from: "top.left",
                to: "top.right",
            },
        )
        .unwrap();
        assert!(
            three
                .text
                .contains("replace part slat[left, middle, right] : oak_19 {")
        );
        let tried = fits(&folder, &files, &three);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        assert!(design.part_named("slat[middle]").is_some());
    }

    #[test]
    fn a_shares_grip_moves_two_shares_and_keeps_their_sum() {
        let bays = r#"region shelf_bay[a, b, c] = split(inside, up, shares: [100, 200, 100])"#;
        let (folder, files) = small_project(&[bays]);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let design = base.opened().design.as_ref().unwrap();
        // The inside is 500mm tall, less nothing: the sides stand on the
        // base and carry the top. Pieces a and b are 125mm and 250mm.
        let piece = |label: &str| {
            design
                .regions
                .iter()
                .find(|r| r.name == format!("shelf_bay[{label}]"))
                .unwrap()
                .bounds
                .size(Axis::Up)
        };
        assert_eq!(piece("a"), Length::from_mm(125));
        // Drag the boundary between a and b up to 150.2mm: it snaps to
        // 150mm, a step of the 0.5mm resolution. The pieces are then 150mm,
        // 225mm and 125mm: 300, 450 and 250 steps, or 6, 9 and 5.
        let grip = SharesMove {
            region: "shelf_bay",
            lower: 0,
            boundary: Length::from_nm(150_200_000),
        };
        let patch = move_shares(&scope, design, grip).unwrap();
        assert!(
            patch.text.contains(
                "replace region shelf_bay[a, b, c] = split(inside, up, shares: [6, 9, 5])"
            ),
            "{}",
            patch.text
        );
        let tried = fits(&folder, &files, &patch);
        let moved = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        let size = |label: &str| {
            moved
                .regions
                .iter()
                .find(|r| r.name == format!("shelf_bay[{label}]"))
                .unwrap()
                .bounds
                .size(Axis::Up)
        };
        assert_eq!(size("a"), Length::from_mm(150));
        assert_eq!(size("b"), Length::from_mm(225));
        assert_eq!(size("c"), Length::from_mm(125));

        // Equal pieces count as ones; a share that isn't whole stays exact.
        let even = r#"region box_bay[left, right] = split(inside, across)"#;
        let (folder, files) = small_project(&[even]);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let design = base.opened().design.as_ref().unwrap();
        let grip = SharesMove {
            region: "box_bay",
            lower: 0,
            boundary: Length::from_mm(100),
        };
        let patch = move_shares(&scope, design, grip).unwrap();
        // The inside is 400 - 2 * 19 = 362mm wide: pieces of 100mm and
        // 262mm, 200 and 524 steps of 0.5mm, written as 50 and 131.
        assert!(
            patch.text.contains(
                "replace region box_bay[left, right] = split(inside, across, shares: [50, 131])"
            ),
            "{}",
            patch.text
        );
        let tried = fits(&folder, &files, &patch);
        let moved = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        let left = moved
            .regions
            .iter()
            .find(|r| r.name == "box_bay[left]")
            .unwrap();
        assert_eq!(left.bounds.size(Axis::Across), Length::from_mm(100));
        assert!(matches!(
            move_shares(
                &scope,
                design,
                SharesMove {
                    region: "box_bay",
                    lower: 0,
                    boundary: Length::from_mm(362)
                }
            ),
            Err(RepeatError::BoundaryOutside { .. })
        ));
        assert!(matches!(
            move_shares(
                &scope,
                design,
                SharesMove {
                    region: "box_bay",
                    lower: 1,
                    boundary: Length::from_mm(10)
                }
            ),
            Err(RepeatError::NoSuchPieces { .. })
        ));

        // The grip moves again after the first drag, and again after that.
        let mut files = files;
        commit(&mut files, &tried);
        for (boundary, shares, left_mm) in [
            (Length::from_nm(140_400_000), "[281, 443]", None),
            (Length::from_mm(181), "[1, 1]", Some(181)),
        ] {
            let base = clean_base(&folder, &files);
            let scope = EditScope::new(base.opened()).unwrap();
            let design = base.opened().design.as_ref().unwrap();
            let grip = SharesMove {
                region: "box_bay",
                lower: 0,
                boundary,
            };
            let patch = move_shares(&scope, design, grip).unwrap();
            assert!(
                patch.text.contains(&format!(
                    "replace region box_bay[left, right] = split(inside, across, shares: {shares})"
                )),
                "{}",
                patch.text
            );
            let tried = fits(&folder, &files, &patch);
            let moved = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
            let left = moved
                .regions
                .iter()
                .find(|r| r.name == "box_bay[left]")
                .unwrap();
            // 140.4mm snaps to 140.5mm.
            let expected = left_mm.map_or(Length::from_nm(140_500_000), Length::from_mm);
            assert_eq!(left.bounds.size(Axis::Across), expected);
            commit(&mut files, &tried);
        }
    }

    /// The assemblies cabinet with one door, not yet mirrored: the door is
    /// `frame_panel_door`, and the scene poses it by that name.
    fn one_door_cabinet() -> MemoryFiles {
        let mut files = assemblies();
        let doors = text(&files, ASSEMBLIES, "doors.stave")
            .replace("assembly door_l :", "assembly frame_panel_door :")
            .replace(
                "assembly door_r = mirror(door_l, about: mid(base, across))\n",
                "",
            );
        files.insert(Path::new(ASSEMBLIES).join("doors.stave"), doors);
        let cabinet = text(&files, ASSEMBLIES, "cabinet.stave").replace(
            "  pose door_l = open\n  pose door_r = open\n",
            "  pose frame_panel_door = open\n",
        );
        files.insert(Path::new(ASSEMBLIES).join("cabinet.stave"), cabinet);
        files
    }

    #[test]
    fn a_door_on_a_mirrored_side_gets_its_mirror_image() {
        let files = one_door_cabinet();
        let base = clean_base(ASSEMBLIES, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let analysis = Analysis::load(Path::new(ASSEMBLIES), &files).unwrap();
        let design = base.opened().design.as_ref().unwrap();
        let plan = mirror_assembly(&scope, &analysis, design, "frame_panel_door").unwrap();
        assert_eq!(plan.renamed.as_str(), "frame_panel_door_left");
        assert_eq!(plan.image.as_str(), "frame_panel_door_right");
        assert_eq!(plan.about, "mid(base, across)");
        assert!(plan.patch.text.contains(
            "add assembly frame_panel_door_right = mirror(frame_panel_door_left, about: mid(base, across)) in \"doors.stave\""
        ));
        // The rename reaches the scene in another file.
        assert_eq!(plan.rename.len(), 2);

        // Apply the rename, then try the patch on the renamed copy.
        let mut renamed = files.clone();
        for edit in &plan.rename {
            let Document::File(path) = &edit.document else {
                panic!("a standard file was renamed in");
            };
            renamed.insert(path.clone(), edit.text.clone());
        }
        let cabinet = text(&renamed, ASSEMBLIES, "cabinet.stave");
        assert!(cabinet.contains("pose frame_panel_door_left = open"));
        let tried = fits(ASSEMBLIES, &renamed, &plan.patch);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        assert!(
            design
                .parts
                .iter()
                .any(|p| p.name.starts_with("frame_panel_door_right."))
        );
    }

    /// An assembly already named with its side keeps that name, and its
    /// image takes the other side: never `door_left_left`.
    #[test]
    fn mirroring_an_item_named_with_a_side_does_not_double_it() {
        let mut files = one_door_cabinet();
        for name in ["doors.stave", "cabinet.stave"] {
            let renamed = text(&files, ASSEMBLIES, name).replace("frame_panel_door", "door_left");
            files.insert(Path::new(ASSEMBLIES).join(name), renamed);
        }
        let base = clean_base(ASSEMBLIES, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let analysis = Analysis::load(Path::new(ASSEMBLIES), &files).unwrap();
        let design = base.opened().design.as_ref().unwrap();
        let plan = mirror_assembly(&scope, &analysis, design, "door_left").unwrap();
        assert_eq!(
            (plan.renamed.as_str(), plan.image.as_str()),
            ("door_left", "door_right")
        );
        assert!(plan.rename.is_empty(), "nothing to rename");
        let tried = fits(ASSEMBLIES, &files, &plan.patch);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        assert!(
            design
                .parts
                .iter()
                .any(|p| p.name.starts_with("door_right."))
        );
    }

    #[test]
    fn a_piece_of_an_even_split_mirrors_about_the_split() {
        let (folder, files) =
            small_project(&["region tray_bay[left, right] = split(inside, across)"]);
        let mut files = files;
        let case = text(&files, &folder, "case.stave")
            .replace("opening:     inside", "opening:     tray_bay[left]");
        files.insert(Path::new(&folder).join("case.stave"), case);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let analysis = Analysis::load(Path::new(&folder), &files).unwrap();
        let design = base.opened().design.as_ref().unwrap();
        let plan = mirror_assembly(&scope, &analysis, design, "tray").unwrap();
        assert_eq!(plan.about, "mid(inside, across)");
        assert_eq!(
            (plan.renamed.as_str(), plan.image.as_str()),
            ("tray_left", "tray_right")
        );
        let mut renamed = files.clone();
        for edit in &plan.rename {
            let Document::File(path) = &edit.document else {
                panic!("a standard file was renamed in");
            };
            renamed.insert(path.clone(), edit.text.clone());
        }
        let tried = fits(&folder, &renamed, &plan.patch);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        assert!(design.part_named("tray_right.board").is_some());
        // Nothing to mirror: the tray filling the whole inside.
        let (folder, files) = small_project(&[]);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let analysis = Analysis::load(Path::new(&folder), &files).unwrap();
        let design = base.opened().design.as_ref().unwrap();
        assert!(matches!(
            mirror_assembly(&scope, &analysis, design, "tray"),
            Err(RepeatError::NoMirror(_))
        ));
    }

    /// Applies a mirror's rename to a copy of `files`.
    fn renamed_by(files: &MemoryFiles, plan: &MirrorPlan) -> MemoryFiles {
        let mut renamed = files.clone();
        for edit in &plan.rename {
            let Document::File(path) = &edit.document else {
                panic!("a standard file was renamed in");
            };
            renamed.insert(path.clone(), edit.text.clone());
        }
        renamed
    }

    #[test]
    fn an_assembly_filling_a_whole_space_becomes_a_mirrored_pair() {
        let (folder, files) = small_project(&[]);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let analysis = Analysis::load(Path::new(&folder), &files).unwrap();
        let plan = mirror_split_assembly(&scope, &analysis, "tray", Axis::Across).unwrap();
        assert_eq!(
            (plan.renamed.as_str(), plan.image.as_str()),
            ("tray_left", "tray_right")
        );
        assert_eq!(plan.about, "mid(inside, across)");
        for line in [
            "add region tray_bay[left, right] = split(inside, across)",
            "set tray_left.opening = tray_bay[left]",
            "add assembly tray_right = mirror(tray_left, about: mid(inside, across)) in \"case.stave\"",
        ] {
            assert!(plan.patch.text.contains(line), "{}", plan.patch.text);
        }
        let renamed = renamed_by(&files, &plan);
        let tried = fits(&folder, &renamed, &plan.patch);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        let left = design.part_named("tray_left.board").unwrap();
        let right = design.part_named("tray_right.board").unwrap();
        assert_eq!(left.cut, right.cut);

        // Up, the pieces are bottom and top.
        let plan = mirror_split_assembly(&scope, &analysis, "tray", Axis::Up).unwrap();
        assert_eq!(
            (plan.renamed.as_str(), plan.image.as_str()),
            ("tray_bottom", "tray_top")
        );

        // A tray already in one of two matching pieces is mirrored without
        // an axis, and a mirror image is not mirrored again.
        let mut files = small_project(&["region tray_bay[left, right] = split(inside, across)"]).1;
        let case = text(&files, &folder, "case.stave")
            .replace("opening:     inside", "opening:     tray_bay[left]");
        files.insert(Path::new(&folder).join("case.stave"), case);
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let analysis = Analysis::load(Path::new(&folder), &files).unwrap();
        assert!(matches!(
            mirror_split_assembly(&scope, &analysis, "tray", Axis::Across),
            Err(RepeatError::NotRepeatable { .. })
        ));
        assert!(matches!(
            mirror_split_assembly(&scope, &analysis, "inside", Axis::Across),
            Err(RepeatError::NotRepeatable { .. })
        ));
    }

    /// `examples/side_table_from_box` with its back apron (and the back
    /// cleat, which needs it) left out and the front one called `apron`,
    /// in memory.
    fn one_apron_table() -> (String, MemoryFiles) {
        let folder = "d4/side_table".to_owned();
        let example = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/side_table_from_box/table.stave");
        let table = std::fs::read_to_string(example)
            .unwrap()
            .replace(
                "assembly apron_back = mirror(apron_front, about: mid(top.board, deep))\n",
                "",
            )
            .replace(
                "assembly cleat_back = mirror(cleat_front, about: mid(top.board, deep))\n",
                "",
            )
            .replace("apron_front", "apron");
        let mut files = MemoryFiles::new();
        files.insert(Path::new(&folder).join("table.stave"), table);
        (folder, files)
    }

    #[test]
    fn an_apron_between_two_legs_mirrors_onto_the_other_two() {
        let (folder, files) = one_apron_table();
        let base = clean_base(&folder, &files);
        let scope = EditScope::new(base.opened()).unwrap();
        let analysis = Analysis::load(Path::new(&folder), &files).unwrap();
        let design = base.opened().design.as_ref().unwrap();
        // The legs are elements of an array, not mirror images, so only
        // their places say the back legs reflect the front ones; the
        // apron lies astride the plane across, so not that one.
        // The top names no part, so it has nothing to be mirrored against.
        assert!(matches!(
            mirror_assembly(&scope, &analysis, design, "top"),
            Err(RepeatError::NoMirror(_))
        ));
        let plan = mirror_assembly(&scope, &analysis, design, "apron").unwrap();
        assert_eq!(plan.about, "mid(top.board, deep)");
        assert_eq!(
            (plan.renamed.as_str(), plan.image.as_str()),
            ("apron_front", "apron_back")
        );
        let renamed = renamed_by(&files, &plan);
        let tried = fits(&folder, &renamed, &plan.patch);
        let design = tried.opened().and_then(|o| o.design.as_ref()).unwrap();
        let front = design.part_named("apron_front.apron").unwrap();
        let back = design.part_named("apron_back.apron").unwrap();
        assert_eq!(front.cut, back.cut);
    }

    #[test]
    fn a_rename_checks_clean_and_leaves_no_use_of_the_old_name() {
        let files = assemblies();
        let analysis = Analysis::load(Path::new(ASSEMBLIES), &files).unwrap();
        let divider = analysis.project_symbol("divider").unwrap().id;
        let uses = analysis.uses(divider).unwrap().count();
        // A clash is refused.
        assert!(analysis.rename(divider, "shelf").is_err());
        let edits = analysis.rename(divider, "middle_panel").unwrap();
        let mut renamed = files.clone();
        for edit in &edits {
            let Document::File(path) = &edit.document else {
                panic!("a standard file was renamed in");
            };
            renamed.insert(path.clone(), edit.text.clone());
            // Outside comments, the old name is gone as a word.
            for line in edit.text.lines() {
                let code = line.split("//").next().unwrap_or("");
                let words = code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'));
                assert!(!words.into_iter().any(|w| w == "divider"), "{line}");
            }
        }
        let opened = crate::open_with(Path::new(ASSEMBLIES), &renamed).unwrap();
        assert_eq!(
            opened.error_count(),
            0,
            "{}",
            stave_lang::render_diagnostics(&opened.sources, &opened.diagnostics)
        );
        let design = opened.design.as_ref().unwrap();
        assert!(design.part_named("middle_panel.panel").is_some());
        let again = Analysis::load(Path::new(ASSEMBLIES), &renamed).unwrap();
        assert!(again.project_symbol("divider").is_none());
        let panel = again.project_symbol("middle_panel").unwrap().id;
        assert_eq!(again.uses(panel).unwrap().count(), uses);
    }

    #[test]
    fn shares_are_reduced_by_what_they_have_in_common() {
        assert_eq!(reduced(&[300, 450, 250]).unwrap(), [6, 9, 5]);
        assert_eq!(reduced(&[200, 524]).unwrap(), [50, 131]);
        assert_eq!(reduced(&[281, 443]).unwrap(), [281, 443]);
        assert_eq!(reduced(&[0, 0]), None);
    }
}

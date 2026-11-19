//! Models checked against the text (`warning[model-mismatch]`; grammar,
//! "Checked against the text"). The `.stave` numbers are the truth: each
//! node a kind's text declares is looked up by name in its `.glb` and
//! compared, to 0.01mm, and a pinned file is compared by its SHA-256. A
//! model that can't be found is drawn as its members and is not a
//! diagnostic.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use stave_geom::glb::{self, Axes, FrameBounds, GlbError, ModelNode};
use stave_lang::units::Length;
use stave_lang::{Code, Diagnostic, Label};
use stave_model::solved::{DeclaredValue, FramePoint, Hardware, Model, SolvedDesign};
use stave_model::value::Sha256;

use crate::names::{length_text, list};

/// How far a model may stray from the text: 0.01mm.
const TOLERANCE: Length = Length::from_nm(10_000);

/// What the most common mistake makes a model's numbers.
const SCALE: i64 = 1000;

/// One way a model and its kind's text disagree.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ModelMismatch {
    /// A pivot's or a slot's node is somewhere else.
    Point {
        /// The node.
        name: String,
        /// Where the text puts it.
        declared: FramePoint,
        /// Where the model puts it.
        found: FramePoint,
        /// What probably went wrong.
        cause: Cause,
    },
    /// A shape's node has other bounds.
    Shape {
        /// The node.
        name: String,
        /// The text's bounds.
        declared: FrameBounds,
        /// The model's.
        found: FrameBounds,
        /// What probably went wrong.
        cause: Cause,
    },
    /// The file is not the one `sha256:` pins.
    Hash {
        /// The file's digest.
        found: Sha256,
    },
    /// Nodes the text needs that the model lacks (a shape's node lacks a
    /// mesh), in the kind's order.
    MissingNode {
        /// Their names.
        names: Vec<String>,
    },
    /// The file is not a model Stave can read.
    Unreadable(GlbError),
}

/// What probably makes a model disagree with its text, for the help line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cause {
    /// It matches when read Z up.
    ZUp,
    /// It matches a thousand times smaller: drawn in millimetres.
    TooLarge,
    /// It matches a thousand times larger.
    TooSmall,
    /// Nothing simple.
    Unknown,
}

impl ModelMismatch {
    /// The warning, for a piece of hardware whose kind has this model.
    fn diagnostic(&self, hardware: &Hardware, model: &Model) -> Diagnostic {
        let kind = &hardware.kind;
        let label = Label::new(model.file, model.span);
        let (message, help) = match self {
            ModelMismatch::Point {
                name,
                declared,
                found,
                cause,
            } => (
                format!(
                    "{kind}'s model puts '{name}' at {}, but the kind says {}",
                    point_text(found),
                    point_text(declared)
                ),
                cause.help(),
            ),
            ModelMismatch::Shape {
                name,
                declared,
                found,
                cause,
            } => (
                format!(
                    "{kind}'s model has '{name}' over {}, but the kind says {}",
                    bounds_text(found),
                    bounds_text(declared)
                ),
                cause.help(),
            ),
            ModelMismatch::Hash { found } => (
                format!("{kind}'s model is not the pinned file: its SHA-256 is {found}"),
                format!(
                    "the file changed since it was pinned: check that it still matches the kind, then write sha256: \"{found}\""
                ),
            ),
            ModelMismatch::MissingNode { names } => {
                let quoted: Vec<String> = names.iter().map(|name| format!("'{name}'")).collect();
                (
                    format!("{kind}'s model has no node {}", list(&quoted)),
                    "name each object in the model as the kind's text does: '<member> <shape>' for a named shape and '<member>' for the rest, each with its mesh, and '<a> <b>' at each pivot".to_owned(),
                )
            }
            ModelMismatch::Unreadable(why) => (
                format!("can't read {kind}'s model: {why}"),
                "export it from Blender as glTF Binary (.glb), or leave model: out".to_owned(),
            ),
        };
        Diagnostic::warning(Code::ModelMismatch, message, label).with_help(help)
    }
}

impl Cause {
    fn help(self) -> String {
        match self {
            Cause::ZUp => "the model is Z up: glTF is +Y up, so export from Blender with \"+Y Up\" ticked",
            Cause::TooLarge => "the model is 1000 times too large: glTF is in metres, so a model drawn in millimetres needs its unit scale set to 0.001, or its object's scale applied (Ctrl+A) before export",
            Cause::TooSmall => "the model is 1000 times too small: glTF is in metres; apply the object's scale (Ctrl+A) before export",
            Cause::Unknown => "the kind's numbers are the truth: glTF is in metres with +Y up, while Blender is Z up and often in millimetres, and an object's scale or rotation that was not applied (Ctrl+A) is exported as a transform; fix the model, or the kind if the model is right",
        }
        .to_owned()
    }
}

/// A model file, read: its digest and its nodes.
struct ReadFile {
    digest: Sha256,
    model: Result<glb::Model, GlbError>,
}

/// Every model of every piece of hardware, checked against its kind's text.
/// `read` gives a file's bytes; a file it can't give is not checked.
pub(crate) fn models(
    design: &SolvedDesign,
    read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
) -> Vec<Diagnostic> {
    // Each file once: `None` when it can't be read.
    let mut files: BTreeMap<PathBuf, Option<ReadFile>> = BTreeMap::new();
    let mut out: Vec<Diagnostic> = Vec::new();
    for hardware in design.hardware() {
        let Some(model) = &hardware.model else {
            continue;
        };
        let Some(path) = &model.path else {
            continue;
        };
        // The path is copied into the table only the first time it is read.
        if !files.contains_key(path) {
            let file = read(path).ok().map(|bytes| ReadFile {
                digest: glb::sha256(&bytes),
                model: glb::read(&bytes),
            });
            files.insert(path.clone(), file);
        }
        let Some(Some(file)) = files.get(path) else {
            continue;
        };
        for mismatch in mismatches(model, file.digest, &file.model) {
            let diagnostic = mismatch.diagnostic(hardware, model);
            if !out.contains(&diagnostic) {
                out.push(diagnostic);
            }
        }
    }
    out
}

/// How one model disagrees with its kind's text, in a fixed order: the
/// pin, the file, the missing nodes, then each node in the kind's order.
pub(crate) fn mismatches(
    model: &Model,
    digest: Sha256,
    parsed: &Result<glb::Model, GlbError>,
) -> Vec<ModelMismatch> {
    let mut out = Vec::new();
    if let Some(pinned) = model.sha256
        && pinned != digest
    {
        out.push(ModelMismatch::Hash { found: digest });
    }
    let file = match parsed {
        Ok(file) => file,
        Err(why) => {
            out.push(ModelMismatch::Unreadable(why.clone()));
            return out;
        }
    };
    let mut missing = Vec::new();
    let origin = match &model.node {
        Some(name) => match file.node(name) {
            Some(node) => node.translation,
            None => {
                missing.push(name.clone());
                [0.0; 3]
            }
        },
        None => [0.0; 3],
    };
    let mut found = Vec::new();
    for declared in &model.nodes {
        let Some(node) = file.node(&declared.name) else {
            missing.push(declared.name.clone());
            continue;
        };
        match &declared.value {
            DeclaredValue::Point(point) => {
                if let Some(mismatch) = point_mismatch(&declared.name, point, node, origin) {
                    found.push(mismatch);
                }
            }
            DeclaredValue::Shapes(shapes) => {
                let Some(text) = glb::shape_bounds(shapes) else {
                    continue;
                };
                if node.bounds.is_none() {
                    missing.push(declared.name.clone());
                    continue;
                }
                if let Some(mismatch) = shape_mismatch(&declared.name, &text, node, origin) {
                    found.push(mismatch);
                }
            }
        }
    }
    if !missing.is_empty() {
        out.push(ModelMismatch::MissingNode { names: missing });
    }
    out.extend(found);
    out
}

fn point_mismatch(
    name: &str,
    declared: &FramePoint,
    node: &ModelNode,
    origin: [f64; 3],
) -> Option<ModelMismatch> {
    let values = |p: &FramePoint| [p.along, p.across, p.depth];
    let found = node.point(origin, Axes::YUp)?;
    if close(&values(&found), &values(declared), 1) {
        return None;
    }
    let z_up = node
        .point(origin, Axes::ZUp)
        .is_some_and(|p| close(&values(&p), &values(declared), 1));
    Some(ModelMismatch::Point {
        name: name.to_owned(),
        declared: *declared,
        found,
        cause: cause(z_up, &values(&found), &values(declared)),
    })
}

fn shape_mismatch(
    name: &str,
    declared: &FrameBounds,
    node: &ModelNode,
    origin: [f64; 3],
) -> Option<ModelMismatch> {
    let values = |b: &FrameBounds| {
        [
            b.along[0],
            b.along[1],
            b.across[0],
            b.across[1],
            b.depth[0],
            b.depth[1],
        ]
    };
    let found = node.bounds(origin, Axes::YUp)?;
    if close(&values(&found), &values(declared), 1) {
        return None;
    }
    let z_up = node
        .bounds(origin, Axes::ZUp)
        .is_some_and(|b| close(&values(&b), &values(declared), 1));
    Some(ModelMismatch::Shape {
        name: name.to_owned(),
        declared: *declared,
        found,
        cause: cause(z_up, &values(&found), &values(declared)),
    })
}

/// What probably went wrong, given whether the model matches read Z up.
fn cause(z_up: bool, found: &[Length], declared: &[Length]) -> Cause {
    if z_up {
        Cause::ZUp
    } else if scaled(declared).is_some_and(|large| close(found, &large, SCALE)) {
        Cause::TooLarge
    } else if scaled(found).is_some_and(|large| close(&large, declared, SCALE)) {
        Cause::TooSmall
    } else {
        Cause::Unknown
    }
}

/// Each value a thousand times larger; `None` on overflow.
fn scaled(values: &[Length]) -> Option<Vec<Length>> {
    values
        .iter()
        .map(|value| value.checked_mul(SCALE))
        .collect()
}

/// `true` when every value is within the tolerance (times `scale`) of its
/// counterpart.
fn close(a: &[Length], b: &[Length], scale: i64) -> bool {
    let tolerance = TOLERANCE.nm().saturating_mul(scale);
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.nm()
                .checked_sub(y.nm())
                .is_some_and(|difference| difference.saturating_abs() <= tolerance)
        })
}

/// `along 0mm, across 13.5mm, depth 10.9mm`.
fn point_text(point: &FramePoint) -> String {
    format!(
        "along {}, across {}, depth {}",
        length_text(point.along),
        length_text(point.across),
        length_text(point.depth)
    )
}

/// `along -8mm..8mm, across 0mm..15mm, depth 0mm..12.4mm`.
fn bounds_text(bounds: &FrameBounds) -> String {
    let range = |[low, high]: [Length; 2]| format!("{}..{}", length_text(low), length_text(high));
    format!(
        "along {}, across {}, depth {}",
        range(bounds.along),
        range(bounds.across),
        range(bounds.depth)
    )
}

//! What a hardware kind's text says its model has (grammar, "Checked
//! against the text"): one node per shape name and per pin, in the kind's
//! frame with the hardware closed, for the checker to compare with the
//! `.glb`. Nothing here reads the file.

use std::path::{Path, PathBuf};

use stave_lang::units::{ExactLength, Length};
use stave_model::defs::BaseEntry;
use stave_model::defs::{BodyId, ConstraintDef, KindDef};
use stave_model::solved::{DeclaredNode, DeclaredValue, FramePoint, FrameShape, Model};
use stave_model::value::ModelRef;
use stave_schema::kinds::BaseField;

use crate::hardware::{FrameShapes, Outline};
use crate::joints::Binding;
use crate::mechanism::ExactPoint;
use crate::{Failure, Fault, Solved, Solver};

impl Solver<'_> {
    /// A piece of hardware's model, with the nodes its kind's text
    /// declares: `bodies` are its members' shapes in the kind's frame, as
    /// declared.
    pub(crate) fn hardware_model(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        model: &ModelRef,
        bodies: &[(BodyId, FrameShapes)],
    ) -> Solved<Model> {
        let mut nodes: Vec<DeclaredNode> = Vec::new();
        for (id, shapes) in bodies {
            let body = kind
                .bodies
                .get(id.index())
                .ok_or(Fault::Internal("a member of a kind"))?;
            for (named, (frame_box, _, outline)) in body.shapes.iter().zip(shapes) {
                let shape = match outline {
                    Some(Outline::Bar {
                        from,
                        to,
                        thickness,
                    }) => FrameShape::Bar {
                        from: pair(*from)?,
                        to: pair(*to)?,
                        thickness: near(*thickness)?,
                        along: pair([frame_box.along.0, frame_box.along.1])?,
                    },
                    Some(Outline::Prism(_)) | None => FrameShape::Bounds {
                        along: pair([frame_box.along.0, frame_box.along.1])?,
                        across: pair([frame_box.across.0, frame_box.across.1])?,
                        depth: pair([frame_box.depth.0, frame_box.depth.1])?,
                    },
                };
                let name = match &named.name {
                    Some(shape_name) => format!("{} {shape_name}", body.name),
                    None => body.name.as_str().to_owned(),
                };
                match nodes.iter_mut().find(|node| node.name == name) {
                    Some(DeclaredNode {
                        value: DeclaredValue::Shapes(list),
                        ..
                    }) => list.push(shape),
                    Some(_) => return Err(Fault::Earlier.into()),
                    None => nodes.push(DeclaredNode {
                        name,
                        value: DeclaredValue::Shapes(vec![shape]),
                    }),
                }
            }
        }
        if let Some(mechanism) = kind.mechanism() {
            let body_name = |id: BodyId| -> Solved<&str> {
                kind.bodies
                    .get(id.index())
                    .map(|body| body.name.as_str())
                    .ok_or(Failure::Fault(Fault::Internal("a member of a kind")))
            };
            for constraint in &mechanism.constraints {
                let (bodies, points) = match constraint {
                    ConstraintDef::Pivot { bodies, at, .. } => (bodies, vec![(None, &**at)]),
                    ConstraintDef::Slot {
                        bodies, from, to, ..
                    } => (bodies, vec![(Some("from"), &**from), (Some("to"), &**to)]),
                    ConstraintDef::Rail { .. } => continue,
                };
                let [a, b] = *bodies;
                let pair_name = format!("{} {}", body_name(a)?, body_name(b)?);
                for (end, point) in points {
                    let exact =
                        self.in_kind(binding, kind, |solver, env| solver.kind_point(point, env))?;
                    nodes.push(DeclaredNode {
                        name: match end {
                            Some(end) => format!("{pair_name} {end}"),
                            None => pair_name.clone(),
                        },
                        value: DeclaredValue::Point(frame_point(exact)?),
                    });
                }
            }
        }
        let span = match kind.base_entry(BaseField::Model) {
            Some(BaseEntry::Field(index)) => kind.fields().get(index).map(|field| field.value.span),
            Some(BaseEntry::Param(index)) => kind.params.get(index).map(|param| param.decl.span),
            None => None,
        };
        let (file, span) = (kind.decl.file, span.unwrap_or(kind.decl.name_span));
        Ok(Model {
            source: model.source.clone(),
            node: model.align.clone(),
            path: self.model_path(&model.source, file),
            sha256: model.sha256,
            file,
            span,
            nodes,
        })
    }

    /// Where a model's file is: its source relative to the folder of the
    /// module that declares the kind. `None` for a standard module and for
    /// a remote source (`https://...`, `github:...`).
    fn model_path(&self, source: &str, file: stave_lang::FileId) -> Option<PathBuf> {
        if let Some((scheme, _)) = source.split_once(':')
            && is_scheme(scheme)
        {
            return None;
        }
        let path = Path::new(source);
        if path.is_absolute() {
            return Some(stave_modules::normalize(path));
        }
        let module = self.model.defs.files.get(&file)?.module?;
        let folder = self
            .model
            .defs
            .modules
            .get(module.index())?
            .folder
            .as_ref()?;
        Some(stave_modules::normalize(&folder.join(path)))
    }
}

/// `true` for a URL scheme of two letters or more (one letter is a drive):
/// a letter, then letters, digits, `+`, `-` or `.`.
fn is_scheme(text: &str) -> bool {
    let mut chars = text.chars();
    text.len() > 1
        && chars
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn near(value: ExactLength) -> Solved<Length> {
    value
        .to_nearest_length()
        .ok_or(Failure::Fault(Fault::Overflow))
}

fn pair([a, b]: [ExactLength; 2]) -> Solved<[Length; 2]> {
    Ok([near(a)?, near(b)?])
}

fn frame_point(point: ExactPoint) -> Solved<FramePoint> {
    Ok(FramePoint {
        along: near(point.along)?,
        across: near(point.across)?,
        depth: near(point.depth)?,
    })
}

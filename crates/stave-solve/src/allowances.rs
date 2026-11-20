//! The cut list's allowances (grammar, "Cut list"): the saw's kerf, the
//! rough size a solid part is first cut at, the thickness a stock is bought
//! at, and the edge banding on a sheet part's edges.

use stave_lang::ast::Span;
use stave_lang::placement::PartAxis;
use stave_lang::units::{ExactLength, Length, Rational};
use stave_lang::{Code, FileId};
use stave_model::defs::{DefId, RExpr, StockSectionDef};
use stave_model::ids::{PartId, StockId};
use stave_model::instance::Ctx;
use stave_model::solved::{Allowances, Banding};
use stave_model::value::format_length;
use stave_schema::words::StockForm;

use crate::eval::Env;
use crate::joints::{GeomTable, round_length};
use crate::{Fault, Solved, Solver};

/// The project's allowances, before a stock overrides them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProjectAllowances {
    /// `kerf:`
    pub(crate) kerf: Length,
    /// `rough_length:`
    pub(crate) rough_length: Length,
    /// `rough_width:`
    pub(crate) rough_width: Length,
}

impl Solver<'_> {
    /// An allowance written in `file` (`kerf:`, `rough_length:`,
    /// `rough_width:`): 0mm or more, reported once however often it is
    /// read.
    fn allowance(&mut self, expr: &RExpr, env: &Env<'_>, name: &str) -> Solved<Length> {
        let value = self.eval_length(expr, env)?;
        if value < ExactLength::ZERO {
            return Err(self.out_of_range_once(
                env.file,
                expr.span,
                format!(
                    "'{name}' must be 0mm or more, found {}",
                    format_length(value)
                ),
                "write how much wood the allowance takes, 0mm or more, such as 3mm",
            ));
        }
        round_length(value)
    }

    /// An `error[out-of-range]` at `span`, unless the same one is already
    /// reported there.
    fn out_of_range_once(
        &mut self,
        file: FileId,
        span: Span,
        message: String,
        help: &str,
    ) -> crate::Failure {
        let reported = self.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == Code::OutOfRange
                && diagnostic.message == message
                && diagnostic
                    .primary()
                    .is_some_and(|label| label.file == file && label.span == span)
        });
        if reported {
            return self.skip().into();
        }
        self.error(Code::OutOfRange, message, file, span, help)
            .into()
    }

    /// The project's `kerf:`, `rough_length:` and `rough_width:`, or their
    /// defaults (3mm, 0mm, 0mm).
    pub(crate) fn project_allowances(&mut self) -> Solved<ProjectAllowances> {
        let Some(project) = self.model.defs.project(self.model.root) else {
            return Err(Fault::Internal("the project's declaration").into());
        };
        let root = self
            .model
            .module_instance(self.model.root)
            .ok_or(Fault::Internal("the project's own items"))?;
        let env = Env {
            ctx: Ctx {
                instance: root,
                element: None,
            },
            file: project.file,
            kind: None,
            array: None,
        };
        let read = |this: &mut Self, expr: &Option<RExpr>, name, default| match expr {
            Some(expr) => this.allowance(expr, &env, name),
            None => Ok(default),
        };
        Ok(ProjectAllowances {
            kerf: read(
                self,
                &project.kerf,
                "kerf",
                Length::from_nm(stave_schema::fields::DEFAULT_KERF_NM),
            )?,
            rough_length: read(self, &project.rough_length, "rough_length", Length::ZERO)?,
            rough_width: read(self, &project.rough_width, "rough_width", Length::ZERO)?,
        })
    }

    /// A stock's allowances: its own `kerf:`, `rough_length:`,
    /// `rough_width:` and `sold:`, or the project's where it doesn't say.
    /// A sheet's or a dowel's rough allowances are zero: they are cut
    /// straight to size.
    pub(crate) fn stock_allowances(&mut self, stock: StockId) -> Solved<Allowances> {
        let def = self.sound(self.model.defs.sound_stock(stock), "a stock's declaration")?;
        let project = self.project_allowances()?;
        let Some(ctx) = self.module_ctx(DefId::Stock(stock)) else {
            return Err(Fault::Internal("the file an item is declared in").into());
        };
        let env = Env {
            ctx,
            file: def.decl.file,
            kind: None,
            array: None,
        };
        let read = |this: &mut Self, expr: &Option<RExpr>, name, default| match expr {
            Some(expr) => this.allowance(expr, &env, name),
            None => Ok(default),
        };
        let kerf = read(self, &def.kerf, "kerf", project.kerf)?;
        let solid = def.form == StockForm::Solid;
        let (rough_length, rough_width) = if solid {
            (
                read(
                    self,
                    &def.rough_length,
                    "rough_length",
                    project.rough_length,
                )?,
                read(self, &def.rough_width, "rough_width", project.rough_width)?,
            )
        } else {
            (Length::ZERO, Length::ZERO)
        };
        let sold = match (&def.sold, &def.section) {
            (Some(expr), Some(StockSectionDef::Board { .. })) if solid => {
                let sold = self.eval_length(expr, &env)?;
                let thickness = self
                    .stock_length(stock, stave_model::defs::StockField::Thickness)?
                    .ok_or(Fault::Earlier)?;
                if sold < thickness {
                    let name = def.decl.name.clone();
                    let finished = format_length(thickness);
                    return Err(self.out_of_range_once(
                        def.decl.file,
                        expr.span,
                        format!(
                            "{name} is sold {} thick, thinner than its finished thickness of {finished}",
                            format_length(sold)
                        ),
                        &format!(
                            "write the thickness you buy, at least {finished}: 4/4 lumber is sold: 1in"
                        ),
                    ));
                }
                Some(round_length(sold)?)
            }
            _ => None,
        };
        Ok(Allowances {
            kerf,
            rough_length,
            rough_width,
            sold,
        })
    }

    /// The rough allowance a part of `stock` is cut with along its grain:
    /// the stock's `rough_length`, zero for a sheet or a dowel.
    pub(crate) fn rough_length_of(&mut self, stock: StockId) -> Solved<ExactLength> {
        Ok(self.stock_allowances(stock)?.rough_length.to_exact())
    }

    /// The length a part or piece is first cut at along its grain, said
    /// for a message: `1825mm (its cut length of 1800mm and 25mm
    /// rough_length)`, or just the cut length when there is no rough
    /// allowance. `None` when the rough length is within `max_length`.
    pub(crate) fn rough_over(
        cut_length: ExactLength,
        rough: ExactLength,
        max_length: ExactLength,
    ) -> Solved<Option<(&'static str, String)>> {
        let total = cut_length.checked_add(rough).ok_or(Fault::Overflow)?;
        if total <= max_length {
            return Ok(None);
        }
        Ok(Some(if rough == ExactLength::ZERO {
            ("cut", format_length(cut_length))
        } else {
            (
                "rough",
                format!(
                    "{} (its cut length of {} and {} rough_length)",
                    format_length(total),
                    format_length(cut_length),
                    format_length(rough)
                ),
            )
        }))
    }

    /// Every part's edge banding, by part index: `None` where it has none
    /// or its banding is wrong (which is reported, at the part's source).
    /// A mirror image, and a part of a mirrored assembly, has its source's
    /// banding on the faces of its own frame that the source's are on.
    pub(crate) fn make_banding(&mut self, geoms: &GeomTable) -> Vec<Option<Banding>> {
        let count = self.model.parts.len();
        let mut out: Vec<Option<Banding>> = vec![None; count];
        for index in 0..count {
            let part = PartId::from_usize(index);
            if self
                .model
                .parts
                .get(index)
                .is_none_or(|inst| inst.mirror_of.is_some())
            {
                continue;
            }
            let result = self.part_banding(part, geoms);
            let (file, span) = self.part_place(part);
            if let Some(Some(banding)) = self.settled(result, file, span)
                && let Some(slot) = out.get_mut(index)
            {
                *slot = Some(banding);
            }
        }
        // A part of a mirrored assembly takes its source's.
        for index in 0..count {
            let source = self.model.parts.get(index).and_then(|inst| inst.mirror_of);
            if let Some(source) = source {
                let copied = out.get(source.index()).cloned().flatten();
                if let Some(slot) = out.get_mut(index) {
                    *slot = copied;
                }
            }
        }
        out
    }

    fn part_banding(&mut self, part: PartId, geoms: &GeomTable) -> Solved<Option<Banding>> {
        let source = self.body_source(part)?;
        let Some(body) = self.part_body(source) else {
            return Ok(None);
        };
        let Some(banding) = &body.banding else {
            return Ok(None);
        };
        // A mirror image's source reports.
        let report = source == part;
        let env = self
            .part_env(source)
            .ok_or(Fault::Internal("where a part is declared"))?;
        let name = self.part_name(source);
        let stock = self.part_stock(source)?;
        let form = self
            .sound(self.model.defs.sound_stock(stock), "a stock's declaration")?
            .form;
        match form {
            StockForm::Plywood | StockForm::Mdf => {}
            StockForm::Solid | StockForm::Dowel => {
                if !report {
                    return Err(Fault::Earlier.into());
                }
                let what = match form {
                    StockForm::Dowel => "a dowel",
                    StockForm::Solid | StockForm::Plywood | StockForm::Mdf => "solid wood",
                };
                return Err(self
                    .error(
                        Code::TypeMismatch,
                        format!("{name} is cut from {what}, so it takes no edge banding"),
                        env.file,
                        banding.span,
                        format!(
                            "edge banding covers the edges of plywood and MDF; take banding: off {name}"
                        ),
                    )
                    .into());
            }
        }
        let thickness = self.eval_length(&banding.thickness, &env)?;
        if thickness <= ExactLength::ZERO {
            if !report {
                return Err(Fault::Earlier.into());
            }
            return Err(self
                .error(
                    Code::OutOfRange,
                    format!(
                        "edge banding must be more than 0mm thick, found {}",
                        format_length(thickness)
                    ),
                    env.file,
                    banding.thickness.span,
                    "write the band's thickness, such as edge_band(1mm, on: front)",
                )
                .into());
        }
        let geom = geoms.get(source).ok_or(Fault::Earlier)?;
        let orientation = geom.orientation;
        let mut faces = Vec::with_capacity(banding.faces.len());
        for (face, span) in &banding.faces {
            let local = crate::edges::local_face(&orientation, *face);
            if local.axis == PartAxis::Thickness {
                if !report {
                    return Err(Fault::Earlier.into());
                }
                return Err(self
                    .error(
                        Code::TypeMismatch,
                        format!("{} is a broad face of {name}, not an edge", face.word()),
                        env.file,
                        *span,
                        format!(
                            "band the edges: the faces at the ends of {name}'s length and width"
                        ),
                    )
                    .into());
            }
            faces.push(local);
        }
        faces.sort();
        faces.dedup();
        // Something must be left to cut along each banded axis.
        for axis in [PartAxis::Length, PartAxis::Width] {
            let ends = faces.iter().filter(|face| face.axis == axis).count();
            if ends == 0 {
                continue;
            }
            let ends = i128::try_from(ends).map_err(|_| Fault::Overflow)?;
            let taken = thickness
                .checked_mul(Rational::from_int(ends))
                .ok_or(Fault::Overflow)?;
            let size = geom.cut_size(axis)?;
            if taken >= size {
                if !report {
                    return Err(Fault::Earlier.into());
                }
                let measure = match axis {
                    PartAxis::Length => "long",
                    PartAxis::Width | PartAxis::Thickness => "wide",
                };
                return Err(self
                    .error(
                        Code::OutOfRange,
                        format!(
                            "{name} is only {} {measure}, and its edge banding takes {} of it",
                            format_length(size),
                            format_length(taken)
                        ),
                        env.file,
                        banding.thickness.span,
                        "make the band thinner, or band fewer edges",
                    )
                    .into());
            }
        }
        Ok(Some(Banding {
            thickness: round_length(thickness)?,
            faces,
        }))
    }
}

//! The spaces a window offers repeats and boundaries for (`docs/gui.md`,
//! "Changing what is there"), read from the model rather than guessed from
//! boxes on the screen.
//!
//! ```text
//! filled_spaces(model, "drawer") ──▶ [drawer_bay[bottom], drawer_bay[top]]
//!     each element's first Region param, as the model binds it
//! split_boundaries(model, design, drawer_bay[bottom]) ──▶ [SplitBoundary]
//!     the split's boundary grips (grips::split_grips), with their sizes
//! SplitBoundary::settle(raw) ──▶ where a boundary dragged to `raw` settles
//! ```
//!
//! - [`filled_spaces`] says which space an assembly of the project fills:
//!   for each of its elements, the region its component's first `Region`
//!   param is bound to, followed through the model (`bay`,
//!   `drawer_bay[same]`, `case.inside`, a mirror's own param). This is the
//!   param [`crate::repeat::repeat_assembly`] rewrites, so an assembly with
//!   a filled space is one a repeat can be asked of.
//! - [`split_boundaries`] lists the boundaries between the pieces of a
//!   split from [`crate::grips::split_grips`] (label order, the region
//!   array's name, the two pieces' ids), with each piece's size, and
//!   [`SplitBoundary::settle`] snaps a dragged boundary to an even
//!   division, to round sizes, else to the project's resolution.

use stave_lang::placement::Axis;
use stave_lang::units::{Length, NM_PER_MM};
use stave_model::Model;
use stave_model::defs::{AssemblyKind, DefId, Index, Member, RExpr, RExprKind, Symbol};
use stave_model::ids::{AssemblyId, ParamId, RegionId};
use stave_model::instance::{Ctx, ItemSlot, ParamSource, Slot};
use stave_model::solved::SolvedDesign;
use stave_model::types::Ty;

use crate::grips::{GripSize, split_grips};

/// The spaces the assembly `item` of the project fills: for each of its
/// elements in order (one for an assembly that is not an array), the
/// region its component's first `Region` param is bound to. Empty when it
/// is not an assembly of the project, its component has no `Region` param,
/// or the param is not bound to one region.
pub fn filled_spaces(model: &Model, item: &str) -> Vec<RegionId> {
    let Some(root) = model.module_instance(model.root) else {
        return Vec::new();
    };
    let Some(instance) = model.instance(root) else {
        return Vec::new();
    };
    let slot = instance
        .items
        .iter()
        .find_map(|(def, slot)| match (def, slot) {
            (DefId::Assembly(id), ItemSlot::Assembly(slot))
                if model
                    .defs
                    .assembly(*id)
                    .is_some_and(|def| def.decl.name == item) =>
            {
                Some(slot)
            }
            _ => None,
        });
    let Some(slot) = slot else {
        return Vec::new();
    };
    let found: Option<Vec<RegionId>> = slot
        .all()
        .into_iter()
        .map(|assembly| space_of(model, assembly))
        .collect();
    found.unwrap_or_default()
}

/// The region one assembly's first `Region` param is bound to.
fn space_of(model: &Model, assembly: AssemblyId) -> Option<RegionId> {
    let inst = model.assemblies.get(assembly.index())?;
    let component = model.defs.component(inst.component)?;
    let param_def = component
        .params
        .iter()
        .copied()
        .find(|param| model.defs.param(*param).is_some_and(|p| p.ty == Ty::Region))?;
    let inner = model.instance(inst.inner)?;
    let ItemSlot::Param(param) = inner.items.get(&DefId::Param(param_def))? else {
        return None;
    };
    param_region(model, *param, 0)
}

/// The region a `Region` param's value names, following a mirror's param
/// to the one it is the same as (at most a few deep).
fn param_region(model: &Model, param: ParamId, depth: usize) -> Option<RegionId> {
    let inst = model.params.get(param.index())?;
    match inst.source {
        ParamSource::Argument {
            assembly,
            index,
            ctx,
        } => {
            let def = model.defs.assembly(assembly)?;
            let AssemblyKind::Instance { args, .. } = &def.kind else {
                return None;
            };
            let (_, expr) = args.get(index)?;
            match bound(model, expr, ctx)? {
                Bound::Region(region) => Some(region),
                Bound::Assembly(_) => None,
            }
        }
        ParamSource::SameAs(other) if depth < 8 => param_region(model, other, depth + 1),
        // A default is evaluated in the component's own instance, and a
        // space a component makes for itself is not one it fills.
        ParamSource::SameAs(_) | ParamSource::Default => None,
    }
}

/// What a name in a `Region` param's value stands for.
enum Bound {
    Region(RegionId),
    Assembly(AssemblyId),
}

/// The one region or assembly an expression names where it is evaluated:
/// `bay`, `drawer_bay[same]`, `drawer_bay[top]`, `case.inside`,
/// `divider.above`.
fn bound(model: &Model, expr: &RExpr, ctx: Ctx) -> Option<Bound> {
    match &expr.kind {
        RExprKind::Symbol(Symbol::Def(def)) => single(model.item(*def, ctx)?, None),
        RExprKind::Index { base, index } => {
            let RExprKind::Symbol(Symbol::Def(def)) = &base.kind else {
                return None;
            };
            let element = match index {
                Index::Same => ctx.element?,
                Index::Label(position) => *position,
                Index::All | Index::Number(_) => return None,
            };
            single(model.item(*def, ctx)?, Some(element))
        }
        RExprKind::Member {
            base,
            member: Member::AssemblyItem(def),
        } => {
            let Bound::Assembly(assembly) = bound(model, base, ctx)? else {
                return None;
            };
            let inner = model.assemblies.get(assembly.index())?.inner;
            single(model.instance(inner)?.items.get(def)?, None)
        }
        _ => None,
    }
}

/// The region or assembly in a slot: the item itself, or one element of
/// an array.
fn single(slot: &ItemSlot, element: Option<usize>) -> Option<Bound> {
    match slot {
        ItemSlot::Region(slot) => pick(slot, element).map(Bound::Region),
        ItemSlot::Assembly(slot) => pick(slot, element).map(Bound::Assembly),
        ItemSlot::Param(_)
        | ItemSlot::Let(_)
        | ItemSlot::Part(_)
        | ItemSlot::Joint(_)
        | ItemSlot::Hardware(_)
        | ItemSlot::Require(_) => None,
    }
}

/// The item of a slot, or one element of an array slot.
fn pick<T: Copy>(slot: &Slot<T>, element: Option<usize>) -> Option<T> {
    match (slot, element) {
        (Slot::One(id), None) => Some(*id),
        (Slot::Many(_), Some(index)) => slot.element(index),
        (Slot::One(_), Some(_)) | (Slot::Many(_), None) => None,
    }
}

/// The boundary between two neighbouring pieces of a split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitBoundary {
    /// The region array the split makes: `drawer_bay`.
    pub region: String,
    /// The first of the two pieces, from 0 in label order: what
    /// [`crate::editing::EditCommand::MoveShares`] takes.
    pub lower: usize,
    /// The piece below (left of, in front of) the boundary.
    pub low: RegionId,
    /// The piece above it.
    pub high: RegionId,
    /// The axis the split runs along.
    pub axis: Axis,
    /// The two pieces' sizes along it.
    pub sizes: (Length, Length),
    /// Every piece's size if the split were even.
    pub even: Length,
    /// The project's `resolution:`.
    pub resolution: Length,
}

impl SplitBoundary {
    /// The two pieces' sizes together: what moving the boundary keeps.
    pub fn pair(&self) -> Length {
        self.sizes
            .0
            .checked_add(self.sizes.1)
            .unwrap_or(self.sizes.0)
    }

    /// Where a boundary dragged so the lower piece is `raw` long settles:
    /// on half of the two pieces, on the whole split's even size, or on a
    /// round size (a whole 10mm) when it is near one, else on a step of the
    /// project's resolution; never less than one step either side.
    pub fn settle(&self, raw: Length) -> Length {
        let total = self.pair().nm();
        let step = self.resolution.nm().max(1);
        let near = (total / 30).max(NM_PER_MM);
        let raw = raw.nm().clamp(step, (total - step).max(step));
        for target in [total / 2, self.even.nm()] {
            if target > 0 && target < total && (raw - target).abs() <= near {
                return Length::from_nm(target);
            }
        }
        let round = 10 * NM_PER_MM;
        let nearest_round = (raw + round / 2) / round * round;
        if (raw - nearest_round).abs() <= near / 6 && nearest_round > 0 && nearest_round < total {
            return Length::from_nm(nearest_round);
        }
        Length::from_nm((raw + step / 2) / step * step)
    }
}

/// The boundaries between the pieces of the split `region` is one piece
/// of, in label order. Empty when it is not a piece of a split array.
pub fn split_boundaries(
    model: &Model,
    design: &SolvedDesign,
    region: RegionId,
) -> Vec<SplitBoundary> {
    let Ok(grips) = split_grips(model, design, region) else {
        return Vec::new();
    };
    let size = |id: RegionId, axis: Axis| {
        design
            .regions()
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.bounds.size(axis))
    };
    let mut found: Vec<SplitBoundary> = grips
        .into_iter()
        .filter_map(|grip| {
            let GripSize::Boundary {
                region,
                lower,
                low,
                high,
            } = grip.size
            else {
                return None;
            };
            let axis = grip.axis?;
            Some(SplitBoundary {
                sizes: (size(low, axis)?, size(high, axis)?),
                region,
                lower,
                low,
                high,
                axis,
                even: Length::ZERO,
                resolution: design.project.resolution,
            })
        })
        .collect();
    // The even size: every piece's size together, shared out.
    let whole = found
        .iter()
        .map(|boundary| boundary.sizes.0)
        .chain(found.last().map(|boundary| boundary.sizes.1))
        .try_fold(Length::ZERO, Length::checked_add);
    let pieces = i64::try_from(found.len().saturating_add(1)).unwrap_or(1);
    if let Some(whole) = whole {
        for boundary in &mut found {
            boundary.even = Length::from_nm(whole.nm() / pieces);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use stave_modules::MemoryFiles;

    use super::*;

    /// A chest with a case in its envelope, a back, and two drawers in a
    /// split of the space in front of the back, 3 : 2 from the bottom.
    const CHEST: &str = r#"stave 1

import "std:casework"

project "Chest" {
  units:      mm
  resolution: 0.5mm
}

param overall_width  = 600mm
param overall_height = 700mm
param overall_depth  = 450mm

region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)

stock oak_19   { species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2700mm }
stock maple_12 { species: hard_maple, form: solid, thickness: 12mm, max_width: 150mm, max_length: 2400mm }
stock ply_6    { species: birch, form: plywood, thickness: 6mm, max_length: 2440mm }

assembly case : casework.Case {
  space:       envelope
  panel_stock: oak_19
}

assembly back : casework.Back {
  sides:       case.side[*]
  stands_on:   case.base
  under:       case.top
  panel_stock: ply_6
}

region drawer_bay[bottom, top] = split(back.in_front, up, shares: [3, 2])

assembly drawer[bottom, top] : casework.Drawer {
  opening:      drawer_bay[same]
  mount:        overlay(left: 18mm, right: 18mm, top: -1.5mm, bottom: 8mm, meeting: -0.75mm)
  front_stock:  oak_19
  box_stock:    maple_12
  bottom_stock: ply_6
  slide:        side_mount_slide(length: 400mm)
}
"#;

    #[test]
    fn the_space_an_assembly_fills_is_read_from_the_model() {
        let files = MemoryFiles::new().with("chest/chest.stave", CHEST);
        let opened = crate::open_with(Path::new("chest"), &files).unwrap();
        let (model, design) = (
            opened.model.as_ref().unwrap(),
            opened.design.as_ref().unwrap(),
        );
        let named = |ids: Vec<RegionId>| {
            ids.iter()
                .map(|id| {
                    design
                        .regions()
                        .iter()
                        .find(|r| r.id == *id)
                        .map(|r| r.name.clone())
                        .unwrap()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(named(filled_spaces(model, "case")), ["envelope"]);
        assert_eq!(
            named(filled_spaces(model, "drawer")),
            ["drawer_bay[bottom]", "drawer_bay[top]"]
        );
        // The back is put against parts, not into a space.
        assert!(filled_spaces(model, "back").is_empty());
        assert!(filled_spaces(model, "nothing").is_empty());

        let bottom = filled_spaces(model, "drawer")[0];
        let boundaries = split_boundaries(model, design, bottom);
        let [boundary] = boundaries.as_slice() else {
            panic!("one boundary: {boundaries:?}");
        };
        assert_eq!(boundary.region, "drawer_bay");
        assert_eq!(boundary.lower, 0);
        assert_eq!(
            named(vec![boundary.low, boundary.high]),
            ["drawer_bay[bottom]", "drawer_bay[top]"]
        );
        assert_eq!(boundary.axis, Axis::Up);
        // 3 : 2.
        assert_eq!(boundary.sizes.0.nm() * 2, boundary.sizes.1.nm() * 3);
        assert_eq!(boundary.even.nm() * 2, boundary.pair().nm());
        assert_eq!(boundary.resolution, Length::from_nm(500_000));
        // The envelope is not a piece of a split.
        assert!(split_boundaries(model, design, filled_spaces(model, "case")[0]).is_empty());
    }

    fn boundary(sizes: (i64, i64), even: i64) -> SplitBoundary {
        SplitBoundary {
            region: "bay".to_owned(),
            lower: 0,
            low: RegionId::from_usize(0),
            high: RegionId::from_usize(1),
            axis: Axis::Up,
            sizes: (Length::from_mm(sizes.0), Length::from_mm(sizes.1)),
            even: Length::from_mm(even),
            resolution: Length::from_mm(1),
        }
    }

    #[test]
    fn a_boundary_settles_on_even_and_round_sizes() {
        let mm = Length::from_mm;
        let pair = boundary((250, 250), 0);
        // Near half of the two: half.
        assert_eq!(pair.settle(mm(247)), mm(250));
        // Near the whole split's even size.
        assert_eq!(boundary((250, 250), 166).settle(mm(168)), mm(166));
        // Near a round size: it.
        assert_eq!(pair.settle(Length::from_nm(179_600_000)), mm(180));
        // Elsewhere, a step of the resolution.
        assert_eq!(pair.settle(Length::from_nm(134_400_000)), mm(134));
        // Never past either end.
        assert_eq!(pair.settle(mm(-20)), mm(1));
        assert_eq!(pair.settle(mm(900)), mm(499));
    }
}

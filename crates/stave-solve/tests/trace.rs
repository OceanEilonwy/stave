//! The dependency trace (`stave_solve::trace`) and single-param inversion
//! (`stave_model::trace::invert`), on `examples/clothing_cabinet` and on
//! small designs these tests write themselves.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use stave_lang::placement::{Axis, PartAxis};
use stave_lang::units::{ExactLength, Length, Rational};
use stave_model::Model;
use stave_model::defs::StockField;
use stave_model::ids::{ParamId, PartId, RegionId};
use stave_model::trace::{
    Amount, Followed, Form, Freedom, InvertError, Linear, Quantity, Reason, Source, Trace, invert,
};
use stave_modules::{DiskFiles, FileSource, ModuleSet};
use stave_solve::trace::{TraceError, trace};

/// The disk, with some files' texts replaced.
struct Changed(BTreeMap<PathBuf, String>);

impl FileSource for Changed {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        DiskFiles.list_stave_files(folder)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        match self.0.get(&key) {
            Some(text) => Ok(text.clone()),
            None => DiskFiles.read(path),
        }
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        DiskFiles.canonical(folder)
    }
}

fn cabinet_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet")
}

fn model_from(modules: &ModuleSet) -> Model {
    let (defs, diagnostics) = stave_model::resolve(modules);
    assert!(
        diagnostics.iter().all(|d| !d.is_error()),
        "resolve: {diagnostics:?}"
    );
    let (model, diagnostics) = stave_model::instance::instantiate(defs, modules.root);
    assert!(
        diagnostics.iter().all(|d| !d.is_error()),
        "instantiate: {diagnostics:?}"
    );
    model
}

fn cabinet() -> Model {
    model_from(&ModuleSet::load(&cabinet_folder()).unwrap())
}

/// A one-file project written to a folder of its own.
fn design(name: &str, text: &str) -> Model {
    let folder = std::env::temp_dir().join(format!("stave-trace-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("project.stave"), text).unwrap();
    let model = model_from(&ModuleSet::load(&folder).unwrap());
    std::fs::remove_dir_all(&folder).unwrap();
    model
}

fn part(model: &Model, name: &str) -> PartId {
    model
        .parts
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no part {name}"))
        .id
}

fn region(model: &Model, name: &str) -> RegionId {
    model
        .regions
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no region {name}"))
        .id
}

fn param(model: &Model, name: &str) -> ParamId {
    model
        .params
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no param {name}"))
        .id
}

fn stock(model: &Model, name: &str) -> stave_model::ids::StockId {
    let index = model
        .defs
        .stocks
        .iter()
        .position(|s| s.decl.name == name)
        .unwrap_or_else(|| panic!("no stock {name}"));
    stave_model::ids::StockId::from_usize(index)
}

fn mm(n: i64) -> Amount {
    Amount::Length(Length::from_mm(n).to_exact())
}

/// `numerator / denominator` millimetres.
fn mm_fraction(numerator: i128, denominator: i128) -> Amount {
    let nm = Rational::new(numerator * 1_000_000, denominator).unwrap();
    Amount::Length(ExactLength::new(nm))
}

fn linear(trace: &Trace) -> &Linear {
    match &trace.form {
        Form::Linear(linear) => linear,
        Form::NotLinear(not) => panic!("not linear: {not:?}"),
    }
}

fn coefficient(linear: &Linear, source: &Source) -> Rational {
    linear
        .term(source)
        .unwrap_or_else(|| panic!("no term for {source:?} in {linear:?}"))
        .coefficient
}

fn whole(n: i64) -> Rational {
    Rational::from(n)
}

#[test]
fn the_drawer_box_width_follows_cabinet_width() {
    let model = cabinet();
    // The box's width between its sides: the sub-front, a fit between
    // the two sides, which are spaced across the opening a slide's
    // clearance in from each side. The opening is a piece of a split up
    // the front opening, which runs from side_l's right face to side_r's
    // left face: cabinet_width less two oak_19 sides.
    //
    //   sub_front = cabinet_width - 2 × oak_19.thickness
    //               - 2 × clearance - 2 × maple_12.thickness
    //             = 900 - 38 - 25.4 - 24 = 812.6mm
    let traced = trace(
        &model,
        Quantity::PartSize {
            part: part(&model, "drawer[top].sub_front"),
            axis: PartAxis::Length,
        },
    )
    .unwrap();
    assert_eq!(traced.value, mm_fraction(8126, 10));
    let linear = linear(&traced);
    let cabinet_width = param(&model, "cabinet_width");
    assert_eq!(linear.coefficient(cabinet_width), whole(1));
    assert_eq!(
        coefficient(
            linear,
            &Source::StockField {
                stock: stock(&model, "oak_19"),
                field: StockField::Thickness
            }
        ),
        whole(-2)
    );
    assert_eq!(
        coefficient(
            linear,
            &Source::StockField {
                stock: stock(&model, "maple_12"),
                field: StockField::Thickness
            }
        ),
        whole(-2)
    );
    let clearance = linear
        .terms
        .iter()
        .find(|t| matches!(&t.dependence.source, Source::KindSize { name, .. } if name == "clearance"))
        .expect("the slide's clearance");
    assert_eq!(clearance.coefficient, whole(-2));
    assert_eq!(clearance.dependence.freedom, Freedom::KindSize);
    // Everything written as a plain number is in the constant: here
    // nothing, since every number comes from a stock, a kind or a param.
    assert_eq!(linear.constant, mm(0));
    assert_eq!(linear.terms.len(), 4);

    // Holding what you buy, it is cabinet_width less 87.4mm.
    let held = linear.holding_stops().unwrap();
    assert_eq!(held.constant, mm_fraction(-874, 10));
    assert_eq!(held.terms.len(), 1);

    // cabinet_width is a plain project param, one component level out.
    let width = linear.term(&Source::Param(cabinet_width)).unwrap();
    assert_eq!(width.dependence.freedom, Freedom::Plain);
    assert_eq!(width.dependence.levels, 1);
    assert_eq!(width.dependence.value, mm(900));
    assert!(width.dependence.domain.is_some());
}

#[test]
fn a_size_written_as_a_plain_number_traces_to_itself() {
    let model = cabinet();
    // shelf: against(top.bottom, gap: 160mm)
    let traced = trace(
        &model,
        Quantity::PlaceOffset {
            part: part(&model, "shelf"),
            axis: Axis::Up,
        },
    )
    .unwrap();
    assert_eq!(traced.value, mm(160));
    let linear = linear(&traced);
    assert_eq!(linear.constant, mm(160));
    assert!(linear.terms.is_empty());
    assert!(traced.followed.is_empty());
}

#[test]
fn a_component_param_written_as_a_number_is_its_own_candidate() {
    let model = cabinet();
    let top_gap = param(&model, "drawer[top].top_gap");
    let traced = trace(&model, Quantity::Param(top_gap)).unwrap();
    let linear = linear(&traced);
    assert_eq!(linear.constant, mm(0));
    assert_eq!(linear.coefficient(top_gap), whole(1));
    let term = linear.term(&Source::Param(top_gap)).unwrap();
    assert_eq!(term.dependence.levels, 0);
    assert_eq!(term.dependence.freedom, Freedom::Plain);
}

#[test]
fn a_param_whose_value_is_an_expression_is_followed() {
    let model = cabinet();
    // box_height = opening.up - bottom_gap - top_gap, and the opening is
    // half of the drawer zone's opening: drawer_zone less base and the
    // divider's thickness... read from the solver, checked by the trace.
    let box_height = param(&model, "drawer[top].box_height");
    let traced = trace(&model, Quantity::Param(box_height)).unwrap();
    let linear = linear(&traced);
    assert_eq!(linear.coefficient(box_height), Rational::ZERO);
    assert_eq!(
        linear.coefficient(param(&model, "drawer[top].top_gap")),
        whole(-1)
    );
    assert_eq!(
        linear.coefficient(param(&model, "drawer[top].bottom_gap")),
        whole(-1)
    );
    assert_eq!(
        linear.coefficient(param(&model, "drawer_zone")),
        Rational::new(1, 2).unwrap()
    );
    assert!(traced.followed.contains(&Followed::Param {
        id: box_height,
        freedom: Freedom::Plain,
        levels: 0
    }));
    // The drawer side's width is box_height, and follows it the same way.
    let side = trace(
        &model,
        Quantity::PartSize {
            part: part(&model, "drawer[top].side[left]"),
            axis: PartAxis::Width,
        },
    )
    .unwrap();
    assert_eq!(side.form, traced.form);
}

#[test]
fn the_reveal_shows_as_locked() {
    let model = cabinet();
    // door_l's stiles run the height of the door: the bay, plus 8mm below,
    // less the reveal above (top: -reveal).
    let traced = trace(
        &model,
        Quantity::PartSize {
            part: part(&model, "door_l.stile[hinge]"),
            axis: PartAxis::Length,
        },
    )
    .unwrap();
    let linear = linear(&traced);
    let reveal = param(&model, "reveal");
    let term = linear.term(&Source::Param(reveal)).unwrap();
    assert_eq!(term.dependence.freedom, Freedom::Locked);
    assert!(!term.dependence.freedom.is_candidate());
    assert_eq!(term.coefficient, whole(-1));
    assert_eq!(term.dependence.levels, 1);
    assert_eq!(invert(&traced, reveal, mm(1000)), Err(InvertError::Locked));
    // The mirrored door follows the same values.
    let mirrored = trace(
        &model,
        Quantity::PartSize {
            part: part(&model, "door_r.stile[hinge]"),
            axis: PartAxis::Length,
        },
    )
    .unwrap();
    assert_eq!(mirrored.form, traced.form);
}

#[test]
fn the_door_bay_is_half_the_opening() {
    let model = cabinet();
    let traced = trace(
        &model,
        Quantity::RegionSize {
            region: region(&model, "door_bay[left]"),
            axis: Axis::Across,
        },
    )
    .unwrap();
    // (cabinet_width - 2 × oak_19.thickness) / 2 = 431mm
    assert_eq!(traced.value, mm(431));
    let linear = linear(&traced);
    let cabinet_width = param(&model, "cabinet_width");
    assert_eq!(
        linear.coefficient(cabinet_width),
        Rational::new(1, 2).unwrap()
    );
    assert_eq!(
        coefficient(
            linear,
            &Source::StockField {
                stock: stock(&model, "oak_19"),
                field: StockField::Thickness
            }
        ),
        whole(-1)
    );
    assert_eq!(linear.terms.len(), 2);
    let width = linear.term(&Source::Param(cabinet_width)).unwrap();
    assert_eq!(width.dependence.levels, 0);
}

#[test]
fn making_the_door_bay_400mm_wide_by_cabinet_width_re_solves_to_400mm() {
    let model = cabinet();
    let bay = region(&model, "door_bay[left]");
    let cabinet_width = param(&model, "cabinet_width");
    let traced = trace(
        &model,
        Quantity::RegionSize {
            region: bay,
            axis: Axis::Across,
        },
    )
    .unwrap();
    let answer = invert(&traced, cabinet_width, mm(400)).unwrap();
    assert_eq!(answer, mm(838));
    let Amount::Length(length) = answer else {
        panic!("a length");
    };
    let length = length.to_length().unwrap();

    // Write it into the source and solve again.
    let folder = cabinet_folder();
    let file = std::fs::canonicalize(folder.join("cabinet.stave")).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let changed_text = set_param(
        &text,
        "cabinet_width",
        &format!("{}mm", length.nm() / 1_000_000),
    );
    assert_ne!(changed_text, text);
    let changed = Changed(BTreeMap::from([(file, changed_text)]));
    let modules = ModuleSet::load_with(&folder, &changed).unwrap();
    let model = model_from(&modules);
    let (design, diagnostics) = stave_solve::solve(&model);
    assert!(
        diagnostics.iter().all(|d| !d.is_error()),
        "solve: {diagnostics:?}"
    );
    let design = design.unwrap();
    let solved = design
        .regions()
        .iter()
        .find(|r| r.name == "door_bay[left]")
        .unwrap();
    assert_eq!(solved.bounds.size(Axis::Across), Length::from_mm(400));

    // A target the range doesn't allow is refused, with the value it
    // would need.
    assert_eq!(
        invert(&traced, cabinet_width, mm(700)),
        Err(InvertError::OutsideRange { value: mm(1438) })
    );
    // A param the bay doesn't follow has no answer.
    assert_eq!(
        invert(&traced, param(&model, "drawer_zone"), mm(400)),
        Err(InvertError::ZeroCoefficient)
    );
}

/// The text with the value of `name`'s declaration replaced.
fn set_param(text: &str, name: &str, value: &str) -> String {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let words: Vec<&str> = line.split_whitespace().collect();
        let declares = words
            .iter()
            .position(|w| *w == "param")
            .is_some_and(|i| words.get(i + 1) == Some(&name));
        if declares {
            let equals = line.find('=').unwrap();
            let rest = &line[equals + 1..];
            let start = rest.len() - rest.trim_start().len();
            let end = rest[start..]
                .find(char::is_whitespace)
                .map_or(rest.len(), |e| start + e);
            out.push_str(&line[..equals + 1]);
            out.push_str(&rest[..start]);
            out.push_str(value);
            out.push_str(&rest[end..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

const PRODUCT: &str = r#"stave 1

project "Trace test" {
  units: mm
}

param width  = 600mm within 400mm..800mm
param factor = 1.5 within 1..2
param extra  = 100mm within 0mm..400mm
let   margin = 10mm

stock pine_19 { species: white_oak, form: solid, thickness: 19mm, max_width: 300mm, max_length: 2400mm }

part top : pine_19 {
  length: width * factor
  width:  200mm
  place {
    ground
  }
}

part shelf : pine_19 {
  length: width / factor
  width:  200mm - margin
  place {
    against(top.bottom, gap: 100mm)
    flush(top.left)
    flush(top.front)
  }
}

part bench : pine_19 {
  length: width * factor + extra
  width:  200mm
  place {
    against(shelf.bottom, gap: 100mm)
    flush(top.left)
    flush(top.front)
  }
}

part brace : pine_19 {
  length: width / factor * factor
  width:  100mm
  place {
    against(bench.bottom, gap: 100mm)
    flush(top.left)
    flush(top.front)
  }
}
"#;

#[test]
fn a_product_of_two_params_is_not_linear() {
    let model = design("product", PRODUCT);
    let width = param(&model, "width");
    let factor = param(&model, "factor");
    let traced = trace(
        &model,
        Quantity::PartSize {
            part: part(&model, "top"),
            axis: PartAxis::Length,
        },
    )
    .unwrap();
    assert_eq!(traced.value, mm(900));
    let Form::NotLinear(not) = &traced.form else {
        panic!("linear: {:?}", traced.form);
    };
    assert_eq!(not.reasons, vec![Reason::Product]);
    // With the other held, the size is still linear in each of the two
    // (grammar, "Smart edits", rule 2): `width`'s coefficient is the
    // factor, `factor`'s is the width in nanometres.
    let held: Vec<(&Source, Rational)> = not
        .held
        .iter()
        .map(|t| (&t.dependence.source, t.coefficient))
        .collect();
    assert_eq!(
        held,
        vec![
            (&Source::Param(width), Rational::new(3, 2).unwrap()),
            (&Source::Param(factor), whole(600_000_000)),
        ]
    );
    assert!(not.dependences.is_empty());
    // 1000mm / 1.5 = 666 2/3 mm.
    assert_eq!(invert(&traced, width, mm(1000)), Ok(mm_fraction(2000, 3)));

    // A division by a param is not linear in it; the dividend still is.
    let shelf = trace(
        &model,
        Quantity::PartSize {
            part: part(&model, "shelf"),
            axis: PartAxis::Length,
        },
    )
    .unwrap();
    let Form::NotLinear(not) = &shelf.form else {
        panic!("linear: {:?}", shelf.form);
    };
    assert_eq!(not.reasons, vec![Reason::Division]);
    let named: Vec<&Source> = not.dependences.iter().map(|d| &d.source).collect();
    assert_eq!(named, vec![&Source::Param(factor)]);
    assert_eq!(not.held.len(), 1);
    assert_eq!(not.held[0].dependence.source, Source::Param(width));
    assert_eq!(not.held[0].coefficient, Rational::new(2, 3).unwrap());
    assert_eq!(invert(&shelf, factor, mm(300)), Err(InvertError::NotLinear));
    assert_eq!(invert(&shelf, width, mm(500)), Ok(mm(750)));

    // A param multiplied by itself is not linear in it: `width / factor *
    // factor` has `factor` on both sides of the product, while `width`
    // keeps its coefficient of exactly one.
    let brace = trace(
        &model,
        Quantity::PartSize {
            part: part(&model, "brace"),
            axis: PartAxis::Length,
        },
    )
    .unwrap();
    assert_eq!(brace.value, mm(600));
    let Form::NotLinear(not) = &brace.form else {
        panic!("linear: {:?}", brace.form);
    };
    assert_eq!(not.reasons, vec![Reason::Product, Reason::Division]);
    let named: Vec<&Source> = not.dependences.iter().map(|d| &d.source).collect();
    assert_eq!(named, vec![&Source::Param(factor)]);
    let held: Vec<(&Source, Rational)> = not
        .held
        .iter()
        .map(|t| (&t.dependence.source, t.coefficient))
        .collect();
    assert_eq!(held, vec![(&Source::Param(width), Rational::ONE)]);

    // A let is followed to its number and is never a candidate.
    let narrow = trace(
        &model,
        Quantity::PartSize {
            part: part(&model, "shelf"),
            axis: PartAxis::Width,
        },
    )
    .unwrap();
    let linear = linear(&narrow);
    assert_eq!(linear.constant, mm(190));
    assert!(linear.terms.is_empty());
    assert!(matches!(
        narrow.followed.as_slice(),
        [Followed::Let { levels: 0, .. }]
    ));
}

/// The length of `bench` in the PRODUCT design after `name` is set to
/// `value`, solved again from the changed text.
fn bench_length_with(name: &str, value: &str) -> Amount {
    let changed = set_param(PRODUCT, name, value);
    assert_ne!(changed, PRODUCT);
    let model = design(&format!("product-{name}"), &changed);
    trace(
        &model,
        Quantity::PartSize {
            part: part(&model, "bench"),
            axis: PartAxis::Length,
        },
    )
    .unwrap()
    .value
}

#[test]
fn a_param_added_to_a_product_is_still_linear_in_itself() {
    // `width * factor + extra`: the product makes the size not linear as
    // a whole, but `extra` enters with a coefficient of exactly one, and
    // each factor is linear with the other held.
    let model = design("bench", PRODUCT);
    let width = param(&model, "width");
    let factor = param(&model, "factor");
    let extra = param(&model, "extra");
    let traced = trace(
        &model,
        Quantity::PartSize {
            part: part(&model, "bench"),
            axis: PartAxis::Length,
        },
    )
    .unwrap();
    assert_eq!(traced.value, mm(1000));
    let Form::NotLinear(not) = &traced.form else {
        panic!("linear: {:?}", traced.form);
    };
    assert_eq!(not.reasons, vec![Reason::Product]);
    assert!(not.dependences.is_empty());
    let held: Vec<(&Source, Rational)> = not
        .held
        .iter()
        .map(|t| (&t.dependence.source, t.coefficient))
        .collect();
    assert_eq!(
        held,
        vec![
            (&Source::Param(width), Rational::new(3, 2).unwrap()),
            (&Source::Param(factor), whole(600_000_000)),
            (&Source::Param(extra), Rational::ONE),
        ]
    );
    assert!(
        not.held
            .iter()
            .all(|t| t.dependence.freedom == Freedom::Plain)
    );

    // Each answer, written into the source and solved again, gives the
    // target exactly.
    assert_eq!(invert(&traced, extra, mm(1200)), Ok(mm(300)));
    assert_eq!(bench_length_with("extra", "300mm"), mm(1200));
    assert_eq!(invert(&traced, width, mm(1150)), Ok(mm(700)));
    assert_eq!(bench_length_with("width", "700mm"), mm(1150));
    assert_eq!(
        invert(&traced, factor, mm(1300)),
        Ok(Amount::Ratio(Rational::from(2_i64)))
    );
    assert_eq!(bench_length_with("factor", "2"), mm(1300));

    // The domain still applies to a param held this way.
    assert_eq!(
        invert(&traced, extra, mm(1500)),
        Err(InvertError::OutsideRange { value: mm(600) })
    );
}

#[test]
fn traces_are_the_same_on_every_run() {
    let model = cabinet();
    let quantity = Quantity::PartSize {
        part: part(&model, "drawer[bottom].front"),
        axis: PartAxis::Width,
    };
    let first = trace(&model, quantity).unwrap();
    for _ in 0..3 {
        assert_eq!(trace(&cabinet(), quantity).unwrap(), first);
    }
}

#[test]
fn every_part_and_region_of_the_cabinet_traces_or_says_why_not() {
    // The trace checks itself against the solver: no size may disagree.
    let model = cabinet();
    let mut quantities = Vec::new();
    for p in &model.parts {
        for axis in [PartAxis::Length, PartAxis::Width, PartAxis::Thickness] {
            quantities.push(Quantity::PartSize { part: p.id, axis });
        }
        for axis in Axis::ALL {
            quantities.push(Quantity::PlaceOffset { part: p.id, axis });
        }
    }
    for r in &model.regions {
        for axis in Axis::ALL {
            quantities.push(Quantity::RegionSize { region: r.id, axis });
        }
    }
    for p in &model.params {
        quantities.push(Quantity::Param(p.id));
    }
    let mut linear = 0;
    for (quantity, result) in quantities
        .iter()
        .zip(stave_solve::trace::trace_all(&model, &quantities))
    {
        match result {
            Ok(trace) => {
                if matches!(trace.form, Form::Linear(_)) {
                    linear += 1;
                }
            }
            // A term with no offset, and a param that is a region, a
            // stock or a kind.
            Err(TraceError::NoOffset | TraceError::NotAQuantity) => {}
            Err(other) => panic!("{quantity:?}: {other}"),
        }
    }
    assert!(linear > 100, "only {linear} sizes are linear");
}

#[test]
fn an_equal_split_has_no_shares() {
    let model = cabinet();
    assert_eq!(
        trace(
            &model,
            Quantity::Share {
                region: region(&model, "drawer_bay[top]")
            }
        ),
        Err(TraceError::NoShares)
    );
}

#[test]
fn no_size_in_the_examples_disagrees_with_the_solver() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut folders: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        let model = model_from(&ModuleSet::load(&folder).unwrap());
        let mut quantities = Vec::new();
        for p in &model.parts {
            for axis in [PartAxis::Length, PartAxis::Width, PartAxis::Thickness] {
                quantities.push(Quantity::PartSize { part: p.id, axis });
            }
        }
        for r in &model.regions {
            for axis in Axis::ALL {
                quantities.push(Quantity::RegionSize { region: r.id, axis });
            }
        }
        for (quantity, result) in quantities
            .iter()
            .zip(stave_solve::trace::trace_all(&model, &quantities))
        {
            if let Err(error @ TraceError::Disagrees { .. }) = result {
                panic!("{}: {quantity:?}: {error}", folder.display());
            }
        }
    }
}

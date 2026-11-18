//! The two check tiers over the corpus: the fast tier's diagnostics
//! finished by the slow tier run to completion give exactly the single
//! check's diagnostics, in the same order; the fast tier's are the single
//! check's layers 1 to 3, in order; and its box overlaps
//! cover every overlap at rest between two parts that aren't joined to
//! each other.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use stave_check::{Cancelled, ContactItem, check, check_fast, check_slow, check_with_contacts};
use stave_lang::Code;
use stave_model::Model;
use stave_model::solved::{CutSource, HardwareId, PartId, SolvedDesign};
use stave_modules::ModuleSet;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every project folder of `examples/`, `tests/broken/` and
/// `tests/designs/`, sorted.
fn corpus() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    for group in ["examples", "tests/broken", "tests/designs"] {
        let Ok(entries) = std::fs::read_dir(workspace().join(group)) else {
            continue;
        };
        folders.extend(
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir()),
        );
    }
    folders.sort();
    assert!(folders.len() > 50, "the corpus is there");
    folders
}

/// The model and solved design of a folder, when it solved without an
/// error before the checks (the only designs the checks run on).
fn solved(folder: &Path) -> Option<(Model, SolvedDesign)> {
    let modules = ModuleSet::load(folder).ok()?;
    let mut errors = modules.has_errors();
    let (defs, diagnostics) = stave_model::resolve(&modules);
    errors |= diagnostics.iter().any(|d| d.is_error());
    let (model, diagnostics) = stave_model::instance::instantiate(defs, modules.root);
    errors |= diagnostics.iter().any(|d| d.is_error());
    let (design, diagnostics) = stave_solve::solve(&model);
    errors |= diagnostics.iter().any(|d| d.is_error());
    let design = design?;
    (!errors).then_some((model, design))
}

#[test]
fn fast_then_slow_gives_exactly_the_single_check() {
    let mut checked = 0;
    let mut layered = 0;
    for folder in corpus() {
        let Some((model, design)) = solved(&folder) else {
            continue;
        };
        let what = folder.display().to_string();
        let fast = check_fast(&model, &design);
        let slow = check_slow(&model, &design, &fast, &AtomicBool::new(false))
            .unwrap_or_else(|_| panic!("{what}: never cancelled"));
        let single = check_with_contacts(&model, &design);
        assert_eq!(slow, single, "{what}");
        // The fast tier's diagnostics are the single check's layers 1 to
        // 3, in its order: what an editor shows while the pointer moves
        // is what it shows once the slow tier is done. (A wall rule the
        // walls check can't measure is that layer-4 check's own warning.)
        let early: Vec<_> = single
            .iter()
            .filter(|(d, _)| d.code.layer() <= 3 && !d.message.starts_with("measuring wall("))
            .cloned()
            .collect();
        assert_eq!(fast.diagnostics, early, "{what}");
        layered += usize::from(!early.is_empty());
        let diagnostics: Vec<_> = slow.into_iter().map(|(d, _)| d).collect();
        assert_eq!(diagnostics, check(&model, &design), "{what}");
        checked += 1;
    }
    assert!(checked > 30, "most of the corpus reaches the checks");
    assert!(
        layered >= 2,
        "the checks make layer 1 to 3 diagnostics: {layered}"
    );
}

#[test]
fn a_cancelled_slow_tier_stops() {
    let folder = workspace().join("examples/clothing_cabinet");
    let (model, design) = solved(&folder).unwrap();
    assert_eq!(
        check_slow(
            &model,
            &design,
            &check_fast(&model, &design),
            &AtomicBool::new(true)
        ),
        Err(Cancelled)
    );
}

#[test]
fn the_examples_have_no_box_overlaps() {
    for folder in corpus() {
        if !folder.starts_with(workspace().join("examples")) {
            continue;
        }
        // A library has nothing to place.
        let Some((model, design)) = solved(&folder) else {
            continue;
        };
        assert_eq!(
            check_fast(&model, &design).overlaps,
            Vec::new(),
            "{}",
            folder.display()
        );
    }
}

/// `true` if a joint joins the two parts, or the hardware cuts the part:
/// what lets two items share room.
fn related(design: &SolvedDesign, a: ContactItem, b: ContactItem) -> bool {
    let joined = |x: PartId, y: PartId| {
        design
            .joints
            .iter()
            .flat_map(|joint| &joint.pairs)
            .any(|pair| {
                let has = |part| {
                    pair.roles.iter().any(|role| role.part == part)
                        || pair.made_parts.contains(&part)
                };
                has(x) && has(y)
            })
    };
    let cuts = |hardware: HardwareId, part: PartId| {
        design.part(part).is_some_and(|part| {
            part.cuts
                .iter()
                .any(|cut| cut.source == CutSource::Hardware { hardware })
        })
    };
    match (a, b) {
        (ContactItem::Part(x), ContactItem::Part(y)) => joined(x, y),
        (ContactItem::Part(part), ContactItem::Hardware(hardware))
        | (ContactItem::Hardware(hardware), ContactItem::Part(part)) => cuts(hardware, part),
        (ContactItem::Hardware(x), ContactItem::Hardware(y)) => x == y,
        (ContactItem::Member(_), _) | (_, ContactItem::Member(_)) => true,
    }
}

#[test]
fn every_overlap_at_rest_of_unrelated_items_is_a_box_overlap() {
    let mut seen = 0;
    for folder in corpus() {
        let Some((model, design)) = solved(&folder) else {
            continue;
        };
        let what = folder.display().to_string();
        let fast = check_fast(&model, &design).overlaps;
        for (diagnostic, contact) in check_with_contacts(&model, &design) {
            let Some(contact) = contact else { continue };
            let at_rest = contact.scene.is_none() && contact.motion.is_none();
            if diagnostic.code != Code::Overlap || !at_rest {
                continue;
            }
            // The item a diagnostic names first, against each other one.
            let Some((first, others)) = contact.items.split_first() else {
                continue;
            };
            for other in others {
                if related(&design, *first, *other) {
                    continue;
                }
                let (a, b) = ((*first).min(*other), (*first).max(*other));
                assert!(
                    fast.iter().any(|overlap| overlap.a == a && overlap.b == b),
                    "{what}: {} has no box overlap for {a:?} and {b:?}",
                    diagnostic.message
                );
                seen += 1;
            }
        }
    }
    assert!(seen >= 3, "the corpus has overlaps of unrelated items");
}

#[test]
fn box_overlaps_name_the_items_of_the_broken_examples() {
    let cases = [
        ("tests/broken/knob_hits_top", 1),
        ("tests/broken/door_overlay_into_top", 2),
        ("tests/broken/dowels_cross_dowels", 1),
    ];
    for (folder, count) in cases {
        let (model, design) = solved(&workspace().join(folder)).unwrap();
        let fast = check_fast(&model, &design).overlaps;
        assert_eq!(fast.len(), count, "{folder}: {fast:?}");
        for overlap in fast {
            assert!(overlap.a < overlap.b && !overlap.regions.is_empty());
        }
    }
}

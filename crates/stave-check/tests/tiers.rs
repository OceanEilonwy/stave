//! The two check tiers over the corpus: the fast tier's diagnostics
//! finished by the slow tier run to completion give exactly the single
//! check's diagnostics, in the same order; the fast tier's are the single
//! check's layers 1 to 3, in order; and its box overlaps
//! cover every overlap at rest between two parts that aren't joined to
//! each other.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use stave_check::{Cancelled, ContactItem, check, check_fast, check_slow, check_with_contacts};
use stave_lang::{Code, Diagnostic, Severity};
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
    solved_with_warnings(folder).map(|(model, design, _)| (model, design))
}

/// [`solved`], with the warnings the stages before the checks made.
fn solved_with_warnings(folder: &Path) -> Option<(Model, SolvedDesign, Vec<Diagnostic>)> {
    let modules = ModuleSet::load(folder).ok()?;
    let mut before = modules.diagnostics.clone();
    let (defs, diagnostics) = stave_model::resolve(&modules);
    before.extend(diagnostics);
    let (model, diagnostics) = stave_model::instance::instantiate(defs, modules.root);
    before.extend(diagnostics);
    let (design, diagnostics) = stave_solve::solve(&model);
    before.extend(diagnostics);
    let design = design?;
    let errors = modules.has_errors() || before.iter().any(Diagnostic::is_error);
    (!errors).then_some((model, design, before))
}

/// What a folder of the corpus must report, written down independently of
/// the checks: nothing for an example or a design of `tests/designs/`
/// (they check clean), and for a broken design its `// expect:` line, or
/// nothing while it is `// pending:`.
fn expected(folder: &Path) -> Vec<(Severity, Code, String)> {
    if !folder.starts_with(workspace().join("tests/broken")) {
        return Vec::new();
    }
    let text = std::fs::read_to_string(folder.join("project.stave")).unwrap();
    let mut lines = text.lines();
    let first = lines.next().unwrap();
    if lines
        .next()
        .is_some_and(|line| line.starts_with("// pending: "))
    {
        return Vec::new();
    }
    let (severity, rest) = match first.strip_prefix("// expect: error[") {
        Some(rest) => (Severity::Error, rest),
        None => (
            Severity::Warning,
            first.strip_prefix("// expect: warning[").unwrap(),
        ),
    };
    let (code, message) = rest.split_once("] ").unwrap();
    vec![(
        severity,
        Code::from_str_code(code).unwrap(),
        message.to_owned(),
    )]
}

/// Diagnostics as what they say, sorted and without repeats, the way the
/// pipeline shows them.
fn said(diagnostics: impl IntoIterator<Item = Diagnostic>) -> Vec<(Severity, Code, String)> {
    let mut said: Vec<_> = diagnostics
        .into_iter()
        .map(|d| (d.severity, d.code, d.message))
        .collect();
    said.sort_by(|a, b| (a.1, &a.2).cmp(&(b.1, &b.2)));
    said.dedup();
    said
}

#[test]
fn fast_then_slow_gives_exactly_the_single_check() {
    let mut checked = 0;
    let mut layered = 0;
    let mut found = 0;
    for folder in corpus() {
        let Some((model, design, before)) = solved_with_warnings(&folder) else {
            continue;
        };
        let what = folder.display().to_string();
        let fast = check_fast(&model, &design);
        let slow = check_slow(&model, &design, &fast, &AtomicBool::new(false))
            .unwrap_or_else(|_| panic!("{what}: never cancelled"));
        // Against an answer that doesn't come from the checks at all: the
        // corpus says what each design reports, and the stages before the
        // checks with the two tiers together say exactly that.
        let whole = said(
            before
                .iter()
                .cloned()
                .chain(slow.iter().map(|(d, _)| d.clone())),
        );
        assert_eq!(whole, expected(&folder), "{what}");
        found += usize::from(!slow.is_empty());
        // And against the single check, which doesn't take the fast
        // tier's findings: the same diagnostics in the same order.
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
    assert!(
        found >= 10,
        "the two tiers find the broken designs' diagnostics: {found}"
    );
}

/// Cancelled at any moment, the slow tier either says it was cancelled or
/// gives the whole list: never a list cut short that looks complete. The
/// flag is set from another thread at moments spread over a whole run.
#[test]
fn a_cancelled_slow_tier_says_so() {
    let folder = workspace().join("examples/clothing_cabinet");
    let (model, design) = solved(&folder).unwrap();
    let fast = check_fast(&model, &design);
    // Set before it starts: cancelled at once.
    assert_eq!(
        check_slow(&model, &design, &fast, &AtomicBool::new(true)),
        Err(Cancelled)
    );
    let started = Instant::now();
    let whole = check_slow(&model, &design, &fast, &AtomicBool::new(false)).unwrap();
    let full_run = started.elapsed();
    const MOMENTS: u32 = 24;
    let mut cancelled = 0;
    for step in 0..MOMENTS {
        let cancel = AtomicBool::new(false);
        let delay = full_run * step / MOMENTS;
        let result = std::thread::scope(|scope| {
            let run = scope.spawn(|| check_slow(&model, &design, &fast, &cancel));
            std::thread::sleep(delay);
            cancel.store(true, Ordering::Relaxed);
            run.join().unwrap()
        });
        match result {
            Err(Cancelled) => cancelled += 1,
            Ok(found) => assert_eq!(found, whole, "cancelled after {delay:?}: a partial list"),
        }
    }
    assert!(cancelled > 0, "some runs were cancelled part way");
    assert_eq!(
        Cancelled.to_string(),
        "the check was cancelled before it finished"
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
            .joints()
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

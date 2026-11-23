//! Fixes (grammar, "Fixes"): every fix a check offers is a patch that,
//! applied to the design with `stave_lang`'s patch application, makes its
//! diagnostic go away. Each test finds the diagnostic, applies its fix,
//! checks the patched design again, and looks for the diagnostic.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stave_lang::{
    Code, Diagnostic, Fix, ModuleFile, PatchTarget, SourceMap, apply_patch, parse_file,
    parse_patch_file,
};
use stave_modules::{MemoryFiles, ModuleSet};

/// A project's files by name, all in one folder (the project's module).
type Files = BTreeMap<String, String>;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The files of a design in `tests/broken`.
fn broken(name: &str) -> Files {
    let folder = workspace().join("tests/broken").join(name);
    let mut files = Files::new();
    for entry in std::fs::read_dir(&folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "stave") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            files.insert(name, std::fs::read_to_string(&path).unwrap());
        }
    }
    files
}

/// A one-file project.
fn one_file(text: &str) -> Files {
    Files::from([("project.stave".to_owned(), text.to_owned())])
}

/// Every diagnostic of every stage and the checks.
fn check(files: &Files) -> Vec<Diagnostic> {
    let folder = Path::new("/memory/project");
    let memory = files
        .iter()
        .fold(MemoryFiles::new(), |memory, (name, text)| {
            memory.with(folder.join(name), text.as_str())
        });
    let modules = ModuleSet::load_with(folder, &memory).unwrap();
    let mut diagnostics = modules.diagnostics.clone();
    let (defs, found) = stave_model::resolve(&modules);
    diagnostics.extend(found);
    let (model, found) = stave_model::instance::instantiate(defs, modules.root);
    diagnostics.extend(found);
    let (design, found) = stave_solve::solve(&model);
    diagnostics.extend(found);
    if let Some(design) = &design
        && !diagnostics.iter().any(Diagnostic::is_error)
    {
        diagnostics.extend(stave_check::check(&model, design));
    }
    diagnostics
}

/// The one diagnostic with this code whose message starts with `start`.
fn find(diagnostics: &[Diagnostic], code: Code, start: &str) -> Option<Diagnostic> {
    diagnostics
        .iter()
        .find(|d| d.code == code && d.message.starts_with(start))
        .cloned()
}

/// The files with the fix applied.
fn apply(files: &Files, fix: &Fix) -> Files {
    let mut sources = SourceMap::new();
    let parses: Vec<(String, _)> = files
        .iter()
        .map(|(name, text)| {
            let file = sources.add(name.as_str(), text.as_str()).unwrap();
            (name.clone(), parse_file(file, text))
        })
        .collect();
    let target = PatchTarget::new(
        parses
            .iter()
            .map(|(name, parse)| ModuleFile {
                name: name.as_str(),
                parse,
            })
            .collect(),
    );
    let text = fix.patch_file();
    let file = sources.add("fix.stave", text.as_str()).unwrap();
    let patch = parse_patch_file(file, &text);
    let changed =
        apply_patch(&target, &patch).unwrap_or_else(|error| panic!("{fix} doesn't apply: {error}"));
    assert!(!changed.is_empty(), "{fix} changes nothing");
    let mut patched = files.clone();
    for file in changed {
        patched.insert(file.name, file.text);
    }
    patched
}

/// Checks `files`, finds the diagnostic (code and the start of its
/// message), and returns its fixes, each proved: applied, the patched
/// design no longer has the diagnostic.
fn proved_fixes(files: &Files, code: Code, start: &str) -> Vec<Fix> {
    let before = check(files);
    let found = find(&before, code, start).unwrap_or_else(|| {
        panic!("no {code} '{start}...' in {before:#?}");
    });
    assert!(found.help.is_some(), "a fix comes with its help line");
    for fix in &found.fixes {
        let patched = apply(files, fix);
        let after = check(&patched);
        assert!(
            find(&after, code, start).is_none(),
            "{fix} leaves the diagnostic: {after:#?}"
        );
    }
    found.fixes
}

fn texts(fixes: &[Fix]) -> Vec<String> {
    fixes.iter().map(ToString::to_string).collect()
}

#[test]
fn a_kind_setting_out_of_range_is_set_to_the_nearest_allowed_value() {
    let fixes = proved_fixes(&broken("out_of_range"), Code::OutOfRange, "'length'");
    assert_eq!(
        texts(&fixes),
        ["patch \"Use 300mm\" { set runner.length = 300mm }"]
    );
}

const SHELF: &str = "stave 1

project \"Shelf\" {
  units: mm
}

stock pine_19 { species: pine, form: solid, thickness: 19mm, max_length: 2400mm }

PARAM

part base : pine_19 {
  length: width
  width:  300mm
  place {
    ground
  }
}

component Block {
  param height : Length within 50mm..200mm
  part block : pine_19 {
    length: height
    width:  100mm
    place {
      orient(length: up, width: back)
      against(base.top)
      flush(base.left)
      flush(base.front)
    }
  }
  joint fixing : glued_face(block, into: base)
}

assembly block : Block {
  height: HEIGHT
}
";

fn shelf(param: &str, height: &str) -> Files {
    one_file(&SHELF.replace("PARAM", param).replace("HEIGHT", height))
}

#[test]
fn a_component_argument_out_of_range_is_set_to_the_nearest_end() {
    let files = shelf("param width = 600mm", "250mm");
    let fixes = proved_fixes(&files, Code::OutOfRange, "'height' must be within");
    assert_eq!(
        texts(&fixes),
        ["patch \"Use 200mm\" { set block.height = 200mm }"]
    );
    let files = shelf("param width = 600mm", "20mm");
    let fixes = proved_fixes(&files, Code::OutOfRange, "'height' must be within");
    assert_eq!(
        texts(&fixes),
        ["patch \"Use 50mm\" { set block.height = 50mm }"]
    );
}

/// One mistake, one error: a param outside its `within` is not also a
/// placement cycle of the part that reads it, whether the value is a
/// component argument or a module param's default.
#[test]
fn a_param_out_of_range_reports_nothing_else() {
    for files in [
        shelf("param width = 600mm", "250mm"),
        shelf("param width = 1300mm within 400mm..1200mm", "100mm"),
    ] {
        let diagnostics = check(&files);
        let codes: Vec<Code> = diagnostics.iter().map(|d| d.code).collect();
        assert_eq!(codes, [Code::OutOfRange], "{diagnostics:#?}");
    }
}

#[test]
fn a_param_default_out_of_its_range_is_set_to_the_nearest_end() {
    let files = shelf("param width = 1300mm within 400mm..1200mm", "100mm");
    let fixes = proved_fixes(&files, Code::OutOfRange, "'width' must be within");
    assert_eq!(
        texts(&fixes),
        ["patch \"Use 1200mm\" { set width = 1200mm }"]
    );
}

#[test]
fn a_param_default_out_of_its_list_is_set_to_the_nearest_value() {
    let files = shelf("param width = 640mm within [500mm, 600mm, 700mm]", "100mm");
    let fixes = proved_fixes(&files, Code::OutOfRange, "'width' must be within");
    assert_eq!(texts(&fixes), ["patch \"Use 600mm\" { set width = 600mm }"]);
}

#[test]
fn a_solved_param_without_a_range_has_no_fix() {
    let diagnostics = check(&broken("solved_without_within"));
    let found = find(&diagnostics, Code::MissingField, "solved param").unwrap();
    assert!(found.help.is_some());
    assert!(found.fixes.is_empty(), "{:?}", found.fixes);
}

#[test]
fn a_door_that_hits_beside_its_hinge_edge_gets_the_overlay_that_clears() {
    // The overlay is written in the component's cover region: the fix
    // sets that edge, to an exact length, and keeps the rest as written.
    let fixes = proved_fixes(
        &broken("door_hits_side"),
        Code::Motion,
        "door.panel hits wall when door is opened past",
    );
    assert_eq!(
        texts(&fixes),
        [
            "patch \"Use left: 16mm\" { set Door.face = cover(opening, overlay(left: 16mm, right: 17.8mm), thickness: ply_18.thickness) }"
        ]
    );
}

#[test]
fn mirrored_doors_that_hit_each_other_get_the_overlay_that_clears() {
    // The overlay is the source door's mount; the mirrored door follows.
    let fixes = proved_fixes(
        &broken("double_doors_no_gap"),
        Code::Motion,
        "door_l.panel hits door_r.panel when door_l is opened past",
    );
    assert_eq!(
        texts(&fixes),
        [
            "patch \"Use right: 8mm\" { set door_l.mount = overlay(left: 16mm, right: 8mm, top: -2mm, bottom: 16mm) }"
        ]
    );
}

#[test]
fn a_door_that_hits_late_opens_no_further_where_its_kind_allows() {
    let fixes = proved_fixes(
        &broken("knob_hits_door"),
        Code::Motion,
        "door_r.panel hits door_l.handle's knob when door_r is opened past",
    );
    assert_eq!(
        texts(&fixes),
        ["patch \"Open no further than 106.8deg\" { set Door.hinges.opening_angle = 106.8deg }"]
    );
    let fixes = proved_fixes(
        &broken("lid_hits_back"),
        Code::Motion,
        "lid.panel hits back when lid is opened past",
    );
    assert_eq!(
        texts(&fixes),
        ["patch \"Open no further than 109.9deg\" { set Lid.hinges.opening_angle = 109.9deg }"]
    );
}

#[test]
fn an_opening_the_kind_does_not_allow_is_not_offered() {
    // The doors meet at 71.5deg, and the hinge opens to no less than
    // 85deg: there is nothing to set, so the help explains in words.
    let fixes = proved_fixes(
        &broken("doors_hit_each_other"),
        Code::Motion,
        "door_r.panel hits door_l.panel when door_r is opened past",
    );
    assert!(fixes.is_empty(), "{:?}", texts(&fixes));
}

#[test]
fn a_door_given_a_mount_gets_the_mount_that_clears() {
    // `door_hits_side` with the overlay passed in as the door's mount:
    // the fix sets that edge of the mount, and leaves the component alone.
    let mut files = broken("door_hits_side");
    let text = files.get_mut("project.stave").unwrap();
    *text = text
        .replace(
            "  param hinge_side : Part\n",
            "  param hinge_side : Part\n  param mount      : Mount\n",
        )
        .replace(
            "cover(opening, overlay(left: 17.8mm, right: 17.8mm), thickness: ply_18.thickness)",
            "cover(opening, mount, thickness: ply_18.thickness)",
        )
        .replace(
            "  hinge_side: side_l\n",
            "  hinge_side: side_l\n  mount:      overlay(left: 17.8mm, right: 17.8mm)\n",
        );
    let fixes = proved_fixes(
        &files,
        Code::Motion,
        "door.panel hits wall when door is opened past",
    );
    assert_eq!(
        texts(&fixes),
        ["patch \"Use left: 16mm\" { set door.mount = overlay(left: 16mm, right: 17.8mm) }"]
    );
}

const MIRRORED_DOOR_HITS_WALL: &str = include_str!("fixtures/mirrored_door_hits_wall.stave");

#[test]
fn a_mirrored_door_that_hits_gets_its_sources_overlay_on_the_mirrored_edge() {
    // door_r's right edge hits the wall; door_r is door_l's image, so the
    // overlay that clears is door_l's left one.
    let fixes = proved_fixes(
        &one_file(MIRRORED_DOOR_HITS_WALL),
        Code::Motion,
        "door_r.panel hits wall when door_r is opened past",
    );
    assert_eq!(
        texts(&fixes),
        ["patch \"Use left: 16mm\" { set door_l.mount = overlay(left: 16mm, right: 8mm) }"]
    );
}

#[test]
fn a_mirrored_door_with_its_overlay_in_the_component_gets_the_mirrored_edge() {
    let text = MIRRORED_DOOR_HITS_WALL
        .replace("  param mount      : Mount\n", "")
        .replace(
            "cover(opening, mount, thickness: ply_18.thickness)",
            "cover(opening, overlay(left: 17.8mm, right: 8mm), thickness: ply_18.thickness)",
        )
        .replace("  mount:      overlay(left: 17.8mm, right: 8mm)\n", "");
    let fixes = proved_fixes(
        &one_file(&text),
        Code::Motion,
        "door_r.panel hits wall when door_r is opened past",
    );
    assert_eq!(
        texts(&fixes),
        [
            "patch \"Use left: 16mm\" { set Door.face = cover(opening, overlay(left: 16mm, right: 8mm), thickness: ply_18.thickness) }"
        ]
    );
}

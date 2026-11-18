//! Signature matching and the patch a pick writes, on the clothing cabinet
//! (`examples/clothing_cabinet`): a click in `door_bay[left]` lists
//! `casework.Drawer` and `casework.FramePanelDoor`, and picking the drawer
//! writes a patch that applies and parses, and whose result resolves.

use std::path::{Path, PathBuf};

use stave_app::candidates::{EditScope, Repeat};
use stave_app::edit::{
    Applicability, Candidate, Declaration, MenuGroup, Picked, PlaceSlot, SettingValue,
};
use stave_app::naming::RepeatCount;
use stave_lang::placement::Axis;
use stave_lang::{Diagnostic, apply_patch};
use stave_modules::MemoryFiles;

fn cabinet_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/clothing_cabinet")
        .canonicalize()
        .unwrap()
}

/// Every `.stave` file under `folder`, read into memory.
fn read_tree(folder: &Path, files: &mut MemoryFiles) {
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            read_tree(&path, files);
        } else if path.extension().is_some_and(|e| e == "stave") {
            files.insert(&path, std::fs::read_to_string(&path).unwrap());
        }
    }
}

/// Applies a patch to the cabinet and opens the result: the patched files'
/// text, and the errors of layers 1 and 2 (syntax, names and types).
fn apply_and_open(
    scope: &EditScope<'_>,
    patch: &stave_app::candidates::WrittenPatch,
) -> (Vec<(String, String)>, Vec<Diagnostic>) {
    let changed = apply_patch(&scope.patch_target(), &patch.parse)
        .unwrap_or_else(|error| panic!("the patch applies: {error}\n{}", patch.text));
    let folder = cabinet_folder();
    let mut files = MemoryFiles::new();
    read_tree(&folder, &mut files);
    let mut texts = Vec::new();
    for file in &changed {
        let parse = stave_lang::parse_file(
            stave_lang::SourceMap::new()
                .add(&file.name, file.text.as_str())
                .unwrap(),
            &file.text,
        );
        assert!(
            !parse.has_errors(),
            "{} parses after the patch:\n{}\n{:?}",
            file.name,
            file.text,
            parse.diagnostics
        );
        files.insert(folder.join(&file.name), file.text.clone());
        texts.push((file.name.clone(), file.text.clone()));
    }
    let reopened = stave_app::open_with(&folder, &files).unwrap();
    let model = reopened
        .model
        .as_ref()
        .expect("the patched design resolves");
    assert!(
        model
            .assemblies
            .iter()
            .any(|a| a.name.starts_with(patch.name.as_str())),
        "the new assembly is in the patched design"
    );
    let early: Vec<Diagnostic> = reopened
        .diagnostics
        .into_iter()
        .filter(|d| d.is_error() && d.code.layer() <= 2)
        .collect();
    (texts, early)
}

fn by_label<'a>(candidates: &'a [Candidate], label: &str) -> &'a Candidate {
    candidates
        .iter()
        .find(|c| c.entry.label.as_str() == label)
        .unwrap_or_else(|| {
            let labels: Vec<&str> = candidates.iter().map(|c| c.entry.label.as_str()).collect();
            panic!("{label} is in the menu {labels:?}")
        })
}

fn setting<'a>(candidate: &'a Candidate, name: &str) -> &'a SettingValue {
    &candidate
        .settings
        .iter()
        .find(|s| s.name.as_str() == name)
        .unwrap_or_else(|| panic!("{name} is a setting"))
        .value
}

#[test]
fn a_click_behind_the_left_door_lists_the_drawer_and_the_door() {
    let opened = stave_app::open(&cabinet_folder()).unwrap();
    let scope = EditScope::new(&opened).unwrap();
    let model = scope.model();
    let door_bay_left = model
        .regions
        .iter()
        .find(|r| r.name == "door_bay[left]")
        .unwrap()
        .id;
    let menu = scope.candidates(&Picked::Region(door_bay_left));

    let drawer = by_label(&menu, "Drawer");
    let door = by_label(&menu, "FramePanelDoor");
    for candidate in [drawer, door] {
        assert!(matches!(candidate.declaration, Declaration::Component(_)));
        assert_eq!(
            candidate.entry.group,
            MenuGroup::Standard(
                stave_lang::ast::Name::new("casework", Default::default()).unwrap()
            )
        );
        assert_eq!(candidate.applicability, Applicability::NotChecked);
        assert_eq!(candidate.place[0].name.as_str(), "opening");
        assert_eq!(
            candidate.place[0].slot,
            PlaceSlot::Region(Some(door_bay_left))
        );
    }
    assert_eq!(
        drawer.entry.hint.as_deref(),
        Some("A drawer on side-mount slides: an applied front on a box with a grooved-in bottom.")
    );
    assert_eq!(
        door.entry.hint.as_deref(),
        Some("A frame-and-panel door: two stiles, two rails and a floating panel.")
    );

    // The drawer is complete: its settings without a default are copied
    // from drawer[top], the last drawer in the text.
    assert!(drawer.is_complete());
    let drawer_top = model
        .assemblies
        .iter()
        .find(|a| a.name == "drawer[top]")
        .unwrap()
        .id;
    for name in ["mount", "front_stock", "box_stock", "bottom_stock", "slide"] {
        assert!(
            matches!(setting(drawer, name), SettingValue::Copied { from, .. } if *from == drawer_top),
            "{name}: {:?}",
            setting(drawer, name)
        );
    }
    for name in ["pull_kind", "bottom_gap", "top_gap", "box_height"] {
        assert_eq!(setting(drawer, name), &SettingValue::Default, "{name}");
    }

    // The door still needs its hinge side pointed at; its settings come
    // from door_l.
    let unbound: Vec<&str> = door.unbound_place().map(|p| p.name.as_str()).collect();
    assert_eq!(unbound, ["hinge_side"]);
    let door_l = model
        .assemblies
        .iter()
        .find(|a| a.name == "door_l")
        .unwrap()
        .id;
    assert!(matches!(setting(door, "frame"), SettingValue::Copied { from, .. } if *from == door_l));
    assert_eq!(setting(door, "knob_kind"), &SettingValue::Default);

    // Menu order: the project's own group (empty here), then imported
    // modules, then the standard ones, each by name.
    let groups: Vec<&MenuGroup> = menu.iter().map(|c| &c.entry.group).collect();
    let mut sorted = groups.clone();
    sorted.sort_by(|a, b| stave_app::candidates::compare_groups(a, b));
    assert_eq!(groups, sorted);
}

#[test]
fn picking_the_drawer_writes_a_patch_that_applies_and_parses() {
    let opened = stave_app::open(&cabinet_folder()).unwrap();
    let scope = EditScope::new(&opened).unwrap();
    let door_bay_left = scope
        .model()
        .regions
        .iter()
        .find(|r| r.name == "door_bay[left]")
        .unwrap()
        .id;
    let menu = scope.candidates(&Picked::Region(door_bay_left));
    let drawer = by_label(&menu, "Drawer");

    let written = scope.write_patch(drawer, None).unwrap();
    assert_eq!(written.name.as_str(), "drawer_2");
    // door_bay is the first item of the project it refers to.
    assert_eq!(written.file, "carcass.stave");
    assert_eq!(written.patch.ops.len(), 2, "{}", written.text);
    assert_eq!(
        written.text,
        r#"stave 1

patch "Add a drawer" {
  add import "std:casework" in "carcass.stave"
  add assembly drawer_2 : casework.Drawer {
    opening:      door_bay[left]
    mount:        overlay(left: 18mm, right: 18mm, top: 8mm, bottom: 8mm, meeting: -reveal / 2)
    front_stock:  oak_19
    box_stock:    maple_12
    bottom_stock: ply_6
    slide:        side_mount_slide(length: slide_length)
  } in "carcass.stave"
}
"#
    );

    let (texts, early) = apply_and_open(&scope, &written);
    assert_eq!(texts.len(), 1);
    assert!(texts[0].1.contains("import \"std:casework\""));
    assert!(texts[0].1.contains("assembly drawer_2 : casework.Drawer {"));
    assert!(early.is_empty(), "names and types resolve: {early:#?}");
}

#[test]
fn a_repeat_of_drawers_adds_its_bay_and_an_array() {
    let opened = stave_app::open(&cabinet_folder()).unwrap();
    let scope = EditScope::new(&opened).unwrap();
    let door_bay_left = scope
        .model()
        .regions
        .iter()
        .find(|r| r.name == "door_bay[left]")
        .unwrap()
        .id;
    let menu = scope.candidates(&Picked::Region(door_bay_left));
    let drawer = by_label(&menu, "Drawer");
    let repeat = Repeat {
        count: RepeatCount::new(3).unwrap(),
        axis: Axis::Up,
    };
    let written = scope.write_patch(drawer, Some(repeat)).unwrap();
    // drawer_bay is taken by the cabinet's own drawers.
    assert_eq!(written.bay.as_ref().unwrap().as_str(), "drawer_bay_2");
    assert!(
        written.text.contains(
            "  add region drawer_bay_2[bottom, middle, top] = split(door_bay[left], up) in \"carcass.stave\"\n"
        ),
        "{}",
        written.text
    );
    assert!(
        written
            .text
            .contains("add assembly drawer_2[bottom, middle, top] : casework.Drawer {")
    );
    assert!(written.text.contains("opening:      drawer_bay_2[same]"));
    assert!(
        written
            .text
            .starts_with("stave 1\n\npatch \"Add 3 drawers\"")
    );
    let (_, early) = apply_and_open(&scope, &written);
    assert!(early.is_empty(), "names and types resolve: {early:#?}");
}

#[test]
fn the_door_waits_for_its_hinge_side() {
    let opened = stave_app::open(&cabinet_folder()).unwrap();
    let scope = EditScope::new(&opened).unwrap();
    let model = scope.model();
    let door_bay_left = model
        .regions
        .iter()
        .find(|r| r.name == "door_bay[left]")
        .unwrap()
        .id;
    let menu = scope.candidates(&Picked::Region(door_bay_left));
    let mut door = by_label(&menu, "FramePanelDoor").clone();
    assert_eq!(
        scope.write_patch(&door, None).unwrap_err(),
        stave_app::candidates::WriteError::Incomplete(vec!["hinge_side".to_owned()])
    );
    // Point at side_l: now it writes, into doors.stave's neighbour
    // carcass.stave (door_bay comes first), with the mount copied from
    // door_l.
    let side_l = model.parts.iter().find(|p| p.name == "side_l").unwrap().id;
    door.place[1].slot = PlaceSlot::Part(Some(side_l));
    let written = scope.write_patch(&door, None).unwrap();
    assert_eq!(written.name.as_str(), "frame_panel_door");
    assert!(
        written.text.contains("hinge_side:  side_l"),
        "{}",
        written.text
    );
    assert!(written.text.contains("mount:       overlay(left: 18mm, right: 18mm, top: -reveal, bottom: 8mm, meeting: -reveal / 2)"), "{}", written.text);
    let (_, early) = apply_and_open(&scope, &written);
    assert!(early.is_empty(), "names and types resolve: {early:#?}");
}

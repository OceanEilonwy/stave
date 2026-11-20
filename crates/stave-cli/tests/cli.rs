//! End-to-end tests of the `stave` binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn stave(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stave"))
        .args(args)
        .current_dir(workspace())
        .output()
        .unwrap()
}

/// A fresh folder under the system temp folder.
fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("stave-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

#[test]
fn check_passes_on_the_examples_and_std() {
    for folder in ["examples", "std"] {
        let output = stave(&["check", folder]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(stderr.contains(" 0 errors, 0 warnings"), "{stderr}");
    }
}

#[test]
fn check_reports_errors_and_fails() {
    let output = stave(&["check", "tests/broken/missing_value"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with(
            "error[missing-value]: 'length' has no value\n  --> tests/broken/missing_value/project.stave:14:3\n"
        ),
        "{stderr}"
    );
    assert!(
        stderr.ends_with("checked 1 file: 1 error, 0 warnings\n"),
        "{stderr}"
    );
}

#[test]
fn check_fails_cleanly_on_a_missing_path() {
    let output = stave(&["check", "no/such/folder"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("error: can't read 'no/such/folder'")
    );
}

#[test]
fn fmt_formats_in_place_and_checks() {
    let folder = scratch("fmt");
    let nested = folder.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let messy = nested.join("messy.stave");
    std::fs::write(&messy, "stave 1\nlet  a=1\n").unwrap();
    let tidy = folder.join("tidy.stave");
    std::fs::write(&tidy, "stave 1\nlet a = 1\n").unwrap();
    let folder_arg = folder.to_string_lossy().into_owned();

    let output = stave(&["fmt", "--check", &folder_arg]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, format!("would reformat {}\n", messy.display()));
    assert_eq!(
        std::fs::read_to_string(&messy).unwrap(),
        "stave 1\nlet  a=1\n"
    );

    let output = stave(&["fmt", &folder_arg]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&messy).unwrap(),
        "stave 1\nlet a = 1\n"
    );

    let output = stave(&["fmt", "--check", &folder_arg]);
    assert!(output.status.success());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn fmt_skips_files_with_errors() {
    let folder = scratch("fmt-errors");
    let broken = folder.join("broken.stave");
    std::fs::write(&broken, "stave 1\nlet a =\n").unwrap();
    let output = stave(&["fmt", "--check", &broken.to_string_lossy()]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr,
        format!(
            "skipped {}: 1 syntax error (see stave check)\n",
            broken.display()
        )
    );
    let output = stave(&["fmt", &broken.to_string_lossy()]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&broken).unwrap(),
        "stave 1\nlet a =\n"
    );
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn the_whole_repository_is_formatted() {
    let output = stave(&["fmt", "--check", "."]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn check_runs_the_whole_pipeline() {
    let output = stave(&["check", "tests/broken/axis_not_set"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("error[axis-not-set]: shelf: position along 'deep' is not set\n"),
        "{stderr}"
    );
}

/// The solved cabinet, checked by hand once (see the report of the change
/// that added it) and kept as a snapshot. Set `STAVE_UPDATE_SNAPSHOTS=1` to
/// rewrite it after an intended change, then check the difference by hand.
#[test]
fn dump_of_the_clothing_cabinet_matches_its_snapshot() {
    dump_matches_its_snapshot("clothing_cabinet");
}

/// `stave dump examples/<example>` is the same as
/// `snapshots/<example>.dump`, which was checked by hand when it was added.
fn dump_matches_its_snapshot(example: &str) {
    folder_dump_matches_its_snapshot(&format!("examples/{example}"), example);
}

/// `stave dump <folder>` is the same as `snapshots/<name>.dump`.
fn folder_dump_matches_its_snapshot(folder: &str, example: &str) {
    let output = stave(&["dump", folder]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let dump = String::from_utf8(output.stdout).unwrap();
    let snapshot = workspace()
        .join("crates/stave-cli/tests/snapshots")
        .join(format!("{example}.dump"));
    if std::env::var_os("STAVE_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&snapshot, &dump).unwrap();
    }
    let expected = std::fs::read_to_string(&snapshot).unwrap();
    assert!(
        dump == expected,
        "the dump changed; diff it against {}",
        snapshot.display()
    );
}

/// A spliced part (grammar, "Splices"): its pieces as parts, each with
/// its own cuts, and each splice as a joint.
#[test]
fn dump_of_the_spliced_designs_matches_their_snapshots() {
    folder_dump_matches_its_snapshot("tests/designs/spliced_bench", "spliced_bench");
    folder_dump_matches_its_snapshot("tests/designs/spliced_beam", "spliced_beam");
}

#[test]
fn dump_of_the_box_matches_its_snapshot() {
    dump_matches_its_snapshot("box");
}

#[test]
fn dump_of_the_side_table_matches_its_snapshot() {
    dump_matches_its_snapshot("side_table");
}

#[test]
fn dump_of_the_bookshelf_matches_its_snapshot() {
    dump_matches_its_snapshot("bookshelf");
}

#[test]
fn dump_of_the_stool_matches_its_snapshot() {
    dump_matches_its_snapshot("stool");
}

#[test]
fn dump_of_the_nightstand_matches_its_snapshot() {
    dump_matches_its_snapshot("nightstand");
}

#[test]
fn dump_of_the_wall_shelf_matches_its_snapshot() {
    dump_matches_its_snapshot("wall_shelf");
}

#[test]
fn dump_writes_json() {
    let output = stave(&["dump", "--json", "examples/clothing_cabinet"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["parts"].as_array().unwrap().len(), 34);
    assert_eq!(json["parts"][0]["name"], "base");
    assert_eq!(json["parts"][0]["cut"]["length"], 900_000_000);
    assert_eq!(json["project"]["units"], "mm");
    // Each hardware's solved `adjust` settings: the cabinet's first hinge
    // has its arm (member 1) set -0.5mm across.
    let setting = &json["hardware"][0]["settings"][0];
    assert_eq!(setting["body"]["body"], 1);
    assert_eq!(setting["axis"], "Across");
    assert_eq!(setting["offset"], -500_000);
}

#[test]
fn dump_refuses_a_library_and_reports_errors() {
    let output = stave(&["dump", "std/joinery"]);
    assert_eq!(output.status.code(), Some(2));
    let output = stave(&["dump", "tests/broken/unknown_name"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

#[test]
fn export_writes_a_valid_gltf_of_the_cabinet() {
    let output = scratch("export").join("cabinet.glb");
    let result = stave(&[
        "export",
        workspace()
            .join("examples/clothing_cabinet")
            .to_str()
            .unwrap(),
        "--gltf",
        output.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let (document, buffers, _) = gltf::import(&output).expect("a valid glTF file");
    // One node per part, named as in the design, and one per member of
    // each piece of hardware, named `<hardware> <member>`: five for each of
    // the four cup hinges (plate, arm, cup and two links), and two for each
    // of the four slides and the four handles (a bar or knob and its
    // screws); and one per joint pair's member, named `<joint>'s <member>`:
    // the dowels of the base and the top into each side, and the screws of
    // each drawer's front.
    let names: Vec<&str> = document.nodes().filter_map(|n| n.name()).collect();
    assert_eq!(names.len(), 34 + 4 * 5 + 4 * 2 + 4 * 2 + 2 * 2 + 2);
    assert!(names.contains(&"top_dowels's pin between side_l and top"));
    assert!(names.contains(&"door_l.hinges[lower] plate"));
    assert!(names.contains(&"door_l.hinges[lower] arm"));
    assert!(names.contains(&"door_l.hinges[lower] outer_link"));
    assert!(names.contains(&"drawer[top].slides[left] cabinet_member"));
    for name in [
        "base",
        "side_r",
        "door_l.stile[hinge]",
        "drawer[top].pull bar",
        "foot[back_left]",
    ] {
        assert!(names.contains(&name), "{name} missing");
    }
    // Wood by species and a metal for the hardware.
    let materials: Vec<&str> = document.materials().filter_map(|m| m.name()).collect();
    assert!(materials.contains(&"white oak"), "{materials:?}");
    assert!(materials.contains(&"brushed metal"), "{materials:?}");
    let metal = document
        .materials()
        .find(|m| m.name() == Some("brushed metal"))
        .unwrap();
    assert!(metal.pbr_metallic_roughness().metallic_factor() > 0.5);
    // Positions are in metres: the top is 1.7 m up; the feet go below 0.
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
            for position in reader.read_positions().unwrap() {
                low = low.min(position[1]);
                high = high.max(position[1]);
            }
            assert!(reader.read_indices().is_some());
        }
    }
    assert!((high - 1.7).abs() < 1e-4, "{high}");
    assert!((low + 0.08).abs() < 1e-4, "{low}");
}

#[test]
fn view_says_when_the_viewer_is_missing_or_starts_it() {
    // The test binary's folder holds stave; stave-gtk is there only when the
    // whole workspace was built. Either way the outcome is clear.
    let result = stave(&["view", "/nonexistent/folder"]);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success());
    assert!(
        stderr.contains("the viewer isn't installed") || stderr.contains("can't find the folder"),
        "{stderr}"
    );
}

/// `stave cut-list <folder>` is the same as `snapshots/<name>.cut-list`,
/// checked by hand when it was added (grammar, "Cut list").
fn cut_list_matches_its_snapshot(folder: &str, name: &str) {
    let output = stave(&["cut-list", folder]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let list = String::from_utf8(output.stdout).unwrap();
    let snapshot = workspace()
        .join("crates/stave-cli/tests/snapshots")
        .join(format!("{name}.cut-list"));
    if std::env::var_os("STAVE_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&snapshot, &list).unwrap();
    }
    let expected = std::fs::read_to_string(&snapshot).unwrap();
    assert!(
        list == expected,
        "the cut list changed; diff it against {}",
        snapshot.display()
    );
}

/// The shelf unit uses every allowance: rough sizes and the bought
/// thickness of its oak, a stock's own kerf, and edge banding.
#[test]
fn cut_list_of_the_shelf_unit_matches_its_snapshot() {
    cut_list_matches_its_snapshot("examples/shelf_unit", "shelf_unit");
}

/// The bookshelf, with no allowances written: the defaults.
#[test]
fn cut_list_of_the_bookshelf_matches_its_snapshot() {
    cut_list_matches_its_snapshot("examples/bookshelf", "bookshelf");
}

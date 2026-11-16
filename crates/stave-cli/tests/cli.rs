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
    let output = stave(&["dump", "examples/clothing_cabinet"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let dump = String::from_utf8(output.stdout).unwrap();
    let snapshot = workspace().join("crates/stave-cli/tests/snapshots/clothing_cabinet.dump");
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

#[test]
fn dump_writes_json() {
    let output = stave(&["dump", "--json", "examples/clothing_cabinet"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["parts"].as_array().unwrap().len(), 34);
    assert_eq!(json["parts"][0]["name"], "base");
    assert_eq!(json["parts"][0]["cut"]["length"], 900_000_000);
    assert_eq!(json["project"]["units"], "mm");
}

#[test]
fn dump_refuses_a_library_and_reports_errors() {
    let output = stave(&["dump", "std/joinery"]);
    assert_eq!(output.status.code(), Some(2));
    let output = stave(&["dump", "tests/broken/unknown_name"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

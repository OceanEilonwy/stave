//! Embeds the standard library: every `.stave` file under the repository's
//! `std/<module>/` becomes an `include_str!` entry of a generated table.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// The standard modules, the prelude's first. Kept equal to `STD_MODULES`.
const STD_MODULES: [&str; 5] = ["joinery", "hardware", "tools", "casework", "frames"];

fn main() -> Result<(), Box<dyn Error>> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let std_folder = manifest.join("../../std");
    println!("cargo:rerun-if-changed={}", std_folder.display());

    let mut table = String::from(
        "/// Every embedded standard-library file: (module, file name, text),\n\
         /// modules in prelude order, files sorted by name.\n\
         pub(crate) static STD_FILES: &[(&str, &str, &str)] = &[\n",
    );
    for module in STD_MODULES {
        let folder = std_folder.join(module);
        println!("cargo:rerun-if-changed={}", folder.display());
        let mut files: Vec<(String, PathBuf)> = Vec::new();
        for entry in fs::read_dir(&folder)? {
            let path = entry?.path();
            let is_stave = path
                .extension()
                .is_some_and(|extension| extension == "stave");
            if !is_stave || !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                return Err(format!("non-UTF-8 file name in {}", folder.display()).into());
            };
            files.push((name.to_owned(), path.clone()));
        }
        files.sort();
        for (name, path) in files {
            println!("cargo:rerun-if-changed={}", path.display());
            let absolute = absolute(&path)?;
            writeln!(
                table,
                "    ({module:?}, {name:?}, include_str!({absolute:?})),"
            )?;
        }
    }
    table.push_str("];\n");

    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    fs::write(out.join("std_files.rs"), table)?;
    Ok(())
}

/// The file's canonical path as a string, for `include_str!`.
fn absolute(path: &Path) -> Result<String, Box<dyn Error>> {
    let canonical = fs::canonicalize(path)?;
    canonical
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("non-UTF-8 path {}", canonical.display()).into())
}

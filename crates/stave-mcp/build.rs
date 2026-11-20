//! Puts the viewer page into the server: `target/viewer/stave-viewer.html`
//! as `tools/build-viewer.sh` builds it (or the file `STAVE_VIEWER_PAGE`
//! names), copied to `$OUT_DIR/stave-viewer.html` for `include_str!`.
//! When it hasn't been built, a page that says how to build it goes in
//! its place, so the server always builds.

use std::path::PathBuf;
use std::{env, fs};

/// The page embedded when the viewer hasn't been built. It still sets
/// `window.staveViewer`, so the host scripts around it work.
const NOT_BUILT: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Stave preview</title></head>
<body>
<p style="font: 16px system-ui, sans-serif; padding: 16px">Viewer not built: run tools/build-viewer.sh</p>
<script>window.staveViewer = { show: function () { return Promise.resolve(false); } };</script>
</body>
</html>
"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo::rerun-if-env-changed=STAVE_VIEWER_PAGE");
    let built = match env::var_os("STAVE_VIEWER_PAGE") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("no CARGO_MANIFEST_DIR")?)
            .join("../../target/viewer/stave-viewer.html"),
    };
    println!("cargo::rerun-if-changed={}", built.display());
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("no OUT_DIR")?).join("stave-viewer.html");
    if built.is_file() {
        fs::copy(&built, &out)?;
    } else {
        println!(
            "cargo::warning=the viewer isn't built ({}); the preview page says so. Run tools/build-viewer.sh.",
            built.display()
        );
        fs::write(&out, NOT_BUILT)?;
    }
    Ok(())
}

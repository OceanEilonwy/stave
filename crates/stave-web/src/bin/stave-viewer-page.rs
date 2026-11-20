//! Puts the viewer's page together: `stave-viewer-page GLUE.js MODULE.wasm
//! OUT.html` inlines the `wasm-bindgen` glue and the WASM into the page
//! template ([`stave_web::page::assemble`]). `tools/build-viewer.sh` runs
//! it after building the WASM.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, bail};

fn main() -> anyhow::Result<()> {
    let arguments: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [glue, wasm, out] = arguments.as_slice() else {
        bail!("usage: stave-viewer-page GLUE.js MODULE.wasm OUT.html");
    };
    let glue_text =
        fs::read_to_string(glue).with_context(|| format!("reading {}", glue.display()))?;
    let wasm_bytes = fs::read(wasm).with_context(|| format!("reading {}", wasm.display()))?;
    let page = stave_web::page::assemble(&glue_text, &wasm_bytes);
    if let Some(folder) = out.parent() {
        fs::create_dir_all(folder).with_context(|| format!("making {}", folder.display()))?;
    }
    fs::write(out, page).with_context(|| format!("writing {}", out.display()))?;
    Ok(())
}

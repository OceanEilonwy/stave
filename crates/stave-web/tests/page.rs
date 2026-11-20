//! The viewer's page is one self-contained file: nothing in it loads from
//! another origin, so it works in an MCP App's sandbox and offline.

use std::fs;
use std::path::Path;

use stave_web::page::{NEEDS_WEBGPU, WASM_BLOCKED, assemble};

/// What would make the page reach out to another origin.
fn outside_references(page: &str) -> Vec<&'static str> {
    [
        "src=\"http",
        "src='http",
        "href=\"http",
        "href='http",
        "<script src",
        "<link",
        "@import",
    ]
    .into_iter()
    .filter(|needle| page.contains(needle))
    .collect()
}

#[test]
fn viewer_html_is_self_contained() {
    // The page as the build script makes it, around stand-in glue.
    let page = assemble("let wasm_bindgen = function () {};", b"\0asm\x01\0\0\0");
    assert_eq!(outside_references(&page), Vec::<&str>::new());
    assert!(page.contains("AGFzbQEAAAA="), "the module is inlined");
    assert!(page.contains("window.staveViewer"));
    assert!(page.contains(NEEDS_WEBGPU));
    assert!(page.contains(WASM_BLOCKED));

    // The page tools/build-viewer.sh built, when it has been run.
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/viewer/stave-viewer.html");
    match fs::read_to_string(&built) {
        Ok(page) => {
            assert_eq!(outside_references(&page), Vec::<&str>::new());
            assert!(page.contains("AGFzbQ"), "the built module is inlined");
            assert!(!page.contains("@STAVE_WASM_BASE64@"));
            assert!(!page.contains("/*@STAVE_GLUE@*/"));
        }
        Err(_) => eprintln!(
            "{} isn't built (run tools/build-viewer.sh); checked the template only",
            built.display()
        ),
    }
}

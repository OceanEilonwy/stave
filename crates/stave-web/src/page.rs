//! The viewer's one self-contained page: the HTML around the canvas, the
//! `wasm-bindgen` glue and the WASM itself (as base64), all inlined, so
//! the page needs no other origin (an MCP App page runs in a sandbox that
//! allows none). `tools/build-viewer.sh` builds the WASM and the glue and
//! puts them together with [`assemble`].
//!
//! The page sets `window.staveViewer.show({files, suggestion, folder})`
//! for its host: `files` maps each file's path to its text, `suggestion`
//! (optional) maps the files an agent's suggestion changes to their new
//! text, and `folder` is the project folder (`/p`). Without WebGPU the
//! page says "This preview needs WebGPU." and does nothing else.

/// The page, with the places the glue and the WASM go marked.
pub const TEMPLATE: &str = include_str!("viewer.html");

/// Where the `wasm-bindgen` glue (`--target no-modules`) goes.
const GLUE_MARK: &str = "/*@STAVE_GLUE@*/";

/// Where the WASM, as base64, goes.
const WASM_MARK: &str = "@STAVE_WASM_BASE64@";

/// The glue's test for its own script element.
const CURRENT_SCRIPT: &str = "document.currentScript !== null";

/// [`CURRENT_SCRIPT`], also asking that the script came from a file.
const CURRENT_SCRIPT_WITH_SOURCE: &str =
    "document.currentScript !== null && document.currentScript.src";

/// The message a browser without WebGPU shows.
pub const NEEDS_WEBGPU: &str = "This preview needs WebGPU.";

/// The page with `glue` (the JS `wasm-bindgen --target no-modules`
/// writes) and `wasm` (the module) inlined.
pub fn assemble(glue: &str, wasm: &[u8]) -> String {
    // Nothing in the glue may end the script element it sits in.
    let glue = glue.replace("</script", "<\\/script");
    // The glue works out where its module would be from its own script's
    // `src`. Inlined, the script has none, and in a page loaded from
    // `srcdoc` (as an MCP App page can be) resolving "" against
    // `about:srcdoc` throws; the module is handed over directly anyway.
    let glue = glue.replace(CURRENT_SCRIPT, CURRENT_SCRIPT_WITH_SOURCE);
    TEMPLATE
        .replacen(GLUE_MARK, &glue, 1)
        .replacen(WASM_MARK, &base64(wasm), 1)
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let letter = |index: u32| {
        ALPHABET
            .get(usize::try_from(index & 63).unwrap_or(0))
            .map_or('A', |byte| char::from(*byte))
    };
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let [a, b, c] = match *chunk {
            [a, b, c] => [a, b, c],
            [a, b] => [a, b, 0],
            [a] => [a, 0, 0],
            _ => continue,
        };
        let group = u32::from(a) << 16 | u32::from(b) << 8 | u32::from(c);
        text.push(letter(group >> 18));
        text.push(letter(group >> 12));
        if chunk.len() > 1 {
            text.push(letter(group >> 6));
        } else {
            text.push('=');
        }
        if chunk.len() > 2 {
            text.push(letter(group));
        } else {
            text.push('=');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn the_template_has_one_place_for_each() {
        assert_eq!(TEMPLATE.matches(GLUE_MARK).count(), 1);
        assert_eq!(TEMPLATE.matches(WASM_MARK).count(), 1);
        assert!(TEMPLATE.contains(NEEDS_WEBGPU));
    }

    #[test]
    fn the_glue_cannot_end_its_script() {
        let page = assemble("let x = '</script>';", b"\0asm");
        assert!(!page.contains("'</script>'"));
        assert!(page.contains("AGFzbQ=="));
    }

    #[test]
    fn the_glue_does_not_look_for_its_own_file() {
        let glue = "if (typeof document !== 'undefined' && document.currentScript !== null) {";
        let page = assemble(glue, b"");
        assert!(page.contains("document.currentScript !== null && document.currentScript.src) {"));
    }
}

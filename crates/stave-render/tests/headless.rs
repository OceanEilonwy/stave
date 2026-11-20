//! The renderer drawing offscreen, as the MCP server's `render_view` does.

use std::path::Path;

use stave_render::{Backdrop, Frame, Renderer};
use stave_scene::Orbit;

#[test]
fn renders_box_example_headless() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/box");
    let opened = stave_app::open(&folder).unwrap();
    let scene = stave_app::view::scene(opened.design.as_ref().unwrap());
    let posed = scene.rest();
    let mut renderer = match pollster::block_on(Renderer::headless()) {
        Ok(renderer) => renderer,
        Err(error) => {
            eprintln!("no GPU adapter: {error}");
            return;
        }
    };
    renderer.upload(&scene);
    let bounds = scene.bounds(&posed).unwrap();
    let image = renderer
        .render(&Frame {
            scene: &scene,
            posed: &posed,
            orbit: Orbit::framing(bounds, 1.0),
            selection: &[],
            highlight: None,
            section: None,
            looks: None,
            overlay: None,
            tint_cut_faces: true,
            lit: None,
            tints: None,
            backdrop: Backdrop::Light,
            width: 64,
            height: 64,
        })
        .unwrap();
    assert_eq!((image.width, image.height), (64, 64));
    assert_eq!(image.rgba.len(), 64 * 64 * 4);
    // The corner shows the backdrop; somewhere the box is drawn.
    let corner = image.rgba.get(..4).unwrap();
    assert!(image.rgba.chunks(4).any(|pixel| pixel != corner));
    let png = Renderer::encode_png(&image).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
}

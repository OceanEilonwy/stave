//! `--screenshot`: one frame, offscreen, with the default framing.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use stave_lang::render_diagnostics;
use stave_scene::{Orbit, SceneGraph};

use crate::render::{Frame, Image, Renderer, write_png};

/// Renders a scene of a design to an image.
pub fn render_scene(
    renderer: &mut Renderer,
    scene: &SceneGraph,
    scene_name: Option<&str>,
    (width, height): (u32, u32),
) -> Result<Image> {
    let posed = match scene_name {
        None => scene.rest(),
        Some(name) => {
            let Some(entry) = scene.scene_named(name) else {
                let names: Vec<&str> = scene.scenes.iter().map(|s| s.name.as_str()).collect();
                bail!(
                    "there is no scene '{name}'; the scenes are {}",
                    names.join(", ")
                );
            };
            scene.pose(entry)
        }
    };
    let aspect = width as f32 / height as f32;
    let bounds = scene.bounds(&posed).context("the scene shows nothing")?;
    renderer.upload(scene);
    renderer.render(&Frame {
        scene,
        posed: &posed,
        orbit: Orbit::framing(bounds, aspect),
        selection: None,
        width,
        height,
    })
}

/// Opens a project, renders it and writes a PNG.
pub fn run(folder: &Path, output: &Path, scene_name: Option<&str>, size: (u32, u32)) -> Result<()> {
    let opened =
        stave_app::open(folder).with_context(|| format!("can't open '{}'", folder.display()))?;
    eprint!(
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    let Some(design) = &opened.design else {
        bail!(
            "'{}' has {} errors, so there is nothing to draw",
            folder.display(),
            opened.error_count()
        );
    };
    let scene = stave_app::view::scene(design);
    let mut renderer = Renderer::new()?;
    let image = render_scene(&mut renderer, &scene, scene_name, size)?;
    write_png(&image, output)?;
    eprintln!(
        "wrote {} ({}x{}, {})",
        output.display(),
        image.width,
        image.height,
        renderer.adapter_name()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// Renders the cabinet offscreen; skips when this machine has no GPU
    /// adapter wgpu can use.
    #[test]
    fn the_cabinet_renders_offscreen() {
        let mut renderer = match Renderer::new() {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!("skipped: {error:#}");
                return;
            }
        };
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
        let opened = stave_app::open(&folder).unwrap();
        let scene = stave_app::view::scene(opened.design.as_ref().unwrap());
        for scene_name in [None, Some("all_open")] {
            let image = render_scene(&mut renderer, &scene, scene_name, (320, 200)).unwrap();
            assert_eq!(image.rgba.len(), 320 * 200 * 4);
            let pixel = |x: usize, y: usize| {
                let at = (y * 320 + x) * 4;
                [image.rgba[at], image.rgba[at + 1], image.rgba[at + 2]]
            };
            // The corner is the light grey stage; the middle is wood: warm,
            // more red than blue.
            let corner = pixel(2, 2);
            assert!(corner.iter().all(|c| *c > 200), "{corner:?}");
            let middle = pixel(160, 100);
            assert!(middle[0] > middle[2] + 30, "{middle:?}");
        }
        assert!(render_scene(&mut renderer, &scene, Some("no_such_scene"), (64, 64)).is_err());
    }
}

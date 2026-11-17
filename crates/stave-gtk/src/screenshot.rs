//! `--screenshot`: one frame, offscreen, framed on the whole design from
//! the default direction, optionally turned with `--turn`, pulled apart
//! with `--explode`, or posed at a problem with `--diagnostic`.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use stave_app::view;
use stave_lang::render_diagnostics;
use stave_scene::{Highlight, Orbit, Posed, SceneGraph};

use crate::render::{Frame, Image, Renderer, write_png};

/// How to pose a screenshot.
#[derive(Debug, Clone, Default)]
pub struct Shot {
    /// The scene (default: rest).
    pub scene: Option<String>,
    /// How far apart to pull the parts, 0 to 1 (default: the scene's own).
    pub explode: Option<f32>,
    /// Degrees to turn the camera about the up axis from the default view
    /// (positive toward the right; 180 looks at the back).
    pub turn: f32,
}

/// Where everything is for a shot: the named scene (or rest), exploded as
/// asked.
pub fn pose_shot(scene: &SceneGraph, shot: &Shot) -> Result<Posed> {
    let entry = match &shot.scene {
        None => scene.scenes.first().context("the design has no scenes")?,
        Some(name) => {
            let Some(entry) = scene.scene_named(name) else {
                let names: Vec<&str> = scene.scenes.iter().map(|s| s.name.as_str()).collect();
                bail!(
                    "there is no scene '{name}'; the scenes are {}",
                    names.join(", ")
                );
            };
            entry
        }
    };
    let amount = shot
        .explode
        .unwrap_or(if entry.explode { 1.0 } else { 0.0 });
    Ok(scene.pose_with(entry, &[], amount))
}

/// Renders a posed scene to an image, framed on what is shown and seen
/// from the default direction turned `turn_degrees` about the up axis.
pub fn render_posed(
    renderer: &mut Renderer,
    scene: &SceneGraph,
    posed: &Posed,
    highlight: Option<&Highlight>,
    (width, height): (u32, u32),
    turn_degrees: f32,
) -> Result<Image> {
    let aspect = crate::render::aspect(width, height);
    let bounds = scene.bounds(posed).context("the scene shows nothing")?;
    renderer.upload(scene);
    let default = Orbit::framing(bounds, aspect);
    let orbit = Orbit {
        yaw: (default.yaw + turn_degrees.to_radians()).rem_euclid(std::f32::consts::TAU),
        ..default
    }
    .framed(bounds, aspect);
    renderer.render(&Frame {
        scene,
        posed,
        orbit,
        selection: None,
        highlight,
        width,
        height,
    })
}

/// Renders a scene of a design to an image.
pub fn render_scene(
    renderer: &mut Renderer,
    scene: &SceneGraph,
    shot: &Shot,
    size: (u32, u32),
) -> Result<Image> {
    let posed = pose_shot(scene, shot)?;
    render_posed(renderer, scene, &posed, None, size, shot.turn)
}

/// Opens a project, renders it and writes a PNG.
///
/// With `diagnostic`, the design is posed where that problem (its index in
/// the problems list, from 0, in the order `stave check` prints them)
/// happens, with its parts tinted red and the shared wood drawn as red
/// boxes; the problem must be one the checks can show (a fit, overlap or
/// motion problem).
pub fn run(
    folder: &Path,
    output: &Path,
    shot: &Shot,
    diagnostic: Option<usize>,
    size: (u32, u32),
) -> Result<()> {
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
    let image = match diagnostic {
        None => render_scene(&mut renderer, &scene, shot, size)?,
        Some(index) => {
            let problems = view::problems(&opened, folder);
            let problem = problems.get(index).with_context(|| {
                format!(
                    "there is no problem {index}; '{}' has {}",
                    folder.display(),
                    problems.len()
                )
            })?;
            let contact = problem.contact.as_ref().with_context(|| {
                format!(
                    "problem {index} ({}) has nothing to show in the design",
                    problem.message
                )
            })?;
            let posed = view::pose_at_contact(&scene, contact, None)
                .context("the problem's scene is missing")?;
            let highlight = view::highlight(&scene, contact);
            render_posed(
                &mut renderer,
                &scene,
                &posed,
                Some(&highlight),
                size,
                shot.turn,
            )?
        }
    };
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
            let shot = Shot {
                scene: scene_name.map(str::to_owned),
                ..Shot::default()
            };
            let image = render_scene(&mut renderer, &scene, &shot, (320, 200)).unwrap();
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
        let missing = Shot {
            scene: Some("no_such_scene".to_owned()),
            ..Shot::default()
        };
        assert!(render_scene(&mut renderer, &scene, &missing, (64, 64)).is_err());
        // Turned to look at the back, the cabinet is still framed (wood in
        // the middle) but the picture differs: the back is plywood, no doors.
        let front = render_scene(&mut renderer, &scene, &Shot::default(), (320, 200)).unwrap();
        let turned = Shot {
            turn: 180.0,
            ..Shot::default()
        };
        let back = render_scene(&mut renderer, &scene, &turned, (320, 200)).unwrap();
        assert_ne!(front.rgba, back.rgba);
        let at = (100 * 320 + 160) * 4;
        let middle = [back.rgba[at], back.rgba[at + 1], back.rgba[at + 2]];
        assert!(middle[0] > middle[2] + 30, "{middle:?}");
    }
}

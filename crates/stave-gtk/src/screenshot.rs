//! `--screenshot`: one frame, offscreen, framed on the whole design from
//! the default direction, optionally turned with `--turn` and `--pitch`,
//! pulled apart with `--explode`, posed at a problem with `--diagnostic`,
//! with assemblies opened by `--open`, items hidden by `--hide`, a section
//! cut by `--section`, and framed close on one item by `--frame`.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use stave_app::view;
use stave_lang::placement::Axis;
use stave_lang::render_diagnostics;
use stave_model::solved::SolvedDesign;
use stave_scene::{Aabb, Highlight, NodeId, Orbit, Posed, SceneGraph, Section};

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
    /// Degrees above the horizontal to look from (default: the viewer's
    /// first view, about 17; 90 looks straight down).
    pub pitch: Option<f32>,
    /// Frame the shot on the items whose names start with this
    /// (`door_l.hinges[lower]`: its cup, its plate and arm, its links).
    pub frame: Option<String>,
    /// Assemblies opened by an angle in degrees (hinges) or a length in
    /// millimetres (slides): `("door_l", 45.0)`.
    pub open: Vec<(String, f32)>,
    /// Items hidden, by the start of their names.
    pub hide: Vec<String>,
    /// A section: cut away what is above (`up`), right of (`across`) or in
    /// front of (`deep`) this many millimetres. Default: the scene's own.
    pub section: Option<(Axis, f32)>,
}

/// Does a node's name start with `prefix` as a whole name: `door_l.hinges`
/// matches `door_l.hinges[lower] plate and arm` but not `door_l.hingesx`.
fn named(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '.', '[']))
}

/// The nodes a name picks out.
fn nodes_named(scene: &SceneGraph, prefix: &str) -> Vec<NodeId> {
    scene
        .node_ids()
        .filter(|id| {
            scene
                .node(*id)
                .is_some_and(|node| named(&node.name, prefix))
        })
        .collect()
}

/// The section of a shot: its own, else the scene's.
fn section_of(shot: &Shot, scene: &SceneGraph) -> Option<Section> {
    match shot.section {
        Some((Axis::Up, at)) => Some(Section {
            normal: glam::Vec3::Y,
            offset: at,
        }),
        Some((Axis::Across, at)) => Some(Section {
            normal: glam::Vec3::X,
            offset: at,
        }),
        Some((Axis::Deep, at)) => Some(Section {
            normal: glam::Vec3::Z,
            offset: -at,
        }),
        None => match &shot.scene {
            None => None,
            Some(name) => scene.scene_named(name).and_then(|entry| entry.section),
        },
    }
}

/// Where everything is for a shot: the named scene (or rest), exploded,
/// opened and hidden as asked.
pub fn pose_shot(design: &SolvedDesign, scene: &SceneGraph, shot: &Shot) -> Result<Posed> {
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
    let mut overrides = Vec::new();
    for (name, value) in &shot.open {
        let assembly = design
            .assemblies
            .iter()
            .find(|a| a.name == *name)
            .with_context(|| format!("there is no assembly '{name}'"))?;
        let range = scene
            .motion_range(assembly.id)
            .with_context(|| format!("'{name}' doesn't move"))?;
        let amount = if range.turns {
            value.to_radians()
        } else {
            *value
        };
        overrides.push((assembly.id, amount.clamp(0.0, range.full)));
    }
    let mut posed = scene.pose_with(entry, &overrides, amount);
    for prefix in &shot.hide {
        let hidden = nodes_named(scene, prefix);
        if hidden.is_empty() {
            bail!("nothing is called '{prefix}'");
        }
        for id in hidden {
            if let Some(slot) = posed.visible.get_mut(id.index()) {
                *slot = false;
            }
        }
    }
    Ok(posed)
}

/// How a shot is seen: the box it frames, and the camera's turn and pitch.
pub struct View<'a> {
    /// What to frame; `None` frames everything shown.
    pub frame: Option<&'a str>,
    /// Degrees about the up axis from the default view.
    pub turn: f32,
    /// Degrees above the horizontal; `None` for the default.
    pub pitch: Option<f32>,
    /// A section.
    pub section: Option<Section>,
}

/// Renders a posed scene to an image, framed on what is shown (or on the
/// items `view.frame` names) and seen from the default direction turned
/// `view.turn` degrees about the up axis and from `view.pitch` degrees up.
pub fn render_posed(
    renderer: &mut Renderer,
    scene: &SceneGraph,
    posed: &Posed,
    highlight: Option<&Highlight>,
    (width, height): (u32, u32),
    view: &View<'_>,
) -> Result<Image> {
    let aspect = crate::render::aspect(width, height);
    let bounds: Aabb = match view.frame {
        None => scene.bounds(posed).context("the scene shows nothing")?,
        Some(prefix) => {
            let nodes = nodes_named(scene, prefix);
            if nodes.is_empty() {
                bail!("nothing is called '{prefix}'");
            }
            scene
                .bounds_of(posed, &nodes)
                .with_context(|| format!("'{prefix}' has nothing to frame"))?
        }
    };
    renderer.upload(scene);
    let default = Orbit::framing(bounds, aspect);
    let orbit = Orbit {
        yaw: (default.yaw + view.turn.to_radians()).rem_euclid(std::f32::consts::TAU),
        pitch: view.pitch.map_or(default.pitch, f32::to_radians),
        ..default
    }
    .framed(bounds, aspect);
    renderer.render(&Frame {
        scene,
        posed,
        orbit,
        selection: None,
        highlight,
        section: view.section,
        width,
        height,
    })
}

/// Renders a scene of a design to an image.
pub fn render_scene(
    renderer: &mut Renderer,
    design: &SolvedDesign,
    scene: &SceneGraph,
    shot: &Shot,
    size: (u32, u32),
) -> Result<Image> {
    let posed = pose_shot(design, scene, shot)?;
    let view = View {
        frame: shot.frame.as_deref(),
        turn: shot.turn,
        pitch: shot.pitch,
        section: section_of(shot, scene),
    };
    render_posed(renderer, scene, &posed, None, size, &view)
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
        None => render_scene(&mut renderer, design, &scene, shot, size)?,
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
            let view = View {
                frame: shot.frame.as_deref(),
                turn: shot.turn,
                pitch: shot.pitch,
                section: section_of(shot, &scene),
            };
            render_posed(&mut renderer, &scene, &posed, Some(&highlight), size, &view)?
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
        let design = opened.design.as_ref().unwrap();
        let scene = stave_app::view::scene(design);
        for scene_name in [None, Some("all_open")] {
            let shot = Shot {
                scene: scene_name.map(str::to_owned),
                ..Shot::default()
            };
            let image = render_scene(&mut renderer, design, &scene, &shot, (320, 200)).unwrap();
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
        assert!(render_scene(&mut renderer, design, &scene, &missing, (64, 64)).is_err());
        // Turned to look at the back, the cabinet is still framed (wood in
        // the middle) but the picture differs: the back is plywood, no doors.
        let front =
            render_scene(&mut renderer, design, &scene, &Shot::default(), (320, 200)).unwrap();
        let turned = Shot {
            turn: 180.0,
            ..Shot::default()
        };
        let back = render_scene(&mut renderer, design, &scene, &turned, (320, 200)).unwrap();
        assert_ne!(front.rgba, back.rgba);
        let at = (100 * 320 + 160) * 4;
        let middle = [back.rgba[at], back.rgba[at + 1], back.rgba[at + 2]];
        assert!(middle[0] > middle[2] + 30, "{middle:?}");
        // A hinge close up, from above, cut just over it, at two angles:
        // the door moves, so the pictures differ; a name that isn't there
        // is an error.
        let hinge = |open: f32| Shot {
            frame: Some("door_l.hinges[lower]".to_owned()),
            open: vec![("door_l".to_owned(), open)],
            pitch: Some(89.0),
            section: Some((Axis::Up, 657.0)),
            ..Shot::default()
        };
        let closed = render_scene(&mut renderer, design, &scene, &hinge(0.0), (160, 120)).unwrap();
        let open = render_scene(&mut renderer, design, &scene, &hinge(90.0), (160, 120)).unwrap();
        assert_ne!(closed.rgba, open.rgba);
        let nowhere = Shot {
            frame: Some("no_such_hinge".to_owned()),
            ..Shot::default()
        };
        assert!(render_scene(&mut renderer, design, &scene, &nowhere, (64, 64)).is_err());
        assert!(named(
            "door_l.hinges[lower] plate and arm",
            "door_l.hinges[lower]"
        ));
        assert!(named("door_l.hinges[lower]", "door_l.hinges"));
        assert!(!named("door_l.hingesx", "door_l.hinges"));
    }
}

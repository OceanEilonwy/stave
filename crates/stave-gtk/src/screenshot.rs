//! `--screenshot`: one frame, offscreen, framed on the whole design from
//! the default direction, optionally turned with `--turn` and `--pitch`,
//! pulled apart with `--explode`, posed at a problem with `--diagnostic`,
//! with assemblies opened by `--open`, items hidden by `--hide`, a section
//! cut by `--section`, and framed close on one item by `--frame`.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use stave_app::{focus, view};
use stave_lang::placement::Axis;
use stave_lang::render_diagnostics;
use stave_model::solved::SolvedDesign;
use stave_scene::{
    Aabb, Highlight, Look, NodeId, Orbit, Overlay, Posed, Projection, SceneGraph, Section,
};

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
    /// (`door_l.hinges[lower]`: every member of the hinge, named
    /// `door_l.hinges[lower] cup` and so on).
    pub frame: Option<String>,
    /// Assemblies opened by an angle in degrees (hinges) or a length in
    /// millimetres (slides): `("door_l", 45.0)`.
    pub open: Vec<(String, f32)>,
    /// Items hidden, by the start of their names.
    pub hide: Vec<String>,
    /// A section: cut away what is above (`up`), right of (`across`) or in
    /// front of (`deep`) this many millimetres. Default: the scene's own.
    pub section: Option<(Axis, f32)>,
    /// Draw flat (orthographic) instead of in perspective.
    pub flat: bool,
    /// Don't tint the faces cuts made.
    pub no_tint: bool,
    /// Focus mode on a joint or a piece of hardware, by name.
    pub focus: Option<FocusShot>,
}

/// Focus mode for a screenshot.
#[derive(Debug, Clone, Default)]
pub struct FocusShot {
    /// The joint or hardware.
    pub name: String,
    /// Which pair of a joint (default the first).
    pub pair: Option<usize>,
    /// How far along: 0 apart (closed) to 1 together (fully open).
    /// Default: half way. An `--open` of the focused assembly overrides it.
    pub step: Option<f32>,
    /// Look straight down, flat, cut through the hardware or the joint.
    pub plan: bool,
    /// Stay in 3D even where focus would open looking from above.
    pub in_3d: bool,
    /// The display toggles.
    pub options: stave_app::focus::Options,
}

/// Does a node's name start with `prefix` as a whole name: `door_l.hinges`
/// matches `door_l.hinges[lower] inner_link` but not `door_l.hingesx`.
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
    /// Draw flat.
    pub flat: bool,
    /// Look straight down on this box, flat (a plan), turned by this yaw
    /// (radians); overrides `frame`.
    pub plan: Option<(Aabb, f32)>,
    /// Look from this yaw and pitch (radians) instead of `turn` and
    /// `pitch`.
    pub look: Option<(f32, f32)>,
    /// How each node looks (focus mode).
    pub looks: Option<&'a [Look]>,
    /// The overlay (focus mode).
    pub overlay: Option<&'a Overlay>,
    /// Tint the faces cuts made.
    pub tint_cut_faces: bool,
}

impl<'a> View<'a> {
    /// A plain view of a shot.
    fn of(shot: &'a Shot, scene: &SceneGraph) -> View<'a> {
        View {
            frame: shot.frame.as_deref(),
            turn: shot.turn,
            pitch: shot.pitch,
            section: section_of(shot, scene),
            flat: shot.flat,
            plan: None,
            look: None,
            looks: None,
            overlay: None,
            tint_cut_faces: !shot.no_tint,
        }
    }
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
    render_bounds(
        renderer,
        scene,
        posed,
        highlight,
        (width, height),
        view,
        bounds,
    )
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
    render_posed(renderer, scene, &posed, None, size, &View::of(shot, scene))
}

/// Renders focus mode on a joint or a piece of hardware, and returns the
/// side panel's text with the image.
pub fn render_focus(
    renderer: &mut Renderer,
    opened: &stave_app::Opened,
    design: &SolvedDesign,
    scene: &SceneGraph,
    shot: &Shot,
    focus_shot: &FocusShot,
    size: (u32, u32),
) -> Result<(Image, String)> {
    let target = focus::find(design, &focus_shot.name, focus_shot.pair)?;
    let focus = focus::open(opened, design, scene, target)
        .with_context(|| format!("can't focus on '{}'", focus_shot.name))?;
    let entry = match &shot.scene {
        None => scene.scenes.first().context("the design has no scenes")?,
        Some(name) => scene
            .scene_named(name)
            .with_context(|| format!("there is no scene '{name}'"))?,
    };
    // An --open of the focused assembly sets how far it is open.
    let mut progress = focus_shot.step.unwrap_or(0.5);
    if let Some((assembly, range)) = focus.motion() {
        let name = design
            .assemblies
            .get(assembly.index())
            .map(|a| a.name.as_str());
        if let Some((_, value)) = shot.open.iter().find(|(n, _)| Some(n.as_str()) == name) {
            let amount = if range.turns {
                value.to_radians()
            } else {
                *value
            };
            progress = if range.full > 0.0 {
                amount / range.full
            } else {
                0.0
            };
        }
    }
    // A joint that turns upright opens looking from above, unless the
    // shot asks for 3D.
    let from_above = focus_shot.plan || (focus.prefers_plan() && !focus_shot.in_3d);
    // From above, the section cuts the receiving part open: it is drawn
    // solid.
    let options = stave_app::focus::Options {
        xray: focus_shot.options.xray && !from_above,
        ..focus_shot.options
    };
    let frame = focus.frame(scene, entry, progress, &options);
    let mut posed = frame.posed;
    for prefix in &shot.hide {
        for id in nodes_named(scene, prefix) {
            if let Some(slot) = posed.visible.get_mut(id.index()) {
                *slot = false;
            }
        }
    }
    let plan = from_above.then(|| focus.plan()).flatten();
    if focus_shot.plan && plan.is_none() {
        bail!("this has no view from above: hinges, slides and joints that turn upright do");
    }
    let framed = if shot.frame.is_none() {
        focus
            .zone()
            .or_else(|| scene.bounds_of(&posed, focus.nodes()))
    } else {
        None
    };
    // A hinge is seen from inside the cabinet unless the shot turns.
    let look = focus
        .view()
        .filter(|_| shot.turn == 0.0 && shot.pitch.is_none());
    let view = View {
        look,
        section: plan.map(|p| p.section).or_else(|| section_of(shot, scene)),
        plan: plan.map(|p| (p.bounds, p.yaw)),
        looks: Some(&frame.looks),
        overlay: Some(&frame.overlay),
        tint_cut_faces: focus_shot.options.tint_cut_faces,
        ..View::of(shot, scene)
    };
    let image = match framed {
        Some(bounds) => render_bounds(renderer, scene, &posed, None, size, &view, bounds)?,
        None => render_posed(renderer, scene, &posed, None, size, &view)?,
    };
    let mut panel = vec![focus.context.clone(), String::new(), focus.heading.clone()];
    if let Some(description) = &focus.description {
        panel.push(description.clone());
    }
    for row in &focus.rows {
        panel.push(format!("  {}: {}", row.label, row.value));
    }
    for member in &focus.members {
        panel.push(format!(
            "  {}. {}: {}",
            member.number, member.name, member.caption
        ));
    }
    if let Some(insert) = &focus.insert {
        panel.push(format!("  {}   ({})", insert.text, insert.source));
    }
    if let Some(step) = &frame.step {
        panel.push(step.clone());
    }
    panel.push(frame.status.clone());
    if plan.is_some()
        && let Some(caption) = focus.plan_caption()
    {
        panel.push(caption.to_owned());
    }
    let poses: Vec<&str> = focus.key_poses.iter().map(|p| p.caption.as_str()).collect();
    panel.push(format!("Small views: {}", poses.join(", ")));
    Ok((image, panel.join("\n")))
}

/// Renders framed on a box, from the view's direction.
fn render_bounds(
    renderer: &mut Renderer,
    scene: &SceneGraph,
    posed: &Posed,
    highlight: Option<&Highlight>,
    (width, height): (u32, u32),
    view: &View<'_>,
    bounds: Aabb,
) -> Result<Image> {
    let aspect = crate::render::aspect(width, height);
    renderer.upload(scene);
    let orbit = match view.plan {
        Some((plan, yaw)) => Orbit::plan(plan, aspect, yaw + view.turn.to_radians()),
        None => {
            let default = Orbit::framing(bounds, aspect);
            let (yaw, pitch) = view.look.unwrap_or((
                (default.yaw + view.turn.to_radians()).rem_euclid(std::f32::consts::TAU),
                view.pitch.map_or(default.pitch, f32::to_radians),
            ));
            Orbit {
                yaw,
                pitch,
                projection: if view.flat {
                    Projection::Flat
                } else {
                    Projection::Perspective
                },
                ..default
            }
            .framed(bounds, aspect)
        }
    };
    renderer.render(&Frame {
        backdrop: crate::render::Backdrop::Light,
        scene,
        posed,
        orbit,
        selection: &[],
        highlight: highlight.map(Into::into),
        section: view.section,
        looks: view.looks,
        overlay: view.overlay,
        lit: None,
        tints: None,
        tint_cut_faces: view.tint_cut_faces,
        width,
        height,
    })
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
    let image = match (diagnostic, &shot.focus) {
        (None, Some(focus_shot)) => {
            let (image, panel) = render_focus(
                &mut renderer,
                &opened,
                design,
                &scene,
                shot,
                focus_shot,
                size,
            )?;
            println!("{panel}");
            image
        }
        (None, None) => render_scene(&mut renderer, design, &scene, shot, size)?,
        (Some(index), _) => {
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
                &View::of(shot, &scene),
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
            "door_l.hinges[lower] inner_link",
            "door_l.hinges[lower]"
        ));
        assert!(named("door_l.hinges[lower]", "door_l.hinges"));
        assert!(!named("door_l.hingesx", "door_l.hinges"));
    }

    /// Focus mode renders offscreen: a joint mid-way, X-ray on and off,
    /// and a hinge from above; skips without a GPU adapter.
    #[test]
    fn focus_renders_offscreen() {
        let mut renderer = match Renderer::new() {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!("skipped: {error:#}");
                return;
            }
        };
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stool");
        let opened = stave_app::open(&folder).unwrap();
        let design = opened.design.as_ref().unwrap();
        let scene = stave_app::view::scene(design);
        let shot = |step: f32, xray: bool| Shot {
            focus: Some(FocusShot {
                name: "rung_sockets".to_owned(),
                step: Some(step),
                options: stave_app::focus::Options {
                    xray,
                    ..stave_app::focus::Options::default()
                },
                ..FocusShot::default()
            }),
            ..Shot::default()
        };
        let render = |renderer: &mut Renderer, shot: &Shot| {
            let focus_shot = shot.focus.clone().unwrap();
            render_focus(
                renderer,
                &opened,
                design,
                &scene,
                shot,
                &focus_shot,
                (240, 160),
            )
            .unwrap()
        };
        let (mid, panel) = render(&mut renderer, &shot(0.6, true));
        assert!(
            panel.contains("Step 1 of 1 · push(rung[front], along: length)"),
            "{panel}"
        );
        assert!(panel.contains("rung[front] is 6 mm from home"), "{panel}");
        let (solid, _) = render(&mut renderer, &shot(0.6, false));
        let (home, _) = render(&mut renderer, &shot(1.0, true));
        assert_ne!(mid.rgba, solid.rgba);
        assert_ne!(mid.rgba, home.rgba);

        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
        let opened = stave_app::open(&folder).unwrap();
        let design = opened.design.as_ref().unwrap();
        let scene = stave_app::view::scene(design);
        let hinge = |open: f32, plan: bool| Shot {
            open: vec![("door_l".to_owned(), open)],
            focus: Some(FocusShot {
                name: "door_l.hinges[upper]".to_owned(),
                plan,
                ..FocusShot::default()
            }),
            ..Shot::default()
        };
        let mut images = Vec::new();
        for (open, plan) in [(0.0, true), (45.0, true), (110.0, true), (45.0, false)] {
            let shot = hinge(open, plan);
            let focus_shot = shot.focus.clone().unwrap();
            let (image, panel) = render_focus(
                &mut renderer,
                &opened,
                design,
                &scene,
                &shot,
                &focus_shot,
                (240, 160),
            )
            .unwrap();
            assert!(panel.contains("1. plate: The mounting plate"), "{panel}");
            assert!(panel.contains("Closest to the side: 2.9 mm"), "{panel}");
            images.push(image.rgba);
        }
        for (i, a) in images.iter().enumerate() {
            for b in images.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
        // The chidori opens from above by default; --in-3d keeps it in 3D.
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/joint_samples");
        let samples = stave_app::open(&folder).unwrap();
        let samples_design = samples.design.as_ref().unwrap();
        let samples_scene = stave_app::view::scene(samples_design);
        let chidori = |in_3d: bool| FocusShot {
            name: "node".to_owned(),
            step: Some(0.83),
            in_3d,
            ..FocusShot::default()
        };
        let mut views = Vec::new();
        for in_3d in [false, true] {
            let focus_shot = chidori(in_3d);
            let shot = Shot {
                focus: Some(focus_shot.clone()),
                ..Shot::default()
            };
            let (image, panel) = render_focus(
                &mut renderer,
                &samples,
                samples_design,
                &samples_scene,
                &shot,
                &focus_shot,
                (160, 120),
            )
            .unwrap();
            assert_eq!(
                panel.contains("Looking down on the joint, cut through its middle"),
                !in_3d,
                "{panel}"
            );
            views.push(image.rgba);
        }
        assert_ne!(views[0], views[1]);
        // A joint that doesn't turn has no view from above.
        let mut joint = hinge(0.0, true);
        if let Some(focus) = joint.focus.as_mut() {
            focus.name = "shelf_dados".to_owned();
        }
        let focus_shot = joint.focus.clone().unwrap();
        assert!(
            render_focus(
                &mut renderer,
                &opened,
                design,
                &scene,
                &joint,
                &focus_shot,
                (64, 64)
            )
            .is_err()
        );
    }
}

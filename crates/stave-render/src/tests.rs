//! The renderer's own tests: what it draws, and what it keeps from frame
//! to frame.

use std::path::Path;

use super::*;

/// A renderer, or `None` (and a note) on a machine without a GPU
/// adapter wgpu can use. An adapter that refuses a device fails the test.
fn renderer() -> Option<Renderer> {
    match pollster::block_on(Renderer::headless()) {
        Ok(renderer) => Some(renderer),
        Err(RenderError::NoAdapter(error)) => {
            eprintln!("skipped: no GPU adapter: {error}");
            None
        }
        Err(error) => panic!("{error}"),
    }
}

/// The stool's scene, at rest.
fn stool() -> (SceneGraph, Posed) {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stool");
    let opened = stave_app::open(&folder).unwrap();
    let scene = stave_app::view::scene(opened.design.as_ref().unwrap());
    let posed = scene.rest();
    (scene, posed)
}

fn frame<'a>(scene: &'a SceneGraph, posed: &'a Posed, yaw: f32) -> Frame<'a> {
    let bounds = scene.bounds(posed).unwrap();
    let orbit = Orbit::framing(bounds, aspect(96, 64));
    Frame {
        scene,
        posed,
        orbit: Orbit {
            yaw: orbit.yaw + yaw,
            ..orbit
        },
        selection: &[],
        highlight: None,
        section: None,
        looks: None,
        overlay: None,
        tint_cut_faces: true,
        lit: None,
        tints: None,
        backdrop: Backdrop::Light,
        width: 96,
        height: 64,
    }
}

#[test]
fn the_shadow_map_is_kept_while_only_the_camera_moves() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    renderer.upload(&scene);
    renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
    let first = renderer.shadow_key.as_ref().map(|key| key.casters.len());
    assert!(first.is_some_and(|casters| casters > 0));
    // Turned: the same map, kept, and the picture as with a map drawn
    // afresh.
    let turned = frame(&scene, &posed, 0.7);
    let generation = renderer.meshes_generation;
    let kept = renderer.render(&turned).unwrap();
    assert_eq!(renderer.meshes_generation, generation);
    renderer.shadow_key = None;
    let fresh = renderer.render(&turned).unwrap();
    assert_eq!(kept, fresh);
    // A part moved: the key differs, so the map is drawn again.
    let caster = renderer
        .shadow_key
        .as_ref()
        .and_then(|key| key.casters.first())
        .map(|(index, _)| usize::try_from(*index).unwrap())
        .unwrap();
    let mut moved = posed.clone();
    let caster_node = scene.node_ids().nth(caster).unwrap();
    moved.move_node(
        caster_node,
        Mat4::from_translation(Vec3::new(0.0, 50.0, 0.0)),
    );
    let before = renderer.shadow_key.take();
    renderer.render(&frame(&scene, &moved, 0.7)).unwrap();
    assert_ne!(renderer.shadow_key, before);
    // An upload of the same meshes keeps it; one with a mesh changed
    // draws it again, even with nothing moved.
    let generation = renderer.meshes_generation;
    renderer.upload(&scene);
    assert_eq!(renderer.meshes_generation, generation);
    let mut changed = scene.clone();
    raise(&mut changed.nodes[caster], 5.0);
    renderer.upload(&changed);
    assert_ne!(
        renderer.shadow_key.as_ref().map(|key| key.meshes),
        Some(renderer.meshes_generation)
    );
}

/// The stool's frame at another size.
fn sized<'a>(scene: &'a SceneGraph, posed: &'a Posed, width: u32, height: u32) -> Frame<'a> {
    Frame {
        width,
        height,
        ..frame(scene, posed, 0.0)
    }
}

#[test]
fn targets_have_room_to_grow_and_still_pictures_keep_their_own() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    renderer.upload(&scene);
    // A window dragged larger a few pixels at a time: one target.
    for (width, height) in [(1030, 600), (1031, 600), (1100, 700), (1280, 768)] {
        live(&mut renderer, &sized(&scene, &posed, width, height));
    }
    assert_eq!(renderer.targets_made, 1);
    // Small pictures between frames get their own, and leave the
    // view's alone.
    for _ in 0..3 {
        renderer.render(&sized(&scene, &posed, 160, 90)).unwrap();
        live(&mut renderer, &sized(&scene, &posed, 1280, 768));
    }
    assert_eq!(renderer.targets_made, 2);
    // Much smaller: a smaller target is made.
    live(&mut renderer, &sized(&scene, &posed, 64, 48));
    assert_eq!(renderer.targets_made, 3);
    assert_eq!(
        renderer.live_target.as_ref().map(|t| (t.width, t.height)),
        Some((256, 256))
    );
}

#[test]
fn a_frame_drawn_in_a_larger_target_is_the_same_frame() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    let selection = [scene.node_ids().next().unwrap()];
    let selected = |width, height| Frame {
        selection: &selection,
        ..sized(&scene, &posed, width, height)
    };
    renderer.upload(&scene);
    renderer.render(&selected(500, 300)).unwrap();
    let small = renderer.render(&selected(200, 120)).unwrap();
    assert_eq!(
        renderer.still_target.as_ref().map(|t| (t.width, t.height)),
        Some((512, 512))
    );
    let mut fresh = pollster::block_on(Renderer::headless()).unwrap();
    fresh.upload(&scene);
    fresh.render(&selected(200, 120)).unwrap();
    assert_eq!(
        fresh.still_target.as_ref().map(|t| (t.width, t.height)),
        Some((256, 256))
    );
    assert_eq!(small, fresh.render(&selected(200, 120)).unwrap());
}

/// Moves a node's mesh and outline up by `millimetres`, as an edit
/// to one part would change it.
fn raise(node: &mut stave_scene::Node, millimetres: f32) {
    let geometry = std::sync::Arc::make_mut(&mut node.local);
    for vertex in &mut geometry.mesh.vertices {
        vertex.position[1] += millimetres;
    }
    for segment in &mut geometry.edges.segments {
        for point in segment {
            point[1] += millimetres;
        }
    }
}

#[test]
fn a_box_on_screen_covers_its_pixels_and_a_margin() {
    let scene_box = Aabb {
        min: Vec3::new(-1.0, -1.0, -1.0),
        max: Vec3::new(1.0, 1.0, 1.0),
    };
    // Flat, looking down -z, the box filling the middle half.
    let flat = Mat4::orthographic_rh(-2.0, 2.0, -2.0, 2.0, -10.0, 10.0);
    assert_eq!(
        screen_rect(flat, scene_box, 200, 100),
        Some((46, 21, 108, 58))
    );
    // Off the edge: clamped to the frame.
    let moved = flat * Mat4::from_translation(Vec3::new(1.5, 0.0, 0.0));
    assert_eq!(
        screen_rect(moved, scene_box, 200, 100),
        Some((121, 21, 79, 58))
    );
    // Wholly off screen: nothing.
    let away = flat * Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
    assert_eq!(screen_rect(away, scene_box, 200, 100), None);
    // Partly behind the eye: the whole frame.
    let eye = Mat4::perspective_rh(1.0, 2.0, 0.1, 100.0);
    assert_eq!(
        screen_rect(eye, scene_box, 200, 100),
        Some((0, 0, 200, 100))
    );
}

#[test]
fn the_shortcuts_draw_the_frame_the_full_passes_do() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    renderer.upload(&scene);
    let selection: Vec<NodeId> = scene.node_ids().take(2).collect();
    let selected = |yaw| Frame {
        selection: &selection,
        ..frame(&scene, &posed, yaw)
    };
    // Nothing selected: no mask is made, and nothing multisampled kept.
    let plain = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
    assert!(renderer.still_target.as_ref().unwrap().mask.is_none());
    let pictures: Vec<Image> = [0.0, 0.7, 2.4]
        .into_iter()
        .map(|yaw| renderer.render(&selected(yaw)).unwrap())
        .collect();
    assert!(renderer.still_target.as_ref().unwrap().mask.is_some());
    assert_ne!(plain, pictures[0]);
    renderer.full_passes = true;
    assert_eq!(renderer.render(&frame(&scene, &posed, 0.0)).unwrap(), plain);
    for (yaw, picture) in [0.0, 0.7, 2.4].into_iter().zip(&pictures) {
        assert_eq!(&renderer.render(&selected(yaw)).unwrap(), picture);
    }
}

#[test]
fn the_nodes_are_written_only_when_they_change() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    renderer.upload(&scene);
    let selection = [scene.node_ids().next().unwrap()];
    renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
    let writes = renderer.node_writes;
    // The camera turned: the nodes are as they were.
    let turned = renderer.render(&frame(&scene, &posed, 0.7)).unwrap();
    assert_eq!(renderer.node_writes, writes);
    // Selected, then not: written each time, and the picture is the
    // one drawn before.
    renderer
        .render(&Frame {
            selection: &selection,
            ..frame(&scene, &posed, 0.7)
        })
        .unwrap();
    assert_eq!(
        renderer.render(&frame(&scene, &posed, 0.7)).unwrap(),
        turned
    );
    assert_eq!(renderer.node_writes, writes + 2);
}

#[test]
fn an_upload_writes_only_the_meshes_the_gpu_does_not_hold() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    renderer.upload(&scene);
    let held = renderer.meshes.held();
    assert!(held > 0);
    let first = renderer.meshes.ranges().to_vec();
    let picture = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
    // The same scene again (a renamed part, a comment): nothing new.
    let generation = renderer.meshes_generation;
    renderer.upload(&scene);
    assert_eq!(renderer.meshes_generation, generation);
    assert_eq!(renderer.meshes.held(), held);
    // One part changed: only its mesh is written; the rest keep their
    // place.
    let mut changed = scene.clone();
    raise(&mut changed.nodes[0], 5.0);
    renderer.upload(&changed);
    assert_ne!(renderer.meshes_generation, generation);
    assert_eq!(renderer.meshes.held(), held + 1);
    let second = renderer.meshes.ranges().to_vec();
    assert_ne!(first[0], second[0]);
    assert_eq!(first[1..], second[1..]);
    // Drawn from the shared buffers, it looks as a renderer with
    // nothing else uploaded draws it.
    let reused = renderer.render(&frame(&changed, &posed, 0.0)).unwrap();
    let mut fresh = pollster::block_on(Renderer::headless()).unwrap();
    fresh.upload(&changed);
    assert_eq!(reused, fresh.render(&frame(&changed, &posed, 0.0)).unwrap());
    assert_ne!(reused, picture);
    // Back to the first: every mesh is still held.
    renderer.upload(&scene);
    assert_eq!(renderer.meshes.ranges(), first.as_slice());
    assert_eq!(
        renderer.render(&frame(&scene, &posed, 0.0)).unwrap(),
        picture
    );
}

#[test]
fn the_shared_buffers_grow_and_start_afresh_when_meshes_no_longer_fit() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    renderer.upload(&scene);
    let picture = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
    // Far more meshes than the buffers were made for, none of them the
    // stool's: the buffers grow and are written from the start.
    let many: Vec<stave_scene::Node> = (1..=60)
        .flat_map(|step| {
            scene.nodes.iter().map(move |node| {
                let mut node = node.clone();
                raise(&mut node, f32::from(u8::try_from(step).unwrap()));
                node
            })
        })
        .collect();
    assert!(
        renderer
            .meshes
            .upload(&renderer.device, &renderer.queue, &many)
    );
    assert!(renderer.meshes.failed().is_none());
    // Nodes alike in shape (the stool's legs) share one mesh.
    let mut distinct: Vec<&stave_geom::Geometry> = Vec::new();
    for node in &scene.nodes {
        if !distinct.iter().any(|mesh| **mesh == *node.local) {
            distinct.push(&node.local);
        }
    }
    assert!(distinct.len() < scene.nodes.len());
    assert_eq!(renderer.meshes.held(), distinct.len() * 60);
    // The stool again, written into the grown buffers, looks the same.
    renderer.upload(&scene);
    assert_eq!(
        renderer.render(&frame(&scene, &posed, 0.0)).unwrap(),
        picture
    );
}

/// Draws a frame into the live target, copying it nowhere.
fn live(renderer: &mut Renderer, frame: &Frame<'_>) {
    renderer.draw(frame, Slot::Live, |_, _| {}).unwrap();
}

#[test]
fn a_frame_drawn_live_and_copied_out_is_the_still_picture() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    renderer.upload(&scene);
    let still = renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
    assert!(still.rgba.chunks(4).all(|pixel| pixel[3] == 255));
    let layout = Layout::new(96, 64);
    let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: layout.bytes(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let submitted = renderer
        .draw(
            &frame(&scene, &posed, 0.0),
            Slot::Live,
            |encoder, resolved| {
                assert_eq!(resolved.extent().width, 96);
                encoder.copy_texture_to_buffer(
                    resolved.copy_source(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(layout.padded_row),
                            rows_per_image: Some(layout.height),
                        },
                    },
                    resolved.extent(),
                );
            },
        )
        .unwrap();
    let copied = renderer.read_back(buffer, layout, submitted).unwrap();
    assert_eq!(copied, still);
}

#[test]
fn a_lost_device_fails_the_frame_and_says_so() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    renderer.upload(&scene);
    renderer.render(&frame(&scene, &posed, 0.0)).unwrap();
    renderer.device().destroy();
    // wgpu may say the device is lost at once or only once a frame on it
    // fails; either way the frames fail, and it says so.
    let mut failed = None;
    for _ in 0..3 {
        if let Err(error) = renderer.render(&frame(&scene, &posed, 0.0)) {
            failed = Some(error);
            if renderer.is_lost() {
                break;
            }
        }
    }
    assert!(failed.is_some());
    assert!(renderer.is_lost());
    assert!(renderer.lost().is_some());
    assert!(matches!(
        renderer.render(&frame(&scene, &posed, 0.0)),
        Err(RenderError::DeviceLost(_))
    ));
}

#[test]
fn an_offscreen_renderer_has_no_surface_and_ignores_resizing() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    renderer.upload(&scene);
    renderer.resize(300, 200);
    assert!(matches!(
        renderer.present(&frame(&scene, &posed, 0.0)),
        Err(RenderError::NoSurface)
    ));
}

#[test]
fn a_frame_blitted_onto_a_bgra_surface_is_the_still_picture() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (scene, posed) = stool();
    renderer.upload(&scene);
    // 192 by 128 fills three quarters across and half down of its
    // 256-pixel target, so the blit's share is tested, and each output
    // pixel samples one texel's centre exactly.
    let (width, height) = (192, 128);
    let still = renderer
        .render(&sized(&scene, &posed, width, height))
        .unwrap();
    // A texture as a canvas offers it: `bgra8unorm`, drawn through its
    // sRGB view.
    let blit = present::Blit::new(
        renderer.device(),
        renderer.queue(),
        wgpu::TextureFormat::Bgra8UnormSrgb,
    );
    let output = renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("test surface"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[wgpu::TextureFormat::Bgra8UnormSrgb],
    });
    let layout = Layout::new(width, height);
    let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: layout.bytes(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let submitted = renderer
        .draw(
            &sized(&scene, &posed, width, height),
            Slot::Live,
            |encoder, resolved| {
                assert_eq!(resolved.texture.size().width, 256);
                assert_eq!(resolved.texture.size().height, 256);
                blit.draw(encoder, resolved, &output);
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &output,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(layout.padded_row),
                            rows_per_image: Some(layout.height),
                        },
                    },
                    resolved.extent(),
                );
            },
        )
        .unwrap();
    let mut blitted = renderer.read_back(buffer, layout, submitted).unwrap();
    for pixel in blitted.rgba.chunks_mut(4) {
        pixel.swap(0, 2);
    }
    assert_eq!((blitted.width, blitted.height), (still.width, still.height));
    // Decoded from sRGB to sample and encoded again to draw: the same
    // bytes, or one off at most.
    let worst = blitted
        .rgba
        .iter()
        .zip(&still.rgba)
        .map(|(a, b)| a.abs_diff(*b))
        .max();
    assert!(worst.is_some_and(|worst| worst <= 1), "off by {worst:?}");
}

#[test]
fn a_picture_encodes_as_an_srgb_png() {
    let image = Image {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
    };
    let bytes = Renderer::encode_png(&image).unwrap();
    assert!(bytes.starts_with(b"\x89PNG"));
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let reader = decoder.read_info().unwrap();
    assert_eq!(reader.info().size(), (2, 1));
    assert!(reader.info().srgb.is_some());
}

#[test]
fn an_aspect_is_width_over_height() {
    assert_eq!(aspect(200, 100), 2.0);
    assert_eq!(aspect(0, 0), 1.0);
}

/// The stage's sRGB colour is the linear colour a frame is cleared to,
/// encoded: the 3D view fills uncovered room with it during a resize.
#[test]
fn the_backdrop_srgb_matches_its_clear_colour() {
    let encode = |linear: f64| {
        let encoded = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        encoded * 255.0
    };
    for backdrop in [Backdrop::Light, Backdrop::Dark] {
        let linear = backdrop.colour();
        let srgb = backdrop.srgb();
        for (channel, value) in [linear.r, linear.g, linear.b].into_iter().zip(srgb) {
            assert!(
                (encode(channel) - f64::from(value)).abs() < 1.0,
                "{backdrop:?}: {channel} encodes to {}, not {value}",
                encode(channel)
            );
        }
    }
}

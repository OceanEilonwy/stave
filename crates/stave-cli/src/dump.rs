//! `stave dump`: the solved design as a table people can check by hand.
//!
//! Every length is in millimetres with trailing zeros trimmed. Positions are
//! world coordinates (`across` left to right, `up` floor to ceiling, `deep`
//! front to back) with the origin at the left-bottom-front corner of the
//! ground part's drawn box. A part's box is its **drawn** box (the sizes
//! the file writes); its cut size includes what joints add.

use std::fmt::Write as _;

use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_model::solved::{
    Contact, CutKind, CutPlace, Depth, End, HardwareBase, HardwareData, LocalFace, Motion,
    Orientation, Point, PoseValue, Shape, SolvedDesign, WorldBox,
};

/// A length in millimetres without the unit: `19`, `1104.75`.
pub fn mm(length: Length) -> String {
    let text = length.to_string();
    text.strip_suffix("mm").unwrap_or(&text).to_owned()
}

fn point(p: Point) -> String {
    format!("{}, {}, {}", mm(p.across), mm(p.up), mm(p.deep))
}

fn extent(bounds: &WorldBox) -> String {
    let axis = |a: Axis| format!("{}..{}", mm(bounds.min().get(a)), mm(bounds.max().get(a)));
    format!(
        "{:<20} {:<20} {:<20}",
        axis(Axis::Across),
        axis(Axis::Up),
        axis(Axis::Deep)
    )
}

fn orientation(o: &Orientation) -> String {
    format!(
        "{}/{}/{}",
        o.length().word(),
        o.width().word(),
        o.thickness().word()
    )
}

fn face(face: LocalFace) -> String {
    let end = match face.end {
        End::Start => "start",
        End::End => "end",
    };
    format!("{} {end}", face.axis.word())
}

fn depth(depth: Depth) -> String {
    match depth {
        Depth::Length(length) => mm(length),
        Depth::Through => "through".into(),
    }
}

fn cut_kind(kind: &CutKind) -> String {
    match kind {
        CutKind::Notch { depth: d } => format!("notch depth {}", depth(*d)),
        CutKind::Slot {
            width,
            depth: d,
            angle,
            from_centre,
        } => format!(
            "slot width {} depth {} angle {angle}{}",
            mm(*width),
            depth(*d),
            if *from_centre { " from centre" } else { "" }
        ),
        CutKind::Pocket {
            width,
            thickness,
            depth: d,
        } => format!(
            "pocket {} x {} depth {}",
            mm(*width),
            mm(*thickness),
            depth(*d)
        ),
        CutKind::Tenon {
            length,
            width,
            thickness,
        } => format!(
            "tenon {} x {} x {}",
            mm(*length),
            mm(*width),
            mm(*thickness)
        ),
        CutKind::Spigot { length, diameter } => {
            format!("spigot length {} diameter {}", mm(*length), mm(*diameter))
        }
        CutKind::Round {
            diameter,
            flats,
            flats_angle,
        } => match flats {
            Some(flats) => format!(
                "round diameter {} flats {} at {flats_angle}",
                mm(*diameter),
                mm(*flats)
            ),
            None => format!("round diameter {}", mm(*diameter)),
        },
        CutKind::Drill {
            diameter,
            depth: d,
            count,
        } => format!(
            "drill {count} x diameter {} depth {}",
            mm(*diameter),
            depth(*d)
        ),
        CutKind::DovetailSocket {
            neck,
            depth: d,
            slope,
        } => format!(
            "dovetail socket neck {} depth {} {slope}",
            mm(*neck),
            mm(*d)
        ),
        CutKind::DovetailTail {
            neck,
            depth: d,
            slope,
        } => format!("dovetail tail neck {} depth {} {slope}", mm(*neck), mm(*d)),
        CutKind::Outline {
            marks,
            depth: d,
            from_depth,
        } => format!(
            "outline of {} marks depth {} from {}",
            marks.len(),
            depth(*d),
            mm(*from_depth)
        ),
    }
}

/// The table.
pub fn table(design: &SolvedDesign) -> String {
    let mut out = String::new();
    let name = |id: stave_model::ids::PartId| {
        design
            .part(id)
            .map_or_else(|| id.to_string(), |p| p.name.clone())
    };
    let _ = writeln!(out, "{}", design.project.title);
    let _ = writeln!(
        out,
        "units: {}, resolution: {}mm",
        design.project.units.word(),
        mm(design.project.resolution)
    );
    let _ = writeln!(
        out,
        "all lengths in mm; boxes are drawn boxes as across, up, deep ranges from the ground part's left-bottom-front corner"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "PARTS ({})", design.parts.len());
    let _ = writeln!(
        out,
        "{:<28} {:<14} {:<26} {:<20} {:<20} {:<20} length/width/thickness",
        "part", "stock", "cut L x W x T", "across", "up", "deep"
    );
    for part in &design.parts {
        let size = format!(
            "{} x {} x {}",
            mm(part.cut.length),
            mm(part.cut.width),
            mm(part.cut.thickness)
        );
        let _ = writeln!(
            out,
            "{:<28} {:<14} {:<26} {} {}",
            part.name,
            part.stock.name,
            size,
            extent(&part.drawn_box()),
            orientation(&part.pose.orientation)
        );
        if part.cut != part.drawn {
            let _ = writeln!(
                out,
                "    drawn {} x {} x {}, starting {} x {} x {} into the cut size",
                mm(part.drawn.length),
                mm(part.drawn.width),
                mm(part.drawn.thickness),
                mm(part.drawn_offset.length),
                mm(part.drawn_offset.width),
                mm(part.drawn_offset.thickness)
            );
        }
        if let Shape::Turned {
            profile, stations, ..
        } = &part.shape
        {
            let list: Vec<String> = stations
                .iter()
                .map(|s| format!("{}:{}", mm(s.at), mm(s.diameter)))
                .collect();
            let _ = writeln!(out, "    turned to {profile}: {}", list.join(" "));
        }
        for cut in &part.cuts {
            let place = match &cut.place {
                CutPlace::End { toward } => format!("end toward {}", name(*toward)),
                CutPlace::Side { from, .. } => format!("side from {}", name(*from)),
                CutPlace::Crossing { other, footprint } => format!(
                    "where {} meets it, local {}..{} x {}..{} x {}..{}",
                    name(*other),
                    mm(footprint.min.length),
                    mm(footprint.max.length),
                    mm(footprint.min.width),
                    mm(footprint.max.width),
                    mm(footprint.min.thickness),
                    mm(footprint.max.thickness)
                ),
                CutPlace::Member {
                    hardware,
                    footprint,
                    ..
                } => format!(
                    "where {} meets it, local {}..{} x {}..{} x {}..{}",
                    design
                        .hardware
                        .get(hardware.index())
                        .map_or("", |h| h.name.as_str()),
                    mm(footprint.min.length),
                    mm(footprint.max.length),
                    mm(footprint.min.width),
                    mm(footprint.max.width),
                    mm(footprint.min.thickness),
                    mm(footprint.max.thickness)
                ),
            };
            let _ = writeln!(
                out,
                "    cut {} on {}: {place}",
                cut_kind(&cut.kind),
                face(cut.face)
            );
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "HARDWARE ({})", design.hardware.len());
    for hardware in &design.hardware {
        let detail = match &hardware.data {
            HardwareData::Slide {
                length,
                clearance,
                travel,
                ..
            } => format!(
                "length {}, clearance {}, travel {}",
                mm(*length),
                mm(*clearance),
                mm(*travel)
            ),
            HardwareData::ButtHinge { opening_angle, on } => {
                format!(
                    "opens {opening_angle}, leaves housed in it and {}",
                    name(*on)
                )
            }
            HardwareData::Hinge {
                opening_angle,
                cup_diameter,
                on,
                overlay,
                ..
            } => format!(
                "opens {opening_angle}, cup {}, on {}, overlays it by {}",
                mm(*cup_diameter),
                name(*on),
                mm(*overlay)
            ),
            HardwareData::Pull {
                centres,
                screw_diameter,
                ..
            } => match centres {
                Some(centres) => format!("screws {} apart, {}", mm(*centres), mm(*screw_diameter)),
                None => format!("one screw, {}", mm(*screw_diameter)),
            },
        };
        let _ = writeln!(
            out,
            "{:<28} {:<16} on {} ({} face)",
            hardware.name,
            hardware.kind,
            name(hardware.piece),
            hardware.mount_face.word()
        );
        let (moving, fixed) = match (hardware.base, &hardware.data) {
            (HardwareBase::Hinge, HardwareData::ButtHinge { .. }) => ("leaf", "fixed leaf"),
            (HardwareBase::Hinge, _) => ("cup", "plate"),
            (HardwareBase::Slide, _) => ("drawer member", "cabinet member"),
            (HardwareBase::Pull, _) => ("box", "fixed"),
        };
        let _ = writeln!(out, "    {moving:<15}{}", extent(&hardware.bounds));
        if let Some(member) = &hardware.fixed {
            let _ = writeln!(out, "    {fixed:<15}{}", extent(&member.bounds));
        }
        if let Some(mechanism) = &hardware.mechanism {
            let _ = writeln!(out, "    {:<15}{}", "arm", extent(&mechanism.arm.bounds));
            let _ = writeln!(
                out,
                "    {:<15}{}",
                "knuckle",
                extent(&mechanism.knuckle.bounds)
            );
        }
        let _ = writeln!(out, "    {detail}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "JOINTS ({})", design.joints.len());
    for joint in &design.joints {
        let _ = writeln!(
            out,
            "{:<28} {}{}",
            joint.name,
            joint.kind,
            if joint.glue { "" } else { ", no glue" }
        );
        for pair in &joint.pairs {
            let roles: Vec<String> = pair
                .roles
                .iter()
                .map(|r| format!("{}: {}", r.role, name(r.part)))
                .collect();
            let contact = match pair.contact {
                Contact::Butt { axis } => format!("butt across {}", axis.word()),
                Contact::Cross => "cross".into(),
                Contact::Connection => "connection".into(),
            };
            let _ = writeln!(out, "    {} ({contact})", roles.join(", "));
            for extension in &pair.extensions {
                let _ = writeln!(
                    out,
                    "        adds {} to the {} {} of {}",
                    mm(extension.by),
                    extension.axis.word(),
                    extension.end,
                    name(extension.part)
                );
            }
            for made in &pair.made_parts {
                let _ = writeln!(out, "        makes {}", name(*made));
            }
        }
        for part in &joint.unmatched {
            let _ = writeln!(out, "    {} meets none of its counterparts", name(*part));
        }
    }
    if !design.fasteners.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "FASTENERS");
        for fastener in &design.fasteners {
            let joint = design
                .joints
                .get(fastener.joint.index())
                .map_or("", |j| j.name.as_str());
            let _ = writeln!(
                out,
                "{:<28} {} x {} ({} x {})",
                joint,
                fastener.count,
                fastener.kind,
                mm(fastener.diameter),
                mm(fastener.length)
            );
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "REGIONS ({})", design.regions.len());
    for region in &design.regions {
        let _ = writeln!(out, "{:<28} {}", region.name, extent(&region.bounds));
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "ASSEMBLIES ({})", design.assemblies.len());
    for assembly in &design.assemblies {
        let motion = match &assembly.motion {
            Some(Motion::Turn {
                pivot,
                opening_angle,
            }) => format!(
                "turns up to {opening_angle} about the {} line through ({}), free side {}, swinging {}",
                pivot.axis.word(),
                point(pivot.through),
                pivot.free_side.word(),
                pivot.swing_toward.word()
            ),
            Some(Motion::Linkage { linkage }) => {
                let frame = linkage.frame();
                let [first, second] = linkage.links();
                let pin = |p: &stave_model::solved::PlanePoint| format!("{}, {}", mm(p.x), mm(p.y));
                format!(
                    "opens up to {} on a cup hinge's linkage across the {} axis from ({}), free side {}, swinging {}; links ({}) to ({}) and ({}) to ({})",
                    linkage.opening_angle(),
                    frame.axis().word(),
                    point(frame.origin()),
                    frame.free_side().word(),
                    frame.swing_toward().word(),
                    pin(&first.ground),
                    pin(&first.coupler),
                    pin(&second.ground),
                    pin(&second.coupler)
                )
            }
            Some(Motion::Travel { toward, travel }) => {
                format!("travels {} toward the {}", mm(*travel), toward.word())
            }
            None => "fixed".into(),
        };
        let mirror = assembly
            .mirror_of
            .and_then(|m| design.assemblies.get(m.index()))
            .map(|m| format!(", mirror of {}", m.name))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{:<28} {} ({} parts{mirror}): {motion}",
            assembly.name,
            assembly.component,
            assembly.parts.len()
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "REQUIRES ({})", design.requires.len());
    for require in &design.requires {
        let _ = writeln!(
            out,
            "{:<44} {}",
            require.name,
            if require.holds { "holds" } else { "FAILS" }
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "SCENES ({})", design.scenes.len());
    for scene in &design.scenes {
        let poses: Vec<String> = scene
            .poses
            .iter()
            .map(|p| {
                let assembly = design
                    .assemblies
                    .get(p.assembly.index())
                    .map_or("", |a| a.name.as_str());
                let value = match p.value {
                    PoseValue::Closed => "closed".to_owned(),
                    PoseValue::Open => "open".to_owned(),
                    PoseValue::Angle(angle) => angle.to_string(),
                    PoseValue::Travel(travel) => format!("{} out", mm(travel)),
                };
                format!("{assembly} {value}")
            })
            .collect();
        let _ = writeln!(
            out,
            "{:<28} {}{}",
            scene.name,
            scene.title.clone().unwrap_or_default(),
            if poses.is_empty() {
                String::new()
            } else {
                format!(": {}", poses.join(", "))
            }
        );
    }
    out
}

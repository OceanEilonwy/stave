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
use stave_model::Model;
use stave_model::defs::KindDef;
use stave_model::mechanism::{
    DriveValue, FramePoint, Mechanism, MechanismEntry, MemberIndex, MotionShape,
};
use stave_model::solved::{
    Body, BodyCarrier, Contact, CutKind, CutPlace, Depth, End, EntryName, EntryValue, Hardware,
    LocalFace, Orientation, Point, PoseValue, Shape, SolvedDesign, WorldBox,
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

/// A name as words: `fixed_leaf` is `fixed leaf`.
fn words(name: &str) -> String {
    name.replace('_', " ")
}

/// A piece of hardware's entries (its kind's params, then its fields, as
/// the kind writes them; by name when the kind isn't at hand), each as
/// `name value`.
fn entries_in_kind_order(hardware: &Hardware, kind: Option<&KindDef>) -> Vec<String> {
    let value = |value: &EntryValue| match value {
        EntryValue::Length(length) => mm(*length),
        EntryValue::Angle(angle) => angle.to_string(),
        EntryValue::Count(count) => count.to_string(),
        EntryValue::Bool(true) => "yes".to_owned(),
        EntryValue::Bool(false) => "no".to_owned(),
        EntryValue::Text(text) => format!("\"{text}\""),
    };
    let names: Vec<&str> = match kind {
        Some(kind) => kind
            .params
            .iter()
            .map(|param| param.decl.name.as_str())
            .chain(kind.fields.iter().map(|field| field.name.as_str()))
            .collect(),
        None => hardware.entries.keys().map(EntryName::as_str).collect(),
    };
    names
        .into_iter()
        .filter_map(|name| {
            let found = hardware
                .entries
                .iter()
                .find(|(entry, _)| entry.as_str() == name)?;
            Some(format!("{} {}", words(name), value(found.1)))
        })
        .collect()
}

/// A piece of hardware's solved settings, one for each of its kind's
/// `adjust` entries in the kind's order, each as `<member> set <offset>
/// <axis>`: `arm set -0.5mm across`. The member and axis words are the
/// kind's own.
fn settings(hardware: &Hardware) -> Vec<String> {
    hardware
        .settings
        .iter()
        .map(|setting| {
            let member = hardware
                .members
                .iter()
                .find(|body| body.id == setting.body)
                .map_or_else(|| "?".to_owned(), |body| words(&body.name));
            format!("{member} set {} {}", setting.offset, setting.axis.word())
        })
        .collect()
}

/// A point in a mechanism's plane.
fn frame_point(point: FramePoint) -> String {
    format!("{}, {}", mm(point.across), mm(point.depth))
}

/// An assembly's motion, from its mechanism: a turn about one pin and a
/// travel along one rail in world terms, anything else as its plane and
/// entries.
fn motion(design: &SolvedDesign, mechanism: &Mechanism) -> String {
    match mechanism.shape() {
        MotionShape::Travel { toward, travel } => {
            format!("travels {} toward the {}", mm(travel), toward.word())
        }
        MotionShape::Turn { line, opening } => {
            // Any point of the line will do: the one level with the low
            // end of the part the hardware is on.
            let low = mechanism
                .hardware()
                .first()
                .and_then(|id| design.hardware.get(id.index()))
                .and_then(|hardware| design.part(hardware.piece))
                .map(|part| part.drawn_box().min().get(line.axis));
            let through = match low {
                Some(low) => line.through.with(line.axis, low),
                None => line.through,
            };
            format!(
                "turns up to {opening} about the {} line through ({}), free side {}, swinging {}",
                line.axis.word(),
                point(through),
                line.free_side.word(),
                line.swing_toward.word()
            )
        }
        MotionShape::General => {
            let frame = mechanism.frame();
            let member = |index: MemberIndex| {
                mechanism
                    .members()
                    .get(index.index())
                    .map_or_else(|| "?".to_owned(), |m| m.name.clone())
            };
            let entries: Vec<String> = mechanism
                .entries()
                .iter()
                .map(|entry| match *entry {
                    MechanismEntry::Pivot { first, second, at } => format!(
                        "pivot {} and {} at ({})",
                        member(first),
                        member(second),
                        frame_point(at)
                    ),
                    MechanismEntry::Slot {
                        pin,
                        guide,
                        from,
                        to,
                    } => format!(
                        "slot {} in {} from ({}) to ({})",
                        member(pin),
                        member(guide),
                        frame_point(from),
                        frame_point(to)
                    ),
                    MechanismEntry::Rail {
                        runner,
                        guide,
                        direction,
                    } => format!(
                        "rail {} along {} toward ({}, {})",
                        member(runner),
                        member(guide),
                        mm(direction.across()),
                        mm(direction.depth())
                    ),
                    MechanismEntry::Couple {
                        follower,
                        leader,
                        ratio,
                    } => format!("couple {} to {}: {ratio}", member(follower), member(leader)),
                })
                .collect();
            let reach = match mechanism.drive().extent() {
                DriveValue::Turn(angle) => format!("turns up to {angle}"),
                DriveValue::Travel(length) => format!("travels up to {}", mm(length)),
            };
            format!(
                "{reach} on its mechanism across the {} axis from ({}), across {}, depth {}; {}",
                frame.axis().word(),
                point(frame.origin()),
                frame.across().word(),
                frame.depth().word(),
                entries.join(", ")
            )
        }
    }
}

/// The table. `model` gives each hardware kind's order of entries.
pub fn table(design: &SolvedDesign, model: Option<&Model>) -> String {
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
        let _ = writeln!(
            out,
            "{:<28} {:<16} on {} ({} face)",
            hardware.name,
            hardware.kind,
            name(hardware.piece),
            hardware.mount_face.word()
        );
        // The members on the piece first, then the rest in the kind's
        // order; a member the assembly's mechanism places (a link) is
        // given by its pivots under ASSEMBLIES instead.
        let on_piece = |body: &Body| body.carrier == BodyCarrier::Part(hardware.piece);
        let placed = |body: &&Body| !matches!(body.carrier, BodyCarrier::Link(_));
        for body in hardware
            .members
            .iter()
            .filter(|body| on_piece(body))
            .chain(hardware.members.iter().filter(|body| !on_piece(body)))
            .filter(placed)
        {
            let _ = writeln!(
                out,
                "    {:<15}{}",
                words(&body.name),
                extent(&body.bounds())
            );
        }
        let entries =
            entries_in_kind_order(hardware, model.and_then(|m| m.defs.kind(hardware.kind_id)));
        if !entries.is_empty() {
            let _ = writeln!(out, "    {}", entries.join(", "));
        }
        let settings = settings(hardware);
        if !settings.is_empty() {
            let _ = writeln!(out, "    {}", settings.join(", "));
        }
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
        let motion = match &assembly.mechanism {
            Some(mechanism) => motion(design, mechanism),
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

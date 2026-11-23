//! The rest of the headless editor's surface (`docs/editing.md`), driven
//! through the session on copies of projects in temporary folders: a
//! refusal's fixes and applying one, the card's texts in the contract,
//! the changes of the design (repeat, the shares grip, mirror, hold,
//! remove, rename), and the one writer that never overwrites a file
//! changed on disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use stave_app::change::ChangeDetail;
use stave_app::edit::{Candidate, Names, PartFace, Picked, SettingValue, Verdict};
use stave_app::editing::{
    CommitError, EditCommand, EditEvent, Progress, Rejection, commit_tried, entry_matches,
    parse_length, parse_name, parse_value,
};
use stave_app::hold::{HoldRequest, Measured};
use stave_app::item_offer::MirrorGesture;
use stave_app::naming::{Bound, RepeatCount};
use stave_app::scratch::{DiskWriter, EditBase};
use stave_app::session::{Command, Event, Session};
use stave_lang::placement::{Axis, Face};
use stave_modules::DiskFiles;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh temporary folder named for the test.
fn scratch_folder(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "stave-session-changes-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A copy of an example (its subfolders included) in a fresh temporary
/// folder. Returns the project folder.
fn copy_of(example: &str, name: &str) -> PathBuf {
    let project = scratch_folder(name).join("project");
    let mut folders = vec![(workspace().join("examples").join(example), project.clone())];
    while let Some((from, to)) = folders.pop() {
        std::fs::create_dir_all(&to).unwrap();
        for entry in std::fs::read_dir(&from).unwrap() {
            let path = entry.unwrap().path();
            let target = to.join(path.file_name().unwrap());
            if path.is_dir() {
                folders.push((path, target));
            } else {
                std::fs::copy(&path, &target).unwrap();
            }
        }
    }
    project
}

fn opened(folder: &Path) -> stave_app::Opened {
    stave_app::open(folder).unwrap()
}

fn errors(folder: &Path) -> String {
    let opened = opened(folder);
    let errors: Vec<_> = opened
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .cloned()
        .collect();
    stave_lang::render_diagnostics(&opened.sources, &errors)
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "a test picks out the variant it checks, and any other is a failure or not what it counts"
)]
fn send(session: &mut Session, command: EditCommand) -> Vec<EditEvent> {
    session
        .apply(Command::Edit(command))
        .into_iter()
        .filter_map(|event| match event {
            Event::Edit(event) => Some(event),
            _ => None,
        })
        .collect()
}

fn name(text: &str) -> stave_lang::ast::Ident {
    parse_name(text).unwrap()
}

fn entry(menu: &[Candidate], label: &str) -> usize {
    menu.iter()
        .position(|candidate| entry_matches(candidate, label))
        .unwrap_or_else(|| panic!("an entry {label}"))
}

// ---------------------------------------------------------------------------
// A refusal carries the checker's fixes
// ---------------------------------------------------------------------------

/// A cabinet against a wall that reaches as far forward as a door would,
/// with a `Door` component of the project's own that takes its mount, so
/// a door hinged on the left side, overlaying it almost to the wall,
/// swings into the wall as it opens.
const DOOR_PROJECT: &str = include_str!("projects/door_hits_wall/cabinet.stave");
fn door_project(name: &str) -> PathBuf {
    let folder = scratch_folder(name).join("door");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("cabinet.stave"), DOOR_PROJECT).unwrap();
    assert_eq!(errors(&folder), "");
    folder
}

#[test]
fn a_door_that_hits_the_wall_is_refused_with_its_fix_and_the_fix_makes_it_fit() {
    let folder = door_project("fix");
    let base = opened(&folder);
    let model = base.model.as_ref().unwrap();
    let bay = model.regions.iter().find(|r| r.name == "bay").unwrap().id;
    let side_l = model.parts.iter().find(|p| p.name == "side_l").unwrap().id;

    let mut session = Session::new(folder.clone());
    send(&mut session, EditCommand::Start(Picked::Region(bay)));
    let index = entry(session.edit().unwrap().menu(), "Door");
    send(&mut session, EditCommand::Choose(index));

    // The card's texts: the prompt is the param's own doc comment, the
    // generated name, the default as written, the range of a `within`.
    let chosen = session.edit().unwrap().chosen().unwrap();
    let door = chosen.candidate();
    assert_eq!(door.generated_name.as_ref().unwrap().as_str(), "door");
    let hinge_side = door
        .place
        .iter()
        .find(|p| p.name.as_str() == "hinge_side")
        .unwrap();
    assert_eq!(
        hinge_side.prompt,
        "Point at the side the hinges are screwed to."
    );
    assert_eq!(hinge_side.text, None);
    let opening = door
        .place
        .iter()
        .find(|p| p.name.as_str() == "opening")
        .unwrap();
    assert_eq!(opening.text.as_deref(), Some("bay"));
    let setting = |name: &str| {
        door.settings
            .iter()
            .find(|s| s.name.as_str() == name)
            .unwrap()
    };
    let inset = setting("hinge_inset");
    assert_eq!(inset.default_text.as_deref(), Some("80mm"));
    assert_eq!(inset.text.as_deref(), Some("80mm"));
    let range = inset.range.as_ref().unwrap();
    assert_eq!((range.low.as_str(), range.high.as_str()), ("60mm", "120mm"));
    let panel = setting("panel");
    assert_eq!(panel.default_text.as_deref(), Some("ply_18"));
    assert!(panel.options.iter().any(|option| option.text == "ply_18"));
    let mount = setting("mount");
    assert!(matches!(mount.value, SettingValue::Unset));
    let mounts: Vec<&str> = mount.options.iter().map(|o| o.text.as_str()).collect();
    assert_eq!(mounts, ["overlay()", "inset(reveal: 2mm)"]);

    send(
        &mut session,
        EditCommand::Bind(Picked::Face(PartFace {
            part: side_l,
            face: Face::Right,
        })),
    );
    let events = send(
        &mut session,
        EditCommand::Set {
            name: name("mount"),
            value: parse_value("overlay(left: 17.8mm, right: 17.8mm)").unwrap(),
        },
    );
    assert!(events.contains(&EditEvent::VerdictChanged), "{events:?}");
    let chosen = session.edit().unwrap().chosen().unwrap();
    assert_eq!(
        chosen.candidate().place[1].text.as_deref(),
        Some("side_l"),
        "a bound place param names what fills it"
    );
    assert_eq!(
        chosen
            .candidate()
            .settings
            .iter()
            .find(|s| s.name.as_str() == "mount")
            .unwrap()
            .text
            .as_deref(),
        Some("overlay(left: 17.8mm, right: 17.8mm)")
    );
    // Once tried, a default stands for a value: worked out for a
    // quantity, its own text for one that names a value; a chosen value
    // has none.
    let tried_setting = |name: &str| {
        chosen
            .candidate()
            .settings
            .iter()
            .find(|s| s.name.as_str() == name)
            .unwrap()
    };
    assert_eq!(
        tried_setting("hinge_inset").default_value.as_deref(),
        Some("80mm")
    );
    assert_eq!(
        tried_setting("panel").default_value.as_deref(),
        Some("ply_18")
    );
    assert_eq!(tried_setting("mount").default_value, None);
    let Progress::Tried(Verdict::Refused(refusal)) = chosen.progress() else {
        panic!("the door hits the wall: {:#?}", chosen.progress());
    };
    let problem = refusal.problems().first();
    assert_eq!(problem.diagnostic().code, stave_lang::Code::Motion);
    // The refusal carries the fix the diagnostic holds, as a patch.
    let fix = problem
        .fixes()
        .first()
        .expect("the motion check offers a fix");
    assert!(fix.label().starts_with("Use left: "), "{}", fix.label());
    assert_eq!(fix.patch().ops.len(), 1);
    // Its text is what `stave check` prints, the source excerpt included.
    let rendered = problem.rendered().unwrap();
    assert!(
        rendered.starts_with("error[motion]: door.leaf hits wall"),
        "{rendered}"
    );
    assert!(rendered.contains("  --> "), "{rendered}");
    assert!(rendered.contains("= fix: patch \"Use left: "), "{rendered}");

    // Applying the fix gives a verdict that fits, and Add writes it.
    let events = send(&mut session, EditCommand::ApplyFix { problem: 0, fix: 0 });
    assert!(events.contains(&EditEvent::FixApplied), "{events:?}");
    let chosen = session.edit().unwrap().chosen().unwrap();
    assert!(
        matches!(chosen.progress(), Progress::Tried(Verdict::Fits(_))),
        "{:#?}\n{}",
        chosen.progress(),
        chosen.patch().unwrap_or_default()
    );
    let events = send(&mut session, EditCommand::Add);
    assert!(events.contains(&EditEvent::Committed), "{events:?}");
    let text = std::fs::read_to_string(folder.join("cabinet.stave")).unwrap();
    assert!(text.contains("assembly door : Door {"), "{text}");
    assert!(text.contains("mount:      overlay(left: 16mm"), "{text}");
    assert_eq!(errors(&folder), "");
}

// ---------------------------------------------------------------------------
// The card's texts for a copied setting and the kinds in scope
// ---------------------------------------------------------------------------

#[test]
fn a_setting_copied_from_another_assembly_names_it_and_lists_what_it_can_be() {
    let folder = copy_of("clothing_cabinet_assemblies", "texts");
    let base = opened(&folder);
    let model = base.model.as_ref().unwrap();
    let below = model
        .regions
        .iter()
        .find(|r| r.name == "drawer_bay[top]")
        .unwrap()
        .id;
    let mut session = Session::new(folder.clone());
    send(&mut session, EditCommand::Start(Picked::Region(below)));
    let edit = session.edit().unwrap();
    let drawer = &edit.menu()[entry(edit.menu(), "casework.Drawer")];
    assert_eq!(drawer.generated_name.as_ref().unwrap().as_str(), "drawer_2");
    let setting = |name: &str| {
        drawer
            .settings
            .iter()
            .find(|s| s.name.as_str() == name)
            .unwrap()
    };
    let front = setting("front_stock");
    assert_eq!(front.text.as_deref(), Some("oak_19"));
    assert_eq!(front.copied_from.as_deref(), Some("drawer[top]"));
    let stocks: Vec<&str> = front.options.iter().map(|o| o.text.as_str()).collect();
    assert!(
        stocks.contains(&"oak_19") && stocks.contains(&"ply_6"),
        "{stocks:?}"
    );
    let slides: Vec<&str> = setting("slide")
        .options
        .iter()
        .map(|o| o.text.as_str())
        .collect();
    assert!(slides.contains(&"side_mount_slide"), "{slides:?}");
    let pulls: Vec<&str> = setting("pull_kind")
        .options
        .iter()
        .map(|o| o.text.as_str())
        .collect();
    assert!(
        pulls.contains(&"hw.brass_knob_26") && pulls.contains(&"knob"),
        "{pulls:?}"
    );
    assert_eq!(
        setting("pull_kind").default_text.as_deref(),
        Some("bar_pull(centres: 160mm)")
    );
    let mounts: Vec<&str> = setting("mount")
        .options
        .iter()
        .map(|o| o.text.as_str())
        .collect();
    assert_eq!(
        mounts.first().copied(),
        Some("overlay(left: 18mm, right: 18mm, top: -reveal, bottom: 8mm, meeting: -reveal / 2)")
    );
    assert!(mounts.contains(&"overlay()"), "{mounts:?}");
    // Names for the ids the contract holds.
    let names = Names::new(edit.base().model.as_ref().unwrap());
    assert_eq!(names.region(below), Some("drawer_bay[top]"));
    // Listed, a default worked out from other values has no value yet;
    // chosen and tried, it stands for the length the scratch copy gives.
    let box_height = setting("box_height");
    assert_eq!(
        box_height.default_text.as_deref(),
        Some("opening.up - bottom_gap - top_gap")
    );
    assert_eq!(box_height.default_value, None);
    let index = entry(edit.menu(), "casework.Drawer");
    send(&mut session, EditCommand::Choose(index));
    let chosen = session.edit().unwrap().chosen().unwrap();
    let box_height = chosen
        .candidate()
        .settings
        .iter()
        .find(|s| s.name.as_str() == "box_height")
        .unwrap();
    assert!(matches!(box_height.value, SettingValue::Default));
    assert_eq!(
        box_height.text.as_deref(),
        Some("opening.up - bottom_gap - top_gap")
    );
    // Even refused (it overlaps the drawer above), the copy was solved.
    assert_eq!(box_height.default_value.as_deref(), Some("220mm"));
}

// ---------------------------------------------------------------------------
// Changes of the design
// ---------------------------------------------------------------------------

fn fits(session: &Session) {
    let change = session.change().expect("a change in progress");
    assert!(
        matches!(change.progress(), Progress::Tried(Verdict::Fits(_))),
        "{}: {:#?}\n{}",
        change.summary(),
        change.progress(),
        change.patch().unwrap_or_default()
    );
}

fn add(session: &mut Session, folder: &Path) {
    let events = send(session, EditCommand::Add);
    assert!(events.contains(&EditEvent::Committed), "{events:?}");
    assert!(session.change().is_none());
    assert!(!session.last_commit().is_empty());
    assert_eq!(errors(folder), "");
}

#[test]
fn repeat_shares_hold_rename_and_remove_each_commit_a_design_that_checks_clean() {
    let folder = copy_of("chest_of_drawers", "changes");
    let chest = folder.join("chest.stave");
    let mut session = Session::new(folder.clone());

    // Four drawers become three.
    let events = send(
        &mut session,
        EditCommand::RepeatItem {
            item: name("drawer"),
            count: RepeatCount::new(3).unwrap(),
            axis: None,
        },
    );
    assert!(events.contains(&EditEvent::ChangeStarted), "{events:?}");
    fits(&session);
    add(&mut session, &folder);
    let text = std::fs::read_to_string(&chest).unwrap();
    assert!(
        text.contains("assembly drawer[bottom, middle, top] : casework.Drawer {"),
        "{text}"
    );

    // The boundary above the bottom drawer moves.
    send(
        &mut session,
        EditCommand::MoveShares {
            region: name("drawer_bay"),
            lower: 0,
            boundary: parse_length("300mm").unwrap(),
        },
    );
    fits(&session);
    add(&mut session, &folder);
    let text = std::fs::read_to_string(&chest).unwrap();
    assert!(
        text.contains("split(back.in_front, up, shares: ["),
        "{text}"
    );

    // A held distance between two faces.
    send(
        &mut session,
        EditCommand::Hold(HoldRequest {
            first: Measured::Face {
                item: "drawer[top].front".to_owned(),
                face: Face::Top,
            },
            second: Measured::Face {
                item: "case.top".to_owned(),
                face: Face::Bottom,
            },
            bound: Bound::Max,
            value: parse_length("40mm").unwrap(),
            reason: "the top drawer stays close under the top".to_owned(),
        }),
    );
    let Some(ChangeDetail::Hold { name: rule, .. }) = session.change().map(|c| c.detail().clone())
    else {
        panic!("a held rule");
    };
    fits(&session);
    add(&mut session, &folder);
    let text = std::fs::read_to_string(&chest).unwrap();
    assert!(
        text.contains(&format!("require {} : between(", rule.as_str())),
        "{text}"
    );

    // A rename reaches every use.
    send(
        &mut session,
        EditCommand::RenameItem {
            item: name("feet"),
            to: name("bun_feet"),
        },
    );
    fits(&session);
    add(&mut session, &folder);
    let text = std::fs::read_to_string(&chest).unwrap();
    assert!(
        text.contains("assembly bun_feet : casework.Feet {"),
        "{text}"
    );
    // A name already used is refused before anything is tried.
    let events = send(
        &mut session,
        EditCommand::RenameItem {
            item: name("bun_feet"),
            to: name("case"),
        },
    );
    assert!(
        matches!(
            events.last(),
            Some(EditEvent::Rejected(Rejection::CantStart(_)))
        ),
        "{events:?}"
    );

    // Removing the back first says what depends on it.
    send(&mut session, EditCommand::Remove(name("back")));
    let change = session.change().unwrap();
    let Progress::NeedsChoice(dependants) = change.progress() else {
        panic!("the back is used: {:#?}", change.progress());
    };
    let dependants: Vec<&str> = dependants.iter().map(|n| n.as_str()).collect();
    assert!(
        dependants.contains(&"drawer_bay") && dependants.contains(&"drawer"),
        "{dependants:?}"
    );
    // Add is refused while it waits; nothing is written.
    let before = std::fs::read_to_string(&chest).unwrap();
    let events = send(&mut session, EditCommand::Add);
    assert!(
        events.contains(&EditEvent::Rejected(Rejection::DoesNotFit)),
        "{events:?}"
    );
    let events = send(&mut session, EditCommand::IncludeDependants);
    assert!(
        events.contains(&EditEvent::DependantsIncluded),
        "{events:?}"
    );
    fits(&session);
    add(&mut session, &folder);
    let text = std::fs::read_to_string(&chest).unwrap();
    assert_ne!(text, before);
    assert!(!text.contains("assembly back"), "{text}");
    assert!(!text.contains("drawer_bay"), "{text}");
}

#[test]
fn a_door_on_a_mirrored_side_is_offered_its_mirror_image_and_takes_it() {
    let folder = copy_of("clothing_cabinet_assemblies", "mirror");
    let doors = folder.join("doors.stave");
    let text = std::fs::read_to_string(&doors)
        .unwrap()
        .replace("assembly door_l :", "assembly door :")
        .replace(
            "assembly door_r = mirror(door_l, about: mid(base, across))\n",
            "",
        );
    std::fs::write(&doors, text).unwrap();
    let cabinet = folder.join("cabinet.stave");
    let text = std::fs::read_to_string(&cabinet).unwrap().replace(
        "  pose door_l = open\n  pose door_r = open\n",
        "  pose door = open\n",
    );
    std::fs::write(&cabinet, text).unwrap();
    assert_eq!(errors(&folder), "");

    let mut session = Session::new(folder.clone());
    let offer = session
        .mirror_offer("door")
        .expect("the door has a mirror image");
    assert_eq!(
        (
            offer.renamed.as_str(),
            offer.image.as_str(),
            offer.about.as_str()
        ),
        ("door_left", "door_right", "mid(base, across)")
    );
    assert!(session.mirror_offer("divider").is_none());

    send(&mut session, EditCommand::Mirror(name("door")));
    fits(&session);
    let renamed: Vec<PathBuf> = session
        .change()
        .unwrap()
        .renamed_files()
        .map(Path::to_path_buf)
        .collect();
    assert_eq!(renamed.len(), 2, "{renamed:?}");
    add(&mut session, &folder);
    let doors = std::fs::read_to_string(&doors).unwrap();
    assert!(
        doors.contains("assembly door_left : casework.FramePanelDoor {"),
        "{doors}"
    );
    assert!(
        doors.contains("assembly door_right = mirror(door_left, about: mid(base, across))"),
        "{doors}"
    );
    let cabinet = std::fs::read_to_string(&cabinet).unwrap();
    assert!(cabinet.contains("pose door_left = open"), "{cabinet}");
}

#[test]
fn a_chosen_candidate_repeats_into_an_array_filling_its_space() {
    let folder = copy_of("clothing_cabinet_assemblies", "repeat-chosen");
    let text = std::fs::read_to_string(folder.join("drawers.stave")).unwrap();
    assert!(text.contains("assembly drawer["), "{text}");
    // Remove the drawers and their bays, then add two again by a repeat.
    let mut session = Session::new(folder.clone());
    send(&mut session, EditCommand::Remove(name("drawer_bay")));
    send(&mut session, EditCommand::IncludeDependants);
    fits(&session);
    add(&mut session, &folder);
    let base = opened(&folder);
    let below = base
        .model
        .as_ref()
        .unwrap()
        .regions
        .iter()
        .find(|r| r.name == "divider.below")
        .unwrap()
        .id;
    send(&mut session, EditCommand::Start(Picked::Region(below)));
    let index = entry(session.edit().unwrap().menu(), "casework.Drawer");
    send(&mut session, EditCommand::Choose(index));
    for (setting, value) in [
        (
            "mount",
            "overlay(left: 18mm, right: 18mm, top: 8mm, bottom: 8mm, meeting: -2.5mm)",
        ),
        ("front_stock", "oak_19"),
        ("box_stock", "maple_12"),
        ("bottom_stock", "ply_6"),
        ("slide", "side_mount_slide(length: 450mm)"),
    ] {
        send(
            &mut session,
            EditCommand::Set {
                name: name(setting),
                value: parse_value(value).unwrap(),
            },
        );
    }
    let events = send(
        &mut session,
        EditCommand::Repeat(Some(stave_app::candidates::Repeat {
            count: RepeatCount::new(2).unwrap(),
            axis: Axis::Up,
        })),
    );
    assert!(events.contains(&EditEvent::RepeatChanged), "{events:?}");
    let chosen = session.edit().unwrap().chosen().unwrap();
    let patch = chosen.patch().unwrap_or_default().to_owned();
    assert!(
        patch.contains("add region drawer_bay[bottom, top] = split(divider.below, up)"),
        "{patch}"
    );
    assert!(
        matches!(chosen.progress(), Progress::Tried(Verdict::Fits(_))),
        "{:#?}\n{patch}",
        chosen.progress()
    );
    let events = send(&mut session, EditCommand::Add);
    assert!(events.contains(&EditEvent::Committed), "{events:?}");
    assert_eq!(errors(&folder), "");
}

// ---------------------------------------------------------------------------
// One writer
// ---------------------------------------------------------------------------

#[test]
fn the_writer_refuses_a_file_changed_on_disk_since_it_was_read() {
    let folder = copy_of("clothing_cabinet_assemblies", "writer");
    let base = EditBase::new(&folder, &DiskFiles).unwrap();
    let text = "stave 1\n\npatch \"Wider\" {\n  set cabinet_width = 920mm\n}\n";
    let file = stave_lang::SourceMap::new()
        .add("wider.stave", text)
        .unwrap();
    let patch = stave_lang::parse_patch_file(file, text);
    let tried = base
        .try_patch("wider.stave", &patch, &AtomicBool::new(false))
        .unwrap();
    assert!(tried.fits(), "{:#?}", tried.verdict());
    let changed = tried.changed().first().unwrap().path.clone();
    let edited = format!(
        "{}\n// changed in another editor\n",
        std::fs::read_to_string(&changed).unwrap()
    );
    std::fs::write(&changed, &edited).unwrap();
    let result = commit_tried(base.opened(), &tried, &patch, &mut DiskWriter);
    assert!(
        matches!(result, Err(CommitError::Write { .. })),
        "{result:?}"
    );
    assert_eq!(std::fs::read_to_string(&changed).unwrap(), edited);

    // Read again, it writes.
    let base = EditBase::new(&folder, &DiskFiles).unwrap();
    let tried = base
        .try_patch("wider.stave", &patch, &AtomicBool::new(false))
        .unwrap();
    let written = commit_tried(base.opened(), &tried, &patch, &mut DiskWriter).unwrap();
    assert_eq!(written.len(), 1);
    let now = std::fs::read_to_string(&changed).unwrap();
    assert!(
        now.contains("920mm") && now.contains("changed in another editor"),
        "{now}"
    );
    let _ = Arc::new(DiskFiles);
}

// ---------------------------------------------------------------------------
// What "And the other side" offers, decided by the session
// ---------------------------------------------------------------------------

#[test]
fn an_item_with_a_mirror_image_is_not_offered_another() {
    // The doors and the side table's aprons, cleats and stretchers are
    // pairs already: none offers a second image (`door_2_right`), and
    // the image itself offers nothing.
    let cabinet = copy_of("clothing_cabinet_from_box", "offer_pairs_cabinet");
    let session = Session::new(cabinet);
    for item in ["frame_panel_door_left", "frame_panel_door_right"] {
        assert!(session.mirror_offer(item).is_none(), "{item}");
        let offer = session.item_offer(item).expect("the cabinet places");
        assert_eq!(offer.mirror, None, "{item}");
    }
    let table = copy_of("side_table_from_box", "offer_pairs_table");
    let session = Session::new(table);
    for item in [
        "apron_front",
        "apron_left",
        "cleat_front",
        "stretcher_front",
    ] {
        assert!(session.mirror_offer(item).is_none(), "{item}");
        let offer = session.item_offer(item).expect("the table places");
        assert_eq!(offer.mirror, None, "{item}");
    }
}

#[test]
fn only_a_handed_piece_filling_a_space_is_offered_a_mirrored_pair() {
    // The case, the divider, the shelf and the top fill a space but are
    // the same either way round: no mirrored pair. The drawers are two
    // pieces of a repeat.
    let cabinet = copy_of("clothing_cabinet_from_box", "offer_split_cabinet");
    let session = Session::new(cabinet.clone());
    for item in ["case", "divider", "shelf"] {
        let offer = session.item_offer(item).expect("the cabinet places");
        assert_eq!((offer.pieces, offer.repeatable), (1, true), "{item}");
        assert_eq!(offer.mirror, None, "{item}");
    }
    let drawer = session.item_offer("drawer").unwrap();
    assert_eq!(
        (drawer.pieces, drawer.repeatable, drawer.mirror),
        (2, true, None)
    );
    let table = copy_of("side_table_from_box", "offer_split_table");
    let top = Session::new(table).item_offer("top").unwrap();
    assert_eq!(top.mirror, None);

    // One door filling the space above the divider, hinged on the left
    // side: it is offered the pair, and the pair fits.
    let file = cabinet.join("cabinet.stave");
    let text = std::fs::read_to_string(&file)
        .unwrap()
        .replace(
            "region frame_panel_door_bay[left, right] = split(divider.above, across)\n",
            "",
        )
        .replace(
            "assembly frame_panel_door_left : casework.FramePanelDoor {\n  opening:     frame_panel_door_bay[left]",
            "assembly door : casework.FramePanelDoor {\n  opening:     divider.above",
        )
        .replace(
            "assembly frame_panel_door_right = mirror(frame_panel_door_left, about: mid(envelope, across))\n",
            "",
        );
    std::fs::write(&file, text).unwrap();
    assert_eq!(errors(&cabinet), "");
    let mut session = Session::new(cabinet.clone());
    let offer = session.item_offer("door").expect("the cabinet places");
    assert_eq!(offer.mirror, Some(MirrorGesture::Split(Axis::Across)));
    let command = offer.mirror.unwrap().command(name("door"));
    send(&mut session, command);
    fits(&session);
    add(&mut session, &cabinet);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("assembly door_right = mirror(door_left"),
        "{text}"
    );
}

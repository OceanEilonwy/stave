//! The inspector names other things of the design as links: the parts a
//! part is joined to, the hardware on it, what it turns on, and the
//! spaces and parts an edit is placed in. Read from the clothing cabinet.

use std::path::{Path, PathBuf};

use stave_app::check::{self, Checked, Outcome};
use stave_app::inspect::{self, ConnectionSection, LinkTarget, NameLink};
use stave_app::session::Selection;

fn cabinet() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/clothing_cabinet")
        .canonicalize()
        .unwrap()
}

fn checked() -> Box<Checked> {
    match check::run(&cabinet()) {
        Outcome::Checked(checked) => checked,
        Outcome::CantOpen { message, .. } => panic!("{message}"),
    }
}

fn names(links: &[NameLink]) -> Vec<&str> {
    links.iter().map(|link| link.name.as_str()).collect()
}

#[test]
fn a_door_stile_links_to_what_it_is_joined_to() {
    let checked = checked();
    let design = checked.design().expect("the cabinet solves");
    let stile = design.part_named("door_l.stile[hinge]").expect("the stile");
    let inspection =
        inspect::inspect(&checked, &Selection::Part(stile.id)).expect("the stile is described");
    let joined: Vec<_> = inspection
        .connections
        .iter()
        .filter(|c| c.section == ConnectionSection::Joined)
        .collect();
    let row = |lead: &str| {
        joined
            .iter()
            .find(|c| c.lead.starts_with(lead))
            .unwrap_or_else(|| panic!("no {lead} row in {joined:#?}"))
    };

    let mut rails = names(&row("cope_and_stick").partners);
    rails.sort_unstable();
    assert_eq!(rails, ["rail[bottom]", "rail[top]"]);
    assert_eq!(names(&row("groove").partners), ["panel"]);

    let hinges = row("euro_hinge");
    assert_eq!(hinges.lead, "euro_hinge ×2");
    assert_eq!(hinges.note, "back face");
    let mut hinge_names = names(&hinges.partners);
    hinge_names.sort_unstable();
    assert_eq!(hinge_names, ["hinges[lower]", "hinges[upper]"]);
    assert!(
        hinges
            .partners
            .iter()
            .all(|link| matches!(link.target, LinkTarget::Select(Selection::Hardware(_))))
    );

    let side_l = design.part_named("side_l").expect("side_l").id;
    assert_eq!(
        row("hinged on").partners,
        [NameLink {
            name: "side_l".to_owned(),
            target: LinkTarget::Select(Selection::Part(side_l)),
        }]
    );

    // Each partner selects the part it names.
    let rail = design
        .part_named("door_l.rail[top]")
        .expect("the top rail")
        .id;
    assert!(
        row("cope_and_stick")
            .partners
            .iter()
            .any(|link| link.target == LinkTarget::Select(Selection::Part(rail)))
    );
    // The bores the hinges make are cuts, not joints.
    assert!(
        inspection
            .connections
            .iter()
            .any(|c| c.section == ConnectionSection::Cuts && c.lead.contains("bore"))
    );
    assert!(inspection.checks.is_empty(), "{:?}", inspection.checks);
    assert!(inspection.swatch.is_some());
}

#[test]
fn a_place_links_to_its_space_and_parts() {
    let checked = checked();
    let design = checked.design().expect("the cabinet solves");
    let bay = inspect::place_links(design, "bay[doors]");
    assert_eq!(names(&bay), ["bay[doors]"]);
    assert!(matches!(bay[0].target, LinkTarget::Space(_)));

    let sides = inspect::place_links(design, "[side_l, side_r]");
    assert_eq!(names(&sides), ["side_l", "side_r"]);
    let side_r = design.part_named("side_r").expect("side_r").id;
    assert_eq!(sides[1].target, LinkTarget::Select(Selection::Part(side_r)));

    // A face links to its part, named as written.
    let face = inspect::place_links(design, "side_l.right");
    assert_eq!(names(&face), ["side_l.right"]);
    assert!(inspect::place_links(design, "no_such_thing").is_empty());
}

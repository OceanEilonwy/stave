//! A setting's short list, the search over its options, and the repeat
//! rule (`docs/editing.md`, "The short list and the search", "Repeat and
//! mirror"), driven through the session on projects the tests build in
//! temporary folders: a new project, a click in the envelope, Case.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use stave_app::candidates::new_project::{BoardForm, NewProject, NewStock, StockCut};
use stave_app::change::new_project_file;
use stave_app::choices::{ChoiceRole, Match, search, short_list};
use stave_app::edit::{OptionFit, Picked, SettingParam, SettingValue, Verdict};
use stave_app::editing::{
    EditCommand, EditEvent, Progress, entry_matches, parse_name, parse_value, write_all,
};
use stave_app::item_offer::{RepeatFold, candidate_repeat_fold};
use stave_app::scratch::DiskWriter;
use stave_app::session::{Command, Event, Session};
use stave_lang::units::Length;
use stave_schema::species::Species;

const TOOLS: &[&str] = &[
    "pencil",
    "marking_knife",
    "square",
    "tape",
    "marking_gauge",
    "awl",
    "calipers",
    "tenon_saw",
    "chisel",
    "mallet",
    "router_plane",
    "clamps",
    "screwdriver",
    "router",
    "straight_bit",
    "drill",
    "depth_stop",
];

/// A new project, 940 x 1780 x 580mm, importing `std:casework`, with one
/// oak board, written into a fresh temporary folder: the folder.
fn new_case_project(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "stave-setting-choices-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let project = NewProject {
        title: "Case test".to_owned(),
        size: [
            Length::from_mm(940),
            Length::from_mm(1780),
            Length::from_mm(580),
        ],
        stock: vec![NewStock {
            name: parse_name("oak_19").unwrap(),
            species: Species::from_word("white_oak").unwrap(),
            cut: StockCut::Board {
                form: BoardForm::Solid,
                thickness: Length::from_mm(19),
                width: None,
                max_width: Some(Length::from_mm(200)),
                max_length: Length::from_mm(2700),
            },
        }],
        tools: TOOLS.iter().map(|tool| parse_name(tool).unwrap()).collect(),
        imports: vec!["std:casework".to_owned()],
    };
    let file = new_project_file(&folder, &project).unwrap();
    write_all(std::slice::from_ref(&file), &mut DiskWriter).unwrap();
    folder
}

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

fn set(session: &mut Session, name: &str, value: &str) -> Vec<EditEvent> {
    send(
        session,
        EditCommand::Set {
            name: parse_name(name).unwrap(),
            value: parse_value(value).unwrap(),
        },
    )
}

/// A session on the folder with the Case chosen for the envelope; its
/// panel stock set to `oak_19` when `with_stock`.
fn case_chosen(folder: &Path, with_stock: bool) -> Session {
    let mut session = Session::new(folder.to_path_buf());
    let opened = stave_app::open(folder).unwrap();
    let model = opened.model.as_ref().unwrap();
    let envelope = model
        .regions
        .iter()
        .find(|region| region.name == "envelope")
        .unwrap()
        .id;
    send(&mut session, EditCommand::Start(Picked::Region(envelope)));
    let index = session
        .edit()
        .unwrap()
        .menu()
        .iter()
        .position(|candidate| entry_matches(candidate, "casework.Case"))
        .unwrap();
    send(&mut session, EditCommand::Choose(index));
    if with_stock {
        set(&mut session, "panel_stock", "oak_19");
        assert!(matches!(
            progress(&session),
            Progress::Tried(Verdict::Fits(_))
        ));
    }
    session
}

fn progress(session: &Session) -> Progress {
    session.edit().unwrap().chosen().unwrap().progress().clone()
}

fn setting(session: &Session, name: &str) -> SettingParam {
    session
        .edit()
        .unwrap()
        .chosen()
        .unwrap()
        .candidate()
        .settings
        .iter()
        .find(|s| s.name.as_str() == name)
        .unwrap()
        .clone()
}

#[test]
fn a_new_projects_case_offers_its_default_joinery_first_and_nothing_recent() {
    let folder = new_case_project("new");
    let session = case_chosen(&folder, true);
    let joinery = setting(&session, "joinery");
    assert!(joinery.recent.is_empty(), "{:?}", joinery.recent);
    let list = short_list(&joinery);
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].text, "dowel(count: 4)");
    assert_eq!(list[0].role, ChoiceRole::Default);
    assert!(list[0].current);
    // The stock chosen is the value it holds now, after no default.
    let stock = short_list(&setting(&session, "panel_stock"));
    let roles: Vec<(&str, ChoiceRole)> = stock.iter().map(|c| (c.text.as_str(), c.role)).collect();
    assert_eq!(roles, [("oak_19", ChoiceRole::Current)]);
}

#[test]
fn a_joint_kind_the_project_already_uses_is_offered_as_recent() {
    let folder = new_case_project("recent");
    let mut session = case_chosen(&folder, true);
    set(&mut session, "joinery", "screw");
    assert!(matches!(
        progress(&session),
        Progress::Tried(Verdict::Fits(_))
    ));
    let events = send(&mut session, EditCommand::Add);
    assert!(events.contains(&EditEvent::Committed), "{events:?}");

    // The envelope again: the Case's card, whether or not it fits there
    // now, reads the project's text.
    let session = case_chosen(&folder, false);
    let joinery = setting(&session, "joinery");
    assert_eq!(joinery.recent, ["screw"]);
    let list = short_list(&joinery);
    let roles: Vec<(&str, ChoiceRole)> = list.iter().map(|c| (c.text.as_str(), c.role)).collect();
    assert_eq!(
        roles,
        [
            ("dowel(count: 4)", ChoiceRole::Default),
            ("screw", ChoiceRole::Recent)
        ]
    );
    // The stock the case uses is recent for every stock setting.
    assert_eq!(setting(&session, "panel_stock").recent, ["oak_19"]);
}

#[test]
fn a_search_finds_chigiri_by_its_description_and_dowel_by_its_name_each_with_its_fit() {
    let folder = new_case_project("search");
    let mut session = case_chosen(&folder, true);
    session.check_setting_options(Some("joinery"), &AtomicBool::new(false));
    let joinery = setting(&session, "joinery");

    let found = search(&joinery, "key");
    let hits: Vec<_> = found.iter().flat_map(|group| group.hits.iter()).collect();
    let chigiri = hits
        .iter()
        .find(|hit| hit.option.text == "chigiri")
        .unwrap_or_else(|| panic!("chigiri not found: {hits:?}"));
    assert_eq!(chigiri.matched, Match::InDescription);
    assert!(chigiri.option.fit.is_refused());
    assert!(!chigiri.choosable());
    assert_eq!(found[0].title(), "Standard · joinery");

    let found = search(&joinery, "dow");
    let first = &found[0].hits[0];
    assert_eq!(first.option.text, "dowel");
    assert_eq!(first.matched, Match::Prefix);
    assert_eq!(first.option.fit, OptionFit::Fits);
    assert!(first.choosable());
}

#[test]
fn the_short_lists_default_puts_the_setting_back_to_its_default() {
    let folder = new_case_project("default");
    let mut session = case_chosen(&folder, true);
    set(&mut session, "joinery", "screw");
    assert!(!short_list(&setting(&session, "joinery"))[0].current);
    let events = send(
        &mut session,
        EditCommand::UseDefault(parse_name("joinery").unwrap()),
    );
    assert!(events.contains(&EditEvent::SettingChanged), "{events:?}");
    let joinery = setting(&session, "joinery");
    assert_eq!(joinery.value, SettingValue::Default);
    assert!(short_list(&joinery)[0].current);
    // A setting with no default can't go back to one.
    let events = send(
        &mut session,
        EditCommand::UseDefault(parse_name("panel_stock").unwrap()),
    );
    assert!(
        matches!(events.as_slice(), [EditEvent::Rejected(_)]),
        "{events:?}"
    );
}

#[test]
fn a_case_in_the_envelope_has_its_repeat_folded_until_asked_or_repeated() {
    let folder = new_case_project("repeat");
    let mut session = case_chosen(&folder, true);
    let fold = |session: &Session, asked: bool| {
        let chosen = session.edit().unwrap().chosen().unwrap();
        candidate_repeat_fold(chosen.candidate(), chosen.repeat(), asked)
    };
    assert_eq!(fold(&session, false), RepeatFold::Folded);
    assert_eq!(fold(&session, true), RepeatFold::Open);
    send(
        &mut session,
        EditCommand::Repeat(Some(stave_app::candidates::Repeat {
            count: stave_app::naming::RepeatCount::new(2).unwrap(),
            axis: stave_lang::placement::Axis::Across,
        })),
    );
    assert_eq!(fold(&session, false), RepeatFold::Open);
}

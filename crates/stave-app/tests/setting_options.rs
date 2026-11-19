//! Options are checked, and a refusal caused by a setting blames it and
//! offers the way back (`docs/editing.md`, "Options are checked" and
//! "Blame and the way back"). Driven through the session on a project
//! the test builds in a temporary folder, as the owner did by hand: a new
//! project, a click in the envelope, Case, its panel stock, then its
//! joinery.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use stave_app::candidates::new_project::{BoardForm, NewProject, NewStock, StockCut};
use stave_app::change::new_project_file;
use stave_app::edit::{OptionFit, Picked, SettingOption, SettingValue, Verdict};
use stave_app::editing::{
    EditCommand, EditEvent, Progress, Rejection, entry_matches, parse_name, parse_value, write_all,
};
use stave_app::scratch::DiskWriter;
use stave_app::session::{Command, Event, Session};
use stave_lang::Code;
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
        "stave-setting-options-{}-{name}",
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

fn options(session: &Session, setting: &str) -> Vec<SettingOption> {
    session
        .edit()
        .unwrap()
        .chosen()
        .unwrap()
        .candidate()
        .settings
        .iter()
        .find(|s| s.name.as_str() == setting)
        .unwrap()
        .options
        .clone()
}

fn fit_of(options: &[SettingOption], text: &str) -> OptionFit {
    options
        .iter()
        .find(|option| option.text == text)
        .unwrap_or_else(|| panic!("no option {text}"))
        .fit
        .clone()
}

fn refused_code(options: &[SettingOption], text: &str) -> Code {
    match fit_of(options, text) {
        OptionFit::Refused(refusal) => refusal.first.code,
        other => panic!("{text} should be refused, is {other:?}"),
    }
}

fn check_options(session: &mut Session, only: Option<&str>) -> Vec<EditEvent> {
    session
        .check_setting_options(only, &AtomicBool::new(false))
        .into_iter()
        .filter_map(|event| match event {
            Event::Edit(event) => Some(event),
            _ => None,
        })
        .collect()
}

#[test]
fn the_cases_joinery_options_are_checked_each_with_the_checkers_reason() {
    let folder = new_case_project("joinery");
    let mut session = case_chosen(&folder, true);

    // Before the check, nothing is known about the other kinds.
    let before = options(&session, "joinery");
    assert_eq!(fit_of(&before, "chigiri"), OptionFit::NotChecked);

    let events = check_options(&mut session, Some("joinery"));
    assert!(events.contains(&EditEvent::OptionsChanged), "{events:?}");
    let joinery = options(&session, "joinery");
    for fits in ["dowel", "dado", "screw"] {
        assert_eq!(fit_of(&joinery, fits), OptionFit::Fits, "{fits}");
    }
    // Edge joints: refused; which rule says so is the language's to
    // decide, so only the refusal is pinned here.
    for edge in ["chigiri", "splined_edge"] {
        assert!(fit_of(&joinery, edge).is_refused(), "{edge}");
    }
    assert_eq!(refused_code(&joinery, "half_lap"), Code::TypeMismatch);
    assert_eq!(refused_code(&joinery, "mortise_tenon"), Code::Require);

    // The ones that fit first, in their own order, the refused after.
    let first_refused = joinery
        .iter()
        .position(|option| option.fit.is_refused())
        .unwrap();
    assert!(joinery[first_refused..].iter().all(|o| o.fit.is_refused()));
    assert!(joinery[..first_refused].iter().all(|o| !o.fit.is_refused()));
    let texts = |part: &[SettingOption]| -> Vec<String> {
        part.iter().map(|option| option.text.clone()).collect()
    };
    let (fitting, refused) = (
        texts(&joinery[..first_refused]),
        texts(&joinery[first_refused..]),
    );
    let mut sorted = fitting.clone();
    sorted.sort();
    assert_eq!(fitting, sorted, "the fitting options keep the listed order");
    let mut sorted = refused.clone();
    sorted.sort();
    assert_eq!(refused, sorted, "the refused options keep the listed order");
    // Nothing is hidden.
    assert_eq!(joinery.len(), before.len());
}

#[test]
fn an_options_job_hands_back_answers_one_at_a_time_and_drops_stale_ones() {
    let folder = new_case_project("job");
    let mut session = case_chosen(&folder, true);
    let job = session.options_job().unwrap().only("joinery");
    assert_eq!(job.tries().len(), options(&session, "joinery").len());
    let job = std::thread::spawn(move || {
        let mut found = Vec::new();
        job.run(&AtomicBool::new(false), &mut |fit| found.push(fit))
            .unwrap();
        found
    });
    let found = job.join().unwrap();
    assert_eq!(found.len(), options(&session, "joinery").len());

    // A change to the candidate before the answers arrive: they are for
    // the candidate as it was, and are dropped.
    send(
        &mut session,
        EditCommand::Rename(parse_name("tall_case").unwrap()),
    );
    for fit in found.clone() {
        assert!(session.option_fit_found(fit).is_empty());
    }
    assert_eq!(
        fit_of(&options(&session, "joinery"), "chigiri"),
        OptionFit::NotChecked
    );

    // A cancelled job stops before it tries anything.
    let job = session.options_job().unwrap();
    let mut reached = 0;
    let result = job.run(&AtomicBool::new(true), &mut |_| reached += 1);
    assert!(result.is_err());
    assert_eq!(reached, 0);
}

#[test]
fn the_options_are_checked_again_after_another_setting_changes() {
    let folder = new_case_project("again");
    let mut session = case_chosen(&folder, true);
    check_options(&mut session, None);
    assert_eq!(
        fit_of(&options(&session, "joinery"), "dado"),
        OptionFit::Fits
    );
    assert_eq!(
        fit_of(&options(&session, "panel_stock"), "oak_19"),
        OptionFit::Fits
    );

    // Another setting changes: the joinery's answers are thrown away and
    // worked out again, for the candidate as it now is. An overhang as
    // wide as the case refuses every joinery option.
    set(&mut session, "overhang", "500mm");
    assert_eq!(
        fit_of(&options(&session, "joinery"), "dado"),
        OptionFit::NotChecked
    );
    check_options(&mut session, Some("joinery"));
    let joinery = options(&session, "joinery");
    assert!(
        joinery.iter().all(|option| option.fit.is_refused()),
        "{joinery:?}"
    );
    assert_eq!(refused_code(&joinery, "dado"), Code::Require);
}

#[test]
fn the_setting_just_set_keeps_its_options_answers() {
    let folder = new_case_project("kept");
    let mut session = case_chosen(&folder, true);
    check_options(&mut session, Some("joinery"));
    set(&mut session, "joinery", "dado");
    // Each option of joinery was tried with the other settings, which did
    // not change.
    let joinery = options(&session, "joinery");
    assert_eq!(fit_of(&joinery, "screw"), OptionFit::Fits);
    assert!(fit_of(&joinery, "chigiri").is_refused());
    // The others were tried with the old joinery.
    assert_eq!(
        fit_of(&options(&session, "panel_stock"), "oak_19"),
        OptionFit::Fits,
        "the option the setting holds has the candidate's own verdict"
    );
}

#[test]
fn options_wait_while_something_else_is_still_to_choose() {
    let folder = new_case_project("waiting");
    let session = case_chosen(&folder, false);
    // The panel stock has no value yet: no joinery can be tried.
    let joinery = options(&session, "joinery");
    assert!(joinery.iter().all(|option| matches!(
        &option.fit,
        OptionFit::NeedsChoice(names) if names.first().as_str() == "panel_stock"
    )));
    // The panel stock's own options can be: every other setting has a
    // value.
    assert_eq!(
        fit_of(&options(&session, "panel_stock"), "oak_19"),
        OptionFit::NotChecked
    );
    let mut session = session;
    check_options(&mut session, Some("panel_stock"));
    assert_eq!(
        fit_of(&options(&session, "panel_stock"), "oak_19"),
        OptionFit::Fits
    );
}

#[test]
fn typing_chigiri_blames_the_joinery_and_putting_it_back_fits_again() {
    let folder = new_case_project("blame");
    let mut session = case_chosen(&folder, true);
    let events = set(&mut session, "joinery", "chigiri");
    assert!(events.contains(&EditEvent::VerdictChanged));
    assert!(matches!(
        progress(&session),
        Progress::Tried(Verdict::Refused(_))
    ));
    let chosen = session.edit().unwrap().chosen().unwrap();
    let blame = chosen.blame().expect("the refusal blames the joinery");
    assert_eq!(blame.setting.as_str(), "joinery");
    assert_eq!(blame.from, SettingValue::Default);
    assert_eq!(blame.from_text.as_deref(), Some("dowel(count: 4)"));
    assert_eq!(blame.to_text, "chigiri");
    assert_eq!(
        blame.text(),
        "It stopped fitting when joinery became chigiri."
    );
    assert_eq!(
        blame.put_back_label(),
        "Put joinery back to dowel(count: 4), the default"
    );

    // Changing the same setting again while it is to blame keeps where it
    // came from.
    set(&mut session, "joinery", "splined_edge");
    let blame = session.edit().unwrap().chosen().unwrap().blame().unwrap();
    assert_eq!(blame.from_text.as_deref(), Some("dowel(count: 4)"));
    assert_eq!(blame.to_text, "splined_edge");

    let events = send(&mut session, EditCommand::PutBack);
    assert!(events.contains(&EditEvent::SettingChanged), "{events:?}");
    assert!(matches!(
        progress(&session),
        Progress::Tried(Verdict::Fits(_))
    ));
    let chosen = session.edit().unwrap().chosen().unwrap();
    assert!(chosen.blame().is_none());
    let joinery = chosen
        .candidate()
        .settings
        .iter()
        .find(|s| s.name.as_str() == "joinery")
        .unwrap();
    assert_eq!(
        joinery.value,
        SettingValue::Default,
        "a default stays a default"
    );

    // Nothing left to put back.
    let events = send(&mut session, EditCommand::PutBack);
    assert!(events.contains(&EditEvent::Rejected(Rejection::NothingToPutBack)));
}

#[test]
fn a_stock_that_needed_choosing_is_blamed_and_cleared_again() {
    let folder = new_case_project("unset");
    let mut session = case_chosen(&folder, false);
    // The joinery set first, while the stock is still to choose.
    set(&mut session, "joinery", "chigiri");
    assert!(matches!(progress(&session), Progress::NeedsChoice(_)));
    set(&mut session, "panel_stock", "oak_19");
    let blame = session
        .edit()
        .unwrap()
        .chosen()
        .unwrap()
        .blame()
        .cloned()
        .unwrap();
    assert_eq!(blame.setting.as_str(), "panel_stock");
    assert_eq!(blame.from, SettingValue::Unset);
    assert_eq!(
        blame.text(),
        "It doesn't fit now that panel_stock is oak_19."
    );
    assert_eq!(blame.put_back_label(), "Clear panel_stock again");
    send(&mut session, EditCommand::PutBack);
    assert!(matches!(progress(&session), Progress::NeedsChoice(_)));
}

#[test]
fn a_candidate_already_refused_gets_no_blame() {
    let folder = new_case_project("already");
    let mut session = case_chosen(&folder, true);
    // Refused by an overhang wider than the case, set while the candidate
    // fitted: the overhang is blamed.
    set(&mut session, "overhang", "500mm");
    assert!(matches!(
        progress(&session),
        Progress::Tried(Verdict::Refused(_))
    ));
    let blame = session.edit().unwrap().chosen().unwrap().blame().cloned();
    assert_eq!(blame.unwrap().setting.as_str(), "overhang");
    // Then the joinery changes while it is refused for the overhang: no
    // blame for the joinery, and none left for the overhang.
    set(&mut session, "joinery", "chigiri");
    assert!(matches!(
        progress(&session),
        Progress::Tried(Verdict::Refused(_))
    ));
    assert!(session.edit().unwrap().chosen().unwrap().blame().is_none());
    let events = send(&mut session, EditCommand::PutBack);
    assert!(events.contains(&EditEvent::Rejected(Rejection::NothingToPutBack)));
}

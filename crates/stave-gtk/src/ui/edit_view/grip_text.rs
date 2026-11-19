//! The words and numbers the grips show (`docs/gui.md`, "Grips"): what a
//! grip's label says, how a value typed becomes an amount of the grip's
//! kind, what the size section of the Edit panel says while a drag is
//! open, and the one-line readout at the top of the view. Nothing here decides what
//! a grip may do: that is the session's (`stave_app::gripping`), and
//! where a drag snaps, how far it goes and whether a typed value is
//! written as typed are `stave_app::grip_values`; these only put the
//! answers into words and turn the pointer's position into a value.

use stave_app::change::{Change, ChangeDetail, ChangeRequest};
use stave_app::edit::Verdict;
use stave_app::editing::Progress;
pub(super) use stave_app::grip_values::{Held, grip_value};
use stave_app::grip_values::{TypedProblem, grip_bounds};
use stave_app::gripping::{GripDetail, GripStops, ListedGrip};
use stave_app::grips::{FollowKind, GripAction, GripSize, MovableGrip};
use stave_app::stops::{Side, SideEnd, amount_text};
use stave_lang::ast::{ExprKind, Ident};
use stave_lang::units::{Length, Rational};
use stave_model::ids::ParamId;
use stave_model::trace::{Amount, Freedom};

/// How a grip is drawn: never by colour alone, each has its own shape,
/// line and words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GripLook {
    /// A plain number: a round handle, a solid dimension line, its value.
    Moves,
    /// A size worked out from params: a round handle as any other, a
    /// dashed line; "follows overall_width" is said in words.
    Follows,
    /// Nothing it follows can change, or it can't move either way: a
    /// padlock and no handle, a dotted line.
    Locked,
}

/// How a listed grip is drawn.
pub(super) fn grip_look(listed: &ListedGrip) -> GripLook {
    match &listed.grip.action {
        GripAction::Moves(movable) => {
            if stuck(movable, &listed.stops) {
                GripLook::Locked
            } else {
                GripLook::Moves
            }
        }
        GripAction::Follows(follows) => {
            if follows.follows.iter().any(|value| {
                value.kind.is_candidate() && value.coefficient.is_some() && value.path.is_some()
            }) {
                GripLook::Follows
            } else {
                GripLook::Locked
            }
        }
    }
}

/// `true` when a movable grip's stops were found and it can't go a step
/// either way.
fn stuck(movable: &MovableGrip, stops: &GripStops) -> bool {
    match stops {
        GripStops::Found { below, above } => {
            below.last_good == movable.value && above.last_good == movable.value
        }
        GripStops::NotSearched | GripStops::NotSearchable | GripStops::Failed(_) => false,
    }
}

/// An amount as `.stave` writes it.
pub(super) fn amount_words(amount: Amount) -> String {
    amount_text(amount).unwrap_or_else(|| "?".to_owned())
}

/// What a grip's label says: its value; for a size that follows params,
/// what it follows; for one that can't move, why not.
pub(super) fn label_words(listed: &ListedGrip) -> String {
    let value = amount_words(grip_value(listed));
    match (&listed.grip.action, grip_look(listed)) {
        (GripAction::Moves(movable), GripLook::Locked) => {
            let why = stop_words(&listed.stops)
                .map(|(below, _)| below)
                .unwrap_or_else(|| "it can't move".to_owned());
            let _ = movable;
            format!("{value} · stuck: {why}")
        }
        (GripAction::Moves(movable), GripLook::Moves | GripLook::Follows) => {
            match movable.freedom {
                Freedom::Solved => format!("{value} · solved"),
                Freedom::Plain | Freedom::Locked | Freedom::StockField | Freedom::KindSize => value,
            }
        }
        (GripAction::Follows(follows), look) => {
            let followed: Vec<String> = follows
                .follows
                .iter()
                .filter(|value| match look {
                    GripLook::Follows => value.kind.is_candidate(),
                    GripLook::Moves | GripLook::Locked => true,
                })
                .map(|value| match value.kind {
                    FollowKind::Plain => value.name.clone(),
                    kind => format!("{} ({})", value.name, kind.words()),
                })
                .collect();
            let list = if followed.is_empty() {
                "numbers only".to_owned()
            } else {
                followed.join(", ")
            };
            match look {
                GripLook::Follows => format!("{value} · follows {list}"),
                GripLook::Moves | GripLook::Locked => {
                    format!("{value} · locked: follows {list}")
                }
            }
        }
    }
}

/// What a grip's stops say, each way: "down to 0mm: legs and
/// apron_front.apron overlap"; `None` before they are found.
pub(super) fn stop_words(stops: &GripStops) -> Option<(String, String)> {
    match stops {
        GripStops::Found { below, above } => Some((side_words(below), side_words(above))),
        GripStops::NotSearched | GripStops::NotSearchable | GripStops::Failed(_) => None,
    }
}

/// Where one side stops, and what stops it.
pub(super) fn side_words(side: &Side) -> String {
    let at = amount_words(side.last_good);
    match &side.end {
        SideEnd::Stopped {
            at: next,
            diagnostic,
        } => format!(
            "stops at {at}, because at {} {}",
            amount_words(*next),
            diagnostic.message
        ),
        SideEnd::Domain => format!("stops at {at}: the end of its range"),
        SideEnd::Limit => format!("as far as {at} was tried"),
    }
}

/// How one end of a grip's range is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RangeEnd {
    /// A rule stops it there: a hard red end, hatched beyond.
    Stop,
    /// The end of its `within`: a hard grey end.
    Domain,
    /// Only as far as was searched: an open end.
    Open,
}

/// How a side's end is drawn.
pub(super) fn range_end(side: &Side) -> RangeEnd {
    match side.end {
        SideEnd::Stopped { .. } => RangeEnd::Stop,
        SideEnd::Domain => RangeEnd::Domain,
        SideEnd::Limit => RangeEnd::Open,
    }
}

/// A grip's range with how each end is drawn: its two last good values
/// once its stops are found; before, its `within` (or how far it may be
/// searched), both ends open.
pub(super) fn grip_range(listed: &ListedGrip) -> Option<((Amount, RangeEnd), (Amount, RangeEnd))> {
    let (low, high) = grip_bounds(listed)?;
    Some(match &listed.stops {
        GripStops::Found { below, above } => ((low, range_end(below)), (high, range_end(above))),
        GripStops::NotSearched | GripStops::NotSearchable | GripStops::Failed(_) => {
            ((low, RangeEnd::Open), (high, RangeEnd::Open))
        }
    })
}

/// A length as millimetres, for drawing.
pub(super) fn length_mm(amount: Amount) -> Option<f64> {
    match amount {
        Amount::Length(length) => length.to_nearest_length().map(Length::to_mm_f64),
        Amount::Angle(_) | Amount::Count(_) | Amount::Ratio(_) => None,
    }
}

/// Millimetres from the pointer as a length amount (to the nanometre).
pub(super) fn mm_amount(mm: f64) -> Option<Amount> {
    stave_geom::render::length_from_nm(mm * 1e6).map(|length| Amount::Length(length.to_exact()))
}

/// What the handle says when a drag is held at an end of its range: the
/// rule that stops it there.
pub(super) fn held_words(listed: &ListedGrip, held: Held) -> Option<String> {
    match &listed.stops {
        GripStops::Found { below, above } => Some(side_words(match held {
            Held::Below => below,
            Held::Above => above,
        })),
        GripStops::NotSearched | GripStops::NotSearchable | GripStops::Failed(_) => None,
    }
}

/// A value typed for a grip whose value is now `now`, read as the same
/// kind of value: a length (`400mm`), an angle (`95deg`) or a plain
/// number. What is wrong, in words, when it can't be.
pub(super) fn amount_like(now: Amount, text: &str) -> Result<Amount, String> {
    let expr = stave_app::editing::parse_value(text)
        .map_err(|error| format!("'{text}' isn't a value Stave can read: {}", error.0))?;
    let wrong = || {
        format!(
            "'{text}' isn't the same kind of value as {}",
            amount_words(now)
        )
    };
    Ok(match (now, &expr.kind) {
        (Amount::Length(_), ExprKind::Quantity(quantity)) => {
            Amount::Length(stave_lang::units::quantity_to_length(*quantity).map_err(|_| wrong())?)
        }
        (Amount::Angle(_), ExprKind::Quantity(quantity)) => {
            Amount::Angle(stave_lang::units::quantity_to_angle(*quantity).map_err(|_| wrong())?)
        }
        (Amount::Count(_), ExprKind::Number(number)) => {
            Amount::Count(stave_lang::units::number_to_rational(*number).map_err(|_| wrong())?)
        }
        (Amount::Ratio(_), ExprKind::Number(number)) => {
            Amount::Ratio(stave_lang::units::number_to_rational(*number).map_err(|_| wrong())?)
        }
        _ => return Err(wrong()),
    })
}

/// A value typed for a movable grip checked before it is tried
/// ([`stave_app::grip_values::typed_check`]): what the session would
/// write differently, in words.
pub(super) fn typed_check(listed: &ListedGrip, value: Amount) -> Result<(), String> {
    let why = match stave_app::grip_values::typed_check(listed, value) {
        Ok(()) => return Ok(()),
        Err(TypedProblem::OutOfRange { held, low, high }) => held_words(listed, held).map_or_else(
            || {
                format!(
                    "this grip moves it between {} and {}",
                    amount_words(low),
                    amount_words(high)
                )
            },
            |why| format!("it {why}"),
        ),
        Err(TypedProblem::OffStep { step, nearest }) => format!(
            "it goes in steps of {}; the nearest is {}",
            amount_words(step),
            amount_words(nearest)
        ),
        Err(TypedProblem::NotListed(values)) => {
            let words: Vec<String> = values.iter().map(|v| amount_words(*v)).collect();
            format!("it can only be one of {}", words.join(", "))
        }
        Err(TypedProblem::Unworkable) => "it can't be worked out".to_owned(),
    };
    Err(format!("Can't be {}: {why}", amount_words(value)))
}

/// A param of the project a grip's switch can make solved, or plain
/// again: its name and whether it is solved now. Only the project's own
/// params can change kind (`param_kind_patch` refuses the rest), so a
/// component's param (`divider.height`) has no switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SolverSwitch {
    /// The param.
    pub(super) param: Ident,
    /// Its value now, to suggest a range around.
    pub(super) value: Amount,
    /// `true` when it is a `solved param`.
    pub(super) solved: bool,
}

/// The switches a grip offers: its own param when it moves a param of
/// the project, else each param of the project the size follows.
pub(super) fn solver_switches(listed: &ListedGrip) -> Vec<SolverSwitch> {
    let own = |path: &str| stave_app::editing::parse_name(path).ok();
    match (&listed.grip.size, &listed.grip.action) {
        (
            GripSize::Param {
                path: Some(path), ..
            },
            GripAction::Moves(movable),
        ) => own(path)
            .map(|param| SolverSwitch {
                param,
                value: movable.value,
                solved: movable.freedom == Freedom::Solved,
            })
            .into_iter()
            .collect(),
        (_, GripAction::Follows(follows)) => follows
            .follows
            .iter()
            .filter(|value| value.kind.is_candidate())
            .filter_map(|value| {
                Some(SolverSwitch {
                    param: own(value.path.as_deref()?)?,
                    value: value.value,
                    solved: value.kind == FollowKind::Solved,
                })
            })
            .collect(),
        (_, GripAction::Moves(_)) => Vec::new(),
    }
}

/// A range to suggest for a param about to be made solved: a fifth either
/// side of its value, on whole millimetres for a length.
pub(super) fn suggested_range(value: Amount) -> (String, String) {
    let part = |numerator: i128| -> Amount {
        let raw = value
            .raw()
            .checked_mul(Rational::new(numerator, 5).unwrap_or(Rational::ONE))
            .unwrap_or(value.raw());
        let raw = match value {
            Amount::Length(_) => Rational::from_int(
                raw.checked_div(Rational::from_int(1_000_000))
                    .map_or(0, Rational::round),
            )
            .checked_mul(Rational::from_int(1_000_000))
            .unwrap_or(raw),
            Amount::Angle(_) | Amount::Count(_) | Amount::Ratio(_) => raw,
        };
        value.with_raw(raw)
    };
    (amount_words(part(4)), amount_words(part(6)))
}

/// One change a drag makes, in words: "overall_width 900mm to 950mm".
pub(super) fn change_words(path: &str, from: Amount, to: Amount) -> String {
    format!("{path} {} to {}", amount_words(from), amount_words(to))
}

/// From one value to another, short: "940 → 1040 mm" (the unit once
/// when both are millimetres), "20 → -30 mm", "3 → 4".
pub(super) fn arrow_words(from: Amount, to: Amount) -> String {
    let (from, to) = (amount_words(from), amount_words(to));
    match (from.strip_suffix("mm"), to.strip_suffix("mm")) {
        (Some(from), Some(to)) => format!("{from} → {to} mm"),
        _ => format!("{from} → {to}"),
    }
}

/// What a grip is beside its value, from its label's words: "follows
/// overall_width", "solved", "stuck: …"; `None` for a plain size.
pub(super) fn about_words(listed: &ListedGrip) -> Option<String> {
    label_words(listed)
        .split_once(" · ")
        .map(|(_, about)| about.to_owned())
}

/// The verdict's look in the panel: its class and its words, which never
/// rest on colour alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PanelVerdict {
    /// `fits`, `refused`, `checking` or `waiting`.
    pub(super) class: &'static str,
    /// What it says.
    pub(super) words: String,
}

/// A size a follows grip could change, for the list to pick from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PanelChoice {
    /// The param.
    pub(super) param: ParamId,
    /// "overall_width 500mm to 520mm", or why it can't.
    pub(super) words: String,
    /// The size's name: "overall_width".
    pub(super) name: String,
    /// What it would go from and to ("500 → 520 mm"), or why it can't.
    pub(super) value: String,
    /// `false` when it can't give the size.
    pub(super) possible: bool,
    /// `true` when it is the one picked.
    pub(super) picked: bool,
}

/// What the panel beside a drag (or a change of a param's kind) shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PanelModel {
    /// What is being changed: "divider.height".
    pub(super) title: String,
    /// The value it is moved to and what it was: "420mm (was 400mm)".
    pub(super) value: Option<String>,
    /// What the grip is beside its value: "follows overall_width".
    pub(super) about: Option<String>,
    /// What changes and to what: "Changes overall_width 900mm to 950mm".
    pub(super) changes: Vec<String>,
    /// Several plain params could give the size: pick one.
    pub(super) choices: Vec<PanelChoice>,
    /// Held at an end of its range: the rule that stops it there.
    pub(super) stop: Option<String>,
    /// The verdict.
    pub(super) verdict: PanelVerdict,
    /// Apply writes it: only when it fits.
    pub(super) can_apply: bool,
    /// The one line at the top of the view: the size's name and from what
    /// to what ("overall_width", "940 → 1040 mm").
    pub(super) readout: Option<(String, String)>,
}

/// The verdict for where a change stands.
fn panel_verdict(progress: &Progress, checking: bool) -> PanelVerdict {
    if checking {
        return PanelVerdict {
            class: "checking",
            words: "Checking…".to_owned(),
        };
    }
    match progress {
        Progress::Tried(Verdict::Checking) => PanelVerdict {
            class: "checking",
            words: "Checking…".to_owned(),
        },
        Progress::Tried(Verdict::Fits(_)) => PanelVerdict {
            class: "fits",
            words: "✓ It fits".to_owned(),
        },
        Progress::Tried(Verdict::Refused(refusal)) => {
            let problems = refusal.problems();
            let first = &problems.first().diagnostic().message;
            let more = problems.iter().count().saturating_sub(1);
            PanelVerdict {
                class: "refused",
                words: if more == 0 {
                    format!("✕ It doesn't fit: {first}")
                } else {
                    format!("✕ It doesn't fit: {first} (and {more} more)")
                },
            }
        }
        Progress::NeedsChoice(_) => PanelVerdict {
            class: "waiting",
            words: "Pick which size changes".to_owned(),
        },
        Progress::Unwritable(reason) => PanelVerdict {
            class: "refused",
            words: format!("✕ {}", capitalised(reason)),
        },
    }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The panel for a drag, from its change and its grip.
fn drag_panel(
    detail: &GripDetail,
    listed: Option<&ListedGrip>,
    progress: &Progress,
    checking: bool,
    stop: Option<String>,
) -> PanelModel {
    let was = listed.map(grip_value);
    let value = detail.target.map(|target| {
        // A movable grip shows the value written (snapped), a follows
        // grip the size it is moved to.
        let written = match listed.map(|listed| &listed.grip.action) {
            Some(GripAction::Moves(_)) => detail.changes.first().map_or(target, |change| change.to),
            Some(GripAction::Follows(_)) | None => detail.reached.unwrap_or(target),
        };
        match was {
            Some(was) if was != written => {
                format!("{} (was {})", amount_words(written), amount_words(was))
            }
            Some(_) | None => amount_words(written),
        }
    });
    let follows = matches!(
        listed.map(|listed| &listed.grip.action),
        Some(GripAction::Follows(_))
    );
    let changes = if follows {
        detail
            .changes
            .iter()
            .map(|change| {
                let words = change_words(&change.path, change.from, change.to);
                if detail.solved {
                    format!("The solver changes {words}")
                } else {
                    format!("Changes {words}")
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let choices = detail
        .choices
        .iter()
        .map(|choice| match &choice.change {
            Ok(change) => PanelChoice {
                param: choice.param,
                words: change_words(&choice.name, change.from, change.to),
                name: choice.name.clone(),
                value: arrow_words(change.from, change.to),
                possible: true,
                picked: detail.picked == Some(choice.param),
            },
            Err(why) => PanelChoice {
                param: choice.param,
                words: format!("{}: {why}", choice.name),
                name: choice.name.clone(),
                value: why.to_string(),
                possible: false,
                picked: false,
            },
        })
        .collect();
    let mut verdict = panel_verdict(progress, checking);
    if detail.target.is_none() && !checking {
        verdict = PanelVerdict {
            class: "waiting",
            words: "Drag the handle, or type a value".to_owned(),
        };
    }
    let fits = matches!(progress, Progress::Tried(Verdict::Fits(_))) && !checking;
    // The readout: the size that changes, from and to; before one is
    // picked, the grip's own size.
    let readout = detail
        .changes
        .first()
        .map(|change| (change.path.clone(), arrow_words(change.from, change.to)))
        .or_else(|| {
            let target = detail.target?;
            let words = match was {
                Some(was) => arrow_words(was, detail.reached.unwrap_or(target)),
                None => amount_words(target),
            };
            Some((detail.label.clone(), words))
        });
    PanelModel {
        title: detail.label.clone(),
        value,
        about: listed.and_then(about_words),
        changes,
        choices,
        stop,
        verdict,
        can_apply: fits,
        readout,
    }
}

/// What the panel shows for a change the grips started (a drag, or a
/// param made solved or plain); `None` for another change.
pub(super) fn panel_model(
    change: &Change,
    listed: impl Fn(usize) -> Option<ListedGrip>,
    checking: bool,
    stop: Option<String>,
) -> Option<PanelModel> {
    match (change.request(), change.detail()) {
        (ChangeRequest::Grip { index }, ChangeDetail::Grip(detail)) => Some(drag_panel(
            detail,
            listed(*index).as_ref(),
            change.progress(),
            checking,
            stop,
        )),
        (ChangeRequest::ParamKind { .. }, _) => {
            let fits = matches!(change.progress(), Progress::Tried(Verdict::Fits(_))) && !checking;
            Some(PanelModel {
                title: change.summary().to_owned(),
                value: None,
                about: None,
                changes: Vec::new(),
                choices: Vec::new(),
                stop: None,
                verdict: panel_verdict(change.progress(), checking),
                can_apply: fits,
                readout: None,
            })
        }
        _ => None,
    }
}

/// Why a typed value was not applied, in words, for under the entry;
/// `None` when it fits.
pub(super) fn typed_refusal(progress: &Progress) -> Option<String> {
    match progress {
        Progress::Tried(Verdict::Fits(_)) => None,
        progress => Some(panel_verdict(progress, false).words),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_app::grip_values::{clamped, snapped};
    use stave_app::grips::{FollowedValue, FollowsGrip, SelectionGrip};
    use stave_app::stops::SearchSteps;
    use stave_lang::{Code, Diagnostic, Severity};
    use stave_model::trace::Source;

    fn mm(n: i64) -> Amount {
        Amount::Length(Length::from_mm(n).to_exact())
    }

    fn movable(value: i64, stops: GripStops) -> ListedGrip {
        ListedGrip {
            grip: SelectionGrip {
                size: GripSize::Param {
                    param: ParamId::from_index(0).unwrap(),
                    path: Some("overall_width".to_owned()),
                },
                label: "overall_width".to_owned(),
                axis: None,
                action: GripAction::Moves(Box::new(MovableGrip {
                    value: mm(value),
                    low: mm(600),
                    high: mm(1200),
                    steps: SearchSteps::Step(Amount::Length(Length::from_nm(500_000).to_exact())),
                    freedom: Freedom::Plain,
                    stops: None,
                })),
                place: None,
            },
            stops,
        }
    }

    fn stopped_side(at: i64, next: i64, message: &str) -> Side {
        Side {
            last_good: mm(at),
            end: SideEnd::Stopped {
                at: mm(next),
                diagnostic: Diagnostic {
                    severity: Severity::Error,
                    code: Code::Require,
                    message: message.to_owned(),
                    labels: Vec::new(),
                    notes: Vec::new(),
                    merged: Vec::new(),
                    help: None,
                    fixes: Vec::new(),
                    cause: None,
                },
            },
        }
    }

    #[test]
    fn a_movable_grip_says_its_value_and_snaps_to_the_resolution() {
        let grip = movable(900, GripStops::NotSearched);
        assert_eq!(grip_look(&grip), GripLook::Moves);
        assert_eq!(label_words(&grip), "900mm");
        let GripAction::Moves(m) = &grip.grip.action else {
            unreachable!()
        };
        let wanted = mm_amount(912.3).unwrap();
        assert_eq!(
            amount_words(snapped(m.value, wanted, Some(&m.steps), false)),
            "912.5mm"
        );
        assert_eq!(
            amount_words(snapped(m.value, wanted, Some(&m.steps), true)),
            "910mm"
        );
        assert_eq!(length_mm(mm(912)), Some(912.0));
    }

    #[test]
    fn a_drag_past_a_stop_is_held_there_and_names_the_rule() {
        let grip = movable(
            5,
            GripStops::Found {
                below: Box::new(Side {
                    last_good: mm(0),
                    end: SideEnd::Domain,
                }),
                above: Box::new(stopped_side(7, 8, "'mortise_wall' fails")),
            },
        );
        let ((low, low_end), (high, high_end)) = grip_range(&grip).unwrap();
        assert_eq!((low_end, high_end), (RangeEnd::Domain, RangeEnd::Stop));
        let (value, held) = clamped(mm(9), low, high);
        assert_eq!((value, held), (mm(7), Some(Held::Above)));
        assert_eq!(
            held_words(&grip, Held::Above).unwrap(),
            "stops at 7mm, because at 8mm 'mortise_wall' fails"
        );
        assert_eq!(clamped(mm(6), low, high), (mm(6), None));
    }

    #[test]
    fn a_typed_value_the_session_would_change_is_refused_whatever_the_stops() {
        // Before the stops are searched: the range is the within (or the
        // reach), and a value past it is refused, not held at its end.
        for stops in [
            GripStops::NotSearched,
            GripStops::NotSearchable,
            GripStops::Failed("no".to_owned()),
        ] {
            let grip = movable(900, stops);
            assert_eq!(
                typed_check(&grip, mm(9000)).unwrap_err(),
                "Can't be 9000mm: this grip moves it between 600mm and 1200mm"
            );
            assert_eq!(
                typed_check(&grip, mm(100)).unwrap_err(),
                "Can't be 100mm: this grip moves it between 600mm and 1200mm"
            );
            assert_eq!(typed_check(&grip, mm(950)), Ok(()));
            assert_eq!(typed_check(&grip, mm(1200)), Ok(()));
        }
        // Found: the rule that stops it.
        let grip = movable(
            900,
            GripStops::Found {
                below: Box::new(Side {
                    last_good: mm(600),
                    end: SideEnd::Domain,
                }),
                above: Box::new(stopped_side(1000, 1001, "'fits_room' fails")),
            },
        );
        assert_eq!(
            typed_check(&grip, mm(1100)).unwrap_err(),
            "Can't be 1100mm: it stops at 1000mm, because at 1001mm 'fits_room' fails"
        );
        // Off the resolution: refused with the nearest step, not rounded.
        let grip = movable(900, GripStops::NotSearched);
        assert_eq!(
            typed_check(&grip, mm_amount(912.3).unwrap()).unwrap_err(),
            "Can't be 912.3mm: it goes in steps of 0.5mm; the nearest is 912.5mm"
        );
        assert_eq!(typed_check(&grip, mm_amount(912.5).unwrap()), Ok(()));
    }

    #[test]
    fn a_grip_stuck_both_ways_is_locked() {
        let grip = movable(
            5,
            GripStops::Found {
                below: Box::new(stopped_side(5, 4, "parts overlap")),
                above: Box::new(stopped_side(5, 6, "parts overlap")),
            },
        );
        assert_eq!(grip_look(&grip), GripLook::Locked);
        assert!(label_words(&grip).starts_with("5mm · stuck: stops at 5mm, because"));
    }

    fn follows(kind: FollowKind) -> ListedGrip {
        ListedGrip {
            grip: SelectionGrip {
                size: GripSize::Param {
                    param: ParamId::from_index(1).unwrap(),
                    path: None,
                },
                label: "case.width".to_owned(),
                axis: None,
                action: GripAction::Follows(Box::new(FollowsGrip {
                    value: mm(900),
                    trace: stave_model::trace::Trace {
                        quantity: stave_model::trace::Quantity::Param(
                            ParamId::from_index(1).unwrap(),
                        ),
                        value: mm(900),
                        form: stave_model::trace::Form::Linear(stave_model::trace::Linear {
                            constant: mm(0),
                            terms: Vec::new(),
                        }),
                        followed: Vec::new(),
                    },
                    follows: vec![FollowedValue {
                        source: Source::Param(ParamId::from_index(0).unwrap()),
                        name: "overall_width".to_owned(),
                        path: Some("overall_width".to_owned()),
                        kind,
                        coefficient: Some(Rational::ONE),
                        value: mm(900),
                    }],
                })),
                place: None,
            },
            stops: GripStops::NotSearchable,
        }
    }

    #[test]
    fn a_size_that_follows_says_what_it_follows_and_offers_its_switch() {
        let plain = follows(FollowKind::Plain);
        assert_eq!(grip_look(&plain), GripLook::Follows);
        assert_eq!(label_words(&plain), "900mm · follows overall_width");
        let switches = solver_switches(&plain);
        assert_eq!(switches.len(), 1);
        assert!(!switches[0].solved);
        let locked = follows(FollowKind::Locked);
        assert_eq!(grip_look(&locked), GripLook::Locked);
        assert_eq!(
            label_words(&locked),
            "900mm · locked: follows overall_width (locked)"
        );
        assert!(solver_switches(&locked).is_empty());
    }

    #[test]
    fn a_typed_value_is_read_as_the_grips_kind() {
        assert_eq!(amount_like(mm(900), "950mm"), Ok(mm(950)));
        assert!(amount_like(mm(900), "3").unwrap_err().contains("same kind"));
        assert!(
            amount_like(mm(900), "9 9")
                .unwrap_err()
                .contains("can read")
        );
        assert_eq!(
            suggested_range(mm(900)),
            ("720mm".to_owned(), "1080mm".to_owned())
        );
        assert_eq!(
            change_words("overall_width", mm(900), mm(950)),
            "overall_width 900mm to 950mm"
        );
    }
}

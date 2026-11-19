//! Shows the editing menu and card over the design from the contract's
//! hand-built fixture (`stave_app::edit::clothing_cabinet_fixture`), for
//! seeing them before the edit loop drives them: `--edit-fixture menu`
//! (or `refused`, `fits`, `choose`, `greyed`). What the user does on them
//! is printed, nothing else; nothing here edits the design.
//!
//! The fixture's ids are illustrative indexes, so the words for them
//! (`door_bay[left]`, `drawer[top]`) are the fixture's own documented
//! names, and expressions are written back with [`expression_text`]
//! (the contract carries parsed expressions, not their text).

use gtk::prelude::*;
use stave_app::edit::{
    Applicability, Candidate, Fits, Fixture, PlaceSlot, SettingValue, Verdict,
    clothing_cabinet_fixture,
};
use stave_lang::ast::{
    DesignItem, Expr, ExprKind, Index, ItemKind, Magnitude, Number, PatchOpKind, StringPart,
};
use stave_lang::{Code, Diagnostic, Label, SourceMap};
use stave_model::ids::AssemblyId;
use stave_model::nonempty::NonEmpty;

use super::edit_card::{CardModel, EditCard, Words};
use super::edit_menu::{Checked, EditMenu, MenuItem};

/// Which state of the editing widgets to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Showcase {
    /// The menu for a click in `door_bay[left]`: Drawer fits,
    /// FramePanelDoor still being checked.
    Menu,
    /// The menu with FramePanelDoor greyed, with an illustrative reason.
    Greyed,
    /// The card for Drawer, refused: it runs into `door_l`, with the fix.
    Refused,
    /// The card for Drawer once it fits: Add enabled.
    Fits,
    /// The card for Drawer with a setting still to choose: Add disabled.
    Choose,
    /// The card for FramePanelDoor, its hinge side still to point at.
    Point,
}

impl Showcase {
    /// The showcase a word names.
    pub fn from_word(word: &str) -> Option<Showcase> {
        match word {
            "menu" => Some(Showcase::Menu),
            "greyed" => Some(Showcase::Greyed),
            "refused" => Some(Showcase::Refused),
            "fits" => Some(Showcase::Fits),
            "choose" => Some(Showcase::Choose),
            "point" => Some(Showcase::Point),
            _ => None,
        }
    }
}

/// The fixture's names for its illustrative ids.
pub(super) struct FixtureWords;

impl Words for FixtureWords {
    fn expression(&self, expression: &Expr) -> String {
        expression_text(expression)
    }

    fn assembly(&self, assembly: AssemblyId) -> String {
        match assembly.index() {
            1 => "drawer[top]".to_owned(),
            2 => "door_l".to_owned(),
            index => format!("assembly {index}"),
        }
    }

    fn place(&self, slot: &PlaceSlot) -> Option<String> {
        match slot {
            PlaceSlot::Region(Some(_)) => Some("door_bay[left]".to_owned()),
            PlaceSlot::Anchor(Some(_)) => Some("a face".to_owned()),
            PlaceSlot::Part(Some(_)) => Some("a part".to_owned()),
            PlaceSlot::Parts(Some(parts)) => Some(format!("{} parts", parts.iter().count())),
            PlaceSlot::Region(None)
            | PlaceSlot::Anchor(None)
            | PlaceSlot::Part(None)
            | PlaceSlot::Parts(None) => None,
        }
    }
}

/// The name the fixture's tried patch adds: `drawer_2`.
fn added_name(fixture: &Fixture) -> String {
    fixture
        .tried
        .ops
        .first()
        .and_then(|op| match &op.kind {
            PatchOpKind::Add { item, .. } => match &item.kind {
                ItemKind::Design(DesignItem::Assembly(assembly)) => {
                    Some(assembly.name.as_str().to_owned())
                }
                _ => None,
            },
            _ => None,
        })
        .unwrap_or_else(|| "drawer_2".to_owned())
}

/// `candidate` greyed, with an illustrative reason.
fn greyed(candidate: &Candidate) -> Option<Candidate> {
    let mut sources = SourceMap::new();
    let file = sources.add("illustration.stave", " ").ok()?;
    let reason = Diagnostic::error(
        Code::Overlap,
        "door_bay[left] already holds door_l",
        Label::new(file, stave_lang::ast::Span::default()),
    );
    let mut candidate = candidate.clone();
    candidate.applicability = Applicability::Greyed(NonEmpty::new(vec![reason])?);
    Some(candidate)
}

/// Puts the showcase over the design: the menu near the left door, the
/// card at the top right.
pub fn show(design: &gtk::Overlay, showcase: Showcase) {
    let Some(fixture) = clothing_cabinet_fixture() else {
        eprintln!("the editing fixture didn't build");
        return;
    };
    let heading = fixture.picked.heading();
    match showcase {
        Showcase::Menu | Showcase::Greyed => {
            let door = match showcase {
                Showcase::Greyed => fixture.candidates.get(1).and_then(greyed),
                Showcase::Menu
                | Showcase::Refused
                | Showcase::Fits
                | Showcase::Choose
                | Showcase::Point => None,
            };
            let mut items: Vec<MenuItem> = fixture
                .candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| MenuItem {
                    candidate,
                    checked: if index == 0 {
                        Checked::Done
                    } else {
                        Checked::Pending
                    },
                })
                .collect();
            if let (Some(door), Some(item)) = (&door, items.get_mut(1)) {
                *item = MenuItem {
                    candidate: door,
                    checked: Checked::Done,
                };
            }
            let menu = EditMenu::new();
            menu.show(heading, &items);
            menu.connect_chosen(|declaration| eprintln!("edit menu: chose {declaration:?}"));
            place(&menu.root, gtk::Align::Start, 470);
            design.add_overlay(&menu.root);
        }
        Showcase::Refused | Showcase::Fits | Showcase::Choose | Showcase::Point => {
            let Some(drawer) = fixture.candidates.first() else {
                return;
            };
            let name = added_name(&fixture);
            let fits = Fits::new(Vec::new()).map(Verdict::Fits).ok();
            let mut chosen = drawer.clone();
            if let Some(setting) = chosen
                .settings
                .iter_mut()
                .find(|setting| setting.name.as_str() == "front_stock")
            {
                setting.value = SettingValue::Unset;
            }
            let door = fixture.candidates.get(1).unwrap_or(drawer);
            let (candidate, verdict, name) = match showcase {
                Showcase::Refused => (drawer, Some(&fixture.verdict), name),
                Showcase::Fits => (drawer, fits.as_ref(), name),
                Showcase::Point => (door, None, "frame_panel_door".to_owned()),
                Showcase::Choose | Showcase::Menu | Showcase::Greyed => (&chosen, None, name),
            };
            let card = EditCard::new();
            card.show(
                &CardModel {
                    applied: &[],
                    blame: None,
                    heading,
                    candidate,
                    name: &name,
                    verdict,
                },
                &FixtureWords,
            );
            card.connect_action(|action| eprintln!("edit card: {action:?}"));
            place(&card.root, gtk::Align::End, 64);
            design.add_overlay(&card.root);
        }
    }
}

/// Floats a panel at the top of the view, `margin` from one side.
fn place(panel: &gtk::Box, side: gtk::Align, margin: i32) {
    panel.set_halign(side);
    panel.set_valign(gtk::Align::Start);
    panel.set_margin_top(16);
    panel.set_margin_bottom(90);
    match side {
        gtk::Align::End => panel.set_margin_end(margin),
        _ => panel.set_margin_start(margin),
    }
}

/// An expression written back as text, close to how `stave fmt` writes
/// it: `overlay(left: 18mm, meeting: -reveal / 2)`.
pub fn expression_text(expression: &Expr) -> String {
    match &expression.kind {
        ExprKind::Quantity(quantity) => {
            format!(
                "{}{}",
                magnitude_text(quantity.magnitude),
                quantity.unit.suffix()
            )
        }
        ExprKind::Number(Number::Integer(value)) => value.to_string(),
        ExprKind::Number(Number::Decimal(decimal)) => decimal_text(decimal.digits, decimal.scale),
        ExprKind::String(literal) => {
            let mut text = String::from("\"");
            for part in &literal.parts {
                match part {
                    StringPart::Text { text: words, .. } => text.push_str(words),
                    StringPart::Interp { root, members, .. } => {
                        text.push('{');
                        text.push_str(root.as_str());
                        for member in members {
                            text.push('.');
                            text.push_str(member.as_str());
                        }
                        text.push('}');
                    }
                }
            }
            text.push('"');
            text
        }
        ExprKind::Bool(value) => value.to_string(),
        ExprKind::Fit => "fit".to_owned(),
        ExprKind::Name(name) => name.as_str().to_owned(),
        ExprKind::Member { base, member } => {
            format!("{}.{}", expression_text(base), member.as_str())
        }
        ExprKind::Index { base, index } => {
            let inside = match index {
                Index::All(_) => "*".to_owned(),
                Index::Same(_) => "same".to_owned(),
                Index::At(at) => expression_text(at),
            };
            format!("{}[{inside}]", expression_text(base))
        }
        ExprKind::Call { callee, args } => {
            let args: Vec<String> = args
                .iter()
                .map(|arg| match &arg.name {
                    Some(name) => format!("{}: {}", name.as_str(), expression_text(&arg.value)),
                    None => expression_text(&arg.value),
                })
                .collect();
            format!("{}({})", expression_text(callee), args.join(", "))
        }
        ExprKind::Unary { op, operand } => format!("{}{}", op.symbol(), expression_text(operand)),
        ExprKind::Binary { op, lhs, rhs } => format!(
            "{} {} {}",
            expression_text(lhs),
            op.symbol(),
            expression_text(rhs)
        ),
        ExprKind::List(items) => format!(
            "[{}]",
            items
                .iter()
                .map(expression_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ExprKind::Paren(inner) => format!("({})", expression_text(inner)),
        ExprKind::Range { lo, hi } => format!("{}..{}", expression_text(lo), expression_text(hi)),
    }
}

fn magnitude_text(magnitude: Magnitude) -> String {
    match magnitude {
        Magnitude::Integer(value) => value.to_string(),
        Magnitude::Decimal(decimal) => decimal_text(decimal.digits, decimal.scale),
        Magnitude::Fraction(fraction) if fraction.whole == 0 => {
            format!("{}/{}", fraction.numerator, fraction.denominator)
        }
        Magnitude::Fraction(fraction) => format!(
            "{}-{}/{}",
            fraction.whole, fraction.numerator, fraction.denominator
        ),
    }
}

fn decimal_text(digits: u64, scale: u8) -> String {
    let text = format!("{digits:0>width$}", width = usize::from(scale) + 1);
    let split = text.len().saturating_sub(usize::from(scale));
    match (text.get(..split), text.get(split..)) {
        (Some(whole), Some(fraction)) if !fraction.is_empty() => format!("{whole}.{fraction}"),
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions_read_back_as_written() {
        let fixture = clothing_cabinet_fixture().unwrap();
        let drawer = &fixture.candidates[0];
        let text = |name: &str| {
            let setting = drawer
                .settings
                .iter()
                .find(|setting| setting.name.as_str() == name)
                .unwrap();
            let SettingValue::Copied { value, .. } = &setting.value else {
                panic!("copied");
            };
            expression_text(value)
        };
        assert_eq!(text("front_stock"), "oak_19");
        assert_eq!(
            text("mount"),
            "overlay(left: 18mm, right: 18mm, top: 8mm, bottom: 8mm, meeting: -reveal / 2)"
        );
        assert_eq!(text("slide"), "side_mount_slide(length: slide_length)");
        assert_eq!(decimal_text(1825, 2), "18.25");
        assert_eq!(decimal_text(5, 1), "0.5");
        assert_eq!(added_name(&fixture), "drawer_2");
    }

    #[test]
    fn showcases_are_named_by_word() {
        for (word, showcase) in [
            ("menu", Showcase::Menu),
            ("greyed", Showcase::Greyed),
            ("refused", Showcase::Refused),
            ("fits", Showcase::Fits),
            ("choose", Showcase::Choose),
            ("point", Showcase::Point),
        ] {
            assert_eq!(Showcase::from_word(word), Some(showcase));
        }
        assert_eq!(Showcase::from_word("other"), None);
    }
}

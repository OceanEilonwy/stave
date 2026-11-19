//! What a setting's control offers (`docs/editing.md`, "The short list and
//! the search"): a short list (the default, the values the project already
//! uses for settings like it, and the value it holds now), and a search
//! over every option, grouped as the menu groups its entries and ranked by
//! how well each matches. The window draws these and reports what was
//! chosen; the command line prints them (`choices`, `search`).
//!
//! ```text
//! SettingParam ──short_list──▶ [ShortChoice]  default · recent… · current
//!              └─search(query)─▶ [SearchGroup] project · modules · std · values
//! ```
//!
//! "Recently used" is read from the project's own text when the candidate
//! is described ([`crate::edit::SettingParam::recent`]); nothing is kept
//! outside the `.stave` files.

use std::cmp::Ordering;

use stave_lang::ast::TypeName;

use crate::candidates::compare_groups;
use crate::edit::{MenuGroup, OptionFit, SettingOption, SettingParam, SettingValue};

/// The most recently used values a short list offers.
pub const RECENT_LIMIT: usize = 4;

/// `true` when values are recent for a setting only if a param of the same
/// name holds them: a quantity, whose type (a length, an angle) every size
/// of every kind shares. A type with its own list of options (a stock, a
/// mount, a kind, a `Bool`) matches by type alone.
pub fn matched_by_name(ty: TypeName) -> bool {
    match ty {
        TypeName::Stock
        | TypeName::Mount
        | TypeName::JointKind
        | TypeName::HardwareKind
        | TypeName::SlideKind
        | TypeName::HingeKind
        | TypeName::PullKind
        | TypeName::FastenerKind
        | TypeName::Tool
        | TypeName::Bool
        | TypeName::Profile => false,
        TypeName::Length
        | TypeName::Angle
        | TypeName::Ratio
        | TypeName::Count
        | TypeName::Text
        | TypeName::Part
        | TypeName::Parts
        | TypeName::Region
        | TypeName::Anchor
        | TypeName::Axis
        | TypeName::Direction
        | TypeName::Shape
        | TypeName::Boards
        | TypeName::Splices
        | TypeName::Envelope
        | TypeName::Model
        | TypeName::Spacing
        | TypeName::Patch
        | TypeName::Recipe => true,
    }
}

/// Why an entry is on a setting's short list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceRole {
    /// The param's own default, as its declaration writes it.
    Default,
    /// A value the project already gives a setting like it.
    Recent,
    /// The value the setting holds now, when neither of the above.
    Current,
}

impl ChoiceRole {
    /// The word the list marks it with: "default", "used", "now".
    pub fn word(self) -> &'static str {
        match self {
            ChoiceRole::Default => "default",
            ChoiceRole::Recent => "used",
            ChoiceRole::Current => "now",
        }
    }
}

/// One entry of a setting's short list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortChoice {
    /// The value as `.stave` text: `dowel(count: 4)`, `screw`, `oak_19`.
    pub text: String,
    /// Why it is listed.
    pub role: ChoiceRole,
    /// `true` for the value the setting holds now (the default while it is
    /// left at its default).
    pub current: bool,
    /// Whether the candidate fits with it, when the text is one of the
    /// setting's options; `None` for a value that is no option as written
    /// (a default with arguments, `dowel(count: 4)`).
    pub fit: Option<OptionFit>,
    /// Its declaration's first sentence, when it is an option that has one.
    pub hint: Option<String>,
}

impl ShortChoice {
    /// `false` when it can't be chosen from the list: an option the
    /// checker refused. The default can always be gone back to.
    pub fn choosable(&self) -> bool {
        self.role == ChoiceRole::Default || !self.fit.as_ref().is_some_and(OptionFit::is_refused)
    }
}

/// A setting's short list, in order: its default (when its param has
/// one), then the values the project already uses for settings like it,
/// the most recent first ([`SettingParam::recent`]), then the value it
/// holds now when neither lists it. Each text once.
pub fn short_list(setting: &SettingParam) -> Vec<ShortChoice> {
    let now = match &setting.value {
        SettingValue::Default => None,
        SettingValue::Copied { .. } | SettingValue::Chosen(_) => setting.text.as_deref(),
        SettingValue::Unset => None,
    };
    let at_default = setting.value == SettingValue::Default;
    let mut out: Vec<ShortChoice> = Vec::new();
    let mut push = |text: &str, role: ChoiceRole, current: bool| {
        if out.iter().any(|choice| choice.text == text) {
            return;
        }
        let option = setting.options.iter().find(|option| option.text == text);
        out.push(ShortChoice {
            text: text.to_owned(),
            role,
            current,
            fit: option.map(|option| option.fit.clone()),
            hint: option.and_then(|option| option.hint.clone()),
        });
    };
    if let Some(default) = &setting.default_text {
        push(default, ChoiceRole::Default, at_default);
    }
    for recent in &setting.recent {
        push(recent, ChoiceRole::Recent, now == Some(recent.as_str()));
    }
    if let Some(now) = now {
        push(now, ChoiceRole::Current, true);
    }
    out
}

/// How an option matched a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Match {
    /// Its name is the words searched for.
    Exact,
    /// Its name starts with them.
    Prefix,
    /// Its name holds them.
    InName,
    /// Its description (the first sentence of its doc comment) holds them.
    InDescription,
    /// Nothing was typed: every option is listed.
    All,
}

impl Match {
    /// How the match is said: "name", "description".
    pub fn word(self) -> &'static str {
        match self {
            Match::Exact | Match::Prefix | Match::InName => "name",
            Match::InDescription => "description",
            Match::All => "all",
        }
    }
}

/// One option a search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// The option, with its fit as the session has it now.
    pub option: SettingOption,
    /// How it matched.
    pub matched: Match,
}

impl SearchHit {
    /// `false` for an option the checker refused: it can't be chosen.
    pub fn choosable(&self) -> bool {
        !self.option.fit.is_refused()
    }
}

/// The hits from one module, in rank order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchGroup {
    /// The module, `None` for values that are no declaration.
    pub group: Option<MenuGroup>,
    /// The hits.
    pub hits: Vec<SearchHit>,
}

impl SearchGroup {
    /// The group's title, as the menu titles its groups: "In this
    /// project", `casework`, "Standard · joinery", or "Values".
    pub fn title(&self) -> String {
        match &self.group {
            Some(group) => group_title(group),
            None => "Values".to_owned(),
        }
    }
}

/// A menu group's title: "In this project", `casework` (an imported
/// module, by the name it is imported under), "Standard · joinery".
pub fn group_title(group: &MenuGroup) -> String {
    match group {
        MenuGroup::Project => "In this project".to_owned(),
        MenuGroup::Module(name) => name.as_str().to_owned(),
        MenuGroup::Standard(name) => format!("Standard · {}", name.as_str()),
    }
}

/// How well an option matches a query: every word of the query must be in
/// its name or its description (case does not matter), and the match is
/// the weakest of the words'. `None` when it doesn't match.
pub fn match_option(option: &SettingOption, query: &str) -> Option<Match> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Some(Match::All);
    }
    // The name without the import's prefix: `casework.oak_19` is found by
    // `oak`.
    let name = option.text.to_lowercase();
    let short = name.rsplit('.').next().unwrap_or(&name).to_owned();
    let hint = option.hint.as_deref().unwrap_or_default().to_lowercase();
    let whole = words.join(" ");
    if short == whole || name == whole {
        return Some(Match::Exact);
    }
    let mut weakest = Match::Exact;
    for word in &words {
        let found = if short.starts_with(word.as_str()) || name.starts_with(word.as_str()) {
            Match::Prefix
        } else if name.contains(word.as_str()) {
            Match::InName
        } else if hint.contains(word.as_str()) {
            Match::InDescription
        } else {
            return None;
        };
        weakest = weakest.max(found);
    }
    Some(weakest)
}

/// Every option of a setting a query matches ([`match_option`]), grouped
/// by the module it comes from in menu order (the project's own, imported
/// modules by name, `std`, then values that are no declaration), and in
/// each group the best matches first, ties in the order the session lists
/// the options (those that fit before those refused). An empty query
/// lists every option.
pub fn search(setting: &SettingParam, query: &str) -> Vec<SearchGroup> {
    let mut hits: Vec<(usize, SearchHit)> = setting
        .options
        .iter()
        .enumerate()
        .filter_map(|(order, option)| {
            match_option(option, query).map(|matched| {
                (
                    order,
                    SearchHit {
                        option: option.clone(),
                        matched,
                    },
                )
            })
        })
        .collect();
    hits.sort_by(|(left_order, left), (right_order, right)| {
        compare_option_groups(left.option.group.as_ref(), right.option.group.as_ref())
            .then(left.matched.cmp(&right.matched))
            .then(left_order.cmp(right_order))
    });
    let mut groups: Vec<SearchGroup> = Vec::new();
    for (_, hit) in hits {
        match groups.last_mut() {
            Some(last) if last.group == hit.option.group => last.hits.push(hit),
            Some(_) | None => groups.push(SearchGroup {
                group: hit.option.group.clone(),
                hits: vec![hit],
            }),
        }
    }
    groups
}

/// Menu order, with values that are no declaration last.
fn compare_option_groups(left: Option<&MenuGroup>, right: Option<&MenuGroup>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => compare_groups(left, right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use stave_lang::ast::{Ident, Name, Span};

    use super::*;

    fn ident(text: &str) -> Ident {
        Ident::new(text, Span::default()).unwrap()
    }

    fn option(text: &str, hint: &str, group: MenuGroup) -> SettingOption {
        let mut option = SettingOption::new(text.to_owned(), Some(hint.to_owned()));
        option.group = Some(group);
        option
    }

    fn joinery() -> SettingParam {
        let joinery = MenuGroup::Standard(Name::new("joinery", Span::default()).unwrap());
        let mut setting =
            SettingParam::new(ident("joinery"), TypeName::JointKind, SettingValue::Default);
        setting.default_text = Some("dowel(count: 4)".to_owned());
        setting.text = setting.default_text.clone();
        setting.options = vec![
            option("dado", "A dado: a groove across.", joinery.clone()),
            option("dowel", "A dowel joint: matching holes.", joinery.clone()),
            option(
                "chigiri",
                "Chigiri: a butterfly key let into the face across a seam.",
                joinery.clone(),
            ),
            option("my_joint", "Our own joint.", MenuGroup::Project),
        ];
        setting
    }

    #[test]
    fn the_short_list_is_the_default_then_recent_then_now() {
        let mut setting = joinery();
        let list = short_list(&setting);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].text, "dowel(count: 4)");
        assert_eq!(list[0].role, ChoiceRole::Default);
        assert!(list[0].current);
        assert_eq!(list[0].fit, None);

        setting.recent = vec!["screw".to_owned(), "dado".to_owned()];
        setting.value = SettingValue::Chosen(crate::editing::parse_value("chigiri").unwrap());
        setting.text = Some("chigiri".to_owned());
        let list = short_list(&setting);
        let texts: Vec<(&str, ChoiceRole, bool)> = list
            .iter()
            .map(|choice| (choice.text.as_str(), choice.role, choice.current))
            .collect();
        assert_eq!(
            texts,
            [
                ("dowel(count: 4)", ChoiceRole::Default, false),
                ("screw", ChoiceRole::Recent, false),
                ("dado", ChoiceRole::Recent, false),
                ("chigiri", ChoiceRole::Current, true),
            ]
        );
        // A recent value that is held now is not listed twice.
        setting.value = SettingValue::Chosen(crate::editing::parse_value("dado").unwrap());
        setting.text = Some("dado".to_owned());
        let list = short_list(&setting);
        assert_eq!(list.len(), 3);
        assert!(list[2].current && list[2].role == ChoiceRole::Recent);
        assert_eq!(list[2].hint.as_deref(), Some("A dado: a groove across."));
    }

    #[test]
    fn a_refused_recent_value_cant_be_chosen_but_the_default_can() {
        let mut setting = joinery();
        setting.recent = vec!["chigiri".to_owned()];
        let refusal = crate::edit::OptionRefusal {
            first: stave_lang::Diagnostic {
                severity: stave_lang::Severity::Error,
                code: stave_lang::Code::Require,
                message: "not for a corner".to_owned(),
                labels: Vec::new(),
                notes: Vec::new(),
                merged: Vec::new(),
                help: None,
                fixes: Vec::new(),
                cause: None,
            },
            more: 0,
        };
        for option in &mut setting.options {
            if option.text == "chigiri" {
                option.fit = OptionFit::Refused(refusal.clone());
            }
        }
        let list = short_list(&setting);
        assert!(list[0].choosable());
        assert!(!list[1].choosable());
    }

    #[test]
    fn a_search_finds_by_name_and_by_description() {
        let setting = joinery();
        let found = search(&setting, "dow");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].hits[0].option.text, "dowel");
        assert_eq!(found[0].hits[0].matched, Match::Prefix);

        let found = search(&setting, "KEY");
        assert_eq!(found[0].hits.len(), 1);
        assert_eq!(found[0].hits[0].option.text, "chigiri");
        assert_eq!(found[0].hits[0].matched, Match::InDescription);

        assert!(search(&setting, "nothing like it").is_empty());
        // Every word must match.
        assert_eq!(search(&setting, "butterfly seam")[0].hits.len(), 1);
    }

    #[test]
    fn a_search_groups_as_the_menu_and_ranks_the_name_first() {
        let setting = joinery();
        let all = search(&setting, "");
        let titles: Vec<String> = all.iter().map(SearchGroup::title).collect();
        assert_eq!(titles, ["In this project", "Standard · joinery"]);
        assert_eq!(all[1].hits.len(), 3);
        // "do" starts dowel's name and is inside dado's: dowel first,
        // though the session lists dado before it.
        let found = search(&setting, "do");
        let texts: Vec<&str> = found[0]
            .hits
            .iter()
            .map(|hit| hit.option.text.as_str())
            .collect();
        assert_eq!(texts, ["dowel", "dado"]);
        assert_eq!(found[0].hits[1].matched, Match::InName);
    }

    #[test]
    fn quantities_are_recent_by_name_and_listed_kinds_by_type() {
        assert!(!matched_by_name(TypeName::JointKind));
        assert!(!matched_by_name(TypeName::Stock));
        assert!(matched_by_name(TypeName::Length));
    }
}

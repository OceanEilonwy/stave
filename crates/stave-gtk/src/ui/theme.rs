//! The one table of code colours: how the editor (`editor.rs`), the
//! inline peek and search results (`files.rs`) and the inspector's and
//! problems' snippets (`inspector.rs`, through `style::CODE`) draw each
//! kind of word. Each [`Role`] has a colour for the light style, one for
//! the dark style and one for the snippets' code blocks (dark in every
//! style), a slant and a weight; a name that means an item declared
//! elsewhere is also underlined ([`REFERENCE_UNDERLINE`]).
//!
//! The colours are GNOME's palette, chosen so each reads at 4.5:1 or
//! better on its background and the words a beginner meets most (parts,
//! joints, params, hardware, regions, units) are told apart by colour
//! (tested below).

use stave_ide::{SymbolKind, TokenClass, TokenModifier, TokenModifiers};

/// What a run of code is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// `assembly`, `import`, `within`.
    Keyword,
    /// `// …`.
    Comment,
    /// `/// …`.
    DocComment,
    /// A string.
    Text,
    /// A number, or a quantity's number: the `18` of `18mm`.
    Number,
    /// A quantity's unit: the `mm` of `18mm`.
    Unit,
    /// A built-in call or a call head fixed by its position: `overlay`,
    /// `flush`.
    Function,
    /// A component, a kind or a type name.
    Type,
    /// A module's name.
    Module,
    /// A word of a closed list: `up`, `plywood`.
    EnumWord,
    /// A field, a named argument or a member after `.`.
    Field,
    /// A part, or a part a kind makes.
    Part,
    /// An assembly.
    Assembly,
    /// A joint, or a role of a kind (the pieces it joins).
    Joint,
    /// Hardware, or a member of a hardware kind.
    Hardware,
    /// A param or a let.
    Param,
    /// A region.
    Region,
    /// A stock.
    Stock,
    /// A recipe, a profile or a scene.
    Recipe,
    /// A require.
    Require,
    /// A label of an array.
    Label,
    /// An error: its underline and its `error[code]`.
    Error,
    /// A warning: its underline and its `warning[code]`.
    Warning,
}

/// Upright or italic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slant {
    /// Upright.
    Upright,
    /// Italic.
    Italic,
}

/// Regular or bold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    /// Regular.
    Regular,
    /// Semibold (600).
    Bold,
}

impl Weight {
    /// The Pango weight.
    pub fn pango(self) -> i32 {
        match self {
            Weight::Regular => 400,
            Weight::Bold => 600,
        }
    }
}

/// How a [`Role`] is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    /// `#rrggbb` on the light style's editor.
    pub light: &'static str,
    /// `#rrggbb` on the dark style's editor.
    pub dark: &'static str,
    /// `#rrggbb` in a snippet's code block, dark in either style.
    pub on_code: &'static str,
    /// Upright or italic.
    pub slant: Slant,
    /// Regular or bold.
    pub weight: Weight,
}

impl Look {
    /// Its colour in the editor in the light or dark style.
    pub fn colour(self, dark: bool) -> &'static str {
        if dark { self.dark } else { self.light }
    }
}

/// A plain upright, regular look.
const fn look(light: &'static str, dark: &'static str, on_code: &'static str) -> Look {
    Look {
        light,
        dark,
        on_code,
        slant: Slant::Upright,
        weight: Weight::Regular,
    }
}

/// How a reference (a name that means an item declared elsewhere) is
/// marked on top of its colour.
pub const REFERENCE_UNDERLINE: gtk::pango::Underline = gtk::pango::Underline::Single;

impl Role {
    /// Every role (the tests walk it).
    #[cfg(test)]
    pub const ALL: [Role; 23] = [
        Role::Keyword,
        Role::Comment,
        Role::DocComment,
        Role::Text,
        Role::Number,
        Role::Unit,
        Role::Function,
        Role::Type,
        Role::Module,
        Role::EnumWord,
        Role::Field,
        Role::Part,
        Role::Assembly,
        Role::Joint,
        Role::Hardware,
        Role::Param,
        Role::Region,
        Role::Stock,
        Role::Recipe,
        Role::Require,
        Role::Label,
        Role::Error,
        Role::Warning,
    ];

    /// The roles the editor colours a word with (all but the problems,
    /// which draw underlines of their own).
    pub const SYNTAX: [Role; 21] = [
        Role::Keyword,
        Role::Comment,
        Role::DocComment,
        Role::Text,
        Role::Number,
        Role::Unit,
        Role::Function,
        Role::Type,
        Role::Module,
        Role::EnumWord,
        Role::Field,
        Role::Part,
        Role::Assembly,
        Role::Joint,
        Role::Hardware,
        Role::Param,
        Role::Region,
        Role::Stock,
        Role::Recipe,
        Role::Require,
        Role::Label,
    ];

    /// How it is drawn: the theme table.
    pub const fn look(self) -> Look {
        match self {
            Role::Keyword => look("#9141ac", "#dc8add", "#f0a3c4"),
            Role::Comment | Role::DocComment => Look {
                slant: Slant::Italic,
                ..look("#6f6e73", "#9a9996", "#8b8a93")
            },
            Role::Text => look("#1b7a4c", "#8ff0a4", "#8ff0a4"),
            Role::Number => look("#c64600", "#ffa348", "#ffcf80"),
            Role::Unit => Look {
                slant: Slant::Italic,
                ..look("#0b6e75", "#5fd4d8", "#5fd4d8")
            },
            Role::Function | Role::EnumWord | Role::Recipe => look("#1c71d8", "#78aeed", "#8fd0ff"),
            Role::Type => Look {
                weight: Weight::Bold,
                ..look("#1a5fb4", "#99c1f1", "#8fd0ff")
            },
            Role::Module => look("#3d3846", "#deddda", "#e7e5ea"),
            Role::Field => look("#5e5c64", "#c0bfbc", "#c0bfbc"),
            Role::Part => look("#865e3c", "#deb88f", "#deb88f"),
            Role::Assembly => Look {
                weight: Weight::Bold,
                ..look("#865e3c", "#deb88f", "#deb88f")
            },
            Role::Joint => look("#5a5bb5", "#a6a8f0", "#a6a8f0"),
            Role::Hardware => look("#a3306f", "#f19ac6", "#f19ac6"),
            Role::Param => look("#7a5c00", "#f5c211", "#f5c211"),
            Role::Region => look("#6c4f7c", "#c8a8d8", "#c8a8d8"),
            Role::Stock => look("#4f6a00", "#c3e07a", "#c3e07a"),
            Role::Require => look("#a1453e", "#f0928a", "#f0928a"),
            Role::Label => look("#4a6a7a", "#a0c4d4", "#a0c4d4"),
            Role::Error => look("#c01c28", "#ff7b63", "#ff7b85"),
            Role::Warning => look("#9c6e03", "#f8e45c", "#ffcf80"),
        }
    }

    /// The tag name the editor's buffers know it by.
    pub fn tag_name(self) -> &'static str {
        match self {
            Role::Keyword => "keyword",
            Role::Comment => "comment",
            Role::DocComment => "doc",
            Role::Text => "text",
            Role::Number => "number",
            Role::Unit => "unit",
            Role::Function => "function",
            Role::Type => "type",
            Role::Module => "module",
            Role::EnumWord => "word",
            Role::Field => "field",
            Role::Part => "part",
            Role::Assembly => "assembly",
            Role::Joint => "joint",
            Role::Hardware => "hardware",
            Role::Param => "param",
            Role::Region => "region",
            Role::Stock => "stock",
            Role::Recipe => "recipe",
            Role::Require => "require",
            Role::Label => "label",
            Role::Error => "error",
            Role::Warning => "warning",
        }
    }

    /// The role of a word the navigation layer classed, or `None` for
    /// what is drawn plain. A character the lexer couldn't read is an
    /// [`Role::Error`].
    pub fn of_token(class: TokenClass) -> Option<Role> {
        Some(match class {
            TokenClass::Keyword => Role::Keyword,
            TokenClass::Comment => Role::Comment,
            TokenClass::DocComment => Role::DocComment,
            TokenClass::Text => Role::Text,
            TokenClass::Number => Role::Number,
            TokenClass::Unit => Role::Unit,
            TokenClass::Function => Role::Function,
            TokenClass::Type => Role::Type,
            TokenClass::Module => Role::Module,
            TokenClass::EnumWord => Role::EnumWord,
            TokenClass::Field => Role::Field,
            TokenClass::Error => Role::Error,
            TokenClass::Item(kind) => Role::of_item(kind),
            TokenClass::Punctuation | TokenClass::Plain => return None,
        })
    }

    /// The role of a name that means an item of this kind.
    fn of_item(kind: SymbolKind) -> Role {
        match kind {
            SymbolKind::Module => Role::Module,
            SymbolKind::Component | SymbolKind::Kind => Role::Type,
            SymbolKind::Part | SymbolKind::MadePart => Role::Part,
            SymbolKind::Assembly => Role::Assembly,
            SymbolKind::Joint | SymbolKind::Role => Role::Joint,
            SymbolKind::Hardware | SymbolKind::Member => Role::Hardware,
            SymbolKind::Param | SymbolKind::Let => Role::Param,
            SymbolKind::Region => Role::Region,
            SymbolKind::Stock => Role::Stock,
            SymbolKind::Recipe | SymbolKind::Profile | SymbolKind::Scene => Role::Recipe,
            SymbolKind::Require => Role::Require,
            SymbolKind::Label => Role::Label,
        }
    }

    /// The role of a word of a snippet `stave-app` highlighted.
    pub fn of_snippet(class: stave_app::inspect::TokenClass) -> Option<Role> {
        use stave_app::inspect::TokenClass as Snippet;
        Some(match class {
            Snippet::Keyword => Role::Keyword,
            Snippet::Type | Snippet::Function => Role::Type,
            Snippet::Number => Role::Number,
            Snippet::Text => Role::Text,
            Snippet::Comment => Role::Comment,
            Snippet::Plain => return None,
        })
    }
}

/// `true` if a token with these modifiers is underlined: it names an
/// item declared elsewhere.
pub fn is_underlined(modifiers: TokenModifiers) -> bool {
    modifiers.contains(TokenModifier::Reference)
}

/// A run of code as Pango markup in the editor's light or dark colours:
/// `piece` must already be escaped.
pub fn markup(piece: &str, class: TokenClass, modifiers: TokenModifiers, dark: bool) -> String {
    let underline = if is_underlined(modifiers) {
        " underline=\"single\""
    } else {
        ""
    };
    let Some(look) = Role::of_token(class).map(Role::look) else {
        return if underline.is_empty() {
            piece.to_owned()
        } else {
            format!("<span{underline}>{piece}</span>")
        };
    };
    let slant = match look.slant {
        Slant::Upright => "",
        Slant::Italic => " style=\"italic\"",
    };
    let weight = match look.weight {
        Weight::Regular => "",
        Weight::Bold => " weight=\"600\"",
    };
    format!(
        "<span foreground=\"{}\"{slant}{weight}{underline}>{piece}</span>",
        look.colour(dark)
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{Role, Slant, markup};
    use stave_ide::{TokenClass, TokenModifier, TokenModifiers};

    /// The relative luminance of `#rrggbb` (WCAG 2).
    fn luminance(hex: &str) -> f64 {
        let channel = |range: std::ops::Range<usize>| {
            let value = f64::from(u8::from_str_radix(&hex[range], 16).unwrap()) / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(1..3) + 0.7152 * channel(3..5) + 0.0722 * channel(5..7)
    }

    /// The WCAG 2 contrast ratio of two `#rrggbb` colours.
    pub(crate) fn contrast(a: &str, b: &str) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// The editor's backgrounds (the `stave-light` and `stave-dark`
    /// style schemes) and the snippets' code blocks.
    const LIGHT: &str = "#ffffff";
    const DARK: &str = "#1d1d20";
    const CODE: [&str; 2] = crate::ui::style::CODE_BACKGROUNDS;

    #[test]
    fn every_colour_reads_on_its_background() {
        for role in Role::ALL {
            let look = role.look();
            for (colour, background) in [
                (look.light, LIGHT),
                (look.dark, DARK),
                (look.on_code, CODE[0]),
                (look.on_code, CODE[1]),
            ] {
                let ratio = contrast(colour, background);
                assert!(
                    ratio >= 4.5,
                    "{role:?}: {colour} on {background}: {ratio:.2}:1"
                );
            }
        }
    }

    /// The words a beginner meets most each have their own colour, in
    /// both styles.
    #[test]
    fn the_common_words_are_told_apart() {
        let distinct = [
            Role::Keyword,
            Role::Text,
            Role::Number,
            Role::Unit,
            Role::Function,
            Role::Type,
            Role::Module,
            Role::Field,
            Role::Part,
            Role::Joint,
            Role::Hardware,
            Role::Param,
            Role::Region,
            Role::Stock,
            Role::Require,
            Role::Label,
        ];
        for dark in [false, true] {
            for (index, a) in distinct.iter().enumerate() {
                for b in &distinct[index + 1..] {
                    assert_ne!(
                        a.look().colour(dark),
                        b.look().colour(dark),
                        "{a:?} and {b:?} (dark: {dark})"
                    );
                }
            }
        }
        assert_eq!(Role::Unit.look().slant, Slant::Italic);
    }

    #[test]
    fn tag_names_are_distinct() {
        let mut names: Vec<&str> = Role::ALL.iter().map(|role| role.tag_name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Role::ALL.len());
        for role in Role::SYNTAX {
            assert!(!matches!(role, Role::Error | Role::Warning));
        }
    }

    #[test]
    fn markup_underlines_references_and_slants_units() {
        let reference = TokenModifiers::NONE.with(TokenModifier::Reference);
        assert_eq!(
            markup(
                "side_l",
                TokenClass::Item(stave_ide::SymbolKind::Part),
                reference,
                false
            ),
            "<span foreground=\"#865e3c\" underline=\"single\">side_l</span>"
        );
        assert_eq!(
            markup("mm", TokenClass::Unit, TokenModifiers::NONE, true),
            "<span foreground=\"#5fd4d8\" style=\"italic\">mm</span>"
        );
        assert_eq!(
            markup(",", TokenClass::Punctuation, TokenModifiers::NONE, true),
            ","
        );
    }
}

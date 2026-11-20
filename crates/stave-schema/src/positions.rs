//! Questions asked by position, for editors: "which fields does this
//! block take" ([`Block::fields`]) and "what does this argument expect"
//! ([`Callee::expects`]). The answers come from the tables the checker
//! reads ([`crate::fields`], [`crate::kinds`], [`crate::operations`],
//! [`crate::placement`], [`crate::functions`]); nothing here is a second
//! copy of them.

use stave_lang::ast::{BaseKind, CutOperation};

use crate::fields::{self, FieldSpec, ValueType};
use crate::functions::Builtin;
use crate::kinds;
use crate::operations::{self, ArgumentDefault, Selector};
use crate::placement::PlacementTerm;

/// A block whose entries are fields from a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Block {
    /// `project "Title" { }`
    Project,
    /// `library "Title" { }`
    Library,
    /// `stock name { }`
    Stock,
    /// `part name : stock { }` or `makes part name : stock { }`.
    Part,
    /// `profile name { }`
    Profile,
    /// `scene name "Title" { }`
    Scene,
    /// `kind name : <base> { }`
    Kind(BaseKind),
    /// `recipe name : operation { }`
    Recipe,
}

impl Block {
    /// The fields the block takes, in the order its table lists them.
    /// A joint kind takes its own entries; a hardware, fastener or tool
    /// kind the entries its base fixes.
    pub fn fields(self) -> Vec<&'static FieldSpec> {
        match self {
            Block::Project => fields::PROJECT.iter().collect(),
            Block::Library => fields::LIBRARY.iter().collect(),
            Block::Stock => fields::STOCK.iter().collect(),
            Block::Part => fields::PART.iter().collect(),
            Block::Profile => fields::PROFILE.iter().collect(),
            Block::Scene => fields::SCENE.iter().collect(),
            Block::Recipe => fields::RECIPE_FIELDS.iter().collect(),
            Block::Kind(base) => {
                let spec = kinds::base(base);
                let own: &[FieldSpec] = if base == BaseKind::Joint {
                    fields::JOINT_KIND
                } else {
                    &[]
                };
                own.iter()
                    .chain(spec.required)
                    .chain(spec.optional)
                    .collect()
            }
        }
    }

    /// The field of the block with this name.
    pub fn field(self, name: &str) -> Option<&'static FieldSpec> {
        self.fields().into_iter().find(|spec| spec.name == name)
    }
}

/// A call whose arguments are fixed by the language: a placement term, a
/// built-in function, a face selector after `cut`, or a cut operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Callee {
    /// `flush(face, inset: d)` in `place { }`.
    Placement(PlacementTerm),
    /// `mid(item, axis)`.
    Builtin(Builtin),
    /// `crossing(p, q, normal: thickness)` after `cut`.
    Selector(Selector),
    /// `drill(diameter:, depth:)` inside a `cut` block.
    Cut(CutOperation),
}

/// One argument of a [`Callee`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Argument {
    /// Its name; a positional argument has one too, for messages and
    /// signature help.
    pub name: &'static str,
    /// `true` if written without its name, in order.
    pub positional: bool,
    /// What it expects.
    pub ty: ValueType,
    /// What it is when left out.
    pub default: ArgumentDefault,
}

/// Where an argument stands in a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArgumentPosition<'a> {
    /// The argument written without a name at this place, from 0.
    Positional(usize),
    /// The argument written with this name.
    Named(&'a str),
}

/// The names of a selector's positional parts, for signature help: the
/// part cut, and the part entering it for `crossing`.
const SELECTOR_PARTS: [&str; 2] = ["part", "other"];

/// Where a call is written, which decides what its head word may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallSite {
    /// A term of `place { }`: a placement term or a built-in function.
    Place,
    /// On a `cut` line or in its block: a face selector, a cut operation
    /// or a built-in function.
    Cut,
    /// Anywhere else: a built-in function.
    Expression,
}

impl Callee {
    /// The callee a word names where it is written: a placement term
    /// inside `place { }`, a selector or cut operation inside a cut, else
    /// a built-in function.
    pub fn from_word(word: &str, site: CallSite) -> Option<Callee> {
        match site {
            CallSite::Place => {
                if let Some(term) = PlacementTerm::from_word(word) {
                    return Some(Callee::Placement(term));
                }
            }
            CallSite::Cut => {
                if let Some(selector) = Selector::from_word(word) {
                    return Some(Callee::Selector(selector));
                }
                if let Some(operation) = CutOperation::from_word(word) {
                    return Some(Callee::Cut(operation));
                }
            }
            CallSite::Expression => {}
        }
        Builtin::from_word(word).map(Callee::Builtin)
    }

    /// The word the call is written with.
    pub fn word(self) -> &'static str {
        match self {
            Callee::Placement(term) => term.word(),
            Callee::Builtin(builtin) => builtin.word(),
            Callee::Selector(selector) => selector.word(),
            Callee::Cut(operation) => operation.word(),
        }
    }

    /// The arguments, positional ones first, in order.
    pub fn arguments(self) -> Vec<Argument> {
        fn from_parameter(parameter: &crate::functions::Parameter) -> Argument {
            Argument {
                name: parameter.name,
                positional: parameter.positional,
                ty: parameter.ty,
                default: if parameter.required {
                    ArgumentDefault::Required
                } else {
                    ArgumentDefault::Absent
                },
            }
        }
        fn from_spec(spec: &operations::ArgumentSpec) -> Argument {
            Argument {
                name: spec.name,
                positional: false,
                ty: spec.ty,
                default: spec.default,
            }
        }
        match self {
            Callee::Placement(term) => term.arguments().iter().map(from_parameter).collect(),
            Callee::Builtin(builtin) => builtin.parameters().iter().map(from_parameter).collect(),
            Callee::Selector(selector) => SELECTOR_PARTS
                .iter()
                .take(selector.positional_parts())
                .map(|name| Argument {
                    name,
                    positional: true,
                    ty: ValueType::Type(stave_lang::ast::TypeName::Part),
                    default: ArgumentDefault::Required,
                })
                .chain(selector.arguments().iter().map(from_spec))
                .collect(),
            Callee::Cut(operation) => operations::cut_operation(operation)
                .arguments
                .iter()
                .map(from_spec)
                .collect(),
        }
    }

    /// What the argument at a position expects, if the call takes one
    /// there.
    pub fn expects(self, position: ArgumentPosition<'_>) -> Option<ValueType> {
        let arguments = self.arguments();
        match position {
            ArgumentPosition::Positional(index) => arguments
                .iter()
                .filter(|argument| argument.positional)
                .nth(index)
                .map(|argument| argument.ty),
            ArgumentPosition::Named(name) => arguments
                .iter()
                .find(|argument| !argument.positional && argument.name == name)
                .map(|argument| argument.ty),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{call_arguments, call_name, table_rows};
    use crate::words::WordSet;

    #[test]
    fn blocks_answer_from_the_checker_tables() {
        assert_eq!(Block::Part.fields().len(), fields::PART.len());
        assert_eq!(
            Block::Stock.field("form").map(|f| f.ty),
            Some(ValueType::Words(WordSet::StockForm))
        );
        let joint: Vec<&str> = Block::Kind(BaseKind::Joint)
            .fields()
            .iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(joint, ["insert", "cut_first", "adds", "uses", "note"]);
        let fastener: Vec<&str> = Block::Kind(BaseKind::Fastener)
            .fields()
            .iter()
            .map(|f| f.name)
            .collect();
        assert!(fastener.starts_with(&["diameter", "length"]));
        assert!(Block::Part.field("place").is_none());
    }

    #[test]
    fn arguments_expect_their_types() {
        let center = Callee::Placement(PlacementTerm::Center);
        assert_eq!(
            center.expects(ArgumentPosition::Positional(1)),
            Some(ValueType::Words(WordSet::Axis))
        );
        assert_eq!(center.expects(ArgumentPosition::Positional(2)), None);
        let orient = Callee::Placement(PlacementTerm::Orient);
        assert_eq!(
            orient.expects(ArgumentPosition::Named("width")),
            Some(ValueType::Words(WordSet::Direction))
        );
        let drill = Callee::Cut(CutOperation::Drill);
        assert_eq!(
            drill.expects(ArgumentPosition::Named("depth")),
            Some(ValueType::Depth)
        );
        let side = Callee::Selector(Selector::Side);
        assert_eq!(
            side.expects(ArgumentPosition::Named("normal")),
            Some(ValueType::Words(WordSet::PartAxis))
        );
        let mid = Callee::from_word("mid", CallSite::Expression);
        assert_eq!(mid, Some(Callee::Builtin(Builtin::Mid)));
        assert_eq!(
            Callee::from_word("flush", CallSite::Place),
            Some(Callee::Placement(PlacementTerm::Flush))
        );
        assert_eq!(Callee::from_word("flush", CallSite::Expression), None);
        assert_eq!(
            Callee::from_word("crossing", CallSite::Cut),
            Some(Callee::Selector(Selector::Crossing))
        );
        assert_eq!(
            Callee::from_word("drill", CallSite::Cut),
            Some(Callee::Cut(CutOperation::Drill))
        );
    }

    #[test]
    fn placement_arguments_match_the_grammar() {
        for row in table_rows("| Term | Fixes | Meaning |\n") {
            let cell = &row[0];
            let Some(term) = PlacementTerm::from_word(&call_name(cell)) else {
                panic!("{cell}");
            };
            let named = term.named_arguments();
            for name in call_arguments(cell) {
                assert!(named.contains(&name.as_str()), "{cell}: {name}");
            }
        }
    }
}

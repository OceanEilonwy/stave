//! Reading the concrete syntax tree: items, their names, blocks and values.

use crate::ast::ItemTag;
use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

/// The tag a patch uses for an item node, if a patch can address it.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
)]
pub(super) fn tag_of(kind: SyntaxKind) -> Option<ItemTag> {
    Some(match kind {
        SyntaxKind::KIND => ItemTag::Kind,
        SyntaxKind::RECIPE => ItemTag::Recipe,
        SyntaxKind::PARAM => ItemTag::Param,
        SyntaxKind::LET => ItemTag::Let,
        SyntaxKind::REGION => ItemTag::Region,
        SyntaxKind::STOCK => ItemTag::Stock,
        SyntaxKind::PART => ItemTag::Part,
        SyntaxKind::JOINT => ItemTag::Joint,
        SyntaxKind::HARDWARE => ItemTag::Hardware,
        SyntaxKind::ASSEMBLY => ItemTag::Assembly,
        SyntaxKind::PROFILE => ItemTag::Profile,
        SyntaxKind::COMPONENT => ItemTag::Component,
        SyntaxKind::SCENE => ItemTag::Scene,
        SyntaxKind::REQUIRE => ItemTag::Require,
        _ => return None,
    })
}

/// The text of the first word token directly inside `node`.
pub(super) fn word(node: &SyntaxNode) -> Option<String> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind().is_word())
        .map(|token| token.text().to_owned())
}

/// The declared name of an item: its first `NAME` child.
pub(super) fn item_name(item: &SyntaxNode) -> Option<String> {
    item.children()
        .find(|child| child.kind() == SyntaxKind::NAME)
        .and_then(|name| word(&name))
}

/// The items directly inside a file or a block, in source order.
pub(super) fn items(container: &SyntaxNode) -> impl Iterator<Item = SyntaxNode> + use<> {
    container.children().filter(|child| child.kind().is_item())
}

/// The item of that name directly inside a file or a block.
pub(super) fn item_named(container: &SyntaxNode, name: &str) -> Option<SyntaxNode> {
    items(container)
        .find(|item| tag_of(item.kind()).is_some() && item_name(item).as_deref() == Some(name))
}

/// The `{ }` written directly after an item's header (or a `place`).
pub(super) fn block(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.children()
        .find(|child| child.kind() == SyntaxKind::BLOCK)
}

/// The first child token of `kind`.
pub(super) fn token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == kind)
}

/// The expression directly after the first `token` of `kind` in `node`:
/// the default after a param's `=`, a field's value after `:`.
pub(super) fn expr_after(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxNode> {
    let mut seen = false;
    for element in node.children_with_tokens() {
        match element {
            rowan::NodeOrToken::Token(token) if token.kind() == kind => seen = true,
            rowan::NodeOrToken::Node(child) if seen && child.kind().is_expr() => {
                return Some(child);
            }
            rowan::NodeOrToken::Node(_) | rowan::NodeOrToken::Token(_) => {}
        }
    }
    None
}

/// The field named `key` directly in a block.
pub(super) fn field(block: &SyntaxNode, key: &str) -> Option<SyntaxNode> {
    block.children().find(|child| {
        child.kind() == SyntaxKind::FIELD
            && child
                .children()
                .find(|name| name.kind() == SyntaxKind::NAME)
                .and_then(|name| word(&name))
                .as_deref()
                == Some(key)
    })
}

/// The name of a named argument, or `None` for a positional one.
pub(super) fn arg_name(arg: &SyntaxNode) -> Option<String> {
    token(arg, SyntaxKind::COLON)?;
    arg.children()
        .find(|child| child.kind() == SyntaxKind::NAME)
        .and_then(|name| word(&name))
}

/// The `( )` of a joint's or hardware's kind call.
pub(super) fn kind_call_args(item: &SyntaxNode) -> Option<SyntaxNode> {
    item.children()
        .find(|child| child.kind() == SyntaxKind::KIND_CALL)?
        .children()
        .find(|child| child.kind() == SyntaxKind::ARG_LIST)
}

/// The value of an argument: its expression.
pub(super) fn arg_value(arg: &SyntaxNode) -> Option<SyntaxNode> {
    arg.children().find(|child| child.kind().is_expr())
}

/// The `place` entry of a part's or hardware's body.
pub(super) fn place(body: &SyntaxNode) -> Option<SyntaxNode> {
    body.children()
        .find(|child| child.kind() == SyntaxKind::PLACE)
}

/// The words of the root name and members of a reference (`casework.Drawer`).
pub(super) fn ref_words(reference: &SyntaxNode) -> Vec<String> {
    reference
        .children()
        .filter(|child| child.kind() == SyntaxKind::NAME)
        .filter_map(|name| word(&name))
        .collect()
}

/// Where an item starts after its doc comments: the offset of its first
/// element that is not a doc comment, a newline or trivia. `None` when the
/// item has no such element.
pub(super) fn after_docs(item: &SyntaxNode) -> Option<usize> {
    item.children_with_tokens()
        .find(|element| {
            let kind = element.kind();
            !(kind == SyntaxKind::DOC || kind == SyntaxKind::NEWLINE || kind.is_trivia())
        })
        .map(|element| usize::from(element.text_range().start()))
}

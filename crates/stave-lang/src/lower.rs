//! Lowering: the concrete syntax tree to the typed [`crate::ast`].
//!
//! The typed tree holds only well-formed items. An item that contains a
//! syntax error is left out and listed as a [`BrokenItem`], so later stages
//! can check the rest of the file and stay quiet about the broken item
//! (grammar, "Checks and require": a layer runs only if the layers before it
//! found no errors in the same item). Inside a component, each item is
//! judged on its own, so one mistake doesn't hide the whole component.

use crate::ast::*;
use crate::diagnostics::{Code, Diagnostic, FileId, Label};
use crate::lexer::{parse_number, parse_quantity, unescape};
use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

/// An item left out of the typed tree because it contains an error.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BrokenItem {
    /// The item's keyword, when it is one a patch can address.
    pub tag: Option<ItemTag>,
    /// The item's name as written, when it got that far. For an item inside
    /// a component, `Component.item`.
    pub name: Option<String>,
    /// The whole item.
    pub span: Span,
}

pub(crate) struct Lowering {
    file: FileId,
    /// Primary spans of the error diagnostics, sorted by start.
    errors: Vec<Span>,
    /// Items left out of the typed tree.
    pub broken: Vec<BrokenItem>,
    /// Errors found while lowering (only for items the parser left
    /// incomplete without saying why, which would be a parser bug).
    pub diagnostics: Vec<Diagnostic>,
}

fn span_of(node: &SyntaxNode) -> Span {
    let range = node.text_range();
    Span::new(range.start().into(), range.end().into())
}

fn token_span(token: &SyntaxToken) -> Span {
    let range = token.text_range();
    Span::new(range.start().into(), range.end().into())
}

fn child(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxNode> {
    node.children().find(|child| child.kind() == kind)
}

fn children(node: &SyntaxNode, kind: SyntaxKind) -> impl Iterator<Item = SyntaxNode> {
    node.children().filter(move |child| child.kind() == kind)
}

fn token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == kind)
}

fn exprs(node: &SyntaxNode) -> impl Iterator<Item = SyntaxNode> {
    node.children().filter(|child| child.kind().is_expr())
}

/// The word inside a `NAME` node.
fn word(name: &SyntaxNode) -> Option<SyntaxToken> {
    name.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind().is_word())
}

fn ident(name: &SyntaxNode) -> Option<Ident> {
    let word = word(name)?;
    Ident::new(word.text(), token_span(&word)).ok()
}

fn name(node: &SyntaxNode) -> Option<Name> {
    let word = word(node)?;
    Name::new(word.text(), token_span(&word)).ok()
}

/// `///` doc comments directly inside `node`, without the `///` and one
/// following space.
fn docs(node: &SyntaxNode) -> Docs {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::DOC)
        .map(|token| {
            let text = token.text().strip_prefix("///").unwrap_or(token.text());
            text.strip_prefix(' ').unwrap_or(text).to_owned()
        })
        .collect()
}

impl Lowering {
    pub(crate) fn new(file: FileId, diagnostics: &[Diagnostic]) -> Self {
        let mut errors: Vec<Span> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .filter_map(|diagnostic| diagnostic.primary())
            .map(|label| label.span)
            .collect();
        errors.sort_by_key(|span| (span.start(), span.end()));
        Self {
            file,
            errors,
            broken: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    /// `true` if an error points inside `span` (its end included, for
    /// errors reported at the end of a line), outside the `except` spans.
    fn has_error(&self, span: Span, except: &[Span]) -> bool {
        let first = self
            .errors
            .partition_point(|error| error.start() < span.start());
        self.errors
            .get(first..)
            .unwrap_or(&[])
            .iter()
            .take_while(|error| error.start() <= span.end())
            .any(|error| {
                !except
                    .iter()
                    .any(|inner| inner.start() <= error.start() && error.start() < inner.end())
            })
    }

    /// Records an item as broken. If no error explains why it could not be
    /// lowered, one is reported so that nothing disappears silently.
    fn broken(&mut self, node: &SyntaxNode, prefix: Option<&str>) {
        let span = span_of(node);
        let tag = item_keyword(node).and_then(|keyword| ItemTag::from_keyword(keyword.text()));
        let name = child(node, SyntaxKind::NAME)
            .and_then(|name| word(&name))
            .map(|word| match prefix {
                Some(prefix) => format!("{prefix}.{}", word.text()),
                None => word.text().to_owned(),
            });
        if !self.has_error(span, &[]) {
            let found =
                item_keyword(node).map_or_else(String::new, |keyword| keyword.text().to_owned());
            self.diagnostics.push(Diagnostic::error(
                Code::Syntax,
                format!("expected a complete declaration, found '{found}'"),
                Label::new(self.file, span),
            ));
        }
        self.broken.push(BrokenItem { tag, name, span });
    }

    // -----------------------------------------------------------------------
    // Files
    // -----------------------------------------------------------------------

    pub(crate) fn file(&mut self, root: &SyntaxNode) -> Option<File> {
        let header = child(root, SyntaxKind::HEADER).and_then(|node| self.header(&node));
        let mut imports = Vec::new();
        let mut items = Vec::new();
        for node in root.children() {
            match node.kind() {
                SyntaxKind::IMPORT => match self.import(&node) {
                    Some(import) if !self.has_error(span_of(&node), &[]) => imports.push(import),
                    _ => self.broken(&node, None),
                },
                kind if kind.is_item() => {
                    if let Some(item) = self.checked_item(&node) {
                        items.push(item);
                    }
                }
                _ => {}
            }
        }
        Some(File {
            header: header?,
            imports,
            items,
        })
    }

    pub(crate) fn patch_file(&mut self, root: &SyntaxNode) -> Option<PatchFile> {
        let header = child(root, SyntaxKind::HEADER).and_then(|node| self.header(&node));
        let mut patches = Vec::new();
        for node in children(root, SyntaxKind::PATCH) {
            match self.patch(&node) {
                Some(patch) if !self.has_error(span_of(&node), &[]) => patches.push(patch),
                _ => self.broken(&node, None),
            }
        }
        Some(PatchFile {
            header: header?,
            patches,
        })
    }

    fn header(&self, node: &SyntaxNode) -> Option<Header> {
        let version = token(node, SyntaxKind::INT_NUMBER)?;
        let version = match parse_number(version.text()).ok()? {
            Number::Integer(value) => u32::try_from(value).ok()?,
            Number::Decimal(_) => return None,
        };
        if self.has_error(span_of(node), &[]) {
            return None;
        }
        Some(Header {
            version,
            span: span_of(node),
        })
    }

    fn import(&self, node: &SyntaxNode) -> Option<Import> {
        Some(Import {
            source: string(&child(node, SyntaxKind::STRING)?)?,
            alias: match child(node, SyntaxKind::NAME) {
                Some(alias) => Some(ident(&alias)?),
                None => None,
            },
            span: span_of(node),
        })
    }

    // -----------------------------------------------------------------------
    // Items
    // -----------------------------------------------------------------------

    /// Lowers an item if it has no error, else records it as broken.
    fn checked_item(&mut self, node: &SyntaxNode) -> Option<Item> {
        // A component's items are judged one by one.
        let inner: Vec<Span> = if node.kind() == SyntaxKind::COMPONENT {
            child(node, SyntaxKind::BLOCK)
                .map(|block| {
                    block
                        .children()
                        .filter(|c| c.kind().is_item())
                        .map(|c| span_of(&c))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if self.has_error(span_of(node), &inner) {
            self.broken(node, None);
            return None;
        }
        match self.item(node) {
            Some(item) => Some(item),
            None => {
                self.broken(node, None);
                None
            }
        }
    }

    fn item(&mut self, node: &SyntaxNode) -> Option<Item> {
        let kind = match node.kind() {
            SyntaxKind::PROJECT => ItemKind::Project(Project {
                title: string(&child(node, SyntaxKind::STRING)?)?,
                body: field_block(&child(node, SyntaxKind::BLOCK)?)?,
            }),
            SyntaxKind::LIBRARY => ItemKind::Library(Library {
                title: string(&child(node, SyntaxKind::STRING)?)?,
                body: match child(node, SyntaxKind::BLOCK) {
                    Some(block) => Some(field_block(&block)?),
                    None => None,
                },
            }),
            SyntaxKind::KIND => ItemKind::Kind(kind(node)?),
            SyntaxKind::RECIPE => ItemKind::Recipe(recipe(node)?),
            SyntaxKind::STOCK => ItemKind::Stock(Stock {
                name: ident(&child(node, SyntaxKind::NAME)?)?,
                body: field_block(&child(node, SyntaxKind::BLOCK)?)?,
            }),
            SyntaxKind::PROFILE => ItemKind::Profile(profile(node)?),
            SyntaxKind::COMPONENT => ItemKind::Component(self.component(node)?),
            SyntaxKind::SCENE => ItemKind::Scene(scene(node)?),
            _ => ItemKind::Design(design_item(node)?),
        };
        Some(Item {
            docs: docs(node),
            private: token(node, SyntaxKind::KW_PRIVATE).map(|token| token_span(&token)),
            kind,
            span: span_of(node),
        })
    }

    fn component(&mut self, node: &SyntaxNode) -> Option<Component> {
        let name = ident(&child(node, SyntaxKind::NAME)?)?;
        let block = child(node, SyntaxKind::BLOCK)?;
        let mut items = Vec::new();
        for inner in block.children().filter(|c| c.kind().is_item()) {
            let lowered = if self.has_error(span_of(&inner), &[]) {
                None
            } else {
                design_item(&inner)
            };
            match lowered {
                Some(item) => items.push(Documented {
                    docs: docs(&inner),
                    node: item,
                    span: span_of(&inner),
                }),
                None => self.broken(&inner, Some(name.as_str())),
            }
        }
        Some(Component {
            name,
            items,
            span: span_of(&block),
        })
    }

    fn patch(&mut self, node: &SyntaxNode) -> Option<Patch> {
        let block = child(node, SyntaxKind::BLOCK)?;
        let mut ops = Vec::new();
        for op in block.children() {
            ops.push(PatchOp {
                kind: self.patch_op(&op)?,
                span: span_of(&op),
            });
        }
        Some(Patch {
            summary: string(&child(node, SyntaxKind::STRING)?)?,
            ops,
            span: span_of(node),
        })
    }

    fn patch_op(&mut self, node: &SyntaxNode) -> Option<PatchOpKind> {
        let item_child = || node.children().find(|c| c.kind().is_item());
        let strings: Vec<SyntaxNode> = children(node, SyntaxKind::STRING).collect();
        Some(match node.kind() {
            SyntaxKind::ADD_OP => match child(node, SyntaxKind::IMPORT) {
                Some(import) => PatchOpKind::AddImport {
                    import: self.import(&import)?,
                    file: optional_string(strings.first())?,
                },
                None => PatchOpKind::Add {
                    item: Box::new(self.item(&item_child()?)?),
                    to: match child(node, SyntaxKind::REF) {
                        Some(to) => Some(reference(&to)?),
                        None => None,
                    },
                    file: optional_string(strings.first())?,
                },
            },
            SyntaxKind::REPLACE_OP => PatchOpKind::Replace {
                item: Box::new(self.item(&item_child()?)?),
            },
            SyntaxKind::SET_OP => PatchOpKind::Set {
                target: path(&child(node, SyntaxKind::PATH)?)?,
                value: expr(&exprs(node).next()?)?,
            },
            SyntaxKind::UNSET_OP => PatchOpKind::Unset {
                target: path(&child(node, SyntaxKind::PATH)?)?,
            },
            SyntaxKind::REMOVE_OP => {
                if token(node, SyntaxKind::KW_IMPORT).is_some() {
                    PatchOpKind::RemoveImport {
                        source: string(strings.first()?)?,
                        file: optional_string(strings.get(1))?,
                    }
                } else {
                    PatchOpKind::Remove {
                        tag: item_tag(node)?,
                        target: path(&child(node, SyntaxKind::PATH)?)?,
                    }
                }
            }
            SyntaxKind::RENAME_OP => PatchOpKind::Rename {
                tag: item_tag(node)?,
                target: path(&child(node, SyntaxKind::PATH)?)?,
                to: ident(&child(node, SyntaxKind::NAME)?)?,
            },
            _ => return None,
        })
    }
}

/// The keyword that starts an item node (after docs and `private`).
fn item_keyword(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind().is_item_keyword() && token.kind() != SyntaxKind::KW_PRIVATE)
}

fn design_item(node: &SyntaxNode) -> Option<DesignItem> {
    Some(match node.kind() {
        SyntaxKind::PARAM => DesignItem::Param(param(node)?),
        SyntaxKind::LET => DesignItem::Let(Let {
            name: ident(&child(node, SyntaxKind::NAME)?)?,
            value: expr(&exprs(node).next()?)?,
        }),
        SyntaxKind::REGION => DesignItem::Region(Region {
            name: ident(&child(node, SyntaxKind::NAME)?)?,
            array: array_spec(node)?,
            value: expr(&exprs(node).next()?)?,
        }),
        SyntaxKind::PART => {
            let name = ident(&child(node, SyntaxKind::NAME)?)?;
            let array = array_spec(node)?;
            let def = match child(node, SyntaxKind::MIRROR) {
                Some(mirror_node) => PartDef::Mirror(mirror(&mirror_node)?),
                None => PartDef::FromStock {
                    stock: reference(&child(node, SyntaxKind::REF)?)?,
                    body: part_body(&child(node, SyntaxKind::BLOCK)?)?,
                },
            };
            DesignItem::Part(Part { name, array, def })
        }
        SyntaxKind::JOINT => DesignItem::Joint(Joint {
            name: ident(&child(node, SyntaxKind::NAME)?)?,
            array: array_spec(node)?,
            call: kind_call(&child(node, SyntaxKind::KIND_CALL)?)?,
        }),
        SyntaxKind::HARDWARE => DesignItem::Hardware(Hardware {
            name: ident(&child(node, SyntaxKind::NAME)?)?,
            array: array_spec(node)?,
            call: kind_call(&child(node, SyntaxKind::KIND_CALL)?)?,
            body: match child(node, SyntaxKind::BLOCK) {
                Some(block) => Some(HardwareBody {
                    place: match child(&block, SyntaxKind::PLACE) {
                        Some(place) => Some(place_block(&place)?),
                        None => None,
                    },
                    span: span_of(&block),
                }),
                None => None,
            },
        }),
        SyntaxKind::ASSEMBLY => {
            let name = ident(&child(node, SyntaxKind::NAME)?)?;
            let array = array_spec(node)?;
            let def = match child(node, SyntaxKind::MIRROR) {
                Some(mirror_node) => AssemblyDef::Mirror(mirror(&mirror_node)?),
                None => AssemblyDef::Instance {
                    component: reference(&child(node, SyntaxKind::REF)?)?,
                    body: field_block(&child(node, SyntaxKind::BLOCK)?)?,
                },
            };
            DesignItem::Assembly(Assembly { name, array, def })
        }
        SyntaxKind::REQUIRE => DesignItem::Require(require(node)?),
        _ => return None,
    })
}

fn param(node: &SyntaxNode) -> Option<Param> {
    let mut names = children(node, SyntaxKind::NAME);
    let name = ident(&names.next()?)?;
    let ty = match names.next() {
        Some(ty) => Some(ident(&ty)?),
        None => None,
    };
    let default = match exprs(node).next() {
        Some(default) => Some(expr(&default)?),
        None => None,
    };
    let spec = match (ty, default) {
        (Some(ty), None) => ParamSpec::Type(ty),
        (None, Some(default)) => ParamSpec::Default(default),
        (Some(ty), Some(default)) => ParamSpec::TypeAndDefault { ty, default },
        (None, None) => return None,
    };
    let within = match child(node, SyntaxKind::DOMAIN) {
        Some(domain_node) => Some(domain(&domain_node)?),
        None => None,
    };
    Some(Param {
        name,
        spec,
        within,
        locked: token(node, SyntaxKind::KW_LOCKED).map(|token| token_span(&token)),
    })
}

fn domain(node: &SyntaxNode) -> Option<Domain> {
    let mut values = exprs(node);
    let first = values.next()?;
    if first.kind() == SyntaxKind::LIST_EXPR && token(node, SyntaxKind::DOT2).is_none() {
        return Some(Domain::OneOf {
            values: exprs(&first)
                .map(|value| expr(&value))
                .collect::<Option<_>>()?,
            span: span_of(node),
        });
    }
    Some(Domain::Range {
        lo: expr(&first)?,
        hi: expr(&values.next()?)?,
        span: span_of(node),
    })
}

fn require(node: &SyntaxNode) -> Option<Require> {
    let parts: Vec<SyntaxNode> = exprs(node).collect();
    let [condition, message] = parts.as_slice() else {
        return None;
    };
    if message.kind() != SyntaxKind::STRING {
        return None;
    }
    Some(Require {
        name: ident(&child(node, SyntaxKind::NAME)?)?,
        condition: expr(condition)?,
        message: string(message)?,
    })
}

fn kind(node: &SyntaxNode) -> Option<Kind> {
    let mut names = children(node, SyntaxKind::NAME);
    let kind_name = ident(&names.next()?)?;
    let base_node = names.next()?;
    let base_word = word(&base_node)?;
    let block = child(node, SyntaxKind::BLOCK)?;
    let mut entries = Vec::new();
    for entry in block.children() {
        let documented = |entry: &SyntaxNode| -> (Docs, Span) { (docs(entry), span_of(entry)) };
        entries.push(match entry.kind() {
            SyntaxKind::PARAM => {
                let (docs, span) = documented(&entry);
                KindEntry::Param(Documented {
                    docs,
                    node: param(&entry)?,
                    span,
                })
            }
            SyntaxKind::REQUIRE => {
                let (docs, span) = documented(&entry);
                KindEntry::Require(Documented {
                    docs,
                    node: require(&entry)?,
                    span,
                })
            }
            SyntaxKind::MAKES => {
                let (docs, span) = documented(&entry);
                KindEntry::Makes(Documented {
                    docs,
                    node: MadePart {
                        name: ident(&child(&entry, SyntaxKind::NAME)?)?,
                        stock: reference(&child(&entry, SyntaxKind::REF)?)?,
                        body: part_body(&child(&entry, SyntaxKind::BLOCK)?)?,
                    },
                    span,
                })
            }
            SyntaxKind::ROLES => {
                let roles_block = child(&entry, SyntaxKind::BLOCK)?;
                let mut roles = Vec::new();
                for role in roles_block.children() {
                    let mut names = children(&role, SyntaxKind::NAME);
                    roles.push(Role {
                        name: name(&names.next()?)?,
                        ty: ident(&names.next()?)?,
                        span: span_of(&role),
                    });
                }
                KindEntry::Roles(RolesBlock {
                    roles,
                    span: span_of(&entry),
                })
            }
            SyntaxKind::CUT => KindEntry::Cut(Cut {
                selector: call(&child(&entry, SyntaxKind::CALL)?)?,
                operations: calls(&child(&entry, SyntaxKind::BLOCK)?)?,
                span: span_of(&entry),
            }),
            SyntaxKind::FIELD => KindEntry::Field(field(&entry)?),
            _ => return None,
        });
    }
    Some(Kind {
        name: kind_name,
        base: Spanned {
            value: BaseKind::from_word(base_word.text())?,
            span: token_span(&base_word),
        },
        body: KindBody {
            entries,
            span: span_of(&block),
        },
    })
}

fn recipe(node: &SyntaxNode) -> Option<Recipe> {
    let mut names = children(node, SyntaxKind::NAME);
    let name = ident(&names.next()?)?;
    let operation = ident(&names.next()?)?;
    let block = child(node, SyntaxKind::BLOCK)?;
    let mut entries = Vec::new();
    for entry in block.children() {
        entries.push(match entry.kind() {
            SyntaxKind::STEP_BLOCK => {
                let steps = StepBlock {
                    steps: calls(&child(&entry, SyntaxKind::BLOCK)?)?,
                    span: span_of(&entry),
                };
                match token(&entry, SyntaxKind::IDENT)?.text() {
                    "mark" => RecipeEntry::Mark(steps),
                    _ => RecipeEntry::Cut(steps),
                }
            }
            SyntaxKind::FIELD => RecipeEntry::Field(field(&entry)?),
            _ => return None,
        });
    }
    Some(Recipe {
        name,
        operation,
        body: RecipeBody {
            entries,
            span: span_of(&block),
        },
    })
}

fn profile(node: &SyntaxNode) -> Option<Profile> {
    let block = child(node, SyntaxKind::BLOCK)?;
    let mut entries = Vec::new();
    for entry in block.children() {
        entries.push(match entry.kind() {
            SyntaxKind::STATION => {
                let mut values = exprs(&entry);
                ProfileEntry::Station {
                    at: expr(&values.next()?)?,
                    value: expr(&values.next()?)?,
                    span: span_of(&entry),
                }
            }
            SyntaxKind::FIELD => ProfileEntry::Field(field(&entry)?),
            _ => return None,
        });
    }
    Some(Profile {
        name: ident(&child(node, SyntaxKind::NAME)?)?,
        entries,
        span: span_of(&block),
    })
}

fn scene(node: &SyntaxNode) -> Option<Scene> {
    let block = child(node, SyntaxKind::BLOCK)?;
    let mut entries = Vec::new();
    for entry in block.children() {
        entries.push(match entry.kind() {
            SyntaxKind::POSE => SceneEntry::Pose {
                target: path(&child(&entry, SyntaxKind::PATH)?)?,
                value: expr(&exprs(&entry).next()?)?,
                span: span_of(&entry),
            },
            SyntaxKind::FIELD => SceneEntry::Field(field(&entry)?),
            _ => return None,
        });
    }
    Some(Scene {
        name: ident(&child(node, SyntaxKind::NAME)?)?,
        title: match child(node, SyntaxKind::STRING) {
            Some(title) => Some(string(&title)?),
            None => None,
        },
        entries,
        span: span_of(&block),
    })
}

/// `Some(None)` when there is no string, `None` when it doesn't lower.
fn optional_string(node: Option<&SyntaxNode>) -> Option<Option<StringLit>> {
    match node {
        Some(node) => Some(Some(string(node)?)),
        None => Some(None),
    }
}

fn item_tag(node: &SyntaxNode) -> Option<ItemTag> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find_map(|token| {
            token
                .kind()
                .is_keyword()
                .then(|| ItemTag::from_keyword(token.text()))
                .flatten()
        })
}

// ---------------------------------------------------------------------------
// Blocks, fields, calls
// ---------------------------------------------------------------------------

fn field_block(block: &SyntaxNode) -> Option<FieldBlock> {
    Some(FieldBlock {
        fields: block
            .children()
            .map(|entry| match entry.kind() {
                SyntaxKind::FIELD => field(&entry),
                _ => None,
            })
            .collect::<Option<_>>()?,
        span: span_of(block),
    })
}

fn part_body(block: &SyntaxNode) -> Option<PartBody> {
    Some(PartBody {
        entries: block
            .children()
            .map(|entry| match entry.kind() {
                SyntaxKind::FIELD => field(&entry).map(PartEntry::Field),
                SyntaxKind::PLACE => place_block(&entry).map(PartEntry::Place),
                _ => None,
            })
            .collect::<Option<_>>()?,
        span: span_of(block),
    })
}

fn place_block(node: &SyntaxNode) -> Option<PlaceBlock> {
    Some(PlaceBlock {
        terms: calls(&child(node, SyntaxKind::BLOCK)?)?,
        span: span_of(node),
    })
}

fn calls(block: &SyntaxNode) -> Option<Vec<Call>> {
    block
        .children()
        .map(|entry| match entry.kind() {
            SyntaxKind::CALL => call(&entry),
            _ => None,
        })
        .collect()
}

fn field(node: &SyntaxNode) -> Option<Field> {
    Some(Field {
        key: name(&child(node, SyntaxKind::NAME)?)?,
        value: expr(&exprs(node).next()?)?,
        span: span_of(node),
    })
}

fn call(node: &SyntaxNode) -> Option<Call> {
    Some(Call {
        head: ident(&child(node, SyntaxKind::NAME)?)?,
        args: match child(node, SyntaxKind::ARG_LIST) {
            Some(list) => args(&list)?,
            None => Vec::new(),
        },
        span: span_of(node),
    })
}

fn kind_call(node: &SyntaxNode) -> Option<KindCall> {
    Some(KindCall {
        kind: reference(&child(node, SyntaxKind::REF)?)?,
        args: args(&child(node, SyntaxKind::ARG_LIST)?)?,
        span: span_of(node),
    })
}

fn mirror(node: &SyntaxNode) -> Option<Mirror> {
    let mut values = exprs(node);
    Some(Mirror {
        source: expr(&values.next()?)?,
        about: expr(&values.next()?)?,
        span: span_of(node),
    })
}

fn args(list: &SyntaxNode) -> Option<Vec<Arg>> {
    children(list, SyntaxKind::ARG)
        .map(|arg| {
            Some(Arg {
                name: match child(&arg, SyntaxKind::NAME) {
                    Some(arg_name) => Some(name(&arg_name)?),
                    None => None,
                },
                value: expr(&exprs(&arg).next()?)?,
                span: span_of(&arg),
            })
        })
        .collect()
}

fn reference(node: &SyntaxNode) -> Option<Ref> {
    let mut names = children(node, SyntaxKind::NAME);
    Some(Ref {
        root: ident(&names.next()?)?,
        rest: names.map(|rest| name(&rest)).collect::<Option<_>>()?,
        span: span_of(node),
    })
}

fn path(node: &SyntaxNode) -> Option<Path> {
    let mut nodes = node.children();
    let root = ident(&nodes.next()?)?;
    let segments = nodes
        .map(|segment| match segment.kind() {
            SyntaxKind::NAME => name(&segment).map(PathSegment::Member),
            SyntaxKind::INDEX => index(&segment).map(PathSegment::Index),
            _ => None,
        })
        .collect::<Option<_>>()?;
    Some(Path {
        root,
        segments,
        span: span_of(node),
    })
}

fn index(node: &SyntaxNode) -> Option<Index> {
    if token(node, SyntaxKind::STAR).is_some() {
        return Some(Index::All(span_of(node)));
    }
    if let Some(same) = token(node, SyntaxKind::IDENT) {
        return (same.text() == "same").then(|| Index::Same(span_of(node)));
    }
    Some(Index::At(Box::new(expr(&exprs(node).next()?)?)))
}

/// `Some(None)` without an array spec, `None` when it doesn't lower.
fn array_spec(node: &SyntaxNode) -> Option<Option<ArraySpec>> {
    let Some(spec) = child(node, SyntaxKind::ARRAY_SPEC) else {
        return Some(None);
    };
    let labels: Vec<SyntaxNode> = children(&spec, SyntaxKind::NAME).collect();
    if labels.is_empty() {
        return Some(Some(ArraySpec::Count(expr(&exprs(&spec).next()?)?)));
    }
    let labels = labels.iter().map(ident).collect::<Option<Vec<_>>>()?;
    Some(Some(ArraySpec::Labels(
        Labels::new(labels, span_of(&spec)).ok()?,
    )))
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

fn string(node: &SyntaxNode) -> Option<StringLit> {
    let mut parts = Vec::new();
    for element in node.children_with_tokens() {
        match element {
            rowan::NodeOrToken::Token(token) if token.kind() == SyntaxKind::STRING_TEXT => {
                parts.push(StringPart::Text {
                    text: unescape(token.text()),
                    span: token_span(&token),
                });
            }
            rowan::NodeOrToken::Node(interp) if interp.kind() == SyntaxKind::INTERP => {
                let mut names = children(&interp, SyntaxKind::NAME);
                parts.push(StringPart::Interp {
                    root: name(&names.next()?)?,
                    members: names.map(|member| name(&member)).collect::<Option<_>>()?,
                    span: span_of(&interp),
                });
            }
            rowan::NodeOrToken::Token(_) | rowan::NodeOrToken::Node(_) => {}
        }
    }
    Some(StringLit {
        parts,
        span: span_of(node),
    })
}

fn expr(node: &SyntaxNode) -> Option<Expr> {
    let span = span_of(node);
    let kind = match node.kind() {
        SyntaxKind::LITERAL => {
            let literal = node.first_token()?;
            match literal.kind() {
                SyntaxKind::QUANTITY => ExprKind::Quantity(parse_quantity(literal.text()).ok()?),
                SyntaxKind::INT_NUMBER | SyntaxKind::DECIMAL_NUMBER => {
                    ExprKind::Number(parse_number(literal.text()).ok()?)
                }
                SyntaxKind::KW_TRUE => ExprKind::Bool(true),
                SyntaxKind::KW_FALSE => ExprKind::Bool(false),
                SyntaxKind::KW_FIT => ExprKind::Fit,
                _ => return None,
            }
        }
        SyntaxKind::STRING => ExprKind::String(string(node)?),
        SyntaxKind::NAME_REF => {
            let word = node.first_token()?;
            ExprKind::Name(Ident::new(word.text(), token_span(&word)).ok()?)
        }
        SyntaxKind::MEMBER_EXPR => ExprKind::Member {
            base: Box::new(expr(&exprs(node).next()?)?),
            member: name(&child(node, SyntaxKind::NAME)?)?,
        },
        SyntaxKind::INDEX_EXPR => ExprKind::Index {
            base: Box::new(expr(&exprs(node).next()?)?),
            index: index(&child(node, SyntaxKind::INDEX)?)?,
        },
        SyntaxKind::CALL_EXPR => ExprKind::Call {
            callee: Box::new(expr(&exprs(node).next()?)?),
            args: args(&child(node, SyntaxKind::ARG_LIST)?)?,
        },
        SyntaxKind::UNARY_EXPR => {
            let op = match node.first_token()?.kind() {
                SyntaxKind::MINUS => UnaryOp::Neg,
                SyntaxKind::BANG => UnaryOp::Not,
                _ => return None,
            };
            ExprKind::Unary {
                op,
                operand: Box::new(expr(&exprs(node).next()?)?),
            }
        }
        SyntaxKind::BINARY_EXPR => {
            let mut operands = exprs(node);
            let lhs = expr(&operands.next()?)?;
            let rhs = expr(&operands.next()?)?;
            let op = node
                .children_with_tokens()
                .filter_map(|element| element.into_token())
                .find_map(|token| binary_op(token.kind()))?;
            ExprKind::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            }
        }
        SyntaxKind::LIST_EXPR => {
            ExprKind::List(exprs(node).map(|item| expr(&item)).collect::<Option<_>>()?)
        }
        SyntaxKind::PAREN_EXPR => ExprKind::Paren(Box::new(expr(&exprs(node).next()?)?)),
        _ => return None,
    };
    Some(Expr { kind, span })
}

fn binary_op(kind: SyntaxKind) -> Option<BinaryOp> {
    Some(match kind {
        SyntaxKind::PIPE2 => BinaryOp::Or,
        SyntaxKind::AMP2 => BinaryOp::And,
        SyntaxKind::EQ2 => BinaryOp::Eq,
        SyntaxKind::NEQ => BinaryOp::Ne,
        SyntaxKind::LT => BinaryOp::Lt,
        SyntaxKind::LE => BinaryOp::Le,
        SyntaxKind::GT => BinaryOp::Gt,
        SyntaxKind::GE => BinaryOp::Ge,
        SyntaxKind::PLUS => BinaryOp::Add,
        SyntaxKind::MINUS => BinaryOp::Sub,
        SyntaxKind::STAR => BinaryOp::Mul,
        SyntaxKind::SLASH => BinaryOp::Div,
        _ => return None,
    })
}

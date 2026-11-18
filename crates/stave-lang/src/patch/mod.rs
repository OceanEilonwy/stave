//! Applying a patch to the text of a module.
//!
//! A patch (grammar, "Patches") is how an AI, a suggestion or a mouse tool
//! changes a design. [`apply_patch`] takes the parsed files of the
//! project's module (and of the local modules it imports) and a parsed
//! patch file, and returns the new text of every file the patch changed,
//! with the smallest [`TextEdit`]s that turn the old text into the new, so
//! an open editor buffer and its undo can take the change.
//!
//! The patcher edits text, guided by the lossless syntax tree:
//!
//! - Only the bytes an operation is about change. Comments, doc comments,
//!   blank lines and the layout of everything else are kept byte for byte,
//!   and a comment at the end of an edited line stays where it is.
//! - The lines a patch writes are laid out as `stave fmt` lays them out in
//!   their place (indentation, spacing, alignment with their neighbours).
//!   A file that doesn't parse keeps the patch's own layout, re-indented.
//! - A `set` whose value says the same thing as the value already written
//!   changes nothing, however the patch lays it out.
//!
//! Operations apply in order, each to the result of the one before, so a
//! later operation can refer to what an earlier one added. A patch that
//! refers to something that doesn't exist is refused with a
//! [`PatchErrorKind`] naming it, and nothing is changed. `rename` is not
//! applied ([`PatchErrorKind::NotSupported`]): it needs name resolution,
//! which this crate doesn't have.
//!
//! The patcher checks only what it needs to place the text: the patched
//! design is to be checked again as a whole, as for any edit.

mod diff;
mod placement;
mod text;
mod tree;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::ast::{self, ItemTag, Span};
use crate::diagnostics::{Diagnostic, FileId};
use crate::placement::Axis;
use crate::syntax::{SyntaxKind, SyntaxNode};
use crate::{FORMAT_VERSION, Parse, parse_file};

pub use diff::{TextEdit, apply_text_edits};
pub use placement::{TermAddress, TermFixes};
use placement::{fixes, term_text_fixes};
use text::{
    Edit, indentation, is_space, line_end, range, reindent, relayout, remove_entry, tokens,
};

/// Which module a file belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModuleKey {
    /// The project's own module: the folder of the design being edited.
    Project,
    /// A local module the project imports, by the name the project uses
    /// for it (`casework` for `import "./casework"`, `hw` for
    /// `import "./hardware" as hw`).
    Local(String),
}

impl fmt::Display for ModuleKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModuleKey::Project => f.write_str("the project"),
            ModuleKey::Local(name) => write!(f, "module '{name}'"),
        }
    }
}

/// Where a patch looked for an item, or where an item already is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// The top level of a module.
    Module(ModuleKey),
    /// The body of a component, by its name (`Drawer`).
    Component(String),
    /// The local modules the patch was given, when a path's first word is
    /// none of them.
    LocalModules,
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::Module(module) => module.fmt(f),
            Scope::Component(name) => write!(f, "component '{name}'"),
            Scope::LocalModules => f.write_str("the local modules"),
        }
    }
}

/// One parsed file of a module.
#[derive(Debug, Clone, Copy)]
pub struct ModuleFile<'a> {
    /// The file's name within its module's folder: `carcass.stave`.
    pub name: &'a str,
    /// The file, parsed with [`crate::parse_file`].
    pub parse: &'a Parse<ast::File>,
}

/// What a patch is applied to: the project's module and the local modules
/// it may reach into. A module from a remote source is read-only and is
/// left out, so a patch that reaches into it is refused.
#[derive(Debug, Clone, Default)]
pub struct PatchTarget<'a> {
    /// The files of the project's module.
    pub project: Vec<ModuleFile<'a>>,
    /// The files of each local module, by the name the project uses for it.
    pub local_modules: BTreeMap<String, Vec<ModuleFile<'a>>>,
}

impl<'a> PatchTarget<'a> {
    /// A target with the project's files and no local modules.
    pub fn new(project: Vec<ModuleFile<'a>>) -> Self {
        Self {
            project,
            local_modules: BTreeMap::new(),
        }
    }

    /// Adds a local module the patch may reach into.
    pub fn with_local_module(
        mut self,
        name: impl Into<String>,
        files: Vec<ModuleFile<'a>>,
    ) -> Self {
        self.local_modules.insert(name.into(), files);
        self
    }
}

/// A file a patch changed or created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchedFile {
    /// The module the file belongs to.
    pub module: ModuleKey,
    /// The file's name within its module's folder.
    pub name: String,
    /// The file's new text.
    pub text: String,
    /// `true` for a file the patch created (`add ... in "new.stave"`).
    pub created: bool,
    /// The smallest edits that turn the file's old text (empty for a file
    /// the patch created) into [`PatchedFile::text`], last in the file
    /// first: applied in this order, each edit's range is still a range of
    /// the old text. See [`apply_text_edits`].
    pub edits: Vec<TextEdit>,
}

/// Why a patch was not applied: what went wrong, and the operation it went
/// wrong at.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind}")]
pub struct PatchError {
    /// What went wrong.
    pub kind: PatchErrorKind,
    /// The operation in the patch file (the whole patch file for
    /// [`PatchErrorKind::Syntax`]).
    pub span: Span,
}

/// What went wrong while applying a patch.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatchErrorKind {
    /// The patch file has syntax errors.
    #[error("the patch has {} syntax error(s); fix them before applying it", .0.len())]
    Syntax(Vec<Diagnostic>),
    /// The patch names an item that isn't there.
    #[error("there is no item named '{name}' in {scope}")]
    UnknownItem {
        /// The name the patch used.
        name: String,
        /// Where it was looked for.
        scope: Scope,
    },
    /// The patch names something inside an item that isn't there.
    #[error("'{item}' has no {what} named '{name}'")]
    UnknownMember {
        /// The item, as the patch wrote it.
        item: String,
        /// What was looked for: "field", "placement axis".
        what: &'static str,
        /// The name the patch used.
        name: String,
    },
    /// `unset` of something that isn't written, so its default already
    /// applies.
    #[error("'{0}' is not written, so there is nothing to unset")]
    NotSet(String),
    /// `to <component>` names something that is not a component.
    #[error("'{0}' is not a component; only a component can hold other items")]
    NotAComponent(String),
    /// `remove <tag>` names an item of another kind.
    #[error("'{path}' is a {found}, not a {expected}")]
    WrongTag {
        /// The item, as the patch wrote it.
        path: String,
        /// The tag the patch wrote.
        expected: &'static str,
        /// The item's own keyword.
        found: &'static str,
    },
    /// `add` of an item whose name is taken.
    #[error("there is already an item named '{name}' in {scope}")]
    AlreadyExists {
        /// The name.
        name: String,
        /// Where.
        scope: Scope,
    },
    /// A path that names nothing a patch can change.
    #[error("'{path}' can't be changed by a patch: {reason}")]
    Unaddressable {
        /// The path, as the patch wrote it.
        path: String,
        /// Why, and what to write instead.
        reason: String,
    },
    /// `in "file.stave"` names a file the module doesn't have (for an
    /// import), or a name that is not a `.stave` file in the folder.
    #[error("there is no file named '{0}' in the project's folder")]
    UnknownFile(String),
    /// `in "..."` that is not a plain `.stave` file name.
    #[error(
        "'{0}' is not a file name in the project's folder; write a name such as \"carcass.stave\""
    )]
    InvalidFileName(String),
    /// `add ... to <component> in "file"` where the component is in
    /// another file.
    #[error("component '{component}' is in '{actual}', not in '{requested}'")]
    ComponentElsewhere {
        /// The component.
        component: String,
        /// The file it is in.
        actual: String,
        /// The file the patch named.
        requested: String,
    },
    /// The module has no file to add to.
    #[error("the project has no file to add to")]
    NoFiles,
    /// `remove import` of an import that isn't there.
    #[error("no file imports {import}{}", .file.as_ref().map(|file| format!(" in '{file}'")).unwrap_or_default())]
    UnknownImport {
        /// The import's source, quotes included.
        import: String,
        /// The file named by `in`.
        file: Option<String>,
    },
    /// `add import` of a source the file already imports differently.
    #[error("'{file}' already imports {import}; remove that import first")]
    ImportExists {
        /// The import's source, quotes included.
        import: String,
        /// The file.
        file: String,
    },
    /// `set <item>.place.<axis>` with a term that fixes another axis (or
    /// the orientation): the term would not be the one the path names.
    #[error("'{term}' fixes {fixes}, not {addressed}, so it can't be set as '{path}'")]
    WrongTerm {
        /// The path, as the patch wrote it.
        path: String,
        /// The new term, as the patch wrote it.
        term: String,
        /// What the path names.
        addressed: TermAddress,
        /// What the new term fixes.
        fixes: TermFixes,
    },
    /// `set <item>.place.<axis>` with a term that also fixes an axis
    /// another of the item's terms fixes (`corners` where a `flush` fixes
    /// `deep`): the axis would be set twice.
    #[error(
        "'{term}' also fixes {}, which '{other}' already fixes; unset that term first, so each \
         axis has one term",
        .axis.word()
    )]
    TermClash {
        /// The new term, as the patch wrote it.
        term: String,
        /// The axis both fix.
        axis: Axis,
        /// The term already written, as the file writes it.
        other: String,
    },
    /// An operation the patcher doesn't apply yet.
    #[error("'{0}' is not supported yet")]
    NotSupported(&'static str),
    /// The patch would leave a file that parsed with syntax errors.
    #[error("the patch would leave '{file}' with {} syntax error(s)", .diagnostics.len())]
    BreaksSyntax {
        /// The file.
        file: String,
        /// Its errors after the operation.
        diagnostics: Vec<Diagnostic>,
    },
    /// Two edits of one operation overlap: a bug in the patcher.
    #[error("the patch's edits overlap; please report this as a bug")]
    Conflict,
}

/// Applies every patch of a patch file, operation by operation, to the
/// files of `target`. Returns the files that changed or were created,
/// ordered by module and name; files the patch didn't change are left out.
///
/// # Errors
///
/// A patch file with syntax errors, or an operation that refers to
/// something that doesn't exist or can't be changed, is refused with what
/// it referred to; nothing is applied.
pub fn apply_patch(
    target: &PatchTarget<'_>,
    patch: &Parse<ast::PatchFile>,
) -> Result<Vec<PatchedFile>, PatchError> {
    let patch_text = patch.syntax().to_string();
    let whole = Span::new(0, u32::try_from(patch_text.len()).unwrap_or(u32::MAX));
    let syntax_error = || PatchError {
        kind: PatchErrorKind::Syntax(
            patch
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .cloned()
                .collect(),
        ),
        span: whole,
    };
    if patch.has_errors() {
        return Err(syntax_error());
    }
    let Some(file) = &patch.ast else {
        return Err(syntax_error());
    };
    let mut applier = Applier::new(target, &patch_text, patch.syntax());
    for one in &file.patches {
        for op in &one.ops {
            applier.apply(op).map_err(|kind| PatchError {
                kind,
                span: op.span,
            })?;
        }
    }
    Ok(applier.finish())
}

/// A file while the patch is applied.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FileKey {
    module: ModuleKey,
    name: String,
}

#[derive(Debug)]
struct FileState {
    /// The text before the patch; `None` for a file the patch creates.
    original: Option<String>,
    /// `true` if the text before the patch had syntax errors.
    had_errors: bool,
    text: String,
    root: SyntaxNode,
}

impl FileState {
    fn new(original: Option<String>, text: String) -> Self {
        let parse = parse_file(FileId::DETACHED, &text);
        Self {
            had_errors: original.is_some() && parse.has_errors(),
            original,
            root: parse.syntax(),
            text,
        }
    }

    /// Replaces the text; returns its syntax errors.
    fn set(&mut self, text: String) -> Vec<Diagnostic> {
        let parse = parse_file(FileId::DETACHED, &text);
        self.root = parse.syntax();
        self.text = text;
        parse
            .diagnostics
            .into_iter()
            .filter(Diagnostic::is_error)
            .collect()
    }
}

/// One step of a patch path, with the text it was written as.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// `.name`
    Member(String),
    /// `[*]` (`all`) or another index.
    Index { all: bool },
}

/// An item a path reaches, and the rest of the path after it.
#[derive(Debug)]
struct Located {
    file: FileKey,
    item: SyntaxNode,
    rest: Vec<Segment>,
}

/// What a `set` or `unset` path addresses.
#[derive(Debug)]
enum Slot {
    /// The default of a param; `value` is `None` when it has none.
    ParamDefault {
        item: SyntaxNode,
        value: Option<SyntaxNode>,
    },
    /// The value of a `let` or a `region`.
    Value(SyntaxNode),
    /// A field of a block; `field` is `None` when it isn't written.
    Field {
        block: SyntaxNode,
        key: String,
        field: Option<SyntaxNode>,
    },
    /// A named argument of a kind call; `arg` is `None` when it isn't
    /// written.
    Arg {
        args: SyntaxNode,
        name: String,
        arg: Option<SyntaxNode>,
    },
    /// The placement term that fixes an axis, or the `orient` term.
    /// `others` are the item's other terms.
    Term {
        item: SyntaxNode,
        term: Option<SyntaxNode>,
        address: TermAddress,
        others: Vec<SyntaxNode>,
    },
}

struct Applier<'p> {
    files: BTreeMap<FileKey, FileState>,
    modules: BTreeSet<ModuleKey>,
    patch: &'p str,
    patch_root: SyntaxNode,
}

impl<'p> Applier<'p> {
    fn new(target: &PatchTarget<'_>, patch: &'p str, patch_root: SyntaxNode) -> Self {
        let mut files = BTreeMap::new();
        let mut modules = BTreeSet::new();
        let all = std::iter::once((ModuleKey::Project, &target.project)).chain(
            target
                .local_modules
                .iter()
                .map(|(name, files)| (ModuleKey::Local(name.clone()), files)),
        );
        for (module, module_files) in all {
            modules.insert(module.clone());
            for file in module_files {
                let text = file.parse.syntax().to_string();
                files.insert(
                    FileKey {
                        module: module.clone(),
                        name: file.name.to_owned(),
                    },
                    FileState::new(Some(text.clone()), text),
                );
            }
        }
        Self {
            files,
            modules,
            patch,
            patch_root,
        }
    }

    fn finish(self) -> Vec<PatchedFile> {
        self.files
            .into_iter()
            .filter(|(_, state)| state.original.as_deref() != Some(state.text.as_str()))
            .map(|(key, state)| PatchedFile {
                module: key.module,
                name: key.name,
                created: state.original.is_none(),
                edits: diff::text_edits(state.original.as_deref().unwrap_or(""), &state.text),
                text: state.text,
            })
            .collect()
    }

    fn apply(&mut self, op: &ast::PatchOp) -> Result<(), PatchErrorKind> {
        match &op.kind {
            ast::PatchOpKind::Add { item, to, file } => self.add(item, to.as_ref(), file.as_ref()),
            ast::PatchOpKind::AddImport { import, file } => self.add_import(import, file.as_ref()),
            ast::PatchOpKind::Replace { item } => self.replace(item),
            ast::PatchOpKind::Set { target, value } => self.set(target, value),
            ast::PatchOpKind::Unset { target } => self.unset(target),
            ast::PatchOpKind::Remove { tag, target } => self.remove(*tag, target),
            ast::PatchOpKind::RemoveImport { source, file } => {
                self.remove_import(source, file.as_ref())
            }
            ast::PatchOpKind::Rename { .. } => Err(PatchErrorKind::NotSupported("rename")),
        }
    }

    // ---- reading the patch ----

    /// The patch text a span covers.
    fn slice(&self, span: Span) -> &'p str {
        let patch: &'p str = self.patch;
        usize::try_from(span.start())
            .ok()
            .zip(usize::try_from(span.end()).ok())
            .and_then(|(start, end)| patch.get(start..end))
            .unwrap_or("")
    }

    /// The text of a span of the patch, its later lines indented relative
    /// to its first.
    fn snippet(&self, span: Span) -> String {
        let start = usize::try_from(span.start()).unwrap_or(0);
        reindent(self.slice(span), indentation(self.patch, start), "")
    }

    fn segments(path: &ast::Path) -> Vec<Segment> {
        path.segments
            .iter()
            .map(|segment| match segment {
                ast::PathSegment::Member(name) => Segment::Member(name.as_str().to_owned()),
                ast::PathSegment::Index(index) => Segment::Index {
                    all: matches!(index, ast::Index::All(_)),
                },
            })
            .collect()
    }

    // ---- reading the module ----

    fn state(&self, key: &FileKey) -> Result<&FileState, PatchErrorKind> {
        self.files
            .get(key)
            .ok_or_else(|| PatchErrorKind::UnknownFile(key.name.clone()))
    }

    fn module_files<'s>(
        &'s self,
        module: &'s ModuleKey,
    ) -> impl Iterator<Item = (&'s FileKey, &'s FileState)> + 's {
        self.files
            .iter()
            .filter(move |(key, _)| key.module == *module)
    }

    /// The top-level item of that name in a module, and its file.
    fn find_item(&self, module: &ModuleKey, name: &str) -> Option<(FileKey, SyntaxNode)> {
        self.module_files(module).find_map(|(key, state)| {
            tree::item_named(&state.root, name).map(|item| (key.clone(), item))
        })
    }

    /// The file holding the `project` declaration, or else the first file.
    fn project_file(&self) -> Result<FileKey, PatchErrorKind> {
        let mut files = self.module_files(&ModuleKey::Project);
        let first = files.next().map(|(key, _)| key.clone());
        self.module_files(&ModuleKey::Project)
            .find(|(_, state)| {
                state
                    .root
                    .children()
                    .any(|child| child.kind() == SyntaxKind::PROJECT)
            })
            .map(|(key, _)| key.clone())
            .or(first)
            .ok_or(PatchErrorKind::NoFiles)
    }

    /// Follows a path to the item it names: through a local module's name
    /// and into components.
    fn locate(&self, path: &ast::Path) -> Result<Located, PatchErrorKind> {
        let segments = Self::segments(path);
        let root = path.root.as_str();
        let mut used = 0;
        let (file, mut item) = match self.find_item(&ModuleKey::Project, root) {
            Some(found) => found,
            None => {
                let module = ModuleKey::Local(root.to_owned());
                if !self.modules.contains(&module) {
                    return Err(PatchErrorKind::UnknownItem {
                        name: root.to_owned(),
                        scope: Scope::Module(ModuleKey::Project),
                    });
                }
                let Some(Segment::Member(name)) = segments.first() else {
                    return Err(PatchErrorKind::Unaddressable {
                        path: self.slice(path.span).to_owned(),
                        reason: format!("'{root}' is a module; name an item in it"),
                    });
                };
                used = 1;
                self.find_item(&module, name)
                    .ok_or_else(|| PatchErrorKind::UnknownItem {
                        name: name.clone(),
                        scope: Scope::Module(module.clone()),
                    })?
            }
        };
        while item.kind() == SyntaxKind::COMPONENT {
            let Some(Segment::Member(name)) = segments.get(used) else {
                break;
            };
            let component = tree::item_name(&item).unwrap_or_default();
            item = tree::block(&item)
                .and_then(|body| tree::item_named(&body, name))
                .ok_or_else(|| PatchErrorKind::UnknownItem {
                    name: name.clone(),
                    scope: Scope::Component(component.clone()),
                })?;
            used += 1;
        }
        Ok(Located {
            file,
            item,
            rest: segments.get(used..).unwrap_or_default().to_vec(),
        })
    }

    /// What a `set` or `unset` path addresses (grammar, "Patches", the path
    /// table).
    fn slot(&self, located: &Located, path: &ast::Path) -> Result<Slot, PatchErrorKind> {
        let path_text = self.slice(path.span).to_owned();
        let item = &located.item;
        let name = tree::item_name(item).unwrap_or_default();
        let unaddressable = |reason: String| PatchErrorKind::Unaddressable {
            path: path_text.clone(),
            reason,
        };
        let mut rest = located.rest.as_slice();
        while let [Segment::Index { all }, tail @ ..] = rest {
            if !all {
                return Err(unaddressable(format!(
                    "the elements of '{name}' share one declaration; set the value on '{name}' to \
                     change it for all of them"
                )));
            }
            rest = tail;
        }
        let keyword = tree::tag_of(item.kind()).map_or("item", ItemTag::keyword);
        let body = |what: &str| {
            tree::block(item).ok_or_else(|| {
                unaddressable(format!(
                    "'{name}' has no body to hold {what}; it is a mirror image or has no braces"
                ))
            })
        };
        let field = |key: &str| -> Result<Slot, PatchErrorKind> {
            let block = body("fields")?;
            Ok(Slot::Field {
                field: tree::field(&block, key),
                key: key.to_owned(),
                block,
            })
        };
        match (item.kind(), rest) {
            (SyntaxKind::PARAM, []) => Ok(Slot::ParamDefault {
                value: tree::expr_after(item, SyntaxKind::EQ),
                item: item.clone(),
            }),
            (SyntaxKind::LET | SyntaxKind::REGION, []) => tree::expr_after(item, SyntaxKind::EQ)
                .map(Slot::Value)
                .ok_or_else(|| unaddressable(format!("the {keyword} has no value to replace"))),
            (_, []) => Err(unaddressable(format!(
                "a {keyword} has no single value; name one of its fields"
            ))),
            (
                SyntaxKind::PART | SyntaxKind::HARDWARE,
                [Segment::Member(place), Segment::Member(word)],
            ) if place == "place" => {
                let address =
                    TermAddress::from_word(word).ok_or_else(|| PatchErrorKind::UnknownMember {
                        item: format!("{name}.place"),
                        what: "placement axis",
                        name: word.clone(),
                    })?;
                if item.kind() == SyntaxKind::PART {
                    body("a place block")?;
                }
                let (terms, others): (Vec<SyntaxNode>, Vec<SyntaxNode>) = tree::block(item)
                    .and_then(|body| tree::place(&body))
                    .and_then(|place| tree::block(&place))
                    .map(|block| {
                        block
                            .children()
                            .filter(|child| child.kind() == SyntaxKind::CALL)
                            .partition(|term| address.matches(fixes(term)))
                    })
                    .unwrap_or_default();
                if terms.len() > 1 {
                    return Err(unaddressable(format!(
                        "{} terms of '{name}' fix '{word}'; a part has one term per axis, so remove \
                         the extra ones first",
                        terms.len()
                    )));
                }
                Ok(Slot::Term {
                    item: item.clone(),
                    term: terms.into_iter().next(),
                    address,
                    others,
                })
            }
            (SyntaxKind::PART | SyntaxKind::HARDWARE, [Segment::Member(place), ..])
                if place == "place" =>
            {
                Err(unaddressable(
                    "write the axis the term fixes: place.up, place.across, place.deep, or \
                     place.orient for the orient term"
                        .to_owned(),
                ))
            }
            (SyntaxKind::JOINT | SyntaxKind::HARDWARE, [Segment::Member(key)]) => {
                let args = tree::kind_call_args(item).ok_or_else(|| {
                    unaddressable(format!("the {keyword} has no kind call to change"))
                })?;
                let arg = args
                    .children()
                    .filter(|child| child.kind() == SyntaxKind::ARG)
                    .find(|arg| tree::arg_name(arg).as_deref() == Some(key.as_str()));
                Ok(Slot::Arg {
                    args,
                    name: key.clone(),
                    arg,
                })
            }
            (SyntaxKind::KIND, [Segment::Member(key)]) => {
                let block = body("fields")?;
                if tree::field(&block, key).is_none()
                    && let Some(param) = tree::item_named(&block, key)
                        .filter(|param| param.kind() == SyntaxKind::PARAM)
                {
                    return Ok(Slot::ParamDefault {
                        value: tree::expr_after(&param, SyntaxKind::EQ),
                        item: param,
                    });
                }
                field(key)
            }
            (
                SyntaxKind::PART
                | SyntaxKind::STOCK
                | SyntaxKind::SCENE
                | SyntaxKind::RECIPE
                | SyntaxKind::PROFILE
                | SyntaxKind::ASSEMBLY,
                [Segment::Member(key)],
            ) => field(key),
            _ => Err(unaddressable(format!(
                "a patch can set the default of a param, the value of a let or region, a field \
                 of an item, an argument of a joint or hardware, or a part's place.<axis>; this \
                 {keyword} has nothing at that path"
            ))),
        }
    }

    // ---- writing ----

    /// Applies edits to one file, lays out the lines they wrote, and
    /// refuses an edit that breaks a file which parsed.
    fn edit(&mut self, key: &FileKey, edits: Vec<Edit>) -> Result<(), PatchErrorKind> {
        if edits.is_empty() {
            return Ok(());
        }
        let state = self
            .files
            .get_mut(key)
            .ok_or_else(|| PatchErrorKind::UnknownFile(key.name.clone()))?;
        let (edited, touched) = text::apply(&state.text, edits).ok_or(PatchErrorKind::Conflict)?;
        let laid_out = relayout(&edited, &touched);
        let errors = state.set(laid_out);
        if !state.had_errors && !errors.is_empty() {
            return Err(PatchErrorKind::BreaksSyntax {
                file: key.name.clone(),
                diagnostics: errors,
            });
        }
        Ok(())
    }

    // ---- set and unset ----

    fn set(&mut self, path: &ast::Path, value: &ast::Expr) -> Result<(), PatchErrorKind> {
        let located = self.locate(path)?;
        let slot = self.slot(&located, path)?;
        let text = &self.state(&located.file)?.text;
        let value_text = self.snippet(value.span);
        if let Slot::Term {
            address, others, ..
        } = &slot
        {
            check_term(self.slice(path.span), &value_text, *address, others, text)?;
        }
        let replace = |node: &SyntaxNode| -> Vec<Edit> {
            let old = range(node);
            let old_text = text.get(old.clone()).unwrap_or("");
            if tokens(old_text) == tokens(&value_text) {
                return Vec::new();
            }
            let indent = indentation(text, old.start).to_owned();
            vec![Edit::replace(old, indent_lines(&value_text, &indent))]
        };
        let edits = match slot {
            Slot::ParamDefault {
                value: Some(node), ..
            }
            | Slot::Value(node)
            | Slot::Term {
                term: Some(node), ..
            } => replace(&node),
            Slot::Field {
                field: Some(field),
                key,
                ..
            } => match tree::expr_after(&field, SyntaxKind::COLON) {
                Some(node) => replace(&node),
                None => vec![Edit::replace(
                    range(&field),
                    format!(
                        "{key}: {}",
                        indent_lines(&value_text, indentation(text, range(&field).start))
                    ),
                )],
            },
            Slot::Arg { arg: Some(arg), .. } => match tree::arg_value(&arg) {
                Some(node) => replace(&node),
                None => Vec::new(),
            },
            Slot::ParamDefault { item, value: None } => {
                let names: Vec<SyntaxNode> = item
                    .children()
                    .filter(|child| child.kind() == SyntaxKind::NAME)
                    .collect();
                let after = names
                    .get(1)
                    .or(names.first())
                    .map_or(range(&item).end, |name| range(name).end);
                vec![Edit::insert(
                    after,
                    format!(" = {}", indent_lines(&value_text, indentation(text, after))),
                )]
            }
            Slot::Field {
                block,
                key,
                field: None,
            } => {
                let anchor = block
                    .children()
                    .filter(|child| child.kind() == SyntaxKind::FIELD)
                    .last();
                vec![add_entry(
                    text,
                    &block,
                    anchor.as_ref(),
                    &format!("{key}: {value_text}"),
                    false,
                )]
            }
            Slot::Arg {
                args,
                name,
                arg: None,
            } => vec![append_arg(text, &args, &format!("{name}: {value_text}"))],
            Slot::Term {
                item, term: None, ..
            } => vec![add_term(text, &item, &value_text)],
        };
        let key = located.file;
        self.edit(&key, edits)
    }

    fn unset(&mut self, path: &ast::Path) -> Result<(), PatchErrorKind> {
        let located = self.locate(path)?;
        let slot = self.slot(&located, path)?;
        let text = &self.state(&located.file)?.text;
        let path_text = self.slice(path.span).to_owned();
        let not_set = || PatchErrorKind::NotSet(path_text.clone());
        let edit = match slot {
            Slot::ParamDefault { item, value } => {
                let value = value.ok_or_else(not_set)?;
                let names: Vec<SyntaxNode> = item
                    .children()
                    .filter(|child| child.kind() == SyntaxKind::NAME)
                    .collect();
                let Some(ty) = names.get(1) else {
                    return Err(PatchErrorKind::Unaddressable {
                        path: path_text,
                        reason: "a param without a type needs its default; give it a type \
                                 first (param name : Type)"
                            .to_owned(),
                    });
                };
                Edit::replace(range(ty).end..range(&value).end, String::new())
            }
            Slot::Value(_) => {
                return Err(PatchErrorKind::Unaddressable {
                    path: path_text,
                    reason: "a let or region always has a value; set a new one, or remove the \
                             item"
                        .to_owned(),
                });
            }
            Slot::Field { field, .. } => remove_entry(text, range(&field.ok_or_else(not_set)?)),
            Slot::Arg { arg, .. } => remove_entry(text, range(&arg.ok_or_else(not_set)?)),
            Slot::Term { term, .. } => remove_entry(text, range(&term.ok_or_else(not_set)?)),
        };
        let key = located.file;
        self.edit(&key, vec![edit])
    }

    // ---- replace ----

    /// Replaces the item of the same kind and name at the top level of the
    /// project's module. The old item's doc comment stays unless the new
    /// item writes its own; a comment after its last line stays too.
    fn replace(&mut self, item: &ast::Item) -> Result<(), PatchErrorKind> {
        let entry = self.snippet(item.span);
        let Some(name) = item.kind.name().map(|name| name.as_str().to_owned()) else {
            return Err(PatchErrorKind::Unaddressable {
                path: entry.lines().next().unwrap_or("").to_owned(),
                reason: "replace finds the item to replace by its name, and this has none"
                    .to_owned(),
            });
        };
        let (key, old) = self.find_item(&ModuleKey::Project, &name).ok_or_else(|| {
            PatchErrorKind::UnknownItem {
                name: name.clone(),
                scope: Scope::Module(ModuleKey::Project),
            }
        })?;
        let found = tree::tag_of(old.kind());
        let wanted = item.kind.tag();
        if found != wanted {
            return Err(PatchErrorKind::WrongTag {
                path: name,
                expected: wanted.map_or("declaration", ItemTag::keyword),
                found: found.map_or("declaration", ItemTag::keyword),
            });
        }
        let text = &self.state(&key)?.text;
        let whole = range(&old);
        let old_range = if item.docs.is_empty() {
            tree::after_docs(&old).unwrap_or(whole.start)..whole.end
        } else {
            whole
        };
        let old_text = text.get(old_range.clone()).unwrap_or("");
        if tokens(old_text) == tokens(&entry) {
            return Ok(());
        }
        let indent = indentation(text, old_range.start).to_owned();
        let edit = Edit::replace(old_range, indent_lines(&entry, &indent));
        self.edit(&key, vec![edit])
    }

    // ---- remove ----

    fn remove(&mut self, tag: ItemTag, path: &ast::Path) -> Result<(), PatchErrorKind> {
        let located = self.locate(path)?;
        let path_text = self.slice(path.span).to_owned();
        if !located.rest.is_empty() {
            return Err(PatchErrorKind::Unaddressable {
                path: path_text,
                reason: "remove takes an item; to remove a field, an argument or a placement \
                         term, write unset"
                    .to_owned(),
            });
        }
        let found = tree::tag_of(located.item.kind());
        if found != Some(tag) {
            return Err(PatchErrorKind::WrongTag {
                path: path_text,
                expected: tag.keyword(),
                found: found.map_or("declaration", ItemTag::keyword),
            });
        }
        let text = &self.state(&located.file)?.text;
        let edit = remove_entry(text, range(&located.item));
        let key = located.file;
        self.edit(&key, vec![edit])
    }

    // ---- add ----

    fn add(
        &mut self,
        item: &ast::Item,
        to: Option<&ast::Ref>,
        file: Option<&ast::StringLit>,
    ) -> Result<(), PatchErrorKind> {
        let entry = self.snippet(item.span);
        let name = item.kind.name().map(|name| name.as_str().to_owned());
        let tag = item.kind.tag();
        let requested = file.map(|file| self.file_name(file)).transpose()?;
        if let Some(to) = to {
            return self.add_to_component(&entry, name, tag, to, requested);
        }
        if let Some(name) = &name
            && self.find_item(&ModuleKey::Project, name).is_some()
        {
            return Err(PatchErrorKind::AlreadyExists {
                name: name.clone(),
                scope: Scope::Module(ModuleKey::Project),
            });
        }
        let key = match requested {
            Some(name) => FileKey {
                module: ModuleKey::Project,
                name,
            },
            None => match self.referenced_file(item.span) {
                Some(key) => key,
                None => self.project_file()?,
            },
        };
        if let std::collections::btree_map::Entry::Vacant(vacant) = self.files.entry(key.clone()) {
            let text = format!("stave {FORMAT_VERSION}\n\n{entry}\n");
            let whole = [std::ops::Range {
                start: 0,
                end: text.len(),
            }];
            let mut state = FileState::new(None, text.clone());
            let errors = state.set(relayout(&text, &whole));
            if !errors.is_empty() {
                return Err(PatchErrorKind::BreaksSyntax {
                    file: key.name,
                    diagnostics: errors,
                });
            }
            vacant.insert(state);
            return Ok(());
        }
        let state = self.state(&key)?;
        let edit = add_item(&state.text, &state.root, tag, &entry);
        self.edit(&key, vec![edit])
    }

    fn add_to_component(
        &mut self,
        entry: &str,
        name: Option<String>,
        tag: Option<ItemTag>,
        to: &ast::Ref,
        requested: Option<String>,
    ) -> Result<(), PatchErrorKind> {
        let to_text = self.slice(to.span).to_owned();
        let root = to.root.as_str();
        let (key, component) = match to.rest.as_slice() {
            [] => self.find_item(&ModuleKey::Project, root).ok_or_else(|| {
                PatchErrorKind::UnknownItem {
                    name: root.to_owned(),
                    scope: Scope::Module(ModuleKey::Project),
                }
            })?,
            [inner] => {
                let module = ModuleKey::Local(root.to_owned());
                if !self.modules.contains(&module) {
                    return Err(PatchErrorKind::UnknownItem {
                        name: root.to_owned(),
                        scope: Scope::LocalModules,
                    });
                }
                self.find_item(&module, inner.as_str()).ok_or_else(|| {
                    PatchErrorKind::UnknownItem {
                        name: inner.as_str().to_owned(),
                        scope: Scope::Module(module.clone()),
                    }
                })?
            }
            _ => return Err(PatchErrorKind::NotAComponent(to_text)),
        };
        if component.kind() != SyntaxKind::COMPONENT {
            return Err(PatchErrorKind::NotAComponent(to_text));
        }
        let component_name = tree::item_name(&component).unwrap_or_default();
        if let Some(requested) = requested
            && (key.module != ModuleKey::Project || requested != key.name)
        {
            return Err(PatchErrorKind::ComponentElsewhere {
                component: to_text,
                actual: key.name,
                requested,
            });
        }
        let design = matches!(
            tag,
            Some(
                ItemTag::Param
                    | ItemTag::Let
                    | ItemTag::Region
                    | ItemTag::Part
                    | ItemTag::Joint
                    | ItemTag::Hardware
                    | ItemTag::Assembly
                    | ItemTag::Require
            )
        );
        if !design {
            return Err(PatchErrorKind::Unaddressable {
                path: to_text,
                reason: "a component holds only params, lets, regions, parts, joints, hardware, \
                         assemblies and requires"
                    .to_owned(),
            });
        }
        let block = tree::block(&component).ok_or_else(|| PatchErrorKind::Unaddressable {
            path: to_text.clone(),
            reason: "the component has no body".to_owned(),
        })?;
        if let Some(name) = name
            && tree::item_named(&block, &name).is_some()
        {
            return Err(PatchErrorKind::AlreadyExists {
                name,
                scope: Scope::Component(component_name),
            });
        }
        let text = &self.state(&key)?.text;
        let (anchor, blank) = item_anchor(&block, tag, entry);
        let edit = add_entry(text, &block, anchor.as_ref(), entry, blank);
        self.edit(&key, vec![edit])
    }

    /// A file name written after `in`.
    fn file_name(&self, file: &ast::StringLit) -> Result<String, PatchErrorKind> {
        let mut name = String::new();
        for part in &file.parts {
            match part {
                ast::StringPart::Text { text, .. } => name.push_str(text),
                ast::StringPart::Interp { .. } => {
                    return Err(PatchErrorKind::InvalidFileName(
                        self.slice(file.span).to_owned(),
                    ));
                }
            }
        }
        let plain = name.len() > ".stave".len()
            && name.ends_with(".stave")
            && !name.contains(['/', '\\'])
            && !name.starts_with('.');
        if plain {
            Ok(name)
        } else {
            Err(PatchErrorKind::InvalidFileName(name))
        }
    }

    /// The file of the first item of the project's module that the added
    /// item refers to, in the order it is written.
    fn referenced_file(&self, span: Span) -> Option<FileKey> {
        let added = self.patch_root.descendants().find(|node| {
            node.kind().is_item()
                && u32::from(node.text_range().start()) == span.start()
                && u32::from(node.text_range().end()) == span.end()
        })?;
        added
            .descendants()
            .filter(|node| matches!(node.kind(), SyntaxKind::NAME_REF | SyntaxKind::REF))
            .filter_map(|node| match node.kind() {
                SyntaxKind::REF => tree::ref_words(&node).into_iter().next(),
                _ => tree::word(&node),
            })
            .find_map(|name| self.find_item(&ModuleKey::Project, &name))
            .map(|(key, _)| key)
    }

    // ---- imports ----

    /// The file an import operation is about: the one named by `in`, or
    /// else the module's only file, or else the file holding `project`.
    fn import_file(&self, file: Option<&ast::StringLit>) -> Result<FileKey, PatchErrorKind> {
        if let Some(file) = file {
            let key = FileKey {
                module: ModuleKey::Project,
                name: self.file_name(file)?,
            };
            self.state(&key)?;
            return Ok(key);
        }
        let mut files = self.module_files(&ModuleKey::Project);
        match (files.next(), files.next()) {
            (Some((key, _)), None) => Ok(key.clone()),
            _ => self.project_file(),
        }
    }

    fn add_import(
        &mut self,
        import: &ast::Import,
        file: Option<&ast::StringLit>,
    ) -> Result<(), PatchErrorKind> {
        let key = self.import_file(file)?;
        let state = self.state(&key)?;
        let source = self.slice(import.source.span);
        let written = self.slice(import.span);
        let imports: Vec<SyntaxNode> = state
            .root
            .children()
            .filter(|child| child.kind() == SyntaxKind::IMPORT)
            .collect();
        for existing in &imports {
            let same_source = existing
                .children()
                .find(|child| child.kind() == SyntaxKind::STRING)
                .is_some_and(|string| string.text() == source);
            if same_source {
                if tokens(&existing.text().to_string()) == tokens(written) {
                    return Ok(());
                }
                return Err(PatchErrorKind::ImportExists {
                    import: source.to_owned(),
                    file: key.name,
                });
            }
        }
        let edit = match imports.last() {
            Some(last) => Edit::insert(
                line_end(&state.text, range(last).end),
                format!("\n{written}"),
            ),
            None => {
                let header = state
                    .root
                    .children()
                    .find(|child| child.kind() == SyntaxKind::HEADER)
                    .map_or(0, |header| line_end(&state.text, range(&header).end));
                Edit::insert(header, format!("\n\n{written}"))
            }
        };
        self.edit(&key, vec![edit])
    }

    fn remove_import(
        &mut self,
        source: &ast::StringLit,
        file: Option<&ast::StringLit>,
    ) -> Result<(), PatchErrorKind> {
        let source_text = self.slice(source.span).to_owned();
        let keys: Vec<FileKey> = match file {
            Some(_) => vec![self.import_file(file)?],
            None => self
                .module_files(&ModuleKey::Project)
                .map(|(key, _)| key.clone())
                .collect(),
        };
        let mut removed = false;
        for key in keys {
            let state = self.state(&key)?;
            let edits: Vec<Edit> = state
                .root
                .children()
                .filter(|child| child.kind() == SyntaxKind::IMPORT)
                .filter(|import| {
                    import
                        .children()
                        .find(|child| child.kind() == SyntaxKind::STRING)
                        .is_some_and(|string| string.text() == source_text.as_str())
                })
                .map(|import| remove_entry(&state.text, range(&import)))
                .collect();
            removed |= !edits.is_empty();
            self.edit(&key, edits)?;
        }
        if removed {
            Ok(())
        } else {
            Err(PatchErrorKind::UnknownImport {
                import: source_text,
                file: file.map(|file| self.slice(file.span).trim_matches('"').to_owned()),
            })
        }
    }
}

/// Refuses a placement term that doesn't fix what the path names, or that
/// also fixes an axis another term of the item fixes. A term whose axes
/// can't be read from its text (an `Anchor` param) is let through: the
/// patched design is checked again as a whole.
fn check_term(
    path: &str,
    term: &str,
    address: TermAddress,
    others: &[SyntaxNode],
    text: &str,
) -> Result<(), PatchErrorKind> {
    let new = term_text_fixes(term);
    if new == TermFixes::Unknown {
        return Ok(());
    }
    if !address.matches(new) {
        return Err(PatchErrorKind::WrongTerm {
            path: path.to_owned(),
            term: term.to_owned(),
            addressed: address,
            fixes: new,
        });
    }
    for other in others {
        let other_axes = fixes(other).axes();
        if let Some(axis) = new.axes().iter().find(|axis| other_axes.contains(axis)) {
            return Err(PatchErrorKind::TermClash {
                term: term.to_owned(),
                axis: *axis,
                other: text.get(range(other)).unwrap_or("").to_owned(),
            });
        }
    }
    Ok(())
}

/// Indents every line of `text` after the first by `indent`.
fn indent_lines(text: &str, indent: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(indent);
            }
        }
        out.push_str(line);
    }
    out
}

/// Where a new item goes among the items of a file or block: after the
/// last item with the same keyword, or else after the last item. A blank
/// line separates it, except between one-line items of the same keyword.
fn item_anchor(
    container: &SyntaxNode,
    tag: Option<ItemTag>,
    entry: &str,
) -> (Option<SyntaxNode>, bool) {
    let items: Vec<SyntaxNode> = tree::items(container).collect();
    let same = items
        .iter()
        .rev()
        .find(|item| tag.is_some() && tree::tag_of(item.kind()) == tag);
    match same {
        Some(anchor) => {
            let one_line = !anchor.text().to_string().contains('\n') && !entry.contains('\n');
            (Some(anchor.clone()), !one_line)
        }
        None => (items.last().cloned(), true),
    }
}

/// The edit that adds a top-level item to a file.
fn add_item(text: &str, root: &SyntaxNode, tag: Option<ItemTag>, entry: &str) -> Edit {
    let (anchor, blank) = item_anchor(root, tag, entry);
    let (anchor, blank) = match anchor {
        Some(anchor) => (Some(anchor), blank),
        None => (
            root.children()
                .filter(|child| matches!(child.kind(), SyntaxKind::IMPORT | SyntaxKind::HEADER))
                .last(),
            true,
        ),
    };
    let blank = if blank { "\n" } else { "" };
    match anchor {
        Some(anchor) => Edit::insert(
            line_end(text, range(&anchor).end),
            format!("\n{blank}{entry}"),
        ),
        None => {
            let separator = if text.is_empty() || text.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            Edit::insert(text.len(), format!("{separator}{entry}\n"))
        }
    }
}

/// The edit that adds an entry to a block: after `anchor` on a new line,
/// or at the top of the block without one. In a block written on one line
/// the entry joins that line after a comma.
fn add_entry(
    text: &str,
    block: &SyntaxNode,
    anchor: Option<&SyntaxNode>,
    entry: &str,
    blank: bool,
) -> Edit {
    let block_range = range(block);
    let open_end = block_range.start + 1;
    let close_start = block
        .last_token()
        .filter(|token| token.kind() == SyntaxKind::R_BRACE)
        .map_or(block_range.end, |token| {
            usize::from(token.text_range().start())
        });
    let inner = text.get(open_end..close_start).unwrap_or("");
    let base = indentation(text, block_range.start).to_owned();
    let inside = format!("{base}  ");
    if !inner.contains('\n') {
        if is_space(inner) {
            return if entry.contains('\n') {
                Edit::replace(
                    open_end..close_start,
                    format!("\n{inside}{}\n{base}", indent_lines(entry, &inside)),
                )
            } else {
                Edit::replace(open_end..close_start, format!(" {entry} "))
            };
        }
        if entry.contains('\n') {
            return explode(text, block, anchor, entry, blank, &base);
        }
        let last = anchor.cloned().or_else(|| block.children().last());
        let at = last.map_or(open_end, |node| range(&node).end);
        return Edit::insert(at, format!(", {}", indent_lines(entry, &inside)));
    }
    let blank = if blank { "\n" } else { "" };
    match anchor {
        Some(anchor) => {
            let anchor_range = range(anchor);
            let indent = indentation(text, anchor_range.start).to_owned();
            let end_of_line = line_end(text, anchor_range.end);
            let at = if close_start < end_of_line {
                anchor_range.end
            } else {
                end_of_line
            };
            Edit::insert(
                at,
                format!("\n{blank}{indent}{}", indent_lines(entry, &indent)),
            )
        }
        None => Edit::insert(
            line_end(text, open_end),
            format!("\n{inside}{}", indent_lines(entry, &inside)),
        ),
    }
}

/// The edit that adds a multi-line entry to a block written on one line:
/// the block is written out as the formatter lays out a multi-line block,
/// one entry to a line, with the new entry after `anchor` (after a blank
/// line when `blank` is set), or at the top without one.
fn explode(
    text: &str,
    block: &SyntaxNode,
    anchor: Option<&SyntaxNode>,
    entry: &str,
    blank: bool,
    base: &str,
) -> Edit {
    let block_range = range(block);
    let open_end = block_range.start + 1;
    let close_start = block
        .last_token()
        .filter(|token| token.kind() == SyntaxKind::R_BRACE)
        .map_or(block_range.end, |token| {
            usize::from(token.text_range().start())
        });
    let inside = format!("{base}  ");
    let entry = indent_lines(entry, &inside);
    let mut out = String::new();
    if anchor.is_none() {
        out.push('\n');
        out.push_str(&inside);
        out.push_str(&entry);
    }
    for child in block.children() {
        out.push('\n');
        out.push_str(&inside);
        out.push_str(text.get(range(&child)).unwrap_or("").trim());
        if anchor == Some(&child) {
            out.push_str(if blank { "\n\n" } else { "\n" });
            out.push_str(&inside);
            out.push_str(&entry);
        }
    }
    out.push('\n');
    out.push_str(base);
    Edit::replace(open_end..close_start, out)
}

/// The edit that adds a named argument at the end of an argument list.
fn append_arg(text: &str, args: &SyntaxNode, arg: &str) -> Edit {
    let list = range(args);
    let close_start = args
        .last_token()
        .filter(|token| token.kind() == SyntaxKind::R_PAREN)
        .map_or(list.end, |token| usize::from(token.text_range().start()));
    let Some(last) = args
        .children()
        .filter(|child| child.kind() == SyntaxKind::ARG)
        .last()
    else {
        return Edit::insert(close_start, arg.to_owned());
    };
    let last_range = range(&last);
    let between = text.get(last_range.end..close_start).unwrap_or("");
    let indent = indentation(text, last_range.start).to_owned();
    if between.contains('\n')
        && let Some(comma) = between.find(',')
    {
        let comma_end = last_range.end + comma + 1;
        let end_of_line = line_end(text, comma_end);
        let at = if end_of_line < close_start {
            end_of_line
        } else {
            comma_end
        };
        return Edit::insert(at, format!("\n{indent}{},", indent_lines(arg, &indent)));
    }
    Edit::insert(last_range.end, format!(", {}", indent_lines(arg, &indent)))
}

/// The edit that adds a placement term to a part or hardware that has no
/// term for that axis: into its `place` block, adding the block (and a
/// hardware's body) when it is missing.
fn add_term(text: &str, item: &SyntaxNode, term: &str) -> Edit {
    let body = tree::block(item);
    if let Some(place) = body.as_ref().and_then(tree::place)
        && let Some(block) = tree::block(&place)
    {
        let anchor = block
            .children()
            .filter(|child| child.kind() == SyntaxKind::CALL)
            .last();
        return add_entry(text, &block, anchor.as_ref(), term, false);
    }
    let place = format!("place {{\n  {}\n}}", indent_lines(term, "  "));
    match body {
        Some(body) => {
            let anchor = body.children().last();
            let blank = anchor.is_some();
            add_entry(text, &body, anchor.as_ref(), &place, blank)
        }
        None => {
            let indent = indentation(text, range(item).start).to_owned();
            let inside = format!("{indent}  ");
            Edit::insert(
                range(item).end,
                format!(" {{\n{inside}{}\n{indent}}}", indent_lines(&place, &inside)),
            )
        }
    }
}

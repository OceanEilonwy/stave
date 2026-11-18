//! The depth-first module walk behind [`ModuleSet::load_with`].

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use stave_lang::ast::{Import, Item, ItemKind, Span, StringLit, StringPart, is_word};
use stave_lang::diagnostics::{SourceFile, SourceMapError};
use stave_lang::{
    Code, Diagnostic, FileId, Label, SourceMap, is_keyword, is_patch_source, parse_file,
    parse_patch_file,
};

use crate::source::{FileSource, is_stave_path, normalize};
use crate::std_library::{STD_MODULES, module_files, std_module};
use crate::{
    FileImport, LoadError, Module, ModuleFile, ModuleHeader, ModuleId, ModuleSet, ModuleSource,
};

/// Loads the project in `folder`. See the crate docs for the rules and the
/// order.
pub(crate) fn load(
    folder: &Path,
    files: &dyn FileSource,
    prior: Option<&ModuleSet>,
) -> Result<ModuleSet, LoadError> {
    let canonical = files
        .canonical(folder)
        .map_err(|source| root_folder_error(folder, source))?;
    let texts = read_folder(files, folder).map_err(|error| match error {
        FolderError::Unlisted(source) => root_folder_error(folder, source),
        FolderError::Unreadable(path, source) => LoadError::UnreadableFile { path, source },
    })?;
    if texts.is_empty() {
        return Err(LoadError::NoStaveFiles {
            folder: folder.to_path_buf(),
        });
    }

    let mut loader = Loader::new(files, prior);
    let standard = STD_MODULES
        .into_iter()
        .find(|module| is_same_as_std(&texts, module));
    let (name, source) = match standard {
        Some(module) => (module.to_owned(), ModuleSource::Std { name: module }),
        None => (
            folder_name(&canonical, folder),
            ModuleSource::Local {
                folder: canonical.clone(),
            },
        ),
    };
    let root = loader.add_module(name, source, Some(folder.to_path_buf()), folder, texts)?;
    if let Some(module) = standard {
        loader.by_std.insert(module, root);
    }
    loader.by_folder.insert(canonical, root);
    loader.visit(root)?;

    let mut prelude = Vec::with_capacity(STD_MODULES.len());
    for module in STD_MODULES {
        prelude.push(loader.std_module(module)?);
    }
    Ok(loader.finish(root, prelude))
}

/// The [`LoadError`] for a root folder that can't be listed.
fn root_folder_error(folder: &Path, source: io::Error) -> LoadError {
    if source.kind() == io::ErrorKind::NotADirectory {
        LoadError::NotAFolder {
            folder: folder.to_path_buf(),
        }
    } else {
        LoadError::Unreadable {
            folder: folder.to_path_buf(),
            source,
        }
    }
}

/// One `.stave` file of a folder, read but not yet added to the source map.
struct FolderFile {
    /// The file name, without the folder.
    name: String,
    /// The text.
    text: String,
}

/// Why a folder's files can't be read.
enum FolderError {
    /// The folder can't be listed (missing, not a folder, no permission).
    Unlisted(io::Error),
    /// A file in it can't be read.
    Unreadable(PathBuf, io::Error),
}

/// Reads every `.stave` file directly in `folder`, sorted by file name.
fn read_folder(files: &dyn FileSource, folder: &Path) -> Result<Vec<FolderFile>, FolderError> {
    let paths = files
        .list_stave_files(folder)
        .map_err(FolderError::Unlisted)?;
    let mut read = Vec::with_capacity(paths.len());
    for path in paths {
        if !is_stave_path(&path) {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        let name = name.to_string_lossy().into_owned();
        let text = files
            .read(&path)
            .map_err(|source| FolderError::Unreadable(path.clone(), source))?;
        read.push(FolderFile { name, text });
    }
    read.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(read)
}

/// `true` if `texts` has exactly the file names and texts of the standard
/// module `module`.
fn is_same_as_std(texts: &[FolderFile], module: &str) -> bool {
    let standard: Vec<(&str, &str)> = module_files(module).collect();
    texts.len() == standard.len()
        && texts
            .iter()
            .zip(&standard)
            .all(|(file, (name, text))| file.name == *name && file.text == *text)
}

/// A module's default name: the folder's last segment.
fn folder_name(canonical: &Path, folder: &Path) -> String {
    let normalized = normalize(folder);
    canonical
        .file_name()
        .or_else(|| normalized.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("project"))
}

/// What an import's source names.
enum SourceKind<'a> {
    /// `std:<name>`.
    Std(&'a str),
    /// `github:...`, `https://...` or any other `scheme:`.
    Remote,
    /// A folder relative to the importing file.
    Relative,
}

/// Classifies an import source by its `scheme:` prefix, if any.
fn classify(source: &str) -> SourceKind<'_> {
    match source.split_once(':') {
        Some(("std", name)) => SourceKind::Std(name),
        Some((scheme, _)) if is_scheme(scheme) => SourceKind::Remote,
        Some(_) | None => SourceKind::Relative,
    }
}

/// `true` if `text` is a URL scheme: a letter, then letters, digits, `+`,
/// `-` or `.`.
fn is_scheme(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The source string's text, or `None` if it contains `{name}` parts.
fn plain_text(literal: &StringLit) -> Option<String> {
    let mut text = String::new();
    for part in &literal.parts {
        match part {
            StringPart::Text { text: piece, .. } => text.push_str(piece),
            StringPart::Interp { .. } => return None,
        }
    }
    Some(text)
}

/// A name to suggest after `as` for a module called `name`.
fn suggested_alias(name: &str) -> String {
    let word: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let word = word.trim_matches('_').to_owned();
    let usable = |candidate: &str| is_word(candidate) && !is_keyword(candidate);
    if usable(&word) {
        return word;
    }
    // A keyword: its first letter and the letter half way along
    // (`hardware` -> `hw`).
    let letters: Vec<char> = word.chars().filter(char::is_ascii_alphabetic).collect();
    let short: String = letters
        .first()
        .into_iter()
        .chain(letters.get(letters.len() / 2))
        .collect();
    if usable(&short) {
        short
    } else {
        String::from("lib")
    }
}

/// The label span of a `project` or `library` item: its title.
fn header_span(item: &Item) -> Span {
    match &item.kind {
        ItemKind::Project(project) => project.title.span,
        ItemKind::Library(library) => library.title.span,
        ItemKind::Kind(_)
        | ItemKind::Recipe(_)
        | ItemKind::Stock(_)
        | ItemKind::Profile(_)
        | ItemKind::Component(_)
        | ItemKind::Scene(_)
        | ItemKind::Design(_) => item.span,
    }
}

/// Where a module is in the depth-first walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visit {
    /// Added, its imports not yet followed.
    New,
    /// Its imports are being followed: it is on the stack.
    InProgress,
    /// All its imports have been followed.
    Done,
}

/// Loader bookkeeping per module, indexed like `modules`.
struct ModuleState {
    /// The folder relative imports are resolved against, as displayed;
    /// `None` for an embedded standard module.
    folder: Option<PathBuf>,
    /// Where the module is in the walk.
    visit: Visit,
}

/// The state of one load.
struct Loader<'a> {
    files: &'a dyn FileSource,
    sources: SourceMap,
    modules: Vec<Module>,
    states: Vec<ModuleState>,
    diagnostics: Vec<Diagnostic>,
    /// Local modules by canonical folder.
    by_folder: BTreeMap<PathBuf, ModuleId>,
    /// Standard modules by name.
    by_std: BTreeMap<&'static str, ModuleId>,
    /// The modules whose imports are being followed, outermost first.
    stack: Vec<ModuleId>,
    /// A previous load whose parses may be reused ([`Loader::reusable`]).
    prior: Option<Prior<'a>>,
}

/// A previous load: its sources and its parsed files by id.
struct Prior<'a> {
    sources: &'a SourceMap,
    /// Its module files that had no diagnostic at all, by id.
    clean: BTreeMap<FileId, &'a ModuleFile>,
}

impl<'a> Prior<'a> {
    fn of(modules: &'a ModuleSet) -> Self {
        let mut clean: BTreeMap<FileId, &'a ModuleFile> = modules
            .modules
            .iter()
            .flat_map(|module| &module.files)
            .map(|file| (file.file, file))
            .collect();
        for diagnostic in &modules.diagnostics {
            for label in &diagnostic.labels {
                clean.remove(&label.file);
            }
        }
        Prior {
            sources: &modules.sources,
            clean,
        }
    }
}

impl<'a> Loader<'a> {
    fn new(files: &'a dyn FileSource, prior: Option<&'a ModuleSet>) -> Self {
        Self {
            prior: prior.map(Prior::of),
            files,
            sources: SourceMap::new(),
            modules: Vec::new(),
            states: Vec::new(),
            diagnostics: Vec::new(),
            by_folder: BTreeMap::new(),
            by_std: BTreeMap::new(),
            stack: Vec::new(),
        }
    }

    /// Sorts the diagnostics and builds the result.
    fn finish(mut self, root: ModuleId, prelude: Vec<ModuleId>) -> ModuleSet {
        self.diagnostics.sort_by_key(|diagnostic| {
            let primary = diagnostic.primary();
            (
                primary.is_none(),
                primary.map(|label| (label.file, label.span.start(), label.span.end())),
            )
        });
        ModuleSet {
            sources: self.sources,
            modules: self.modules,
            root,
            prelude,
            diagnostics: self.diagnostics,
        }
    }

    fn module_name(&self, id: ModuleId) -> &str {
        self.modules
            .get(id.index())
            .map_or("", |module| module.name.as_str())
    }

    /// The parse of a file the previous load parsed without any
    /// diagnostic, when the file has the same id, display path and text:
    /// parsing is a function of those, so parsing again would give the
    /// same tree and again no diagnostic.
    fn reusable(&self, file: FileId, path: &str, text: &str) -> Option<ModuleFile> {
        let prior = self.prior.as_ref()?;
        let before = prior.clean.get(&file)?;
        let same_text = prior
            .sources
            .get(file)
            .is_some_and(|source| source.path() == path && source.text() == text);
        (same_text && before.path == path).then(|| ModuleFile {
            file,
            path: path.to_owned(),
            ast: before.ast.clone(),
            imports: Vec::new(),
            broken: before.broken.clone(),
        })
    }

    /// Adds a module's files to the source map, parses them and checks its
    /// header. Its imports are followed later, by [`Loader::visit`].
    fn add_module(
        &mut self,
        name: String,
        source: ModuleSource,
        folder: Option<PathBuf>,
        display_folder: &Path,
        texts: Vec<FolderFile>,
    ) -> Result<ModuleId, SourceMapError> {
        let id =
            ModuleId(u32::try_from(self.modules.len()).map_err(|_| SourceMapError::TooManyFiles)?);
        let mut files = Vec::with_capacity(texts.len());
        let mut first_file = None;
        for FolderFile { name, text } in texts {
            let path = display_folder.join(&name).display().to_string();
            let file = self.sources.add(path.clone(), text)?;
            first_file.get_or_insert(file);
            let text = self.sources.get(file).map_or("", SourceFile::text);
            if let Some(reused) = self.reusable(file, &path, text) {
                files.push(reused);
            } else if is_patch_source(text) {
                let parse = parse_patch_file(file, text);
                self.diagnostics.extend(parse.diagnostics);
            } else {
                let parse = parse_file(file, text);
                self.diagnostics.extend(parse.diagnostics);
                if let Some((version, header_span)) = parse
                    .ast
                    .as_ref()
                    .map(|ast| (ast.header.version, ast.header.span))
                    && version != stave_lang::FORMAT_VERSION
                {
                    self.diagnostics.push(
                        Diagnostic::error(
                            Code::NotSupported,
                            format!("format version {version} is not supported yet"),
                            Label::new(file, header_span),
                        )
                        .with_help(format!(
                            "this version of Stave reads files that start with 'stave {}'",
                            stave_lang::FORMAT_VERSION
                        )),
                    );
                }
                files.push(ModuleFile {
                    file,
                    path,
                    ast: parse.ast,
                    imports: Vec::new(),
                    broken: parse.broken,
                });
            }
        }
        let header = self.header(&name, &files, first_file);
        self.modules.push(Module {
            id,
            name,
            source,
            header,
            files,
        });
        self.states.push(ModuleState {
            folder,
            visit: Visit::New,
        });
        Ok(id)
    }

    /// Finds the module's single `project` or `library` item, diagnosing
    /// none or several.
    fn header(
        &mut self,
        name: &str,
        files: &[ModuleFile],
        first_file: Option<FileId>,
    ) -> Option<ModuleHeader> {
        let headers: Vec<(FileId, usize, &Item)> = files
            .iter()
            .flat_map(|file| {
                file.ast.iter().flat_map(move |ast| {
                    ast.items
                        .iter()
                        .enumerate()
                        .filter(|(_, item)| {
                            matches!(item.kind, ItemKind::Project(_) | ItemKind::Library(_))
                        })
                        .map(move |(index, item)| (file.file, index, item))
                })
            })
            .collect();
        // A project or library item with a syntax error is already
        // reported: it is there, just broken.
        let broken_header = files.iter().any(|file| {
            let text = self.sources.get(file.file).map_or("", SourceFile::text);
            file.broken.iter().any(|item| {
                let start = usize::try_from(item.span.start()).unwrap_or(usize::MAX);
                text.get(start..).is_some_and(|rest| {
                    // Past doc comments and `private`, the item's keyword.
                    let keyword = rest
                        .lines()
                        .map(str::trim_start)
                        .find(|line| !line.is_empty() && !line.starts_with("///"))
                        .unwrap_or_default();
                    let keyword = keyword
                        .strip_prefix("private")
                        .map_or(keyword, str::trim_start);
                    keyword.starts_with("project") || keyword.starts_with("library")
                })
            })
        });
        match headers.as_slice() {
            [] if broken_header => None,
            [] => {
                let label = files
                    .first()
                    .map(|file| {
                        let span = file
                            .ast
                            .as_ref()
                            .map_or(Span::new(0, 0), |ast| ast.header.span);
                        Label::new(file.file, span)
                    })
                    .or_else(|| first_file.map(|file| Label::new(file, Span::new(0, 0))));
                if let Some(label) = label {
                    self.diagnostics.push(
                        Diagnostic::error(
                            Code::MissingField,
                            format!("module '{name}' is missing 'project' or 'library'"),
                            label,
                        )
                        .with_help(
                            "add 'project \"Title\" { }' to one file of a project, or \
                             'library \"Title\"' to one file of a library",
                        ),
                    );
                }
                None
            }
            [(file, index, item)] => Some(match item.kind {
                ItemKind::Library(_) => ModuleHeader::Library {
                    file: *file,
                    item: *index,
                },
                ItemKind::Project(_)
                | ItemKind::Kind(_)
                | ItemKind::Recipe(_)
                | ItemKind::Stock(_)
                | ItemKind::Profile(_)
                | ItemKind::Component(_)
                | ItemKind::Scene(_)
                | ItemKind::Design(_) => ModuleHeader::Project {
                    file: *file,
                    item: *index,
                },
            }),
            [(first_file, _, first), rest @ ..] => {
                for (file, _, item) in rest {
                    let keyword = item.kind.keyword();
                    self.diagnostics.push(
                        Diagnostic::error(
                            Code::DuplicateName,
                            format!("'{keyword}' is declared twice"),
                            Label::new(*file, header_span(item)),
                        )
                        .with_label(Label::with_message(
                            *first_file,
                            header_span(first),
                            format!("the module's '{}' is here", first.kind.keyword()),
                        ))
                        .with_help(
                            "a module has exactly one 'project' or 'library' item, in any \
                             one of its files; remove the other",
                        ),
                    );
                }
                None
            }
        }
    }

    /// The first declaration of every item name in the module.
    fn item_names(&self, id: ModuleId) -> BTreeMap<String, (FileId, Span)> {
        let mut names = BTreeMap::new();
        let Some(module) = self.modules.get(id.index()) else {
            return names;
        };
        for file in &module.files {
            let Some(ast) = &file.ast else {
                continue;
            };
            for item in &ast.items {
                if let Some(name) = item.kind.name() {
                    names
                        .entry(name.as_str().to_owned())
                        .or_insert((file.file, name.span()));
                }
            }
        }
        names
    }

    fn set_visit(&mut self, id: ModuleId, visit: Visit) {
        if let Some(state) = self.states.get_mut(id.index()) {
            state.visit = visit;
        }
    }

    /// Follows every import of the module, depth first.
    fn visit(&mut self, id: ModuleId) -> Result<(), SourceMapError> {
        if self.states.get(id.index()).map(|state| state.visit) != Some(Visit::New) {
            return Ok(());
        }
        self.set_visit(id, Visit::InProgress);
        self.stack.push(id);
        let item_names = self.item_names(id);
        let file_count = self.modules.get(id.index()).map_or(0, |m| m.files.len());
        for index in 0..file_count {
            let Some((file, imports)) = self
                .modules
                .get(id.index())
                .and_then(|module| module.files.get(index))
                .map(|file| {
                    let imports = file
                        .ast
                        .as_ref()
                        .map(|ast| ast.imports.clone())
                        .unwrap_or_default();
                    (file.file, imports)
                })
            else {
                continue;
            };
            let mut used = BTreeMap::new();
            let mut resolved = Vec::new();
            for import in &imports {
                if let Some(import) = self.resolve(id, file, import, &mut used, &item_names)? {
                    resolved.push(import);
                }
            }
            if let Some(module_file) = self
                .modules
                .get_mut(id.index())
                .and_then(|module| module.files.get_mut(index))
            {
                module_file.imports = resolved;
            }
        }
        self.stack.pop();
        self.set_visit(id, Visit::Done);
        Ok(())
    }

    /// Resolves one import of `module`'s file `file`, loading the module it
    /// names if needed. `used` holds the names this file's earlier imports
    /// took.
    fn resolve(
        &mut self,
        module: ModuleId,
        file: FileId,
        import: &Import,
        used: &mut BTreeMap<String, Span>,
        item_names: &BTreeMap<String, (FileId, Span)>,
    ) -> Result<Option<FileImport>, SourceMapError> {
        let source_span = import.source.span;
        let Some(source) = plain_text(&import.source) else {
            let written = self
                .sources
                .get(file)
                .and_then(|text| {
                    let start = usize::try_from(source_span.start()).ok()?;
                    let end = usize::try_from(source_span.end()).ok()?;
                    text.text().get(start..end)
                })
                .unwrap_or_default()
                .trim_matches('"')
                .to_owned();
            self.diagnostics.push(
                Diagnostic::error(
                    Code::UnknownModule,
                    format!("can't find module '{written}'"),
                    Label::new(file, source_span),
                )
                .with_help("write the folder as plain text, without {names}"),
            );
            return Ok(None);
        };

        let target = match classify(&source) {
            SourceKind::Remote => {
                self.diagnostics.push(
                    Diagnostic::error(
                        Code::NotSupported,
                        format!("importing from '{source}' is not supported yet"),
                        Label::new(file, source_span),
                    )
                    .with_help(
                        "only folders on this computer and std: modules can be imported in \
                         this version",
                    ),
                );
                return Ok(None);
            }
            SourceKind::Std(name) => match std_module(name) {
                Some(name) => self.std_module(name)?,
                None => {
                    self.diagnostics.push(
                        Diagnostic::error(
                            Code::UnknownModule,
                            format!("can't find module '{source}'"),
                            Label::new(file, source_span),
                        )
                        .with_help(
                            "the standard modules are std:joinery, std:hardware and std:tools",
                        ),
                    );
                    return Ok(None);
                }
            },
            SourceKind::Relative => match self.local_module(module, file, &source, source_span)? {
                Ok(target) => target,
                Err(diagnostic) => {
                    self.diagnostics.push(diagnostic);
                    return Ok(None);
                }
            },
        };

        if let Some(position) = self.stack.iter().position(|id| *id == target) {
            let names: Vec<&str> = std::iter::once(module)
                .chain(
                    self.stack
                        .get(position..)
                        .unwrap_or_default()
                        .iter()
                        .copied(),
                )
                .map(|id| self.module_name(id))
                .collect();
            self.diagnostics.push(
                Diagnostic::error(
                    Code::ImportCycle,
                    format!("modules import each other: {}", names.join(" -> ")),
                    Label::new(file, import.span),
                )
                .with_help(
                    "modules can't import each other in a circle; move what both need \
                     into a third module that both import",
                ),
            );
        }

        let (name, name_span) = match &import.alias {
            Some(alias) => (alias.as_str().to_owned(), alias.span()),
            None => {
                let default = self.module_name(target).to_owned();
                let message = if is_keyword(&default) {
                    format!("'{default}' is a reserved word and can't be used as a name")
                } else if !is_word(&default) {
                    format!("'{default}' can't be used as a name")
                } else {
                    String::new()
                };
                if !message.is_empty() {
                    self.diagnostics.push(
                        Diagnostic::error(
                            Code::ReservedWord,
                            message,
                            Label::new(file, source_span),
                        )
                        .with_help(format!(
                            "give the module another name: import \"{source}\" as {}",
                            suggested_alias(&default)
                        )),
                    );
                    return Ok(None);
                }
                (default, source_span)
            }
        };

        if let Some(first) = used.get(&name) {
            self.diagnostics.push(
                Diagnostic::error(
                    Code::DuplicateName,
                    format!("'{name}' is declared twice"),
                    Label::new(file, name_span),
                )
                .with_label(Label::with_message(file, *first, "first declared here"))
                .with_help(format!(
                    "give one of the imports another name with 'as', or remove it: \
                     import \"{source}\" as {name}2"
                )),
            );
            return Ok(None);
        }
        if let Some((item_file, item_span)) = item_names.get(&name) {
            self.diagnostics.push(
                Diagnostic::error(
                    Code::DuplicateName,
                    format!("'{name}' is declared twice"),
                    Label::new(file, name_span),
                )
                .with_label(Label::with_message(
                    *item_file,
                    *item_span,
                    "also declared here",
                ))
                .with_help(format!(
                    "a module name can't equal an item name; give the module another \
                     name: import \"{source}\" as {name}_module"
                )),
            );
            return Ok(None);
        }
        used.insert(name.clone(), name_span);
        Ok(Some(FileImport {
            name,
            name_span,
            module: target,
            span: import.span,
        }))
    }

    /// The standard module `name`, loading it (and following its imports)
    /// the first time.
    fn std_module(&mut self, name: &'static str) -> Result<ModuleId, SourceMapError> {
        if let Some(id) = self.by_std.get(name) {
            return Ok(*id);
        }
        let texts = module_files(name)
            .map(|(file, text)| FolderFile {
                name: file.to_owned(),
                text: text.to_owned(),
            })
            .collect();
        let display_folder = Path::new("std").join(name);
        let id = self.add_module(
            name.to_owned(),
            ModuleSource::Std { name },
            None,
            &display_folder,
            texts,
        )?;
        self.by_std.insert(name, id);
        self.visit(id)?;
        Ok(id)
    }

    /// The local module `source` names, relative to `module`'s folder,
    /// loading it (and following its imports) the first time. The inner
    /// `Err` is the diagnostic for a folder that can't be found.
    fn local_module(
        &mut self,
        module: ModuleId,
        file: FileId,
        source: &str,
        span: Span,
    ) -> Result<Result<ModuleId, Diagnostic>, SourceMapError> {
        let not_found = |help: String| {
            Diagnostic::error(
                Code::UnknownModule,
                format!("can't find module '{source}'"),
                Label::new(file, span),
            )
            .with_help(help)
        };
        let Some(base) = self
            .states
            .get(module.index())
            .and_then(|state| state.folder.clone())
        else {
            return Ok(Err(not_found(String::from(
                "a standard module can only import other std: modules",
            ))));
        };
        let folder = normalize(&base.join(source));
        let Ok(canonical) = self.files.canonical(&folder) else {
            return Ok(Err(not_found(format!(
                "there is no folder at '{}'",
                folder.display()
            ))));
        };
        if let Some(id) = self.by_folder.get(&canonical) {
            return Ok(Ok(*id));
        }
        let texts = match read_folder(self.files, &folder) {
            Ok(texts) if texts.is_empty() => {
                return Ok(Err(not_found(format!(
                    "there are no .stave files in '{}'",
                    folder.display()
                ))));
            }
            Ok(texts) => texts,
            Err(FolderError::Unlisted(error)) => {
                return Ok(Err(not_found(format!(
                    "looked for a folder at '{}', but it can't be read: {error}",
                    folder.display()
                ))));
            }
            Err(FolderError::Unreadable(path, error)) => {
                return Ok(Err(not_found(format!(
                    "found the folder '{}', but '{}' can't be read: {error}",
                    folder.display(),
                    path.display()
                ))));
            }
        };
        let name = folder_name(&canonical, &folder);
        let id = self.add_module(
            name,
            ModuleSource::Local {
                folder: canonical.clone(),
            },
            Some(folder.clone()),
            &folder,
            texts,
        )?;
        self.by_folder.insert(canonical, id);
        self.visit(id)?;
        Ok(Ok(id))
    }
}

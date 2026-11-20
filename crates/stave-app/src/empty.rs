//! What the 3D view says when it has nothing to draw: still checking, a
//! folder that can't be read, a design whose errors stop it solving, a
//! library of kinds with no parts, or a project without parts yet. The
//! view never shows a blank area; it shows one of these. The parts
//! sidebar and the inspector say what is true for the same state: a
//! library's sidebar lists what the library defines ([`Library`]) and its
//! inspector says what a library folder is and how a project uses it
//! ([`EmptyState::inspector`], [`EmptyState::sidebar`]).

use crate::i18n::{fill_in, ntr, tr, tr_noop};
use stave_lang::ast::BaseKind;
use stave_model::defs::{Decl, DefId};

use crate::check::Checked;
use crate::session::{LineNumber, Session, SourceLocation, Verdict};

/// Why there is nothing to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// The first check hasn't finished.
    Checking,
    /// The folder couldn't be read.
    CantOpen,
    /// The design has errors and never solved.
    Errors,
    /// The folder is a library: kinds and components, nothing placed.
    Library,
    /// The project places no parts yet.
    NoParts,
}

impl Reason {
    /// Why the build button is off, in words: "Nothing to build: a
    /// library has no parts".
    pub fn nothing_to_build(self) -> String {
        match self {
            Reason::Checking => tr("Nothing to build yet: the design is still being checked"),
            Reason::CantOpen => tr("Nothing to build: the folder couldn't be opened"),
            Reason::Errors => tr("Nothing to build yet: fix the problems first"),
            Reason::Library => tr("Nothing to build: a library has no parts"),
            Reason::NoParts => tr("Nothing to build: the project has no parts yet"),
        }
    }
}

/// The view's empty state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyState {
    /// Why.
    pub reason: Reason,
    /// "Nothing to show".
    pub title: String,
    /// What it means and what to do.
    pub description: String,
    /// Whether to offer "Open a project…".
    pub offers_open: bool,
    /// What a library folder defines, for [`Reason::Library`].
    pub library: Option<Library>,
}

/// What the inspector shows when there is nothing to select: a title, a
/// sentence or two, and for a library the line that imports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// "A library folder".
    pub title: String,
    /// What it means and what to do.
    pub description: String,
    /// A line of Stave text to show as code: `import "../x" as x`.
    pub code: Option<String>,
    /// What follows the code.
    pub after: Option<String>,
}

/// What a library folder defines, for the parts sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// Its `library "..."` title.
    pub title: String,
    /// Its `about:`.
    pub about: Option<String>,
    /// The name a project would import it under: the folder's name.
    pub import_name: String,
    /// What it defines, by group in [`LibraryGroup`] order, each in source
    /// order; groups with nothing in them are left out.
    pub groups: Vec<(LibraryGroup, Vec<LibraryItem>)>,
}

/// A group of what a library defines, in the order the sidebar lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LibraryGroup {
    /// `kind x : joint`.
    Joints,
    /// `kind x : hinge`, `slide` or `pull`.
    Hardware,
    /// `kind x : fastener`.
    Fasteners,
    /// `kind x : tool`.
    Tools,
    /// `recipe`s: ways to mark and cut.
    Recipes,
    /// `component`s: parts that go together, to place in a project.
    Components,
    /// `stock`s.
    Stock,
    /// `profile`s.
    Profiles,
}

impl LibraryGroup {
    /// The sidebar's heading: "JOINT KINDS".
    pub fn heading(self) -> String {
        match self {
            LibraryGroup::Joints => tr("JOINT KINDS"),
            LibraryGroup::Hardware => tr("HARDWARE KINDS"),
            LibraryGroup::Fasteners => tr("FASTENERS"),
            LibraryGroup::Tools => tr("TOOLS"),
            LibraryGroup::Recipes => tr("RECIPES"),
            LibraryGroup::Components => tr("COMPONENTS"),
            LibraryGroup::Stock => tr("STOCK"),
            LibraryGroup::Profiles => tr("PROFILES"),
        }
    }
}

/// One thing a library defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryItem {
    /// Its name: `ari_tsugi`.
    pub name: String,
    /// The first line of its doc comment, if it has one.
    pub summary: Option<String>,
    /// The line its name is on (the Files view opens there).
    pub location: Option<SourceLocation>,
}

impl EmptyState {
    /// What the inspector says, since there is nothing to select.
    pub fn inspector(&self) -> Note {
        let note = |title: &str, description: &str| Note {
            title: tr(title),
            description: if description.is_empty() {
                String::new()
            } else {
                tr(description)
            },
            code: None,
            after: None,
        };
        match self.reason {
            Reason::Checking => note(tr_noop("Checking…"), ""),
            Reason::CantOpen => note(
                tr_noop("Nothing to inspect"),
                tr_noop("Open a project folder to see its parts here."),
            ),
            Reason::Errors => note(
                tr_noop("Nothing to inspect yet"),
                tr_noop(
                    "Once the errors are fixed, click a part in the list or in the 3D view to see its stock, size, joints and where it is written.",
                ),
            ),
            Reason::Library => self.library_note(),
            Reason::NoParts => note(
                tr_noop("No parts yet"),
                tr_noop(
                    "Add a part to the project's text; then click it in the list or in the 3D view to see it here.",
                ),
            ),
        }
    }

    /// What a library folder is and how a project uses this one.
    fn library_note(&self) -> Note {
        let name = self
            .library
            .as_ref()
            .map_or("library", |library| library.import_name.as_str());
        let example = self
            .library
            .as_ref()
            .and_then(|library| library.groups.first())
            .and_then(|(_, items)| items.first())
            .map_or("a_kind", |item| item.name.as_str());
        Note {
            title: tr("A library folder"),
            description: tr(
                "A library holds kinds of joints and hardware, recipes and components for projects to use. It has no parts of its own, so there is nothing to select or build here. To use it, import it at the top of a project file:",
            ),
            code: Some(format!("import \"../{name}\" as {name}")),
            after: Some(fill_in(
                &tr(
                    "Then name what you need through it, such as {name}.{example}. Click a name in the list to read how it is written.",
                ),
                &[("name", &name), ("example", &example)],
            )),
        }
    }

    /// What the parts sidebar says when it has no parts to list (a library
    /// lists what it defines under this line).
    pub fn sidebar(&self) -> String {
        match self.reason {
            Reason::Checking => tr("Checking…"),
            Reason::CantOpen => tr("No folder open"),
            Reason::Errors => tr("The parts appear once the errors are fixed"),
            Reason::Library => tr("A library has no parts. It defines:"),
            Reason::NoParts => tr("No parts yet"),
        }
    }
}

/// The first line of a doc comment.
fn summary(decl: &Decl) -> Option<String> {
    decl.docs
        .iter()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

/// What a library folder defines (its public items), read off its
/// resolved definitions; `None` when the folder isn't a library or its
/// names didn't resolve.
pub fn library(checked: &Checked) -> Option<Library> {
    let model = checked.opened.model.as_ref()?;
    let defs = &model.defs;
    let module = defs.modules.get(model.root.index())?;
    let library = module.library.as_ref()?;
    let mut groups: Vec<(LibraryGroup, Vec<LibraryItem>)> = Vec::new();
    for id in &module.items {
        let group = match *id {
            DefId::Kind(kind) => match defs.kind(kind).map(|kind| kind.base()) {
                Some(BaseKind::Joint) => LibraryGroup::Joints,
                Some(
                    BaseKind::Slide
                    | BaseKind::Hinge
                    | BaseKind::Pull
                    | BaseKind::Support
                    | BaseKind::Fitting,
                ) => LibraryGroup::Hardware,
                Some(BaseKind::Fastener) => LibraryGroup::Fasteners,
                Some(BaseKind::Tool) => LibraryGroup::Tools,
                None => continue,
            },
            DefId::Recipe(_) => LibraryGroup::Recipes,
            DefId::Component(_) => LibraryGroup::Components,
            DefId::Stock(_) => LibraryGroup::Stock,
            DefId::Profile(_) => LibraryGroup::Profiles,
            DefId::Param(_)
            | DefId::Let(_)
            | DefId::Region(_)
            | DefId::Part(_)
            | DefId::Joint(_)
            | DefId::Hardware(_)
            | DefId::Assembly(_)
            | DefId::Require(_)
            | DefId::Scene(_) => continue,
        };
        let Some(decl) = defs.decl(*id) else {
            continue;
        };
        if decl.private {
            continue;
        }
        let location = checked.opened.sources.get(decl.file).and_then(|file| {
            let line = LineNumber::from_usize(file.line_column(decl.name_span.start()).line)?;
            Some(SourceLocation {
                path: file.path().to_owned(),
                line,
            })
        });
        let item = LibraryItem {
            name: decl.name.clone(),
            summary: summary(decl),
            location,
        };
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, items)) => items.push(item),
            None => groups.push((group, vec![item])),
        }
    }
    groups.sort_by_key(|(group, _)| *group);
    let import_name = checked.folder.file_name().map_or_else(
        || module.name.clone(),
        |name| name.to_string_lossy().into_owned(),
    );
    Some(Library {
        title: library.title.clone(),
        about: library.about.clone(),
        import_name,
        groups,
    })
}

/// The empty state for what the session holds, or `None` when there is a
/// design with parts to draw.
pub fn of(session: &Session) -> Option<EmptyState> {
    if let Some((design, _)) = session.drawable() {
        if design.parts().is_empty() && design.hardware().is_empty() {
            return Some(EmptyState {
                reason: Reason::NoParts,
                title: tr("No parts yet"),
                description: tr(
                    "The project places no parts. Add a part to its text and it appears here as soon as the file is saved.",
                ),
                offers_open: false,
                library: None,
            });
        }
        return None;
    }
    let state = match session.latest() {
        Some(latest) if latest.summary.errors > 0 => EmptyState {
            reason: Reason::Errors,
            title: tr("Nothing to show yet"),
            description: fill_in(
                &ntr(
                    "The design has {count} error, so it can't be put together yet. The problems chip at the top steps through them (F8).",
                    "The design has {count} errors, so it can't be put together yet. The problems chip at the top steps through them (F8).",
                    latest.summary.errors,
                ),
                &[("count", &latest.summary.errors)],
            ),
            offers_open: false,
            library: None,
        },
        Some(latest) => EmptyState {
            reason: Reason::Library,
            title: tr("Nothing to show"),
            description: tr(
                "This folder is a library of kinds and components, with no parts to place. Open a project that uses it to see it in 3D.",
            ),
            offers_open: true,
            library: library(latest),
        },
        None => match session.check_status().verdict() {
            Some(Verdict::CantOpen(message)) => EmptyState {
                reason: Reason::CantOpen,
                title: tr("Can't open the folder"),
                description: message.clone(),
                offers_open: true,
                library: None,
            },
            Some(Verdict::Checked(_)) | None => EmptyState {
                reason: Reason::Checking,
                title: tr("Checking…"),
                description: String::new(),
                offers_open: false,
                library: None,
            },
        },
    };
    Some(state)
}

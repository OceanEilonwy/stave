//! The edit loop, headless (`docs/editing.md`, "The edit loop", "Verdicts
//! and strict commit", "Verdict speed"): try a patch on a scratch copy of
//! a project, and commit it to the text only when it fits.
//!
//! ```text
//! EditBase ──start(patch)──▶ Scratch ──finish(cancel)──▶ Tried ──editing::commit_tried──▶ files written
//!   │  (apply to a scratch     │ fast tier:            │ slow tier:               only on Fits
//!   │   copy, re-solve)        │ Refused or Checking   │ Fits or Refused
//!   └─ unchanged: many candidates are tried from one base
//! ```
//!
//! - [`EditBase`] is a prepared project ([`crate::prepare`]): its files
//!   parsed once, its libraries resolved once, its diagnostics known. A
//!   candidate is tried from `&EditBase`, so the base never changes and
//!   any number of candidates can be tried from it.
//! - [`EditBase::start`] applies the patch to a scratch copy of the
//!   project's files (through [`stave_lang::apply_patch`], so comments and
//!   layout are kept), opens the copy again reusing the libraries, and runs
//!   the fast check tier ([`stave_check::check_fast`]). A patch that can't
//!   be applied is refused with why, as a diagnostic on the patch.
//! - [`Scratch::finish`] runs the slow tier ([`stave_check::check_slow`]),
//!   which the caller can cancel with a flag, and gives a [`Tried`] with
//!   the final [`Verdict`]: *fits* or *refused*.
//! - A try that fits is written by the editor's one writer,
//!   [`crate::editing::commit_tried`] (or the edit in progress's commit),
//!   which never overwrites a file changed since it was read.
//!
//! **Strict commit.** A candidate is refused when it causes an error. An
//! error the design already had is not caused by the edit and doesn't
//! refuse it: the candidate's diagnostics are compared with the base's by
//! severity, code, message and place, where the place is the file and the
//! text the diagnostic points at (byte offsets move when a patch adds
//! text above, the text doesn't). Diagnostics are compared as a multiset,
//! so a second copy of an error the base had once is caused. Warnings
//! never refuse; a fit carries the warnings the edit caused.
//!
//! **The checks always run when there is a design.** [`crate::open`] runs
//! the fit and geometry checks (layer 4) only when nothing before them
//! found an error, so one mistake is one error. The edit loop can't: a
//! design that already has an error (a failing `require`, say) would then
//! never be checked again, and every edit would fit. Here the gate is
//! judged against the base instead: the checks run on a candidate unless
//! the edit itself caused an error before them (the edit is refused for
//! that anyway, and its own mistake stays one error), and the base's
//! diagnostics are taken with the checks run the same way, so the errors
//! they find in the base are not counted against the edit.
//!
//! Most errors before the checks (a failing `require`, an unknown name)
//! stop the solver writing a design at all, so there is nothing to check
//! for fit. Such a candidate is refused even when it caused no error
//! itself, with a diagnostic pointing at the error that stops the design
//! being placed: strict commit never writes what wasn't checked.

use crate::i18n::tr;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use stave_check::{BoxOverlap, Cancelled, Contact, Fast};
use stave_lang::ast::{self, Span};
use stave_lang::{
    Code, Diagnostic, FileId, Label, ModuleKey, Parse, PatchError, PatchErrorKind, PatchTarget,
    SourceMap, apply_patch, parse_file, parse_patch_file,
};
use stave_model::Model;
use stave_model::solved::SolvedDesign;
use stave_modules::{FileSource, LoadError, ModuleSet, ModuleSource};

use crate::edit::{Fits, Fix, Problem, Refusal, Verdict};
use crate::{Opened, Prepared};

// ---------------------------------------------------------------------------
// The base
// ---------------------------------------------------------------------------

/// A prepared project that candidates are tried from. Trying a candidate
/// borrows it and leaves it as it was.
pub struct EditBase<'f> {
    /// The project folder, as given.
    folder: PathBuf,
    /// Where the project's files are read from.
    files: Source<'f>,
    /// Files whose text differs from `files`' after [`EditBase::advance`],
    /// by the path the loader reads them at.
    overrides: BTreeMap<PathBuf, String>,
    /// The parsed files and resolved libraries a scratch copy reuses.
    prepared: Prepared,
    /// The base design as checked.
    opened: Opened,
    /// The base's diagnostics, counted by what they say and where.
    known: BTreeMap<Key, usize>,
    /// The parsed files a patch is applied to.
    target: Target,
}

impl<'f> EditBase<'f> {
    /// Prepares the project in `folder`, read through `files`.
    ///
    /// # Errors
    ///
    /// When the folder itself can't be loaded ([`crate::prepare`]).
    pub fn new(folder: &Path, files: &'f dyn FileSource) -> Result<Self, LoadError> {
        let (opened, prepared) = crate::prepare(folder, files)?;
        Ok(Self::from_prepared(folder, files, opened, prepared))
    }

    /// [`EditBase::new`] reading through a source the base shares rather
    /// than borrows, so it can be kept in a long-lived value (an editor
    /// session) without a borrow tying it down.
    ///
    /// # Errors
    ///
    /// When the folder itself can't be loaded ([`crate::prepare`]).
    pub fn shared(
        folder: &Path,
        files: Arc<dyn FileSource + Send + Sync>,
    ) -> Result<EditBase<'static>, LoadError> {
        let (opened, prepared) = crate::prepare(folder, files.as_ref())?;
        Ok(EditBase::from_source(
            folder,
            Source::Shared(files),
            opened,
            prepared,
        ))
    }

    /// A base from a project already prepared with [`crate::prepare`]
    /// from the same `folder` and `files`; `opened` and `prepared` are
    /// what that call returned.
    pub fn from_prepared(
        folder: &Path,
        files: &'f dyn FileSource,
        opened: Opened,
        prepared: Prepared,
    ) -> Self {
        Self::from_source(folder, Source::Borrowed(files), opened, prepared)
    }

    fn from_source(folder: &Path, files: Source<'f>, opened: Opened, prepared: Prepared) -> Self {
        let target = Target::of(&prepared.modules, folder);
        let known = count(&opened.sources, &base_diagnostics(&opened));
        Self {
            folder: folder.to_path_buf(),
            files,
            overrides: BTreeMap::new(),
            prepared,
            opened,
            known,
            target,
        }
    }

    /// The base design as checked.
    pub fn opened(&self) -> &Opened {
        &self.opened
    }

    /// Applies `patch` to a scratch copy of the project, opens the copy
    /// again reusing the libraries, and runs the fast check tier. `name`
    /// is the patch file's display path: a refusal's diagnostics on the
    /// patch point into it.
    ///
    /// # Errors
    ///
    /// When the scratch copy can't be loaded (the base's folder can no
    /// longer be read, or the source map is full). A patch that can't be
    /// applied is not an error: it is refused.
    pub fn start(
        &self,
        name: &str,
        patch: &Parse<ast::PatchFile>,
    ) -> Result<Scratch<'_, 'f>, LoadError> {
        let patch_text = patch.syntax().to_string();
        let applied = apply_patch(&self.target.patch_target(), patch);
        let patched = match applied {
            Ok(patched) => patched,
            Err(error) => {
                let mut sources = self.opened.sources.clone();
                let file = sources.add(name, patch_text)?;
                let errors = unapplied(&error, file);
                return Ok(Scratch {
                    base: self,
                    state: State::Done(Tried {
                        verdict: verdict(&sources, errors.iter().collect(), Vec::new()),
                        changed: Vec::new(),
                        sources,
                        scratch: None,
                    }),
                });
            }
        };
        let mut overrides = self.overrides.clone();
        let mut changed = Vec::with_capacity(patched.len());
        for file in patched {
            let Some(folder) = self.target.folders.get(&file.module) else {
                continue;
            };
            let path = folder.join(&file.name);
            overrides.insert(path.clone(), file.text.clone());
            changed.push(ChangedFile {
                path,
                text: file.text,
                created: file.created,
            });
        }
        let overlay = Overlay {
            files: self.files.get(),
            overrides: &overrides,
        };
        let stages = place(&self.prepared, &self.folder, &overlay)?;
        let mut sources = stages.modules.sources.clone();
        sources.add(name, patch_text)?;
        let pending = Pending {
            changed,
            overrides,
            sources,
        };
        let state = if let Some((model, design)) = self.checkable(&stages) {
            let fast = stave_check::check_fast(model, design);
            let mut so_far = stages.diagnostics.clone();
            so_far.extend(fast.diagnostics.iter().map(|(d, _)| d.clone()));
            let caused = self.caused(&stages.modules.sources, &so_far);
            if caused.errors.is_empty() {
                State::Checking {
                    stages,
                    fast,
                    pending,
                }
            } else {
                // An error in the fast tier refuses at once.
                let contacts = fast
                    .diagnostics
                    .iter()
                    .filter_map(|(d, contact)| Some((d.clone(), contact.clone()?)))
                    .collect();
                State::Done(self.tried(stages, so_far, contacts, pending))
            }
        } else {
            let diagnostics = stages.diagnostics.clone();
            State::Done(self.tried(stages, diagnostics, Vec::new(), pending))
        };
        Ok(Scratch { base: self, state })
    }

    /// [`EditBase::start`] and [`Scratch::finish`]: the whole check of a
    /// candidate.
    ///
    /// # Errors
    ///
    /// [`TryError::Load`] as for [`EditBase::start`];
    /// [`TryError::Cancelled`] when `cancel` was set before the slow tier
    /// finished.
    pub fn try_patch(
        &self,
        name: &str,
        patch: &Parse<ast::PatchFile>,
        cancel: &AtomicBool,
    ) -> Result<Tried, TryError> {
        Ok(self.start(name, patch)?.finish(cancel)?)
    }

    /// Makes the base the project as it is after `tried`, a candidate
    /// tried from this base that fits: what an editor does once a commit
    /// has written it. Later candidates are tried on top of it.
    ///
    /// # Errors
    ///
    /// [`CommitError::Refused`] when `tried` doesn't fit; the base is
    /// left as it was.
    pub fn advance(&mut self, tried: Tried) -> Result<(), CommitError> {
        let Verdict::Fits(_) = tried.verdict else {
            return Err(CommitError::Refused);
        };
        let Some(scratch) = tried.scratch else {
            return Err(CommitError::Refused);
        };
        self.target = Target::of(&scratch.modules, &self.folder);
        self.known = count(&scratch.opened.sources, &scratch.opened.diagnostics);
        self.prepared = Prepared {
            modules: scratch.modules,
            libraries: self.prepared.libraries.clone(),
        };
        self.opened = scratch.opened;
        self.overrides = scratch.overrides;
        Ok(())
    }

    /// The design of a scratch copy, when the checks are to run on it: it
    /// was placed, and the edit caused no error before the checks. An
    /// error the base already had there doesn't stop them.
    fn checkable<'s>(&self, stages: &'s Stages) -> Option<(&'s Model, &'s SolvedDesign)> {
        let found = stages.model.as_ref().zip(stages.design.as_ref())?;
        self.caused(&stages.modules.sources, &stages.diagnostics)
            .errors
            .is_empty()
            .then_some(found)
    }

    /// The diagnostics in `diagnostics` that the base doesn't have,
    /// split into errors and warnings, in their order. A diagnostic the
    /// checker merged from several problems is the base's only when every
    /// one of them is: a new clash on a line that already had one is
    /// folded into the old diagnostic, and is still caused by the edit.
    fn caused<'d>(&self, sources: &SourceMap, diagnostics: &'d [Diagnostic]) -> Caused<'d> {
        let mut left = self.known.clone();
        let mut caused = Caused::default();
        for diagnostic in diagnostics {
            let mut new = false;
            for key in Key::all(sources, diagnostic) {
                match left.get_mut(&key) {
                    Some(remaining) if *remaining > 0 => *remaining -= 1,
                    _ => new = true,
                }
            }
            if new {
                if diagnostic.is_error() {
                    caused.errors.push(diagnostic);
                } else {
                    caused.warnings.push(diagnostic);
                }
            }
        }
        caused
    }

    /// The final result of a scratch copy whose diagnostics are complete.
    fn tried(
        &self,
        stages: Stages,
        mut diagnostics: Vec<Diagnostic>,
        contacts: Vec<(Diagnostic, Contact)>,
        pending: Pending,
    ) -> Tried {
        crate::sort(&mut diagnostics);
        let caused = self.caused(&stages.modules.sources, &diagnostics);
        let unplaced = unplaced(&stages, &diagnostics, &caused);
        let verdict = match &unplaced {
            Some(unplaced) => verdict(&pending.sources, vec![unplaced], Vec::new()),
            None => verdict(&pending.sources, caused.errors, caused.warnings),
        };
        let opened = Opened {
            sources: stages.modules.sources.clone(),
            model: stages.model,
            design: stages.design,
            diagnostics,
            contacts,
            root_files: stages.root_files,
        };
        Tried {
            verdict,
            changed: pending.changed,
            sources: pending.sources,
            scratch: Some(ScratchCopy {
                modules: stages.modules,
                opened,
                overrides: pending.overrides,
            }),
        }
    }
}

/// The verdict for the errors and warnings an edit causes: refused with
/// the errors, their fixes and their text as `stave check` prints it
/// (rendered from `sources`) when there is one, else fits with the
/// warnings.
fn verdict(sources: &SourceMap, errors: Vec<&Diagnostic>, warnings: Vec<&Diagnostic>) -> Verdict {
    let problems = errors
        .into_iter()
        .filter_map(|diagnostic| {
            Problem::new(diagnostic.clone(), fixes(diagnostic))
                .ok()
                .map(|problem| {
                    problem.with_rendered(stave_lang::render_diagnostic(sources, diagnostic))
                })
        })
        .collect();
    match Refusal::new(problems) {
        Ok(refusal) => Verdict::Refused(refusal),
        Err(_) => match Fits::new(warnings.into_iter().cloned().collect()) {
            Ok(fits) => Verdict::Fits(fits),
            // Can't happen: errors go to the refusal. Not fitting is the
            // safe answer, as nothing is committed on it.
            Err(_) => Verdict::Checking,
        },
    }
}

/// Why a candidate that caused no error still can't fit: it has no design
/// to check, because of an error the base already had (a failing
/// `require`, say, stops the solver writing the design). Its fit and
/// geometry can't be checked, and strict commit never writes what wasn't
/// checked, so it is refused, pointing at the first such error. `None`
/// when the candidate has a design, caused an error itself, or has no
/// error at all (a library, which has nothing to place).
fn unplaced(
    stages: &Stages,
    diagnostics: &[Diagnostic],
    caused: &Caused<'_>,
) -> Option<Diagnostic> {
    if stages.design.is_some() || !caused.errors.is_empty() {
        return None;
    }
    let blocking = diagnostics.iter().find(|d| d.is_error())?;
    let mut unplaced = blocking.clone();
    unplaced.code = Code::NotSupported;
    unplaced.message = "the edit can't be checked: the design already has this error, which \
                        stops it being placed"
        .to_owned();
    unplaced.labels.truncate(1);
    unplaced.notes = vec![format!(
        "the error: error[{}]: {}",
        blocking.code, blocking.message
    )];
    unplaced.help =
        Some("fix that error first; until the design places, no edit can be checked".to_owned());
    Some(unplaced)
}

/// The fixes a diagnostic holds (grammar, "Fixes"), as contract fixes,
/// in the order the help recommends them.
fn fixes(diagnostic: &Diagnostic) -> Vec<Fix> {
    diagnostic
        .fixes
        .iter()
        .filter_map(Fix::from_diagnostic_fix)
        .collect()
}

/// What the edit caused.
#[derive(Default)]
struct Caused<'d> {
    errors: Vec<&'d Diagnostic>,
    warnings: Vec<&'d Diagnostic>,
}

/// Why a whole try didn't give a verdict.
#[derive(Debug, thiserror::Error)]
pub enum TryError {
    /// The scratch copy can't be loaded.
    #[error(transparent)]
    Load(#[from] LoadError),
    /// The slow tier was cancelled.
    #[error(transparent)]
    Cancelled(#[from] Cancelled),
}

// ---------------------------------------------------------------------------
// A candidate being tried
// ---------------------------------------------------------------------------

/// A candidate applied to a scratch copy, after the fast tier.
pub struct Scratch<'b, 'f> {
    base: &'b EditBase<'f>,
    state: State,
}

enum State {
    /// The verdict is final: the patch was refused before the slow tier,
    /// or the slow tier has nothing to check.
    Done(Tried),
    /// The fast tier found no error the edit causes; the slow tier is to
    /// run.
    Checking {
        stages: Stages,
        fast: Fast,
        pending: Pending,
    },
}

/// What a scratch copy keeps until its verdict is final.
struct Pending {
    changed: Vec<ChangedFile>,
    overrides: BTreeMap<PathBuf, String>,
    /// The scratch copy's files and the patch file.
    sources: SourceMap,
}

impl Scratch<'_, '_> {
    /// The verdict so far: *refused* when the patch can't be applied or
    /// the fast tier found an error the edit causes, *checking* while the
    /// slow tier is still to run, and the final verdict when there is
    /// nothing more to check.
    pub fn verdict(&self) -> Verdict {
        match &self.state {
            State::Done(tried) => tried.verdict.clone(),
            State::Checking { .. } => Verdict::Checking,
        }
    }

    /// The pairs of items whose boxes overlap at rest in the scratch copy
    /// (the fast tier's box overlap: they may overlap), for the ghost.
    /// Empty when the fast tier didn't run or the verdict is final.
    pub fn overlaps(&self) -> &[BoxOverlap] {
        match &self.state {
            State::Checking { fast, .. } => &fast.overlaps,
            State::Done(_) => &[],
        }
    }

    /// Runs the slow tier and gives the final verdict. A verdict that was
    /// already final is returned as it is.
    ///
    /// # Errors
    ///
    /// [`Cancelled`] when `cancel` is set before the slow tier finishes;
    /// the scratch copy is dropped.
    pub fn finish(self, cancel: &AtomicBool) -> Result<Tried, Cancelled> {
        let (stages, fast, pending) = match self.state {
            State::Done(tried) => return Ok(tried),
            State::Checking {
                stages,
                fast,
                pending,
            } => (stages, fast, pending),
        };
        let Some((model, design)) = self.base.checkable(&stages) else {
            // `Checking` is only made when the checks may run.
            let diagnostics = stages.diagnostics.clone();
            return Ok(self.base.tried(stages, diagnostics, Vec::new(), pending));
        };
        let found = stave_check::check_slow(model, design, &fast, cancel)?;
        let mut diagnostics = stages.diagnostics.clone();
        let mut contacts = Vec::new();
        for (diagnostic, contact) in found {
            if let Some(contact) = contact {
                contacts.push((diagnostic.clone(), contact));
            }
            diagnostics.push(diagnostic);
        }
        Ok(self.base.tried(stages, diagnostics, contacts, pending))
    }
}

/// A file a candidate changes or creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// Where it is written: the module's folder, as the project was
    /// opened, and the file's name.
    pub path: PathBuf,
    /// Its new text.
    pub text: String,
    /// `true` when the patch creates the file.
    pub created: bool,
}

/// A candidate's final verdict, with the files it changes and the scratch
/// copy as checked.
#[derive(Debug)]
pub struct Tried {
    /// *Fits* or *refused*, never *checking*.
    verdict: Verdict,
    changed: Vec<ChangedFile>,
    sources: SourceMap,
    /// `None` when the patch couldn't be applied.
    scratch: Option<ScratchCopy>,
}

/// The scratch copy a patch was applied to, as checked.
#[derive(Debug)]
struct ScratchCopy {
    modules: ModuleSet,
    opened: Opened,
    overrides: BTreeMap<PathBuf, String>,
}

impl Tried {
    /// The verdict: [`Verdict::Fits`] or [`Verdict::Refused`].
    pub fn verdict(&self) -> &Verdict {
        &self.verdict
    }

    /// `true` when the verdict is *fits*.
    pub fn fits(&self) -> bool {
        matches!(self.verdict, Verdict::Fits(_))
    }

    /// The files the patch changes or creates, by path; empty when it
    /// couldn't be applied.
    pub fn changed(&self) -> &[ChangedFile] {
        &self.changed
    }

    /// The scratch copy as checked, for the ghost; `None` when the patch
    /// couldn't be applied. After a refusal by the fast tier its
    /// diagnostics are what the fast tier found. Unlike [`crate::open`],
    /// the checks ran even when an error came before them, unless the
    /// edit caused that error (see the module's notes).
    pub fn opened(&self) -> Option<&Opened> {
        self.scratch.as_ref().map(|scratch| &scratch.opened)
    }

    /// The scratch copy as checked, taken out of the result: after a
    /// commit, the design an editor shows. `None` when the patch couldn't
    /// be applied.
    pub fn into_opened(self) -> Option<Opened> {
        self.scratch.map(|scratch| scratch.opened)
    }

    /// Every file the verdict's diagnostics point into: the scratch copy's
    /// and the patch file, for [`stave_lang::render_diagnostics`].
    pub fn sources(&self) -> &SourceMap {
        &self.sources
    }

    /// The diagnostics the verdict shows: a refusal's errors, or a fit's
    /// warnings.
    pub fn shown(&self) -> Vec<Diagnostic> {
        match &self.verdict {
            Verdict::Checking => Vec::new(),
            Verdict::Fits(fits) => fits.warnings().to_vec(),
            Verdict::Refused(refusal) => refusal
                .problems()
                .iter()
                .map(|problem| problem.diagnostic().clone())
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Files on disk: the editor's writer for the command line. It writes a
/// file only while it still holds the text the edits were computed from
/// (see [`crate::editing::WriteEdits`]), so a change saved since the
/// project was read is never overwritten.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiskWriter;

/// Why [`EditBase::advance`] didn't move the base.
#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    /// The candidate doesn't fit.
    #[error("{}", self.text())]
    Refused,
}

impl CommitError {
    /// The error in the window's words (translated).
    fn text(&self) -> String {
        match self {
            CommitError::Refused => tr("the edit was refused, so nothing was written"),
        }
    }
}

// ---------------------------------------------------------------------------
// Patch files
// ---------------------------------------------------------------------------

/// Each patch of a patch file as a patch file of its own, in order, so
/// they can be tried one after another. Every one keeps the whole file's
/// byte offsets (the other patches are blanked out with spaces), so its
/// diagnostics point at the same lines. A file with syntax errors, or no
/// patches, is given back whole.
pub fn one_patch_each(file: FileId, patches: &Parse<ast::PatchFile>) -> Vec<Parse<ast::PatchFile>> {
    let whole = || vec![patches.clone()];
    let Some(ast) = &patches.ast else {
        return whole();
    };
    if patches.has_errors() || ast.patches.len() < 2 {
        return whole();
    }
    let text = patches.syntax().to_string();
    let spans: Vec<Span> = ast.patches.iter().map(|patch| patch.span).collect();
    spans
        .iter()
        .enumerate()
        .map(|(keep, _)| {
            let blanked: String = text
                .char_indices()
                .flat_map(|(offset, character)| {
                    let offset = u32::try_from(offset).unwrap_or(u32::MAX);
                    let hidden = character != '\n'
                        && spans.iter().enumerate().any(|(index, span)| {
                            index != keep && span.start() <= offset && offset < span.end()
                        });
                    let width = if hidden { character.len_utf8() } else { 0 };
                    let shown = if hidden { None } else { Some(character) };
                    shown.into_iter().chain(std::iter::repeat_n(' ', width))
                })
                .collect();
            parse_patch_file(file, &blanked)
        })
        .collect()
}

/// A patch that can't be applied, as diagnostics on the patch file.
fn unapplied(error: &PatchError, file: FileId) -> Vec<Diagnostic> {
    let label = Label::new(file, error.span);
    let (code, help) = match &error.kind {
        PatchErrorKind::Syntax(diagnostics) if !diagnostics.is_empty() => {
            return diagnostics
                .iter()
                .map(|diagnostic| {
                    let mut diagnostic = diagnostic.clone();
                    for label in &mut diagnostic.labels {
                        label.file = file;
                    }
                    diagnostic
                })
                .collect();
        }
        PatchErrorKind::Syntax(_) => (
            Code::Syntax,
            "start the patch file with the line 'stave 1'".to_owned(),
        ),
        PatchErrorKind::UnknownItem { .. } => (
            Code::UnknownName,
            "check the spelling, or add the item in an earlier operation of the patch".to_owned(),
        ),
        PatchErrorKind::UnknownMember { item, .. } => (
            Code::UnknownField,
            format!("check the spelling against what '{item}' writes"),
        ),
        PatchErrorKind::NotSet(_) => (
            Code::MissingField,
            "leave this 'unset' out: the default already applies".to_owned(),
        ),
        PatchErrorKind::NotAComponent(_) => (
            Code::TypeMismatch,
            "name a component after 'to', or leave 'to' out to add the item to the project"
                .to_owned(),
        ),
        PatchErrorKind::WrongTag { found, .. } => {
            (Code::TypeMismatch, format!("write 'remove {found}'"))
        }
        PatchErrorKind::AlreadyExists { .. } => (
            Code::DuplicateName,
            "choose another name, or change the item that is there with 'set'".to_owned(),
        ),
        PatchErrorKind::Unaddressable { .. } => (
            Code::NotSupported,
            "a patch can set the paths listed in the grammar's \"Patches\" section".to_owned(),
        ),
        PatchErrorKind::UnknownFile(_) | PatchErrorKind::InvalidFileName(_) => (
            Code::UnknownName,
            "name a .stave file in the project's folder, or leave 'in' out".to_owned(),
        ),
        PatchErrorKind::ComponentElsewhere { actual, .. } => (
            Code::UnknownName,
            format!("write in \"{actual}\", or leave 'in' out"),
        ),
        PatchErrorKind::NoFiles => (
            Code::NotSupported,
            "add a .stave file with the project in it first".to_owned(),
        ),
        PatchErrorKind::UnknownImport { .. } => (
            Code::UnknownModule,
            "write the import's source as the file writes it".to_owned(),
        ),
        PatchErrorKind::ImportExists { .. } => (
            Code::DuplicateName,
            "remove the import that is there first, in the same patch".to_owned(),
        ),
        PatchErrorKind::WrongTerm { .. } => (
            Code::TypeMismatch,
            "set the path for the axis the term fixes, or write a term for this axis".to_owned(),
        ),
        PatchErrorKind::TermClash { other, .. } => (
            Code::AxisSetTwice,
            format!("unset '{other}' earlier in the patch, so each axis has one term"),
        ),
        PatchErrorKind::NotSupported(_) => (
            Code::NotSupported,
            "write the change with 'remove' and 'add' for now".to_owned(),
        ),
        PatchErrorKind::BreaksSyntax { .. } => (
            Code::Syntax,
            "check the values the operation writes".to_owned(),
        ),
        PatchErrorKind::Conflict => (
            Code::NotSupported,
            "try the operations as separate patches".to_owned(),
        ),
    };
    vec![Diagnostic::error(code, error.kind.to_string(), label).with_help(help)]
}

// ---------------------------------------------------------------------------
// The scratch copy
// ---------------------------------------------------------------------------

/// The parsed files of the project's module and of the local modules it
/// imports, which a patch is applied to, and each module's folder.
struct Target {
    project: Vec<(String, Parse<ast::File>)>,
    locals: BTreeMap<String, Vec<(String, Parse<ast::File>)>>,
    folders: BTreeMap<ModuleKey, PathBuf>,
}

impl Target {
    fn of(modules: &ModuleSet, folder: &Path) -> Self {
        let parsed = |module: &stave_modules::Module| -> Vec<(String, Parse<ast::File>)> {
            module
                .files
                .iter()
                .filter_map(|file| {
                    let text = modules.sources.get(file.file)?.text();
                    let name = Path::new(&file.path).file_name()?.to_string_lossy();
                    Some((name.into_owned(), parse_file(file.file, text)))
                })
                .collect()
        };
        let mut target = Target {
            project: Vec::new(),
            locals: BTreeMap::new(),
            folders: BTreeMap::from([(ModuleKey::Project, folder.to_path_buf())]),
        };
        let Some(root) = modules.module(modules.root) else {
            return target;
        };
        target.project = parsed(root);
        for import in root.files.iter().flat_map(|file| &file.imports) {
            let Some(module) = modules.module(import.module) else {
                continue;
            };
            match module.source {
                ModuleSource::Local { .. } => {}
                ModuleSource::Std { .. } => continue,
            }
            if target.locals.contains_key(&import.name) {
                continue;
            }
            let Some(module_folder) = module
                .files
                .first()
                .and_then(|file| Path::new(&file.path).parent())
            else {
                continue;
            };
            target.folders.insert(
                ModuleKey::Local(import.name.clone()),
                module_folder.to_path_buf(),
            );
            target.locals.insert(import.name.clone(), parsed(module));
        }
        target
    }

    fn patch_target(&self) -> PatchTarget<'_> {
        fn files(files: &[(String, Parse<ast::File>)]) -> Vec<stave_lang::ModuleFile<'_>> {
            files
                .iter()
                .map(|(name, parse)| stave_lang::ModuleFile { name, parse })
                .collect()
        }
        let mut target = PatchTarget::new(files(&self.project));
        for (name, module) in &self.locals {
            target = target.with_local_module(name.clone(), files(module));
        }
        target
    }
}

/// Where an [`EditBase`] reads the project's files.
enum Source<'f> {
    /// A source the caller keeps.
    Borrowed(&'f dyn FileSource),
    /// A source the base shares ([`EditBase::shared`]).
    Shared(Arc<dyn FileSource + Send + Sync>),
}

impl Source<'_> {
    fn get(&self) -> &dyn FileSource {
        match self {
            Source::Borrowed(files) => *files,
            Source::Shared(files) => files.as_ref(),
        }
    }
}

/// The project's files with some replaced or added: a scratch copy.
struct Overlay<'a> {
    files: &'a dyn FileSource,
    overrides: &'a BTreeMap<PathBuf, String>,
}

impl FileSource for Overlay<'_> {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        let mut listed = self.files.list_stave_files(folder)?;
        for path in self.overrides.keys() {
            let in_folder = path.parent() == Some(folder);
            let listed_already = listed
                .iter()
                .any(|other| other.file_name() == path.file_name());
            if in_folder && !listed_already {
                listed.push(path.clone());
            }
        }
        Ok(listed)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        match self.overrides.get(path) {
            Some(text) => Ok(text.clone()),
            None => self.files.read(path),
        }
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        self.files.canonical(folder)
    }
}

/// The base's diagnostics as the edit loop takes them: with the checks run
/// whenever there is a design. [`crate::open`] leaves them out when an
/// error came before them, so they are run here for such a base.
fn base_diagnostics(opened: &Opened) -> Vec<Diagnostic> {
    let mut diagnostics = opened.diagnostics.clone();
    if opened.error_count() > 0
        && let Some((model, design)) = opened.model.as_ref().zip(opened.design.as_ref())
    {
        // When the checks did run, what they find again is already there;
        // sorting drops the copies.
        diagnostics.extend(
            stave_check::check_with_contacts(model, design)
                .into_iter()
                .map(|(diagnostic, _)| diagnostic),
        );
        crate::sort(&mut diagnostics);
    }
    diagnostics
}

/// A scratch copy loaded, resolved and placed, before the checks of this
/// crate's tiers.
struct Stages {
    modules: ModuleSet,
    model: Option<Model>,
    design: Option<SolvedDesign>,
    diagnostics: Vec<Diagnostic>,
    root_files: usize,
}

/// The pipeline of [`crate::open`] up to the checks, reusing what
/// `prepared` kept.
fn place(prepared: &Prepared, folder: &Path, files: &dyn FileSource) -> Result<Stages, LoadError> {
    let modules = prepared.modules.reload_with(folder, files)?;
    let root_files = modules
        .module(modules.root)
        .map_or(0, |module| module.files.len());
    let mut diagnostics = modules.diagnostics.clone();
    let root_has_header = modules
        .module(modules.root)
        .is_some_and(|module| module.header.is_some());
    let cycle = diagnostics.iter().any(|d| d.code == Code::ImportCycle);
    if !root_has_header || cycle {
        return Ok(Stages {
            modules,
            model: None,
            design: None,
            diagnostics,
            root_files,
        });
    }
    let (defs, resolve_diagnostics) = prepared
        .libraries
        .as_ref()
        .and_then(|libraries| stave_model::resolve_reusing(&modules, libraries))
        .unwrap_or_else(|| stave_model::resolve(&modules));
    diagnostics.extend(resolve_diagnostics);
    let (model, instance_diagnostics) = stave_model::instance::instantiate(defs, modules.root);
    diagnostics.extend(instance_diagnostics);
    let (design, solve_diagnostics) = stave_solve::solve(&model);
    diagnostics.extend(solve_diagnostics);
    Ok(Stages {
        modules,
        model: Some(model),
        design,
        diagnostics,
        root_files,
    })
}

// ---------------------------------------------------------------------------
// Comparing diagnostics
// ---------------------------------------------------------------------------

/// What makes two problems the same across an edit: what they say, and
/// the file and the words of the text they point at. A diagnostic stands
/// for one problem, or for several when the checker merged them
/// ([`Diagnostic::merged`]): [`Key::all`] gives one key for each.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    error: bool,
    code: Code,
    message: String,
    place: Option<(String, String)>,
}

impl Key {
    /// A key for each problem `diagnostic` stands for: its own message
    /// first, then each merged into it.
    fn all(sources: &SourceMap, diagnostic: &Diagnostic) -> Vec<Self> {
        let place = diagnostic.primary().and_then(|label| {
            let file = sources.get(label.file)?;
            let start = usize::try_from(label.span.start()).ok()?;
            let end = usize::try_from(label.span.end()).ok()?;
            // Its words, not its spacing: a patch that adds a line to an
            // aligned run pads the lines already there, and an error on
            // one of them is still the same error.
            let text = file.text().get(start..end).unwrap_or_default();
            let words: Vec<&str> = text.split_whitespace().collect();
            Some((file.path().to_owned(), words.join(" ")))
        });
        std::iter::once(&diagnostic.message)
            .chain(&diagnostic.merged)
            .map(|message| Self {
                error: diagnostic.is_error(),
                code: diagnostic.code,
                message: message.clone(),
                place: place.clone(),
            })
            .collect()
    }
}

/// Each problem's key, counted.
fn count(sources: &SourceMap, diagnostics: &[Diagnostic]) -> BTreeMap<Key, usize> {
    let mut counted = BTreeMap::new();
    for diagnostic in diagnostics {
        for key in Key::all(sources, diagnostic) {
            *counted.entry(key).or_insert(0) += 1;
        }
    }
    counted
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};

    use stave_lang::{Code, Parse, SourceMap, ast, parse_patch_file, render_diagnostics};
    use stave_modules::MemoryFiles;

    use super::{CommitError, EditBase, Tried, TryError, one_patch_each};
    use crate::edit::Verdict;

    /// Where the tests' copies of the cabinet live in memory.
    const FOLDER: &str = "scratch/clothing_cabinet";

    /// A copy of `examples/clothing_cabinet` (its `hardware` module
    /// included), made in memory under [`FOLDER`].
    fn cabinet() -> MemoryFiles {
        let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
        let mut files = MemoryFiles::new();
        let mut folders = vec![(example, PathBuf::from(FOLDER))];
        while let Some((from, to)) = folders.pop() {
            for entry in std::fs::read_dir(&from).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_owned();
                if path.is_dir() {
                    folders.push((path, to.join(name)));
                } else if path.extension().is_some_and(|e| e == "stave") {
                    files.insert(to.join(name), std::fs::read_to_string(&path).unwrap());
                }
            }
        }
        files
    }

    fn patch(text: &str) -> Parse<ast::PatchFile> {
        let mut sources = SourceMap::new();
        let file = sources.add("change.stave", text).unwrap();
        let parse = parse_patch_file(file, text);
        assert!(!parse.has_errors(), "{:?}", parse.diagnostics);
        parse
    }

    /// Writes a fitting try's files into `files`; `Err` for a refusal,
    /// with nothing written.
    fn commit(tried: &Tried, files: &mut MemoryFiles) -> Result<(), ()> {
        if !tried.fits() {
            return Err(());
        }
        for file in tried.changed() {
            files.insert(&file.path, file.text.clone());
        }
        Ok(())
    }

    fn try_patch(base: &EditBase<'_>, text: &str) -> Tried {
        base.try_patch("change.stave", &patch(text), &AtomicBool::new(false))
            .unwrap()
    }

    /// The codes of a refusal's errors, or `None` for a fit.
    fn refused_codes(tried: &Tried) -> Option<Vec<Code>> {
        match tried.verdict() {
            Verdict::Refused(refusal) => Some(
                refusal
                    .problems()
                    .iter()
                    .map(|problem| problem.diagnostic().code)
                    .collect(),
            ),
            Verdict::Fits(_) => None,
            Verdict::Checking => panic!("a tried candidate is never checking"),
        }
    }

    const ADD_SHELF: &str = r#"stave 1

patch "Add a shelf" {
  add part shelf_2 : oak_19 {
    length: fit
    width:  cabinet_depth - back.thickness
    place {
      against(divider.top, gap: 400mm)
      center(base, across)
      flush(side_l.front)
    }
  } in "carcass.stave"
  add joint shelf_2_dados : dado(shelf_2, into: [side_l, side_r], depth: 9mm)
}
"#;

    /// A shelf whose height runs through the divider.
    const THROUGH_DIVIDER: &str = r#"stave 1

patch "Add a shelf through the divider" {
  add part shelf_3 : oak_19 {
    length: fit
    width:  cabinet_depth - back.thickness
    place {
      against(base.top, gap: drawer_zone - 10mm)
      center(base, across)
      flush(side_l.front)
    }
  } in "carcass.stave"
  add joint shelf_3_dados : dado(shelf_3, into: [side_l, side_r], depth: 9mm)
}
"#;

    #[test]
    fn a_shelf_fits_and_commits_and_the_project_checks_clean() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        assert_eq!(base.opened().error_count(), 0);
        let tried = try_patch(&base, ADD_SHELF);
        assert_eq!(
            refused_codes(&tried),
            None,
            "{}",
            render_diagnostics(tried.sources(), &tried.shown())
        );
        let changed: Vec<&Path> = tried.changed().iter().map(|f| f.path.as_path()).collect();
        assert_eq!(changed, [Path::new(FOLDER).join("carcass.stave")]);

        let mut written = files.clone();
        commit(&tried, &mut written).unwrap();
        assert_ne!(written, files);
        let reopened = crate::open_with(Path::new(FOLDER), &written).unwrap();
        assert_eq!(
            reopened.error_count(),
            0,
            "{}",
            render_diagnostics(&reopened.sources, &reopened.diagnostics)
        );
        let opened = tried.opened().unwrap();
        assert_eq!(opened.diagnostics, reopened.diagnostics);
    }

    #[test]
    fn a_part_through_another_is_refused_and_nothing_is_written() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let tried = try_patch(&base, THROUGH_DIVIDER);
        let codes = refused_codes(&tried).expect("refused");
        assert!(
            codes.contains(&Code::Overlap),
            "{}",
            render_diagnostics(tried.sources(), &tried.shown())
        );
        let shown = render_diagnostics(tried.sources(), &tried.shown());
        assert!(shown.contains("shelf_3"), "{shown}");
        assert!(shown.contains("divider"), "{shown}");

        let mut written = files.clone();
        assert!(commit(&tried, &mut written).is_err());
        assert_eq!(written, files);
    }

    /// The cabinet with a `require` added to `cabinet.stave` that fails:
    /// an error from the solver, which then writes no design.
    fn cabinet_with_a_failing_require() -> MemoryFiles {
        let mut files = cabinet();
        let project = Path::new(FOLDER).join("cabinet.stave");
        let text = files_text(&files, &project);
        files.insert(
            &project,
            format!("{text}\nrequire too_narrow : cabinet_width >= 1000mm \"a wide cabinet\"\n"),
        );
        files
    }

    /// The cabinet with the shelf through the divider already in it: an
    /// error of the checks themselves, with a design.
    fn cabinet_with_an_overlap() -> MemoryFiles {
        let clean = cabinet();
        let through = try_patch(
            &EditBase::new(Path::new(FOLDER), &clean).unwrap(),
            THROUGH_DIVIDER,
        );
        let mut files = clean.clone();
        for file in through.changed() {
            files.insert(&file.path, file.text.clone());
        }
        files
    }

    #[test]
    fn an_error_the_design_had_does_not_refuse_an_unrelated_edit() {
        let files = cabinet_with_an_overlap();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        assert!(base.opened().design.is_some());
        assert!(
            base.opened()
                .diagnostics
                .iter()
                .any(|d| d.is_error() && d.code == Code::Overlap)
        );
        let tried = try_patch(&base, ADD_SHELF);
        assert_eq!(
            refused_codes(&tried),
            None,
            "{}",
            render_diagnostics(tried.sources(), &tried.shown())
        );
        assert_eq!(
            tried.opened().unwrap().error_count(),
            base.opened().error_count()
        );
        let mut written = files.clone();
        commit(&tried, &mut written).unwrap();
        assert_ne!(written, files);
    }

    #[test]
    fn an_error_the_design_had_does_not_hide_one_the_edit_causes() {
        // A second shelf through the divider, beside the one already there:
        // its own overlap refuses it.
        let files = cabinet_with_an_overlap();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let another = THROUGH_DIVIDER.replace("shelf_3", "shelf_4");
        let tried = try_patch(&base, &another);
        let shown = render_diagnostics(tried.sources(), &tried.shown());
        let codes = refused_codes(&tried).expect("refused");
        assert!(codes.contains(&Code::Overlap), "{shown}");
        assert!(shown.contains("shelf_4"), "{shown}");
        let mut written = files.clone();
        assert!(commit(&tried, &mut written).is_err());
        assert_eq!(written, files);
    }

    /// A cupboard whose left door already reaches 1mm into the case's
    /// top (its overlay's `top: 1mm`): one error at the case's line, the
    /// door's other clashes merged into it.
    const CUPBOARD: &str = r#"stave 1

import "std:casework"

project "Hall cupboard" {
  units: mm
}

param overall_width  = 600mm
param overall_height = 900mm
param overall_depth  = 400mm

region envelope                        = box(across: overall_width, up: overall_height, deep: overall_depth)
region drawer_bay[bottom, middle, top] = split(divider.below, up)
region door_bay[left, right]           = split(divider.above, across)

stock oak_19   { species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2700mm }
stock maple_12 { species: hard_maple, form: solid, thickness: 12mm, max_width: 150mm, max_length: 2400mm }
stock ply_6    { species: birch, form: plywood, thickness: 6mm, max_length: 2440mm }

assembly case : casework.Case {
  space:       envelope
  panel_stock: oak_19
}

assembly back : casework.Back {
  sides:       [case.side[left], case.side[right]]
  stands_on:   case.base
  under:       case.top
  panel_stock: ply_6
}

assembly divider : casework.Divider {
  space:       back.in_front
  sides:       [case.side[left], case.side[right]]
  panel_stock: oak_19
  height:      300mm
}

assembly drawer[bottom, middle, top] : casework.Drawer {
  opening:      drawer_bay[same]
  mount:        overlay(left: 18mm, right: 18mm, top: 8mm, bottom: 8mm, meeting: -0.75mm)
  front_stock:  oak_19
  box_stock:    maple_12
  bottom_stock: ply_6
  slide:        side_mount_slide(length: 300mm)
  pull_kind:    bar_pull(centres: 128mm)
}

assembly door_left : casework.FramePanelDoor {
  opening:     door_bay[left]
  hinge_side:  case.side[left]
  mount:       overlay(left: 18mm, right: 18mm, top: 1mm, bottom: 8mm, meeting: -0.75mm)
  frame:       oak_19
  panel_stock: ply_6
}


"#;

    #[test]
    fn a_new_clash_merged_into_an_old_error_still_refuses() {
        // The mirrored right door reaches into the top as the left one
        // does. The checker reports its clashes at the case's line, the
        // same code at the same place as the left door's, so they are
        // merged into the error the design had: the edit still causes
        // them, and is refused.
        let folder = Path::new("scratch/hall_cupboard");
        let mut files = MemoryFiles::new();
        files.insert(folder.join("hall_cupboard.stave"), CUPBOARD.to_owned());
        let base = EditBase::new(folder, &files).unwrap();
        assert_eq!(
            base.opened().error_count(),
            1,
            "{}",
            render_diagnostics(&base.opened().sources, &base.opened().diagnostics)
        );
        let tried = try_patch(
            &base,
            r#"stave 1

patch "Add the right door" {
  add assembly door_right = mirror(door_left, about: mid(divider.above, across)) in "hall_cupboard.stave"
}
"#,
        );
        let shown = render_diagnostics(tried.sources(), &tried.shown());
        let codes = refused_codes(&tried).expect("refused");
        assert_eq!(codes, [Code::Overlap], "{shown}");
        assert!(shown.contains("door_right"), "{shown}");
    }

    #[test]
    fn a_design_that_cannot_be_placed_refuses_edits_it_cannot_check() {
        // The failing `require` stops the solver writing a design, so no
        // candidate's fit can be checked: none fits, not even one that
        // would pass through the divider, and nothing is written.
        let files = cabinet_with_a_failing_require();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        assert_eq!(base.opened().error_count(), 1);
        assert!(base.opened().design.is_none());
        for text in [ADD_SHELF, THROUGH_DIVIDER] {
            let tried = try_patch(&base, text);
            assert_eq!(refused_codes(&tried), Some(vec![Code::NotSupported]));
            let shown = render_diagnostics(tried.sources(), &tried.shown());
            assert!(
                shown.starts_with(
                    "error[not-supported]: the edit can't be checked: the design already has this error, which stops it being placed\n  --> scratch/clothing_cabinet/cabinet.stave:"
                ),
                "{shown}"
            );
            assert!(
                shown.contains("= note: the error: error[require]: "),
                "{shown}"
            );
            let mut written = files.clone();
            assert!(commit(&tried, &mut written).is_err());
            assert_eq!(written, files);
        }
        // A new error is the edit's own, and refuses as itself.
        let tried = try_patch(
            &base,
            "stave 1\n\npatch \"Another rule\" {\n  add require too_shallow : cabinet_depth >= 600mm \"deep\" in \"cabinet.stave\"\n}\n",
        );
        assert_eq!(refused_codes(&tried), Some(vec![Code::Require]));
    }

    fn files_text(files: &MemoryFiles, path: &Path) -> String {
        use stave_modules::FileSource as _;
        files.read(path).unwrap()
    }

    #[test]
    fn a_patch_that_cannot_be_applied_is_refused_with_why() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let tried = try_patch(
            &base,
            "stave 1\n\npatch \"Wider\" {\n  set no_such_param = 950mm\n}\n",
        );
        assert_eq!(refused_codes(&tried), Some(vec![Code::UnknownName]));
        assert!(tried.changed().is_empty());
        assert!(tried.opened().is_none());
        let shown = render_diagnostics(tried.sources(), &tried.shown());
        assert!(
            shown.starts_with(
                "error[unknown-name]: there is no item named 'no_such_param' in the project\n  --> change.stave:4:3\n"
            ),
            "{shown}"
        );
        let mut written = files.clone();
        assert!(commit(&tried, &mut written).is_err());
        assert_eq!(written, files);
    }

    #[test]
    fn many_candidates_are_tried_from_one_base_without_changing_it() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let before = base.opened().diagnostics.clone();
        for _ in 0..2 {
            assert_eq!(refused_codes(&try_patch(&base, ADD_SHELF)), None);
            assert!(refused_codes(&try_patch(&base, THROUGH_DIVIDER)).is_some());
        }
        assert_eq!(base.opened().diagnostics, before);
    }

    #[test]
    fn the_fast_tier_says_checking_and_the_slow_tier_can_be_cancelled() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let scratch = base.start("change.stave", &patch(THROUGH_DIVIDER)).unwrap();
        // The overlap is a layer 4 error: the fast tier only sees boxes.
        assert_eq!(scratch.verdict(), Verdict::Checking);
        assert!(!scratch.overlaps().is_empty());
        let cancel = AtomicBool::new(true);
        assert!(scratch.finish(&cancel).is_err());
        cancel.store(false, Ordering::Relaxed);
        let scratch = base.start("change.stave", &patch(THROUGH_DIVIDER)).unwrap();
        assert!(refused_codes(&scratch.finish(&cancel).unwrap()).is_some());
        assert!(matches!(
            base.try_patch("change.stave", &patch(ADD_SHELF), &AtomicBool::new(true)),
            Err(TryError::Cancelled(_))
        ));
    }

    #[test]
    fn a_base_advances_past_a_fit() {
        let files = cabinet();
        let mut base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let tried = try_patch(&base, ADD_SHELF);
        base.advance(tried).unwrap();
        assert!(base.opened().diagnostics.iter().all(|d| !d.is_error()));
        // The shelf is there now, so adding it again is refused.
        let again = try_patch(&base, ADD_SHELF);
        assert_eq!(refused_codes(&again), Some(vec![Code::DuplicateName]));
        assert!(matches!(base.advance(again), Err(CommitError::Refused)));
    }

    #[test]
    fn a_patch_file_splits_into_one_patch_each_keeping_offsets() {
        let text =
            "stave 1\n\npatch \"One\" {\n  set a = 1mm\n}\n\npatch \"Two\" {\n  set b = 2mm\n}\n";
        let mut sources = SourceMap::new();
        let file = sources.add("change.stave", text).unwrap();
        let whole = parse_patch_file(file, text);
        let each = one_patch_each(file, &whole);
        assert_eq!(each.len(), 2);
        for (index, one) in each.iter().enumerate() {
            let ast = one.ast.as_ref().unwrap();
            assert_eq!(ast.patches.len(), 1);
            let original = whole.ast.as_ref().unwrap().patches.get(index).unwrap();
            assert_eq!(ast.patches.first().unwrap().span, original.span);
            assert_eq!(one.syntax().to_string().len(), text.len());
        }
    }
}

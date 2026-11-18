//! A change of the design that doesn't come from the menu
//! (`docs/editing.md`, "Changes"): repeat an assembly or change how many
//! pieces it has, move the boundary between two pieces of a split, add
//! the mirror image of an assembly, hold a measurement, remove an item,
//! rename one. Each is written by its pure function ([`crate::repeat`],
//! [`crate::hold`], [`crate::remove`], the rename of
//! [`stave_ide::Analysis`]), judged by the same scratch check as a menu
//! pick ([`crate::scratch`]) and committed by the same writer
//! ([`crate::editing::write_all`]).
//!
//! ```text
//! ChangeRequest ──Change::start──▶ patch (+ the rename's edits) ──scratch check──▶ Progress
//!                                   │ ApplyFix: the fix's patch is added, tried again
//!                                   │ IncludeDependants: a removal takes what uses it too
//!                                   ▼
//!                       Change::commit (only on Fits) ──WriteEdits──▶ buffers or disk
//! ```
//!
//! A rename isn't a patch operation the patcher applies (it needs every
//! use, which only the navigation layer knows), so a rename, and the
//! rename a mirror makes, comes as whole-file texts: the change is tried
//! on a scratch copy with those texts in place, then the patch (if any)
//! on top of it. What is written is, per file, the edit from the text the
//! change started from to the final text, so a file changed since is
//! never overwritten.

use std::collections::BTreeMap;
use std::io;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use stave_ide::{Analysis, Document, RenameEdit};
use stave_lang::ast::{Ident, Name, Span};
use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_lang::{Fix as LangFix, SourceMap, TextEdit, parse_patch_file};
use stave_model::nonempty::NonEmpty;
use stave_modules::FileSource;

use crate::Opened;
use crate::candidates::EditScope;
use crate::candidates::new_project::{NewProject, NewProjectError, new_project_text};
use crate::edit::{Fits, Problem, Refusal, Verdict};
use crate::editing::{
    Commit, CommitError, EditEvent, FileEdit, Progress, Rejection, StartError, WriteEdits,
    write_all,
};
use crate::hold::{HoldRequest, hold_patch};
use crate::naming::RepeatCount;
use crate::remove::{Dependants, Removal, patch_for, removal};
use crate::repeat::{RepeatRequest, SharesMove, mirror_assembly, move_shares, repeat_assembly};
use crate::scratch::{EditBase, Tried, TryError};

/// The name a change's patch file is shown under in its diagnostics.
pub const CHANGE_PATCH_NAME: &str = "change.stave";

/// A change the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeRequest {
    /// Repeat an assembly that fills a region `count` times, or change
    /// the count of one already repeated ([`crate::repeat::repeat_assembly`]).
    Repeat {
        /// The assembly: `drawer`.
        item: Ident,
        /// How many pieces it becomes.
        count: RepeatCount,
        /// The axis to split its region along; a first repeat needs it.
        axis: Option<Axis>,
        /// Relative sizes, one per piece; `None` for equal pieces.
        shares: Option<Vec<NonZeroU32>>,
    },
    /// Move the boundary between two neighbouring pieces of a split
    /// ([`crate::repeat::move_shares`]).
    MoveShares {
        /// The region array: `drawer_bay`.
        region: Ident,
        /// The first of the two pieces, counted from 0 in label order.
        lower: usize,
        /// The new size of that piece along the split.
        boundary: Length,
    },
    /// Add the mirror image of an assembly
    /// ([`crate::repeat::mirror_assembly`]).
    Mirror {
        /// The assembly: `door`.
        item: Ident,
    },
    /// Hold a measured distance with a minimum or a maximum
    /// ([`crate::hold::hold_patch`]).
    Hold(HoldRequest),
    /// Remove an item; while something depends on it, the change lists
    /// what and waits for [`Change::include_dependants`].
    Remove {
        /// The item.
        item: Ident,
    },
    /// Give an item of the project another name, everywhere it is used.
    Rename {
        /// The item.
        item: Ident,
        /// Its new name.
        to: Ident,
    },
}

/// What a change does, for its card: the words and what the change found
/// along the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeDetail {
    /// A repeat or a shares grip.
    Repeat,
    /// A mirror: the source is renamed to its side and the image added.
    Mirror {
        /// The name the source takes: `door_left`.
        renamed: Ident,
        /// The image's name: `door_right`.
        image: Ident,
        /// The plane it is mirrored about, as written.
        about: String,
    },
    /// A held rule.
    Hold {
        /// The rule's name: `slat_to_apron_front_min`.
        name: Ident,
        /// Its condition as written.
        condition: String,
    },
    /// A removal and what depends on the item.
    Remove {
        /// The item and its dependants, nearest first.
        removal: Removal,
        /// Whether the dependants are removed too.
        dependants: Dependants,
    },
    /// A rename.
    Rename {
        /// The old name.
        from: String,
        /// The new name.
        to: Ident,
    },
}

/// A change in progress: what was asked, its patch and its verdict.
pub struct Change {
    folder: PathBuf,
    files: Arc<dyn FileSource + Send + Sync>,
    base: EditBase<'static>,
    request: ChangeRequest,
    summary: String,
    detail: ChangeDetail,
    /// The rename's files (path and whole new text), applied before the
    /// patch.
    renamed: Vec<(PathBuf, String)>,
    /// The change's own patch file, without fixes; `None` for a rename
    /// alone.
    patch: Option<String>,
    fixes: Vec<LangFix>,
    /// The patch as last tried: the change's, then each fix.
    tried_patch: Option<String>,
    progress: Progress,
    tried: Option<Tried>,
    /// The scratch base with the rename in place, kept for the ghost of
    /// a rename alone.
    renamed_base: Option<EditBase<'static>>,
}

impl std::fmt::Debug for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Change")
            .field("folder", &self.folder)
            .field("request", &self.request)
            .field("summary", &self.summary)
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}

/// What a mirror would add, without trying it: the offer the card makes
/// for an assembly that has a mirror image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorOffer {
    /// The assembly as named now.
    pub source: String,
    /// The name it would take: `door_left`.
    pub renamed: Ident,
    /// The image's name: `door_right`.
    pub image: Ident,
    /// The plane, as written: `mid(base, across)`.
    pub about: String,
}

/// Why a change can't start.
#[derive(Debug, thiserror::Error)]
pub enum ChangeError {
    /// The project can't be opened, resolved or placed ([`StartError`]).
    #[error(transparent)]
    Start(#[from] StartError),
    /// The change can't be written for this design, in words (an
    /// unknown item, a repeat of something that fills no space, no
    /// mirror image, a name already used, ...).
    #[error("{0}")]
    Unwritable(String),
}

impl Change {
    /// Starts a change of the project in `folder`, read through `files`:
    /// writes its patch and tries it.
    ///
    /// # Errors
    ///
    /// The project can't be opened or doesn't place, or the change can't
    /// be written for it ([`ChangeError`]).
    pub fn start(
        folder: &Path,
        files: Arc<dyn FileSource + Send + Sync>,
        request: ChangeRequest,
    ) -> Result<Change, ChangeError> {
        let base = EditBase::shared(folder, files.clone()).map_err(StartError::from)?;
        if base.opened().design.is_none() {
            return Err(StartError::NotSolved.into());
        }
        let mut change = Change {
            folder: folder.to_path_buf(),
            files,
            base,
            request: request.clone(),
            summary: String::new(),
            detail: ChangeDetail::Repeat,
            renamed: Vec::new(),
            patch: None,
            fixes: Vec::new(),
            tried_patch: None,
            progress: Progress::Unwritable(String::new()),
            tried: None,
            renamed_base: None,
        };
        change.write(&request, Dependants::Refuse)?;
        change.retry();
        Ok(change)
    }

    /// The project folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// What was asked.
    pub fn request(&self) -> &ChangeRequest {
        &self.request
    }

    /// What the change does, in words: "Make 3 of drawer".
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// What the change found: the mirror's names, the held rule, what
    /// depends on a removed item, the rename.
    pub fn detail(&self) -> &ChangeDetail {
        &self.detail
    }

    /// Where it stands: *needs a choice* (a removal with dependants, which
    /// names them), *fits* or *refused*.
    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// The patch as last tried (the change's, then each fix), when there
    /// is one; a rename alone has none.
    pub fn patch(&self) -> Option<&str> {
        self.tried_patch.as_deref()
    }

    /// The files a rename rewrites before the patch, by path.
    pub fn renamed_files(&self) -> impl Iterator<Item = &Path> {
        self.renamed.iter().map(|(path, _)| path.as_path())
    }

    /// The fixes added so far.
    pub fn fixes(&self) -> &[LangFix] {
        &self.fixes
    }

    /// The last try, with its sources for rendering its diagnostics;
    /// `None` for a rename alone, or before anything was tried.
    pub fn tried(&self) -> Option<&Tried> {
        self.tried.as_ref()
    }

    /// The design as it would be after the change, for the ghost.
    pub fn ghost(&self) -> Option<&Opened> {
        match &self.tried {
            Some(tried) => tried.opened(),
            None => self.renamed_base.as_ref().map(EditBase::opened),
        }
    }

    /// The design the change is tried on, as it was when it began.
    pub fn base(&self) -> &Opened {
        self.base.opened()
    }

    /// A removal waiting on its dependants: remove them too, and try
    /// again.
    pub fn include_dependants(&mut self) -> Vec<EditEvent> {
        let ChangeRequest::Remove { item } = self.request.clone() else {
            return vec![EditEvent::Rejected(Rejection::NothingToInclude)];
        };
        if let ChangeDetail::Remove {
            dependants: Dependants::Included,
            ..
        } = self.detail
        {
            return vec![EditEvent::Rejected(Rejection::NothingToInclude)];
        }
        let request = ChangeRequest::Remove { item };
        if let Err(error) = self.write(&request, Dependants::Included) {
            return vec![EditEvent::Rejected(Rejection::CantStart(error.to_string()))];
        }
        let mut events = vec![EditEvent::DependantsIncluded];
        let before = self.progress.clone();
        self.retry();
        if self.progress != before {
            events.push(EditEvent::VerdictChanged);
        }
        events
    }

    /// Adds the fix at `fix` of the problem at `problem` (both from 0) of
    /// the refusal to the change, and tries it again.
    pub fn apply_fix(&mut self, problem: usize, fix: usize) -> Vec<EditEvent> {
        let found = match &self.progress {
            Progress::Tried(Verdict::Refused(refusal)) => refusal
                .problems()
                .get(problem)
                .and_then(|problem| problem.diagnostic().fixes.get(fix))
                .cloned(),
            Progress::Tried(Verdict::Fits(_) | Verdict::Checking)
            | Progress::NeedsChoice(_)
            | Progress::Unwritable(_) => None,
        };
        let Some(found) = found else {
            return vec![EditEvent::Rejected(Rejection::NoSuchFix)];
        };
        self.fixes.push(found);
        let mut events = vec![EditEvent::FixApplied];
        let before = self.progress.clone();
        self.retry();
        if self.progress != before {
            events.push(EditEvent::VerdictChanged);
        }
        events
    }

    /// Writes the change into the text through `to`, when it fits: every
    /// file is checked before any is written, and one changed since the
    /// change began stops the commit with nothing written.
    ///
    /// # Errors
    ///
    /// A change that doesn't fit, or a file that can't be written
    /// ([`CommitError`]).
    pub fn commit(&mut self, to: &mut dyn WriteEdits) -> Result<Commit, CommitError> {
        if !matches!(self.progress, Progress::Tried(Verdict::Fits(_))) {
            return Err(CommitError::DoesNotFit);
        }
        let files = self.file_edits();
        write_all(&files, to)?;
        let opened = match self.tried.take() {
            Some(tried) => tried.into_opened(),
            None => {
                let overlay = Renamed {
                    inner: self.files.clone(),
                    texts: self.renamed.iter().cloned().collect(),
                };
                crate::open_with(&self.folder, &overlay).ok()
            }
        }
        .ok_or(CommitError::DoesNotFit)?;
        Ok(Commit { files, opened })
    }

    /// Writes the change's patch (and rename) for `request`.
    fn write(
        &mut self,
        request: &ChangeRequest,
        dependants: Dependants,
    ) -> Result<(), ChangeError> {
        let unwritable = |error: &dyn std::fmt::Display| ChangeError::Unwritable(error.to_string());
        let opened = self.base.opened();
        let scope = EditScope::new(opened).map_err(|error| unwritable(&error))?;
        let design = opened.design.as_ref().ok_or(StartError::NotSolved)?;
        self.renamed.clear();
        self.fixes.clear();
        match request {
            ChangeRequest::Repeat {
                item,
                count,
                axis,
                shares,
            } => {
                let patch = repeat_assembly(
                    &scope,
                    RepeatRequest {
                        assembly: item.as_str(),
                        count: *count,
                        axis: *axis,
                        shares: shares.as_deref(),
                    },
                )
                .map_err(|error| unwritable(&error))?;
                self.summary = patch.summary;
                self.patch = Some(patch.text);
                self.detail = ChangeDetail::Repeat;
            }
            ChangeRequest::MoveShares {
                region,
                lower,
                boundary,
            } => {
                let patch = move_shares(
                    &scope,
                    design,
                    SharesMove {
                        region: region.as_str(),
                        lower: *lower,
                        boundary: *boundary,
                    },
                )
                .map_err(|error| unwritable(&error))?;
                self.summary = patch.summary;
                self.patch = Some(patch.text);
                self.detail = ChangeDetail::Repeat;
            }
            ChangeRequest::Mirror { item } => {
                let analysis = self.analysis()?;
                let plan = mirror_assembly(&scope, &analysis, design, item.as_str())
                    .map_err(|error| unwritable(&error))?;
                self.renamed = rename_files(&plan.rename);
                self.summary = plan.patch.summary;
                self.patch = Some(plan.patch.text);
                self.detail = ChangeDetail::Mirror {
                    renamed: plan.renamed,
                    image: plan.image,
                    about: plan.about,
                };
            }
            ChangeRequest::Hold(hold) => {
                let held = hold_patch(&scope, design, hold).map_err(|error| unwritable(&error))?;
                self.summary = held.patch.summary;
                self.patch = Some(held.patch.text);
                self.detail = ChangeDetail::Hold {
                    name: held.name,
                    condition: held.condition,
                };
            }
            ChangeRequest::Remove { item } => {
                let analysis = self.analysis()?;
                let found =
                    removal(&analysis, item.as_str()).map_err(|error| unwritable(&error))?;
                let mut taken = found.clone();
                if dependants == Dependants::Refuse {
                    taken.dependants.clear();
                }
                let patch = patch_for(&taken).map_err(|error| unwritable(&error))?;
                self.summary = patch.summary;
                self.patch = Some(patch.text);
                self.detail = ChangeDetail::Remove {
                    removal: found,
                    dependants,
                };
            }
            ChangeRequest::Rename { item, to } => {
                let analysis = self.analysis()?;
                let taken = crate::naming::TakenNames::of_project(scope.model());
                if taken.is_unavailable(to.as_str()) {
                    return Err(ChangeError::Unwritable(
                        Rejection::NameUnavailable(to.as_str().to_owned()).text(),
                    ));
                }
                let symbol = analysis.project_symbol(item.as_str()).ok_or_else(|| {
                    ChangeError::Unwritable(format!(
                        "there is no item named '{}' in this project",
                        item.as_str()
                    ))
                })?;
                let edits = analysis
                    .rename(symbol.id, to.as_str())
                    .map_err(|error| unwritable(&error))?;
                self.renamed = rename_files(&edits);
                self.summary = format!("Rename {} to {}", item.as_str(), to.as_str());
                self.patch = None;
                self.detail = ChangeDetail::Rename {
                    from: item.as_str().to_owned(),
                    to: to.clone(),
                };
            }
        }
        Ok(())
    }

    fn analysis(&self) -> Result<Analysis, ChangeError> {
        Analysis::load(&self.folder, self.files.as_ref())
            .map_err(|error| ChangeError::Unwritable(error.to_string()))
    }

    /// Tries the change as it now is, recording where it stands.
    fn retry(&mut self) {
        self.tried = None;
        self.tried_patch = None;
        self.renamed_base = None;
        if let ChangeDetail::Remove {
            removal,
            dependants: Dependants::Refuse,
        } = &self.detail
        {
            let names: Vec<Name> = removal
                .dependants
                .iter()
                .filter_map(|item| Name::new(&item.name, Span::default()).ok())
                .collect();
            if let Some(names) = NonEmpty::new(names) {
                self.progress = Progress::NeedsChoice(names);
                return;
            }
        }
        let mut text = self.patch.clone();
        for fix in &self.fixes {
            let joined = text.get_or_insert_with(|| "stave 1\n".to_owned());
            joined.push('\n');
            joined.push_str(&fix.to_string());
            joined.push('\n');
        }
        // The base the patch is tried on: the change's own, or a scratch
        // base with the rename in place.
        let renamed_base = if self.renamed.is_empty() {
            None
        } else {
            let overlay = Renamed {
                inner: self.files.clone(),
                texts: self.renamed.iter().cloned().collect(),
            };
            match EditBase::shared(&self.folder, Arc::new(overlay)) {
                Ok(base) => Some(base),
                Err(error) => {
                    self.progress = Progress::Unwritable(error.to_string());
                    return;
                }
            }
        };
        let Some(text) = text else {
            // A rename alone: it fits when it leaves the design placed and
            // causes no error.
            if let Some(renamed) = renamed_base {
                self.progress =
                    Progress::Tried(rename_verdict(self.base.opened(), renamed.opened()));
                self.renamed_base = Some(renamed);
            }
            return;
        };
        let base = renamed_base.as_ref().unwrap_or(&self.base);
        let parse = match SourceMap::new().add(CHANGE_PATCH_NAME, text.as_str()) {
            Ok(file) => parse_patch_file(file, &text),
            Err(error) => {
                self.progress = Progress::Unwritable(error.to_string());
                return;
            }
        };
        match base.try_patch(CHANGE_PATCH_NAME, &parse, &AtomicBool::new(false)) {
            Ok(tried) => {
                let mut verdict = tried.verdict().clone();
                if let Some(renamed) = &renamed_base
                    && let Verdict::Refused(_) =
                        rename_verdict(self.base.opened(), renamed.opened())
                {
                    verdict = rename_verdict(self.base.opened(), renamed.opened());
                }
                self.progress = Progress::Tried(verdict);
                self.tried = Some(tried);
            }
            Err(TryError::Load(error)) => self.progress = Progress::Unwritable(error.to_string()),
            Err(TryError::Cancelled(_)) => {
                self.progress = Progress::Unwritable("the check was stopped".to_owned());
            }
        }
        self.tried_patch = Some(text);
        self.renamed_base = renamed_base;
    }

    /// Per file, the edit from the text the change started from to the
    /// final text: the rename's, then the patch's on top, in path order.
    fn file_edits(&self) -> Vec<FileEdit> {
        let mut finals: BTreeMap<PathBuf, (String, bool)> = self
            .renamed
            .iter()
            .map(|(path, text)| (path.clone(), (text.clone(), false)))
            .collect();
        if let Some(tried) = &self.tried {
            for changed in tried.changed() {
                let created = changed.created && !finals.contains_key(&changed.path);
                finals.insert(changed.path.clone(), (changed.text.clone(), created));
            }
        }
        finals
            .into_iter()
            .map(|(path, (after, created))| {
                let before = if created {
                    String::new()
                } else {
                    self.base
                        .opened()
                        .sources
                        .files()
                        .find(|(_, source)| Path::new(source.path()) == path)
                        .map(|(_, source)| source.text().to_owned())
                        .unwrap_or_default()
                };
                let edits = smallest_edit(&before, &after).into_iter().collect();
                FileEdit {
                    path,
                    created,
                    before,
                    edits,
                    after,
                }
            })
            .collect()
    }
}

/// What a mirror of `item` would add in the design, when it has a mirror
/// image: the offer the card makes. `None` when it has none, or the
/// project doesn't place.
pub fn mirror_offer(
    folder: &Path,
    files: &dyn FileSource,
    opened: &Opened,
    item: &str,
) -> Option<MirrorOffer> {
    let scope = EditScope::new(opened).ok()?;
    let design = opened.design.as_ref()?;
    let analysis = Analysis::load(folder, files).ok()?;
    let plan = mirror_assembly(&scope, &analysis, design, item).ok()?;
    Some(MirrorOffer {
        source: plan.source,
        renamed: plan.renamed,
        image: plan.image,
        about: plan.about,
    })
}

/// The file a new project is written to in `folder`, as a commit writes
/// it: created, never over a file that is there. Its name is the title in
/// lower case with `_` between words (`Hall table` gives
/// `hall_table.stave`; `project.stave` when the title has no letters).
///
/// # Errors
///
/// The project's text can't be written ([`NewProjectError`]).
pub fn new_project_file(folder: &Path, project: &NewProject) -> Result<FileEdit, NewProjectError> {
    let text = new_project_text(project)?;
    let mut stem = String::new();
    for c in project.title.chars() {
        if c.is_ascii_alphanumeric() {
            stem.push(c.to_ascii_lowercase());
        } else if !stem.is_empty() && !stem.ends_with('_') {
            stem.push('_');
        }
    }
    let stem = stem.trim_end_matches('_');
    let name = if stem.is_empty() || !stem.starts_with(|c: char| c.is_ascii_lowercase()) {
        "project".to_owned()
    } else {
        stem.to_owned()
    };
    Ok(FileEdit {
        path: folder.join(format!("{name}.stave")),
        created: true,
        before: String::new(),
        edits: vec![TextEdit {
            range: 0..0,
            text: text.clone(),
        }],
        after: text,
    })
}

/// The verdict of a rename by itself: *refused* with the errors of the
/// renamed design when it has more errors than the design had, or no
/// longer places; otherwise *fits* with no warnings.
fn rename_verdict(base: &Opened, renamed: &Opened) -> Verdict {
    let worse = renamed.error_count() > base.error_count()
        || (base.design.is_some() && renamed.design.is_none());
    if worse {
        let problems: Vec<Problem> = renamed
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .filter_map(|diagnostic| {
                Problem::new(diagnostic.clone(), Vec::new())
                    .ok()
                    .map(|problem| {
                        problem.with_rendered(stave_lang::render_diagnostic(
                            &renamed.sources,
                            diagnostic,
                        ))
                    })
            })
            .collect();
        if let Ok(refusal) = Refusal::new(problems) {
            return Verdict::Refused(refusal);
        }
    }
    match Fits::new(Vec::new()) {
        Ok(fits) => Verdict::Fits(fits),
        Err(_) => Verdict::Checking,
    }
}

/// A rename's edits as whole-file texts, by path. Standard files are
/// never renamed in.
fn rename_files(edits: &[RenameEdit]) -> Vec<(PathBuf, String)> {
    edits
        .iter()
        .filter_map(|edit| match &edit.document {
            Document::File(path) => Some((path.clone(), edit.text.clone())),
            Document::Std { .. } => None,
        })
        .collect()
}

/// The one edit that turns `before` into `after`: what lies between their
/// common start and common end. `None` when they are the same.
pub fn smallest_edit(before: &str, after: &str) -> Option<TextEdit> {
    if before == after {
        return None;
    }
    let mut start = before
        .bytes()
        .zip(after.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !before.is_char_boundary(start) || !after.is_char_boundary(start) {
        start = start.saturating_sub(1);
    }
    let longest_end = before.len().min(after.len()).saturating_sub(start);
    let mut end = before
        .bytes()
        .rev()
        .zip(after.bytes().rev())
        .take(longest_end)
        .take_while(|(a, b)| a == b)
        .count();
    while !before.is_char_boundary(before.len() - end) || !after.is_char_boundary(after.len() - end)
    {
        end = end.saturating_sub(1);
    }
    let text = after.get(start..after.len() - end)?.to_owned();
    Some(TextEdit {
        range: start..before.len() - end,
        text,
    })
}

/// The project's files with a rename's texts in place.
struct Renamed {
    inner: Arc<dyn FileSource + Send + Sync>,
    texts: BTreeMap<PathBuf, String>,
}

impl FileSource for Renamed {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        self.inner.list_stave_files(folder)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        match self.texts.get(path) {
            Some(text) => Ok(text.clone()),
            None => self.inner.read(path),
        }
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        self.inner.canonical(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::smallest_edit;

    #[test]
    fn the_smallest_edit_covers_only_what_changed() {
        let edit = smallest_edit(
            "param a = 1mm\nparam b = 2mm\n",
            "param a = 1mm\nparam c = 2mm\n",
        )
        .unwrap();
        assert_eq!(edit.range, 20..21);
        assert_eq!(edit.text, "c");
        assert!(smallest_edit("same", "same").is_none());
        let edit = smallest_edit("aaa", "aaaa").unwrap();
        assert_eq!(
            stave_lang::apply_text_edits("aaa", &[edit]).unwrap(),
            "aaaa"
        );
    }
}

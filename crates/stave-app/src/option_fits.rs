//! Options are checked (`docs/editing.md`, "Options are checked"): each
//! option of each setting of the chosen candidate is tried in a scratch
//! copy, with that setting set to that option and every other setting as
//! it stands, so the card can grey the options that can't work with the
//! checker's own reason, as the menu greys its entries.
//!
//! ```text
//! retry of the chosen candidate ──prepare──▶ every option NotChecked (a new ticket)
//!                                            the option it holds now: from its own verdict
//!                                            a candidate still waiting on something else: NeedsChoice
//! OptionsJob::run on another thread ──▶ FoundFit ──Session::option_fit_found──▶ Fits / Refused
//! ```
//!
//! The job prepares the project itself, so it can run off the main
//! thread; it hands each answer back as soon as it is known and stops
//! between checks when its cancel flag is set. Every change to the chosen
//! candidate throws the answers away and takes a new ticket, so an answer
//! for the candidate as it was is dropped; the options of a setting just
//! set keep theirs (the other settings, which they were tried with, did
//! not change).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use stave_check::Cancelled;
use stave_lang::Fix as LangFix;
use stave_lang::ast::{Ident, Name, Span};
use stave_model::nonempty::NonEmpty;
use stave_modules::FileSource;

use crate::candidates::Repeat;
use crate::candidates::applicability::unchecked;
use crate::edit::{Candidate, OptionFit, OptionRefusal, SettingOption, SettingValue, Verdict};
use crate::editing::{Progress, parse_value, try_candidate};
use crate::scratch::EditBase;

/// The candidate as it stood when its options' checks were asked for: an
/// answer for another ticket is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OptionsTicket(u64);

/// The next ticket; counted across every edit, so an answer for an edit
/// that was dropped never matches a later one.
static NEXT_TICKET: AtomicU64 = AtomicU64::new(0);

impl OptionsTicket {
    /// A ticket no earlier candidate had.
    pub(crate) fn next() -> OptionsTicket {
        OptionsTicket(NEXT_TICKET.fetch_add(1, Ordering::Relaxed))
    }
}

/// One option to try: a setting of the chosen candidate and one of its
/// options, as `.stave` text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionTry {
    /// The setting: `joinery`.
    pub setting: Ident,
    /// The option: `chigiri`.
    pub option: String,
}

/// One option's fit, found by an [`OptionsJob`], for
/// [`crate::session::Session::option_fit_found`].
#[derive(Debug, Clone, PartialEq)]
pub struct FoundFit {
    /// The candidate it was tried for.
    pub ticket: OptionsTicket,
    /// The setting.
    pub setting: Ident,
    /// The option, as `.stave` text.
    pub option: String,
    /// Whether it fits.
    pub fit: OptionFit,
}

/// The checks of the options of the chosen candidate's settings, apart
/// from the session so it can run on another thread: it prepares the
/// project again itself.
#[derive(Clone)]
pub struct OptionsJob {
    folder: PathBuf,
    files: Arc<dyn FileSource + Send + Sync>,
    ticket: OptionsTicket,
    candidate: Candidate,
    repeat: Option<Repeat>,
    name: Option<Ident>,
    fixes: Vec<LangFix>,
    tries: Vec<OptionTry>,
}

impl std::fmt::Debug for OptionsJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OptionsJob")
            .field("folder", &self.folder)
            .field("ticket", &self.ticket)
            .field("tries", &self.tries)
            .finish_non_exhaustive()
    }
}

impl OptionsJob {
    /// The job for `candidate` as it stands (with its repeat, typed name
    /// and fixes): every option still [`OptionFit::NotChecked`], setting
    /// by setting in card order, each setting's options in list order.
    pub(crate) fn new(
        folder: &Path,
        files: Arc<dyn FileSource + Send + Sync>,
        ticket: OptionsTicket,
        candidate: &Candidate,
        repeat: Option<Repeat>,
        name: Option<&Ident>,
        fixes: &[LangFix],
    ) -> OptionsJob {
        let tries = candidate
            .settings
            .iter()
            .flat_map(|setting| {
                setting
                    .options
                    .iter()
                    .filter(|option| option.fit == OptionFit::NotChecked)
                    .map(|option| OptionTry {
                        setting: setting.name.clone(),
                        option: option.text.clone(),
                    })
            })
            .collect();
        OptionsJob {
            folder: folder.to_path_buf(),
            files,
            ticket,
            candidate: candidate.clone(),
            repeat,
            name: name.cloned(),
            fixes: fixes.to_vec(),
            tries,
        }
    }

    /// The candidate it is for.
    pub fn ticket(&self) -> OptionsTicket {
        self.ticket
    }

    /// The options it will try, in order.
    pub fn tries(&self) -> &[OptionTry] {
        &self.tries
    }

    /// The same job for the options of one setting only.
    #[must_use]
    pub fn only(mut self, setting: &str) -> OptionsJob {
        self.tries.retain(|one| one.setting.as_str() == setting);
        self
    }

    /// The same job with the options of one setting first (the one whose
    /// list is open), the rest after in their order.
    #[must_use]
    pub fn prefer(mut self, setting: &str) -> OptionsJob {
        let (mut first, rest): (Vec<OptionTry>, Vec<OptionTry>) = self
            .tries
            .into_iter()
            .partition(|one| one.setting.as_str() == setting);
        first.extend(rest);
        self.tries = first;
        self
    }

    /// Tries each option in turn, handing each answer to `found` as soon
    /// as it is known. Setting `cancel` stops it between checks (and
    /// inside a slow check); the options reached keep their answer.
    ///
    /// # Errors
    ///
    /// The project can't be prepared, or the job was cancelled, in words.
    pub fn run(&self, cancel: &AtomicBool, found: &mut dyn FnMut(FoundFit)) -> Result<(), String> {
        if self.tries.is_empty() {
            return Ok(());
        }
        let base = EditBase::shared(&self.folder, self.files.clone()).map_err(|e| e.to_string())?;
        for one in &self.tries {
            if cancel.load(Ordering::Relaxed) {
                return Err(Cancelled.to_string());
            }
            let fit = self
                .fit_of_option(&base, one, cancel)
                .map_err(|cancelled| cancelled.to_string())?;
            found(FoundFit {
                ticket: self.ticket,
                setting: one.setting.clone(),
                option: one.option.clone(),
                fit,
            });
        }
        Ok(())
    }

    /// Builds the candidate with one setting set to one option and checks
    /// it.
    fn fit_of_option(
        &self,
        base: &EditBase<'_>,
        one: &OptionTry,
        cancel: &AtomicBool,
    ) -> Result<OptionFit, Cancelled> {
        let value = match parse_value(&one.option) {
            Ok(value) => value,
            Err(error) => return Ok(refused_for(&error.to_string())),
        };
        let mut candidate = self.candidate.clone();
        let Some(setting) = candidate
            .settings
            .iter_mut()
            .find(|setting| setting.name.as_str() == one.setting.as_str())
        else {
            return Ok(refused_for("the candidate has no such setting"));
        };
        setting.value = SettingValue::Chosen(value);
        let (progress, ..) = try_candidate(
            base,
            &candidate,
            self.repeat,
            self.name.as_ref(),
            &self.fixes,
            cancel,
        )?;
        Ok(fit_of(&progress))
    }
}

/// An option refused for a reason that has no place in a file.
fn refused_for(message: &str) -> OptionFit {
    OptionFit::Refused(OptionRefusal {
        first: unchecked(message),
        more: 0,
    })
}

/// The fit a finished try gives: fits, refused with its first error, or
/// still waiting on a choice. A try still checking is not an answer.
pub(crate) fn fit_of(progress: &Progress) -> OptionFit {
    match progress {
        Progress::NeedsChoice(names) => OptionFit::NeedsChoice(names.clone()),
        Progress::Unwritable(reason) => refused_for(reason),
        Progress::Tried(Verdict::Fits(_)) => OptionFit::Fits,
        Progress::Tried(Verdict::Refused(refusal)) => {
            let problems = refusal.problems();
            OptionFit::Refused(OptionRefusal {
                first: problems.first().diagnostic().clone(),
                more: problems.count().saturating_sub(1),
            })
        }
        Progress::Tried(Verdict::Checking) => OptionFit::NotChecked,
    }
}

/// Fills the fits of every option of `candidate`, just described and
/// tried with `progress`, before any is checked: an option of a setting
/// the candidate would still wait on something else for can't be tried
/// ([`OptionFit::NeedsChoice`]); the option the setting holds now has the
/// candidate's own verdict; the options of `kept` (a setting just set,
/// with its options as they were) keep their answers; the rest are not
/// checked yet. Then each setting's refused options go after the rest.
pub(crate) fn prepare(
    candidate: &mut Candidate,
    progress: &Progress,
    kept: Option<(&Ident, &[SettingOption])>,
) {
    let place: Vec<Name> = candidate
        .unbound_place()
        .map(|param| param.name.clone())
        .collect();
    let unset: Vec<Ident> = candidate
        .unset_settings()
        .map(|setting| setting.name.clone())
        .collect();
    for setting in &mut candidate.settings {
        // What would still be missing with this setting given a value.
        let waiting: Vec<Name> = place
            .iter()
            .cloned()
            .chain(
                unset
                    .iter()
                    .filter(|name| name.as_str() != setting.name.as_str())
                    .filter_map(|name| Name::new(name.as_str(), Span::default()).ok()),
            )
            .collect();
        let waiting = NonEmpty::new(waiting);
        let kept_options = kept
            .filter(|(name, _)| name.as_str() == setting.name.as_str())
            .map(|(_, options)| options);
        for option in &mut setting.options {
            option.fit = if let Some(names) = &waiting {
                OptionFit::NeedsChoice(names.clone())
            } else if let Some(fit) = kept_options
                .and_then(|options| options.iter().find(|old| old.text == option.text))
                .map(|old| old.fit.clone())
                .filter(|fit| *fit != OptionFit::NotChecked)
            {
                fit
            } else if setting.text.as_deref() == Some(option.text.as_str())
                && !matches!(setting.value, SettingValue::Unset)
            {
                fit_of(progress)
            } else {
                OptionFit::NotChecked
            };
        }
        sort_options(&mut setting.options);
    }
}

/// Puts the refused options after the rest, each part in the order the
/// options are listed by their type.
pub(crate) fn sort_options(options: &mut [SettingOption]) {
    options.sort_by_key(|option| (option.fit.is_refused(), option.position));
}

/// Records a found fit in `candidate`; `false` when it has no such setting
/// or option, or the option's fit is already that.
pub(crate) fn record(candidate: &mut Candidate, found: FoundFit) -> bool {
    let Some(setting) = candidate
        .settings
        .iter_mut()
        .find(|setting| setting.name.as_str() == found.setting.as_str())
    else {
        return false;
    };
    let Some(option) = setting
        .options
        .iter_mut()
        .find(|option| option.text == found.option)
    else {
        return false;
    };
    if option.fit == found.fit {
        return false;
    }
    option.fit = found.fit;
    sort_options(&mut setting.options);
    true
}

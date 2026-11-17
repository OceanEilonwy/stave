//! What the 3D view says when it has nothing to draw: still checking, a
//! folder that can't be read, a design whose errors stop it solving, a
//! library of kinds with no parts, or a project without parts yet. The
//! view never shows a blank area; it shows one of these.

use crate::session::{Session, Verdict};

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
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The empty state for what the session holds, or `None` when there is a
/// design with parts to draw.
pub fn of(session: &Session) -> Option<EmptyState> {
    if let Some((design, _)) = session.drawable() {
        if design.parts.is_empty() && design.hardware.is_empty() {
            return Some(EmptyState {
                reason: Reason::NoParts,
                title: "No parts yet".to_owned(),
                description: "The project places no parts. Add a part to its text and it appears here as soon as the file is saved.".to_owned(),
                offers_open: false,
            });
        }
        return None;
    }
    let state = match session.latest() {
        Some(latest) if latest.summary.errors > 0 => EmptyState {
            reason: Reason::Errors,
            title: "Nothing to show yet".to_owned(),
            description: format!(
                "The design has {}, so it can't be put together yet. The status pill lists them.",
                plural(latest.summary.errors, "error", "errors")
            ),
            offers_open: false,
        },
        Some(_) => EmptyState {
            reason: Reason::Library,
            title: "Nothing to show".to_owned(),
            description: "This folder is a library of kinds and components, with no parts to place. Open a project that uses it to see it in 3D.".to_owned(),
            offers_open: true,
        },
        None => match session.check_status().verdict() {
            Some(Verdict::CantOpen(message)) => EmptyState {
                reason: Reason::CantOpen,
                title: "Can't open the folder".to_owned(),
                description: message.clone(),
                offers_open: true,
            },
            Some(Verdict::Checked(_)) | None => EmptyState {
                reason: Reason::Checking,
                title: "Checking…".to_owned(),
                description: String::new(),
                offers_open: false,
            },
        },
    };
    Some(state)
}

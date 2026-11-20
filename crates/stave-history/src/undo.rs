//! Undo through jj's operation log.
//!
//! Every step Stave records is one operation tagged with who made it and
//! what it did. Undo finds the newest such operation (by anyone, or only by
//! the asker) that hasn't been undone and reverts it on top of the
//! repository as it is now; the undo is an operation too, tagged with the
//! operation it reverted, so undoing again reaches further back.

use std::collections::BTreeSet;

use futures::TryStreamExt as _;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_walk::walk_ancestors;
use jj_lib::operation::Operation;

use crate::actor::Actor;
use crate::error::HistoryError;
use crate::history::{History, block_on};
use crate::store::{
    self, ACTION_ATTRIBUTE, ACTOR_ATTRIBUTE, MOVED_MAIN_ATTRIBUTE, UNDOES_ATTRIBUTE, WINDOW,
};

/// Whose changes an undo may reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoScope {
    /// Only the asker's own last change (an agent's undo). Refused when
    /// someone else's later change to main builds on it.
    Own,
    /// The last change by anyone (the window's Undo).
    Any,
}

/// Who made an operation and what it did, as Stave tagged it.
fn tags(operation: &Operation) -> (Option<Actor>, String) {
    let attributes = &operation.metadata().attributes;
    let actor = attributes
        .get(ACTOR_ATTRIBUTE)
        .and_then(|line| Actor::from_line(line));
    let action = attributes
        .get(ACTION_ATTRIBUTE)
        .cloned()
        .unwrap_or_else(|| operation.metadata().description.clone());
    (actor, action)
}

fn moved_main(operation: &Operation) -> bool {
    operation
        .metadata()
        .attributes
        .contains_key(MOVED_MAIN_ATTRIBUTE)
}

impl History {
    /// Undoes the last change `scope` allows `actor` to undo, and says what
    /// was undone (as "save: First cut" or "apply suggestion: Widen the
    /// dresser"). Main moves back when that change moved it; the window's
    /// folder then catches up as for any change to main.
    ///
    /// # Errors
    ///
    /// [`HistoryError::NothingToUndo`] when there is no such change;
    /// [`HistoryError::UndoBlocked`] when `scope` is [`UndoScope::Own`] and a
    /// later change by someone else to main builds on it.
    pub fn undo(&mut self, actor: &Actor, scope: UndoScope) -> Result<String, HistoryError> {
        let _lock = self.lock_main()?;
        block_on(async {
            let repo = self.head().await?;
            let mut undone = BTreeSet::new();
            let mut later_by_others: Vec<Operation> = Vec::new();
            let mut ancestors = Box::pin(walk_ancestors(std::slice::from_ref(repo.operation())));
            let target = loop {
                let Some(operation) = ancestors.try_next().await.map_err(HistoryError::store)?
                else {
                    return Err(HistoryError::NothingToUndo);
                };
                let attributes = &operation.metadata().attributes;
                if let Some(reverted) = attributes.get(UNDOES_ATTRIBUTE) {
                    undone.insert(reverted.clone());
                    continue;
                }
                if undone.contains(&operation.id().hex()) {
                    continue;
                }
                let (Some(maker), _action) = tags(&operation) else {
                    continue;
                };
                match scope {
                    UndoScope::Any => break operation,
                    UndoScope::Own if maker.same_as(actor) => break operation,
                    UndoScope::Own => later_by_others.push(operation),
                }
            };
            let (_maker, action) = tags(&target);

            if moved_main(&target)
                && let Some(later) = later_by_others.iter().rev().find(|later| moved_main(later))
            {
                let (later_maker, later_action) = tags(later);
                let who = later_maker.map_or_else(|| "someone".to_owned(), |maker| maker.name);
                return Err(HistoryError::UndoBlocked {
                    undone: action,
                    later: format!("{who}'s {later_action}"),
                });
            }

            let parents = target.parents().await.map_err(HistoryError::store)?;
            let (parent_repo, _rebased) = self
                .loader
                .merge_operations(parents, None, Some("undo"), [])
                .await
                .map_err(HistoryError::store)?;
            let target_repo = self
                .loader
                .load_at(&target)
                .await
                .map_err(HistoryError::store)?;
            let window = store::bookmark(&repo, WINDOW);

            let description = format!("undo: {action}");
            let mut transaction = Self::transaction(&repo, Some(actor), &description);
            transaction.set_attribute(UNDOES_ATTRIBUTE.to_owned(), target.id().hex());
            if moved_main(&target) {
                transaction.set_attribute(MOVED_MAIN_ATTRIBUTE.to_owned(), "true".to_owned());
            }
            transaction
                .repo_mut()
                .merge(&target_repo, &parent_repo)
                .await
                .map_err(HistoryError::store)?;
            transaction
                .repo_mut()
                .rebase_descendants()
                .await
                .map_err(HistoryError::store)?;
            // The window's folder still holds what it held; keeping its base
            // where it was lets the next catch-up write the undone files.
            store::set_bookmark(transaction.repo_mut(), WINDOW, window.as_ref());
            transaction
                .commit(description)
                .await
                .map_err(HistoryError::store)?;
            Ok(action)
        })
    }
}

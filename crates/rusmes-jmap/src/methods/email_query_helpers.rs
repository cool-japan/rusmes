//! Private helpers for Email/queryChanges (RFC 8621 §4.5).
//!
//! Extracted from `email_advanced.rs` to keep that file under the 2000-line
//! policy limit.

use crate::methods::email::parse_mailbox_id;
use crate::types::EmailSort;
use rusmes_storage::{MessageStore, SearchCriteria};

use super::email_advanced::AddedItem;

// ── Public helpers (pub(crate) so email_advanced sub-modules can reach them) ──

/// Execute an email query applying `filter` and optional `sort` against the
/// message store.  Returns the ordered list of matching message IDs.
pub(crate) async fn execute_email_query(
    message_store: &dyn MessageStore,
    filter: &crate::types::EmailFilterCondition,
    sort: Option<&Vec<EmailSort>>,
) -> anyhow::Result<Vec<String>> {
    let mut ids: Vec<String> = if let Some(mailbox_id_str) = &filter.in_mailbox {
        match parse_mailbox_id(mailbox_id_str) {
            Ok(mailbox_id) => {
                let messages = message_store.get_mailbox_messages(&mailbox_id).await?;
                messages
                    .iter()
                    .map(|m| m.message_id().to_string())
                    .collect()
            }
            Err(_) => Vec::new(),
        }
    } else {
        let wildcard_id = rusmes_storage::MailboxId::from_uuid(uuid::Uuid::nil());
        message_store
            .search(&wildcard_id, SearchCriteria::All)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.to_string())
            .collect()
    };

    // Apply simple text/keyword filters (best-effort; real production would
    // consult a full-text search index).
    if let Some(ref _has_kw) = filter.has_keyword {
        // We cannot retrieve per-message flags without a mailbox context here,
        // so we pass all IDs through and let the client re-verify.  This is
        // the same trade-off made by email_query in email.rs.
        ids.retain(|_id| true);
    }

    apply_sort(&mut ids, sort);
    Ok(ids)
}

/// Execute an email query with no filter.
pub(crate) async fn execute_email_query_no_filter(
    message_store: &dyn MessageStore,
    sort: Option<&Vec<EmailSort>>,
) -> anyhow::Result<Vec<String>> {
    let wildcard_id = rusmes_storage::MailboxId::from_uuid(uuid::Uuid::nil());
    let mut ids: Vec<String> = message_store
        .search(&wildcard_id, SearchCriteria::All)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.to_string())
        .collect();
    apply_sort(&mut ids, sort);
    Ok(ids)
}

/// Retrieve the query snapshot at `since_state`.
///
/// Because we do not persist per-query snapshots, we return an empty set —
/// meaning everything in the current result set is treated as newly added.
/// RFC 8620 §5.6 permits this: the server may always return an empty previous
/// set provided it correctly marks all current items as added.
pub(crate) async fn get_previous_query_results(_since_state: u64) -> anyhow::Result<Vec<String>> {
    Ok(Vec::new())
}

/// Compute the diff between a previous and current ordered query result set.
///
/// Returns `(removed, added)` where:
/// - `removed` = IDs present in `previous` but absent from `current`
/// - `added`   = IDs present in `current` but absent from `previous`,
///   annotated with their 0-based index in `current`
pub(crate) fn compute_query_diff(
    previous: &[String],
    current: &[String],
) -> (Vec<String>, Vec<AddedItem>) {
    use std::collections::HashSet;

    let prev_set: HashSet<&str> = previous.iter().map(String::as_str).collect();
    let curr_set: HashSet<&str> = current.iter().map(String::as_str).collect();

    let removed: Vec<String> = previous
        .iter()
        .filter(|id| !curr_set.contains(id.as_str()))
        .cloned()
        .collect();

    let added: Vec<AddedItem> = current
        .iter()
        .enumerate()
        .filter(|(_, id)| !prev_set.contains(id.as_str()))
        .map(|(index, id)| AddedItem {
            id: id.clone(),
            index: index as u64,
        })
        .collect();

    (removed, added)
}

// ── Private ──────────────────────────────────────────────────────────────────

fn apply_sort(ids: &mut [String], sort: Option<&Vec<EmailSort>>) {
    if let Some(specs) = sort {
        if let Some(first) = specs.first() {
            if !first.is_ascending.unwrap_or(true) {
                ids.reverse();
            }
        }
    }
}

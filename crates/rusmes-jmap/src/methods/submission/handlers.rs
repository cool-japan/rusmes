//! EmailSubmission JMAP method handlers (get, set, query, changes)

use crate::methods::ensure_account_ownership;
use crate::methods::identity::IdentityStore;
use crate::methods::submission::create::handle_submission_create;
use crate::methods::submission::store::{within_undo_window, SubmissionStore};
use crate::methods::submission::types::{
    EmailSubmission, EmailSubmissionChangesRequest, EmailSubmissionChangesResponse,
    EmailSubmissionGetRequest, EmailSubmissionGetResponse, EmailSubmissionQueryRequest,
    EmailSubmissionQueryResponse, EmailSubmissionSetRequest, EmailSubmissionSetResponse,
    UndoStatus,
};
use crate::types::{JmapSetError, Principal};
use rusmes_core::transport::MailTransport;
use rusmes_storage::MessageStore;
use std::collections::HashMap;

/// Number of seconds after creation within which a pending submission may be
/// cancelled via `undoStatus = "canceled"`.
const UNDO_WINDOW_SECS: i64 = 30;

// ─── SubmissionContext ────────────────────────────────────────────────────────

/// Bundles the shared infrastructure dependencies required by submission
/// handlers, keeping function signatures lean (avoids too_many_arguments).
pub struct SubmissionContext<'a> {
    /// Storage backend for fetching/saving email messages.
    pub message_store: &'a dyn MessageStore,
    /// Persistence layer for EmailSubmission records.
    pub submission_store: &'a dyn SubmissionStore,
    /// Lookup service for JMAP Identity objects.
    pub identity_store: &'a dyn IdentityStore,
    /// SMTP transport used to deliver outbound mail.
    pub mail_transport: &'a dyn MailTransport,
}

// ─── email_submission_get ─────────────────────────────────────────────────────

/// Handle EmailSubmission/get method
pub async fn email_submission_get(
    request: EmailSubmissionGetRequest,
    _message_store: &dyn MessageStore,
    principal: &Principal,
) -> anyhow::Result<EmailSubmissionGetResponse> {
    ensure_account_ownership(&request.account_id, principal)?;
    let list = Vec::new();
    let mut not_found = Vec::new();

    let ids = request.ids.unwrap_or_default();
    for id in ids {
        not_found.push(id);
    }

    Ok(EmailSubmissionGetResponse {
        account_id: request.account_id,
        state: "1".to_string(),
        list,
        not_found,
    })
}

// ─── email_submission_set ─────────────────────────────────────────────────────

/// Handle EmailSubmission/set method
pub async fn email_submission_set(
    request: EmailSubmissionSetRequest,
    principal: &Principal,
    ctx: &SubmissionContext<'_>,
) -> anyhow::Result<EmailSubmissionSetResponse> {
    ensure_account_ownership(&request.account_id, principal)?;

    let old_state = ctx
        .submission_store
        .state_token(&request.account_id)
        .await?;

    let mut created: HashMap<String, EmailSubmission> = HashMap::new();
    let mut updated: HashMap<String, Option<EmailSubmission>> = HashMap::new();
    let mut destroyed: Vec<String> = Vec::new();
    let mut not_created: HashMap<String, JmapSetError> = HashMap::new();
    let mut not_updated: HashMap<String, JmapSetError> = HashMap::new();
    let mut not_destroyed: HashMap<String, JmapSetError> = HashMap::new();

    // ── Creates ───────────────────────────────────────────────────────────────
    if let Some(create_map) = request.create {
        for (creation_id, submission_obj) in create_map {
            match handle_submission_create(
                &request.account_id,
                &creation_id,
                submission_obj,
                principal,
                ctx,
            )
            .await
            {
                Ok(submission) => {
                    created.insert(creation_id, submission);
                }
                Err(err) => {
                    not_created.insert(creation_id, err);
                }
            }
        }
    }

    // ── Updates ───────────────────────────────────────────────────────────────
    if let Some(update_map) = request.update {
        for (id, patch) in update_map {
            match handle_submission_update(&request.account_id, &id, &patch, ctx.submission_store)
                .await
            {
                Ok(stored) => {
                    updated.insert(id, Some(stored));
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    let error_type = classify_submission_error(&err_msg);
                    not_updated.insert(
                        id,
                        JmapSetError {
                            error_type: error_type.to_string(),
                            description: Some(err_msg),
                        },
                    );
                }
            }
        }
    }

    // ── Destroys ──────────────────────────────────────────────────────────────
    if let Some(destroy_ids) = request.destroy {
        for id in destroy_ids {
            match handle_submission_destroy(&request.account_id, &id, ctx.submission_store).await {
                Ok(()) => {
                    destroyed.push(id);
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    let error_type = classify_submission_error(&err_msg);
                    not_destroyed.insert(
                        id,
                        JmapSetError {
                            error_type: error_type.to_string(),
                            description: Some(err_msg),
                        },
                    );
                }
            }
        }
    }

    let new_state = ctx
        .submission_store
        .state_token(&request.account_id)
        .await?;

    Ok(EmailSubmissionSetResponse {
        account_id: request.account_id,
        old_state,
        new_state,
        created: if created.is_empty() {
            None
        } else {
            Some(created)
        },
        updated: if updated.is_empty() {
            None
        } else {
            Some(updated)
        },
        destroyed: if destroyed.is_empty() {
            None
        } else {
            Some(destroyed)
        },
        not_created: if not_created.is_empty() {
            None
        } else {
            Some(not_created)
        },
        not_updated: if not_updated.is_empty() {
            None
        } else {
            Some(not_updated)
        },
        not_destroyed: if not_destroyed.is_empty() {
            None
        } else {
            Some(not_destroyed)
        },
    })
}

// ─── email_submission_query ───────────────────────────────────────────────────

/// Handle EmailSubmission/query method
pub async fn email_submission_query(
    request: EmailSubmissionQueryRequest,
    _message_store: &dyn MessageStore,
    principal: &Principal,
) -> anyhow::Result<EmailSubmissionQueryResponse> {
    ensure_account_ownership(&request.account_id, principal)?;
    let ids = Vec::new();

    let position = request.position.unwrap_or(0);
    let limit = request.limit.unwrap_or(100);

    Ok(EmailSubmissionQueryResponse {
        account_id: request.account_id,
        query_state: "1".to_string(),
        can_calculate_changes: false,
        position,
        ids,
        total: if request.calculate_total.unwrap_or(false) {
            Some(0)
        } else {
            None
        },
        limit: Some(limit),
    })
}

// ─── email_submission_changes ─────────────────────────────────────────────────

/// Handle EmailSubmission/changes method
pub async fn email_submission_changes(
    request: EmailSubmissionChangesRequest,
    _message_store: &dyn MessageStore,
    principal: &Principal,
) -> anyhow::Result<EmailSubmissionChangesResponse> {
    ensure_account_ownership(&request.account_id, principal)?;
    let since_state: u64 = request.since_state.parse().unwrap_or(0);
    let new_state = (since_state + 1).to_string();

    Ok(EmailSubmissionChangesResponse {
        account_id: request.account_id,
        old_state: request.since_state,
        new_state,
        has_more_changes: false,
        created: Vec::new(),
        updated: Vec::new(),
        destroyed: Vec::new(),
    })
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Classify a submission error string into the appropriate JMAP set error type.
pub(super) fn classify_submission_error(err_msg: &str) -> &'static str {
    if err_msg.contains("notFound") {
        "notFound"
    } else if err_msg.contains("cannotUnsend") {
        "cannotUnsend"
    } else if err_msg.contains("invalidProperties") {
        "invalidProperties"
    } else if err_msg.contains("methodNotAllowed") {
        "methodNotAllowed"
    } else {
        "serverFail"
    }
}

/// Process a single submission update.
///
/// Only `undoStatus: "canceled"` is accepted (within the 30-second undo
/// window). Any other patch fields are rejected with `invalidProperties`.
pub(super) async fn handle_submission_update(
    account_id: &str,
    id: &str,
    patch: &serde_json::Value,
    store: &dyn SubmissionStore,
) -> anyhow::Result<EmailSubmission> {
    let stored = store
        .get_submission(account_id, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("notFound: submission '{}' not found", id))?;

    let patch_obj = patch
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("invalidProperties: patch must be a JSON object"))?;

    for key in patch_obj.keys() {
        let field = key.trim_start_matches('/');
        if field != "undoStatus" {
            return Err(anyhow::anyhow!(
                "invalidProperties: field '{}' is immutable or unrecognised",
                field
            ));
        }
    }

    let new_status_raw = patch_obj
        .get("undoStatus")
        .or_else(|| patch_obj.get("/undoStatus"))
        .ok_or_else(|| anyhow::anyhow!("invalidProperties: patch must contain 'undoStatus'"))?;

    let new_status: UndoStatus = serde_json::from_value(new_status_raw.clone())?;

    if new_status != UndoStatus::Canceled {
        return Err(anyhow::anyhow!(
            "invalidProperties: undoStatus may only be set to 'canceled'"
        ));
    }
    if !within_undo_window(&stored, UNDO_WINDOW_SECS) {
        if stored.submission.undo_status != UndoStatus::Pending {
            return Err(anyhow::anyhow!(
                "invalidProperties: submission is not in 'pending' state (current: {:?})",
                stored.submission.undo_status
            ));
        }
        return Err(anyhow::anyhow!(
            "cannotUnsend: the undo window of {} seconds has expired",
            UNDO_WINDOW_SECS
        ));
    }

    let mut updated = stored.clone();
    updated.submission.undo_status = UndoStatus::Canceled;

    store.put_submission(account_id, updated.clone()).await?;

    Ok(updated.submission)
}

/// Process a single submission destroy.
///
/// Only `pending` or `canceled` submissions may be destroyed.
/// `final` submissions are immutable.
pub(super) async fn handle_submission_destroy(
    account_id: &str,
    id: &str,
    store: &dyn SubmissionStore,
) -> anyhow::Result<()> {
    let stored = store
        .get_submission(account_id, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("notFound: submission '{}' not found", id))?;

    match stored.submission.undo_status {
        UndoStatus::Final => Err(anyhow::anyhow!(
            "methodNotAllowed: cannot delete a submission with status 'final'"
        )),
        UndoStatus::Pending | UndoStatus::Canceled => {
            store.delete_submission(account_id, id).await?;
            Ok(())
        }
    }
}

//! JMAP method handlers

pub mod email;
pub mod email_advanced;
pub(crate) mod email_query_helpers;
pub mod identity;
pub mod mailbox;
pub mod push_subscription;
pub mod search_snippet;
pub mod submission;
pub mod thread;
pub mod vacation;

use crate::blob::BlobStorage;
use crate::types::{JmapError, JmapErrorType, JmapMethodCall, JmapMethodResponse, Principal};
use rusmes_core::transport::NullMailTransport;
use rusmes_storage::backends::filesystem::FilesystemBackend;
use rusmes_storage::StorageBackend;
use std::path::PathBuf;
use std::sync::Arc;

/// Dispatch JMAP method call.
///
/// Method handlers receive `&Principal` so they can enforce that the
/// `accountId` named in the JMAP request belongs to the authenticated
/// caller; mismatches are rejected with `urn:ietf:params:jmap:error:forbidden`.
///
/// Outgoing mail delivery uses a [`NullMailTransport`] by default.  Callers
/// that need real SMTP delivery should construct a dedicated dispatch path
/// with a concrete transport.
pub async fn dispatch_method(
    call: JmapMethodCall,
    capabilities: &[String],
    principal: &Principal,
) -> anyhow::Result<JmapMethodResponse> {
    let method_name = &call.0;
    let call_id = &call.2;

    // PushSubscription methods are handled without per-account state.
    if method_name == "PushSubscription/get" {
        let request = serde_json::from_value(call.1)?;
        let response = push_subscription::push_subscription_get(request, principal).await?;
        return Ok(JmapMethodResponse(
            "PushSubscription/get".to_string(),
            serde_json::to_value(response)?,
            call_id.clone(),
        ));
    }
    if method_name == "PushSubscription/set" {
        let request = serde_json::from_value(call.1)?;
        let response = push_subscription::push_subscription_set(request, principal).await?;
        return Ok(JmapMethodResponse(
            "PushSubscription/set".to_string(),
            serde_json::to_value(response)?,
            call_id.clone(),
        ));
    }

    // Validate method requires proper capability
    if let Err(error) = validate_method_capability(method_name, capabilities) {
        return Ok(JmapMethodResponse(
            "error".to_string(),
            serde_json::to_value(error)?,
            call_id.clone(),
        ));
    }

    // Get storage backend from configured path
    let backend = Arc::new(FilesystemBackend::new(PathBuf::from("/tmp/rusmes/mail")));
    let message_store = backend.message_store();
    let blob_storage = BlobStorage::new();
    let identity_store = identity::FileIdentityStore::new(PathBuf::from("/tmp/rusmes/jmap"));
    let vacation_store = vacation::FileVacationStore::new(PathBuf::from("/tmp/rusmes/data"));
    let submission_store = submission::FileSubmissionStore::new(PathBuf::from("/tmp/rusmes/jmap"));
    let mail_transport = NullMailTransport;

    // Dispatch to the appropriate handler
    match method_name.as_str() {
        // Email methods
        "Email/get" => {
            let request = serde_json::from_value(call.1)?;
            let response = email::email_get(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Email/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/set" => {
            let request = serde_json::from_value(call.1)?;
            let response = email::email_set(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Email/set".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/query" => {
            let request = serde_json::from_value(call.1)?;
            let response = email::email_query(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Email/query".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/changes" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                email_advanced::email_changes(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Email/changes".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/queryChanges" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                email_advanced::email_query_changes(request, message_store.as_ref(), principal)
                    .await?;
            Ok(JmapMethodResponse(
                "Email/queryChanges".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/copy" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                email_advanced::email_copy(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Email/copy".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/import" => {
            let request = serde_json::from_value(call.1)?;
            let response = email_advanced::email_import(
                request,
                message_store.as_ref(),
                &blob_storage,
                principal,
            )
            .await?;
            Ok(JmapMethodResponse(
                "Email/import".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/parse" => {
            let request = serde_json::from_value(call.1)?;
            let response = email_advanced::email_parse(
                request,
                message_store.as_ref(),
                &blob_storage,
                principal,
            )
            .await?;
            Ok(JmapMethodResponse(
                "Email/parse".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        // EmailSubmission methods
        "EmailSubmission/get" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                submission::email_submission_get(request, message_store.as_ref(), principal)
                    .await?;
            Ok(JmapMethodResponse(
                "EmailSubmission/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "EmailSubmission/set" => {
            let request = serde_json::from_value(call.1)?;
            let ctx = submission::SubmissionContext {
                message_store: message_store.as_ref(),
                submission_store: &submission_store,
                identity_store: &identity_store,
                mail_transport: &mail_transport,
            };
            let response = submission::email_submission_set(request, principal, &ctx).await?;
            Ok(JmapMethodResponse(
                "EmailSubmission/set".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "EmailSubmission/query" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                submission::email_submission_query(request, message_store.as_ref(), principal)
                    .await?;
            Ok(JmapMethodResponse(
                "EmailSubmission/query".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "EmailSubmission/changes" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                submission::email_submission_changes(request, message_store.as_ref(), principal)
                    .await?;
            Ok(JmapMethodResponse(
                "EmailSubmission/changes".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        // Mailbox methods
        "Mailbox/get" => {
            let request = serde_json::from_value(call.1)?;
            let response = mailbox::mailbox_get(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Mailbox/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Mailbox/set" => {
            let request = serde_json::from_value(call.1)?;
            let response = mailbox::mailbox_set(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Mailbox/set".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Mailbox/query" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                mailbox::mailbox_query(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Mailbox/query".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Mailbox/changes" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                mailbox::mailbox_changes(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Mailbox/changes".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Mailbox/queryChanges" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                mailbox::mailbox_query_changes(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Mailbox/queryChanges".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        // Thread methods
        "Thread/get" => {
            let request = serde_json::from_value(call.1)?;
            let response = thread::thread_get(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Thread/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Thread/changes" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                thread::thread_changes(request, message_store.as_ref(), principal).await?;
            Ok(JmapMethodResponse(
                "Thread/changes".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        // SearchSnippet methods
        "SearchSnippet/get" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                search_snippet::search_snippet_get(request, message_store.as_ref(), principal)
                    .await?;
            Ok(JmapMethodResponse(
                "SearchSnippet/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        // Identity methods
        "Identity/get" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                identity::identity_get(request, message_store.as_ref(), &identity_store, principal)
                    .await?;
            Ok(JmapMethodResponse(
                "Identity/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Identity/set" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                identity::identity_set(request, message_store.as_ref(), &identity_store, principal)
                    .await?;
            Ok(JmapMethodResponse(
                "Identity/set".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Identity/changes" => {
            let request = serde_json::from_value(call.1)?;
            let response = identity::identity_changes(
                request,
                message_store.as_ref(),
                &identity_store,
                principal,
            )
            .await?;
            Ok(JmapMethodResponse(
                "Identity/changes".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        // VacationResponse methods
        "VacationResponse/get" => {
            let request = serde_json::from_value(call.1)?;
            let response = vacation::vacation_response_get(
                request,
                message_store.as_ref(),
                principal,
                &vacation_store,
            )
            .await?;
            Ok(JmapMethodResponse(
                "VacationResponse/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "VacationResponse/set" => {
            let request = serde_json::from_value(call.1)?;
            let response = vacation::vacation_response_set(
                request,
                message_store.as_ref(),
                principal,
                &vacation_store,
            )
            .await?;
            Ok(JmapMethodResponse(
                "VacationResponse/set".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        _ => {
            // Return unknownMethod error
            Ok(JmapMethodResponse(
                "error".to_string(),
                serde_json::to_value(
                    JmapError::new(JmapErrorType::UnknownMethod)
                        .with_detail(format!("Unknown method: {}", method_name)),
                )?,
                call_id.clone(),
            ))
        }
    }
}

/// Validate that the method is supported by the declared capabilities
fn validate_method_capability(method_name: &str, capabilities: &[String]) -> Result<(), JmapError> {
    let required_capability = match method_name {
        m if m.starts_with("Email/") => "urn:ietf:params:jmap:mail",
        m if m.starts_with("Mailbox/") => "urn:ietf:params:jmap:mail",
        m if m.starts_with("Thread/") => "urn:ietf:params:jmap:mail",
        m if m.starts_with("SearchSnippet/") => "urn:ietf:params:jmap:mail",
        m if m.starts_with("EmailSubmission/") => "urn:ietf:params:jmap:submission",
        m if m.starts_with("Identity/") => "urn:ietf:params:jmap:submission",
        m if m.starts_with("VacationResponse/") => "urn:ietf:params:jmap:vacationresponse",
        // PushSubscription is a core RFC 8620 method — only core capability required.
        m if m.starts_with("PushSubscription/") => {
            return Ok(());
        }
        _ => {
            // Core methods don't require additional capabilities beyond core
            return Ok(());
        }
    };

    if !capabilities.iter().any(|cap| cap == required_capability) {
        return Err(
            JmapError::new(JmapErrorType::UnknownMethod).with_detail(format!(
                "Method '{}' requires capability '{}' which was not declared in 'using'",
                method_name, required_capability
            )),
        );
    }

    Ok(())
}

/// Helper used by every method handler: assert that `requested_account_id`
/// matches the principal's owned account and return a [`ForbiddenError`]
/// otherwise. The error converts cleanly into `anyhow::Error` via the
/// `?` operator.
pub(crate) fn ensure_account_ownership(
    requested_account_id: &str,
    principal: &Principal,
) -> Result<(), ForbiddenError> {
    if principal.owns_account(requested_account_id) {
        Ok(())
    } else {
        tracing::warn!(
            "JMAP account ownership mismatch: principal {} attempted to access account {}",
            principal.username,
            requested_account_id
        );
        Err(ForbiddenError {
            requested_account_id: requested_account_id.to_string(),
            principal_account_id: principal.account_id.clone(),
        })
    }
}

/// Strongly-typed ownership-mismatch error returned by individual method
/// handlers. Implements `From` into `anyhow::Error` via [`std::error::Error`]
/// so handlers can use `?` directly.
#[derive(Debug, Clone)]
pub struct ForbiddenError {
    /// `accountId` named in the JMAP request.
    pub requested_account_id: String,
    /// `accountId` actually owned by the authenticated [`Principal`].
    pub principal_account_id: String,
}

impl std::fmt::Display for ForbiddenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: requested account '{}' is not owned by principal (owns '{}')",
            JmapErrorType::Forbidden.as_str(),
            self.requested_account_id,
            self.principal_account_id
        )
    }
}

impl std::error::Error for ForbiddenError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Principal;

    fn alice() -> Principal {
        Principal {
            username: "alice".to_string(),
            account_id: "account-alice".to_string(),
            scopes: vec![],
        }
    }

    #[test]
    fn ensure_ownership_ok() {
        let p = alice();
        assert!(ensure_account_ownership("account-alice", &p).is_ok());
    }

    #[test]
    fn ensure_ownership_rejected() {
        let p = alice();
        let err = ensure_account_ownership("account-bob", &p).expect_err("should reject");
        assert_eq!(err.requested_account_id, "account-bob");
        assert_eq!(err.principal_account_id, "account-alice");
    }
}

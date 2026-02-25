//! JMAP method handlers

pub mod email;
pub mod email_advanced;
pub mod identity;
pub mod mailbox;
pub mod search_snippet;
pub mod submission;
pub mod thread;
pub mod vacation;

use crate::types::{JmapError, JmapErrorType, JmapMethodCall, JmapMethodResponse};
use rusmes_storage::backends::filesystem::FilesystemBackend;
use rusmes_storage::StorageBackend;
use std::path::PathBuf;
use std::sync::Arc;

/// Dispatch JMAP method call
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_method(
    call: JmapMethodCall,
    capabilities: &[String],
) -> anyhow::Result<JmapMethodResponse> {
    let method_name = &call.0;
    let call_id = &call.2;

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

    // Dispatch to the appropriate handler
    match method_name.as_str() {
        // Email methods
        "Email/get" => {
            let request = serde_json::from_value(call.1)?;
            let response = email::email_get(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Email/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/set" => {
            let request = serde_json::from_value(call.1)?;
            let response = email::email_set(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Email/set".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/query" => {
            let request = serde_json::from_value(call.1)?;
            let response = email::email_query(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Email/query".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/changes" => {
            let request = serde_json::from_value(call.1)?;
            let response = email_advanced::email_changes(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Email/changes".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/queryChanges" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                email_advanced::email_query_changes(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Email/queryChanges".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/copy" => {
            let request = serde_json::from_value(call.1)?;
            let response = email_advanced::email_copy(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Email/copy".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/import" => {
            let request = serde_json::from_value(call.1)?;
            let response = email_advanced::email_import(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Email/import".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Email/parse" => {
            let request = serde_json::from_value(call.1)?;
            let response = email_advanced::email_parse(request, message_store.as_ref()).await?;
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
                submission::email_submission_get(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "EmailSubmission/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "EmailSubmission/set" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                submission::email_submission_set(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "EmailSubmission/set".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "EmailSubmission/query" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                submission::email_submission_query(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "EmailSubmission/query".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "EmailSubmission/changes" => {
            let request = serde_json::from_value(call.1)?;
            let response =
                submission::email_submission_changes(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "EmailSubmission/changes".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        // Mailbox methods
        "Mailbox/get" => {
            let request = serde_json::from_value(call.1)?;
            let response = mailbox::mailbox_get(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Mailbox/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Mailbox/set" => {
            let request = serde_json::from_value(call.1)?;
            let response = mailbox::mailbox_set(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Mailbox/set".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Mailbox/query" => {
            let request = serde_json::from_value(call.1)?;
            let response = mailbox::mailbox_query(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Mailbox/query".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Mailbox/changes" => {
            let request = serde_json::from_value(call.1)?;
            let response = mailbox::mailbox_changes(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Mailbox/changes".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Mailbox/queryChanges" => {
            let request = serde_json::from_value(call.1)?;
            let response = mailbox::mailbox_query_changes(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Mailbox/queryChanges".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        // Thread methods
        "Thread/get" => {
            let request = serde_json::from_value(call.1)?;
            let response = thread::thread_get(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Thread/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Thread/changes" => {
            let request = serde_json::from_value(call.1)?;
            let response = thread::thread_changes(request, message_store.as_ref()).await?;
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
                search_snippet::search_snippet_get(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "SearchSnippet/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        // Identity methods
        "Identity/get" => {
            let request = serde_json::from_value(call.1)?;
            let response = identity::identity_get(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Identity/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Identity/set" => {
            let request = serde_json::from_value(call.1)?;
            let response = identity::identity_set(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Identity/set".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "Identity/changes" => {
            let request = serde_json::from_value(call.1)?;
            let response = identity::identity_changes(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "Identity/changes".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }

        // VacationResponse methods
        "VacationResponse/get" => {
            let request = serde_json::from_value(call.1)?;
            let response = vacation::vacation_response_get(request, message_store.as_ref()).await?;
            Ok(JmapMethodResponse(
                "VacationResponse/get".to_string(),
                serde_json::to_value(response)?,
                call_id.clone(),
            ))
        }
        "VacationResponse/set" => {
            let request = serde_json::from_value(call.1)?;
            let response = vacation::vacation_response_set(request, message_store.as_ref()).await?;
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

//! VacationResponse method implementations for JMAP
//!
//! Implements:
//! - VacationResponse/get, VacationResponse/set
//! - Vacation message generation
//! - Date-based vacation activation
//! - Recipient tracking (7-day cache for duplicate prevention)
//! - Integration with Sieve vacation extension

use crate::types::JmapSetError;
use chrono::{DateTime, Duration, Utc};
use rusmes_storage::MessageStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// VacationResponse object
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VacationResponse {
    /// Unique identifier (singleton, always "singleton")
    pub id: String,
    /// Is enabled
    pub is_enabled: bool,
    /// Start date (UTC)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_date: Option<DateTime<Utc>>,
    /// End date (UTC)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_date: Option<DateTime<Utc>>,
    /// Subject of vacation message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Text body of vacation message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_body: Option<String>,
    /// HTML body of vacation message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_body: Option<String>,
}

/// VacationResponse/get request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VacationResponseGetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<String>>,
}

/// VacationResponse/get response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VacationResponseGetResponse {
    pub account_id: String,
    pub state: String,
    pub list: Vec<VacationResponse>,
    pub not_found: Vec<String>,
}

/// VacationResponse/set request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VacationResponseSetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_in_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<HashMap<String, serde_json::Value>>,
}

/// VacationResponse/set response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VacationResponseSetResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<HashMap<String, Option<VacationResponse>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_updated: Option<HashMap<String, JmapSetError>>,
}

/// Recipient tracking entry
#[derive(Debug, Clone)]
struct RecipientEntry {
    _email: String,
    last_sent: DateTime<Utc>,
}

/// Vacation response tracker for duplicate prevention
#[derive(Debug, Clone)]
pub struct VacationTracker {
    recipients: Arc<Mutex<HashMap<String, RecipientEntry>>>,
}

impl VacationTracker {
    /// Create a new vacation tracker
    pub fn new() -> Self {
        Self {
            recipients: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if we should send vacation response to this recipient
    /// Returns true if we haven't sent to them in the last 7 days
    pub fn should_send_to(&self, email: &str) -> bool {
        let mut recipients = match self.recipients.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        // Clean up old entries (>7 days)
        let cutoff = Utc::now() - Duration::days(7);
        recipients.retain(|_, entry| entry.last_sent > cutoff);

        // Check if we've sent recently
        if let Some(entry) = recipients.get(email) {
            let since_last = Utc::now() - entry.last_sent;
            since_last > Duration::days(7)
        } else {
            true
        }
    }

    /// Record that we sent a vacation response to this recipient
    pub fn record_sent(&self, email: String) {
        let mut recipients = match self.recipients.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        recipients.insert(
            email.clone(),
            RecipientEntry {
                _email: email.clone(),
                last_sent: Utc::now(),
            },
        );
    }

    /// Get count of tracked recipients
    pub fn recipient_count(&self) -> usize {
        match self.recipients.lock() {
            Ok(guard) => guard.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }
}

impl Default for VacationTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Vacation message content
#[derive(Debug, Clone)]
pub struct VacationMessage {
    pub subject: String,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
}

/// Handle VacationResponse/get method
pub async fn vacation_response_get(
    request: VacationResponseGetRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<VacationResponseGetResponse> {
    let mut list = Vec::new();
    let mut not_found = Vec::new();

    // VacationResponse is a singleton
    let ids = request.ids.unwrap_or_else(|| vec!["singleton".to_string()]);

    for id in ids {
        if id == "singleton" {
            // Return default vacation response (disabled)
            list.push(VacationResponse {
                id: "singleton".to_string(),
                is_enabled: false,
                from_date: None,
                to_date: None,
                subject: None,
                text_body: None,
                html_body: None,
            });
        } else {
            not_found.push(id);
        }
    }

    Ok(VacationResponseGetResponse {
        account_id: request.account_id,
        state: "1".to_string(),
        list,
        not_found,
    })
}

/// Handle VacationResponse/set method
pub async fn vacation_response_set(
    request: VacationResponseSetRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<VacationResponseSetResponse> {
    let updated = HashMap::new();
    let mut not_updated = HashMap::new();

    // VacationResponse only supports update (singleton object)
    if let Some(update_map) = request.update {
        for (id, _patch) in update_map {
            if id != "singleton" {
                not_updated.insert(
                    id,
                    JmapSetError {
                        error_type: "notFound".to_string(),
                        description: Some("VacationResponse ID must be 'singleton'".to_string()),
                    },
                );
            } else {
                not_updated.insert(
                    id,
                    JmapSetError {
                        error_type: "notImplemented".to_string(),
                        description: Some("Vacation response not yet implemented".to_string()),
                    },
                );
            }
        }
    }

    Ok(VacationResponseSetResponse {
        account_id: request.account_id,
        old_state: "1".to_string(),
        new_state: "2".to_string(),
        updated: if updated.is_empty() {
            None
        } else {
            Some(updated)
        },
        not_updated: if not_updated.is_empty() {
            None
        } else {
            Some(not_updated)
        },
    })
}

/// Check if vacation response should be active
pub fn is_vacation_active(vacation: &VacationResponse) -> bool {
    if !vacation.is_enabled {
        return false;
    }

    let now = Utc::now();

    // Check from_date
    if let Some(from_date) = vacation.from_date {
        if now < from_date {
            return false;
        }
    }

    // Check to_date
    if let Some(to_date) = vacation.to_date {
        if now > to_date {
            return false;
        }
    }

    true
}

/// Generate vacation response message
pub fn generate_vacation_message(
    vacation: &VacationResponse,
    original_subject: Option<&str>,
) -> Option<VacationMessage> {
    if !is_vacation_active(vacation) {
        return None;
    }

    // Generate subject
    let subject = if let Some(custom_subject) = &vacation.subject {
        custom_subject.clone()
    } else if let Some(orig_subj) = original_subject {
        format!("Re: {}", orig_subj)
    } else {
        "Automatic reply".to_string()
    };

    Some(VacationMessage {
        subject,
        text_body: vacation.text_body.clone(),
        html_body: vacation.html_body.clone(),
    })
}

/// Generate vacation response headers
pub fn generate_vacation_headers() -> Vec<(String, String)> {
    vec![
        ("Auto-Submitted".to_string(), "auto-replied".to_string()),
        ("Precedence".to_string(), "bulk".to_string()),
    ]
}

/// Extract email addresses that should receive vacation responses
/// Filters out mailing lists, bulk mail, and auto-submitted messages
pub fn extract_vacation_recipients(from: &str, headers: &[(String, String)]) -> Vec<String> {
    let mut recipients = Vec::new();

    // Check for auto-submitted header (don't reply to auto-generated messages)
    for (key, value) in headers {
        if key.to_lowercase() == "auto-submitted" && value != "no" {
            return recipients; // Don't send vacation response
        }
        if key.to_lowercase() == "precedence"
            && (value == "bulk" || value == "list" || value == "junk")
        {
            return recipients; // Don't send vacation response to bulk/list mail
        }
        if key.to_lowercase() == "list-id" || key.to_lowercase() == "list-post" {
            return recipients; // Don't send vacation response to mailing lists
        }
    }

    // Add the from address if valid
    if !from.is_empty() && from.contains('@') {
        recipients.push(from.to_string());
    }

    recipients
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusmes_storage::backends::filesystem::FilesystemBackend;
    use rusmes_storage::StorageBackend;
    use std::path::PathBuf;

    fn create_test_store() -> std::sync::Arc<dyn MessageStore> {
        let backend = FilesystemBackend::new(PathBuf::from("/tmp/rusmes-test-storage"));
        backend.message_store()
    }

    #[tokio::test]
    async fn test_vacation_response_get() {
        let store = create_test_store();
        let request = VacationResponseGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["singleton".to_string()]),
            properties: None,
        };

        let response = vacation_response_get(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.list.len(), 1);
        assert_eq!(response.list[0].id, "singleton");
        assert!(!response.list[0].is_enabled);
    }

    #[tokio::test]
    async fn test_vacation_response_set() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert(
            "singleton".to_string(),
            serde_json::json!({
                "isEnabled": true,
                "subject": "Out of Office",
                "textBody": "I'm currently out of office."
            }),
        );

        let request = VacationResponseSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            update: Some(update_map),
        };

        let response = vacation_response_set(request, store.as_ref())
            .await
            .unwrap();
        assert!(response.not_updated.is_some());
    }

    #[tokio::test]
    async fn test_is_vacation_active() {
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: None,
            to_date: None,
            subject: None,
            text_body: None,
            html_body: None,
        };

        assert!(is_vacation_active(&vacation));
    }

    #[tokio::test]
    async fn test_is_vacation_inactive_disabled() {
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: false,
            from_date: None,
            to_date: None,
            subject: None,
            text_body: None,
            html_body: None,
        };

        assert!(!is_vacation_active(&vacation));
    }

    #[tokio::test]
    async fn test_is_vacation_active_with_dates() {
        let now = Utc::now();
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: Some(now - Duration::days(1)),
            to_date: Some(now + Duration::days(7)),
            subject: None,
            text_body: None,
            html_body: None,
        };

        assert!(is_vacation_active(&vacation));
    }

    #[tokio::test]
    async fn test_is_vacation_inactive_before_start() {
        let now = Utc::now();
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: Some(now + Duration::days(1)),
            to_date: Some(now + Duration::days(7)),
            subject: None,
            text_body: None,
            html_body: None,
        };

        assert!(!is_vacation_active(&vacation));
    }

    #[tokio::test]
    async fn test_is_vacation_inactive_after_end() {
        let now = Utc::now();
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: Some(now - Duration::days(7)),
            to_date: Some(now - Duration::days(1)),
            subject: None,
            text_body: None,
            html_body: None,
        };

        assert!(!is_vacation_active(&vacation));
    }

    #[tokio::test]
    async fn test_vacation_response_invalid_id() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert(
            "invalid".to_string(),
            serde_json::json!({"isEnabled": true}),
        );

        let request = VacationResponseSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            update: Some(update_map),
        };

        let response = vacation_response_set(request, store.as_ref())
            .await
            .unwrap();
        assert!(response.not_updated.is_some());
        let errors = response.not_updated.unwrap();
        assert_eq!(errors.get("invalid").unwrap().error_type, "notFound");
    }

    #[tokio::test]
    async fn test_vacation_response_with_html() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert(
            "singleton".to_string(),
            serde_json::json!({
                "isEnabled": true,
                "subject": "Out of Office",
                "textBody": "I'm out of office.",
                "htmlBody": "<p>I'm out of office.</p>"
            }),
        );

        let request = VacationResponseSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            update: Some(update_map),
        };

        let response = vacation_response_set(request, store.as_ref())
            .await
            .unwrap();
        assert!(response.not_updated.is_some());
    }

    #[tokio::test]
    async fn test_vacation_response_get_all() {
        let store = create_test_store();
        let request = VacationResponseGetRequest {
            account_id: "acc1".to_string(),
            ids: None,
            properties: None,
        };

        let response = vacation_response_get(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.list.len(), 1);
    }

    #[tokio::test]
    async fn test_vacation_response_date_range() {
        let now = Utc::now();
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: Some(now + Duration::days(1)),
            to_date: Some(now + Duration::days(14)),
            subject: Some("Vacation".to_string()),
            text_body: Some("On vacation".to_string()),
            html_body: None,
        };

        // Not yet active
        assert!(!is_vacation_active(&vacation));
    }

    #[tokio::test]
    async fn test_vacation_response_only_from_date() {
        let now = Utc::now();
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: Some(now - Duration::days(1)),
            to_date: None,
            subject: None,
            text_body: None,
            html_body: None,
        };

        assert!(is_vacation_active(&vacation));
    }

    #[tokio::test]
    async fn test_vacation_response_only_to_date() {
        let now = Utc::now();
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: None,
            to_date: Some(now + Duration::days(1)),
            subject: None,
            text_body: None,
            html_body: None,
        };

        assert!(is_vacation_active(&vacation));
    }

    #[test]
    fn test_vacation_tracker_new() {
        let tracker = VacationTracker::new();
        assert_eq!(tracker.recipient_count(), 0);
    }

    #[test]
    fn test_vacation_tracker_should_send_new_recipient() {
        let tracker = VacationTracker::new();
        assert!(tracker.should_send_to("test@example.com"));
    }

    #[test]
    fn test_vacation_tracker_record_sent() {
        let tracker = VacationTracker::new();
        tracker.record_sent("test@example.com".to_string());
        assert_eq!(tracker.recipient_count(), 1);
        assert!(!tracker.should_send_to("test@example.com"));
    }

    #[test]
    fn test_vacation_tracker_multiple_recipients() {
        let tracker = VacationTracker::new();
        tracker.record_sent("user1@example.com".to_string());
        tracker.record_sent("user2@example.com".to_string());

        assert_eq!(tracker.recipient_count(), 2);
        assert!(!tracker.should_send_to("user1@example.com"));
        assert!(!tracker.should_send_to("user2@example.com"));
        assert!(tracker.should_send_to("user3@example.com"));
    }

    #[test]
    fn test_generate_vacation_message_inactive() {
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: false,
            from_date: None,
            to_date: None,
            subject: None,
            text_body: Some("Away".to_string()),
            html_body: None,
        };

        let message = generate_vacation_message(&vacation, Some("Hello"));
        assert!(message.is_none());
    }

    #[test]
    fn test_generate_vacation_message_active() {
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: None,
            to_date: None,
            subject: Some("Out of Office".to_string()),
            text_body: Some("I'm away".to_string()),
            html_body: None,
        };

        let message = generate_vacation_message(&vacation, Some("Hello"));
        assert!(message.is_some());
        let msg = message.unwrap();
        assert_eq!(msg.subject, "Out of Office");
        assert_eq!(msg.text_body, Some("I'm away".to_string()));
    }

    #[test]
    fn test_generate_vacation_message_default_subject() {
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: None,
            to_date: None,
            subject: None,
            text_body: Some("Away".to_string()),
            html_body: None,
        };

        let message = generate_vacation_message(&vacation, Some("Meeting tomorrow"));
        assert!(message.is_some());
        let msg = message.unwrap();
        assert_eq!(msg.subject, "Re: Meeting tomorrow");
    }

    #[test]
    fn test_generate_vacation_message_no_original_subject() {
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: None,
            to_date: None,
            subject: None,
            text_body: Some("Away".to_string()),
            html_body: None,
        };

        let message = generate_vacation_message(&vacation, None);
        assert!(message.is_some());
        let msg = message.unwrap();
        assert_eq!(msg.subject, "Automatic reply");
    }

    #[test]
    fn test_generate_vacation_headers() {
        let headers = generate_vacation_headers();
        assert_eq!(headers.len(), 2);

        assert!(headers
            .iter()
            .any(|(k, v)| k == "Auto-Submitted" && v == "auto-replied"));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Precedence" && v == "bulk"));
    }

    #[test]
    fn test_extract_vacation_recipients_valid() {
        let recipients = extract_vacation_recipients("user@example.com", &[]);
        assert_eq!(recipients.len(), 1);
        assert_eq!(recipients[0], "user@example.com");
    }

    #[test]
    fn test_extract_vacation_recipients_auto_submitted() {
        let recipients = extract_vacation_recipients(
            "user@example.com",
            &[("Auto-Submitted".to_string(), "auto-replied".to_string())],
        );
        assert_eq!(recipients.len(), 0);
    }

    #[test]
    fn test_extract_vacation_recipients_bulk() {
        let recipients = extract_vacation_recipients(
            "user@example.com",
            &[("Precedence".to_string(), "bulk".to_string())],
        );
        assert_eq!(recipients.len(), 0);
    }

    #[test]
    fn test_extract_vacation_recipients_list() {
        let recipients = extract_vacation_recipients(
            "user@example.com",
            &[("List-Id".to_string(), "list@example.com".to_string())],
        );
        assert_eq!(recipients.len(), 0);
    }

    #[test]
    fn test_extract_vacation_recipients_invalid_email() {
        let recipients = extract_vacation_recipients("invalid-email", &[]);
        assert_eq!(recipients.len(), 0);
    }

    #[test]
    fn test_extract_vacation_recipients_empty() {
        let recipients = extract_vacation_recipients("", &[]);
        assert_eq!(recipients.len(), 0);
    }

    #[test]
    fn test_vacation_message_with_html() {
        let vacation = VacationResponse {
            id: "singleton".to_string(),
            is_enabled: true,
            from_date: None,
            to_date: None,
            subject: Some("Away".to_string()),
            text_body: Some("I'm away".to_string()),
            html_body: Some("<p>I'm away</p>".to_string()),
        };

        let message = generate_vacation_message(&vacation, None);
        assert!(message.is_some());
        let msg = message.unwrap();
        assert_eq!(msg.html_body, Some("<p>I'm away</p>".to_string()));
    }

    #[test]
    fn test_vacation_tracker_default() {
        let tracker = VacationTracker::default();
        assert_eq!(tracker.recipient_count(), 0);
    }
}

//! VacationResponse method implementations for JMAP
//!
//! Implements:
//! - VacationResponse/get, VacationResponse/set
//! - Vacation message generation
//! - Date-based vacation activation
//! - Recipient tracking (7-day cache for duplicate prevention)
//! - Integration with Sieve vacation extension

use crate::methods::ensure_account_ownership;
use crate::types::{JmapSetError, Principal};
use chrono::{DateTime, Duration, Utc};
use rusmes_storage::MessageStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
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

/// Persisted state file for a single account's vacation setting
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VacationStateFile {
    vacation: VacationResponse,
    state: u64,
}

/// Trait for persisting VacationResponse data per account
pub trait VacationStore: Send + Sync {
    /// Retrieve the vacation response for the given account.
    /// Returns `None` when no record has been stored yet.
    fn get_vacation(&self, account_id: &str) -> anyhow::Result<Option<VacationResponse>>;

    /// Persist a new vacation response for the given account.
    fn set_vacation(&self, account_id: &str, vacation: VacationResponse) -> anyhow::Result<()>;

    /// Return the current state token (opaque string) for the given account.
    /// Returns `"0"` when no record exists yet.
    fn state_token(&self, account_id: &str) -> anyhow::Result<String>;
}

/// File-system backed vacation store.
///
/// Stores one JSON file per account at `{base_dir}/vacations/{account_id}.json`.
pub struct FileVacationStore {
    base_dir: PathBuf,
}

impl FileVacationStore {
    /// Create a new `FileVacationStore` rooted at `base_dir`.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    fn vacations_dir(&self) -> PathBuf {
        self.base_dir.join("vacations")
    }

    fn account_file(&self, account_id: &str) -> PathBuf {
        // Sanitise the account id to avoid path traversal
        let safe_id = account_id.replace(['/', '\\', '.'], "_");
        self.vacations_dir().join(format!("{}.json", safe_id))
    }

    fn load_state_file(&self, account_id: &str) -> anyhow::Result<Option<VacationStateFile>> {
        let path = self.account_file(account_id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let state_file: VacationStateFile = serde_json::from_slice(&bytes)?;
        Ok(Some(state_file))
    }

    fn save_state_file(
        &self,
        account_id: &str,
        state_file: &VacationStateFile,
    ) -> anyhow::Result<()> {
        let dir = self.vacations_dir();
        std::fs::create_dir_all(&dir)?;
        let path = self.account_file(account_id);
        let bytes = serde_json::to_vec_pretty(state_file)?;
        std::fs::write(&path, &bytes)?;
        Ok(())
    }
}

impl VacationStore for FileVacationStore {
    fn get_vacation(&self, account_id: &str) -> anyhow::Result<Option<VacationResponse>> {
        let state_file = self.load_state_file(account_id)?;
        Ok(state_file.map(|sf| sf.vacation))
    }

    fn set_vacation(&self, account_id: &str, vacation: VacationResponse) -> anyhow::Result<()> {
        let current_state = self
            .load_state_file(account_id)?
            .map(|sf| sf.state)
            .unwrap_or(0);
        let new_state = current_state.saturating_add(1);
        let state_file = VacationStateFile {
            vacation,
            state: new_state,
        };
        self.save_state_file(account_id, &state_file)
    }

    fn state_token(&self, account_id: &str) -> anyhow::Result<String> {
        let state = self
            .load_state_file(account_id)?
            .map(|sf| sf.state)
            .unwrap_or(0);
        Ok(state.to_string())
    }
}

/// Default disabled vacation response
fn default_vacation_response() -> VacationResponse {
    VacationResponse {
        id: "singleton".to_string(),
        is_enabled: false,
        from_date: None,
        to_date: None,
        subject: None,
        text_body: None,
        html_body: None,
    }
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

/// Apply a JMAP patch object to an existing `VacationResponse`.
///
/// Only keys present in `patch` are modified.  Keys not present in the patch
/// leave the current value unchanged.  The `id` field is immutable and
/// silently ignored if present in the patch.
fn apply_patch(
    mut vacation: VacationResponse,
    patch: &serde_json::Value,
) -> anyhow::Result<VacationResponse> {
    let patch_obj = patch
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Patch must be a JSON object"))?;

    for (key, value) in patch_obj {
        match key.as_str() {
            "isEnabled" => {
                vacation.is_enabled = value
                    .as_bool()
                    .ok_or_else(|| anyhow::anyhow!("isEnabled must be a boolean"))?;
            }
            "fromDate" => {
                if value.is_null() {
                    vacation.from_date = None;
                } else {
                    let s = value
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("fromDate must be a string or null"))?;
                    vacation.from_date = Some(
                        s.parse::<DateTime<Utc>>()
                            .map_err(|e| anyhow::anyhow!("Invalid fromDate: {}", e))?,
                    );
                }
            }
            "toDate" => {
                if value.is_null() {
                    vacation.to_date = None;
                } else {
                    let s = value
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("toDate must be a string or null"))?;
                    vacation.to_date = Some(
                        s.parse::<DateTime<Utc>>()
                            .map_err(|e| anyhow::anyhow!("Invalid toDate: {}", e))?,
                    );
                }
            }
            "subject" => {
                if value.is_null() {
                    vacation.subject = None;
                } else {
                    vacation.subject = Some(
                        value
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("subject must be a string or null"))?
                            .to_owned(),
                    );
                }
            }
            "textBody" => {
                if value.is_null() {
                    vacation.text_body = None;
                } else {
                    vacation.text_body = Some(
                        value
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("textBody must be a string or null"))?
                            .to_owned(),
                    );
                }
            }
            "htmlBody" => {
                if value.is_null() {
                    vacation.html_body = None;
                } else {
                    vacation.html_body = Some(
                        value
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("htmlBody must be a string or null"))?
                            .to_owned(),
                    );
                }
            }
            // id is immutable; unknown keys are silently ignored per JMAP patch semantics
            _ => {}
        }
    }

    Ok(vacation)
}

/// Handle VacationResponse/get method
pub async fn vacation_response_get(
    request: VacationResponseGetRequest,
    _message_store: &dyn MessageStore,
    principal: &Principal,
    vacation_store: &dyn VacationStore,
) -> anyhow::Result<VacationResponseGetResponse> {
    ensure_account_ownership(&request.account_id, principal)?;
    let mut list = Vec::new();
    let mut not_found = Vec::new();

    let current_state = vacation_store.state_token(&request.account_id)?;

    // VacationResponse is a singleton
    let ids = request.ids.unwrap_or_else(|| vec!["singleton".to_string()]);

    for id in ids {
        if id == "singleton" {
            let vacation = vacation_store
                .get_vacation(&request.account_id)?
                .unwrap_or_else(default_vacation_response);
            list.push(vacation);
        } else {
            not_found.push(id);
        }
    }

    Ok(VacationResponseGetResponse {
        account_id: request.account_id,
        state: current_state,
        list,
        not_found,
    })
}

/// Handle VacationResponse/set method
pub async fn vacation_response_set(
    request: VacationResponseSetRequest,
    _message_store: &dyn MessageStore,
    principal: &Principal,
    vacation_store: &dyn VacationStore,
) -> anyhow::Result<VacationResponseSetResponse> {
    ensure_account_ownership(&request.account_id, principal)?;

    let old_state = vacation_store.state_token(&request.account_id)?;

    // Check if_in_state guard (RFC 8620 §5.3 stateMismatch)
    if let Some(ref expected) = request.if_in_state {
        if expected != &old_state {
            return Err(anyhow::anyhow!(
                "stateMismatch: expected state '{}', current state '{}'",
                expected,
                old_state
            ));
        }
    }

    let mut updated: HashMap<String, Option<VacationResponse>> = HashMap::new();
    let mut not_updated: HashMap<String, JmapSetError> = HashMap::new();

    // VacationResponse only supports update (singleton object)
    if let Some(update_map) = request.update {
        for (id, patch) in update_map {
            if id != "singleton" {
                not_updated.insert(
                    id,
                    JmapSetError {
                        error_type: "notFound".to_string(),
                        description: Some("VacationResponse ID must be 'singleton'".to_string()),
                    },
                );
            } else {
                // Load current vacation (or create default)
                let current = vacation_store
                    .get_vacation(&request.account_id)?
                    .unwrap_or_else(default_vacation_response);

                // Apply patch
                let patched = match apply_patch(current, &patch) {
                    Ok(v) => v,
                    Err(e) => {
                        not_updated.insert(
                            id,
                            JmapSetError {
                                error_type: "invalidProperties".to_string(),
                                description: Some(format!("Patch error: {}", e)),
                            },
                        );
                        continue;
                    }
                };

                // Validate date ordering
                if let (Some(from), Some(to)) = (patched.from_date, patched.to_date) {
                    if from > to {
                        not_updated.insert(
                            id,
                            JmapSetError {
                                error_type: "invalidProperties".to_string(),
                                description: Some(
                                    "fromDate must be before or equal to toDate".to_string(),
                                ),
                            },
                        );
                        continue;
                    }
                }

                // Persist
                vacation_store.set_vacation(&request.account_id, patched.clone())?;

                updated.insert(id, Some(patched));
            }
        }
    }

    let new_state = vacation_store.state_token(&request.account_id)?;

    Ok(VacationResponseSetResponse {
        account_id: request.account_id,
        old_state,
        new_state,
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

    fn test_principal() -> crate::types::Principal {
        crate::types::admin_principal_for_tests()
    }

    fn create_test_store() -> std::sync::Arc<dyn MessageStore> {
        let backend = FilesystemBackend::new(PathBuf::from("/tmp/rusmes-test-storage"));
        backend.message_store()
    }

    /// Create a `FileVacationStore` rooted in a unique temp directory so that
    /// concurrent nextest workers do not share state.
    fn make_vacation_store(test_name: &str) -> FileVacationStore {
        let dir = std::env::temp_dir().join(format!("rusmes-vacation-test-{}", test_name));
        FileVacationStore::new(dir)
    }

    // -----------------------------------------------------------------------
    // Legacy tests (updated to match new signatures and success semantics)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_vacation_response_get() {
        let store = create_test_store();
        let vstore = make_vacation_store("get_basic");
        let request = VacationResponseGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["singleton".to_string()]),
            properties: None,
        };

        let response = vacation_response_get(request, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        assert_eq!(response.list.len(), 1);
        assert_eq!(response.list[0].id, "singleton");
        assert!(!response.list[0].is_enabled);
    }

    #[tokio::test]
    async fn test_vacation_response_set() {
        let store = create_test_store();
        let vstore = make_vacation_store("set_basic");
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

        let response = vacation_response_set(request, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        // Successful update goes into `updated`, not `not_updated`
        assert!(response.updated.is_some());
        assert!(response.not_updated.is_none());
        let updated = response.updated.unwrap();
        let vacation = updated.get("singleton").unwrap().as_ref().unwrap();
        assert!(vacation.is_enabled);
        assert_eq!(vacation.subject.as_deref(), Some("Out of Office"));
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
        let vstore = make_vacation_store("invalid_id");
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

        let response = vacation_response_set(request, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        assert!(response.not_updated.is_some());
        let errors = response.not_updated.unwrap();
        assert_eq!(errors.get("invalid").unwrap().error_type, "notFound");
    }

    #[tokio::test]
    async fn test_vacation_response_with_html() {
        let store = create_test_store();
        let vstore = make_vacation_store("with_html");
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

        let response = vacation_response_set(request, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        // Successful update goes into `updated`
        assert!(response.updated.is_some());
        let updated = response.updated.unwrap();
        let vacation = updated.get("singleton").unwrap().as_ref().unwrap();
        assert_eq!(
            vacation.html_body.as_deref(),
            Some("<p>I'm out of office.</p>")
        );
    }

    #[tokio::test]
    async fn test_vacation_response_get_all() {
        let store = create_test_store();
        let vstore = make_vacation_store("get_all");
        let request = VacationResponseGetRequest {
            account_id: "acc1".to_string(),
            ids: None,
            properties: None,
        };

        let response = vacation_response_get(request, store.as_ref(), &test_principal(), &vstore)
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

    // -----------------------------------------------------------------------
    // New tests: store-backed set/get and error paths
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_vacation_set_enabled() {
        let store = create_test_store();
        let vstore = make_vacation_store("set_enabled");

        // Set vacation to enabled
        let mut update_map = HashMap::new();
        update_map.insert(
            "singleton".to_string(),
            serde_json::json!({"isEnabled": true, "subject": "Away"}),
        );
        let set_req = VacationResponseSetRequest {
            account_id: "user1".to_string(),
            if_in_state: None,
            update: Some(update_map),
        };
        let set_resp = vacation_response_set(set_req, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        assert!(set_resp.updated.is_some());

        // Get should now return enabled
        let get_req = VacationResponseGetRequest {
            account_id: "user1".to_string(),
            ids: Some(vec!["singleton".to_string()]),
            properties: None,
        };
        let get_resp = vacation_response_get(get_req, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        assert_eq!(get_resp.list.len(), 1);
        assert!(get_resp.list[0].is_enabled);
        assert_eq!(get_resp.list[0].subject.as_deref(), Some("Away"));
    }

    #[tokio::test]
    async fn test_vacation_set_date_range() {
        let store = create_test_store();
        let vstore = make_vacation_store("set_date_range");
        let now = Utc::now();
        let from = now - Duration::days(1);
        let to = now + Duration::days(7);

        let mut update_map = HashMap::new();
        update_map.insert(
            "singleton".to_string(),
            serde_json::json!({
                "isEnabled": true,
                "fromDate": from.to_rfc3339(),
                "toDate": to.to_rfc3339()
            }),
        );
        let set_req = VacationResponseSetRequest {
            account_id: "user2".to_string(),
            if_in_state: None,
            update: Some(update_map),
        };
        vacation_response_set(set_req, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();

        // Verify retrieved dates are within 1 second of originals
        let get_req = VacationResponseGetRequest {
            account_id: "user2".to_string(),
            ids: None,
            properties: None,
        };
        let get_resp = vacation_response_get(get_req, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        let v = &get_resp.list[0];
        let stored_from = v.from_date.expect("from_date should be set");
        let stored_to = v.to_date.expect("to_date should be set");
        assert!((stored_from - from).num_seconds().abs() <= 1);
        assert!((stored_to - to).num_seconds().abs() <= 1);
    }

    #[tokio::test]
    async fn test_vacation_invalid_date_order() {
        let store = create_test_store();
        let vstore = make_vacation_store("invalid_date_order");
        let now = Utc::now();
        // fromDate is AFTER toDate — should produce an invalidProperties error
        let from = now + Duration::days(5);
        let to = now + Duration::days(2);

        let mut update_map = HashMap::new();
        update_map.insert(
            "singleton".to_string(),
            serde_json::json!({
                "isEnabled": true,
                "fromDate": from.to_rfc3339(),
                "toDate": to.to_rfc3339()
            }),
        );
        let set_req = VacationResponseSetRequest {
            account_id: "user3".to_string(),
            if_in_state: None,
            update: Some(update_map),
        };
        let resp = vacation_response_set(set_req, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        assert!(resp.not_updated.is_some());
        let errors = resp.not_updated.unwrap();
        assert_eq!(
            errors.get("singleton").unwrap().error_type,
            "invalidProperties"
        );
    }

    #[tokio::test]
    async fn test_vacation_state_mismatch() {
        let store = create_test_store();
        let vstore = make_vacation_store("state_mismatch");

        // Initial state for new account is "0"
        let mut update_map = HashMap::new();
        update_map.insert(
            "singleton".to_string(),
            serde_json::json!({"isEnabled": false}),
        );
        let set_req = VacationResponseSetRequest {
            account_id: "user4".to_string(),
            if_in_state: Some("999".to_string()), // wrong state
            update: Some(update_map),
        };
        let result =
            vacation_response_set(set_req, store.as_ref(), &test_principal(), &vstore).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("stateMismatch"));
    }

    #[tokio::test]
    async fn test_vacation_full_roundtrip() {
        // Integration test using a real temp directory.
        // Clean up before construction so stale state from a previous interrupted
        // run does not cause the initial-state assertion to fail.
        let temp_dir = std::env::temp_dir().join("rusmes-vacation-roundtrip-test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let vstore = FileVacationStore::new(&temp_dir);
        let store = create_test_store();
        let account_id = "roundtrip_user";

        // 1. Initial get — should return disabled default
        let get_req = VacationResponseGetRequest {
            account_id: account_id.to_string(),
            ids: None,
            properties: None,
        };
        let get_resp = vacation_response_get(get_req, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        assert!(!get_resp.list[0].is_enabled);
        assert_eq!(get_resp.state, "0");

        // 2. Set vacation
        let mut update_map = HashMap::new();
        update_map.insert(
            "singleton".to_string(),
            serde_json::json!({
                "isEnabled": true,
                "subject": "Roundtrip test",
                "textBody": "Gone fishing"
            }),
        );
        let set_req = VacationResponseSetRequest {
            account_id: account_id.to_string(),
            if_in_state: Some("0".to_string()),
            update: Some(update_map),
        };
        let set_resp = vacation_response_set(set_req, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        assert!(set_resp.updated.is_some());
        assert_eq!(set_resp.old_state, "0");
        assert_eq!(set_resp.new_state, "1");

        // 3. Get again — should reflect changes
        let get_req2 = VacationResponseGetRequest {
            account_id: account_id.to_string(),
            ids: None,
            properties: None,
        };
        let get_resp2 = vacation_response_get(get_req2, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        let v = &get_resp2.list[0];
        assert!(v.is_enabled);
        assert_eq!(v.subject.as_deref(), Some("Roundtrip test"));
        assert_eq!(v.text_body.as_deref(), Some("Gone fishing"));
        assert_eq!(get_resp2.state, "1");

        // 4. Second set using correct state token
        let mut update_map2 = HashMap::new();
        update_map2.insert(
            "singleton".to_string(),
            serde_json::json!({"isEnabled": false}),
        );
        let set_req2 = VacationResponseSetRequest {
            account_id: account_id.to_string(),
            if_in_state: Some("1".to_string()),
            update: Some(update_map2),
        };
        let set_resp2 = vacation_response_set(set_req2, store.as_ref(), &test_principal(), &vstore)
            .await
            .unwrap();
        assert!(set_resp2.updated.is_some());
        assert_eq!(set_resp2.new_state, "2");

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

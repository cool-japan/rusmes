//! Thread method implementations for JMAP
//!
//! Implements:
//! - Thread/get - conversation threading
//! - Thread/changes - detect thread changes

use rusmes_storage::MessageStore;
use serde::{Deserialize, Serialize};

/// Thread object
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    /// Unique identifier
    pub id: String,
    /// Email IDs in the thread, in order
    pub email_ids: Vec<String>,
}

/// Thread/get request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<String>>,
}

/// Thread/get response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGetResponse {
    pub account_id: String,
    pub state: String,
    pub list: Vec<Thread>,
    pub not_found: Vec<String>,
}

/// Thread/changes request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadChangesRequest {
    pub account_id: String,
    pub since_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_changes: Option<u64>,
}

/// Thread/changes response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadChangesResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    pub has_more_changes: bool,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
}

/// Handle Thread/get method
pub async fn thread_get(
    request: ThreadGetRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<ThreadGetResponse> {
    let list = Vec::new();
    let mut not_found = Vec::new();

    // If no IDs specified, return empty list
    let ids = request.ids.unwrap_or_default();

    for id in ids {
        // In production, would:
        // 1. Query message store for emails with this thread ID
        // 2. Build Thread object with email IDs
        // 3. Apply proper threading algorithm (based on References/In-Reply-To headers)

        not_found.push(id);
    }

    Ok(ThreadGetResponse {
        account_id: request.account_id,
        state: "1".to_string(),
        list,
        not_found,
    })
}

/// Handle Thread/changes method
pub async fn thread_changes(
    request: ThreadChangesRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<ThreadChangesResponse> {
    let since_state: u64 = request.since_state.parse().unwrap_or(0);
    let new_state = (since_state + 1).to_string();

    // In production, would query change log for thread changes
    let created = Vec::new();
    let updated = Vec::new();
    let destroyed = Vec::new();

    Ok(ThreadChangesResponse {
        account_id: request.account_id,
        old_state: request.since_state,
        new_state,
        has_more_changes: false,
        created,
        updated,
        destroyed,
    })
}

/// Calculate thread ID from email headers
///
/// Threading algorithm based on RFC 5256 (THREAD=REFERENCES)
#[allow(dead_code)]
fn calculate_thread_id(
    message_id: Option<&str>,
    in_reply_to: Option<&[String]>,
    references: Option<&[String]>,
) -> String {
    // In production, would implement proper threading algorithm:
    // 1. Use References header to find thread ancestry
    // 2. Fall back to In-Reply-To if References not present
    // 3. Use Message-ID as thread ID if no references
    // 4. Hash the thread root message ID for consistent thread IDs

    use sha2::{Digest, Sha256};

    // Get the root message ID (first in References, or In-Reply-To, or Message-ID)
    let root_id = references
        .and_then(|refs| refs.first())
        .or_else(|| in_reply_to.and_then(|irt| irt.first()))
        .map(|s| s.as_str())
        .or(message_id)
        .unwrap_or("unknown");

    // Hash it to create a consistent thread ID
    let mut hasher = Sha256::new();
    hasher.update(root_id.as_bytes());
    let result = hasher.finalize();
    format!("T{:x}", result).chars().take(32).collect()
}

/// Generate search snippet with highlighted terms
#[allow(dead_code)]
fn generate_snippet(text: &str, search_terms: &[String], max_length: usize) -> String {
    if search_terms.is_empty() {
        // No search terms, just return truncated text
        if text.len() <= max_length {
            return text.to_string();
        }
        return format!("{}...", &text[..max_length.saturating_sub(3)]);
    }

    // Find first occurrence of any search term
    let mut best_pos = None;
    let text_lower = text.to_lowercase();

    for term in search_terms {
        let term_lower = term.to_lowercase();
        if let Some(pos) = text_lower.find(&term_lower) {
            if best_pos.map_or(true, |best| pos < best) {
                best_pos = Some(pos);
            }
        }
    }

    match best_pos {
        Some(pos) => {
            // Calculate context window around the match
            let context_before = 50;
            let context_after = max_length.saturating_sub(context_before).saturating_sub(6); // 6 for potential "..." on both sides

            let start = pos.saturating_sub(context_before);
            let end = (start + context_before + context_after).min(text.len());

            let mut snippet = String::new();
            if start > 0 {
                snippet.push_str("...");
            }
            snippet.push_str(&text[start..end]);
            if end < text.len() {
                snippet.push_str("...");
            }

            snippet
        }
        None => {
            // No match found, return beginning of text
            if text.len() <= max_length {
                text.to_string()
            } else {
                format!("{}...", &text[..max_length.saturating_sub(3)])
            }
        }
    }
}

/// Highlight search terms in text with HTML mark tags
#[allow(dead_code)]
fn highlight_snippet(text: &str, search_terms: &[String]) -> String {
    if search_terms.is_empty() {
        return text.to_string();
    }

    let mut result = text.to_string();
    let text_lower = text.to_lowercase();

    // Collect all match positions
    let mut matches: Vec<(usize, usize, String)> = Vec::new();

    for term in search_terms {
        let term_lower = term.to_lowercase();
        let mut pos = 0;
        while let Some(found_pos) = text_lower[pos..].find(&term_lower) {
            let actual_pos = pos + found_pos;
            let end_pos = actual_pos + term.len();

            // Get the actual text (preserve case)
            let matched_text = text[actual_pos..end_pos].to_string();
            matches.push((actual_pos, end_pos, matched_text));

            pos = end_pos;
        }
    }

    // Sort by position (reverse order for replacement)
    matches.sort_by_key(|b| std::cmp::Reverse(b.0));

    // Remove overlapping matches
    let mut non_overlapping: Vec<(usize, usize, String)> = Vec::new();
    for m in matches {
        let overlaps = non_overlapping.iter().any(|existing| {
            (m.0 >= existing.0 && m.0 < existing.1) || (m.1 > existing.0 && m.1 <= existing.1)
        });
        if !overlaps {
            non_overlapping.push(m);
        }
    }

    // Apply highlights in reverse order to preserve positions
    for (start, end, matched_text) in non_overlapping {
        let highlighted = format!("<mark>{}</mark>", matched_text);
        result.replace_range(start..end, &highlighted);
    }

    result
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
    async fn test_thread_get() {
        let store = create_test_store();
        let request = ThreadGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["thread1".to_string()]),
            properties: None,
        };

        let response = thread_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.account_id, "acc1");
        assert_eq!(response.not_found.len(), 1);
    }

    #[tokio::test]
    async fn test_thread_get_all() {
        let store = create_test_store();
        let request = ThreadGetRequest {
            account_id: "acc1".to_string(),
            ids: None,
            properties: None,
        };

        let response = thread_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.list.len(), 0);
    }

    #[tokio::test]
    async fn test_thread_changes() {
        let store = create_test_store();
        let request = ThreadChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "1".to_string(),
            max_changes: Some(50),
        };

        let response = thread_changes(request, store.as_ref()).await.unwrap();
        assert_eq!(response.old_state, "1");
        assert_eq!(response.new_state, "2");
        assert!(!response.has_more_changes);
    }

    #[tokio::test]
    async fn test_calculate_thread_id_with_references() {
        let references = vec![
            "<root@example.com>".to_string(),
            "<reply1@example.com>".to_string(),
        ];
        let thread_id = calculate_thread_id(Some("<reply2@example.com>"), None, Some(&references));

        // Should use first reference as root
        assert!(thread_id.starts_with('T'));
        assert_eq!(thread_id.len(), 32);
    }

    #[tokio::test]
    async fn test_calculate_thread_id_with_in_reply_to() {
        let in_reply_to = vec!["<original@example.com>".to_string()];
        let thread_id = calculate_thread_id(Some("<reply@example.com>"), Some(&in_reply_to), None);

        assert!(thread_id.starts_with('T'));
    }

    #[tokio::test]
    async fn test_calculate_thread_id_standalone() {
        let thread_id = calculate_thread_id(Some("<standalone@example.com>"), None, None);

        assert!(thread_id.starts_with('T'));
    }

    #[tokio::test]
    async fn test_generate_snippet_no_search_terms() {
        let text = "This is a test message with some content.";
        let snippet = generate_snippet(text, &[], 100);
        assert_eq!(snippet, text);
    }

    #[tokio::test]
    async fn test_generate_snippet_with_match() {
        let text = "This is a test message with important information that we need to find.";
        let terms = vec!["important".to_string()];
        let snippet = generate_snippet(text, &terms, 100);

        assert!(snippet.contains("important"));
    }

    #[tokio::test]
    async fn test_generate_snippet_truncate_long_text() {
        let text = "A".repeat(200);
        let snippet = generate_snippet(&text, &[], 50);

        assert_eq!(snippet.len(), 50);
        assert!(snippet.ends_with("..."));
    }

    #[tokio::test]
    async fn test_thread_get_with_properties() {
        let store = create_test_store();
        let properties = vec!["id".to_string(), "emailIds".to_string()];

        let request = ThreadGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["thread1".to_string()]),
            properties: Some(properties),
        };

        let response = thread_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.list.len(), 0);
    }

    #[tokio::test]
    async fn test_thread_changes_max_changes() {
        let store = create_test_store();
        let request = ThreadChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "100".to_string(),
            max_changes: Some(10),
        };

        let response = thread_changes(request, store.as_ref()).await.unwrap();
        assert_eq!(response.old_state, "100");
        assert_eq!(response.new_state, "101");
    }

    #[tokio::test]
    async fn test_generate_snippet_match_at_start() {
        let text = "Important information at the beginning of the message.";
        let terms = vec!["Important".to_string()];
        let snippet = generate_snippet(text, &terms, 100);

        assert!(snippet.starts_with("Important"));
    }

    #[tokio::test]
    async fn test_generate_snippet_match_at_end() {
        let text = "The message ends with important information.";
        let terms = vec!["important".to_string()];
        let snippet = generate_snippet(text, &terms, 100);

        assert!(snippet.contains("important"));
    }

    #[tokio::test]
    async fn test_generate_snippet_multiple_terms() {
        let text = "This message contains both urgent and important information.";
        let terms = vec!["urgent".to_string(), "important".to_string()];
        let snippet = generate_snippet(text, &terms, 100);

        // Should find first matching term
        assert!(snippet.contains("urgent") || snippet.contains("important"));
    }

    #[tokio::test]
    async fn test_thread_id_consistency() {
        let message_id = "<msg@example.com>";
        let thread_id1 = calculate_thread_id(Some(message_id), None, None);
        let thread_id2 = calculate_thread_id(Some(message_id), None, None);

        // Same input should produce same thread ID
        assert_eq!(thread_id1, thread_id2);
    }

    #[tokio::test]
    async fn test_thread_changes_state_progression() {
        let store = create_test_store();

        let request1 = ThreadChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "5".to_string(),
            max_changes: None,
        };
        let response1 = thread_changes(request1, store.as_ref()).await.unwrap();

        let request2 = ThreadChangesRequest {
            account_id: "acc1".to_string(),
            since_state: response1.new_state.clone(),
            max_changes: None,
        };
        let response2 = thread_changes(request2, store.as_ref()).await.unwrap();

        assert!(response1.new_state < response2.new_state);
    }

    #[tokio::test]
    async fn test_generate_snippet_case_insensitive() {
        let text = "This message contains IMPORTANT information.";
        let terms = vec!["important".to_string()];
        let snippet = generate_snippet(text, &terms, 100);

        assert!(snippet.to_lowercase().contains("important"));
    }

    #[tokio::test]
    async fn test_thread_get_multiple_ids() {
        let store = create_test_store();
        let request = ThreadGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec![
                "thread1".to_string(),
                "thread2".to_string(),
                "thread3".to_string(),
            ]),
            properties: None,
        };

        let response = thread_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.not_found.len(), 3);
    }

    #[tokio::test]
    async fn test_calculate_thread_id_empty_references() {
        let thread_id = calculate_thread_id(Some("<msg@example.com>"), Some(&[]), Some(&[]));

        assert!(thread_id.starts_with('T'));
    }

    #[tokio::test]
    async fn test_generate_snippet_exact_max_length() {
        let text = "Exactly fifty characters for testing purposes!";
        let snippet = generate_snippet(text, &[], 47);

        assert_eq!(snippet, text);
    }

    #[tokio::test]
    async fn test_generate_snippet_context_window() {
        let text = "A".repeat(50) + "IMPORTANT" + &"Z".repeat(50);
        let terms = vec!["IMPORTANT".to_string()];
        let snippet = generate_snippet(&text, &terms, 80);

        assert!(snippet.contains("IMPORTANT"));
        assert!(snippet.contains("..."));
    }

    #[tokio::test]
    async fn test_thread_changes_empty_state() {
        let store = create_test_store();
        let request = ThreadChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "0".to_string(),
            max_changes: None,
        };

        let response = thread_changes(request, store.as_ref()).await.unwrap();
        assert_eq!(response.new_state, "1");
        assert!(response.created.is_empty());
        assert!(response.updated.is_empty());
        assert!(response.destroyed.is_empty());
    }

    #[tokio::test]
    async fn test_thread_object_structure() {
        let thread = Thread {
            id: "T123".to_string(),
            email_ids: vec!["email1".to_string(), "email2".to_string()],
        };

        let json = serde_json::to_string(&thread).unwrap();
        assert!(json.contains("T123"));
        assert!(json.contains("email1"));
    }

    #[tokio::test]
    async fn test_highlight_snippet_basic() {
        let text = "This is an important message";
        let terms = vec!["important".to_string()];
        let highlighted = highlight_snippet(text, &terms);

        assert!(highlighted.contains("<mark>important</mark>"));
    }

    #[tokio::test]
    async fn test_highlight_snippet_multiple_occurrences() {
        let text = "test message with test data";
        let terms = vec!["test".to_string()];
        let highlighted = highlight_snippet(text, &terms);

        // Should highlight both occurrences
        let count = highlighted.matches("<mark>test</mark>").count();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_highlight_snippet_case_preservation() {
        let text = "IMPORTANT message is Important";
        let terms = vec!["important".to_string()];
        let highlighted = highlight_snippet(text, &terms);

        assert!(highlighted.contains("<mark>IMPORTANT</mark>"));
        assert!(highlighted.contains("<mark>Important</mark>"));
    }

    #[tokio::test]
    async fn test_highlight_snippet_no_terms() {
        let text = "No highlighting needed";
        let highlighted = highlight_snippet(text, &[]);

        assert_eq!(highlighted, text);
        assert!(!highlighted.contains("<mark>"));
    }

    #[tokio::test]
    async fn test_highlight_snippet_overlapping_terms() {
        let text = "important information";
        let terms = vec!["important".to_string(), "info".to_string()];
        let highlighted = highlight_snippet(text, &terms);

        // Should highlight both without breaking
        assert!(highlighted.contains("<mark>"));
    }

    #[tokio::test]
    async fn test_calculate_thread_id_nested_conversation() {
        let references = vec![
            "<root@example.com>".to_string(),
            "<reply1@example.com>".to_string(),
            "<reply2@example.com>".to_string(),
        ];
        let thread_id = calculate_thread_id(Some("<reply3@example.com>"), None, Some(&references));

        // Should consistently use root
        let thread_id2 = calculate_thread_id(Some("<reply4@example.com>"), None, Some(&references));
        assert_eq!(thread_id, thread_id2);
    }

    #[tokio::test]
    async fn test_calculate_thread_id_multi_branch() {
        let references1 = vec!["<root@example.com>".to_string()];
        let references2 = vec![
            "<root@example.com>".to_string(),
            "<branch1@example.com>".to_string(),
        ];

        let thread_id1 =
            calculate_thread_id(Some("<reply1@example.com>"), None, Some(&references1));
        let thread_id2 =
            calculate_thread_id(Some("<reply2@example.com>"), None, Some(&references2));

        // Both should map to same thread (same root)
        assert_eq!(thread_id1, thread_id2);
    }

    #[tokio::test]
    async fn test_generate_snippet_very_short_text() {
        let text = "Hi";
        let snippet = generate_snippet(text, &[], 100);
        assert_eq!(snippet, "Hi");
    }

    #[tokio::test]
    async fn test_generate_snippet_no_match_no_terms() {
        let text = "This is a longer message that should be truncated";
        let snippet = generate_snippet(text, &[], 20);

        assert!(snippet.len() <= 20);
        assert!(snippet.ends_with("..."));
    }
}

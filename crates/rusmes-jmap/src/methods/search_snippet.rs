//! SearchSnippet method implementations for JMAP
//!
//! Implements:
//! - SearchSnippet/get - search result preview snippets with highlighting

use crate::methods::ensure_account_ownership;
use crate::types::Principal;
use rusmes_storage::MessageStore;
use serde::{Deserialize, Serialize};

/// SearchSnippet object
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSnippet {
    /// Email ID
    pub email_id: String,
    /// Subject snippet with highlighting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Preview snippet with highlighting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// SearchSnippet/get request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSnippetGetRequest {
    pub account_id: String,
    /// Email IDs to get snippets for
    pub email_ids: Vec<String>,
    /// Filter from the Email/query that generated these results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<crate::types::EmailFilterCondition>,
}

/// SearchSnippet/get response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSnippetGetResponse {
    pub account_id: String,
    pub list: Vec<SearchSnippet>,
    pub not_found: Vec<String>,
}

/// Handle SearchSnippet/get method
pub async fn search_snippet_get(
    request: SearchSnippetGetRequest,
    _message_store: &dyn MessageStore,
    principal: &Principal,
) -> anyhow::Result<SearchSnippetGetResponse> {
    ensure_account_ownership(&request.account_id, principal)?;
    let mut list = Vec::new();
    let mut not_found = Vec::new();

    // Extract search terms from filter
    let search_terms = extract_search_terms(&request.filter);

    for email_id in request.email_ids {
        // In production, would:
        // 1. Retrieve email by ID from message store
        // 2. Extract subject and body text
        // 3. Generate snippets with highlighted context around matches
        // 4. Truncate to reasonable length

        // For now, create a simple snippet or mark as not found
        if search_terms.is_empty() {
            // No search terms, return basic snippet
            list.push(SearchSnippet {
                email_id: email_id.clone(),
                subject: None,
                preview: None,
            });
        } else {
            // Would generate actual highlighted snippets in production
            not_found.push(email_id);
        }
    }

    Ok(SearchSnippetGetResponse {
        account_id: request.account_id,
        list,
        not_found,
    })
}

/// Extract search terms from email filter condition
fn extract_search_terms(filter: &Option<crate::types::EmailFilterCondition>) -> Vec<String> {
    let mut terms = Vec::new();

    if let Some(f) = filter {
        if let Some(text) = &f.text {
            terms.extend(text.split_whitespace().map(|s| s.to_string()));
        }
        if let Some(subject) = &f.subject {
            terms.extend(subject.split_whitespace().map(|s| s.to_string()));
        }
        if let Some(body) = &f.body {
            terms.extend(body.split_whitespace().map(|s| s.to_string()));
        }
        if let Some(from) = &f.from {
            terms.extend(from.split_whitespace().map(|s| s.to_string()));
        }
    }

    terms
}

/// Generate search snippet with highlighted terms
///
/// Extracts context around matching keywords and highlights matches with `<mark>` tags.
/// Configurable context size (default: 50 chars before/after).
pub fn generate_snippet(text: &str, search_terms: &[String], max_length: usize) -> String {
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

            // Highlight the search terms
            highlight_snippet(&snippet, search_terms)
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
///
/// Example: "important message" with term "important" becomes "<mark>important</mark> message"
pub fn highlight_snippet(text: &str, search_terms: &[String]) -> String {
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

    fn test_principal() -> crate::types::Principal {
        crate::types::admin_principal_for_tests()
    }

    use super::*;
    use rusmes_storage::backends::filesystem::FilesystemBackend;
    use rusmes_storage::StorageBackend;
    use std::path::PathBuf;

    fn create_test_store() -> std::sync::Arc<dyn MessageStore> {
        let backend = FilesystemBackend::new(PathBuf::from("/tmp/rusmes-test-storage"));
        backend.message_store()
    }

    #[tokio::test]
    async fn test_search_snippet_get() {
        let store = create_test_store();
        let request = SearchSnippetGetRequest {
            account_id: "acc1".to_string(),
            email_ids: vec!["email1".to_string()],
            filter: None,
        };

        let response = search_snippet_get(request, store.as_ref(), &test_principal())
            .await
            .unwrap();
        assert_eq!(response.account_id, "acc1");
        assert_eq!(response.list.len(), 1);
    }

    #[tokio::test]
    async fn test_search_snippet_multiple_emails() {
        let store = create_test_store();
        let request = SearchSnippetGetRequest {
            account_id: "acc1".to_string(),
            email_ids: vec![
                "email1".to_string(),
                "email2".to_string(),
                "email3".to_string(),
            ],
            filter: None,
        };

        let response = search_snippet_get(request, store.as_ref(), &test_principal())
            .await
            .unwrap();
        assert_eq!(response.list.len(), 3);
    }

    #[tokio::test]
    async fn test_search_snippet_with_filter() {
        let store = create_test_store();
        let filter = crate::types::EmailFilterCondition {
            in_mailbox: None,
            in_mailbox_other_than: None,
            before: None,
            after: None,
            min_size: None,
            max_size: None,
            all_in_thread_have_keyword: None,
            some_in_thread_have_keyword: None,
            none_in_thread_have_keyword: None,
            has_keyword: None,
            not_keyword: None,
            has_attachment: None,
            text: Some("search term".to_string()),
            from: None,
            to: None,
            cc: None,
            bcc: None,
            subject: None,
            body: None,
            header: None,
        };

        let request = SearchSnippetGetRequest {
            account_id: "acc1".to_string(),
            email_ids: vec!["email1".to_string()],
            filter: Some(filter),
        };

        let response = search_snippet_get(request, store.as_ref(), &test_principal())
            .await
            .unwrap();
        assert_eq!(response.account_id, "acc1");
    }

    #[tokio::test]
    async fn test_search_snippet_empty_email_ids() {
        let store = create_test_store();
        let request = SearchSnippetGetRequest {
            account_id: "acc1".to_string(),
            email_ids: vec![],
            filter: None,
        };

        let response = search_snippet_get(request, store.as_ref(), &test_principal())
            .await
            .unwrap();
        assert_eq!(response.list.len(), 0);
        assert_eq!(response.not_found.len(), 0);
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
        assert!(snippet.contains("<mark>"));
    }

    #[tokio::test]
    async fn test_generate_snippet_truncate_long_text() {
        let text = "A".repeat(200);
        let snippet = generate_snippet(&text, &[], 50);

        assert!(snippet.len() <= 53); // 50 + "..."
        assert!(snippet.ends_with("..."));
    }

    #[tokio::test]
    async fn test_generate_snippet_match_at_start() {
        let text = "Important information at the beginning of the message.";
        let terms = vec!["Important".to_string()];
        let snippet = generate_snippet(text, &terms, 100);

        assert!(snippet.contains("<mark>Important</mark>"));
    }

    #[tokio::test]
    async fn test_generate_snippet_match_at_end() {
        let text = "The message ends with important information.";
        let terms = vec!["important".to_string()];
        let snippet = generate_snippet(text, &terms, 100);

        assert!(snippet.contains("<mark>important</mark>"));
    }

    #[tokio::test]
    async fn test_generate_snippet_multiple_terms() {
        let text = "This message contains both urgent and important information.";
        let terms = vec!["urgent".to_string(), "important".to_string()];
        let snippet = generate_snippet(text, &terms, 100);

        // Should find first matching term
        assert!(
            snippet.contains("<mark>urgent</mark>") || snippet.contains("<mark>important</mark>")
        );
    }

    #[tokio::test]
    async fn test_generate_snippet_case_insensitive() {
        let text = "This message contains IMPORTANT information.";
        let terms = vec!["important".to_string()];
        let snippet = generate_snippet(text, &terms, 100);

        assert!(snippet.to_lowercase().contains("<mark>important</mark>"));
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

        assert!(snippet.contains("<mark>IMPORTANT</mark>"));
        assert!(snippet.contains("..."));
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
    async fn test_extract_search_terms_text() {
        let filter = crate::types::EmailFilterCondition {
            in_mailbox: None,
            in_mailbox_other_than: None,
            before: None,
            after: None,
            min_size: None,
            max_size: None,
            all_in_thread_have_keyword: None,
            some_in_thread_have_keyword: None,
            none_in_thread_have_keyword: None,
            has_keyword: None,
            not_keyword: None,
            has_attachment: None,
            text: Some("search term".to_string()),
            from: None,
            to: None,
            cc: None,
            bcc: None,
            subject: None,
            body: None,
            header: None,
        };

        let terms = extract_search_terms(&Some(filter));
        assert_eq!(terms.len(), 2);
        assert!(terms.contains(&"search".to_string()));
        assert!(terms.contains(&"term".to_string()));
    }

    #[tokio::test]
    async fn test_extract_search_terms_multiple_fields() {
        let filter = crate::types::EmailFilterCondition {
            in_mailbox: None,
            in_mailbox_other_than: None,
            before: None,
            after: None,
            min_size: None,
            max_size: None,
            all_in_thread_have_keyword: None,
            some_in_thread_have_keyword: None,
            none_in_thread_have_keyword: None,
            has_keyword: None,
            not_keyword: None,
            has_attachment: None,
            text: Some("search".to_string()),
            from: Some("user".to_string()),
            to: None,
            cc: None,
            bcc: None,
            subject: Some("important".to_string()),
            body: Some("message".to_string()),
            header: None,
        };

        let terms = extract_search_terms(&Some(filter));
        assert!(terms.len() >= 4);
    }

    #[tokio::test]
    async fn test_extract_search_terms_empty() {
        let terms = extract_search_terms(&None);
        assert_eq!(terms.len(), 0);
    }

    #[tokio::test]
    async fn test_search_snippet_object_structure() {
        let snippet = SearchSnippet {
            email_id: "email1".to_string(),
            subject: Some("Test Subject".to_string()),
            preview: Some("This is a preview...".to_string()),
        };

        let json = serde_json::to_string(&snippet).unwrap();
        assert!(json.contains("email1"));
        assert!(json.contains("Test Subject"));
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

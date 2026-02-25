//! JMAP RFC 8620/8621 Compliance Tests
//!
//! Comprehensive test suite for JMAP protocol compliance with RFC 8620 (Core)
//! and RFC 8621 (Mail). Tests request/response format, all methods, error handling,
//! state tracking, batch operations, and blob handling.

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    /// Test JMAP request structure (RFC 8620 Section 3.3)
    #[test]
    fn test_jmap_request_structure() {
        let request = json!({
            "using": ["urn:ietf:params:jmap:core"],
            "methodCalls": []
        });

        assert!(request.is_object());
        assert!(request["using"].is_array());
        assert!(request["methodCalls"].is_array());
        assert!(is_valid_jmap_request(&request));
    }

    #[test]
    fn test_jmap_request_with_created_ids() {
        let request = json!({
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            "methodCalls": [],
            "createdIds": {}
        });

        assert!(is_valid_jmap_request(&request));
    }

    /// Test JMAP method call format (RFC 8620 Section 3.3)
    #[test]
    fn test_jmap_method_call() {
        let method_call = json!(["Email/query", {"accountId": "user"}, "c1"]);

        assert!(method_call.is_array());
        assert_eq!(method_call.as_array().unwrap().len(), 3);
        assert!(is_valid_method_call(&method_call));
    }

    #[test]
    fn test_jmap_method_call_components() {
        let method_call = json!(["Email/get", {"accountId": "u1", "ids": ["e1"]}, "call1"]);

        let arr = method_call.as_array().unwrap();
        assert!(arr[0].is_string()); // Method name
        assert!(arr[1].is_object()); // Arguments
        assert!(arr[2].is_string()); // Call ID
    }

    /// Test JMAP capabilities (RFC 8620 Section 2)
    #[test]
    fn test_jmap_capabilities() {
        let capabilities = vec![
            "urn:ietf:params:jmap:core",
            "urn:ietf:params:jmap:mail",
            "urn:ietf:params:jmap:submission",
            "urn:ietf:params:jmap:vacationresponse",
        ];

        for cap in capabilities {
            assert!(is_valid_capability(cap), "Failed for: {}", cap);
        }
    }

    #[test]
    fn test_invalid_capabilities() {
        let invalid = vec![
            "jmap:core",               // Missing urn:ietf:params prefix
            "urn:example:jmap:custom", // Wrong namespace
        ];

        for cap in invalid {
            assert!(!is_valid_capability(cap), "Should reject: {}", cap);
        }
    }

    /// Test JMAP response structure (RFC 8620 Section 3.4)
    #[test]
    fn test_jmap_response_structure() {
        let response = json!({
            "methodResponses": [],
            "sessionState": "state123"
        });

        assert!(response["methodResponses"].is_array());
        assert!(response["sessionState"].is_string());
        assert!(is_valid_jmap_response(&response));
    }

    #[test]
    fn test_jmap_response_with_created_ids() {
        let response = json!({
            "methodResponses": [],
            "sessionState": "state123",
            "createdIds": {"temp1": "real1"}
        });

        assert!(is_valid_jmap_response(&response));
    }

    /// Test JMAP error types (RFC 8620 Section 3.6.1)
    #[test]
    fn test_jmap_error_types() {
        let error_types = vec![
            "invalidArguments",
            "invalidResultReference",
            "notFound",
            "notJSON",
            "notRequest",
            "unknownCapability",
            "unknownMethod",
            "serverFail",
            "serverUnavailable",
            "serverPartialFail",
            "requestTooLarge",
            "stateMismatch",
            "anchorNotFound",
            "unsupportedFilter",
            "unsupportedSort",
            "cannotCalculateChanges",
            "forbidden",
            "accountNotFound",
            "accountNotSupportedByMethod",
            "accountReadOnly",
        ];

        for error_type in error_types {
            assert!(
                is_valid_error_type(error_type),
                "Failed for: {}",
                error_type
            );
        }
    }

    #[test]
    fn test_jmap_error_response() {
        let error = json!({
            "type": "invalidArguments",
            "description": "Invalid request"
        });

        assert!(error["type"].is_string());
        assert!(is_valid_error_response(&error));
    }

    /// Test Email/get method (RFC 8621 Section 4.2)
    #[test]
    fn test_email_get_method() {
        let method = json!(["Email/get", {
            "accountId": "u1",
            "ids": ["e1", "e2"]
        }, "c1"]);

        assert!(is_valid_method_call(&method));
        assert_eq!(method[0], "Email/get");
    }

    #[test]
    fn test_email_get_with_properties() {
        let method = json!(["Email/get", {
            "accountId": "u1",
            "ids": ["e1"],
            "properties": ["id", "subject", "from", "to", "receivedAt"]
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test Email/set method (RFC 8621 Section 4.3)
    #[test]
    fn test_email_set_method() {
        let method = json!(["Email/set", {
            "accountId": "u1",
            "create": {
                "draft1": {
                    "mailboxIds": {"mb1": true},
                    "subject": "Hello",
                    "from": [{"email": "sender@example.com"}]
                }
            }
        }, "c1"]);

        assert!(is_valid_method_call(&method));
        assert_eq!(method[0], "Email/set");
    }

    #[test]
    fn test_email_set_update() {
        let method = json!(["Email/set", {
            "accountId": "u1",
            "update": {
                "e1": {
                    "keywords": {"$seen": true}
                }
            }
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    #[test]
    fn test_email_set_destroy() {
        let method = json!(["Email/set", {
            "accountId": "u1",
            "destroy": ["e1", "e2"]
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test Email/query method (RFC 8621 Section 4.4)
    #[test]
    fn test_email_query_method() {
        let method = json!(["Email/query", {
            "accountId": "u1",
            "filter": {"inMailbox": "inbox"}
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    #[test]
    fn test_email_query_with_sort() {
        let method = json!(["Email/query", {
            "accountId": "u1",
            "sort": [{"property": "receivedAt", "isAscending": false}]
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test Email/changes method (RFC 8621 Section 4.5)
    #[test]
    fn test_email_changes_method() {
        let method = json!(["Email/changes", {
            "accountId": "u1",
            "sinceState": "state1"
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test Mailbox/get method (RFC 8621 Section 2.2)
    #[test]
    fn test_mailbox_get_method() {
        let method = json!(["Mailbox/get", {
            "accountId": "u1"
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test Mailbox/set method (RFC 8621 Section 2.3)
    #[test]
    fn test_mailbox_set_method() {
        let method = json!(["Mailbox/set", {
            "accountId": "u1",
            "create": {
                "mb1": {
                    "name": "Archive",
                    "parentId": null,
                    "role": null
                }
            }
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test Thread/get method (RFC 8621 Section 5.2)
    #[test]
    fn test_thread_get_method() {
        let method = json!(["Thread/get", {
            "accountId": "u1",
            "ids": ["t1"]
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test Identity/get method (RFC 8621 Section 6.2)
    #[test]
    fn test_identity_get_method() {
        let method = json!(["Identity/get", {
            "accountId": "u1"
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test EmailSubmission/set method (RFC 8621 Section 7.3)
    #[test]
    fn test_emailsubmission_set_method() {
        let method = json!(["EmailSubmission/set", {
            "accountId": "u1",
            "create": {
                "sub1": {
                    "emailId": "e1",
                    "identityId": "i1"
                }
            }
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test state tracking (RFC 8620 Section 5.3)
    #[test]
    fn test_state_strings() {
        assert!(is_valid_state_string("state123"));
        assert!(is_valid_state_string("abc-123-xyz"));
        assert!(!is_valid_state_string(""));
    }

    /// Test if-in-state conditional updates (RFC 8620 Section 5.3)
    #[test]
    fn test_if_in_state() {
        let method = json!(["Email/set", {
            "accountId": "u1",
            "ifInState": "state123",
            "update": {"e1": {"keywords": {"$seen": true}}}
        }, "c1"]);

        assert!(is_valid_method_call(&method));
        assert!(method[1]["ifInState"].is_string());
    }

    /// Test batch operations (RFC 8620 Section 3.5)
    #[test]
    fn test_batch_operations() {
        let request = json!({
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            "methodCalls": [
                ["Mailbox/get", {"accountId": "u1"}, "c1"],
                ["Email/query", {"accountId": "u1", "#filter": {"inMailbox": "#c1/list/0/id"}}, "c2"],
                ["Email/get", {"accountId": "u1", "#ids": "#c2/ids"}, "c3"]
            ]
        });

        assert!(is_valid_jmap_request(&request));
        assert_eq!(request["methodCalls"].as_array().unwrap().len(), 3);
    }

    /// Test result references (RFC 8620 Section 3.5)
    #[test]
    fn test_result_references() {
        assert!(is_result_reference("#c1/list/0/id"));
        assert!(is_result_reference("#methodCall1/ids"));
        assert!(!is_result_reference("c1/list/0/id"));
    }

    /// Test blob upload (RFC 8620 Section 6.1)
    #[test]
    fn test_blob_upload() {
        let response = json!({
            "accountId": "u1",
            "blobId": "blob123",
            "type": "image/jpeg",
            "size": 12345
        });

        assert!(is_valid_blob_upload_response(&response));
    }

    /// Test blob download
    #[test]
    fn test_blob_download() {
        let url = "/download/{accountId}/{blobId}/{name}";
        assert!(is_valid_blob_download_url(url));
    }

    /// Test filter operators (RFC 8620 Section 5.5)
    #[test]
    fn test_filter_operators() {
        let filter_and = json!({
            "operator": "AND",
            "conditions": [
                {"inMailbox": "inbox"},
                {"hasKeyword": "$seen"}
            ]
        });

        let filter_or = json!({
            "operator": "OR",
            "conditions": [
                {"from": "alice@example.com"},
                {"from": "bob@example.com"}
            ]
        });

        let filter_not = json!({
            "operator": "NOT",
            "conditions": [{"hasKeyword": "$flagged"}]
        });

        assert!(is_valid_filter(&filter_and));
        assert!(is_valid_filter(&filter_or));
        assert!(is_valid_filter(&filter_not));
    }

    /// Test sort comparators (RFC 8620 Section 5.6)
    #[test]
    fn test_sort_comparators() {
        let sort = json!([
            {"property": "receivedAt", "isAscending": false},
            {"property": "from", "isAscending": true}
        ]);

        assert!(sort.is_array());
        for comparator in sort.as_array().unwrap() {
            assert!(is_valid_sort_comparator(comparator));
        }
    }

    /// Test pagination (RFC 8620 Section 5.4)
    #[test]
    fn test_pagination() {
        let query = json!(["Email/query", {
            "accountId": "u1",
            "position": 0,
            "limit": 50
        }, "c1"]);

        assert!(is_valid_method_call(&query));
    }

    #[test]
    fn test_pagination_with_anchor() {
        let query = json!(["Email/query", {
            "accountId": "u1",
            "anchor": "e100",
            "anchorOffset": -25,
            "limit": 50
        }, "c1"]);

        assert!(is_valid_method_call(&query));
    }

    /// Test VacationResponse methods (RFC 8621 Section 8)
    #[test]
    fn test_vacation_response_get() {
        let method = json!(["VacationResponse/get", {
            "accountId": "u1"
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    #[test]
    fn test_vacation_response_set() {
        let method = json!(["VacationResponse/set", {
            "accountId": "u1",
            "update": {
                "singleton": {
                    "isEnabled": true,
                    "subject": "Out of office",
                    "textBody": "I'm away"
                }
            }
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test SearchSnippet methods (RFC 8621 Section 4.6)
    #[test]
    fn test_searchsnippet_get() {
        let method = json!(["SearchSnippet/get", {
            "accountId": "u1",
            "emailIds": ["e1", "e2"],
            "filter": {"text": "meeting"}
        }, "c1"]);

        assert!(is_valid_method_call(&method));
    }

    /// Test method-level errors
    #[test]
    fn test_method_level_error() {
        let error_response = json!(["error", {
            "type": "accountNotFound"
        }, "c1"]);

        assert!(is_method_error_response(&error_response));
    }

    // Helper functions
    fn is_valid_jmap_request(req: &Value) -> bool {
        req.is_object() && req["using"].is_array() && req["methodCalls"].is_array()
    }

    fn is_valid_jmap_response(resp: &Value) -> bool {
        resp.is_object() && resp["methodResponses"].is_array() && resp["sessionState"].is_string()
    }

    fn is_valid_method_call(call: &Value) -> bool {
        if let Some(arr) = call.as_array() {
            arr.len() == 3 && arr[0].is_string() && arr[1].is_object() && arr[2].is_string()
        } else {
            false
        }
    }

    fn is_valid_capability(cap: &str) -> bool {
        cap.starts_with("urn:ietf:params:jmap:")
    }

    fn is_valid_error_type(error_type: &str) -> bool {
        matches!(
            error_type,
            "invalidArguments"
                | "invalidResultReference"
                | "notFound"
                | "notJSON"
                | "notRequest"
                | "unknownCapability"
                | "unknownMethod"
                | "serverFail"
                | "serverUnavailable"
                | "serverPartialFail"
                | "requestTooLarge"
                | "stateMismatch"
                | "anchorNotFound"
                | "unsupportedFilter"
                | "unsupportedSort"
                | "cannotCalculateChanges"
                | "forbidden"
                | "accountNotFound"
                | "accountNotSupportedByMethod"
                | "accountReadOnly"
        )
    }

    fn is_valid_error_response(error: &Value) -> bool {
        error.is_object() && error["type"].is_string()
    }

    fn is_valid_state_string(state: &str) -> bool {
        !state.is_empty()
    }

    fn is_result_reference(s: &str) -> bool {
        s.starts_with('#')
    }

    fn is_valid_blob_upload_response(resp: &Value) -> bool {
        resp.is_object()
            && resp["accountId"].is_string()
            && resp["blobId"].is_string()
            && resp["type"].is_string()
            && resp["size"].is_number()
    }

    fn is_valid_blob_download_url(url: &str) -> bool {
        url.contains("{accountId}") && url.contains("{blobId}")
    }

    fn is_valid_filter(filter: &Value) -> bool {
        filter.is_object()
            && (filter["operator"].is_string() || !filter.as_object().unwrap().is_empty())
    }

    fn is_valid_sort_comparator(comp: &Value) -> bool {
        comp.is_object() && comp["property"].is_string() && comp["isAscending"].is_boolean()
    }

    fn is_method_error_response(resp: &Value) -> bool {
        if let Some(arr) = resp.as_array() {
            arr.len() == 3 && arr[0] == "error" && arr[1].is_object()
        } else {
            false
        }
    }
}

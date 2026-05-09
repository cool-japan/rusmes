//! RFC 8620 §3.7 back-reference resolution for JMAP method call batches.
//!
//! When a JMAP request contains multiple method calls, later calls may
//! reference the results of earlier calls using `ResultReference` objects.
//! Any argument value that is a JSON object of the form
//!
//! ```json
//! { "#refKey": { "resultOf": "call-id", "name": "Method/name", "path": "/json/pointer" } }
//! ```
//!
//! is resolved by the dispatcher before invoking the method. The `#refKey`
//! argument is replaced with the value extracted from the referenced response
//! body using the RFC 6901 JSON Pointer in `path`.

use serde::Deserialize;
use serde_json::{Map, Value};

// ── Public types ─────────────────────────────────────────────────────────────

/// A RFC 8620 §3.7 result reference embedded inside a method call argument.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct ResultReference {
    /// The `id` of the earlier method call whose response is referenced.
    #[serde(rename = "resultOf")]
    pub result_of: String,

    /// The method name of the earlier call (must match to guard against
    /// accidentally picking up a different method's response for the same ID).
    pub name: String,

    /// RFC 6901 JSON Pointer into the response body, e.g. `/list/0/id`.
    pub path: String,
}

/// Error variants returned when a back-reference cannot be resolved.
#[derive(thiserror::Error, Debug)]
pub enum BackRefError {
    /// No completed call with the requested `id` (and `name`) was found.
    #[error("result not found: no completed call with id={0:?} and method={1:?}")]
    ResultNotFound(String, String),

    /// The referenced call completed with an error response.
    #[error("referenced call id={0:?} returned an error response")]
    ResultWasError(String),

    /// The JSON Pointer path did not resolve to any value in the response body.
    #[error("path {0:?} resolved to nothing in the response for call id={1:?}")]
    PathNotFound(String, String),
}

// ── Core algorithm ────────────────────────────────────────────────────────────

/// Walk `args` and resolve every `#key` result-reference in-place.
///
/// `completed` is the list of already-executed calls represented as
/// `(call_id, method_name, response_body)` triples.
///
/// The function only considers top-level keys in `args` that start with `#`.
/// It attempts to deserialise each such value as a `ResultReference`. If the
/// shape does not match (i.e. the value is not an object with `resultOf`,
/// `name`, and `path` fields), the key is left unchanged — it may be a
/// legitimate `#`-prefixed JSON key from a protocol extension.
///
/// On success, the `#key` entry is removed from `args` and replaced with
/// `key` (without the leading `#`) set to the extracted value.
///
/// # Errors
///
/// Returns the *first* error encountered if any reference fails to resolve.
pub fn resolve_back_references(
    args: &mut Map<String, Value>,
    completed: &[(String, String, Value)],
) -> Result<(), BackRefError> {
    // Collect the keys we need to process. Avoid mutating while iterating.
    let ref_keys: Vec<String> = args
        .keys()
        .filter(|k| k.starts_with('#'))
        .cloned()
        .collect();

    for hash_key in ref_keys {
        let raw = match args.get(&hash_key) {
            Some(v) => v.clone(),
            None => continue,
        };

        // Try to deserialise the value as a ResultReference.
        // If it doesn't match the shape, leave the key alone.
        let reference: ResultReference = match serde_json::from_value(raw) {
            Ok(r) => r,
            Err(_) => continue,
        };

        // Locate the completed response with matching id + name.
        let (_id, _name, response_body) = match completed
            .iter()
            .find(|(id, name, _)| id == &reference.result_of && name == &reference.name)
        {
            Some(entry) => entry,
            None => {
                return Err(BackRefError::ResultNotFound(
                    reference.result_of,
                    reference.name,
                ));
            }
        };

        // Reject if the response body represents a method-level error.
        // RFC 8620 §3.7.2: it is an error if the referenced call returned
        // a method error (the response name would be "error").
        //
        // We detect this by checking whether the response body has a
        // top-level "type" key whose value starts with the JMAP error URN
        // prefix — consistent with the way JmapError is serialised in this
        // codebase.
        if is_error_response(response_body) {
            return Err(BackRefError::ResultWasError(reference.result_of.clone()));
        }

        // Apply the JSON Pointer to extract the target value.
        let extracted = match response_body.pointer(&reference.path) {
            Some(v) => v.clone(),
            None => {
                return Err(BackRefError::PathNotFound(
                    reference.path.clone(),
                    reference.result_of.clone(),
                ));
            }
        };

        // Replace #key with key (sans leading #).
        let plain_key = hash_key[1..].to_string();
        args.remove(&hash_key);
        args.insert(plain_key, extracted);
    }

    Ok(())
}

// ── Private helpers ──────────────────────────────────────────────────────────

/// Returns `true` if `body` looks like a serialised JMAP method error.
///
/// The heuristic checks for a top-level `"type"` key whose string value
/// contains the JMAP error URN prefix `urn:ietf:params:jmap:error:`.
fn is_error_response(body: &Value) -> bool {
    body.get("type")
        .and_then(Value::as_str)
        .map(|t| t.starts_with("urn:ietf:params:jmap:error:"))
        .unwrap_or(false)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── helper ────────────────────────────────────────────────────────────────

    fn completed_entry(id: &str, name: &str, body: Value) -> (String, String, Value) {
        (id.to_string(), name.to_string(), body)
    }

    // ── test_back_ref_resolves_correctly ─────────────────────────────────────

    /// A two-call batch where call 1 uses a ResultReference into call 0's
    /// response.  The `#ids` argument must be resolved to the `/ids` array
    /// from the Email/query response.
    #[test]
    fn test_back_ref_resolves_correctly() {
        // Simulate call 0 having returned an Email/query response.
        let query_response = json!({
            "accountId": "acc1",
            "queryState": "s1",
            "canCalculateChanges": false,
            "position": 0,
            "ids": ["email-1", "email-2", "email-3"]
        });

        let completed = vec![completed_entry("c0", "Email/query", query_response)];

        // Build arguments for call 1 that back-reference call 0.
        let mut args = serde_json::Map::new();
        args.insert("accountId".to_string(), json!("acc1"));
        // The # key references call 0's /ids array.
        args.insert(
            "#ids".to_string(),
            json!({
                "resultOf": "c0",
                "name": "Email/query",
                "path": "/ids"
            }),
        );

        resolve_back_references(&mut args, &completed).expect("should resolve");

        // #ids was removed and ids was inserted with the extracted array.
        assert!(!args.contains_key("#ids"), "#ids should have been removed");
        assert_eq!(
            args["ids"],
            json!(["email-1", "email-2", "email-3"]),
            "ids should equal the extracted array"
        );
        // accountId should be untouched.
        assert_eq!(args["accountId"], json!("acc1"));
    }

    /// Back-reference to a specific nested value inside the response body.
    #[test]
    fn test_back_ref_resolves_nested_path() {
        let email_get_response = json!({
            "accountId": "acc1",
            "state": "s2",
            "list": [
                { "id": "email-1", "threadId": "T1", "subject": "Hello" }
            ],
            "notFound": []
        });

        let completed = vec![completed_entry("c0", "Email/get", email_get_response)];

        let mut args = serde_json::Map::new();
        args.insert(
            "#threadId".to_string(),
            json!({
                "resultOf": "c0",
                "name": "Email/get",
                "path": "/list/0/threadId"
            }),
        );

        resolve_back_references(&mut args, &completed).expect("should resolve nested");

        assert!(!args.contains_key("#threadId"));
        assert_eq!(args["threadId"], json!("T1"));
    }

    // ── test_back_ref_result_not_found ────────────────────────────────────────

    /// A reference to a non-existent call ID must return `ResultNotFound`.
    #[test]
    fn test_back_ref_result_not_found() {
        let completed: Vec<(String, String, Value)> = vec![];

        let mut args = serde_json::Map::new();
        args.insert(
            "#ids".to_string(),
            json!({
                "resultOf": "ghost-call",
                "name": "Email/query",
                "path": "/ids"
            }),
        );

        let err =
            resolve_back_references(&mut args, &completed).expect_err("should return an error");

        assert!(
            matches!(err, BackRefError::ResultNotFound(ref id, _) if id == "ghost-call"),
            "unexpected error variant: {err}"
        );
    }

    /// A reference whose method name does not match the completed call must
    /// also return `ResultNotFound`.
    #[test]
    fn test_back_ref_result_not_found_wrong_name() {
        let completed = vec![completed_entry(
            "c0",
            "Email/get",
            json!({ "list": [], "notFound": [] }),
        )];

        let mut args = serde_json::Map::new();
        args.insert(
            "#ids".to_string(),
            json!({
                "resultOf": "c0",
                "name": "Email/query",   // wrong method name
                "path": "/ids"
            }),
        );

        let err = resolve_back_references(&mut args, &completed)
            .expect_err("should return an error for method-name mismatch");

        assert!(matches!(err, BackRefError::ResultNotFound(..)));
    }

    // ── test_back_ref_path_not_found ──────────────────────────────────────────

    /// A valid call ID but an invalid (non-existent) JSON Pointer path must
    /// return `PathNotFound`.
    #[test]
    fn test_back_ref_path_not_found() {
        let completed = vec![completed_entry(
            "c0",
            "Email/query",
            json!({
                "accountId": "acc1",
                "ids": ["e1"]
            }),
        )];

        let mut args = serde_json::Map::new();
        args.insert(
            "#missingKey".to_string(),
            json!({
                "resultOf": "c0",
                "name": "Email/query",
                "path": "/doesNotExist/deeply/nested"
            }),
        );

        let err =
            resolve_back_references(&mut args, &completed).expect_err("should return PathNotFound");

        assert!(
            matches!(err, BackRefError::PathNotFound(ref p, _) if p == "/doesNotExist/deeply/nested"),
            "unexpected error: {err}"
        );
    }

    // ── error-response detection ──────────────────────────────────────────────

    /// A reference whose earlier call returned a method-level error must
    /// return `ResultWasError`.
    #[test]
    fn test_back_ref_result_was_error() {
        let error_body = json!({
            "type": "urn:ietf:params:jmap:error:serverFail",
            "detail": "something went wrong"
        });
        let completed = vec![completed_entry("c0", "Email/query", error_body)];

        let mut args = serde_json::Map::new();
        args.insert(
            "#ids".to_string(),
            json!({
                "resultOf": "c0",
                "name": "Email/query",
                "path": "/ids"
            }),
        );

        let err = resolve_back_references(&mut args, &completed)
            .expect_err("should return ResultWasError");

        assert!(
            matches!(err, BackRefError::ResultWasError(ref id) if id == "c0"),
            "unexpected error: {err}"
        );
    }

    // ── non-ResultReference shaped # keys are left alone ─────────────────────

    /// A `#`-prefixed key whose value is not a ResultReference object must
    /// be left untouched (it may be a legitimate protocol-extension key).
    #[test]
    fn test_back_ref_non_ref_shape_left_alone() {
        let completed: Vec<(String, String, Value)> = vec![];

        let mut args = serde_json::Map::new();
        args.insert("#notARef".to_string(), json!("plain string value"));
        args.insert("#alsoNotARef".to_string(), json!({ "someOtherField": 42 }));

        resolve_back_references(&mut args, &completed).expect("should succeed");

        // Neither key was touched.
        assert_eq!(args["#notARef"], json!("plain string value"));
        assert_eq!(args["#alsoNotARef"], json!({ "someOtherField": 42 }));
    }
}

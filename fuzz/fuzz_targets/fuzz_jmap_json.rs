#![no_main]

use libfuzzer_sys::fuzz_target;
use rusmes_jmap::types::{JmapRequest, JmapResponse};

fuzz_target!(|data: &[u8]| {
    // Test JMAP request JSON parsing
    if let Ok(s) = std::str::from_utf8(data) {
        // Basic JSON parsing
        let _: Result<JmapRequest, _> = serde_json::from_str(s);
        let _: Result<JmapResponse, _> = serde_json::from_str(s);

        // Test with serde_json::Value for generic JSON
        let _: Result<serde_json::Value, _> = serde_json::from_str(s);

        // Test JMAP-specific structures
        if s.len() < 100 {
            // Test valid JMAP request structure
            let jmap_req = format!(
                r#"{{
                    "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
                    "methodCalls": [
                        ["Email/query", {{}}, "c1"]
                    ]
                }}"#
            );
            let _: Result<JmapRequest, _> = serde_json::from_str(&jmap_req);

            // Test with fuzzed data in method calls
            let fuzzed_method = format!(
                r#"{{
                    "using": ["urn:ietf:params:jmap:core"],
                    "methodCalls": [
                        ["Email/get", {{"ids": ["{}"], "properties": null}}, "c1"]
                    ]
                }}"#,
                s.replace('"', r#"\""#).replace('\n', "\\n")
            );
            let _: Result<JmapRequest, _> = serde_json::from_str(&fuzzed_method);
        }

        // Test deeply nested JSON objects (potential stack overflow)
        if s.len() < 20 {
            let mut nested = String::new();
            for _ in 0..100 {
                nested.push_str(r#"{"a":"#);
            }
            nested.push_str(r#""""#);
            for _ in 0..100 {
                nested.push('}');
            }
            let _: Result<serde_json::Value, _> = serde_json::from_str(&nested);
        }

        // Test large arrays (potential memory exhaustion)
        if s.len() < 50 {
            let mut large_array = String::from("[");
            for i in 0..1000 {
                if i > 0 {
                    large_array.push(',');
                }
                large_array.push_str(&format!(r#""{}""#, s.replace('"', "")));
            }
            large_array.push(']');
            let _: Result<serde_json::Value, _> = serde_json::from_str(&large_array);
        }

        // Test invalid UTF-8 in JSON strings
        let with_escapes = format!(r#"{{"test": "{}"}}"#, s.replace('"', r#"\""#).replace('\\', r#"\\"#));
        let _: Result<serde_json::Value, _> = serde_json::from_str(&with_escapes);

        // Test type confusion
        let type_tests = [
            format!(r#"{{"number": {}}}"#, s),
            format!(r#"{{"bool": {}}}"#, s),
            format!(r#"{{"null": {}}}"#, s),
            format!(r#"{{"string": "{}"}}"#, s.replace('"', r#"\""#)),
            format!(r#"{{"array": [{}]}}"#, s),
            format!(r#"{{"object": {{{}}}}}"#, s),
        ];

        for test in &type_tests {
            let _: Result<serde_json::Value, _> = serde_json::from_str(test);
        }
    }

    // Test binary data as JSON
    let _: Result<serde_json::Value, _> = serde_json::from_slice(data);

    // Test with BOM and other Unicode edge cases
    if data.len() > 0 && data.len() < 256 {
        let mut with_bom = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        with_bom.extend_from_slice(data);
        let _: Result<serde_json::Value, _> = serde_json::from_slice(&with_bom);
    }

    // Test malformed JSON
    let malformed_tests = [
        &b"{"[..],
        &b"}"[..],
        &b"["[..],
        &b"]"[..],
        &b"{]"[..],
        &b"[}"[..],
        &b"{\"a\":}"[..],
        &b"{\"a\""[..],
        &b"\"unclosed"[..],
    ];

    for test in &malformed_tests {
        let _: Result<serde_json::Value, _> = serde_json::from_slice(test);
    }
});

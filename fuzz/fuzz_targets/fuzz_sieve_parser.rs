#![no_main]

use libfuzzer_sys::fuzz_target;
use rusmes_core::sieve::parser::SieveScript;

fuzz_target!(|data: &[u8]| {
    // Test valid UTF-8 Sieve scripts
    if let Ok(s) = std::str::from_utf8(data) {
        // Basic parsing
        let _ = SieveScript::parse(s);

        // Test with string escapes
        if s.len() < 50 {
            let escaped = format!(r#"fileinto "{}";"#, s.replace('"', r#"\""#));
            let _ = SieveScript::parse(&escaped);

            // Test multi-line strings
            let multiline = format!(r#"
                require "fileinto";
                if header :contains "Subject" "{}" {{
                    fileinto "Test";
                }}
            "#, s);
            let _ = SieveScript::parse(&multiline);

            // Test nested conditions (potential stack overflow)
            let mut nested = String::from("if true {");
            for _ in 0..20 {
                nested.push_str(" if true {");
            }
            nested.push_str(" keep; ");
            for _ in 0..20 {
                nested.push_str(" }");
            }
            nested.push_str(" }");
            let _ = SieveScript::parse(&nested);

            // Test allof/anyof with many conditions
            let mut conditions = String::from("if allof(");
            for i in 0..50 {
                if i > 0 {
                    conditions.push(',');
                }
                conditions.push_str("true");
            }
            conditions.push_str(") { keep; }");
            let _ = SieveScript::parse(&conditions);

            // Test various string formats
            let tests = [
                format!(r#"redirect "{}";"#, s),
                format!(r#"set "var" "{}";"#, s),
                format!(r#"vacation :subject "{}" "message";"#, s),
                format!(r#"if header :is "From" "{}" {{ keep; }}"#, s),
                format!(r#"if size :over {}K {{ discard; }}"#, s.len()),
            ];

            for test in &tests {
                let _ = SieveScript::parse(test);
            }

            // Test comments
            let with_comments = format!("# Comment\nkeep; # {}", s);
            let _ = SieveScript::parse(&with_comments);
        }

        // Test deeply nested tests
        if s.len() < 20 {
            let deep = String::from("if not not not not not not not not not not true { keep; }");
            let _ = SieveScript::parse(&deep);
        }
    }

    // Test invalid UTF-8
    let s = String::from_utf8_lossy(data);
    let _ = SieveScript::parse(&s);

    // Test binary data
    if data.len() < 256 {
        let binary = String::from_utf8_lossy(data).to_string();
        let script = format!("fileinto \"{}\";", binary);
        let _ = SieveScript::parse(&script);
    }
});

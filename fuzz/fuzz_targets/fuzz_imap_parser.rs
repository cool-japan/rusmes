#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Test valid UTF-8 IMAP commands
    if let Ok(s) = std::str::from_utf8(data) {
        // Test basic command parsing
        let _ = parse_imap_line(s);

        // Test literal syntax {size}
        if s.len() < 100 {
            let with_literal = format!("A001 APPEND INBOX {{{}}} {}", s.len(), s);
            let _ = parse_imap_line(&with_literal);

            // Test literal+ syntax {size+}
            let with_literal_plus = format!("A001 APPEND INBOX {{{}+}} {}", s.len(), s);
            let _ = parse_imap_line(&with_literal_plus);
        }

        // Test quoted strings with escapes
        let quoted = format!(r#"A001 LOGIN "user" "{}""#, s.replace('"', r#"\""#));
        let _ = parse_imap_line(&quoted);

        // Test parenthesized lists
        let parens = format!("A001 FETCH 1:* ({})", s);
        let _ = parse_imap_line(&parens);

        // Test deeply nested parentheses (potential stack overflow)
        if s.len() < 50 {
            let mut nested = String::new();
            for _ in 0..20 {
                nested.push('(');
            }
            nested.push_str(s);
            for _ in 0..20 {
                nested.push(')');
            }
            let nested_cmd = format!("A001 FETCH 1 {}", nested);
            let _ = parse_imap_line(&nested_cmd);
        }
    }

    // Test invalid UTF-8
    let s = String::from_utf8_lossy(data);
    let _ = parse_imap_line(&s);
});

fn parse_imap_line(input: &str) -> Result<(), ()> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(());
    }

    // Tag + Command
    let _tag = parts[0];
    let command = parts[1].to_uppercase();

    match command.as_str() {
        "CAPABILITY" | "LOGIN" | "SELECT" | "EXAMINE" | "CREATE" | "DELETE" |
        "RENAME" | "SUBSCRIBE" | "UNSUBSCRIBE" | "LIST" | "LSUB" | "STATUS" |
        "APPEND" | "CHECK" | "CLOSE" | "EXPUNGE" | "SEARCH" | "FETCH" |
        "STORE" | "COPY" | "UID" | "LOGOUT" | "IDLE" | "NAMESPACE" => Ok(()),
        _ => Err(()),
    }
}

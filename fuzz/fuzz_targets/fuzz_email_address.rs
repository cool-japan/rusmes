#![no_main]

use libfuzzer_sys::fuzz_target;
use rusmes_proto::{MailAddress, Domain};

fuzz_target!(|data: &[u8]| {
    // Test valid UTF-8 email addresses
    if let Ok(s) = std::str::from_utf8(data) {
        // Test basic email address parsing
        let _ = s.parse::<MailAddress>();

        // Test with various local part formats
        if s.len() < 32 {
            // Quoted local part
            let quoted = format!(r#""{}@test"@example.com"#, s.replace('"', ""));
            let _ = quoted.parse::<MailAddress>();

            // Local part with dots
            let with_dots = format!("{}.{}@example.com", s.replace('@', ""), s.replace('@', ""));
            let _ = with_dots.parse::<MailAddress>();

            // Local part with plus
            let with_plus = format!("{}+tag@example.com", s.replace('@', ""));
            let _ = with_plus.parse::<MailAddress>();

            // Local part with underscore
            let with_underscore = format!("{}_test@example.com", s.replace('@', ""));
            let _ = with_underscore.parse::<MailAddress>();

            // Local part with hyphen
            let with_hyphen = format!("{}-test@example.com", s.replace('@', ""));
            let _ = with_hyphen.parse::<MailAddress>();
        }

        // Test domain parsing
        let _ = s.parse::<Domain>();

        // Test international domains (IDN)
        if s.len() < 32 {
            let idn = format!("user@{}.example.com", s.replace('@', "").replace('.', "-"));
            let _ = idn.parse::<MailAddress>();
        }

        // Test edge cases
        let edge_cases = [
            format!("{}@", s),                    // Missing domain
            format!("@{}", s),                    // Missing local part
            format!("{}@@{}", s, s),              // Double @
            format!("{}@.{}", s, s),              // Domain starts with dot
            format!("{}@{}.", s, s),              // Domain ends with dot
            format!("..{}@{}", s, s),             // Local part starts with dots
            format!("{}..@{}", s, s),             // Consecutive dots in local
        ];

        for case in &edge_cases {
            let _ = case.parse::<MailAddress>();
        }

        // Test very long local parts (>64 chars limit)
        let long_local = s.repeat(10);
        let long_addr = format!("{}@example.com", long_local);
        let _ = long_addr.parse::<MailAddress>();

        // Test very long domains (>255 chars limit)
        let long_domain = s.repeat(50);
        let long_dom_addr = format!("user@{}.com", long_domain);
        let _ = long_dom_addr.parse::<MailAddress>();
    }

    // Test invalid UTF-8
    let s = String::from_utf8_lossy(data);
    let _ = s.parse::<MailAddress>();
});

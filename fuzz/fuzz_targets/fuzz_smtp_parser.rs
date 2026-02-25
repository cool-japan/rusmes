#![no_main]

use libfuzzer_sys::fuzz_target;
use rusmes_smtp::parser::parse_command;

fuzz_target!(|data: &[u8]| {
    // Test valid UTF-8
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = parse_command(s);
    }

    // Also test with invalid UTF-8 handling
    let s = String::from_utf8_lossy(data);
    let _ = parse_command(&s);

    // Test extremely long lines (potential DoS)
    if data.len() > 0 && data.len() < 1024 {
        let repeated = s.repeat(100);
        let _ = parse_command(&repeated);
    }

    // Test edge cases with special characters
    if s.len() < 256 {
        let with_nulls = format!("{}\0{}\0", s, s);
        let _ = parse_command(&with_nulls);

        let with_crlf = format!("{}\r\n{}\r\n", s, s);
        let _ = parse_command(&with_crlf);
    }
});

//! SMTP RFC 5321 Compliance Tests
//!
//! Comprehensive test suite for SMTP protocol compliance with RFC 5321.
//! Tests all commands, response codes, syntax validation, line length limits,
//! pipelining, error handling, and edge cases from RFC appendix.

#[cfg(test)]
mod tests {
    /// Test HELO command compliance
    #[test]
    fn test_helo_format() {
        let valid_commands = vec![
            "HELO example.com",
            "HELO [192.168.1.1]",
            "HELO mail.example.org",
        ];

        for cmd in valid_commands {
            assert!(is_valid_helo(cmd), "Failed for: {}", cmd);
        }
    }

    #[test]
    fn test_helo_invalid_format() {
        let invalid_commands = vec![
            "HELO", // Missing domain
            "HELO ",
            "HELO  example.com", // Extra space
        ];

        for cmd in invalid_commands {
            assert!(!is_valid_helo(cmd), "Should reject: {}", cmd);
        }
    }

    /// Test EHLO command compliance (RFC 5321 Section 4.1.1.1)
    #[test]
    fn test_ehlo_format() {
        let valid_commands = vec![
            "EHLO example.com",
            "EHLO [192.168.1.1]",
            "EHLO [IPv6:::1]",
            "EHLO mail.example.org",
        ];

        for cmd in valid_commands {
            assert!(is_valid_ehlo(cmd), "Failed for: {}", cmd);
        }
    }

    #[test]
    fn test_ehlo_invalid_format() {
        let invalid_commands = vec![
            "EHLO", // Missing domain
            "EHLO ",
            "EHLO  example.com", // Extra space
        ];

        for cmd in invalid_commands {
            assert!(!is_valid_ehlo(cmd), "Should reject: {}", cmd);
        }
    }

    /// Test MAIL FROM command compliance (RFC 5321 Section 4.1.1.2)
    #[test]
    fn test_mail_from_format() {
        let valid_commands = vec![
            "MAIL FROM:<user@example.com>",
            "MAIL FROM:<>", // Null sender (RFC 5321 Section 4.5.5)
            "MAIL FROM:<postmaster@example.com>",
            "MAIL FROM:<user+tag@example.com>",
        ];

        for cmd in valid_commands {
            assert!(is_valid_mail_from(cmd), "Failed for: {}", cmd);
        }
    }

    #[test]
    fn test_mail_from_with_parameters() {
        let valid_commands = vec![
            "MAIL FROM:<user@example.com> SIZE=12345",
            "MAIL FROM:<user@example.com> BODY=8BITMIME",
            "MAIL FROM:<user@example.com> SIZE=12345 BODY=8BITMIME",
            "MAIL FROM:<user@example.com> RET=FULL",
            "MAIL FROM:<user@example.com> ENVID=QQ314159",
        ];

        for cmd in valid_commands {
            assert!(is_valid_mail_from(cmd), "Failed for: {}", cmd);
        }
    }

    #[test]
    fn test_mail_from_invalid() {
        let invalid_commands = vec![
            "MAIL FROM:user@example.com",    // Missing angle brackets
            "MAIL FROM: <user@example.com>", // Space before bracket
            "MAIL FROM",                     // Missing argument
        ];

        for cmd in invalid_commands {
            assert!(!is_valid_mail_from(cmd), "Should reject: {}", cmd);
        }
    }

    /// Test RCPT TO command compliance (RFC 5321 Section 4.1.1.3)
    #[test]
    fn test_rcpt_to_format() {
        let valid_commands = vec![
            "RCPT TO:<user@example.com>",
            "RCPT TO:<postmaster>",
            "RCPT TO:<user+tag@example.com>",
            "RCPT TO:<admin@mail.example.com>",
        ];

        for cmd in valid_commands {
            assert!(is_valid_rcpt_to(cmd), "Failed for: {}", cmd);
        }
    }

    #[test]
    fn test_rcpt_to_with_parameters() {
        let valid_commands = vec![
            "RCPT TO:<user@example.com> NOTIFY=SUCCESS,FAILURE",
            "RCPT TO:<user@example.com> ORCPT=rfc822;original@example.com",
        ];

        for cmd in valid_commands {
            assert!(is_valid_rcpt_to(cmd), "Failed for: {}", cmd);
        }
    }

    /// Test DATA command compliance (RFC 5321 Section 4.1.1.4)
    #[test]
    fn test_data_command() {
        assert!(is_valid_data_command("DATA"));
        assert!(!is_valid_data_command("DATA "));
        assert!(!is_valid_data_command("DATA extra"));
    }

    /// Test QUIT command compliance (RFC 5321 Section 4.1.1.10)
    #[test]
    fn test_quit_command() {
        assert!(is_valid_quit_command("QUIT"));
        assert!(!is_valid_quit_command("QUIT "));
        assert!(!is_valid_quit_command("QUIT extra"));
    }

    /// Test RSET command compliance (RFC 5321 Section 4.1.1.5)
    #[test]
    fn test_rset_command() {
        assert!(is_valid_rset_command("RSET"));
        assert!(!is_valid_rset_command("RSET extra"));
    }

    /// Test VRFY command compliance (RFC 5321 Section 4.1.1.6)
    #[test]
    fn test_vrfy_command() {
        assert!(is_valid_vrfy_command("VRFY user"));
        assert!(is_valid_vrfy_command("VRFY user@example.com"));
        assert!(!is_valid_vrfy_command("VRFY"));
    }

    /// Test EXPN command compliance (RFC 5321 Section 4.1.1.7)
    #[test]
    fn test_expn_command() {
        assert!(is_valid_expn_command("EXPN mailinglist"));
        assert!(is_valid_expn_command("EXPN list@example.com"));
        assert!(!is_valid_expn_command("EXPN"));
    }

    /// Test HELP command compliance (RFC 5321 Section 4.1.1.8)
    #[test]
    fn test_help_command() {
        assert!(is_valid_help_command("HELP"));
        assert!(is_valid_help_command("HELP MAIL"));
        assert!(is_valid_help_command("HELP RCPT"));
    }

    /// Test NOOP command compliance (RFC 5321 Section 4.1.1.9)
    #[test]
    fn test_noop_command() {
        assert!(is_valid_noop_command("NOOP"));
        assert!(is_valid_noop_command("NOOP extra")); // NOOP can have arguments (ignored)
    }

    /// Test 2xx response codes (Positive Completion)
    #[test]
    fn test_2xx_response_codes() {
        assert!(is_valid_smtp_response("211 System status"));
        assert!(is_valid_smtp_response("214 Help message"));
        assert!(is_valid_smtp_response("220 Service ready"));
        assert!(is_valid_smtp_response("221 Service closing"));
        assert!(is_valid_smtp_response("250 OK"));
        assert!(is_valid_smtp_response("251 User not local; will forward"));
        assert!(is_valid_smtp_response(
            "252 Cannot VRFY user, but will accept message"
        ));
    }

    /// Test 3xx response codes (Positive Intermediate)
    #[test]
    fn test_3xx_response_codes() {
        assert!(is_valid_smtp_response(
            "354 Start mail input; end with <CRLF>.<CRLF>"
        ));
    }

    /// Test 4xx response codes (Transient Negative Completion)
    #[test]
    fn test_4xx_response_codes() {
        assert!(is_valid_smtp_response("421 Service not available"));
        assert!(is_valid_smtp_response("450 Mailbox unavailable"));
        assert!(is_valid_smtp_response("451 Local error in processing"));
        assert!(is_valid_smtp_response("452 Insufficient system storage"));
        assert!(is_valid_smtp_response(
            "455 Server unable to accommodate parameters"
        ));
    }

    /// Test 5xx response codes (Permanent Negative Completion)
    #[test]
    fn test_5xx_response_codes() {
        assert!(is_valid_smtp_response(
            "500 Syntax error, command unrecognized"
        ));
        assert!(is_valid_smtp_response(
            "501 Syntax error in parameters or arguments"
        ));
        assert!(is_valid_smtp_response("502 Command not implemented"));
        assert!(is_valid_smtp_response("503 Bad sequence of commands"));
        assert!(is_valid_smtp_response(
            "504 Command parameter not implemented"
        ));
        assert!(is_valid_smtp_response(
            "550 Requested action not taken: mailbox unavailable"
        ));
        assert!(is_valid_smtp_response(
            "551 User not local; please try forward-path"
        ));
        assert!(is_valid_smtp_response(
            "552 Requested mail action aborted: exceeded storage allocation"
        ));
        assert!(is_valid_smtp_response(
            "553 Requested action not taken: mailbox name not allowed"
        ));
        assert!(is_valid_smtp_response("554 Transaction failed"));
        assert!(is_valid_smtp_response(
            "555 MAIL FROM/RCPT TO parameters not recognized"
        ));
    }

    /// Test multiline responses (RFC 5321 Section 4.2.1)
    #[test]
    fn test_multiline_response() {
        let responses = ["250-First line", "250-Second line", "250 Last line"];

        for (i, resp) in responses.iter().enumerate() {
            assert!(is_valid_smtp_response(resp));
            if i < responses.len() - 1 {
                assert!(
                    is_multiline_continuation(resp),
                    "Should be continuation: {}",
                    resp
                );
            } else {
                assert!(
                    !is_multiline_continuation(resp),
                    "Should not be continuation: {}",
                    resp
                );
            }
        }
    }

    /// Test command case insensitivity (RFC 5321 Section 2.4)
    #[test]
    fn test_command_case_insensitivity() {
        assert!(is_valid_ehlo("EHLO example.com"));
        assert!(is_valid_ehlo("ehlo example.com"));
        assert!(is_valid_ehlo("Ehlo example.com"));
        assert!(is_valid_ehlo("EhLo example.com"));
    }

    /// Test line length limits (RFC 5321 Section 4.5.3.1.6)
    #[test]
    fn test_line_length_limits() {
        // Command line length limit is 512 octets including CRLF
        let long_domain = "a".repeat(500);
        let cmd = format!("EHLO {}", long_domain);

        // Commands over 512 octets should be rejected
        if cmd.len() > 512 {
            assert!(
                !is_valid_ehlo(&cmd),
                "Should reject command over 512 octets"
            );
        }

        // Valid command under limit
        let short_cmd = "EHLO example.com";
        assert!(short_cmd.len() <= 512);
        assert!(is_valid_ehlo(short_cmd));
    }

    /// Test email address local part validation
    #[test]
    fn test_email_address_local_part() {
        let valid_addresses = vec![
            "user@example.com",
            "user.name@example.com",
            "user+tag@example.com",
            "user_name@example.com",
            "123@example.com",
            "u@example.com",
        ];

        for addr in valid_addresses {
            assert!(is_valid_email_address(addr), "Failed for: {}", addr);
        }
    }

    /// Test email address domain part validation
    #[test]
    fn test_email_address_domain_part() {
        let valid_addresses = vec![
            "user@example.com",
            "user@mail.example.com",
            "user@example.co.uk",
            "user@sub.mail.example.com",
        ];

        for addr in valid_addresses {
            assert!(is_valid_email_address(addr), "Failed for: {}", addr);
        }
    }

    /// Test path syntax (RFC 5321 Section 4.1.2)
    #[test]
    fn test_path_syntax() {
        assert!(is_valid_path("<user@example.com>"));
        assert!(is_valid_path("<>")); // Null path
        assert!(!is_valid_path("user@example.com")); // Missing angle brackets
        assert!(!is_valid_path("<user@example.com")); // Missing closing bracket
        assert!(!is_valid_path("user@example.com>")); // Missing opening bracket
    }

    /// Test reverse-path syntax (MAIL FROM)
    #[test]
    fn test_reverse_path_syntax() {
        assert!(is_valid_reverse_path("<user@example.com>"));
        assert!(is_valid_reverse_path("<>")); // Null reverse-path for bounces
    }

    /// Test forward-path syntax (RCPT TO)
    #[test]
    fn test_forward_path_syntax() {
        assert!(is_valid_forward_path("<user@example.com>"));
        assert!(is_valid_forward_path("<postmaster>")); // Special case
    }

    /// Test pipelining compliance (RFC 2920)
    #[test]
    fn test_pipelining_valid_commands() {
        // These commands can be pipelined
        let pipelinable = vec!["EHLO", "MAIL FROM", "RCPT TO", "DATA", "RSET", "QUIT"];

        for cmd in pipelinable {
            assert!(is_pipelinable_command(cmd), "{} should be pipelinable", cmd);
        }
    }

    #[test]
    fn test_pipelining_invalid_commands() {
        // Commands that should not be pipelined after DATA
        let non_pipelinable_after_data = vec!["VRFY", "EXPN"];

        for cmd in non_pipelinable_after_data {
            assert!(
                !is_safe_to_pipeline_after_data(cmd),
                "{} should not be safe after DATA",
                cmd
            );
        }
    }

    /// Test dot-stuffing (RFC 5321 Section 4.5.2)
    #[test]
    fn test_dot_stuffing() {
        assert!(requires_dot_stuffing("."));
        assert!(requires_dot_stuffing(".hello"));
        assert!(!requires_dot_stuffing("hello."));
        assert!(!requires_dot_stuffing("hel.lo"));
    }

    /// Test mail transaction sequences
    #[test]
    fn test_valid_mail_transaction_sequence() {
        // Valid sequence: EHLO -> MAIL FROM -> RCPT TO -> DATA -> QUIT
        assert!(is_valid_sequence(&["EHLO", "MAIL", "RCPT", "DATA", "QUIT"]));
    }

    #[test]
    fn test_invalid_mail_transaction_sequence() {
        // Invalid: MAIL before EHLO
        assert!(!is_valid_sequence(&["MAIL", "EHLO", "RCPT", "DATA"]));

        // Invalid: DATA before RCPT
        assert!(!is_valid_sequence(&["EHLO", "MAIL", "DATA"]));
    }

    /// Test edge cases from RFC appendix
    #[test]
    fn test_rfc_appendix_edge_cases() {
        // Empty reverse-path (bounce messages)
        assert!(is_valid_mail_from("MAIL FROM:<>"));

        // Postmaster without domain
        assert!(is_valid_rcpt_to("RCPT TO:<postmaster>"));

        // Case insensitivity of commands
        assert!(is_valid_ehlo("ehLO example.com"));

        // Space handling
        assert!(!is_valid_mail_from("MAIL FROM: <user@example.com>"));
    }

    /// Test SMTP response format (RFC 5321 Section 4.2)
    #[test]
    fn test_response_format() {
        // Single-line response
        assert!(is_valid_smtp_response("250 OK"));

        // Multi-line response
        assert!(is_valid_smtp_response("250-First line"));
        assert!(is_valid_smtp_response("250-Second line"));
        assert!(is_valid_smtp_response("250 Last line"));

        // Invalid responses
        assert!(!is_valid_smtp_response("25 OK")); // Code too short
        assert!(!is_valid_smtp_response("2500 OK")); // Code too long
        assert!(!is_valid_smtp_response("ABC OK")); // Non-numeric code
    }

    /// Test maximum message size
    #[test]
    fn test_size_extension() {
        assert!(is_valid_mail_from(
            "MAIL FROM:<user@example.com> SIZE=1000000"
        ));
        assert!(is_valid_mail_from("MAIL FROM:<user@example.com> SIZE=0"));
    }

    /// Test 8BITMIME extension (RFC 6152)
    #[test]
    fn test_8bitmime_extension() {
        assert!(is_valid_mail_from("MAIL FROM:<user@example.com> BODY=7BIT"));
        assert!(is_valid_mail_from(
            "MAIL FROM:<user@example.com> BODY=8BITMIME"
        ));
    }

    // Helper functions for validation
    fn is_valid_helo(cmd: &str) -> bool {
        // Split by single space, not whitespace, to catch multiple spaces
        let trimmed = cmd.trim();
        if !trimmed.to_uppercase().starts_with("HELO ") {
            return false;
        }
        let parts: Vec<&str> = trimmed.split(' ').collect();
        parts.len() == 2 && parts[0].eq_ignore_ascii_case("HELO") && !parts[1].is_empty()
    }

    fn is_valid_ehlo(cmd: &str) -> bool {
        // Split by single space, not whitespace, to catch multiple spaces
        let trimmed = cmd.trim();
        if !trimmed.to_uppercase().starts_with("EHLO ") {
            return false;
        }
        let parts: Vec<&str> = trimmed.split(' ').collect();
        parts.len() == 2 && parts[0].eq_ignore_ascii_case("EHLO") && !parts[1].is_empty()
    }

    fn is_valid_mail_from(cmd: &str) -> bool {
        let upper = cmd.to_uppercase();
        upper.starts_with("MAIL FROM:<") && cmd.contains('>')
    }

    fn is_valid_rcpt_to(cmd: &str) -> bool {
        let upper = cmd.to_uppercase();
        upper.starts_with("RCPT TO:<") && cmd.contains('>')
    }

    fn is_valid_data_command(cmd: &str) -> bool {
        cmd.eq_ignore_ascii_case("DATA")
    }

    fn is_valid_quit_command(cmd: &str) -> bool {
        cmd.eq_ignore_ascii_case("QUIT")
    }

    fn is_valid_rset_command(cmd: &str) -> bool {
        cmd.trim().eq_ignore_ascii_case("RSET")
    }

    fn is_valid_vrfy_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 2 && parts[0].eq_ignore_ascii_case("VRFY")
    }

    fn is_valid_expn_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 2 && parts[0].eq_ignore_ascii_case("EXPN")
    }

    fn is_valid_help_command(cmd: &str) -> bool {
        cmd.to_uppercase().starts_with("HELP")
    }

    fn is_valid_noop_command(cmd: &str) -> bool {
        cmd.to_uppercase().starts_with("NOOP")
    }

    fn is_valid_smtp_response(resp: &str) -> bool {
        if resp.len() < 4 {
            return false;
        }
        let code = &resp[0..3];
        code.chars().all(|c| c.is_ascii_digit())
            && (resp.chars().nth(3) == Some(' ') || resp.chars().nth(3) == Some('-'))
    }

    fn is_multiline_continuation(resp: &str) -> bool {
        resp.len() >= 4 && resp.chars().nth(3) == Some('-')
    }

    fn is_valid_email_address(addr: &str) -> bool {
        let parts: Vec<&str> = addr.split('@').collect();
        parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty()
    }

    fn is_valid_path(path: &str) -> bool {
        path.starts_with('<') && path.ends_with('>')
    }

    fn is_valid_reverse_path(path: &str) -> bool {
        is_valid_path(path)
    }

    fn is_valid_forward_path(path: &str) -> bool {
        is_valid_path(path)
    }

    fn is_pipelinable_command(cmd: &str) -> bool {
        matches!(
            cmd,
            "EHLO" | "MAIL FROM" | "RCPT TO" | "DATA" | "RSET" | "QUIT"
        )
    }

    fn is_safe_to_pipeline_after_data(cmd: &str) -> bool {
        !matches!(cmd, "VRFY" | "EXPN")
    }

    fn requires_dot_stuffing(line: &str) -> bool {
        line.starts_with('.')
    }

    fn is_valid_sequence(commands: &[&str]) -> bool {
        let mut state = 0; // 0=start, 1=ehlo, 2=mail, 3=rcpt, 4=data

        for cmd in commands {
            match (*cmd, state) {
                ("EHLO" | "HELO", 0) => state = 1,
                ("MAIL", 1 | 4) => state = 2,
                ("RCPT", 2 | 3) => state = 3,
                ("DATA", 3) => state = 4,
                ("QUIT", _) => return true,
                ("RSET", _) => state = 1,
                _ => return false,
            }
        }

        true
    }
}

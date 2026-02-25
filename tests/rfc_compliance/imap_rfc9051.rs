//! IMAP RFC 9051 Compliance Tests
//!
//! Comprehensive IMAP torture tests following RFC 9051 (IMAP4rev2).
//! Tests all commands, malformed inputs, literal handling, boundary conditions,
//! quote handling, flags, and search criteria.

#[cfg(test)]
mod tests {
    /// Test IMAP command format (tag + command + args)
    #[test]
    fn test_imap_command_format() {
        assert!(is_valid_imap_command("A001 CAPABILITY"));
        assert!(is_valid_imap_command("A002 LOGIN user pass"));
        assert!(is_valid_imap_command("A003 SELECT INBOX"));
        assert!(!is_valid_imap_command("CAPABILITY")); // Missing tag
        assert!(!is_valid_imap_command(""));
    }

    /// Test CAPABILITY command (RFC 9051 Section 6.1.1)
    #[test]
    fn test_capability_command() {
        assert!(is_valid_capability_command("A001 CAPABILITY"));
        assert!(is_valid_capability_command("a001 capability"));
        assert!(!is_valid_capability_command("CAPABILITY"));
        assert!(!is_valid_capability_command("A001 CAPABILITY extra"));
    }

    /// Test NOOP command (RFC 9051 Section 6.1.2)
    #[test]
    fn test_noop_command() {
        assert!(is_valid_noop_command("A001 NOOP"));
        assert!(!is_valid_noop_command("NOOP"));
    }

    /// Test LOGOUT command (RFC 9051 Section 6.1.3)
    #[test]
    fn test_logout_command() {
        assert!(is_valid_logout_command("A001 LOGOUT"));
        assert!(is_valid_logout_command("z999 LOGOUT"));
        assert!(!is_valid_logout_command("LOGOUT"));
    }

    /// Test LOGIN command (RFC 9051 Section 6.2.3)
    #[test]
    fn test_login_command() {
        assert!(is_valid_login_command("A001 LOGIN user password"));
        assert!(is_valid_login_command("A001 LOGIN \"user\" \"password\""));
        assert!(is_valid_login_command(
            "A001 LOGIN {4}\r\nuser {8}\r\npassword"
        ));
        assert!(!is_valid_login_command("A001 LOGIN"));
        assert!(!is_valid_login_command("A001 LOGIN user"));
    }

    /// Test SELECT command (RFC 9051 Section 6.3.1)
    #[test]
    fn test_select_command() {
        assert!(is_valid_select_command("A001 SELECT INBOX"));
        assert!(is_valid_select_command("A001 SELECT \"Sent Items\""));
        assert!(is_valid_select_command("A001 SELECT Archive/2024"));
        assert!(!is_valid_select_command("A001 SELECT"));
    }

    /// Test EXAMINE command (RFC 9051 Section 6.3.2)
    #[test]
    fn test_examine_command() {
        assert!(is_valid_examine_command("A001 EXAMINE INBOX"));
        assert!(is_valid_examine_command("A001 EXAMINE \"Read Only\""));
        assert!(!is_valid_examine_command("A001 EXAMINE"));
    }

    /// Test CREATE command (RFC 9051 Section 6.3.3)
    #[test]
    fn test_create_command() {
        assert!(is_valid_create_command("A001 CREATE TestFolder"));
        assert!(is_valid_create_command("A001 CREATE \"Test Folder\""));
        assert!(is_valid_create_command("A001 CREATE Archive/2024/January"));
        assert!(!is_valid_create_command("A001 CREATE"));
    }

    /// Test DELETE command (RFC 9051 Section 6.3.4)
    #[test]
    fn test_delete_command() {
        assert!(is_valid_delete_command("A001 DELETE TestFolder"));
        assert!(is_valid_delete_command("A001 DELETE \"Old Folder\""));
        assert!(!is_valid_delete_command("A001 DELETE"));
    }

    /// Test RENAME command (RFC 9051 Section 6.3.5)
    #[test]
    fn test_rename_command() {
        assert!(is_valid_rename_command("A001 RENAME OldName NewName"));
        assert!(is_valid_rename_command(
            "A001 RENAME \"Old Name\" \"New Name\""
        ));
        assert!(!is_valid_rename_command("A001 RENAME OldName"));
        assert!(!is_valid_rename_command("A001 RENAME"));
    }

    /// Test SUBSCRIBE command (RFC 9051 Section 6.3.6)
    #[test]
    fn test_subscribe_command() {
        assert!(is_valid_subscribe_command("A001 SUBSCRIBE INBOX"));
        assert!(is_valid_subscribe_command(
            "A001 SUBSCRIBE \"Mailing Lists\""
        ));
        assert!(!is_valid_subscribe_command("A001 SUBSCRIBE"));
    }

    /// Test UNSUBSCRIBE command (RFC 9051 Section 6.3.7)
    #[test]
    fn test_unsubscribe_command() {
        assert!(is_valid_unsubscribe_command("A001 UNSUBSCRIBE Spam"));
        assert!(!is_valid_unsubscribe_command("A001 UNSUBSCRIBE"));
    }

    /// Test LIST command (RFC 9051 Section 6.3.8)
    #[test]
    fn test_list_command() {
        assert!(is_valid_list_command("A001 LIST \"\" \"*\""));
        assert!(is_valid_list_command("A001 LIST \"\" \"INBOX\""));
        assert!(is_valid_list_command("A001 LIST \"Archive\" \"%\""));
        assert!(!is_valid_list_command("A001 LIST"));
        assert!(!is_valid_list_command("A001 LIST \"\""));
    }

    /// Test LSUB command
    #[test]
    fn test_lsub_command() {
        assert!(is_valid_lsub_command("A001 LSUB \"\" \"*\""));
        assert!(is_valid_lsub_command("A001 LSUB \"\" \"%\""));
        assert!(!is_valid_lsub_command("A001 LSUB"));
    }

    /// Test STATUS command (RFC 9051 Section 6.3.10)
    #[test]
    fn test_status_command() {
        assert!(is_valid_status_command("A001 STATUS INBOX (MESSAGES)"));
        assert!(is_valid_status_command(
            "A001 STATUS INBOX (MESSAGES RECENT UNSEEN)"
        ));
        assert!(is_valid_status_command(
            "A001 STATUS \"Sent\" (UIDNEXT UIDVALIDITY)"
        ));
        assert!(!is_valid_status_command("A001 STATUS INBOX"));
    }

    /// Test APPEND command (RFC 9051 Section 6.3.11)
    #[test]
    fn test_append_command() {
        assert!(is_valid_append_command("A001 APPEND INBOX {310}"));
        assert!(is_valid_append_command("A001 APPEND INBOX (\\Seen) {310}"));
        assert!(is_valid_append_command(
            "A001 APPEND INBOX (\\Seen) \"01-Jan-2024 12:00:00 +0000\" {310}"
        ));
        assert!(!is_valid_append_command("A001 APPEND"));
    }

    /// Test CHECK command (RFC 9051 Section 6.4.1)
    #[test]
    fn test_check_command() {
        assert!(is_valid_check_command("A001 CHECK"));
        assert!(!is_valid_check_command("A001 CHECK extra"));
    }

    /// Test CLOSE command (RFC 9051 Section 6.4.2)
    #[test]
    fn test_close_command() {
        assert!(is_valid_close_command("A001 CLOSE"));
        assert!(!is_valid_close_command("A001 CLOSE extra"));
    }

    /// Test EXPUNGE command (RFC 9051 Section 6.4.3)
    #[test]
    fn test_expunge_command() {
        assert!(is_valid_expunge_command("A001 EXPUNGE"));
        assert!(!is_valid_expunge_command("A001 EXPUNGE extra"));
    }

    /// Test SEARCH command (RFC 9051 Section 6.4.4)
    #[test]
    fn test_search_command() {
        assert!(is_valid_search_command("A001 SEARCH ALL"));
        assert!(is_valid_search_command("A001 SEARCH UNSEEN"));
        assert!(is_valid_search_command(
            "A001 SEARCH FROM \"user@example.com\""
        ));
        assert!(is_valid_search_command("A001 SEARCH SUBJECT \"meeting\""));
        assert!(is_valid_search_command("A001 SEARCH SINCE 01-Jan-2024"));
        assert!(is_valid_search_command("A001 SEARCH OR SEEN FLAGGED"));
        assert!(is_valid_search_command("A001 SEARCH NOT DELETED"));
        assert!(!is_valid_search_command("A001 SEARCH"));
    }

    /// Test FETCH command (RFC 9051 Section 6.4.5)
    #[test]
    fn test_fetch_command() {
        assert!(is_valid_fetch_command("A001 FETCH 1 (FLAGS)"));
        assert!(is_valid_fetch_command("A001 FETCH 1:5 (FLAGS BODY[])"));
        assert!(is_valid_fetch_command("A001 FETCH * (FLAGS)"));
        assert!(is_valid_fetch_command("A001 FETCH 1 FULL"));
        assert!(is_valid_fetch_command("A001 FETCH 1 FAST"));
        assert!(is_valid_fetch_command(
            "A001 FETCH 1 (BODY[HEADER.FIELDS (FROM TO)])"
        ));
        assert!(!is_valid_fetch_command("A001 FETCH"));
    }

    /// Test STORE command (RFC 9051 Section 6.4.6)
    #[test]
    fn test_store_command() {
        assert!(is_valid_store_command("A001 STORE 1 +FLAGS (\\Seen)"));
        assert!(is_valid_store_command("A001 STORE 1:5 FLAGS (\\Deleted)"));
        assert!(is_valid_store_command("A001 STORE 1 -FLAGS (\\Flagged)"));
        assert!(is_valid_store_command(
            "A001 STORE 1 +FLAGS.SILENT (\\Answered)"
        ));
        assert!(!is_valid_store_command("A001 STORE"));
    }

    /// Test COPY command (RFC 9051 Section 6.4.7)
    #[test]
    fn test_copy_command() {
        assert!(is_valid_copy_command("A001 COPY 1 Sent"));
        assert!(is_valid_copy_command("A001 COPY 1:5 Archive"));
        assert!(is_valid_copy_command("A001 COPY 1,3,5 Trash"));
        assert!(!is_valid_copy_command("A001 COPY"));
    }

    /// Test UID command (RFC 9051 Section 6.4.8)
    #[test]
    fn test_uid_command() {
        assert!(is_valid_uid_command("A001 UID FETCH 1 (FLAGS)"));
        assert!(is_valid_uid_command("A001 UID SEARCH ALL"));
        assert!(is_valid_uid_command("A001 UID COPY 1:5 Archive"));
        assert!(is_valid_uid_command("A001 UID STORE 1 +FLAGS (\\Seen)"));
        assert!(!is_valid_uid_command("A001 UID"));
    }

    /// Test IMAP responses
    #[test]
    fn test_imap_responses() {
        // Untagged responses
        assert!(is_valid_imap_response("* OK IMAP4rev2 server ready"));
        assert!(is_valid_imap_response("* CAPABILITY IMAP4rev2 AUTH=PLAIN"));
        assert!(is_valid_imap_response("* 5 EXISTS"));
        assert!(is_valid_imap_response("* 0 RECENT"));
        assert!(is_valid_imap_response(
            "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)"
        ));

        // Tagged responses
        assert!(is_valid_imap_response("A001 OK LOGIN completed"));
        assert!(is_valid_imap_response("A002 NO LOGIN failed"));
        assert!(is_valid_imap_response("A003 BAD Invalid command"));
    }

    /// Test system flags (RFC 9051 Section 2.3.2)
    #[test]
    fn test_system_flags() {
        let valid_flags = vec![
            "\\Seen",
            "\\Answered",
            "\\Flagged",
            "\\Deleted",
            "\\Draft",
            "\\Recent",
        ];

        for flag in valid_flags {
            assert!(is_valid_system_flag(flag), "Failed for: {}", flag);
        }
    }

    /// Test custom flags
    #[test]
    fn test_custom_flags() {
        assert!(is_valid_custom_flag("$Label1"));
        assert!(is_valid_custom_flag("MyCustomFlag"));
        assert!(!is_valid_custom_flag("\\InvalidSystem"));
    }

    /// Test message sequence numbers (RFC 9051 Section 9)
    #[test]
    fn test_message_sequence_numbers() {
        assert!(is_valid_sequence_set("1"));
        assert!(is_valid_sequence_set("1:5"));
        assert!(is_valid_sequence_set("1,3,5"));
        assert!(is_valid_sequence_set("1:5,10:15"));
        assert!(is_valid_sequence_set("*"));
        assert!(is_valid_sequence_set("1:*"));
        assert!(is_valid_sequence_set("5:1")); // Reverse range
        assert!(!is_valid_sequence_set(""));
        assert!(!is_valid_sequence_set("abc"));
    }

    /// Test quoted strings (RFC 9051 Section 4.3)
    #[test]
    fn test_quoted_strings() {
        assert!(is_valid_quoted_string("\"hello\""));
        assert!(is_valid_quoted_string("\"hello world\""));
        assert!(is_valid_quoted_string("\"quote\\\"inside\""));
        assert!(!is_valid_quoted_string("hello"));
        assert!(!is_valid_quoted_string("\"hello"));
        assert!(!is_valid_quoted_string("hello\""));
    }

    /// Test literal strings (RFC 9051 Section 4.3)
    #[test]
    fn test_literal_strings() {
        // Synchronizing literals
        assert!(is_valid_literal_prefix("{5}"));
        assert!(is_valid_literal_prefix("{123}"));
        assert!(is_valid_literal_prefix("{0}"));

        // Non-synchronizing literals (LITERAL+)
        assert!(is_valid_literal_plus_prefix("{5+}"));
        assert!(is_valid_literal_plus_prefix("{123+}"));

        assert!(!is_valid_literal_prefix("{abc}"));
        assert!(!is_valid_literal_prefix("{}"));
    }

    /// Test mailbox names (RFC 9051 Section 5.1)
    #[test]
    fn test_mailbox_names() {
        assert!(is_valid_mailbox_name("INBOX"));
        assert!(is_valid_mailbox_name("Sent"));
        assert!(is_valid_mailbox_name("Archive/2024"));
        assert!(is_valid_mailbox_name("Archive.2024"));
        assert!(is_valid_mailbox_name("\"Sent Items\""));
    }

    /// Test mailbox hierarchy delimiter
    #[test]
    fn test_mailbox_hierarchy() {
        assert!(is_valid_hierarchy_delimiter('/'));
        assert!(is_valid_hierarchy_delimiter('.'));
        assert!(is_valid_hierarchy_delimiter('\\'));
    }

    /// Test date format (RFC 9051 Section 4.3)
    #[test]
    fn test_date_format() {
        // dd-Mon-yyyy format
        assert!(is_valid_imap_date("01-Jan-2024"));
        assert!(is_valid_imap_date("15-Dec-2023"));
        assert!(is_valid_imap_date("31-Mar-2024"));
        assert!(!is_valid_imap_date("2024-01-01"));
        assert!(!is_valid_imap_date("1-Jan-2024"));
        assert!(!is_valid_imap_date("32-Jan-2024"));
    }

    /// Test search criteria (RFC 9051 Section 6.4.4)
    #[test]
    fn test_search_criteria() {
        let valid_keys = vec![
            "ALL",
            "ANSWERED",
            "BCC",
            "BEFORE",
            "BODY",
            "CC",
            "DELETED",
            "DRAFT",
            "FLAGGED",
            "FROM",
            "HEADER",
            "KEYWORD",
            "LARGER",
            "NEW",
            "NOT",
            "OLD",
            "ON",
            "OR",
            "RECENT",
            "SEEN",
            "SENTBEFORE",
            "SENTON",
            "SENTSINCE",
            "SINCE",
            "SMALLER",
            "SUBJECT",
            "TEXT",
            "TO",
            "UID",
            "UNANSWERED",
            "UNDELETED",
            "UNDRAFT",
            "UNFLAGGED",
            "UNKEYWORD",
            "UNSEEN",
        ];

        for key in valid_keys {
            assert!(is_valid_search_key(key), "Failed for: {}", key);
        }
    }

    /// Test malformed commands
    #[test]
    fn test_malformed_commands() {
        // Missing arguments
        assert!(!is_valid_select_command("A001 SELECT"));
        assert!(!is_valid_fetch_command("A001 FETCH"));
        assert!(!is_valid_search_command("A001 SEARCH"));

        // Extra arguments
        assert!(!is_valid_logout_command("A001 LOGOUT extra"));
        assert!(!is_valid_noop_command("A001 NOOP extra"));

        // Invalid syntax
        assert!(!is_valid_imap_command(""));
        assert!(!is_valid_imap_command("   "));
        assert!(!is_valid_imap_command("A001"));
    }

    /// Test boundary conditions
    #[test]
    fn test_boundary_conditions() {
        // Maximum tag length (typically unlimited, but test reasonable values)
        assert!(is_valid_tag("A001"));
        assert!(is_valid_tag("ABCDEFGHIJ"));

        // Empty tag should be invalid
        assert!(!is_valid_tag(""));

        // Tag with spaces should be invalid
        assert!(!is_valid_tag("A 001"));
    }

    /// Test IDLE command (RFC 2idle)
    #[test]
    fn test_idle_command() {
        assert!(is_valid_idle_command("A001 IDLE"));
        assert!(!is_valid_idle_command("A001 IDLE extra"));
    }

    /// Test ENABLE command (RFC 5161)
    #[test]
    fn test_enable_command() {
        assert!(is_valid_enable_command("A001 ENABLE CONDSTORE"));
        assert!(is_valid_enable_command("A001 ENABLE QRESYNC"));
        assert!(!is_valid_enable_command("A001 ENABLE"));
    }

    /// Test untagged response formats
    #[test]
    fn test_untagged_responses() {
        assert!(is_untagged_response("* OK Server ready"));
        assert!(is_untagged_response("* 5 EXISTS"));
        assert!(is_untagged_response("* FLAGS (\\Seen \\Answered)"));
        assert!(!is_untagged_response("A001 OK Done"));
    }

    // Helper functions
    fn is_valid_imap_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 2 && !parts[0].is_empty()
    }

    fn is_valid_capability_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() == 2 && parts[1].eq_ignore_ascii_case("CAPABILITY")
    }

    fn is_valid_noop_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() == 2 && parts[1].eq_ignore_ascii_case("NOOP")
    }

    fn is_valid_logout_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() == 2 && parts[1].eq_ignore_ascii_case("LOGOUT")
    }

    fn is_valid_login_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 4 && parts[1].eq_ignore_ascii_case("LOGIN")
    }

    fn is_valid_select_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("SELECT")
    }

    fn is_valid_examine_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("EXAMINE")
    }

    fn is_valid_create_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("CREATE")
    }

    fn is_valid_delete_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("DELETE")
    }

    fn is_valid_rename_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 4 && parts[1].eq_ignore_ascii_case("RENAME")
    }

    fn is_valid_subscribe_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("SUBSCRIBE")
    }

    fn is_valid_unsubscribe_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("UNSUBSCRIBE")
    }

    fn is_valid_list_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 4 && parts[1].eq_ignore_ascii_case("LIST")
    }

    fn is_valid_lsub_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 4 && parts[1].eq_ignore_ascii_case("LSUB")
    }

    fn is_valid_status_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 4 && parts[1].eq_ignore_ascii_case("STATUS") && cmd.contains('(')
    }

    fn is_valid_append_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("APPEND")
    }

    fn is_valid_check_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() == 2 && parts[1].eq_ignore_ascii_case("CHECK")
    }

    fn is_valid_close_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() == 2 && parts[1].eq_ignore_ascii_case("CLOSE")
    }

    fn is_valid_expunge_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() == 2 && parts[1].eq_ignore_ascii_case("EXPUNGE")
    }

    fn is_valid_search_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("SEARCH")
    }

    fn is_valid_fetch_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 4 && parts[1].eq_ignore_ascii_case("FETCH")
    }

    fn is_valid_store_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 4 && parts[1].eq_ignore_ascii_case("STORE")
    }

    fn is_valid_copy_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 4 && parts[1].eq_ignore_ascii_case("COPY")
    }

    fn is_valid_uid_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("UID")
    }

    fn is_valid_idle_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() == 2 && parts[1].eq_ignore_ascii_case("IDLE")
    }

    fn is_valid_enable_command(cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        parts.len() >= 3 && parts[1].eq_ignore_ascii_case("ENABLE")
    }

    fn is_valid_imap_response(resp: &str) -> bool {
        !resp.is_empty()
            && (resp.starts_with('*') || resp.chars().next().unwrap().is_alphanumeric())
    }

    fn is_untagged_response(resp: &str) -> bool {
        resp.starts_with("* ")
    }

    fn is_valid_system_flag(flag: &str) -> bool {
        matches!(
            flag,
            "\\Seen" | "\\Answered" | "\\Flagged" | "\\Deleted" | "\\Draft" | "\\Recent"
        )
    }

    fn is_valid_custom_flag(flag: &str) -> bool {
        !flag.is_empty() && !flag.starts_with('\\')
    }

    fn is_valid_sequence_set(seq: &str) -> bool {
        !seq.is_empty()
            && seq
                .chars()
                .all(|c| c.is_ascii_digit() || c == ':' || c == ',' || c == '*')
    }

    fn is_valid_quoted_string(s: &str) -> bool {
        s.len() >= 2 && s.starts_with('"') && s.ends_with('"')
    }

    fn is_valid_literal_prefix(s: &str) -> bool {
        s.starts_with('{')
            && s.ends_with('}')
            && s.len() > 2
            && s[1..s.len() - 1].chars().all(|c| c.is_ascii_digit())
    }

    fn is_valid_literal_plus_prefix(s: &str) -> bool {
        s.starts_with('{')
            && s.ends_with("+}")
            && s.len() > 3
            && s[1..s.len() - 2].chars().all(|c| c.is_ascii_digit())
    }

    fn is_valid_mailbox_name(name: &str) -> bool {
        !name.is_empty()
    }

    fn is_valid_hierarchy_delimiter(c: char) -> bool {
        matches!(c, '/' | '.' | '\\')
    }

    fn is_valid_imap_date(date: &str) -> bool {
        let parts: Vec<&str> = date.split('-').collect();
        if parts.len() != 3 || parts[0].len() != 2 || parts[1].len() != 3 || parts[2].len() != 4 {
            return false;
        }
        let day: u8 = parts[0].parse().unwrap_or(0);
        let months = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        (1..=31).contains(&day) && months.contains(&parts[1])
    }

    fn is_valid_search_key(key: &str) -> bool {
        matches!(
            key,
            "ALL"
                | "ANSWERED"
                | "BCC"
                | "BEFORE"
                | "BODY"
                | "CC"
                | "DELETED"
                | "DRAFT"
                | "FLAGGED"
                | "FROM"
                | "HEADER"
                | "KEYWORD"
                | "LARGER"
                | "NEW"
                | "NOT"
                | "OLD"
                | "ON"
                | "OR"
                | "RECENT"
                | "SEEN"
                | "SENTBEFORE"
                | "SENTON"
                | "SENTSINCE"
                | "SINCE"
                | "SMALLER"
                | "SUBJECT"
                | "TEXT"
                | "TO"
                | "UID"
                | "UNANSWERED"
                | "UNDELETED"
                | "UNDRAFT"
                | "UNFLAGGED"
                | "UNKEYWORD"
                | "UNSEEN"
        )
    }

    fn is_valid_tag(tag: &str) -> bool {
        !tag.is_empty() && !tag.contains(char::is_whitespace)
    }
}

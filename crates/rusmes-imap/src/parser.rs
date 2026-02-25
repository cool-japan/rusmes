//! IMAP command parser
//!
//! This module provides parsing functionality for IMAP commands, including support
//! for RFC 7888 LITERAL+ extension for non-synchronizing literals.
//!
//! # LITERAL+ Extension (RFC 7888)
//!
//! The LITERAL+ extension allows clients to send literal data without waiting for
//! server continuation. This significantly improves APPEND performance by reducing
//! round-trips.
//!
//! ## Traditional Synchronizing Literals
//!
//! ```text
//! C: A001 APPEND INBOX {310}
//! S: + Ready for literal data
//! C: <310 bytes of message data>
//! S: A001 OK APPEND completed
//! ```
//!
//! ## Non-Synchronizing Literals (LITERAL+)
//!
//! ```text
//! C: A001 APPEND INBOX {310+}
//! C: <310 bytes of message data>
//! S: A001 OK APPEND completed
//! ```
//!
//! The parser automatically detects both literal types:
//! - `{size}` - Synchronizing literal (requires server continuation)
//! - `{size+}` - Non-synchronizing literal (no continuation needed)

use crate::command::{ImapCommand, UidSubcommand};
use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while1},
    character::complete::space1,
    IResult, Parser,
};

/// Type of literal in IMAP command
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralType {
    /// Synchronizing literal {size} - requires server continuation
    Synchronizing,
    /// Non-synchronizing literal {size+} (RFC 7888) - no server continuation needed
    NonSynchronizing,
}

/// Parse an IMAP command
pub fn parse_command(input: &str) -> Result<(String, ImapCommand), String> {
    // IMAP commands are: tag COMMAND args
    // Example: A001 LOGIN user password

    let parts: Vec<&str> = input.splitn(3, ' ').collect();
    if parts.is_empty() {
        return Err("Empty command".to_string());
    }

    let tag = parts[0].to_string();

    if parts.len() < 2 {
        return Err("No command specified".to_string());
    }

    let cmd_line = if parts.len() == 3 {
        format!("{} {}", parts[1], parts[2])
    } else {
        parts[1].to_string()
    };

    let (_rest, command) = parse_imap_command(&cmd_line).map_err(|e| e.to_string())?;

    Ok((tag, command))
}

/// Parse APPEND command with literal data
/// This is separate because it needs access to the full input including literal data
pub fn parse_append_command(
    input: &str,
    literal_data: Vec<u8>,
) -> Result<(String, ImapCommand), String> {
    // Format: tag APPEND mailbox [flags] [date-time] {size}
    let parts: Vec<&str> = input.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return Err("Invalid APPEND command".to_string());
    }

    let tag = parts[0].to_string();

    // Parse the rest after APPEND
    let args = parts[2];
    let (mailbox, flags, date_time) = parse_append_args(args)?;

    Ok((
        tag,
        ImapCommand::Append {
            mailbox,
            flags,
            date_time,
            message_literal: literal_data,
        },
    ))
}

/// Parse APPEND arguments (mailbox, optional flags, optional date-time)
fn parse_append_args(input: &str) -> Result<(String, Vec<String>, Option<String>), String> {
    let mut parts: Vec<String> = Vec::new();
    let mut in_quotes = false;
    let mut in_parens = false;
    let mut current = String::new();

    for c in input.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            '(' if !in_quotes => {
                in_parens = true;
                current.push(c);
            }
            ')' if !in_quotes => {
                in_parens = false;
                current.push(c);
            }
            ' ' if !in_quotes && !in_parens => {
                if !current.is_empty() {
                    parts.push(current.clone());
                    current.clear();
                }
            }
            '{' if !in_quotes => {
                // Start of literal, stop here
                if !current.is_empty() {
                    parts.push(current.clone());
                }
                break;
            }
            _ => current.push(c),
        }
    }

    if !current.is_empty() && !current.starts_with('{') {
        parts.push(current.clone());
    }

    if parts.is_empty() {
        return Err("Missing mailbox name".to_string());
    }

    // First part is always the mailbox name
    let mailbox = parts[0].trim_matches('"').to_string();
    let mut flags = Vec::new();
    let mut date_time = None;

    // Parse remaining optional parts
    let mut i = 1;
    while i < parts.len() {
        let part = &parts[i];
        if part.starts_with('(') && part.ends_with(')') {
            // Flags list
            let flags_str = &part[1..part.len() - 1];
            flags = flags_str
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
        } else if part.starts_with('"') {
            // Date-time string
            date_time = Some(part.trim_matches('"').to_string());
        }
        i += 1;
    }

    Ok((mailbox, flags, date_time))
}

/// Check if command line contains a literal and return its size and type
///
/// Returns `Some((size, LiteralType))` if a literal is found:
/// - {size} -> Synchronizing literal (traditional, requires server continuation)
/// - {size+} -> Non-synchronizing literal (RFC 7888 LITERAL+, no continuation needed)
///
/// # Examples
/// ```
/// use rusmes_imap::parser::{has_literal, LiteralType};
///
/// let sync = has_literal("A001 APPEND INBOX {100}");
/// assert_eq!(sync, Some((100, LiteralType::Synchronizing)));
///
/// let non_sync = has_literal("A001 APPEND INBOX {100+}");
/// assert_eq!(non_sync, Some((100, LiteralType::NonSynchronizing)));
/// ```
pub fn has_literal(input: &str) -> Option<(usize, LiteralType)> {
    // Look for {size} or {size+} pattern at the end of the line
    if let Some(start) = input.rfind('{') {
        if let Some(end) = input[start..].find('}') {
            let size_str = &input[start + 1..start + end];

            // Reject if empty or starts with invalid characters
            if size_str.is_empty() || size_str.starts_with('+') || size_str.starts_with('-') {
                return None;
            }

            // Check for non-synchronizing literal (ends with +)
            if let Some(stripped) = size_str.strip_suffix('+') {
                if let Ok(size) = stripped.parse::<usize>() {
                    return Some((size, LiteralType::NonSynchronizing));
                }
            } else {
                // Traditional synchronizing literal
                if let Ok(size) = size_str.parse::<usize>() {
                    return Some((size, LiteralType::Synchronizing));
                }
            }
        }
    }
    None
}

/// Legacy function for backward compatibility
/// Returns just the size of the literal, ignoring the type
#[allow(dead_code)]
pub fn get_literal_size(input: &str) -> Option<usize> {
    has_literal(input).map(|(size, _)| size)
}

fn parse_imap_command(input: &str) -> IResult<&str, ImapCommand> {
    // Split into two groups to work around nom's 21-alternative limit
    alt((parse_imap_command_group1, parse_imap_command_group2)).parse(input)
}

fn parse_imap_command_group1(input: &str) -> IResult<&str, ImapCommand> {
    alt((
        parse_uid,
        parse_login,
        parse_authenticate,
        parse_select,
        parse_examine,
        parse_fetch,
        parse_store,
        parse_search,
        parse_list,
        parse_lsub,
        parse_subscribe,
        parse_unsubscribe,
    ))
    .parse(input)
}

fn parse_imap_command_group2(input: &str) -> IResult<&str, ImapCommand> {
    alt((
        parse_create_special_use,
        parse_create,
        parse_delete,
        parse_rename,
        parse_copy,
        parse_move,
        parse_expunge,
        parse_close,
        parse_capability,
        parse_logout,
        parse_noop,
        parse_idle,
        parse_namespace,
    ))
    .parse(input)
}

fn parse_idle(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("IDLE").parse(input)?;
    Ok((input, ImapCommand::Idle))
}

fn parse_namespace(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("NAMESPACE").parse(input)?;
    Ok((input, ImapCommand::Namespace))
}

fn parse_login(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("LOGIN").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, user) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, password) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;

    Ok((
        input,
        ImapCommand::Login {
            user: user.to_string(),
            password: password.to_string(),
        },
    ))
}

fn parse_authenticate(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("AUTHENTICATE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mechanism) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;

    // Check for optional initial response (SASL-IR, RFC 4959)
    let (input, initial_response) =
        if let Ok((remaining, _)) = space1::<_, nom::error::Error<&str>>(input) {
            let (remaining, response) = nom::combinator::rest(remaining)?;
            (remaining, Some(response.trim().to_string()))
        } else {
            (input, None)
        };

    Ok((
        input,
        ImapCommand::Authenticate {
            mechanism: mechanism.to_uppercase(),
            initial_response,
        },
    ))
}

fn parse_select(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("SELECT").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;

    Ok((
        input,
        ImapCommand::Select {
            mailbox: mailbox.to_string(),
        },
    ))
}

fn parse_examine(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("EXAMINE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;

    Ok((
        input,
        ImapCommand::Examine {
            mailbox: mailbox.to_string(),
        },
    ))
}

fn parse_fetch(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("FETCH").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, sequence) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, items_str) = nom::combinator::rest(input)?;

    // Parse items (simplified)
    let items: Vec<String> = items_str
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    Ok((
        input,
        ImapCommand::Fetch {
            sequence: sequence.to_string(),
            items,
        },
    ))
}

fn parse_list(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("LIST").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, reference) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;

    Ok((
        input,
        ImapCommand::List {
            reference: reference.to_string(),
            mailbox: mailbox.to_string(),
        },
    ))
}

fn parse_create_special_use(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("CREATE-SPECIAL-USE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, special_use) = nom::combinator::rest(input)?;

    Ok((
        input,
        ImapCommand::CreateSpecialUse {
            mailbox: mailbox.trim().to_string(),
            special_use: special_use.trim().to_string(),
        },
    ))
}

fn parse_create(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("CREATE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = nom::combinator::rest(input)?;

    Ok((
        input,
        ImapCommand::Create {
            mailbox: mailbox.trim().to_string(),
        },
    ))
}

fn parse_delete(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("DELETE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = nom::combinator::rest(input)?;

    Ok((
        input,
        ImapCommand::Delete {
            mailbox: mailbox.trim().to_string(),
        },
    ))
}

fn parse_rename(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("RENAME").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, old) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, new) = nom::combinator::rest(input)?;

    Ok((
        input,
        ImapCommand::Rename {
            old: old.trim_matches('"').to_string(),
            new: new.trim().trim_matches('"').to_string(),
        },
    ))
}

fn parse_logout(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("LOGOUT").parse(input)?;
    Ok((input, ImapCommand::Logout))
}

fn parse_noop(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("NOOP").parse(input)?;
    Ok((input, ImapCommand::Noop))
}

fn parse_store(input: &str) -> IResult<&str, ImapCommand> {
    use crate::command::StoreMode;

    let (input, _) = tag_no_case("STORE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, sequence) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mode_str) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;

    // Determine store mode
    let mode = if mode_str.eq_ignore_ascii_case("FLAGS") {
        StoreMode::Replace
    } else if mode_str.eq_ignore_ascii_case("+FLAGS") {
        StoreMode::Add
    } else if mode_str.eq_ignore_ascii_case("-FLAGS") {
        StoreMode::Remove
    } else {
        // Default to replace
        StoreMode::Replace
    };

    // Parse flags - rest of the line
    let (input, _) = space1(input)?;
    let (input, flags_str) = nom::combinator::rest(input)?;

    // Parse flags (simplified - handle parenthesized list or single flag)
    let flags_str = flags_str.trim();
    let flags: Vec<String> = if flags_str.starts_with('(') && flags_str.ends_with(')') {
        // Parenthesized list
        flags_str[1..flags_str.len() - 1]
            .split_whitespace()
            .map(|s| s.to_string())
            .collect()
    } else {
        // Single flag
        vec![flags_str.to_string()]
    };

    Ok((
        input,
        ImapCommand::Store {
            sequence: sequence.to_string(),
            mode,
            flags,
        },
    ))
}

fn parse_search(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("SEARCH").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, criteria_str) = nom::combinator::rest(input)?;

    // Parse search criteria (simplified - just split by whitespace for now)
    let criteria: Vec<String> = criteria_str
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    Ok((input, ImapCommand::Search { criteria }))
}

fn parse_capability(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("CAPABILITY").parse(input)?;
    Ok((input, ImapCommand::Capability))
}

fn parse_copy(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("COPY").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, sequence) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = nom::combinator::rest(input)?;

    Ok((
        input,
        ImapCommand::Copy {
            sequence: sequence.to_string(),
            mailbox: mailbox.trim().trim_matches('"').to_string(),
        },
    ))
}

fn parse_move(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("MOVE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, sequence) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = nom::combinator::rest(input)?;

    Ok((
        input,
        ImapCommand::Move {
            sequence: sequence.to_string(),
            mailbox: mailbox.trim().trim_matches('"').to_string(),
        },
    ))
}

fn parse_lsub(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("LSUB").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, reference) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;

    Ok((
        input,
        ImapCommand::Lsub {
            reference: reference.to_string(),
            mailbox: mailbox.to_string(),
        },
    ))
}

fn parse_subscribe(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("SUBSCRIBE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = nom::combinator::rest(input)?;

    Ok((
        input,
        ImapCommand::Subscribe {
            mailbox: mailbox.trim().to_string(),
        },
    ))
}

fn parse_unsubscribe(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("UNSUBSCRIBE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = nom::combinator::rest(input)?;

    Ok((
        input,
        ImapCommand::Unsubscribe {
            mailbox: mailbox.trim().to_string(),
        },
    ))
}

fn parse_expunge(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("EXPUNGE").parse(input)?;
    Ok((input, ImapCommand::Expunge))
}

fn parse_close(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("CLOSE").parse(input)?;
    Ok((input, ImapCommand::Close))
}

fn parse_uid(input: &str) -> IResult<&str, ImapCommand> {
    let (input, _) = tag_no_case("UID").parse(input)?;
    let (input, _) = space1(input)?;

    // Parse the subcommand
    let (input, subcommand) = alt((
        parse_uid_fetch,
        parse_uid_store,
        parse_uid_search,
        parse_uid_copy,
        parse_uid_move,
        parse_uid_expunge,
    ))
    .parse(input)?;

    Ok((
        input,
        ImapCommand::Uid {
            subcommand: Box::new(subcommand),
        },
    ))
}

fn parse_uid_fetch(input: &str) -> IResult<&str, crate::command::UidSubcommand> {
    let (input, _) = tag_no_case("FETCH").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, sequence) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, items_str) = nom::combinator::rest(input)?;

    // Parse items (simplified)
    let items: Vec<String> = items_str
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    Ok((
        input,
        UidSubcommand::Fetch {
            sequence: sequence.to_string(),
            items,
        },
    ))
}

fn parse_uid_store(input: &str) -> IResult<&str, crate::command::UidSubcommand> {
    use crate::command::{StoreMode, UidSubcommand};

    let (input, _) = tag_no_case("STORE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, sequence) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mode_str) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;

    // Determine store mode
    let mode = if mode_str.eq_ignore_ascii_case("FLAGS") {
        StoreMode::Replace
    } else if mode_str.eq_ignore_ascii_case("+FLAGS") {
        StoreMode::Add
    } else if mode_str.eq_ignore_ascii_case("-FLAGS") {
        StoreMode::Remove
    } else {
        StoreMode::Replace
    };

    // Parse flags - rest of the line
    let (input, _) = space1(input)?;
    let (input, flags_str) = nom::combinator::rest(input)?;

    // Parse flags (simplified - handle parenthesized list or single flag)
    let flags_str = flags_str.trim();
    let flags: Vec<String> = if flags_str.starts_with('(') && flags_str.ends_with(')') {
        // Parenthesized list
        flags_str[1..flags_str.len() - 1]
            .split_whitespace()
            .map(|s| s.to_string())
            .collect()
    } else {
        // Single flag
        vec![flags_str.to_string()]
    };

    Ok((
        input,
        UidSubcommand::Store {
            sequence: sequence.to_string(),
            mode,
            flags,
        },
    ))
}

fn parse_uid_search(input: &str) -> IResult<&str, crate::command::UidSubcommand> {
    use crate::command::UidSubcommand;

    let (input, _) = tag_no_case("SEARCH").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, criteria_str) = nom::combinator::rest(input)?;

    // Parse search criteria (simplified - just split by whitespace for now)
    let criteria: Vec<String> = criteria_str
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    Ok((input, UidSubcommand::Search { criteria }))
}

fn parse_uid_copy(input: &str) -> IResult<&str, crate::command::UidSubcommand> {
    use crate::command::UidSubcommand;

    let (input, _) = tag_no_case("COPY").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, sequence) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = nom::combinator::rest(input)?;

    Ok((
        input,
        UidSubcommand::Copy {
            sequence: sequence.to_string(),
            mailbox: mailbox.trim().trim_matches('"').to_string(),
        },
    ))
}

fn parse_uid_move(input: &str) -> IResult<&str, crate::command::UidSubcommand> {
    use crate::command::UidSubcommand;

    let (input, _) = tag_no_case("MOVE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, sequence) = take_while1(|c: char| !c.is_whitespace()).parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mailbox) = nom::combinator::rest(input)?;

    Ok((
        input,
        UidSubcommand::Move {
            sequence: sequence.to_string(),
            mailbox: mailbox.trim().trim_matches('"').to_string(),
        },
    ))
}

fn parse_uid_expunge(input: &str) -> IResult<&str, crate::command::UidSubcommand> {
    use crate::command::UidSubcommand;

    let (input, _) = tag_no_case("EXPUNGE").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, sequence) = nom::combinator::rest(input)?;

    Ok((
        input,
        UidSubcommand::Expunge {
            sequence: sequence.trim().to_string(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_login() {
        let (tag, cmd) =
            parse_command("A001 LOGIN user password").expect("LOGIN command parse should succeed");
        assert_eq!(tag, "A001");
        match cmd {
            ImapCommand::Login { user, password } => {
                assert_eq!(user, "user");
                assert_eq!(password, "password");
            }
            _ => panic!("Expected Login command"),
        }
    }

    #[test]
    fn test_parse_select() {
        let (tag, cmd) =
            parse_command("A002 SELECT INBOX").expect("SELECT INBOX parse should succeed");
        assert_eq!(tag, "A002");
        match cmd {
            ImapCommand::Select { mailbox } => {
                assert_eq!(mailbox, "INBOX");
            }
            _ => panic!("Expected Select command"),
        }
    }

    #[test]
    fn test_parse_logout() {
        let (tag, cmd) = parse_command("A003 LOGOUT").expect("LOGOUT parse should succeed");
        assert_eq!(tag, "A003");
        assert!(matches!(cmd, ImapCommand::Logout));
    }

    // LITERAL+ (RFC 7888) Tests
    #[test]
    fn test_has_literal_synchronizing() {
        // Traditional synchronizing literal {size}
        let result = has_literal("A001 APPEND INBOX {100}");
        assert_eq!(result, Some((100, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_non_synchronizing() {
        // Non-synchronizing literal {size+} - RFC 7888
        let result = has_literal("A001 APPEND INBOX {100+}");
        assert_eq!(result, Some((100, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_with_flags_synchronizing() {
        let result = has_literal("A001 APPEND INBOX (\\Seen \\Draft) {250}");
        assert_eq!(result, Some((250, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_with_flags_non_synchronizing() {
        let result = has_literal("A001 APPEND INBOX (\\Seen \\Draft) {250+}");
        assert_eq!(result, Some((250, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_with_date_synchronizing() {
        let result = has_literal("A001 APPEND INBOX \"7-Feb-1994 21:52:25 -0800\" {1024}");
        assert_eq!(result, Some((1024, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_with_date_non_synchronizing() {
        let result = has_literal("A001 APPEND INBOX \"7-Feb-1994 21:52:25 -0800\" {1024+}");
        assert_eq!(result, Some((1024, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_complete_append_synchronizing() {
        let result = has_literal("A001 APPEND INBOX (\\Seen) \"7-Feb-1994 21:52:25 -0800\" {5000}");
        assert_eq!(result, Some((5000, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_complete_append_non_synchronizing() {
        let result =
            has_literal("A001 APPEND INBOX (\\Seen) \"7-Feb-1994 21:52:25 -0800\" {5000+}");
        assert_eq!(result, Some((5000, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_no_literal() {
        // No literal in command
        let result = has_literal("A001 SELECT INBOX");
        assert_eq!(result, None);
    }

    #[test]
    fn test_has_literal_invalid_format() {
        // Invalid literal format
        let result = has_literal("A001 APPEND INBOX {abc}");
        assert_eq!(result, None);
    }

    #[test]
    fn test_has_literal_invalid_format_plus() {
        // Invalid literal format with plus
        let result = has_literal("A001 APPEND INBOX {abc+}");
        assert_eq!(result, None);
    }

    #[test]
    fn test_has_literal_zero_size_synchronizing() {
        // Edge case: zero-size literal
        let result = has_literal("A001 APPEND INBOX {0}");
        assert_eq!(result, Some((0, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_zero_size_non_synchronizing() {
        // Edge case: zero-size non-synchronizing literal
        let result = has_literal("A001 APPEND INBOX {0+}");
        assert_eq!(result, Some((0, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_large_size_synchronizing() {
        // Large literal size
        let result = has_literal("A001 APPEND INBOX {999999999}");
        assert_eq!(result, Some((999999999, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_large_size_non_synchronizing() {
        // Large non-synchronizing literal size
        let result = has_literal("A001 APPEND INBOX {999999999+}");
        assert_eq!(result, Some((999999999, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_unclosed_brace() {
        // Unclosed brace
        let result = has_literal("A001 APPEND INBOX {100");
        assert_eq!(result, None);
    }

    #[test]
    fn test_has_literal_multiple_literals_takes_last() {
        // If multiple literal patterns exist, rfind ensures we get the last one
        let result = has_literal("A001 {50} APPEND {100}");
        assert_eq!(result, Some((100, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_empty_braces() {
        // Empty braces
        let result = has_literal("A001 APPEND INBOX {}");
        assert_eq!(result, None);
    }

    #[test]
    fn test_has_literal_plus_only() {
        // Just a plus sign
        let result = has_literal("A001 APPEND INBOX {+}");
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_literal_size_synchronizing() {
        // Legacy function test - synchronizing
        let result = get_literal_size("A001 APPEND INBOX {100}");
        assert_eq!(result, Some(100));
    }

    #[test]
    fn test_get_literal_size_non_synchronizing() {
        // Legacy function test - non-synchronizing
        let result = get_literal_size("A001 APPEND INBOX {100+}");
        assert_eq!(result, Some(100));
    }

    // Additional comprehensive LITERAL+ tests
    #[test]
    fn test_has_literal_mixed_case_append() {
        // Test case insensitivity
        let result = has_literal("a001 append inbox {50+}");
        assert_eq!(result, Some((50, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_with_special_chars_in_mailbox_name() {
        // Mailbox with special characters
        let result = has_literal("A001 APPEND \"Sent Items\" {128+}");
        assert_eq!(result, Some((128, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_all_flags_sync() {
        // Multiple flags with synchronizing literal
        let result = has_literal("A001 APPEND INBOX (\\Seen \\Flagged \\Draft \\Answered) {1000}");
        assert_eq!(result, Some((1000, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_all_flags_non_sync() {
        // Multiple flags with non-synchronizing literal
        let result = has_literal("A001 APPEND INBOX (\\Seen \\Flagged \\Draft \\Answered) {1000+}");
        assert_eq!(result, Some((1000, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_whitespace_before_brace_sync() {
        // Extra whitespace before literal
        let result = has_literal("A001 APPEND INBOX  {200}");
        assert_eq!(result, Some((200, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_whitespace_before_brace_non_sync() {
        // Extra whitespace before literal with LITERAL+
        let result = has_literal("A001 APPEND INBOX  {200+}");
        assert_eq!(result, Some((200, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_only_braces_no_command() {
        // Just braces with size
        let result = has_literal("{500}");
        assert_eq!(result, Some((500, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_only_braces_plus_no_command() {
        // Just braces with size and plus
        let result = has_literal("{500+}");
        assert_eq!(result, Some((500, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_date_time_rfc2822_sync() {
        // RFC 2822 style date-time with synchronizing literal
        let result = has_literal("A001 APPEND INBOX \"07-Feb-1994 21:52:25 -0800\" {2048}");
        assert_eq!(result, Some((2048, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_date_time_rfc2822_non_sync() {
        // RFC 2822 style date-time with non-synchronizing literal
        let result = has_literal("A001 APPEND INBOX \"07-Feb-1994 21:52:25 -0800\" {2048+}");
        assert_eq!(result, Some((2048, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_complete_with_custom_flags_sync() {
        // Custom flags with synchronizing literal
        let result = has_literal(
            "A001 APPEND INBOX (\\Seen $Important) \"01-Jan-2024 12:00:00 +0000\" {4096}",
        );
        assert_eq!(result, Some((4096, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_complete_with_custom_flags_non_sync() {
        // Custom flags with non-synchronizing literal
        let result = has_literal(
            "A001 APPEND INBOX (\\Seen $Important) \"01-Jan-2024 12:00:00 +0000\" {4096+}",
        );
        assert_eq!(result, Some((4096, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_single_digit_sync() {
        // Single digit size - synchronizing
        let result = has_literal("A001 APPEND INBOX {5}");
        assert_eq!(result, Some((5, LiteralType::Synchronizing)));
    }

    #[test]
    fn test_has_literal_single_digit_non_sync() {
        // Single digit size - non-synchronizing
        let result = has_literal("A001 APPEND INBOX {5+}");
        assert_eq!(result, Some((5, LiteralType::NonSynchronizing)));
    }

    #[test]
    fn test_has_literal_double_plus() {
        // Invalid: double plus
        let result = has_literal("A001 APPEND INBOX {100++}");
        assert_eq!(result, None);
    }

    #[test]
    fn test_has_literal_plus_at_start() {
        // Plus at start is invalid - explicitly rejected by the parser
        // IMAP spec requires literal size to be a non-negative decimal number
        let result = has_literal("A001 APPEND INBOX {+100}");
        assert_eq!(result, None);
    }

    #[test]
    fn test_has_literal_minus_sign() {
        // Invalid: negative size
        let result = has_literal("A001 APPEND INBOX {-100}");
        assert_eq!(result, None);
    }

    #[test]
    fn test_has_literal_with_spaces_inside() {
        // Invalid: spaces inside braces
        let result = has_literal("A001 APPEND INBOX {100 }");
        assert_eq!(result, None);
    }

    #[test]
    fn test_has_literal_scientific_notation() {
        // Invalid: scientific notation
        let result = has_literal("A001 APPEND INBOX {1e5}");
        assert_eq!(result, None);
    }

    #[test]
    fn test_has_literal_hexadecimal() {
        // Invalid: hexadecimal
        let result = has_literal("A001 APPEND INBOX {0x100}");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_append_args_basic() {
        // Basic APPEND with just mailbox
        let (mailbox, flags, date_time) =
            parse_append_args("INBOX {100}").expect("basic APPEND args parse should succeed");
        assert_eq!(mailbox, "INBOX");
        assert!(flags.is_empty());
        assert_eq!(date_time, None);
    }

    #[test]
    fn test_parse_append_args_with_flags() {
        // APPEND with flags
        let (mailbox, flags, date_time) = parse_append_args("INBOX (\\Seen \\Draft) {100}")
            .expect("APPEND args with flags parse should succeed");
        assert_eq!(mailbox, "INBOX");
        assert_eq!(flags, vec!["\\Seen", "\\Draft"]);
        assert_eq!(date_time, None);
    }

    #[test]
    fn test_parse_append_args_with_date() {
        // APPEND with date-time
        let (mailbox, flags, date_time) =
            parse_append_args("INBOX \"7-Feb-1994 21:52:25 -0800\" {100}")
                .expect("APPEND args with date-time parse should succeed");
        assert_eq!(mailbox, "INBOX");
        assert!(flags.is_empty());
        assert_eq!(date_time, Some("7-Feb-1994 21:52:25 -0800".to_string()));
    }

    #[test]
    fn test_parse_append_args_complete() {
        // APPEND with flags and date-time
        let (mailbox, flags, date_time) =
            parse_append_args("INBOX (\\Seen) \"7-Feb-1994 21:52:25 -0800\" {100}")
                .expect("complete APPEND args parse should succeed");
        assert_eq!(mailbox, "INBOX");
        assert_eq!(flags, vec!["\\Seen"]);
        assert_eq!(date_time, Some("7-Feb-1994 21:52:25 -0800".to_string()));
    }

    #[test]
    fn test_parse_append_args_quoted_mailbox() {
        // APPEND with quoted mailbox name
        let (mailbox, flags, date_time) = parse_append_args("\"Sent Items\" {100}")
            .expect("APPEND args with quoted mailbox name parse should succeed");
        assert_eq!(mailbox, "Sent Items");
        assert!(flags.is_empty());
        assert_eq!(date_time, None);
    }

    #[test]
    fn test_parse_append_command_basic() {
        // Basic APPEND command parsing
        let literal_data = b"Subject: Test\r\n\r\nHello World".to_vec();
        let (tag, cmd) = parse_append_command("A001 APPEND INBOX {30}", literal_data.clone())
            .expect("basic APPEND command parse should succeed");
        assert_eq!(tag, "A001");
        match cmd {
            ImapCommand::Append {
                mailbox,
                flags,
                date_time,
                message_literal,
            } => {
                assert_eq!(mailbox, "INBOX");
                assert!(flags.is_empty());
                assert_eq!(date_time, None);
                assert_eq!(message_literal, literal_data);
            }
            _ => panic!("Expected Append command"),
        }
    }

    #[test]
    fn test_parse_append_command_with_flags() {
        // APPEND command with flags
        let literal_data = b"Subject: Test\r\n\r\nHello".to_vec();
        let (tag, cmd) = parse_append_command(
            "A002 APPEND INBOX (\\Seen \\Flagged) {25}",
            literal_data.clone(),
        )
        .expect("APPEND command with flags parse should succeed");
        assert_eq!(tag, "A002");
        match cmd {
            ImapCommand::Append {
                mailbox,
                flags,
                date_time,
                message_literal,
            } => {
                assert_eq!(mailbox, "INBOX");
                assert_eq!(flags, vec!["\\Seen", "\\Flagged"]);
                assert_eq!(date_time, None);
                assert_eq!(message_literal, literal_data);
            }
            _ => panic!("Expected Append command"),
        }
    }

    #[test]
    fn test_parse_append_command_complete() {
        // Complete APPEND command with all parameters
        let literal_data = b"Subject: Test\r\n\r\nTest message".to_vec();
        let (tag, cmd) = parse_append_command(
            "A003 APPEND INBOX (\\Seen) \"15-Feb-2026 10:30:00 +0000\" {32}",
            literal_data.clone(),
        )
        .expect("complete APPEND command parse should succeed");
        assert_eq!(tag, "A003");
        match cmd {
            ImapCommand::Append {
                mailbox,
                flags,
                date_time,
                message_literal,
            } => {
                assert_eq!(mailbox, "INBOX");
                assert_eq!(flags, vec!["\\Seen"]);
                assert_eq!(date_time, Some("15-Feb-2026 10:30:00 +0000".to_string()));
                assert_eq!(message_literal, literal_data);
            }
            _ => panic!("Expected Append command"),
        }
    }

    #[test]
    fn test_literal_type_equality() {
        // Test LiteralType enum equality
        assert_eq!(LiteralType::Synchronizing, LiteralType::Synchronizing);
        assert_eq!(LiteralType::NonSynchronizing, LiteralType::NonSynchronizing);
        assert_ne!(LiteralType::Synchronizing, LiteralType::NonSynchronizing);
    }

    #[test]
    fn test_literal_type_clone() {
        // Test LiteralType clone
        let sync = LiteralType::Synchronizing;
        let sync_clone = sync;
        assert_eq!(sync, sync_clone);
    }
}

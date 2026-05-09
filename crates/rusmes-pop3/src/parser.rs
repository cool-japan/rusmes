//! POP3 command parser using nom

use crate::command::Pop3Command;
use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while1},
    character::complete::{digit1, space0, space1},
    combinator::{map, map_res, opt},
    sequence::preceded,
    IResult, Parser,
};

/// Parse a complete POP3 command line
pub fn parse_command(input: &str) -> Result<Pop3Command, String> {
    let input = input.trim();

    // Try to parse each command type
    if let Ok((_, cmd)) = pop3_command(input) {
        Ok(cmd)
    } else {
        Err(format!("Failed to parse command: {}", input))
    }
}

/// Parse any POP3 command
fn pop3_command(input: &str) -> IResult<&str, Pop3Command> {
    alt((
        user_command,
        pass_command,
        stat_command,
        list_command,
        retr_command,
        dele_command,
        noop_command,
        rset_command,
        quit_command,
        top_command,
        uidl_command,
        apop_command,
        capa_command,
        stls_command,
        auth_command,
    ))
    .parse(input)
}

/// Parse USER command
fn user_command(input: &str) -> IResult<&str, Pop3Command> {
    map(
        preceded(tag_no_case("USER"), preceded(space1, username)),
        Pop3Command::User,
    )
    .parse(input)
}

/// Parse PASS command
fn pass_command(input: &str) -> IResult<&str, Pop3Command> {
    map(
        preceded(tag_no_case("PASS"), preceded(space1, password)),
        Pop3Command::Pass,
    )
    .parse(input)
}

/// Parse STAT command
fn stat_command(input: &str) -> IResult<&str, Pop3Command> {
    map(tag_no_case("STAT"), |_| Pop3Command::Stat).parse(input)
}

/// Parse LIST command
fn list_command(input: &str) -> IResult<&str, Pop3Command> {
    map(
        (tag_no_case("LIST"), opt(preceded(space1, message_number))),
        |(_, msg)| Pop3Command::List(msg),
    )
    .parse(input)
}

/// Parse RETR command
fn retr_command(input: &str) -> IResult<&str, Pop3Command> {
    map(
        preceded(tag_no_case("RETR"), preceded(space1, message_number)),
        Pop3Command::Retr,
    )
    .parse(input)
}

/// Parse DELE command
fn dele_command(input: &str) -> IResult<&str, Pop3Command> {
    map(
        preceded(tag_no_case("DELE"), preceded(space1, message_number)),
        Pop3Command::Dele,
    )
    .parse(input)
}

/// Parse NOOP command
fn noop_command(input: &str) -> IResult<&str, Pop3Command> {
    map(tag_no_case("NOOP"), |_| Pop3Command::Noop).parse(input)
}

/// Parse RSET command
fn rset_command(input: &str) -> IResult<&str, Pop3Command> {
    map(tag_no_case("RSET"), |_| Pop3Command::Rset).parse(input)
}

/// Parse QUIT command
fn quit_command(input: &str) -> IResult<&str, Pop3Command> {
    map(tag_no_case("QUIT"), |_| Pop3Command::Quit).parse(input)
}

/// Parse TOP command
fn top_command(input: &str) -> IResult<&str, Pop3Command> {
    map(
        (
            tag_no_case("TOP"),
            preceded(space1, message_number),
            preceded(space1, line_count),
        ),
        |(_, msg, lines)| Pop3Command::Top { msg, lines },
    )
    .parse(input)
}

/// Parse UIDL command
fn uidl_command(input: &str) -> IResult<&str, Pop3Command> {
    map(
        (tag_no_case("UIDL"), opt(preceded(space1, message_number))),
        |(_, msg)| Pop3Command::Uidl(msg),
    )
    .parse(input)
}

/// Parse APOP command
fn apop_command(input: &str) -> IResult<&str, Pop3Command> {
    map(
        (
            tag_no_case("APOP"),
            preceded(space1, username),
            preceded(space1, digest),
        ),
        |(_, name, digest)| Pop3Command::Apop { name, digest },
    )
    .parse(input)
}

/// Parse a username (alphanumeric and some special chars)
fn username(input: &str) -> IResult<&str, String> {
    map(
        take_while1(|c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '@'),
        |s: &str| s.to_string(),
    )
    .parse(input)
}

/// Parse a password (any non-whitespace characters)
fn password(input: &str) -> IResult<&str, String> {
    map(take_while1(|c: char| !c.is_whitespace()), |s: &str| {
        s.to_string()
    })
    .parse(input)
}

/// Parse a message number (positive integer)
fn message_number(input: &str) -> IResult<&str, u32> {
    map_res(digit1, |s: &str| s.parse::<u32>()).parse(input)
}

/// Parse a line count (non-negative integer)
fn line_count(input: &str) -> IResult<&str, u32> {
    map_res(digit1, |s: &str| s.parse::<u32>()).parse(input)
}

/// Parse a digest (hexadecimal string for APOP)
fn digest(input: &str) -> IResult<&str, String> {
    map(take_while1(|c: char| c.is_ascii_hexdigit()), |s: &str| {
        s.to_string()
    })
    .parse(input)
}

/// Parse CAPA command
fn capa_command(input: &str) -> IResult<&str, Pop3Command> {
    map(tag_no_case("CAPA"), |_| Pop3Command::Capa).parse(input)
}

/// Parse STLS command
fn stls_command(input: &str) -> IResult<&str, Pop3Command> {
    map(tag_no_case("STLS"), |_| Pop3Command::Stls).parse(input)
}

/// Parse a SASL mechanism token (uppercase letters, digits, dash, underscore).
fn sasl_mechanism(input: &str) -> IResult<&str, String> {
    map(
        take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        |s: &str| s.to_string(),
    )
    .parse(input)
}

/// Parse a SASL initial-response token (base64 alphabet plus `=` padding,
/// or the special token `=` meaning "empty initial response" per RFC 5034).
fn sasl_initial_response(input: &str) -> IResult<&str, String> {
    map(
        take_while1(|c: char| {
            c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == ','
        }),
        |s: &str| s.to_string(),
    )
    .parse(input)
}

/// Parse AUTH command (RFC 1734 + RFC 5034)
///
/// Forms accepted:
/// - `AUTH`                          — request mechanism listing
/// - `AUTH MECHANISM`                — start exchange, no initial response
/// - `AUTH MECHANISM <base64-or-=>`  — start exchange with initial response
fn auth_command(input: &str) -> IResult<&str, Pop3Command> {
    map(
        (
            tag_no_case("AUTH"),
            opt(preceded(space1, sasl_mechanism)),
            opt(preceded(space1, sasl_initial_response)),
        ),
        |(_, mechanism, initial_response)| Pop3Command::Auth {
            mechanism,
            initial_response,
        },
    )
    .parse(input)
}

/// Helper to consume optional trailing whitespace
#[allow(dead_code)]
fn trailing_space(input: &str) -> IResult<&str, ()> {
    map(space0, |_| ()).parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_capa() {
        let result = parse_command("CAPA");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("CAPA parse should succeed"),
            Pop3Command::Capa
        );
    }

    #[test]
    fn test_parse_capa_case_insensitive() {
        let result = parse_command("capa");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("lowercase capa parse should succeed"),
            Pop3Command::Capa
        );

        let result = parse_command("CaPa");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("mixed-case CaPa parse should succeed"),
            Pop3Command::Capa
        );
    }

    #[test]
    fn test_parse_stls() {
        let result = parse_command("STLS");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("STLS parse should succeed"),
            Pop3Command::Stls
        );
    }

    #[test]
    fn test_parse_stls_case_insensitive() {
        let result = parse_command("stls");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("lowercase stls parse should succeed"),
            Pop3Command::Stls
        );

        let result = parse_command("StLs");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("mixed-case StLs parse should succeed"),
            Pop3Command::Stls
        );
    }

    #[test]
    fn test_parse_stls_with_whitespace() {
        let result = parse_command("  STLS  ");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("STLS with surrounding whitespace parse should succeed"),
            Pop3Command::Stls
        );
    }

    #[test]
    fn test_parse_capa_with_whitespace() {
        let result = parse_command("  CAPA  ");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("CAPA with surrounding whitespace parse should succeed"),
            Pop3Command::Capa
        );
    }

    #[test]
    fn test_parse_auth_bare() {
        // "AUTH" with no arguments — request mechanism listing.
        let result = parse_command("AUTH").expect("bare AUTH should parse");
        assert_eq!(
            result,
            Pop3Command::Auth {
                mechanism: None,
                initial_response: None,
            }
        );
    }

    #[test]
    fn test_parse_auth_mechanism_only() {
        let result = parse_command("AUTH PLAIN").expect("AUTH PLAIN should parse");
        assert_eq!(
            result,
            Pop3Command::Auth {
                mechanism: Some("PLAIN".to_string()),
                initial_response: None,
            }
        );
    }

    #[test]
    fn test_parse_auth_with_initial_response() {
        let result =
            parse_command("AUTH PLAIN AGFsaWNlAHNlY3JldA==").expect("AUTH PLAIN <ir> should parse");
        assert_eq!(
            result,
            Pop3Command::Auth {
                mechanism: Some("PLAIN".to_string()),
                initial_response: Some("AGFsaWNlAHNlY3JldA==".to_string()),
            }
        );
    }

    #[test]
    fn test_parse_auth_case_insensitive_verb() {
        let result = parse_command("auth PLAIN").expect("lowercase auth should parse");
        assert_eq!(
            result,
            Pop3Command::Auth {
                mechanism: Some("PLAIN".to_string()),
                initial_response: None,
            }
        );
    }

    #[test]
    fn test_parse_auth_scram_sha_256() {
        // Mechanism names with dashes must be accepted.
        let result = parse_command("AUTH SCRAM-SHA-256").expect("AUTH SCRAM-SHA-256 should parse");
        assert_eq!(
            result,
            Pop3Command::Auth {
                mechanism: Some("SCRAM-SHA-256".to_string()),
                initial_response: None,
            }
        );
    }

    #[test]
    fn test_parse_auth_empty_initial_response_token() {
        // RFC 5034: a single `=` is the canonical encoding of an empty IR.
        let result = parse_command("AUTH PLAIN =").expect("AUTH PLAIN = should parse");
        assert_eq!(
            result,
            Pop3Command::Auth {
                mechanism: Some("PLAIN".to_string()),
                initial_response: Some("=".to_string()),
            }
        );
    }
}

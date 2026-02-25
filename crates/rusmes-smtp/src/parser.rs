//! SMTP command parser using nom

use crate::command::{MailParam, SmtpCommand};
use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while1},
    character::complete::{char, space0, space1},
    combinator::{map, opt, rest},
    sequence::{delimited, preceded},
    IResult, Parser,
};
use rusmes_proto::MailAddress;

/// Parse a complete SMTP command line
pub fn parse_command(input: &str) -> Result<SmtpCommand, String> {
    let input = input.trim();

    // Try to parse each command type
    if let Ok((_, cmd)) = smtp_command(input) {
        Ok(cmd)
    } else {
        Err(format!("Failed to parse command: {}", input))
    }
}

/// Parse any SMTP command
fn smtp_command(input: &str) -> IResult<&str, SmtpCommand> {
    alt((
        helo_command,
        ehlo_command,
        mail_command,
        rcpt_command,
        data_command,
        bdat_command,
        rset_command,
        noop_command,
        quit_command,
        vrfy_command,
        expn_command,
        help_command,
        starttls_command,
        auth_command,
    ))
    .parse(input)
}

/// Parse HELO command
fn helo_command(input: &str) -> IResult<&str, SmtpCommand> {
    map(
        preceded(tag_no_case("HELO"), preceded(space1, domain)),
        SmtpCommand::Helo,
    )
    .parse(input)
}

/// Parse EHLO command
fn ehlo_command(input: &str) -> IResult<&str, SmtpCommand> {
    map(
        preceded(tag_no_case("EHLO"), preceded(space1, domain)),
        SmtpCommand::Ehlo,
    )
    .parse(input)
}

/// Parse MAIL FROM command
fn mail_command(input: &str) -> IResult<&str, SmtpCommand> {
    let (input, _) = tag_no_case("MAIL FROM:").parse(input)?;
    let (input, _) = space0(input)?;
    let (input, from) = reverse_path(input)?;
    let (input, params) = opt(preceded(space1, mail_parameters)).parse(input)?;

    Ok((
        input,
        SmtpCommand::Mail {
            from,
            params: params.unwrap_or_default(),
        },
    ))
}

/// Parse RCPT TO command
fn rcpt_command(input: &str) -> IResult<&str, SmtpCommand> {
    let (input, _) = tag_no_case("RCPT TO:").parse(input)?;
    let (input, _) = space0(input)?;
    let (input, to) = forward_path(input)?;
    let (input, params) = opt(preceded(space1, mail_parameters)).parse(input)?;

    Ok((
        input,
        SmtpCommand::Rcpt {
            to,
            params: params.unwrap_or_default(),
        },
    ))
}

/// Parse DATA command
fn data_command(input: &str) -> IResult<&str, SmtpCommand> {
    map(tag_no_case("DATA"), |_| SmtpCommand::Data).parse(input)
}

/// Parse BDAT command
fn bdat_command(input: &str) -> IResult<&str, SmtpCommand> {
    use nom::character::complete::digit1;

    let (input, _) = tag_no_case("BDAT").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, size_str) = digit1(input)?;
    let (input, last) = opt(preceded(space1, tag_no_case("LAST"))).parse(input)?;

    // Parse chunk size
    let chunk_size = size_str.parse::<usize>().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit))
    })?;

    Ok((
        input,
        SmtpCommand::Bdat {
            chunk_size,
            last: last.is_some(),
        },
    ))
}

/// Parse RSET command
fn rset_command(input: &str) -> IResult<&str, SmtpCommand> {
    map(tag_no_case("RSET"), |_| SmtpCommand::Rset).parse(input)
}

/// Parse NOOP command
fn noop_command(input: &str) -> IResult<&str, SmtpCommand> {
    map(tag_no_case("NOOP"), |_| SmtpCommand::Noop).parse(input)
}

/// Parse QUIT command
fn quit_command(input: &str) -> IResult<&str, SmtpCommand> {
    map(tag_no_case("QUIT"), |_| SmtpCommand::Quit).parse(input)
}

/// Parse VRFY command
fn vrfy_command(input: &str) -> IResult<&str, SmtpCommand> {
    map(
        preceded(tag_no_case("VRFY"), preceded(space1, rest)),
        |s: &str| SmtpCommand::Vrfy(s.to_string()),
    )
    .parse(input)
}

/// Parse EXPN command
fn expn_command(input: &str) -> IResult<&str, SmtpCommand> {
    map(
        preceded(tag_no_case("EXPN"), preceded(space1, rest)),
        |s: &str| SmtpCommand::Expn(s.to_string()),
    )
    .parse(input)
}

/// Parse HELP command
fn help_command(input: &str) -> IResult<&str, SmtpCommand> {
    map(
        preceded(tag_no_case("HELP"), opt(preceded(space1, rest))),
        |s: Option<&str>| SmtpCommand::Help(s.map(|x| x.to_string())),
    )
    .parse(input)
}

/// Parse STARTTLS command
fn starttls_command(input: &str) -> IResult<&str, SmtpCommand> {
    map(tag_no_case("STARTTLS"), |_| SmtpCommand::StartTls).parse(input)
}

/// Parse AUTH command
fn auth_command(input: &str) -> IResult<&str, SmtpCommand> {
    let (input, _) = tag_no_case("AUTH").parse(input)?;
    let (input, _) = space1(input)?;
    let (input, mechanism) =
        take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-').parse(input)?;
    let (input, initial_response) = opt(preceded(space1, rest)).parse(input)?;

    Ok((
        input,
        SmtpCommand::Auth {
            mechanism: mechanism.to_string(),
            initial_response: initial_response.map(|s| s.to_string()),
        },
    ))
}

/// Parse reverse-path (MAIL FROM)
fn reverse_path(input: &str) -> IResult<&str, MailAddress> {
    delimited(char('<'), mailbox, char('>')).parse(input)
}

/// Parse forward-path (RCPT TO)
fn forward_path(input: &str) -> IResult<&str, MailAddress> {
    delimited(char('<'), mailbox, char('>')).parse(input)
}

/// Parse mailbox (email address)
fn mailbox(input: &str) -> IResult<&str, MailAddress> {
    let (input, addr_str) = take_while1(|c: char| {
        c.is_ascii_alphanumeric() || c == '@' || c == '.' || c == '-' || c == '_' || c == '+'
    })
    .parse(input)?;

    // Parse the address string
    match addr_str.parse::<MailAddress>() {
        Ok(addr) => Ok((input, addr)),
        Err(_) => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
    }
}

/// Parse domain name
fn domain(input: &str) -> IResult<&str, String> {
    map(
        take_while1(|c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-'),
        |s: &str| s.to_string(),
    )
    .parse(input)
}

/// Parse mail parameters (ESMTP)
fn mail_parameters(input: &str) -> IResult<&str, Vec<MailParam>> {
    let mut params = Vec::new();
    let mut remaining = input;

    while let Ok((rest, param)) = mail_parameter(remaining) {
        params.push(param);
        remaining = rest;

        // Skip any spaces before checking for more parameters
        remaining = remaining.trim_start();

        // If we have more content, continue parsing
        if remaining.is_empty() {
            break;
        }
    }

    Ok((remaining, params))
}

/// Parse a single mail parameter
fn mail_parameter(input: &str) -> IResult<&str, MailParam> {
    let (input, keyword) =
        take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-').parse(input)?;
    let (input, value) = opt(preceded(char('='), parameter_value)).parse(input)?;

    Ok((
        input,
        MailParam::new(keyword.to_string(), value.map(|s| s.to_string())),
    ))
}

/// Parse parameter value
fn parameter_value(input: &str) -> IResult<&str, String> {
    map(
        take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '.'),
        |s: &str| s.to_string(),
    )
    .parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_helo() {
        let cmd = parse_command("HELO example.com").expect("HELO command parse");
        assert!(matches!(cmd, SmtpCommand::Helo(domain) if domain == "example.com"));
    }

    #[test]
    fn test_parse_ehlo() {
        let cmd = parse_command("EHLO mail.example.com").expect("EHLO command parse");
        assert!(matches!(cmd, SmtpCommand::Ehlo(domain) if domain == "mail.example.com"));
    }

    #[test]
    fn test_parse_mail_from() {
        let cmd = parse_command("MAIL FROM:<user@example.com>").expect("MAIL FROM parse");
        match cmd {
            SmtpCommand::Mail { from, .. } => {
                assert_eq!(from.as_string(), "user@example.com");
            }
            _ => panic!("Expected Mail command"),
        }
    }

    #[test]
    fn test_parse_rcpt_to() {
        let cmd = parse_command("RCPT TO:<recipient@example.com>").expect("RCPT TO parse");
        match cmd {
            SmtpCommand::Rcpt { to, .. } => {
                assert_eq!(to.as_string(), "recipient@example.com");
            }
            _ => panic!("Expected Rcpt command"),
        }
    }

    #[test]
    fn test_parse_data() {
        let cmd = parse_command("DATA").expect("DATA command parse");
        assert!(matches!(cmd, SmtpCommand::Data));
    }

    #[test]
    fn test_parse_quit() {
        let cmd = parse_command("QUIT").expect("QUIT command parse");
        assert!(matches!(cmd, SmtpCommand::Quit));
    }

    #[test]
    fn test_parse_rset() {
        let cmd = parse_command("RSET").expect("RSET command parse");
        assert!(matches!(cmd, SmtpCommand::Rset));
    }

    #[test]
    fn test_parse_starttls() {
        let cmd = parse_command("STARTTLS").expect("STARTTLS command parse");
        assert!(matches!(cmd, SmtpCommand::StartTls));
    }

    #[test]
    fn test_parse_auth() {
        let cmd = parse_command("AUTH PLAIN dGVzdA==").expect("AUTH PLAIN command parse");
        match cmd {
            SmtpCommand::Auth {
                mechanism,
                initial_response,
            } => {
                assert_eq!(mechanism, "PLAIN");
                assert_eq!(initial_response, Some("dGVzdA==".to_string()));
            }
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn test_parse_case_insensitive() {
        let cmd1 = parse_command("quit").expect("lowercase quit parse");
        let cmd2 = parse_command("QUIT").expect("uppercase QUIT parse");
        let cmd3 = parse_command("QuIt").expect("mixed-case QuIt parse");

        assert!(matches!(cmd1, SmtpCommand::Quit));
        assert!(matches!(cmd2, SmtpCommand::Quit));
        assert!(matches!(cmd3, SmtpCommand::Quit));
    }

    #[test]
    fn test_parse_mail_with_size() {
        let cmd = parse_command("MAIL FROM:<user@example.com> SIZE=12345")
            .expect("MAIL FROM with SIZE param parse");
        match cmd {
            SmtpCommand::Mail { from, params } => {
                assert_eq!(from.as_string(), "user@example.com");
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].keyword, "SIZE");
                assert_eq!(params[0].value, Some("12345".to_string()));
            }
            _ => panic!("Expected Mail command"),
        }
    }

    #[test]
    fn test_parse_mail_with_body() {
        let cmd = parse_command("MAIL FROM:<user@example.com> BODY=8BITMIME")
            .expect("MAIL FROM with BODY param parse");
        match cmd {
            SmtpCommand::Mail { from, params } => {
                assert_eq!(from.as_string(), "user@example.com");
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].keyword, "BODY");
                assert_eq!(params[0].value, Some("8BITMIME".to_string()));
            }
            _ => panic!("Expected Mail command"),
        }
    }

    #[test]
    fn test_parse_mail_with_smtputf8() {
        let cmd = parse_command("MAIL FROM:<user@example.com> SMTPUTF8")
            .expect("MAIL FROM with SMTPUTF8 param parse");
        match cmd {
            SmtpCommand::Mail { from, params } => {
                assert_eq!(from.as_string(), "user@example.com");
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].keyword, "SMTPUTF8");
                assert_eq!(params[0].value, None);
            }
            _ => panic!("Expected Mail command"),
        }
    }

    #[test]
    fn test_parse_mail_with_multiple_params() {
        let cmd = parse_command("MAIL FROM:<user@example.com> SIZE=12345 BODY=8BITMIME SMTPUTF8")
            .expect("MAIL FROM with multiple params parse");
        match cmd {
            SmtpCommand::Mail { from, params } => {
                assert_eq!(from.as_string(), "user@example.com");
                assert_eq!(params.len(), 3);
                assert_eq!(params[0].keyword, "SIZE");
                assert_eq!(params[0].value, Some("12345".to_string()));
                assert_eq!(params[1].keyword, "BODY");
                assert_eq!(params[1].value, Some("8BITMIME".to_string()));
                assert_eq!(params[2].keyword, "SMTPUTF8");
                assert_eq!(params[2].value, None);
            }
            _ => panic!("Expected Mail command"),
        }
    }

    #[test]
    fn test_parse_bdat() {
        let cmd = parse_command("BDAT 1024").expect("BDAT without LAST parse");
        match cmd {
            SmtpCommand::Bdat { chunk_size, last } => {
                assert_eq!(chunk_size, 1024);
                assert!(!last);
            }
            _ => panic!("Expected Bdat command"),
        }
    }

    #[test]
    fn test_parse_bdat_last() {
        let cmd = parse_command("BDAT 512 LAST").expect("BDAT with LAST parse");
        match cmd {
            SmtpCommand::Bdat { chunk_size, last } => {
                assert_eq!(chunk_size, 512);
                assert!(last);
            }
            _ => panic!("Expected Bdat command"),
        }
    }

    #[test]
    fn test_parse_bdat_case_insensitive() {
        let cmd1 = parse_command("bdat 100").expect("lowercase bdat parse");
        let cmd2 = parse_command("BDAT 100").expect("uppercase BDAT parse");
        let cmd3 = parse_command("BdAt 100").expect("mixed-case BdAt parse");
        let cmd4 = parse_command("BDAT 256 last").expect("BDAT with lowercase last parse");
        let cmd5 = parse_command("bdat 256 LAST").expect("bdat with uppercase LAST parse");

        match (cmd1, cmd2, cmd3, cmd4, cmd5) {
            (
                SmtpCommand::Bdat {
                    chunk_size: s1,
                    last: l1,
                },
                SmtpCommand::Bdat {
                    chunk_size: s2,
                    last: l2,
                },
                SmtpCommand::Bdat {
                    chunk_size: s3,
                    last: l3,
                },
                SmtpCommand::Bdat {
                    chunk_size: s4,
                    last: l4,
                },
                SmtpCommand::Bdat {
                    chunk_size: s5,
                    last: l5,
                },
            ) => {
                assert_eq!(s1, 100);
                assert_eq!(s2, 100);
                assert_eq!(s3, 100);
                assert_eq!(s4, 256);
                assert_eq!(s5, 256);
                assert!(!l1);
                assert!(!l2);
                assert!(!l3);
                assert!(l4);
                assert!(l5);
            }
            _ => panic!("Expected Bdat commands"),
        }
    }
}

//! SMTP command types

use rusmes_proto::MailAddress;
use std::fmt;

/// SMTP commands as defined in RFC 5321
#[derive(Debug, Clone, PartialEq)]
pub enum SmtpCommand {
    /// HELO domain
    Helo(String),
    /// EHLO domain
    Ehlo(String),
    /// MAIL FROM:`<address>` \[parameters\]
    Mail {
        from: MailAddress,
        params: Vec<MailParam>,
    },
    /// RCPT TO:`<address>` \[parameters\]
    Rcpt {
        to: MailAddress,
        params: Vec<MailParam>,
    },
    /// DATA
    Data,
    /// BDAT `<chunk-size>` \[LAST\] - RFC 3030 CHUNKING
    Bdat { chunk_size: usize, last: bool },
    /// RSET - reset session
    Rset,
    /// NOOP - no operation
    Noop,
    /// QUIT - close connection
    Quit,
    /// VRFY `<string>` - verify address
    Vrfy(String),
    /// EXPN `<string>` - expand mailing list
    Expn(String),
    /// HELP \[`<string>`\]
    Help(Option<String>),
    /// STARTTLS - initiate TLS
    StartTls,
    /// AUTH `<mechanism>` [initial-response]
    Auth {
        mechanism: String,
        initial_response: Option<String>,
    },
}

impl fmt::Display for SmtpCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmtpCommand::Helo(domain) => write!(f, "HELO {}", domain),
            SmtpCommand::Ehlo(domain) => write!(f, "EHLO {}", domain),
            SmtpCommand::Mail { from, .. } => write!(f, "MAIL FROM:<{}>", from),
            SmtpCommand::Rcpt { to, .. } => write!(f, "RCPT TO:<{}>", to),
            SmtpCommand::Data => write!(f, "DATA"),
            SmtpCommand::Bdat { chunk_size, last } => {
                if *last {
                    write!(f, "BDAT {} LAST", chunk_size)
                } else {
                    write!(f, "BDAT {}", chunk_size)
                }
            }
            SmtpCommand::Rset => write!(f, "RSET"),
            SmtpCommand::Noop => write!(f, "NOOP"),
            SmtpCommand::Quit => write!(f, "QUIT"),
            SmtpCommand::Vrfy(addr) => write!(f, "VRFY {}", addr),
            SmtpCommand::Expn(list) => write!(f, "EXPN {}", list),
            SmtpCommand::Help(topic) => {
                if let Some(t) = topic {
                    write!(f, "HELP {}", t)
                } else {
                    write!(f, "HELP")
                }
            }
            SmtpCommand::StartTls => write!(f, "STARTTLS"),
            SmtpCommand::Auth { mechanism, .. } => write!(f, "AUTH {}", mechanism),
        }
    }
}

/// SMTP command parameters (ESMTP extensions)
#[derive(Debug, Clone, PartialEq)]
pub struct MailParam {
    pub keyword: String,
    pub value: Option<String>,
}

impl MailParam {
    /// Create a new mail parameter
    pub fn new(keyword: impl Into<String>, value: Option<String>) -> Self {
        Self {
            keyword: keyword.into(),
            value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_display() {
        let cmd = SmtpCommand::Helo("example.com".to_string());
        assert_eq!(cmd.to_string(), "HELO example.com");

        let cmd = SmtpCommand::Data;
        assert_eq!(cmd.to_string(), "DATA");

        let cmd = SmtpCommand::Quit;
        assert_eq!(cmd.to_string(), "QUIT");
    }
}

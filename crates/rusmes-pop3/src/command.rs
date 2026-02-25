//! POP3 command types

use std::fmt;

/// POP3 commands as defined in RFC 1939
#[derive(Debug, Clone, PartialEq)]
pub enum Pop3Command {
    /// USER name - specify username
    User(String),
    /// PASS string - authenticate with password
    Pass(String),
    /// STAT - get mailbox statistics
    Stat,
    /// LIST \[msg\] - list messages
    List(Option<u32>),
    /// RETR msg - retrieve a message
    Retr(u32),
    /// DELE msg - mark message for deletion
    Dele(u32),
    /// NOOP - no operation
    Noop,
    /// RSET - reset session (unmark deletions)
    Rset,
    /// QUIT - quit session
    Quit,
    /// TOP msg n - retrieve message headers and n lines of body
    Top { msg: u32, lines: u32 },
    /// UIDL \[msg\] - unique-id listing
    Uidl(Option<u32>),
    /// APOP name digest - alternative authentication (MD5 digest)
    Apop { name: String, digest: String },
    /// CAPA - list capabilities
    Capa,
    /// STLS - start TLS (STARTTLS)
    Stls,
}

impl fmt::Display for Pop3Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pop3Command::User(name) => write!(f, "USER {}", name),
            Pop3Command::Pass(_) => write!(f, "PASS <hidden>"),
            Pop3Command::Stat => write!(f, "STAT"),
            Pop3Command::List(Some(msg)) => write!(f, "LIST {}", msg),
            Pop3Command::List(None) => write!(f, "LIST"),
            Pop3Command::Retr(msg) => write!(f, "RETR {}", msg),
            Pop3Command::Dele(msg) => write!(f, "DELE {}", msg),
            Pop3Command::Noop => write!(f, "NOOP"),
            Pop3Command::Rset => write!(f, "RSET"),
            Pop3Command::Quit => write!(f, "QUIT"),
            Pop3Command::Top { msg, lines } => write!(f, "TOP {} {}", msg, lines),
            Pop3Command::Uidl(Some(msg)) => write!(f, "UIDL {}", msg),
            Pop3Command::Uidl(None) => write!(f, "UIDL"),
            Pop3Command::Apop { name, .. } => write!(f, "APOP {}", name),
            Pop3Command::Capa => write!(f, "CAPA"),
            Pop3Command::Stls => write!(f, "STLS"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_display_capa() {
        let cmd = Pop3Command::Capa;
        assert_eq!(cmd.to_string(), "CAPA");
    }

    #[test]
    fn test_command_display_stls() {
        let cmd = Pop3Command::Stls;
        assert_eq!(cmd.to_string(), "STLS");
    }

    #[test]
    fn test_command_equality() {
        assert_eq!(Pop3Command::Capa, Pop3Command::Capa);
        assert_eq!(Pop3Command::Stls, Pop3Command::Stls);
        assert_ne!(Pop3Command::Capa, Pop3Command::Stls);
    }
}

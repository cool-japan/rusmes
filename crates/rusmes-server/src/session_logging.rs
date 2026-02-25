//! Per-session structured logging
//!
//! This module provides session-aware structured logging for all protocol servers
//! (SMTP, IMAP, POP3, JMAP). Each connection gets a unique session ID that is
//! attached to all log events, making it easy to trace and debug individual sessions.
//!
//! # Features
//!
//! - Unique session IDs (UUID v4)
//! - Session context with client IP, protocol, and user information
//! - Integration with tracing-subscriber for structured logging
//! - Helper macros for convenient session-aware logging
//! - Automatic span management per session
//!
//! # Example
//!
//! ```no_run
//! use rusmes_server::session_logging::{SessionContext, SessionLogger};
//! use std::net::IpAddr;
//!
//! # async fn example() {
//! let session = SessionContext::new(
//!     IpAddr::from([127, 0, 0, 1]),
//!     "SMTP",
//! );
//!
//! let logger = SessionLogger::new(session);
//! let _guard = logger.enter();
//!
//! // All logs within this span will include session context
//! tracing::info!("Connection established");
//! # }
//! ```

use std::net::IpAddr;
use tracing::{span, Level, Span};
use uuid::Uuid;

/// Session context for structured logging
///
/// Contains all session-level information that should be attached to log events:
/// - Unique session ID
/// - Client IP address
/// - Protocol name (SMTP, IMAP, POP3, JMAP)
/// - Optional authenticated username
#[derive(Debug, Clone)]
pub struct SessionContext {
    /// Unique session identifier (UUID v4)
    pub session_id: String,

    /// Client IP address
    pub client_ip: IpAddr,

    /// Protocol name (e.g., "SMTP", "IMAP", "POP3", "JMAP")
    pub protocol: String,

    /// Authenticated username (if any)
    pub username: Option<String>,
}

impl SessionContext {
    /// Create a new session context with a generated UUID
    ///
    /// # Arguments
    ///
    /// * `client_ip` - The client's IP address
    /// * `protocol` - The protocol name (e.g., "SMTP", "IMAP")
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rusmes_server::session_logging::SessionContext;
    /// use std::net::IpAddr;
    ///
    /// let session = SessionContext::new(
    ///     IpAddr::from([192, 168, 1, 100]),
    ///     "SMTP",
    /// );
    /// ```
    pub fn new(client_ip: IpAddr, protocol: impl Into<String>) -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            client_ip,
            protocol: protocol.into(),
            username: None,
        }
    }

    /// Create a new session context with a custom session ID
    ///
    /// Useful for testing or when you need a specific ID format.
    ///
    /// # Arguments
    ///
    /// * `session_id` - Custom session ID
    /// * `client_ip` - The client's IP address
    /// * `protocol` - The protocol name
    pub fn with_id(
        session_id: impl Into<String>,
        client_ip: IpAddr,
        protocol: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            client_ip,
            protocol: protocol.into(),
            username: None,
        }
    }

    /// Set the authenticated username
    ///
    /// Call this after successful authentication to include the username
    /// in all subsequent log events.
    ///
    /// # Arguments
    ///
    /// * `username` - The authenticated username
    pub fn set_username(&mut self, username: impl Into<String>) {
        self.username = Some(username.into());
    }

    /// Get the session ID
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get the client IP
    pub fn client_ip(&self) -> IpAddr {
        self.client_ip
    }

    /// Get the protocol name
    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    /// Get the username if authenticated
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }
}

/// Session-aware logger that wraps tracing spans
///
/// Creates a tracing span with session context fields and provides
/// convenient methods for logging with session information.
#[derive(Debug)]
pub struct SessionLogger {
    /// Session context
    context: SessionContext,

    /// Tracing span for this session
    span: Span,
}

impl SessionLogger {
    /// Create a new session logger
    ///
    /// This creates a tracing span at INFO level with all session context fields.
    ///
    /// # Arguments
    ///
    /// * `context` - The session context
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rusmes_server::session_logging::{SessionContext, SessionLogger};
    /// use std::net::IpAddr;
    ///
    /// let session = SessionContext::new(
    ///     IpAddr::from([127, 0, 0, 1]),
    ///     "IMAP",
    /// );
    /// let logger = SessionLogger::new(session);
    /// ```
    pub fn new(context: SessionContext) -> Self {
        let span = if let Some(ref username) = context.username {
            span!(
                Level::INFO,
                "session",
                session_id = %context.session_id,
                client_ip = %context.client_ip,
                protocol = %context.protocol,
                username = %username,
            )
        } else {
            span!(
                Level::INFO,
                "session",
                session_id = %context.session_id,
                client_ip = %context.client_ip,
                protocol = %context.protocol,
            )
        };

        Self { context, span }
    }

    /// Enter the session span
    ///
    /// Returns a guard that will exit the span when dropped.
    /// All logging done while the guard is held will include session context.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rusmes_server::session_logging::{SessionContext, SessionLogger};
    /// use std::net::IpAddr;
    ///
    /// # async fn example() {
    /// let session = SessionContext::new(
    ///     IpAddr::from([127, 0, 0, 1]),
    ///     "POP3",
    /// );
    /// let logger = SessionLogger::new(session);
    /// let _guard = logger.enter();
    ///
    /// tracing::info!("This log will include session context");
    /// # }
    /// ```
    pub fn enter(&self) -> tracing::span::Entered<'_> {
        self.span.enter()
    }

    /// Get the session context
    pub fn context(&self) -> &SessionContext {
        &self.context
    }

    /// Update the session context with a username
    ///
    /// This creates a new span with the updated username field.
    ///
    /// # Arguments
    ///
    /// * `username` - The authenticated username
    pub fn set_username(&mut self, username: impl Into<String>) {
        self.context.set_username(username);

        // Create a new span with the username
        let username_str = self.context.username.as_deref().unwrap_or("<unknown>");
        self.span = span!(
            Level::INFO,
            "session",
            session_id = %self.context.session_id,
            client_ip = %self.context.client_ip,
            protocol = %self.context.protocol,
            username = %username_str,
        );
    }

    /// Get the tracing span
    pub fn span(&self) -> &Span {
        &self.span
    }
}

/// Helper macros for session-aware logging
///
/// These macros make it convenient to log with session context
/// without having to manually specify fields each time.
/// Log an info message with session context
///
/// # Example
///
/// ```ignore
/// session_info!(logger, "Command received", command = "HELO");
/// ```
#[macro_export]
macro_rules! session_info {
    ($logger:expr, $($arg:tt)*) => {
        {
            let _guard = $logger.enter();
            tracing::info!($($arg)*);
        }
    };
}

/// Log a debug message with session context
///
/// # Example
///
/// ```ignore
/// session_debug!(logger, "Parsing command", input = &line);
/// ```
#[macro_export]
macro_rules! session_debug {
    ($logger:expr, $($arg:tt)*) => {
        {
            let _guard = $logger.enter();
            tracing::debug!($($arg)*);
        }
    };
}

/// Log a warning message with session context
///
/// # Example
///
/// ```ignore
/// session_warn!(logger, "Rate limit approaching", remaining = 10);
/// ```
#[macro_export]
macro_rules! session_warn {
    ($logger:expr, $($arg:tt)*) => {
        {
            let _guard = $logger.enter();
            tracing::warn!($($arg)*);
        }
    };
}

/// Log an error message with session context
///
/// # Example
///
/// ```ignore
/// session_error!(logger, "Authentication failed", reason = "invalid_password");
/// ```
#[macro_export]
macro_rules! session_error {
    ($logger:expr, $($arg:tt)*) => {
        {
            let _guard = $logger.enter();
            tracing::error!($($arg)*);
        }
    };
}

/// Log a trace message with session context
///
/// # Example
///
/// ```ignore
/// session_trace!(logger, "State transition", from = "CONNECTED", to = "AUTHENTICATED");
/// ```
#[macro_export]
macro_rules! session_trace {
    ($logger:expr, $($arg:tt)*) => {
        {
            let _guard = $logger.enter();
            tracing::trace!($($arg)*);
        }
    };
}

/// Helper function to format session context for response headers
///
/// This is primarily useful for JMAP and other HTTP-based protocols
/// where you can include session IDs in response headers.
///
/// # Arguments
///
/// * `context` - The session context
///
/// # Returns
///
/// A formatted string suitable for use in a response header (just the session ID)
///
/// # Example
///
/// ```no_run
/// use rusmes_server::session_logging::{SessionContext, format_session_header};
/// use std::net::IpAddr;
///
/// let session = SessionContext::new(
///     IpAddr::from([127, 0, 0, 1]),
///     "JMAP",
/// );
/// let header_value = format_session_header(&session);
/// // Use in HTTP response: X-Session-Id: <uuid>
/// ```
pub fn format_session_header(context: &SessionContext) -> String {
    context.session_id.clone()
}

/// Create a JSON representation of session context
///
/// Useful for structured log outputs or external monitoring systems.
///
/// # Arguments
///
/// * `context` - The session context
///
/// # Returns
///
/// A JSON string with session information
pub fn format_session_json(context: &SessionContext) -> String {
    let username_str = context.username.as_deref().unwrap_or("");
    format!(
        r#"{{"session_id":"{}","client_ip":"{}","protocol":"{}","username":"{}"}}"#,
        context.session_id, context.client_ip, context.protocol, username_str
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn test_session_context_creation() {
        let ip = IpAddr::from([192, 168, 1, 100]);
        let session = SessionContext::new(ip, "SMTP");

        assert!(!session.session_id.is_empty());
        assert_eq!(session.client_ip, ip);
        assert_eq!(session.protocol, "SMTP");
        assert!(session.username.is_none());
    }

    #[test]
    fn test_session_context_with_id() {
        let ip = IpAddr::from([10, 0, 0, 1]);
        let custom_id = "test-session-123";
        let session = SessionContext::with_id(custom_id, ip, "IMAP");

        assert_eq!(session.session_id, custom_id);
        assert_eq!(session.client_ip, ip);
        assert_eq!(session.protocol, "IMAP");
    }

    #[test]
    fn test_set_username() {
        let ip = IpAddr::from([127, 0, 0, 1]);
        let mut session = SessionContext::new(ip, "POP3");

        assert!(session.username.is_none());

        session.set_username("alice");
        assert_eq!(session.username.as_deref(), Some("alice"));
    }

    #[test]
    fn test_session_logger_creation() {
        let ip = IpAddr::from([192, 168, 1, 1]);
        let context = SessionContext::new(ip, "JMAP");
        let logger = SessionLogger::new(context);

        assert_eq!(logger.context().protocol, "JMAP");
        assert_eq!(logger.context().client_ip, ip);
    }

    #[test]
    fn test_session_logger_set_username() {
        let ip = IpAddr::from([172, 16, 0, 1]);
        let context = SessionContext::new(ip, "SMTP");
        let mut logger = SessionLogger::new(context);

        assert!(logger.context().username.is_none());

        logger.set_username("bob");
        assert_eq!(logger.context().username.as_deref(), Some("bob"));
    }

    #[test]
    fn test_format_session_header() {
        let ip = IpAddr::from([127, 0, 0, 1]);
        let session = SessionContext::with_id("abc-123", ip, "JMAP");
        let header = format_session_header(&session);

        assert_eq!(header, "abc-123");
    }

    #[test]
    fn test_format_session_json() {
        let ip = IpAddr::from([192, 168, 1, 50]);
        let mut session = SessionContext::with_id("test-id", ip, "IMAP");
        session.set_username("alice");

        let json = format_session_json(&session);
        assert!(json.contains(r#""session_id":"test-id""#));
        assert!(json.contains(r#""client_ip":"192.168.1.50""#));
        assert!(json.contains(r#""protocol":"IMAP""#));
        assert!(json.contains(r#""username":"alice""#));
    }

    #[test]
    fn test_format_session_json_no_username() {
        let ip = IpAddr::from([10, 0, 0, 1]);
        let session = SessionContext::with_id("session-123", ip, "SMTP");

        let json = format_session_json(&session);
        assert!(json.contains(r#""username":"""#));
    }

    #[test]
    fn test_session_id_is_uuid() {
        let ip = IpAddr::from([127, 0, 0, 1]);
        let session = SessionContext::new(ip, "POP3");

        // Try to parse as UUID to verify format
        let parsed = Uuid::parse_str(&session.session_id);
        assert!(parsed.is_ok(), "Session ID should be a valid UUID");
    }

    #[test]
    fn test_unique_session_ids() {
        let ip = IpAddr::from([127, 0, 0, 1]);
        let session1 = SessionContext::new(ip, "SMTP");
        let session2 = SessionContext::new(ip, "SMTP");

        assert_ne!(
            session1.session_id, session2.session_id,
            "Session IDs should be unique"
        );
    }
}

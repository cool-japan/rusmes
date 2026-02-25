//! SMTP DSN (Delivery Status Notification) Extension - RFC 3461
//!
//! This module implements Delivery Status Notifications for SMTP,
//! allowing senders to request notifications about message delivery status.

use std::fmt;

/// Return type for DSN - how much of the message to return
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsnRet {
    /// Return full message in DSN
    Full,
    /// Return headers only in DSN
    Hdrs,
}

impl DsnRet {
    /// Parse DSN RET parameter value
    pub fn parse(s: &str) -> Result<Self, DsnError> {
        match s.to_uppercase().as_str() {
            "FULL" => Ok(DsnRet::Full),
            "HDRS" => Ok(DsnRet::Hdrs),
            _ => Err(DsnError::InvalidRet(s.to_string())),
        }
    }
}

impl fmt::Display for DsnRet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DsnRet::Full => write!(f, "FULL"),
            DsnRet::Hdrs => write!(f, "HDRS"),
        }
    }
}

/// Notification conditions for DSN
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsnNotify {
    /// Never send DSN
    Never,
    /// Send DSN on successful delivery
    Success,
    /// Send DSN on delivery failure
    Failure,
    /// Send DSN on delivery delay
    Delay,
}

impl DsnNotify {
    /// Parse DSN NOTIFY parameter value (comma-separated list)
    pub fn parse_list(s: &str) -> Result<Vec<Self>, DsnError> {
        if s.eq_ignore_ascii_case("NEVER") {
            return Ok(vec![DsnNotify::Never]);
        }

        let mut notifications = Vec::new();
        for part in s.split(',') {
            let notify = match part.trim().to_uppercase().as_str() {
                "SUCCESS" => DsnNotify::Success,
                "FAILURE" => DsnNotify::Failure,
                "DELAY" => DsnNotify::Delay,
                other => return Err(DsnError::InvalidNotify(other.to_string())),
            };
            notifications.push(notify);
        }

        if notifications.is_empty() {
            return Err(DsnError::InvalidNotify(s.to_string()));
        }

        Ok(notifications)
    }
}

impl fmt::Display for DsnNotify {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DsnNotify::Never => write!(f, "NEVER"),
            DsnNotify::Success => write!(f, "SUCCESS"),
            DsnNotify::Failure => write!(f, "FAILURE"),
            DsnNotify::Delay => write!(f, "DELAY"),
        }
    }
}

/// DSN parameters for MAIL FROM command
#[derive(Debug, Clone, Default)]
pub struct DsnMailParams {
    /// RET parameter - how much to return
    pub ret: Option<DsnRet>,
    /// ENVID parameter - envelope identifier
    pub envid: Option<String>,
}

impl DsnMailParams {
    /// Create new DSN mail parameters
    pub fn new() -> Self {
        Self::default()
    }

    /// Set RET parameter
    pub fn with_ret(mut self, ret: DsnRet) -> Self {
        self.ret = Some(ret);
        self
    }

    /// Set ENVID parameter
    pub fn with_envid(mut self, envid: String) -> Self {
        self.envid = Some(envid);
        self
    }

    /// Parse DSN parameters from MAIL FROM command
    pub fn parse(params: &[(String, Option<String>)]) -> Result<Self, DsnError> {
        let mut dsn = Self::new();

        for (key, value) in params {
            match key.to_uppercase().as_str() {
                "RET" => {
                    let val = value
                        .as_ref()
                        .ok_or_else(|| DsnError::MissingValue("RET".to_string()))?;
                    dsn.ret = Some(DsnRet::parse(val)?);
                }
                "ENVID" => {
                    let val = value
                        .as_ref()
                        .ok_or_else(|| DsnError::MissingValue("ENVID".to_string()))?;
                    // ENVID can be xtext-encoded, but we accept any printable string
                    if val.len() > 100 {
                        return Err(DsnError::EnvidTooLong);
                    }
                    dsn.envid = Some(val.clone());
                }
                _ => {} // Ignore unknown parameters
            }
        }

        Ok(dsn)
    }
}

/// DSN parameters for RCPT TO command
#[derive(Debug, Clone, Default)]
pub struct DsnRcptParams {
    /// NOTIFY parameter - when to send notifications
    pub notify: Vec<DsnNotify>,
    /// ORCPT parameter - original recipient
    pub orcpt: Option<String>,
}

impl DsnRcptParams {
    /// Create new DSN recipient parameters
    pub fn new() -> Self {
        Self::default()
    }

    /// Set NOTIFY parameter
    pub fn with_notify(mut self, notify: Vec<DsnNotify>) -> Self {
        self.notify = notify;
        self
    }

    /// Set ORCPT parameter
    pub fn with_orcpt(mut self, orcpt: String) -> Self {
        self.orcpt = Some(orcpt);
        self
    }

    /// Parse DSN parameters from RCPT TO command
    pub fn parse(params: &[(String, Option<String>)]) -> Result<Self, DsnError> {
        let mut dsn = Self::new();

        for (key, value) in params {
            match key.to_uppercase().as_str() {
                "NOTIFY" => {
                    let val = value
                        .as_ref()
                        .ok_or_else(|| DsnError::MissingValue("NOTIFY".to_string()))?;
                    dsn.notify = DsnNotify::parse_list(val)?;
                }
                "ORCPT" => {
                    let val = value
                        .as_ref()
                        .ok_or_else(|| DsnError::MissingValue("ORCPT".to_string()))?;
                    // ORCPT format: addr-type;address (e.g., "rfc822;user@example.com")
                    if !val.contains(';') {
                        return Err(DsnError::InvalidOrcpt(val.clone()));
                    }
                    dsn.orcpt = Some(val.clone());
                }
                _ => {} // Ignore unknown parameters
            }
        }

        Ok(dsn)
    }

    /// Check if DSN should be sent for a specific condition
    pub fn should_notify(&self, condition: DsnNotify) -> bool {
        if self.notify.contains(&DsnNotify::Never) {
            return false;
        }
        self.notify.contains(&condition)
    }
}

/// DSN-related errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DsnError {
    /// Invalid RET parameter value
    InvalidRet(String),
    /// Invalid NOTIFY parameter value
    InvalidNotify(String),
    /// Invalid ORCPT parameter format
    InvalidOrcpt(String),
    /// Missing required parameter value
    MissingValue(String),
    /// ENVID too long (max 100 characters)
    EnvidTooLong,
}

impl fmt::Display for DsnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DsnError::InvalidRet(val) => write!(f, "Invalid RET value: {}", val),
            DsnError::InvalidNotify(val) => write!(f, "Invalid NOTIFY value: {}", val),
            DsnError::InvalidOrcpt(val) => write!(f, "Invalid ORCPT format: {}", val),
            DsnError::MissingValue(param) => write!(f, "Missing value for parameter: {}", param),
            DsnError::EnvidTooLong => write!(f, "ENVID too long (max 100 characters)"),
        }
    }
}

impl std::error::Error for DsnError {}

#[cfg(test)]
mod tests {
    use super::*;

    // DsnRet tests
    #[test]
    fn test_dsn_ret_parse() {
        assert_eq!(
            DsnRet::parse("FULL").expect("FULL is valid DsnRet"),
            DsnRet::Full
        );
        assert_eq!(
            DsnRet::parse("HDRS").expect("HDRS is valid DsnRet"),
            DsnRet::Hdrs
        );
        assert_eq!(
            DsnRet::parse("full").expect("lowercase full is valid DsnRet"),
            DsnRet::Full
        );
        assert!(DsnRet::parse("INVALID").is_err());
    }

    #[test]
    fn test_dsn_ret_parse_case_insensitive() {
        assert_eq!(
            DsnRet::parse("hdrs").expect("lowercase hdrs is valid DsnRet"),
            DsnRet::Hdrs
        );
        assert_eq!(
            DsnRet::parse("FuLl").expect("mixed-case FuLl is valid DsnRet"),
            DsnRet::Full
        );
    }

    #[test]
    fn test_dsn_ret_display() {
        assert_eq!(DsnRet::Full.to_string(), "FULL");
        assert_eq!(DsnRet::Hdrs.to_string(), "HDRS");
    }

    #[test]
    fn test_dsn_ret_parse_invalid() {
        assert!(DsnRet::parse("").is_err());
        assert!(DsnRet::parse("PARTIAL").is_err());
        assert!(DsnRet::parse("123").is_err());
    }

    // DsnNotify tests
    #[test]
    fn test_dsn_notify_parse_list() {
        let result = DsnNotify::parse_list("SUCCESS,FAILURE")
            .expect("SUCCESS,FAILURE is valid DsnNotify list");
        assert_eq!(result.len(), 2);
        assert!(result.contains(&DsnNotify::Success));
        assert!(result.contains(&DsnNotify::Failure));

        let never =
            DsnNotify::parse_list("NEVER").expect("NEVER is valid DsnNotify singleton list");
        assert_eq!(never, vec![DsnNotify::Never]);

        assert!(DsnNotify::parse_list("INVALID").is_err());
    }

    #[test]
    fn test_dsn_notify_parse_single() {
        let success = DsnNotify::parse_list("SUCCESS").expect("SUCCESS is valid single DsnNotify");
        assert_eq!(success, vec![DsnNotify::Success]);

        let failure = DsnNotify::parse_list("FAILURE").expect("FAILURE is valid single DsnNotify");
        assert_eq!(failure, vec![DsnNotify::Failure]);

        let delay = DsnNotify::parse_list("DELAY").expect("DELAY is valid single DsnNotify");
        assert_eq!(delay, vec![DsnNotify::Delay]);
    }

    #[test]
    fn test_dsn_notify_parse_all_three() {
        let result = DsnNotify::parse_list("SUCCESS,FAILURE,DELAY")
            .expect("SUCCESS,FAILURE,DELAY is valid DsnNotify list");
        assert_eq!(result.len(), 3);
        assert!(result.contains(&DsnNotify::Success));
        assert!(result.contains(&DsnNotify::Failure));
        assert!(result.contains(&DsnNotify::Delay));
    }

    #[test]
    fn test_dsn_notify_parse_with_spaces() {
        let result = DsnNotify::parse_list("SUCCESS, FAILURE, DELAY")
            .expect("space-separated DsnNotify list should be valid");
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_dsn_notify_display() {
        assert_eq!(DsnNotify::Never.to_string(), "NEVER");
        assert_eq!(DsnNotify::Success.to_string(), "SUCCESS");
        assert_eq!(DsnNotify::Failure.to_string(), "FAILURE");
        assert_eq!(DsnNotify::Delay.to_string(), "DELAY");
    }

    #[test]
    fn test_dsn_notify_parse_empty() {
        assert!(DsnNotify::parse_list("").is_err());
    }

    // DsnMailParams tests
    #[test]
    fn test_dsn_mail_params_parse() {
        let params = vec![
            ("RET".to_string(), Some("FULL".to_string())),
            ("ENVID".to_string(), Some("abc123".to_string())),
        ];

        let dsn = DsnMailParams::parse(&params).expect("valid RET+ENVID mail params");
        assert_eq!(dsn.ret, Some(DsnRet::Full));
        assert_eq!(dsn.envid, Some("abc123".to_string()));
    }

    #[test]
    fn test_dsn_mail_params_parse_only_ret() {
        let params = vec![("RET".to_string(), Some("HDRS".to_string()))];

        let dsn = DsnMailParams::parse(&params).expect("valid RET-only mail params");
        assert_eq!(dsn.ret, Some(DsnRet::Hdrs));
        assert_eq!(dsn.envid, None);
    }

    #[test]
    fn test_dsn_mail_params_parse_only_envid() {
        let params = vec![("ENVID".to_string(), Some("xyz789".to_string()))];

        let dsn = DsnMailParams::parse(&params).expect("valid ENVID-only mail params");
        assert_eq!(dsn.ret, None);
        assert_eq!(dsn.envid, Some("xyz789".to_string()));
    }

    #[test]
    fn test_dsn_mail_params_parse_empty() {
        let params = vec![];
        let dsn = DsnMailParams::parse(&params).expect("empty params should parse to defaults");
        assert_eq!(dsn.ret, None);
        assert_eq!(dsn.envid, None);
    }

    #[test]
    fn test_dsn_mail_params_builder() {
        let dsn = DsnMailParams::new()
            .with_ret(DsnRet::Full)
            .with_envid("test123".to_string());

        assert_eq!(dsn.ret, Some(DsnRet::Full));
        assert_eq!(dsn.envid, Some("test123".to_string()));
    }

    #[test]
    fn test_dsn_mail_params_missing_ret_value() {
        let params = vec![("RET".to_string(), None)];

        assert!(DsnMailParams::parse(&params).is_err());
    }

    #[test]
    fn test_dsn_mail_params_missing_envid_value() {
        let params = vec![("ENVID".to_string(), None)];

        assert!(DsnMailParams::parse(&params).is_err());
    }

    #[test]
    fn test_envid_too_long() {
        let long_envid = "a".repeat(101);
        let params = vec![("ENVID".to_string(), Some(long_envid))];

        assert!(DsnMailParams::parse(&params).is_err());
    }

    #[test]
    fn test_envid_max_length() {
        let max_envid = "a".repeat(100);
        let params = vec![("ENVID".to_string(), Some(max_envid.clone()))];

        let dsn = DsnMailParams::parse(&params).expect("100-char ENVID is at max allowed length");
        assert_eq!(dsn.envid, Some(max_envid));
    }

    // DsnRcptParams tests
    #[test]
    fn test_dsn_rcpt_params_parse() {
        let params = vec![
            ("NOTIFY".to_string(), Some("SUCCESS,FAILURE".to_string())),
            (
                "ORCPT".to_string(),
                Some("rfc822;user@example.com".to_string()),
            ),
        ];

        let dsn = DsnRcptParams::parse(&params).expect("valid NOTIFY+ORCPT rcpt params");
        assert_eq!(dsn.notify.len(), 2);
        assert_eq!(dsn.orcpt, Some("rfc822;user@example.com".to_string()));
    }

    #[test]
    fn test_dsn_rcpt_params_parse_only_notify() {
        let params = vec![("NOTIFY".to_string(), Some("SUCCESS".to_string()))];

        let dsn = DsnRcptParams::parse(&params).expect("valid NOTIFY-only rcpt params");
        assert_eq!(dsn.notify.len(), 1);
        assert_eq!(dsn.orcpt, None);
    }

    #[test]
    fn test_dsn_rcpt_params_parse_only_orcpt() {
        let params = vec![(
            "ORCPT".to_string(),
            Some("rfc822;test@test.com".to_string()),
        )];

        let dsn = DsnRcptParams::parse(&params).expect("valid ORCPT-only rcpt params");
        assert!(dsn.notify.is_empty());
        assert_eq!(dsn.orcpt, Some("rfc822;test@test.com".to_string()));
    }

    #[test]
    fn test_dsn_rcpt_params_builder() {
        let dsn = DsnRcptParams::new()
            .with_notify(vec![DsnNotify::Success])
            .with_orcpt("rfc822;user@test.com".to_string());

        assert_eq!(dsn.notify.len(), 1);
        assert_eq!(dsn.orcpt, Some("rfc822;user@test.com".to_string()));
    }

    #[test]
    fn test_should_notify() {
        let dsn = DsnRcptParams {
            notify: vec![DsnNotify::Success, DsnNotify::Failure],
            orcpt: None,
        };

        assert!(dsn.should_notify(DsnNotify::Success));
        assert!(dsn.should_notify(DsnNotify::Failure));
        assert!(!dsn.should_notify(DsnNotify::Delay));

        let never = DsnRcptParams {
            notify: vec![DsnNotify::Never],
            orcpt: None,
        };

        assert!(!never.should_notify(DsnNotify::Success));
        assert!(!never.should_notify(DsnNotify::Failure));
    }

    #[test]
    fn test_should_notify_delay_only() {
        let dsn = DsnRcptParams {
            notify: vec![DsnNotify::Delay],
            orcpt: None,
        };

        assert!(!dsn.should_notify(DsnNotify::Success));
        assert!(!dsn.should_notify(DsnNotify::Failure));
        assert!(dsn.should_notify(DsnNotify::Delay));
    }

    #[test]
    fn test_should_notify_empty_list() {
        let dsn = DsnRcptParams {
            notify: vec![],
            orcpt: None,
        };

        assert!(!dsn.should_notify(DsnNotify::Success));
        assert!(!dsn.should_notify(DsnNotify::Failure));
        assert!(!dsn.should_notify(DsnNotify::Delay));
    }

    #[test]
    fn test_invalid_orcpt() {
        let params = vec![("ORCPT".to_string(), Some("invalid".to_string()))];

        assert!(DsnRcptParams::parse(&params).is_err());
    }

    #[test]
    fn test_invalid_orcpt_no_address() {
        let params = vec![("ORCPT".to_string(), Some("rfc822;".to_string()))];

        let dsn = DsnRcptParams::parse(&params)
            .expect("rfc822 with empty address should be accepted as-is");
        assert_eq!(dsn.orcpt, Some("rfc822;".to_string()));
    }

    #[test]
    fn test_orcpt_different_addr_types() {
        let params = vec![(
            "ORCPT".to_string(),
            Some("x400;o=example;s=user".to_string()),
        )];

        let dsn = DsnRcptParams::parse(&params).expect("x400 address type ORCPT should be valid");
        assert_eq!(dsn.orcpt, Some("x400;o=example;s=user".to_string()));
    }

    #[test]
    fn test_dsn_rcpt_params_missing_notify_value() {
        let params = vec![("NOTIFY".to_string(), None)];

        assert!(DsnRcptParams::parse(&params).is_err());
    }

    #[test]
    fn test_dsn_rcpt_params_missing_orcpt_value() {
        let params = vec![("ORCPT".to_string(), None)];

        assert!(DsnRcptParams::parse(&params).is_err());
    }

    // DsnError tests
    #[test]
    fn test_dsn_error_display() {
        let err = DsnError::InvalidRet("INVALID".to_string());
        assert_eq!(err.to_string(), "Invalid RET value: INVALID");

        let err = DsnError::InvalidNotify("BAD".to_string());
        assert_eq!(err.to_string(), "Invalid NOTIFY value: BAD");

        let err = DsnError::InvalidOrcpt("noSemicolon".to_string());
        assert_eq!(err.to_string(), "Invalid ORCPT format: noSemicolon");

        let err = DsnError::MissingValue("TEST".to_string());
        assert_eq!(err.to_string(), "Missing value for parameter: TEST");

        let err = DsnError::EnvidTooLong;
        assert_eq!(err.to_string(), "ENVID too long (max 100 characters)");
    }

    #[test]
    fn test_dsn_error_clone_and_eq() {
        let err1 = DsnError::InvalidRet("TEST".to_string());
        let err2 = err1.clone();
        assert_eq!(err1, err2);
    }

    // Integration tests
    #[test]
    fn test_dsn_params_ignore_unknown() {
        let params = vec![
            ("RET".to_string(), Some("FULL".to_string())),
            ("UNKNOWN".to_string(), Some("value".to_string())),
            ("ENVID".to_string(), Some("abc".to_string())),
        ];

        let dsn =
            DsnMailParams::parse(&params).expect("mail params with unknown keys should succeed");
        assert_eq!(dsn.ret, Some(DsnRet::Full));
        assert_eq!(dsn.envid, Some("abc".to_string()));
    }

    #[test]
    fn test_dsn_rcpt_params_ignore_unknown() {
        let params = vec![
            ("NOTIFY".to_string(), Some("SUCCESS".to_string())),
            ("UNKNOWN".to_string(), Some("value".to_string())),
        ];

        let dsn =
            DsnRcptParams::parse(&params).expect("rcpt params with unknown keys should succeed");
        assert_eq!(dsn.notify.len(), 1);
    }
}

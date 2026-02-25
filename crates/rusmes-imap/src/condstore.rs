//! IMAP CONDSTORE Extension - RFC 7162
//!
//! This module implements Conditional STORE with MODSEQ tracking,
//! enabling efficient synchronization of IMAP mailboxes.

use rusmes_storage::ModSeq;
use std::fmt;

/// CONDSTORE capability enablement state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondStoreState {
    /// CONDSTORE not enabled
    Disabled,
    /// CONDSTORE enabled explicitly via ENABLE
    Enabled,
    /// CONDSTORE enabled implicitly (via SELECT/FETCH with CONDSTORE params)
    ImplicitlyEnabled,
}

impl CondStoreState {
    /// Check if CONDSTORE is enabled (explicitly or implicitly)
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled | Self::ImplicitlyEnabled)
    }
}

/// CHANGEDSINCE search modifier for FETCH/SEARCH commands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangedSince {
    /// MODSEQ value to compare against
    pub modseq: ModSeq,
}

impl ChangedSince {
    /// Create new CHANGEDSINCE modifier
    pub fn new(modseq: ModSeq) -> Self {
        Self { modseq }
    }

    /// Parse CHANGEDSINCE from command arguments
    ///
    /// Format: CHANGEDSINCE `<modseq>`
    pub fn parse(args: &str) -> Result<Self, CondStoreError> {
        let modseq = args
            .trim()
            .parse::<u64>()
            .map_err(|_| CondStoreError::InvalidModSeq(args.to_string()))?;

        if modseq == 0 {
            return Err(CondStoreError::ZeroModSeq);
        }

        Ok(Self::new(ModSeq::new(modseq)))
    }

    /// Check if a message with the given MODSEQ matches this criteria
    pub fn matches(&self, message_modseq: ModSeq) -> bool {
        message_modseq > self.modseq
    }
}

/// UNCHANGEDSINCE store modifier for conditional STORE
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnchangedSince {
    /// MODSEQ value to compare against
    pub modseq: ModSeq,
}

impl UnchangedSince {
    /// Create new UNCHANGEDSINCE modifier
    pub fn new(modseq: ModSeq) -> Self {
        Self { modseq }
    }

    /// Parse UNCHANGEDSINCE from command arguments
    ///
    /// Format: (UNCHANGEDSINCE `<modseq>`)
    pub fn parse(args: &str) -> Result<Self, CondStoreError> {
        // Remove parentheses if present
        let args = args.trim().trim_matches(|c| c == '(' || c == ')');

        // Check for UNCHANGEDSINCE keyword
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() != 2 || !parts[0].eq_ignore_ascii_case("UNCHANGEDSINCE") {
            return Err(CondStoreError::InvalidUnchangedSince(args.to_string()));
        }

        let modseq = parts[1]
            .parse::<u64>()
            .map_err(|_| CondStoreError::InvalidModSeq(parts[1].to_string()))?;

        if modseq == 0 {
            return Err(CondStoreError::ZeroModSeq);
        }

        Ok(Self::new(ModSeq::new(modseq)))
    }

    /// Check if a message can be modified (MODSEQ hasn't changed)
    pub fn can_modify(&self, current_modseq: ModSeq) -> bool {
        current_modseq <= self.modseq
    }
}

/// CONDSTORE-related errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CondStoreError {
    /// Invalid MODSEQ value
    InvalidModSeq(String),
    /// MODSEQ cannot be zero
    ZeroModSeq,
    /// Invalid UNCHANGEDSINCE syntax
    InvalidUnchangedSince(String),
    /// CONDSTORE not enabled for this session
    NotEnabled,
    /// STORE failed due to UNCHANGEDSINCE condition
    StoreFailedModified {
        /// UIDs that were not modified due to UNCHANGEDSINCE
        failed_uids: Vec<u32>,
    },
}

impl fmt::Display for CondStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CondStoreError::InvalidModSeq(s) => write!(f, "Invalid MODSEQ: {}", s),
            CondStoreError::ZeroModSeq => write!(f, "MODSEQ cannot be zero"),
            CondStoreError::InvalidUnchangedSince(s) => {
                write!(f, "Invalid UNCHANGEDSINCE syntax: {}", s)
            }
            CondStoreError::NotEnabled => write!(f, "CONDSTORE not enabled"),
            CondStoreError::StoreFailedModified { failed_uids } => {
                write!(f, "STORE failed for UIDs (modified): {:?}", failed_uids)
            }
        }
    }
}

impl std::error::Error for CondStoreError {}

/// CONDSTORE response data for FETCH
#[derive(Debug, Clone)]
pub struct CondStoreResponse {
    /// Message UID
    pub uid: u32,
    /// Current MODSEQ value
    pub modseq: ModSeq,
    /// Message sequence number
    pub seq: u32,
}

impl CondStoreResponse {
    /// Create new CONDSTORE response
    pub fn new(uid: u32, modseq: ModSeq, seq: u32) -> Self {
        Self { uid, modseq, seq }
    }

    /// Format as IMAP FETCH response
    ///
    /// Example: * 1 FETCH (UID 42 MODSEQ (12345))
    pub fn to_fetch_response(&self) -> String {
        format!(
            "* {} FETCH (UID {} MODSEQ ({}))",
            self.seq, self.uid, self.modseq
        )
    }
}

/// Mailbox status with CONDSTORE support
#[derive(Debug, Clone)]
pub struct CondStoreStatus {
    /// Mailbox name
    pub mailbox: String,
    /// Highest MODSEQ in the mailbox
    pub highestmodseq: ModSeq,
    /// Number of messages
    pub exists: u32,
    /// Number of recent messages
    pub recent: u32,
    /// Number of unseen messages
    pub unseen: u32,
    /// UIDVALIDITY
    pub uidvalidity: u32,
    /// Next UID
    pub uidnext: u32,
}

impl CondStoreStatus {
    /// Format as IMAP STATUS response
    ///
    /// Example: * STATUS INBOX (MESSAGES 5 HIGHESTMODSEQ 12345)
    pub fn to_status_response(&self) -> String {
        format!(
            "* STATUS {} (MESSAGES {} RECENT {} UNSEEN {} UIDVALIDITY {} UIDNEXT {} HIGHESTMODSEQ {})",
            self.mailbox,
            self.exists,
            self.recent,
            self.unseen,
            self.uidvalidity,
            self.uidnext,
            self.highestmodseq
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_condstore_state() {
        let state = CondStoreState::Disabled;
        assert!(!state.is_enabled());

        let state = CondStoreState::Enabled;
        assert!(state.is_enabled());

        let state = CondStoreState::ImplicitlyEnabled;
        assert!(state.is_enabled());
    }

    #[test]
    fn test_condstore_state_equality() {
        assert_eq!(CondStoreState::Disabled, CondStoreState::Disabled);
        assert_eq!(CondStoreState::Enabled, CondStoreState::Enabled);
        assert_ne!(CondStoreState::Disabled, CondStoreState::Enabled);
    }

    #[test]
    fn test_changed_since_parse() {
        let cs = ChangedSince::parse("12345").expect("valid CHANGEDSINCE value");
        assert_eq!(cs.modseq.value(), 12345);

        assert!(ChangedSince::parse("0").is_err());
        assert!(ChangedSince::parse("abc").is_err());
    }

    #[test]
    fn test_changed_since_parse_with_whitespace() {
        let cs =
            ChangedSince::parse("  12345  ").expect("CHANGEDSINCE with whitespace should parse");
        assert_eq!(cs.modseq.value(), 12345);
    }

    #[test]
    fn test_changed_since_parse_large_value() {
        let cs = ChangedSince::parse("18446744073709551615")
            .expect("CHANGEDSINCE with u64::MAX should parse");
        assert_eq!(cs.modseq.value(), u64::MAX);
    }

    #[test]
    fn test_changed_since_parse_invalid_values() {
        assert!(matches!(
            ChangedSince::parse("0").unwrap_err(),
            CondStoreError::ZeroModSeq
        ));
        assert!(matches!(
            ChangedSince::parse("abc").unwrap_err(),
            CondStoreError::InvalidModSeq(_)
        ));
        assert!(matches!(
            ChangedSince::parse("-1").unwrap_err(),
            CondStoreError::InvalidModSeq(_)
        ));
    }

    #[test]
    fn test_changed_since_matches() {
        let cs = ChangedSince::new(ModSeq::new(100));

        assert!(!cs.matches(ModSeq::new(50)));
        assert!(!cs.matches(ModSeq::new(100)));
        assert!(cs.matches(ModSeq::new(150)));
    }

    #[test]
    fn test_changed_since_matches_edge_cases() {
        let cs = ChangedSince::new(ModSeq::new(1));
        assert!(!cs.matches(ModSeq::new(1)));
        assert!(cs.matches(ModSeq::new(2)));

        let cs = ChangedSince::new(ModSeq::new(u64::MAX - 1));
        assert!(cs.matches(ModSeq::new(u64::MAX)));
    }

    #[test]
    fn test_changed_since_clone() {
        let cs1 = ChangedSince::new(ModSeq::new(100));
        let cs2 = cs1;
        assert_eq!(cs1.modseq, cs2.modseq);
    }

    #[test]
    fn test_unchanged_since_parse() {
        let us = UnchangedSince::parse("(UNCHANGEDSINCE 12345)")
            .expect("UNCHANGEDSINCE in parens should parse");
        assert_eq!(us.modseq.value(), 12345);

        let us = UnchangedSince::parse("UNCHANGEDSINCE 12345")
            .expect("UNCHANGEDSINCE without parens should parse");
        assert_eq!(us.modseq.value(), 12345);

        assert!(UnchangedSince::parse("INVALID 12345").is_err());
        assert!(UnchangedSince::parse("UNCHANGEDSINCE 0").is_err());
    }

    #[test]
    fn test_unchanged_since_parse_case_insensitive() {
        let us = UnchangedSince::parse("unchangedsince 12345")
            .expect("lowercase unchangedsince should parse");
        assert_eq!(us.modseq.value(), 12345);

        let us = UnchangedSince::parse("UnChAnGeDsInCe 12345")
            .expect("mixed-case UnChAnGeDsInCe should parse");
        assert_eq!(us.modseq.value(), 12345);
    }

    #[test]
    fn test_unchanged_since_parse_with_multiple_spaces() {
        let us = UnchangedSince::parse("  UNCHANGEDSINCE   12345  ")
            .expect("UNCHANGEDSINCE with extra spaces should parse");
        assert_eq!(us.modseq.value(), 12345);
    }

    #[test]
    fn test_unchanged_since_parse_invalid() {
        assert!(UnchangedSince::parse("CHANGEDSINCE 12345").is_err());
        assert!(UnchangedSince::parse("12345").is_err());
        assert!(UnchangedSince::parse("UNCHANGEDSINCE").is_err());
        assert!(UnchangedSince::parse("UNCHANGEDSINCE abc").is_err());
    }

    #[test]
    fn test_unchanged_since_can_modify() {
        let us = UnchangedSince::new(ModSeq::new(100));

        assert!(us.can_modify(ModSeq::new(50)));
        assert!(us.can_modify(ModSeq::new(100)));
        assert!(!us.can_modify(ModSeq::new(150)));
    }

    #[test]
    fn test_unchanged_since_can_modify_edge_cases() {
        let us = UnchangedSince::new(ModSeq::new(1));
        assert!(us.can_modify(ModSeq::new(1)));
        assert!(!us.can_modify(ModSeq::new(2)));

        let us = UnchangedSince::new(ModSeq::new(u64::MAX));
        assert!(us.can_modify(ModSeq::new(u64::MAX)));
    }

    #[test]
    fn test_condstore_error_display() {
        let err = CondStoreError::InvalidModSeq("abc".to_string());
        assert_eq!(err.to_string(), "Invalid MODSEQ: abc");

        let err = CondStoreError::ZeroModSeq;
        assert_eq!(err.to_string(), "MODSEQ cannot be zero");

        let err = CondStoreError::NotEnabled;
        assert_eq!(err.to_string(), "CONDSTORE not enabled");
    }

    #[test]
    fn test_condstore_error_store_failed() {
        let err = CondStoreError::StoreFailedModified {
            failed_uids: vec![1, 2, 3],
        };
        assert!(err.to_string().contains("STORE failed"));
        assert!(err.to_string().contains("[1, 2, 3]"));
    }

    #[test]
    fn test_condstore_response() {
        let resp = CondStoreResponse::new(42, ModSeq::new(12345), 1);
        assert_eq!(
            resp.to_fetch_response(),
            "* 1 FETCH (UID 42 MODSEQ (12345))"
        );
    }

    #[test]
    fn test_condstore_response_multiple() {
        let resp1 = CondStoreResponse::new(1, ModSeq::new(100), 1);
        let resp2 = CondStoreResponse::new(2, ModSeq::new(200), 2);
        let resp3 = CondStoreResponse::new(3, ModSeq::new(300), 3);

        assert_eq!(resp1.to_fetch_response(), "* 1 FETCH (UID 1 MODSEQ (100))");
        assert_eq!(resp2.to_fetch_response(), "* 2 FETCH (UID 2 MODSEQ (200))");
        assert_eq!(resp3.to_fetch_response(), "* 3 FETCH (UID 3 MODSEQ (300))");
    }

    #[test]
    fn test_condstore_response_clone() {
        let resp1 = CondStoreResponse::new(42, ModSeq::new(12345), 1);
        let resp2 = resp1.clone();
        assert_eq!(resp1.uid, resp2.uid);
        assert_eq!(resp1.modseq, resp2.modseq);
        assert_eq!(resp1.seq, resp2.seq);
    }

    #[test]
    fn test_condstore_status() {
        let status = CondStoreStatus {
            mailbox: "INBOX".to_string(),
            highestmodseq: ModSeq::new(12345),
            exists: 5,
            recent: 2,
            unseen: 3,
            uidvalidity: 1,
            uidnext: 6,
        };

        let response = status.to_status_response();
        assert!(response.contains("INBOX"));
        assert!(response.contains("HIGHESTMODSEQ 12345"));
        assert!(response.contains("MESSAGES 5"));
    }

    #[test]
    fn test_condstore_status_format() {
        let status = CondStoreStatus {
            mailbox: "Sent".to_string(),
            highestmodseq: ModSeq::new(99999),
            exists: 100,
            recent: 5,
            unseen: 10,
            uidvalidity: 42,
            uidnext: 101,
        };

        let response = status.to_status_response();
        assert!(response.starts_with("* STATUS Sent"));
        assert!(response.contains("MESSAGES 100"));
        assert!(response.contains("RECENT 5"));
        assert!(response.contains("UNSEEN 10"));
        assert!(response.contains("UIDVALIDITY 42"));
        assert!(response.contains("UIDNEXT 101"));
        assert!(response.contains("HIGHESTMODSEQ 99999"));
    }

    #[test]
    fn test_condstore_status_clone() {
        let status1 = CondStoreStatus {
            mailbox: "INBOX".to_string(),
            highestmodseq: ModSeq::new(12345),
            exists: 5,
            recent: 2,
            unseen: 3,
            uidvalidity: 1,
            uidnext: 6,
        };
        let status2 = status1.clone();
        assert_eq!(status1.mailbox, status2.mailbox);
        assert_eq!(status1.highestmodseq, status2.highestmodseq);
    }

    #[test]
    fn test_condstore_status_zero_messages() {
        let status = CondStoreStatus {
            mailbox: "Empty".to_string(),
            highestmodseq: ModSeq::new(1),
            exists: 0,
            recent: 0,
            unseen: 0,
            uidvalidity: 1,
            uidnext: 1,
        };

        let response = status.to_status_response();
        assert!(response.contains("MESSAGES 0"));
        assert!(response.contains("RECENT 0"));
        assert!(response.contains("UNSEEN 0"));
    }
}

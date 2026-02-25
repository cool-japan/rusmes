//! IMAP QRESYNC Extension - RFC 7162
//!
//! This module implements Quick Resynchronization (QRESYNC) for efficient
//! mailbox synchronization. QRESYNC requires CONDSTORE and ENABLE support.
//!
//! Key features:
//! - SELECT/EXAMINE with QRESYNC parameters
//! - VANISHED responses for efficiently reporting expunged messages
//! - UID mapping for sequence number changes
//! - Known UIDs optimization

use rusmes_storage::ModSeq;
use std::fmt;

/// QRESYNC enablement state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QResyncState {
    /// QRESYNC not enabled
    Disabled,
    /// QRESYNC enabled via ENABLE command
    Enabled,
}

impl QResyncState {
    /// Check if QRESYNC is enabled
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// QRESYNC parameters for SELECT/EXAMINE commands
///
/// Format: QRESYNC (uidvalidity modseq [known-uids [seq-match-data]])
///
/// Example: SELECT INBOX (QRESYNC (67890007 20050715194045000 41,43:211,214:541))
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QResyncParams {
    /// Last known UIDVALIDITY value
    pub uidvalidity: u32,
    /// Last known modification sequence
    pub modseq: ModSeq,
    /// Optional set of known UIDs (for optimization)
    pub known_uids: Option<UidSet>,
    /// Optional sequence-to-UID mapping data
    pub seq_match_data: Option<SeqMatchData>,
}

impl QResyncParams {
    /// Create new QRESYNC parameters
    pub fn new(uidvalidity: u32, modseq: ModSeq) -> Self {
        Self {
            uidvalidity,
            modseq,
            known_uids: None,
            seq_match_data: None,
        }
    }

    /// Create QRESYNC parameters with known UIDs
    pub fn with_known_uids(uidvalidity: u32, modseq: ModSeq, known_uids: UidSet) -> Self {
        Self {
            uidvalidity,
            modseq,
            known_uids: Some(known_uids),
            seq_match_data: None,
        }
    }

    /// Create QRESYNC parameters with sequence match data
    pub fn with_seq_match_data(
        uidvalidity: u32,
        modseq: ModSeq,
        known_uids: UidSet,
        seq_match_data: SeqMatchData,
    ) -> Self {
        Self {
            uidvalidity,
            modseq,
            known_uids: Some(known_uids),
            seq_match_data: Some(seq_match_data),
        }
    }

    /// Parse QRESYNC parameters from command arguments
    ///
    /// Format: (uidvalidity modseq [known-uids [seq-match-data]])
    pub fn parse(args: &str) -> Result<Self, QResyncError> {
        let args = args.trim().trim_matches(|c| c == '(' || c == ')');
        let parts: Vec<&str> = args.split_whitespace().collect();

        if parts.len() < 2 {
            return Err(QResyncError::InvalidSyntax(args.to_string()));
        }

        // Parse uidvalidity
        let uidvalidity = parts[0]
            .parse::<u32>()
            .map_err(|_| QResyncError::InvalidUidValidity(parts[0].to_string()))?;

        // Parse modseq
        let modseq_value = parts[1]
            .parse::<u64>()
            .map_err(|_| QResyncError::InvalidModSeq(parts[1].to_string()))?;

        if modseq_value == 0 {
            return Err(QResyncError::ZeroModSeq);
        }

        let modseq = ModSeq::new(modseq_value);

        // Parse optional known-uids
        let known_uids = if parts.len() > 2 {
            Some(UidSet::parse(parts[2])?)
        } else {
            None
        };

        // Parse optional seq-match-data
        let seq_match_data = if parts.len() > 3 {
            Some(SeqMatchData::parse(parts[3])?)
        } else {
            None
        };

        Ok(Self {
            uidvalidity,
            modseq,
            known_uids,
            seq_match_data,
        })
    }

    /// Format as IMAP QRESYNC parameter
    pub fn to_imap_string(&self) -> String {
        let mut result = format!("({} {}", self.uidvalidity, self.modseq);

        if let Some(ref known_uids) = self.known_uids {
            result.push(' ');
            result.push_str(&known_uids.to_string());

            if let Some(ref seq_match_data) = self.seq_match_data {
                result.push(' ');
                result.push_str(&seq_match_data.to_string());
            }
        }

        result.push(')');
        result
    }
}

/// UID set representation
///
/// Examples: "1:5", "1,3,5", "1:100,200:*"
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UidSet {
    /// Raw UID set string
    ranges: Vec<UidRange>,
}

impl UidSet {
    /// Create new UID set
    pub fn new(ranges: Vec<UidRange>) -> Self {
        Self { ranges }
    }

    /// Parse UID set from string
    ///
    /// Examples: "1:5", "1,3,5", "1:100,200:*"
    pub fn parse(s: &str) -> Result<Self, QResyncError> {
        let parts: Vec<&str> = s.split(',').collect();
        let mut ranges = Vec::new();

        for part in parts {
            ranges.push(UidRange::parse(part)?);
        }

        if ranges.is_empty() {
            return Err(QResyncError::InvalidUidSet(s.to_string()));
        }

        Ok(Self { ranges })
    }

    /// Check if a UID is in this set
    pub fn contains(&self, uid: u32) -> bool {
        self.ranges.iter().any(|range| range.contains(uid))
    }

    /// Get all ranges
    pub fn ranges(&self) -> &[UidRange] {
        &self.ranges
    }
}

impl fmt::Display for UidSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ranges_str: Vec<String> = self.ranges.iter().map(|r| r.to_string()).collect();
        write!(f, "{}", ranges_str.join(","))
    }
}

/// UID range
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UidRange {
    /// Single UID
    Single(u32),
    /// Range start:end
    Range { start: u32, end: u32 },
    /// Range start:* (to highest)
    RangeToMax { start: u32 },
}

impl UidRange {
    /// Parse UID range from string
    pub fn parse(s: &str) -> Result<Self, QResyncError> {
        if s.contains(':') {
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() != 2 {
                return Err(QResyncError::InvalidUidRange(s.to_string()));
            }

            let start = parts[0]
                .parse::<u32>()
                .map_err(|_| QResyncError::InvalidUidRange(s.to_string()))?;

            if parts[1] == "*" {
                Ok(Self::RangeToMax { start })
            } else {
                let end = parts[1]
                    .parse::<u32>()
                    .map_err(|_| QResyncError::InvalidUidRange(s.to_string()))?;

                if start > end {
                    return Err(QResyncError::InvalidUidRange(s.to_string()));
                }

                Ok(Self::Range { start, end })
            }
        } else {
            let uid = s
                .parse::<u32>()
                .map_err(|_| QResyncError::InvalidUidRange(s.to_string()))?;
            Ok(Self::Single(uid))
        }
    }

    /// Check if this range contains the given UID
    pub fn contains(&self, uid: u32) -> bool {
        match self {
            Self::Single(u) => *u == uid,
            Self::Range { start, end } => uid >= *start && uid <= *end,
            Self::RangeToMax { start } => uid >= *start,
        }
    }
}

impl fmt::Display for UidRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Single(uid) => write!(f, "{}", uid),
            Self::Range { start, end } => write!(f, "{}:{}", start, end),
            Self::RangeToMax { start } => write!(f, "{}:*", start),
        }
    }
}

/// Sequence match data (sequence:UID pairs)
///
/// Example: "(1 2 3 4 5 6)" maps to UIDs in known-uids
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqMatchData {
    /// Sequence numbers
    sequences: Vec<u32>,
}

impl SeqMatchData {
    /// Create new sequence match data
    pub fn new(sequences: Vec<u32>) -> Self {
        Self { sequences }
    }

    /// Parse sequence match data from string
    ///
    /// Format: (seq1 seq2 seq3 ...)
    pub fn parse(s: &str) -> Result<Self, QResyncError> {
        let s = s.trim().trim_matches(|c| c == '(' || c == ')');
        let parts: Vec<&str> = s.split_whitespace().collect();

        let mut sequences = Vec::new();
        for part in parts {
            let seq = part
                .parse::<u32>()
                .map_err(|_| QResyncError::InvalidSeqMatchData(s.to_string()))?;
            sequences.push(seq);
        }

        Ok(Self { sequences })
    }

    /// Get sequences
    pub fn sequences(&self) -> &[u32] {
        &self.sequences
    }
}

impl fmt::Display for SeqMatchData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seqs: Vec<String> = self.sequences.iter().map(|s| s.to_string()).collect();
        write!(f, "({})", seqs.join(" "))
    }
}

/// VANISHED response for reporting expunged messages
///
/// VANISHED responses efficiently report UIDs that have been expunged
/// since the last synchronization point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VanishedResponse {
    /// Set of vanished (expunged) UIDs
    pub uids: UidSet,
    /// Whether this is an earlier response (used with EARLIER tag)
    pub earlier: bool,
}

impl VanishedResponse {
    /// Create new VANISHED response
    pub fn new(uids: UidSet) -> Self {
        Self {
            uids,
            earlier: false,
        }
    }

    /// Create VANISHED (EARLIER) response
    pub fn earlier(uids: UidSet) -> Self {
        Self {
            uids,
            earlier: true,
        }
    }

    /// Format as IMAP VANISHED response
    ///
    /// Examples:
    /// - * VANISHED 1:5,7,9:12
    /// - * VANISHED (EARLIER) 1:5,7,9:12
    pub fn to_imap_response(&self) -> String {
        if self.earlier {
            format!("* VANISHED (EARLIER) {}", self.uids)
        } else {
            format!("* VANISHED {}", self.uids)
        }
    }

    /// Parse VANISHED response
    pub fn parse(line: &str) -> Result<Self, QResyncError> {
        let line = line.trim();

        if !line.starts_with("* VANISHED") && !line.starts_with("VANISHED") {
            return Err(QResyncError::InvalidVanishedResponse(line.to_string()));
        }

        let line = line
            .trim_start_matches("* ")
            .trim_start_matches("VANISHED")
            .trim();

        let (earlier, uid_str) = if line.starts_with("(EARLIER)") {
            (true, line.trim_start_matches("(EARLIER)").trim())
        } else {
            (false, line)
        };

        let uids = UidSet::parse(uid_str)?;

        Ok(Self { uids, earlier })
    }
}

/// QRESYNC-related errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QResyncError {
    /// Invalid QRESYNC syntax
    InvalidSyntax(String),
    /// Invalid UIDVALIDITY value
    InvalidUidValidity(String),
    /// Invalid MODSEQ value
    InvalidModSeq(String),
    /// MODSEQ cannot be zero
    ZeroModSeq,
    /// Invalid UID set
    InvalidUidSet(String),
    /// Invalid UID range
    InvalidUidRange(String),
    /// Invalid sequence match data
    InvalidSeqMatchData(String),
    /// Invalid VANISHED response
    InvalidVanishedResponse(String),
    /// QRESYNC not enabled
    NotEnabled,
    /// CONDSTORE not enabled (required for QRESYNC)
    CondStoreRequired,
    /// UIDVALIDITY mismatch
    UidValidityMismatch {
        /// Expected UIDVALIDITY
        expected: u32,
        /// Actual UIDVALIDITY
        actual: u32,
    },
}

impl fmt::Display for QResyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QResyncError::InvalidSyntax(s) => write!(f, "Invalid QRESYNC syntax: {}", s),
            QResyncError::InvalidUidValidity(s) => write!(f, "Invalid UIDVALIDITY: {}", s),
            QResyncError::InvalidModSeq(s) => write!(f, "Invalid MODSEQ: {}", s),
            QResyncError::ZeroModSeq => write!(f, "MODSEQ cannot be zero"),
            QResyncError::InvalidUidSet(s) => write!(f, "Invalid UID set: {}", s),
            QResyncError::InvalidUidRange(s) => write!(f, "Invalid UID range: {}", s),
            QResyncError::InvalidSeqMatchData(s) => write!(f, "Invalid sequence match data: {}", s),
            QResyncError::InvalidVanishedResponse(s) => {
                write!(f, "Invalid VANISHED response: {}", s)
            }
            QResyncError::NotEnabled => write!(f, "QRESYNC not enabled"),
            QResyncError::CondStoreRequired => write!(f, "CONDSTORE required for QRESYNC"),
            QResyncError::UidValidityMismatch { expected, actual } => {
                write!(
                    f,
                    "UIDVALIDITY mismatch: expected {}, got {}",
                    expected, actual
                )
            }
        }
    }
}

impl std::error::Error for QResyncError {}

/// Quick resynchronization logic
///
/// This structure provides the logic for efficiently resynchronizing a mailbox
/// using QRESYNC parameters.
#[derive(Debug)]
pub struct QResyncLogic {
    /// Current UIDVALIDITY
    pub uidvalidity: u32,
    /// Highest MODSEQ in mailbox
    pub highest_modseq: ModSeq,
}

impl QResyncLogic {
    /// Create new QRESYNC logic handler
    pub fn new(uidvalidity: u32, highest_modseq: ModSeq) -> Self {
        Self {
            uidvalidity,
            highest_modseq,
        }
    }

    /// Validate QRESYNC parameters against current mailbox state
    ///
    /// Returns an error if UIDVALIDITY doesn't match
    pub fn validate_params(&self, params: &QResyncParams) -> Result<(), QResyncError> {
        if params.uidvalidity != self.uidvalidity {
            return Err(QResyncError::UidValidityMismatch {
                expected: params.uidvalidity,
                actual: self.uidvalidity,
            });
        }
        Ok(())
    }

    /// Determine which UIDs have been expunged since the given MODSEQ
    ///
    /// This should be called with the list of all UIDs that existed at the
    /// client's last known MODSEQ and the current list of UIDs in the mailbox.
    pub fn find_vanished_uids(&self, known_uids: &UidSet, current_uids: &[u32]) -> Vec<u32> {
        let mut vanished = Vec::new();

        // Check each range in the known UIDs set
        for range in known_uids.ranges() {
            match range {
                UidRange::Single(uid) => {
                    if !current_uids.contains(uid) {
                        vanished.push(*uid);
                    }
                }
                UidRange::Range { start, end } => {
                    for uid in *start..=*end {
                        if !current_uids.contains(&uid) {
                            vanished.push(uid);
                        }
                    }
                }
                UidRange::RangeToMax { start } => {
                    // For open-ended ranges, check from start to max known UID
                    let max_uid = current_uids.iter().max().copied().unwrap_or(*start);
                    for uid in *start..=max_uid {
                        if !current_uids.contains(&uid) {
                            vanished.push(uid);
                        }
                    }
                }
            }
        }

        vanished.sort_unstable();
        vanished
    }

    /// Create a VANISHED response from expunged UIDs
    ///
    /// Optimizes the list of UIDs into ranges for efficiency
    pub fn create_vanished_response(
        &self,
        vanished_uids: Vec<u32>,
        earlier: bool,
    ) -> Option<VanishedResponse> {
        if vanished_uids.is_empty() {
            return None;
        }

        let ranges = Self::compress_to_ranges(vanished_uids);
        let uid_set = UidSet::new(ranges);

        Some(if earlier {
            VanishedResponse::earlier(uid_set)
        } else {
            VanishedResponse::new(uid_set)
        })
    }

    /// Compress a list of UIDs into ranges
    ///
    /// Example: [1,2,3,5,6,7,10] -> [1:3, 5:7, 10]
    fn compress_to_ranges(mut uids: Vec<u32>) -> Vec<UidRange> {
        if uids.is_empty() {
            return Vec::new();
        }

        uids.sort_unstable();
        uids.dedup();

        let mut ranges = Vec::new();
        let mut range_start = uids[0];
        let mut range_end = uids[0];

        for &uid in &uids[1..] {
            if uid == range_end + 1 {
                // Continue current range
                range_end = uid;
            } else {
                // End current range and start new one
                if range_start == range_end {
                    ranges.push(UidRange::Single(range_start));
                } else {
                    ranges.push(UidRange::Range {
                        start: range_start,
                        end: range_end,
                    });
                }
                range_start = uid;
                range_end = uid;
            }
        }

        // Add final range
        if range_start == range_end {
            ranges.push(UidRange::Single(range_start));
        } else {
            ranges.push(UidRange::Range {
                start: range_start,
                end: range_end,
            });
        }

        ranges
    }

    /// Check if resynchronization is needed
    ///
    /// Returns true if the client's MODSEQ is outdated
    pub fn needs_resync(&self, client_modseq: ModSeq) -> bool {
        client_modseq < self.highest_modseq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qresync_state() {
        let state = QResyncState::Disabled;
        assert!(!state.is_enabled());

        let state = QResyncState::Enabled;
        assert!(state.is_enabled());
    }

    #[test]
    fn test_qresync_params_basic() {
        let params = QResyncParams::new(12345, ModSeq::new(67890));
        assert_eq!(params.uidvalidity, 12345);
        assert_eq!(params.modseq.value(), 67890);
        assert!(params.known_uids.is_none());
        assert!(params.seq_match_data.is_none());
    }

    #[test]
    fn test_qresync_params_parse_basic() {
        let params = QResyncParams::parse("(12345 67890)").expect("basic QRESYNC params parse");
        assert_eq!(params.uidvalidity, 12345);
        assert_eq!(params.modseq.value(), 67890);
        assert!(params.known_uids.is_none());
    }

    #[test]
    fn test_qresync_params_parse_with_known_uids() {
        let params = QResyncParams::parse("(12345 67890 1:100)")
            .expect("QRESYNC params with known UIDs parse");
        assert_eq!(params.uidvalidity, 12345);
        assert_eq!(params.modseq.value(), 67890);
        assert!(params.known_uids.is_some());
    }

    #[test]
    fn test_qresync_params_parse_full() {
        let params = QResyncParams::parse("(12345 67890 1:100 (1 2 3 4 5))")
            .expect("full QRESYNC params parse");
        assert_eq!(params.uidvalidity, 12345);
        assert_eq!(params.modseq.value(), 67890);
        assert!(params.known_uids.is_some());
        assert!(params.seq_match_data.is_some());
    }

    #[test]
    fn test_qresync_params_parse_invalid() {
        assert!(QResyncParams::parse("(12345)").is_err());
        assert!(QResyncParams::parse("(abc 67890)").is_err());
        assert!(QResyncParams::parse("(12345 0)").is_err());
    }

    #[test]
    fn test_qresync_params_to_imap_string() {
        let params = QResyncParams::new(12345, ModSeq::new(67890));
        assert_eq!(params.to_imap_string(), "(12345 67890)");

        let params = QResyncParams::with_known_uids(
            12345,
            ModSeq::new(67890),
            UidSet::parse("1:100").expect("valid UID set parse"),
        );
        assert_eq!(params.to_imap_string(), "(12345 67890 1:100)");
    }

    #[test]
    fn test_uid_range_parse_single() {
        let range = UidRange::parse("42").expect("single UID range parse");
        assert_eq!(range, UidRange::Single(42));
        assert!(range.contains(42));
        assert!(!range.contains(41));
        assert_eq!(range.to_string(), "42");
    }

    #[test]
    fn test_uid_range_parse_range() {
        let range = UidRange::parse("10:20").expect("UID range 10:20 parse");
        assert_eq!(range, UidRange::Range { start: 10, end: 20 });
        assert!(range.contains(10));
        assert!(range.contains(15));
        assert!(range.contains(20));
        assert!(!range.contains(9));
        assert!(!range.contains(21));
        assert_eq!(range.to_string(), "10:20");
    }

    #[test]
    fn test_uid_range_parse_to_max() {
        let range = UidRange::parse("100:*").expect("UID range 100:* parse");
        assert_eq!(range, UidRange::RangeToMax { start: 100 });
        assert!(range.contains(100));
        assert!(range.contains(1000));
        assert!(range.contains(u32::MAX));
        assert!(!range.contains(99));
        assert_eq!(range.to_string(), "100:*");
    }

    #[test]
    fn test_uid_range_parse_invalid() {
        assert!(UidRange::parse("abc").is_err());
        assert!(UidRange::parse("10:5").is_err());
        assert!(UidRange::parse("10:20:30").is_err());
    }

    #[test]
    fn test_uid_set_parse_single() {
        let set = UidSet::parse("42").expect("single UID set parse");
        assert_eq!(set.ranges.len(), 1);
        assert!(set.contains(42));
        assert!(!set.contains(41));
        assert_eq!(set.to_string(), "42");
    }

    #[test]
    fn test_uid_set_parse_multiple() {
        let set = UidSet::parse("1,3,5").expect("multiple single UIDs set parse");
        assert_eq!(set.ranges.len(), 3);
        assert!(set.contains(1));
        assert!(!set.contains(2));
        assert!(set.contains(3));
        assert!(!set.contains(4));
        assert!(set.contains(5));
        assert_eq!(set.to_string(), "1,3,5");
    }

    #[test]
    fn test_uid_set_parse_ranges() {
        let set = UidSet::parse("1:5,10:20,100:*").expect("UID set with ranges parse");
        assert_eq!(set.ranges.len(), 3);
        assert!(set.contains(1));
        assert!(set.contains(5));
        assert!(!set.contains(6));
        assert!(set.contains(10));
        assert!(set.contains(20));
        assert!(!set.contains(21));
        assert!(set.contains(100));
        assert!(set.contains(1000));
        assert_eq!(set.to_string(), "1:5,10:20,100:*");
    }

    #[test]
    fn test_uid_set_parse_mixed() {
        let set = UidSet::parse("1,5:10,15,20:*").expect("mixed UID set parse");
        assert_eq!(set.ranges.len(), 4);
        assert!(set.contains(1));
        assert!(set.contains(7));
        assert!(set.contains(15));
        assert!(set.contains(100));
        assert_eq!(set.to_string(), "1,5:10,15,20:*");
    }

    #[test]
    fn test_seq_match_data_parse() {
        let data = SeqMatchData::parse("(1 2 3 4 5)").expect("SeqMatchData with parens parse");
        assert_eq!(data.sequences.len(), 5);
        assert_eq!(data.sequences[0], 1);
        assert_eq!(data.sequences[4], 5);
        assert_eq!(data.to_string(), "(1 2 3 4 5)");
    }

    #[test]
    fn test_seq_match_data_parse_without_parens() {
        let data = SeqMatchData::parse("1 2 3").expect("SeqMatchData without parens parse");
        assert_eq!(data.sequences.len(), 3);
        assert_eq!(data.sequences(), &[1, 2, 3]);
    }

    #[test]
    fn test_vanished_response_new() {
        let uids = UidSet::parse("1:5,7,9:12").expect("UID set parse for VANISHED test");
        let response = VanishedResponse::new(uids);
        assert!(!response.earlier);
        assert_eq!(response.to_imap_response(), "* VANISHED 1:5,7,9:12");
    }

    #[test]
    fn test_vanished_response_earlier() {
        let uids = UidSet::parse("1:5,7,9:12").expect("UID set parse for VANISHED EARLIER test");
        let response = VanishedResponse::earlier(uids);
        assert!(response.earlier);
        assert_eq!(
            response.to_imap_response(),
            "* VANISHED (EARLIER) 1:5,7,9:12"
        );
    }

    #[test]
    fn test_vanished_response_parse() {
        let response =
            VanishedResponse::parse("* VANISHED 1:5,7").expect("VANISHED response parse");
        assert!(!response.earlier);
        assert!(response.uids.contains(1));
        assert!(response.uids.contains(5));
        assert!(response.uids.contains(7));

        let response = VanishedResponse::parse("* VANISHED (EARLIER) 1:5,7")
            .expect("VANISHED (EARLIER) response parse");
        assert!(response.earlier);
        assert!(response.uids.contains(1));
    }

    #[test]
    fn test_vanished_response_parse_without_star() {
        let response =
            VanishedResponse::parse("VANISHED 1:5").expect("VANISHED without * prefix parse");
        assert!(!response.earlier);
        assert!(response.uids.contains(1));
    }

    #[test]
    fn test_qresync_error_display() {
        let err = QResyncError::NotEnabled;
        assert_eq!(err.to_string(), "QRESYNC not enabled");

        let err = QResyncError::ZeroModSeq;
        assert_eq!(err.to_string(), "MODSEQ cannot be zero");

        let err = QResyncError::UidValidityMismatch {
            expected: 100,
            actual: 200,
        };
        assert_eq!(
            err.to_string(),
            "UIDVALIDITY mismatch: expected 100, got 200"
        );
    }

    #[test]
    fn test_uid_set_empty() {
        assert!(UidSet::parse("").is_err());
    }

    #[test]
    fn test_qresync_params_with_seq_match_data() {
        let known_uids = UidSet::parse("1:100").expect("UID set 1:100 parse");
        let seq_match = SeqMatchData::new(vec![1, 2, 3]);
        let params =
            QResyncParams::with_seq_match_data(12345, ModSeq::new(67890), known_uids, seq_match);
        assert_eq!(params.uidvalidity, 12345);
        assert!(params.seq_match_data.is_some());
        assert_eq!(params.to_imap_string(), "(12345 67890 1:100 (1 2 3))");
    }

    #[test]
    fn test_qresync_logic_new() {
        let logic = QResyncLogic::new(12345, ModSeq::new(67890));
        assert_eq!(logic.uidvalidity, 12345);
        assert_eq!(logic.highest_modseq.value(), 67890);
    }

    #[test]
    fn test_qresync_logic_validate_params_success() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let params = QResyncParams::new(12345, ModSeq::new(50));
        assert!(logic.validate_params(&params).is_ok());
    }

    #[test]
    fn test_qresync_logic_validate_params_mismatch() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let params = QResyncParams::new(99999, ModSeq::new(50));
        let result = logic.validate_params(&params);
        assert!(result.is_err());
        match result.unwrap_err() {
            QResyncError::UidValidityMismatch { expected, actual } => {
                assert_eq!(expected, 99999);
                assert_eq!(actual, 12345);
            }
            _ => panic!("Expected UidValidityMismatch error"),
        }
    }

    #[test]
    fn test_qresync_logic_find_vanished_uids_single() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let known_uids = UidSet::parse("1,2,3,4,5").expect("UID set 1,2,3,4,5 parse");
        let current_uids = vec![1, 3, 5];
        let vanished = logic.find_vanished_uids(&known_uids, &current_uids);
        assert_eq!(vanished, vec![2, 4]);
    }

    #[test]
    fn test_qresync_logic_find_vanished_uids_range() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let known_uids = UidSet::parse("1:10").expect("UID set 1:10 parse");
        let current_uids = vec![1, 2, 5, 6, 7, 10];
        let vanished = logic.find_vanished_uids(&known_uids, &current_uids);
        assert_eq!(vanished, vec![3, 4, 8, 9]);
    }

    #[test]
    fn test_qresync_logic_find_vanished_uids_none() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let known_uids = UidSet::parse("1,2,3").expect("UID set 1,2,3 parse");
        let current_uids = vec![1, 2, 3];
        let vanished = logic.find_vanished_uids(&known_uids, &current_uids);
        assert!(vanished.is_empty());
    }

    #[test]
    fn test_qresync_logic_find_vanished_uids_mixed() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let known_uids = UidSet::parse("1:5,10,15:20").expect("UID set 1:5,10,15:20 parse");
        let current_uids = vec![1, 3, 5, 15, 17, 20];
        let vanished = logic.find_vanished_uids(&known_uids, &current_uids);
        assert_eq!(vanished, vec![2, 4, 10, 16, 18, 19]);
    }

    #[test]
    fn test_qresync_logic_compress_to_ranges_single() {
        let ranges = QResyncLogic::compress_to_ranges(vec![5]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], UidRange::Single(5));
    }

    #[test]
    fn test_qresync_logic_compress_to_ranges_consecutive() {
        let ranges = QResyncLogic::compress_to_ranges(vec![1, 2, 3, 4, 5]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], UidRange::Range { start: 1, end: 5 });
    }

    #[test]
    fn test_qresync_logic_compress_to_ranges_gaps() {
        let ranges = QResyncLogic::compress_to_ranges(vec![1, 3, 5, 7]);
        assert_eq!(ranges.len(), 4);
        assert_eq!(ranges[0], UidRange::Single(1));
        assert_eq!(ranges[1], UidRange::Single(3));
        assert_eq!(ranges[2], UidRange::Single(5));
        assert_eq!(ranges[3], UidRange::Single(7));
    }

    #[test]
    fn test_qresync_logic_compress_to_ranges_mixed() {
        let ranges = QResyncLogic::compress_to_ranges(vec![1, 2, 3, 5, 7, 8, 9, 15]);
        assert_eq!(ranges.len(), 4);
        assert_eq!(ranges[0], UidRange::Range { start: 1, end: 3 });
        assert_eq!(ranges[1], UidRange::Single(5));
        assert_eq!(ranges[2], UidRange::Range { start: 7, end: 9 });
        assert_eq!(ranges[3], UidRange::Single(15));
    }

    #[test]
    fn test_qresync_logic_compress_to_ranges_unordered() {
        let ranges = QResyncLogic::compress_to_ranges(vec![5, 1, 3, 2, 4]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], UidRange::Range { start: 1, end: 5 });
    }

    #[test]
    fn test_qresync_logic_compress_to_ranges_duplicates() {
        let ranges = QResyncLogic::compress_to_ranges(vec![1, 2, 2, 3, 3, 4]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], UidRange::Range { start: 1, end: 4 });
    }

    #[test]
    fn test_qresync_logic_create_vanished_response_empty() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let response = logic.create_vanished_response(vec![], false);
        assert!(response.is_none());
    }

    #[test]
    fn test_qresync_logic_create_vanished_response_single() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let response = logic
            .create_vanished_response(vec![5], false)
            .expect("VANISHED response for single UID");
        assert!(!response.earlier);
        assert_eq!(response.to_imap_response(), "* VANISHED 5");
    }

    #[test]
    fn test_qresync_logic_create_vanished_response_range() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let response = logic
            .create_vanished_response(vec![1, 2, 3, 4, 5], false)
            .expect("VANISHED response for sequential UIDs");
        assert_eq!(response.to_imap_response(), "* VANISHED 1:5");
    }

    #[test]
    fn test_qresync_logic_create_vanished_response_earlier() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let response = logic
            .create_vanished_response(vec![1, 2, 3], true)
            .expect("VANISHED (EARLIER) response");
        assert!(response.earlier);
        assert_eq!(response.to_imap_response(), "* VANISHED (EARLIER) 1:3");
    }

    #[test]
    fn test_qresync_logic_create_vanished_response_mixed() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        let response = logic
            .create_vanished_response(vec![1, 2, 3, 5, 7, 8, 9], false)
            .expect("VANISHED response for mixed UIDs");
        assert_eq!(response.to_imap_response(), "* VANISHED 1:3,5,7:9");
    }

    #[test]
    fn test_qresync_logic_needs_resync() {
        let logic = QResyncLogic::new(12345, ModSeq::new(100));
        assert!(logic.needs_resync(ModSeq::new(50)));
        assert!(logic.needs_resync(ModSeq::new(99)));
        assert!(!logic.needs_resync(ModSeq::new(100)));
        assert!(!logic.needs_resync(ModSeq::new(101)));
    }

    #[test]
    fn test_qresync_integration_full_resync() {
        // Simulate a full QRESYNC operation
        let logic = QResyncLogic::new(12345, ModSeq::new(200));

        // Client's last known state
        let params = QResyncParams::with_known_uids(
            12345,
            ModSeq::new(100),
            UidSet::parse("1:50").expect("UID set 1:50 parse"),
        );

        // Validate
        assert!(logic.validate_params(&params).is_ok());
        assert!(logic.needs_resync(params.modseq));

        // Current mailbox state (some messages deleted)
        let current_uids: Vec<u32> = (1..=50).filter(|&n| n % 3 != 0).collect();

        // Find vanished UIDs
        let known = params
            .known_uids
            .as_ref()
            .expect("known_uids should be set");
        let vanished = logic.find_vanished_uids(known, &current_uids);
        assert!(!vanished.is_empty());

        // Create VANISHED response
        let response = logic.create_vanished_response(vanished, true);
        assert!(response.is_some());
        assert!(response.expect("VANISHED response should be Some").earlier);
    }

    #[test]
    fn test_uid_range_boundary_values() {
        // Test with boundary values
        let range = UidRange::Range {
            start: 1,
            end: u32::MAX,
        };
        assert!(range.contains(1));
        assert!(range.contains(u32::MAX));
        assert!(range.contains(u32::MAX / 2));
    }

    #[test]
    fn test_uid_set_large_range() {
        let set = UidSet::parse("1:4294967295").expect("UID set spanning full u32 range parse");
        assert!(set.contains(1));
        assert!(set.contains(1000000));
        assert!(set.contains(u32::MAX));
    }

    #[test]
    fn test_vanished_response_roundtrip() {
        let original =
            VanishedResponse::earlier(UidSet::parse("1:5,10,20:25").expect("UID set parse"));
        let imap_str = original.to_imap_response();
        let parsed = VanishedResponse::parse(&imap_str).expect("roundtrip VANISHED response parse");
        assert_eq!(parsed.earlier, original.earlier);
        assert_eq!(parsed.to_imap_response(), original.to_imap_response());
    }

    #[test]
    fn test_qresync_params_roundtrip() {
        let original = QResyncParams::with_seq_match_data(
            12345,
            ModSeq::new(67890),
            UidSet::parse("1:100,200:300").expect("UID set 1:100,200:300 parse"),
            SeqMatchData::new(vec![1, 2, 3, 4, 5]),
        );
        let imap_str = original.to_imap_string();
        let parsed = QResyncParams::parse(&imap_str).expect("roundtrip QRESYNC params parse");
        assert_eq!(parsed.uidvalidity, original.uidvalidity);
        assert_eq!(parsed.modseq.value(), original.modseq.value());
    }
}

//! SMTP CHUNKING/BDAT Extension - RFC 3030
//!
//! This module implements the BDAT command for binary data transfer,
//! which is more efficient than DATA for large messages.
//!
//! # Overview
//!
//! The CHUNKING extension allows clients to send message data in chunks without
//! the need for dot-stuffing (transparency) that is required by the DATA command.
//! This is especially useful for:
//! - Binary data transfer (no need to escape lines starting with '.')
//! - Large messages that can be sent in multiple chunks
//! - More efficient transfer of already-encoded MIME messages
//!
//! # RFC 3030 Compliance
//!
//! This implementation follows RFC 3030 and provides:
//! - `BdatCommand`: Parser for BDAT commands with chunk size and LAST flag
//! - `BdatState`: State machine for accumulating chunks and validating message size
//! - `BdatError`: Comprehensive error handling for all edge cases
//!
//! # Usage Example
//!
//! ```rust
//! use rusmes_smtp::{BdatCommand, BdatState};
//!
//! // Parse BDAT command
//! let cmd = BdatCommand::parse("1024 LAST").expect("valid BDAT parse");
//! assert_eq!(cmd.chunk_size, 1024);
//! assert!(cmd.last);
//!
//! // Accumulate message chunks
//! let mut state = BdatState::new(10 * 1024 * 1024); // 10MB max
//! state.add_chunk(b"First chunk".to_vec(), false).expect("chunk add");
//! state.add_chunk(b" Second chunk".to_vec(), true).expect("last chunk add");
//!
//! // Get complete message
//! let message = state.into_message().expect("complete message");
//! assert_eq!(message, b"First chunk Second chunk");
//! ```
//!
//! # Key Features
//!
//! - **No Dot-Stuffing**: Binary data can be transferred without transparency
//! - **Chunk Validation**: Validates that received chunk size matches declared size
//! - **Size Limits**: Enforces maximum message size across all chunks
//! - **Error Handling**: Comprehensive errors for all failure modes
//! - **LAST Flag**: Proper handling of the final chunk marker
//!
//! # Integration with SMTP Server
//!
//! The SMTP server advertises CHUNKING in EHLO and handles BDAT commands:
//! 1. Client sends: `BDAT 1024`
//! 2. Server reads exactly 1024 bytes
//! 3. Server responds: `250 1024 octets received`
//! 4. Repeat for more chunks or send `BDAT 512 LAST` for final chunk

use std::fmt;

/// BDAT command for chunked message transfer
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BdatCommand {
    /// Size of this chunk in octets
    pub chunk_size: usize,
    /// Whether this is the last chunk
    pub last: bool,
}

impl BdatCommand {
    /// Create a new BDAT command
    pub fn new(chunk_size: usize, last: bool) -> Self {
        Self { chunk_size, last }
    }

    /// Parse BDAT command from arguments
    ///
    /// Format: BDAT `<chunk-size>` \[LAST\]
    pub fn parse(args: &str) -> Result<Self, BdatError> {
        let parts: Vec<&str> = args.split_whitespace().collect();

        if parts.is_empty() {
            return Err(BdatError::MissingChunkSize);
        }

        let chunk_size = parts[0]
            .parse::<usize>()
            .map_err(|_| BdatError::InvalidChunkSize(parts[0].to_string()))?;

        if chunk_size == 0 {
            return Err(BdatError::ZeroChunkSize);
        }

        let last = parts.get(1).is_some_and(|s| s.eq_ignore_ascii_case("LAST"));

        // Check for invalid extra arguments
        if parts.len() > 2 || (parts.len() == 2 && !last) {
            return Err(BdatError::InvalidSyntax);
        }

        Ok(Self::new(chunk_size, last))
    }
}

impl fmt::Display for BdatCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.last {
            write!(f, "BDAT {} LAST", self.chunk_size)
        } else {
            write!(f, "BDAT {}", self.chunk_size)
        }
    }
}

/// State machine for BDAT message accumulation
#[derive(Debug, Clone)]
pub struct BdatState {
    /// Accumulated message data
    chunks: Vec<u8>,
    /// Total size accumulated so far
    total_size: usize,
    /// Maximum allowed message size
    max_size: usize,
    /// Whether we've received the LAST chunk
    complete: bool,
}

impl BdatState {
    /// Create a new BDAT state
    pub fn new(max_size: usize) -> Self {
        Self {
            chunks: Vec::new(),
            total_size: 0,
            max_size,
            complete: false,
        }
    }

    /// Add a chunk of data
    pub fn add_chunk(&mut self, data: Vec<u8>, last: bool) -> Result<(), BdatError> {
        if self.complete {
            return Err(BdatError::AlreadyComplete);
        }

        let chunk_size = data.len();

        // Check size limit
        if self.total_size + chunk_size > self.max_size {
            return Err(BdatError::MessageTooLarge {
                current: self.total_size + chunk_size,
                max: self.max_size,
            });
        }

        self.chunks.extend(data);
        self.total_size += chunk_size;
        self.complete = last;

        Ok(())
    }

    /// Add a chunk with size validation
    ///
    /// This validates that the actual data size matches the expected size from BDAT command
    pub fn add_chunk_with_validation(
        &mut self,
        data: Vec<u8>,
        expected_size: usize,
        last: bool,
    ) -> Result<(), BdatError> {
        let actual_size = data.len();
        if actual_size != expected_size {
            return Err(BdatError::ChunkSizeMismatch {
                expected: expected_size,
                actual: actual_size,
            });
        }

        self.add_chunk(data, last)
    }

    /// Check if the message is complete
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Get the total size accumulated
    pub fn total_size(&self) -> usize {
        self.total_size
    }

    /// Get the maximum size allowed
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Consume the state and return the complete message
    pub fn into_message(self) -> Result<Vec<u8>, BdatError> {
        if !self.complete {
            return Err(BdatError::Incomplete);
        }
        Ok(self.chunks)
    }

    /// Get a reference to the accumulated data (for inspection)
    pub fn data(&self) -> &[u8] {
        &self.chunks
    }

    /// Reset the state to accept a new message
    pub fn reset(&mut self) {
        self.chunks.clear();
        self.total_size = 0;
        self.complete = false;
    }
}

/// BDAT-related errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BdatError {
    /// Missing chunk size argument
    MissingChunkSize,
    /// Invalid chunk size format
    InvalidChunkSize(String),
    /// Chunk size is zero
    ZeroChunkSize,
    /// Invalid BDAT syntax
    InvalidSyntax,
    /// Message size exceeds limit
    MessageTooLarge { current: usize, max: usize },
    /// Already received LAST chunk
    AlreadyComplete,
    /// Message not complete (no LAST chunk yet)
    Incomplete,
    /// Received chunk size doesn't match actual data size
    ChunkSizeMismatch { expected: usize, actual: usize },
}

impl fmt::Display for BdatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BdatError::MissingChunkSize => write!(f, "Missing chunk size"),
            BdatError::InvalidChunkSize(s) => write!(f, "Invalid chunk size: {}", s),
            BdatError::ZeroChunkSize => write!(f, "Chunk size cannot be zero"),
            BdatError::InvalidSyntax => write!(f, "Invalid BDAT syntax"),
            BdatError::MessageTooLarge { current, max } => {
                write!(
                    f,
                    "Message too large: {} bytes exceeds {} bytes",
                    current, max
                )
            }
            BdatError::AlreadyComplete => write!(f, "Message already complete"),
            BdatError::Incomplete => write!(f, "Message incomplete (no LAST chunk)"),
            BdatError::ChunkSizeMismatch { expected, actual } => {
                write!(
                    f,
                    "Chunk size mismatch: expected {} bytes, got {}",
                    expected, actual
                )
            }
        }
    }
}

impl std::error::Error for BdatError {}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== BdatCommand Parsing Tests =====

    #[test]
    fn test_bdat_parse_basic() {
        let cmd = BdatCommand::parse("1024").expect("valid BDAT parse without LAST");
        assert_eq!(cmd.chunk_size, 1024);
        assert!(!cmd.last);
    }

    #[test]
    fn test_bdat_parse_with_last() {
        let cmd_last = BdatCommand::parse("512 LAST").expect("valid BDAT parse with LAST");
        assert_eq!(cmd_last.chunk_size, 512);
        assert!(cmd_last.last);
    }

    #[test]
    fn test_bdat_parse_last_case_insensitive() {
        let cmd_upper =
            BdatCommand::parse("256 LAST").expect("valid BDAT parse with uppercase LAST");
        assert_eq!(cmd_upper.chunk_size, 256);
        assert!(cmd_upper.last);

        let cmd_lower =
            BdatCommand::parse("128 last").expect("valid BDAT parse with lowercase last");
        assert_eq!(cmd_lower.chunk_size, 128);
        assert!(cmd_lower.last);

        let cmd_mixed =
            BdatCommand::parse("64 LaSt").expect("valid BDAT parse with mixed-case LaSt");
        assert_eq!(cmd_mixed.chunk_size, 64);
        assert!(cmd_mixed.last);
    }

    #[test]
    fn test_bdat_parse_large_chunk() {
        let cmd = BdatCommand::parse("1073741824").expect("valid BDAT parse for 1GB chunk"); // 1GB
        assert_eq!(cmd.chunk_size, 1073741824);
        assert!(!cmd.last);
    }

    #[test]
    fn test_bdat_parse_with_extra_whitespace() {
        let cmd =
            BdatCommand::parse("  512   LAST  ").expect("valid BDAT parse with extra whitespace");
        assert_eq!(cmd.chunk_size, 512);
        assert!(cmd.last);
    }

    #[test]
    fn test_bdat_parse_missing_chunk_size() {
        assert!(matches!(
            BdatCommand::parse(""),
            Err(BdatError::MissingChunkSize)
        ));

        assert!(matches!(
            BdatCommand::parse("   "),
            Err(BdatError::MissingChunkSize)
        ));
    }

    #[test]
    fn test_bdat_parse_invalid_chunk_size() {
        assert!(matches!(
            BdatCommand::parse("abc"),
            Err(BdatError::InvalidChunkSize(_))
        ));

        assert!(matches!(
            BdatCommand::parse("12.34"),
            Err(BdatError::InvalidChunkSize(_))
        ));

        assert!(matches!(
            BdatCommand::parse("-100"),
            Err(BdatError::InvalidChunkSize(_))
        ));
    }

    #[test]
    fn test_bdat_parse_zero_chunk_size() {
        assert!(matches!(
            BdatCommand::parse("0"),
            Err(BdatError::ZeroChunkSize)
        ));

        assert!(matches!(
            BdatCommand::parse("0 LAST"),
            Err(BdatError::ZeroChunkSize)
        ));
    }

    #[test]
    fn test_bdat_parse_invalid_syntax() {
        // Invalid second argument
        assert!(matches!(
            BdatCommand::parse("100 INVALID"),
            Err(BdatError::InvalidSyntax)
        ));

        // Too many arguments
        assert!(matches!(
            BdatCommand::parse("100 LAST EXTRA"),
            Err(BdatError::InvalidSyntax)
        ));
    }

    // ===== BdatCommand Display Tests =====

    #[test]
    fn test_bdat_display_without_last() {
        let cmd = BdatCommand::new(1024, false);
        assert_eq!(cmd.to_string(), "BDAT 1024");
    }

    #[test]
    fn test_bdat_display_with_last() {
        let cmd_last = BdatCommand::new(512, true);
        assert_eq!(cmd_last.to_string(), "BDAT 512 LAST");
    }

    // ===== BdatState Tests =====

    #[test]
    fn test_bdat_state_new() {
        let state = BdatState::new(1024);
        assert_eq!(state.total_size(), 0);
        assert!(!state.is_complete());
        assert_eq!(state.data().len(), 0);
    }

    #[test]
    fn test_bdat_state_single_chunk() {
        let mut state = BdatState::new(1024);
        state
            .add_chunk(b"Hello World".to_vec(), true)
            .expect("single chunk add should succeed");

        assert!(state.is_complete());
        assert_eq!(state.total_size(), 11);
        assert_eq!(state.data(), b"Hello World");

        let message = state.into_message().expect("complete message extraction");
        assert_eq!(message, b"Hello World");
    }

    #[test]
    fn test_bdat_state_multiple_chunks() {
        let mut state = BdatState::new(1024);

        // Add first chunk
        state
            .add_chunk(b"Hello ".to_vec(), false)
            .expect("first chunk add should succeed");
        assert!(!state.is_complete());
        assert_eq!(state.total_size(), 6);

        // Add second chunk
        state
            .add_chunk(b"World".to_vec(), false)
            .expect("second chunk add should succeed");
        assert!(!state.is_complete());
        assert_eq!(state.total_size(), 11);

        // Add final chunk
        state
            .add_chunk(b"!".to_vec(), true)
            .expect("final chunk add should succeed");
        assert!(state.is_complete());
        assert_eq!(state.total_size(), 12);

        // Get complete message
        let message = state.into_message().expect("complete message extraction");
        assert_eq!(message, b"Hello World!");
    }

    #[test]
    fn test_bdat_state_empty_last_chunk() {
        let mut state = BdatState::new(1024);

        state
            .add_chunk(b"Data".to_vec(), false)
            .expect("data chunk add should succeed");
        assert!(!state.is_complete());

        // Empty LAST chunk is valid
        state
            .add_chunk(Vec::new(), true)
            .expect("empty LAST chunk should be valid");
        assert!(state.is_complete());
        assert_eq!(state.total_size(), 4);
    }

    #[test]
    fn test_bdat_state_binary_data() {
        let mut state = BdatState::new(1024);

        // Binary data including null bytes
        let binary_data = vec![0x00, 0xFF, 0x01, 0x02, 0x03, 0x00, 0xFE];
        state
            .add_chunk(binary_data.clone(), true)
            .expect("binary chunk add should succeed");

        assert!(state.is_complete());
        assert_eq!(state.total_size(), 7);

        let message = state.into_message().expect("complete message extraction");
        assert_eq!(message, binary_data);
    }

    #[test]
    fn test_bdat_state_size_limit_exact() {
        let mut state = BdatState::new(10);

        // Exactly at the limit
        state
            .add_chunk(b"1234567890".to_vec(), true)
            .expect("chunk at exact size limit should succeed");
        assert_eq!(state.total_size(), 10);
        assert!(state.is_complete());
    }

    #[test]
    fn test_bdat_state_size_limit_exceeded() {
        let mut state = BdatState::new(10);

        // This should exceed the limit
        let result = state.add_chunk(b"12345678901".to_vec(), true);
        assert!(matches!(
            result,
            Err(BdatError::MessageTooLarge {
                current: 11,
                max: 10
            })
        ));
    }

    #[test]
    fn test_bdat_state_size_limit_multiple_chunks() {
        let mut state = BdatState::new(20);

        state
            .add_chunk(b"1234567890".to_vec(), false)
            .expect("first chunk within limit should succeed");
        assert_eq!(state.total_size(), 10);

        // This chunk would exceed the limit
        let result = state.add_chunk(b"12345678901".to_vec(), false);
        assert!(matches!(
            result,
            Err(BdatError::MessageTooLarge {
                current: 21,
                max: 20
            })
        ));
    }

    #[test]
    fn test_bdat_state_already_complete() {
        let mut state = BdatState::new(1024);

        state
            .add_chunk(b"Data".to_vec(), true)
            .expect("LAST chunk add should succeed");
        assert!(state.is_complete());

        // Try to add another chunk after LAST
        let result = state.add_chunk(b"More".to_vec(), false);
        assert!(matches!(result, Err(BdatError::AlreadyComplete)));
    }

    #[test]
    fn test_bdat_state_incomplete() {
        let mut state = BdatState::new(1024);

        state
            .add_chunk(b"Partial".to_vec(), false)
            .expect("partial chunk add should succeed");
        assert!(!state.is_complete());

        // Try to get message before LAST
        let result = state.into_message();
        assert!(matches!(result, Err(BdatError::Incomplete)));
    }

    #[test]
    fn test_bdat_state_data_reference() {
        let mut state = BdatState::new(1024);

        state
            .add_chunk(b"Test".to_vec(), false)
            .expect("first chunk add should succeed");
        assert_eq!(state.data(), b"Test");

        state
            .add_chunk(b" Data".to_vec(), false)
            .expect("second chunk add should succeed");
        assert_eq!(state.data(), b"Test Data");
    }

    // ===== BdatError Display Tests =====

    #[test]
    fn test_bdat_error_display_missing_chunk_size() {
        let err = BdatError::MissingChunkSize;
        assert_eq!(err.to_string(), "Missing chunk size");
    }

    #[test]
    fn test_bdat_error_display_invalid_chunk_size() {
        let err = BdatError::InvalidChunkSize("abc".to_string());
        assert_eq!(err.to_string(), "Invalid chunk size: abc");
    }

    #[test]
    fn test_bdat_error_display_zero_chunk_size() {
        let err = BdatError::ZeroChunkSize;
        assert_eq!(err.to_string(), "Chunk size cannot be zero");
    }

    #[test]
    fn test_bdat_error_display_invalid_syntax() {
        let err = BdatError::InvalidSyntax;
        assert_eq!(err.to_string(), "Invalid BDAT syntax");
    }

    #[test]
    fn test_bdat_error_display_message_too_large() {
        let err = BdatError::MessageTooLarge {
            current: 1000,
            max: 500,
        };
        assert_eq!(
            err.to_string(),
            "Message too large: 1000 bytes exceeds 500 bytes"
        );
    }

    #[test]
    fn test_bdat_error_display_already_complete() {
        let err = BdatError::AlreadyComplete;
        assert_eq!(err.to_string(), "Message already complete");
    }

    #[test]
    fn test_bdat_error_display_incomplete() {
        let err = BdatError::Incomplete;
        assert_eq!(err.to_string(), "Message incomplete (no LAST chunk)");
    }

    #[test]
    fn test_bdat_error_display_chunk_size_mismatch() {
        let err = BdatError::ChunkSizeMismatch {
            expected: 100,
            actual: 95,
        };
        assert_eq!(
            err.to_string(),
            "Chunk size mismatch: expected 100 bytes, got 95"
        );
    }

    // ===== Integration Tests =====

    #[test]
    fn test_bdat_workflow_complete() {
        // Simulate complete BDAT workflow
        let cmd1 = BdatCommand::parse("11").expect("valid BDAT parse for 11-byte chunk");
        assert_eq!(cmd1.chunk_size, 11);
        assert!(!cmd1.last);

        let cmd2 = BdatCommand::parse("13 LAST").expect("valid BDAT parse for 13-byte LAST chunk");
        assert_eq!(cmd2.chunk_size, 13);
        assert!(cmd2.last);

        let mut state = BdatState::new(1024);

        state
            .add_chunk(b"First chunk".to_vec(), false)
            .expect("first chunk add should succeed");
        assert_eq!(state.total_size(), 11);

        state
            .add_chunk(b" second chunk".to_vec(), true)
            .expect("second (LAST) chunk add should succeed");
        assert_eq!(state.total_size(), 24);
        assert!(state.is_complete());

        let message = state.into_message().expect("complete message extraction");
        assert_eq!(message, b"First chunk second chunk");
    }

    #[test]
    fn test_bdat_clone() {
        let cmd = BdatCommand::new(100, true);
        let cloned = cmd.clone();
        assert_eq!(cmd, cloned);

        let mut state = BdatState::new(1024);
        state
            .add_chunk(b"test".to_vec(), false)
            .expect("chunk add before clone should succeed");
        let cloned_state = state.clone();
        assert_eq!(cloned_state.total_size(), state.total_size());
        assert_eq!(cloned_state.is_complete(), state.is_complete());
    }

    #[test]
    fn test_bdat_command_equality() {
        let cmd1 = BdatCommand::new(100, false);
        let cmd2 = BdatCommand::new(100, false);
        let cmd3 = BdatCommand::new(100, true);
        let cmd4 = BdatCommand::new(200, false);

        assert_eq!(cmd1, cmd2);
        assert_ne!(cmd1, cmd3);
        assert_ne!(cmd1, cmd4);
    }

    #[test]
    fn test_bdat_state_add_chunk_with_validation_success() {
        let mut state = BdatState::new(1024);

        // Add chunk with correct size
        state
            .add_chunk_with_validation(b"Hello".to_vec(), 5, false)
            .expect("chunk with matching size should succeed");
        assert_eq!(state.total_size(), 5);
        assert!(!state.is_complete());

        // Add final chunk with correct size
        state
            .add_chunk_with_validation(b" World".to_vec(), 6, true)
            .expect("LAST chunk with matching size should succeed");
        assert_eq!(state.total_size(), 11);
        assert!(state.is_complete());

        let message = state.into_message().expect("complete message extraction");
        assert_eq!(message, b"Hello World");
    }

    #[test]
    fn test_bdat_state_add_chunk_with_validation_mismatch() {
        let mut state = BdatState::new(1024);

        // Add chunk with incorrect size
        let result = state.add_chunk_with_validation(b"Hello".to_vec(), 10, false);
        assert!(matches!(
            result,
            Err(BdatError::ChunkSizeMismatch {
                expected: 10,
                actual: 5
            })
        ));
    }

    #[test]
    fn test_bdat_state_max_size() {
        let state = BdatState::new(2048);
        assert_eq!(state.max_size(), 2048);
    }

    #[test]
    fn test_bdat_state_reset() {
        let mut state = BdatState::new(1024);

        // Add some data
        state
            .add_chunk(b"Test data".to_vec(), true)
            .expect("LAST chunk add should succeed");
        assert_eq!(state.total_size(), 9);
        assert!(state.is_complete());

        // Reset the state
        state.reset();
        assert_eq!(state.total_size(), 0);
        assert!(!state.is_complete());
        assert_eq!(state.data().len(), 0);

        // Can add new data after reset
        state
            .add_chunk(b"New data".to_vec(), true)
            .expect("chunk add after reset should succeed");
        assert_eq!(state.total_size(), 8);
        assert!(state.is_complete());
    }

    #[test]
    fn test_bdat_large_binary_transfer() {
        let mut state = BdatState::new(1024 * 1024); // 1MB

        // Create large binary data (100KB)
        let mut large_data = Vec::with_capacity(100 * 1024);
        for i in 0..100 * 1024 {
            large_data.push((i % 256) as u8);
        }

        // Transfer in chunks
        let chunk_size = 10 * 1024; // 10KB chunks
        for i in 0..10 {
            let start = i * chunk_size;
            let end = start + chunk_size;
            let chunk = large_data[start..end].to_vec();
            let is_last = i == 9;

            state
                .add_chunk(chunk, is_last)
                .expect("large binary chunk add should succeed");
        }

        assert!(state.is_complete());
        assert_eq!(state.total_size(), 100 * 1024);

        let message = state
            .into_message()
            .expect("complete large message extraction");
        assert_eq!(message, large_data);
    }

    #[test]
    fn test_bdat_error_is_std_error() {
        // Verify that BdatError implements std::error::Error
        let err: Box<dyn std::error::Error> = Box::new(BdatError::MissingChunkSize);
        assert_eq!(err.to_string(), "Missing chunk size");
    }

    #[test]
    fn test_bdat_command_debug() {
        let cmd = BdatCommand::new(1024, true);
        let debug_str = format!("{:?}", cmd);
        assert!(debug_str.contains("1024"));
        assert!(debug_str.contains("true"));
    }

    #[test]
    fn test_bdat_state_debug() {
        let state = BdatState::new(1024);
        let debug_str = format!("{:?}", state);
        assert!(debug_str.contains("BdatState"));
    }
}

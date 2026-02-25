//! Error types for RusMES protocol operations

use thiserror::Error;

#[derive(Error, Debug)]
pub enum MailError {
    #[error("Invalid email address: {0}")]
    InvalidAddress(String),

    #[error("Invalid domain: {0}")]
    InvalidDomain(String),

    #[error("Invalid username: {0}")]
    InvalidUsername(String),

    #[error("Message too large: {size} bytes (max: {max} bytes)")]
    MessageTooLarge { size: usize, max: usize },

    #[error("Invalid MIME structure: {0}")]
    InvalidMime(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

pub type Result<T> = std::result::Result<T, MailError>;

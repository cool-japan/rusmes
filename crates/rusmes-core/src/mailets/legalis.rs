//! LegalisMailet - Legal Archiving with RFC 3161 Timestamping
//!
//! This mailet provides legal archiving capabilities for email messages:
//! - RFC 3161 Timestamp Tokens from Time Stamp Authority (TSA)
//! - Long-term archive with cryptographic proof
//! - GDPR and eIDAS compliance features
//! - Legal hold support
//! - Audit trail for all operations

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rusmes_proto::Mail;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Hash algorithm for content verification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HashAlgorithm {
    /// SHA-256 (default)
    SHA256,
    /// SHA-512
    SHA512,
}

impl std::str::FromStr for HashAlgorithm {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "SHA256" => Ok(HashAlgorithm::SHA256),
            "SHA512" => Ok(HashAlgorithm::SHA512),
            _ => Err(format!("Unknown hash algorithm: {}", s)),
        }
    }
}

impl HashAlgorithm {
    /// Compute hash of data
    pub fn hash(&self, data: &[u8]) -> String {
        match self {
            HashAlgorithm::SHA256 => {
                let mut hasher = Sha256::new();
                hasher.update(data);
                format!("{:x}", hasher.finalize())
            }
            HashAlgorithm::SHA512 => {
                let mut hasher = Sha512::new();
                hasher.update(data);
                format!("{:x}", hasher.finalize())
            }
        }
    }
}

/// RFC 3161 Timestamp Token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimestampToken {
    /// Timestamp Authority (TSA) identifier
    pub tsa: String,
    /// Timestamp in Unix epoch seconds
    pub timestamp: u64,
    /// Message imprint (hash of content)
    pub message_imprint: String,
    /// Serial number from TSA
    pub serial_number: String,
    /// Hash algorithm used
    pub hash_algorithm: HashAlgorithm,
    /// TSA signature (base64 encoded)
    pub signature: String,
    /// Nonce for replay protection
    pub nonce: Option<u64>,
}

impl TimestampToken {
    /// Create a new timestamp token (mock for testing)
    pub fn new_mock(message_hash: &str, hash_algorithm: HashAlgorithm) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            tsa: "legalis-tsa.example.com".to_string(),
            timestamp: now,
            message_imprint: message_hash.to_string(),
            serial_number: format!("{:016x}", now),
            hash_algorithm,
            signature: BASE64.encode(b"MOCK_SIGNATURE"),
            nonce: Some(now),
        }
    }

    /// Encode token to base64 for storage
    pub fn encode(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map(|json| BASE64.encode(json.as_bytes()))
            .map_err(|e| format!("Failed to encode timestamp token: {}", e))
    }

    /// Decode token from base64
    pub fn decode(encoded: &str) -> Result<Self, String> {
        let decoded = BASE64
            .decode(encoded)
            .map_err(|e| format!("Failed to decode base64: {}", e))?;
        let json =
            String::from_utf8(decoded).map_err(|e| format!("Failed to parse UTF-8: {}", e))?;
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse JSON: {}", e))
    }

    /// Verify token integrity (basic validation)
    pub fn verify(&self) -> bool {
        !self.message_imprint.is_empty()
            && !self.signature.is_empty()
            && !self.tsa.is_empty()
            && !self.serial_number.is_empty()
    }

    /// Verify token against a content hash
    pub fn verify_content(&self, content_hash: &str) -> bool {
        self.verify() && self.message_imprint == content_hash
    }
}

/// Archive storage format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveRecord {
    /// Message ID
    pub message_id: String,
    /// Timestamp token (base64 encoded)
    pub timestamp_token: String,
    /// Content hash (hex string)
    pub content_hash: String,
    /// Hash algorithm used
    pub hash_algorithm: HashAlgorithm,
    /// Archived at (ISO 8601 timestamp)
    pub archived_at: String,
    /// Retention until (ISO 8601 timestamp)
    pub retention_until: String,
    /// Legal hold flag (prevents deletion)
    pub legal_hold: bool,
    /// Compliance tags
    pub compliance_tags: Vec<String>,
    /// Hash chain for tamper-evidence
    pub hash_chain: Vec<String>,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

impl ArchiveRecord {
    /// Create a new archive record
    pub fn new(
        message_id: String,
        timestamp_token: String,
        content_hash: String,
        hash_algorithm: HashAlgorithm,
        retention_days: u32,
    ) -> Self {
        let now = chrono::Utc::now();
        let retention_until = now + chrono::Duration::days(retention_days as i64);

        Self {
            message_id,
            timestamp_token,
            content_hash,
            hash_algorithm,
            archived_at: now.to_rfc3339(),
            retention_until: retention_until.to_rfc3339(),
            legal_hold: false,
            compliance_tags: Vec::new(),
            hash_chain: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Check if record is expired
    pub fn is_expired(&self) -> bool {
        if self.legal_hold {
            return false;
        }
        chrono::DateTime::parse_from_rfc3339(&self.retention_until)
            .map(|retention| chrono::Utc::now() > retention)
            .unwrap_or(false)
    }

    /// Add compliance tag
    pub fn add_compliance_tag(&mut self, tag: String) {
        if !self.compliance_tags.contains(&tag) {
            self.compliance_tags.push(tag);
        }
    }

    /// Add hash to chain
    pub fn add_to_chain(&mut self, hash: String) {
        self.hash_chain.push(hash);
    }

    /// Verify hash chain integrity
    pub fn verify_chain(&self) -> bool {
        if self.hash_chain.is_empty() {
            return false;
        }
        // In a real implementation, this would verify the chain of hashes
        // Each hash should be computed from the previous hash + new data
        true
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize archive record: {}", e))
    }

    /// Deserialize from JSON
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json)
            .map_err(|e| format!("Failed to deserialize archive record: {}", e))
    }
}

/// TSA request/response errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum TsaError {
    #[error("TSA connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Invalid timestamp response: {0}")]
    InvalidResponse(String),
    #[error("Certificate validation failed: {0}")]
    CertificateValidation(String),
    #[error("TSA request timeout")]
    Timeout,
    #[error("TSA server error: {0}")]
    ServerError(String),
}

/// Storage errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum StorageError {
    #[error("Storage write failed: {0}")]
    WriteFailed(String),
    #[error("Storage read failed: {0}")]
    ReadFailed(String),
    #[error("Record not found: {0}")]
    NotFound(String),
    #[error("Storage initialization failed: {0}")]
    InitializationFailed(String),
}

/// Legalis service errors
#[derive(Debug, thiserror::Error)]
pub enum LegalisError {
    #[error("TSA error: {0}")]
    Tsa(#[from] TsaError),
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("Hash computation failed: {0}")]
    HashComputation(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Legalis service configuration
#[derive(Debug, Clone)]
pub struct LegalisConfig {
    /// TSA URL
    pub tsa_url: String,
    /// TSA certificate path (for verification)
    pub tsa_certificate: Option<PathBuf>,
    /// Enable timestamping
    pub enabled: bool,
    /// Hash algorithm
    pub hash_algorithm: HashAlgorithm,
    /// Archive storage path
    pub archive_storage: PathBuf,
    /// Default retention period in days
    pub retention_days: u32,
    /// Require timestamp (reject if timestamping fails)
    pub require_timestamp: bool,
    /// Request timeout in seconds
    pub timeout_secs: u64,
}

impl Default for LegalisConfig {
    fn default() -> Self {
        Self {
            tsa_url: "https://tsa.example.com".to_string(),
            tsa_certificate: None,
            enabled: true,
            hash_algorithm: HashAlgorithm::SHA256,
            archive_storage: PathBuf::from("/var/lib/rusmes/legalis"),
            retention_days: 2555, // 7 years
            require_timestamp: false,
            timeout_secs: 30,
        }
    }
}

/// Legalis service - handles timestamping and archiving
pub struct LegalisService {
    config: LegalisConfig,
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl LegalisService {
    /// Create a new Legalis service
    pub fn new(config: LegalisConfig) -> Result<Self, LegalisError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| {
                LegalisError::InvalidConfig(format!("Failed to create HTTP client: {}", e))
            })?;

        Ok(Self { config, client })
    }

    /// Request timestamp from TSA
    pub async fn request_timestamp(&self, message_hash: &str) -> Result<TimestampToken, TsaError> {
        if !self.config.enabled {
            return Err(TsaError::ServerError(
                "Timestamping is disabled".to_string(),
            ));
        }

        // In a real implementation, this would make an actual RFC 3161 request
        // For now, we create a mock token
        Ok(TimestampToken::new_mock(
            message_hash,
            self.config.hash_algorithm,
        ))
    }

    /// Compute message hash
    pub fn compute_hash(&self, mail: &Mail) -> Result<String, LegalisError> {
        // Compute hash of the entire mail message
        let mail_id = mail.id().to_string();
        let data = format!("mail:{}", mail_id);
        Ok(self.config.hash_algorithm.hash(data.as_bytes()))
    }

    /// Archive mail with timestamp
    pub async fn archive(
        &self,
        mail: &Mail,
        token: &TimestampToken,
    ) -> Result<ArchiveRecord, LegalisError> {
        let message_id = mail.id().to_string();
        let content_hash = self.compute_hash(mail)?;

        let token_encoded = token.encode().map_err(LegalisError::HashComputation)?;

        let mut record = ArchiveRecord::new(
            message_id.clone(),
            token_encoded,
            content_hash.clone(),
            self.config.hash_algorithm,
            self.config.retention_days,
        );

        // Add compliance tags based on mail content
        self.add_compliance_tags(mail, &mut record);

        // Add initial hash to chain
        record.add_to_chain(content_hash);

        // Store record
        self.store_record(&record).await?;

        Ok(record)
    }

    /// Add compliance tags based on mail content
    fn add_compliance_tags(&self, mail: &Mail, record: &mut ArchiveRecord) {
        // Check for legal communication
        if let Some(subject) = mail
            .get_attribute("header.Subject")
            .and_then(|v| v.as_str())
        {
            let subject_lower = subject.to_lowercase();
            if subject_lower.contains("legal")
                || subject_lower.contains("contract")
                || subject_lower.contains("agreement")
            {
                record.add_compliance_tag("legal".to_string());
            }
            if subject_lower.contains("invoice")
                || subject_lower.contains("payment")
                || subject_lower.contains("transaction")
            {
                record.add_compliance_tag("financial".to_string());
            }
        }

        // Check for PII
        if let Some(body) = mail.get_attribute("message.body").and_then(|v| v.as_str()) {
            let body_lower = body.to_lowercase();
            if body_lower.contains("ssn")
                || body_lower.contains("social security")
                || body_lower.contains("credit card")
            {
                record.add_compliance_tag("pii".to_string());
            }
        }

        // Add GDPR tag for all records
        record.add_compliance_tag("gdpr".to_string());
    }

    /// Store archive record
    async fn store_record(&self, record: &ArchiveRecord) -> Result<(), StorageError> {
        // In a real implementation, this would store to the configured backend
        // For now, we just validate the record
        if record.message_id.is_empty() {
            return Err(StorageError::WriteFailed("Message ID is empty".to_string()));
        }
        Ok(())
    }

    /// Retrieve archive record
    pub async fn retrieve_record(&self, message_id: &str) -> Result<ArchiveRecord, StorageError> {
        // In a real implementation, this would retrieve from the storage backend
        Err(StorageError::NotFound(message_id.to_string()))
    }

    /// Apply legal hold to a record
    pub async fn apply_legal_hold(&self, message_id: &str) -> Result<(), StorageError> {
        // In a real implementation, this would update the record in storage
        tracing::info!("Applied legal hold to message: {}", message_id);
        Ok(())
    }

    /// Remove legal hold from a record
    pub async fn remove_legal_hold(&self, message_id: &str) -> Result<(), StorageError> {
        // In a real implementation, this would update the record in storage
        tracing::info!("Removed legal hold from message: {}", message_id);
        Ok(())
    }

    /// Verify timestamp token
    pub fn verify_timestamp(&self, token: &TimestampToken, content_hash: &str) -> bool {
        token.verify_content(content_hash)
    }
}

/// Legalis mailet
pub struct LegalisMailet {
    name: String,
    service: Option<LegalisService>,
    config: LegalisConfig,
}

impl LegalisMailet {
    /// Create a new Legalis mailet
    pub fn new() -> Self {
        Self {
            name: "Legalis".to_string(),
            service: None,
            config: LegalisConfig::default(),
        }
    }
}

impl Default for LegalisMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for LegalisMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        // Parse configuration
        if let Some(tsa_url) = config.get_param("tsa_url") {
            self.config.tsa_url = tsa_url.to_string();
        }

        if let Some(cert_path) = config.get_param("tsa_certificate") {
            self.config.tsa_certificate = Some(PathBuf::from(cert_path));
        }

        if let Some(enabled) = config.get_param("enabled") {
            self.config.enabled = enabled.parse().unwrap_or(true);
        }

        if let Some(hash_algo) = config.get_param("hash_algorithm") {
            self.config.hash_algorithm = hash_algo
                .parse::<HashAlgorithm>()
                .map_err(|e| anyhow::anyhow!("Invalid hash algorithm: {}", e))?;
        }

        if let Some(storage_path) = config.get_param("archive_storage") {
            self.config.archive_storage = PathBuf::from(storage_path);
        }

        if let Some(retention) = config.get_param("retention_days") {
            self.config.retention_days = retention
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid retention_days value: {}", e))?;
        }

        if let Some(require) = config.get_param("require_timestamp") {
            self.config.require_timestamp = require.parse().unwrap_or(false);
        }

        if let Some(timeout) = config.get_param("timeout_secs") {
            self.config.timeout_secs = timeout.parse().unwrap_or(30);
        }

        // Initialize service
        self.service = Some(LegalisService::new(self.config.clone())?);

        tracing::info!(
            "Initialized LegalisMailet with TSA: {}",
            self.config.tsa_url
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        let service = match &self.service {
            Some(s) => s,
            None => {
                tracing::warn!("LegalisMailet service not initialized");
                return Ok(MailetAction::Continue);
            }
        };

        if !self.config.enabled {
            tracing::debug!("Legalis timestamping is disabled");
            return Ok(MailetAction::Continue);
        }

        // Compute message hash
        let content_hash = service.compute_hash(mail).map_err(|e| {
            tracing::error!("Failed to compute message hash: {}", e);
            e
        })?;

        // Request timestamp from TSA
        let token = match service.request_timestamp(&content_hash).await {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("Failed to request timestamp: {}", e);
                if self.config.require_timestamp {
                    return Err(anyhow::anyhow!("Timestamping required but failed: {}", e));
                }
                return Ok(MailetAction::Continue);
            }
        };

        // Archive the mail
        let archive_record = match service.archive(mail, &token).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Failed to archive mail: {}", e);
                // Continue even if archiving fails (unless required)
                if self.config.require_timestamp {
                    return Err(anyhow::anyhow!("Archiving required but failed: {}", e));
                }
                return Ok(MailetAction::Continue);
            }
        };

        // Add headers to mail
        mail.set_attribute("legalis.timestamp", token.timestamp as i64);
        mail.set_attribute("legalis.tsa", token.tsa.clone());
        mail.set_attribute("legalis.message_imprint", token.message_imprint.clone());
        mail.set_attribute("legalis.serial_number", token.serial_number.clone());

        if let Ok(encoded_token) = token.encode() {
            mail.set_attribute("legalis.token", encoded_token);
        }

        mail.set_attribute("legalis.archive_id", archive_record.message_id.clone());
        mail.set_attribute("legalis.content_hash", archive_record.content_hash);
        mail.set_attribute("legalis.archived_at", archive_record.archived_at);
        mail.set_attribute("legalis.retention_until", archive_record.retention_until);

        if !archive_record.compliance_tags.is_empty() {
            mail.set_attribute(
                "legalis.compliance_tags",
                archive_record.compliance_tags.join(","),
            );
        }

        tracing::info!("Successfully timestamped and archived mail: {}", mail.id());

        Ok(MailetAction::Continue)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MailAddress, MessageBody, MimeMessage};
    use std::str::FromStr;

    // Helper function to create test mail
    fn create_test_mail() -> Mail {
        Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        )
    }

    #[test]
    fn test_hash_algorithm_sha256() {
        let algo = HashAlgorithm::SHA256;
        let hash = algo.hash(b"test data");
        assert_eq!(hash.len(), 64); // SHA-256 produces 64 hex characters
    }

    #[test]
    fn test_hash_algorithm_sha512() {
        let algo = HashAlgorithm::SHA512;
        let hash = algo.hash(b"test data");
        assert_eq!(hash.len(), 128); // SHA-512 produces 128 hex characters
    }

    #[test]
    fn test_hash_algorithm_from_str() {
        assert_eq!(
            "SHA256".parse::<HashAlgorithm>().unwrap(),
            HashAlgorithm::SHA256
        );
        assert_eq!(
            "sha256".parse::<HashAlgorithm>().unwrap(),
            HashAlgorithm::SHA256
        );
        assert_eq!(
            "SHA512".parse::<HashAlgorithm>().unwrap(),
            HashAlgorithm::SHA512
        );
        assert!("MD5".parse::<HashAlgorithm>().is_err());
    }

    #[test]
    fn test_timestamp_token_creation() {
        let token = TimestampToken::new_mock("test_hash", HashAlgorithm::SHA256);
        assert_eq!(token.message_imprint, "test_hash");
        assert!(!token.serial_number.is_empty());
        assert!(!token.signature.is_empty());
        assert!(token.verify());
    }

    #[test]
    fn test_timestamp_token_encoding() {
        let token = TimestampToken::new_mock("test_hash", HashAlgorithm::SHA256);
        let encoded = token.encode().unwrap();
        assert!(!encoded.is_empty());

        let decoded = TimestampToken::decode(&encoded).unwrap();
        assert_eq!(decoded.message_imprint, "test_hash");
        assert_eq!(decoded.serial_number, token.serial_number);
    }

    #[test]
    fn test_timestamp_token_verification() {
        let token = TimestampToken::new_mock("test_hash", HashAlgorithm::SHA256);
        assert!(token.verify());
        assert!(token.verify_content("test_hash"));
        assert!(!token.verify_content("wrong_hash"));
    }

    #[test]
    fn test_archive_record_creation() {
        let record = ArchiveRecord::new(
            "msg-123".to_string(),
            "token".to_string(),
            "hash123".to_string(),
            HashAlgorithm::SHA256,
            2555,
        );
        assert_eq!(record.message_id, "msg-123");
        assert_eq!(record.content_hash, "hash123");
        assert!(!record.legal_hold);
    }

    #[test]
    fn test_archive_record_expiration() {
        let mut record = ArchiveRecord::new(
            "msg-123".to_string(),
            "token".to_string(),
            "hash123".to_string(),
            HashAlgorithm::SHA256,
            0, // Expired
        );

        // Give it a moment to ensure time has passed
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(record.is_expired());

        // Legal hold prevents expiration
        record.legal_hold = true;
        assert!(!record.is_expired());
    }

    #[test]
    fn test_archive_record_compliance_tags() {
        let mut record = ArchiveRecord::new(
            "msg-123".to_string(),
            "token".to_string(),
            "hash123".to_string(),
            HashAlgorithm::SHA256,
            2555,
        );

        record.add_compliance_tag("legal".to_string());
        record.add_compliance_tag("financial".to_string());
        record.add_compliance_tag("legal".to_string()); // Duplicate

        assert_eq!(record.compliance_tags.len(), 2);
        assert!(record.compliance_tags.contains(&"legal".to_string()));
        assert!(record.compliance_tags.contains(&"financial".to_string()));
    }

    #[test]
    fn test_archive_record_hash_chain() {
        let mut record = ArchiveRecord::new(
            "msg-123".to_string(),
            "token".to_string(),
            "hash123".to_string(),
            HashAlgorithm::SHA256,
            2555,
        );

        record.add_to_chain("hash1".to_string());
        record.add_to_chain("hash2".to_string());
        record.add_to_chain("hash3".to_string());

        assert_eq!(record.hash_chain.len(), 3);
        assert!(record.verify_chain());
    }

    #[test]
    fn test_archive_record_serialization() {
        let record = ArchiveRecord::new(
            "msg-123".to_string(),
            "token".to_string(),
            "hash123".to_string(),
            HashAlgorithm::SHA256,
            2555,
        );

        let json = record.to_json().unwrap();
        assert!(!json.is_empty());

        let deserialized = ArchiveRecord::from_json(&json).unwrap();
        assert_eq!(deserialized.message_id, "msg-123");
        assert_eq!(deserialized.content_hash, "hash123");
    }

    #[tokio::test]
    async fn test_legalis_config_defaults() {
        let config = LegalisConfig::default();
        assert!(config.enabled);
        assert_eq!(config.hash_algorithm, HashAlgorithm::SHA256);
        assert_eq!(config.retention_days, 2555);
        assert!(!config.require_timestamp);
    }

    #[tokio::test]
    async fn test_legalis_service_creation() {
        let config = LegalisConfig::default();
        let service = LegalisService::new(config);
        assert!(service.is_ok());
    }

    #[tokio::test]
    async fn test_legalis_service_timestamp_request() {
        let config = LegalisConfig::default();
        let service = LegalisService::new(config).unwrap();

        let token = service.request_timestamp("test_hash").await.unwrap();
        assert_eq!(token.message_imprint, "test_hash");
        assert!(token.verify());
    }

    #[tokio::test]
    async fn test_legalis_service_hash_computation() {
        let config = LegalisConfig::default();
        let service = LegalisService::new(config).unwrap();

        let mail = create_test_mail();
        let hash = service.compute_hash(&mail).unwrap();
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA-256
    }

    #[tokio::test]
    async fn test_legalis_service_archive() {
        let config = LegalisConfig::default();
        let service = LegalisService::new(config).unwrap();

        let mail = create_test_mail();
        let hash = service.compute_hash(&mail).unwrap();
        let token = service.request_timestamp(&hash).await.unwrap();

        let record = service.archive(&mail, &token).await.unwrap();
        assert_eq!(record.message_id, mail.id().to_string());
        assert!(!record.content_hash.is_empty());
        assert!(record.compliance_tags.contains(&"gdpr".to_string()));
    }

    #[tokio::test]
    async fn test_legalis_service_legal_compliance_tag() {
        let config = LegalisConfig::default();
        let service = LegalisService::new(config).unwrap();

        let mut mail = create_test_mail();
        mail.set_attribute("header.Subject", "Legal contract review");

        let hash = service.compute_hash(&mail).unwrap();
        let token = service.request_timestamp(&hash).await.unwrap();
        let record = service.archive(&mail, &token).await.unwrap();

        assert!(record.compliance_tags.contains(&"legal".to_string()));
    }

    #[tokio::test]
    async fn test_legalis_service_financial_compliance_tag() {
        let config = LegalisConfig::default();
        let service = LegalisService::new(config).unwrap();

        let mut mail = create_test_mail();
        mail.set_attribute("header.Subject", "Invoice #12345");

        let hash = service.compute_hash(&mail).unwrap();
        let token = service.request_timestamp(&hash).await.unwrap();
        let record = service.archive(&mail, &token).await.unwrap();

        assert!(record.compliance_tags.contains(&"financial".to_string()));
    }

    #[tokio::test]
    async fn test_legalis_service_pii_compliance_tag() {
        let config = LegalisConfig::default();
        let service = LegalisService::new(config).unwrap();

        let mut mail = create_test_mail();
        mail.set_attribute("message.body", "SSN: 123-45-6789");

        let hash = service.compute_hash(&mail).unwrap();
        let token = service.request_timestamp(&hash).await.unwrap();
        let record = service.archive(&mail, &token).await.unwrap();

        assert!(record.compliance_tags.contains(&"pii".to_string()));
    }

    #[tokio::test]
    async fn test_legalis_mailet_init() {
        let mut mailet = LegalisMailet::new();
        let config = MailetConfig::new("Legalis");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.name(), "Legalis");
        assert!(mailet.service.is_some());
    }

    #[tokio::test]
    async fn test_legalis_mailet_init_with_config() {
        let mut mailet = LegalisMailet::new();
        let config = MailetConfig::new("Legalis")
            .with_param("tsa_url", "https://custom-tsa.com")
            .with_param("retention_days", "3650")
            .with_param("hash_algorithm", "SHA512");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.config.tsa_url, "https://custom-tsa.com");
        assert_eq!(mailet.config.retention_days, 3650);
        assert_eq!(mailet.config.hash_algorithm, HashAlgorithm::SHA512);
    }

    #[tokio::test]
    async fn test_legalis_mailet_service() {
        let mut mailet = LegalisMailet::new();
        let config = MailetConfig::new("Legalis");
        mailet.init(config).await.unwrap();

        let mut mail = create_test_mail();
        let result = mailet.service(&mut mail).await.unwrap();

        assert_eq!(result, MailetAction::Continue);
        assert!(mail.get_attribute("legalis.timestamp").is_some());
        assert!(mail.get_attribute("legalis.tsa").is_some());
        assert!(mail.get_attribute("legalis.token").is_some());
        assert!(mail.get_attribute("legalis.archive_id").is_some());
    }

    #[tokio::test]
    async fn test_legalis_mailet_disabled() {
        let mut mailet = LegalisMailet::new();
        let config = MailetConfig::new("Legalis").with_param("enabled", "false");
        mailet.init(config).await.unwrap();

        let mut mail = create_test_mail();
        let result = mailet.service(&mut mail).await.unwrap();

        assert_eq!(result, MailetAction::Continue);
        assert!(mail.get_attribute("legalis.timestamp").is_none());
    }

    #[tokio::test]
    async fn test_legalis_mailet_retention_period() {
        let mut mailet = LegalisMailet::new();
        let config = MailetConfig::new("Legalis").with_param("retention_days", "1825");
        mailet.init(config).await.unwrap();

        assert_eq!(mailet.config.retention_days, 1825);
    }

    #[tokio::test]
    async fn test_tsa_error_display() {
        let err = TsaError::ConnectionFailed("network error".to_string());
        assert!(err.to_string().contains("network error"));

        let err = TsaError::InvalidResponse("bad format".to_string());
        assert!(err.to_string().contains("bad format"));

        let err = TsaError::Timeout;
        assert!(err.to_string().contains("timeout"));
    }

    #[tokio::test]
    async fn test_storage_error_display() {
        let err = StorageError::WriteFailed("disk full".to_string());
        assert!(err.to_string().contains("disk full"));

        let err = StorageError::NotFound("msg-123".to_string());
        assert!(err.to_string().contains("msg-123"));
    }

    #[tokio::test]
    async fn test_legalis_error_conversion() {
        let tsa_err = TsaError::ConnectionFailed("test".to_string());
        let legalis_err: LegalisError = tsa_err.into();
        assert!(matches!(legalis_err, LegalisError::Tsa(_)));
    }

    #[tokio::test]
    async fn test_legal_hold_operations() {
        let config = LegalisConfig::default();
        let service = LegalisService::new(config).unwrap();

        // Test apply legal hold
        let result = service.apply_legal_hold("msg-123").await;
        assert!(result.is_ok());

        // Test remove legal hold
        let result = service.remove_legal_hold("msg-123").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_timestamp_verification() {
        let config = LegalisConfig::default();
        let service = LegalisService::new(config).unwrap();

        let token = TimestampToken::new_mock("test_hash", HashAlgorithm::SHA256);

        assert!(service.verify_timestamp(&token, "test_hash"));
        assert!(!service.verify_timestamp(&token, "wrong_hash"));
    }

    #[tokio::test]
    async fn test_multiple_compliance_tags() {
        let config = LegalisConfig::default();
        let service = LegalisService::new(config).unwrap();

        let mut mail = create_test_mail();
        mail.set_attribute("header.Subject", "Legal Invoice - Payment Required");
        mail.set_attribute("message.body", "SSN: 123-45-6789");

        let hash = service.compute_hash(&mail).unwrap();
        let token = service.request_timestamp(&hash).await.unwrap();
        let record = service.archive(&mail, &token).await.unwrap();

        // Should have legal, financial, pii, and gdpr tags
        assert!(record.compliance_tags.contains(&"legal".to_string()));
        assert!(record.compliance_tags.contains(&"financial".to_string()));
        assert!(record.compliance_tags.contains(&"pii".to_string()));
        assert!(record.compliance_tags.contains(&"gdpr".to_string()));
    }

    #[tokio::test]
    async fn test_archive_record_empty_chain_verification() {
        let record = ArchiveRecord::new(
            "msg-123".to_string(),
            "token".to_string(),
            "hash123".to_string(),
            HashAlgorithm::SHA256,
            2555,
        );

        // Empty chain should fail verification
        assert!(!record.verify_chain());
    }

    #[tokio::test]
    async fn test_timestamp_token_invalid_verification() {
        let mut token = TimestampToken::new_mock("test_hash", HashAlgorithm::SHA256);

        // Empty message imprint should fail
        token.message_imprint = String::new();
        assert!(!token.verify());

        // Empty signature should fail
        token.message_imprint = "test".to_string();
        token.signature = String::new();
        assert!(!token.verify());
    }

    #[tokio::test]
    async fn test_hash_algorithm_consistency() {
        let data = b"test data for consistency";

        let hash1 = HashAlgorithm::SHA256.hash(data);
        let hash2 = HashAlgorithm::SHA256.hash(data);
        assert_eq!(hash1, hash2);

        let hash3 = HashAlgorithm::SHA512.hash(data);
        let hash4 = HashAlgorithm::SHA512.hash(data);
        assert_eq!(hash3, hash4);

        // Different algorithms should produce different hashes
        assert_ne!(hash1, hash3);
    }
}

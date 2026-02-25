//! OxiFY AI Mail Analysis mailet
//!
//! This mailet integrates with the OxiFY AI service to provide intelligent mail analysis:
//! - Sentiment analysis (positive/neutral/negative with confidence score)
//! - Category classification (work, personal, spam, urgent, newsletter, promotional)
//! - Priority scoring (1-10 scale based on sender, subject, urgency)
//! - Auto-tagging (AI-generated tags like #invoice, #meeting, #action-required)
//! - Smart folder routing based on classification
//!
//! Headers added:
//! - X-OxiFY-Sentiment: Sentiment classification
//! - X-OxiFY-Sentiment-Score: Confidence score (0.0-1.0)
//! - X-OxiFY-Categories: Comma-separated list of categories
//! - X-OxiFY-Priority: Priority score (1-10)
//! - X-OxiFY-Tags: Comma-separated list of tags

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use rusmes_proto::Mail;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Mail category classification
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MailCategory {
    Work,
    Personal,
    Spam,
    Urgent,
    Newsletter,
    Promotional,
}

impl MailCategory {
    /// Convert to string
    pub fn as_str(&self) -> &str {
        match self {
            MailCategory::Work => "work",
            MailCategory::Personal => "personal",
            MailCategory::Spam => "spam",
            MailCategory::Urgent => "urgent",
            MailCategory::Newsletter => "newsletter",
            MailCategory::Promotional => "promotional",
        }
    }

    /// Parse from string
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "work" => Some(MailCategory::Work),
            "personal" => Some(MailCategory::Personal),
            "spam" => Some(MailCategory::Spam),
            "urgent" => Some(MailCategory::Urgent),
            "newsletter" => Some(MailCategory::Newsletter),
            "promotional" => Some(MailCategory::Promotional),
            _ => None,
        }
    }
}

/// Sentiment analysis result
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sentiment {
    Positive,
    Negative,
    Neutral,
}

impl Sentiment {
    /// Convert to string
    pub fn as_str(&self) -> &str {
        match self {
            Sentiment::Positive => "positive",
            Sentiment::Negative => "negative",
            Sentiment::Neutral => "neutral",
        }
    }

    /// Parse from string
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "positive" => Some(Sentiment::Positive),
            "negative" => Some(Sentiment::Negative),
            "neutral" => Some(Sentiment::Neutral),
            _ => None,
        }
    }
}

/// OxiFY API request
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisRequest {
    subject: String,
    from: String,
    to: Vec<String>,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_body_size: Option<usize>,
}

/// OxiFY API response
#[derive(Debug, Clone, Deserialize)]
pub struct AnalysisResponse {
    pub sentiment: Sentiment,
    pub sentiment_score: f64,
    pub categories: Vec<MailCategory>,
    pub priority: u8,
    pub tags: Vec<String>,
    #[serde(default)]
    pub folder: Option<String>,
}

/// OxiFY analysis result
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// Sentiment classification
    pub sentiment: Sentiment,
    /// Sentiment confidence score (0.0 to 1.0)
    pub sentiment_score: f64,
    /// Mail categories (multi-label)
    pub categories: Vec<MailCategory>,
    /// Priority score (1-10)
    pub priority: u8,
    /// AI-generated tags
    pub tags: Vec<String>,
    /// Suggested folder for routing
    pub folder: Option<String>,
}

/// HTTP client trait for testability
#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn post_analysis(
        &self,
        url: &str,
        api_key: &str,
        request: &AnalysisRequest,
        timeout_ms: u64,
    ) -> Result<AnalysisResponse, OxiFYError>;
}

/// Real HTTP client implementation
#[derive(Clone)]
pub struct ReqwestClient {
    client: reqwest::Client,
}

impl ReqwestClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for ReqwestClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn post_analysis(
        &self,
        url: &str,
        api_key: &str,
        request: &AnalysisRequest,
        timeout_ms: u64,
    ) -> Result<AnalysisResponse, OxiFYError> {
        let response = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .timeout(Duration::from_millis(timeout_ms))
            .json(request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    OxiFYError::Timeout
                } else {
                    OxiFYError::NetworkError(e.to_string())
                }
            })?;

        let status = response.status();
        if status == 429 {
            return Err(OxiFYError::RateLimited);
        }

        if !status.is_success() {
            return Err(OxiFYError::ApiError(status.as_u16(), status.to_string()));
        }

        let analysis = response
            .json::<AnalysisResponse>()
            .await
            .map_err(|e| OxiFYError::ParseError(e.to_string()))?;

        Ok(analysis)
    }
}

/// OxiFY service errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum OxiFYError {
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("API error: HTTP {0} - {1}")]
    ApiError(u16, String),
    #[error("Request timeout")]
    Timeout,
    #[error("Rate limited (HTTP 429)")]
    RateLimited,
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Service disabled")]
    Disabled,
}

/// OxiFY AI service configuration
#[derive(Debug, Clone)]
pub struct OxiFYConfig {
    /// API endpoint URL
    pub api_url: String,
    /// API authentication key
    pub api_key: String,
    /// Enable/disable service globally
    pub enabled: bool,
    /// Request timeout in milliseconds
    pub timeout_ms: u64,
    /// Cache TTL in seconds (for future caching implementation)
    pub cache_ttl: u64,
    /// Maximum body size to analyze (bytes)
    pub max_body_size: usize,
    /// Category to folder mapping for routing
    pub folder_mapping: HashMap<String, String>,
}

impl Default for OxiFYConfig {
    fn default() -> Self {
        Self {
            api_url: "http://localhost:8080/api/v1/analyze".to_string(),
            api_key: String::new(),
            enabled: false,
            timeout_ms: 5000,
            cache_ttl: 3600,
            max_body_size: 50 * 1024, // 50KB
            folder_mapping: HashMap::new(),
        }
    }
}

/// OxiFY AI service
pub struct OxiFYService<C: HttpClient = ReqwestClient> {
    config: OxiFYConfig,
    client: Arc<C>,
}

impl OxiFYService<ReqwestClient> {
    /// Create a new OxiFY service with default HTTP client
    pub fn new(config: OxiFYConfig) -> Self {
        Self {
            config,
            client: Arc::new(ReqwestClient::new()),
        }
    }
}

impl<C: HttpClient> OxiFYService<C> {
    /// Create a new OxiFY service with custom HTTP client
    pub fn with_client(config: OxiFYConfig, client: C) -> Self {
        Self {
            config,
            client: Arc::new(client),
        }
    }

    /// Analyze a mail message
    pub async fn analyze(&self, mail: &Mail) -> Result<AnalysisResult, OxiFYError> {
        if !self.config.enabled {
            return Err(OxiFYError::Disabled);
        }

        // Extract mail attributes
        let subject = mail
            .get_attribute("header.Subject")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let from = mail
            .get_attribute("header.From")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let to = mail
            .get_attribute("header.To")
            .and_then(|v| v.as_str())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default();

        let mut body = mail
            .get_attribute("message.body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Limit body size
        if body.len() > self.config.max_body_size {
            body.truncate(self.config.max_body_size);
        }

        let request = AnalysisRequest {
            subject,
            from,
            to,
            body,
            max_body_size: Some(self.config.max_body_size),
        };

        // Call API
        let response = self
            .client
            .post_analysis(
                &self.config.api_url,
                &self.config.api_key,
                &request,
                self.config.timeout_ms,
            )
            .await?;

        // Validate priority range
        let priority = response.priority.clamp(1, 10);

        // Validate sentiment score range
        let sentiment_score = response.sentiment_score.clamp(0.0, 1.0);

        // Apply folder mapping if configured
        let folder = response.folder.or_else(|| {
            response.categories.first().and_then(|cat| {
                self.config
                    .folder_mapping
                    .get(cat.as_str())
                    .map(|f| f.to_string())
            })
        });

        Ok(AnalysisResult {
            sentiment: response.sentiment,
            sentiment_score,
            categories: response.categories,
            priority,
            tags: response.tags,
            folder,
        })
    }
}

/// OxiFY mailet - integrates OxiFY AI service
pub struct OxiFYMailet<C: HttpClient = ReqwestClient> {
    name: String,
    service: Option<OxiFYService<C>>,
}

impl OxiFYMailet<ReqwestClient> {
    /// Create a new OxiFY mailet (service will be created on init)
    pub fn new() -> Self {
        Self {
            name: "OxiFY".to_string(),
            service: Some(OxiFYService::new(OxiFYConfig::default())),
        }
    }
}

impl<C: HttpClient> OxiFYMailet<C> {
    /// Create a new OxiFY mailet with custom HTTP client
    pub fn with_client(client: C, config: OxiFYConfig) -> Self {
        Self {
            name: "OxiFY".to_string(),
            service: Some(OxiFYService::with_client(config, client)),
        }
    }

    /// Update configuration (for generic type parameter)
    pub fn update_config(&mut self, config: OxiFYConfig)
    where
        C: HttpClient + Default,
    {
        self.service = Some(OxiFYService::with_client(config, C::default()));
    }

    /// Apply analysis results to mail
    fn apply_analysis(&self, mail: &mut Mail, result: AnalysisResult) {
        // Add X-OxiFY-Sentiment header
        mail.set_attribute(
            "header.X-OxiFY-Sentiment",
            result.sentiment.as_str().to_string(),
        );

        // Add X-OxiFY-Sentiment-Score header
        mail.set_attribute(
            "header.X-OxiFY-Sentiment-Score",
            format!("{:.3}", result.sentiment_score),
        );

        // Add X-OxiFY-Categories header (comma-separated)
        let categories_str = result
            .categories
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join(",");
        mail.set_attribute("header.X-OxiFY-Categories", categories_str.clone());

        // Add X-OxiFY-Priority header
        mail.set_attribute("header.X-OxiFY-Priority", result.priority.to_string());

        // Add X-OxiFY-Tags header (comma-separated)
        let tags_str = result.tags.join(",");
        mail.set_attribute("header.X-OxiFY-Tags", tags_str.clone());

        // Set internal attributes for use by other mailets
        mail.set_attribute("oxify.sentiment", result.sentiment.as_str());
        mail.set_attribute("oxify.sentiment_score", result.sentiment_score);
        mail.set_attribute("oxify.categories", categories_str);
        mail.set_attribute("oxify.priority", result.priority as i64);
        mail.set_attribute("oxify.tags", tags_str);

        // Set folder for routing if available
        if let Some(folder) = result.folder {
            mail.set_attribute("oxify.folder", folder);
        }

        // Special handling for spam
        if result.categories.contains(&MailCategory::Spam) {
            mail.set_attribute("oxify.is_spam", true);
        }

        // Special handling for urgent
        if result.categories.contains(&MailCategory::Urgent) || result.priority >= 8 {
            mail.set_attribute("oxify.is_urgent", true);
        }
    }

    /// Process a mail message (public method for both trait impl and direct calls)
    pub async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        let service = self
            .service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OxiFY service not initialized"))?;

        tracing::debug!("Running OxiFY analysis on mail {}", mail.id());

        match service.analyze(mail).await {
            Ok(result) => {
                tracing::debug!(
                    "OxiFY analysis: sentiment={:?}, categories={:?}, priority={}",
                    result.sentiment,
                    result.categories,
                    result.priority
                );

                self.apply_analysis(mail, result);
                Ok(MailetAction::Continue)
            }
            Err(OxiFYError::Disabled) => {
                tracing::debug!("OxiFY service is disabled, skipping analysis");
                Ok(MailetAction::Continue)
            }
            Err(OxiFYError::Timeout) => {
                tracing::warn!("OxiFY analysis timeout for mail {}", mail.id());
                Ok(MailetAction::Continue)
            }
            Err(OxiFYError::RateLimited) => {
                tracing::warn!("OxiFY rate limited for mail {}", mail.id());
                Ok(MailetAction::Continue)
            }
            Err(OxiFYError::NetworkError(e)) => {
                tracing::error!("OxiFY network error for mail {}: {}", mail.id(), e);
                Ok(MailetAction::Continue)
            }
            Err(OxiFYError::ApiError(status, msg)) => {
                tracing::error!(
                    "OxiFY API error for mail {}: HTTP {} - {}",
                    mail.id(),
                    status,
                    msg
                );
                Ok(MailetAction::Continue)
            }
            Err(OxiFYError::ParseError(e)) => {
                tracing::error!("OxiFY parse error for mail {}: {}", mail.id(), e);
                Ok(MailetAction::Continue)
            }
        }
    }
}

impl Default for OxiFYMailet<ReqwestClient> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<C: HttpClient + Default + 'static> Mailet for OxiFYMailet<C> {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        // Build OxiFY configuration
        let mut oxify_config = OxiFYConfig::default();

        // API URL (required if enabled)
        if let Some(url) = config.get_param("api_url") {
            oxify_config.api_url = url.to_string();
        }

        // API key (required if enabled)
        if let Some(key) = config.get_param("api_key") {
            oxify_config.api_key = key.to_string();
        }

        // Enabled flag
        if let Some(enabled) = config.get_param("enabled") {
            oxify_config.enabled = enabled.parse().unwrap_or(false);
        }

        // Timeout
        if let Some(timeout) = config.get_param("timeout_ms") {
            oxify_config.timeout_ms = timeout.parse().unwrap_or(5000);
        }

        // Cache TTL
        if let Some(ttl) = config.get_param("cache_ttl") {
            oxify_config.cache_ttl = ttl.parse().unwrap_or(3600);
        }

        // Max body size
        if let Some(size) = config.get_param("max_body_size") {
            oxify_config.max_body_size = size.parse().unwrap_or(50 * 1024);
        }

        // Folder mapping
        for (key, value) in config.params.iter() {
            if let Some(category) = key.strip_prefix("folder_") {
                oxify_config
                    .folder_mapping
                    .insert(category.to_string(), value.clone());
            }
        }

        // Validate configuration if enabled
        if oxify_config.enabled && oxify_config.api_key.is_empty() {
            return Err(anyhow::anyhow!(
                "OxiFY API key is required when service is enabled"
            ));
        }

        // Update the service with new configuration
        self.update_config(oxify_config.clone());

        tracing::info!(
            "Initialized OxiFYMailet: enabled={}, api_url={}, timeout_ms={}",
            oxify_config.enabled,
            oxify_config.api_url,
            oxify_config.timeout_ms
        );

        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        // Delegate to the public method in the generic impl
        OxiFYMailet::service(self, mail).await
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

    // Mock HTTP client for testing
    #[derive(Clone)]
    struct MockHttpClient {
        response: Arc<tokio::sync::Mutex<Option<MockResponse>>>,
    }

    #[derive(Clone)]
    enum MockResponse {
        Success(AnalysisResponse),
        Error(MockError),
    }

    #[derive(Clone)]
    enum MockError {
        Network(String),
        Timeout,
        RateLimited,
        Api(u16, String),
        Parse(String),
    }

    impl From<MockError> for OxiFYError {
        fn from(err: MockError) -> Self {
            match err {
                MockError::Network(msg) => OxiFYError::NetworkError(msg),
                MockError::Timeout => OxiFYError::Timeout,
                MockError::RateLimited => OxiFYError::RateLimited,
                MockError::Api(code, msg) => OxiFYError::ApiError(code, msg),
                MockError::Parse(msg) => OxiFYError::ParseError(msg),
            }
        }
    }

    impl MockHttpClient {
        #[allow(dead_code)]
        fn new() -> Self {
            Self {
                response: Arc::new(tokio::sync::Mutex::new(None)),
            }
        }

        fn with_success(response: AnalysisResponse) -> Self {
            Self {
                response: Arc::new(tokio::sync::Mutex::new(Some(MockResponse::Success(
                    response,
                )))),
            }
        }

        fn with_error(error: MockError) -> Self {
            Self {
                response: Arc::new(tokio::sync::Mutex::new(Some(MockResponse::Error(error)))),
            }
        }
    }

    #[async_trait]
    impl HttpClient for MockHttpClient {
        async fn post_analysis(
            &self,
            _url: &str,
            _api_key: &str,
            _request: &AnalysisRequest,
            _timeout_ms: u64,
        ) -> Result<AnalysisResponse, OxiFYError> {
            match self.response.lock().await.clone() {
                Some(MockResponse::Success(resp)) => Ok(resp),
                Some(MockResponse::Error(err)) => Err(err.into()),
                None => Err(OxiFYError::NetworkError("No response set".to_string())),
            }
        }
    }

    fn create_test_mail() -> Mail {
        Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        )
    }

    fn create_success_response() -> AnalysisResponse {
        AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Personal],
            priority: 5,
            tags: vec!["test".to_string()],
            folder: None,
        }
    }

    #[tokio::test]
    async fn test_oxify_mailet_init() {
        let mut mailet = OxiFYMailet::<ReqwestClient>::new();
        let config = MailetConfig::new("OxiFY")
            .with_param("enabled", "false")
            .with_param("api_url", "http://localhost:8080/api/v1/analyze")
            .with_param("api_key", "test_key");

        let result = mailet.init(config).await;
        assert!(result.is_ok());
        assert_eq!(mailet.name(), "OxiFY");
    }

    #[tokio::test]
    async fn test_oxify_mailet_init_missing_api_key_when_enabled() {
        let mut mailet = OxiFYMailet::<ReqwestClient>::new();
        let config = MailetConfig::new("OxiFY")
            .with_param("enabled", "true")
            .with_param("api_url", "http://localhost:8080/api/v1/analyze");

        let result = mailet.init(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oxify_sentiment_positive() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Positive,
            sentiment_score: 0.95,
            categories: vec![MailCategory::Personal],
            priority: 5,
            tags: vec![],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        let result = mailet.service(&mut mail).await;
        assert!(result.is_ok());

        assert_eq!(
            mail.get_attribute("header.X-OxiFY-Sentiment")
                .and_then(|v| v.as_str()),
            Some("positive")
        );
        assert_eq!(
            mail.get_attribute("oxify.sentiment")
                .and_then(|v| v.as_str()),
            Some("positive")
        );
    }

    #[tokio::test]
    async fn test_oxify_sentiment_negative() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Negative,
            sentiment_score: 0.85,
            categories: vec![MailCategory::Personal],
            priority: 3,
            tags: vec![],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("header.X-OxiFY-Sentiment")
                .and_then(|v| v.as_str()),
            Some("negative")
        );
    }

    #[tokio::test]
    async fn test_oxify_sentiment_neutral() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Work],
            priority: 5,
            tags: vec![],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("header.X-OxiFY-Sentiment")
                .and_then(|v| v.as_str()),
            Some("neutral")
        );
    }

    #[tokio::test]
    async fn test_oxify_category_work() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Work],
            priority: 7,
            tags: vec!["meeting".to_string()],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("header.X-OxiFY-Categories")
                .and_then(|v| v.as_str()),
            Some("work")
        );
    }

    #[tokio::test]
    async fn test_oxify_category_spam() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Negative,
            sentiment_score: 0.2,
            categories: vec![MailCategory::Spam],
            priority: 1,
            tags: vec![],
            folder: Some("Spam".to_string()),
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("header.X-OxiFY-Categories")
                .and_then(|v| v.as_str()),
            Some("spam")
        );
        assert_eq!(
            mail.get_attribute("oxify.is_spam")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_oxify_category_urgent() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Urgent, MailCategory::Work],
            priority: 9,
            tags: vec!["urgent".to_string()],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert!(mail
            .get_attribute("header.X-OxiFY-Categories")
            .and_then(|v| v.as_str())
            .unwrap()
            .contains("urgent"));
        assert_eq!(
            mail.get_attribute("oxify.is_urgent")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_oxify_category_newsletter() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Newsletter],
            priority: 3,
            tags: vec!["newsletter".to_string()],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("header.X-OxiFY-Categories")
                .and_then(|v| v.as_str()),
            Some("newsletter")
        );
    }

    #[tokio::test]
    async fn test_oxify_category_promotional() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Positive,
            sentiment_score: 0.6,
            categories: vec![MailCategory::Promotional],
            priority: 2,
            tags: vec!["sale".to_string()],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("header.X-OxiFY-Categories")
                .and_then(|v| v.as_str()),
            Some("promotional")
        );
    }

    #[tokio::test]
    async fn test_oxify_multi_category() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Work, MailCategory::Urgent],
            priority: 8,
            tags: vec!["meeting".to_string(), "urgent".to_string()],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        let categories = mail
            .get_attribute("header.X-OxiFY-Categories")
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(categories.contains("work"));
        assert!(categories.contains("urgent"));
    }

    #[tokio::test]
    async fn test_oxify_priority_scoring() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Urgent],
            priority: 10,
            tags: vec![],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("header.X-OxiFY-Priority")
                .and_then(|v| v.as_str()),
            Some("10")
        );
        assert_eq!(
            mail.get_attribute("oxify.priority")
                .and_then(|v| v.as_i64()),
            Some(10)
        );
    }

    #[tokio::test]
    async fn test_oxify_priority_high_urgent() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Work],
            priority: 9,
            tags: vec![],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("oxify.is_urgent")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_oxify_auto_tagging() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Work],
            priority: 5,
            tags: vec![
                "meeting".to_string(),
                "invoice".to_string(),
                "action-required".to_string(),
            ],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        let tags = mail
            .get_attribute("header.X-OxiFY-Tags")
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(tags.contains("meeting"));
        assert!(tags.contains("invoice"));
        assert!(tags.contains("action-required"));
    }

    #[tokio::test]
    async fn test_oxify_folder_routing() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Newsletter],
            priority: 3,
            tags: vec![],
            folder: Some("Newsletters".to_string()),
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("oxify.folder").and_then(|v| v.as_str()),
            Some("Newsletters")
        );
    }

    #[tokio::test]
    async fn test_oxify_folder_mapping() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Work],
            priority: 5,
            tags: vec![],
            folder: None,
        });

        let mut folder_mapping = HashMap::new();
        folder_mapping.insert("work".to_string(), "Work".to_string());

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            folder_mapping,
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("oxify.folder").and_then(|v| v.as_str()),
            Some("Work")
        );
    }

    #[tokio::test]
    async fn test_oxify_network_error() {
        let mock = MockHttpClient::with_error(MockError::Network("DNS failed".to_string()));

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        // Should continue on network error
        let result = mailet.service(&mut mail).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), MailetAction::Continue));
    }

    #[tokio::test]
    async fn test_oxify_timeout_error() {
        let mock = MockHttpClient::with_error(MockError::Timeout);

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        // Should continue on timeout
        let result = mailet.service(&mut mail).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), MailetAction::Continue));
    }

    #[tokio::test]
    async fn test_oxify_rate_limited() {
        let mock = MockHttpClient::with_error(MockError::RateLimited);

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        // Should continue on rate limit
        let result = mailet.service(&mut mail).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), MailetAction::Continue));
    }

    #[tokio::test]
    async fn test_oxify_api_error() {
        let mock =
            MockHttpClient::with_error(MockError::Api(500, "Internal Server Error".to_string()));

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        // Should continue on API error
        let result = mailet.service(&mut mail).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), MailetAction::Continue));
    }

    #[tokio::test]
    async fn test_oxify_parse_error() {
        let mock = MockHttpClient::with_error(MockError::Parse("Invalid JSON".to_string()));

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        // Should continue on parse error
        let result = mailet.service(&mut mail).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), MailetAction::Continue));
    }

    #[tokio::test]
    async fn test_oxify_disabled() {
        let mock = MockHttpClient::with_success(create_success_response());

        let config = OxiFYConfig {
            enabled: false,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        // Should continue when disabled
        let result = mailet.service(&mut mail).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), MailetAction::Continue));

        // Should not add any headers
        assert!(mail.get_attribute("header.X-OxiFY-Sentiment").is_none());
    }

    #[tokio::test]
    async fn test_oxify_config_timeout() {
        let mut mailet = OxiFYMailet::<ReqwestClient>::new();
        let config = MailetConfig::new("OxiFY")
            .with_param("enabled", "false")
            .with_param("api_url", "http://localhost:8080/api/v1/analyze")
            .with_param("api_key", "test_key")
            .with_param("timeout_ms", "10000");

        mailet.init(config).await.unwrap();
    }

    #[tokio::test]
    async fn test_oxify_config_cache_ttl() {
        let mut mailet = OxiFYMailet::<ReqwestClient>::new();
        let config = MailetConfig::new("OxiFY")
            .with_param("enabled", "false")
            .with_param("api_url", "http://localhost:8080/api/v1/analyze")
            .with_param("api_key", "test_key")
            .with_param("cache_ttl", "7200");

        mailet.init(config).await.unwrap();
    }

    #[tokio::test]
    async fn test_oxify_config_max_body_size() {
        let mut mailet = OxiFYMailet::<ReqwestClient>::new();
        let config = MailetConfig::new("OxiFY")
            .with_param("enabled", "false")
            .with_param("api_url", "http://localhost:8080/api/v1/analyze")
            .with_param("api_key", "test_key")
            .with_param("max_body_size", "102400");

        mailet.init(config).await.unwrap();
    }

    #[tokio::test]
    async fn test_mail_category_from_str() {
        assert_eq!(MailCategory::parse("work"), Some(MailCategory::Work));
        assert_eq!(MailCategory::parse("SPAM"), Some(MailCategory::Spam));
        assert_eq!(MailCategory::parse("urgent"), Some(MailCategory::Urgent));
        assert_eq!(MailCategory::parse("invalid"), None);
    }

    #[tokio::test]
    async fn test_mail_category_as_str() {
        assert_eq!(MailCategory::Work.as_str(), "work");
        assert_eq!(MailCategory::Spam.as_str(), "spam");
        assert_eq!(MailCategory::Urgent.as_str(), "urgent");
    }

    #[tokio::test]
    async fn test_sentiment_from_str() {
        assert_eq!(Sentiment::parse("positive"), Some(Sentiment::Positive));
        assert_eq!(Sentiment::parse("NEGATIVE"), Some(Sentiment::Negative));
        assert_eq!(Sentiment::parse("neutral"), Some(Sentiment::Neutral));
        assert_eq!(Sentiment::parse("invalid"), None);
    }

    #[tokio::test]
    async fn test_sentiment_as_str() {
        assert_eq!(Sentiment::Positive.as_str(), "positive");
        assert_eq!(Sentiment::Negative.as_str(), "negative");
        assert_eq!(Sentiment::Neutral.as_str(), "neutral");
    }

    #[tokio::test]
    async fn test_oxify_sentiment_score_validation() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Positive,
            sentiment_score: 1.5, // Out of range, should be clamped
            categories: vec![MailCategory::Personal],
            priority: 5,
            tags: vec![],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let service = OxiFYService::with_client(config, mock);
        let mail = create_test_mail();

        let result = service.analyze(&mail).await.unwrap();
        assert_eq!(result.sentiment_score, 1.0); // Clamped to 1.0
    }

    #[tokio::test]
    async fn test_oxify_priority_validation() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Work],
            priority: 15, // Out of range, should be clamped
            tags: vec![],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let service = OxiFYService::with_client(config, mock);
        let mail = create_test_mail();

        let result = service.analyze(&mail).await.unwrap();
        assert_eq!(result.priority, 10); // Clamped to 10
    }

    #[tokio::test]
    async fn test_oxify_empty_mail() {
        let mock = MockHttpClient::with_success(AnalysisResponse {
            sentiment: Sentiment::Neutral,
            sentiment_score: 0.5,
            categories: vec![MailCategory::Personal],
            priority: 5,
            tags: vec![],
            folder: None,
        });

        let config = OxiFYConfig {
            enabled: true,
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mailet = OxiFYMailet::with_client(mock, config);
        let mut mail = create_test_mail();

        // Should process without errors
        let result = mailet.service(&mut mail).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_oxify_default() {
        let mailet = OxiFYMailet::<ReqwestClient>::default();
        assert_eq!(mailet.name(), "OxiFY");
    }
}

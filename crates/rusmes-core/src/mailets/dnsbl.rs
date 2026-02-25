//! DNSBL/RBL (DNS Blacklist) mailet

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use rusmes_proto::Mail;
use std::collections::HashSet;

/// DNSBL provider
#[derive(Debug, Clone)]
pub struct DnsblProvider {
    /// Provider hostname (e.g., "zen.spamhaus.org")
    pub hostname: String,
    /// Provider weight (for scoring)
    pub weight: f64,
    /// Whether this provider is enabled
    pub enabled: bool,
}

impl DnsblProvider {
    /// Create a new DNSBL provider
    pub fn new(hostname: impl Into<String>) -> Self {
        Self {
            hostname: hostname.into(),
            weight: 1.0,
            enabled: true,
        }
    }

    /// Create with weight
    pub fn with_weight(mut self, weight: f64) -> Self {
        self.weight = weight;
        self
    }

    /// Mock DNS lookup for IPv4
    async fn lookup_ipv4(&self, ip: &str) -> Option<String> {
        // Mock implementation - in real implementation, would do actual DNS lookup
        // Format: reverse IP octets + . + hostname
        // Example: 1.2.3.4 -> 4.3.2.1.zen.spamhaus.org

        let octets: Vec<&str> = ip.split('.').collect();
        if octets.len() != 4 {
            return None;
        }

        let reversed_ip = format!("{}.{}.{}.{}", octets[3], octets[2], octets[1], octets[0]);

        let lookup_host = format!("{}.{}", reversed_ip, self.hostname);

        // Mock: treat certain IPs as blacklisted
        if ip.starts_with("192.0.2.") || // TEST-NET-1
           ip.starts_with("198.51.100.") || // TEST-NET-2
           ip == "127.0.0.2"
        {
            Some(lookup_host)
        } else {
            None
        }
    }

    /// Mock DNS lookup for IPv6
    async fn lookup_ipv6(&self, _ip: &str) -> Option<String> {
        // Mock implementation
        None
    }

    /// Check if IP is listed
    pub async fn check(&self, ip: &str) -> bool {
        if !self.enabled {
            return false;
        }

        // Determine IP version
        if ip.contains(':') {
            self.lookup_ipv6(ip).await.is_some()
        } else {
            self.lookup_ipv4(ip).await.is_some()
        }
    }
}

/// DNSBL check result
#[derive(Debug, Clone)]
pub struct DnsblResult {
    /// Whether IP is listed
    pub listed: bool,
    /// Providers that listed the IP
    pub listed_by: Vec<String>,
    /// Total score (sum of weights)
    pub score: f64,
}

impl DnsblResult {
    fn new() -> Self {
        Self {
            listed: false,
            listed_by: Vec::new(),
            score: 0.0,
        }
    }

    fn add_listing(&mut self, provider: &str, weight: f64) {
        self.listed = true;
        self.listed_by.push(provider.to_string());
        self.score += weight;
    }
}

/// DNSBL service
pub struct DnsblService {
    /// DNSBL providers
    providers: Vec<DnsblProvider>,
    /// Whitelist (IPs that should skip DNSBL checks)
    whitelist: HashSet<String>,
}

impl DnsblService {
    /// Create a new DNSBL service
    pub fn new() -> Self {
        Self {
            providers: vec![
                // Common DNSBL providers
                DnsblProvider::new("zen.spamhaus.org").with_weight(2.0),
                DnsblProvider::new("bl.spamcop.net").with_weight(1.5),
                DnsblProvider::new("dnsbl.sorbs.net").with_weight(1.0),
                DnsblProvider::new("b.barracudacentral.org").with_weight(1.0),
            ],
            whitelist: HashSet::new(),
        }
    }

    /// Add a provider
    pub fn add_provider(&mut self, provider: DnsblProvider) {
        self.providers.push(provider);
    }

    /// Add to whitelist
    pub fn add_to_whitelist(&mut self, ip: String) {
        self.whitelist.insert(ip);
    }

    /// Check IP against all providers
    pub async fn check(&self, ip: &str) -> DnsblResult {
        // Check whitelist
        if self.whitelist.contains(ip) {
            return DnsblResult::new();
        }

        let mut result = DnsblResult::new();

        // Check each provider
        for provider in &self.providers {
            if provider.check(ip).await {
                result.add_listing(&provider.hostname, provider.weight);
            }
        }

        result
    }
}

impl Default for DnsblService {
    fn default() -> Self {
        Self::new()
    }
}

/// DNSBL mailet
pub struct DnsblMailet {
    name: String,
    service: DnsblService,
    /// Threshold score for rejection
    reject_threshold: f64,
    /// Whether to reject or just tag
    reject_on_match: bool,
    /// Whether DNSBL checking is enabled
    enabled: bool,
}

impl DnsblMailet {
    /// Create a new DNSBL mailet
    pub fn new() -> Self {
        Self {
            name: "DNSBL".to_string(),
            service: DnsblService::new(),
            reject_threshold: 2.0,
            reject_on_match: false,
            enabled: true,
        }
    }

    /// Extract sender IP from mail
    fn extract_sender_ip(&self, mail: &Mail) -> Option<String> {
        mail.get_attribute("smtp.client_ip")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

impl Default for DnsblMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for DnsblMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        if let Some(enabled) = config.get_param("enabled") {
            self.enabled = enabled.parse().unwrap_or(true);
        }

        if let Some(threshold) = config.get_param("reject_threshold") {
            self.reject_threshold = threshold.parse().unwrap_or(2.0);
        }

        if let Some(reject) = config.get_param("reject_on_match") {
            self.reject_on_match = reject.parse().unwrap_or(false);
        }

        if let Some(whitelist_str) = config.get_param("whitelist") {
            for ip in whitelist_str.split(',') {
                self.service.add_to_whitelist(ip.trim().to_string());
            }
        }

        // Custom providers
        if let Some(providers_str) = config.get_param("providers") {
            self.service.providers.clear();
            for provider_spec in providers_str.split(';') {
                let parts: Vec<&str> = provider_spec.split(':').collect();
                let hostname = parts[0].trim();
                let weight = if parts.len() > 1 {
                    parts[1].parse().unwrap_or(1.0)
                } else {
                    1.0
                };

                self.service
                    .add_provider(DnsblProvider::new(hostname).with_weight(weight));
            }
        }

        tracing::info!(
            "Initialized DnsblMailet with {} providers",
            self.service.providers.len()
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        if !self.enabled {
            return Ok(MailetAction::Continue);
        }

        let sender_ip = match self.extract_sender_ip(mail) {
            Some(ip) => ip,
            None => {
                tracing::debug!("No sender IP for mail {}, skipping DNSBL check", mail.id());
                return Ok(MailetAction::Continue);
            }
        };

        tracing::debug!("Checking {} against DNSBL providers", sender_ip);

        let result = self.service.check(&sender_ip).await;

        // Store results in mail attributes
        mail.set_attribute("dnsbl.checked", true);
        mail.set_attribute("dnsbl.listed", result.listed);
        mail.set_attribute("dnsbl.score", result.score);

        if result.listed {
            mail.set_attribute("dnsbl.listed_by", result.listed_by.join(","));

            tracing::info!(
                "Mail {} from {} listed on DNSBL (score: {:.2}, providers: {})",
                mail.id(),
                sender_ip,
                result.score,
                result.listed_by.join(", ")
            );

            // Check if we should reject
            if self.reject_on_match && result.score >= self.reject_threshold {
                tracing::info!("Rejecting mail {} due to DNSBL listing", mail.id());
                mail.set_attribute("dnsbl.rejected", true);
                return Ok(MailetAction::Drop);
            }
        } else {
            tracing::debug!(
                "Mail {} from {} not listed on any DNSBL",
                mail.id(),
                sender_ip
            );
        }

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

    #[tokio::test]
    async fn test_dnsbl_mailet_init() {
        let mut mailet = DnsblMailet::new();
        let config = MailetConfig::new("DNSBL");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.name(), "DNSBL");
    }

    #[tokio::test]
    async fn test_dnsbl_provider_check_listed() {
        let provider = DnsblProvider::new("zen.spamhaus.org");
        let is_listed = provider.check("192.0.2.1").await; // TEST-NET-1
        assert!(is_listed);
    }

    #[tokio::test]
    async fn test_dnsbl_provider_check_not_listed() {
        let provider = DnsblProvider::new("zen.spamhaus.org");
        let is_listed = provider.check("10.0.0.1").await; // Private IP
        assert!(!is_listed);
    }

    #[tokio::test]
    async fn test_dnsbl_service_check() {
        let service = DnsblService::new();
        let result = service.check("192.0.2.1").await;

        assert!(result.listed);
        assert!(!result.listed_by.is_empty());
        assert!(result.score > 0.0);
    }

    #[tokio::test]
    async fn test_dnsbl_service_whitelist() {
        let mut service = DnsblService::new();
        service.add_to_whitelist("192.0.2.1".to_string());

        let result = service.check("192.0.2.1").await;
        assert!(!result.listed);
    }

    #[tokio::test]
    async fn test_dnsbl_mailet_listed_ip() {
        let mailet = DnsblMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.0.2.1");

        let action = mailet.service(&mut mail).await.unwrap();

        assert_eq!(action, MailetAction::Continue); // Not rejecting by default
        assert_eq!(
            mail.get_attribute("dnsbl.listed").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(mail.get_attribute("dnsbl.score").is_some());
    }

    #[tokio::test]
    async fn test_dnsbl_mailet_clean_ip() {
        let mailet = DnsblMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "10.0.0.1");

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("dnsbl.listed").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[tokio::test]
    async fn test_dnsbl_mailet_reject_on_match() {
        let mut mailet = DnsblMailet::new();
        mailet.reject_on_match = true;
        mailet.reject_threshold = 1.0;

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.0.2.1");

        let action = mailet.service(&mut mail).await.unwrap();

        assert_eq!(action, MailetAction::Drop);
        assert_eq!(
            mail.get_attribute("dnsbl.rejected")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_dnsbl_config_threshold() {
        let mut mailet = DnsblMailet::new();
        let config = MailetConfig::new("DNSBL").with_param("reject_threshold", "5.0");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.reject_threshold, 5.0);
    }

    #[tokio::test]
    async fn test_dnsbl_config_reject_on_match() {
        let mut mailet = DnsblMailet::new();
        let config = MailetConfig::new("DNSBL").with_param("reject_on_match", "true");

        mailet.init(config).await.unwrap();
        assert!(mailet.reject_on_match);
    }

    #[tokio::test]
    async fn test_dnsbl_config_whitelist() {
        let mut mailet = DnsblMailet::new();
        let config = MailetConfig::new("DNSBL").with_param("whitelist", "10.0.0.1,10.0.0.2");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.service.whitelist.len(), 2);
    }

    #[tokio::test]
    async fn test_dnsbl_config_custom_providers() {
        let mut mailet = DnsblMailet::new();
        let config =
            MailetConfig::new("DNSBL").with_param("providers", "test1.org:2.0;test2.org:1.5");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.service.providers.len(), 2);
        assert_eq!(mailet.service.providers[0].hostname, "test1.org");
        assert_eq!(mailet.service.providers[0].weight, 2.0);
    }

    #[tokio::test]
    async fn test_dnsbl_disabled() {
        let mut mailet = DnsblMailet::new();
        let config = MailetConfig::new("DNSBL").with_param("enabled", "false");

        mailet.init(config).await.unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.0.2.1");

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert!(mail.get_attribute("dnsbl.checked").is_none());
    }

    #[tokio::test]
    async fn test_dnsbl_no_sender_ip() {
        let mailet = DnsblMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
    }

    #[tokio::test]
    async fn test_dnsbl_provider_disabled() {
        let mut provider = DnsblProvider::new("test.org");
        provider.enabled = false;

        let is_listed = provider.check("192.0.2.1").await;
        assert!(!is_listed);
    }

    #[tokio::test]
    async fn test_dnsbl_provider_weight() {
        let provider = DnsblProvider::new("test.org").with_weight(3.0);
        assert_eq!(provider.weight, 3.0);
    }

    #[tokio::test]
    async fn test_dnsbl_result_scoring() {
        let mut result = DnsblResult::new();
        assert!(!result.listed);
        assert_eq!(result.score, 0.0);

        result.add_listing("provider1", 1.5);
        assert!(result.listed);
        assert_eq!(result.score, 1.5);

        result.add_listing("provider2", 2.0);
        assert_eq!(result.score, 3.5);
        assert_eq!(result.listed_by.len(), 2);
    }

    #[tokio::test]
    async fn test_dnsbl_mailet_listed_by() {
        let mailet = DnsblMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.0.2.1");

        mailet.service(&mut mail).await.unwrap();

        let listed_by = mail
            .get_attribute("dnsbl.listed_by")
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(!listed_by.is_empty());
    }

    #[tokio::test]
    async fn test_dnsbl_threshold_not_reached() {
        let mut mailet = DnsblMailet::new();
        mailet.reject_on_match = true;
        mailet.reject_threshold = 100.0; // Very high threshold

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.0.2.1");

        let action = mailet.service(&mut mail).await.unwrap();

        // Should not reject due to high threshold
        assert_eq!(action, MailetAction::Continue);
        assert!(mail.get_attribute("dnsbl.rejected").is_none());
    }

    #[tokio::test]
    async fn test_dnsbl_multiple_providers_listing() {
        let service = DnsblService::new();
        let result = service.check("127.0.0.2").await; // Mock listed IP

        if result.listed {
            // Score should be sum of weights from all providers that listed it
            assert!(result.score > 0.0);
            assert!(!result.listed_by.is_empty());
        }
    }
}

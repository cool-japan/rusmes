//! SPF (Sender Policy Framework) verification mailet
//!
//! Implements RFC 7208 - Sender Policy Framework (SPF)

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use hickory_resolver::TokioResolver;
use ipnetwork::{Ipv4Network, Ipv6Network};
use rusmes_proto::Mail;
use std::net::IpAddr;

/// SPF mechanism types
#[derive(Debug, Clone)]
#[allow(clippy::upper_case_acronyms)]
enum SpfMechanism {
    /// a[:domain][/prefix]
    A {
        domain: Option<String>,
        prefix: Option<u8>,
    },
    /// mx[:domain][/prefix]
    MX {
        domain: Option<String>,
        prefix: Option<u8>,
    },
    /// ip4:network[/prefix]
    IP4 { network: Ipv4Network },
    /// ip6:network[/prefix]
    IP6 { network: Ipv6Network },
    /// include:domain
    Include { domain: String },
    /// all
    All,
    /// exists:domain
    Exists { domain: String },
    /// ptr[:domain]
    Ptr {
        #[allow(dead_code)]
        domain: Option<String>,
    },
}

/// SPF qualifier (result modifier)
#[derive(Debug, Clone, Copy)]
enum SpfQualifier {
    /// + (Pass)
    Pass,
    /// - (Fail)
    Fail,
    /// ~ (SoftFail)
    SoftFail,
    /// ? (Neutral)
    Neutral,
}

/// SPF evaluation result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpfResult {
    /// SPF check passed
    Pass,
    /// SPF check failed (hard fail)
    Fail,
    /// SPF check failed (soft fail)
    SoftFail,
    /// SPF is neutral (not pass, not fail)
    Neutral,
    /// Temporary DNS error
    TempError,
    /// Permanent error in SPF record
    PermError,
    /// No SPF record found
    None,
}

impl SpfResult {
    /// Convert to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            SpfResult::Pass => "pass",
            SpfResult::Fail => "fail",
            SpfResult::SoftFail => "softfail",
            SpfResult::Neutral => "neutral",
            SpfResult::TempError => "temperror",
            SpfResult::PermError => "permerror",
            SpfResult::None => "none",
        }
    }
}

/// SPF policy checking mailet
pub struct SpfCheckMailet {
    name: String,
    reject_on_fail: bool,
    max_dns_lookups: usize,
}

impl SpfCheckMailet {
    /// Create a new SPF check mailet
    pub fn new() -> Self {
        Self {
            name: "SpfCheck".to_string(),
            reject_on_fail: false,
            max_dns_lookups: 10, // RFC 7208 limit
        }
    }

    /// Lookup SPF record from DNS TXT records
    async fn lookup_spf_record(resolver: &TokioResolver, domain: &str) -> Result<Option<String>> {
        match resolver.txt_lookup(domain).await {
            Ok(txt_records) => {
                // Find the SPF record (starts with "v=spf1")
                for record in txt_records.iter() {
                    // Concatenate all strings in the TXT record
                    let txt = record
                        .txt_data()
                        .iter()
                        .map(|data| String::from_utf8_lossy(data))
                        .collect::<Vec<_>>()
                        .join("");

                    if txt.starts_with("v=spf1") {
                        return Ok(Some(txt));
                    }
                }
                Ok(None)
            }
            Err(e) => {
                if e.to_string().contains("no records found") {
                    Ok(None)
                } else {
                    Err(anyhow!("DNS lookup error: {}", e))
                }
            }
        }
    }

    /// Parse qualifier from mechanism string
    fn parse_qualifier(s: &str) -> (SpfQualifier, &str) {
        match s.chars().next() {
            Some('+') => (SpfQualifier::Pass, &s[1..]),
            Some('-') => (SpfQualifier::Fail, &s[1..]),
            Some('~') => (SpfQualifier::SoftFail, &s[1..]),
            Some('?') => (SpfQualifier::Neutral, &s[1..]),
            _ => (SpfQualifier::Pass, s), // Default is Pass
        }
    }

    /// Parse SPF mechanism from string
    fn parse_mechanism(s: &str) -> Result<SpfMechanism> {
        if s == "all" {
            return Ok(SpfMechanism::All);
        }

        if let Some(domain) = s.strip_prefix("include:") {
            return Ok(SpfMechanism::Include {
                domain: domain.to_string(),
            });
        }

        if let Some(network_str) = s.strip_prefix("ip4:") {
            let network = if network_str.contains('/') {
                network_str.parse::<Ipv4Network>()?
            } else {
                Ipv4Network::new(network_str.parse()?, 32)?
            };
            return Ok(SpfMechanism::IP4 { network });
        }

        if let Some(network_str) = s.strip_prefix("ip6:") {
            let network = if network_str.contains('/') {
                network_str.parse::<Ipv6Network>()?
            } else {
                Ipv6Network::new(network_str.parse()?, 128)?
            };
            return Ok(SpfMechanism::IP6 { network });
        }

        if let Some(domain_str) = s.strip_prefix("exists:") {
            return Ok(SpfMechanism::Exists {
                domain: domain_str.to_string(),
            });
        }

        if s.starts_with("a:") || s.starts_with("a/") || s == "a" {
            let (domain, prefix) = Self::parse_domain_and_prefix(&s[1..])?;
            return Ok(SpfMechanism::A { domain, prefix });
        }

        if s.starts_with("mx:") || s.starts_with("mx/") || s == "mx" {
            let (domain, prefix) = Self::parse_domain_and_prefix(&s[2..])?;
            return Ok(SpfMechanism::MX { domain, prefix });
        }

        if s.starts_with("ptr:") || s == "ptr" {
            let domain = if s.len() > 4 {
                Some(s[4..].to_string())
            } else {
                None
            };
            return Ok(SpfMechanism::Ptr { domain });
        }

        Err(anyhow!("Unknown SPF mechanism: {}", s))
    }

    /// Parse domain and prefix from mechanism string
    fn parse_domain_and_prefix(s: &str) -> Result<(Option<String>, Option<u8>)> {
        if s.is_empty() {
            return Ok((None, None));
        }

        if let Some(rest) = s.strip_prefix(':') {
            if let Some((domain, prefix_str)) = rest.split_once('/') {
                let prefix = prefix_str.parse()?;
                Ok((Some(domain.to_string()), Some(prefix)))
            } else {
                Ok((Some(rest.to_string()), None))
            }
        } else if let Some(stripped) = s.strip_prefix('/') {
            let prefix = stripped.parse()?;
            Ok((None, Some(prefix)))
        } else {
            Err(anyhow!("Invalid domain/prefix format: {}", s))
        }
    }

    /// Parse SPF record into list of mechanisms with qualifiers
    fn parse_spf_record(record: &str) -> Result<Vec<(SpfQualifier, SpfMechanism)>> {
        let parts: Vec<&str> = record.split_whitespace().collect();
        let mut mechanisms = Vec::new();

        for part in parts.iter().skip(1) {
            // Skip "v=spf1"
            // Skip modifiers (redirect=, exp=)
            if part.contains('=') && !part.starts_with("ip4:") && !part.starts_with("ip6:") {
                continue;
            }

            let (qualifier, mechanism_str) = Self::parse_qualifier(part);
            match Self::parse_mechanism(mechanism_str) {
                Ok(mechanism) => {
                    mechanisms.push((qualifier, mechanism));
                }
                Err(e) => {
                    tracing::warn!("Failed to parse SPF mechanism '{}': {}", part, e);
                    // Continue parsing other mechanisms
                }
            }
        }

        Ok(mechanisms)
    }

    /// Check if IP matches A record mechanism
    async fn check_a_mechanism(
        resolver: &TokioResolver,
        sender_ip: IpAddr,
        domain: &str,
        prefix: Option<u8>,
        lookup_count: &mut usize,
    ) -> Result<bool> {
        *lookup_count += 1;
        if *lookup_count > 10 {
            return Err(anyhow!("Too many DNS lookups"));
        }

        match sender_ip {
            IpAddr::V4(v4) => {
                let a_records = resolver.ipv4_lookup(domain).await?;
                for record in a_records.iter() {
                    let ip = record.0;
                    let network = if let Some(prefix_len) = prefix {
                        Ipv4Network::new(ip, prefix_len)?
                    } else {
                        Ipv4Network::new(ip, 32)?
                    };
                    if network.contains(v4) {
                        return Ok(true);
                    }
                }
            }
            IpAddr::V6(v6) => {
                let aaaa_records = resolver.ipv6_lookup(domain).await?;
                for record in aaaa_records.iter() {
                    let ip = record.0;
                    let network = if let Some(prefix_len) = prefix {
                        Ipv6Network::new(ip, prefix_len)?
                    } else {
                        Ipv6Network::new(ip, 128)?
                    };
                    if network.contains(v6) {
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    /// Check if IP matches MX record mechanism
    async fn check_mx_mechanism(
        resolver: &TokioResolver,
        sender_ip: IpAddr,
        domain: &str,
        prefix: Option<u8>,
        lookup_count: &mut usize,
    ) -> Result<bool> {
        *lookup_count += 1;
        if *lookup_count > 10 {
            return Err(anyhow!("Too many DNS lookups"));
        }

        let mx_records = resolver.mx_lookup(domain).await?;

        for mx in mx_records.iter() {
            let mx_host = mx.exchange().to_string();
            // Remove trailing dot
            let mx_host = mx_host.trim_end_matches('.');

            // Lookup A/AAAA records for MX host
            if Self::check_a_mechanism(resolver, sender_ip, mx_host, prefix, lookup_count)
                .await
                .unwrap_or(false)
            {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Check if domain exists (for exists: mechanism)
    async fn check_exists(
        resolver: &TokioResolver,
        domain: &str,
        lookup_count: &mut usize,
    ) -> Result<bool> {
        *lookup_count += 1;
        if *lookup_count > 10 {
            return Err(anyhow!("Too many DNS lookups"));
        }

        // Check if domain has any A records
        match resolver.ipv4_lookup(domain).await {
            Ok(records) => Ok(records.iter().count() > 0),
            Err(_) => Ok(false),
        }
    }

    /// Evaluate SPF for given sender IP and domain
    #[allow(clippy::too_many_arguments)]
    async fn evaluate_spf_internal(
        resolver: &TokioResolver,
        sender_ip: IpAddr,
        sender_domain: &str,
        lookup_count: &mut usize,
        recursion_depth: usize,
    ) -> Result<SpfResult> {
        // Prevent infinite recursion
        if recursion_depth > 10 {
            return Ok(SpfResult::PermError);
        }

        // Lookup SPF record
        let spf_record = match Self::lookup_spf_record(resolver, sender_domain).await {
            Ok(Some(record)) => record,
            Ok(None) => return Ok(SpfResult::None),
            Err(e) => {
                tracing::warn!("SPF lookup error for {}: {}", sender_domain, e);
                return Ok(SpfResult::TempError);
            }
        };

        tracing::debug!("Found SPF record for {}: {}", sender_domain, spf_record);

        // Parse mechanisms
        let mechanisms = match Self::parse_spf_record(&spf_record) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Failed to parse SPF record: {}", e);
                return Ok(SpfResult::PermError);
            }
        };

        // Evaluate each mechanism
        for (qualifier, mechanism) in mechanisms {
            let matches = match mechanism {
                SpfMechanism::A { domain, prefix } => {
                    let check_domain = domain.as_deref().unwrap_or(sender_domain);
                    Self::check_a_mechanism(resolver, sender_ip, check_domain, prefix, lookup_count)
                        .await
                        .unwrap_or(false)
                }
                SpfMechanism::MX { domain, prefix } => {
                    let check_domain = domain.as_deref().unwrap_or(sender_domain);
                    Self::check_mx_mechanism(
                        resolver,
                        sender_ip,
                        check_domain,
                        prefix,
                        lookup_count,
                    )
                    .await
                    .unwrap_or(false)
                }
                SpfMechanism::IP4 { network } => match sender_ip {
                    IpAddr::V4(v4) => network.contains(v4),
                    IpAddr::V6(_) => false,
                },
                SpfMechanism::IP6 { network } => match sender_ip {
                    IpAddr::V6(v6) => network.contains(v6),
                    IpAddr::V4(_) => false,
                },
                SpfMechanism::Include { domain } => {
                    let result = Box::pin(Self::evaluate_spf_internal(
                        resolver,
                        sender_ip,
                        &domain,
                        lookup_count,
                        recursion_depth + 1,
                    ))
                    .await
                    .unwrap_or(SpfResult::TempError);

                    matches!(result, SpfResult::Pass)
                }
                SpfMechanism::Exists { domain } => {
                    Self::check_exists(resolver, &domain, lookup_count)
                        .await
                        .unwrap_or(false)
                }
                SpfMechanism::Ptr { .. } => {
                    // Ptr mechanism is deprecated and not recommended
                    // We'll skip it for now
                    tracing::warn!("Ptr mechanism is deprecated and not supported");
                    false
                }
                SpfMechanism::All => true,
            };

            if matches {
                return Ok(match qualifier {
                    SpfQualifier::Pass => SpfResult::Pass,
                    SpfQualifier::Fail => SpfResult::Fail,
                    SpfQualifier::SoftFail => SpfResult::SoftFail,
                    SpfQualifier::Neutral => SpfResult::Neutral,
                });
            }
        }

        // No mechanisms matched
        Ok(SpfResult::Neutral)
    }

    /// Evaluate SPF for given sender IP and domain
    async fn evaluate_spf(sender_ip: IpAddr, sender_domain: &str) -> Result<SpfResult> {
        let resolver = TokioResolver::builder_tokio()
            .map_err(|e| anyhow!("Failed to create DNS resolver: {}", e))?
            .build();

        let mut lookup_count = 0;
        Self::evaluate_spf_internal(&resolver, sender_ip, sender_domain, &mut lookup_count, 0).await
    }
}

impl Default for SpfCheckMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for SpfCheckMailet {
    async fn init(&mut self, config: MailetConfig) -> Result<()> {
        if let Some(reject_str) = config.get_param("reject_on_fail") {
            self.reject_on_fail = reject_str.parse()?;
        }

        if let Some(max_lookups_str) = config.get_param("max_dns_lookups") {
            self.max_dns_lookups = max_lookups_str.parse()?;
        }

        tracing::info!(
            "Initialized SpfCheckMailet (reject on fail: {}, max DNS lookups: {})",
            self.reject_on_fail,
            self.max_dns_lookups
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> Result<MailetAction> {
        tracing::debug!("Checking SPF for mail {}", mail.id());

        // Get sender IP address
        let sender_ip = match mail.remote_addr() {
            Some(ip) => *ip,
            None => {
                tracing::warn!(
                    "No remote address for mail {}, skipping SPF check",
                    mail.id()
                );
                mail.set_attribute("spf.result", SpfResult::None.as_str());
                return Ok(MailetAction::Continue);
            }
        };

        // Get sender domain from envelope sender
        let sender_domain = match mail.sender() {
            Some(addr) => addr.domain().as_str().to_string(),
            None => {
                tracing::debug!(
                    "No sender for mail {} (bounce?), skipping SPF check",
                    mail.id()
                );
                mail.set_attribute("spf.result", SpfResult::None.as_str());
                return Ok(MailetAction::Continue);
            }
        };

        // Evaluate SPF
        let result = Self::evaluate_spf(sender_ip, &sender_domain).await;

        let spf_result = match result {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("SPF evaluation error for mail {}: {}", mail.id(), e);
                SpfResult::TempError
            }
        };

        // Set attributes
        mail.set_attribute("spf.result", spf_result.as_str());
        mail.set_attribute("spf.client_ip", sender_ip.to_string());
        mail.set_attribute("spf.sender_domain", sender_domain.clone());

        // Handle result
        match spf_result {
            SpfResult::Pass => {
                tracing::info!(
                    "SPF check passed for mail {} (domain: {}, ip: {})",
                    mail.id(),
                    sender_domain,
                    sender_ip
                );
            }
            SpfResult::Fail => {
                tracing::warn!(
                    "SPF check failed for mail {} (domain: {}, ip: {})",
                    mail.id(),
                    sender_domain,
                    sender_ip
                );
                if self.reject_on_fail {
                    mail.state = rusmes_proto::MailState::Error;
                    return Ok(MailetAction::Drop);
                }
            }
            SpfResult::SoftFail => {
                tracing::info!(
                    "SPF soft fail for mail {} (domain: {}, ip: {})",
                    mail.id(),
                    sender_domain,
                    sender_ip
                );
            }
            SpfResult::Neutral => {
                tracing::debug!(
                    "SPF neutral for mail {} (domain: {}, ip: {})",
                    mail.id(),
                    sender_domain,
                    sender_ip
                );
            }
            SpfResult::TempError => {
                tracing::warn!(
                    "SPF temporary error for mail {} (domain: {}, ip: {})",
                    mail.id(),
                    sender_domain,
                    sender_ip
                );
            }
            SpfResult::PermError => {
                tracing::warn!(
                    "SPF permanent error for mail {} (domain: {}, ip: {})",
                    mail.id(),
                    sender_domain,
                    sender_ip
                );
            }
            SpfResult::None => {
                tracing::debug!(
                    "No SPF record for mail {} (domain: {})",
                    mail.id(),
                    sender_domain
                );
            }
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
    use std::net::IpAddr;
    use std::str::FromStr;

    fn create_test_mail(sender: &str, recipients: Vec<&str>, remote_ip: Option<IpAddr>) -> Mail {
        let sender_addr = MailAddress::from_str(sender).ok();
        let recipient_addrs: Vec<MailAddress> = recipients
            .iter()
            .filter_map(|r| MailAddress::from_str(r).ok())
            .collect();

        let message = MimeMessage::new(
            HeaderMap::new(),
            MessageBody::Small(Bytes::from("Test message")),
        );

        Mail::new(sender_addr, recipient_addrs, message, remote_ip, None)
    }

    #[tokio::test]
    async fn test_spf_check_mailet_creation() {
        let mailet = SpfCheckMailet::new();
        assert_eq!(mailet.name(), "SpfCheck");
        assert!(!mailet.reject_on_fail);
        assert_eq!(mailet.max_dns_lookups, 10);
    }

    #[tokio::test]
    async fn test_spf_check_mailet_default() {
        let mailet = SpfCheckMailet::default();
        assert_eq!(mailet.name(), "SpfCheck");
    }

    #[tokio::test]
    async fn test_spf_check_init_with_config() {
        let mut mailet = SpfCheckMailet::new();
        let mut config = MailetConfig::new("SpfCheck");
        config = config.with_param("reject_on_fail".to_string(), "true".to_string());
        config = config.with_param("max_dns_lookups".to_string(), "15".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_ok());
        assert!(mailet.reject_on_fail);
        assert_eq!(mailet.max_dns_lookups, 15);
    }

    #[tokio::test]
    async fn test_spf_check_no_remote_address() {
        let mailet = SpfCheckMailet::new();
        let mut mail = create_test_mail("sender@example.com", vec!["recipient@test.com"], None);

        let action = mailet.service(&mut mail).await.unwrap();
        assert!(matches!(action, MailetAction::Continue));

        let spf_result = mail.get_attribute("spf.result").and_then(|v| v.as_str());
        assert_eq!(spf_result, Some("none"));
    }

    #[tokio::test]
    async fn test_spf_check_no_sender() {
        let mailet = SpfCheckMailet::new();
        let remote_ip = IpAddr::from_str("192.0.2.1").unwrap();
        let recipient_addrs = vec![MailAddress::from_str("recipient@test.com").unwrap()];

        let message = MimeMessage::new(
            HeaderMap::new(),
            MessageBody::Small(Bytes::from("Test message")),
        );

        let mut mail = Mail::new(None, recipient_addrs, message, Some(remote_ip), None);

        let action = mailet.service(&mut mail).await.unwrap();
        assert!(matches!(action, MailetAction::Continue));

        let spf_result = mail.get_attribute("spf.result").and_then(|v| v.as_str());
        assert_eq!(spf_result, Some("none"));
    }

    #[tokio::test]
    async fn test_spf_result_as_str() {
        assert_eq!(SpfResult::Pass.as_str(), "pass");
        assert_eq!(SpfResult::Fail.as_str(), "fail");
        assert_eq!(SpfResult::SoftFail.as_str(), "softfail");
        assert_eq!(SpfResult::Neutral.as_str(), "neutral");
        assert_eq!(SpfResult::TempError.as_str(), "temperror");
        assert_eq!(SpfResult::PermError.as_str(), "permerror");
        assert_eq!(SpfResult::None.as_str(), "none");
    }

    #[test]
    fn test_parse_qualifier_pass() {
        let (qual, rest) = SpfCheckMailet::parse_qualifier("+ip4:192.0.2.0/24");
        assert!(matches!(qual, SpfQualifier::Pass));
        assert_eq!(rest, "ip4:192.0.2.0/24");
    }

    #[test]
    fn test_parse_qualifier_fail() {
        let (qual, rest) = SpfCheckMailet::parse_qualifier("-all");
        assert!(matches!(qual, SpfQualifier::Fail));
        assert_eq!(rest, "all");
    }

    #[test]
    fn test_parse_qualifier_softfail() {
        let (qual, rest) = SpfCheckMailet::parse_qualifier("~all");
        assert!(matches!(qual, SpfQualifier::SoftFail));
        assert_eq!(rest, "all");
    }

    #[test]
    fn test_parse_qualifier_neutral() {
        let (qual, rest) = SpfCheckMailet::parse_qualifier("?all");
        assert!(matches!(qual, SpfQualifier::Neutral));
        assert_eq!(rest, "all");
    }

    #[test]
    fn test_parse_qualifier_default() {
        let (qual, rest) = SpfCheckMailet::parse_qualifier("ip4:192.0.2.0/24");
        assert!(matches!(qual, SpfQualifier::Pass));
        assert_eq!(rest, "ip4:192.0.2.0/24");
    }

    #[test]
    fn test_parse_mechanism_all() {
        let mechanism = SpfCheckMailet::parse_mechanism("all").unwrap();
        assert!(matches!(mechanism, SpfMechanism::All));
    }

    #[test]
    fn test_parse_mechanism_ip4() {
        let mechanism = SpfCheckMailet::parse_mechanism("ip4:192.0.2.0/24").unwrap();
        if let SpfMechanism::IP4 { network } = mechanism {
            assert_eq!(network.to_string(), "192.0.2.0/24");
        } else {
            panic!("Expected IP4 mechanism");
        }
    }

    #[test]
    fn test_parse_mechanism_ip4_single() {
        let mechanism = SpfCheckMailet::parse_mechanism("ip4:192.0.2.1").unwrap();
        if let SpfMechanism::IP4 { network } = mechanism {
            assert_eq!(network.to_string(), "192.0.2.1/32");
        } else {
            panic!("Expected IP4 mechanism");
        }
    }

    #[test]
    fn test_parse_mechanism_ip6() {
        let mechanism = SpfCheckMailet::parse_mechanism("ip6:2001:db8::/32").unwrap();
        if let SpfMechanism::IP6 { network } = mechanism {
            assert_eq!(network.to_string(), "2001:db8::/32");
        } else {
            panic!("Expected IP6 mechanism");
        }
    }

    #[test]
    fn test_parse_mechanism_ip6_single() {
        let mechanism = SpfCheckMailet::parse_mechanism("ip6:2001:db8::1").unwrap();
        if let SpfMechanism::IP6 { network } = mechanism {
            assert_eq!(network.to_string(), "2001:db8::1/128");
        } else {
            panic!("Expected IP6 mechanism");
        }
    }

    #[test]
    fn test_parse_mechanism_include() {
        let mechanism = SpfCheckMailet::parse_mechanism("include:example.com").unwrap();
        if let SpfMechanism::Include { domain } = mechanism {
            assert_eq!(domain, "example.com");
        } else {
            panic!("Expected Include mechanism");
        }
    }

    #[test]
    fn test_parse_mechanism_a() {
        let mechanism = SpfCheckMailet::parse_mechanism("a").unwrap();
        if let SpfMechanism::A { domain, prefix } = mechanism {
            assert_eq!(domain, None);
            assert_eq!(prefix, None);
        } else {
            panic!("Expected A mechanism");
        }
    }

    #[test]
    fn test_parse_mechanism_a_with_domain() {
        let mechanism = SpfCheckMailet::parse_mechanism("a:example.com").unwrap();
        if let SpfMechanism::A { domain, prefix } = mechanism {
            assert_eq!(domain, Some("example.com".to_string()));
            assert_eq!(prefix, None);
        } else {
            panic!("Expected A mechanism with domain");
        }
    }

    #[test]
    fn test_parse_mechanism_a_with_prefix() {
        let mechanism = SpfCheckMailet::parse_mechanism("a/24").unwrap();
        if let SpfMechanism::A { domain, prefix } = mechanism {
            assert_eq!(domain, None);
            assert_eq!(prefix, Some(24));
        } else {
            panic!("Expected A mechanism with prefix");
        }
    }

    #[test]
    fn test_parse_mechanism_a_with_domain_and_prefix() {
        let mechanism = SpfCheckMailet::parse_mechanism("a:example.com/24").unwrap();
        if let SpfMechanism::A { domain, prefix } = mechanism {
            assert_eq!(domain, Some("example.com".to_string()));
            assert_eq!(prefix, Some(24));
        } else {
            panic!("Expected A mechanism with domain and prefix");
        }
    }

    #[test]
    fn test_parse_mechanism_mx() {
        let mechanism = SpfCheckMailet::parse_mechanism("mx").unwrap();
        if let SpfMechanism::MX { domain, prefix } = mechanism {
            assert_eq!(domain, None);
            assert_eq!(prefix, None);
        } else {
            panic!("Expected MX mechanism");
        }
    }

    #[test]
    fn test_parse_mechanism_mx_with_domain() {
        let mechanism = SpfCheckMailet::parse_mechanism("mx:example.com").unwrap();
        if let SpfMechanism::MX { domain, prefix } = mechanism {
            assert_eq!(domain, Some("example.com".to_string()));
            assert_eq!(prefix, None);
        } else {
            panic!("Expected MX mechanism with domain");
        }
    }

    #[test]
    fn test_parse_mechanism_exists() {
        let mechanism = SpfCheckMailet::parse_mechanism("exists:example.com").unwrap();
        if let SpfMechanism::Exists { domain } = mechanism {
            assert_eq!(domain, "example.com");
        } else {
            panic!("Expected Exists mechanism");
        }
    }

    #[test]
    fn test_parse_mechanism_ptr() {
        let mechanism = SpfCheckMailet::parse_mechanism("ptr").unwrap();
        if let SpfMechanism::Ptr { domain } = mechanism {
            assert_eq!(domain, None);
        } else {
            panic!("Expected Ptr mechanism");
        }
    }

    #[test]
    fn test_parse_mechanism_ptr_with_domain() {
        let mechanism = SpfCheckMailet::parse_mechanism("ptr:example.com").unwrap();
        if let SpfMechanism::Ptr { domain } = mechanism {
            assert_eq!(domain, Some("example.com".to_string()));
        } else {
            panic!("Expected Ptr mechanism with domain");
        }
    }

    #[test]
    fn test_parse_mechanism_unknown() {
        let result = SpfCheckMailet::parse_mechanism("unknown:test");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_spf_record_simple() {
        let record = "v=spf1 ip4:192.0.2.0/24 -all";
        let mechanisms = SpfCheckMailet::parse_spf_record(record).unwrap();

        assert_eq!(mechanisms.len(), 2);

        assert!(matches!(mechanisms[0].0, SpfQualifier::Pass));
        assert!(matches!(mechanisms[0].1, SpfMechanism::IP4 { .. }));

        assert!(matches!(mechanisms[1].0, SpfQualifier::Fail));
        assert!(matches!(mechanisms[1].1, SpfMechanism::All));
    }

    #[test]
    fn test_parse_spf_record_complex() {
        let record = "v=spf1 +ip4:192.0.2.0/24 a mx include:example.com ~all";
        let mechanisms = SpfCheckMailet::parse_spf_record(record).unwrap();

        assert_eq!(mechanisms.len(), 5);
        assert!(matches!(mechanisms[0].1, SpfMechanism::IP4 { .. }));
        assert!(matches!(mechanisms[1].1, SpfMechanism::A { .. }));
        assert!(matches!(mechanisms[2].1, SpfMechanism::MX { .. }));
        assert!(matches!(mechanisms[3].1, SpfMechanism::Include { .. }));
        assert!(matches!(mechanisms[4].1, SpfMechanism::All));
        assert!(matches!(mechanisms[4].0, SpfQualifier::SoftFail));
    }

    #[test]
    fn test_parse_spf_record_with_modifiers() {
        let record = "v=spf1 ip4:192.0.2.0/24 redirect=example.com -all";
        let mechanisms = SpfCheckMailet::parse_spf_record(record).unwrap();

        // Should skip redirect= modifier
        assert_eq!(mechanisms.len(), 2);
    }

    #[test]
    fn test_parse_spf_record_with_exp() {
        let record = "v=spf1 ip4:192.0.2.0/24 exp=explain.example.com -all";
        let mechanisms = SpfCheckMailet::parse_spf_record(record).unwrap();

        // Should skip exp= modifier
        assert_eq!(mechanisms.len(), 2);
    }

    #[test]
    fn test_parse_domain_and_prefix_empty() {
        let result = SpfCheckMailet::parse_domain_and_prefix("").unwrap();
        assert_eq!(result, (None, None));
    }

    #[test]
    fn test_parse_domain_and_prefix_domain_only() {
        let result = SpfCheckMailet::parse_domain_and_prefix(":example.com").unwrap();
        assert_eq!(result, (Some("example.com".to_string()), None));
    }

    #[test]
    fn test_parse_domain_and_prefix_prefix_only() {
        let result = SpfCheckMailet::parse_domain_and_prefix("/24").unwrap();
        assert_eq!(result, (None, Some(24)));
    }

    #[test]
    fn test_parse_domain_and_prefix_both() {
        let result = SpfCheckMailet::parse_domain_and_prefix(":example.com/24").unwrap();
        assert_eq!(result, (Some("example.com".to_string()), Some(24)));
    }

    #[test]
    fn test_parse_domain_and_prefix_invalid() {
        let result = SpfCheckMailet::parse_domain_and_prefix("invalid");
        assert!(result.is_err());
    }
}

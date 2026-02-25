//! DMARC (Domain-based Message Authentication) verification mailet
//! RFC 7489 - Domain-based Message Authentication, Reporting, and Conformance

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use hickory_resolver::TokioResolver;
use rusmes_proto::{AttributeValue, Mail};

/// DMARC disposition policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DmarcDisposition {
    None,
    Quarantine,
    Reject,
}

impl DmarcDisposition {
    fn as_str(&self) -> &str {
        match self {
            DmarcDisposition::None => "none",
            DmarcDisposition::Quarantine => "quarantine",
            DmarcDisposition::Reject => "reject",
        }
    }
}

/// DMARC alignment mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DmarcAlignment {
    Relaxed, // Organizational domain match (default)
    Strict,  // Exact domain match
}

/// DMARC verification result
#[derive(Debug, Clone, PartialEq, Eq)]
enum DmarcResult {
    Pass,
    Fail(String),
    #[allow(dead_code)]
    TempError(String),
    PermError(String),
    None,
}

impl DmarcResult {
    #[allow(dead_code)]
    fn as_str(&self) -> &str {
        match self {
            DmarcResult::Pass => "pass",
            DmarcResult::Fail(_) => "fail",
            DmarcResult::TempError(_) => "temperror",
            DmarcResult::PermError(_) => "permerror",
            DmarcResult::None => "none",
        }
    }
}

/// DMARC policy record
#[derive(Debug, Clone)]
struct DmarcPolicy {
    #[allow(dead_code)]
    version: String,
    policy: DmarcDisposition,
    subdomain_policy: Option<DmarcDisposition>,
    alignment_dkim: DmarcAlignment,
    alignment_spf: DmarcAlignment,
    #[allow(dead_code)]
    percentage: u8,
    #[allow(dead_code)]
    report_uri_aggregate: Vec<String>,
    #[allow(dead_code)]
    report_uri_forensic: Vec<String>,
}

impl Default for DmarcPolicy {
    fn default() -> Self {
        Self {
            version: "DMARC1".to_string(),
            policy: DmarcDisposition::None,
            subdomain_policy: None,
            alignment_dkim: DmarcAlignment::Relaxed,
            alignment_spf: DmarcAlignment::Relaxed,
            percentage: 100,
            report_uri_aggregate: Vec::new(),
            report_uri_forensic: Vec::new(),
        }
    }
}

/// DMARC policy enforcement
pub struct DmarcVerifyMailet {
    name: String,
    honor_policy: bool,
    resolver: Option<TokioResolver>,
}

impl DmarcVerifyMailet {
    /// Create a new DMARC verify mailet
    pub fn new() -> Self {
        Self {
            name: "DmarcVerify".to_string(),
            honor_policy: true,
            resolver: None,
        }
    }

    /// Lookup DMARC policy from DNS
    async fn lookup_dmarc_policy(&self, domain: &str) -> Result<DmarcPolicy> {
        let resolver = self
            .resolver
            .as_ref()
            .ok_or_else(|| anyhow!("DNS resolver not initialized"))?;

        // Try _dmarc.domain
        let dmarc_domain = format!("_dmarc.{}", domain);

        let txt_records = match resolver.txt_lookup(&dmarc_domain).await {
            Ok(records) => records,
            Err(e) => {
                tracing::debug!("DMARC DNS lookup failed for {}: {}", dmarc_domain, e);
                return Err(anyhow!("No DMARC record found for {}", domain));
            }
        };

        // Find DMARC record (starts with "v=DMARC1")
        for record in txt_records.iter() {
            // TXT records are stored as Vec<Box<[u8]>>, need to convert to string
            let txt_parts: Vec<String> = record
                .txt_data()
                .iter()
                .filter_map(|bytes| String::from_utf8(bytes.to_vec()).ok())
                .collect();
            let txt = txt_parts.join("");

            if txt.starts_with("v=DMARC1") {
                tracing::debug!("Found DMARC record for {}: {}", domain, txt);
                return Self::parse_dmarc_record(&txt);
            }
        }

        Err(anyhow!("No valid DMARC record found for {}", domain))
    }

    /// Parse DMARC record string
    fn parse_dmarc_record(record: &str) -> Result<DmarcPolicy> {
        let mut policy = DmarcPolicy::default();

        for tag in record.split(';') {
            let tag = tag.trim();
            if let Some((key, value)) = tag.split_once('=') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "v" => {
                        if value != "DMARC1" {
                            return Err(anyhow!("Invalid DMARC version: {}", value));
                        }
                    }
                    "p" => policy.policy = Self::parse_disposition(value)?,
                    "sp" => policy.subdomain_policy = Some(Self::parse_disposition(value)?),
                    "adkim" => policy.alignment_dkim = Self::parse_alignment(value)?,
                    "aspf" => policy.alignment_spf = Self::parse_alignment(value)?,
                    "pct" => {
                        policy.percentage = value
                            .parse()
                            .map_err(|_| anyhow!("Invalid percentage: {}", value))?;
                        if policy.percentage > 100 {
                            return Err(anyhow!(
                                "Percentage must be 0-100, got {}",
                                policy.percentage
                            ));
                        }
                    }
                    "rua" => policy.report_uri_aggregate = Self::parse_uri_list(value),
                    "ruf" => policy.report_uri_forensic = Self::parse_uri_list(value),
                    _ => {
                        // Ignore unknown tags per RFC 7489
                        tracing::debug!("Unknown DMARC tag: {}", key);
                    }
                }
            }
        }

        Ok(policy)
    }

    /// Parse disposition value (none/quarantine/reject)
    fn parse_disposition(value: &str) -> Result<DmarcDisposition> {
        match value.to_lowercase().as_str() {
            "none" => Ok(DmarcDisposition::None),
            "quarantine" => Ok(DmarcDisposition::Quarantine),
            "reject" => Ok(DmarcDisposition::Reject),
            _ => Err(anyhow!("Invalid DMARC disposition: {}", value)),
        }
    }

    /// Parse alignment mode (r/s)
    fn parse_alignment(value: &str) -> Result<DmarcAlignment> {
        match value.to_lowercase().as_str() {
            "r" => Ok(DmarcAlignment::Relaxed),
            "s" => Ok(DmarcAlignment::Strict),
            _ => Err(anyhow!("Invalid DMARC alignment: {}", value)),
        }
    }

    /// Parse comma-separated URI list
    fn parse_uri_list(value: &str) -> Vec<String> {
        value
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Extract organizational domain (e.g., "mail.example.com" -> "example.com")
    fn org_domain(domain: &str) -> &str {
        // Simple heuristic: take last 2 labels
        // In production, should use Public Suffix List (PSL)
        let parts: Vec<&str> = domain.split('.').collect();
        if parts.len() >= 2 {
            let last_two_len = parts[parts.len() - 2].len() + parts[parts.len() - 1].len() + 1;
            &domain[domain.len() - last_two_len..]
        } else {
            domain
        }
    }

    /// Extract From header domain
    fn extract_from_domain(mail: &Mail) -> Result<String> {
        let headers = mail.message().headers();
        let from_header = headers
            .get_first("from")
            .ok_or_else(|| anyhow!("No From header found"))?;

        // Parse email from From header (e.g., "Name <user@domain.com>")
        let from_addr = if let Some(start) = from_header.rfind('<') {
            if let Some(end) = from_header[start..].find('>') {
                &from_header[start + 1..start + end]
            } else {
                from_header
            }
        } else {
            from_header
        };

        // Extract domain part
        if let Some(at_pos) = from_addr.rfind('@') {
            Ok(from_addr[at_pos + 1..].trim().to_lowercase())
        } else {
            Err(anyhow!("Invalid From address: {}", from_addr))
        }
    }

    /// Check DKIM alignment
    fn check_dkim_alignment(
        mail: &Mail,
        from_domain: &str,
        alignment: DmarcAlignment,
    ) -> Result<bool> {
        // Get DKIM domain from attributes (set by DkimVerifyMailet)
        let dkim_domain = match mail.get_attribute("dkim.domain") {
            Some(AttributeValue::String(d)) => d.to_lowercase(),
            _ => return Ok(false),
        };

        match alignment {
            DmarcAlignment::Relaxed => {
                // Organizational domain match
                Ok(Self::org_domain(&dkim_domain) == Self::org_domain(from_domain))
            }
            DmarcAlignment::Strict => {
                // Exact domain match
                Ok(dkim_domain == from_domain)
            }
        }
    }

    /// Check SPF alignment
    fn check_spf_alignment(
        mail: &Mail,
        from_domain: &str,
        alignment: DmarcAlignment,
    ) -> Result<bool> {
        // Get SPF domain (MAIL FROM envelope sender domain)
        let spf_domain = mail
            .sender()
            .ok_or_else(|| anyhow!("No envelope sender"))?
            .domain()
            .as_str()
            .to_lowercase();

        match alignment {
            DmarcAlignment::Relaxed => {
                // Organizational domain match
                Ok(Self::org_domain(&spf_domain) == Self::org_domain(from_domain))
            }
            DmarcAlignment::Strict => {
                // Exact domain match
                Ok(spf_domain == from_domain)
            }
        }
    }

    /// Check if DKIM passed
    fn dkim_passed(mail: &Mail) -> bool {
        match mail.get_attribute("dkim.result") {
            Some(AttributeValue::String(s)) => s == "pass",
            Some(AttributeValue::Boolean(b)) => *b,
            _ => {
                // Also check dkim.verified for backwards compatibility
                match mail.get_attribute("dkim.verified") {
                    Some(AttributeValue::Boolean(b)) => *b,
                    _ => false,
                }
            }
        }
    }

    /// Check if SPF passed
    fn spf_passed(mail: &Mail) -> bool {
        match mail.get_attribute("spf.result") {
            Some(AttributeValue::String(s)) => s == "pass",
            _ => false,
        }
    }

    /// Verify DMARC policy
    async fn verify_dmarc(&self, mail: &Mail) -> Result<DmarcResult> {
        // 1. Extract From header domain
        let from_domain = match Self::extract_from_domain(mail) {
            Ok(domain) => domain,
            Err(e) => {
                tracing::warn!("Failed to extract From domain: {}", e);
                return Ok(DmarcResult::PermError(e.to_string()));
            }
        };

        // 2. Lookup DMARC policy
        let policy = match self.lookup_dmarc_policy(&from_domain).await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("DMARC policy lookup failed for {}: {}", from_domain, e);
                // No DMARC policy = None result (not an error)
                return Ok(DmarcResult::None);
            }
        };

        // 3. Check DKIM alignment
        let dkim_aligned =
            Self::check_dkim_alignment(mail, &from_domain, policy.alignment_dkim).unwrap_or(false);
        let dkim_pass = Self::dkim_passed(mail);

        // 4. Check SPF alignment
        let spf_aligned =
            Self::check_spf_alignment(mail, &from_domain, policy.alignment_spf).unwrap_or(false);
        let spf_pass = Self::spf_passed(mail);

        tracing::debug!(
            "DMARC alignment check for {}: DKIM aligned={} pass={}, SPF aligned={} pass={}",
            from_domain,
            dkim_aligned,
            dkim_pass,
            spf_aligned,
            spf_pass
        );

        // 5. Determine result (pass if either DKIM or SPF is aligned AND passed)
        let dmarc_pass = (dkim_aligned && dkim_pass) || (spf_aligned && spf_pass);

        if dmarc_pass {
            return Ok(DmarcResult::Pass);
        }

        // 6. Failed - return disposition
        Ok(DmarcResult::Fail(policy.policy.as_str().to_string()))
    }
}

impl Default for DmarcVerifyMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for DmarcVerifyMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        if let Some(honor_str) = config.get_param("honor_policy") {
            self.honor_policy = honor_str.parse()?;
        }

        // Initialize DNS resolver
        self.resolver = Some(
            TokioResolver::builder_tokio()
                .map_err(|e| anyhow!("Failed to initialize DNS resolver: {}", e))?
                .build(),
        );

        tracing::info!(
            "Initialized DmarcVerifyMailet (honor policy: {})",
            self.honor_policy
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        tracing::debug!("Checking DMARC policy for mail {}", mail.id());

        let result = self.verify_dmarc(mail).await?;

        match &result {
            DmarcResult::Pass => {
                mail.set_attribute("dmarc.result", "pass");
                tracing::info!("DMARC verification passed for mail {}", mail.id());
            }
            DmarcResult::Fail(policy) => {
                mail.set_attribute("dmarc.result", "fail");
                mail.set_attribute("dmarc.policy", policy.as_str());
                tracing::warn!(
                    "DMARC verification failed for mail {}: policy={}",
                    mail.id(),
                    policy
                );

                // Enforce policy if enabled
                if self.honor_policy {
                    match policy.as_str() {
                        "reject" => {
                            tracing::warn!("DMARC policy=reject, dropping mail {}", mail.id());
                            return Ok(MailetAction::Drop);
                        }
                        "quarantine" => {
                            tracing::warn!("DMARC policy=quarantine for mail {}", mail.id());
                            mail.set_attribute("mail.quarantine", true);
                        }
                        _ => {}
                    }
                }
            }
            DmarcResult::None => {
                mail.set_attribute("dmarc.result", "none");
                tracing::debug!("No DMARC policy found for mail {}", mail.id());
            }
            DmarcResult::TempError(msg) => {
                mail.set_attribute("dmarc.result", "temperror");
                tracing::warn!("DMARC temporary error for mail {}: {}", mail.id(), msg);
            }
            DmarcResult::PermError(msg) => {
                mail.set_attribute("dmarc.result", "permerror");
                tracing::warn!("DMARC permanent error for mail {}: {}", mail.id(), msg);
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
    use std::str::FromStr;

    #[allow(dead_code)]
    fn create_test_mail(sender: &str, recipients: Vec<&str>) -> Mail {
        let sender_addr = MailAddress::from_str(sender).ok();
        let recipient_addrs: Vec<MailAddress> = recipients
            .iter()
            .filter_map(|r| MailAddress::from_str(r).ok())
            .collect();

        let message = MimeMessage::new(
            HeaderMap::new(),
            MessageBody::Small(Bytes::from("Test message")),
        );

        Mail::new(sender_addr, recipient_addrs, message, None, None)
    }

    #[tokio::test]
    async fn test_dmarc_verify_mailet_creation() {
        let mailet = DmarcVerifyMailet::new();
        assert_eq!(mailet.name(), "DmarcVerify");
        assert!(mailet.honor_policy); // honor_policy is true by default
    }

    #[tokio::test]
    async fn test_dmarc_verify_mailet_default() {
        let mailet = DmarcVerifyMailet::default();
        assert_eq!(mailet.name(), "DmarcVerify");
    }

    #[tokio::test]
    async fn test_dmarc_verify_init_with_config() {
        let mut mailet = DmarcVerifyMailet::new();
        let mut config = MailetConfig::new("DmarcVerify");
        config = config.with_param("honor_policy".to_string(), "true".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_ok());
        assert!(mailet.honor_policy);
    }

    #[tokio::test]
    async fn test_dmarc_verify_init_creates_resolver() {
        let mut mailet = DmarcVerifyMailet::new();
        let config = MailetConfig::new("DmarcVerify");

        assert!(mailet.resolver.is_none());
        let result = mailet.init(config).await;
        assert!(result.is_ok());
        assert!(mailet.resolver.is_some());
    }

    #[tokio::test]
    async fn test_dmarc_disposition_as_str() {
        assert_eq!(DmarcDisposition::None.as_str(), "none");
        assert_eq!(DmarcDisposition::Quarantine.as_str(), "quarantine");
        assert_eq!(DmarcDisposition::Reject.as_str(), "reject");
    }

    #[tokio::test]
    async fn test_dmarc_result_as_str() {
        assert_eq!(DmarcResult::Pass.as_str(), "pass");
        assert_eq!(DmarcResult::Fail("reason".to_string()).as_str(), "fail");
        assert_eq!(
            DmarcResult::TempError("reason".to_string()).as_str(),
            "temperror"
        );
        assert_eq!(
            DmarcResult::PermError("reason".to_string()).as_str(),
            "permerror"
        );
        assert_eq!(DmarcResult::None.as_str(), "none");
    }

    #[tokio::test]
    async fn test_dmarc_policy_default() {
        let policy = DmarcPolicy::default();
        assert_eq!(policy.version, "DMARC1");
        assert_eq!(policy.policy, DmarcDisposition::None);
        assert_eq!(policy.subdomain_policy, None);
        assert_eq!(policy.alignment_dkim, DmarcAlignment::Relaxed);
        assert_eq!(policy.alignment_spf, DmarcAlignment::Relaxed);
        assert_eq!(policy.percentage, 100);
        assert!(policy.report_uri_aggregate.is_empty());
        assert!(policy.report_uri_forensic.is_empty());
    }
}

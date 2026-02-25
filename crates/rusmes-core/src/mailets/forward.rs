//! Forward mailet - forwards messages to external addresses

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};
use std::collections::HashMap;
use std::str::FromStr;

/// Maximum forwarding hops to prevent loops
const MAX_FORWARDING_HOPS: u32 = 10;

/// Forwards messages to configured addresses
pub struct ForwardMailet {
    name: String,
    /// Forward rules: recipient -> list of forward addresses
    forward_rules: HashMap<String, Vec<MailAddress>>,
    /// Whether to preserve original headers
    preserve_headers: bool,
    /// Maximum forwarding hops
    max_hops: u32,
    /// Server hostname for X-Forwarded-By header
    hostname: String,
}

impl ForwardMailet {
    /// Create a new forward mailet
    pub fn new() -> Self {
        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "localhost".to_string());

        Self {
            name: "Forward".to_string(),
            forward_rules: HashMap::new(),
            preserve_headers: true,
            max_hops: MAX_FORWARDING_HOPS,
            hostname,
        }
    }

    /// Create a new forward mailet with custom hostname
    pub fn with_hostname(hostname: String) -> Self {
        Self {
            name: "Forward".to_string(),
            forward_rules: HashMap::new(),
            preserve_headers: true,
            max_hops: MAX_FORWARDING_HOPS,
            hostname,
        }
    }

    /// Add a forwarding rule
    pub fn add_rule(&mut self, recipient: String, forward_to: Vec<MailAddress>) {
        self.forward_rules.insert(recipient, forward_to);
    }

    /// Check if forwarding would create a loop
    fn check_forwarding_loop(&self, mail: &Mail, forward_to: &MailAddress) -> bool {
        // Check X-Forwarded-To header for loops
        if let Some(forwarded_to) = mail.get_attribute("header.X-Forwarded-To") {
            if let Some(addrs) = forwarded_to.as_str() {
                for addr in addrs.split(',') {
                    let trimmed = addr.trim();
                    if trimmed == forward_to.to_string() {
                        return true;
                    }
                }
            }
        }

        // Check if we're forwarding back to the original sender
        if let Some(original_from) = mail.get_attribute("header.X-Forwarded-From") {
            if let Some(from_addr) = original_from.as_str() {
                if from_addr == forward_to.to_string() {
                    return true;
                }
            }
        }

        // Check if we're forwarding to one of the original recipients
        for recipient in mail.recipients() {
            if recipient.to_string() == forward_to.to_string() {
                // This is allowed - we're adding the forward address, not creating a loop
                // The loop check is for addresses already in the forwarding chain
                continue;
            }
        }

        false
    }

    /// Detect circular forwarding patterns
    fn detect_circular_pattern(&self, mail: &Mail, forward_to: &MailAddress) -> bool {
        // Build forwarding chain from headers
        let mut chain = Vec::new();

        if let Some(forwarded_to) = mail.get_attribute("header.X-Forwarded-To") {
            if let Some(addrs) = forwarded_to.as_str() {
                for addr in addrs.split(',') {
                    chain.push(addr.trim().to_string());
                }
            }
        }

        // Check if forward_to appears multiple times in the chain
        let forward_to_str = forward_to.to_string();
        let count = chain.iter().filter(|a| *a == &forward_to_str).count();

        // If we see the same address twice, it's a circular pattern
        count > 0
    }

    /// Get current forwarding hop count
    fn get_hop_count(&self, mail: &Mail) -> u32 {
        if let Some(hops) = mail.get_attribute("header.X-Forwarded-Count") {
            if let Some(count_str) = hops.as_str() {
                return count_str.parse().unwrap_or(0);
            }
        }
        0
    }

    /// Add forwarding headers to mail
    fn add_forwarding_headers(&self, mail: &mut Mail, forward_to: &[MailAddress]) {
        let hop_count = self.get_hop_count(mail) + 1;

        // Update hop count
        mail.set_attribute("header.X-Forwarded-Count", hop_count.to_string());

        // Add X-Forwarded-From with original sender (only on first forward)
        if mail.get_attribute("header.X-Forwarded-From").is_none() {
            if let Some(sender) = mail.sender() {
                mail.set_attribute("header.X-Forwarded-From", sender.to_string());
            }
        }

        // Add X-Forwarded-Date with original timestamp (only on first forward)
        if mail.get_attribute("header.X-Forwarded-Date").is_none() {
            // Get current time in RFC 2822 format
            let now = chrono::Utc::now();
            mail.set_attribute("header.X-Forwarded-Date", now.to_rfc2822());
        }

        // Add X-Forwarded-By with server hostname
        let mut forwarded_by = Vec::new();
        if let Some(existing) = mail.get_attribute("header.X-Forwarded-By") {
            if let Some(hosts) = existing.as_str() {
                forwarded_by.push(hosts.to_string());
            }
        }
        forwarded_by.push(self.hostname.clone());
        mail.set_attribute("header.X-Forwarded-By", forwarded_by.join(", "));

        // Add forwarded-to header
        let mut forwarded_to = Vec::new();
        if let Some(existing) = mail.get_attribute("header.X-Forwarded-To") {
            if let Some(addrs) = existing.as_str() {
                forwarded_to.push(addrs.to_string());
            }
        }
        for addr in forward_to {
            forwarded_to.push(addr.to_string());
        }
        mail.set_attribute("header.X-Forwarded-To", forwarded_to.join(", "));

        // Add X-Forwarded-For with original sender (for compatibility)
        if mail.get_attribute("header.X-Forwarded-For").is_none() {
            if let Some(sender) = mail.sender() {
                mail.set_attribute("header.X-Forwarded-For", sender.to_string());
            }
        }
    }
}

impl Default for ForwardMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for ForwardMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        // Parse forward rules from config
        // Format: "recipient1=forward1,forward2;recipient2=forward3"
        if let Some(rules_str) = config.get_param("rules") {
            for rule in rules_str.split(';') {
                let parts: Vec<&str> = rule.split('=').collect();
                if parts.len() == 2 {
                    let recipient = parts[0].trim().to_string();
                    let forwards: Result<Vec<MailAddress>, _> = parts[1]
                        .split(',')
                        .map(|s| MailAddress::from_str(s.trim()))
                        .collect();

                    match forwards {
                        Ok(addrs) => {
                            self.forward_rules.insert(recipient, addrs);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to parse forward addresses in rule '{}': {}",
                                rule,
                                e
                            );
                        }
                    }
                }
            }
        }

        if let Some(preserve) = config.get_param("preserve_headers") {
            self.preserve_headers = preserve.parse().unwrap_or(true);
        }

        if let Some(max_hops_str) = config.get_param("max_hops") {
            self.max_hops = max_hops_str.parse().unwrap_or(MAX_FORWARDING_HOPS);
        }

        if let Some(hostname) = config.get_param("hostname") {
            self.hostname = hostname.to_string();
        }

        tracing::info!(
            "Initialized ForwardMailet with {} rules, hostname: {}",
            self.forward_rules.len(),
            self.hostname
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        let hop_count = self.get_hop_count(mail);

        // Check max hops
        if hop_count >= self.max_hops {
            tracing::warn!(
                "Mail {} exceeded max forwarding hops ({})",
                mail.id(),
                self.max_hops
            );
            return Ok(MailetAction::Continue);
        }

        let mut forwards_to_add = Vec::new();

        // Check each recipient against forward rules
        for recipient in mail.recipients() {
            let recipient_str = recipient.to_string();

            // Check exact match first
            if let Some(forward_addresses) = self.forward_rules.get(&recipient_str) {
                for forward_addr in forward_addresses {
                    // Check for forwarding loops and circular patterns
                    if !self.check_forwarding_loop(mail, forward_addr)
                        && !self.detect_circular_pattern(mail, forward_addr)
                    {
                        forwards_to_add.push(forward_addr.clone());
                    } else {
                        tracing::warn!(
                            "Detected forwarding loop or circular pattern for {} -> {}",
                            recipient_str,
                            forward_addr
                        );
                    }
                }
            }

            // Check domain wildcard (e.g., "*@example.com")
            let domain = recipient.domain();
            let wildcard_key = format!("*@{}", domain);
            if let Some(forward_addresses) = self.forward_rules.get(&wildcard_key) {
                for forward_addr in forward_addresses {
                    if !self.check_forwarding_loop(mail, forward_addr)
                        && !self.detect_circular_pattern(mail, forward_addr)
                    {
                        forwards_to_add.push(forward_addr.clone());
                    }
                }
            }
        }

        if !forwards_to_add.is_empty() {
            tracing::info!(
                "Forwarding mail {} to {} addresses",
                mail.id(),
                forwards_to_add.len()
            );

            // Add forwarding headers
            self.add_forwarding_headers(mail, &forwards_to_add);

            // Add forward addresses to recipients
            let mut recipients = mail.recipients().to_vec();
            recipients.extend(forwards_to_add);
            mail.set_recipients(recipients);
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
    use rusmes_proto::{HeaderMap, MailState, MessageBody, MimeMessage};

    #[tokio::test]
    async fn test_forward_mailet_init() {
        let mut mailet = ForwardMailet::new();
        let config = MailetConfig::new("Forward").with_param(
            "rules",
            "user1@example.com=forward1@test.com,forward2@test.com",
        );

        mailet.init(config).await.unwrap();

        assert_eq!(mailet.forward_rules.len(), 1);
        assert!(mailet.forward_rules.contains_key("user1@example.com"));
    }

    #[tokio::test]
    async fn test_forward_simple() {
        let mut mailet = ForwardMailet::new();
        let forward_addr = MailAddress::from_str("forward@test.com").unwrap();
        mailet.add_rule("user@example.com".to_string(), vec![forward_addr.clone()]);

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.state = MailState::Root;

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);

        // Should have original + forwarded recipient
        assert_eq!(mail.recipients().len(), 2);
        assert!(mail.recipients().contains(&forward_addr));
    }

    #[tokio::test]
    async fn test_forward_multiple_addresses() {
        let mut mailet = ForwardMailet::new();
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![
                MailAddress::from_str("forward1@test.com").unwrap(),
                MailAddress::from_str("forward2@test.com").unwrap(),
            ],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        // Original + 2 forwards
        assert_eq!(mail.recipients().len(), 3);
    }

    #[tokio::test]
    async fn test_forward_domain_wildcard() {
        let mut mailet = ForwardMailet::new();
        let forward_addr = MailAddress::from_str("catch-all@test.com").unwrap();
        mailet.add_rule("*@example.com".to_string(), vec![forward_addr.clone()]);

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("anyone@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(mail.recipients().len(), 2);
        assert!(mail.recipients().contains(&forward_addr));
    }

    #[tokio::test]
    async fn test_forward_headers_added() {
        let mut mailet = ForwardMailet::new();
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        // Check forwarding headers
        assert!(mail.get_attribute("header.X-Forwarded-Count").is_some());
        assert!(mail.get_attribute("header.X-Forwarded-To").is_some());
        assert!(mail.get_attribute("header.X-Forwarded-For").is_some());
    }

    #[tokio::test]
    async fn test_forward_hop_count_increments() {
        let mut mailet = ForwardMailet::new();
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        // First forward
        mailet.service(&mut mail).await.unwrap();
        assert_eq!(mailet.get_hop_count(&mail), 1);

        // Second forward will not happen due to loop detection
        // (forward@test.com is already in X-Forwarded-To header)
        mailet.service(&mut mail).await.unwrap();
        // Hop count should remain 1 as no new forward was added
        assert_eq!(mailet.get_hop_count(&mail), 1);
    }

    #[tokio::test]
    async fn test_forward_max_hops_prevention() {
        let mut mailet = ForwardMailet::new();
        mailet.max_hops = 3;
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        // Set hop count to max
        mail.set_attribute("header.X-Forwarded-Count", "3");

        let original_count = mail.recipients().len();
        mailet.service(&mut mail).await.unwrap();

        // Should not add new recipients when max hops reached
        assert_eq!(mail.recipients().len(), original_count);
    }

    #[tokio::test]
    async fn test_forward_loop_detection() {
        let mut mailet = ForwardMailet::new();
        let forward_addr = MailAddress::from_str("forward@test.com").unwrap();
        mailet.add_rule("user@example.com".to_string(), vec![forward_addr.clone()]);

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        // Simulate already forwarded to this address
        mail.set_attribute("header.X-Forwarded-To", forward_addr.to_string());

        let original_count = mail.recipients().len();
        mailet.service(&mut mail).await.unwrap();

        // Should not add recipient due to loop detection
        assert_eq!(mail.recipients().len(), original_count);
    }

    #[tokio::test]
    async fn test_forward_no_matching_rules() {
        let mut mailet = ForwardMailet::new();
        mailet.add_rule(
            "other@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let original_count = mail.recipients().len();
        mailet.service(&mut mail).await.unwrap();

        // No rules matched, recipients unchanged
        assert_eq!(mail.recipients().len(), original_count);
    }

    #[tokio::test]
    async fn test_forward_preserve_headers() {
        let mut mailet = ForwardMailet::new();
        mailet.preserve_headers = true;
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Subject", "Test Subject");

        mailet.service(&mut mail).await.unwrap();

        // Original headers preserved
        assert_eq!(
            mail.get_attribute("header.Subject")
                .and_then(|v| v.as_str()),
            Some("Test Subject")
        );
    }

    #[tokio::test]
    async fn test_forward_multiple_recipients() {
        let mut mailet = ForwardMailet::new();
        mailet.add_rule(
            "user1@example.com".to_string(),
            vec![MailAddress::from_str("forward1@test.com").unwrap()],
        );
        mailet.add_rule(
            "user2@example.com".to_string(),
            vec![MailAddress::from_str("forward2@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![
                MailAddress::from_str("user1@example.com").unwrap(),
                MailAddress::from_str("user2@example.com").unwrap(),
            ],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        // 2 original + 2 forwards
        assert_eq!(mail.recipients().len(), 4);
    }

    #[tokio::test]
    async fn test_forward_config_parsing() {
        let mut mailet = ForwardMailet::new();
        let config = MailetConfig::new("Forward")
            .with_param(
                "rules",
                "user1@example.com=forward1@test.com;user2@example.com=forward2@test.com,forward3@test.com"
            );

        mailet.init(config).await.unwrap();

        assert_eq!(mailet.forward_rules.len(), 2);
        assert_eq!(
            mailet.forward_rules.get("user1@example.com").unwrap().len(),
            1
        );
        assert_eq!(
            mailet.forward_rules.get("user2@example.com").unwrap().len(),
            2
        );
    }

    #[tokio::test]
    async fn test_forward_config_max_hops() {
        let mut mailet = ForwardMailet::new();
        let config = MailetConfig::new("Forward").with_param("max_hops", "5");

        mailet.init(config).await.unwrap();

        assert_eq!(mailet.max_hops, 5);
    }

    #[tokio::test]
    async fn test_forward_config_preserve_headers() {
        let mut mailet = ForwardMailet::new();
        let config = MailetConfig::new("Forward").with_param("preserve_headers", "false");

        mailet.init(config).await.unwrap();

        assert!(!mailet.preserve_headers);
    }

    #[tokio::test]
    async fn test_forward_xforwarded_for_header() {
        let mut mailet = ForwardMailet::new();
        let sender = MailAddress::from_str("sender@test.com").unwrap();
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(sender.clone()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        let forwarded_for = mail
            .get_attribute("header.X-Forwarded-For")
            .and_then(|v| v.as_str());
        assert_eq!(forwarded_for, Some(sender.to_string().as_str()));
    }

    #[tokio::test]
    async fn test_forward_empty_rules() {
        let mailet = ForwardMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let original_count = mail.recipients().len();
        mailet.service(&mut mail).await.unwrap();

        assert_eq!(mail.recipients().len(), original_count);
    }

    #[tokio::test]
    async fn test_forward_invalid_address_in_config() {
        let mut mailet = ForwardMailet::new();
        let config =
            MailetConfig::new("Forward").with_param("rules", "user@example.com=invalid-email");

        // Should not panic, just skip invalid addresses
        mailet.init(config).await.unwrap();
        assert_eq!(mailet.forward_rules.len(), 0);
    }

    #[tokio::test]
    async fn test_forward_case_sensitive_matching() {
        let mut mailet = ForwardMailet::new();
        mailet.add_rule(
            "User@Example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let original_count = mail.recipients().len();
        mailet.service(&mut mail).await.unwrap();

        // Case-sensitive, should not match
        assert_eq!(mail.recipients().len(), original_count);
    }

    #[tokio::test]
    async fn test_forward_duplicate_detection() {
        let mut mailet = ForwardMailet::new();
        let forward_addr = MailAddress::from_str("forward@test.com").unwrap();
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![forward_addr.clone(), forward_addr.clone()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        // Both duplicates added (deduplication happens at delivery time)
        assert_eq!(mail.recipients().len(), 3);
    }

    #[tokio::test]
    async fn test_forward_mailet_name() {
        let mailet = ForwardMailet::new();
        assert_eq!(mailet.name(), "Forward");
    }

    #[tokio::test]
    async fn test_forward_xforwarded_from_header() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        let sender = MailAddress::from_str("sender@test.com").unwrap();
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(sender.clone()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        let forwarded_from = mail
            .get_attribute("header.X-Forwarded-From")
            .and_then(|v| v.as_str());
        assert_eq!(forwarded_from, Some(sender.to_string().as_str()));
    }

    #[tokio::test]
    async fn test_forward_xforwarded_date_header() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        let forwarded_date = mail
            .get_attribute("header.X-Forwarded-Date")
            .and_then(|v| v.as_str());
        assert!(forwarded_date.is_some());
        assert!(!forwarded_date.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_forward_xforwarded_by_header() {
        let hostname = "testhost.example.com".to_string();
        let mut mailet = ForwardMailet::with_hostname(hostname.clone());
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        let forwarded_by = mail
            .get_attribute("header.X-Forwarded-By")
            .and_then(|v| v.as_str());
        assert_eq!(forwarded_by, Some(hostname.as_str()));
    }

    #[tokio::test]
    async fn test_forward_xforwarded_by_multiple_hops() {
        let hostname = "testhost.example.com".to_string();
        let mut mailet = ForwardMailet::with_hostname(hostname.clone());
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        // First hop
        mailet.service(&mut mail).await.unwrap();

        // Second hop will not happen due to loop detection
        mailet.service(&mut mail).await.unwrap();

        let forwarded_by = mail
            .get_attribute("header.X-Forwarded-By")
            .and_then(|v| v.as_str());
        assert!(forwarded_by.is_some());
        // Should contain hostname once (second forward prevented by loop detection)
        let by_list: Vec<&str> = forwarded_by.unwrap().split(',').map(|s| s.trim()).collect();
        assert_eq!(by_list.len(), 1);
        assert_eq!(by_list[0], hostname.as_str());
    }

    #[tokio::test]
    async fn test_forward_circular_pattern_detection() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        let forward_addr = MailAddress::from_str("forward@test.com").unwrap();
        mailet.add_rule("user@example.com".to_string(), vec![forward_addr.clone()]);

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        // First forward
        mailet.service(&mut mail).await.unwrap();
        let count_after_first = mail.recipients().len();

        // Try to forward again - should detect circular pattern
        mailet.service(&mut mail).await.unwrap();
        let count_after_second = mail.recipients().len();

        // Should not add more recipients due to circular pattern
        assert_eq!(count_after_second, count_after_first);
    }

    #[tokio::test]
    async fn test_forward_config_hostname() {
        let mut mailet = ForwardMailet::new();
        let config = MailetConfig::new("Forward").with_param("hostname", "custom.example.com");

        mailet.init(config).await.unwrap();

        assert_eq!(mailet.hostname, "custom.example.com");
    }

    #[tokio::test]
    async fn test_forward_headers_preserve_original() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        // Set original headers
        mail.set_attribute("header.From", "sender@test.com");
        mail.set_attribute("header.To", "user@example.com");
        mail.set_attribute("header.Subject", "Original Subject");
        mail.set_attribute("header.Date", "Mon, 1 Jan 2024 12:00:00 +0000");
        mail.set_attribute("header.Message-ID", "<original@test.com>");

        mailet.service(&mut mail).await.unwrap();

        // Original headers should be preserved
        assert_eq!(
            mail.get_attribute("header.From").and_then(|v| v.as_str()),
            Some("sender@test.com")
        );
        assert_eq!(
            mail.get_attribute("header.To").and_then(|v| v.as_str()),
            Some("user@example.com")
        );
        assert_eq!(
            mail.get_attribute("header.Subject")
                .and_then(|v| v.as_str()),
            Some("Original Subject")
        );
        assert_eq!(
            mail.get_attribute("header.Date").and_then(|v| v.as_str()),
            Some("Mon, 1 Jan 2024 12:00:00 +0000")
        );
        assert_eq!(
            mail.get_attribute("header.Message-ID")
                .and_then(|v| v.as_str()),
            Some("<original@test.com>")
        );
    }

    #[tokio::test]
    async fn test_forward_default_constructor() {
        let mailet = ForwardMailet::default();
        assert_eq!(mailet.name(), "Forward");
        assert!(mailet.preserve_headers);
        assert_eq!(mailet.max_hops, MAX_FORWARDING_HOPS);
    }

    #[tokio::test]
    async fn test_forward_with_hostname_constructor() {
        let hostname = "custom.example.com".to_string();
        let mailet = ForwardMailet::with_hostname(hostname.clone());
        assert_eq!(mailet.hostname, hostname);
    }

    #[tokio::test]
    async fn test_forward_zero_max_hops() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        mailet.max_hops = 0;
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let original_count = mail.recipients().len();
        mailet.service(&mut mail).await.unwrap();

        // Should not forward with max_hops = 0
        assert_eq!(mail.recipients().len(), original_count);
    }

    #[tokio::test]
    async fn test_forward_complex_email_addresses() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        mailet.add_rule(
            "user+tag@example.com".to_string(),
            vec![MailAddress::from_str("forward+tag@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user+tag@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        assert_eq!(mail.recipients().len(), 2);
    }

    #[tokio::test]
    async fn test_forward_xforwarded_from_only_once() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        let sender = MailAddress::from_str("sender@test.com").unwrap();
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(sender.clone()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        // Forward twice
        mailet.service(&mut mail).await.unwrap();
        mailet.service(&mut mail).await.unwrap();

        let forwarded_from = mail
            .get_attribute("header.X-Forwarded-From")
            .and_then(|v| v.as_str());

        // Should only have original sender, not duplicated
        assert_eq!(forwarded_from, Some(sender.to_string().as_str()));
    }

    #[tokio::test]
    async fn test_forward_xforwarded_date_only_once() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        // Forward twice
        mailet.service(&mut mail).await.unwrap();
        let first_date = mail
            .get_attribute("header.X-Forwarded-Date")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        mailet.service(&mut mail).await.unwrap();
        let second_date = mail
            .get_attribute("header.X-Forwarded-Date")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Should be the same date (not updated on second forward)
        assert_eq!(first_date, second_date);
    }

    #[tokio::test]
    async fn test_forward_back_to_original_sender() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        let sender = MailAddress::from_str("sender@test.com").unwrap();

        // Rule that would forward back to sender
        mailet.add_rule("user@example.com".to_string(), vec![sender.clone()]);

        let mut mail = Mail::new(
            Some(sender.clone()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        // First forward adds X-Forwarded-From
        mailet.service(&mut mail).await.unwrap();

        let original_count = mail.recipients().len();

        // Try to forward again - should detect loop back to original sender
        mailet.service(&mut mail).await.unwrap();

        // Should not add recipient due to loop detection
        assert_eq!(mail.recipients().len(), original_count);
    }

    #[tokio::test]
    async fn test_forward_mixed_rules_exact_and_wildcard() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        mailet.add_rule(
            "specific@example.com".to_string(),
            vec![MailAddress::from_str("specific-forward@test.com").unwrap()],
        );
        mailet.add_rule(
            "*@example.com".to_string(),
            vec![MailAddress::from_str("wildcard-forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("specific@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        // Should match both exact and wildcard rules
        assert_eq!(mail.recipients().len(), 3); // original + 2 forwards
    }

    #[tokio::test]
    async fn test_forward_all_headers_present() {
        let mut mailet = ForwardMailet::with_hostname("testhost.example.com".to_string());
        mailet.add_rule(
            "user@example.com".to_string(),
            vec![MailAddress::from_str("forward@test.com").unwrap()],
        );

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail).await.unwrap();

        // Verify all required X-Forwarded-* headers are present
        assert!(mail.get_attribute("header.X-Forwarded-From").is_some());
        assert!(mail.get_attribute("header.X-Forwarded-To").is_some());
        assert!(mail.get_attribute("header.X-Forwarded-Date").is_some());
        assert!(mail.get_attribute("header.X-Forwarded-By").is_some());
        assert!(mail.get_attribute("header.X-Forwarded-Count").is_some());
        assert!(mail.get_attribute("header.X-Forwarded-For").is_some());
    }
}

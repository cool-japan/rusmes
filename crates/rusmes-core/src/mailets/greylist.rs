//! Greylisting mailet - temporarily reject first delivery attempts

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use rusmes_proto::Mail;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Greylisting tuple (sender IP, mail from, rcpt to)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GreylistTuple {
    sender_ip: String,
    mail_from: String,
    rcpt_to: String,
}

impl GreylistTuple {
    fn new(sender_ip: String, mail_from: String, rcpt_to: String) -> Self {
        Self {
            sender_ip,
            mail_from,
            rcpt_to,
        }
    }
}

/// Greylist entry
#[derive(Debug, Clone)]
struct GreylistEntry {
    /// First attempt timestamp
    first_seen: u64,
    /// Last attempt timestamp
    last_seen: u64,
    /// Number of attempts
    attempts: u32,
    /// Whether passed greylisting
    passed: bool,
}

impl GreylistEntry {
    fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            first_seen: now,
            last_seen: now,
            attempts: 1,
            passed: false,
        }
    }

    fn update(&mut self) {
        self.last_seen = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.attempts += 1;
    }
}

/// Greylisting storage
#[derive(Clone)]
struct GreylistStore {
    entries: Arc<Mutex<HashMap<GreylistTuple, GreylistEntry>>>,
}

impl GreylistStore {
    fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn check_and_update(
        &self,
        tuple: GreylistTuple,
        greylist_period: u64,
    ) -> (bool, GreylistEntry) {
        let mut entries = match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let is_new = !entries.contains_key(&tuple);
        let entry = entries.entry(tuple).or_insert_with(GreylistEntry::new);

        // Only update if not a new entry
        if !is_new {
            entry.update();
        } else {
            // For new entries, just update last_seen to current time
            entry.last_seen = now;
        }

        // Check if greylist period has passed
        let time_since_first = now.saturating_sub(entry.first_seen);
        let should_pass = time_since_first >= greylist_period;

        if should_pass {
            entry.passed = true;
        }

        (entry.passed, entry.clone())
    }

    fn is_whitelisted(&self, tuple: &GreylistTuple, whitelist: &HashSet<String>) -> bool {
        // Check if IP is whitelisted
        if whitelist.contains(&tuple.sender_ip) {
            return true;
        }

        // Check if sender is whitelisted
        if whitelist.contains(&tuple.mail_from) {
            return true;
        }

        // Check if domain is whitelisted
        if let Some(domain) = tuple.mail_from.split('@').nth(1) {
            if whitelist.contains(domain) {
                return true;
            }
        }

        false
    }

    fn cleanup_old_entries(&self, max_age: u64) {
        let mut entries = match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        entries.retain(|_, entry| now.saturating_sub(entry.last_seen) < max_age);
    }
}

/// Greylisting mailet
pub struct GreylistMailet {
    name: String,
    /// Greylist storage
    store: GreylistStore,
    /// Greylist period in seconds (default: 5 minutes)
    greylist_period: u64,
    /// Maximum age for entries (default: 24 hours)
    max_entry_age: u64,
    /// Whitelist (IPs, addresses, domains)
    whitelist: HashSet<String>,
    /// Whether greylisting is enabled
    enabled: bool,
}

impl GreylistMailet {
    /// Create a new greylist mailet
    pub fn new() -> Self {
        Self {
            name: "Greylist".to_string(),
            store: GreylistStore::new(),
            greylist_period: 300, // 5 minutes
            max_entry_age: 86400, // 24 hours
            whitelist: HashSet::new(),
            enabled: true,
        }
    }

    /// Add to whitelist
    pub fn add_to_whitelist(&mut self, entry: String) {
        self.whitelist.insert(entry);
    }

    /// Extract sender IP from mail
    fn extract_sender_ip(&self, mail: &Mail) -> Option<String> {
        mail.get_attribute("smtp.client_ip")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// Create greylist tuple from mail
    fn create_tuple(&self, mail: &Mail) -> Option<GreylistTuple> {
        let sender_ip = self.extract_sender_ip(mail)?;

        let mail_from = mail.sender()?.to_string();

        // For simplicity, use first recipient
        let rcpt_to = mail.recipients().first()?.to_string();

        Some(GreylistTuple::new(sender_ip, mail_from, rcpt_to))
    }
}

impl Default for GreylistMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for GreylistMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        if let Some(period) = config.get_param("greylist_period") {
            self.greylist_period = period.parse().unwrap_or(300);
        }

        if let Some(max_age) = config.get_param("max_entry_age") {
            self.max_entry_age = max_age.parse().unwrap_or(86400);
        }

        if let Some(enabled) = config.get_param("enabled") {
            self.enabled = enabled.parse().unwrap_or(true);
        }

        if let Some(whitelist_str) = config.get_param("whitelist") {
            for entry in whitelist_str.split(',') {
                self.whitelist.insert(entry.trim().to_string());
            }
        }

        tracing::info!(
            "Initialized GreylistMailet (period: {}s, max_age: {}s, whitelist: {})",
            self.greylist_period,
            self.max_entry_age,
            self.whitelist.len()
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        if !self.enabled {
            return Ok(MailetAction::Continue);
        }

        let tuple = match self.create_tuple(mail) {
            Some(t) => t,
            None => {
                tracing::debug!("Cannot create greylist tuple for mail {}", mail.id());
                return Ok(MailetAction::Continue);
            }
        };

        // Check whitelist
        if self.store.is_whitelisted(&tuple, &self.whitelist) {
            tracing::debug!("Mail {} is whitelisted, skipping greylist", mail.id());
            mail.set_attribute("greylist.whitelisted", true);
            return Ok(MailetAction::Continue);
        }

        // Check and update greylist
        let (passed, entry) = self.store.check_and_update(tuple, self.greylist_period);

        mail.set_attribute("greylist.attempts", entry.attempts as i64);
        mail.set_attribute("greylist.first_seen", entry.first_seen as i64);

        if passed {
            tracing::debug!("Mail {} passed greylist check", mail.id());
            mail.set_attribute("greylist.passed", true);
            Ok(MailetAction::Continue)
        } else {
            tracing::info!(
                "Mail {} greylisted (attempt {}), deferring",
                mail.id(),
                entry.attempts
            );
            mail.set_attribute("greylist.passed", false);
            mail.set_attribute("greylist.deferred", true);

            // Defer for greylist period
            Ok(MailetAction::Defer(Duration::from_secs(
                self.greylist_period,
            )))
        }
    }

    async fn destroy(&mut self) -> anyhow::Result<()> {
        // Cleanup old entries
        self.store.cleanup_old_entries(self.max_entry_age);
        Ok(())
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
    async fn test_greylist_mailet_init() {
        let mut mailet = GreylistMailet::new();
        let config = MailetConfig::new("Greylist");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.name(), "Greylist");
        assert_eq!(mailet.greylist_period, 300);
    }

    #[tokio::test]
    async fn test_greylist_first_attempt_deferred() {
        let mailet = GreylistMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.168.1.1");

        let action = mailet.service(&mut mail).await.unwrap();

        // First attempt should be deferred
        assert!(matches!(action, MailetAction::Defer(_)));
        assert_eq!(
            mail.get_attribute("greylist.passed")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            mail.get_attribute("greylist.attempts")
                .and_then(|v| v.as_i64()),
            Some(1)
        );
    }

    #[tokio::test]
    async fn test_greylist_whitelist() {
        let mut mailet = GreylistMailet::new();
        mailet.add_to_whitelist("192.168.1.100".to_string());

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.168.1.100");

        let action = mailet.service(&mut mail).await.unwrap();

        // Whitelisted, should continue
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("greylist.whitelisted")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_greylist_domain_whitelist() {
        let mut mailet = GreylistMailet::new();
        mailet.add_to_whitelist("trusted.com".to_string());

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@trusted.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.168.1.1");

        let action = mailet.service(&mut mail).await.unwrap();

        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("greylist.whitelisted")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_greylist_config_period() {
        let mut mailet = GreylistMailet::new();
        let config = MailetConfig::new("Greylist").with_param("greylist_period", "600");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.greylist_period, 600);
    }

    #[tokio::test]
    async fn test_greylist_config_max_age() {
        let mut mailet = GreylistMailet::new();
        let config = MailetConfig::new("Greylist").with_param("max_entry_age", "172800");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.max_entry_age, 172800);
    }

    #[tokio::test]
    async fn test_greylist_config_whitelist() {
        let mut mailet = GreylistMailet::new();
        let config = MailetConfig::new("Greylist")
            .with_param("whitelist", "192.168.1.1,192.168.1.2,trusted.com");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.whitelist.len(), 3);
        assert!(mailet.whitelist.contains("192.168.1.1"));
        assert!(mailet.whitelist.contains("trusted.com"));
    }

    #[tokio::test]
    async fn test_greylist_disabled() {
        let mut mailet = GreylistMailet::new();
        let config = MailetConfig::new("Greylist").with_param("enabled", "false");

        mailet.init(config).await.unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.168.1.1");

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
    }

    #[tokio::test]
    async fn test_greylist_tuple_creation() {
        let mailet = GreylistMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.168.1.1");

        let tuple = mailet.create_tuple(&mail).unwrap();
        assert_eq!(tuple.sender_ip, "192.168.1.1");
        assert_eq!(tuple.mail_from, "sender@test.com");
        assert_eq!(tuple.rcpt_to, "rcpt@example.com");
    }

    #[tokio::test]
    async fn test_greylist_multiple_attempts() {
        let mailet = GreylistMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.168.1.1");

        // First attempt
        mailet.service(&mut mail).await.unwrap();
        let attempts1 = mail
            .get_attribute("greylist.attempts")
            .and_then(|v| v.as_i64());

        // Second attempt
        mailet.service(&mut mail).await.unwrap();
        let attempts2 = mail
            .get_attribute("greylist.attempts")
            .and_then(|v| v.as_i64());

        assert_eq!(attempts1, Some(1));
        assert_eq!(attempts2, Some(2));
    }

    #[test]
    fn test_greylist_entry_creation() {
        let entry = GreylistEntry::new();
        assert_eq!(entry.attempts, 1);
        assert!(!entry.passed);
    }

    #[test]
    fn test_greylist_entry_update() {
        let mut entry = GreylistEntry::new();
        let first_seen = entry.first_seen;

        std::thread::sleep(std::time::Duration::from_millis(10));
        entry.update();

        assert_eq!(entry.attempts, 2);
        assert_eq!(entry.first_seen, first_seen); // Should not change
        assert!(entry.last_seen >= first_seen);
    }

    #[test]
    fn test_greylist_store_check() {
        let store = GreylistStore::new();
        let tuple = GreylistTuple::new(
            "192.168.1.1".to_string(),
            "sender@test.com".to_string(),
            "rcpt@example.com".to_string(),
        );

        let (passed1, entry1) = store.check_and_update(tuple.clone(), 5);
        assert!(!passed1);
        assert_eq!(entry1.attempts, 1);

        let (passed2, entry2) = store.check_and_update(tuple.clone(), 5);
        assert!(!passed2);
        assert_eq!(entry2.attempts, 2);
    }

    #[test]
    fn test_greylist_store_whitelist_ip() {
        let store = GreylistStore::new();
        let tuple = GreylistTuple::new(
            "192.168.1.1".to_string(),
            "sender@test.com".to_string(),
            "rcpt@example.com".to_string(),
        );

        let mut whitelist = HashSet::new();
        whitelist.insert("192.168.1.1".to_string());

        assert!(store.is_whitelisted(&tuple, &whitelist));
    }

    #[test]
    fn test_greylist_store_whitelist_sender() {
        let store = GreylistStore::new();
        let tuple = GreylistTuple::new(
            "192.168.1.1".to_string(),
            "sender@test.com".to_string(),
            "rcpt@example.com".to_string(),
        );

        let mut whitelist = HashSet::new();
        whitelist.insert("sender@test.com".to_string());

        assert!(store.is_whitelisted(&tuple, &whitelist));
    }

    #[test]
    fn test_greylist_store_whitelist_domain() {
        let store = GreylistStore::new();
        let tuple = GreylistTuple::new(
            "192.168.1.1".to_string(),
            "sender@test.com".to_string(),
            "rcpt@example.com".to_string(),
        );

        let mut whitelist = HashSet::new();
        whitelist.insert("test.com".to_string());

        assert!(store.is_whitelisted(&tuple, &whitelist));
    }

    #[test]
    fn test_greylist_store_cleanup() {
        let store = GreylistStore::new();
        let tuple = GreylistTuple::new(
            "192.168.1.1".to_string(),
            "sender@test.com".to_string(),
            "rcpt@example.com".to_string(),
        );

        store.check_and_update(tuple.clone(), 0);

        // Entries count before cleanup
        let count_before = store
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        assert_eq!(count_before, 1);

        // Cleanup with 0 max age (should remove all)
        store.cleanup_old_entries(0);

        let count_after = store
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        assert_eq!(count_after, 0);
    }

    #[tokio::test]
    async fn test_greylist_no_sender_ip() {
        let mailet = GreylistMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        // No sender IP set

        let action = mailet.service(&mut mail).await.unwrap();
        // Should continue without greylisting
        assert_eq!(action, MailetAction::Continue);
    }

    #[tokio::test]
    async fn test_greylist_destroy_cleanup() {
        let mut mailet = GreylistMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.client_ip", "192.168.1.1");

        mailet.service(&mut mail).await.unwrap();

        // Destroy should cleanup
        mailet.destroy().await.unwrap();
    }
}

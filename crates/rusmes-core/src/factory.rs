//! Factory for creating mailets and matchers from configuration

use crate::mailet::{Mailet, MailetConfig};
use crate::mailets::{
    AddHeaderMailet, DkimVerifyMailet, DmarcVerifyMailet, LocalDeliveryMailet,
    RemoteDeliveryMailet, RemoveMimeHeaderMailet, SpamAssassinMailet, SpfCheckMailet,
    VirusScanMailet,
};
use crate::matcher::Matcher;
use crate::matcher::{AllMatcher, NoneMatcher};
use crate::matchers::{
    AndMatcher, HeaderContainsMatcher, IsInBlacklistMatcher, IsInWhitelistMatcher, NotMatcher,
    OrMatcher, RecipientIsLocalMatcher, RemoteAddressMatcher, SizeGreaterThanMatcher,
};
use rusmes_storage::StorageBackend;
use std::net::IpAddr;
use std::sync::Arc;

/// Create a mailet from configuration
pub async fn create_mailet(name: &str, config: MailetConfig) -> anyhow::Result<Arc<dyn Mailet>> {
    create_mailet_with_storage(name, config, None).await
}

/// Create a mailet from configuration with optional storage backend
pub async fn create_mailet_with_storage(
    name: &str,
    config: MailetConfig,
    storage: Option<Arc<dyn StorageBackend>>,
) -> anyhow::Result<Arc<dyn Mailet>> {
    match name {
        "LocalDelivery" => {
            let mut mailet = if let Some(storage) = storage {
                LocalDeliveryMailet::with_storage(storage)
            } else {
                LocalDeliveryMailet::new()
            };
            mailet.init(config).await?;
            Ok(Arc::new(mailet))
        }
        "RemoteDelivery" => {
            let mut mailet = RemoteDeliveryMailet::new();
            mailet.init(config).await?;
            Ok(Arc::new(mailet))
        }
        "AddHeader" => {
            let mut mailet = AddHeaderMailet::new();
            mailet.init(config).await?;
            Ok(Arc::new(mailet))
        }
        "RemoveMimeHeader" => {
            let mut mailet = RemoveMimeHeaderMailet::new();
            mailet.init(config).await?;
            Ok(Arc::new(mailet))
        }
        "SpamAssassin" => {
            let mut mailet = SpamAssassinMailet::new();
            mailet.init(config).await?;
            Ok(Arc::new(mailet))
        }
        "VirusScan" => {
            let mut mailet = VirusScanMailet::new();
            mailet.init(config).await?;
            Ok(Arc::new(mailet))
        }
        "DkimVerify" => {
            let mut mailet = DkimVerifyMailet::new();
            mailet.init(config).await?;
            Ok(Arc::new(mailet))
        }
        "SpfCheck" => {
            let mut mailet = SpfCheckMailet::new();
            mailet.init(config).await?;
            Ok(Arc::new(mailet))
        }
        "DmarcVerify" => {
            let mut mailet = DmarcVerifyMailet::new();
            mailet.init(config).await?;
            Ok(Arc::new(mailet))
        }
        _ => Err(anyhow::anyhow!("Unknown mailet: {}", name)),
    }
}

/// Create a matcher from configuration
pub fn create_matcher(name: &str, local_domains: Vec<String>) -> anyhow::Result<Arc<dyn Matcher>> {
    match name {
        "All" => Ok(Arc::new(AllMatcher)),
        "None" => Ok(Arc::new(NoneMatcher)),
        "RecipientIsLocal" => Ok(Arc::new(RecipientIsLocalMatcher::new(local_domains))),
        _ => Err(anyhow::anyhow!("Unknown matcher: {}", name)),
    }
}

/// Create a SizeGreaterThan matcher
pub fn create_size_greater_than_matcher(threshold_bytes: usize) -> Arc<dyn Matcher> {
    Arc::new(SizeGreaterThanMatcher::new(threshold_bytes))
}

/// Create a HeaderContains matcher
pub fn create_header_contains_matcher(header_name: String, value: String) -> Arc<dyn Matcher> {
    Arc::new(HeaderContainsMatcher::new(header_name, value))
}

/// Create a RemoteAddress matcher
pub fn create_remote_address_matcher(
    allowed_ips: Vec<IpAddr>,
    allowed_cidrs: Vec<(IpAddr, u8)>,
) -> Arc<dyn Matcher> {
    Arc::new(RemoteAddressMatcher::new(allowed_ips, allowed_cidrs))
}

/// Create an IsInWhitelist matcher
pub fn create_whitelist_matcher(whitelist: Vec<String>) -> Arc<dyn Matcher> {
    Arc::new(IsInWhitelistMatcher::new(whitelist))
}

/// Create an IsInBlacklist matcher
pub fn create_blacklist_matcher(blacklist: Vec<String>) -> Arc<dyn Matcher> {
    Arc::new(IsInBlacklistMatcher::new(blacklist))
}

/// Create an And matcher
pub fn create_and_matcher(matchers: Vec<Arc<dyn Matcher>>) -> Arc<dyn Matcher> {
    Arc::new(AndMatcher::new(matchers))
}

/// Create an Or matcher
pub fn create_or_matcher(matchers: Vec<Arc<dyn Matcher>>) -> Arc<dyn Matcher> {
    Arc::new(OrMatcher::new(matchers))
}

/// Create a Not matcher
pub fn create_not_matcher(matcher: Arc<dyn Matcher>) -> Arc<dyn Matcher> {
    Arc::new(NotMatcher::new(matcher))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_mailet() {
        let config = MailetConfig::new("LocalDelivery");
        let mailet = create_mailet("LocalDelivery", config).await;
        assert!(mailet.is_ok());
    }

    #[test]
    fn test_create_matcher() {
        let matcher = create_matcher("All", vec![]);
        assert!(matcher.is_ok());
        assert_eq!(matcher.unwrap().name(), "All");
    }
}

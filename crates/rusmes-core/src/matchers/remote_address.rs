//! Matcher for messages from specific remote addresses or CIDR ranges

use crate::matcher::Matcher;
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};
use std::net::IpAddr;

/// Matches messages from specific IP addresses or CIDR ranges
pub struct RemoteAddressMatcher {
    allowed_ips: Vec<IpAddr>,
    allowed_cidrs: Vec<(IpAddr, u8)>,
}

impl RemoteAddressMatcher {
    /// Create a new RemoteAddress matcher with IP addresses and CIDR ranges
    pub fn new(allowed_ips: Vec<IpAddr>, allowed_cidrs: Vec<(IpAddr, u8)>) -> Self {
        Self {
            allowed_ips,
            allowed_cidrs,
        }
    }

    /// Check if an IP address matches any of the allowed CIDR ranges
    fn matches_cidr(&self, addr: &IpAddr) -> bool {
        for (cidr_addr, prefix_len) in &self.allowed_cidrs {
            if Self::ip_in_cidr(addr, cidr_addr, *prefix_len) {
                return true;
            }
        }
        false
    }

    /// Check if an IP address is within a CIDR range
    fn ip_in_cidr(addr: &IpAddr, cidr_addr: &IpAddr, prefix_len: u8) -> bool {
        match (addr, cidr_addr) {
            (IpAddr::V4(a), IpAddr::V4(c)) => {
                let addr_bits = u32::from_be_bytes(a.octets());
                let cidr_bits = u32::from_be_bytes(c.octets());
                let mask = !0u32 << (32 - prefix_len);
                (addr_bits & mask) == (cidr_bits & mask)
            }
            (IpAddr::V6(a), IpAddr::V6(c)) => {
                let addr_bits = u128::from_be_bytes(a.octets());
                let cidr_bits = u128::from_be_bytes(c.octets());
                let mask = !0u128 << (128 - prefix_len);
                (addr_bits & mask) == (cidr_bits & mask)
            }
            _ => false,
        }
    }
}

#[async_trait]
impl Matcher for RemoteAddressMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        if let Some(remote_addr) = mail.remote_addr() {
            if self.allowed_ips.contains(remote_addr) || self.matches_cidr(remote_addr) {
                return Ok(mail.recipients().to_vec());
            }
        }
        Ok(Vec::new())
    }

    fn name(&self) -> &str {
        "RemoteAddress"
    }
}

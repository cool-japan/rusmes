//! Matcher for messages with specific header values

use crate::matcher::Matcher;
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};

/// Matches messages where a header contains a specific value
pub struct HeaderContainsMatcher {
    header_name: String,
    value: String,
}

impl HeaderContainsMatcher {
    /// Create a new HeaderContains matcher
    pub fn new(header_name: String, value: String) -> Self {
        Self { header_name, value }
    }
}

#[async_trait]
impl Matcher for HeaderContainsMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        let headers = mail.message().headers();

        if let Some(header_values) = headers.get(&self.header_name) {
            for header_value in header_values {
                if header_value.contains(&self.value) {
                    return Ok(mail.recipients().to_vec());
                }
            }
        }

        Ok(Vec::new())
    }

    fn name(&self) -> &str {
        "HeaderContains"
    }
}

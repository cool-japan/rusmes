//! Sieve mail filtering mailet (RFC 5228)

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use crate::sieve::{SieveAction, SieveInterpreter, SieveScript};
use async_trait::async_trait;
use rusmes_proto::Mail;
use std::collections::HashMap;
use std::path::PathBuf;

/// Sieve mail filtering mailet
pub struct SieveMailet {
    name: String,
    /// Per-user Sieve scripts
    user_scripts: HashMap<String, SieveScript>,
    /// Global script applied to all mail
    global_script: Option<SieveScript>,
    /// Script directory path
    script_dir: Option<PathBuf>,
}

impl SieveMailet {
    /// Create a new Sieve mailet
    pub fn new() -> Self {
        Self {
            name: "Sieve".to_string(),
            user_scripts: HashMap::new(),
            global_script: None,
            script_dir: None,
        }
    }

    /// Add a user script
    pub fn add_user_script(&mut self, user: String, script: SieveScript) -> Result<(), String> {
        script.validate()?;
        self.user_scripts.insert(user, script);
        Ok(())
    }

    /// Set global script
    pub fn set_global_script(&mut self, script: SieveScript) -> Result<(), String> {
        script.validate()?;
        self.global_script = Some(script);
        Ok(())
    }

    /// Get script for a user
    fn get_script_for_user(&self, user: &str) -> Option<&SieveScript> {
        self.user_scripts.get(user)
    }

    /// Extract user from recipient address
    fn extract_user(&self, mail: &Mail) -> Option<String> {
        if let Some(first_rcpt) = mail.recipients().first() {
            let local = first_rcpt.local_part();
            return Some(local.to_string());
        }
        None
    }

    /// Apply Sieve actions to mail
    fn apply_actions(&self, mail: &mut Mail, actions: Vec<SieveAction>) -> MailetAction {
        for action in actions {
            match action {
                SieveAction::Keep | SieveAction::ImplicitKeep => {
                    // Keep in inbox - set attribute for local delivery
                    mail.set_attribute("sieve.action", "keep");
                }
                SieveAction::Fileinto(mailbox) => {
                    // File into specific mailbox
                    mail.set_attribute("sieve.action", "fileinto");
                    mail.set_attribute("sieve.mailbox", mailbox);
                }
                SieveAction::Redirect(address) => {
                    // Redirect - set attribute for forwarding
                    mail.set_attribute("sieve.action", "redirect");
                    mail.set_attribute("sieve.redirect_to", address);
                }
                SieveAction::Discard => {
                    // Discard message
                    tracing::info!("Sieve: Discarding mail {}", mail.id());
                    return MailetAction::Drop;
                }
            }
        }

        MailetAction::Continue
    }
}

impl Default for SieveMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for SieveMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        // Set script directory
        if let Some(dir) = config.get_param("script_dir") {
            self.script_dir = Some(PathBuf::from(dir));
        }

        // Load global script if specified
        if let Some(global_script_text) = config.get_param("global_script") {
            let script = SieveScript::parse(global_script_text)
                .map_err(|e| anyhow::anyhow!("Failed to parse global script: {}", e))?;
            self.set_global_script(script)
                .map_err(|e| anyhow::anyhow!("Failed to validate global script: {}", e))?;
        }

        tracing::info!(
            "Initialized SieveMailet with {} user scripts",
            self.user_scripts.len()
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        // Try to get user-specific script
        let script = if let Some(user) = self.extract_user(mail) {
            self.get_script_for_user(&user)
                .or(self.global_script.as_ref())
        } else {
            self.global_script.as_ref()
        };

        if let Some(script) = script {
            tracing::debug!("Executing Sieve script for mail {}", mail.id());

            let interpreter = SieveInterpreter::new(mail.clone());
            let actions = interpreter
                .execute(script)
                .map_err(|e| anyhow::anyhow!("Sieve execution error: {}", e))?;

            tracing::debug!("Sieve actions: {:?}", actions);

            Ok(self.apply_actions(mail, actions))
        } else {
            tracing::debug!("No Sieve script found for mail {}", mail.id());
            Ok(MailetAction::Continue)
        }
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
    async fn test_sieve_mailet_init() {
        let mut mailet = SieveMailet::new();
        let config = MailetConfig::new("Sieve");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.name(), "Sieve");
    }

    #[tokio::test]
    async fn test_sieve_mailet_keep() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse("keep;").unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.action").and_then(|v| v.as_str()),
            Some("keep")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_fileinto() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse(r#"fileinto "Spam";"#).unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.action").and_then(|v| v.as_str()),
            Some("fileinto")
        );
        assert_eq!(
            mail.get_attribute("sieve.mailbox").and_then(|v| v.as_str()),
            Some("Spam")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_discard() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse("discard;").unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Drop);
    }

    #[tokio::test]
    async fn test_sieve_mailet_redirect() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse(r#"redirect "other@test.com";"#).unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.action").and_then(|v| v.as_str()),
            Some("redirect")
        );
        assert_eq!(
            mail.get_attribute("sieve.redirect_to")
                .and_then(|v| v.as_str()),
            Some("other@test.com")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_header_test() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse(
            r#"
            if header :contains "Subject" "spam" {
                fileinto "Spam";
            }
            "#,
        )
        .unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Subject", "This is spam");

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.mailbox").and_then(|v| v.as_str()),
            Some("Spam")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_global_script() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse("keep;").unwrap();
        mailet.set_global_script(script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("unknown@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.action").and_then(|v| v.as_str()),
            Some("keep")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_no_script() {
        let mailet = SieveMailet::new();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
    }

    #[tokio::test]
    async fn test_sieve_mailet_size_test() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse(
            r#"
            if size :over 100000 {
                discard;
            }
            "#,
        )
        .unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("message.size", 200000_i64);

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Drop);
    }

    #[tokio::test]
    async fn test_sieve_mailet_exists_test() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse(
            r#"
            if exists "X-Spam-Flag" {
                fileinto "Spam";
            }
            "#,
        )
        .unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.X-Spam-Flag", "YES");

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.mailbox").and_then(|v| v.as_str()),
            Some("Spam")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_if_else() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse(
            r#"
            if false {
                discard;
            } else {
                keep;
            }
            "#,
        )
        .unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.action").and_then(|v| v.as_str()),
            Some("keep")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_allof() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse(
            r#"
            if allof(true, exists "Subject") {
                keep;
            }
            "#,
        )
        .unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Subject", "Test");

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.action").and_then(|v| v.as_str()),
            Some("keep")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_anyof() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse(
            r#"
            if anyof(true, false) {
                keep;
            }
            "#,
        )
        .unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
    }

    #[tokio::test]
    async fn test_sieve_mailet_not() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse(
            r#"
            if not false {
                keep;
            }
            "#,
        )
        .unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
    }

    #[tokio::test]
    async fn test_sieve_mailet_validation_error() {
        let mut mailet = SieveMailet::new();
        let mut script = SieveScript::new();
        script.requires.push("unknown_extension".to_string());

        let result = mailet.add_user_script("user".to_string(), script);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_sieve_mailet_parse_error() {
        let result = SieveScript::parse("invalid syntax ;;;");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_sieve_mailet_config_global_script() {
        let mut mailet = SieveMailet::new();
        let config = MailetConfig::new("Sieve").with_param("global_script", "keep;");

        mailet.init(config).await.unwrap();
        assert!(mailet.global_script.is_some());
    }

    #[tokio::test]
    async fn test_sieve_mailet_config_script_dir() {
        let mut mailet = SieveMailet::new();
        let config = MailetConfig::new("Sieve").with_param("script_dir", "/var/sieve");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.script_dir, Some(PathBuf::from("/var/sieve")));
    }

    #[tokio::test]
    async fn test_sieve_mailet_user_precedence() {
        let mut mailet = SieveMailet::new();

        // Set global script to discard
        let global = SieveScript::parse("discard;").unwrap();
        mailet.set_global_script(global).unwrap();

        // Set user script to keep
        let user = SieveScript::parse("keep;").unwrap();
        mailet.add_user_script("user".to_string(), user).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        // User script should take precedence
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.action").and_then(|v| v.as_str()),
            Some("keep")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_complex_script() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse(
            r#"
            require "fileinto";

            if header :contains "Subject" "urgent" {
                fileinto "Urgent";
                stop;
            }

            if header :contains "From" "boss@example.com" {
                fileinto "Important";
            } else {
                keep;
            }
            "#,
        )
        .unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Subject", "urgent matter");

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.mailbox").and_then(|v| v.as_str()),
            Some("Urgent")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_implicit_keep() {
        let mut mailet = SieveMailet::new();
        let script = SieveScript::parse("# Empty script, should implicit keep").unwrap();
        mailet.add_user_script("user".to_string(), script).unwrap();

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("sieve.action").and_then(|v| v.as_str()),
            Some("keep")
        );
    }

    #[tokio::test]
    async fn test_sieve_mailet_multiple_users() {
        let mut mailet = SieveMailet::new();

        let script1 = SieveScript::parse(r#"fileinto "User1";"#).unwrap();
        mailet
            .add_user_script("user1".to_string(), script1)
            .unwrap();

        let script2 = SieveScript::parse(r#"fileinto "User2";"#).unwrap();
        mailet
            .add_user_script("user2".to_string(), script2)
            .unwrap();

        let mut mail1 = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user1@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail1).await.unwrap();
        assert_eq!(
            mail1
                .get_attribute("sieve.mailbox")
                .and_then(|v| v.as_str()),
            Some("User1")
        );

        let mut mail2 = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user2@example.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mailet.service(&mut mail2).await.unwrap();
        assert_eq!(
            mail2
                .get_attribute("sieve.mailbox")
                .and_then(|v| v.as_str()),
            Some("User2")
        );
    }
}

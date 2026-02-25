//! Sieve script interpreter

use super::parser::{SieveCommand, SieveScript, SieveTest};
use rusmes_proto::Mail;
use std::collections::HashMap;

#[cfg(test)]
use bytes::Bytes;
#[cfg(test)]
use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

/// Sieve action result
#[derive(Debug, Clone, PartialEq)]
pub enum SieveAction {
    /// Keep message in default location
    Keep,
    /// File into specific mailbox
    Fileinto(String),
    /// Redirect to address
    Redirect(String),
    /// Discard message
    Discard,
    /// Implicit keep (no action taken)
    ImplicitKeep,
}

/// Sieve execution context
#[derive(Debug, Clone)]
pub struct SieveContext {
    /// Variables (RFC 5229)
    pub variables: HashMap<String, String>,
    /// Whether implicit keep is canceled
    pub implicit_keep_canceled: bool,
    /// Actions to perform
    pub actions: Vec<SieveAction>,
    /// Whether to stop processing
    pub stopped: bool,
}

impl SieveContext {
    /// Create a new context
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            implicit_keep_canceled: false,
            actions: Vec::new(),
            stopped: false,
        }
    }

    /// Add an action
    pub fn add_action(&mut self, action: SieveAction) {
        if action != SieveAction::ImplicitKeep {
            self.implicit_keep_canceled = true;
        }
        self.actions.push(action);
    }

    /// Get final actions
    pub fn finalize(&mut self) -> Vec<SieveAction> {
        if !self.implicit_keep_canceled && self.actions.is_empty() {
            vec![SieveAction::ImplicitKeep]
        } else {
            self.actions.clone()
        }
    }
}

impl Default for SieveContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Sieve script interpreter
pub struct SieveInterpreter {
    /// Mail message being processed
    mail: Mail,
}

impl SieveInterpreter {
    /// Create a new interpreter
    pub fn new(mail: Mail) -> Self {
        Self { mail }
    }

    /// Execute a Sieve script
    pub fn execute(&self, script: &SieveScript) -> Result<Vec<SieveAction>, String> {
        let mut context = SieveContext::new();

        for command in &script.commands {
            if context.stopped {
                break;
            }
            self.execute_command(command, &mut context)?;
        }

        Ok(context.finalize())
    }

    fn execute_command(
        &self,
        command: &SieveCommand,
        context: &mut SieveContext,
    ) -> Result<(), String> {
        match command {
            SieveCommand::Keep => {
                context.add_action(SieveAction::Keep);
            }
            SieveCommand::Fileinto(mailbox) => {
                context.add_action(SieveAction::Fileinto(mailbox.clone()));
            }
            SieveCommand::Redirect(address) => {
                context.add_action(SieveAction::Redirect(address.clone()));
            }
            SieveCommand::Discard => {
                context.add_action(SieveAction::Discard);
            }
            SieveCommand::Stop => {
                context.stopped = true;
            }
            SieveCommand::If {
                test,
                then_commands,
                elsif_branches,
                else_commands,
            } => {
                if self.evaluate_test(test, context)? {
                    for cmd in then_commands {
                        self.execute_command(cmd, context)?;
                    }
                } else {
                    let mut executed = false;
                    for (elsif_test, elsif_commands) in elsif_branches {
                        if self.evaluate_test(elsif_test, context)? {
                            for cmd in elsif_commands {
                                self.execute_command(cmd, context)?;
                            }
                            executed = true;
                            break;
                        }
                    }
                    if !executed {
                        if let Some(else_cmds) = else_commands {
                            for cmd in else_cmds {
                                self.execute_command(cmd, context)?;
                            }
                        }
                    }
                }
            }
            SieveCommand::Require(_) => {
                // Already processed during parsing
            }
            SieveCommand::Set { name, value } => {
                context.variables.insert(name.clone(), value.clone());
            }
            SieveCommand::Vacation { .. } => {
                // Vacation handling would require tracking sent auto-replies
                // For now, just ignore
            }
        }
        Ok(())
    }

    #[allow(clippy::only_used_in_recursion)]
    fn evaluate_test(&self, test: &SieveTest, context: &SieveContext) -> Result<bool, String> {
        match test {
            SieveTest::True => Ok(true),
            SieveTest::False => Ok(false),
            SieveTest::Header {
                comparator: _,
                match_type,
                headers,
                keys,
            } => self.evaluate_header_test(match_type, headers, keys),
            SieveTest::Address {
                comparator: _,
                match_type,
                headers,
                keys,
            } => self.evaluate_address_test(match_type, headers, keys),
            SieveTest::Envelope {
                comparator: _,
                match_type,
                parts,
                keys,
            } => self.evaluate_envelope_test(match_type, parts, keys),
            SieveTest::Exists(headers) => Ok(self.evaluate_exists_test(headers)),
            SieveTest::Size { over, limit } => Ok(self.evaluate_size_test(*over, *limit)),
            SieveTest::AllOf(tests) => {
                for t in tests {
                    if !self.evaluate_test(t, context)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            SieveTest::AnyOf(tests) => {
                for t in tests {
                    if self.evaluate_test(t, context)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            SieveTest::Not(test) => Ok(!self.evaluate_test(test, context)?),
        }
    }

    fn evaluate_header_test(
        &self,
        match_type: &str,
        headers: &[String],
        keys: &[String],
    ) -> Result<bool, String> {
        for header_name in headers {
            let attr_key = format!("header.{}", header_name);
            if let Some(header_value) = self.mail.get_attribute(&attr_key) {
                if let Some(value_str) = header_value.as_str() {
                    for key in keys {
                        if self.string_match(match_type, value_str, key) {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    fn evaluate_address_test(
        &self,
        match_type: &str,
        headers: &[String],
        keys: &[String],
    ) -> Result<bool, String> {
        // For simplicity, treat as header test
        // In real implementation, would parse addresses from headers
        self.evaluate_header_test(match_type, headers, keys)
    }

    fn evaluate_envelope_test(
        &self,
        match_type: &str,
        parts: &[String],
        keys: &[String],
    ) -> Result<bool, String> {
        for part in parts {
            let value = match part.as_str() {
                "from" => {
                    if let Some(sender) = self.mail.sender() {
                        sender.to_string()
                    } else {
                        continue;
                    }
                }
                "to" => {
                    if let Some(first_rcpt) = self.mail.recipients().first() {
                        first_rcpt.to_string()
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };

            for key in keys {
                if self.string_match(match_type, &value, key) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn evaluate_exists_test(&self, headers: &[String]) -> bool {
        for header_name in headers {
            let attr_key = format!("header.{}", header_name);
            if self.mail.get_attribute(&attr_key).is_some() {
                return true;
            }
        }
        false
    }

    fn evaluate_size_test(&self, over: bool, limit: i64) -> bool {
        // Get message size from attributes
        if let Some(size_attr) = self.mail.get_attribute("message.size") {
            if let Some(size) = size_attr.as_i64() {
                if over {
                    return size > limit;
                } else {
                    return size < limit;
                }
            }
        }
        false
    }

    fn string_match(&self, match_type: &str, value: &str, pattern: &str) -> bool {
        match match_type {
            "is" => value.eq_ignore_ascii_case(pattern),
            "contains" => value.to_lowercase().contains(&pattern.to_lowercase()),
            "matches" => {
                // Simple wildcard matching (* and ?)
                self.wildcard_match(value, pattern)
            }
            _ => false,
        }
    }

    fn wildcard_match(&self, value: &str, pattern: &str) -> bool {
        let value_lower = value.to_lowercase();
        let pattern_lower = pattern.to_lowercase();

        let mut v_idx = 0;
        let mut p_idx = 0;
        let v_chars: Vec<char> = value_lower.chars().collect();
        let p_chars: Vec<char> = pattern_lower.chars().collect();

        let mut star_idx = None;
        let mut match_idx = 0;

        while v_idx < v_chars.len() {
            if p_idx < p_chars.len() && (p_chars[p_idx] == '?' || p_chars[p_idx] == v_chars[v_idx])
            {
                v_idx += 1;
                p_idx += 1;
            } else if p_idx < p_chars.len() && p_chars[p_idx] == '*' {
                star_idx = Some(p_idx);
                match_idx = v_idx;
                p_idx += 1;
            } else if let Some(s_idx) = star_idx {
                p_idx = s_idx + 1;
                match_idx += 1;
                v_idx = match_idx;
            } else {
                return false;
            }
        }

        while p_idx < p_chars.len() && p_chars[p_idx] == '*' {
            p_idx += 1;
        }

        p_idx == p_chars.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusmes_proto::MailAddress;
    use std::str::FromStr;

    #[test]
    fn test_implicit_keep() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        let script = SieveScript::new();
        let interpreter = SieveInterpreter::new(mail);

        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::ImplicitKeep]);
    }

    #[test]
    fn test_keep_command() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        let mut script = SieveScript::new();
        script.add_command(SieveCommand::Keep);

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Keep]);
    }

    #[test]
    fn test_fileinto_command() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        let mut script = SieveScript::new();
        script.add_command(SieveCommand::Fileinto("Spam".to_string()));

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Fileinto("Spam".to_string())]);
    }

    #[test]
    fn test_discard_command() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        let mut script = SieveScript::new();
        script.add_command(SieveCommand::Discard);

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Discard]);
    }

    #[test]
    fn test_redirect_command() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        let mut script = SieveScript::new();
        script.add_command(SieveCommand::Redirect("other@test.com".to_string()));

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(
            actions,
            vec![SieveAction::Redirect("other@test.com".to_string())]
        );
    }

    #[test]
    fn test_if_true() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::True,
            then_commands: vec![SieveCommand::Discard],
            elsif_branches: vec![],
            else_commands: None,
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Discard]);
    }

    #[test]
    fn test_if_false() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::False,
            then_commands: vec![SieveCommand::Discard],
            elsif_branches: vec![],
            else_commands: Some(vec![SieveCommand::Keep]),
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Keep]);
    }

    #[test]
    fn test_header_test_is() {
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Subject", "Test");

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::Header {
                comparator: None,
                match_type: "is".to_string(),
                headers: vec!["Subject".to_string()],
                keys: vec!["Test".to_string()],
            },
            then_commands: vec![SieveCommand::Discard],
            elsif_branches: vec![],
            else_commands: None,
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Discard]);
    }

    #[test]
    fn test_header_test_contains() {
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Subject", "This is spam");

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::Header {
                comparator: None,
                match_type: "contains".to_string(),
                headers: vec!["Subject".to_string()],
                keys: vec!["spam".to_string()],
            },
            then_commands: vec![SieveCommand::Fileinto("Spam".to_string())],
            elsif_branches: vec![],
            else_commands: None,
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Fileinto("Spam".to_string())]);
    }

    #[test]
    fn test_exists_test() {
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.X-Spam-Flag", "YES");

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::Exists(vec!["X-Spam-Flag".to_string()]),
            then_commands: vec![SieveCommand::Fileinto("Spam".to_string())],
            elsif_branches: vec![],
            else_commands: None,
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Fileinto("Spam".to_string())]);
    }

    #[test]
    fn test_size_test_over() {
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("message.size", 200000_i64);

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::Size {
                over: true,
                limit: 100000,
            },
            then_commands: vec![SieveCommand::Discard],
            elsif_branches: vec![],
            else_commands: None,
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Discard]);
    }

    #[test]
    fn test_size_test_under() {
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("message.size", 500_i64);

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::Size {
                over: false,
                limit: 1000,
            },
            then_commands: vec![SieveCommand::Keep],
            elsif_branches: vec![],
            else_commands: None,
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Keep]);
    }

    #[test]
    fn test_allof_test() {
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Subject", "Test");

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::AllOf(vec![
                SieveTest::True,
                SieveTest::Exists(vec!["Subject".to_string()]),
            ]),
            then_commands: vec![SieveCommand::Keep],
            elsif_branches: vec![],
            else_commands: None,
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Keep]);
    }

    #[test]
    fn test_anyof_test() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::AnyOf(vec![SieveTest::True, SieveTest::False]),
            then_commands: vec![SieveCommand::Keep],
            elsif_branches: vec![],
            else_commands: None,
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Keep]);
    }

    #[test]
    fn test_not_test() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::Not(Box::new(SieveTest::False)),
            then_commands: vec![SieveCommand::Keep],
            elsif_branches: vec![],
            else_commands: None,
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Keep]);
    }

    #[test]
    fn test_stop_command() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::Discard);
        script.add_command(SieveCommand::Stop);
        script.add_command(SieveCommand::Keep);

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        // Only discard should execute, keep should be skipped
        assert_eq!(actions, vec![SieveAction::Discard]);
    }

    #[test]
    fn test_set_command() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::Set {
            name: "myvar".to_string(),
            value: "myvalue".to_string(),
        });
        script.add_command(SieveCommand::Keep);

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Keep]);
    }

    #[test]
    fn test_wildcard_match_star() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        let interpreter = SieveInterpreter::new(mail);

        assert!(interpreter.wildcard_match("hello world", "hello*"));
        assert!(interpreter.wildcard_match("hello world", "*world"));
        assert!(interpreter.wildcard_match("hello world", "hello*world"));
        assert!(!interpreter.wildcard_match("hello", "world*"));
    }

    #[test]
    fn test_wildcard_match_question() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        let interpreter = SieveInterpreter::new(mail);

        assert!(interpreter.wildcard_match("hello", "h?llo"));
        assert!(interpreter.wildcard_match("hello", "?ello"));
        assert!(!interpreter.wildcard_match("hello", "h?o"));
    }

    #[test]
    fn test_envelope_test() {
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let mut script = SieveScript::new();
        script.add_command(SieveCommand::If {
            test: SieveTest::Envelope {
                comparator: None,
                match_type: "contains".to_string(),
                parts: vec!["from".to_string()],
                keys: vec!["test.com".to_string()],
            },
            then_commands: vec![SieveCommand::Keep],
            elsif_branches: vec![],
            else_commands: None,
        });

        let interpreter = SieveInterpreter::new(mail);
        let actions = interpreter.execute(&script).unwrap();
        assert_eq!(actions, vec![SieveAction::Keep]);
    }
}

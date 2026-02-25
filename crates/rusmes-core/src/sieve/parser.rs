//! Sieve script parser (RFC 5228)

/// Sieve script value
#[derive(Debug, Clone, PartialEq)]
pub enum SieveValue {
    String(String),
    StringList(Vec<String>),
    Number(i64),
    Tag(String),
}

/// Sieve test condition
#[derive(Debug, Clone, PartialEq)]
pub enum SieveTest {
    /// true test
    True,
    /// false test
    False,
    /// header test: header ["comparator"] ["match-type"] <header-names: string-list> <key-list: string-list>
    Header {
        comparator: Option<String>,
        match_type: String,
        headers: Vec<String>,
        keys: Vec<String>,
    },
    /// address test: address \["comparator"\] \["match-type"\] `<header-list>` `<key-list>`
    Address {
        comparator: Option<String>,
        match_type: String,
        headers: Vec<String>,
        keys: Vec<String>,
    },
    /// envelope test
    Envelope {
        comparator: Option<String>,
        match_type: String,
        parts: Vec<String>,
        keys: Vec<String>,
    },
    /// exists test: exists <header-names: string-list>
    Exists(Vec<String>),
    /// size test: size <":over" / ":under"> <limit: number>
    Size { over: bool, limit: i64 },
    /// allof test: allof <tests: test-list>
    AllOf(Vec<SieveTest>),
    /// anyof test: anyof <tests: test-list>
    AnyOf(Vec<SieveTest>),
    /// not test: not `<test>`
    Not(Box<SieveTest>),
}

/// Sieve command/action
#[derive(Debug, Clone, PartialEq)]
pub enum SieveCommand {
    /// keep - keep message in inbox
    Keep,
    /// fileinto - file message into mailbox
    Fileinto(String),
    /// redirect - forward to address
    Redirect(String),
    /// discard - silently discard message
    Discard,
    /// stop - stop processing
    Stop,
    /// if/elsif/else control structure
    If {
        test: SieveTest,
        then_commands: Vec<SieveCommand>,
        elsif_branches: Vec<(SieveTest, Vec<SieveCommand>)>,
        else_commands: Option<Vec<SieveCommand>>,
    },
    /// require - declare required extensions
    Require(Vec<String>),
    /// set - set variable (RFC 5229)
    Set { name: String, value: String },
    /// vacation - auto-reply (RFC 5230)
    Vacation {
        days: Option<i64>,
        subject: Option<String>,
        from: Option<String>,
        addresses: Vec<String>,
        message: String,
    },
}

/// Parsed Sieve script
#[derive(Debug, Clone)]
pub struct SieveScript {
    /// Script commands
    pub commands: Vec<SieveCommand>,
    /// Required extensions
    pub requires: Vec<String>,
}

impl SieveScript {
    /// Create an empty Sieve script
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            requires: Vec::new(),
        }
    }

    /// Parse a Sieve script from text
    pub fn parse(script: &str) -> Result<Self, String> {
        let mut parser = Parser::new(script);
        parser.parse()
    }

    /// Add a command
    pub fn add_command(&mut self, command: SieveCommand) {
        if let SieveCommand::Require(exts) = &command {
            self.requires.extend(exts.clone());
        }
        self.commands.push(command);
    }

    /// Validate script
    pub fn validate(&self) -> Result<(), String> {
        // Check for unknown extensions
        let known_extensions = ["fileinto", "envelope", "variables", "vacation"];
        for ext in &self.requires {
            if !known_extensions.contains(&ext.as_str()) {
                return Err(format!("Unknown extension: {}", ext));
            }
        }
        Ok(())
    }
}

impl Default for SieveScript {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple Sieve script parser
struct Parser {
    input: String,
    pos: usize,
}

impl Parser {
    fn new(input: &str) -> Self {
        Self {
            input: input.to_string(),
            pos: 0,
        }
    }

    fn parse(&mut self) -> Result<SieveScript, String> {
        let mut script = SieveScript::new();

        self.skip_whitespace();
        while self.pos < self.input.len() {
            let cmd = self.parse_command()?;
            script.add_command(cmd);
            self.skip_whitespace();
        }

        Ok(script)
    }

    fn parse_command(&mut self) -> Result<SieveCommand, String> {
        self.skip_whitespace();
        let word = self.parse_word()?;

        match word.as_str() {
            "require" => self.parse_require(),
            "if" => self.parse_if(),
            "keep" => {
                self.expect(";")?;
                Ok(SieveCommand::Keep)
            }
            "discard" => {
                self.expect(";")?;
                Ok(SieveCommand::Discard)
            }
            "stop" => {
                self.expect(";")?;
                Ok(SieveCommand::Stop)
            }
            "fileinto" => self.parse_fileinto(),
            "redirect" => self.parse_redirect(),
            "set" => self.parse_set(),
            "vacation" => self.parse_vacation(),
            _ => Err(format!("Unknown command: {}", word)),
        }
    }

    fn parse_require(&mut self) -> Result<SieveCommand, String> {
        self.skip_whitespace();
        let extensions = if self.peek_char() == Some('"') {
            vec![self.parse_string()?]
        } else if self.peek_char() == Some('[') {
            self.parse_string_list()?
        } else {
            return Err("Expected string or string list after 'require'".to_string());
        };
        self.expect(";")?;
        Ok(SieveCommand::Require(extensions))
    }

    fn parse_if(&mut self) -> Result<SieveCommand, String> {
        self.skip_whitespace();
        let test = self.parse_test()?;
        self.skip_whitespace();
        let then_commands = self.parse_block()?;

        let mut elsif_branches = Vec::new();
        let mut else_commands = None;

        loop {
            self.skip_whitespace();
            if self.peek_word() == Some("elsif".to_string()) {
                self.parse_word()?; // consume "elsif"
                self.skip_whitespace();
                let elsif_test = self.parse_test()?;
                self.skip_whitespace();
                let elsif_commands = self.parse_block()?;
                elsif_branches.push((elsif_test, elsif_commands));
            } else if self.peek_word() == Some("else".to_string()) {
                self.parse_word()?; // consume "else"
                self.skip_whitespace();
                else_commands = Some(self.parse_block()?);
                break;
            } else {
                break;
            }
        }

        Ok(SieveCommand::If {
            test,
            then_commands,
            elsif_branches,
            else_commands,
        })
    }

    fn parse_fileinto(&mut self) -> Result<SieveCommand, String> {
        self.skip_whitespace();
        let mailbox = self.parse_string()?;
        self.expect(";")?;
        Ok(SieveCommand::Fileinto(mailbox))
    }

    fn parse_redirect(&mut self) -> Result<SieveCommand, String> {
        self.skip_whitespace();
        let address = self.parse_string()?;
        self.expect(";")?;
        Ok(SieveCommand::Redirect(address))
    }

    fn parse_set(&mut self) -> Result<SieveCommand, String> {
        self.skip_whitespace();
        let name = self.parse_string()?;
        self.skip_whitespace();
        let value = self.parse_string()?;
        self.expect(";")?;
        Ok(SieveCommand::Set { name, value })
    }

    fn parse_vacation(&mut self) -> Result<SieveCommand, String> {
        self.skip_whitespace();

        let mut days = None;
        let mut subject = None;
        let mut from = None;
        let mut addresses = Vec::new();

        // Parse optional tags
        while self.peek_char() == Some(':') {
            let tag = self.parse_tag()?;
            self.skip_whitespace();

            match tag.as_str() {
                ":days" => {
                    days = Some(self.parse_number()?);
                }
                ":subject" => {
                    subject = Some(self.parse_string()?);
                }
                ":from" => {
                    from = Some(self.parse_string()?);
                }
                ":addresses" => {
                    addresses = self.parse_string_list()?;
                }
                _ => return Err(format!("Unknown vacation tag: {}", tag)),
            }
            self.skip_whitespace();
        }

        let message = self.parse_string()?;
        self.expect(";")?;

        Ok(SieveCommand::Vacation {
            days,
            subject,
            from,
            addresses,
            message,
        })
    }

    fn parse_test(&mut self) -> Result<SieveTest, String> {
        self.skip_whitespace();
        let word = self.parse_word()?;

        match word.as_str() {
            "true" => Ok(SieveTest::True),
            "false" => Ok(SieveTest::False),
            "header" => self.parse_header_test(),
            "address" => self.parse_address_test(),
            "envelope" => self.parse_envelope_test(),
            "exists" => self.parse_exists_test(),
            "size" => self.parse_size_test(),
            "allof" => self.parse_allof_test(),
            "anyof" => self.parse_anyof_test(),
            "not" => self.parse_not_test(),
            _ => Err(format!("Unknown test: {}", word)),
        }
    }

    fn parse_header_test(&mut self) -> Result<SieveTest, String> {
        self.skip_whitespace();

        let mut comparator = None;
        let mut match_type = "is".to_string();

        // Parse optional tags
        while self.peek_char() == Some(':') {
            let tag = self.parse_tag()?;
            self.skip_whitespace();

            match tag.as_str() {
                ":comparator" => {
                    comparator = Some(self.parse_string()?);
                    self.skip_whitespace();
                }
                ":is" | ":contains" | ":matches" => {
                    match_type = tag[1..].to_string();
                }
                _ => return Err(format!("Unknown header test tag: {}", tag)),
            }
        }

        let headers = self.parse_string_or_list()?;
        self.skip_whitespace();
        let keys = self.parse_string_or_list()?;

        Ok(SieveTest::Header {
            comparator,
            match_type,
            headers,
            keys,
        })
    }

    fn parse_address_test(&mut self) -> Result<SieveTest, String> {
        self.skip_whitespace();

        let mut comparator = None;
        let mut match_type = "is".to_string();

        while self.peek_char() == Some(':') {
            let tag = self.parse_tag()?;
            self.skip_whitespace();

            match tag.as_str() {
                ":comparator" => {
                    comparator = Some(self.parse_string()?);
                    self.skip_whitespace();
                }
                ":is" | ":contains" | ":matches" => {
                    match_type = tag[1..].to_string();
                }
                _ => {}
            }
        }

        let headers = self.parse_string_or_list()?;
        self.skip_whitespace();
        let keys = self.parse_string_or_list()?;

        Ok(SieveTest::Address {
            comparator,
            match_type,
            headers,
            keys,
        })
    }

    fn parse_envelope_test(&mut self) -> Result<SieveTest, String> {
        self.skip_whitespace();

        let mut comparator = None;
        let mut match_type = "is".to_string();

        while self.peek_char() == Some(':') {
            let tag = self.parse_tag()?;
            self.skip_whitespace();

            match tag.as_str() {
                ":comparator" => {
                    comparator = Some(self.parse_string()?);
                    self.skip_whitespace();
                }
                ":is" | ":contains" | ":matches" => {
                    match_type = tag[1..].to_string();
                }
                _ => {}
            }
        }

        let parts = self.parse_string_or_list()?;
        self.skip_whitespace();
        let keys = self.parse_string_or_list()?;

        Ok(SieveTest::Envelope {
            comparator,
            match_type,
            parts,
            keys,
        })
    }

    fn parse_exists_test(&mut self) -> Result<SieveTest, String> {
        self.skip_whitespace();
        let headers = self.parse_string_or_list()?;
        Ok(SieveTest::Exists(headers))
    }

    fn parse_size_test(&mut self) -> Result<SieveTest, String> {
        self.skip_whitespace();
        let tag = self.parse_tag()?;
        let over = match tag.as_str() {
            ":over" => true,
            ":under" => false,
            _ => return Err(format!("Expected :over or :under, got {}", tag)),
        };
        self.skip_whitespace();
        let limit = self.parse_number()?;
        Ok(SieveTest::Size { over, limit })
    }

    fn parse_allof_test(&mut self) -> Result<SieveTest, String> {
        self.skip_whitespace();
        self.expect("(")?;
        let mut tests = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek_char() == Some(')') {
                break;
            }
            tests.push(self.parse_test()?);
            self.skip_whitespace();
            if self.peek_char() == Some(',') {
                self.advance();
            }
        }
        self.expect(")")?;
        Ok(SieveTest::AllOf(tests))
    }

    fn parse_anyof_test(&mut self) -> Result<SieveTest, String> {
        self.skip_whitespace();
        self.expect("(")?;
        let mut tests = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek_char() == Some(')') {
                break;
            }
            tests.push(self.parse_test()?);
            self.skip_whitespace();
            if self.peek_char() == Some(',') {
                self.advance();
            }
        }
        self.expect(")")?;
        Ok(SieveTest::AnyOf(tests))
    }

    fn parse_not_test(&mut self) -> Result<SieveTest, String> {
        self.skip_whitespace();
        let test = self.parse_test()?;
        Ok(SieveTest::Not(Box::new(test)))
    }

    fn parse_block(&mut self) -> Result<Vec<SieveCommand>, String> {
        self.expect("{")?;
        let mut commands = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek_char() == Some('}') {
                break;
            }
            commands.push(self.parse_command()?);
        }
        self.expect("}")?;
        Ok(commands)
    }

    fn parse_string_or_list(&mut self) -> Result<Vec<String>, String> {
        self.skip_whitespace();
        if self.peek_char() == Some('"') {
            Ok(vec![self.parse_string()?])
        } else if self.peek_char() == Some('[') {
            self.parse_string_list()
        } else {
            Err("Expected string or string list".to_string())
        }
    }

    fn parse_string_list(&mut self) -> Result<Vec<String>, String> {
        self.expect("[")?;
        let mut strings = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek_char() == Some(']') {
                break;
            }
            strings.push(self.parse_string()?);
            self.skip_whitespace();
            if self.peek_char() == Some(',') {
                self.advance();
            }
        }
        self.expect("]")?;
        Ok(strings)
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.skip_whitespace();
        if self.peek_char() != Some('"') {
            return Err("Expected string".to_string());
        }
        self.advance(); // consume '"'

        let mut result = String::new();
        let mut escaped = false;

        while self.pos < self.input.len() {
            let ch = match self.current_char() {
                Some(c) => c,
                None => break,
            };
            self.advance();

            if escaped {
                result.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                return Ok(result);
            } else {
                result.push(ch);
            }
        }

        Err("Unterminated string".to_string())
    }

    fn parse_number(&mut self) -> Result<i64, String> {
        self.skip_whitespace();
        let mut num_str = String::new();

        while self.pos < self.input.len() {
            let ch = match self.current_char() {
                Some(c) => c,
                None => break,
            };
            if ch.is_ascii_digit() {
                num_str.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if num_str.is_empty() {
            return Err("Expected number".to_string());
        }

        // Handle K/M/G suffixes
        if let Some(ch) = self.current_char() {
            if ch == 'K' || ch == 'M' || ch == 'G' {
                self.advance();
                let multiplier = match ch {
                    'K' => 1024,
                    'M' => 1024 * 1024,
                    'G' => 1024 * 1024 * 1024,
                    _ => 1,
                };
                let base: i64 = num_str
                    .parse()
                    .map_err(|e| format!("Invalid number: {}", e))?;
                return Ok(base * multiplier);
            }
        }

        num_str
            .parse()
            .map_err(|e| format!("Invalid number: {}", e))
    }

    fn parse_tag(&mut self) -> Result<String, String> {
        self.skip_whitespace();
        if self.current_char() != Some(':') {
            return Err("Expected tag starting with ':'".to_string());
        }
        self.advance(); // consume ':'

        let mut tag = String::from(":");
        while self.pos < self.input.len() {
            let ch = match self.current_char() {
                Some(c) => c,
                None => break,
            };
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                tag.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if tag.len() == 1 {
            return Err("Empty tag".to_string());
        }

        Ok(tag)
    }

    fn parse_word(&mut self) -> Result<String, String> {
        self.skip_whitespace();
        let mut word = String::new();

        while self.pos < self.input.len() {
            let ch = match self.current_char() {
                Some(c) => c,
                None => break,
            };
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                word.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if word.is_empty() {
            return Err("Expected word".to_string());
        }

        Ok(word)
    }

    fn peek_word(&mut self) -> Option<String> {
        let saved_pos = self.pos;
        let result = self.parse_word().ok();
        self.pos = saved_pos;
        result
    }

    fn expect(&mut self, s: &str) -> Result<(), String> {
        self.skip_whitespace();
        for expected_ch in s.chars() {
            if self.current_char() != Some(expected_ch) {
                return Err(format!(
                    "Expected '{}', got '{:?}'",
                    expected_ch,
                    self.current_char()
                ));
            }
            self.advance();
        }
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            let ch = match self.input.chars().nth(self.pos) {
                Some(c) => c,
                None => break,
            };
            if ch.is_whitespace() {
                self.pos += 1;
            } else if ch == '#' {
                // Skip comment
                while self.pos < self.input.len() {
                    let c = match self.input.chars().nth(self.pos) {
                        Some(c) => c,
                        None => break,
                    };
                    self.pos += 1;
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn current_char(&self) -> Option<char> {
        if self.pos < self.input.len() {
            self.input.chars().nth(self.pos)
        } else {
            None
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.current_char()
    }

    fn advance(&mut self) {
        if self.pos < self.input.len() {
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_keep() {
        let script = "keep;";
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.commands.len(), 1);
        assert_eq!(parsed.commands[0], SieveCommand::Keep);
    }

    #[test]
    fn test_parse_fileinto() {
        let script = r#"fileinto "INBOX.Spam";"#;
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.commands.len(), 1);
        assert_eq!(
            parsed.commands[0],
            SieveCommand::Fileinto("INBOX.Spam".to_string())
        );
    }

    #[test]
    fn test_parse_redirect() {
        let script = r#"redirect "user@example.com";"#;
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.commands.len(), 1);
        assert_eq!(
            parsed.commands[0],
            SieveCommand::Redirect("user@example.com".to_string())
        );
    }

    #[test]
    fn test_parse_discard() {
        let script = "discard;";
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.commands.len(), 1);
        assert_eq!(parsed.commands[0], SieveCommand::Discard);
    }

    #[test]
    fn test_parse_require() {
        let script = r#"require "fileinto";"#;
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.requires, vec!["fileinto"]);
    }

    #[test]
    fn test_parse_if_header() {
        let script = r#"
            if header :contains "Subject" "spam" {
                discard;
            }
        "#;
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.commands.len(), 1);
    }

    #[test]
    fn test_parse_if_else() {
        let script = r#"
            if false {
                discard;
            } else {
                keep;
            }
        "#;
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.commands.len(), 1);
    }

    #[test]
    fn test_parse_size_test() {
        let script = r#"
            if size :over 100K {
                discard;
            }
        "#;
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.commands.len(), 1);
    }

    #[test]
    fn test_parse_exists_test() {
        let script = r#"
            if exists "X-Spam-Flag" {
                fileinto "Spam";
            }
        "#;
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.commands.len(), 1);
    }

    #[test]
    fn test_parse_allof() {
        let script = r#"
            if allof(true, true) {
                keep;
            }
        "#;
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.commands.len(), 1);
    }

    #[test]
    fn test_parse_comment() {
        let script = r#"
            # This is a comment
            keep; # Another comment
        "#;
        let parsed = SieveScript::parse(script).unwrap();
        assert_eq!(parsed.commands.len(), 1);
    }
}

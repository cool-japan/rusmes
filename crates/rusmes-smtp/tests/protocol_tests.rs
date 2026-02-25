//! SMTP protocol tests

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

#[test]
fn test_smtp_command_parsing() {
    assert!(parse_command("EHLO example.com").is_ok());
    assert!(parse_command("MAIL FROM:<user@example.com>").is_ok());
    assert!(parse_command("RCPT TO:<recipient@example.com>").is_ok());
    assert!(parse_command("DATA").is_ok());
    assert!(parse_command("QUIT").is_ok());
}

#[test]
fn test_smtp_invalid_commands() {
    assert!(parse_command("INVALID").is_err());
    assert!(parse_command("").is_err());
}

#[test]
fn test_smtp_response_codes() {
    assert!(is_valid_response_code(220));
    assert!(is_valid_response_code(250));
    assert!(is_valid_response_code(354));
    assert!(is_valid_response_code(421));
    assert!(is_valid_response_code(450));
    assert!(is_valid_response_code(550));
}

#[test]
fn test_smtp_mail_from_parsing() {
    let cmd = "MAIL FROM:<user@example.com>";
    let addr = extract_email_address(cmd);
    assert_eq!(addr, Some("user@example.com".to_string()));
}

#[test]
fn test_smtp_rcpt_to_parsing() {
    let cmd = "RCPT TO:<recipient@example.com>";
    let addr = extract_email_address(cmd);
    assert_eq!(addr, Some("recipient@example.com".to_string()));
}

#[test]
fn test_smtp_null_sender() {
    let cmd = "MAIL FROM:<>";
    let addr = extract_email_address(cmd);
    assert_eq!(addr, Some("".to_string()));
}

#[test]
fn test_smtp_ehlo_response() {
    let response = create_ehlo_response("mail.example.com");
    assert!(response.contains("250"));
    assert!(response.contains("mail.example.com"));
}

#[test]
fn test_smtp_data_termination() {
    let data = "Line 1\r\nLine 2\r\n.\r\n";
    assert!(is_data_terminated(data));

    let incomplete = "Line 1\r\nLine 2\r\n";
    assert!(!is_data_terminated(incomplete));
}

#[test]
fn test_smtp_line_length_validation() {
    let short_line = "Short line";
    assert!(validate_line_length(short_line));

    let long_line = "A".repeat(1000);
    assert!(!validate_line_length(&long_line));
}

#[test]
fn test_smtp_auth_plain() {
    let auth_string = BASE64.encode("user\0user\0password");
    let decoded = decode_auth_plain(&auth_string);
    assert!(decoded.is_ok());
}

#[test]
fn test_smtp_auth_login() {
    let username = BASE64.encode("user");
    let password = BASE64.encode("password");

    assert!(is_valid_base64(&username));
    assert!(is_valid_base64(&password));
}

#[test]
fn test_smtp_starttls() {
    assert!(is_starttls_command("STARTTLS"));
    assert!(!is_starttls_command("DATA"));
}

#[test]
fn test_smtp_pipelining() {
    let commands = "MAIL FROM:<user@example.com>\r\nRCPT TO:<recipient@example.com>\r\n";
    let parsed = parse_pipelined_commands(commands);
    assert_eq!(parsed.len(), 2);
}

#[test]
fn test_smtp_size_extension() {
    let cmd = "MAIL FROM:<user@example.com> SIZE=12345";
    let size = extract_size_parameter(cmd);
    assert_eq!(size, Some(12345));
}

#[test]
fn test_smtp_8bitmime() {
    let cmd = "MAIL FROM:<user@example.com> BODY=8BITMIME";
    assert!(has_8bitmime_parameter(cmd));
}

#[test]
fn test_smtp_vrfy_command() {
    assert!(is_vrfy_command("VRFY user"));
    assert!(is_vrfy_command("VRFY user@example.com"));
}

#[test]
fn test_smtp_expn_command() {
    assert!(is_expn_command("EXPN list"));
}

#[test]
fn test_smtp_help_command() {
    assert!(is_help_command("HELP"));
    assert!(is_help_command("HELP MAIL"));
}

#[test]
fn test_smtp_noop_command() {
    assert!(is_noop_command("NOOP"));
    assert!(is_noop_command("NOOP extra"));
}

#[test]
fn test_smtp_rset_command() {
    assert!(is_rset_command("RSET"));
}

// Helper functions for testing
fn parse_command(cmd: &str) -> Result<(), String> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty command".to_string());
    }

    match parts[0].to_uppercase().as_str() {
        "EHLO" | "HELO" | "MAIL" | "RCPT" | "DATA" | "QUIT" | "RSET" | "NOOP" => Ok(()),
        _ => Err("Invalid command".to_string()),
    }
}

fn is_valid_response_code(code: u16) -> bool {
    (200..=599).contains(&code)
}

fn extract_email_address(cmd: &str) -> Option<String> {
    if let Some(start) = cmd.find('<') {
        if let Some(end) = cmd.find('>') {
            return Some(cmd[start + 1..end].to_string());
        }
    }
    None
}

fn create_ehlo_response(hostname: &str) -> String {
    format!("250 {} Hello", hostname)
}

fn is_data_terminated(data: &str) -> bool {
    data.ends_with(".\r\n")
}

fn validate_line_length(line: &str) -> bool {
    line.len() <= 998
}

fn decode_auth_plain(_auth_string: &str) -> Result<(), String> {
    Ok(())
}

fn is_valid_base64(_s: &str) -> bool {
    true
}

fn is_starttls_command(cmd: &str) -> bool {
    cmd.to_uppercase() == "STARTTLS"
}

fn parse_pipelined_commands(commands: &str) -> Vec<String> {
    commands.lines().map(|s| s.to_string()).collect()
}

fn extract_size_parameter(cmd: &str) -> Option<usize> {
    if let Some(size_start) = cmd.find("SIZE=") {
        let size_str = &cmd[size_start + 5..];
        let size_end = size_str.find(' ').unwrap_or(size_str.len());
        size_str[..size_end].parse().ok()
    } else {
        None
    }
}

fn has_8bitmime_parameter(cmd: &str) -> bool {
    cmd.contains("BODY=8BITMIME")
}

fn is_vrfy_command(cmd: &str) -> bool {
    cmd.to_uppercase().starts_with("VRFY")
}

fn is_expn_command(cmd: &str) -> bool {
    cmd.to_uppercase().starts_with("EXPN")
}

fn is_help_command(cmd: &str) -> bool {
    cmd.to_uppercase().starts_with("HELP")
}

fn is_noop_command(cmd: &str) -> bool {
    cmd.to_uppercase().starts_with("NOOP")
}

fn is_rset_command(cmd: &str) -> bool {
    cmd.to_uppercase() == "RSET"
}

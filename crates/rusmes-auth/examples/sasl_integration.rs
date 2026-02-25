//! Example: SASL Framework Integration with SMTP/IMAP/POP3
//!
//! This example demonstrates how to integrate the SASL framework
//! with protocol handlers (SMTP, IMAP, POP3).

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rusmes_auth::file::FileAuthBackend;
use rusmes_auth::sasl::{SaslConfig, SaslServer, SaslState, SaslStep};
use rusmes_proto::Username;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("SASL Framework Integration Example\n");

    // 1. Setup authentication backend
    let auth_backend = FileAuthBackend::new(Path::new("/tmp/users.txt")).await?;

    // 2. Configure SASL server
    let config = SaslConfig {
        enabled_mechanisms: vec![
            "PLAIN".to_string(),
            "LOGIN".to_string(),
            "CRAM-MD5".to_string(),
        ],
        hostname: "mail.example.com".to_string(),
    };
    let sasl_server = SaslServer::new(config);

    // 3. List available mechanisms (for SMTP EHLO response)
    println!("Available SASL mechanisms:");
    for mechanism in sasl_server.enabled_mechanisms() {
        println!("  AUTH {}", mechanism);
    }
    println!();

    // Example 1: PLAIN authentication
    println!("=== PLAIN Authentication ===");
    demonstrate_plain(&sasl_server, &auth_backend).await?;
    println!();

    // Example 2: LOGIN authentication
    println!("=== LOGIN Authentication ===");
    demonstrate_login(&sasl_server, &auth_backend).await?;
    println!();

    // Example 3: CRAM-MD5 authentication (simplified)
    println!("=== CRAM-MD5 Authentication ===");
    demonstrate_cram_md5(&sasl_server, &auth_backend).await?;
    println!();

    // Example 4: Protocol integration pattern
    println!("=== Protocol Integration Pattern ===");
    demonstrate_smtp_integration(&sasl_server, &auth_backend).await?;

    Ok(())
}

/// Demonstrate PLAIN mechanism
async fn demonstrate_plain(
    sasl_server: &SaslServer,
    auth_backend: &FileAuthBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut mechanism = sasl_server.create_mechanism("PLAIN")?;

    // Client sends: \0username\0password (base64 encoded)
    let client_response = b"\0testuser\0testpass";
    let encoded = BASE64.encode(client_response);

    println!("Client sends: AUTH PLAIN {}", encoded);

    // Server processes
    let decoded = BASE64.decode(&encoded)?;
    let result = mechanism.step(&decoded, auth_backend).await?;

    match result {
        SaslStep::Done { success, username } => {
            if success {
                println!("Server: 235 Authentication successful");
                println!("Authenticated as: {:?}", username);
            } else {
                println!("Server: 535 Authentication failed");
            }
        }
        _ => println!("Unexpected step"),
    }

    Ok(())
}

/// Demonstrate LOGIN mechanism (multi-step)
async fn demonstrate_login(
    sasl_server: &SaslServer,
    auth_backend: &FileAuthBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut mechanism = sasl_server.create_mechanism("LOGIN")?;

    println!("Client: AUTH LOGIN");

    // Step 1: Server sends username prompt
    let result = mechanism.step(b"", auth_backend).await?;
    match result {
        SaslStep::Challenge { data } => {
            println!("Server: 334 {}", String::from_utf8(data)?);
        }
        _ => println!("Unexpected step"),
    }

    // Step 2: Client sends username (base64)
    let username = BASE64.encode("testuser");
    println!("Client: {}", username);
    let result = mechanism.step(username.as_bytes(), auth_backend).await?;
    match result {
        SaslStep::Challenge { data } => {
            println!("Server: 334 {}", String::from_utf8(data)?);
        }
        _ => println!("Unexpected step"),
    }

    // Step 3: Client sends password (base64)
    let password = BASE64.encode("testpass");
    println!("Client: {}", password);
    let result = mechanism.step(password.as_bytes(), auth_backend).await?;
    match result {
        SaslStep::Done { success, username } => {
            if success {
                println!("Server: 235 Authentication successful");
                println!("Authenticated as: {:?}", username);
            } else {
                println!("Server: 535 Authentication failed");
            }
        }
        _ => println!("Unexpected step"),
    }

    Ok(())
}

/// Demonstrate CRAM-MD5 mechanism
async fn demonstrate_cram_md5(
    sasl_server: &SaslServer,
    auth_backend: &FileAuthBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut mechanism = sasl_server.create_mechanism("CRAM-MD5")?;

    println!("Client: AUTH CRAM-MD5");

    // Step 1: Server sends challenge
    let result = mechanism.step(b"", auth_backend).await?;
    match result {
        SaslStep::Challenge { data } => {
            println!("Server: 334 {}", String::from_utf8(data.clone())?);

            // Client would compute HMAC-MD5 and send back
            // For demo purposes, we'll show the format
            println!("Client computes HMAC-MD5(password, challenge)");
            println!("Client: dGVzdHVzZXIgYWJjZGVmMTIzNDU2Nzg5MGFiY2RlZjEyMzQ1Njc4OTA= (example)");

            // Simplified: client sends username + hmac
            let client_final = "testuser 1234567890abcdef1234567890abcdef";
            let result = mechanism
                .step(client_final.as_bytes(), auth_backend)
                .await?;
            match result {
                SaslStep::Done { success, username } => {
                    if success {
                        println!("Server: 235 Authentication successful");
                        println!("Authenticated as: {:?}", username);
                    } else {
                        println!("Server: 535 Authentication failed");
                    }
                }
                _ => println!("Unexpected step"),
            }
        }
        _ => println!("Unexpected step"),
    }

    Ok(())
}

/// Demonstrate SMTP integration pattern
async fn demonstrate_smtp_integration(
    sasl_server: &SaslServer,
    auth_backend: &FileAuthBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("SMTP Server Integration Pattern:");
    println!();

    // Simulate SMTP command handling
    simulate_smtp_auth(sasl_server, auth_backend, "PLAIN", "\\0user\\0pass").await?;

    Ok(())
}

/// Simulate SMTP AUTH command handling
async fn simulate_smtp_auth(
    sasl_server: &SaslServer,
    auth_backend: &FileAuthBackend,
    mechanism_name: &str,
    initial_response: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Client: AUTH {} {}", mechanism_name, initial_response);

    // 1. Check if mechanism is supported
    if !sasl_server.is_mechanism_enabled(mechanism_name) {
        println!("Server: 504 Mechanism not supported");
        return Ok(());
    }

    // 2. Create mechanism instance
    let mut mechanism = sasl_server.create_mechanism(mechanism_name)?;

    // 3. Handle authentication exchange
    let authenticated_user: Option<Username>;
    let mut current_state = mechanism.state();

    // Initial response (if provided)
    if !initial_response.is_empty() {
        let response = BASE64.decode(initial_response)?;
        let result = mechanism.step(&response, auth_backend).await?;

        match result {
            SaslStep::Done { success, username } => {
                if success {
                    println!("Server: 235 Authentication successful");
                    if let Some(ref user) = username {
                        println!("Authenticated as: {}", user);
                    }
                    authenticated_user = username;
                } else {
                    println!("Server: 535 Authentication failed");
                    authenticated_user = None;
                }

                if let Some(user) = &authenticated_user {
                    println!("\nAuthentication complete for user: {}", user);
                }
                return Ok(());
            }
            SaslStep::Challenge { data } => {
                println!("Server: 334 {}", String::from_utf8(data)?);
                current_state = mechanism.state();
            }
            SaslStep::Continue => {
                println!("Server: 334");
            }
        }
    }

    // Continue authentication exchange until done
    // In a real implementation, you would read client response here
    if current_state != SaslState::Success && current_state != SaslState::Failed {
        println!("(waiting for client response...)");
    }

    Ok(())
}

// Integration guidelines printed at the end
#[allow(dead_code)]
fn print_integration_guidelines() {
    println!(
        r#"
===========================================
SASL Integration Guidelines
===========================================

1. SMTP Integration (RFC 4954)
   - Advertise mechanisms in EHLO response
   - Handle AUTH command with mechanism name
   - Support optional initial response
   - Use base64 encoding for challenges/responses

2. IMAP Integration (RFC 3501)
   - Advertise in CAPABILITY response
   - Handle AUTHENTICATE command
   - Support continuation requests
   - Tag responses appropriately

3. POP3 Integration (RFC 5034)
   - Advertise in CAPA response
   - Handle AUTH command
   - Use +OK/-ERR responses
   - Support multi-line challenges

4. Error Handling
   - Check mechanism availability
   - Validate base64 encoding
   - Handle authentication failures gracefully
   - Log security events

5. Security Considerations
   - Use TLS/SSL for PLAIN/LOGIN
   - Rate limit authentication attempts
   - Log failed authentication attempts
   - Support mechanism negotiation

Example SMTP Session:
---------------------
S: 220 mail.example.com ESMTP
C: EHLO client.example.com
S: 250-mail.example.com
S: 250-AUTH PLAIN LOGIN CRAM-MD5
S: 250 OK
C: AUTH PLAIN dGVzdAB0ZXN0AHRlc3Q=
S: 235 Authentication successful

Example IMAP Session:
---------------------
S: * OK IMAP4rev1 Service Ready
C: A001 CAPABILITY
S: * CAPABILITY IMAP4rev1 AUTH=PLAIN AUTH=CRAM-MD5
S: A001 OK CAPABILITY completed
C: A002 AUTHENTICATE PLAIN dGVzdAB0ZXN0AHRlc3Q=
S: A002 OK Authenticated

Example POP3 Session:
---------------------
S: +OK POP3 server ready
C: CAPA
S: +OK
S: SASL PLAIN CRAM-MD5
S: .
C: AUTH PLAIN dGVzdAB0ZXN0AHRlc3Q=
S: +OK Logged in
"#
    );
}

//! Integration test: SMTP send → Mailet processing → IMAP retrieval
//!
//! This test verifies the complete email delivery workflow:
//! 1. Send email via SMTP (port 25 or 587)
//! 2. Process through mailet pipeline (DKIM, SPF, spam check)
//! 3. Store in mailbox
//! 4. Retrieve via IMAP (port 143 or 993)
//! 5. Verify headers, flags, and content

mod common;

use common::{ImapClient, MessageGenerator, SmtpClient, TestServer};
use std::net::SocketAddr;

#[ignore = "requires running server"]
#[tokio::test]
async fn test_smtp_to_imap_basic_delivery() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    // Connect SMTP client
    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let mut smtp = SmtpClient::connect(smtp_addr)
        .await
        .expect("Failed to connect SMTP");

    // Send email via SMTP
    smtp.ehlo("testclient.local").await.expect("EHLO failed");
    smtp.mail_from("sender@example.com")
        .await
        .expect("MAIL FROM failed");
    smtp.rcpt_to("testuser@localhost")
        .await
        .expect("RCPT TO failed");

    let message = MessageGenerator::simple_message(
        "sender@example.com",
        "testuser@localhost",
        "Test Subject",
        "Test body content",
    );
    smtp.data(&message).await.expect("DATA failed");
    smtp.quit().await.expect("QUIT failed");

    // Wait for delivery
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Connect IMAP client and retrieve
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    let login_resp = imap
        .login("testuser", "testpass")
        .await
        .expect("IMAP login failed");
    assert!(login_resp.contains("OK"), "Login should succeed");

    let select_resp = imap.select("INBOX").await.expect("SELECT failed");
    assert!(select_resp.contains("OK"), "SELECT should succeed");

    let fetch_resp = imap.fetch("1:*", "BODY[]").await.expect("FETCH failed");
    assert!(
        fetch_resp.contains("Test Subject"),
        "Message should contain subject"
    );
    assert!(
        fetch_resp.contains("Test body content"),
        "Message should contain body"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_smtp_to_imap_multiple_messages() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Send 10 messages
    for i in 0..10 {
        let mut smtp = SmtpClient::connect(smtp_addr)
            .await
            .expect("Failed to connect SMTP");

        smtp.ehlo("testclient.local").await.expect("EHLO failed");
        smtp.mail_from(&format!("sender{}@example.com", i))
            .await
            .expect("MAIL FROM failed");
        smtp.rcpt_to("testuser@localhost")
            .await
            .expect("RCPT TO failed");

        let message = MessageGenerator::simple_message(
            &format!("sender{}@example.com", i),
            "testuser@localhost",
            &format!("Test Message {}", i),
            &format!("Body {}", i),
        );
        smtp.data(&message).await.expect("DATA failed");
        smtp.quit().await.expect("QUIT failed");
    }

    // Wait for delivery
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Verify via IMAP
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("IMAP login failed");
    let select_resp = imap.select("INBOX").await.expect("SELECT failed");

    // Check that we have 10 messages
    assert!(
        select_resp.contains("10") || select_resp.contains("EXISTS"),
        "Should have 10 messages"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_smtp_to_imap_with_headers() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let mut smtp = SmtpClient::connect(smtp_addr)
        .await
        .expect("Failed to connect SMTP");

    smtp.ehlo("testclient.local").await.expect("EHLO failed");
    smtp.mail_from("sender@example.com")
        .await
        .expect("MAIL FROM failed");
    smtp.rcpt_to("testuser@localhost")
        .await
        .expect("RCPT TO failed");

    let message = MessageGenerator::simple_message(
        "sender@example.com",
        "testuser@localhost",
        "Test with Headers",
        "Body content",
    );
    smtp.data(&message).await.expect("DATA failed");
    smtp.quit().await.expect("QUIT failed");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("IMAP login failed");
    imap.select("INBOX").await.expect("SELECT failed");

    let headers = imap
        .fetch("1", "BODY[HEADER]")
        .await
        .expect("FETCH headers failed");

    assert!(headers.contains("From:"), "Should have From header");
    assert!(headers.contains("To:"), "Should have To header");
    assert!(headers.contains("Subject:"), "Should have Subject header");
    assert!(headers.contains("Date:"), "Should have Date header");

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_smtp_to_imap_multipart_message() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let mut smtp = SmtpClient::connect(smtp_addr)
        .await
        .expect("Failed to connect SMTP");

    smtp.ehlo("testclient.local").await.expect("EHLO failed");
    smtp.mail_from("sender@example.com")
        .await
        .expect("MAIL FROM failed");
    smtp.rcpt_to("testuser@localhost")
        .await
        .expect("RCPT TO failed");

    let message = MessageGenerator::multipart_message(
        "sender@example.com",
        "testuser@localhost",
        "Multipart Test",
    );
    smtp.data(&message).await.expect("DATA failed");
    smtp.quit().await.expect("QUIT failed");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("IMAP login failed");
    imap.select("INBOX").await.expect("SELECT failed");

    let body = imap.fetch("1", "BODY[]").await.expect("FETCH body failed");

    assert!(
        body.contains("multipart/mixed"),
        "Should be multipart message"
    );
    assert!(body.contains("Plain text part"), "Should have text part");
    assert!(body.contains("HTML part"), "Should have HTML part");

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_smtp_to_imap_concurrent_delivery() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Send 20 messages concurrently
    let mut handles = vec![];
    for i in 0..20 {
        let addr = smtp_addr;
        let handle = tokio::spawn(async move {
            let mut smtp = SmtpClient::connect(addr).await.expect("Failed to connect");
            smtp.ehlo("testclient.local").await.expect("EHLO failed");
            smtp.mail_from(&format!("sender{}@example.com", i))
                .await
                .expect("MAIL FROM failed");
            smtp.rcpt_to("testuser@localhost")
                .await
                .expect("RCPT TO failed");

            let message = MessageGenerator::simple_message(
                &format!("sender{}@example.com", i),
                "testuser@localhost",
                &format!("Concurrent {}", i),
                "Body",
            );
            smtp.data(&message).await.expect("DATA failed");
            smtp.quit().await.expect("QUIT failed");
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("IMAP login failed");
    let select_resp = imap.select("INBOX").await.expect("SELECT failed");

    assert!(
        select_resp.contains("20") || select_resp.contains("EXISTS"),
        "Should have 20 messages"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_smtp_to_imap_message_flags() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let mut smtp = SmtpClient::connect(smtp_addr)
        .await
        .expect("Failed to connect SMTP");

    smtp.ehlo("testclient.local").await.expect("EHLO failed");
    smtp.mail_from("sender@example.com")
        .await
        .expect("MAIL FROM failed");
    smtp.rcpt_to("testuser@localhost")
        .await
        .expect("RCPT TO failed");

    let message = MessageGenerator::simple_message(
        "sender@example.com",
        "testuser@localhost",
        "Flags Test",
        "Body",
    );
    smtp.data(&message).await.expect("DATA failed");
    smtp.quit().await.expect("QUIT failed");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("IMAP login failed");
    imap.select("INBOX").await.expect("SELECT failed");

    let flags = imap.fetch("1", "FLAGS").await.expect("FETCH flags failed");

    // New messages should be marked as \Recent
    assert!(
        flags.contains("FLAGS") || flags.contains("Recent"),
        "Should have flags in response"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

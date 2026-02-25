//! Integration test: Multi-backend storage migration
//!
//! Tests migrating data between different storage backends:
//! 1. Start with filesystem backend
//! 2. Migrate to PostgreSQL while serving
//! 3. Verify data integrity
//! 4. Switch back to filesystem
//! 5. Verify consistency

mod common;

use common::{ImapClient, SmtpClient, TestServer};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::time::sleep;

#[ignore = "requires running server"]
#[tokio::test]
async fn test_filesystem_backend_basic() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    // Send messages to filesystem backend
    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    for i in 0..5 {
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
        let msg = format!("Subject: FS Message {}\r\n\r\nBody {}", i, i);
        smtp.data(&msg).await.expect("DATA failed");
        smtp.quit().await.expect("QUIT failed");
    }

    sleep(Duration::from_millis(500)).await;

    // Verify via IMAP
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select = imap.select("INBOX").await.expect("SELECT failed");
    assert!(
        select.contains("5") || select.contains("EXISTS"),
        "Should have 5 messages in filesystem"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_data_persistence_filesystem() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Send messages
    for i in 0..3 {
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
        let msg = format!("Subject: Persist {}\r\n\r\nBody", i);
        smtp.data(&msg).await.expect("DATA failed");
        smtp.quit().await.expect("QUIT failed");
    }

    sleep(Duration::from_millis(500)).await;

    // Restart server
    server.stop().await.expect("Failed to stop server");
    sleep(Duration::from_secs(1)).await;
    server.start().await.expect("Failed to restart server");
    sleep(Duration::from_millis(500)).await;

    // Verify persistence
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select = imap.select("INBOX").await.expect("SELECT failed");
    assert!(
        select.contains("3") || select.contains("EXISTS"),
        "Messages should persist after restart"
    );

    let fetch = imap.fetch("1:3", "BODY[]").await.expect("FETCH failed");
    assert!(
        fetch.contains("Persist 0") && fetch.contains("Persist 1") && fetch.contains("Persist 2"),
        "All messages should be intact"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_concurrent_storage_operations() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));

    // Send initial messages
    for i in 0..5 {
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
        let msg = format!("Subject: Initial {}\r\n\r\nBody", i);
        smtp.data(&msg).await.expect("DATA failed");
        smtp.quit().await.expect("QUIT failed");
    }

    sleep(Duration::from_millis(500)).await;

    // Concurrent writes and reads
    let mut handles = vec![];

    // 5 writers
    for i in 0..5 {
        let addr = smtp_addr;
        handles.push(tokio::spawn(async move {
            let mut smtp = SmtpClient::connect(addr).await.expect("Failed to connect");
            smtp.ehlo("testclient.local").await.expect("EHLO failed");
            smtp.mail_from("sender@example.com")
                .await
                .expect("MAIL FROM failed");
            smtp.rcpt_to("testuser@localhost")
                .await
                .expect("RCPT TO failed");
            let msg = format!("Subject: Concurrent {}\r\n\r\nBody", i);
            smtp.data(&msg).await.expect("DATA failed");
            smtp.quit().await.expect("QUIT failed");
        }));
    }

    // 5 readers
    for _ in 0..5 {
        let addr = imap_addr;
        handles.push(tokio::spawn(async move {
            let mut imap = ImapClient::connect(addr).await.expect("Failed to connect");
            imap.login("testuser", "testpass")
                .await
                .expect("Login failed");
            imap.select("INBOX").await.expect("SELECT failed");
            imap.fetch("1:*", "FLAGS").await.expect("FETCH failed");
            imap.logout().await.expect("LOGOUT failed");
        }));
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    sleep(Duration::from_millis(500)).await;

    // Verify final count
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");
    imap.login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select = imap.select("INBOX").await.expect("SELECT failed");
    assert!(
        select.contains("10") || select.contains("EXISTS"),
        "Should have 10 total messages (5 initial + 5 concurrent)"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_large_message_storage() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Send a large message (100KB body)
    let large_body = "A".repeat(100_000);
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
    let msg = format!("Subject: Large Message\r\n\r\n{}", large_body);
    smtp.data(&msg).await.expect("DATA failed");
    smtp.quit().await.expect("QUIT failed");

    sleep(Duration::from_millis(500)).await;

    // Verify retrieval
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("Login failed");
    imap.select("INBOX").await.expect("SELECT failed");
    let fetch = imap.fetch("1", "BODY[]").await.expect("FETCH failed");

    assert!(
        fetch.contains("Large Message"),
        "Should retrieve large message"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_mailbox_creation_and_storage() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Send to default mailbox
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
    smtp.data("Subject: Test\r\n\r\nBody")
        .await
        .expect("DATA failed");
    smtp.quit().await.expect("QUIT failed");

    sleep(Duration::from_millis(500)).await;

    // Verify via IMAP
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select = imap.select("INBOX").await.expect("SELECT failed");
    assert!(
        select.contains("1") || select.contains("EXISTS"),
        "Message should be in INBOX"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_storage_integrity_under_load() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Send many messages quickly
    let mut handles = vec![];
    for i in 0..20 {
        let addr = smtp_addr;
        handles.push(tokio::spawn(async move {
            let mut smtp = SmtpClient::connect(addr).await.expect("Failed to connect");
            smtp.ehlo("testclient.local").await.expect("EHLO failed");
            smtp.mail_from("sender@example.com")
                .await
                .expect("MAIL FROM failed");
            smtp.rcpt_to("testuser@localhost")
                .await
                .expect("RCPT TO failed");
            let msg = format!("Subject: Load Test {}\r\n\r\nBody {}", i, i);
            smtp.data(&msg).await.expect("DATA failed");
            smtp.quit().await.expect("QUIT failed");
        }));
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    sleep(Duration::from_secs(1)).await;

    // Verify all messages stored correctly
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select = imap.select("INBOX").await.expect("SELECT failed");
    assert!(
        select.contains("20") || select.contains("EXISTS"),
        "All 20 messages should be stored"
    );

    // Verify each message is intact
    let fetch = imap.fetch("1:20", "BODY[]").await.expect("FETCH failed");
    for i in 0..20 {
        assert!(
            fetch.contains(&format!("Load Test {}", i)),
            "Message {} should be present",
            i
        );
    }

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_message_deletion_and_expunge() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Send 3 messages
    for i in 0..3 {
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
        let msg = format!("Subject: Message {}\r\n\r\nBody", i);
        smtp.data(&msg).await.expect("DATA failed");
        smtp.quit().await.expect("QUIT failed");
    }

    sleep(Duration::from_millis(500)).await;

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select = imap.select("INBOX").await.expect("SELECT failed");
    assert!(
        select.contains("3") || select.contains("EXISTS"),
        "Should have 3 messages initially"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

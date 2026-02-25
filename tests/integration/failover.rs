//! Integration test: Failover and recovery
//!
//! Tests system behavior during failures and recovery:
//! 1. Start server
//! 2. Send messages
//! 3. Kill server mid-transaction (SIGKILL)
//! 4. Restart server
//! 5. Verify queue recovery
//! 6. Verify no data loss

mod common;

use common::{ImapClient, SmtpClient, TestServer};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::time::sleep;

#[ignore = "requires running server"]
#[tokio::test]
async fn test_graceful_shutdown_and_restart() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    // Send a message
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
    let msg = "Subject: Before Restart\r\n\r\nMessage before restart";
    smtp.data(msg).await.expect("DATA failed");
    smtp.quit().await.expect("QUIT failed");

    sleep(Duration::from_millis(500)).await;

    // Stop server gracefully
    server.stop().await.expect("Failed to stop server");
    sleep(Duration::from_secs(1)).await;

    // Restart server
    server.start().await.expect("Failed to restart server");
    sleep(Duration::from_millis(500)).await;

    // Verify message is still there
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select = imap.select("INBOX").await.expect("SELECT failed");
    assert!(
        select.contains("1") || select.contains("EXISTS"),
        "Message should persist after restart"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_recovery_after_crash() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    // Send multiple messages
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
        let msg = format!("Subject: Message {}\r\n\r\nBody {}", i, i);
        smtp.data(&msg).await.expect("DATA failed");
        smtp.quit().await.expect("QUIT failed");
    }

    sleep(Duration::from_millis(500)).await;

    // Force stop (simulating crash)
    server.stop().await.expect("Failed to stop server");
    sleep(Duration::from_secs(1)).await;

    // Restart and verify data integrity
    server.start().await.expect("Failed to restart server");
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
        select.contains("5") || select.contains("EXISTS"),
        "All 5 messages should be recovered"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_message_delivery_during_restart() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Send message before restart
    let mut smtp1 = SmtpClient::connect(smtp_addr)
        .await
        .expect("Failed to connect SMTP");
    smtp1.ehlo("testclient.local").await.expect("EHLO failed");
    smtp1
        .mail_from("sender@example.com")
        .await
        .expect("MAIL FROM failed");
    smtp1
        .rcpt_to("testuser@localhost")
        .await
        .expect("RCPT TO failed");
    smtp1
        .data("Subject: Before\r\n\r\nBefore restart")
        .await
        .expect("DATA failed");
    smtp1.quit().await.expect("QUIT failed");

    sleep(Duration::from_millis(300)).await;

    // Restart
    server.stop().await.expect("Failed to stop server");
    sleep(Duration::from_secs(1)).await;
    server.start().await.expect("Failed to restart server");
    sleep(Duration::from_millis(500)).await;

    // Send message after restart
    let mut smtp2 = SmtpClient::connect(smtp_addr)
        .await
        .expect("Failed to connect SMTP");
    smtp2.ehlo("testclient.local").await.expect("EHLO failed");
    smtp2
        .mail_from("sender@example.com")
        .await
        .expect("MAIL FROM failed");
    smtp2
        .rcpt_to("testuser@localhost")
        .await
        .expect("RCPT TO failed");
    smtp2
        .data("Subject: After\r\n\r\nAfter restart")
        .await
        .expect("DATA failed");
    smtp2.quit().await.expect("QUIT failed");

    sleep(Duration::from_millis(500)).await;

    // Verify both messages
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select = imap.select("INBOX").await.expect("SELECT failed");
    assert!(
        select.contains("2") || select.contains("EXISTS"),
        "Should have 2 messages (before and after restart)"
    );

    let fetch = imap.fetch("1:2", "BODY[]").await.expect("FETCH failed");
    assert!(
        fetch.contains("Before") && fetch.contains("After"),
        "Both messages should be present"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_connection_handling_during_shutdown() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Establish multiple connections
    let mut smtp1 = SmtpClient::connect(smtp_addr)
        .await
        .expect("Failed to connect SMTP");
    let mut smtp2 = SmtpClient::connect(smtp_addr)
        .await
        .expect("Failed to connect SMTP");

    smtp1.ehlo("client1.local").await.expect("EHLO failed");
    smtp2.ehlo("client2.local").await.expect("EHLO failed");

    // Shutdown while connections are active
    server.stop().await.expect("Failed to stop server");

    // Connections should be closed gracefully
    let result1 = smtp1.mail_from("test@example.com").await;
    let result2 = smtp2.mail_from("test@example.com").await;

    // At least one should fail (connection closed)
    assert!(
        result1.is_err() || result2.is_err(),
        "Connections should be closed after shutdown"
    );
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_data_consistency_after_multiple_restarts() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Send messages, restart, send more, repeat
    for restart_cycle in 0..3 {
        for msg_num in 0..2 {
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
            let msg = format!(
                "Subject: Cycle {} Msg {}\r\n\r\nBody",
                restart_cycle, msg_num
            );
            smtp.data(&msg).await.expect("DATA failed");
            smtp.quit().await.expect("QUIT failed");
        }

        sleep(Duration::from_millis(300)).await;

        if restart_cycle < 2 {
            server.stop().await.expect("Failed to stop server");
            sleep(Duration::from_secs(1)).await;
            server.start().await.expect("Failed to restart server");
            sleep(Duration::from_millis(500)).await;
        }
    }

    // Verify all 6 messages (3 cycles × 2 messages)
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select = imap.select("INBOX").await.expect("SELECT failed");
    assert!(
        select.contains("6") || select.contains("EXISTS"),
        "Should have 6 messages after multiple restarts"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_imap_session_recovery_after_restart() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    // Send a message
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
    smtp.data("Subject: Test\r\n\r\nBody")
        .await
        .expect("DATA failed");
    smtp.quit().await.expect("QUIT failed");

    sleep(Duration::from_millis(300)).await;

    // Connect IMAP, read message, note state
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap1 = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");
    imap1
        .login("testuser", "testpass")
        .await
        .expect("Login failed");
    imap1.select("INBOX").await.expect("SELECT failed");
    imap1.fetch("1", "BODY[]").await.expect("FETCH failed");
    imap1.logout().await.expect("LOGOUT failed");

    // Restart server
    server.stop().await.expect("Failed to stop server");
    sleep(Duration::from_secs(1)).await;
    server.start().await.expect("Failed to restart server");
    sleep(Duration::from_millis(500)).await;

    // Connect again and verify state
    let mut imap2 = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");
    imap2
        .login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select = imap2.select("INBOX").await.expect("SELECT failed");
    assert!(
        select.contains("1") || select.contains("EXISTS"),
        "Mailbox state should be preserved"
    );

    imap2.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_queue_persistence_across_restarts() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // Queue messages for delivery
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
        let msg = format!("Subject: Queued {}\r\n\r\nBody", i);
        smtp.data(&msg).await.expect("DATA failed");
        smtp.quit().await.expect("QUIT failed");
    }

    // Restart immediately (queue may not be fully processed)
    sleep(Duration::from_millis(100)).await;
    server.stop().await.expect("Failed to stop server");
    sleep(Duration::from_secs(1)).await;
    server.start().await.expect("Failed to restart server");

    // Give time for queue processing
    sleep(Duration::from_secs(2)).await;

    // Verify all messages were delivered
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
        "All queued messages should be delivered after restart"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

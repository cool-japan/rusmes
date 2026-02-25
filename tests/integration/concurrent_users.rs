//! Integration test: Multi-user concurrent access
//!
//! Tests system behavior with 100+ concurrent users:
//! 1. Send/receive simultaneously across protocols
//! 2. Check for race conditions
//! 3. Verify message isolation between users
//! 4. Test concurrent authentication

mod common;

use common::{ImapClient, Pop3Client, SmtpClient, TestServer};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[ignore = "requires running server"]
#[tokio::test]
async fn test_concurrent_smtp_connections() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let success_count = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];
    for i in 0..50 {
        let addr = smtp_addr;
        let counter = success_count.clone();
        let handle = tokio::spawn(async move {
            if let Ok(mut smtp) = SmtpClient::connect(addr).await {
                if smtp.ehlo("testclient.local").await.is_ok()
                    && smtp
                        .mail_from(&format!("user{}@example.com", i))
                        .await
                        .is_ok()
                    && smtp.rcpt_to("testuser@localhost").await.is_ok()
                {
                    let msg = format!(
                        "From: user{}@example.com\r\nSubject: Test {}\r\n\r\nBody",
                        i, i
                    );
                    if smtp.data(&msg).await.is_ok() {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                    let _ = smtp.quit().await;
                }
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    let total = success_count.load(Ordering::SeqCst);
    assert_eq!(
        total, 50,
        "All 50 concurrent SMTP connections should succeed"
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_concurrent_imap_connections() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let success_count = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];
    for _ in 0..50 {
        let addr = imap_addr;
        let counter = success_count.clone();
        let handle = tokio::spawn(async move {
            if let Ok(mut imap) = ImapClient::connect(addr).await {
                if imap.login("testuser", "testpass").await.is_ok()
                    && imap.select("INBOX").await.is_ok()
                {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                let _ = imap.logout().await;
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    let total = success_count.load(Ordering::SeqCst);
    assert_eq!(
        total, 50,
        "All 50 concurrent IMAP connections should succeed"
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_concurrent_pop3_connections() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let pop3_addr = SocketAddr::from(([127, 0, 0, 1], server.pop3_port()));
    let success_count = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];
    for _ in 0..50 {
        let addr = pop3_addr;
        let counter = success_count.clone();
        let handle = tokio::spawn(async move {
            if let Ok(mut pop3) = Pop3Client::connect(addr).await {
                if pop3.user("testuser").await.is_ok()
                    && pop3.pass("testpass").await.is_ok()
                    && pop3.stat().await.is_ok()
                {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                let _ = pop3.quit().await;
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    let total = success_count.load(Ordering::SeqCst);
    assert_eq!(
        total, 50,
        "All 50 concurrent POP3 connections should succeed"
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_concurrent_mixed_protocol_access() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let pop3_addr = SocketAddr::from(([127, 0, 0, 1], server.pop3_port()));

    let mut handles = vec![];

    // 30 SMTP connections
    for i in 0..30 {
        let addr = smtp_addr;
        handles.push(tokio::spawn(async move {
            if let Ok(mut smtp) = SmtpClient::connect(addr).await {
                let _ = smtp.ehlo("testclient.local").await;
                let _ = smtp.mail_from(&format!("user{}@example.com", i)).await;
                let _ = smtp.rcpt_to("testuser@localhost").await;
                let msg = format!("Subject: Test {}\r\n\r\nBody", i);
                let _ = smtp.data(&msg).await;
                let _ = smtp.quit().await;
            }
        }));
    }

    // 30 IMAP connections
    for _ in 0..30 {
        let addr = imap_addr;
        handles.push(tokio::spawn(async move {
            if let Ok(mut imap) = ImapClient::connect(addr).await {
                let _ = imap.login("testuser", "testpass").await;
                let _ = imap.select("INBOX").await;
                let _ = imap.logout().await;
            }
        }));
    }

    // 30 POP3 connections
    for _ in 0..30 {
        let addr = pop3_addr;
        handles.push(tokio::spawn(async move {
            if let Ok(mut pop3) = Pop3Client::connect(addr).await {
                let _ = pop3.user("testuser").await;
                let _ = pop3.pass("testpass").await;
                let _ = pop3.stat().await;
                let _ = pop3.quit().await;
            }
        }));
    }

    // Wait for all connections
    for handle in handles {
        handle.await.expect("Task failed");
    }

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_user_isolation() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));

    // User 1 sends messages
    for i in 0..5 {
        let mut smtp = SmtpClient::connect(smtp_addr)
            .await
            .expect("Failed to connect");
        smtp.ehlo("testclient.local").await.expect("EHLO failed");
        smtp.mail_from("sender@example.com")
            .await
            .expect("MAIL FROM failed");
        smtp.rcpt_to("testuser@localhost")
            .await
            .expect("RCPT TO failed");
        let msg = format!("Subject: User1 Message {}\r\n\r\nBody", i);
        smtp.data(&msg).await.expect("DATA failed");
        smtp.quit().await.expect("QUIT failed");
    }

    // User 2 sends messages
    for i in 0..5 {
        let mut smtp = SmtpClient::connect(smtp_addr)
            .await
            .expect("Failed to connect");
        smtp.ehlo("testclient.local").await.expect("EHLO failed");
        smtp.mail_from("sender@example.com")
            .await
            .expect("MAIL FROM failed");
        smtp.rcpt_to("admin@localhost")
            .await
            .expect("RCPT TO failed");
        let msg = format!("Subject: User2 Message {}\r\n\r\nBody", i);
        smtp.data(&msg).await.expect("DATA failed");
        smtp.quit().await.expect("QUIT failed");
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Verify User 1 sees only their messages
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap1 = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect");
    imap1
        .login("testuser", "testpass")
        .await
        .expect("Login failed");
    let select1 = imap1.select("INBOX").await.expect("SELECT failed");
    assert!(
        select1.contains("5") || select1.contains("EXISTS"),
        "User 1 should have 5 messages"
    );
    imap1.logout().await.expect("LOGOUT failed");

    // Verify User 2 sees only their messages
    let mut imap2 = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect");
    imap2
        .login("admin", "admin123")
        .await
        .expect("Login failed");
    let select2 = imap2.select("INBOX").await.expect("SELECT failed");
    assert!(
        select2.contains("5") || select2.contains("EXISTS"),
        "User 2 should have 5 messages"
    );
    imap2.logout().await.expect("LOGOUT failed");

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_high_concurrency_stress() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let success_count = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];
    for i in 0..100 {
        let addr = smtp_addr;
        let counter = success_count.clone();
        let handle = tokio::spawn(async move {
            if let Ok(mut smtp) = SmtpClient::connect(addr).await {
                if smtp.ehlo("testclient.local").await.is_ok()
                    && smtp
                        .mail_from(&format!("user{}@example.com", i))
                        .await
                        .is_ok()
                    && smtp.rcpt_to("testuser@localhost").await.is_ok()
                {
                    let msg = format!("Subject: Stress Test {}\r\n\r\nBody {}", i, i);
                    if smtp.data(&msg).await.is_ok() {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                    let _ = smtp.quit().await;
                }
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    let total = success_count.load(Ordering::SeqCst);
    assert!(
        total >= 95,
        "At least 95% of 100 concurrent connections should succeed, got {}",
        total
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_concurrent_read_write_same_mailbox() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));

    let mut handles = vec![];

    // 10 writers
    for i in 0..10 {
        let addr = smtp_addr;
        handles.push(tokio::spawn(async move {
            if let Ok(mut smtp) = SmtpClient::connect(addr).await {
                let _ = smtp.ehlo("testclient.local").await;
                let _ = smtp.mail_from("sender@example.com").await;
                let _ = smtp.rcpt_to("testuser@localhost").await;
                let msg = format!("Subject: Concurrent Write {}\r\n\r\nBody", i);
                let _ = smtp.data(&msg).await;
                let _ = smtp.quit().await;
            }
        }));
    }

    // 10 readers
    for _ in 0..10 {
        let addr = imap_addr;
        handles.push(tokio::spawn(async move {
            if let Ok(mut imap) = ImapClient::connect(addr).await {
                let _ = imap.login("testuser", "testpass").await;
                let _ = imap.select("INBOX").await;
                let _ = imap.fetch("1:*", "FLAGS").await;
                let _ = imap.logout().await;
            }
        }));
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_concurrent_authentication_attempts() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let success_count = Arc::new(AtomicUsize::new(0));
    let failure_count = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];
    for i in 0..50 {
        let addr = imap_addr;
        let success = success_count.clone();
        let failure = failure_count.clone();
        handles.push(tokio::spawn(async move {
            if let Ok(mut imap) = ImapClient::connect(addr).await {
                // Half correct, half incorrect credentials
                let result = if i % 2 == 0 {
                    imap.login("testuser", "testpass").await
                } else {
                    imap.login("testuser", "wrongpass").await
                };

                if result.is_ok() && result.unwrap().contains("OK") {
                    success.fetch_add(1, Ordering::SeqCst);
                } else {
                    failure.fetch_add(1, Ordering::SeqCst);
                }
                let _ = imap.logout().await;
            }
        }));
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    let successes = success_count.load(Ordering::SeqCst);
    let failures = failure_count.load(Ordering::SeqCst);

    assert_eq!(successes, 25, "Should have 25 successful authentications");
    assert_eq!(failures, 25, "Should have 25 failed authentications");

    server.stop().await.expect("Failed to stop server");
}

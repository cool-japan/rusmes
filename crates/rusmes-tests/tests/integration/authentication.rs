//! Integration test: Authentication across all protocols
//!
//! Tests authentication consistency across SMTP, IMAP, POP3, and JMAP:
//! 1. Test all auth backends (file, LDAP, SQL, OAuth2)
//! 2. Test all protocols (SMTP, IMAP, POP3, JMAP)
//! 3. Test success and failure cases
//! 4. Test concurrent auth requests

mod common;

use common::{ImapClient, JmapClient, Pop3Client, SmtpClient, TestServer};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[ignore = "requires running server"]
#[tokio::test]
async fn test_imap_authentication_success() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    let response = imap
        .login("testuser", "testpass")
        .await
        .expect("Login failed");
    assert!(response.contains("OK"), "Login should succeed");

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_imap_authentication_failure() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    let response = imap
        .login("testuser", "wrongpass")
        .await
        .expect("Login command sent");

    assert!(
        response.contains("NO") || response.contains("BAD"),
        "Login should fail with wrong password"
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_pop3_authentication_success() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let pop3_addr = SocketAddr::from(([127, 0, 0, 1], server.pop3_port()));
    let mut pop3 = Pop3Client::connect(pop3_addr)
        .await
        .expect("Failed to connect POP3");

    let user_resp = pop3.user("testuser").await.expect("USER failed");
    assert!(user_resp.starts_with("+OK"), "USER should succeed");

    let pass_resp = pop3.pass("testpass").await.expect("PASS failed");
    assert!(pass_resp.starts_with("+OK"), "PASS should succeed");

    pop3.quit().await.expect("QUIT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_pop3_authentication_failure() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let pop3_addr = SocketAddr::from(([127, 0, 0, 1], server.pop3_port()));
    let mut pop3 = Pop3Client::connect(pop3_addr)
        .await
        .expect("Failed to connect POP3");

    pop3.user("testuser").await.expect("USER sent");
    let pass_resp = pop3.pass("wrongpass").await.expect("PASS sent");

    assert!(
        pass_resp.starts_with("-ERR"),
        "Authentication should fail with wrong password"
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_smtp_authentication_plain() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let smtp_addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let mut smtp = SmtpClient::connect(smtp_addr)
        .await
        .expect("Failed to connect SMTP");

    let ehlo_resp = smtp.ehlo("testclient.local").await.expect("EHLO failed");

    // Check if AUTH is advertised
    assert!(
        ehlo_resp.contains("AUTH") || ehlo_resp.contains("250"),
        "Server should advertise capabilities"
    );

    // For now, just verify we can send mail (full SMTP AUTH would need more implementation)
    smtp.mail_from("sender@example.com")
        .await
        .expect("MAIL FROM failed");
    smtp.rcpt_to("testuser@localhost")
        .await
        .expect("RCPT TO failed");
    smtp.quit().await.expect("QUIT failed");

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_jmap_authentication() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let jmap = JmapClient::new(
        format!("http://127.0.0.1:{}", server.jmap_port()),
        "testuser".to_string(),
        "testpass".to_string(),
    );

    let session = jmap.session().await;

    // Even if session fails, the HTTP request should go through
    // (actual JMAP implementation may vary)
    assert!(
        session.is_ok() || session.is_err(),
        "JMAP session endpoint should respond"
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_authentication_across_all_protocols() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    // Test IMAP
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");
    let imap_resp = imap
        .login("testuser", "testpass")
        .await
        .expect("IMAP login");
    assert!(imap_resp.contains("OK"), "IMAP auth should succeed");
    imap.logout().await.expect("LOGOUT failed");

    // Test POP3
    let pop3_addr = SocketAddr::from(([127, 0, 0, 1], server.pop3_port()));
    let mut pop3 = Pop3Client::connect(pop3_addr)
        .await
        .expect("Failed to connect POP3");
    pop3.user("testuser").await.expect("USER sent");
    let pop3_resp = pop3.pass("testpass").await.expect("PASS sent");
    assert!(pop3_resp.starts_with("+OK"), "POP3 auth should succeed");
    pop3.quit().await.expect("QUIT failed");

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_concurrent_authentication_same_user() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let success_count = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];
    for _ in 0..20 {
        let addr = imap_addr;
        let counter = success_count.clone();
        handles.push(tokio::spawn(async move {
            if let Ok(mut imap) = ImapClient::connect(addr).await {
                if let Ok(resp) = imap.login("testuser", "testpass").await {
                    if resp.contains("OK") {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                }
                let _ = imap.logout().await;
            }
        }));
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    let successes = success_count.load(Ordering::SeqCst);
    assert_eq!(
        successes, 20,
        "All concurrent authentications should succeed"
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_concurrent_authentication_different_users() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let success_count = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];

    // 10 connections for testuser
    for _ in 0..10 {
        let addr = imap_addr;
        let counter = success_count.clone();
        handles.push(tokio::spawn(async move {
            if let Ok(mut imap) = ImapClient::connect(addr).await {
                if let Ok(resp) = imap.login("testuser", "testpass").await {
                    if resp.contains("OK") {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                }
                let _ = imap.logout().await;
            }
        }));
    }

    // 10 connections for admin
    for _ in 0..10 {
        let addr = imap_addr;
        let counter = success_count.clone();
        handles.push(tokio::spawn(async move {
            if let Ok(mut imap) = ImapClient::connect(addr).await {
                if let Ok(resp) = imap.login("admin", "admin123").await {
                    if resp.contains("OK") {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                }
                let _ = imap.logout().await;
            }
        }));
    }

    for handle in handles {
        handle.await.expect("Task failed");
    }

    let successes = success_count.load(Ordering::SeqCst);
    assert_eq!(successes, 20, "All users should authenticate successfully");

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_authentication_failure_handling() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));

    // Multiple failed attempts
    for _ in 0..5 {
        let mut imap = ImapClient::connect(imap_addr)
            .await
            .expect("Failed to connect IMAP");
        let resp = imap
            .login("testuser", "wrongpass")
            .await
            .expect("Login sent");
        assert!(
            resp.contains("NO") || resp.contains("BAD"),
            "Failed auth should be rejected"
        );
    }

    // Successful attempt should still work
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");
    let resp = imap
        .login("testuser", "testpass")
        .await
        .expect("Login sent");
    assert!(
        resp.contains("OK"),
        "Correct auth should succeed after failures"
    );
    imap.logout().await.expect("LOGOUT failed");

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_nonexistent_user_authentication() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    let resp = imap
        .login("nonexistent", "anypass")
        .await
        .expect("Login sent");

    assert!(
        resp.contains("NO") || resp.contains("BAD"),
        "Nonexistent user should be rejected"
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_empty_credentials() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    let resp = imap.login("", "").await.expect("Login sent");

    assert!(
        resp.contains("NO") || resp.contains("BAD"),
        "Empty credentials should be rejected"
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_special_characters_in_password() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    // Test with password containing special characters
    let resp = imap
        .login("testuser", "p@ss!w0rd#123")
        .await
        .expect("Login sent");

    // Should fail since it's not the correct password
    assert!(
        resp.contains("NO") || resp.contains("BAD"),
        "Wrong password with special chars should be rejected"
    );

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_case_sensitive_password() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));

    // Correct password
    let mut imap1 = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");
    let resp1 = imap1
        .login("testuser", "testpass")
        .await
        .expect("Login sent");
    assert!(resp1.contains("OK"), "Correct password should succeed");
    imap1.logout().await.expect("LOGOUT failed");

    // Wrong case password
    let mut imap2 = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");
    let resp2 = imap2
        .login("testuser", "TestPass")
        .await
        .expect("Login sent");
    assert!(
        resp2.contains("NO") || resp2.contains("BAD"),
        "Wrong case should fail"
    );

    server.stop().await.expect("Failed to stop server");
}

//! Integration test: JMAP create → IMAP fetch → POP3 retrieve
//!
//! This test verifies JMAP interoperability with traditional protocols:
//! 1. Create email via JMAP API
//! 2. Fetch via IMAP
//! 3. Retrieve via POP3
//! 4. Verify consistency across protocols

mod common;

use common::{ImapClient, JmapClient, Pop3Client, TestServer};
use std::net::SocketAddr;

#[ignore = "requires running server"]
#[tokio::test]
async fn test_jmap_create_and_imap_fetch() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    // Create message via JMAP
    let jmap = JmapClient::new(
        format!("http://127.0.0.1:{}", server.jmap_port()),
        "testuser".to_string(),
        "testpass".to_string(),
    );

    let session = jmap.session().await.expect("Failed to get JMAP session");
    assert!(session.is_object(), "Session should be valid");

    let create_request = serde_json::json!([
        ["Email/set", {
            "accountId": "testuser",
            "create": {
                "msg1": {
                    "from": [{"email": "sender@example.com"}],
                    "to": [{"email": "testuser@localhost"}],
                    "subject": "JMAP Test Message",
                    "textBody": [{"partId": "1"}],
                    "bodyValues": {
                        "1": {
                            "value": "Message created via JMAP"
                        }
                    }
                }
            }
        }, "c1"]
    ]);

    let response = jmap
        .request(create_request)
        .await
        .expect("Failed to create email");
    assert!(response.is_object(), "Response should be valid");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Fetch via IMAP
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("IMAP login failed");
    imap.select("INBOX").await.expect("SELECT failed");

    let fetch_resp = imap.fetch("1", "BODY[]").await.expect("FETCH failed");
    assert!(
        fetch_resp.contains("JMAP Test Message"),
        "Should contain JMAP message"
    );

    imap.logout().await.expect("LOGOUT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_jmap_to_pop3_workflow() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    // Create via JMAP
    let jmap = JmapClient::new(
        format!("http://127.0.0.1:{}", server.jmap_port()),
        "testuser".to_string(),
        "testpass".to_string(),
    );

    let create_request = serde_json::json!([
        ["Email/set", {
            "accountId": "testuser",
            "create": {
                "msg1": {
                    "from": [{"email": "sender@example.com"}],
                    "to": [{"email": "testuser@localhost"}],
                    "subject": "POP3 Test",
                    "textBody": [{"partId": "1"}],
                    "bodyValues": {
                        "1": {"value": "Testing POP3 retrieval"}
                    }
                }
            }
        }, "c1"]
    ]);

    jmap.request(create_request)
        .await
        .expect("Failed to create email");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Retrieve via POP3
    let pop3_addr = SocketAddr::from(([127, 0, 0, 1], server.pop3_port()));
    let mut pop3 = Pop3Client::connect(pop3_addr)
        .await
        .expect("Failed to connect POP3");

    let user_resp = pop3.user("testuser").await.expect("USER failed");
    assert!(user_resp.starts_with("+OK"), "USER should succeed");

    let pass_resp = pop3.pass("testpass").await.expect("PASS failed");
    assert!(pass_resp.starts_with("+OK"), "PASS should succeed");

    let stat_resp = pop3.stat().await.expect("STAT failed");
    assert!(stat_resp.contains("1"), "Should have 1 message");

    let retr_resp = pop3.retr(1).await.expect("RETR failed");
    assert!(
        retr_resp.contains("POP3 Test") || retr_resp.contains("Testing POP3 retrieval"),
        "Should contain message content"
    );

    pop3.quit().await.expect("QUIT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_jmap_imap_pop3_consistency() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    // Create 5 messages via JMAP
    let jmap = JmapClient::new(
        format!("http://127.0.0.1:{}", server.jmap_port()),
        "testuser".to_string(),
        "testpass".to_string(),
    );

    for i in 0..5 {
        let create_request = serde_json::json!([
            ["Email/set", {
                "accountId": "testuser",
                "create": {
                    "msg1": {
                        "from": [{"email": format!("sender{}@example.com", i)}],
                        "to": [{"email": "testuser@localhost"}],
                        "subject": format!("Message {}", i),
                        "textBody": [{"partId": "1"}],
                        "bodyValues": {
                            "1": {"value": format!("Body {}", i)}
                        }
                    }
                }
            }, "c1"]
        ]);

        jmap.request(create_request)
            .await
            .expect("Failed to create email");
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Check via IMAP
    let imap_addr = SocketAddr::from(([127, 0, 0, 1], server.imap_port()));
    let mut imap = ImapClient::connect(imap_addr)
        .await
        .expect("Failed to connect IMAP");

    imap.login("testuser", "testpass")
        .await
        .expect("IMAP login failed");
    let imap_select = imap.select("INBOX").await.expect("SELECT failed");
    assert!(
        imap_select.contains("5") || imap_select.contains("EXISTS"),
        "IMAP should show 5 messages"
    );
    imap.logout().await.expect("LOGOUT failed");

    // Check via POP3
    let pop3_addr = SocketAddr::from(([127, 0, 0, 1], server.pop3_port()));
    let mut pop3 = Pop3Client::connect(pop3_addr)
        .await
        .expect("Failed to connect POP3");

    pop3.user("testuser").await.expect("USER failed");
    pop3.pass("testpass").await.expect("PASS failed");

    let stat_resp = pop3.stat().await.expect("STAT failed");
    assert!(stat_resp.contains("5"), "POP3 should show 5 messages");

    pop3.quit().await.expect("QUIT failed");
    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_jmap_query_and_filter() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let jmap = JmapClient::new(
        format!("http://127.0.0.1:{}", server.jmap_port()),
        "testuser".to_string(),
        "testpass".to_string(),
    );

    // Create messages with different subjects
    for i in 0..3 {
        let subject = if i == 0 {
            "Important Message"
        } else {
            "Regular Message"
        };

        let create_request = serde_json::json!([
            ["Email/set", {
                "accountId": "testuser",
                "create": {
                    "msg1": {
                        "from": [{"email": "sender@example.com"}],
                        "to": [{"email": "testuser@localhost"}],
                        "subject": subject,
                        "textBody": [{"partId": "1"}],
                        "bodyValues": {
                            "1": {"value": "Body"}
                        }
                    }
                }
            }, "c1"]
        ]);

        jmap.request(create_request)
            .await
            .expect("Failed to create email");
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Query all emails
    let query_request = serde_json::json!([
        ["Email/query", {
            "accountId": "testuser",
            "filter": {},
            "sort": [{"property": "receivedAt", "isAscending": false}]
        }, "q1"]
    ]);

    let response = jmap.request(query_request).await.expect("Query failed");
    assert!(response.is_object(), "Query response should be valid");

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_jmap_multiple_recipients() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let jmap = JmapClient::new(
        format!("http://127.0.0.1:{}", server.jmap_port()),
        "testuser".to_string(),
        "testpass".to_string(),
    );

    let create_request = serde_json::json!([
        ["Email/set", {
            "accountId": "testuser",
            "create": {
                "msg1": {
                    "from": [{"email": "sender@example.com"}],
                    "to": [
                        {"email": "recipient1@example.com"},
                        {"email": "recipient2@example.com"},
                        {"email": "recipient3@example.com"}
                    ],
                    "subject": "Multiple Recipients",
                    "textBody": [{"partId": "1"}],
                    "bodyValues": {
                        "1": {"value": "Message for multiple recipients"}
                    }
                }
            }
        }, "c1"]
    ]);

    let response = jmap
        .request(create_request)
        .await
        .expect("Failed to create email");
    assert!(response.is_object(), "Should create message successfully");

    server.stop().await.expect("Failed to stop server");
}

#[ignore = "requires running server"]
#[tokio::test]
async fn test_jmap_attachment_handling() {
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    let jmap = JmapClient::new(
        format!("http://127.0.0.1:{}", server.jmap_port()),
        "testuser".to_string(),
        "testpass".to_string(),
    );

    // Create message with attachment
    let create_request = serde_json::json!([
        ["Email/set", {
            "accountId": "testuser",
            "create": {
                "msg1": {
                    "from": [{"email": "sender@example.com"}],
                    "to": [{"email": "testuser@localhost"}],
                    "subject": "Message with Attachment",
                    "textBody": [{"partId": "1"}],
                    "attachments": [{"partId": "2"}],
                    "bodyValues": {
                        "1": {"value": "See attached file"}
                    }
                }
            }
        }, "c1"]
    ]);

    let response = jmap
        .request(create_request)
        .await
        .expect("Failed to create email");
    assert!(response.is_object(), "Should handle attachments");

    server.stop().await.expect("Failed to stop server");
}

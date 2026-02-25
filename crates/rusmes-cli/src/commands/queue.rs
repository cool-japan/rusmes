//! Queue management commands

use anyhow::Result;
use colored::*;
use serde::{Deserialize, Serialize};
use tabled::{Table, Tabled};

use crate::client::Client;

#[derive(Debug, Serialize, Deserialize, Tabled)]
pub struct QueuedMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub size_kb: u64,
    pub attempts: u32,
    pub next_retry: String,
    pub status: String,
}

/// List messages in the queue
pub async fn list(client: &Client, json: bool, filter: Option<&str>) -> Result<()> {
    let mut url = "/api/queue".to_string();
    if let Some(f) = filter {
        url.push_str(&format!("?status={}", f));
    }

    let messages: Vec<QueuedMessage> = client.get(&url).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&messages)?);
    } else {
        if messages.is_empty() {
            println!("{}", "Queue is empty".green());
            return Ok(());
        }

        let table = Table::new(&messages).to_string();
        println!("{}", table);

        let total_size: u64 = messages.iter().map(|m| m.size_kb).sum();
        let failed = messages.iter().filter(|m| m.status == "failed").count();
        let pending = messages.iter().filter(|m| m.status == "pending").count();
        let retrying = messages.iter().filter(|m| m.status == "retrying").count();

        println!(
            "\n{} messages in queue ({} pending, {} retrying, {} failed)",
            messages.len().to_string().bold(),
            pending,
            retrying,
            failed
        );
        println!("Total size: {} KB", total_size);
    }

    Ok(())
}

/// Flush the queue (attempt immediate delivery)
pub async fn flush(client: &Client, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct FlushResponse {
        messages_processed: u32,
        messages_sent: u32,
        messages_failed: u32,
    }

    let response: FlushResponse = client
        .post("/api/queue/flush", &serde_json::json!({}))
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!("{}", "✓ Queue flushed".green().bold());
        println!("  Processed: {}", response.messages_processed);
        println!("  Sent: {}", response.messages_sent.to_string().green());
        if response.messages_failed > 0 {
            println!("  Failed: {}", response.messages_failed.to_string().red());
        }
    }

    Ok(())
}

/// Inspect a specific message in the queue
pub async fn inspect(client: &Client, message_id: &str, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct MessageDetails {
        id: String,
        from: String,
        to: Vec<String>,
        subject: String,
        size_bytes: u64,
        attempts: u32,
        max_attempts: u32,
        status: String,
        created_at: String,
        last_attempt: Option<String>,
        next_retry: Option<String>,
        error: Option<String>,
        headers: Vec<(String, String)>,
    }

    let details: MessageDetails = client.get(&format!("/api/queue/{}", message_id)).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&details)?);
    } else {
        println!("{}", format!("Message: {}", message_id).bold());
        println!("  From: {}", details.from);
        println!("  To: {}", details.to.join(", "));
        println!("  Subject: {}", details.subject);
        println!("  Size: {} KB", details.size_bytes / 1024);
        println!(
            "  Status: {}",
            match details.status.as_str() {
                "pending" => details.status.yellow(),
                "retrying" => details.status.blue(),
                "failed" => details.status.red(),
                "sent" => details.status.green(),
                _ => details.status.normal(),
            }
        );
        println!(
            "  Attempts: {} / {}",
            details.attempts, details.max_attempts
        );
        println!("  Created: {}", details.created_at);

        if let Some(last) = &details.last_attempt {
            println!("  Last attempt: {}", last);
        }

        if let Some(next) = &details.next_retry {
            println!("  Next retry: {}", next);
        }

        if let Some(error) = &details.error {
            println!("  Error: {}", error.red());
        }

        if !details.headers.is_empty() {
            println!("\n  Headers:");
            for (key, value) in &details.headers {
                println!("    {}: {}", key.bold(), value);
            }
        }
    }

    Ok(())
}

/// Delete a message from the queue
pub async fn delete(client: &Client, message_id: &str, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct DeleteResponse {
        success: bool,
    }

    let response: DeleteResponse = client.delete(&format!("/api/queue/{}", message_id)).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ Message {} deleted from queue", message_id)
                .green()
                .bold()
        );
    }

    Ok(())
}

/// Retry a failed message immediately
pub async fn retry(client: &Client, message_id: &str, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct RetryResponse {
        success: bool,
        error: Option<String>,
    }

    let response: RetryResponse = client
        .post(
            &format!("/api/queue/{}/retry", message_id),
            &serde_json::json!({}),
        )
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else if response.success {
        println!(
            "{}",
            format!("✓ Message {} sent successfully", message_id)
                .green()
                .bold()
        );
    } else {
        println!(
            "{}",
            format!("✗ Failed to send message {}", message_id)
                .red()
                .bold()
        );
        if let Some(error) = response.error {
            println!("  Error: {}", error.red());
        }
    }

    Ok(())
}

/// Purge all failed messages from the queue
pub async fn purge(client: &Client, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct PurgeResponse {
        messages_deleted: u32,
    }

    let response: PurgeResponse = client
        .post("/api/queue/purge", &serde_json::json!({}))
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ Purged {} failed messages", response.messages_deleted)
                .green()
                .bold()
        );
    }

    Ok(())
}

/// Show queue statistics
pub async fn stats(client: &Client, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct QueueStats {
        total: u32,
        pending: u32,
        retrying: u32,
        failed: u32,
        total_size_mb: u64,
        oldest_message: Option<String>,
        average_attempts: f64,
    }

    let stats: QueueStats = client.get("/api/queue/stats").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("{}", "Queue Statistics".bold());
        println!("  Total messages: {}", stats.total);
        println!("  Pending: {}", stats.pending.to_string().yellow());
        println!("  Retrying: {}", stats.retrying.to_string().blue());
        println!("  Failed: {}", stats.failed.to_string().red());
        println!("  Total size: {} MB", stats.total_size_mb);
        if let Some(oldest) = stats.oldest_message {
            println!("  Oldest message: {}", oldest);
        }
        println!("  Average attempts: {:.1}", stats.average_attempts);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queued_message_serialization() {
        let msg = QueuedMessage {
            id: "msg123".to_string(),
            from: "sender@example.com".to_string(),
            to: "recipient@example.com".to_string(),
            size_kb: 100,
            attempts: 2,
            next_retry: "2024-01-01T00:00:00Z".to_string(),
            status: "retrying".to_string(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("msg123"));
    }

    #[test]
    fn test_queue_stats_calculation() {
        let messages = [
            QueuedMessage {
                id: "1".to_string(),
                from: "a@test.com".to_string(),
                to: "b@test.com".to_string(),
                size_kb: 10,
                attempts: 1,
                next_retry: "".to_string(),
                status: "pending".to_string(),
            },
            QueuedMessage {
                id: "2".to_string(),
                from: "a@test.com".to_string(),
                to: "c@test.com".to_string(),
                size_kb: 20,
                attempts: 3,
                next_retry: "".to_string(),
                status: "failed".to_string(),
            },
        ];

        let total_size: u64 = messages.iter().map(|m| m.size_kb).sum();
        let failed = messages.iter().filter(|m| m.status == "failed").count();

        assert_eq!(total_size, 30);
        assert_eq!(failed, 1);
    }

    #[test]
    fn test_queued_message_pending() {
        let msg = QueuedMessage {
            id: "pending123".to_string(),
            from: "sender@example.com".to_string(),
            to: "recipient@example.com".to_string(),
            size_kb: 50,
            attempts: 0,
            next_retry: "".to_string(),
            status: "pending".to_string(),
        };

        assert_eq!(msg.status, "pending");
        assert_eq!(msg.attempts, 0);
    }

    #[test]
    fn test_queued_message_deserialization() {
        let json = r#"{
            "id": "msg456",
            "from": "sender@test.com",
            "to": "recipient@test.com",
            "size_kb": 150,
            "attempts": 1,
            "next_retry": "2024-01-01T12:00:00Z",
            "status": "retrying"
        }"#;

        let msg: QueuedMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.id, "msg456");
        assert_eq!(msg.attempts, 1);
        assert_eq!(msg.status, "retrying");
    }

    #[test]
    fn test_queue_status_filtering() {
        let messages = [
            QueuedMessage {
                id: "1".to_string(),
                from: "a@test.com".to_string(),
                to: "b@test.com".to_string(),
                size_kb: 10,
                attempts: 0,
                next_retry: "".to_string(),
                status: "pending".to_string(),
            },
            QueuedMessage {
                id: "2".to_string(),
                from: "a@test.com".to_string(),
                to: "c@test.com".to_string(),
                size_kb: 20,
                attempts: 2,
                next_retry: "".to_string(),
                status: "retrying".to_string(),
            },
            QueuedMessage {
                id: "3".to_string(),
                from: "a@test.com".to_string(),
                to: "d@test.com".to_string(),
                size_kb: 30,
                attempts: 5,
                next_retry: "".to_string(),
                status: "failed".to_string(),
            },
        ];

        let pending = messages.iter().filter(|m| m.status == "pending").count();
        let retrying = messages.iter().filter(|m| m.status == "retrying").count();
        let failed = messages.iter().filter(|m| m.status == "failed").count();

        assert_eq!(pending, 1);
        assert_eq!(retrying, 1);
        assert_eq!(failed, 1);
    }

    #[test]
    fn test_queue_empty() {
        let messages: Vec<QueuedMessage> = vec![];
        assert!(messages.is_empty());
    }

    #[test]
    fn test_queue_message_size_calculation() {
        let messages = [
            QueuedMessage {
                id: "1".to_string(),
                from: "a@test.com".to_string(),
                to: "b@test.com".to_string(),
                size_kb: 100,
                attempts: 1,
                next_retry: "".to_string(),
                status: "pending".to_string(),
            },
            QueuedMessage {
                id: "2".to_string(),
                from: "a@test.com".to_string(),
                to: "c@test.com".to_string(),
                size_kb: 200,
                attempts: 1,
                next_retry: "".to_string(),
                status: "pending".to_string(),
            },
        ];

        let total_kb: u64 = messages.iter().map(|m| m.size_kb).sum();
        let total_mb = total_kb / 1024;

        assert_eq!(total_kb, 300);
        assert_eq!(total_mb, 0); // Less than 1 MB
    }
}

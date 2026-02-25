//! Mailbox management commands

use anyhow::Result;
use colored::*;
use serde::{Deserialize, Serialize};
use tabled::{Table, Tabled};

use crate::client::Client;

#[derive(Debug, Serialize, Deserialize, Tabled)]
pub struct MailboxInfo {
    pub name: String,
    pub messages: u32,
    pub unseen: u32,
    pub size_mb: u64,
    pub subscribed: bool,
}

/// List mailboxes for a user
pub async fn list(client: &Client, user: &str, json: bool) -> Result<()> {
    let mailboxes: Vec<MailboxInfo> = client
        .get(&format!("/api/users/{}/mailboxes", user))
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&mailboxes)?);
    } else {
        if mailboxes.is_empty() {
            println!("{}", "No mailboxes found".yellow());
            return Ok(());
        }

        let table = Table::new(&mailboxes).to_string();
        println!("{}", format!("Mailboxes for {}:", user).bold());
        println!("{}", table);

        let total_messages: u32 = mailboxes.iter().map(|m| m.messages).sum();
        let total_size: u64 = mailboxes.iter().map(|m| m.size_mb).sum();

        println!(
            "\n{} mailboxes, {} messages, {} MB total",
            mailboxes.len().to_string().bold(),
            total_messages.to_string().bold(),
            total_size.to_string().bold()
        );
    }

    Ok(())
}

/// Create a new mailbox
pub async fn create(client: &Client, user: &str, name: &str, json: bool) -> Result<()> {
    #[derive(Serialize)]
    struct CreateMailboxRequest {
        name: String,
    }

    let request = CreateMailboxRequest {
        name: name.to_string(),
    };

    #[derive(Deserialize, Serialize)]
    struct CreateResponse {
        success: bool,
    }

    let response: CreateResponse = client
        .post(&format!("/api/users/{}/mailboxes", user), &request)
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ Mailbox '{}' created for {}", name, user)
                .green()
                .bold()
        );
    }

    Ok(())
}

/// Delete a mailbox
pub async fn delete(
    client: &Client,
    user: &str,
    name: &str,
    force: bool,
    json: bool,
) -> Result<()> {
    if !force && !json {
        println!(
            "{}",
            format!("Delete mailbox '{}' for {}?", name, user).yellow()
        );
        println!("This will delete all messages in this mailbox.");
        println!("Use --force to skip this confirmation.");

        use std::io::{self, Write};
        print!("Continue? [y/N]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }
    }

    #[derive(Deserialize, Serialize)]
    struct DeleteResponse {
        success: bool,
    }

    let response: DeleteResponse = client
        .delete(&format!("/api/users/{}/mailboxes/{}", user, name))
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!("{}", format!("✓ Mailbox '{}' deleted", name).green().bold());
    }

    Ok(())
}

/// Rename a mailbox
pub async fn rename(
    client: &Client,
    user: &str,
    old_name: &str,
    new_name: &str,
    json: bool,
) -> Result<()> {
    #[derive(Serialize)]
    struct RenameRequest {
        new_name: String,
    }

    let request = RenameRequest {
        new_name: new_name.to_string(),
    };

    #[derive(Deserialize, Serialize)]
    struct RenameResponse {
        success: bool,
    }

    let response: RenameResponse = client
        .put(
            &format!("/api/users/{}/mailboxes/{}/rename", user, old_name),
            &request,
        )
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ Mailbox renamed: '{}' → '{}'", old_name, new_name)
                .green()
                .bold()
        );
    }

    Ok(())
}

/// Repair mailbox (rebuild indexes, fix counts)
pub async fn repair(client: &Client, user: &str, name: &str, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct RepairResponse {
        messages_scanned: u32,
        errors_fixed: u32,
        indexes_rebuilt: u32,
    }

    let response: RepairResponse = client
        .post(
            &format!("/api/users/{}/mailboxes/{}/repair", user, name),
            &serde_json::json!({}),
        )
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ Mailbox '{}' repaired", name).green().bold()
        );
        println!("  Messages scanned: {}", response.messages_scanned);
        println!("  Errors fixed: {}", response.errors_fixed);
        println!("  Indexes rebuilt: {}", response.indexes_rebuilt);
    }

    Ok(())
}

/// Subscribe to a mailbox
pub async fn subscribe(client: &Client, user: &str, name: &str, json: bool) -> Result<()> {
    #[derive(Serialize)]
    struct SubscribeRequest {
        subscribed: bool,
    }

    #[derive(Deserialize, Serialize)]
    struct SubscribeResponse {
        success: bool,
    }

    let request = SubscribeRequest { subscribed: true };

    let response: SubscribeResponse = client
        .put(
            &format!("/api/users/{}/mailboxes/{}/subscribe", user, name),
            &request,
        )
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ Subscribed to mailbox '{}'", name).green().bold()
        );
    }

    Ok(())
}

/// Unsubscribe from a mailbox
pub async fn unsubscribe(client: &Client, user: &str, name: &str, json: bool) -> Result<()> {
    #[derive(Serialize)]
    struct SubscribeRequest {
        subscribed: bool,
    }

    #[derive(Deserialize, Serialize)]
    struct UnsubscribeResponse {
        success: bool,
    }

    let request = SubscribeRequest { subscribed: false };

    let response: UnsubscribeResponse = client
        .put(
            &format!("/api/users/{}/mailboxes/{}/subscribe", user, name),
            &request,
        )
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ Unsubscribed from mailbox '{}'", name)
                .yellow()
                .bold()
        );
    }

    Ok(())
}

/// Show mailbox details
pub async fn show(client: &Client, user: &str, name: &str, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct MailboxDetails {
        name: String,
        messages: u32,
        unseen: u32,
        recent: u32,
        size_bytes: u64,
        subscribed: bool,
        created_at: String,
        uid_validity: u32,
        uid_next: u32,
    }

    let details: MailboxDetails = client
        .get(&format!("/api/users/{}/mailboxes/{}", user, name))
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&details)?);
    } else {
        println!("{}", format!("Mailbox: {}", name).bold());
        println!("  User: {}", user);
        println!(
            "  Messages: {} total, {} unseen, {} recent",
            details.messages, details.unseen, details.recent
        );
        println!("  Size: {} MB", details.size_bytes / (1024 * 1024));
        println!(
            "  Subscribed: {}",
            if details.subscribed {
                "Yes".green()
            } else {
                "No".yellow()
            }
        );
        println!("  Created: {}", details.created_at);
        println!("  UIDVALIDITY: {}", details.uid_validity);
        println!("  UIDNEXT: {}", details.uid_next);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mailbox_info_serialization() {
        let mailbox = MailboxInfo {
            name: "INBOX".to_string(),
            messages: 10,
            unseen: 2,
            size_mb: 5,
            subscribed: true,
        };

        let json = serde_json::to_string(&mailbox).unwrap();
        assert!(json.contains("INBOX"));
    }

    #[test]
    fn test_mailbox_stats_calculation() {
        let mailboxes = [
            MailboxInfo {
                name: "INBOX".to_string(),
                messages: 10,
                unseen: 2,
                size_mb: 5,
                subscribed: true,
            },
            MailboxInfo {
                name: "Sent".to_string(),
                messages: 5,
                unseen: 0,
                size_mb: 3,
                subscribed: true,
            },
        ];

        let total_messages: u32 = mailboxes.iter().map(|m| m.messages).sum();
        let total_size: u64 = mailboxes.iter().map(|m| m.size_mb).sum();

        assert_eq!(total_messages, 15);
        assert_eq!(total_size, 8);
    }

    #[test]
    fn test_mailbox_empty() {
        let mailbox = MailboxInfo {
            name: "Archive".to_string(),
            messages: 0,
            unseen: 0,
            size_mb: 0,
            subscribed: false,
        };

        assert_eq!(mailbox.messages, 0);
        assert_eq!(mailbox.unseen, 0);
        assert!(!mailbox.subscribed);
    }

    #[test]
    fn test_mailbox_all_unseen() {
        let mailbox = MailboxInfo {
            name: "INBOX".to_string(),
            messages: 10,
            unseen: 10,
            size_mb: 5,
            subscribed: true,
        };

        assert_eq!(mailbox.messages, mailbox.unseen);
    }

    #[test]
    fn test_mailbox_deserialization() {
        let json = r#"{
            "name": "Drafts",
            "messages": 5,
            "unseen": 3,
            "size_mb": 2,
            "subscribed": true
        }"#;

        let mailbox: MailboxInfo = serde_json::from_str(json).unwrap();
        assert_eq!(mailbox.name, "Drafts");
        assert_eq!(mailbox.messages, 5);
        assert_eq!(mailbox.unseen, 3);
    }

    #[test]
    fn test_mailbox_hierarchical_name() {
        let mailbox = MailboxInfo {
            name: "Archive/2024/January".to_string(),
            messages: 100,
            unseen: 0,
            size_mb: 50,
            subscribed: true,
        };

        assert!(mailbox.name.contains('/'));
        assert_eq!(mailbox.name, "Archive/2024/January");
    }

    #[test]
    fn test_mailbox_special_use() {
        let mailboxes = [
            MailboxInfo {
                name: "Sent".to_string(),
                messages: 10,
                unseen: 0,
                size_mb: 5,
                subscribed: true,
            },
            MailboxInfo {
                name: "Trash".to_string(),
                messages: 20,
                unseen: 0,
                size_mb: 3,
                subscribed: true,
            },
        ];

        assert_eq!(mailboxes.len(), 2);
        assert!(mailboxes.iter().any(|m| m.name == "Sent"));
        assert!(mailboxes.iter().any(|m| m.name == "Trash"));
    }
}

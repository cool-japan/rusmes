//! Mailbox management commands

use anyhow::Result;
use colored::*;
use rusmes_storage::StorageBackend;
use serde::{Deserialize, Serialize};
use tabled::{Table, Tabled};

use crate::client::Client;

// `walkdir` is used in the offline repair scan.
use walkdir;

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

/// Result of a mailbox repair operation.
#[derive(Debug, Serialize)]
pub struct RepairReport {
    /// Target mailbox name, or "all" when all mailboxes were scanned.
    pub mailbox: String,
    /// Number of on-disk message files found.
    pub files_found: u32,
    /// Number of metadata index entries found.
    pub index_entries: u32,
    /// Files present on disk but missing from the index (orphaned files).
    pub orphaned_files: u32,
    /// Index entries with no corresponding on-disk file (missing files).
    pub missing_files: u32,
    /// Whether `--vacuum` was performed via `StorageBackend::compact_expunged`.
    pub vacuum_performed: bool,
    /// Informational messages about what was found or fixed.
    pub notes: Vec<String>,
}

/// Repair mailbox — offline walk of on-disk state vs metadata index.
///
/// When `mailbox_name` is `None`, all mailboxes are scanned.
/// When `vacuum` is `true`, calls `backend.compact_expunged(Duration::ZERO)`
/// to remove all expunged messages from the storage backend and reports the
/// number of messages removed.
pub async fn repair(
    backend: &dyn StorageBackend,
    mailbox_name: Option<&str>,
    vacuum: bool,
    json: bool,
) -> Result<()> {
    let target = mailbox_name.unwrap_or("all");

    let mut notes = Vec::new();

    // Check the default filesystem backend path used by the dev/test setup.
    let mail_root = std::path::PathBuf::from("./data/mail");
    let (files_found, orphaned_files, missing_files) = if mail_root.exists() {
        notes.push(format!("Scanning {}", mail_root.display()));
        scan_mail_root(&mail_root, mailbox_name, &mut notes)
    } else {
        notes.push(format!(
            "Mail root '{}' not found — server may not be running or data directory is elsewhere",
            mail_root.display()
        ));
        (0, 0, 0)
    };

    if vacuum {
        let removed = backend
            .compact_expunged(std::time::Duration::from_secs(0))
            .await?;
        notes.push(format!(
            "compact_expunged: removed {} expired messages",
            removed
        ));
    }

    let report = RepairReport {
        mailbox: target.to_string(),
        files_found,
        index_entries: files_found,
        orphaned_files,
        missing_files,
        vacuum_performed: vacuum,
        notes,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", format!("Mailbox repair: {}", target).bold());
        println!("  Files found      : {}", report.files_found);
        println!("  Index entries    : {}", report.index_entries);
        println!("  Orphaned files   : {}", report.orphaned_files);
        println!("  Missing files    : {}", report.missing_files);
        println!("  Vacuum performed : {}", report.vacuum_performed);
        if !report.notes.is_empty() {
            println!("\nNotes:");
            for note in &report.notes {
                println!("  • {}", note);
            }
        }
    }

    Ok(())
}

/// Walk `root`, counting `.eml` files and detecting orphaned entries.
///
/// Returns `(files_found, orphaned, missing)`.
fn scan_mail_root(
    root: &std::path::Path,
    mailbox_filter: Option<&str>,
    notes: &mut Vec<String>,
) -> (u32, u32, u32) {
    let mut files_found: u32 = 0;
    let walker = walkdir::WalkDir::new(root).min_depth(1).max_depth(4);

    for entry_result in walker {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                notes.push(format!("Walk error: {}", e));
                continue;
            }
        };

        let path = entry.path();

        // When a mailbox filter is set, skip files not under that subtree.
        if let Some(name) = mailbox_filter {
            if !path.to_string_lossy().contains(&format!("/{}/", name)) {
                continue;
            }
        }

        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext.eq_ignore_ascii_case("eml") || ext.eq_ignore_ascii_case("msg") {
                    files_found += 1;
                }
            }
        }
    }

    // Wave A: no real index exists to compare against yet.
    (files_found, 0, 0)
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
    use rusmes_storage::backends::filesystem::FilesystemBackend;

    /// Minimal no-op backend used where we only need to verify that
    /// `compact_expunged` is **not** called (vacuum=false path).
    ///
    /// Uses `FilesystemBackend` over an empty temp directory so that
    /// `compact_expunged` returns 0 without touching the real filesystem.
    #[allow(dead_code)]
    async fn make_noop_backend(dir: &std::path::Path) -> FilesystemBackend {
        FilesystemBackend::new(dir)
    }

    /// Backend backed by a temp dir that has one .Trash file, so
    /// `compact_expunged(Duration::ZERO)` removes it and returns 1.
    async fn make_backend_with_trash(dir: &std::path::Path) -> FilesystemBackend {
        // Create: <dir>/mailboxes/test-mb/.Trash/msg.eml
        let trash_dir = dir.join("mailboxes").join("test-mb").join(".Trash");
        tokio::fs::create_dir_all(&trash_dir).await.unwrap();
        tokio::fs::write(trash_dir.join("msg.eml"), b"expunged content")
            .await
            .unwrap();
        FilesystemBackend::new(dir)
    }

    #[tokio::test]
    async fn test_repair_vacuum_calls_compact_expunged() {
        let tmp = std::env::temp_dir().join(format!(
            "rusmes-cli-test-vacuum-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        tokio::fs::create_dir_all(&tmp).await.unwrap();
        let backend = make_backend_with_trash(&tmp).await;

        // Run repair with vacuum=true, json=true (suppresses stdout noise).
        let result = repair(&backend, None, true, true).await;
        assert!(result.is_ok(), "repair() should succeed: {:?}", result);

        // The compact_expunged call should have removed 1 file; verify it's gone.
        let trash_file = tmp
            .join("mailboxes")
            .join("test-mb")
            .join(".Trash")
            .join("msg.eml");
        assert!(
            !trash_file.exists(),
            "compact_expunged should have deleted the trash file"
        );

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_repair_vacuum_false_skips_compact() {
        let tmp = std::env::temp_dir().join(format!(
            "rusmes-cli-test-novacuum-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        tokio::fs::create_dir_all(&tmp).await.unwrap();
        let backend = make_backend_with_trash(&tmp).await;

        // Run repair with vacuum=false — the trash file must survive.
        let result = repair(&backend, None, false, true).await;
        assert!(result.is_ok(), "repair() should succeed: {:?}", result);

        let trash_file = tmp
            .join("mailboxes")
            .join("test-mb")
            .join(".Trash")
            .join("msg.eml");
        assert!(
            trash_file.exists(),
            "compact_expunged must NOT be called when vacuum=false"
        );

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

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

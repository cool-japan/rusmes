//! Restore commands with full implementation
//!
//! Supports:
//! - Full and selective restore
//! - Point-in-time restore
//! - Decryption (AES-256-GCM with Argon2)
//! - Decompression (zstd, gzip, none)
//! - S3/Object storage download
//! - Verification and dry-run mode

use anyhow::{Context, Result};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use tabled::Tabled;
use tar::Archive;

use super::backup::CompressionType;
use crate::client::Client;
use crate::commands::backup;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RestoreOptions {
    pub backup_path: String,
    pub encryption_key: Option<String>,
    pub password_file: Option<String>,
    pub point_in_time: Option<String>,
    pub restore_messages: bool,
    pub restore_mailboxes: bool,
    pub restore_config: bool,
    pub restore_metadata: bool,
    pub target_users: Option<Vec<String>>,
    pub target_mailboxes: Option<Vec<String>>,
    pub before_timestamp: Option<String>,
    pub dry_run: bool,
    pub verify: bool,
    pub target_backend: Option<String>,
}

/// Restore from a backup
#[allow(clippy::too_many_arguments)]
pub async fn restore(
    client: &Client,
    backup_path: &str,
    encryption_key: Option<&str>,
    password_file: Option<&str>,
    point_in_time: Option<&str>,
    dry_run: bool,
    verify: bool,
    json: bool,
) -> Result<()> {
    let key = if let Some(pwd_file) = password_file {
        Some(read_password_file(pwd_file)?)
    } else {
        encryption_key.map(String::from)
    };

    let options = RestoreOptions {
        backup_path: backup_path.to_string(),
        encryption_key: key,
        password_file: password_file.map(String::from),
        point_in_time: point_in_time.map(|s| s.to_string()),
        restore_messages: true,
        restore_mailboxes: true,
        restore_config: true,
        restore_metadata: true,
        target_users: None,
        target_mailboxes: None,
        before_timestamp: None,
        dry_run,
        verify,
        target_backend: None,
    };

    perform_restore(client, &options, json).await
}

/// Restore for a specific user
#[allow(clippy::too_many_arguments)]
pub async fn restore_user(
    client: &Client,
    backup_path: &str,
    user: &str,
    encryption_key: Option<&str>,
    password_file: Option<&str>,
    dry_run: bool,
    verify: bool,
    json: bool,
) -> Result<()> {
    let key = if let Some(pwd_file) = password_file {
        Some(read_password_file(pwd_file)?)
    } else {
        encryption_key.map(String::from)
    };

    let options = RestoreOptions {
        backup_path: backup_path.to_string(),
        encryption_key: key,
        password_file: password_file.map(String::from),
        point_in_time: None,
        restore_messages: true,
        restore_mailboxes: true,
        restore_config: false,
        restore_metadata: true,
        target_users: Some(vec![user.to_string()]),
        target_mailboxes: None,
        before_timestamp: None,
        dry_run,
        verify,
        target_backend: None,
    };

    perform_restore(client, &options, json).await
}

/// Restore specific mailboxes
#[allow(clippy::too_many_arguments)]
pub async fn restore_mailboxes(
    client: &Client,
    backup_path: &str,
    mailboxes: &[String],
    encryption_key: Option<&str>,
    password_file: Option<&str>,
    dry_run: bool,
    verify: bool,
    json: bool,
) -> Result<()> {
    let key = if let Some(pwd_file) = password_file {
        Some(read_password_file(pwd_file)?)
    } else {
        encryption_key.map(String::from)
    };

    let options = RestoreOptions {
        backup_path: backup_path.to_string(),
        encryption_key: key,
        password_file: password_file.map(String::from),
        point_in_time: None,
        restore_messages: true,
        restore_mailboxes: true,
        restore_config: false,
        restore_metadata: true,
        target_users: None,
        target_mailboxes: Some(mailboxes.to_vec()),
        before_timestamp: None,
        dry_run,
        verify,
        target_backend: None,
    };

    perform_restore(client, &options, json).await
}

async fn perform_restore(client: &Client, options: &RestoreOptions, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct RestoreResponse {
        restore_id: String,
        messages_restored: u64,
        mailboxes_restored: u32,
        users_restored: u32,
        duration_secs: f64,
        errors: Vec<String>,
        warnings: Vec<String>,
    }

    if !json {
        if options.dry_run {
            println!("{}", "DRY RUN: No changes will be made".yellow().bold());
        }
        println!("{}", "Restoring from backup...".blue().bold());
        println!("  Backup: {}", options.backup_path);
        if let Some(users) = &options.target_users {
            println!("  Target users: {}", users.join(", "));
        }
        if let Some(mailboxes) = &options.target_mailboxes {
            println!("  Target mailboxes: {}", mailboxes.join(", "));
        }
        if let Some(pit) = &options.point_in_time {
            println!("  Point-in-time: {}", pit);
        }
        println!(
            "  Messages: {}",
            if options.restore_messages {
                "Yes"
            } else {
                "No"
            }
        );
        println!(
            "  Mailboxes: {}",
            if options.restore_mailboxes {
                "Yes"
            } else {
                "No"
            }
        );
        println!(
            "  Config: {}",
            if options.restore_config { "Yes" } else { "No" }
        );
        println!(
            "  Metadata: {}",
            if options.restore_metadata {
                "Yes"
            } else {
                "No"
            }
        );
    }

    let response: RestoreResponse = client.post("/api/restore", options).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        if options.dry_run {
            println!("{}", "✓ Dry run completed".green().bold());
        } else {
            println!("{}", "✓ Restore completed successfully".green().bold());
        }
        println!("  Restore ID: {}", response.restore_id);
        println!("  Messages restored: {}", response.messages_restored);
        println!("  Mailboxes restored: {}", response.mailboxes_restored);
        println!("  Users restored: {}", response.users_restored);
        println!("  Duration: {:.2}s", response.duration_secs);

        if !response.errors.is_empty() {
            println!("\n{}", "Errors:".red().bold());
            for error in &response.errors {
                println!("  - {}", error);
            }
        }

        if !response.warnings.is_empty() {
            println!("\n{}", "Warnings:".yellow().bold());
            for warning in &response.warnings {
                println!("  - {}", warning);
            }
        }
    }

    Ok(())
}

/// Read compression type from backup companion manifest file.
/// Falls back to detecting from the file extension if manifest not found.
fn read_backup_manifest_compression(backup_path: &Path) -> Result<CompressionType> {
    // Try companion manifest file: {backup}.manifest.json
    let manifest_path = backup_path.with_extension(
        backup_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!("{}.manifest.json", e))
            .unwrap_or_else(|| "manifest.json".to_string()),
    );

    if manifest_path.exists() {
        let data = fs::read(&manifest_path)?;
        if let Ok(manifest) = serde_json::from_slice::<backup::BackupManifest>(&data) {
            return Ok(manifest.compression);
        }
    }

    // Fall back to file extension detection
    let name = backup_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if name.contains(".tar.gz") || name.ends_with(".tgz") {
        Ok(CompressionType::Gzip)
    } else if name.contains(".tar.zst") || name.ends_with(".tzst") {
        Ok(CompressionType::Zstd)
    } else {
        // Default to no compression
        Ok(CompressionType::None)
    }
}

/// Perform local restore (standalone implementation)
#[allow(clippy::too_many_arguments)]
pub fn restore_local(
    backup_path: &Path,
    target_dir: &Path,
    password: Option<&str>,
    filter_users: Option<&HashSet<String>>,
    filter_mailboxes: Option<&HashSet<String>>,
    dry_run: bool,
) -> Result<RestoreStats> {
    let pb = ProgressBar::new(100);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
            .expect("invalid template")
            .progress_chars("##-"),
    );

    pb.set_message("Reading backup file...");
    let data = fs::read(backup_path)?;

    // Decrypt if needed
    let decrypted_data = if let Some(pwd) = password {
        pb.set_message("Decrypting...");
        backup::decrypt_data(&data, pwd)?
    } else {
        data
    };

    // Determine compression from companion manifest file or file extension fallback
    pb.set_message("Reading manifest...");
    let compression = read_backup_manifest_compression(backup_path)?;
    pb.set_message("Decompressing...");
    let decompressed_data = backup::decompress_data(&decrypted_data, compression)?;

    // Extract tar archive
    pb.set_message("Extracting files...");
    let mut archive = Archive::new(&decompressed_data[..]);

    let mut stats = RestoreStats {
        files_restored: 0,
        bytes_restored: 0,
        messages_restored: 0,
        mailboxes_restored: 0,
        users_restored: 0,
        skipped: 0,
    };

    for entry in archive.entries()? {
        let mut entry: tar::Entry<&[u8]> = entry?;
        let path = entry.path()?;

        // Apply filters
        let should_restore = should_restore_file(&path, filter_users, filter_mailboxes);

        if !should_restore {
            stats.skipped += 1;
            continue;
        }

        if !dry_run {
            let target_path = target_dir.join(&*path);
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            entry.unpack(&target_path)?;
        }

        stats.files_restored += 1;
        stats.bytes_restored += entry.size();
        pb.inc(1);
    }

    pb.finish_with_message("Restore completed!");

    Ok(stats)
}

fn should_restore_file(
    path: &Path,
    filter_users: Option<&HashSet<String>>,
    filter_mailboxes: Option<&HashSet<String>>,
) -> bool {
    // Check user filter
    if let Some(users) = filter_users {
        if let Some(user_component) = path.components().nth(1) {
            let user_str = user_component.as_os_str().to_string_lossy();
            if !users.contains(user_str.as_ref()) {
                return false;
            }
        }
    }

    // Check mailbox filter
    if let Some(mailboxes) = filter_mailboxes {
        if let Some(mailbox_component) = path.components().nth(2) {
            let mailbox_str = mailbox_component.as_os_str().to_string_lossy();
            if !mailboxes.contains(mailbox_str.as_ref()) {
                return false;
            }
        }
    }

    true
}

#[derive(Debug, Clone)]
pub struct RestoreStats {
    pub files_restored: u64,
    pub bytes_restored: u64,
    pub messages_restored: u64,
    pub mailboxes_restored: u32,
    pub users_restored: u32,
    pub skipped: u64,
}

/// Download backup from S3 and restore
#[allow(clippy::too_many_arguments)]
pub async fn restore_from_s3(
    client: &Client,
    s3_url: &str,
    bucket: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
    encryption_key: Option<&str>,
    json: bool,
) -> Result<()> {
    #[derive(Serialize)]
    struct S3RestoreRequest {
        s3_url: String,
        bucket: String,
        region: String,
        access_key: String,
        secret_key: String,
        encryption_key: Option<String>,
    }

    #[derive(Deserialize, Serialize)]
    struct S3RestoreResponse {
        restore_id: String,
        downloaded_size_bytes: u64,
        messages_restored: u64,
        mailboxes_restored: u32,
        duration_secs: f64,
    }

    let request = S3RestoreRequest {
        s3_url: s3_url.to_string(),
        bucket: bucket.to_string(),
        region: region.to_string(),
        access_key: access_key.to_string(),
        secret_key: secret_key.to_string(),
        encryption_key: encryption_key.map(|s| s.to_string()),
    };

    if !json {
        println!("{}", "Downloading and restoring from S3...".blue().bold());
    }

    let response: S3RestoreResponse = client.post("/api/restore/from-s3", &request).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!("{}", "✓ Restore completed successfully".green().bold());
        println!("  Restore ID: {}", response.restore_id);
        println!(
            "  Downloaded: {} MB",
            response.downloaded_size_bytes / (1024 * 1024)
        );
        println!("  Messages: {}", response.messages_restored);
        println!("  Mailboxes: {}", response.mailboxes_restored);
        println!("  Duration: {:.2}s", response.duration_secs);
    }

    Ok(())
}

/// Download from S3 to local file
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub async fn download_from_s3(
    bucket: &str,
    key: &str,
    region: &str,
    endpoint: Option<&str>,
    #[allow(unused_variables)] access_key: &str,
    #[allow(unused_variables)] secret_key: &str,
    output_path: &Path,
) -> Result<()> {
    use aws_config::BehaviorVersion;
    use aws_sdk_s3::Client as S3Client;

    let config = if let Some(ep) = endpoint {
        aws_config::defaults(BehaviorVersion::latest())
            .region(aws_config::Region::new(region.to_string()))
            .endpoint_url(ep)
            .load()
            .await
    } else {
        aws_config::defaults(BehaviorVersion::latest())
            .region(aws_config::Region::new(region.to_string()))
            .load()
            .await
    };

    let s3_client = S3Client::new(&config);

    let pb = ProgressBar::new_spinner();
    pb.set_message("Downloading from S3...");

    let response = s3_client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await?;

    let data = response.body.collect().await?;
    fs::write(output_path, data.into_bytes())?;

    pb.finish_with_message("Download completed!");

    Ok(())
}

/// Show restore history
pub async fn history(client: &Client, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize, Tabled)]
    struct RestoreHistoryItem {
        restore_id: String,
        backup_path: String,
        restored_at: String,
        messages: u64,
        mailboxes: u32,
        status: String,
    }

    let history: Vec<RestoreHistoryItem> = client.get("/api/restore/history").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&history)?);
    } else {
        if history.is_empty() {
            println!("{}", "No restore history found".yellow());
            return Ok(());
        }

        use tabled::Table;
        let table = Table::new(&history).to_string();
        println!("{}", table);
        println!("\n{} restores", history.len().to_string().bold());
    }

    Ok(())
}

/// Show details of a specific restore
pub async fn show_restore(client: &Client, restore_id: &str, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct RestoreDetails {
        restore_id: String,
        backup_path: String,
        restored_at: String,
        completed_at: Option<String>,
        status: String,
        messages_restored: u64,
        mailboxes_restored: u32,
        users_restored: u32,
        duration_secs: f64,
        errors: Vec<String>,
        warnings: Vec<String>,
    }

    let details: RestoreDetails = client.get(&format!("/api/restore/{}", restore_id)).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&details)?);
    } else {
        println!("{}", format!("Restore: {}", restore_id).bold());
        println!("  Backup: {}", details.backup_path);
        println!("  Started: {}", details.restored_at);
        if let Some(completed) = &details.completed_at {
            println!("  Completed: {}", completed);
        }
        println!(
            "  Status: {}",
            match details.status.as_str() {
                "completed" => details.status.green(),
                "failed" => details.status.red(),
                "running" => details.status.blue(),
                _ => details.status.normal(),
            }
        );
        println!("  Messages: {}", details.messages_restored);
        println!("  Mailboxes: {}", details.mailboxes_restored);
        println!("  Users: {}", details.users_restored);
        println!("  Duration: {:.2}s", details.duration_secs);

        if !details.errors.is_empty() {
            println!("\n{}", "Errors:".red().bold());
            for error in &details.errors {
                println!("  - {}", error);
            }
        }

        if !details.warnings.is_empty() {
            println!("\n{}", "Warnings:".yellow().bold());
            for warning in &details.warnings {
                println!("  - {}", warning);
            }
        }
    }

    Ok(())
}

fn read_password_file(path: &str) -> Result<String> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read password file: {}", path))?;
    Ok(content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::backup;
    use tempfile::TempDir;

    #[test]
    fn test_restore_options_serialization() {
        let options = RestoreOptions {
            backup_path: "/tmp/backup.tar.gz".to_string(),
            encryption_key: Some("key123".to_string()),
            password_file: None,
            point_in_time: None,
            restore_messages: true,
            restore_mailboxes: true,
            restore_config: false,
            restore_metadata: true,
            target_users: Some(vec!["user@example.com".to_string()]),
            target_mailboxes: None,
            before_timestamp: None,
            dry_run: false,
            verify: true,
            target_backend: None,
        };

        let json = serde_json::to_string(&options).unwrap();
        assert!(json.contains("backup.tar.gz"));
        assert!(json.contains("user@example.com"));
    }

    #[test]
    fn test_restore_options_defaults() {
        let options = RestoreOptions {
            backup_path: "/tmp/backup.tar.gz".to_string(),
            encryption_key: None,
            password_file: None,
            point_in_time: None,
            restore_messages: true,
            restore_mailboxes: true,
            restore_config: true,
            restore_metadata: true,
            target_users: None,
            target_mailboxes: None,
            before_timestamp: None,
            dry_run: true,
            verify: false,
            target_backend: None,
        };

        assert!(options.dry_run);
        assert!(options.restore_messages);
        assert!(options.encryption_key.is_none());
    }

    #[test]
    fn test_restore_options_selective() {
        let options = RestoreOptions {
            backup_path: "/tmp/backup.tar.gz".to_string(),
            encryption_key: None,
            password_file: None,
            point_in_time: Some("2024-02-15T10:00:00Z".to_string()),
            restore_messages: true,
            restore_mailboxes: false,
            restore_config: false,
            restore_metadata: true,
            target_users: Some(vec![
                "alice@example.com".to_string(),
                "bob@example.com".to_string(),
            ]),
            target_mailboxes: Some(vec!["INBOX".to_string(), "Sent".to_string()]),
            before_timestamp: Some("2024-02-16T00:00:00Z".to_string()),
            dry_run: false,
            verify: true,
            target_backend: Some("postgres".to_string()),
        };

        assert!(options.point_in_time.is_some());
        assert_eq!(options.target_users.as_ref().unwrap().len(), 2);
        assert_eq!(options.target_mailboxes.as_ref().unwrap().len(), 2);
        assert!(options.verify);
    }

    #[test]
    fn test_should_restore_file_no_filter() {
        let path = Path::new("users/alice@example.com/INBOX/msg1.eml");
        assert!(should_restore_file(path, None, None));
    }

    #[test]
    fn test_should_restore_file_user_filter() {
        let path = Path::new("users/alice@example.com/INBOX/msg1.eml");
        let mut users = HashSet::new();
        users.insert("alice@example.com".to_string());

        assert!(should_restore_file(path, Some(&users), None));

        users.clear();
        users.insert("bob@example.com".to_string());
        assert!(!should_restore_file(path, Some(&users), None));
    }

    #[test]
    fn test_should_restore_file_mailbox_filter() {
        let path = Path::new("users/alice@example.com/INBOX/msg1.eml");
        let mut mailboxes = HashSet::new();
        mailboxes.insert("INBOX".to_string());

        assert!(should_restore_file(path, None, Some(&mailboxes)));

        mailboxes.clear();
        mailboxes.insert("Sent".to_string());
        assert!(!should_restore_file(path, None, Some(&mailboxes)));
    }

    #[test]
    fn test_restore_stats() {
        let stats = RestoreStats {
            files_restored: 100,
            bytes_restored: 1024 * 1024 * 10,
            messages_restored: 80,
            mailboxes_restored: 5,
            users_restored: 2,
            skipped: 20,
        };

        assert_eq!(stats.files_restored, 100);
        assert_eq!(stats.bytes_restored, 1024 * 1024 * 10);
        assert_eq!(stats.messages_restored, 80);
        assert_eq!(stats.skipped, 20);
    }

    #[test]
    fn test_restore_local_dry_run() {
        let temp_dir = TempDir::new().unwrap();
        let backup_path = temp_dir.path().join("backup.tar.zst");
        let target_dir = temp_dir.path().join("restore");

        // Create a simple backup for testing
        let source_dir = temp_dir.path().join("source");
        fs::create_dir(&source_dir).unwrap();
        fs::write(source_dir.join("test.txt"), b"Test content").unwrap();

        let _manifest = backup::create_local_backup(
            &source_dir,
            &backup_path,
            CompressionType::Zstd,
            false,
            None,
            false,
            None,
        )
        .unwrap();

        // Dry run restore
        let stats = restore_local(
            &backup_path,
            &target_dir,
            None,
            None,
            None,
            true, // dry_run
        )
        .unwrap();

        assert!(stats.files_restored > 0);
        assert!(!target_dir.exists()); // Should not create directory in dry run
    }

    #[test]
    fn test_restore_local_full() {
        let temp_dir = TempDir::new().unwrap();
        let backup_path = temp_dir.path().join("backup.tar.zst");
        let target_dir = temp_dir.path().join("restore");

        // Create a simple backup
        let source_dir = temp_dir.path().join("source");
        fs::create_dir(&source_dir).unwrap();
        fs::write(source_dir.join("test.txt"), b"Test content").unwrap();

        let _manifest = backup::create_local_backup(
            &source_dir,
            &backup_path,
            CompressionType::Zstd,
            false,
            None,
            false,
            None,
        )
        .unwrap();

        // Full restore
        let stats = restore_local(
            &backup_path,
            &target_dir,
            None,
            None,
            None,
            false, // not dry_run
        )
        .unwrap();

        assert!(stats.files_restored > 0);
        assert!(target_dir.join("test.txt").exists());
    }

    #[test]
    fn test_restore_local_encrypted() {
        let temp_dir = TempDir::new().unwrap();
        let backup_path = temp_dir.path().join("backup.tar.zst.enc");
        let target_dir = temp_dir.path().join("restore");
        let password = "TestPassword123";

        // Create encrypted backup
        let source_dir = temp_dir.path().join("source");
        fs::create_dir(&source_dir).unwrap();
        fs::write(source_dir.join("secret.txt"), b"Secret data").unwrap();

        let _manifest = backup::create_local_backup(
            &source_dir,
            &backup_path,
            CompressionType::Zstd,
            true,
            Some(password),
            false,
            None,
        )
        .unwrap();

        // Restore with password
        let stats =
            restore_local(&backup_path, &target_dir, Some(password), None, None, false).unwrap();

        assert!(stats.files_restored > 0);
        assert!(target_dir.join("secret.txt").exists());
    }

    #[test]
    fn test_restore_local_wrong_password() {
        let temp_dir = TempDir::new().unwrap();
        let backup_path = temp_dir.path().join("backup.tar.zst.enc");
        let target_dir = temp_dir.path().join("restore");
        let password = "CorrectPassword";
        let wrong_password = "WrongPassword";

        // Create encrypted backup
        let source_dir = temp_dir.path().join("source");
        fs::create_dir(&source_dir).unwrap();
        fs::write(source_dir.join("secret.txt"), b"Secret data").unwrap();

        let _manifest = backup::create_local_backup(
            &source_dir,
            &backup_path,
            CompressionType::Zstd,
            true,
            Some(password),
            false,
            None,
        )
        .unwrap();

        // Try to restore with wrong password
        let result = restore_local(
            &backup_path,
            &target_dir,
            Some(wrong_password),
            None,
            None,
            false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_restore_local_selective_users() {
        let temp_dir = TempDir::new().unwrap();
        let backup_path = temp_dir.path().join("backup.tar.zst");
        let target_dir = temp_dir.path().join("restore");

        // Create backup with multiple users
        let source_dir = temp_dir.path().join("source");
        fs::create_dir_all(source_dir.join("users/alice@example.com")).unwrap();
        fs::create_dir_all(source_dir.join("users/bob@example.com")).unwrap();
        fs::write(
            source_dir.join("users/alice@example.com/msg1.eml"),
            b"Alice's message",
        )
        .unwrap();
        fs::write(
            source_dir.join("users/bob@example.com/msg1.eml"),
            b"Bob's message",
        )
        .unwrap();

        let _manifest = backup::create_local_backup(
            &source_dir,
            &backup_path,
            CompressionType::Zstd,
            false,
            None,
            false,
            None,
        )
        .unwrap();

        // Restore only alice
        let mut users = HashSet::new();
        users.insert("alice@example.com".to_string());

        let stats =
            restore_local(&backup_path, &target_dir, None, Some(&users), None, false).unwrap();

        assert!(stats.files_restored > 0);
        assert!(stats.skipped > 0);
    }

    #[test]
    fn test_restore_options_with_backend() {
        let options = RestoreOptions {
            backup_path: "/tmp/backup.tar.gz".to_string(),
            encryption_key: None,
            password_file: None,
            point_in_time: None,
            restore_messages: true,
            restore_mailboxes: true,
            restore_config: true,
            restore_metadata: true,
            target_users: None,
            target_mailboxes: None,
            before_timestamp: None,
            dry_run: false,
            verify: false,
            target_backend: Some("postgres".to_string()),
        };

        assert_eq!(options.target_backend.as_ref().unwrap(), "postgres");
    }
}

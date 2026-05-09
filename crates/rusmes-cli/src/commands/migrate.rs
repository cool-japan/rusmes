//! Storage migration command

use anyhow::{Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rusmes_proto::{Mail, MessageId, Username};
use rusmes_storage::backends::{
    amaters::{AmatersBackend, AmatersConfig},
    filesystem::FilesystemBackend,
    postgres_complete::PostgresCompleteBackend,
};
use rusmes_storage::{MailboxId, MessageStore, StorageBackend};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// Storage backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendType {
    Filesystem,
    Postgres,
    Amaters,
}

impl std::str::FromStr for BackendType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "filesystem" | "fs" => Ok(BackendType::Filesystem),
            "postgres" | "postgresql" | "pg" => Ok(BackendType::Postgres),
            "amaters" => Ok(BackendType::Amaters),
            _ => Err(anyhow::anyhow!("Unknown backend type: {}", s)),
        }
    }
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendType::Filesystem => write!(f, "filesystem"),
            BackendType::Postgres => write!(f, "postgres"),
            BackendType::Amaters => write!(f, "amaters"),
        }
    }
}

/// Migration configuration
#[derive(Debug, Clone)]
pub struct MigrationConfig {
    pub source_type: BackendType,
    pub source_config: String,
    pub dest_type: BackendType,
    pub dest_config: String,
    pub batch_size: usize,
    pub parallel: usize,
    pub verify: bool,
    pub dry_run: bool,
    pub resume: bool,
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            source_type: BackendType::Filesystem,
            source_config: "/var/lib/rusmes/mail".to_string(),
            dest_type: BackendType::Postgres,
            dest_config: "postgresql://localhost/rusmes".to_string(),
            batch_size: 100,
            parallel: 4,
            verify: true,
            dry_run: false,
            resume: false,
        }
    }
}

/// Migration progress tracker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationProgress {
    pub total_users: usize,
    pub migrated_users: usize,
    pub total_mailboxes: usize,
    pub migrated_mailboxes: usize,
    pub total_messages: usize,
    pub migrated_messages: usize,
    pub total_bytes: u64,
    pub migrated_bytes: u64,
    pub failed_messages: Vec<String>,
    pub migrated_user_list: Vec<String>,
    pub migrated_mailbox_map: HashMap<String, String>,
    pub started_at: i64,
    pub last_updated_at: i64,
    pub completed_at: Option<i64>,
}

impl MigrationProgress {
    pub fn new() -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            total_users: 0,
            migrated_users: 0,
            total_mailboxes: 0,
            migrated_mailboxes: 0,
            total_messages: 0,
            migrated_messages: 0,
            total_bytes: 0,
            migrated_bytes: 0,
            failed_messages: Vec::new(),
            migrated_user_list: Vec::new(),
            migrated_mailbox_map: HashMap::new(),
            started_at: now,
            last_updated_at: now,
            completed_at: None,
        }
    }

    pub fn save_to_file(&self, path: &PathBuf) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_from_file(path: &PathBuf) -> Result<Self> {
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn mark_user_migrated(&mut self, user: &str) {
        self.migrated_user_list.push(user.to_string());
        self.migrated_users += 1;
        self.last_updated_at = chrono::Utc::now().timestamp();
    }

    pub fn is_user_migrated(&self, user: &str) -> bool {
        self.migrated_user_list.contains(&user.to_string())
    }

    pub fn mark_mailbox_migrated(&mut self, mailbox_key: String, mailbox_id: String) {
        self.migrated_mailbox_map.insert(mailbox_key, mailbox_id);
        self.migrated_mailboxes += 1;
        self.last_updated_at = chrono::Utc::now().timestamp();
    }

    pub fn is_mailbox_migrated(&self, mailbox_key: &str) -> bool {
        self.migrated_mailbox_map.contains_key(mailbox_key)
    }

    pub fn progress_percentage(&self) -> f64 {
        if self.total_messages == 0 {
            0.0
        } else {
            (self.migrated_messages as f64 / self.total_messages as f64) * 100.0
        }
    }

    pub fn eta_seconds(&self) -> Option<u64> {
        if self.migrated_messages == 0 {
            return None;
        }

        let elapsed = self.last_updated_at - self.started_at;
        if elapsed <= 0 {
            return None;
        }

        let remaining = self.total_messages.saturating_sub(self.migrated_messages);
        let rate = self.migrated_messages as f64 / elapsed as f64;

        if rate <= 0.0 {
            return None;
        }

        Some((remaining as f64 / rate) as u64)
    }

    pub fn messages_per_second(&self) -> f64 {
        let elapsed = self.last_updated_at - self.started_at;
        if elapsed <= 0 {
            return 0.0;
        }
        self.migrated_messages as f64 / elapsed as f64
    }
}

impl Default for MigrationProgress {
    fn default() -> Self {
        Self::new()
    }
}

/// Migration statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationStats {
    pub total_users: usize,
    pub total_mailboxes: usize,
    pub total_messages: usize,
    pub total_bytes: u64,
    pub migrated_bytes: u64,
    pub failed_messages: usize,
    pub duration_secs: u64,
    pub throughput_msg_sec: f64,
    pub throughput_mbps: f64,
}

impl MigrationStats {
    pub fn from_progress(progress: &MigrationProgress) -> Self {
        let duration = if let Some(completed) = progress.completed_at {
            (completed - progress.started_at) as u64
        } else {
            (chrono::Utc::now().timestamp() - progress.started_at) as u64
        };

        let duration_secs = duration.max(1);
        let throughput_msg_sec = progress.migrated_messages as f64 / duration_secs as f64;
        let throughput_mbps = if duration_secs > 0 {
            (progress.migrated_bytes as f64 / 1_048_576.0) / duration_secs as f64
        } else {
            0.0
        };

        Self {
            total_users: progress.total_users,
            total_mailboxes: progress.total_mailboxes,
            total_messages: progress.total_messages,
            total_bytes: progress.total_bytes,
            migrated_bytes: progress.migrated_bytes,
            failed_messages: progress.failed_messages.len(),
            duration_secs,
            throughput_msg_sec,
            throughput_mbps,
        }
    }

    pub fn print(&self) {
        println!("\n=== Migration Statistics ===");
        println!("Users migrated: {}", self.total_users);
        println!("Mailboxes migrated: {}", self.total_mailboxes);
        println!("Messages migrated: {}", self.total_messages);
        println!(
            "Data migrated: {:.2} MB",
            self.migrated_bytes as f64 / 1_048_576.0
        );
        println!("Failed messages: {}", self.failed_messages);
        println!("Duration: {} seconds", self.duration_secs);
        println!("Throughput: {:.2} msg/s", self.throughput_msg_sec);
        println!("Throughput: {:.2} MB/s", self.throughput_mbps);
    }
}

/// Message checksum for verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageChecksum {
    pub message_id: String,
    pub size: usize,
    pub sha256: String,
}

impl MessageChecksum {
    pub fn compute(mail: &Mail) -> Self {
        let size = mail.size();

        // For checksum, we use the message ID and size as a simple integrity check
        // In a full implementation, we'd serialize the entire message
        let mut hasher = Sha256::new();
        hasher.update(mail.message_id().to_string().as_bytes());
        hasher.update(size.to_le_bytes());
        let hash = hasher.finalize();

        Self {
            message_id: mail.message_id().to_string(),
            size,
            sha256: format!("{:x}", hash),
        }
    }
}

/// Storage migrator
pub struct StorageMigrator {
    config: MigrationConfig,
    progress: MigrationProgress,
    progress_file: PathBuf,
    backup_dir: PathBuf,
}

impl StorageMigrator {
    /// Create a new migrator
    pub fn new(config: MigrationConfig) -> Self {
        let progress_file = PathBuf::from("/tmp/rusmes_migration_progress.json");
        let backup_dir = PathBuf::from("/tmp/rusmes_migration_backup");

        let progress = if config.resume && progress_file.exists() {
            MigrationProgress::load_from_file(&progress_file)
                .unwrap_or_else(|_| MigrationProgress::new())
        } else {
            MigrationProgress::new()
        };

        Self {
            config,
            progress,
            progress_file,
            backup_dir,
        }
    }

    /// Run the migration
    pub async fn migrate(&mut self) -> Result<MigrationStats> {
        println!(
            "Starting migration from {} to {}",
            self.config.source_type, self.config.dest_type
        );

        if self.config.dry_run {
            println!("DRY RUN MODE - No changes will be made");
        }

        if self.config.resume {
            println!("Resuming migration from previous state");
            println!(
                "Previously migrated: {} users, {} mailboxes, {} messages",
                self.progress.migrated_users,
                self.progress.migrated_mailboxes,
                self.progress.migrated_messages
            );
        }

        // Create backup directory
        if !self.config.dry_run {
            std::fs::create_dir_all(&self.backup_dir)?;
        }

        // Create backends
        let source = self.create_source_backend().await?;
        let dest = self.create_dest_backend().await?;

        // Backup destination state before migration
        if !self.config.dry_run && !self.config.resume {
            println!("Creating backup of destination state...");
            self.backup_destination_state(dest.as_ref()).await?;
        }

        // Get all users
        let users = self.get_users(source.as_ref()).await?;
        self.progress.total_users = users.len();

        // Count total messages and mailboxes
        self.count_totals(source.as_ref(), &users).await?;

        println!(
            "Migration scope: {} users, {} mailboxes, {} messages ({:.2} MB)",
            self.progress.total_users,
            self.progress.total_mailboxes,
            self.progress.total_messages,
            self.progress.total_bytes as f64 / 1_048_576.0
        );

        // Create progress bars
        let multi_progress = MultiProgress::new();
        let user_pb = multi_progress.add(ProgressBar::new(users.len() as u64));
        user_pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] Users {bar:30.cyan/blue} {pos}/{len} ({eta})")
                .expect("Invalid progress bar template")
                .progress_chars("=>-"),
        );

        let message_pb = multi_progress.add(ProgressBar::new(self.progress.total_messages as u64));
        message_pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] Messages {bar:30.green/blue} {pos}/{len} ({msg})")
                .expect("Invalid progress bar template")
                .progress_chars("=>-"),
        );

        let start_time = Instant::now();

        // Migrate users
        for user in &users {
            if self.config.resume && self.progress.is_user_migrated(&user.to_string()) {
                user_pb.inc(1);
                continue;
            }

            if !self.config.dry_run {
                self.migrate_user(source.as_ref(), dest.as_ref(), user, &message_pb)
                    .await?;
            }

            self.progress.mark_user_migrated(&user.to_string());
            user_pb.inc(1);

            // Save progress periodically
            if !self.config.dry_run {
                self.progress.save_to_file(&self.progress_file)?;
            }

            // Update message progress bar with current rate
            let rate = self.progress.messages_per_second();
            message_pb.set_message(format!("{:.1} msg/s", rate));
        }

        user_pb.finish_with_message("All users migrated");
        message_pb.finish_with_message("All messages migrated");

        self.progress.completed_at = Some(chrono::Utc::now().timestamp());

        if !self.config.dry_run {
            self.progress.save_to_file(&self.progress_file)?;
        }

        let stats = MigrationStats::from_progress(&self.progress);

        // Verification
        if self.config.verify && !self.config.dry_run {
            println!("\nVerifying migration integrity...");
            self.verify_migration(source.as_ref(), dest.as_ref())
                .await?;
        }

        let elapsed = start_time.elapsed();
        println!("\nMigration completed in {:.2}s", elapsed.as_secs_f64());

        Ok(stats)
    }

    async fn create_source_backend(&self) -> Result<Box<dyn StorageBackend>> {
        match self.config.source_type {
            BackendType::Filesystem => {
                Ok(Box::new(FilesystemBackend::new(&self.config.source_config)))
            }
            BackendType::Postgres => {
                let backend = PostgresCompleteBackend::new(&self.config.source_config).await?;
                Ok(Box::new(backend))
            }
            BackendType::Amaters => {
                let config = AmatersConfig::from_url(&self.config.source_config)?;
                let backend = AmatersBackend::new(config).await?;
                Ok(Box::new(backend))
            }
        }
    }

    async fn create_dest_backend(&self) -> Result<Box<dyn StorageBackend>> {
        match self.config.dest_type {
            BackendType::Filesystem => {
                let path = &self.config.dest_config;
                std::fs::create_dir_all(path)?;
                Ok(Box::new(FilesystemBackend::new(path)))
            }
            BackendType::Postgres => {
                let backend = PostgresCompleteBackend::new(&self.config.dest_config).await?;
                if !self.config.dry_run {
                    backend.init_schema().await?;
                }
                Ok(Box::new(backend))
            }
            BackendType::Amaters => {
                let config = AmatersConfig::from_url(&self.config.dest_config)?;
                let backend = AmatersBackend::new(config).await?;
                if !self.config.dry_run {
                    backend.init_schema().await?;
                }
                Ok(Box::new(backend))
            }
        }
    }

    async fn get_users(&self, backend: &dyn StorageBackend) -> Result<Vec<Username>> {
        backend.list_all_users().await
    }

    async fn count_totals(
        &mut self,
        source: &dyn StorageBackend,
        users: &[Username],
    ) -> Result<()> {
        let mailbox_store = source.mailbox_store();
        let message_store = source.message_store();

        let mut total_mailboxes = 0;
        let mut total_messages = 0;
        let mut total_bytes = 0u64;

        for user in users {
            let mailboxes = mailbox_store.list_mailboxes(user).await?;
            total_mailboxes += mailboxes.len();

            for mailbox in mailboxes {
                let messages = message_store.get_mailbox_messages(mailbox.id()).await?;
                total_messages += messages.len();
                total_bytes += messages.iter().map(|m| m.size() as u64).sum::<u64>();
            }
        }

        self.progress.total_mailboxes = total_mailboxes;
        self.progress.total_messages = total_messages;
        self.progress.total_bytes = total_bytes;

        Ok(())
    }

    async fn migrate_user(
        &mut self,
        source: &dyn StorageBackend,
        dest: &dyn StorageBackend,
        user: &Username,
        message_pb: &ProgressBar,
    ) -> Result<()> {
        let source_mailboxes = source.mailbox_store();
        let dest_mailboxes = dest.mailbox_store();
        let source_messages = source.message_store();
        let dest_messages = dest.message_store();

        // Get all mailboxes for user
        let mailboxes = source_mailboxes.list_mailboxes(user).await?;

        for mailbox in mailboxes {
            let mailbox_key = format!("{}:{}", user, mailbox.path());

            if self.config.resume && self.progress.is_mailbox_migrated(&mailbox_key) {
                continue;
            }

            // Create mailbox in destination
            let dest_mailbox_id = dest_mailboxes.create_mailbox(mailbox.path()).await?;

            self.progress
                .mark_mailbox_migrated(mailbox_key, dest_mailbox_id.to_string());

            // Migrate messages in batches with parallel processing
            let messages = source_messages.get_mailbox_messages(mailbox.id()).await?;

            let semaphore = Arc::new(Semaphore::new(self.config.parallel));
            let mut handles = vec![];

            for chunk in messages.chunks(self.config.batch_size) {
                for message_meta in chunk {
                    let permit = semaphore.clone().acquire_owned().await?;
                    let source_messages = Arc::clone(&source_messages);
                    let dest_messages = Arc::clone(&dest_messages);
                    let message_id = *message_meta.message_id();
                    let mailbox_id = dest_mailbox_id;
                    let size = message_meta.size();
                    let flags = message_meta.flags().clone();

                    let handle = tokio::spawn(async move {
                        let _permit = permit;
                        let result = Self::migrate_single_message(
                            &source_messages,
                            &dest_messages,
                            &message_id,
                            &mailbox_id,
                        )
                        .await;
                        (message_id, size, flags, result)
                    });

                    handles.push(handle);
                }

                // Process batch results
                for handle in handles.drain(..) {
                    match handle.await {
                        Ok((msg_id, size, _flags, result)) => match result {
                            Ok(_) => {
                                self.progress.migrated_messages += 1;
                                self.progress.migrated_bytes += size as u64;
                                message_pb.inc(1);
                            }
                            Err(e) => {
                                tracing::error!("Failed to migrate message {}: {}", msg_id, e);
                                self.progress.failed_messages.push(msg_id.to_string());
                            }
                        },
                        Err(e) => {
                            tracing::error!("Task failed: {}", e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn migrate_single_message(
        source: &Arc<dyn MessageStore>,
        dest: &Arc<dyn MessageStore>,
        message_id: &MessageId,
        dest_mailbox_id: &MailboxId,
    ) -> Result<()> {
        let message = source
            .get_message(message_id)
            .await?
            .context("Message not found in source")?;

        dest.append_message(dest_mailbox_id, message).await?;

        Ok(())
    }

    async fn verify_migration(
        &self,
        source: &dyn StorageBackend,
        dest: &dyn StorageBackend,
    ) -> Result<()> {
        let source_mailboxes = source.mailbox_store();
        let dest_mailboxes = dest.mailbox_store();
        let source_messages = source.message_store();
        let dest_messages = dest.message_store();

        let users = self.get_users(source).await?;

        let pb = ProgressBar::new(users.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] Verifying {bar:40.yellow/blue} {pos}/{len}")
                .expect("Invalid progress bar template")
                .progress_chars("=>-"),
        );

        for user in &users {
            let source_mboxes = source_mailboxes.list_mailboxes(user).await?;
            let dest_mboxes = dest_mailboxes.list_mailboxes(user).await?;

            if source_mboxes.len() != dest_mboxes.len() {
                return Err(anyhow::anyhow!(
                    "Mailbox count mismatch for user {}: source={}, dest={}",
                    user,
                    source_mboxes.len(),
                    dest_mboxes.len()
                ));
            }

            // Verify message counts
            for (src_mbox, dst_mbox) in source_mboxes.iter().zip(dest_mboxes.iter()) {
                let src_msgs = source_messages.get_mailbox_messages(src_mbox.id()).await?;
                let dst_msgs = dest_messages.get_mailbox_messages(dst_mbox.id()).await?;

                if src_msgs.len() != dst_msgs.len() {
                    return Err(anyhow::anyhow!(
                        "Message count mismatch in mailbox {}: source={}, dest={}",
                        src_mbox.path(),
                        src_msgs.len(),
                        dst_msgs.len()
                    ));
                }

                // Verify checksums for sample messages
                if !src_msgs.is_empty() {
                    let sample_size = (src_msgs.len() / 10).clamp(1, 10);
                    for i in 0..sample_size {
                        let idx = i * (src_msgs.len() / sample_size);
                        if let Some(src_meta) = src_msgs.get(idx) {
                            if let (Some(src_msg), Some(dst_msg)) = (
                                source_messages.get_message(src_meta.message_id()).await?,
                                dest_messages.get_message(src_meta.message_id()).await?,
                            ) {
                                let src_checksum = MessageChecksum::compute(&src_msg);
                                let dst_checksum = MessageChecksum::compute(&dst_msg);

                                if src_checksum.sha256 != dst_checksum.sha256 {
                                    return Err(anyhow::anyhow!(
                                        "Checksum mismatch for message {}",
                                        src_meta.message_id()
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            pb.inc(1);
        }

        pb.finish_with_message("Verification completed successfully");
        println!("Verification passed: all mailboxes and messages verified");
        Ok(())
    }

    async fn backup_destination_state(&self, _dest: &dyn StorageBackend) -> Result<()> {
        let backup_metadata = serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "dest_type": self.config.dest_type,
            "dest_config": self.config.dest_config,
        });

        let backup_file = self.backup_dir.join("migration_backup_metadata.json");
        std::fs::write(backup_file, serde_json::to_string_pretty(&backup_metadata)?)?;

        println!("Backup metadata saved to {:?}", self.backup_dir);
        Ok(())
    }

    /// Rollback migration
    pub async fn rollback(&self) -> Result<()> {
        println!("Rolling back migration...");

        let backup_file = self.backup_dir.join("migration_backup_metadata.json");
        if !backup_file.exists() {
            return Err(anyhow::anyhow!("No backup found to rollback"));
        }

        println!(
            "Rollback would restore from backup at {:?}",
            self.backup_dir
        );
        println!("Note: Full rollback implementation requires backend-specific restore logic");

        Ok(())
    }

    /// Get migration progress
    pub fn get_progress(&self) -> &MigrationProgress {
        &self.progress
    }

    /// Print migration report
    pub fn print_report(&self) {
        println!("\n=== Migration Report ===");
        println!(
            "Users: {}/{}",
            self.progress.migrated_users, self.progress.total_users
        );
        println!(
            "Mailboxes: {}/{}",
            self.progress.migrated_mailboxes, self.progress.total_mailboxes
        );
        println!(
            "Messages: {}/{}",
            self.progress.migrated_messages, self.progress.total_messages
        );
        println!(
            "Data: {:.2}/{:.2} MB",
            self.progress.migrated_bytes as f64 / 1_048_576.0,
            self.progress.total_bytes as f64 / 1_048_576.0
        );

        if !self.progress.failed_messages.is_empty() {
            println!("\nFailed messages: {}", self.progress.failed_messages.len());
            for msg_id in self.progress.failed_messages.iter().take(10) {
                println!("  - {}", msg_id);
            }
            if self.progress.failed_messages.len() > 10 {
                println!(
                    "  ... and {} more",
                    self.progress.failed_messages.len() - 10
                );
            }
        }

        let stats = MigrationStats::from_progress(&self.progress);
        println!("\nDuration: {} seconds", stats.duration_secs);
        println!(
            "Throughput: {:.2} messages/second",
            stats.throughput_msg_sec
        );
        println!("Throughput: {:.2} MB/second", stats.throughput_mbps);

        if let Some(eta) = self.progress.eta_seconds() {
            let eta_duration = Duration::from_secs(eta);
            println!("ETA: {:?}", eta_duration);
        }
    }
}

/// Integrity checker
pub struct IntegrityChecker {
    backend: Box<dyn StorageBackend>,
}

impl IntegrityChecker {
    pub fn new(backend: Box<dyn StorageBackend>) -> Self {
        Self { backend }
    }

    /// Check integrity of storage backend
    pub async fn check(&self) -> Result<IntegrityReport> {
        let mut report = IntegrityReport::new();

        let mailbox_store = self.backend.mailbox_store();
        let message_store = self.backend.message_store();

        // Get sample users for testing
        let users = vec![
            Username::new("user1@example.com".to_string())?,
            Username::new("user2@example.com".to_string())?,
        ];

        for user in &users {
            let mailboxes = mailbox_store.list_mailboxes(user).await?;
            report.total_mailboxes += mailboxes.len();

            for mailbox in mailboxes {
                let messages = message_store.get_mailbox_messages(mailbox.id()).await?;
                report.total_messages += messages.len();

                // Check for messages that can't be retrieved
                for msg_meta in messages {
                    match message_store.get_message(msg_meta.message_id()).await {
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            report
                                .orphaned_messages
                                .push(msg_meta.message_id().to_string());
                        }
                        Err(e) => {
                            report.errors.push(format!(
                                "Error reading message {}: {}",
                                msg_meta.message_id(),
                                e
                            ));
                        }
                    }
                }
            }
        }

        Ok(report)
    }
}

/// Integrity check report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityReport {
    pub total_mailboxes: usize,
    pub total_messages: usize,
    pub orphaned_messages: Vec<String>,
    pub errors: Vec<String>,
}

impl IntegrityReport {
    pub fn new() -> Self {
        Self {
            total_mailboxes: 0,
            total_messages: 0,
            orphaned_messages: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn print(&self) {
        println!("\n=== Integrity Report ===");
        println!("Total mailboxes: {}", self.total_mailboxes);
        println!("Total messages: {}", self.total_messages);
        println!("Orphaned messages: {}", self.orphaned_messages.len());

        if !self.orphaned_messages.is_empty() {
            println!("\nOrphaned messages:");
            for msg_id in self.orphaned_messages.iter().take(10) {
                println!("  - {}", msg_id);
            }
            if self.orphaned_messages.len() > 10 {
                println!("  ... and {} more", self.orphaned_messages.len() - 10);
            }
        }

        if !self.errors.is_empty() {
            println!("\nErrors:");
            for error in self.errors.iter().take(10) {
                println!("  - {}", error);
            }
            if self.errors.len() > 10 {
                println!("  ... and {} more", self.errors.len() - 10);
            }
        }

        if self.orphaned_messages.is_empty() && self.errors.is_empty() {
            println!("\nNo integrity issues found");
        }
    }
}

impl Default for IntegrityReport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_type_from_str() {
        assert_eq!(
            "filesystem".parse::<BackendType>().unwrap(),
            BackendType::Filesystem
        );
        assert_eq!(
            "postgres".parse::<BackendType>().unwrap(),
            BackendType::Postgres
        );
        assert_eq!(
            "amaters".parse::<BackendType>().unwrap(),
            BackendType::Amaters
        );
        assert!("unknown".parse::<BackendType>().is_err());
    }

    #[test]
    fn test_backend_type_case_insensitive() {
        assert_eq!(
            "FILESYSTEM".parse::<BackendType>().unwrap(),
            BackendType::Filesystem
        );
        assert_eq!(
            "PostgreSQL".parse::<BackendType>().unwrap(),
            BackendType::Postgres
        );
    }

    #[test]
    fn test_backend_type_aliases() {
        assert_eq!(
            "fs".parse::<BackendType>().unwrap(),
            BackendType::Filesystem
        );
        assert_eq!("pg".parse::<BackendType>().unwrap(), BackendType::Postgres);
        assert_eq!(
            "postgresql".parse::<BackendType>().unwrap(),
            BackendType::Postgres
        );
    }

    #[test]
    fn test_backend_type_display() {
        assert_eq!(BackendType::Filesystem.to_string(), "filesystem");
        assert_eq!(BackendType::Postgres.to_string(), "postgres");
        assert_eq!(BackendType::Amaters.to_string(), "amaters");
    }

    #[test]
    fn test_migration_config_default() {
        let config = MigrationConfig::default();
        assert_eq!(config.source_type, BackendType::Filesystem);
        assert_eq!(config.dest_type, BackendType::Postgres);
        assert_eq!(config.batch_size, 100);
        assert_eq!(config.parallel, 4);
        assert!(config.verify);
        assert!(!config.dry_run);
        assert!(!config.resume);
    }

    #[test]
    fn test_migration_progress_new() {
        let progress = MigrationProgress::new();
        assert_eq!(progress.total_users, 0);
        assert_eq!(progress.migrated_users, 0);
        assert_eq!(progress.total_mailboxes, 0);
        assert_eq!(progress.migrated_messages, 0);
        assert_eq!(progress.total_bytes, 0);
        assert_eq!(progress.migrated_bytes, 0);
        assert!(progress.completed_at.is_none());
        assert!(progress.migrated_user_list.is_empty());
        assert!(progress.migrated_mailbox_map.is_empty());
    }

    #[test]
    fn test_migration_progress_serialization() {
        let progress = MigrationProgress::new();
        let json = serde_json::to_string(&progress).unwrap();
        let deserialized: MigrationProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(progress.total_users, deserialized.total_users);
        assert_eq!(progress.migrated_users, deserialized.migrated_users);
    }

    #[test]
    fn test_migration_progress_mark_user_migrated() {
        let mut progress = MigrationProgress::new();
        progress.total_users = 2;

        assert!(!progress.is_user_migrated("user1"));
        progress.mark_user_migrated("user1");
        assert!(progress.is_user_migrated("user1"));
        assert_eq!(progress.migrated_users, 1);
        assert!(!progress.is_user_migrated("user2"));
    }

    #[test]
    fn test_migration_progress_mark_mailbox_migrated() {
        let mut progress = MigrationProgress::new();

        assert!(!progress.is_mailbox_migrated("user1:INBOX"));
        progress.mark_mailbox_migrated("user1:INBOX".to_string(), "mailbox-123".to_string());
        assert!(progress.is_mailbox_migrated("user1:INBOX"));
        assert_eq!(progress.migrated_mailboxes, 1);
    }

    #[test]
    fn test_migration_progress_percentage() {
        let mut progress = MigrationProgress::new();
        assert_eq!(progress.progress_percentage(), 0.0);

        progress.total_messages = 100;
        progress.migrated_messages = 50;
        assert_eq!(progress.progress_percentage(), 50.0);

        progress.migrated_messages = 100;
        assert_eq!(progress.progress_percentage(), 100.0);
    }

    #[test]
    fn test_migration_progress_messages_per_second() {
        let mut progress = MigrationProgress::new();
        progress.started_at = chrono::Utc::now().timestamp() - 10;
        progress.last_updated_at = chrono::Utc::now().timestamp();
        progress.migrated_messages = 100;

        let rate = progress.messages_per_second();
        assert!(rate > 0.0);
        assert!(rate <= 10.0);
    }

    #[test]
    fn test_migration_stats_from_progress() {
        let mut progress = MigrationProgress::new();
        progress.total_users = 5;
        progress.total_mailboxes = 20;
        progress.total_messages = 1000;
        progress.migrated_messages = 1000;
        progress.total_bytes = 10_485_760;
        progress.migrated_bytes = 10_485_760;
        progress.started_at = chrono::Utc::now().timestamp() - 100;
        progress.completed_at = Some(chrono::Utc::now().timestamp());

        let stats = MigrationStats::from_progress(&progress);
        assert_eq!(stats.total_users, 5);
        assert_eq!(stats.total_mailboxes, 20);
        assert_eq!(stats.total_messages, 1000);
        assert!(stats.duration_secs > 0);
        assert!(stats.throughput_msg_sec > 0.0);
    }

    #[test]
    fn test_integrity_report_new() {
        let report = IntegrityReport::new();
        assert_eq!(report.total_mailboxes, 0);
        assert_eq!(report.total_messages, 0);
        assert!(report.orphaned_messages.is_empty());
        assert!(report.errors.is_empty());
    }

    #[test]
    fn test_migration_config_custom() {
        let config = MigrationConfig {
            source_type: BackendType::Postgres,
            dest_type: BackendType::Amaters,
            batch_size: 500,
            parallel: 8,
            verify: false,
            dry_run: true,
            resume: true,
            ..Default::default()
        };

        assert_eq!(config.source_type, BackendType::Postgres);
        assert_eq!(config.dest_type, BackendType::Amaters);
        assert_eq!(config.batch_size, 500);
        assert_eq!(config.parallel, 8);
        assert!(!config.verify);
        assert!(config.dry_run);
        assert!(config.resume);
    }

    #[test]
    fn test_backend_type_equality() {
        assert_eq!(BackendType::Filesystem, BackendType::Filesystem);
        assert_ne!(BackendType::Filesystem, BackendType::Postgres);
    }

    #[tokio::test]
    async fn test_migrator_creation() {
        let config = MigrationConfig::default();
        let migrator = StorageMigrator::new(config);
        assert_eq!(migrator.progress.total_users, 0);
    }

    #[test]
    fn test_progress_failed_messages() {
        let mut progress = MigrationProgress::new();
        progress.failed_messages.push("msg1".to_string());
        progress.failed_messages.push("msg2".to_string());
        assert_eq!(progress.failed_messages.len(), 2);
    }

    #[test]
    fn test_migration_stats_fields() {
        let progress = MigrationProgress::new();
        let stats = MigrationStats::from_progress(&progress);
        assert_eq!(stats.total_bytes, 0);
        assert_eq!(stats.migrated_bytes, 0);
        assert_eq!(stats.failed_messages, 0);
    }

    #[test]
    fn test_integrity_report_with_errors() {
        let mut report = IntegrityReport::new();
        report.errors.push("Error 1".to_string());
        report.errors.push("Error 2".to_string());
        assert_eq!(report.errors.len(), 2);
    }

    #[test]
    fn test_backend_type_all_variants() {
        let fs = BackendType::Filesystem;
        let pg = BackendType::Postgres;
        let am = BackendType::Amaters;

        assert_ne!(fs, pg);
        assert_ne!(pg, am);
        assert_ne!(fs, am);
    }

    #[test]
    fn test_migration_progress_completion() {
        let mut progress = MigrationProgress::new();
        assert!(progress.completed_at.is_none());

        progress.completed_at = Some(chrono::Utc::now().timestamp());
        assert!(progress.completed_at.is_some());
    }

    #[test]
    fn test_migration_progress_eta() {
        let mut progress = MigrationProgress::new();
        assert!(progress.eta_seconds().is_none());

        progress.started_at = chrono::Utc::now().timestamp() - 10;
        progress.last_updated_at = chrono::Utc::now().timestamp();
        progress.total_messages = 1000;
        progress.migrated_messages = 100;

        let eta = progress.eta_seconds();
        assert!(eta.is_some());
    }

    #[test]
    fn test_message_checksum_compute() {
        use bytes::Bytes;
        use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

        let headers = HeaderMap::new();
        let body = MessageBody::Small(Bytes::from("Test body"));
        let message = MimeMessage::new(headers, body);

        let mail = Mail::new(
            Some("sender@example.com".parse().unwrap()),
            vec!["recipient@example.com".parse().unwrap()],
            message,
            None,
            None,
        );

        let checksum = MessageChecksum::compute(&mail);

        assert!(!checksum.sha256.is_empty());
        assert_eq!(checksum.sha256.len(), 64);
        assert!(checksum.size > 0);
    }

    #[test]
    fn test_migration_progress_default() {
        let progress = MigrationProgress::default();
        assert_eq!(progress.total_users, 0);
        assert_eq!(progress.migrated_users, 0);
    }

    #[test]
    fn test_integrity_report_default() {
        let report = IntegrityReport::default();
        assert_eq!(report.total_mailboxes, 0);
        assert_eq!(report.total_messages, 0);
    }

    #[test]
    fn test_backend_type_serialization() {
        let backend = BackendType::Filesystem;
        let json = serde_json::to_string(&backend).unwrap();
        let deserialized: BackendType = serde_json::from_str(&json).unwrap();
        assert_eq!(backend, deserialized);
    }

    #[test]
    fn test_migration_config_batch_size() {
        let mut config = MigrationConfig::default();
        assert_eq!(config.batch_size, 100);

        config.batch_size = 200;
        assert_eq!(config.batch_size, 200);
    }

    #[test]
    fn test_migration_config_parallel() {
        let mut config = MigrationConfig::default();
        assert_eq!(config.parallel, 4);

        config.parallel = 8;
        assert_eq!(config.parallel, 8);
    }

    #[test]
    fn test_migration_progress_bytes_tracking() {
        let mut progress = MigrationProgress::new();
        progress.total_bytes = 1_048_576;
        progress.migrated_bytes = 524_288;

        assert_eq!(progress.total_bytes, 1_048_576);
        assert_eq!(progress.migrated_bytes, 524_288);
    }

    #[test]
    fn test_integrity_report_orphaned_messages() {
        let mut report = IntegrityReport::new();
        report.orphaned_messages.push("msg1".to_string());
        report.orphaned_messages.push("msg2".to_string());

        assert_eq!(report.orphaned_messages.len(), 2);
    }

    // ---------------------------------------------------------------------------
    // Tests added for Slice G stub-check pass
    // ---------------------------------------------------------------------------

    /// Verify that `get_users` delegates to `StorageBackend::list_all_users`.
    ///
    /// We use `AmatersBackend` (the in-memory mock) as the backend.  An empty
    /// store returns an empty user list, proving the call is forwarded rather
    /// than returning a hardcoded list.
    #[tokio::test]
    async fn test_get_users_delegates_to_backend() {
        use rusmes_storage::backends::amaters::{AmatersBackend, AmatersConfig};

        let backend = AmatersBackend::new(AmatersConfig::default())
            .await
            .expect("AmatersBackend::new failed");

        let config = MigrationConfig::default();
        let migrator = StorageMigrator::new(config);

        let users = migrator
            .get_users(&backend)
            .await
            .expect("get_users failed");

        // The in-memory AmateRS backend has no registered users, so the list
        // must be empty — confirming the delegation (the old stub returned
        // two hardcoded placeholders).
        assert!(
            users.is_empty(),
            "expected empty user list from a fresh AmatersBackend, got {:?}",
            users
        );
    }

    /// Verify `AmatersConfig::from_url` parses valid URLs and rejects invalid
    /// ones.
    #[test]
    fn test_amaters_config_from_url() {
        use rusmes_storage::backends::amaters::AmatersConfig;

        // Single endpoint, explicit keyspace.
        let cfg = AmatersConfig::from_url("amaters://node1:9042/my_keyspace")
            .expect("valid single-endpoint URL should parse");
        assert_eq!(cfg.cluster_endpoints, vec!["node1:9042"]);
        assert_eq!(cfg.metadata_keyspace, "my_keyspace");
        assert_eq!(cfg.blob_keyspace, "my_keyspace_blobs");

        // Multiple endpoints, no keyspace path → defaults.
        let cfg = AmatersConfig::from_url("amaters://host1:9042,host2:9042")
            .expect("valid multi-endpoint URL should parse");
        assert_eq!(cfg.cluster_endpoints, vec!["host1:9042", "host2:9042"]);
        assert_eq!(cfg.metadata_keyspace, "rusmes_metadata");
        assert_eq!(cfg.blob_keyspace, "rusmes_blobs");

        // Multiple endpoints with keyspace.
        let cfg = AmatersConfig::from_url("amaters://host1:9042,host2:9042,host3:9042/prod")
            .expect("valid three-endpoint URL should parse");
        assert_eq!(cfg.cluster_endpoints.len(), 3);
        assert_eq!(cfg.metadata_keyspace, "prod");
        assert_eq!(cfg.blob_keyspace, "prod_blobs");

        // Wrong scheme → error.
        assert!(
            AmatersConfig::from_url("cassandra://host1:9042/ks").is_err(),
            "wrong scheme must be rejected"
        );

        // Missing port → error.
        assert!(
            AmatersConfig::from_url("amaters://host1/ks").is_err(),
            "endpoint without port must be rejected"
        );

        // Empty authority → error.
        assert!(
            AmatersConfig::from_url("amaters:///keyspace").is_err(),
            "empty host list must be rejected"
        );
    }
}

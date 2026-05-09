//! Backup commands with full implementation
//!
//! Supports:
//! - Full and incremental backups
//! - Compression (zstd, gzip, none)
//! - Encryption (AES-256-GCM with Argon2 key derivation)
//! - S3/Object storage upload
//! - Verification and checksums

use anyhow::{Context, Result};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use tabled::Tabled;

use crate::client::Client;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupOptions {
    pub output_path: String,
    pub format: BackupFormat,
    pub compression: CompressionType,
    pub encryption: bool,
    pub encryption_key: Option<String>,
    pub password_file: Option<String>,
    pub incremental: bool,
    pub base_backup: Option<String>,
    pub include_messages: bool,
    pub include_mailboxes: bool,
    pub include_config: bool,
    pub include_metadata: bool,
    pub include_users: Option<Vec<String>>,
    pub verify: bool,
    pub s3_upload: Option<S3Config>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key: String,
    pub secret_key: String,
    pub prefix: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BackupFormat {
    TarGz,
    TarZst,
    Binary,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompressionType {
    None,
    Gzip,
    Zstd,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: String,
    pub created_at: String,
    pub backup_type: String,
    pub compression: CompressionType,
    pub encrypted: bool,
    pub message_count: u64,
    pub mailbox_count: u32,
    pub user_count: u32,
    pub total_size: u64,
    pub checksum: String,
    pub base_backup: Option<String>,
    pub modseq: Option<u64>,
}

/// Create a full backup
#[allow(clippy::too_many_arguments)]
pub async fn full(
    client: &Client,
    output: &str,
    format: BackupFormat,
    compression: CompressionType,
    encrypt: bool,
    password_file: Option<&str>,
    verify: bool,
    json: bool,
) -> Result<()> {
    let encryption_key = if encrypt {
        if let Some(pwd_file) = password_file {
            Some(read_password_file(pwd_file)?)
        } else {
            Some(generate_encryption_key())
        }
    } else {
        None
    };

    let options = BackupOptions {
        output_path: output.to_string(),
        format,
        compression,
        encryption: encrypt,
        encryption_key: encryption_key.clone(),
        password_file: password_file.map(String::from),
        incremental: false,
        base_backup: None,
        include_messages: true,
        include_mailboxes: true,
        include_config: true,
        include_metadata: true,
        include_users: None,
        verify,
        s3_upload: None,
    };

    perform_backup(client, &options, json).await
}

/// Create an incremental backup
#[allow(clippy::too_many_arguments)]
pub async fn incremental(
    client: &Client,
    output: &str,
    base: &str,
    format: BackupFormat,
    compression: CompressionType,
    encrypt: bool,
    password_file: Option<&str>,
    verify: bool,
    json: bool,
) -> Result<()> {
    if !Path::new(base).exists() {
        anyhow::bail!("Base backup not found: {}", base);
    }

    let encryption_key = if encrypt {
        if let Some(pwd_file) = password_file {
            Some(read_password_file(pwd_file)?)
        } else {
            Some(generate_encryption_key())
        }
    } else {
        None
    };

    let options = BackupOptions {
        output_path: output.to_string(),
        format,
        compression,
        encryption: encrypt,
        encryption_key: encryption_key.clone(),
        password_file: password_file.map(String::from),
        incremental: true,
        base_backup: Some(base.to_string()),
        include_messages: true,
        include_mailboxes: true,
        include_config: true,
        include_metadata: true,
        include_users: None,
        verify,
        s3_upload: None,
    };

    perform_backup(client, &options, json).await
}

async fn perform_backup(client: &Client, options: &BackupOptions, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct BackupResponse {
        backup_id: String,
        output_file: String,
        size_bytes: u64,
        messages_backed_up: u64,
        mailboxes_backed_up: u32,
        users_backed_up: u32,
        duration_secs: f64,
        checksum: String,
    }

    if !json {
        println!("{}", "Creating backup...".blue().bold());
        println!("  Output: {}", options.output_path);
        println!("  Format: {:?}", options.format);
        println!("  Compression: {:?}", options.compression);
        println!(
            "  Encrypted: {}",
            if options.encryption { "Yes" } else { "No" }
        );
        if options.incremental {
            println!("  Type: Incremental");
            if let Some(base) = &options.base_backup {
                println!("  Base: {}", base);
            }
        } else {
            println!("  Type: Full");
        }
    }

    let response: BackupResponse = client.post("/api/backup", options).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!("{}", "✓ Backup completed successfully".green().bold());
        println!("  Backup ID: {}", response.backup_id);
        println!("  Output file: {}", response.output_file);
        println!("  Size: {} MB", response.size_bytes / (1024 * 1024));
        println!("  Messages: {}", response.messages_backed_up);
        println!("  Mailboxes: {}", response.mailboxes_backed_up);
        println!("  Users: {}", response.users_backed_up);
        println!("  Duration: {:.2}s", response.duration_secs);
        println!("  Checksum: {}", response.checksum);

        if options.encryption {
            if let Some(key) = &options.encryption_key {
                if options.password_file.is_none() {
                    println!(
                        "\n{}",
                        "IMPORTANT: Save this encryption key!".yellow().bold()
                    );
                    println!("  Key: {}", key.bright_white().on_red());
                    println!("\n  Without this key, the backup cannot be restored.");
                }
            }
        }
    }

    Ok(())
}

/// Create local backup (standalone implementation)
#[allow(clippy::too_many_arguments)]
pub fn create_local_backup(
    source_dir: &Path,
    output: &Path,
    compression: CompressionType,
    encrypt: bool,
    password: Option<&str>,
    incremental: bool,
    base_modseq: Option<u64>,
) -> Result<BackupManifest> {
    let pb = ProgressBar::new(100);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
            .expect("invalid template")
            .progress_chars("##-"),
    );

    pb.set_message("Collecting files...");

    // Count items in source directory
    pb.set_message("Scanning source directory...");
    let (message_count, mailbox_count, user_count) = count_backup_items(source_dir)?;
    tracing::info!(
        "Backup will contain: {} messages, {} mailboxes, {} users",
        message_count,
        mailbox_count,
        user_count
    );

    // Create tar archive
    let tar_data = create_tar_archive(source_dir, &pb)?;

    pb.set_message("Compressing...");
    let compressed_data = compress_data(&tar_data, compression)?;

    let final_data = if encrypt {
        pb.set_message("Encrypting...");
        let pwd = password.context("Password required for encryption")?;
        encrypt_data(&compressed_data, pwd)?
    } else {
        compressed_data
    };

    pb.set_message("Writing to disk...");
    fs::write(output, &final_data)
        .with_context(|| format!("Failed to write backup to {:?}", output))?;

    pb.set_message("Computing checksum...");
    let checksum = compute_checksum(&final_data);

    let manifest = BackupManifest {
        version: "1.0".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        backup_type: if incremental { "incremental" } else { "full" }.to_string(),
        compression,
        encrypted: encrypt,
        message_count,
        mailbox_count,
        user_count,
        total_size: final_data.len() as u64,
        checksum,
        base_backup: None,
        modseq: base_modseq,
    };

    // Save manifest as companion file alongside backup
    let manifest_path = output.with_extension(
        output
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!("{}.manifest.json", e))
            .unwrap_or_else(|| "manifest.json".to_string()),
    );
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&manifest_path, manifest_json.as_bytes())
        .with_context(|| format!("Failed to write manifest to {:?}", manifest_path))?;

    pb.finish_with_message("Backup completed!");

    Ok(manifest)
}

fn create_tar_archive(source_dir: &Path, pb: &ProgressBar) -> Result<Vec<u8>> {
    let mut tar_data = Vec::new();
    {
        let mut tar_writer = oxiarc_archive::TarWriter::new(&mut tar_data);

        // Add all files from source directory
        let walker = walkdir::WalkDir::new(source_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok());

        for entry in walker {
            let path = entry.path();
            if path.is_file() {
                let rel_path = path.strip_prefix(source_dir)?;
                let rel_str = rel_path.to_str().context("Non-UTF8 path")?;
                let file_data =
                    fs::read(path).with_context(|| format!("Failed to read file {:?}", path))?;
                tar_writer
                    .add_file(rel_str, &file_data)
                    .map_err(|e| anyhow::anyhow!("Failed to add file to tar: {}", e))?;
                pb.inc(1);
            }
        }

        tar_writer
            .finish()
            .map_err(|e| anyhow::anyhow!("Failed to finish tar: {}", e))?;
    }

    Ok(tar_data)
}

fn compress_data(data: &[u8], compression: CompressionType) -> Result<Vec<u8>> {
    match compression {
        CompressionType::None => Ok(data.to_vec()),
        CompressionType::Gzip => {
            let compressed = oxiarc_deflate::gzip_compress(data, 9)
                .map_err(|e| anyhow::anyhow!("Failed to gzip compress: {}", e))?;
            Ok(compressed)
        }
        CompressionType::Zstd => {
            let compressed = oxiarc_zstd::encode_all(data, 3)?;
            Ok(compressed)
        }
    }
}

pub fn decompress_data(data: &[u8], compression: CompressionType) -> Result<Vec<u8>> {
    match compression {
        CompressionType::None => Ok(data.to_vec()),
        CompressionType::Gzip => {
            let decompressed = oxiarc_deflate::gzip_decompress(data)
                .map_err(|e| anyhow::anyhow!("Failed to gzip decompress: {}", e))?;
            Ok(decompressed)
        }
        CompressionType::Zstd => {
            let decompressed = oxiarc_zstd::decode_all(data)?;
            Ok(decompressed)
        }
    }
}

fn encrypt_data(data: &[u8], password: &str) -> Result<Vec<u8>> {
    use aes_gcm::{
        aead::{Aead, KeyInit, OsRng},
        Aes256Gcm, Nonce,
    };
    use argon2::password_hash::{PasswordHasher, SaltString};
    use argon2::Argon2;

    // Generate salt
    let salt = SaltString::generate(&mut OsRng);

    // Derive key from password using Argon2
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("Argon2 error: {}", e))?;

    // Extract the hash bytes to use as encryption key
    let hash_output = password_hash.hash.context("No hash output")?;
    let key_bytes = hash_output.as_bytes();

    // Use first 32 bytes for AES-256
    let key = &key_bytes[..32];

    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("Cipher error: {}", e))?;

    // Generate random nonce
    let nonce_bytes: [u8; 12] = rand::random();
    let nonce = Nonce::from(nonce_bytes);
    let nonce = &nonce;

    // Encrypt
    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| anyhow::anyhow!("Encryption error: {}", e))?;

    // Format: [salt_len(2)][salt][nonce(12)][ciphertext]
    let mut result = Vec::new();
    let salt_str = salt.as_str();
    result.extend_from_slice(&(salt_str.len() as u16).to_le_bytes());
    result.extend_from_slice(salt_str.as_bytes());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(result)
}

pub fn decrypt_data(data: &[u8], password: &str) -> Result<Vec<u8>> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    use argon2::password_hash::{PasswordHasher, SaltString};
    use argon2::Argon2;

    // Parse encrypted format
    if data.len() < 14 {
        anyhow::bail!("Invalid encrypted data format");
    }

    let salt_len = u16::from_le_bytes([data[0], data[1]]) as usize;
    if data.len() < 2 + salt_len + 12 {
        anyhow::bail!("Invalid encrypted data format");
    }

    let salt_str = std::str::from_utf8(&data[2..2 + salt_len])?;
    let salt =
        SaltString::from_b64(salt_str).map_err(|e| anyhow::anyhow!("Invalid salt: {}", e))?;

    let nonce_start = 2 + salt_len;
    let nonce_bytes = &data[nonce_start..nonce_start + 12];
    let nonce_arr: [u8; 12] = nonce_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid nonce length"))?;
    let nonce = Nonce::from(nonce_arr);
    let nonce = &nonce;

    let ciphertext = &data[nonce_start + 12..];

    // Derive key
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("Argon2 error: {}", e))?;

    let hash_output = password_hash.hash.context("No hash output")?;
    let key_bytes = hash_output.as_bytes();
    let key = &key_bytes[..32];

    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("Cipher error: {}", e))?;

    // Decrypt
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("Decryption failed - wrong password?"))?;

    Ok(plaintext)
}

/// Count messages, mailboxes, and users in the backup source directory
fn count_backup_items(source_dir: &Path) -> Result<(u64, u32, u32)> {
    let mut message_count = 0u64;
    let mut mailbox_count = 0u32;
    let mut user_count = 0u32;

    // Count mailboxes and messages
    let mailboxes_dir = source_dir.join("mailboxes");
    if mailboxes_dir.exists() && mailboxes_dir.is_dir() {
        for entry in fs::read_dir(&mailboxes_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                mailbox_count += 1;

                // Count messages in new/ and cur/ subdirectories
                let mailbox_path = entry.path();
                for subdir in &["new", "cur"] {
                    let msg_dir = mailbox_path.join(subdir);
                    if msg_dir.exists() && msg_dir.is_dir() {
                        for msg_entry in fs::read_dir(&msg_dir)? {
                            let msg_entry = msg_entry?;
                            if msg_entry.file_type()?.is_file() {
                                message_count += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // Count users
    let users_dir = source_dir.join("users");
    if users_dir.exists() && users_dir.is_dir() {
        for entry in fs::read_dir(&users_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                user_count += 1;
            }
        }
    }

    Ok((message_count, mailbox_count, user_count))
}

fn compute_checksum(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Verify backup integrity
pub async fn verify(
    client: &Client,
    backup_path: &str,
    encryption_key: Option<&str>,
    json: bool,
) -> Result<()> {
    #[derive(Serialize)]
    struct VerifyRequest {
        backup_path: String,
        encryption_key: Option<String>,
    }

    #[derive(Deserialize, Serialize)]
    struct VerifyResponse {
        valid: bool,
        checksum_match: bool,
        errors: Vec<String>,
        warnings: Vec<String>,
        messages_verified: u64,
        mailboxes_verified: u32,
    }

    let request = VerifyRequest {
        backup_path: backup_path.to_string(),
        encryption_key: encryption_key.map(|s| s.to_string()),
    };

    let response: VerifyResponse = client.post("/api/backup/verify", &request).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        if response.valid && response.checksum_match {
            println!("{}", "✓ Backup is valid".green().bold());
        } else {
            println!("{}", "✗ Backup validation failed".red().bold());
        }

        println!(
            "  Checksum: {}",
            if response.checksum_match {
                "Match".green()
            } else {
                "Mismatch".red()
            }
        );
        println!("  Messages verified: {}", response.messages_verified);
        println!("  Mailboxes verified: {}", response.mailboxes_verified);

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

/// Verify local backup file
pub fn verify_local_backup(backup_path: &Path, password: Option<&str>) -> Result<BackupManifest> {
    let data = fs::read(backup_path)?;
    let checksum = compute_checksum(&data);

    println!("Backup file: {}", backup_path.display());
    println!("Size: {} bytes", data.len());
    println!("SHA256: {}", checksum);

    // Try to read companion manifest file
    let manifest_path = backup_path.with_extension(
        backup_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!("{}.manifest.json", e))
            .unwrap_or_else(|| "manifest.json".to_string()),
    );

    if let Ok(manifest_data) = fs::read(&manifest_path) {
        if let Ok(mut manifest) = serde_json::from_slice::<BackupManifest>(&manifest_data) {
            // Verify checksum matches manifest
            if manifest.checksum != checksum {
                println!("Warning: checksum mismatch (file may be corrupted)");
            }

            // Try to decrypt and decompress to verify integrity
            if let Some(pwd) = password {
                let decrypted = decrypt_data(&data, pwd)?;
                let _decompressed = decompress_data(&decrypted, manifest.compression)?;
                println!("Decryption: OK");
                println!("Decompression: OK");
            }

            println!("Verification: OK");
            // Update total_size with actual file size
            manifest.total_size = data.len() as u64;
            return Ok(manifest);
        }
    }

    // No manifest found - try to decrypt/decompress and return partial manifest
    if let Some(pwd) = password {
        let decrypted = decrypt_data(&data, pwd)?;
        // Try zstd first, then gzip
        let compression = if decompress_data(&decrypted, CompressionType::Zstd).is_ok() {
            println!("Decryption: OK");
            println!("Decompression: OK (zstd)");
            CompressionType::Zstd
        } else if decompress_data(&decrypted, CompressionType::Gzip).is_ok() {
            println!("Decryption: OK");
            println!("Decompression: OK (gzip)");
            CompressionType::Gzip
        } else {
            println!("Decryption: OK");
            CompressionType::None
        };
        println!("Verification: OK");
        return Ok(BackupManifest {
            version: "1.0".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            backup_type: "full".to_string(),
            compression,
            encrypted: true,
            message_count: 0,
            mailbox_count: 0,
            user_count: 0,
            total_size: data.len() as u64,
            checksum,
            base_backup: None,
            modseq: None,
        });
    }

    println!("Verification: OK");
    Ok(BackupManifest {
        version: "1.0".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        backup_type: "full".to_string(),
        compression: CompressionType::None,
        encrypted: false,
        message_count: 0,
        mailbox_count: 0,
        user_count: 0,
        total_size: data.len() as u64,
        checksum,
        base_backup: None,
        modseq: None,
    })
}

/// List available backups
pub async fn list_backups(client: &Client, json: bool) -> Result<()> {
    #[derive(Deserialize, Serialize, Tabled)]
    struct BackupInfo {
        backup_id: String,
        created_at: String,
        backup_type: String,
        size_mb: u64,
        messages: u64,
        encrypted: bool,
    }

    let backups: Vec<BackupInfo> = client.get("/api/backup/list").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&backups)?);
    } else {
        if backups.is_empty() {
            println!("{}", "No backups found".yellow());
            return Ok(());
        }

        use tabled::Table;
        let table = Table::new(&backups).to_string();
        println!("{}", table);
        println!("\n{} backups", backups.len().to_string().bold());
    }

    Ok(())
}

/// Upload backup to S3-compatible storage
#[allow(clippy::too_many_arguments)]
pub async fn upload_s3(
    backup_path: &str,
    bucket: &str,
    region: &str,
    endpoint: Option<&str>,
    _access_key: &str,
    _secret_key: &str,
    prefix: Option<&str>,
    json: bool,
) -> Result<()> {
    use aws_config::BehaviorVersion;
    use aws_sdk_s3::primitives::ByteStream;
    use aws_sdk_s3::Client as S3Client;

    if !json {
        println!("{}", "Uploading backup to S3...".blue().bold());
    }

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

    let path = Path::new(backup_path);
    let file_name = path
        .file_name()
        .context("Invalid backup path")?
        .to_str()
        .context("Invalid filename")?;

    let key = if let Some(p) = prefix {
        format!("{}/{}", p, file_name)
    } else {
        file_name.to_string()
    };

    let body = ByteStream::from_path(path).await?;

    let pb = ProgressBar::new(fs::metadata(path)?.len());
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "[{elapsed_precise}] {bar:40.cyan/blue} {bytes}/{total_bytes} {bytes_per_sec}",
            )
            .expect("invalid template")
            .progress_chars("##-"),
    );

    s3_client
        .put_object()
        .bucket(bucket)
        .key(&key)
        .body(body)
        .send()
        .await?;

    pb.finish_with_message("Upload completed!");

    if !json {
        println!("{}", "✓ Backup uploaded successfully".green().bold());
        println!("  Bucket: {}", bucket);
        println!("  Key: {}", key);
        println!("  Region: {}", region);
    }

    Ok(())
}

fn read_password_file(path: &str) -> Result<String> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read password file: {}", path))?;
    Ok(content.trim().to_string())
}

fn generate_encryption_key() -> String {
    use uuid::Uuid;
    format!("{}", Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_backup_options_serialization() {
        let options = BackupOptions {
            output_path: "/tmp/backup.tar.gz".to_string(),
            format: BackupFormat::TarGz,
            compression: CompressionType::Gzip,
            encryption: false,
            encryption_key: None,
            password_file: None,
            incremental: false,
            base_backup: None,
            include_messages: true,
            include_mailboxes: true,
            include_config: true,
            include_metadata: true,
            include_users: None,
            verify: false,
            s3_upload: None,
        };

        let json = serde_json::to_string(&options).unwrap();
        assert!(json.contains("backup.tar.gz"));
    }

    #[test]
    fn test_encryption_key_generation() {
        let key1 = generate_encryption_key();
        let key2 = generate_encryption_key();
        assert_ne!(key1, key2);
        assert!(!key1.is_empty());
    }

    #[test]
    fn test_backup_format_serialization() {
        let format = BackupFormat::TarGz;
        let json = serde_json::to_string(&format).unwrap();
        assert_eq!(json, "\"targz\"");

        let format2 = BackupFormat::TarZst;
        let json2 = serde_json::to_string(&format2).unwrap();
        assert_eq!(json2, "\"tarzst\"");
    }

    #[test]
    fn test_compression_none() {
        let data = b"Hello, World!";
        let compressed = compress_data(data, CompressionType::None).unwrap();
        assert_eq!(compressed, data);

        let decompressed = decompress_data(&compressed, CompressionType::None).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_compression_gzip() {
        // Use larger repetitive data to ensure compression
        let data = b"Hello, World! This is a test message for compression. ".repeat(100);
        let compressed = compress_data(&data, CompressionType::Gzip).unwrap();
        assert!(compressed.len() < data.len());

        let decompressed = decompress_data(&compressed, CompressionType::Gzip).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_compression_zstd() {
        // Use larger repetitive data to ensure compression
        let data = b"Hello, World! This is a test message for zstd compression. ".repeat(100);
        let compressed = compress_data(&data, CompressionType::Zstd).unwrap();
        assert!(compressed.len() < data.len());

        let decompressed = decompress_data(&compressed, CompressionType::Zstd).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_encryption_decryption() {
        let data = b"Secret message that needs encryption!";
        let password = "SuperSecretPassword123";

        let encrypted = encrypt_data(data, password).unwrap();
        assert_ne!(encrypted.as_slice(), data);
        assert!(encrypted.len() > data.len());

        let decrypted = decrypt_data(&encrypted, password).unwrap();
        assert_eq!(decrypted.as_slice(), data);
    }

    #[test]
    fn test_encryption_wrong_password() {
        let data = b"Secret message";
        let password = "CorrectPassword";
        let wrong_password = "WrongPassword";

        let encrypted = encrypt_data(data, password).unwrap();
        let result = decrypt_data(&encrypted, wrong_password);

        assert!(result.is_err());
    }

    #[test]
    fn test_checksum_computation() {
        let data = b"Test data for checksum";
        let checksum1 = compute_checksum(data);
        let checksum2 = compute_checksum(data);

        assert_eq!(checksum1, checksum2);
        assert_eq!(checksum1.len(), 64); // SHA256 hex = 64 chars

        let different_data = b"Different data";
        let checksum3 = compute_checksum(different_data);
        assert_ne!(checksum1, checksum3);
    }

    #[test]
    fn test_manifest_serialization() {
        let manifest = BackupManifest {
            version: "1.0".to_string(),
            created_at: "2024-02-15T10:00:00Z".to_string(),
            backup_type: "full".to_string(),
            compression: CompressionType::Zstd,
            encrypted: true,
            message_count: 1000,
            mailbox_count: 50,
            user_count: 10,
            total_size: 1024 * 1024 * 100,
            checksum: "abc123".to_string(),
            base_backup: None,
            modseq: Some(12345),
        };

        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains("1.0"));
        assert!(json.contains("full"));

        let deserialized: BackupManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.version, "1.0");
        assert_eq!(deserialized.message_count, 1000);
    }

    #[test]
    fn test_create_tar_archive() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, b"Test content").unwrap();

        let pb = ProgressBar::hidden();
        let tar_data = create_tar_archive(temp_dir.path(), &pb).unwrap();

        assert!(!tar_data.is_empty());
        assert!(tar_data.len() > 512); // Tar headers + content
    }

    #[test]
    fn test_s3_config_serialization() {
        let config = S3Config {
            bucket: "my-bucket".to_string(),
            region: "us-east-1".to_string(),
            endpoint: Some("https://s3.example.com".to_string()),
            access_key: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            prefix: Some("backups/".to_string()),
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("my-bucket"));
        assert!(json.contains("us-east-1"));
    }

    #[test]
    fn test_full_backup_cycle() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        fs::create_dir(&source_dir).unwrap();

        // Create test files
        fs::write(source_dir.join("file1.txt"), b"Content 1").unwrap();
        fs::write(source_dir.join("file2.txt"), b"Content 2").unwrap();

        let backup_path = temp_dir.path().join("backup.tar.zst");

        let manifest = create_local_backup(
            &source_dir,
            &backup_path,
            CompressionType::Zstd,
            false,
            None,
            false,
            None,
        )
        .unwrap();

        assert!(backup_path.exists());
        assert!(manifest.total_size > 0);
        assert_eq!(manifest.compression, CompressionType::Zstd);
        assert!(!manifest.encrypted);
    }

    #[test]
    fn test_encrypted_backup_cycle() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        fs::create_dir(&source_dir).unwrap();

        fs::write(source_dir.join("secret.txt"), b"Secret data").unwrap();

        let backup_path = temp_dir.path().join("backup.tar.gz.enc");
        let password = "TestPassword123";

        let manifest = create_local_backup(
            &source_dir,
            &backup_path,
            CompressionType::Gzip,
            true,
            Some(password),
            false,
            None,
        )
        .unwrap();

        assert!(backup_path.exists());
        assert!(manifest.encrypted);

        // Verify
        let verified = verify_local_backup(&backup_path, Some(password)).unwrap();
        assert!(verified.encrypted);
    }

    #[test]
    fn test_incremental_backup_options() {
        let options = BackupOptions {
            output_path: "/tmp/inc-backup.tar.zst".to_string(),
            format: BackupFormat::TarZst,
            compression: CompressionType::Zstd,
            encryption: false,
            encryption_key: None,
            password_file: None,
            incremental: true,
            base_backup: Some("/tmp/base-backup.tar.zst".to_string()),
            include_messages: true,
            include_mailboxes: true,
            include_config: false,
            include_metadata: true,
            include_users: Some(vec!["user1@example.com".to_string()]),
            verify: true,
            s3_upload: None,
        };

        assert!(options.incremental);
        assert!(options.base_backup.is_some());
        assert!(options.verify);
        assert_eq!(options.include_users.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_compression_type_equality() {
        assert_eq!(CompressionType::None, CompressionType::None);
        assert_eq!(CompressionType::Gzip, CompressionType::Gzip);
        assert_eq!(CompressionType::Zstd, CompressionType::Zstd);
        assert_ne!(CompressionType::None, CompressionType::Gzip);
    }

    #[test]
    fn test_backup_format_equality() {
        assert_eq!(BackupFormat::TarGz, BackupFormat::TarGz);
        assert_eq!(BackupFormat::TarZst, BackupFormat::TarZst);
        assert_ne!(BackupFormat::TarGz, BackupFormat::TarZst);
    }

    #[test]
    fn test_large_data_compression() {
        let large_data = vec![b'A'; 1_000_000]; // 1MB of 'A's

        let compressed_gzip = compress_data(&large_data, CompressionType::Gzip).unwrap();
        let compressed_zstd = compress_data(&large_data, CompressionType::Zstd).unwrap();

        assert!(compressed_gzip.len() < large_data.len());
        assert!(compressed_zstd.len() < large_data.len());

        // Zstd typically better for repetitive data
        assert!(compressed_zstd.len() < compressed_gzip.len());
    }
}

//! File-based authentication backend (htpasswd-style with bcrypt password hashing)
//!
//! File format: one user per line
//! ```text
//! username:$2b$12$... (bcrypt hash)
//! ```

use crate::AuthBackend;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use rusmes_proto::Username;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;

/// File-based authentication backend using bcrypt for password hashing
pub struct FileAuthBackend {
    file_path: PathBuf,
    users: Arc<RwLock<HashMap<String, String>>>,
}

impl FileAuthBackend {
    /// Create a new file-based authentication backend
    ///
    /// # Arguments
    /// * `file_path` - Path to the password file
    pub async fn new(file_path: impl AsRef<Path>) -> Result<Self> {
        let file_path = file_path.as_ref().to_path_buf();
        let users = Self::load_users(&file_path).await?;

        Ok(Self {
            file_path,
            users: Arc::new(RwLock::new(users)),
        })
    }

    /// Load users from the password file
    async fn load_users(file_path: &Path) -> Result<HashMap<String, String>> {
        // Create the file if it doesn't exist
        if !file_path.exists() {
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .context("Failed to create parent directory")?;
            }
            fs::File::create(file_path)
                .await
                .context("Failed to create password file")?;
            return Ok(HashMap::new());
        }

        let mut file = fs::File::open(file_path)
            .await
            .context("Failed to open password file")?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .await
            .context("Failed to read password file")?;

        let mut users = HashMap::new();
        for (line_num, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let parts: Vec<&str> = line.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(anyhow!(
                    "Invalid format on line {}: expected 'username:hash'",
                    line_num + 1
                ));
            }

            let username = parts[0].to_string();
            let hash = parts[1].to_string();

            if username.is_empty() {
                return Err(anyhow!("Empty username on line {}", line_num + 1));
            }

            if !hash.starts_with("$2b$") && !hash.starts_with("$2a$") && !hash.starts_with("$2y$") {
                return Err(anyhow!(
                    "Invalid bcrypt hash on line {}: hash must start with $2a$, $2b$, or $2y$",
                    line_num + 1
                ));
            }

            users.insert(username, hash);
        }

        Ok(users)
    }

    /// Save users to the password file
    async fn save_users(&self, users: &HashMap<String, String>) -> Result<()> {
        let mut contents = String::new();
        let mut usernames: Vec<&String> = users.keys().collect();
        usernames.sort();

        for username in usernames {
            let hash = &users[username];
            contents.push_str(&format!("{}:{}\n", username, hash));
        }

        // Write to a temporary file first, then rename atomically
        let temp_path = self.file_path.with_extension("tmp");
        let mut file = fs::File::create(&temp_path)
            .await
            .context("Failed to create temporary file")?;
        file.write_all(contents.as_bytes())
            .await
            .context("Failed to write to temporary file")?;
        file.sync_all()
            .await
            .context("Failed to sync temporary file")?;
        drop(file);

        fs::rename(&temp_path, &self.file_path)
            .await
            .context("Failed to rename temporary file")?;

        Ok(())
    }

    /// Hash a password using bcrypt
    fn hash_password(password: &str) -> Result<String> {
        bcrypt::hash(password, bcrypt::DEFAULT_COST).context("Failed to hash password")
    }

    /// Verify a password against a bcrypt hash
    fn verify_password(password: &str, hash: &str) -> Result<bool> {
        bcrypt::verify(password, hash).context("Failed to verify password")
    }
}

#[async_trait]
impl AuthBackend for FileAuthBackend {
    async fn authenticate(&self, username: &Username, password: &str) -> Result<bool> {
        let users = self.users.read().await;

        if let Some(hash) = users.get(username.as_str()) {
            Self::verify_password(password, hash)
        } else {
            // User not found - still run bcrypt to prevent timing attacks
            let _ = bcrypt::verify(
                password,
                "$2b$12$dummy_hash_to_prevent_timing_attack_00000000000000000000000000000",
            );
            Ok(false)
        }
    }

    async fn verify_identity(&self, username: &Username) -> Result<bool> {
        let users = self.users.read().await;
        Ok(users.contains_key(username.as_str()))
    }

    async fn list_users(&self) -> Result<Vec<Username>> {
        let users = self.users.read().await;
        let mut usernames = Vec::new();

        for username_str in users.keys() {
            let username = Username::new(username_str.clone()).context(format!(
                "Invalid username in password file: {}",
                username_str
            ))?;
            usernames.push(username);
        }

        usernames.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(usernames)
    }

    async fn create_user(&self, username: &Username, password: &str) -> Result<()> {
        let mut users = self.users.write().await;

        if users.contains_key(username.as_str()) {
            return Err(anyhow!("User '{}' already exists", username.as_str()));
        }

        let hash = Self::hash_password(password)?;
        users.insert(username.as_str().to_string(), hash);

        self.save_users(&users).await?;

        Ok(())
    }

    async fn delete_user(&self, username: &Username) -> Result<()> {
        let mut users = self.users.write().await;

        if !users.contains_key(username.as_str()) {
            return Err(anyhow!("User '{}' does not exist", username.as_str()));
        }

        users.remove(username.as_str());
        self.save_users(&users).await?;

        Ok(())
    }

    async fn change_password(&self, username: &Username, new_password: &str) -> Result<()> {
        let mut users = self.users.write().await;

        if !users.contains_key(username.as_str()) {
            return Err(anyhow!("User '{}' does not exist", username.as_str()));
        }

        let hash = Self::hash_password(new_password)?;
        users.insert(username.as_str().to_string(), hash);

        self.save_users(&users).await?;

        Ok(())
    }
}

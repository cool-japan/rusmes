//! File-based authentication backend (htpasswd-style with bcrypt or argon2id password hashing)
//!
//! ## File format (backwards-compatible)
//!
//! Each non-empty, non-comment line contains a `username:` prefix followed by a password-hash
//! field. The hash format is auto-detected by its prefix:
//!
//! * `$2a$…`, `$2b$…`, `$2y$…` — bcrypt (any cost)
//! * `$argon2id$…`, `$argon2i$…`, `$argon2d$…` — argon2 (PHC string format per RFC 9106)
//!
//! New password writes use whichever algorithm is configured via
//! [`FileAuthBackend::with_algorithm`] (default: bcrypt). **Existing hashes verify
//! regardless of the configured algorithm** — i.e. an argon2-backed deployment
//! continues to authenticate users whose hashes are still bcrypt, until each user
//! resets their password.
//!
//! If SCRAM-SHA-256 credentials are also stored, they appear as additional
//! tab-separated columns appended **to the password-hash field** (not as new
//! colon-separated columns):
//!
//! ```text
//! # Old format — still loads correctly:
//! alice:$2b$12$...<bcrypt hash>...
//!
//! # New format — password hash + tab + four SCRAM columns:
//! alice:$2b$12$...<bcrypt hash>...\t<salt_base64>\t<iter>\t<stored_key_base64>\t<server_key_base64>
//!
//! # Argon2 with SCRAM:
//! bob:$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>\t<salt_b64>\t<iter>\t<sk_b64>\t<svk_b64>
//! ```
//!
//! The four SCRAM columns are:
//! * `salt_base64` — Base64-encoded (standard) random salt bytes.
//! * `iter` — PBKDF2 iteration count (decimal integer).
//! * `stored_key_base64` — Base64-encoded `SHA-256(ClientKey)` per RFC 5802.
//! * `server_key_base64` — Base64-encoded `HMAC-SHA-256(SaltedPassword, "Server Key")` per RFC 5802.
//!
//! Old lines without the tab extension parse identically to before.

use crate::{AuthBackend, ScramCredentials};
use anyhow::{anyhow, Context, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rusmes_proto::Username;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;

// ============================================================================
// Hash algorithm selector
// ============================================================================

/// Password-hashing algorithm used when writing **new** password hashes.
///
/// Existing hashes are auto-detected and verified using whichever algorithm
/// produced them, regardless of the configured value here. The configured
/// value only governs `create_user` and `change_password`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HashAlgorithm {
    /// bcrypt with the bcrypt crate's `DEFAULT_COST`. Backwards-compatible default.
    #[default]
    Bcrypt,
    /// argon2id with the argon2 crate's defaults (RFC 9106 second-recommended
    /// parameters: m = 19 MiB, t = 2, p = 1).
    Argon2,
}

impl HashAlgorithm {
    /// Parse a config-string into a [`HashAlgorithm`].
    ///
    /// Accepts `"bcrypt"` (case-insensitive) and `"argon2"` /
    /// `"argon2id"` (case-insensitive). Any other value is an error.
    pub fn from_config_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bcrypt" => Ok(HashAlgorithm::Bcrypt),
            "argon2" | "argon2id" => Ok(HashAlgorithm::Argon2),
            other => Err(anyhow!(
                "unknown hash_algorithm '{}': expected 'bcrypt' or 'argon2'",
                other
            )),
        }
    }
}

/// Returns `true` if the supplied hash field appears to be a bcrypt hash.
fn is_bcrypt_hash(hash: &str) -> bool {
    hash.starts_with("$2a$") || hash.starts_with("$2b$") || hash.starts_with("$2y$")
}

/// Returns `true` if the supplied hash field appears to be an argon2 PHC string.
fn is_argon2_hash(hash: &str) -> bool {
    hash.starts_with("$argon2id$") || hash.starts_with("$argon2i$") || hash.starts_with("$argon2d$")
}

// ============================================================================
// Internal record holding everything we know about a user entry.
// ============================================================================

/// Internal representation of a single passwd-file user record.
///
/// The `hash_field` stores the raw value that appears between the `:` separator
/// and the newline; for old-format entries that is just the password hash
/// (bcrypt or argon2 — auto-detected by prefix). For new-format entries it is
/// `<password_hash>\t<salt_b64>\t<iter>\t<sk_b64>\t<svk_b64>`.
///
/// We keep the whole field intact so that round-trips don't destroy data we
/// didn't understand.
#[derive(Debug, Clone)]
struct UserRecord {
    /// The full hash field (may contain tab-separated SCRAM columns).
    hash_field: String,
}

impl UserRecord {
    /// The password hash alone (everything before the first `\t`).
    ///
    /// May be either a bcrypt hash (`$2{a,b,y}$…`) or an argon2 PHC string
    /// (`$argon2id$…`); use [`is_bcrypt_hash`] / [`is_argon2_hash`] to dispatch.
    fn password_hash(&self) -> &str {
        self.hash_field
            .split('\t')
            .next()
            .unwrap_or(&self.hash_field)
    }

    /// Parse optional SCRAM fields from the tab-separated tail.
    ///
    /// Returns `None` if fewer than four SCRAM columns are present (old format).
    fn scram_credentials(&self) -> Option<ScramCredentials> {
        let mut parts = self.hash_field.splitn(5, '\t');
        // Skip the password hash
        let _ = parts.next()?;

        let salt_b64 = parts.next()?;
        let iter_str = parts.next()?;
        let sk_b64 = parts.next()?;
        let svk_b64 = parts.next()?;

        let salt = BASE64.decode(salt_b64).ok()?;
        let iteration_count = iter_str.parse::<u32>().ok()?;
        let stored_key = BASE64.decode(sk_b64).ok()?;
        let server_key = BASE64.decode(svk_b64).ok()?;

        // Sanity-check: SHA-256 outputs are always 32 bytes
        if stored_key.len() != 32 || server_key.len() != 32 || salt.is_empty() {
            return None;
        }

        Some(ScramCredentials {
            salt,
            iteration_count,
            stored_key,
            server_key,
        })
    }

    /// Rebuild the hash field from a password hash and optional SCRAM credentials.
    ///
    /// `password_hash` may be either a bcrypt hash or an argon2 PHC string.
    fn with_scram(password_hash: &str, creds: &ScramCredentials) -> Self {
        let salt_b64 = BASE64.encode(&creds.salt);
        let sk_b64 = BASE64.encode(&creds.stored_key);
        let svk_b64 = BASE64.encode(&creds.server_key);
        let hash_field = format!(
            "{}\t{}\t{}\t{}\t{}",
            password_hash, salt_b64, creds.iteration_count, sk_b64, svk_b64
        );
        Self { hash_field }
    }
}

// ============================================================================
// FileAuthBackend
// ============================================================================

/// File-based authentication backend supporting bcrypt **and** argon2id password hashing.
///
/// New password writes use whichever algorithm was selected via
/// [`FileAuthBackend::with_algorithm`] (default: bcrypt). Existing hashes are
/// auto-detected by their PHC prefix and verified with the correct algorithm,
/// so a deployment can migrate at any time without invalidating existing users.
///
/// Also supports storing and fetching RFC 5802 SCRAM-SHA-256 credential bundles
/// via an extended tab-separated format that is fully backwards-compatible with
/// the original two-column `username:hash` format.
pub struct FileAuthBackend {
    file_path: PathBuf,
    users: Arc<RwLock<HashMap<String, UserRecord>>>,
    /// Algorithm used for *new* password writes (`create_user`, `change_password`).
    /// Existing hashes verify with whichever algorithm produced them, regardless.
    algorithm: HashAlgorithm,
}

impl FileAuthBackend {
    /// Create a new file-based authentication backend using the default algorithm
    /// ([`HashAlgorithm::Bcrypt`], for backwards compatibility).
    ///
    /// If the file does not exist it is created (along with any missing parent
    /// directories). An existing file is loaded into memory immediately.
    pub async fn new(file_path: impl AsRef<Path>) -> Result<Self> {
        Self::with_algorithm(file_path, HashAlgorithm::default()).await
    }

    /// Create a new file-based authentication backend with an explicit
    /// password-hashing algorithm for new writes.
    ///
    /// Use this constructor when the operator's `[auth.file.hash_algorithm]`
    /// config selects argon2id. Existing bcrypt hashes in the file remain
    /// fully functional — they verify using bcrypt regardless of this setting.
    pub async fn with_algorithm(
        file_path: impl AsRef<Path>,
        algorithm: HashAlgorithm,
    ) -> Result<Self> {
        let file_path = file_path.as_ref().to_path_buf();
        let users = Self::load_users(&file_path).await?;

        Ok(Self {
            file_path,
            users: Arc::new(RwLock::new(users)),
            algorithm,
        })
    }

    /// Returns the algorithm used for new password writes.
    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    // -----------------------------------------------------------------------
    // File I/O helpers
    // -----------------------------------------------------------------------

    /// Load users from the password file into an in-memory map.
    async fn load_users(file_path: &Path) -> Result<HashMap<String, UserRecord>> {
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

            // Split on the first `:` only; everything after is the hash field
            // (which may contain additional tab-separated SCRAM columns).
            let colon_pos = line.find(':').ok_or_else(|| {
                anyhow!(
                    "Invalid format on line {}: expected 'username:hash'",
                    line_num + 1
                )
            })?;

            let username = &line[..colon_pos];
            let hash_field = &line[colon_pos + 1..];

            if username.is_empty() {
                return Err(anyhow!("Empty username on line {}", line_num + 1));
            }

            // The password hash is the portion before the first tab (if any
            // tabs are present for SCRAM columns).
            let password_hash = hash_field.split('\t').next().unwrap_or(hash_field);

            if !is_bcrypt_hash(password_hash) && !is_argon2_hash(password_hash) {
                return Err(anyhow!(
                    "Invalid password hash on line {}: expected bcrypt ($2a$/$2b$/$2y$) or argon2 ($argon2id$/$argon2i$/$argon2d$) prefix",
                    line_num + 1
                ));
            }

            users.insert(
                username.to_string(),
                UserRecord {
                    hash_field: hash_field.to_string(),
                },
            );
        }

        Ok(users)
    }

    /// Persist the in-memory map to disk atomically.
    async fn save_users(&self, users: &HashMap<String, UserRecord>) -> Result<()> {
        let mut contents = String::new();
        let mut usernames: Vec<&String> = users.keys().collect();
        usernames.sort();

        for username in usernames {
            let record = &users[username];
            contents.push_str(&format!("{}:{}\n", username, record.hash_field));
        }

        // Write to a temp file then atomically rename.
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

    // -----------------------------------------------------------------------
    // Password helpers — algorithm-aware (bcrypt or argon2id)
    // -----------------------------------------------------------------------

    /// Hash a password using the configured algorithm.
    ///
    /// * Bcrypt: uses the bcrypt crate's `DEFAULT_COST`.
    /// * Argon2: uses argon2id with the argon2 crate's defaults (RFC 9106).
    fn hash_password(&self, password: &str) -> Result<String> {
        match self.algorithm {
            HashAlgorithm::Bcrypt => bcrypt::hash(password, bcrypt::DEFAULT_COST)
                .context("Failed to hash password (bcrypt)"),
            HashAlgorithm::Argon2 => {
                let salt = SaltString::generate(&mut OsRng);
                let argon2 = Argon2::default();
                argon2
                    .hash_password(password.as_bytes(), &salt)
                    .map_err(|e| anyhow!("Failed to hash password (argon2id): {}", e))
                    .map(|h| h.to_string())
            }
        }
    }

    /// Verify a password against a stored hash.
    ///
    /// The algorithm is auto-detected from the hash's PHC prefix; the
    /// configured [`HashAlgorithm`] is **not** consulted on verify, so
    /// existing bcrypt hashes continue to authenticate after switching the
    /// configured algorithm to argon2 (and vice versa).
    fn verify_password(password: &str, hash: &str) -> Result<bool> {
        if is_bcrypt_hash(hash) {
            bcrypt::verify(password, hash).context("Failed to verify password (bcrypt)")
        } else if is_argon2_hash(hash) {
            let parsed = PasswordHash::new(hash)
                .map_err(|e| anyhow!("Failed to parse argon2 PHC string: {}", e))?;
            match Argon2::default().verify_password(password.as_bytes(), &parsed) {
                Ok(()) => Ok(true),
                Err(argon2::password_hash::Error::Password) => Ok(false),
                Err(e) => Err(anyhow!("Failed to verify password (argon2id): {}", e)),
            }
        } else {
            Err(anyhow!(
                "Unrecognized password hash format (no bcrypt or argon2 prefix)"
            ))
        }
    }

    // -----------------------------------------------------------------------
    // Public SCRAM credential management (file-backend-specific)
    // -----------------------------------------------------------------------

    /// Persist RFC 5802 SCRAM-SHA-256 credentials for `user`.
    ///
    /// If the user already has a SCRAM credential bundle it is replaced.
    /// The bcrypt password hash is preserved unchanged.  Returns an error if
    /// the user does not exist.
    ///
    /// This method is intentionally on `FileAuthBackend` directly (not on the
    /// `AuthBackend` trait) because it is part of the migration/admin tooling
    /// surface, not the per-request hot path.
    pub async fn set_scram_credentials(
        &self,
        user: &str,
        credentials: ScramCredentials,
    ) -> Result<()> {
        let mut users = self.users.write().await;

        let record = users
            .get(user)
            .ok_or_else(|| anyhow!("User '{}' does not exist", user))?;

        let password_hash = record.password_hash().to_string();
        let new_record = UserRecord::with_scram(&password_hash, &credentials);
        users.insert(user.to_string(), new_record);

        self.save_users(&users).await
    }
}

// ============================================================================
// AuthBackend implementation
// ============================================================================

#[async_trait]
impl AuthBackend for FileAuthBackend {
    async fn authenticate(&self, username: &Username, password: &str) -> Result<bool> {
        let users = self.users.read().await;

        if let Some(record) = users.get(username.as_str()) {
            Self::verify_password(password, record.password_hash())
        } else {
            // User not found — still run a hash verification to prevent timing
            // attacks. We always run bcrypt here regardless of the configured
            // algorithm: the goal is just to spend some work, and a constant
            // bcrypt cost is the cheapest way to keep the failure-path latency
            // similar to a real bcrypt verification.
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

        let hash = self.hash_password(password)?;
        users.insert(
            username.as_str().to_string(),
            UserRecord { hash_field: hash },
        );

        self.save_users(&users).await
    }

    async fn delete_user(&self, username: &Username) -> Result<()> {
        let mut users = self.users.write().await;

        if !users.contains_key(username.as_str()) {
            return Err(anyhow!("User '{}' does not exist", username.as_str()));
        }

        users.remove(username.as_str());
        self.save_users(&users).await
    }

    async fn change_password(&self, username: &Username, new_password: &str) -> Result<()> {
        let mut users = self.users.write().await;

        let record = users
            .get(username.as_str())
            .ok_or_else(|| anyhow!("User '{}' does not exist", username.as_str()))?;

        // Preserve any existing SCRAM columns — only replace the password hash.
        let existing_scram = record.scram_credentials();
        let new_hash = self.hash_password(new_password)?;

        let new_record = match existing_scram {
            Some(creds) => UserRecord::with_scram(&new_hash, &creds),
            None => UserRecord {
                hash_field: new_hash,
            },
        };
        users.insert(username.as_str().to_string(), new_record);

        self.save_users(&users).await
    }

    // -----------------------------------------------------------------------
    // SCRAM-SHA-256 — full implementation for the file backend
    // -----------------------------------------------------------------------

    /// Fetch the RFC 5802 SCRAM-SHA-256 credential bundle for `user`.
    ///
    /// Returns `Ok(None)` if no SCRAM columns are stored (old-format entry or
    /// user was never enrolled in SCRAM).
    async fn fetch_scram_credentials(&self, user: &str) -> Result<Option<ScramCredentials>> {
        let users = self.users.read().await;
        let record = match users.get(user) {
            Some(r) => r,
            None => return Ok(None),
        };
        Ok(record.scram_credentials())
    }

    // -----------------------------------------------------------------------
    // Legacy SCRAM methods — forwarded for compatibility with sasl.rs callers
    // -----------------------------------------------------------------------

    async fn get_scram_params(&self, username: &str) -> Result<(Vec<u8>, u32)> {
        let creds = self
            .fetch_scram_credentials(username)
            .await?
            .ok_or_else(|| anyhow!("No SCRAM credentials stored for user '{}'", username))?;
        Ok((creds.salt, creds.iteration_count))
    }

    async fn get_scram_stored_key(&self, username: &str) -> Result<Vec<u8>> {
        let creds = self
            .fetch_scram_credentials(username)
            .await?
            .ok_or_else(|| anyhow!("No SCRAM credentials stored for user '{}'", username))?;
        Ok(creds.stored_key)
    }

    async fn get_scram_server_key(&self, username: &str) -> Result<Vec<u8>> {
        let creds = self
            .fetch_scram_credentials(username)
            .await?
            .ok_or_else(|| anyhow!("No SCRAM credentials stored for user '{}'", username))?;
        Ok(creds.server_key)
    }

    async fn store_scram_credentials(
        &self,
        username: &Username,
        salt: Vec<u8>,
        iterations: u32,
        stored_key: Vec<u8>,
        server_key: Vec<u8>,
    ) -> Result<()> {
        let creds = ScramCredentials {
            salt,
            iteration_count: iterations,
            stored_key,
            server_key,
        };
        self.set_scram_credentials(username.as_str(), creds).await
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::{
        ldap::LdapBackend, ldap::LdapConfig, oauth2::OAuth2Backend, oauth2::OAuth2Config,
    };
    use crate::{AuthBackendKind, FileBackendConfig};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::env;
    use std::fs as std_fs;

    type HmacSha256 = Hmac<Sha256>;

    // Helper: derive SCRAM credentials from a password using the project's
    // derivation logic (mirrors ScramSha256Mechanism::compute_* in sasl.rs).
    fn derive_scram_creds(password: &str, salt: &[u8], iterations: u32) -> ScramCredentials {
        // SaltedPassword = PBKDF2-SHA-256(password, salt, iterations)
        let mut salted_password = vec![0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, iterations, &mut salted_password);

        // ClientKey = HMAC-SHA-256(SaltedPassword, "Client Key")
        let mut mac =
            HmacSha256::new_from_slice(&salted_password).expect("HMAC accepts any key length");
        mac.update(b"Client Key");
        let client_key = mac.finalize().into_bytes();

        // StoredKey = SHA-256(ClientKey)
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(client_key);
        let stored_key = hasher.finalize().to_vec();

        // ServerKey = HMAC-SHA-256(SaltedPassword, "Server Key")
        let mut mac2 =
            HmacSha256::new_from_slice(&salted_password).expect("HMAC accepts any key length");
        mac2.update(b"Server Key");
        let server_key = mac2.finalize().into_bytes().to_vec();

        ScramCredentials {
            salt: salt.to_vec(),
            iteration_count: iterations,
            stored_key,
            server_key,
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: AuthBackendKind::build(File) → authenticate round-trip
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn auth_backend_kind_build_file() {
        let dir = env::temp_dir().join(format!("rusmes_auth_kind_build_{}", std::process::id()));
        std_fs::create_dir_all(&dir).expect("create temp dir");
        let passwd_path = dir.join("passwd");

        // 1. Build via kind factory
        let kind = AuthBackendKind::File(FileBackendConfig {
            path: passwd_path.to_string_lossy().to_string(),
            hash_algorithm: HashAlgorithm::default(),
        });
        let backend = kind.build().await.expect("build file backend");

        // 2. Create a user through the trait
        let username = Username::new("testuser".to_string()).expect("valid username");
        backend
            .create_user(&username, "s3cr3t!")
            .await
            .expect("create user");

        // 3. Authenticate — must succeed
        let ok = backend
            .authenticate(&username, "s3cr3t!")
            .await
            .expect("authenticate");
        assert!(ok, "correct password must authenticate");

        // 4. Wrong password must fail
        let bad = backend
            .authenticate(&username, "wrong")
            .await
            .expect("authenticate with wrong pw");
        assert!(!bad, "wrong password must not authenticate");

        // cleanup
        std_fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------------
    // Test 2: SCRAM credential round-trip + backwards-compat with old format
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn file_backend_scram_credentials_roundtrip() {
        let dir = env::temp_dir().join(format!(
            "rusmes_auth_scram_roundtrip_{}",
            std::process::id()
        ));
        std_fs::create_dir_all(&dir).expect("create temp dir");
        let passwd_path = dir.join("passwd");

        // ---- Part A: backwards-compat — write an old-format line directly ----
        // Write a valid bcrypt hash for password "hunter2"
        let hash = bcrypt::hash("hunter2", 4).expect("bcrypt hash");
        std_fs::write(&passwd_path, format!("olduser:{}\n", hash))
            .expect("write old-format passwd");

        let backend = FileAuthBackend::new(&passwd_path)
            .await
            .expect("load old-format passwd");

        // Old-format user must still authenticate
        let user = Username::new("olduser".to_string()).expect("username");
        assert!(
            backend.authenticate(&user, "hunter2").await.expect("auth"),
            "old-format user must authenticate"
        );

        // Old-format user has no SCRAM credentials
        let none = backend
            .fetch_scram_credentials("olduser")
            .await
            .expect("fetch scram");
        assert!(none.is_none(), "old-format user has no SCRAM credentials");

        // ---- Part B: set + fetch SCRAM credentials ----
        let salt = b"naCl_and_pepper!!"; // 17 bytes
        let creds = derive_scram_creds("hunter2", salt, 4096);

        backend
            .set_scram_credentials("olduser", creds.clone())
            .await
            .expect("set_scram_credentials");

        let fetched = backend
            .fetch_scram_credentials("olduser")
            .await
            .expect("fetch after set")
            .expect("credentials must be present");

        assert_eq!(fetched.salt, creds.salt, "salt round-trip");
        assert_eq!(
            fetched.iteration_count, creds.iteration_count,
            "iteration_count round-trip"
        );
        assert_eq!(
            fetched.stored_key, creds.stored_key,
            "stored_key round-trip"
        );
        assert_eq!(
            fetched.server_key, creds.server_key,
            "server_key round-trip"
        );

        // ---- Part C: reload from disk — persistence test ----
        let backend2 = FileAuthBackend::new(&passwd_path)
            .await
            .expect("reload backend");
        let reloaded = backend2
            .fetch_scram_credentials("olduser")
            .await
            .expect("fetch after reload")
            .expect("credentials survive disk round-trip");
        assert_eq!(
            reloaded.stored_key, creds.stored_key,
            "persisted stored_key"
        );
        assert_eq!(
            reloaded.server_key, creds.server_key,
            "persisted server_key"
        );

        // ---- Part D: bcrypt authentication still works after SCRAM write ----
        assert!(
            backend2
                .authenticate(&user, "hunter2")
                .await
                .expect("re-auth"),
            "bcrypt auth must still work after SCRAM credential write"
        );

        // cleanup
        std_fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------------
    // Test 3: SQL / LDAP / OAuth2 inherit Ok(None) default
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn default_fetch_scram_credentials_returns_none() {
        // LDAP backend — sync constructor, always infallible
        let ldap = LdapBackend::new(LdapConfig::default());
        let result = ldap
            .fetch_scram_credentials("anyuser")
            .await
            .expect("LDAP default must not error");
        assert!(
            result.is_none(),
            "LdapBackend must return Ok(None) for fetch_scram_credentials"
        );

        // OAuth2 backend — sync constructor, always infallible
        let oauth2 = OAuth2Backend::new(OAuth2Config::default());
        let result2 = oauth2
            .fetch_scram_credentials("anyuser")
            .await
            .expect("OAuth2 default must not error");
        assert!(
            result2.is_none(),
            "OAuth2Backend must return Ok(None) for fetch_scram_credentials"
        );

        // Note: SqlBackend requires a live database connection so we skip it
        // here; it inherits the same default impl as LDAP/OAuth2.
    }

    // -----------------------------------------------------------------------
    // Test 4: argon2id round-trip + bcrypt-compat
    //
    // Verifies the dual-algorithm contract:
    //  1. A backend configured for argon2 produces argon2 hashes for new users.
    //  2. Those hashes verify correctly via the same backend.
    //  3. A pre-existing bcrypt hash on disk continues to authenticate even
    //     after the algorithm has been switched to argon2 — the verify path
    //     dispatches on the stored prefix, not on the configured algorithm.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn argon2_roundtrip_and_bcrypt_compat() {
        let dir = env::temp_dir().join(format!(
            "rusmes_auth_argon2_roundtrip_{}",
            std::process::id()
        ));
        std_fs::create_dir_all(&dir).expect("create temp dir");
        let passwd_path = dir.join("passwd");

        // -------- Part A: seed a bcrypt hash directly on disk --------
        // This simulates a deployment that started life on bcrypt and is
        // about to switch its hash_algorithm config to argon2.
        let bcrypt_hash = bcrypt::hash("legacy-pass", 4).expect("bcrypt hash");
        std_fs::write(&passwd_path, format!("legacyuser:{}\n", bcrypt_hash))
            .expect("seed bcrypt-only file");

        // Boot the backend with argon2 selected for *new* writes.
        let backend = FileAuthBackend::with_algorithm(&passwd_path, HashAlgorithm::Argon2)
            .await
            .expect("load passwd file");
        assert_eq!(backend.algorithm(), HashAlgorithm::Argon2);

        // The legacy bcrypt user still authenticates — verify dispatches on
        // the stored hash's prefix, not on the configured algorithm.
        let legacy = Username::new("legacyuser".to_string()).expect("legacy username");
        assert!(
            backend
                .authenticate(&legacy, "legacy-pass")
                .await
                .expect("auth legacy"),
            "pre-existing bcrypt hash must still verify under argon2 config"
        );
        assert!(
            !backend
                .authenticate(&legacy, "wrong")
                .await
                .expect("auth legacy bad"),
            "wrong bcrypt password must fail under argon2 config"
        );

        // -------- Part B: create_user under argon2 produces argon2 hash --------
        let new_user = Username::new("alice".to_string()).expect("alice username");
        backend
            .create_user(&new_user, "Sup3rSecret!")
            .await
            .expect("create alice");

        // Inspect the on-disk file: alice's hash field must start with
        // "$argon2id$" — the argon2 crate's default variant.
        let on_disk = std_fs::read_to_string(&passwd_path).expect("read passwd file");
        let alice_line = on_disk
            .lines()
            .find(|l| l.starts_with("alice:"))
            .expect("alice line");
        let alice_hash_field = &alice_line["alice:".len()..];
        let alice_hash = alice_hash_field
            .split('\t')
            .next()
            .expect("alice hash field non-empty");
        assert!(
            alice_hash.starts_with("$argon2id$"),
            "new password under argon2 config must produce $argon2id$ hash, got: {}",
            alice_hash
        );

        // Verify Alice's password through the trait.
        assert!(
            backend
                .authenticate(&new_user, "Sup3rSecret!")
                .await
                .expect("auth alice"),
            "argon2 hash must verify with correct password"
        );
        assert!(
            !backend
                .authenticate(&new_user, "wrong-pass")
                .await
                .expect("auth alice bad"),
            "argon2 hash must reject wrong password"
        );

        // -------- Part C: legacy bcrypt user still works after argon2 user added --------
        assert!(
            backend
                .authenticate(&legacy, "legacy-pass")
                .await
                .expect("re-auth legacy"),
            "bcrypt user still verifies after argon2 user is created"
        );

        // -------- Part D: change_password preserves SCRAM AND switches algorithm --------
        // Add SCRAM creds to alice (deterministic), then change password under
        // argon2 — the SCRAM bundle must survive the rewrite.
        let scram = derive_scram_creds("Sup3rSecret!", b"some-salt-bytes!", 4096);
        backend
            .set_scram_credentials("alice", scram.clone())
            .await
            .expect("set scram on alice");
        backend
            .change_password(&new_user, "NewArgon2Pass!")
            .await
            .expect("change_password to argon2");
        let after = backend
            .fetch_scram_credentials("alice")
            .await
            .expect("fetch scram after change_password")
            .expect("scram preserved");
        assert_eq!(
            after.salt, scram.salt,
            "SCRAM salt preserved across argon2 password change"
        );
        assert!(
            backend
                .authenticate(&new_user, "NewArgon2Pass!")
                .await
                .expect("auth new password"),
            "argon2 hash from change_password must verify"
        );

        // -------- Part E: reload from disk, all guarantees still hold --------
        let backend2 = FileAuthBackend::with_algorithm(&passwd_path, HashAlgorithm::Argon2)
            .await
            .expect("reload backend");
        assert!(
            backend2
                .authenticate(&legacy, "legacy-pass")
                .await
                .expect("reload bcrypt legacy"),
            "bcrypt legacy verifies after disk reload"
        );
        assert!(
            backend2
                .authenticate(&new_user, "NewArgon2Pass!")
                .await
                .expect("reload argon2 alice"),
            "argon2 alice verifies after disk reload"
        );

        // cleanup
        std_fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hash_algorithm_from_config_str_accepts_known_values() {
        assert_eq!(
            HashAlgorithm::from_config_str("bcrypt").expect("bcrypt"),
            HashAlgorithm::Bcrypt
        );
        assert_eq!(
            HashAlgorithm::from_config_str("BCRYPT").expect("BCRYPT"),
            HashAlgorithm::Bcrypt
        );
        assert_eq!(
            HashAlgorithm::from_config_str("argon2").expect("argon2"),
            HashAlgorithm::Argon2
        );
        assert_eq!(
            HashAlgorithm::from_config_str("argon2id").expect("argon2id"),
            HashAlgorithm::Argon2
        );
        assert_eq!(
            HashAlgorithm::from_config_str("Argon2ID").expect("Argon2ID"),
            HashAlgorithm::Argon2
        );
        assert!(HashAlgorithm::from_config_str("scrypt").is_err());
        assert!(HashAlgorithm::from_config_str("").is_err());
    }
}

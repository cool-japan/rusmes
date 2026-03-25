//! Pure Rust system authentication backend
//!
//! Reads `/etc/passwd` and `/etc/shadow` directly, replacing the C-based PAM backend.
//! Supports SHA-512 (`$6$`), SHA-256 (`$5$`), bcrypt (`$2b$`/`$2a$`/`$2y$`),
//! and MD5-crypt (`$1$`) password hash verification.

use crate::AuthBackend;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use md5::Md5;
use rusmes_proto::Username;
use sha2::{Digest, Sha256, Sha512};
use std::path::{Path, PathBuf};

// ── Custom base64 alphabet (Drepper / crypt(3)) ────────────────────────────

const CRYPT_B64: &[u8; 64] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Encode `n` least-significant bits from `value` into the crypt base64 alphabet.
fn b64_encode_bits(value: u32, n: usize) -> String {
    let mut out = String::with_capacity(n);
    let mut v = value;
    for _ in 0..n {
        out.push(CRYPT_B64[(v & 0x3f) as usize] as char);
        v >>= 6;
    }
    out
}

// ── SHA-512 crypt ($6$) ─────────────────────────────────────────────────────

/// SHA-512 byte-reordering table derived from the crypt(3) transposition table.
/// Each triple (b2, b1, b0) produces 4 base64 chars via
/// `value = (b2 << 16) | (b1 << 8) | b0`, with characters extracted LSB-first.
/// The final singleton (63) produces 2 chars.
const SHA512_REORDER: [(usize, usize, usize); 21] = [
    (0, 21, 42),
    (22, 43, 1),
    (44, 2, 23),
    (3, 24, 45),
    (25, 46, 4),
    (47, 5, 26),
    (6, 27, 48),
    (28, 49, 7),
    (50, 8, 29),
    (9, 30, 51),
    (31, 52, 10),
    (53, 11, 32),
    (12, 33, 54),
    (34, 55, 13),
    (56, 14, 35),
    (15, 36, 57),
    (37, 58, 16),
    (59, 17, 38),
    (18, 39, 60),
    (40, 61, 19),
    (62, 20, 41),
];

/// Implement the Drepper SHA-512 crypt algorithm (`$6$`).
///
/// Reference: <https://www.akkadia.org/drepper/SHA-crypt.txt>
fn sha512_crypt(password: &[u8], salt: &[u8], rounds: u32) -> String {
    let p_len = password.len();
    let s_len = salt.len();

    // ── Step 1-3: Compute digest B ──────────────────────────────────────
    let digest_b = {
        let mut ctx = Sha512::new();
        ctx.update(password);
        ctx.update(salt);
        ctx.update(password);
        ctx.finalize()
    };

    // ── Step 4-8: Compute digest A ──────────────────────────────────────
    let digest_a = {
        let mut ctx = Sha512::new();
        ctx.update(password); // step 4
        ctx.update(salt); // step 5

        // step 6: add bytes from B, repeating the 64-byte digest as needed
        let mut remaining = p_len;
        while remaining >= 64 {
            ctx.update(&digest_b[..]);
            remaining -= 64;
        }
        if remaining > 0 {
            ctx.update(&digest_b[..remaining]);
        }

        // step 7: for each bit of password length (LSB first)
        let mut n = p_len;
        while n > 0 {
            if n & 1 != 0 {
                ctx.update(&digest_b[..]);
            } else {
                ctx.update(password);
            }
            n >>= 1;
        }

        ctx.finalize()
    };

    // ── Step 9-10: Compute DP (P-bytes) ─────────────────────────────────
    let p_bytes = {
        let mut ctx = Sha512::new();
        for _ in 0..p_len {
            ctx.update(password);
        }
        let dp = ctx.finalize();

        let mut buf = vec![0u8; p_len];
        let mut off = 0;
        while off + 64 <= p_len {
            buf[off..off + 64].copy_from_slice(&dp[..]);
            off += 64;
        }
        if off < p_len {
            buf[off..].copy_from_slice(&dp[..p_len - off]);
        }
        buf
    };

    // ── Step 11-12: Compute DS (S-bytes) ────────────────────────────────
    let s_bytes = {
        let repeat_count = 16 + (digest_a[0] as usize);
        let mut ctx = Sha512::new();
        for _ in 0..repeat_count {
            ctx.update(salt);
        }
        let ds = ctx.finalize();

        let mut buf = vec![0u8; s_len];
        let mut off = 0;
        while off + 64 <= s_len {
            buf[off..off + 64].copy_from_slice(&ds[..]);
            off += 64;
        }
        if off < s_len {
            buf[off..].copy_from_slice(&ds[..s_len - off]);
        }
        buf
    };

    // ── Step 13: Rounds ─────────────────────────────────────────────────
    let mut prev: [u8; 64] = digest_a.into();
    for i in 0..rounds {
        let mut ctx = Sha512::new();
        if i & 1 != 0 {
            ctx.update(&p_bytes);
        } else {
            ctx.update(prev);
        }
        if i % 3 != 0 {
            ctx.update(&s_bytes);
        }
        if i % 7 != 0 {
            ctx.update(&p_bytes);
        }
        if i & 1 != 0 {
            ctx.update(prev);
        } else {
            ctx.update(&p_bytes);
        }
        let out = ctx.finalize();
        prev.copy_from_slice(&out);
    }

    // ── Step 14: Encode final digest ────────────────────────────────────
    let mut encoded = String::with_capacity(86);
    for &(a, b, c) in &SHA512_REORDER {
        let v = (prev[a] as u32) << 16 | (prev[b] as u32) << 8 | (prev[c] as u32);
        encoded.push_str(&b64_encode_bits(v, 4));
    }
    // Final single byte (index 63) → 2 chars
    encoded.push_str(&b64_encode_bits(prev[63] as u32, 2));

    encoded
}

/// Produce the full `$6$...` hash string for the given password and salt.
fn sha512_crypt_full(password: &[u8], raw_salt: &str) -> String {
    let (rounds, salt) = parse_rounds_and_salt(raw_salt);
    // Salt is truncated to 16 characters per the spec.
    let salt = if salt.len() > 16 { &salt[..16] } else { salt };
    let hash = sha512_crypt(password, salt.as_bytes(), rounds);
    if rounds == 5000 {
        format!("$6${salt}${hash}")
    } else {
        format!("$6$rounds={rounds}${salt}${hash}")
    }
}

// ── SHA-256 crypt ($5$) ─────────────────────────────────────────────────────

/// SHA-256 byte-reordering table.  Each triple (a,b,c) → 4 chars;
/// the final pair (30,31) → 3 chars.
const SHA256_REORDER: [(usize, usize, usize); 10] = [
    (0, 10, 20),
    (21, 1, 11),
    (12, 22, 2),
    (3, 13, 23),
    (24, 4, 14),
    (15, 25, 5),
    (6, 16, 26),
    (27, 7, 17),
    (18, 28, 8),
    (9, 19, 29),
];

/// Implement the Drepper SHA-256 crypt algorithm (`$5$`).
fn sha256_crypt(password: &[u8], salt: &[u8], rounds: u32) -> String {
    let p_len = password.len();
    let s_len = salt.len();

    // Digest B
    let digest_b = {
        let mut ctx = Sha256::new();
        ctx.update(password);
        ctx.update(salt);
        ctx.update(password);
        ctx.finalize()
    };

    // Digest A
    let digest_a = {
        let mut ctx = Sha256::new();
        ctx.update(password);
        ctx.update(salt);

        let mut remaining = p_len;
        while remaining >= 32 {
            ctx.update(&digest_b[..]);
            remaining -= 32;
        }
        if remaining > 0 {
            ctx.update(&digest_b[..remaining]);
        }

        let mut n = p_len;
        while n > 0 {
            if n & 1 != 0 {
                ctx.update(&digest_b[..]);
            } else {
                ctx.update(password);
            }
            n >>= 1;
        }

        ctx.finalize()
    };

    // P-bytes
    let p_bytes = {
        let mut ctx = Sha256::new();
        for _ in 0..p_len {
            ctx.update(password);
        }
        let dp = ctx.finalize();

        let mut buf = vec![0u8; p_len];
        let mut off = 0;
        while off + 32 <= p_len {
            buf[off..off + 32].copy_from_slice(&dp[..]);
            off += 32;
        }
        if off < p_len {
            buf[off..].copy_from_slice(&dp[..p_len - off]);
        }
        buf
    };

    // S-bytes
    let s_bytes = {
        let repeat_count = 16 + (digest_a[0] as usize);
        let mut ctx = Sha256::new();
        for _ in 0..repeat_count {
            ctx.update(salt);
        }
        let ds = ctx.finalize();

        let mut buf = vec![0u8; s_len];
        let mut off = 0;
        while off + 32 <= s_len {
            buf[off..off + 32].copy_from_slice(&ds[..]);
            off += 32;
        }
        if off < s_len {
            buf[off..].copy_from_slice(&ds[..s_len - off]);
        }
        buf
    };

    // Rounds
    let mut prev: [u8; 32] = digest_a.into();
    for i in 0..rounds {
        let mut ctx = Sha256::new();
        if i & 1 != 0 {
            ctx.update(&p_bytes);
        } else {
            ctx.update(prev);
        }
        if i % 3 != 0 {
            ctx.update(&s_bytes);
        }
        if i % 7 != 0 {
            ctx.update(&p_bytes);
        }
        if i & 1 != 0 {
            ctx.update(prev);
        } else {
            ctx.update(&p_bytes);
        }
        let out = ctx.finalize();
        prev.copy_from_slice(&out);
    }

    // Encode
    let mut encoded = String::with_capacity(43);
    for &(a, b, c) in &SHA256_REORDER {
        let v = (prev[a] as u32) << 16 | (prev[b] as u32) << 8 | (prev[c] as u32);
        encoded.push_str(&b64_encode_bits(v, 4));
    }
    // Final pair (31, 30) → 3 chars  (note: glibc order is b64_from_24bit(0, final[31], final[30]))
    let v = (prev[31] as u32) << 8 | (prev[30] as u32);
    encoded.push_str(&b64_encode_bits(v, 3));

    encoded
}

/// Produce the full `$5$...` hash string.
fn sha256_crypt_full(password: &[u8], raw_salt: &str) -> String {
    let (rounds, salt) = parse_rounds_and_salt(raw_salt);
    let salt = if salt.len() > 16 { &salt[..16] } else { salt };
    let hash = sha256_crypt(password, salt.as_bytes(), rounds);
    if rounds == 5000 {
        format!("$5${salt}${hash}")
    } else {
        format!("$5$rounds={rounds}${salt}${hash}")
    }
}

// ── MD5-crypt ($1$) ─────────────────────────────────────────────────────────

/// MD5-crypt (`$1$`) implementation per the original Poul-Henning Kamp algorithm.
fn md5_crypt(password: &[u8], salt: &[u8]) -> String {
    let p_len = password.len();

    // Step 1: alternate sum
    let digest_b = {
        let mut ctx = Md5::new();
        ctx.update(password);
        ctx.update(salt);
        ctx.update(password);
        ctx.finalize()
    };

    // Step 2: main sum
    let mut ctx = Md5::new();
    ctx.update(password);
    ctx.update(b"$1$");
    ctx.update(salt);

    // Add bytes from alternate sum
    let mut remaining = p_len;
    while remaining >= 16 {
        ctx.update(&digest_b[..]);
        remaining -= 16;
    }
    if remaining > 0 {
        ctx.update(&digest_b[..remaining]);
    }

    // Bit-pattern of password length
    let mut n = p_len;
    while n > 0 {
        if n & 1 != 0 {
            ctx.update([0u8]);
        } else {
            ctx.update(&password[..1]);
        }
        n >>= 1;
    }

    let mut result: [u8; 16] = ctx.finalize().into();

    // 1000 rounds
    for i in 0..1000u32 {
        let mut ctx2 = Md5::new();
        if i & 1 != 0 {
            ctx2.update(password);
        } else {
            ctx2.update(result);
        }
        if i % 3 != 0 {
            ctx2.update(salt);
        }
        if i % 7 != 0 {
            ctx2.update(password);
        }
        if i & 1 != 0 {
            ctx2.update(result);
        } else {
            ctx2.update(password);
        }
        result = ctx2.finalize().into();
    }

    // MD5-crypt byte reordering and encoding
    let mut encoded = String::with_capacity(22);
    let groups: [(usize, usize, usize); 5] =
        [(0, 6, 12), (1, 7, 13), (2, 8, 14), (3, 9, 15), (4, 10, 5)];
    for (a, b, c) in groups {
        let v = (result[a] as u32) << 16 | (result[b] as u32) << 8 | (result[c] as u32);
        encoded.push_str(&b64_encode_bits(v, 4));
    }
    // Final byte (index 11) → 2 chars
    encoded.push_str(&b64_encode_bits(result[11] as u32, 2));

    encoded
}

fn md5_crypt_full(password: &[u8], salt: &str) -> String {
    let hash = md5_crypt(password, salt.as_bytes());
    format!("$1${salt}${hash}")
}

// ── Utilities ───────────────────────────────────────────────────────────────

/// Parse an optional `rounds=N$` prefix and the salt from the combined string.
///
/// Input examples:
///   - `"saltstring"` → (5000, `"saltstring"`)
///   - `"rounds=10000$saltstring"` → (10000, `"saltstring"`)
fn parse_rounds_and_salt(raw: &str) -> (u32, &str) {
    if let Some(rest) = raw.strip_prefix("rounds=") {
        if let Some(dollar_pos) = rest.find('$') {
            let rounds_str = &rest[..dollar_pos];
            let salt = &rest[dollar_pos + 1..];
            let rounds = rounds_str
                .parse::<u32>()
                .unwrap_or(5000)
                .clamp(1000, 999_999_999);
            return (rounds, salt);
        }
    }
    (5000, raw)
}

/// Verify a password against a crypt-style hash string.
///
/// Supported prefixes:
///   - `$6$` — SHA-512 crypt
///   - `$5$` — SHA-256 crypt
///   - `$1$` — MD5 crypt (legacy)
///   - `$2b$` / `$2a$` / `$2y$` — bcrypt (delegated to the `bcrypt` crate)
fn verify_password(password: &str, hash: &str) -> Result<bool> {
    if hash.starts_with("$6$") {
        let inner = hash
            .strip_prefix("$6$")
            .ok_or_else(|| anyhow!("invalid $6$ hash"))?;
        // inner = [rounds=N$]salt$hash
        let last_dollar = inner
            .rfind('$')
            .ok_or_else(|| anyhow!("malformed $6$ hash: missing final delimiter"))?;
        let raw_salt = &inner[..last_dollar];
        let computed = sha512_crypt_full(password.as_bytes(), raw_salt);
        Ok(computed == hash)
    } else if hash.starts_with("$5$") {
        let inner = hash
            .strip_prefix("$5$")
            .ok_or_else(|| anyhow!("invalid $5$ hash"))?;
        let last_dollar = inner
            .rfind('$')
            .ok_or_else(|| anyhow!("malformed $5$ hash: missing final delimiter"))?;
        let raw_salt = &inner[..last_dollar];
        let computed = sha256_crypt_full(password.as_bytes(), raw_salt);
        Ok(computed == hash)
    } else if hash.starts_with("$1$") {
        let inner = hash
            .strip_prefix("$1$")
            .ok_or_else(|| anyhow!("invalid $1$ hash"))?;
        let last_dollar = inner
            .rfind('$')
            .ok_or_else(|| anyhow!("malformed $1$ hash: missing final delimiter"))?;
        let salt = &inner[..last_dollar];
        let computed = md5_crypt_full(password.as_bytes(), salt);
        Ok(computed == hash)
    } else if hash.starts_with("$2b$") || hash.starts_with("$2a$") || hash.starts_with("$2y$") {
        bcrypt::verify(password, hash).map_err(|e| anyhow!("bcrypt verification error: {e}"))
    } else if hash == "*" || hash == "!" || hash == "!!" || hash.starts_with("!") {
        // Locked / disabled account
        Ok(false)
    } else {
        Err(anyhow!(
            "unsupported password hash scheme: {}",
            &hash[..hash.len().min(10)]
        ))
    }
}

// ── /etc/passwd parsing ─────────────────────────────────────────────────────

/// A parsed entry from `/etc/passwd`.
#[derive(Debug, Clone)]
struct PasswdEntry {
    username: String,
    uid: u32,
    #[allow(dead_code)]
    gid: u32,
    #[allow(dead_code)]
    gecos: String,
    #[allow(dead_code)]
    home: String,
    #[allow(dead_code)]
    shell: String,
}

/// Parse a single `/etc/passwd` line.  Returns `None` for comments, blank lines,
/// or malformed entries.
fn parse_passwd_line(line: &str) -> Option<PasswdEntry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let parts: Vec<&str> = line.split(':').collect();
    if parts.len() < 7 {
        return None;
    }
    let uid = parts[2].parse::<u32>().ok()?;
    let gid = parts[3].parse::<u32>().ok()?;
    Some(PasswdEntry {
        username: parts[0].to_owned(),
        uid,
        gid,
        gecos: parts[4].to_owned(),
        home: parts[5].to_owned(),
        shell: parts[6].to_owned(),
    })
}

/// Read and parse all entries from a passwd-format file.
async fn read_passwd_file(path: &Path) -> Result<Vec<PasswdEntry>> {
    let content = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(content.lines().filter_map(parse_passwd_line).collect())
}

// ── /etc/shadow parsing ─────────────────────────────────────────────────────

/// A parsed entry from `/etc/shadow`.
#[derive(Debug, Clone)]
struct ShadowEntry {
    username: String,
    hash: String,
    /// `true` when the password field starts with `!` or `*` (locked).
    locked: bool,
}

/// Parse a single `/etc/shadow` line.
fn parse_shadow_line(line: &str) -> Option<ShadowEntry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let parts: Vec<&str> = line.splitn(9, ':').collect();
    if parts.len() < 2 {
        return None;
    }
    let username = parts[0].to_owned();
    let raw_hash = parts[1];
    let locked = raw_hash.starts_with('!') || raw_hash.starts_with('*');

    // If locked with "!" prefix the real hash follows after the "!"
    let hash = if raw_hash.starts_with('!') && raw_hash.len() > 1 && raw_hash != "!!" {
        raw_hash[1..].to_owned()
    } else {
        raw_hash.to_owned()
    };

    Some(ShadowEntry {
        username,
        hash,
        locked,
    })
}

/// Read and parse all entries from a shadow-format file.
async fn read_shadow_file(path: &Path) -> Result<Vec<ShadowEntry>> {
    let content = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(content.lines().filter_map(parse_shadow_line).collect())
}

// ── SystemAuthBackend ───────────────────────────────────────────────────────

/// Configuration for the system (passwd/shadow) authentication backend.
#[derive(Debug, Clone)]
pub struct SystemConfig {
    /// Path to the passwd file (default: `/etc/passwd`).
    pub passwd_path: PathBuf,
    /// Path to the shadow file (default: `/etc/shadow`).
    pub shadow_path: PathBuf,
    /// Minimum UID considered a "real" (non-system) user.
    pub min_uid: u32,
    /// When `false`, users with UID < `min_uid` are excluded from
    /// `list_users` and `verify_identity`.
    pub allow_system_users: bool,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            passwd_path: PathBuf::from("/etc/passwd"),
            shadow_path: PathBuf::from("/etc/shadow"),
            min_uid: 1000,
            allow_system_users: false,
        }
    }
}

/// Pure Rust system authentication backend.
///
/// Reads `/etc/passwd` and `/etc/shadow` directly — no C libraries required.
pub struct SystemAuthBackend {
    config: SystemConfig,
}

impl SystemAuthBackend {
    /// Create a new system authentication backend with the given configuration.
    pub fn new(config: SystemConfig) -> Self {
        Self { config }
    }

    /// Helper: is a UID considered a "regular" user?
    fn is_regular_uid(&self, uid: u32) -> bool {
        self.config.allow_system_users || uid >= self.config.min_uid
    }
}

#[async_trait]
impl AuthBackend for SystemAuthBackend {
    async fn authenticate(&self, username: &Username, password: &str) -> Result<bool> {
        let entries = read_shadow_file(&self.config.shadow_path).await?;
        let entry = entries.iter().find(|e| e.username == username.as_str());

        let entry = match entry {
            Some(e) => e,
            None => return Ok(false),
        };

        if entry.locked {
            return Ok(false);
        }

        let hash = entry.hash.clone();
        let pw = password.to_owned();

        // Hash verification can be CPU-intensive; run in a blocking thread.
        tokio::task::spawn_blocking(move || verify_password(&pw, &hash))
            .await
            .map_err(|e| anyhow!("join error: {e}"))?
    }

    async fn verify_identity(&self, username: &Username) -> Result<bool> {
        let entries = read_passwd_file(&self.config.passwd_path).await?;
        Ok(entries
            .iter()
            .any(|e| e.username == username.as_str() && self.is_regular_uid(e.uid)))
    }

    async fn list_users(&self) -> Result<Vec<Username>> {
        let entries = read_passwd_file(&self.config.passwd_path).await?;
        let mut users = Vec::new();
        for entry in &entries {
            if !self.is_regular_uid(entry.uid) {
                continue;
            }
            if let Ok(u) = Username::new(entry.username.clone()) {
                users.push(u);
            }
        }
        Ok(users)
    }

    async fn create_user(&self, _username: &Username, _password: &str) -> Result<()> {
        Err(anyhow!(
            "system backend is read-only; use useradd(8) to create system users"
        ))
    }

    async fn delete_user(&self, _username: &Username) -> Result<()> {
        Err(anyhow!(
            "system backend is read-only; use userdel(8) to delete system users"
        ))
    }

    async fn change_password(&self, _username: &Username, _new_password: &str) -> Result<()> {
        Err(anyhow!(
            "system backend is read-only; use passwd(1) to change passwords"
        ))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ── SHA-512 crypt test vectors (from Drepper spec) ──────────────────

    #[test]
    fn test_sha512_crypt_vector_1() {
        let hash = sha512_crypt_full(b"Hello world!", "saltstring");
        assert_eq!(
            hash,
            "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1"
        );
    }

    #[test]
    fn test_sha512_crypt_vector_2() {
        let hash = sha512_crypt_full(b"Hello world!", "rounds=10000$saltstringsaltstring");
        assert_eq!(
            hash,
            "$6$rounds=10000$saltstringsaltst$OW1/O6BYHV6BcXZu8QVeXbDWra3Oeqh0sbHbbMCVNSnCM/UrjmM0Dp8vOuZeHBy/YTBmSK6H9qs/y3RnOaw5v."
        );
    }

    #[test]
    fn test_sha512_crypt_vector_3_rounds_5000() {
        // rounds=5000 is the default; the hash string must NOT contain
        // "rounds=5000" (it is implied).
        let hash = sha512_crypt_full(b"Hello world!", "rounds=5000$saltstring");
        assert_eq!(
            hash,
            "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1"
        );
    }

    #[test]
    fn test_sha512_crypt_vector_4_long_salt() {
        let hash = sha512_crypt_full(
            b"a]very.]long.]password",
            "rounds=1400$anotherlongsaltstring",
        );
        assert_eq!(
            hash,
            "$6$rounds=1400$anotherlongsalts$Qfvpda9/GjV7Wb8GUTT6zacXCbXD87betTdwA7oey1xJInUU7wpEJ4J2WJ0UIrAePuYGKy86Do7Cdj.JxTpiN."
        );
    }

    #[test]
    fn test_sha512_crypt_vector_5_very_long_salt() {
        let hash = sha512_crypt_full(
            b"we have a short salt string but not a short password",
            "rounds=77777$short",
        );
        assert_eq!(
            hash,
            "$6$rounds=77777$short$WuQyW2YR.hBNpjjRhpYD/ifIw05xdfeEyQoMxIXbkvr0gge1a1x3yRULJ5CCaUeOxFmtlcGZelFl5CxtgfiAc0"
        );
    }

    #[test]
    fn test_sha512_crypt_vector_6_low_rounds() {
        // rounds=1000 is the minimum
        let hash = sha512_crypt_full(
            b"the minimum number is still observed",
            "rounds=1000$roundstoolow",
        );
        assert_eq!(
            hash,
            "$6$rounds=1000$roundstoolow$kUMsbe306n21p9R.FRkW3IGn.S9NPN0x50YhH1xhLsPuWGsUSklZt58jaTfF4ZEQpyUNGc0dqbpBYYBaHHrsX."
        );
    }

    // ── SHA-256 crypt test vectors ──────────────────────────────────────

    #[test]
    fn test_sha256_crypt_vector_1() {
        let hash = sha256_crypt_full(b"Hello world!", "saltstring");
        assert_eq!(
            hash,
            "$5$saltstring$5B8vYYiY.CVt1RlTTf8KbXBH3hsxY/GNooZaBBGWEc5"
        );
    }

    #[test]
    fn test_sha256_crypt_vector_2() {
        let hash = sha256_crypt_full(b"Hello world!", "rounds=10000$saltstringsaltstring");
        assert_eq!(
            hash,
            "$5$rounds=10000$saltstringsaltst$3xv.VbSHBb41AL9AvLeujZkZRBAwqFMz2.opqey6IcA"
        );
    }

    #[test]
    fn test_sha256_crypt_vector_3() {
        let hash = sha256_crypt_full(b"Hello world!", "rounds=5000$saltstring");
        assert_eq!(
            hash,
            "$5$saltstring$5B8vYYiY.CVt1RlTTf8KbXBH3hsxY/GNooZaBBGWEc5"
        );
    }

    #[test]
    fn test_sha256_crypt_vector_4() {
        let hash = sha256_crypt_full(
            b"a]very.]long.]password",
            "rounds=1400$anotherlongsaltstring",
        );
        assert_eq!(
            hash,
            "$5$rounds=1400$anotherlongsalts$8fc8RpnsAEYdbUkzdb0Tt9jps8e3xnDYAbqtN8Gmdl3"
        );
    }

    #[test]
    fn test_sha256_crypt_vector_5() {
        let hash = sha256_crypt_full(
            b"we have a short salt string but not a short password",
            "rounds=77777$short",
        );
        assert_eq!(
            hash,
            "$5$rounds=77777$short$JiO1O3ZpDAxGJeaDIuqCoEFysAe1mZNJRs3pw0KQRd/"
        );
    }

    #[test]
    fn test_sha256_crypt_vector_6() {
        let hash = sha256_crypt_full(
            b"the minimum number is still observed",
            "rounds=1000$roundstoolow",
        );
        assert_eq!(
            hash,
            "$5$rounds=1000$roundstoolow$yfvwcWrQ8l/K0DAWyuPMDNHpIVlTQebY9l/gL972bIC"
        );
    }

    // ── Password verification ───────────────────────────────────────────

    #[test]
    fn test_verify_sha512() {
        let hash = "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1";
        assert!(verify_password("Hello world!", hash).expect("verify ok"));
        assert!(!verify_password("wrong", hash).expect("verify ok"));
    }

    #[test]
    fn test_verify_sha256() {
        let hash = "$5$saltstring$5B8vYYiY.CVt1RlTTf8KbXBH3hsxY/GNooZaBBGWEc5";
        assert!(verify_password("Hello world!", hash).expect("verify ok"));
        assert!(!verify_password("wrong", hash).expect("verify ok"));
    }

    #[test]
    fn test_verify_locked_account() {
        assert!(!verify_password("anything", "!").expect("verify ok"));
        assert!(!verify_password("anything", "*").expect("verify ok"));
        assert!(!verify_password("anything", "!!").expect("verify ok"));
    }

    // ── Passwd parsing ──────────────────────────────────────────────────

    #[test]
    fn test_parse_passwd_line_valid() {
        let entry = parse_passwd_line("alice:x:1001:1001:Alice:/home/alice:/bin/bash");
        let e = entry.expect("should parse");
        assert_eq!(e.username, "alice");
        assert_eq!(e.uid, 1001);
    }

    #[test]
    fn test_parse_passwd_line_system_user() {
        let entry = parse_passwd_line("daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin");
        let e = entry.expect("should parse");
        assert_eq!(e.username, "daemon");
        assert_eq!(e.uid, 1);
    }

    #[test]
    fn test_parse_passwd_line_comment() {
        assert!(parse_passwd_line("# a comment").is_none());
    }

    #[test]
    fn test_parse_passwd_line_empty() {
        assert!(parse_passwd_line("").is_none());
        assert!(parse_passwd_line("   ").is_none());
    }

    #[test]
    fn test_parse_passwd_line_malformed() {
        assert!(parse_passwd_line("no-colons-at-all").is_none());
        assert!(parse_passwd_line("only:two:fields").is_none());
    }

    // ── Shadow parsing ──────────────────────────────────────────────────

    #[test]
    fn test_parse_shadow_line_valid() {
        let line = "alice:$6$salt$hash:19000:0:99999:7:::";
        let e = parse_shadow_line(line).expect("should parse");
        assert_eq!(e.username, "alice");
        assert_eq!(e.hash, "$6$salt$hash");
        assert!(!e.locked);
    }

    #[test]
    fn test_parse_shadow_line_locked_bang() {
        let line = "bob:!$6$salt$hash:19000:0:99999:7:::";
        let e = parse_shadow_line(line).expect("should parse");
        assert_eq!(e.username, "bob");
        assert!(e.locked);
        // The underlying hash (without the "!") is preserved
        assert_eq!(e.hash, "$6$salt$hash");
    }

    #[test]
    fn test_parse_shadow_line_locked_star() {
        let line = "nologin:*:19000:0:99999:7:::";
        let e = parse_shadow_line(line).expect("should parse");
        assert!(e.locked);
    }

    #[test]
    fn test_parse_shadow_line_locked_double_bang() {
        let line = "newuser:!!:19000:0:99999:7:::";
        let e = parse_shadow_line(line).expect("should parse");
        assert!(e.locked);
        assert_eq!(e.hash, "!!");
    }

    #[test]
    fn test_parse_shadow_line_comment_and_empty() {
        assert!(parse_shadow_line("# comment").is_none());
        assert!(parse_shadow_line("").is_none());
    }

    // ── SystemAuthBackend unit tests ────────────────────────────────────

    #[test]
    fn test_system_config_default() {
        let cfg = SystemConfig::default();
        assert_eq!(cfg.passwd_path, PathBuf::from("/etc/passwd"));
        assert_eq!(cfg.shadow_path, PathBuf::from("/etc/shadow"));
        assert_eq!(cfg.min_uid, 1000);
        assert!(!cfg.allow_system_users);
    }

    #[test]
    fn test_system_backend_custom_paths() {
        let cfg = SystemConfig {
            passwd_path: PathBuf::from("/tmp/test_passwd"),
            shadow_path: PathBuf::from("/tmp/test_shadow"),
            min_uid: 500,
            allow_system_users: true,
        };
        let backend = SystemAuthBackend::new(cfg);
        assert_eq!(
            backend.config.passwd_path,
            PathBuf::from("/tmp/test_passwd")
        );
        assert!(backend.config.allow_system_users);
    }

    /// Helper: write content to a temporary file and return its path.
    fn write_temp_file(prefix: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = dir.join(format!("rusmes_test_{}_{}", prefix, ts));
        let mut f = std::fs::File::create(&path).expect("create temp file");
        f.write_all(content.as_bytes()).expect("write temp file");
        path
    }

    #[tokio::test]
    async fn test_authenticate_sha512() {
        // "testpass" hashed with $6$testsalt$
        let pw_hash = sha512_crypt_full(b"testpass", "testsalt");
        let shadow_content = format!("alice:{}:19000:0:99999:7:::\n", pw_hash);
        let shadow_path = write_temp_file("shadow_auth", &shadow_content);

        let passwd_content = "alice:x:1001:1001:Alice:/home/alice:/bin/bash\n";
        let passwd_path = write_temp_file("passwd_auth", passwd_content);

        let backend = SystemAuthBackend::new(SystemConfig {
            passwd_path: passwd_path.clone(),
            shadow_path: shadow_path.clone(),
            ..SystemConfig::default()
        });

        let user = Username::new("alice").expect("valid username");
        assert!(backend
            .authenticate(&user, "testpass")
            .await
            .expect("auth ok"));
        assert!(!backend.authenticate(&user, "wrong").await.expect("auth ok"));

        // Clean up
        let _ = std::fs::remove_file(&shadow_path);
        let _ = std::fs::remove_file(&passwd_path);
    }

    #[tokio::test]
    async fn test_authenticate_locked_account() {
        let shadow_content = "bob:!$6$salt$fakehash:19000:0:99999:7:::\n";
        let shadow_path = write_temp_file("shadow_locked", shadow_content);

        let backend = SystemAuthBackend::new(SystemConfig {
            shadow_path: shadow_path.clone(),
            ..SystemConfig::default()
        });

        let user = Username::new("bob").expect("valid username");
        assert!(!backend
            .authenticate(&user, "anything")
            .await
            .expect("auth ok"));

        let _ = std::fs::remove_file(&shadow_path);
    }

    #[tokio::test]
    async fn test_verify_identity() {
        let content = "\
root:x:0:0:root:/root:/bin/bash
alice:x:1001:1001:Alice:/home/alice:/bin/bash
bob:x:1002:1002:Bob:/home/bob:/bin/bash
";
        let path = write_temp_file("passwd_verify", content);

        let backend = SystemAuthBackend::new(SystemConfig {
            passwd_path: path.clone(),
            ..SystemConfig::default()
        });

        let alice = Username::new("alice").expect("valid");
        let root = Username::new("root").expect("valid");
        let ghost = Username::new("ghost").expect("valid");

        assert!(backend.verify_identity(&alice).await.expect("ok"));
        // root has uid 0 < 1000 → excluded by default
        assert!(!backend.verify_identity(&root).await.expect("ok"));
        assert!(!backend.verify_identity(&ghost).await.expect("ok"));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_list_users_filters_system() {
        let content = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin
alice:x:1001:1001:Alice:/home/alice:/bin/bash
bob:x:1002:1002:Bob:/home/bob:/bin/bash
";
        let path = write_temp_file("passwd_list", content);

        let backend = SystemAuthBackend::new(SystemConfig {
            passwd_path: path.clone(),
            ..SystemConfig::default()
        });

        let users = backend.list_users().await.expect("ok");
        let names: Vec<String> = users.iter().map(|u| u.as_str().to_owned()).collect();

        assert_eq!(names, vec!["alice", "bob"]);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_list_users_allow_system() {
        let content = "\
root:x:0:0:root:/root:/bin/bash
alice:x:1001:1001:Alice:/home/alice:/bin/bash
";
        let path = write_temp_file("passwd_allow_sys", content);

        let backend = SystemAuthBackend::new(SystemConfig {
            passwd_path: path.clone(),
            allow_system_users: true,
            ..SystemConfig::default()
        });

        let users = backend.list_users().await.expect("ok");
        assert_eq!(users.len(), 2);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_create_delete_change_return_errors() {
        let backend = SystemAuthBackend::new(SystemConfig::default());
        let user = Username::new("test").expect("valid");

        assert!(backend.create_user(&user, "pw").await.is_err());
        assert!(backend.delete_user(&user).await.is_err());
        assert!(backend.change_password(&user, "new").await.is_err());
    }

    // ── rounds parsing ──────────────────────────────────────────────────

    #[test]
    fn test_parse_rounds_and_salt_default() {
        let (rounds, salt) = parse_rounds_and_salt("mysalt");
        assert_eq!(rounds, 5000);
        assert_eq!(salt, "mysalt");
    }

    #[test]
    fn test_parse_rounds_and_salt_explicit() {
        let (rounds, salt) = parse_rounds_and_salt("rounds=10000$mysalt");
        assert_eq!(rounds, 10000);
        assert_eq!(salt, "mysalt");
    }

    #[test]
    fn test_parse_rounds_and_salt_clamped_low() {
        let (rounds, _) = parse_rounds_and_salt("rounds=100$mysalt");
        assert_eq!(rounds, 1000);
    }

    // ── MD5-crypt ───────────────────────────────────────────────────────

    #[test]
    fn test_md5_crypt_basic() {
        // Reference vector: password "password", salt "3edqd5Yh"
        // Generated by: openssl passwd -1 -salt 3edqd5Yh password
        // → $1$3edqd5Yh$SE3KgrxqSR.n5oJB/Me561
        //
        // We verify round-trip: hash then verify.
        let hash = md5_crypt_full(b"password", "3edqd5Yh");
        assert!(hash.starts_with("$1$3edqd5Yh$"));
        assert!(verify_password("password", &hash).expect("verify ok"));
    }

    // ── b64_encode_bits ─────────────────────────────────────────────────

    #[test]
    fn test_b64_encode_bits_zero() {
        assert_eq!(b64_encode_bits(0, 4), "....");
    }

    #[test]
    fn test_b64_encode_bits_single() {
        // value 1 → first char at index 1 = '/'
        assert_eq!(b64_encode_bits(1, 1), "/");
    }
}

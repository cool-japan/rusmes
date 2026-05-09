//! RFC 5256 email threading engine for the filesystem storage backend.
//!
//! A `ThreadingEngine` maintains a persistent thread index per mailbox, stored
//! as a JSON file at `{mailbox_dir}/.thread_index.json`. It implements the
//! RFC 5256 References-chain algorithm with a normalized-subject fallback.

use rusmes_proto::Mail;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-mailbox thread index: maps message identifiers (RFC 5322 Message-ID
/// strings, stripped of angle-brackets, or `"subj:{normalized}"` sentinel
/// keys) to stable thread ID strings.
type ThreadIndex = HashMap<String, String>;

/// RFC 5256 threading engine for a single mailbox.
///
/// The engine persists a `HashMap<String, String>` thread index alongside the
/// mailbox directory. Callers should create one instance per `append_message`
/// call; the index is loaded from disk on each `assign_thread_id` call.
pub struct ThreadingEngine {
    /// Path to the `.thread_index.json` file for this mailbox.
    index_path: PathBuf,
}

impl ThreadingEngine {
    /// Create a new `ThreadingEngine` for the given mailbox directory.
    pub fn new(mailbox_dir: &Path) -> Self {
        Self {
            index_path: mailbox_dir.join(".thread_index.json"),
        }
    }

    /// Assign a thread ID to `mail` using the RFC 5256 algorithm.
    ///
    /// Steps:
    /// 1. Extract the RFC 5322 `Message-ID` header value (stripped of `<>`).
    ///    Falls back to the internal UUID when the header is absent.
    /// 2. Collect all reference IDs from `References` and `In-Reply-To` headers.
    /// 3. Load the on-disk index; find an existing thread via reference lookup
    ///    then subject-line fallback.
    /// 4. Assign (or create) a thread ID; persist the updated index atomically.
    pub async fn assign_thread_id(&self, mail: &Mail) -> anyhow::Result<String> {
        let message = mail.message();
        let headers = message.headers();

        // --- 1. Extract RFC 5322 Message-ID ---
        let rfc_message_id: String = headers
            .get_first("message-id")
            .map(strip_angle_brackets)
            .unwrap_or_else(|| format!("uuid:{}", mail.message_id()));

        // --- 2. Collect references ---
        let mut refs: Vec<String> = Vec::new();

        // Parse References header (space/comma-separated)
        if let Some(references_hdr) = headers.get_first("references") {
            parse_message_id_list(references_hdr, &mut refs);
        }

        // Parse In-Reply-To header
        if let Some(in_reply_to_hdr) = headers.get_first("in-reply-to") {
            parse_message_id_list(in_reply_to_hdr, &mut refs);
        }

        // Deduplicate while preserving order.
        dedup_keep_order(&mut refs);

        // --- 3. Load the on-disk index ---
        let mut index = load_index(&self.index_path).await?;

        // --- 4. Find an existing thread (References chain first) ---
        let mut found_thread_id: Option<String> = None;

        for ref_id in &refs {
            if let Some(tid) = index.get(ref_id) {
                found_thread_id = Some(tid.clone());
                break;
            }
        }

        // Subject-line fallback when no reference matched.
        if found_thread_id.is_none() {
            let normalized = headers
                .get_first("subject")
                .map(normalize_subject)
                .unwrap_or_default();
            if !normalized.is_empty() {
                let subj_key = format!("subj:{}", normalized);
                if let Some(tid) = index.get(&subj_key) {
                    found_thread_id = Some(tid.clone());
                }
            }
        }

        // --- 5. Assign thread ID ---
        let thread_id = found_thread_id.unwrap_or_else(|| {
            // New thread: first 16 hex chars of SHA-256(message_id).
            let mut hasher = Sha256::new();
            hasher.update(rfc_message_id.as_bytes());
            let digest = hasher.finalize();
            format!("{:x}", digest).chars().take(16).collect()
        });

        // --- 6. Update index ---
        index.insert(rfc_message_id.clone(), thread_id.clone());

        let normalized_subj = headers
            .get_first("subject")
            .map(normalize_subject)
            .unwrap_or_default();
        if !normalized_subj.is_empty() {
            let subj_key = format!("subj:{}", normalized_subj);
            // Only store if not already there (first message in thread "owns" the subject key).
            index.entry(subj_key).or_insert_with(|| thread_id.clone());
        }

        // --- 7. Persist atomically ---
        persist_index(&self.index_path, &index).await?;

        Ok(thread_id)
    }

    /// Look up the thread ID for a known RFC 5322 `Message-ID` string.
    ///
    /// Returns `Ok(None)` when the message is not yet in the index.
    pub async fn get_thread_id(&self, message_id: &str) -> anyhow::Result<Option<String>> {
        let normalized = strip_angle_brackets(message_id);
        let index = load_index(&self.index_path).await?;
        Ok(index.get(&normalized).cloned())
    }
}

// ---------------------------------------------------------------------------
// Helpers (pub(super) for use by the filesystem mod.rs)
// ---------------------------------------------------------------------------

/// Strip surrounding angle-brackets from a Message-ID value and trim whitespace.
pub(super) fn strip_angle_brackets(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Parse a space- or comma-separated list of Message-IDs into `out`.
///
/// Each token is stripped of angle-brackets and empty tokens are ignored.
fn parse_message_id_list(value: &str, out: &mut Vec<String>) {
    for token in value.split([' ', ',', '\t', '\n', '\r']) {
        let stripped = strip_angle_brackets(token);
        if !stripped.is_empty() {
            out.push(stripped);
        }
    }
}

/// Remove duplicates from `v` while preserving original order.
fn dedup_keep_order(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|item| seen.insert(item.clone()));
}

/// Normalize a subject line for thread matching.
///
/// Rules (applied in order, repeatedly):
/// - Strip leading/trailing whitespace
/// - Fold ASCII to lowercase
/// - Strip `Re:`, `Fwd:`, `Fw:` prefixes (case-insensitive)
/// - Strip bracketed `[…]` prefixes (e.g. `[list-name]`)
fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim().to_lowercase();

    // Repeatedly strip known reply/forward prefixes and bracketed tags.
    loop {
        let before = s.clone();

        // Strip Re:, Fwd:, Fw: (with optional whitespace after colon)
        for prefix in &["re:", "fwd:", "fw:"] {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest.trim_start().to_string();
            }
        }

        // Strip leading [tag] brackets.
        if s.starts_with('[') {
            if let Some(end) = s.find(']') {
                s = s[end + 1..].trim_start().to_string();
            }
        }

        if s == before {
            break;
        }
    }

    s
}

/// Load the thread index from disk.
///
/// Returns an empty `HashMap` if the file does not exist.
async fn load_index(path: &Path) -> anyhow::Result<ThreadIndex> {
    if !tokio::fs::try_exists(path).await.unwrap_or(false) {
        return Ok(HashMap::new());
    }

    let bytes = tokio::fs::read(path).await?;
    let index: ThreadIndex = serde_json::from_slice(&bytes).unwrap_or_else(|_| HashMap::new());
    Ok(index)
}

/// Persist the thread index to disk atomically.
///
/// Writes to a `.tmp` file first, then renames into place.
async fn persist_index(path: &Path, index: &ThreadIndex) -> anyhow::Result<()> {
    let json = serde_json::to_vec(index)
        .map_err(|e| anyhow::anyhow!("Failed to serialize thread index: {}", e))?;

    let tmp_path = path.with_extension("json.tmp");

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    tokio::fs::write(&tmp_path, &json).await?;
    tokio::fs::rename(&tmp_path, path).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, Mail, MessageBody, MimeMessage};

    /// Build a minimal `Mail` with the given headers.
    fn make_mail(headers: Vec<(&str, &str)>) -> Mail {
        let mut hmap = HeaderMap::new();
        for (name, value) in headers {
            hmap.insert(name, value.to_string());
        }
        let body = MessageBody::Small(Bytes::from("test body"));
        let mime = MimeMessage::new(hmap, body);
        Mail::new(None, vec![], mime, None, None)
    }

    #[tokio::test]
    async fn test_new_message_gets_new_thread_id() {
        let dir = std::env::temp_dir().join(format!("threading-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let engine = ThreadingEngine::new(&dir);
        let mail = make_mail(vec![
            ("message-id", "<msg001@example.com>"),
            ("subject", "Hello world"),
        ]);

        let tid = engine.assign_thread_id(&mail).await.unwrap();
        assert!(!tid.is_empty(), "thread_id must not be empty");
        assert_eq!(tid.len(), 16, "thread_id must be 16 hex chars");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn test_reply_gets_same_thread_id() {
        let dir = std::env::temp_dir().join(format!("threading-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let engine = ThreadingEngine::new(&dir);

        // Original message
        let original = make_mail(vec![
            ("message-id", "<original@example.com>"),
            ("subject", "Original topic"),
        ]);
        let original_tid = engine.assign_thread_id(&original).await.unwrap();

        // Reply with In-Reply-To
        let reply = make_mail(vec![
            ("message-id", "<reply001@example.com>"),
            ("in-reply-to", "<original@example.com>"),
            ("subject", "Re: Original topic"),
        ]);
        let reply_tid = engine.assign_thread_id(&reply).await.unwrap();

        assert_eq!(
            original_tid, reply_tid,
            "Reply must share thread_id with original"
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn test_references_chain_assigns_thread() {
        let dir = std::env::temp_dir().join(format!("threading-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let engine = ThreadingEngine::new(&dir);

        // Root message
        let root = make_mail(vec![
            ("message-id", "<root@example.com>"),
            ("subject", "Root thread"),
        ]);
        let root_tid = engine.assign_thread_id(&root).await.unwrap();

        // Intermediate reply
        let mid = make_mail(vec![
            ("message-id", "<mid@example.com>"),
            ("references", "<root@example.com>"),
            ("subject", "Re: Root thread"),
        ]);
        let mid_tid = engine.assign_thread_id(&mid).await.unwrap();
        assert_eq!(root_tid, mid_tid, "Mid reply must be in same thread");

        // Late reply via References chain (references root and mid)
        let late = make_mail(vec![
            ("message-id", "<late@example.com>"),
            ("references", "<root@example.com> <mid@example.com>"),
            ("subject", "Re: Root thread"),
        ]);
        let late_tid = engine.assign_thread_id(&late).await.unwrap();
        assert_eq!(root_tid, late_tid, "Late reply must be in same thread");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn test_subject_fallback_threading() {
        let dir = std::env::temp_dir().join(format!("threading-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let engine = ThreadingEngine::new(&dir);

        // Original message
        let first = make_mail(vec![
            ("message-id", "<first@example.com>"),
            ("subject", "Meeting tomorrow"),
        ]);
        let first_tid = engine.assign_thread_id(&first).await.unwrap();

        // Another message: different Message-ID, no references, same normalized subject
        let second = make_mail(vec![
            ("message-id", "<second@example.com>"),
            ("subject", "Re: Meeting tomorrow"),
        ]);
        let second_tid = engine.assign_thread_id(&second).await.unwrap();

        assert_eq!(
            first_tid, second_tid,
            "Same normalized subject must produce same thread_id via fallback"
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn test_different_subjects_different_threads() {
        let dir = std::env::temp_dir().join(format!("threading-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let engine = ThreadingEngine::new(&dir);

        let first = make_mail(vec![
            ("message-id", "<alpha@example.com>"),
            ("subject", "Alpha topic"),
        ]);
        let second = make_mail(vec![
            ("message-id", "<beta@example.com>"),
            ("subject", "Beta topic"),
        ]);

        let tid_a = engine.assign_thread_id(&first).await.unwrap();
        let tid_b = engine.assign_thread_id(&second).await.unwrap();

        assert_ne!(
            tid_a, tid_b,
            "Different subjects must produce different thread IDs"
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn test_strip_angle_brackets() {
        assert_eq!(strip_angle_brackets("<foo@bar.com>"), "foo@bar.com");
        assert_eq!(strip_angle_brackets("foo@bar.com"), "foo@bar.com");
        assert_eq!(
            strip_angle_brackets("  <spaced@bar.com>  "),
            "spaced@bar.com"
        );
    }

    #[test]
    fn test_normalize_subject() {
        assert_eq!(normalize_subject("Re: Hello"), "hello");
        assert_eq!(normalize_subject("Fwd: Test"), "test");
        assert_eq!(normalize_subject("FW: Test"), "test");
        assert_eq!(normalize_subject("[List] Re: Hello"), "hello");
        assert_eq!(normalize_subject("Hello World"), "hello world");
        assert_eq!(normalize_subject("Re: Re: Deep"), "deep");
    }
}

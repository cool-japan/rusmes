//! Submission persistence layer — `SubmissionStore` trait and filesystem backend

use crate::methods::submission::types::{EmailSubmission, UndoStatus};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ── Internal state ────────────────────────────────────────────────────────────

/// Account-scoped state file stored at `{base_dir}/submissions/{account_id}.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SubmissionAccountState {
    /// Map of submission id → StoredSubmission
    pub submissions: HashMap<String, StoredSubmission>,
    /// Monotonic version counter; incremented on every mutation
    pub state_version: u64,
}

impl Default for SubmissionAccountState {
    fn default() -> Self {
        Self {
            submissions: HashMap::new(),
            state_version: 1,
        }
    }
}

/// Persisted form of an `EmailSubmission`.
///
/// We keep `created_at` alongside the public fields so we can enforce the
/// undo-window check without relying on wall-clock drift in tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSubmission {
    /// Public submission object
    #[serde(flatten)]
    pub submission: EmailSubmission,
    /// UTC timestamp at which the submission was created (used for undo window)
    pub created_at: DateTime<Utc>,
}

// ── SubmissionStore trait ─────────────────────────────────────────────────────

/// Trait for submission persistence.
#[async_trait]
pub trait SubmissionStore: Send + Sync {
    /// Return a submission by id, or `None` if not found.
    async fn get_submission(
        &self,
        account_id: &str,
        id: &str,
    ) -> anyhow::Result<Option<StoredSubmission>>;

    /// Persist a new or updated submission.
    async fn put_submission(&self, account_id: &str, entry: StoredSubmission)
        -> anyhow::Result<()>;

    /// Delete a submission by id.  Returns `Ok(())` even if id is absent.
    async fn delete_submission(&self, account_id: &str, id: &str) -> anyhow::Result<()>;

    /// Return the current state token for an account.
    async fn state_token(&self, account_id: &str) -> anyhow::Result<String>;
}

// ── FileSubmissionStore ───────────────────────────────────────────────────────

/// Filesystem-backed submission store.
///
/// Each account's submissions are persisted to
/// `{base_dir}/submissions/{account_id}.json`.
pub struct FileSubmissionStore {
    base_dir: PathBuf,
}

impl FileSubmissionStore {
    /// Create a new store rooted at `base_dir`.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    fn account_path(&self, account_id: &str) -> PathBuf {
        self.base_dir
            .join("submissions")
            .join(format!("{}.json", account_id))
    }

    async fn load(&self, account_id: &str) -> anyhow::Result<SubmissionAccountState> {
        let path = self.account_path(account_id);
        if !path.exists() {
            return Ok(SubmissionAccountState::default());
        }
        let bytes = tokio::fs::read(&path).await?;
        let state: SubmissionAccountState = serde_json::from_slice(&bytes)?;
        Ok(state)
    }

    async fn save(&self, account_id: &str, state: &SubmissionAccountState) -> anyhow::Result<()> {
        let path = self.account_path(account_id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = serde_json::to_vec_pretty(state)?;
        tokio::fs::write(&path, bytes).await?;
        Ok(())
    }
}

#[async_trait]
impl SubmissionStore for FileSubmissionStore {
    async fn get_submission(
        &self,
        account_id: &str,
        id: &str,
    ) -> anyhow::Result<Option<StoredSubmission>> {
        let state = self.load(account_id).await?;
        Ok(state.submissions.get(id).cloned())
    }

    async fn put_submission(
        &self,
        account_id: &str,
        entry: StoredSubmission,
    ) -> anyhow::Result<()> {
        let mut state = self.load(account_id).await?;
        state.submissions.insert(entry.submission.id.clone(), entry);
        state.state_version += 1;
        self.save(account_id, &state).await
    }

    async fn delete_submission(&self, account_id: &str, id: &str) -> anyhow::Result<()> {
        let mut state = self.load(account_id).await?;
        state.submissions.remove(id);
        state.state_version += 1;
        self.save(account_id, &state).await
    }

    async fn state_token(&self, account_id: &str) -> anyhow::Result<String> {
        let state = self.load(account_id).await?;
        Ok(state.state_version.to_string())
    }
}

/// Check whether a `StoredSubmission` can be canceled.
///
/// Returns `true` if the submission is `pending` and within the undo window.
pub(super) fn within_undo_window(stored: &StoredSubmission, window_secs: i64) -> bool {
    stored.submission.undo_status == UndoStatus::Pending
        && Utc::now() <= stored.created_at + chrono::Duration::seconds(window_secs)
}

//! PostgreSQL metadata store implementation (quotas, counters).

use crate::traits::MetadataStore;
use crate::types::{MailboxCounters, MailboxId, Quota};
use async_trait::async_trait;
use rusmes_proto::Username;
use sqlx::postgres::PgPool;
use sqlx::Row;

/// PostgreSQL metadata store implementation
pub(super) struct PostgresMetadataStore {
    pub(super) pool: PgPool,
}

#[async_trait]
impl MetadataStore for PostgresMetadataStore {
    async fn get_user_quota(&self, user: &Username) -> anyhow::Result<Quota> {
        let row = sqlx::query("SELECT used, quota_limit FROM user_quotas WHERE username = $1")
            .bind(user.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get quota: {}", e))?;

        match row {
            Some(r) => {
                let used: i64 = r.try_get("used")?;
                let limit: i64 = r.try_get("quota_limit")?;
                Ok(Quota::new(used as u64, limit as u64))
            }
            None => Ok(Quota::new(0, 1024 * 1024 * 1024)), // Default 1GB
        }
    }

    async fn set_user_quota(&self, user: &Username, quota: Quota) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO user_quotas (username, used, quota_limit)
            VALUES ($1, $2, $3)
            ON CONFLICT (username) DO UPDATE
            SET used = $2, quota_limit = $3, updated_at = NOW()
            "#,
        )
        .bind(user.to_string())
        .bind(quota.used as i64)
        .bind(quota.limit as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set quota: {}", e))?;

        Ok(())
    }

    async fn get_mailbox_counters(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<MailboxCounters> {
        let row = sqlx::query(
            r#"
            SELECT
                COUNT(*) as total,
                COUNT(*) FILTER (WHERE f.flag_recent = TRUE) as recent,
                COUNT(*) FILTER (WHERE f.flag_seen = FALSE) as unseen
            FROM messages m
            LEFT JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get counters: {}", e))?;

        let total: i64 = row.try_get("total")?;
        let recent: i64 = row.try_get("recent")?;
        let unseen: i64 = row.try_get("unseen")?;

        Ok(MailboxCounters {
            exists: total as u32,
            recent: recent as u32,
            unseen: unseen as u32,
        })
    }
}

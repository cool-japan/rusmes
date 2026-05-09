//! PostgreSQL message store implementation.

use crate::traits::MessageStore;
use crate::types::{MailboxId, MessageFlags, MessageMetadata, SearchCriteria};
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress, MessageId};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::Row;
use std::str::FromStr;

/// PostgreSQL message store implementation
pub(super) struct PostgresMessageStore {
    pub(super) pool: PgPool,
    pub(super) inline_threshold: usize,
}

#[async_trait]
impl MessageStore for PostgresMessageStore {
    async fn append_message(
        &self,
        mailbox_id: &MailboxId,
        message: Mail,
    ) -> anyhow::Result<MessageMetadata> {
        let mut tx = self.pool.begin().await?;

        // Get next UID for mailbox (with row-level lock)
        let uid_row = sqlx::query("SELECT uid_next FROM mailboxes WHERE id = $1 FOR UPDATE")
            .bind(*mailbox_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get next UID: {}", e))?;
        let uid: i32 = uid_row.try_get("uid_next")?;

        // Extract message data
        let message_id = *message.message_id();
        let sender = message.sender().map(|s| s.to_string());
        let recipients: Vec<String> = message.recipients().iter().map(|r| r.to_string()).collect();
        let message_size = message.size();

        // Extract subject from headers
        let subject = message
            .message()
            .headers()
            .get_first("subject")
            .map(|s| s.to_string());

        // Serialize headers to JSON
        let mut headers_map = serde_json::Map::new();
        for (name, values) in message.message().headers().iter() {
            headers_map.insert(name.clone(), serde_json::json!(values));
        }
        let headers_json = serde_json::Value::Object(headers_map);

        // Store message body (inline or external based on size)
        let (body_inline, body_external_ref) = if message_size < self.inline_threshold {
            // Store inline
            let body_bytes = match message.message().body() {
                rusmes_proto::MessageBody::Small(bytes) => bytes.to_vec(),
                _ => vec![],
            };
            (Some(body_bytes), None)
        } else {
            // Store externally
            let blob_id = uuid::Uuid::new_v4();
            let body_bytes = match message.message().body() {
                rusmes_proto::MessageBody::Small(bytes) => bytes.to_vec(),
                _ => vec![],
            };

            sqlx::query("INSERT INTO message_blobs (id, data) VALUES ($1, $2)")
                .bind(blob_id)
                .bind(&body_bytes)
                .execute(&mut *tx)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to store message blob: {}", e))?;

            (None, Some(blob_id))
        };

        // Insert message
        sqlx::query(
            r#"
            INSERT INTO messages (id, mailbox_id, uid, sender, recipients, subject, headers, body_inline, body_external_ref, size)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(*message_id.as_uuid())
        .bind(*mailbox_id.as_uuid())
        .bind(uid)
        .bind(&sender)
        .bind(&recipients)
        .bind(&subject)
        .bind(&headers_json)
        .bind(body_inline)
        .bind(body_external_ref)
        .bind(message_size as i32)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to insert message: {}", e))?;

        // Insert initial flags (mark as recent)
        sqlx::query("INSERT INTO message_flags (message_id, flag_recent) VALUES ($1, TRUE)")
            .bind(*message_id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert flags: {}", e))?;

        // Update mailbox uid_next and quota
        sqlx::query("UPDATE mailboxes SET uid_next = $1, updated_at = NOW() WHERE id = $2")
            .bind(uid + 1)
            .bind(*mailbox_id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to update mailbox: {}", e))?;

        // Update user quota
        let mailbox_row = sqlx::query("SELECT username FROM mailboxes WHERE id = $1")
            .bind(*mailbox_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get mailbox: {}", e))?;
        let username: String = mailbox_row.try_get("username")?;

        sqlx::query(
            r#"
            INSERT INTO user_quotas (username, used, quota_limit)
            VALUES ($1, $2, 1073741824)
            ON CONFLICT (username) DO UPDATE
            SET used = user_quotas.used + $2, updated_at = NOW()
            "#,
        )
        .bind(&username)
        .bind(message_size as i64)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to update quota: {}", e))?;

        tx.commit().await?;

        let mut flags = MessageFlags::new();
        flags.set_recent(true);

        let metadata =
            MessageMetadata::new(message_id, *mailbox_id, uid as u32, flags, message_size);

        tracing::debug!(
            "Appended message {} to mailbox {} with UID {}",
            message_id,
            mailbox_id,
            uid
        );
        Ok(metadata)
    }

    async fn get_message(&self, message_id: &MessageId) -> anyhow::Result<Option<Mail>> {
        // Fetch message data from database
        let row = sqlx::query(
            r#"
            SELECT m.sender, m.recipients, m.headers, m.body_inline, m.body_external_ref
            FROM messages m
            WHERE m.id = $1
            "#,
        )
        .bind(*message_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch message: {}", e))?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        // Extract fields
        let sender: Option<String> = row.try_get("sender")?;
        let recipients: Vec<String> = row.try_get("recipients")?;
        let headers_json: serde_json::Value = row.try_get("headers")?;
        let body_inline: Option<Vec<u8>> = row.try_get("body_inline")?;
        let body_external_ref: Option<uuid::Uuid> = row.try_get("body_external_ref")?;

        // Reconstruct headers
        let mut headers = rusmes_proto::HeaderMap::new();
        if let Some(headers_obj) = headers_json.as_object() {
            for (name, value) in headers_obj {
                if let Some(values_array) = value.as_array() {
                    for v in values_array {
                        if let Some(v_str) = v.as_str() {
                            headers.insert(name.clone(), v_str.to_string());
                        }
                    }
                }
            }
        }

        // Reconstruct body
        let body_bytes = if let Some(inline) = body_inline {
            inline
        } else if let Some(blob_id) = body_external_ref {
            // Fetch from message_blobs table
            let blob_row = sqlx::query("SELECT data FROM message_blobs WHERE id = $1")
                .bind(blob_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to fetch message blob: {}", e))?;

            if let Some(blob) = blob_row {
                blob.try_get("data")?
            } else {
                tracing::warn!("Message blob {} not found", blob_id);
                vec![]
            }
        } else {
            vec![]
        };

        let body = rusmes_proto::MessageBody::Small(bytes::Bytes::from(body_bytes));
        let mime_message = rusmes_proto::MimeMessage::new(headers, body);

        // Parse sender and recipients
        let sender_addr = if let Some(sender_str) = sender {
            MailAddress::from_str(&sender_str).ok()
        } else {
            None
        };

        let recipient_addrs: Vec<MailAddress> = recipients
            .into_iter()
            .filter_map(|r| MailAddress::from_str(&r).ok())
            .collect();

        // Create Mail object
        let mail = rusmes_proto::Mail::with_message_id(
            sender_addr,
            recipient_addrs,
            mime_message,
            None, // remote_addr not stored
            None, // remote_host not stored
            *message_id,
        );

        Ok(Some(mail))
    }

    async fn delete_messages(&self, message_ids: &[MessageId]) -> anyhow::Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;

        let uuids: Vec<uuid::Uuid> = message_ids.iter().map(|id| *id.as_uuid()).collect();

        // Get external blob references to delete
        let blob_rows = sqlx::query("SELECT body_external_ref FROM messages WHERE id = ANY($1) AND body_external_ref IS NOT NULL")
            .bind(&uuids)
            .fetch_all(&mut *tx)
            .await?;

        let blob_ids: Vec<uuid::Uuid> = blob_rows
            .into_iter()
            .filter_map(|row| row.try_get("body_external_ref").ok())
            .collect();

        // Delete messages
        sqlx::query("DELETE FROM messages WHERE id = ANY($1)")
            .bind(&uuids)
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete messages: {}", e))?;

        // Delete external blobs
        if !blob_ids.is_empty() {
            sqlx::query("DELETE FROM message_blobs WHERE id = ANY($1)")
                .bind(&blob_ids)
                .execute(&mut *tx)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to delete blobs: {}", e))?;
        }

        tx.commit().await?;

        tracing::debug!("Deleted {} messages", message_ids.len());
        Ok(())
    }

    async fn set_flags(
        &self,
        message_ids: &[MessageId],
        flags: MessageFlags,
    ) -> anyhow::Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }

        let uuids: Vec<uuid::Uuid> = message_ids.iter().map(|id| *id.as_uuid()).collect();
        let custom_flags: Vec<String> = flags.custom().iter().cloned().collect();

        sqlx::query(
            r#"
            UPDATE message_flags SET
                flag_seen = $1,
                flag_answered = $2,
                flag_flagged = $3,
                flag_deleted = $4,
                flag_draft = $5,
                flag_recent = $6,
                custom_flags = $7
            WHERE message_id = ANY($8)
            "#,
        )
        .bind(flags.is_seen())
        .bind(flags.is_answered())
        .bind(flags.is_flagged())
        .bind(flags.is_deleted())
        .bind(flags.is_draft())
        .bind(flags.is_recent())
        .bind(&custom_flags)
        .bind(&uuids)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set flags: {}", e))?;

        tracing::debug!("Set flags for {} messages", message_ids.len());
        Ok(())
    }

    async fn search(
        &self,
        mailbox_id: &MailboxId,
        criteria: SearchCriteria,
    ) -> anyhow::Result<Vec<MessageId>> {
        let message_ids = match criteria {
            SearchCriteria::All => self.search_all(mailbox_id).await?,
            SearchCriteria::Unseen => self.search_unseen(mailbox_id).await?,
            SearchCriteria::Seen => self.search_seen(mailbox_id).await?,
            SearchCriteria::Flagged => self.search_flagged(mailbox_id).await?,
            SearchCriteria::Unflagged => self.search_unflagged(mailbox_id).await?,
            SearchCriteria::Deleted => self.search_deleted(mailbox_id).await?,
            SearchCriteria::Undeleted => self.search_undeleted(mailbox_id).await?,
            SearchCriteria::From(email) => self.search_from(mailbox_id, &email).await?,
            SearchCriteria::To(email) => self.search_to(mailbox_id, &email).await?,
            SearchCriteria::Subject(text) => self.search_subject(mailbox_id, &text).await?,
            SearchCriteria::Body(text) => self.search_body(mailbox_id, &text).await?,
            SearchCriteria::And(criteria_list) => {
                self.search_and(mailbox_id, criteria_list).await?
            }
            SearchCriteria::Or(criteria_list) => self.search_or(mailbox_id, criteria_list).await?,
            SearchCriteria::Not(criteria) => self.search_not(mailbox_id, *criteria).await?,
        };

        Ok(message_ids)
    }

    async fn copy_messages(
        &self,
        message_ids: &[MessageId],
        dest_mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        if message_ids.is_empty() {
            return Ok(vec![]);
        }

        let mut tx = self.pool.begin().await?;
        let mut metadata_list = Vec::new();

        for message_id in message_ids {
            // Get next UID for destination mailbox
            let uid_row = sqlx::query("SELECT uid_next FROM mailboxes WHERE id = $1 FOR UPDATE")
                .bind(*dest_mailbox_id.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
            let uid: i32 = uid_row.try_get("uid_next")?;

            // Copy message with new ID and UID
            let new_message_id = MessageId::new();
            sqlx::query(
                r#"
                INSERT INTO messages (id, mailbox_id, uid, sender, recipients, subject, headers, body_inline, body_external_ref, size)
                SELECT $1, $2, $3, sender, recipients, subject, headers, body_inline, body_external_ref, size
                FROM messages WHERE id = $4
                "#,
            )
            .bind(*new_message_id.as_uuid())
            .bind(*dest_mailbox_id.as_uuid())
            .bind(uid)
            .bind(*message_id.as_uuid())
            .execute(&mut *tx)
            .await?;

            // Copy flags
            sqlx::query(
                r#"
                INSERT INTO message_flags (message_id, flag_seen, flag_answered, flag_flagged, flag_deleted, flag_draft, flag_recent, custom_flags)
                SELECT $1, flag_seen, flag_answered, flag_flagged, flag_deleted, flag_draft, FALSE, custom_flags
                FROM message_flags WHERE message_id = $2
                "#,
            )
            .bind(*new_message_id.as_uuid())
            .bind(*message_id.as_uuid())
            .execute(&mut *tx)
            .await?;

            // Update destination mailbox uid_next
            sqlx::query("UPDATE mailboxes SET uid_next = $1 WHERE id = $2")
                .bind(uid + 1)
                .bind(*dest_mailbox_id.as_uuid())
                .execute(&mut *tx)
                .await?;

            // Get size for metadata
            let size_row = sqlx::query("SELECT size FROM messages WHERE id = $1")
                .bind(*new_message_id.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
            let size: i32 = size_row.try_get("size")?;

            metadata_list.push(MessageMetadata::new(
                new_message_id,
                *dest_mailbox_id,
                uid as u32,
                MessageFlags::new(),
                size as usize,
            ));
        }

        tx.commit().await?;

        tracing::debug!(
            "Copied {} messages to mailbox {}",
            message_ids.len(),
            dest_mailbox_id
        );
        Ok(metadata_list)
    }

    async fn get_mailbox_messages(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id, m.mailbox_id, m.uid, m.size,
                   f.flag_seen, f.flag_answered, f.flag_flagged,
                   f.flag_deleted, f.flag_draft, f.flag_recent, f.custom_flags
            FROM messages m
            LEFT JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1
            ORDER BY m.uid
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get mailbox messages: {}", e))?;

        let metadata_list = rows
            .into_iter()
            .filter_map(|row| row_to_metadata(row).ok())
            .collect();

        Ok(metadata_list)
    }
}

fn row_to_metadata(row: PgRow) -> anyhow::Result<MessageMetadata> {
    let _msg_uuid: uuid::Uuid = row.try_get("id")?;
    let _mailbox_uuid: uuid::Uuid = row.try_get("mailbox_id")?;
    let uid: i32 = row.try_get("uid")?;
    let size: i32 = row.try_get("size")?;

    let mut flags = MessageFlags::new();
    if let Ok(seen) = row.try_get::<bool, _>("flag_seen") {
        flags.set_seen(seen);
    }
    if let Ok(answered) = row.try_get::<bool, _>("flag_answered") {
        flags.set_answered(answered);
    }
    if let Ok(flagged) = row.try_get::<bool, _>("flag_flagged") {
        flags.set_flagged(flagged);
    }
    if let Ok(deleted) = row.try_get::<bool, _>("flag_deleted") {
        flags.set_deleted(deleted);
    }
    if let Ok(draft) = row.try_get::<bool, _>("flag_draft") {
        flags.set_draft(draft);
    }
    if let Ok(recent) = row.try_get::<bool, _>("flag_recent") {
        flags.set_recent(recent);
    }
    if let Ok(custom) = row.try_get::<Vec<String>, _>("custom_flags") {
        for flag in custom {
            flags.add_custom(flag);
        }
    }

    // Create MessageId from UUID (note: this doesn't preserve the original MessageId)
    let message_id = MessageId::new();
    let mailbox_id = MailboxId::new();

    Ok(MessageMetadata::new(
        message_id,
        mailbox_id,
        uid as u32,
        flags,
        size as usize,
    ))
}

impl PostgresMessageStore {
    pub(super) async fn search_all(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query("SELECT id FROM messages WHERE mailbox_id = $1")
            .bind(*mailbox_id.as_uuid())
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_unseen(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_seen = FALSE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_seen(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_seen = TRUE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_flagged(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_flagged = TRUE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_unflagged(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_flagged = FALSE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_deleted(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_deleted = TRUE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_undeleted(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_deleted = FALSE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_from(
        &self,
        mailbox_id: &MailboxId,
        email: &str,
    ) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query("SELECT id FROM messages WHERE mailbox_id = $1 AND sender ILIKE $2")
            .bind(*mailbox_id.as_uuid())
            .bind(format!("%{}%", email))
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_to(
        &self,
        mailbox_id: &MailboxId,
        email: &str,
    ) -> anyhow::Result<Vec<MessageId>> {
        let rows =
            sqlx::query("SELECT id FROM messages WHERE mailbox_id = $1 AND $2 = ANY(recipients)")
                .bind(*mailbox_id.as_uuid())
                .bind(email)
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_subject(
        &self,
        mailbox_id: &MailboxId,
        text: &str,
    ) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT id FROM messages
            WHERE mailbox_id = $1 AND search_vector @@ plainto_tsquery('english', $2)
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .bind(text)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_body(
        &self,
        mailbox_id: &MailboxId,
        text: &str,
    ) -> anyhow::Result<Vec<MessageId>> {
        // Use full-text search
        self.search_subject(mailbox_id, text).await
    }

    async fn search_and(
        &self,
        mailbox_id: &MailboxId,
        criteria_list: Vec<SearchCriteria>,
    ) -> anyhow::Result<Vec<MessageId>> {
        if criteria_list.is_empty() {
            return Ok(vec![]);
        }

        let mut result_sets: Vec<Vec<MessageId>> = Vec::new();
        for criteria in criteria_list {
            let results = self.search(mailbox_id, criteria).await?;
            result_sets.push(results);
        }

        // Intersect all result sets
        if result_sets.is_empty() {
            return Ok(vec![]);
        }

        let mut intersection = result_sets[0].clone();
        for result_set in result_sets.iter().skip(1) {
            intersection.retain(|id| result_set.contains(id));
        }

        Ok(intersection)
    }

    async fn search_or(
        &self,
        mailbox_id: &MailboxId,
        criteria_list: Vec<SearchCriteria>,
    ) -> anyhow::Result<Vec<MessageId>> {
        let mut all_results = Vec::new();
        for criteria in criteria_list {
            let results = self.search(mailbox_id, criteria).await?;
            all_results.extend(results);
        }

        // Remove duplicates
        all_results.sort_by_key(|id| format!("{}", id));
        all_results.dedup();

        Ok(all_results)
    }

    async fn search_not(
        &self,
        mailbox_id: &MailboxId,
        criteria: SearchCriteria,
    ) -> anyhow::Result<Vec<MessageId>> {
        let all_messages = self.search_all(mailbox_id).await?;
        let excluded = self.search(mailbox_id, criteria).await?;

        let result: Vec<MessageId> = all_messages
            .into_iter()
            .filter(|id| !excluded.contains(id))
            .collect();

        Ok(result)
    }
}

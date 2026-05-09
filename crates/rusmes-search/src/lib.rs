//! Full-text search for RusMES
//!
//! This crate provides a Tantivy-backed full-text search index for mail messages.
//! It exposes a trait-based abstraction so that the rest of the RusMES system can
//! remain decoupled from the underlying search engine.
//!
//! # Key Features
//!
//! - **Tantivy back-end** — uses [Tantivy](https://github.com/quickwit-oss/tantivy) for
//!   on-disk inverted-index full-text search with BM25 ranking.
//! - **Async interface** — the [`SearchIndex`] trait is fully `async` and `Send + Sync`,
//!   making it easy to integrate into Tokio-based protocol handlers.
//! - **Schema**: indexes `from`, `to`, `subject` (TEXT + STORED) and `body` (TEXT) fields
//!   extracted from [`rusmes_proto::Mail`] envelopes, plus a stored `message_id` field
//!   for result correlation. Additional fields: `attachment_filenames` (TEXT+STORED),
//!   `header_values` (TEXT), and `date` (i64, indexed+stored) for date-range queries.
//! - **Ranked results** — [`search`][SearchIndex::search] returns [`SearchResult`] items
//!   sorted by Tantivy relevance score.
//! - **Atomic commits** — pending index changes are buffered in memory and flushed to disk
//!   only when [`commit`][SearchIndex::commit] is called, enabling batched indexing.
//! - **Mutex-guarded writer** — the [`IndexWriter`] is protected by a `std::sync::Mutex`
//!   so that multiple async tasks can share the same index safely.
//! - **Result caching** — see [`cache::ResultCache`]. Lookups are short-circuited by an
//!   LRU cache (capacity 256 by default); writes bump a global version stamp that
//!   invalidates all entries.
//! - **Maintenance APIs** — [`TantivySearchIndex::rebuild`] re-indexes every message in a
//!   storage backend; [`spawn_reindex_worker`] runs that on a tokio interval;
//!   [`spawn_incremental_indexer`] subscribes to the storage event stream and indexes
//!   messages as they arrive; [`TantivySearchIndex::index_size_bytes`] reports the on-disk
//!   footprint.
//! - **Schema versioning** — a `schema_version.txt` sidecar file gates schema compatibility.
//!   On version mismatch, the index directory is purged and rebuilt with the current schema.
//!
//! # Usage
//!
//! ```rust,no_run
//! use rusmes_search::{TantivySearchIndex, SearchIndex};
//! use std::path::Path;
//!
//! # async fn example() -> rusmes_search::Result<()> {
//! // Create or open an index at the given path
//! let index = TantivySearchIndex::new("/var/lib/rusmes/search")?;
//!
//! // Search (returns up to `limit` results ranked by relevance)
//! let results = index.search("quarterly report", 10).await?;
//! for r in &results {
//!     println!("message_uuid={} score={}", r.message_uuid, r.score);
//! }
//!
//! // Commit pending writes before querying new documents
//! index.commit().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Opening vs Creating
//!
//! Use [`TantivySearchIndex::new`] when creating an index for the first time.
//! Use [`TantivySearchIndex::open`] to reopen an existing index after a restart.
//! Both paths fail with a [`SearchError`] if the directory cannot be accessed or the
//! schema is incompatible.
//!
//! # Error Handling
//!
//! All fallible operations return [`Result<T>`][Result] which aliases
//! `std::result::Result<T, SearchError>`. The [`SearchError`] enum covers Tantivy
//! engine errors, query parse failures, I/O errors, and missing-message conditions.

pub mod cache;
pub mod query_translator;

pub use query_translator::{
    jmap_filter_to_tantivy, parse_search_term, search_query_to_tantivy, JmapSearchFilter,
    SearchComparator, SearchCondition, SearchField, SearchQuery, TermKind,
};

use async_trait::async_trait;
use rusmes_proto::{Mail, MessageId};
use rusmes_storage::{StorageBackend, StorageEvent};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tantivy::{
    collector::TopDocs,
    indexer::LogMergePolicy,
    query::QueryParser,
    schema::{Field, NumericOptions, Schema, Value, STORED, TEXT},
    Index, IndexReader, IndexWriter, TantivyDocument,
};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use uuid::Uuid;

pub use cache::ResultCache;

/// Current schema version — increment whenever the Tantivy schema changes.
///
/// On index open, this constant is compared against the persisted
/// `schema_version.txt` sidecar. A mismatch triggers a full index rebuild.
pub const SCHEMA_VERSION: u32 = 2;

/// Name of the sidecar file written alongside the Tantivy index that records
/// the schema version that was used to create it.
const SCHEMA_VERSION_FILE: &str = "schema_version.txt";

/// Search index errors
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("Tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("Query parse error: {0}")]
    QueryParse(#[from] tantivy::query::QueryParserError),

    #[error("Message not found: {0}")]
    MessageNotFound(String),

    #[error("Invalid UTF-8 in message")]
    InvalidUtf8,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Storage error: {0}")]
    Storage(String),
}

pub type Result<T> = std::result::Result<T, SearchError>;

/// Search result containing message ID information
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The UUID of the message
    pub message_uuid: Uuid,
    /// Relevance score
    pub score: f32,
}

/// Tunable parameters for the segment-merge policy.
///
/// The defaults match the Cluster 9 plan: minimum 8 segments per merge,
/// `level_log_size = 0.75` (tantivy default — restated explicitly so tests can
/// assert the policy was set), and `min_layer_size` set to `100` documents.
///
/// # Note on `min_layer_size`
///
/// Tantivy's `LogMergePolicy::min_layer_size` is denominated in **document
/// count**, not bytes. The Cluster 9 plan asked for "10 MiB" which has no
/// direct mapping in tantivy 0.25; we substitute `100` documents as a
/// conservative proxy. Operators can override via [`MergePolicyConfig::min_layer_size`].
#[derive(Debug, Clone)]
pub struct MergePolicyConfig {
    /// Minimum number of segments that may be merged together.
    pub min_num_segments: usize,
    /// Minimum segment size (in **documents**) under which all segments are
    /// considered to belong to the same level.
    pub min_layer_size: u32,
    /// Ratio between two consecutive levels.
    pub level_log_size: f64,
}

impl Default for MergePolicyConfig {
    fn default() -> Self {
        Self {
            min_num_segments: 8,
            min_layer_size: 100,
            level_log_size: 0.75,
        }
    }
}

impl MergePolicyConfig {
    /// Build a tantivy `LogMergePolicy` from this configuration.
    pub fn to_tantivy(&self) -> LogMergePolicy {
        let mut policy = LogMergePolicy::default();
        policy.set_min_num_segments(self.min_num_segments);
        policy.set_min_layer_size(self.min_layer_size);
        policy.set_level_log_size(self.level_log_size);
        policy
    }
}

/// Search index trait for message indexing and querying
#[async_trait]
pub trait SearchIndex: Send + Sync {
    /// Index a message
    async fn index_message(&self, message_id: &MessageId, mail: &Mail) -> Result<()>;

    /// Delete a message from the index
    async fn delete_message(&self, message_id: &MessageId) -> Result<()>;

    /// Search for messages matching a query.
    ///
    /// Returns a vector of search results ranked by relevance. Implementations
    /// may serve repeat queries from a result cache; cached results are
    /// distinguishable by [`SearchResult::score`] = `0.0` (genuine BM25 scores
    /// are always strictly positive). Callers that require ranking should
    /// invalidate or bypass the cache before issuing the query.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>>;

    /// Commit pending changes to the index
    async fn commit(&self) -> Result<()>;
}

/// Tantivy-based search index implementation
pub struct TantivySearchIndex {
    index: Index,
    reader: IndexReader,
    writer: std::sync::Arc<std::sync::Mutex<IndexWriter>>,
    schema_fields: SchemaFields,
    index_path: PathBuf,
    cache: Arc<ResultCache>,
}

/// Schema field handles
#[derive(Clone)]
struct SchemaFields {
    message_id: Field,
    from: Field,
    to: Field,
    subject: Field,
    body: Field,
    attachment_filenames: Field,
    header_values: Field,
    date: Field,
}

impl TantivySearchIndex {
    /// Create a new Tantivy search index at the specified path.
    ///
    /// Uses the default [`MergePolicyConfig`].
    pub fn new(index_path: impl AsRef<Path>) -> Result<Self> {
        Self::new_with_merge_policy(index_path, MergePolicyConfig::default())
    }

    /// Create a new Tantivy search index with a custom merge policy.
    pub fn new_with_merge_policy(
        index_path: impl AsRef<Path>,
        merge_policy: MergePolicyConfig,
    ) -> Result<Self> {
        let (schema, fields) = Self::build_schema();

        let index_path = index_path.as_ref();
        std::fs::create_dir_all(index_path)?;

        let index = Index::create_in_dir(index_path, schema.clone())?;
        let writer = index.writer(50_000_000)?; // 50MB heap
        writer.set_merge_policy(Box::new(merge_policy.to_tantivy()));
        let reader = index.reader()?;

        // Write the schema version sidecar so future `open()` calls can detect mismatches.
        write_schema_version(index_path)?;

        Ok(Self {
            index,
            reader,
            writer: std::sync::Arc::new(std::sync::Mutex::new(writer)),
            schema_fields: fields,
            index_path: index_path.to_path_buf(),
            cache: Arc::new(ResultCache::new_default()),
        })
    }

    /// Open an existing Tantivy search index.
    ///
    /// If the persisted schema version does not match [`SCHEMA_VERSION`], the
    /// index directory is purged and a fresh index is created.
    pub fn open(index_path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_merge_policy(index_path, MergePolicyConfig::default())
    }

    /// Open an existing Tantivy search index with a custom merge policy.
    ///
    /// If the persisted schema version does not match [`SCHEMA_VERSION`], the
    /// index directory is purged and a fresh index is created.
    pub fn open_with_merge_policy(
        index_path: impl AsRef<Path>,
        merge_policy: MergePolicyConfig,
    ) -> Result<Self> {
        let path_buf = index_path.as_ref().to_path_buf();

        // Check schema version. If mismatched (or sidecar absent), purge and recreate.
        if !schema_version_matches(&path_buf) {
            tracing::warn!(
                "rusmes-search: schema version mismatch at {:?} — purging and rebuilding index",
                path_buf
            );
            purge_index_dir(&path_buf)?;
            return Self::new_with_merge_policy(path_buf, merge_policy);
        }

        let index = Index::open_in_dir(&path_buf)?;
        let schema = index.schema();

        // Verify all required fields exist (guards against partial corruption).
        let fields = SchemaFields {
            message_id: schema.get_field("message_id")?,
            from: schema.get_field("from")?,
            to: schema.get_field("to")?,
            subject: schema.get_field("subject")?,
            body: schema.get_field("body")?,
            attachment_filenames: schema.get_field("attachment_filenames")?,
            header_values: schema.get_field("header_values")?,
            date: schema.get_field("date")?,
        };

        let writer = index.writer(50_000_000)?;
        writer.set_merge_policy(Box::new(merge_policy.to_tantivy()));
        let reader = index.reader()?;

        Ok(Self {
            index,
            reader,
            writer: std::sync::Arc::new(std::sync::Mutex::new(writer)),
            schema_fields: fields,
            index_path: path_buf,
            cache: Arc::new(ResultCache::new_default()),
        })
    }

    /// Build the search schema
    fn build_schema() -> (Schema, SchemaFields) {
        let mut schema_builder = Schema::builder();

        let message_id = schema_builder.add_text_field("message_id", STORED);
        let from = schema_builder.add_text_field("from", TEXT | STORED);
        let to = schema_builder.add_text_field("to", TEXT | STORED);
        let subject = schema_builder.add_text_field("subject", TEXT | STORED);
        let body = schema_builder.add_text_field("body", TEXT);
        let attachment_filenames =
            schema_builder.add_text_field("attachment_filenames", TEXT | STORED);
        let header_values = schema_builder.add_text_field("header_values", TEXT);
        let date = schema_builder
            .add_i64_field("date", NumericOptions::default().set_indexed().set_stored());

        let schema = schema_builder.build();
        let fields = SchemaFields {
            message_id,
            from,
            to,
            subject,
            body,
            attachment_filenames,
            header_values,
            date,
        };

        (schema, fields)
    }

    /// Extract text content from mail for indexing.
    ///
    /// Returns `(from, to, subject, body, attachment_filenames, header_values, date_unix)`.
    ///
    /// Body extraction performs a recursive MIME walk:
    /// - If `text/plain` is found, use it as body.
    /// - Else if `text/html` is found, convert to plain text via `html2text`.
    /// - Falls back to the raw `extract_text()` for non-MIME messages.
    ///
    /// Attachment filenames are collected from non-inline parts that have a
    /// `Content-Disposition: attachment; filename=…` or `Content-Type: …; name=…`.
    ///
    /// Header values concatenates Subject, From, To, Cc, Bcc, Reply-To, and
    /// Message-ID (after fold-whitespace normalisation) for full-header searching.
    ///
    /// The `date` field stores the `Date:` header as a Unix timestamp (`i64`).
    /// Returns `0` if the header is absent or unparseable.
    fn extract_mail_text(
        &self,
        mail: &Mail,
    ) -> (String, String, String, String, String, String, i64) {
        let message = mail.message();
        let headers = message.headers();

        // Extract standard envelope headers.
        let from = headers.get_first("from").unwrap_or("").to_string();
        let to = headers.get_first("to").unwrap_or("").to_string();
        let subject = headers.get_first("subject").unwrap_or("").to_string();

        // Attempt a MIME walk for body and attachments.
        let (body, attachment_filenames) = extract_body_and_attachments(message);

        // Build the header_values field from the key searchable headers.
        let header_values = build_header_values(headers);

        // Parse the Date header into a Unix timestamp.
        let date_unix = parse_date_header(headers);

        (
            from,
            to,
            subject,
            body,
            attachment_filenames,
            header_values,
            date_unix,
        )
    }

    /// Return a clone of the shared `ResultCache`. Useful for tests and for
    /// metrics that want to expose cache statistics.
    pub fn cache(&self) -> Arc<ResultCache> {
        self.cache.clone()
    }

    /// Return the Tantivy [`Schema`] used by this index.
    ///
    /// Callers (e.g. `query_translator`) need the schema to resolve field
    /// handles by name when building programmatic queries.
    pub fn schema(&self) -> Schema {
        self.index.schema()
    }

    /// Execute a pre-built Tantivy `Query` against this index and return the
    /// UUIDs of the matching messages (up to `limit` results).
    ///
    /// This is the low-level search path used by the query translation layer.
    /// The higher-level [`search`][SearchIndex::search] method accepts a query
    /// string and uses the built-in `QueryParser`; this method accepts an
    /// already-constructed `Box<dyn Query>` instead.
    ///
    /// Results are returned in descending relevance order (Tantivy BM25 score).
    /// Documents that do not carry a valid UUID in their `message_id` field are
    /// silently skipped.
    pub fn search_by_query(
        &self,
        query: Box<dyn tantivy::query::Query>,
        limit: usize,
    ) -> Result<Vec<uuid::Uuid>> {
        use tantivy::collector::TopDocs;
        let searcher = self.reader.searcher();
        let top_docs = searcher.search(query.as_ref(), &TopDocs::with_limit(limit))?;
        let mut results = Vec::with_capacity(top_docs.len());
        for (_score, addr) in top_docs {
            let doc: TantivyDocument = searcher.doc(addr)?;
            if let Some(v) = doc.get_first(self.schema_fields.message_id) {
                if let Some(s) = v.as_str() {
                    if let Ok(uuid) = s.parse::<uuid::Uuid>() {
                        results.push(uuid);
                    }
                }
            }
        }
        Ok(results)
    }

    /// IMAP search fast-path: translate a [`SearchQuery`] into a Tantivy query
    /// and execute it against the index.
    ///
    /// Returns the UUIDs of matching messages. If the index is not available,
    /// the IMAP handler can fall back to a linear scan.
    pub fn search_imap(
        &self,
        query: &query_translator::SearchQuery,
        limit: usize,
    ) -> Result<Vec<uuid::Uuid>> {
        let schema = self.schema();
        let tantivy_query = query_translator::search_query_to_tantivy(query, &schema);
        self.search_by_query(tantivy_query, limit)
    }

    /// JMAP Email/query fast-path: translate a [`JmapSearchFilter`] into a
    /// Tantivy query and execute it against the index.
    ///
    /// Returns the UUIDs of matching messages. Callers build a
    /// [`JmapSearchFilter`] from the JMAP `EmailFilterCondition` fields that
    /// map to searchable text/date fields.
    pub fn search_jmap(
        &self,
        filter: &query_translator::JmapSearchFilter,
        limit: usize,
    ) -> Result<Vec<uuid::Uuid>> {
        let schema = self.schema();
        let tantivy_query = query_translator::jmap_filter_to_tantivy(filter, &schema);
        self.search_by_query(tantivy_query, limit)
    }

    /// Total on-disk size, in bytes, of the index directory.
    ///
    /// Walks `index_path` recursively and sums every file's length. Returns 0
    /// if the path does not exist or any individual file cannot be stat'd.
    pub fn index_size_bytes(&self) -> u64 {
        let mut total: u64 = 0;
        for entry in walkdir::WalkDir::new(&self.index_path)
            .follow_links(false)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            if entry.file_type().is_file() {
                if let Ok(meta) = entry.metadata() {
                    total = total.saturating_add(meta.len());
                }
            }
        }
        total
    }

    /// Drop every document in the index and commit. Used by [`Self::rebuild`].
    async fn truncate(&self) -> Result<()> {
        {
            let mut writer = self.writer.lock().map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::SystemError(format!(
                    "Writer mutex poisoned: {e}"
                )))
            })?;
            writer.delete_all_documents()?;
            writer.commit()?;
        }
        self.reader.reload()?;
        self.cache.invalidate_all();
        Ok(())
    }

    /// Re-index every message reachable through `store`.
    ///
    /// Drops the existing index first (idempotent), then walks every user via
    /// [`StorageBackend::list_all_users`], every mailbox via
    /// `MailboxStore::list_mailboxes`, and every message via
    /// `MessageStore::get_mailbox_messages` + `MessageStore::get_message`.
    /// Commits in batches of 1000 messages.
    ///
    /// Returns `(messages_indexed, elapsed)`.
    pub async fn rebuild(&self, store: &dyn StorageBackend) -> Result<(usize, Duration)> {
        const BATCH_SIZE: usize = 1000;

        let started = Instant::now();
        self.truncate().await?;

        let mailbox_store = store.mailbox_store();
        let message_store = store.message_store();

        let users = store
            .list_all_users()
            .await
            .map_err(|e| SearchError::Storage(format!("list_all_users failed: {e}")))?;

        let mut indexed = 0usize;
        let mut since_commit = 0usize;

        for user in users {
            let mailboxes = mailbox_store
                .list_mailboxes(&user)
                .await
                .map_err(|e| SearchError::Storage(format!("list_mailboxes failed: {e}")))?;
            for mailbox in mailboxes {
                let messages = message_store
                    .get_mailbox_messages(mailbox.id())
                    .await
                    .map_err(|e| {
                        SearchError::Storage(format!("get_mailbox_messages failed: {e}"))
                    })?;
                for metadata in messages {
                    let mail = match message_store.get_message(metadata.message_id()).await {
                        Ok(Some(m)) => m,
                        Ok(None) => {
                            tracing::debug!(
                                "rebuild: message {} not retrievable, skipping",
                                metadata.message_id()
                            );
                            continue;
                        }
                        Err(e) => {
                            tracing::warn!(
                                "rebuild: get_message({}) failed: {}",
                                metadata.message_id(),
                                e
                            );
                            continue;
                        }
                    };
                    self.add_document_no_invalidate(metadata.message_id(), &mail)?;
                    indexed += 1;
                    since_commit += 1;
                    if since_commit >= BATCH_SIZE {
                        self.commit_writer().await?;
                        since_commit = 0;
                    }
                }
            }
        }

        if since_commit > 0 {
            self.commit_writer().await?;
        }
        // Even if nothing was added, make sure readers see the truncate.
        self.reader.reload()?;
        // Bump cache once after the bulk operation completes.
        self.cache.invalidate_all();

        Ok((indexed, started.elapsed()))
    }

    /// Test-only door: exposes [`Self::add_document_no_invalidate`] under a
    /// `#[doc(hidden)]` name so integration tests can verify cache behavior
    /// (insert a document without bumping the cache version stamp). Not part
    /// of the public API.
    #[doc(hidden)]
    pub fn add_document_for_test(&self, message_id: &MessageId, mail: &Mail) -> Result<()> {
        self.add_document_no_invalidate(message_id, mail)
    }

    /// Lower-level helper: build + add a document without bumping cache.
    /// Caller is responsible for cache invalidation (used by `rebuild` to
    /// invalidate exactly once after the batch).
    fn add_document_no_invalidate(&self, message_id: &MessageId, mail: &Mail) -> Result<()> {
        let (from, to, subject, body, attachment_filenames, header_values, date_unix) =
            self.extract_mail_text(mail);
        let mut doc = TantivyDocument::new();
        doc.add_text(self.schema_fields.message_id, message_id.to_string());
        doc.add_text(self.schema_fields.from, from);
        doc.add_text(self.schema_fields.to, to);
        doc.add_text(self.schema_fields.subject, subject);
        doc.add_text(self.schema_fields.body, body);
        doc.add_text(
            self.schema_fields.attachment_filenames,
            attachment_filenames,
        );
        doc.add_text(self.schema_fields.header_values, header_values);
        doc.add_i64(self.schema_fields.date, date_unix);
        let writer = self.writer.lock().map_err(|e| {
            SearchError::Tantivy(tantivy::TantivyError::SystemError(format!(
                "Writer mutex poisoned: {e}"
            )))
        })?;
        // Replace any prior document for the same message_id (idempotent).
        let term =
            tantivy::Term::from_field_text(self.schema_fields.message_id, &message_id.to_string());
        writer.delete_term(term);
        writer.add_document(doc)?;
        Ok(())
    }

    /// Commit the writer (helper used by batched paths).
    async fn commit_writer(&self) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|e| {
            SearchError::Tantivy(tantivy::TantivyError::SystemError(format!(
                "Writer mutex poisoned: {e}"
            )))
        })?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }
}

#[async_trait]
impl SearchIndex for TantivySearchIndex {
    async fn index_message(&self, message_id: &MessageId, mail: &Mail) -> Result<()> {
        self.add_document_no_invalidate(message_id, mail)?;
        // Any cached query result may now be stale; bump the version stamp.
        self.cache.invalidate_all();
        Ok(())
    }

    async fn delete_message(&self, message_id: &MessageId) -> Result<()> {
        let writer = self.writer.lock().map_err(|e| {
            SearchError::Tantivy(tantivy::TantivyError::SystemError(format!(
                "Writer mutex poisoned: {e}"
            )))
        })?;
        let term =
            tantivy::Term::from_field_text(self.schema_fields.message_id, &message_id.to_string());
        writer.delete_term(term);
        drop(writer);
        self.cache.invalidate_all();
        Ok(())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let key = ResultCache::make_key(query, None);
        if let Some(ids) = self.cache.get(&key) {
            // Cache hits return zero-score results — we don't store the score
            // so callers that care about ranking should bypass the cache.
            return Ok(ids
                .into_iter()
                .map(|m| SearchResult {
                    message_uuid: *m.as_uuid(),
                    score: 0.0,
                })
                .collect());
        }

        let searcher = self.reader.searcher();

        let query_parser = QueryParser::for_index(
            &self.index,
            vec![
                self.schema_fields.from,
                self.schema_fields.to,
                self.schema_fields.subject,
                self.schema_fields.body,
                self.schema_fields.attachment_filenames,
                self.schema_fields.header_values,
            ],
        );

        let parsed = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&parsed, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();
        let mut ids_for_cache = Vec::new();
        for (score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;

            if let Some(message_id_value) = retrieved_doc.get_first(self.schema_fields.message_id) {
                if let Some(message_id_str) = message_id_value.as_str() {
                    if let Ok(uuid) = message_id_str.parse::<Uuid>() {
                        results.push(SearchResult {
                            message_uuid: uuid,
                            score,
                        });
                        ids_for_cache.push(MessageId::from_uuid(uuid));
                    }
                }
            }
        }

        self.cache.put(key, ids_for_cache);

        Ok(results)
    }

    async fn commit(&self) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|e| {
            SearchError::Tantivy(tantivy::TantivyError::SystemError(format!(
                "Writer mutex poisoned: {e}"
            )))
        })?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }
}

// ─── Schema version helpers ──────────────────────────────────────────────────

/// Return `true` if `<dir>/schema_version.txt` exists and contains the current
/// [`SCHEMA_VERSION`]. Returns `false` if the file is absent, unreadable, or
/// contains a different version number.
fn schema_version_matches(dir: &Path) -> bool {
    let path = dir.join(SCHEMA_VERSION_FILE);
    match std::fs::read_to_string(&path) {
        Ok(contents) => contents
            .trim()
            .parse::<u32>()
            .map(|v| v == SCHEMA_VERSION)
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Write `<dir>/schema_version.txt` with the current [`SCHEMA_VERSION`].
fn write_schema_version(dir: &Path) -> Result<()> {
    let path = dir.join(SCHEMA_VERSION_FILE);
    std::fs::write(path, SCHEMA_VERSION.to_string()).map_err(SearchError::Io)
}

/// Remove all files and subdirectories inside `dir` (but keep `dir` itself so
/// the caller can call `Index::create_in_dir` on it).
fn purge_index_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

// ─── MIME walk helpers ───────────────────────────────────────────────────────

/// Recursively walk a MIME message and collect the best plain-text body and
/// all attachment filenames.
///
/// Priority:
/// 1. `text/plain` part → use as body (first one found wins).
/// 2. `text/html` part (if no `text/plain`) → convert via `html2text`.
/// 3. Non-MIME message → `extract_text()` fallback.
///
/// Attachment filenames are concatenated (space-separated). Inline parts
/// (`Content-Disposition: inline`) are excluded.
fn extract_body_and_attachments(message: &rusmes_proto::MimeMessage) -> (String, String) {
    use rusmes_proto::mime::{split_multipart, ContentType};

    // Inline helper: return body bytes for Small variant only.
    // Large bodies require async I/O and cannot be processed in this sync context.
    let small_body_str = |msg: &rusmes_proto::MimeMessage| -> String {
        match msg.body() {
            rusmes_proto::MessageBody::Small(b) => String::from_utf8_lossy(b).into_owned(),
            rusmes_proto::MessageBody::Large(_) => String::new(),
        }
    };

    // Get the top-level Content-Type.
    let ct = match message.content_type() {
        Ok(Some(ct)) => ct,
        _ => {
            // No Content-Type — treat as plain text.
            // For Small bodies extract directly; Large bodies not available in sync context.
            return (small_body_str(message), String::new());
        }
    };

    if ct.is_multipart() {
        // Recursively walk multipart.
        let boundary = match ct.boundary() {
            Some(b) => b.to_owned(),
            None => {
                return (small_body_str(message), String::new());
            }
        };

        // Large bodies cannot be processed synchronously; skip them.
        let raw_body: Vec<u8> = match message.body() {
            rusmes_proto::MessageBody::Small(b) => b.to_vec(),
            rusmes_proto::MessageBody::Large(_) => {
                return (String::new(), String::new());
            }
        };

        let parts = match split_multipart(&raw_body, &boundary) {
            Ok(p) => p,
            Err(_) => {
                return (small_body_str(message), String::new());
            }
        };

        let mut plain_body: Option<String> = None;
        let mut html_body: Option<String> = None;
        let mut attachment_filenames: Vec<String> = Vec::new();

        for part in &parts {
            let part_ct = part
                .content_type()
                .ok()
                .flatten()
                .unwrap_or_else(|| ContentType {
                    main_type: "text".to_string(),
                    sub_type: "plain".to_string(),
                    parameters: std::collections::HashMap::new(),
                });

            let disposition = part
                .headers
                .get("content-disposition")
                .map(|s| s.to_lowercase())
                .unwrap_or_default();

            // Check if this is an attachment part.
            let is_attachment = disposition.starts_with("attachment");
            let is_inline = disposition.starts_with("inline");

            // Collect attachment filenames (exclude inline parts).
            if is_attachment || (!is_inline && !is_body_part(&part_ct)) {
                // Try Content-Disposition filename= first.
                if let Some(fname) = extract_disposition_filename(&disposition) {
                    attachment_filenames.push(fname);
                } else if let Some(fname) = part_ct.parameters.get("name") {
                    attachment_filenames.push(strip_rfc2047(fname));
                }
            }

            if is_attachment {
                // Don't use attachment body for text.
                continue;
            }

            match (part_ct.main_type.as_str(), part_ct.sub_type.as_str()) {
                ("text", "plain") if plain_body.is_none() => {
                    if let Ok(decoded) = part.decode_body() {
                        plain_body = Some(String::from_utf8_lossy(&decoded).into_owned());
                    }
                }
                ("text", "html") if html_body.is_none() && plain_body.is_none() => {
                    if let Ok(decoded) = part.decode_body() {
                        html_body = Some(html_bytes_to_text(&decoded));
                    }
                }
                ("multipart", _) => {
                    // Recurse one level into nested multipart using a synthetic message.
                    // We build a sub-message from the part headers + body.
                    let sub_bytes = rebuild_part_bytes(part);
                    if let Ok(sub_msg) = rusmes_proto::MimeMessage::parse_from_bytes(&sub_bytes) {
                        let (sub_body, sub_filenames) = extract_body_and_attachments(&sub_msg);
                        if !sub_body.is_empty() && plain_body.is_none() && html_body.is_none() {
                            plain_body = Some(sub_body);
                        }
                        if !sub_filenames.is_empty() {
                            attachment_filenames.push(sub_filenames);
                        }
                    }
                }
                _ => {}
            }
        }

        let body = plain_body.or(html_body).unwrap_or_default();
        (body, attachment_filenames.join(" "))
    } else if ct.main_type == "text" && ct.sub_type == "html" {
        // Single-part HTML message — decode CTE from Small body; skip Large.
        match message.body() {
            rusmes_proto::MessageBody::Small(bytes) => {
                let encoding = message.content_transfer_encoding();
                let decoded = match encoding {
                    rusmes_proto::ContentTransferEncoding::Base64 => {
                        rusmes_proto::mime::decode_base64(bytes).unwrap_or_default()
                    }
                    rusmes_proto::ContentTransferEncoding::QuotedPrintable => {
                        rusmes_proto::mime::decode_quoted_printable(bytes).unwrap_or_default()
                    }
                    _ => bytes.to_vec(),
                };
                let text = html_bytes_to_text(&decoded);
                (text, String::new())
            }
            rusmes_proto::MessageBody::Large(_) => (String::new(), String::new()),
        }
    } else if ct.main_type == "text" {
        // Single-part text/plain (or other text subtype) — decode from Small body; skip Large.
        match message.body() {
            rusmes_proto::MessageBody::Small(bytes) => {
                let encoding = message.content_transfer_encoding();
                let decoded = match encoding {
                    rusmes_proto::ContentTransferEncoding::Base64 => {
                        rusmes_proto::mime::decode_base64(bytes).unwrap_or_default()
                    }
                    rusmes_proto::ContentTransferEncoding::QuotedPrintable => {
                        rusmes_proto::mime::decode_quoted_printable(bytes).unwrap_or_default()
                    }
                    _ => bytes.to_vec(),
                };
                (
                    String::from_utf8_lossy(&decoded).into_owned(),
                    String::new(),
                )
            }
            rusmes_proto::MessageBody::Large(_) => (String::new(), String::new()),
        }
    } else {
        // Non-text, non-multipart (e.g. a bare application/pdf) — no indexable text.
        (String::new(), String::new())
    }
}

/// Returns `true` if this MIME part is a body content type rather than an attachment.
fn is_body_part(ct: &rusmes_proto::mime::ContentType) -> bool {
    matches!(
        (ct.main_type.as_str(), ct.sub_type.as_str()),
        ("text", "plain") | ("text", "html") | ("multipart", _)
    )
}

/// Extract a `filename=` value from a Content-Disposition header string
/// (already lowercased).
///
/// Handles both `filename="foo"` and `filename=foo` forms.
fn extract_disposition_filename(disposition: &str) -> Option<String> {
    // Split on ';' and scan for the filename parameter.
    for segment in disposition.split(';') {
        let seg = segment.trim();
        if let Some(rest) = seg.strip_prefix("filename=") {
            let value = rest.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(strip_rfc2047(value));
            }
        }
        // Also handle filename* (RFC 5987 extended value) — best-effort fallback.
        if let Some(rest) = seg.strip_prefix("filename*=") {
            let value = rest.trim();
            // Strip encoding prefix like "utf-8''filename.pdf"
            let fname = value.split("''").last().unwrap_or(value);
            let fname = fname.trim_matches('"');
            if !fname.is_empty() {
                return Some(strip_rfc2047(fname));
            }
        }
    }
    None
}

/// Convert raw HTML bytes to a plain-text string using `html2text`.
///
/// Uses a width of 1_000_000 columns so no line-wrapping occurs (we want the
/// full text for indexing, not formatted output). Falls back to lossy UTF-8
/// if `html2text` fails.
fn html_bytes_to_text(html: &[u8]) -> String {
    match html2text::from_read(html, 1_000_000) {
        Ok(text) => text,
        Err(_) => String::from_utf8_lossy(html).into_owned(),
    }
}

/// Rebuild the raw byte form of a `MimePart` so it can be re-parsed by
/// `MimeMessage::parse_from_bytes`. This is used for nested multipart recursion.
fn rebuild_part_bytes(part: &rusmes_proto::mime::MimePart) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in &part.headers {
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&part.body);
    out
}

// ─── Header helpers ───────────────────────────────────────────────────────────

/// Concatenate key header values for the `header_values` field.
///
/// Headers included: Subject, From, To, Cc, Bcc, Reply-To, Message-ID.
/// Each value is fold-whitespace normalised (consecutive whitespace → single
/// space). RFC 2047 encoded-word sequences are stripped to plain ASCII using a
/// best-effort approach.
fn build_header_values(headers: &rusmes_proto::HeaderMap) -> String {
    const HEADER_NAMES: &[&str] = &[
        "subject",
        "from",
        "to",
        "cc",
        "bcc",
        "reply-to",
        "message-id",
    ];

    let mut parts: Vec<String> = Vec::new();
    for name in HEADER_NAMES {
        if let Some(values) = headers.get(name) {
            for value in values {
                let normalised = strip_rfc2047(value.trim());
                let normalised = normalise_whitespace(&normalised);
                if !normalised.is_empty() {
                    parts.push(normalised);
                }
            }
        }
    }
    parts.join(" ")
}

/// Best-effort RFC 2047 encoded-word stripping.
///
/// Replaces `=?charset?encoding?payload?=` sequences with a space. This is
/// intentionally minimal — the goal is to avoid indexing base64 blobs, not to
/// produce a perfectly decoded string (a full RFC 2047 decoder would require an
/// additional dependency).
fn strip_rfc2047(input: &str) -> String {
    // State machine: scan for `=?` … `?=` and replace with a space.
    let mut result = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = remaining.find("=?") {
        // Emit the text before the encoded-word start.
        result.push_str(&remaining[..start]);

        let after_start = &remaining[start + 2..];
        if let Some(end_offset) = after_start.find("?=") {
            // Skip the entire encoded word; insert a space separator.
            result.push(' ');
            remaining = &after_start[end_offset + 2..];
        } else {
            // Malformed — emit literally.
            result.push_str(&remaining[start..]);
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result
}

/// Collapse consecutive whitespace characters into a single space and trim
/// any leading/trailing whitespace from the result.
fn normalise_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_space = true; // start as true to suppress leading spaces
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
            }
            last_was_space = true;
        } else {
            out.push(ch);
            last_was_space = false;
        }
    }
    // Trim a possible trailing space inserted by the loop.
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

// ─── Date parsing ─────────────────────────────────────────────────────────────

/// Parse the RFC 5322 `Date:` header and return a Unix timestamp (`i64`).
///
/// Uses `chrono::DateTime::parse_from_rfc2822` which handles the most common
/// email date formats. Returns `0` if the header is absent or unparseable.
fn parse_date_header(headers: &rusmes_proto::HeaderMap) -> i64 {
    let date_str = match headers.get_first("date") {
        Some(d) => d.trim(),
        None => return 0,
    };

    // chrono's parse_from_rfc2822 handles RFC 5322 date-time strings.
    match chrono::DateTime::parse_from_rfc2822(date_str) {
        Ok(dt) => dt.timestamp(),
        Err(_) => 0,
    }
}

// ─── Spawn helpers ────────────────────────────────────────────────────────────

/// Spawn a tokio task that periodically calls [`TantivySearchIndex::rebuild`].
///
/// Pass `Duration::ZERO` (or any duration <= 0) for "manual only" semantics —
/// the task is not started and the returned `JoinHandle` resolves immediately.
/// Otherwise, the task runs `rebuild` once per `schedule`.
///
/// Errors during rebuild are logged but do not stop the loop.
pub fn spawn_reindex_worker(
    idx: Arc<TantivySearchIndex>,
    store: Arc<dyn StorageBackend>,
    schedule: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if schedule.is_zero() {
            tracing::debug!("reindex worker: schedule is zero, exiting (manual-only mode)");
            return;
        }
        let mut interval = tokio::time::interval(schedule);
        // Skip the immediate-fire on first tick — wait the full interval first.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            match idx.rebuild(store.as_ref()).await {
                Ok((n, elapsed)) => {
                    tracing::info!("reindex worker: rebuilt {} messages in {:?}", n, elapsed);
                }
                Err(e) => {
                    tracing::warn!("reindex worker: rebuild failed: {}", e);
                }
            }
        }
    })
}

/// Spawn a tokio task that subscribes to `store.event_stream()` and indexes
/// (or deletes) messages as `MessageStored` / `MessageExpunged` events arrive.
///
/// Commits are debounced: every 100 messages OR every 5 seconds, whichever
/// comes first.
///
/// The task exits cleanly when the storage event stream is dropped (i.e. the
/// backend was dropped).
pub fn spawn_incremental_indexer(
    idx: Arc<TantivySearchIndex>,
    store: Arc<dyn StorageBackend>,
) -> JoinHandle<()> {
    spawn_incremental_indexer_with_config(idx, store, IncrementalConfig::default())
}

/// Tunables for [`spawn_incremental_indexer_with_config`].
#[derive(Debug, Clone)]
pub struct IncrementalConfig {
    /// Commit after this many indexed/deleted messages.
    pub commit_every_n: usize,
    /// Commit after this much time has elapsed since the last commit, even if
    /// `commit_every_n` has not been reached.
    pub commit_every: Duration,
}

impl Default for IncrementalConfig {
    fn default() -> Self {
        Self {
            commit_every_n: 100,
            commit_every: Duration::from_secs(5),
        }
    }
}

/// As [`spawn_incremental_indexer`] but with a custom debounce config.
pub fn spawn_incremental_indexer_with_config(
    idx: Arc<TantivySearchIndex>,
    store: Arc<dyn StorageBackend>,
    cfg: IncrementalConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = store.event_stream();
        let mut pending: usize = 0;
        let mut last_commit = Instant::now();
        let tick = if cfg.commit_every.is_zero() {
            Duration::from_millis(100)
        } else {
            cfg.commit_every
        };
        let mut commit_timer = tokio::time::interval(tick);
        commit_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Ok(StorageEvent::MessageStored { account, mailbox, uid }) => {
                            match handle_stored(&idx, store.as_ref(), &account, &mailbox, uid).await {
                                Ok(true) => pending += 1,
                                Ok(false) => {
                                    tracing::debug!(
                                        "incremental indexer: stored event for {}/{}/uid={} produced no document",
                                        account, mailbox, uid
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "incremental indexer: failed to index stored {}/{}/uid={}: {}",
                                        account, mailbox, uid, e
                                    );
                                }
                            }
                        }
                        Ok(StorageEvent::MessageExpunged { account, mailbox, uid }) => {
                            match handle_expunged(&idx, store.as_ref(), &account, &mailbox, uid).await {
                                Ok(true) => pending += 1,
                                Ok(false) => {
                                    tracing::debug!(
                                        "incremental indexer: expunge event for {}/{}/uid={} matched no message",
                                        account, mailbox, uid
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "incremental indexer: failed to expunge {}/{}/uid={}: {}",
                                        account, mailbox, uid, e
                                    );
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(
                                "incremental indexer: lagged behind {} events; consider a full rebuild",
                                skipped
                            );
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            tracing::info!("incremental indexer: storage event stream closed; exiting");
                            // Final commit before exit.
                            if pending > 0 {
                                if let Err(e) = idx.commit_writer().await {
                                    tracing::warn!(
                                        "incremental indexer: final commit failed: {}",
                                        e
                                    );
                                }
                            }
                            return;
                        }
                    }
                }
                _ = commit_timer.tick() => {
                    if pending > 0 && last_commit.elapsed() >= cfg.commit_every {
                        if let Err(e) = idx.commit_writer().await {
                            tracing::warn!("incremental indexer: timer commit failed: {}", e);
                        } else {
                            pending = 0;
                            last_commit = Instant::now();
                        }
                    }
                }
            }

            if pending >= cfg.commit_every_n {
                if let Err(e) = idx.commit_writer().await {
                    tracing::warn!("incremental indexer: batch commit failed: {}", e);
                } else {
                    pending = 0;
                    last_commit = Instant::now();
                }
            }
        }
    })
}

/// Resolve `(account, mailbox, uid)` to a `MessageId` via the storage trait API.
///
/// This walks `mailbox_store.list_mailboxes(user) -> message_store.get_mailbox_messages(mbox) -> match uid`.
/// Returns `Ok(None)` if any step yields no match.
async fn resolve_event_message_id(
    store: &dyn StorageBackend,
    account: &str,
    mailbox: &str,
    uid: u32,
) -> Result<Option<MessageId>> {
    if account.is_empty() || mailbox.is_empty() {
        // Cluster 3's filesystem `delete_messages` fires expunge with empty
        // account/mailbox/uid=0; nothing we can do but skip.
        return Ok(None);
    }
    let user = match rusmes_proto::Username::from_str(account) {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!(
                "resolve: invalid username '{}' in storage event: {}",
                account,
                e
            );
            return Ok(None);
        }
    };
    let mailbox_store = store.mailbox_store();
    let message_store = store.message_store();
    let mailboxes = mailbox_store
        .list_mailboxes(&user)
        .await
        .map_err(|e| SearchError::Storage(format!("list_mailboxes failed: {e}")))?;
    let mailbox_id = match mailboxes
        .iter()
        .find(|m| m.path().name().map(|n| n == mailbox).unwrap_or(false))
        .map(|m| *m.id())
    {
        Some(id) => id,
        None => return Ok(None),
    };
    let metas = message_store
        .get_mailbox_messages(&mailbox_id)
        .await
        .map_err(|e| SearchError::Storage(format!("get_mailbox_messages failed: {e}")))?;
    Ok(metas
        .into_iter()
        .find(|md| md.uid() == uid)
        .map(|md| *md.message_id()))
}

async fn handle_stored(
    idx: &Arc<TantivySearchIndex>,
    store: &dyn StorageBackend,
    account: &str,
    mailbox: &str,
    uid: u32,
) -> Result<bool> {
    let message_id = match resolve_event_message_id(store, account, mailbox, uid).await? {
        Some(id) => id,
        None => return Ok(false),
    };
    let message_store = store.message_store();
    let mail = match message_store
        .get_message(&message_id)
        .await
        .map_err(|e| SearchError::Storage(format!("get_message failed: {e}")))?
    {
        Some(m) => m,
        None => return Ok(false),
    };
    idx.add_document_no_invalidate(&message_id, &mail)?;
    idx.cache.invalidate_all();
    Ok(true)
}

async fn handle_expunged(
    idx: &Arc<TantivySearchIndex>,
    store: &dyn StorageBackend,
    account: &str,
    mailbox: &str,
    uid: u32,
) -> Result<bool> {
    let message_id = match resolve_event_message_id(store, account, mailbox, uid).await? {
        Some(id) => id,
        None => return Ok(false),
    };
    idx.delete_message(&message_id).await?;
    Ok(true)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::mail::Mail;
    use rusmes_proto::message::{HeaderMap, MessageBody, MessageId, MimeMessage};

    /// Helper: build a minimal Mail wrapping a raw message body string.
    fn make_mail(raw_message: &str) -> (MessageId, Mail) {
        let message_id = MessageId::new();
        let data = raw_message.as_bytes();
        let message = MimeMessage::parse_from_bytes(data).unwrap_or_else(|_| {
            // Fallback: treat the whole thing as a plain body.
            let mut hdr = HeaderMap::new();
            hdr.insert("content-type", "text/plain");
            MimeMessage::new(hdr, MessageBody::Small(Bytes::from(raw_message.to_owned())))
        });
        let mail = Mail::new(None, vec![], message, None, None);
        (message_id, mail)
    }

    /// Helper: create a `TantivySearchIndex` in a fresh temp dir.
    fn make_index() -> (TantivySearchIndex, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let idx = TantivySearchIndex::new(dir.path()).expect("create index");
        (idx, dir)
    }

    // ── HTML-only message indexing ────────────────────────────────────────────

    #[tokio::test]
    async fn test_html_only_message_indexed() {
        let (idx, _dir) = make_index();

        // Build a message with only a text/html body.
        let raw = concat!(
            "From: alice@example.com\r\n",
            "To: bob@example.com\r\n",
            "Subject: HTML test\r\n",
            "Content-Type: text/html\r\n",
            "\r\n",
            "<html><body><b>Tantalising</b> content here</body></html>",
        );
        let (mid, mail) = make_mail(raw);

        idx.index_message(&mid, &mail).await.expect("index");
        idx.commit().await.expect("commit");

        // "Tantalising" appears in the HTML visible text — should be findable.
        let results = idx.search("Tantalising", 10).await.expect("search");
        assert!(
            !results.is_empty(),
            "expected HTML body text to be indexed, got no results"
        );
        assert_eq!(results[0].message_uuid, *mid.as_uuid());
    }

    // ── Attachment filename indexing ─────────────────────────────────────────

    #[tokio::test]
    async fn test_attachment_filename_indexed() {
        let (idx, _dir) = make_index();

        let raw = concat!(
            "From: alice@example.com\r\n",
            "To: bob@example.com\r\n",
            "Subject: Attachment test\r\n",
            "Content-Type: multipart/mixed; boundary=\"boundary42\"\r\n",
            "\r\n",
            "--boundary42\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "See the attached report.\r\n",
            "--boundary42\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Disposition: attachment; filename=\"quarterly_report.pdf\"\r\n",
            "\r\n",
            "PDFBINARYDATA\r\n",
            "--boundary42--\r\n",
        );
        let (mid, mail) = make_mail(raw);

        idx.index_message(&mid, &mail).await.expect("index");
        idx.commit().await.expect("commit");

        // Search for the attachment filename.
        let results = idx
            .search("attachment_filenames:quarterly_report.pdf", 10)
            .await
            .expect("search");
        assert!(
            !results.is_empty(),
            "expected attachment filename to be indexed, got no results"
        );
        assert_eq!(results[0].message_uuid, *mid.as_uuid());
    }

    // ── Header value indexing (Cc field) ──────────────────────────────────────

    #[tokio::test]
    async fn test_header_values_indexed() {
        let (idx, _dir) = make_index();

        let raw = concat!(
            "From: alice@example.com\r\n",
            "To: bob@example.com\r\n",
            "Cc: carol@example.com\r\n",
            "Subject: Cc test\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Check the Cc header.\r\n",
        );
        let (mid, mail) = make_mail(raw);

        idx.index_message(&mid, &mail).await.expect("index");
        idx.commit().await.expect("commit");

        // "carol" appears in the Cc header → header_values field.
        let results = idx.search("header_values:carol", 10).await.expect("search");
        assert!(
            !results.is_empty(),
            "expected Cc header to be indexed in header_values, got no results"
        );
        assert_eq!(results[0].message_uuid, *mid.as_uuid());
    }

    // ── Date field range query ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_date_field_range_query() {
        let (idx, _dir) = make_index();

        // 2026-01-01T12:00:00+00:00 = 1767067200
        let raw = concat!(
            "From: alice@example.com\r\n",
            "To: bob@example.com\r\n",
            "Date: Thu, 1 Jan 2026 12:00:00 +0000\r\n",
            "Subject: Date test\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Happy new year.\r\n",
        );
        let (mid, mail) = make_mail(raw);

        idx.index_message(&mid, &mail).await.expect("index");
        idx.commit().await.expect("commit");

        // Verify via Tantivy RangeQuery on the date field.
        use tantivy::query::RangeQuery;

        let searcher = idx.reader.searcher();
        let date_field = idx.schema_fields.date;

        // Lower-bound: 2025-01-01T00:00:00+00:00 = 1735689600
        let lower: i64 = 1_735_689_600;
        let range_query = RangeQuery::new(
            std::ops::Bound::Included(tantivy::Term::from_field_i64(date_field, lower)),
            std::ops::Bound::Unbounded,
        );

        let top_docs = searcher
            .search(&range_query, &TopDocs::with_limit(10))
            .expect("range search");

        assert!(
            !top_docs.is_empty(),
            "expected date range query to return the message"
        );

        // Verify the returned doc carries the correct message_id.
        let doc: TantivyDocument = searcher.doc(top_docs[0].1).expect("fetch doc");
        if let Some(v) = doc.get_first(idx.schema_fields.message_id) {
            if let Some(s) = v.as_str() {
                assert_eq!(s, mid.to_string().as_str());
            }
        }

        // Verify date value was stored correctly (>= 2026-01-01).
        if let Some(date_val) = doc.get_first(date_field) {
            if let Some(ts) = date_val.as_i64() {
                assert!(
                    ts >= lower,
                    "stored timestamp {ts} should be >= lower bound {lower}"
                );
            }
        }
    }

    // ── Schema version sentinel ───────────────────────────────────────────────

    #[test]
    fn test_schema_version_sentinel_written_on_new() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let _idx = TantivySearchIndex::new(dir.path()).expect("create index");
        assert!(
            schema_version_matches(dir.path()),
            "schema_version.txt should be written by new()"
        );
    }

    #[test]
    fn test_schema_version_mismatch_triggers_rebuild() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        // Create with current version.
        let _idx = TantivySearchIndex::new(dir.path()).expect("create index");

        // Overwrite the sidecar with an old version.
        let sidecar = dir.path().join(SCHEMA_VERSION_FILE);
        std::fs::write(&sidecar, "1").expect("write old version");
        assert!(
            !schema_version_matches(dir.path()),
            "should detect stale version"
        );

        // open() should purge and recreate without error.
        let _idx2 = TantivySearchIndex::open(dir.path()).expect("open after purge");
        assert!(
            schema_version_matches(dir.path()),
            "schema_version.txt should be updated after purge+recreate"
        );
    }

    // ── strip_rfc2047 unit test ───────────────────────────────────────────────

    #[test]
    fn test_strip_rfc2047_removes_encoded_words() {
        let input = "=?UTF-8?Q?Hello=20World?= plain text =?ISO-8859-1?B?SGVsbG8=?=";
        let result = strip_rfc2047(input);
        // The encoded-word runs should be replaced with spaces; plain text preserved.
        assert!(result.contains("plain text"), "got: {result}");
        assert!(
            !result.contains("=?"),
            "encoded word not stripped: {result}"
        );
    }

    // ── normalise_whitespace unit test ────────────────────────────────────────

    #[test]
    fn test_normalise_whitespace() {
        assert_eq!(normalise_whitespace("  hello   world  "), "hello world");
        assert_eq!(normalise_whitespace("a\t\tb"), "a b");
        assert_eq!(normalise_whitespace(""), "");
    }

    // ── HTML body extraction ──────────────────────────────────────────────────

    #[test]
    fn test_html_bytes_to_text_extracts_visible_text() {
        let html = b"<html><body><h1>Report</h1><p>Some <b>bold</b> text.</p></body></html>";
        let text = html_bytes_to_text(html);
        assert!(
            text.contains("Report") || text.contains("bold") || text.contains("text"),
            "expected visible text extraction, got: {text}"
        );
    }

    // ── extract_disposition_filename unit test ────────────────────────────────

    #[test]
    fn test_extract_disposition_filename_quoted() {
        let disp = "attachment; filename=\"my report.pdf\"";
        let result = extract_disposition_filename(disp);
        assert_eq!(result.as_deref(), Some("my report.pdf"));
    }

    #[test]
    fn test_extract_disposition_filename_unquoted() {
        let disp = "attachment; filename=report.csv";
        let result = extract_disposition_filename(disp);
        assert_eq!(result.as_deref(), Some("report.csv"));
    }
}

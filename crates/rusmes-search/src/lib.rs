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
//!   for result correlation.
//! - **Ranked results** — [`search`][SearchIndex::search] returns [`SearchResult`] items
//!   sorted by Tantivy relevance score.
//! - **Atomic commits** — pending index changes are buffered in memory and flushed to disk
//!   only when [`commit`][SearchIndex::commit] is called, enabling batched indexing.
//! - **Mutex-guarded writer** — the [`IndexWriter`] is protected by a `std::sync::Mutex`
//!   so that multiple async tasks can share the same index safely.
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

use async_trait::async_trait;
use rusmes_proto::{Mail, MessageId};
use std::path::Path;
use tantivy::{
    collector::TopDocs,
    query::QueryParser,
    schema::{Field, Schema, Value, STORED, TEXT},
    Index, IndexReader, IndexWriter, TantivyDocument,
};
use thiserror::Error;
use uuid::Uuid;

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

/// Search index trait for message indexing and querying
#[async_trait]
pub trait SearchIndex: Send + Sync {
    /// Index a message
    async fn index_message(&self, message_id: &MessageId, mail: &Mail) -> Result<()>;

    /// Delete a message from the index
    async fn delete_message(&self, message_id: &MessageId) -> Result<()>;

    /// Search for messages matching a query
    /// Returns a vector of search results ranked by relevance
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
}

/// Schema field handles
#[derive(Clone)]
struct SchemaFields {
    message_id: Field,
    from: Field,
    to: Field,
    subject: Field,
    body: Field,
}

impl TantivySearchIndex {
    /// Create a new Tantivy search index at the specified path
    pub fn new(index_path: impl AsRef<Path>) -> Result<Self> {
        let (schema, fields) = Self::build_schema();

        let index_path = index_path.as_ref();
        std::fs::create_dir_all(index_path)?;

        let index = Index::create_in_dir(index_path, schema.clone())?;
        let writer = index.writer(50_000_000)?; // 50MB heap
        let reader = index.reader()?;

        Ok(Self {
            index,
            reader,
            writer: std::sync::Arc::new(std::sync::Mutex::new(writer)),
            schema_fields: fields,
        })
    }

    /// Open an existing Tantivy search index
    pub fn open(index_path: impl AsRef<Path>) -> Result<Self> {
        let index = Index::open_in_dir(index_path.as_ref())?;
        let schema = index.schema();

        let fields = SchemaFields {
            message_id: schema.get_field("message_id")?,
            from: schema.get_field("from")?,
            to: schema.get_field("to")?,
            subject: schema.get_field("subject")?,
            body: schema.get_field("body")?,
        };

        let writer = index.writer(50_000_000)?;
        let reader = index.reader()?;

        Ok(Self {
            index,
            reader,
            writer: std::sync::Arc::new(std::sync::Mutex::new(writer)),
            schema_fields: fields,
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

        let schema = schema_builder.build();
        let fields = SchemaFields {
            message_id,
            from,
            to,
            subject,
            body,
        };

        (schema, fields)
    }

    /// Extract text content from mail for indexing
    fn extract_mail_text(&self, mail: &Mail) -> (String, String, String, String) {
        let message = mail.message();
        let headers = message.headers();

        // Extract From header
        let from = headers.get_first("from").unwrap_or("").to_string();

        // Extract To header
        let to = headers.get_first("to").unwrap_or("").to_string();

        // Extract Subject header
        let subject = headers.get_first("subject").unwrap_or("").to_string();

        // Extract body text
        let body = message.extract_text().unwrap_or_default();

        (from, to, subject, body)
    }
}

#[async_trait]
impl SearchIndex for TantivySearchIndex {
    async fn index_message(&self, message_id: &MessageId, mail: &Mail) -> Result<()> {
        let (from, to, subject, body) = self.extract_mail_text(mail);

        let mut doc = TantivyDocument::new();
        doc.add_text(self.schema_fields.message_id, message_id.to_string());
        doc.add_text(self.schema_fields.from, from);
        doc.add_text(self.schema_fields.to, to);
        doc.add_text(self.schema_fields.subject, subject);
        doc.add_text(self.schema_fields.body, body);

        let writer = self.writer.lock().map_err(|e| {
            SearchError::Tantivy(tantivy::TantivyError::SystemError(format!(
                "Writer mutex poisoned: {e}"
            )))
        })?;
        writer.add_document(doc)?;

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

        Ok(())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let searcher = self.reader.searcher();

        let query_parser = QueryParser::for_index(
            &self.index,
            vec![
                self.schema_fields.from,
                self.schema_fields.to,
                self.schema_fields.subject,
                self.schema_fields.body,
            ],
        );

        let query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;

            if let Some(message_id_value) = retrieved_doc.get_first(self.schema_fields.message_id) {
                if let Some(message_id_str) = message_id_value.as_str() {
                    if let Ok(uuid) = message_id_str.parse::<Uuid>() {
                        results.push(SearchResult {
                            message_uuid: uuid,
                            score,
                        });
                    }
                }
            }
        }

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

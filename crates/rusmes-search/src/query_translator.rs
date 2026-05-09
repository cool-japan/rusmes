//! Query translation layer: protocol search filters → Tantivy queries
//!
//! This module provides two main translation paths:
//!
//! 1. [`search_query_to_tantivy`] — converts the protocol-agnostic [`SearchQuery`]
//!    intermediary type into a Tantivy [`Query`] ready to be executed against the index.
//!    IMAP / POP3 / other protocol handlers convert their native filter structs into
//!    [`SearchQuery`] first, then call this function.
//!
//! 2. [`jmap_filter_to_tantivy`] — converts a [`JmapSearchFilter`] (reflecting
//!    JMAP RFC 8621 `EmailFilterCondition` fields) directly into a Tantivy query.
//!
//! Both translators handle:
//! - Per-field term searches with tokenizer-aware lowercasing
//! - Full-text search across multiple fields (OR union)
//! - Date-range queries on the i64 `date` field
//! - Boolean AND / OR / NOT composition
//! - Phrase detection (`"hello world"` in double-quotes) → `PhraseQuery`
//! - Fuzzy term matching (`hello~2`) → `FuzzyTermQuery`
//!
//! # Tokenization note
//!
//! Tantivy's default `TEXT` tokenizer lowercases all tokens at index time.
//! Every search term fed into a `TermQuery`, `PhraseQuery`, or `FuzzyTermQuery`
//! on a `TEXT` field **must** therefore be lowercased before the `Term` is
//! constructed, or the query will silently match nothing.

use std::ops::Bound;

use tantivy::{
    query::{
        AllQuery, BooleanQuery, FuzzyTermQuery, Occur, PhraseQuery, Query, RangeQuery, TermQuery,
    },
    schema::{IndexRecordOption, Schema},
    Term,
};

// ─── Protocol-agnostic intermediary types ────────────────────────────────────

/// The field(s) a search condition targets.
#[derive(Debug, Clone)]
pub enum SearchField {
    /// The `subject` index field.
    Subject,
    /// The `from` index field.
    From,
    /// The `to` index field.
    To,
    /// The `body` index field.
    Body,
    /// The `header_values` index field (Cc, Bcc, Reply-To, etc.).
    Header(String),
    /// Shorthand for `header_values` targeting `Cc`.
    Cc,
    /// Shorthand for `header_values` targeting `Bcc`.
    Bcc,
    /// Full-text: searches `subject`, `body`, and `header_values` simultaneously.
    FullText,
    /// The `attachment_filenames` index field.
    AttachmentFilenames,
}

/// The comparison / match style for a search condition.
#[derive(Debug, Clone)]
pub enum SearchComparator {
    /// Match any message where the field contains the given string.
    Contains(String),
    /// Exact equality match (lowercased single token).
    Equals(String),
    /// Date ≥ Unix timestamp.
    DateSince(i64),
    /// Date < Unix timestamp.
    DateBefore(i64),
    /// Date is on a specific day (`[ts, ts + 86400)`).
    DateOn(i64),
}

/// A single field+comparator condition.
#[derive(Debug, Clone)]
pub struct SearchCondition {
    pub field: SearchField,
    pub comparator: SearchComparator,
}

/// Protocol-agnostic search query tree.
///
/// Callers (IMAP handler, POP3 handler, …) convert their native filter types
/// into this enum and pass it to [`search_query_to_tantivy`].
#[derive(Debug, Clone)]
pub enum SearchQuery {
    /// A single field condition.
    Condition(SearchCondition),
    /// All sub-queries must match.
    And(Vec<SearchQuery>),
    /// At least one sub-query must match.
    Or(Vec<SearchQuery>),
    /// The sub-query must NOT match (combined with `AllQuery` positive clause).
    Not(Box<SearchQuery>),
    /// Match every document.
    All,
    /// Match no document (empty BooleanQuery).
    None,
}

// ─── JMAP intermediary ────────────────────────────────────────────────────────

/// Simplified JMAP-derived filter for Tantivy translation.
///
/// Mirrors the fields of `rusmes_jmap::types::EmailFilterCondition` that are
/// searchable via Tantivy. The JMAP handler constructs this from the richer
/// JMAP type.
#[derive(Debug, Clone, Default)]
pub struct JmapSearchFilter {
    /// RFC 8621 `text` — full-text search across all searchable fields.
    pub text: Option<String>,
    /// RFC 8621 `from`.
    pub from: Option<String>,
    /// RFC 8621 `to`.
    pub to: Option<String>,
    /// RFC 8621 `cc`.
    pub cc: Option<String>,
    /// RFC 8621 `bcc`.
    pub bcc: Option<String>,
    /// RFC 8621 `subject`.
    pub subject: Option<String>,
    /// RFC 8621 `body`.
    pub body: Option<String>,
    /// RFC 8621 `before` — Unix timestamp exclusive upper bound.
    pub before: Option<i64>,
    /// RFC 8621 `after` — Unix timestamp inclusive lower bound.
    pub after: Option<i64>,
}

// ─── Term kind detection ─────────────────────────────────────────────────────

/// Internal classification for how a raw search string should be interpreted.
#[derive(Debug, Clone)]
pub enum TermKind {
    /// An exact single token (already lowercased by the caller).
    Exact(String),
    /// A phrase: the string was wrapped in double-quotes.
    /// The inner vector holds the individual tokens (already lowercased).
    Phrase(Vec<String>),
    /// A fuzzy match with edit distance `distance`.
    Fuzzy {
        /// The term to match (already lowercased).
        term: String,
        /// Maximum edit distance (Levenshtein).
        distance: u8,
    },
}

/// Parse a raw search string into a [`TermKind`].
///
/// # Rules
/// - If the string starts and ends with `"`, the inner text is tokenised on
///   whitespace and returned as a [`TermKind::Phrase`].
/// - If the string ends with `~N` where N is a single decimal digit, it is
///   returned as [`TermKind::Fuzzy`] with that edit distance.
/// - Otherwise the whole string is returned as [`TermKind::Exact`].
///
/// All tokens / terms are lowercased so they match the Tantivy `TEXT`
/// tokenizer output.
pub fn parse_search_term(s: &str) -> TermKind {
    let trimmed = s.trim();

    // Phrase: starts and ends with double-quote and has content between them.
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        let inner = &trimmed[1..trimmed.len() - 1];
        let tokens: Vec<String> = inner
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase())
            .collect();
        if !tokens.is_empty() {
            return TermKind::Phrase(tokens);
        }
        // Degenerate empty phrase — fall through to Exact.
    }

    // Fuzzy: ends with ~N where N is a single digit.
    if let Some((base, dist_str)) = trimmed.rsplit_once('~') {
        if dist_str.len() == 1 {
            if let Ok(dist) = dist_str.parse::<u8>() {
                if !base.is_empty() {
                    return TermKind::Fuzzy {
                        term: base.to_lowercase(),
                        distance: dist,
                    };
                }
            }
        }
    }

    // Default: exact / multi-word (handle multi-word as phrase without quotes).
    let lower = trimmed.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.len() > 1 {
        // Multi-word without quotes → treat as phrase.
        return TermKind::Phrase(words.into_iter().map(String::from).collect());
    }

    TermKind::Exact(lower)
}

// ─── Tantivy query building helpers ──────────────────────────────────────────

/// Resolve a field name in `schema` and return the field handle.
///
/// Returns `None` if the field does not exist in the schema (should never
/// happen for well-formed schemas built by `TantivySearchIndex::build_schema`).
fn resolve_field(schema: &Schema, name: &str) -> Option<tantivy::schema::Field> {
    schema.get_field(name).ok()
}

/// Build a single-term or phrase query on a text field for the given search string.
///
/// Parses `value` through [`parse_search_term`] and constructs the appropriate
/// Tantivy query variant. Returns `None` if the field is not found in the
/// schema or the term list is empty.
fn build_text_query(schema: &Schema, field_name: &str, value: &str) -> Option<Box<dyn Query>> {
    let field = resolve_field(schema, field_name)?;
    match parse_search_term(value) {
        TermKind::Exact(word) if !word.is_empty() => {
            let term = Term::from_field_text(field, &word);
            Some(Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)))
        }
        TermKind::Phrase(tokens) if !tokens.is_empty() => {
            if tokens.len() == 1 {
                let term = Term::from_field_text(field, &tokens[0]);
                Some(Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)))
            } else {
                let terms: Vec<Term> = tokens
                    .iter()
                    .map(|t| Term::from_field_text(field, t))
                    .collect();
                Some(Box::new(PhraseQuery::new(terms)))
            }
        }
        TermKind::Fuzzy { term, distance } if !term.is_empty() => {
            let t = Term::from_field_text(field, &term);
            Some(Box::new(FuzzyTermQuery::new(t, distance, true)))
        }
        _ => None,
    }
}

/// Build a full-text query (OR union) across `subject`, `body`, and
/// `header_values` fields for the given search string.
fn build_fulltext_query(schema: &Schema, value: &str) -> Box<dyn Query> {
    let field_names = ["subject", "body", "header_values"];
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    for name in &field_names {
        if let Some(q) = build_text_query(schema, name, value) {
            clauses.push((Occur::Should, q));
        }
    }

    if clauses.is_empty() {
        // Fallback: match everything if no field could be found.
        Box::new(AllQuery)
    } else {
        Box::new(BooleanQuery::union_with_minimum_required_clauses(
            clauses
                .into_iter()
                .map(|(_, q)| q)
                .collect::<Vec<Box<dyn Query>>>(),
            1,
        ))
    }
}

/// Map a [`SearchField`] to the underlying Tantivy field name.
fn field_name_for(field: &SearchField) -> &'static str {
    match field {
        SearchField::Subject => "subject",
        SearchField::From => "from",
        SearchField::To => "to",
        SearchField::Body => "body",
        SearchField::Header(_) | SearchField::Cc | SearchField::Bcc => "header_values",
        SearchField::FullText => "body", // handled separately in the caller
        SearchField::AttachmentFilenames => "attachment_filenames",
    }
}

// ─── Primary translation entry point ─────────────────────────────────────────

/// Translate a [`SearchQuery`] into a Tantivy [`Query`] using the given schema.
///
/// This is the main entry point for IMAP (and other protocol) search handlers.
/// The returned `Box<dyn Query>` can be passed directly to
/// `Searcher::search(&query, &TopDocs::…)`.
///
/// # Invariants
///
/// - [`SearchQuery::All`] → [`AllQuery`] (matches every document).
/// - [`SearchQuery::None`] → an empty MUST-NOT boolean (matches nothing).
/// - [`SearchQuery::Not`] wraps its inner in a `MustNot` + `Must: AllQuery`
///   pair, because Tantivy requires at least one positive clause.
pub fn search_query_to_tantivy(query: &SearchQuery, schema: &Schema) -> Box<dyn Query> {
    match query {
        SearchQuery::All => Box::new(AllQuery),

        SearchQuery::None => {
            // Matches nothing: use an impossible TermQuery on a sentinel value
            // that can never appear in real data (null-byte prefix).
            if let Some(f) = resolve_field(schema, "message_id") {
                let t = Term::from_field_text(f, "\x00__none__\x00");
                Box::new(TermQuery::new(t, IndexRecordOption::Basic)) as Box<dyn Query>
            } else {
                Box::new(AllQuery) as Box<dyn Query>
            }
        }

        SearchQuery::Condition(cond) => translate_condition(cond, schema),

        SearchQuery::And(sub) => {
            if sub.is_empty() {
                return Box::new(AllQuery);
            }
            let clauses: Vec<(Occur, Box<dyn Query>)> = sub
                .iter()
                .map(|q| (Occur::Must, search_query_to_tantivy(q, schema)))
                .collect();
            Box::new(BooleanQuery::new(clauses))
        }

        SearchQuery::Or(sub) => {
            if sub.is_empty() {
                return Box::new(AllQuery);
            }
            let sub_queries: Vec<Box<dyn Query>> = sub
                .iter()
                .map(|q| search_query_to_tantivy(q, schema))
                .collect();
            Box::new(BooleanQuery::union_with_minimum_required_clauses(
                sub_queries,
                1,
            ))
        }

        SearchQuery::Not(inner) => {
            // Tantivy requires at least one positive clause in a BooleanQuery.
            // We add Must(AllQuery) so the MustNot clause is effective.
            let positive: Box<dyn Query> = Box::new(AllQuery);
            let negative = search_query_to_tantivy(inner, schema);
            Box::new(BooleanQuery::new(vec![
                (Occur::Must, positive),
                (Occur::MustNot, negative),
            ]))
        }
    }
}

/// Translate a single [`SearchCondition`] into a Tantivy query.
fn translate_condition(cond: &SearchCondition, schema: &Schema) -> Box<dyn Query> {
    match &cond.comparator {
        SearchComparator::Contains(value) | SearchComparator::Equals(value) => match &cond.field {
            SearchField::FullText => build_fulltext_query(schema, value),
            other => {
                let name = field_name_for(other);
                build_text_query(schema, name, value).unwrap_or_else(|| Box::new(AllQuery))
            }
        },

        SearchComparator::DateSince(ts) => {
            if let Some(date_field) = resolve_field(schema, "date") {
                let lower = Term::from_field_i64(date_field, *ts);
                Box::new(RangeQuery::new(Bound::Included(lower), Bound::Unbounded))
            } else {
                Box::new(AllQuery)
            }
        }

        SearchComparator::DateBefore(ts) => {
            if let Some(date_field) = resolve_field(schema, "date") {
                let upper = Term::from_field_i64(date_field, *ts);
                Box::new(RangeQuery::new(Bound::Unbounded, Bound::Excluded(upper)))
            } else {
                Box::new(AllQuery)
            }
        }

        SearchComparator::DateOn(ts) => {
            // Match messages where `ts <= date < ts + 86400` (one full day).
            if let Some(date_field) = resolve_field(schema, "date") {
                let lower = Term::from_field_i64(date_field, *ts);
                let upper = Term::from_field_i64(date_field, ts + 86_400);
                Box::new(RangeQuery::new(
                    Bound::Included(lower),
                    Bound::Excluded(upper),
                ))
            } else {
                Box::new(AllQuery)
            }
        }
    }
}

// ─── JMAP translation ─────────────────────────────────────────────────────────

/// Translate a [`JmapSearchFilter`] into a Tantivy [`Query`].
///
/// Each populated field in the filter produces a MUST clause; all clauses are
/// ANDed together. An empty filter (all `None`) returns [`AllQuery`].
///
/// The `text` field (RFC 8621 full-text) produces a SHOULD union across
/// `subject`, `body`, and `header_values`.
pub fn jmap_filter_to_tantivy(filter: &JmapSearchFilter, schema: &Schema) -> Box<dyn Query> {
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    // Full-text search across subject, body, header_values.
    if let Some(text) = &filter.text {
        if !text.is_empty() {
            clauses.push((Occur::Must, build_fulltext_query(schema, text)));
        }
    }

    // Per-field text conditions.
    let field_map: &[(&Option<String>, &str)] = &[
        (&filter.from, "from"),
        (&filter.to, "to"),
        (&filter.subject, "subject"),
        (&filter.body, "body"),
    ];

    for (opt, field_name) in field_map {
        if let Some(val) = opt {
            if !val.is_empty() {
                if let Some(q) = build_text_query(schema, field_name, val) {
                    clauses.push((Occur::Must, q));
                }
            }
        }
    }

    // Cc and Bcc map to header_values.
    for val in [&filter.cc, &filter.bcc].into_iter().flatten() {
        if !val.is_empty() {
            if let Some(q) = build_text_query(schema, "header_values", val) {
                clauses.push((Occur::Must, q));
            }
        }
    }

    // Date range (after = inclusive lower bound, before = exclusive upper bound).
    if let (Some(after), Some(before)) = (filter.after, filter.before) {
        if let Some(date_field) = resolve_field(schema, "date") {
            let lower = Term::from_field_i64(date_field, after);
            let upper = Term::from_field_i64(date_field, before);
            let range: Box<dyn Query> = Box::new(RangeQuery::new(
                Bound::Included(lower),
                Bound::Excluded(upper),
            ));
            clauses.push((Occur::Must, range));
        }
    } else if let Some(after) = filter.after {
        if let Some(date_field) = resolve_field(schema, "date") {
            let lower = Term::from_field_i64(date_field, after);
            let range: Box<dyn Query> =
                Box::new(RangeQuery::new(Bound::Included(lower), Bound::Unbounded));
            clauses.push((Occur::Must, range));
        }
    } else if let Some(before) = filter.before {
        if let Some(date_field) = resolve_field(schema, "date") {
            let upper = Term::from_field_i64(date_field, before);
            let range: Box<dyn Query> =
                Box::new(RangeQuery::new(Bound::Unbounded, Bound::Excluded(upper)));
            clauses.push((Occur::Must, range));
        }
    }

    if clauses.is_empty() {
        Box::new(AllQuery)
    } else {
        Box::new(BooleanQuery::new(clauses))
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SearchIndex;
    use bytes::Bytes;
    use rusmes_proto::mail::Mail;
    use rusmes_proto::message::{HeaderMap, MessageBody, MessageId, MimeMessage};

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a minimal schema identical to the one used by `TantivySearchIndex`.
    /// Used by unit tests that only need the schema (no actual index).
    fn make_schema() -> tantivy::schema::Schema {
        use tantivy::schema::{NumericOptions, STORED, TEXT};
        let mut b = tantivy::schema::SchemaBuilder::default();
        b.add_text_field("message_id", STORED);
        b.add_text_field("from", TEXT | STORED);
        b.add_text_field("to", TEXT | STORED);
        b.add_text_field("subject", TEXT | STORED);
        b.add_text_field("body", TEXT);
        b.add_text_field("attachment_filenames", TEXT | STORED);
        b.add_text_field("header_values", TEXT);
        b.add_i64_field("date", NumericOptions::default().set_indexed().set_stored());
        b.build()
    }

    // ─── parse_search_term ────────────────────────────────────────────────────

    #[test]
    fn test_parse_exact() {
        match parse_search_term("hello") {
            TermKind::Exact(s) => assert_eq!(s, "hello"),
            other => panic!("expected Exact, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_exact_lowercases() {
        match parse_search_term("Hello") {
            TermKind::Exact(s) => assert_eq!(s, "hello"),
            other => panic!("expected Exact, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_phrase() {
        match parse_search_term("\"hello world\"") {
            TermKind::Phrase(tokens) => {
                assert_eq!(tokens, vec!["hello", "world"]);
            }
            other => panic!("expected Phrase, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_phrase_lowercases() {
        match parse_search_term("\"Hello World\"") {
            TermKind::Phrase(tokens) => {
                assert_eq!(tokens, vec!["hello", "world"]);
            }
            other => panic!("expected Phrase, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_fuzzy() {
        match parse_search_term("hello~2") {
            TermKind::Fuzzy { term, distance } => {
                assert_eq!(term, "hello");
                assert_eq!(distance, 2);
            }
            other => panic!("expected Fuzzy, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_fuzzy_lowercases() {
        match parse_search_term("Hello~1") {
            TermKind::Fuzzy { term, distance } => {
                assert_eq!(term, "hello");
                assert_eq!(distance, 1);
            }
            other => panic!("expected Fuzzy, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_multiword_becomes_phrase() {
        match parse_search_term("hello world") {
            TermKind::Phrase(tokens) => {
                assert_eq!(tokens, vec!["hello", "world"]);
            }
            other => panic!("expected Phrase for multi-word, got {other:?}"),
        }
    }

    // ─── Full-pipeline tests using TantivySearchIndex ────────────────────────

    fn make_mail_raw(raw: &str) -> (MessageId, Mail) {
        let message_id = MessageId::new();
        let data = raw.as_bytes();
        let message = MimeMessage::parse_from_bytes(data).unwrap_or_else(|_| {
            let mut hdr = HeaderMap::new();
            hdr.insert("content-type", "text/plain");
            MimeMessage::new(hdr, MessageBody::Small(Bytes::from(raw.to_owned())))
        });
        let mail = Mail::new(None, vec![], message, None, None);
        (message_id, mail)
    }

    fn make_search_index() -> (crate::TantivySearchIndex, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let idx = crate::TantivySearchIndex::new(dir.path()).expect("create index");
        (idx, dir)
    }

    /// Index a message into `idx`, commit, and return the message id.
    async fn index_one(idx: &crate::TantivySearchIndex, raw: &str) -> MessageId {
        let (mid, mail) = make_mail_raw(raw);
        idx.index_message(&mid, &mail).await.expect("index");
        idx.commit().await.expect("commit");
        mid
    }

    // ── test_subject_query ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_subject_query() {
        let (idx, _dir) = make_search_index();

        // Note: Subject is "Hello World" — mixed-case, so translator must lowercase.
        let raw = concat!(
            "From: sender@example.com\r\n",
            "To: recv@example.com\r\n",
            "Subject: Hello World\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Some body text.\r\n",
        );
        let mid = index_one(&idx, raw).await;

        let schema = idx.schema();
        let query = search_query_to_tantivy(
            &SearchQuery::Condition(SearchCondition {
                field: SearchField::Subject,
                comparator: SearchComparator::Contains("Hello".to_string()),
            }),
            &schema,
        );

        let results = idx.search_by_query(query, 10).expect("search");
        assert!(
            !results.is_empty(),
            "subject query should return the indexed message"
        );
        assert_eq!(results[0], *mid.as_uuid());
    }

    // ── test_date_range_query ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_date_range_query() {
        let (idx, _dir) = make_search_index();

        // Two messages: one from 2025, one from 2024.
        // 2025-06-01T00:00:00Z = 1748736000
        // 2024-01-01T00:00:00Z = 1704067200
        let raw_recent = concat!(
            "From: alice@example.com\r\n",
            "Date: Sun, 1 Jun 2025 00:00:00 +0000\r\n",
            "Subject: Recent\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Recent message.\r\n",
        );
        let raw_old = concat!(
            "From: bob@example.com\r\n",
            "Date: Mon, 1 Jan 2024 00:00:00 +0000\r\n",
            "Subject: Old\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Old message.\r\n",
        );

        let mid_recent = index_one(&idx, raw_recent).await;
        let _mid_old = index_one(&idx, raw_old).await;

        let schema = idx.schema();

        // DateSince 2025-01-01 (1735689600) should match only the recent message.
        let ts_2025: i64 = 1_735_689_600;
        let query = search_query_to_tantivy(
            &SearchQuery::Condition(SearchCondition {
                field: SearchField::Subject, // field doesn't matter for date query
                comparator: SearchComparator::DateSince(ts_2025),
            }),
            &schema,
        );
        let results = idx.search_by_query(query, 10).expect("search");
        assert!(
            !results.is_empty(),
            "DateSince should match at least one message"
        );
        assert!(
            results.contains(mid_recent.as_uuid()),
            "DateSince should include the 2025 message"
        );

        // DateBefore 2025-01-01 should match only the old message.
        let query_before = search_query_to_tantivy(
            &SearchQuery::Condition(SearchCondition {
                field: SearchField::Subject,
                comparator: SearchComparator::DateBefore(ts_2025),
            }),
            &schema,
        );
        let results_before = idx
            .search_by_query(query_before, 10)
            .expect("search before");
        assert!(
            !results_before.contains(mid_recent.as_uuid()),
            "DateBefore should exclude the 2025 message"
        );
    }

    // ── test_full_text_query ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_full_text_query() {
        let (idx, _dir) = make_search_index();

        // One message has the word in Subject, another in body.
        let raw_subject = concat!(
            "From: alice@example.com\r\n",
            "Subject: Quarterly Report\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "See attached.\r\n",
        );
        let raw_body = concat!(
            "From: bob@example.com\r\n",
            "Subject: Meeting notes\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Quarterly budget review.\r\n",
        );

        let mid1 = index_one(&idx, raw_subject).await;
        let mid2 = index_one(&idx, raw_body).await;

        let schema = idx.schema();
        let filter = JmapSearchFilter {
            text: Some("quarterly".to_string()),
            ..Default::default()
        };
        let query = jmap_filter_to_tantivy(&filter, &schema);
        let results = idx.search_by_query(query, 10).expect("search");

        assert!(
            results.contains(mid1.as_uuid()),
            "full-text query should match subject field"
        );
        assert!(
            results.contains(mid2.as_uuid()),
            "full-text query should match body field"
        );
    }

    // ── test_phrase_query ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_phrase_query() {
        let (idx, _dir) = make_search_index();

        // Two messages: one contains the exact phrase, one has the words reversed.
        let raw_match = concat!(
            "From: alice@example.com\r\n",
            "Subject: Hello World Test\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "The phrase hello world appears here.\r\n",
        );
        let raw_no_match = concat!(
            "From: alice@example.com\r\n",
            "Subject: World Hello Test\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "The words world and hello appear in reverse.\r\n",
        );

        let mid_match = index_one(&idx, raw_match).await;
        let mid_no_match = index_one(&idx, raw_no_match).await;

        let schema = idx.schema();
        // Phrase query: "hello world" (adjacent tokens, must match in order).
        let query = search_query_to_tantivy(
            &SearchQuery::Condition(SearchCondition {
                field: SearchField::Body,
                comparator: SearchComparator::Contains("\"hello world\"".to_string()),
            }),
            &schema,
        );

        let results = idx.search_by_query(query, 10).expect("search");
        assert!(
            results.contains(mid_match.as_uuid()),
            "phrase query must match the message with adjacent 'hello world'"
        );
        assert!(
            !results.contains(mid_no_match.as_uuid()),
            "phrase query must NOT match 'world hello' (reversed order)"
        );
    }

    // ── test_fuzzy_query ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_fuzzy_query() {
        let (idx, _dir) = make_search_index();

        let raw = concat!(
            "From: alice@example.com\r\n",
            "Subject: Typo test\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "The word helo is misspelled.\r\n",
        );
        let mid = index_one(&idx, raw).await;

        let schema = idx.schema();
        // Fuzzy: "hello~1" should match "helo" (1 edit distance).
        let query = search_query_to_tantivy(
            &SearchQuery::Condition(SearchCondition {
                field: SearchField::Body,
                comparator: SearchComparator::Contains("hello~1".to_string()),
            }),
            &schema,
        );

        let results = idx.search_by_query(query, 10).expect("search");
        assert!(
            results.contains(mid.as_uuid()),
            "fuzzy query hello~1 should match 'helo'"
        );
    }

    // ── test_boolean_and ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_boolean_and() {
        let (idx, _dir) = make_search_index();

        let raw_both = concat!(
            "From: alice@example.com\r\n",
            "Subject: Budget Review\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Quarterly budget review.\r\n",
        );
        let raw_subject_only = concat!(
            "From: zach@example.com\r\n",
            "Subject: Budget Review\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Different content.\r\n",
        );

        let mid_both = index_one(&idx, raw_both).await;
        let mid_subject_only = index_one(&idx, raw_subject_only).await;

        let schema = idx.schema();
        // AND: Subject contains "budget" AND From contains "alice".
        let query = search_query_to_tantivy(
            &SearchQuery::And(vec![
                SearchQuery::Condition(SearchCondition {
                    field: SearchField::Subject,
                    comparator: SearchComparator::Contains("budget".to_string()),
                }),
                SearchQuery::Condition(SearchCondition {
                    field: SearchField::From,
                    comparator: SearchComparator::Contains("alice".to_string()),
                }),
            ]),
            &schema,
        );

        let results = idx.search_by_query(query, 10).expect("search");
        assert!(
            results.contains(mid_both.as_uuid()),
            "AND query should match the message with both 'budget' in subject and 'alice' in from"
        );
        assert!(
            !results.contains(mid_subject_only.as_uuid()),
            "AND query should NOT match message where from is 'zach', not 'alice'"
        );
    }

    // ── Unit test for translate_condition helper ───────────────────────────────

    #[test]
    fn test_translate_condition_date_since_does_not_panic() {
        let schema = make_schema();
        let cond = SearchCondition {
            field: SearchField::Subject,
            comparator: SearchComparator::DateSince(1_735_689_600),
        };
        // Must not panic.
        let _q = translate_condition(&cond, &schema);
    }

    #[test]
    fn test_jmap_filter_empty_returns_allquery_type() {
        let schema = make_schema();
        let filter = JmapSearchFilter::default();
        // Empty filter should build without panic and logically match everything.
        let q = jmap_filter_to_tantivy(&filter, &schema);
        // We can't inspect the concrete type, but we can verify it doesn't panic.
        let _ = q.box_clone();
    }
}

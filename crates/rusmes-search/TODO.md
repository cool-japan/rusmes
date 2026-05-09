# rusmes-search TODO

## Implemented ✅
- [x] `SearchIndex` trait definition
- [x] `TantivySearchIndex` implementation (Tantivy-based full-text search)
- [x] `index_message()` — extract text and add to index
- [x] `delete_message()` — remove document from index
- [x] `search()` — execute query, return ranked message IDs
- [x] `commit()` — flush pending changes to index

## Remaining
### Text Extraction
- [x] HTML-to-text conversion for HTML-only messages (done 2026-05-05)
  - **Goal:** Messages that contain only a `text/html` body part are indexed by their visible text, not raw HTML markup.
  - **Design:** Add `html2text = "0.13"` (Pure Rust, html5ever-based, MIT) to workspace deps. In the indexer MIME walk: if the message has no `text/plain` part but has a `text/html` part, pass the HTML bytes through `html2text::from_read(&html_bytes[..], 80)` and use the resulting plain text as the body field value. If both parts exist, use the `text/plain` (existing behaviour).
  - **Files:** `crates/rusmes-search/src/lib.rs`, `Cargo.toml` (workspace)
  - **Tests:** Index a `Content-Type: text/html` message containing `<b>Hello</b>`; search for "Hello" returns the message.
  - **Risk:** `html2text 0.13` is the latest stable release; check crates.io before adding.
- [x] Attachment filename indexing (done 2026-05-05)
  - **Goal:** Email attachments are findable by filename; `search("attachment_filename:report.pdf")` returns messages carrying a `report.pdf` attachment.
  - **Design:** Extend the Tantivy schema with a new `attachment_filenames: TEXT | STORED` field. In the indexer MIME walk, collect `Content-Disposition: filename=` and `Content-Type: name=` values from every non-inline attachment part. Concatenate them (space-separated) and write to the new field. Schema version is bumped and the index is rebuilt if the persisted schema-version sentinel mismatches.
  - **Files:** `crates/rusmes-search/src/lib.rs`
  - **Tests:** Index a message with a `Content-Disposition: attachment; filename="report.pdf"` part; search `attachment_filenames:"report.pdf"` returns it.
  - **Risk:** Schema bump invalidates existing indexes — ensure the rebuild path works correctly.
- [x] Header value extraction and normalization (done 2026-05-05)
  - **Goal:** Headers such as Cc, Bcc, Reply-To, and Message-ID are searchable; date-based range queries are possible.
  - **Design:** Extend Tantivy schema: `header_values: TEXT` (concatenates Subject, From, To, Cc, Bcc, Reply-To, Message-ID after RFC 2047 decoding); `date: i64` (stored as Unix timestamp, indexed as `NumericOptions::default().indexed()`). The `date` field is required by the Slice I query translator (IMAP SEARCH DATE / JMAP inMailboxOtherThan with date filter). Normalize header fold-whitespace before indexing.
  - **Files:** `crates/rusmes-search/src/lib.rs`
  - **Tests:** Index a message with `Cc: carol@example.com`; searching `header_values:"carol@example.com"` returns it. Index a message with `Date: Mon, 1 Jan 2026 12:00:00 +0000`; range query on `date` ≥ 2026-01-01 returns it.
  - **Risk:** RFC 2047 decoder — the project should already have one in `rusmes-proto`; reuse via `rusmes-proto` dep rather than pulling a new dep.

### Query Support
- [x] IMAP SEARCH criteria translation to Tantivy queries (done 2026-05-05)
  - **Goal:** The IMAP server can delegate SEARCH commands to the Tantivy index, returning a ranked list of matching message UIDs without scanning every message header.
  - **Design:** New module `crates/rusmes-search/src/query_translator.rs`. `pub fn imap_search_to_tantivy(criteria: &SearchKey, schema: &Schema) -> Box<dyn Query>` walks the `SearchKey` AST (already defined in `rusmes-proto` or `rusmes-imap`): `Subject(s)` → `TermQuery` on `subject`; `From(s)` → `TermQuery` on `from`; `Body(s)` → `TermQuery` on `body`; `Header(name, val)` → `TermQuery` on `header_values`; `SentSince(date)` → `RangeQuery` on `date` ≥ timestamp; `And(a,b)` / `Or(a,b)` / `Not(a)` → `BooleanQuery`. Phrase queries (adjacent tokens in quotes) → `PhraseQuery`. This slice **depends** on Slice C having added the `header_values` and `date` fields.
  - **Files:** `crates/rusmes-search/src/query_translator.rs` (new), `crates/rusmes-search/src/lib.rs` (re-export + expose `SearchHandle::search_imap`)
  - **Tests:** Representative IMAP SEARCH `SUBJECT "hello" FROM "alice@example.com" SINCE "01-Jan-2026"` returns only matching documents.
  - **Risk:** `SearchKey` type must be importable from `rusmes-imap` or `rusmes-proto`; subagent verifies the import path. Wave 2 slice — only dispatch after Wave 1 Slice C is complete.
- [x] JMAP Email/query filter translation (done 2026-05-05)
  - **Goal:** `Email/query` with a filter argument uses the Tantivy index for fast result sets.
  - **Design:** `pub fn jmap_filter_to_tantivy(filter: &EmailFilter, schema: &Schema) -> Box<dyn Query>` mirrors the IMAP translator. Maps `subject: String` → `TermQuery` on `subject`; `from: String` → `from`; `text: String` → `BooleanQuery { SHOULD: subject, from, to, body, header_values }`; `hasAttachment: true` → presence query on `attachment_filenames`; `before: UTCDate` → `RangeQuery` on `date`. Both translators live in `query_translator.rs`. Wave 2 — depends on Slice C.
  - **Files:** `crates/rusmes-search/src/query_translator.rs`, `crates/rusmes-search/src/lib.rs`
  - **Tests:** JMAP filter `{ "subject": "hi", "from": "alice" }` returns only matching messages; `{ "hasAttachment": true }` returns only messages with attachments.
  - **Risk:** `EmailFilter` type lives in `rusmes-jmap`; if creating a dep cycle, move the filter type to `rusmes-proto`. Subagent verifies and resolves.
- [x] Phrase search and fuzzy matching (done 2026-05-05)
  - **Goal:** Tantivy phrase queries and fuzzy term queries are exposed through the search API so IMAP and JMAP callers can request them explicitly.
  - **Design:** In `query_translator.rs`: detect the `"quoted phrase"` syntax in string search terms → `PhraseQuery::new(field, terms)`. Detect `~2` suffix (distance) → `FuzzyTermQuery::new(term, 2, true)`. Expose a `SearchOptions { phrase: bool, fuzzy: Option<u8> }` argument on `SearchHandle::search_text` (or as part of the JMAP/IMAP translator). Wave 2 — ships alongside Items 4 and 5.
  - **Files:** `crates/rusmes-search/src/query_translator.rs`, `crates/rusmes-search/src/lib.rs`
  - **Tests:** `"hello world"` phrase matches `hello world` but not `hello cruel world`; `hello~1` fuzzy matches `helo`.
  - **Risk:** Tantivy 0.22+ `FuzzyTermQuery` API — confirm constructor signature before calling.

### Performance & Maintenance
- [x] `rebuild()` — full reindex from storage backend (landed 2026-05-03)
  - **Goal.** Stream every message from a `&dyn StorageBackend`, call `index_message` per message, commit in batches of 1000, return `(messages_indexed, elapsed)`. Idempotent (drops the existing index first).
  - **Files.** `crates/rusmes-search/src/lib.rs` — `rebuild` function.
  - **Prerequisites.** Cluster 3 (`StorageEvent` broadcast channel and `event_stream()` trait method). Cluster 9 runs in Wave B after Cluster 3 returns.
  - **Tests.** `rebuild_indexes_all_messages`: store 20 messages, rebuild, assert all 20 are returned by search.
  - **Implementation note.** Cluster 3 did not provide a global message iterator on `StorageBackend`. Added a default trait method `async fn list_all_users(&self) -> anyhow::Result<Vec<Username>>` (returns empty by default; filesystem backend overrides it by walking `base_path/users/`). The rebuild walks `users -> mailboxes -> messages -> get_message` through the existing trait API. Batches commits in groups of 1000.
- [x] Background reindex worker (landed 2026-05-03)
  - **Goal.** `pub fn spawn_reindex_worker(idx: Arc<TantivySearchIndex>, store: Arc<dyn StorageBackend>, schedule: Duration) -> JoinHandle<()>`. Tokio task; on tick, calls `rebuild()`. Default schedule is "manual only" (no schedule) — turned on via config.
  - **Files.** `crates/rusmes-search/src/lib.rs` — `spawn_reindex_worker`.
  - **Prerequisites.** Cluster 3 (for `event_stream()`).
  - **Implementation note.** `Duration::ZERO` is the manual-only sentinel — the spawned task immediately returns without entering the loop. Otherwise the task uses `tokio::time::interval` with `MissedTickBehavior::Delay`. Errors during rebuild are logged but do not stop the loop.
- [x] Incremental indexing on message arrival (landed 2026-05-03)
  - **Goal.** Subscribe to Cluster 3's `StorageEvent` broadcast channel via `storage.event_stream()`. On `MessageStored`, fetch the message and call `index_message`. Commit periodically (every 100 messages or every 5 s, whichever first). On `MessageExpunged`, drop the document from the index.
  - **Files.** `crates/rusmes-search/src/lib.rs` — `spawn_incremental_indexer`, `spawn_incremental_indexer_with_config`, `IncrementalConfig`.
  - **Prerequisites.** Cluster 3 (for the `StorageEvent` broadcast channel and `event_stream()` trait method).
  - **Tests.** `incremental_indexing_on_event`: subscribe, deliver one message via filesystem backend, search finds it.
  - **Implementation note.** `StorageEvent` does not carry a `MessageId`, so the indexer resolves `(account, mailbox, uid)` to a `MessageId` by walking `mailbox_store.list_mailboxes(user) -> message_store.get_mailbox_messages(mbox) -> match uid`. Cluster 3's filesystem `delete_messages` currently fires expunge events with empty `account`/`mailbox`/`uid=0` (a Cluster 3 bug, not blocking here); the indexer treats those as no-ops. Commit is debounced by either 100 messages or 5 s, whichever comes first.
- [x] Index segment merging policy (landed 2026-05-03)
  - **Goal.** Configure tantivy `LogMergePolicy` with `min_merge_size: 8`, `min_layer_size: 10 MiB`, `level_log_size: 0.75`. Expose tunables via config.
  - **Files.** `crates/rusmes-search/src/lib.rs` — `MergePolicyConfig` + `new_with_merge_policy` + `open_with_merge_policy`.
  - **Implementation note.** Tantivy's `LogMergePolicy::min_layer_size` is denominated in **document count** (u32), not bytes. The Cluster 9 plan said `10 MiB`, which has no direct mapping; we substitute `100` documents as a conservative proxy and document the substitution in the rustdoc. `set_min_num_segments(8)`, `set_min_layer_size(100)`, `set_level_log_size(0.75)` are called explicitly so the policy is observable on inspection. `MergePolicyConfig` is `Clone + Default` and exposes all three knobs for operator override.
- [x] Index size monitoring (landed 2026-05-03)
  - **Goal.** `pub fn index_size_bytes(&self) -> u64` — sums `meta.json` + segment files. Cluster 7 publishes as a gauge.
  - **Files.** `crates/rusmes-search/src/lib.rs` — `index_size_bytes`.
  - **Tests.** `index_size_bytes_grows`: index 10 messages, commit, assert size > 0 and >= baseline.
  - **Implementation note.** Walks the index directory recursively via `walkdir::WalkDir` (does not follow symlinks) and sums every file's `metadata.len()`. Uses `saturating_add` so a hypothetical overflow returns `u64::MAX` rather than panicking.
- [x] Search result caching (landed 2026-05-03)
  - **Goal.** `lru::LruCache<(String /* normalized query */, String /* user */), Vec<MessageId>>` with capacity 256. Invalidated on any `index_message` for the same user.
  - **Files.** `crates/rusmes-search/src/cache.rs` — `ResultCache`.
  - **Tests.** `result_cache_hit`: same query twice; second is cached. Five additional unit tests in `cache::tests` cover normalization, key construction, version-stamp invalidation, user-aware keying, and round-trip.
  - **Implementation note.** Adopted the version-stamp approach from the plan rather than per-user walk-and-evict: `index_message` does not carry a user identity through the `SearchIndex` trait, so per-user invalidation cannot be done at the public API; an `AtomicU64` version counter is bumped on every write, and lookups whose stored stamp is below the current version are treated as stale and dropped. Capacity defaults to 256 entries.

## Proposed follow-ups

The following items were scoped and reviewed as part of the Cluster 9 plan:

- **IMAP SEARCH criteria translation to Tantivy queries** — Completed 2026-05-05 (Slice I).
- **JMAP Email/query filter translation** — Completed 2026-05-05 (Slice I).
- **Phrase search and fuzzy matching** — Completed 2026-05-05 (Slice I).

All previously deferred items (HTML-to-text conversion, attachment filename indexing, header value extraction and normalization) completed 2026-05-05.

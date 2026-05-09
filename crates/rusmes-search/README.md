# rusmes-search

Full-text search integration for RusMES. Will provide Tantivy-based indexing and query support for message search across IMAP SEARCH and JMAP Email/query.

## Status

Implemented. `SearchIndex` trait and `TantivySearchIndex` backend are fully
functional with index, delete, search, and commit operations.

## Architecture

```
SearchIndex
  |-- index_message()     - index a new message
  |-- delete_message()    - remove from index
  |-- search()            - execute a search query
  '-- rebuild()           - full reindex from storage

Backed by Tantivy (Rust-native full-text search engine)
```

## Index Fields

| Field | Type | Source |
|-------|------|--------|
| `message_id` | Stored | Message UUID |
| `mailbox_id` | Indexed | Mailbox UUID |
| `from` | Text | From header |
| `to` | Text | To/Cc headers |
| `subject` | Text | Subject header |
| `body` | Text | Plain text body (extracted from MIME) |
| `date` | Date | Date header |
| `has_attachment` | Boolean | MIME structure analysis |

## Dependencies
- `rusmes-proto` - mail types
- `tantivy` - full-text search engine
- `tokio` - async runtime

## Tests

```bash
cargo test -p rusmes-search   # 35 tests
```

Tests cover: index/search/delete operations, result caching, index size monitoring, and reindex worker functionality.

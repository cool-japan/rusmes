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
- [ ] HTML-to-text conversion for HTML-only messages
- [ ] Attachment filename indexing
- [ ] Header value extraction and normalization

### Query Support
- [ ] IMAP SEARCH criteria translation to Tantivy queries
- [ ] JMAP Email/query filter translation
- [ ] Phrase search and fuzzy matching

### Performance & Maintenance
- [ ] `rebuild()` — full reindex from storage backend
- [ ] Background reindex worker
- [ ] Incremental indexing on message arrival (currently manual commit)
- [ ] Index segment merging policy
- [ ] Index size monitoring
- [ ] Search result caching
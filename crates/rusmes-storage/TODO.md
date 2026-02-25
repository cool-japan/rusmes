# rusmes-storage TODO

## Implemented ✅
### Abstraction Layer
- [x] `MailboxStore`, `MessageStore`, `MetadataStore`, `QuotaStore` traits
- [x] `StorageEvent` types for notifications
- [x] MODSEQ tracking for CONDSTORE/QRESYNC (463 lines)
- [x] Storage metrics — Prometheus-compatible (1,000 lines)

### Filesystem Backend (1,582 lines)
- [x] Maildir format with proper flags encoding (`:2,DFPRST`)
- [x] Atomic message delivery (write to `tmp/`, rename to `new/`)
- [x] Quota enforcement
- [x] Mailbox subscriptions (IMAP LSUB)
- [x] Message expunge (permanent deletion)

### PostgreSQL Backend (2,592 lines)
- [x] All `MailboxStore`, `MessageStore`, `MetadataStore` methods
- [x] Connection pool (sqlx)
- [x] Full-text search index on message body/headers
- [x] Quota enforcement
- [x] MODSEQ tracking

### AmateRS Backend (1,390 lines)
- [-] **Mock implementation** — no real distributed system client
- [x] All trait methods implemented (with placeholder values)

## Remaining
### Filesystem
- [ ] Directory locking for concurrent access safety

### PostgreSQL
- [ ] Database migration tooling (sqlx-migrate or refinery)
- [ ] Vacuum/maintenance scheduling

### AmateRS
- [ ] Real distributed system client integration
- [ ] Replication factor and consistency level configuration
- [ ] Failover and retry logic

### General
- [ ] Storage backend factory from configuration enum
- [ ] Backup/restore API (exists in CLI, not as library API)
- [ ] Compaction/cleanup for deleted messages
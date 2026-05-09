# rusmes-storage

Storage abstraction layer for RusMES. Defines traits for mailbox, message, and metadata storage, with pluggable backend implementations.

## Architecture

```
StorageBackend (trait)
├── mailbox_store()  -> Arc<dyn MailboxStore>
├── message_store()  -> Arc<dyn MessageStore>
└── metadata_store() -> Arc<dyn MetadataStore>

Backends:
├── FilesystemBackend  (maildir format)
├── PostgresBackend    (SQL, schema defined)
└── AmatersBackend     (distributed, feature-gated as `amaters-backend`)
```

## Storage Traits

### MailboxStore
- `create_mailbox()` / `delete_mailbox()` / `rename_mailbox()`
- `get_mailbox()` / `list_mailboxes()`

### MessageStore
- `append_message()` - store a new message
- `get_message()` / `delete_messages()`
- `set_flags()` - update message flags (Seen, Answered, Flagged, etc.)
- `search()` - search by `SearchCriteria`

### MetadataStore
- `get_user_quota()` / `set_user_quota()`
- `get_mailbox_counters()` - message count, unread count, total size

## Types

| Type | Description |
|------|-------------|
| `Mailbox` | Mailbox with ID, path, and message count |
| `MailboxId` | UUID-based mailbox identifier |
| `MailboxPath` | Hierarchical mailbox path (user + name) |
| `MessageMetadata` | UID, flags, internal date, size |
| `MessageFlags` | Seen, Answered, Flagged, Deleted, Draft, Recent |
| `Quota` | Storage quota (bytes used / limit, message count / limit) |
| `SearchCriteria` | Search filters for message queries |
| `MailboxCounters` | Total messages, unread count, total size |

## Backends

### Filesystem (`backends::filesystem`)
Stores messages in maildir-compatible directory structure:
```
/var/lib/rusmes/mailboxes/
└── <user>/
    └── <mailbox>/
        ├── new/     (unread messages)
        ├── cur/     (read messages)
        └── tmp/     (delivery in progress)
```

### PostgreSQL (`backends::postgres`)
Schema with tables: `mailboxes`, `messages`, `user_quotas`. Full implementation
with connection pooling (sqlx), full-text search, quota enforcement, and MODSEQ
tracking (2,592 lines).

### AmateRS (`backends::amaters`)
AmateRS distributed storage — real client integration via `amaters-sdk-rust v0.2.0` (feature-gated as `amaters-backend`); initial-connect endpoint cycling for high availability. All trait methods fully implemented.

## Dependencies
- `rusmes-proto` - core types
- `tokio` - async filesystem I/O
- `async-trait` - async traits
- `uuid` - mailbox/message IDs

## Tests

```bash
cargo test -p rusmes-storage   # 195 tests
```

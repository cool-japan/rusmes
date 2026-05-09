-- Initial schema: mailboxes, subscriptions, user quotas
-- Supports both PostgreSQL and SQLite via conditional syntax guards.
-- For SQLite, TSVECTOR and JSONB columns are stored as TEXT/BLOB.

CREATE TABLE IF NOT EXISTS mailboxes (
    id          TEXT PRIMARY KEY,
    username    TEXT NOT NULL,
    path        TEXT NOT NULL,
    uid_validity INTEGER NOT NULL,
    uid_next     INTEGER NOT NULL,
    special_use  TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(username, path)
);

CREATE TABLE IF NOT EXISTS subscriptions (
    username      TEXT NOT NULL,
    mailbox_name  TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY(username, mailbox_name)
);

CREATE TABLE IF NOT EXISTS user_quotas (
    username    TEXT PRIMARY KEY,
    used        INTEGER NOT NULL DEFAULT 0,
    quota_limit INTEGER NOT NULL,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS messages (
    id                TEXT PRIMARY KEY,
    mailbox_id        TEXT NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
    uid               INTEGER NOT NULL,
    sender            TEXT,
    recipients        TEXT NOT NULL DEFAULT '[]',
    subject           TEXT,
    headers           TEXT NOT NULL DEFAULT '{}',
    body_inline       BLOB,
    body_external_ref TEXT,
    size              INTEGER NOT NULL,
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(mailbox_id, uid)
);

CREATE TABLE IF NOT EXISTS message_flags (
    message_id    TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    flag_seen     INTEGER NOT NULL DEFAULT 0,
    flag_answered INTEGER NOT NULL DEFAULT 0,
    flag_flagged  INTEGER NOT NULL DEFAULT 0,
    flag_deleted  INTEGER NOT NULL DEFAULT 0,
    flag_draft    INTEGER NOT NULL DEFAULT 0,
    flag_recent   INTEGER NOT NULL DEFAULT 0,
    custom_flags  TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY(message_id)
);

CREATE TABLE IF NOT EXISTS message_blobs (
    id         TEXT PRIMARY KEY,
    data       BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Indexes for common IMAP query patterns
CREATE INDEX IF NOT EXISTS idx_mailboxes_username ON mailboxes(username);
CREATE INDEX IF NOT EXISTS idx_mailboxes_path     ON mailboxes(path);
CREATE INDEX IF NOT EXISTS idx_messages_mailbox   ON messages(mailbox_id);
CREATE INDEX IF NOT EXISTS idx_messages_mailbox_uid ON messages(mailbox_id, uid);
CREATE INDEX IF NOT EXISTS idx_messages_sender    ON messages(sender);
CREATE INDEX IF NOT EXISTS idx_messages_created   ON messages(created_at);
CREATE INDEX IF NOT EXISTS idx_flags_message      ON message_flags(message_id);

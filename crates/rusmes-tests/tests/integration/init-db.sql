-- PostgreSQL initialization script for RusMES integration tests

-- Create tables for mail storage
CREATE TABLE IF NOT EXISTS users (
    id SERIAL PRIMARY KEY,
    username VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    email VARCHAR(255) UNIQUE NOT NULL,
    enabled BOOLEAN DEFAULT TRUE,
    quota_bytes BIGINT DEFAULT 1073741824, -- 1GB default
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS mailboxes (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    uidvalidity INTEGER NOT NULL,
    uidnext INTEGER DEFAULT 1,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(user_id, name)
);

CREATE TABLE IF NOT EXISTS messages (
    id SERIAL PRIMARY KEY,
    mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
    uid INTEGER NOT NULL,
    message_id VARCHAR(255),
    size_bytes BIGINT NOT NULL,
    internal_date TIMESTAMP NOT NULL,
    flags TEXT[], -- Array of flag strings
    raw_message BYTEA NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(mailbox_id, uid)
);

CREATE TABLE IF NOT EXISTS message_headers (
    id SERIAL PRIMARY KEY,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    header_name VARCHAR(255) NOT NULL,
    header_value TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS queue (
    id SERIAL PRIMARY KEY,
    message_id VARCHAR(255) UNIQUE NOT NULL,
    envelope_from VARCHAR(255) NOT NULL,
    envelope_to TEXT[] NOT NULL, -- Array of recipient addresses
    raw_message BYTEA NOT NULL,
    attempts INTEGER DEFAULT 0,
    max_attempts INTEGER DEFAULT 5,
    next_retry TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_error TEXT
);

-- Create indexes
CREATE INDEX idx_users_username ON users(username);
CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_mailboxes_user_id ON mailboxes(user_id);
CREATE INDEX idx_messages_mailbox_id ON messages(mailbox_id);
CREATE INDEX idx_messages_uid ON messages(mailbox_id, uid);
CREATE INDEX idx_messages_message_id ON messages(message_id);
CREATE INDEX idx_message_headers_message_id ON message_headers(message_id);
CREATE INDEX idx_message_headers_name ON message_headers(header_name);
CREATE INDEX idx_queue_next_retry ON queue(next_retry);

-- Insert test users
INSERT INTO users (username, password_hash, email, enabled) VALUES
    ('testuser', '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LeZDM8gcEZr3.8bXe', 'testuser@localhost', TRUE),
    ('admin', '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LeZDM8gcEZr3.8bXe', 'admin@localhost', TRUE),
    ('user1', '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LeZDM8gcEZr3.8bXe', 'user1@localhost', TRUE),
    ('user2', '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LeZDM8gcEZr3.8bXe', 'user2@localhost', TRUE)
ON CONFLICT (username) DO NOTHING;

-- Create default mailboxes for test users
INSERT INTO mailboxes (user_id, name, uidvalidity)
SELECT id, 'INBOX', extract(epoch from now())::integer
FROM users
WHERE username IN ('testuser', 'admin', 'user1', 'user2')
ON CONFLICT (user_id, name) DO NOTHING;

-- Grant privileges
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO rusmes;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public TO rusmes;

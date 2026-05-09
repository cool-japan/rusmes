# rusmes-proto

Core protocol types for the RusMES mail server. This crate defines the foundational data types used across all other RusMES crates. It has no I/O dependencies and is designed to be lightweight.

## Modules

| Module | Description |
|--------|-------------|
| `mail` | `Mail` envelope, `MailId`, `MailState` state machine, `AttributeValue` |
| `address` | `MailAddress`, `Domain`, `Username` with validation |
| `message` | `MimeMessage`, `MessageBody` (small/large), `HeaderMap`, `MessageId` |
| `error` | `MailError` enum and `Result` type alias |

## Key Types

### Mail
The central mail envelope wrapping a message with routing metadata:
- Envelope sender/recipients (separate from message headers)
- Processing state (`Root`, `Transport`, `LocalDelivery`, `Error`, `Ghost`, `Custom`)
- Attributes map for inter-mailet communication
- Remote client information (IP, hostname)
- `split()` method for partial-recipient processing

### MailAddress
Type-safe email address with validation:
- Local part (1-64 chars, no `@`)
- Domain (1-255 chars, lowercase normalized)
- `FromStr` implementation for parsing `"user@example.com"`

### MimeMessage
Message representation with streaming support:
- `MessageBody::Small(Bytes)` for messages <1MB
- `MessageBody::Large(...)` placeholder for streaming large messages
- `HeaderMap` for message headers

## Dependencies
- `serde` - serialization
- `uuid` - unique IDs
- `bytes` - efficient byte buffers
- `thiserror` - error types

## Usage

```rust
use rusmes_proto::{Mail, MailAddress, MimeMessage, MessageBody, HeaderMap, MailState};
use bytes::Bytes;

let sender: MailAddress = "sender@example.com".parse().unwrap();
let recipient: MailAddress = "recipient@example.com".parse().unwrap();
let message = MimeMessage::new(
    HeaderMap::new(),
    MessageBody::Small(Bytes::from("Hello, World!")),
);

let mail = Mail::new(Some(sender), vec![recipient], message, None, None);
assert_eq!(mail.state, MailState::Root);
```

## Tests

```bash
cargo test -p rusmes-proto   # 21 tests
```
